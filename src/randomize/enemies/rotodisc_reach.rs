//! Instrument: which enemy slots could become a Rotodisc that hits Mario
//! the moment he arrives in an area?
//!
//! A Rotodisc never sits where it is placed: `ObjInit_RotoDiscDual*`
//! (PRG003 `$BE7F`) moves it 48 px left of its spawn point and
//! `RotoDiscSpin` swings it round that point, 48 px out on the axes and
//! up to ~55 px on the diagonals. [`orbit`] replays that code. Mario has
//! no control while he comes out of a door or pipe, so a disc whose orbit
//! crosses his arrival box can hit him before he can move.
//!
//! Arrival points, all decoded from the ROM without running the engine:
//! - **Level start:** header byte 4 bits 5-7 pick the Y (`GamePlay_YStart`,
//!   PRG030), byte 5 bits 5-6 the X (`Level_XStarts`, PRG008).
//! - **Junctions:** each junction command in an area says where Mario
//!   appears in that area's alt area (see "Junction Spawn Positions" in
//!   `docs/smb3_rom_reference.md`).
//!
//! Report-only. Run with
//! `cargo test --release --lib rotodisc_reach -- --ignored --nocapture`.

use std::collections::{BTreeMap, HashSet};

use super::protections::{
    EntryProtection, WalkerSegmentRule, entry_protection_at, walker_segment_rule_at,
};
use super::tables::{
    BRO_ENEMIES, FLYING_ENEMIES, GHOST_ENEMIES, GROUND_ENEMIES, ROTODISCS_DUAL, ROTODISCS_SINGLE,
    SHELL_ENEMIES, THWOMPS, WATER_ENEMIES,
};
use crate::randomize::overworld::node_catalog::NodeCatalog;
use crate::randomize::rom_data::enemy_ptr_to_file_offset;
use crate::randomize::rom_data::level_index::{
    Area, ChainEnd, object_name, read_commands, read_objects, trace,
};
use crate::rom::Rom;

/// Every class that can draw a Rotodisc: its own Shuffle pool, or the
/// global Wild pool any of these joins.
const CLASSES: [&[u8]; 9] = [
    GROUND_ENEMIES,
    SHELL_ENEMIES,
    FLYING_ENEMIES,
    WATER_ENEMIES,
    BRO_ENEMIES,
    THWOMPS,
    GHOST_ENEMIES,
    ROTODISCS_SINGLE,
    ROTODISCS_DUAL,
];

const DISC: i32 = 16;
/// Mario's box from his position: 16 wide, 32 tall (big Mario), which is
/// generous for small Mario on purpose.
const MARIO_W: i32 = 16;
const MARIO_H: i32 = 32;
/// Gap (px) under which a slot is listed as a near miss.
const NEAR: i32 = 16;

/// The disc's top-left, relative to its spawn point, every frame of one
/// lap and a bit (PRG003 `ObjInit_RotoDiscDualCW` + `RotoDiscSpin` with
/// `Var4 = 6`: accel ±2 every other frame, speed limit `$28`, 4.4 fixed
/// point). The counter-clockwise start is the mirror image, so the same
/// set of points covers both.
fn orbit() -> Vec<(i32, i32)> {
    let (lim, acc) = ([0x28, -0x28], [2, -2]);
    let (mut x, mut y, mut xv, mut yv) = (-48 * 16, 3 * 16, 0i32, -0x28i32);
    let (mut tx, mut ty) = (0usize, 0usize);
    let mut out = Vec::new();
    for frame in 1..=400 {
        x += xv;
        y += yv;
        if frame % 2 == 0 {
            if xv == lim[tx] {
                tx ^= 1;
            }
            xv += acc[tx];
            if yv == lim[ty] {
                ty ^= 1;
            }
            yv += acc[ty];
        }
        out.push((x / 16, y / 16));
    }
    out
}

/// Pixel gap between the disc's path and Mario's box; <= 0 is a hit.
fn gap(path: &[(i32, i32)], spawn: (i32, i32), mario: (i32, i32)) -> i32 {
    path.iter()
        .map(|&(dx, dy)| {
            let (x, y) = (spawn.0 + dx, spawn.1 + dy);
            let gx = (mario.0 - (x + DISC)).max(x - (mario.0 + MARIO_W));
            let gy = (mario.1 - (y + DISC)).max(y - (mario.1 + MARIO_H));
            gx.max(gy)
        })
        .min()
        .unwrap()
}

/// Mario's pixel position on arriving through a junction command whose
/// bytes 1-2 are `b1`, `b2` (`LevelJct_General`, PRG026 `$AA87`).
fn junction_arrival(b1: u8, b2: u8) -> (i32, i32) {
    const Y_STARTS: [u8; 8] = [0x00, 0x40, 0x70, 0xB0, 0xF0, 0x41, 0x71, 0x81];
    let centre = if b1 & 0x0F >= 3 { 0 } else { 8 };
    let x = (b2 & 0x0F) as i32 * 256 + (b2 & 0xF0) as i32 + centre;
    let v = Y_STARTS[((b1 >> 4) & 7) as usize];
    (x, (v & 0x0F) as i32 * 256 + (v & 0xF0) as i32)
}

/// Mario's pixel position at the level start (`LevelLoad`, PRG030; the X
/// from `Level_Initialize`, PRG008).
fn start_arrival(header: &[u8]) -> (i32, i32) {
    const Y_HI: [u8; 8] = [0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01];
    const Y_LO: [u8; 8] = [0x70, 0x40, 0x00, 0x40, 0x70, 0xB0, 0xF0, 0x80];
    const X: [u8; 4] = [0x18, 0x70, 0xD8, 0x80];
    let yi = (header[4] >> 5) as usize;
    (X[((header[5] >> 5) & 3) as usize] as i32, Y_HI[yi] as i32 * 256 + Y_LO[yi] as i32)
}

/// Where Mario appears (px) and what brings him there.
type Arrival = ((i32, i32), String);

/// What already keeps a Rotodisc out of this slot, if anything.
fn cover(offset: usize, id: u8, objects_ptr: u16) -> Option<&'static str> {
    match entry_protection_at(offset) {
        Some(EntryProtection::SkipSwap) => return Some("swap skipped"),
        Some(EntryProtection::ForceShell) => return Some("forced shell"),
        Some(EntryProtection::ForceStompable) => return Some("forced stompable"),
        Some(EntryProtection::Exclude(g))
            if ROTODISCS_SINGLE.iter().chain(ROTODISCS_DUAL).all(|&r| g.excludes(r, id)) =>
        {
            return Some("excludes Rotodiscs");
        }
        _ => {}
    }
    match walker_segment_rule_at(enemy_ptr_to_file_offset(objects_ptr)) {
        WalkerSegmentRule::Skip => Some("area skipped"),
        WalkerSegmentRule::HammerBro => Some("bro room"),
        WalkerSegmentRule::Default => None,
    }
}

#[test]
#[ignore = "instrument"]
fn rotodisc_reach() {
    let Ok(bytes) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") else { return };
    let rom = Rom::from_bytes_lax(&bytes, true).unwrap();
    let path = orbit();

    // Every area, named by the first level that reaches it, with where
    // Mario can arrive in it and from where.
    let mut names: BTreeMap<u16, (String, Area)> = BTreeMap::new();
    let mut arrivals: BTreeMap<u16, Vec<Arrival>> = BTreeMap::new();
    let mut seen = HashSet::new();
    for view in NodeCatalog::build(&rom, true).entry_views() {
        let Some(le) = &view.level_entry else { continue };
        let Ok((chain, end)) = trace(&rom, le) else { continue };
        if !seen.insert(chain[0].objects_ptr) {
            continue;
        }
        let at = arrivals.entry(chain[0].objects_ptr).or_default();
        at.push((start_arrival(&chain[0].header.bytes), "level start".into()));
        for (i, area) in chain.iter().enumerate() {
            names.entry(area.objects_ptr).or_insert((format!("{}/{i}", view.name), area.clone()));
            let next = match (i + 1 < chain.len(), end) {
                (true, _) => i + 1,
                (false, ChainEnd::LoopsTo(k)) => k,
                _ => continue,
            };
            for c in read_commands(&rom, area).iter().filter(|c| c.is_junction()) {
                arrivals.entry(chain[next].objects_ptr).or_default().push((
                    junction_arrival(c.bytes[1], c.bytes[2]),
                    format!("{}/{i} junction {:#07X}", view.name, c.offset),
                ));
            }
        }
    }

    let (mut hits, mut near, mut vanilla) = (Vec::new(), Vec::new(), Vec::new());
    for (ptr, points) in &arrivals {
        let Some((name, area)) = names.get(ptr) else { continue };
        let Some(objects) = read_objects(&rom, area.objects_ptr, area.tileset) else { continue };
        // ROTO_AREA=4F2 prints every arrival and slot of that level's areas.
        let detail = std::env::var("ROTO_AREA").is_ok_and(|a| name.starts_with(&format!("{a}/")));
        if detail {
            for ((x, y), from) in points {
                println!("{name}: arrive x={} y={} (px {x},{y}) from {from}", x / 16, y / 16);
            }
        }
        for o in objects.iter().filter(|o| CLASSES.iter().any(|c| c.contains(&o.id))) {
            let spawn = (o.x as i32 * 16, o.y as i32 * 16);
            let Some((g, from)) =
                points.iter().map(|(p, from)| (gap(&path, spawn, *p), from)).min_by_key(|t| t.0)
            else {
                continue;
            };
            if detail {
                println!("{name}:   {:#07X} x={} y={} gap {g}", o.offset, o.x, o.y);
            }
            if g > NEAR {
                continue;
            }
            let line = format!(
                "  {name:<12} {:#07X} {:<22} x={:>3} y={:>2}  gap {g:>3}px  from {from}{}",
                o.offset,
                object_name(o.id).unwrap_or("?"),
                o.x,
                o.y,
                cover(o.offset, o.id, area.objects_ptr)
                    .map_or(String::new(), |c| format!("  [{c}]")),
            );
            let is_disc = ROTODISCS_SINGLE.contains(&o.id) || ROTODISCS_DUAL.contains(&o.id);
            match (is_disc, g <= 0) {
                (true, _) => vanilla.push(line),
                (false, true) => hits.push(line),
                (false, false) => near.push(line),
            }
        }
    }

    let n: usize = arrivals.values().map(Vec::len).sum();
    println!("{n} arrival points in {} areas", arrivals.len());
    println!("\nA Rotodisc here would hit Mario as he arrives ({}):", hits.len());
    hits.iter().for_each(|l| println!("{l}"));
    println!("\nNear misses, gap 1-{NEAR}px ({}):", near.len());
    near.iter().for_each(|l| println!("{l}"));
    println!("\nVanilla Rotodiscs within {NEAR}px of an arrival ({}):", vanilla.len());
    vanilla.iter().for_each(|l| println!("{l}"));
}
