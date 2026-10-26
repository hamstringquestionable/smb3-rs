//! The vanilla map source: each world as the ROM has it, plus the edits the
//! randomizer always makes to it ([`fixes`]).
//!
//! This is the one home for knowledge that is only true of vanilla's
//! hand-drawn maps: cells keyed by position, and tile quirks the neighbour rule
//! gets wrong. A generated map brings its own equivalents instead.

mod fixes;

#[cfg(test)]
pub(crate) use fixes::HAMMER_ROCKS;
pub(crate) use fixes::{W8_BRIDGE_COLS, W8_BRIDGE_ROW};

use crate::randomize::rom_data::{self, FxSlot, Grid, Pos};
use crate::rom::Rom;

use super::WorldMap;

/// Lone blanks sealed off by breakable hammer rocks: `(world_idx, position)`.
/// W3 (8,6) is the vanilla spade between two rocks — once the spade is picked
/// up, the cell is a one-blank island that connectivity would otherwise
/// bridge with a pipe every seed. (W6's rock at (2,13) guards a vanilla PIPE
/// at (2,14) — deliberately NOT listed, bridging it is vanilla-faithful.)
const HAMMER_GATED_POCKETS: &[(usize, Pos)] = &[
    (2, (8, 6)), // W3 spade pocket between two rocks near start
];

/// Position-specific overrides where the neighbor-based heuristic picks the
/// wrong tile. `(world_idx, row, col, tile)`
pub(crate) const BLANK_TILE_OVERRIDES: &[(usize, usize, usize, u8)] = &[
    (2, 8, 6, 0x47), // W3 spade near start — heuristic picks 0x44 (no neighbors), needs 0x47 for BFS
    (7, 5, 8, 0x8D), // W8 battleship entry — its cell is navy water (canoe area), not a land blank.
                     // The entry is still picked up into the pool (placed elsewhere); only this
                     // leftover tile changes, so the heuristic's 0x44 land tile is overridden to water.
];

/// Positions that should use island-themed blank tiles (0xAE/0xAF/0xB5/0xB6)
/// even though the theme sniff would read them as standard land. Only W3
/// (2,4) genuinely needs the override — its connecting path is the standard
/// 0x46 above it while the strip it sits on runs south — but the rest are kept
/// as a safety net, since they stay right even when a lock is stamped over the
/// island path the sniff would otherwise key on.
pub(crate) const ISLAND_POSITIONS: &[(usize, usize, usize)] = &[
    // W3 — narrow island strips
    (2, 2, 4),  // 3-2
    (2, 4, 4),  // hammer
    (2, 6, 4),  // 3-1
    (2, 4, 12), // hammer
    (2, 6, 12), // 3-5
    (3, 6, 20), // W4 4-4
    (6, 5, 10), // W7 7-4
];

/// The vanilla maps with the randomizer's edits applied: what the pipeline
/// builds on.
pub(crate) fn load(rom: &Rom, more_hammer_rocks: bool, eights_are_wild: bool) -> Vec<WorldMap> {
    let mut maps = read_tiles(rom);
    fixes::apply(&mut maps, more_hammer_rocks, eights_are_wild);
    find_gaps(rom, &mut maps);
    maps
}

/// All eight worlds exactly as the ROM has them, with no edits.
pub(crate) fn read(rom: &Rom) -> Vec<WorldMap> {
    let mut maps = read_tiles(rom);
    find_gaps(rom, &mut maps);
    maps
}

/// Write the vanilla map edits into the ROM itself.
///
/// Test harnesses only. The pipeline never does this: its maps carry the
/// edits and the writer puts them in place. A harness that builds from a ROM
/// uses this to put that ROM in the state the pipeline's maps are in.
#[cfg(test)]
pub(crate) fn stamp_fixes(rom: &mut Rom, more_hammer_rocks: bool, eights_are_wild: bool) {
    let mut maps = read_tiles(rom);
    fixes::apply(&mut maps, more_hammer_rocks, eights_are_wild);
    for (wi, map) in maps.iter().enumerate() {
        for row in 0..map.grid.rows() {
            for col in 0..map.grid.cols {
                rom.write_byte(rom_data::map_tile_offset(wi, row, col), map.grid.get(row, col));
            }
        }
    }
}

/// Everything but the gaps, which depend on the finished tiles and so are
/// found last ([`find_gaps`]).
fn read_tiles(rom: &Rom) -> Vec<WorldMap> {
    (0..8)
        .map(|wi| {
            let grid = rom_data::read_tile_grid(rom, wi);
            let world = &rom_data::world_tables(rom, wi);
            let entry_pos = (0..world.entry_count)
                .map(|i| rom_data::entry_grid_position(rom, world, i))
                .collect();
            let (_, objsets, _) = rom_data::table_offsets(world);
            let hammer_bros = (0..world.entry_count)
                .filter(|i| {
                    let obj = rom_data::read_word(rom, objsets + i * 2);
                    rom_data::HAMMER_BRO_OBJ_PTRS.contains(&obj)
                })
                .collect();
            WorldMap {
                grid,
                entry_pos,
                hammer_bros,
                sprites: rom_data::read_parked_sprite_positions(rom, wi),
                hb_sprites: rom_data::read_hb_sprite_positions(rom, wi),
                gaps: Vec::new(),
                hammer_gated: HAMMER_GATED_POCKETS
                    .iter()
                    .filter(|(w, _)| *w == wi)
                    .map(|(_, pos)| *pos)
                    .collect(),
                no_locks: Vec::new(),
                canoe: None,
                pan_limit: None,
                castle: None,
            }
        })
        .collect()
}

/// Fill in each map's gaps from the FX tables, against its finished grid.
fn find_gaps(rom: &Rom, maps: &mut [WorldMap]) {
    let fx_slots = rom_data::read_fx_slots(rom);
    let fx_assignments = rom_data::read_world_fx_assignments(rom);
    for (map, world_fx) in maps.iter_mut().zip(&fx_assignments) {
        map.gaps = fx_gaps(&map.grid, &fx_slots, world_fx);
    }
}

/// Vanilla's gaps: the cells its fortress-FX slots name, each with the path
/// tile it opens to.
///
/// **The path comes from the tile standing there, not from the FX slot.** The
/// slot's own `replace_tile` says the same thing for sixteen of vanilla's
/// seventeen. The odd one out is slot `$08`, W6 `(4, 13)`: it stores the sky
/// path `$DA` for a lock sitting on plain ground, which is what the *preceding*
/// slot legitimately stores for the game's one real sky lock — the value looks
/// carried down a row when the table was authored
/// (`fortress_fx::derivation` pins it).
///
/// It also settles a handshake that used to run the other way. `apply_w8_bridges`
/// stamps a bridge over W8 `(5, 53)`, which is where slot 16 sits, and had to
/// poke `BRIDGE_TILE` into `FortressFX_MapTileReplace` so this loop would not
/// paint a plain path over it. Asking the grid instead answers correctly with
/// no channel at all: a cell already showing an open tile is not a gap, so
/// nothing is written.
fn fx_gaps(grid: &Grid, fx_slots: &[FxSlot], world_fx: &[u8]) -> Vec<(Pos, u8)> {
    world_fx
        .iter()
        .filter_map(|&slot_idx| {
            let slot = &fx_slots[slot_idx as usize];
            let (row, col) = (slot.grid_row, slot.grid_col);
            if row >= grid.rows() || col >= grid.cols {
                return None;
            }
            rom_data::path_for_gap_tile(grid.get(row, col)).map(|path| ((row, col), path))
        })
        .collect()
}
