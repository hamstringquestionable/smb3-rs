//! The overworld tile `Grid` and its readers.

use super::*;

/// Mutable overworld tile grid.
#[derive(Clone, Debug)]
pub(crate) struct Grid {
    pub tiles: Vec<Vec<u8>>,
    pub cols: usize,
    /// Whether `8s are Wild` is active for this run. Rides on the grid so the
    /// map walker and builder can resolve the active canoe edges (via
    /// [`active_canoe_edges`]) without threading the flag through every call
    /// site. Defaults to `false` (the safe default — no phantom W8 canoe); the
    /// overworld builder stamps the real value onto the grids it walks, and it
    /// is preserved through clones.
    pub eights_are_wild: bool,
}

impl Grid {
    pub fn get(&self, row: usize, col: usize) -> u8 {
        self.tiles[row][col]
    }

    pub fn set(&mut self, row: usize, col: usize, tile: u8) {
        self.tiles[row][col] = tile;
    }

    /// Row count — every overworld grid has exactly [`ROWS`] rows; only the
    /// column count varies per world.
    pub fn rows(&self) -> usize {
        ROWS
    }
}

/// Read a world's tile grid from ROM as a mutable Grid. The grid is born with
/// `eights_are_wild = false` (the safe default — no W8 canoe); the overworld
/// builder stamps the real flag onto the grids it walks (see
/// [`Grid::eights_are_wild`] and [`active_canoe_edges`]).
pub(crate) fn read_tile_grid(rom: &Rom, world_idx: usize) -> Grid {
    let info = &MAP_TILE_GRIDS[world_idx];
    let cols = info.columns;

    let mut tiles = Vec::with_capacity(ROWS);
    for r in 0..ROWS {
        let mut row = Vec::with_capacity(cols);
        for c in 0..cols {
            let screen = c / 16;
            let col_in_screen = c % 16;
            let offset = info.file_offset + screen * 144 + r * 16 + col_in_screen;
            row.push(rom.read_byte(offset));
        }
        tiles.push(row);
    }

    Grid { tiles, cols, eights_are_wild: false }
}

/// All eight worlds' tile grids, read off a finished ROM.
///
/// **For callers that only have a ROM.** The randomizer pipeline does not use
/// this: `overworld_writer::WrittenOverworld::grids` hands over the map the
/// writer just committed, which is the same bytes without the round trip, and
/// without the unwritten "run after every grid write" rule that reading back
/// implies. This is for `testrom` (which patches a finished ROM and has no
/// writer), for `lock_keys` (which runs after the packed store is emitted and
/// cross-checks its own reading against it), and for tests.
// Native-only, and that is the point: nothing in a shipped run reads the map
// back any more. `testrom` patches a finished ROM with no writer in the path,
// and the tests build their own ROMs — neither exists on wasm32, where the only
// caller would be a pipeline that no longer needs one.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn read_all_tile_grids(rom: &Rom) -> Vec<Grid> {
    (0..MAP_TILE_GRIDS.len()).map(|w| read_tile_grid(rom, w)).collect()
}

/// Find the START tile position in a grid.
pub(crate) fn find_start(grid: &Grid) -> Option<(usize, usize)> {
    for r in 0..grid.rows() {
        for c in 0..grid.cols {
            if grid.get(r, c) == TILE_START {
                return Some((r, c));
            }
        }
    }
    None
}
