//! The overworld tile `Grid` and its readers.

use super::*;

/// Mutable overworld tile grid.
#[derive(Clone, Debug)]
pub(crate) struct Grid {
    pub(crate) tiles: Vec<Vec<u8>>,
    pub(crate) cols: usize,
    /// Whether `8s are Wild` is active for this run. Rides on the grid so the
    /// map walker and builder can resolve the active canoe edges (via
    /// [`active_canoe_edges`]) without threading the flag through every call
    /// site. Defaults to `false` (the safe default — no phantom W8 canoe); the
    /// overworld builder stamps the real value onto the grids it walks, and it
    /// is preserved through clones.
    pub(crate) eights_are_wild: bool,
    /// Whether this is one of vanilla's hand-drawn layouts, so the tile
    /// fixes keyed to vanilla cells (`map_source::vanilla::BLANK_TILE_OVERRIDES`
    /// and `ISLAND_POSITIONS`) apply. True for any grid read from the ROM; a
    /// generated layout sets it false, since those cells mean nothing there.
    /// Rides on the grid for the same reason as `eights_are_wild`.
    pub(crate) vanilla_layout: bool,
    /// This world's canoe edges when its map source drew its own docks (a
    /// generated world); `None` reads vanilla's table. Read through
    /// [`canoe_edges`]. Rides on the grid for the same reason as
    /// `eights_are_wild`.
    pub(crate) canoe_edges: Option<Vec<TeleportEdge>>,
}

impl Grid {
    pub(crate) fn get(&self, row: usize, col: usize) -> u8 {
        self.tiles[row][col]
    }

    pub(crate) fn set(&mut self, row: usize, col: usize, tile: u8) {
        self.tiles[row][col] = tile;
    }

    /// Row count — every overworld grid has exactly [`ROWS`] rows; only the
    /// column count varies per world.
    pub(crate) fn rows(&self) -> usize {
        ROWS
    }
}

/// The canoe edges for a world as its grid has them: the grid's own when its
/// map source drew docks (a generated world), else vanilla's table through
/// [`active_canoe_edges`]. Every consumer holding a grid reads canoe edges
/// here.
pub(crate) fn canoe_edges(world_idx: usize, grid: &Grid) -> Vec<TeleportEdge> {
    match &grid.canoe_edges {
        Some(edges) => edges.clone(),
        None => active_canoe_edges(world_idx, grid.eights_are_wild),
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

    Grid { tiles, cols, eights_are_wild: false, vanilla_layout: true, canoe_edges: None }
}

/// All eight worlds' tile grids, read off a finished ROM.
///
/// **For callers that only have a ROM.** The randomizer pipeline does not use
/// this: `crate::randomize::overworld::writer::WrittenOverworld::grids` hands over the map the
/// writer just committed, which is the same bytes without the round trip, and
/// without the unwritten "run after every grid write" rule that reading back
/// implies. Only tests read a finished ROM's map now.
// Test-only, and that is the point: nothing in a shipped run reads the map back
// any more. `testrom` was the last non-test caller, and its hammer path now
// reads the allocated lock tiles back instead (`lock_keys::allocated_pairs_on_rom`).
#[cfg(test)]
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
