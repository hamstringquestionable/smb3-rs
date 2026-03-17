/// Overworld helpers: shared constants and pure lookup functions.
///
/// These are stateless helpers used by the overworld builder pipeline for tile
/// classification, gap placement, and FX pattern lookup.

use super::rom_data::{Grid, TILE_AIRSHIP, TILE_BOWSER};

/// All path tiles eligible for lock/water-gap placement.
/// Locks (0x54, 0x56, 0xE4) are visual variants — all block the path equally.
/// Water gaps (0x9D) replace bridge tiles (0xB3) specifically.
pub(super) const LOCKABLE_TILES: &[u8] = &[
    0x45, // horizontal path
    0x46, // vertical path
    0xB3, // bridge (water) — becomes water gap 0x9D
    0xDA, // sky path — becomes sky lock 0xE4
    0xAC, // horizontal path variant
    0xB7, // horizontal path variant
    0xB8, // horizontal path variant
    0xB9, // horizontal path variant
    0xE6, // horizontal path variant
    0xAA, // vertical path variant
    0xAB, // vertical path variant
    0xB0, // vertical path variant
    0xB1, // vertical drawbridge
    0xB2, // horizontal drawbridge
    0xDB, // vertical path variant
    0xBA, // vertical path variant
];

/// Find the airship or Bowser's castle position on the grid.
pub(super) fn find_target(grid: &Grid, world_idx: usize) -> Option<(usize, usize)> {
    let target_tile = if world_idx == 7 { TILE_BOWSER } else { TILE_AIRSHIP };
    for r in 0..grid.rows {
        for c in 0..grid.cols {
            if grid.get(r, c) == target_tile {
                return Some((r, c));
            }
        }
    }
    None
}

/// Determine the lock/gap tile for a given path tile.
/// 0x54 = vertical lock, 0x56 = horizontal lock, 0xE4 = sky lock, 0x9D = water gap.
pub(super) fn gap_tile_for(tile: u8) -> u8 {
    match tile {
        0xB3 => 0x9D,                                          // bridge → water gap
        0xDA => 0xE4,                                          // sky path → sky lock
        0x46 | 0xAA | 0xAB | 0xB0 | 0xB1 | 0xDB | 0xBA => 0x54, // vertical path → vertical lock
        _ => 0x56,                                              // horizontal path → horizontal lock
    }
}

/// Pattern bytes for each FX type (keyed by the original path tile).
pub(super) fn fx_patterns_for(tile: u8) -> [u8; 4] {
    match tile {
        0xB3 => [0xD4, 0xD6, 0xD5, 0xD7],                    // water bridge
        0x46 | 0xAA | 0xAB | 0xB0 | 0xB1 | 0xDB | 0xBA =>
            [0xFE, 0xC0, 0xFE, 0xC0],                          // lock (vertical)
        _ => [0xFE, 0xFE, 0xE1, 0xE1],                        // bridge gap / sky
    }
}
