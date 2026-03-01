/// Overworld helpers: shared constants and read-only scan helpers.
///
/// The mechanical ROM write operations for fortress/lock/pipe placement
/// now live directly in `overworld_builder.rs`. This module provides
/// shared constants and utility functions used across overworld modules.

use crate::rom::Rom;

use super::rom_data::{self, TILE_AIRSHIP, TILE_BOWSER};

/// All path tiles that a lock/gap can be placed on.
pub(super) const LOCKABLE_TILES: &[u8] = &[
    0x45, // horizontal path
    0x46, // vertical path
    0xB3, // water bridge path
    0xDA, // sky bridge path
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

/// Get the airship or Bowser's castle grid position for a world.
/// Scans the map tile grid for the target tile.
#[allow(dead_code)]
pub(super) fn world_target_position(rom: &Rom, world_idx: usize) -> Option<(usize, usize)> {
    let grid = rom_data::read_tile_grid(rom, world_idx);
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
