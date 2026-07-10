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
