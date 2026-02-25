/// Pipe movement helpers: low-level ROM patching for overworld pipe operations.
///
/// These helpers handle the mechanical ROM writes for moving pipe endpoints
/// on the overworld map. The randomization logic (choosing WHERE pipes go)
/// lives in `pipes.rs`; this module handles HOW to write those decisions.

use crate::rom::Rom;

use super::rom_data::{
    self, MAP_TILE_GRIDS, PIPE_MAP_SCRL_XHI, PIPE_MAP_X, PIPE_MAP_XHI, PIPE_MAP_Y, WORLDS,
};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

pub(super) type Pos = (usize, usize);

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// InitIndex master table offset (9 word pointers, one per world + warp zone).
const INIT_INDEX_MASTER: usize = 0x193DA;

// ---------------------------------------------------------------------------
// Position encoding
// ---------------------------------------------------------------------------

/// Convert grid position to pipe destination table nibble values.
/// Returns (screen_nib, col_nib, row_nib).
pub(super) fn grid_pos_to_dest_nibbles(grid_row: usize, grid_col: usize) -> (u8, u8, u8) {
    let row_nib = (grid_row + 2) as u8;
    let screen = (grid_col / 16) as u8;
    let col = (grid_col % 16) as u8;
    (screen, col, row_nib)
}

// ---------------------------------------------------------------------------
// Entry position swapping
// ---------------------------------------------------------------------------

/// Swap the map positions of two pointer table entries.
///
/// Swaps ByRowType (preserving each entry's tileset in the lower nibble)
/// and ByScrCol, plus the tile grid tiles at their positions.
pub(super) fn swap_entry_positions(rom: &mut Rom, world_idx: usize, idx_a: usize, idx_b: usize) {
    let world = &WORLDS[world_idx];
    let n = world.entry_count;
    let rt = world.rowtype_offset;
    let sc = rt + n;
    let grid_offset = MAP_TILE_GRIDS[world_idx].file_offset;

    // Read current values
    let a_rowtype = rom.read_byte(rt + idx_a);
    let a_scrcol = rom.read_byte(sc + idx_a);
    let b_rowtype = rom.read_byte(rt + idx_b);
    let b_scrcol = rom.read_byte(sc + idx_b);

    // Extract row and tileset separately
    let a_row_nib = (a_rowtype >> 4) & 0x0F;
    let a_tileset = a_rowtype & 0x0F;
    let b_row_nib = (b_rowtype >> 4) & 0x0F;
    let b_tileset = b_rowtype & 0x0F;

    // Swap: A gets B's position (keeps A's tileset), B gets A's position
    rom.write_byte(rt + idx_a, (b_row_nib << 4) | a_tileset);
    rom.write_byte(sc + idx_a, b_scrcol);
    rom.write_byte(rt + idx_b, (a_row_nib << 4) | b_tileset);
    rom.write_byte(sc + idx_b, a_scrcol);

    // Swap tiles in the grid (per-screen addressing)
    let a_screen = ((a_scrcol >> 4) & 0x0F) as usize;
    let a_col = (a_scrcol & 0x0F) as usize;
    let a_grid_row = (a_row_nib as usize).wrapping_sub(2);

    let b_screen = ((b_scrcol >> 4) & 0x0F) as usize;
    let b_col = (b_scrcol & 0x0F) as usize;
    let b_grid_row = (b_row_nib as usize).wrapping_sub(2);

    let a_rom_off = grid_offset + a_screen * 144 + a_grid_row * 16 + a_col;
    let b_rom_off = grid_offset + b_screen * 144 + b_grid_row * 16 + b_col;

    let a_tile = rom.read_byte(a_rom_off);
    let b_tile = rom.read_byte(b_rom_off);
    rom.write_byte(a_rom_off, b_tile);
    rom.write_byte(b_rom_off, a_tile);
}

// ---------------------------------------------------------------------------
// Pipe destination table writes
// ---------------------------------------------------------------------------

/// Write all 4 pipe destination tables for one dest index.
///
/// Each table byte packs two nibble values: upper = endpoint A, lower = endpoint B.
/// Tables: MapXHi (screen), MapX (column), MapY (row_nib), MapScrlXHi (= MapXHi).
pub(super) fn write_pipe_dest(rom: &mut Rom, dest_idx: usize, a_pos: Pos, b_pos: Pos) {
    let (a_xhi, a_x, a_y) = grid_pos_to_dest_nibbles(a_pos.0, a_pos.1);
    let (b_xhi, b_x, b_y) = grid_pos_to_dest_nibbles(b_pos.0, b_pos.1);

    rom.write_byte(PIPE_MAP_XHI + dest_idx, (a_xhi << 4) | b_xhi);
    rom.write_byte(PIPE_MAP_X + dest_idx, (a_x << 4) | b_x);
    rom.write_byte(PIPE_MAP_Y + dest_idx, (a_y << 4) | b_y);
    rom.write_byte(PIPE_MAP_SCRL_XHI + dest_idx, (a_xhi << 4) | b_xhi);
}

// ---------------------------------------------------------------------------
// Pointer table re-sorting
// ---------------------------------------------------------------------------

/// Re-sort all pointer table entries by (screen, row_nib, col) and rebuild InitIndex.
///
/// The game scans entries per-screen from InitIndex[screen], matching row first
/// then column. Entries must be sorted for the lookup to work correctly.
pub(super) fn resort_pointer_table(rom: &mut Rom, world_idx: usize) {
    let world = &WORLDS[world_idx];
    let n = world.entry_count;
    let rt = world.rowtype_offset;
    let sc = rt + n;
    let obj = sc + n;
    let lay = obj + n * 2;

    // InitIndex file offset for this world
    let init_ptr = rom_data::read_word(rom, INIT_INDEX_MASTER + world_idx * 2);
    let init_file = 0x18010 + (init_ptr as usize - 0x8000);

    let num_screens = MAP_TILE_GRIDS[world_idx].screens;

    // Read all entries
    struct SortEntry {
        rowtype: u8,
        scrcol: u8,
        obj_lo: u8,
        obj_hi: u8,
        lay_lo: u8,
        lay_hi: u8,
        screen: u8,
        row_nib: u8,
        col: u8,
    }

    let mut entries: Vec<SortEntry> = (0..n)
        .map(|i| {
            let rowtype = rom.read_byte(rt + i);
            let scrcol = rom.read_byte(sc + i);
            SortEntry {
                rowtype,
                scrcol,
                obj_lo: rom.read_byte(obj + i * 2),
                obj_hi: rom.read_byte(obj + i * 2 + 1),
                lay_lo: rom.read_byte(lay + i * 2),
                lay_hi: rom.read_byte(lay + i * 2 + 1),
                screen: (scrcol >> 4) & 0x0F,
                row_nib: (rowtype >> 4) & 0x0F,
                col: scrcol & 0x0F,
            }
        })
        .collect();

    // Sort by (screen, row_nib, col)
    entries.sort_by_key(|e| (e.screen, e.row_nib, e.col));

    // Write back sorted entries
    for (i, e) in entries.iter().enumerate() {
        rom.write_byte(rt + i, e.rowtype);
        rom.write_byte(sc + i, e.scrcol);
        rom.write_byte(obj + i * 2, e.obj_lo);
        rom.write_byte(obj + i * 2 + 1, e.obj_hi);
        rom.write_byte(lay + i * 2, e.lay_lo);
        rom.write_byte(lay + i * 2 + 1, e.lay_hi);
    }

    // Rebuild InitIndex: one byte per screen = offset of first entry on that screen
    for s in 0..num_screens {
        let offset = entries
            .iter()
            .position(|e| e.screen == s as u8)
            .unwrap_or(0);
        rom.write_byte(init_file + s, offset as u8);
    }
}
