//! Instrument: which enemy slots sit on a landing between two gaps?
//!
//! A gap, a landing, then another gap: if Mario can't jump the whole run
//! he has to land on the landing, and a hazard there can't be avoided
//! (4-1's giant red troopas are the vanilla case, already protected).
//! This lists every gap-landing-gap with its widths, so a person can judge
//! which are too wide to jump over; it does not decide that itself.
//!
//! The landing is the run of columns standable on the enemy's floor row.
//! A gap column has nothing standable at or below that row, so a column
//! with lower ground is not a gap (Mario can drop in and walk on).
//!
//! Report-only. Run with
//! `cargo test --release --lib platform_census -- --ignored --nocapture`.

use std::collections::HashSet;

use super::hazard_census::{Solid, other_cover, solidity, swappable};
use super::protections::hazard_rules;
use super::tables::HAZARDS;
use crate::randomize::overworld::node_catalog::NodeCatalog;
use crate::randomize::rom_data::level_index::terrain::{self, ROWS, Terrain};
use crate::randomize::rom_data::level_index::{object_name, read_objects, trace};
use crate::rom::Rom;

/// Nothing Mario can stand on in this column at or below `row`.
fn is_gap(t: &Terrain, col: usize, row: usize) -> bool {
    (row..ROWS).all(|r| solidity(t, col, r) == Solid::No)
}

/// Mario can stand on top of this tile: it is standable and the tile
/// above it is not solid.
fn is_ledge(t: &Terrain, col: usize, row: usize) -> bool {
    solidity(t, col, row) != Solid::No && (row == 0 || solidity(t, col, row - 1) != Solid::Full)
}

/// Gap width counting outward from `start`, one column at a time.
fn gap_width(t: &Terrain, cols: impl Iterator<Item = usize>, row: usize) -> usize {
    cols.take_while(|&c| is_gap(t, c, row)).count()
}

#[test]
#[ignore = "instrument"]
fn platform_census() {
    let Ok(bytes) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") else { return };
    let rom = Rom::from_bytes_lax(&bytes, true).unwrap();
    let rules: HashSet<usize> = hazard_rules().into_iter().map(|r| r.0).collect();

    let mut seen = HashSet::new();
    let mut found = Vec::new();
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
            for o in objects.iter().filter(|o| swappable(o.id)) {
                let (x, y) = (o.x as usize, o.y as usize);
                if x >= width || y >= ROWS {
                    continue;
                }
                // The enemy's floor: the first ledge at or below it.
                let Some(floor) = (y..ROWS).find(|&r| is_ledge(&t, x, r)) else { continue };
                let left = (0..x).rev().take_while(|&c| is_ledge(&t, c, floor)).count();
                let right = (x + 1..width).take_while(|&c| is_ledge(&t, c, floor)).count();
                let (lo, hi) = (x - left, x + right);
                let gl = gap_width(&t, (0..lo).rev(), floor);
                let gr = gap_width(&t, hi + 1..width, floor);
                // A gap that runs off the area's edge is not a gap.
                if gl == 0 || gr == 0 || gl == lo || hi + gr + 1 == width {
                    continue;
                }
                let cover = if rules.contains(&o.offset) {
                    Some("excludes hazards")
                } else {
                    other_cover(o.offset, o.id, area.objects_ptr)
                };
                found.push((
                    left + right + 1,
                    format!(
                        "  {:<10} {:#07X} {:<20} x={:>3} y={:>2}  gap {gl:>2} | land {:>2} | gap {gr:>2}  span {:>2}{}{}",
                        format!("{}/{i}", view.name),
                        o.offset,
                        object_name(o.id).unwrap_or("?"),
                        o.x,
                        o.y,
                        left + right + 1,
                        gl + left + right + 1 + gr,
                        if HAZARDS.excludes(o.id, 0) { "  [vanilla hazard]" } else { "" },
                        cover.map_or(String::new(), |c| format!("  [{c}]")),
                    ),
                ));
            }
        }
    }

    found.sort_by_key(|f| f.0);
    println!("Enemy slots on a landing between two gaps ({}), narrowest first:", found.len());
    found.iter().for_each(|f| println!("{}", f.1));
}
