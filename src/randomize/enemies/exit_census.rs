//! Instrument: which enemy slots sit next to a pipe or door Mario can use?
//!
//! Small Mario can't kill a hazard, so a hazard in front of an exit can
//! block it. Exits are found by tile, the way `Player_DoSpecialTiles` and
//! the door logic (PRG008) recognise them:
//! - **Top/bottom pipes:** `$AD-$B0` (pipes 1 and 2), `$B3-$B4` (pipe 4),
//!   and `$BD-$BE` (pipe 5) where `PipeTile_EnableByTileset` bit 7 allows it.
//! - **Side pipes:** `$B5` (`TILE1_PIPEH1_B`), and `$BF` (`TILE8_PIPEH3_B`)
//!   where bit 6 allows it.
//! - **Doors:** `$43` (`TILEA_DOOR2`) anywhere; `$42` (`TILEA_DOOR1`) only
//!   in fortresses.
//!
//! Distance is in tiles, the larger of the column and row differences
//! between the enemy and the nearest exit tile, so it doesn't care which
//! side of the exit the enemy is on.
//!
//! Report-only. Run with
//! `cargo test --release --lib exit_census -- --ignored --nocapture`;
//! `EXIT_NEAR` sets the listing cut-off (default 3).

use std::collections::HashSet;

use super::hazard_census::{other_cover, swappable};
use super::protections::hazard_rules;
use super::tables::HAZARDS;
use crate::randomize::overworld::node_catalog::NodeCatalog;
use crate::randomize::rom_data::level_index::terrain::{self, ROWS, Terrain};
use crate::randomize::rom_data::level_index::{object_name, read_objects, trace};
use crate::rom::Rom;

/// `PipeTile_EnableByTileset` (PRG008), indexed by `Level_Tileset - 1`.
const PIPE_ENABLE: [u8; 14] =
    [0x00, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x40, 0x00, 0xC0, 0x00, 0xC0, 0x80];

/// Whether `tile` is a pipe mouth or door Mario can use in this tileset.
fn is_exit(tile: u8, tileset: u8) -> bool {
    let enable = PIPE_ENABLE.get((tileset as usize).wrapping_sub(1)).copied().unwrap_or(0);
    match tile {
        0xAD..=0xB0 | 0xB3 | 0xB4 | 0xB5 | 0x43 => true,
        0xBD | 0xBE => enable & 0x80 != 0,
        0xBF => enable & 0x40 != 0,
        0x42 => tileset == 2,
        _ => false,
    }
}

fn exits(t: &Terrain, width: usize, tileset: u8) -> Vec<(usize, usize)> {
    (0..width)
        .flat_map(|c| (0..ROWS).map(move |r| (c, r)))
        .filter(|&(c, r)| is_exit(t.tile(c / 16, r, c % 16), tileset))
        .collect()
}

#[test]
#[ignore = "instrument"]
fn exit_census() {
    let Ok(bytes) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") else { return };
    let rom = Rom::from_bytes_lax(&bytes, true).unwrap();
    let rules: HashSet<usize> = hazard_rules().into_iter().map(|r| r.0).collect();
    let near: usize = std::env::var("EXIT_NEAR").ok().and_then(|v| v.parse().ok()).unwrap_or(3);

    let (mut protected, mut open, mut vanilla) = (Vec::new(), Vec::new(), Vec::new());
    let mut seen = HashSet::new();
    for view in NodeCatalog::build(&rom, true).entry_views() {
        let Some(le) = &view.level_entry else { continue };
        let Ok((chain, _)) = trace(&rom, le) else { continue };
        for (i, area) in chain.iter().enumerate() {
            if !seen.insert(area.objects_ptr) || area.header.vertical() {
                continue;
            }
            let Ok(t) = terrain::load(&rom, area) else { continue };
            let Some(objects) = read_objects(&rom, area.objects_ptr, area.tileset) else {
                continue;
            };
            let width = area.header.screens() as usize * 16;
            let doors = exits(&t, width, area.tileset);
            for o in objects.iter().filter(|o| swappable(o.id)) {
                let (x, y) = (o.x as usize, o.y as usize);
                let d = doors.iter().map(|&(c, r)| c.abs_diff(x).max(r.abs_diff(y))).min();
                let line = |d: Option<usize>| {
                    format!(
                        "  {:<10} {:#07X} {:<20} x={:>3} y={:>2}  exit {}",
                        format!("{}/{i}", view.name),
                        o.offset,
                        object_name(o.id).unwrap_or("?"),
                        o.x,
                        o.y,
                        d.map_or("none in area".into(), |d| format!("{d:>2} tiles")),
                    )
                };
                if rules.contains(&o.offset) {
                    protected.push((d.unwrap_or(usize::MAX), line(d)));
                } else if d.is_some_and(|d| d <= near) {
                    let d = d.unwrap();
                    if HAZARDS.excludes(o.id, 0) {
                        vanilla.push((d, line(Some(d))));
                    } else {
                        let cover = other_cover(o.offset, o.id, area.objects_ptr)
                            .map_or(String::new(), |c| format!("  [{c}]"));
                        open.push((d, line(Some(d)) + &cover));
                    }
                }
            }
        }
    }

    for list in [&mut protected, &mut open, &mut vanilla] {
        list.sort();
    }
    println!(
        "Slots with an Exclude(HAZARDS) rule, by distance to the nearest exit ({}):",
        protected.len()
    );
    protected.iter().for_each(|p| println!("{}", p.1));
    println!("\nUnprotected slots within {near} tiles of an exit ({}):", open.len());
    open.iter().for_each(|p| println!("{}", p.1));
    println!("\nAlready a hazard in vanilla, within {near} tiles of an exit ({}):", vanilla.len());
    vanilla.iter().for_each(|p| println!("{}", p.1));
}
