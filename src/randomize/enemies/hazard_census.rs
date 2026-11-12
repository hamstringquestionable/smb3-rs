//! Instrument: would a terrain-based corridor check find the slots the
//! hand-written `Exclude(HAZARDS)` rules protect?
//!
//! For every swappable enemy slot in every horizontal area, measure the
//! headroom at its position in the area's real terrain (the engine's own
//! loader, `level_index::terrain`): the floor is the first tile at or below
//! the enemy Mario can stand on, the ceiling the first fully solid tile above
//! it. A slot whose headroom is low across its own column and both
//! neighbours is a corridor Mario cannot jump a hazard in.
//!
//! Report-only. Run with
//! `cargo test --release --lib hazard_census -- --ignored --nocapture`.

use std::collections::{BTreeMap, HashMap};

use super::protections::{
    EntryProtection, WalkerSegmentRule, entry_protection_at, hazard_rules, walker_segment_rule_at,
};
use super::tables::{
    BRO_ENEMIES, FLYING_ENEMIES, GHOST_ENEMIES, GROUND_ENEMIES, HAZARDS, ROTODISCS_DUAL,
    ROTODISCS_SINGLE, SHELL_ENEMIES, THWOMPS, WATER_ENEMIES,
};
use crate::randomize::overworld::node_catalog::NodeCatalog;
use crate::randomize::rom_data::level_index::terrain::{self, ROWS, Terrain};
use crate::randomize::rom_data::level_index::{Area, object_name, read_objects, trace};
use crate::randomize::rom_data::{STOMPABLE_ENEMIES, enemy_ptr_to_file_offset};
use crate::rom::Rom;

/// Every class a swap can turn into a hazard, or a hazard can come from.
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

fn swappable(id: u8) -> bool {
    CLASSES.iter().any(|class| class.contains(&id))
}

#[derive(Clone, Copy, PartialEq)]
enum Solid {
    No,
    Top,
    Full,
}

fn solidity(t: &Terrain, col: usize, row: usize) -> Solid {
    let tile = t.tile(col / 16, row, col % 16);
    let q = (tile >> 6) as usize;
    if tile >= t.attr[q + 4] {
        Solid::Full
    } else if tile >= t.attr[q] {
        Solid::Top
    } else {
        Solid::No
    }
}

/// Rows between the ceiling and the floor at `col`, measured from `row`.
/// `None` when there is no floor (a pit): nothing to walk past there.
fn headroom(t: &Terrain, col: usize, row: usize) -> Option<usize> {
    let floor = (row..ROWS).find(|&r| solidity(t, col, r) != Solid::No)?;
    let ceiling = (0..row).rev().find(|&r| solidity(t, col, r) == Solid::Full);
    Some(floor - ceiling.map_or(0, |c| c + 1))
}

/// The slot's headroom: the highest over its column and both neighbours, so
/// one block overhead is not a corridor; all three columns must be low.
/// `None` when the enemy is itself inside solid ground or every column is a pit.
fn slot_headroom(t: &Terrain, screens: usize, x: usize, y: usize) -> Option<usize> {
    if y >= ROWS || solidity(t, x, y) == Solid::Full {
        return None;
    }
    window(t, screens, x, y).filter_map(|c| headroom(t, c, y)).max()
}

/// The enemy's column and both neighbours, less any neighbour that is solid
/// at the enemy's row: that is a wall, not a corridor.
fn window(t: &Terrain, screens: usize, x: usize, y: usize) -> impl Iterator<Item = usize> + '_ {
    (x.saturating_sub(1)..=x + 1)
        .filter(move |&c| c < screens * 16 && solidity(t, c, y) != Solid::Full)
}

/// Whether some column of the window has a floor outside the enemy's own
/// band (on or above its ceiling, within jump height) with two open rows over it,
/// room for big Mario. Such a floor is another way past, so the corridor is
/// not forced. Reachability is not checked: a sealed pocket counts too.
/// How far above the corridor floor a ledge can be and still count as
/// reachable: roughly Mario's jump height, in tiles.
const JUMP_ROWS: usize = 5;

fn has_other_route(t: &Terrain, screens: usize, x: usize, y: usize) -> bool {
    window(t, screens, x, y).any(|c| {
        let Some(floor) = (y..ROWS).find(|&r| solidity(t, c, r) != Solid::No) else {
            return false;
        };
        let Some(ceiling) = (0..y).rev().find(|&r| solidity(t, c, r) == Solid::Full) else {
            return false;
        };
        // Only the top of the ceiling and up to jump height above the
        // corridor floor: further up is out of reach, and below the floor is
        // a pit, not a route.
        let outside = |r: usize| r <= ceiling && floor - r <= JUMP_ROWS;
        (2..ROWS).filter(|&r| outside(r)).any(|r| {
            solidity(t, c, r) != Solid::No
                && solidity(t, c, r - 1) != Solid::Full
                && solidity(t, c, r - 2) != Solid::Full
        })
    })
}

struct Slot {
    name: String,
    offset: usize,
    id: u8,
    x: u8,
    y: u8,
    headroom: Option<usize>,
    /// No other floor in the window: the corridor is the only way past.
    forced: bool,
    rule: Option<&'static str>,
    /// Already a hazard in vanilla: a rule cannot stop one being introduced.
    hazard: bool,
    /// A rule other than `Exclude(HAZARDS)` that still keeps hazards out.
    cover: Option<&'static str>,
}

#[test]
#[ignore = "instrument"]
fn hazard_census() {
    let Ok(bytes) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") else { return };
    let rom = Rom::from_bytes_lax(&bytes, true).unwrap();
    let rules: HashMap<usize, (&str, usize, &str)> =
        hazard_rules().into_iter().map(|(o, l, a, w)| (o, (l, a, w))).collect();

    // Name each area once, by the first level that reaches it.
    let mut areas: Vec<(String, Area)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for view in NodeCatalog::build(&rom, true).entry_views() {
        let Some(le) = &view.level_entry else { continue };
        let Ok((chain, _)) = trace(&rom, le) else { continue };
        for (i, area) in chain.into_iter().enumerate() {
            if seen.insert(area.objects_ptr) {
                areas.push((format!("{}/{i}", view.name), area));
            }
        }
    }

    let mut slots = Vec::new();
    let mut vertical_or_water = 0;
    for (name, area) in &areas {
        let Some(objects) = read_objects(&rom, area.objects_ptr, area.tileset) else { continue };
        let vertical = area.header.vertical();
        let t = if vertical { None } else { terrain::load(&rom, area).ok() };
        for o in objects.iter().filter(|o| swappable(o.id)) {
            let Some(t) = &t else {
                vertical_or_water += 1;
                continue;
            };
            let screens = area.header.screens() as usize;
            slots.push(Slot {
                name: name.clone(),
                offset: o.offset,
                id: o.id,
                x: o.x,
                y: o.y,
                headroom: slot_headroom(t, screens, o.x as usize, o.y as usize),
                forced: !has_other_route(t, screens, o.x as usize, o.y as usize),
                rule: rules.get(&o.offset).map(|r| r.2),
                cover: other_cover(o.offset, o.id, area.objects_ptr),
                hazard: HAZARDS.excludes(o.id, 0),
            });
        }
    }

    let protected: Vec<&Slot> = slots.iter().filter(|s| s.rule.is_some()).collect();
    println!(
        "{} swappable slots in {} areas ({} more in vertical areas, skipped); {} of {} hazard rules on them",
        slots.len(),
        areas.len(),
        vertical_or_water,
        protected.len(),
        rules.len()
    );
    println!(
        "\nheadroom <= T   corridor: protected  unprotected   forced corridor: protected  unprotected"
    );
    let open = |s: &Slot| s.rule.is_none() && s.cover.is_none() && !s.hazard;
    for cut in 2..=8 {
        let low = |s: &Slot| s.headroom.is_some_and(|h| h <= cut);
        let forced = |s: &Slot| low(s) && s.forced;
        println!(
            "  T={cut}            {:>2} / {:<2}     {:>3}                       {:>2} / {:<2}     {:>3}",
            protected.iter().filter(|s| low(s)).count(),
            protected.len(),
            slots.iter().filter(|s| open(s) && low(s)).count(),
            protected.iter().filter(|s| forced(s)).count(),
            protected.len(),
            slots.iter().filter(|s| open(s) && forced(s)).count(),
        );
    }

    println!("\nEvery protected slot, by stated reason:");
    let mut by_why: BTreeMap<&str, Vec<&Slot>> = BTreeMap::new();
    for s in &protected {
        by_why.entry(s.rule.unwrap()).or_default().push(s);
    }
    for (why, list) in by_why {
        println!("  {why}");
        for s in list {
            println!(
                "    {:<9} 0x{:05X} {:<20} x {:>3} y {:>2}  headroom {}{}",
                s.name,
                s.offset,
                object_name(s.id).unwrap_or("?"),
                s.x,
                s.y,
                s.headroom.map_or("-".into(), |h| h.to_string()),
                if s.forced { "  forced" } else { "" }
            );
        }
    }
    let unnamed: Vec<_> = rules.keys().filter(|o| !slots.iter().any(|s| s.offset == **o)).collect();
    println!("\nRules on slots this census did not measure: {}", unnamed.len());
    for o in unnamed {
        let (l, a, w) = rules[o];
        println!("    0x{o:05X} {l}/{a}: {w}");
    }

    let cut: usize = std::env::var("HAZARD_CUT").ok().and_then(|v| v.parse().ok()).unwrap_or(3);
    println!("\nUnprotected forced corridors, headroom <= {cut}:");
    for s in slots.iter().filter(|s| {
        s.rule.is_none()
            && s.cover.is_none()
            && !s.hazard
            && s.headroom.is_some_and(|h| h <= cut)
            && s.forced
    }) {
        println!(
            "    {:<9} 0x{:05X} {:<20} x {:>3} y {:>2}  headroom {}",
            s.name,
            s.offset,
            object_name(s.id).unwrap_or("?"),
            s.x,
            s.y,
            s.headroom.unwrap()
        );
    }
}

/// Whether a rule other than `Exclude(HAZARDS)` keeps hazards out of this
/// slot, and which: a skipped swap, a forced shell (no shell enemy is a
/// hazard), a forced stompable pick when no stompable enemy in the slot's own class
/// is a hazard (the Bros are stompable, but only a Bro slot can draw one), or
/// a whole object list the walker skips.
fn other_cover(offset: usize, id: u8, objects_ptr: u16) -> Option<&'static str> {
    let class = CLASSES.iter().find(|c| c.contains(&id)).copied().unwrap_or(&[]);
    let stompable_is_safe =
        !class.iter().any(|&c| STOMPABLE_ENEMIES.contains(&c) && HAZARDS.excludes(c, 0));
    match entry_protection_at(offset) {
        Some(EntryProtection::SkipSwap) => return Some("swap skipped"),
        Some(EntryProtection::ForceShell) => return Some("forced shell"),
        Some(EntryProtection::ForceStompable) if stompable_is_safe => {
            return Some("forced stompable");
        }
        _ => {}
    }
    let segment = enemy_ptr_to_file_offset(objects_ptr);
    (walker_segment_rule_at(segment) == WalkerSegmentRule::Skip).then_some("area skipped")
}
