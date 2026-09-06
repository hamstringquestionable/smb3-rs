//! Overworld helpers: shared constants and pure lookup functions.
//!
//! These are stateless helpers used by the overworld builder pipeline for tile
//! classification, gap placement, and FX pattern lookup.

use super::rom_data::{Grid, TILE_AIRSHIP, TILE_BOWSER};

/// All path tiles eligible for lock/water-gap placement.
/// Locks (0x54, 0x56, 0xE4) are visual variants — all block the path equally.
/// Water gaps (0x9D) replace bridge tiles (0xB3) specifically.
///
/// The membership rule is **"the engine walks over it"** — presence in
/// [`rom_data::VALID_HORZ`] / [`rom_data::VALID_VERT`], the engine's own
/// `Map_Object_Valid_*` tables — not "it looks like a path". Two entries are
/// therefore not path tiles at all:
///
/// * **`0xE6` is `TILE_HANDTRAP`.** W8 row 5 draws its hand corridor as one
///   continuous run of `0xE6`: nodes on the even columns (22/24/26) and
///   *painted path* on the odd ones (23/25). Pickup blanks the three nodes;
///   the two odd cells have no pointer-table entry, so nothing picks them up
///   and they survive as `0xE6` — those two are the only cells here a lock
///   ever lands on. Harmless, because `Map_CheckDoMove` validates only the
///   cell moved *over* and then moves a hardcoded two tiles, so nothing ever
///   comes to rest on an odd column and those painted hands never grab.
/// * **`0xDB` is `TILE_VERTPATHSKY`** — the one tile here with no matching
///   lock. `0xE4` is the only sky lock and `Map_RemoveTo_Tiles` pairs it to
///   the *horizontal* sky path, so locking one of these writes a ground lock
///   into the clouds and the reload reveals a ground path. Known, measured at
///   3.7% of slots, and deliberately left — see `docs/fx_table_redesign.md`.
pub(super) const LOCKABLE_TILES: &[u8] = &[
    0x45, // horizontal path
    0x46, // vertical path
    0xB3, // bridge (water) — becomes water gap 0x9D
    0xDA, // sky path — becomes sky lock 0xE4
    0xAC, // horizontal path variant
    0xB7, // horizontal path variant
    0xB8, // horizontal path variant
    0xB9, // horizontal path variant
    0xE6, // HANDTRAP paint, walkable per VALID_HORZ — see above
    0xAA, // vertical path variant
    0xAB, // vertical path variant
    0xB0, // vertical path variant
    0xB1, // vertical drawbridge
    0xB2, // horizontal drawbridge
    0xDB, // sky vertical path — no sky vertical lock exists, see above
    0xBA, // vertical path variant
];

/// Find the airship or Bowser's castle position on the grid.
pub(super) fn find_target(grid: &Grid, world_idx: usize) -> Option<(usize, usize)> {
    let target_tile = if world_idx == 7 { TILE_BOWSER } else { TILE_AIRSHIP };
    for r in 0..grid.rows() {
        for c in 0..grid.cols {
            if grid.get(r, c) == target_tile {
                return Some((r, c));
            }
        }
    }
    None
}
