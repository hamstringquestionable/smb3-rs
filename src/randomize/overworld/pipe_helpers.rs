//! Pipe movement helpers: low-level ROM patching for overworld pipe operations.
//!
//! These helpers handle the mechanical ROM writes for moving pipe endpoints
//! on the overworld map. The randomization logic (choosing WHERE pipes go)
//! lives in `overworld::build`; this module handles HOW to write those decisions.

use crate::rom::Rom;

use crate::randomize::rom_data::{
    self, INIT_INDEX_MASTER, MAP_TILE_GRIDS, PIPE_MAP_SCRL_XHI, PIPE_MAP_X, PIPE_MAP_XHI,
    PIPE_MAP_Y, W5_IDX, W8_IDX,
};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type Pos = (usize, usize);

// ---------------------------------------------------------------------------
// Position encoding
// ---------------------------------------------------------------------------

/// Convert grid position to pipe destination table nibble values.
/// Returns (screen_nib, col_nib, row_nib).
pub(crate) fn grid_pos_to_dest_nibbles(grid_row: usize, grid_col: usize) -> (u8, u8, u8) {
    let row_nib = (grid_row + 2) as u8;
    let screen = (grid_col / 16) as u8;
    let col = (grid_col % 16) as u8;
    (screen, col, row_nib)
}

// ---------------------------------------------------------------------------
// Pipe destination table writes
// ---------------------------------------------------------------------------

/// Compute the MapScrlXHi nibble for a pipe endpoint so the camera snaps
/// without triggering a pan.
///
/// The nibble encodes: bits 2-0 = scroll screen, bit 3 = center flag (+128 px).
/// On-screen player position = screen*256 + col*16 - scroll_px.
/// Pan triggers at <33 (left) or >208 (right).  This picks the scroll value
/// that keeps the player in [33, 208].
///
/// W5 and W8 have discrete single-screen map sections that don't smoothly
/// scroll — the camera snaps to each screen boundary. The center flag must
/// never be set for these worlds or it shifts the camera 128px right, cutting
/// off half the visible screen. Vanilla confirms: no W5/W8 pipe scroll
/// nibbles ever use the center flag.
///
/// Nor on a world's last screen, where there is no map to its right: the
/// flag would show half a screen past the edge. W1 is one screen, so it never
/// scrolls; an arrival on the right of it used to (user, 2026-10-16). The
/// engine stops panning at the world's `World_Map_Max_PanR`, which is the
/// last screen everywhere but vanilla W4 (`$30`, a screen past its two). That
/// one cannot bite: vanilla W4's nodes stop at column 28, left of the x > 208
/// that would pan, and a generated W4 writes its own limit.
pub(crate) fn scroll_nibble(world_idx: usize, screen: u8, col_in_screen: u8) -> u8 {
    if world_idx == W5_IDX || world_idx == W8_IDX {
        return screen;
    }
    let last = (MAP_TILE_GRIDS[world_idx].screens - 1) as u8;
    let px = col_in_screen as u16 * 16;
    if px < 33 && screen > 0 {
        (screen - 1) | 0x8
    } else if px > 208 && screen < last {
        screen | 0x8
    } else {
        screen
    }
}

/// Write all 4 pipe destination tables for one dest index.
///
/// Each table byte packs two nibble values: upper = endpoint A, lower = endpoint B.
/// Tables: MapXHi (screen), MapX (column), MapY (row_nib), MapScrlXHi.
///
/// MapScrlXHi controls the camera scroll position after exiting a pipe.
/// Each nibble is computed by `scroll_nibble()` to keep the player's on-screen
/// position in [33, 208], avoiding pan-left (<33) and pan-right (>208) triggers.
/// For worlds with discrete screens (W5, W8), the center flag is never set.
pub(super) fn write_pipe_dest(
    rom: &mut Rom,
    dest_idx: usize,
    a_pos: Pos,
    b_pos: Pos,
    world_idx: usize,
) {
    let (a_xhi, a_x, a_y) = grid_pos_to_dest_nibbles(a_pos.0, a_pos.1);
    let (b_xhi, b_x, b_y) = grid_pos_to_dest_nibbles(b_pos.0, b_pos.1);

    rom.write_byte(PIPE_MAP_XHI + dest_idx, (a_xhi << 4) | b_xhi);
    rom.write_byte(PIPE_MAP_X + dest_idx, (a_x << 4) | b_x);
    rom.write_byte(PIPE_MAP_Y + dest_idx, (a_y << 4) | b_y);

    let a_scrl = scroll_nibble(world_idx, a_xhi, a_x);
    let b_scrl = scroll_nibble(world_idx, b_xhi, b_x);
    rom.write_byte(PIPE_MAP_SCRL_XHI + dest_idx, (a_scrl << 4) | b_scrl);
}

// ---------------------------------------------------------------------------
// Pointer table re-sorting
// ---------------------------------------------------------------------------

/// Re-sort all pointer table entries by (screen, row_nib, col) and rebuild InitIndex.
///
/// The game scans entries per-screen from `InitIndex[screen]`, matching row first
/// then column. Entries must be sorted for the lookup to work correctly.
pub(super) fn resort_pointer_table(rom: &mut Rom, world_idx: usize) {
    let world = rom_data::world_tables(rom, world_idx);
    let n = world.entry_count;
    let rt = world.rowtype_offset;
    let sc = rt + n;
    let obj = sc + n;
    let lay = obj + n * 2;

    // InitIndex file offset for this world.
    // PRG012 is loaded at CPU $A000-$BFFF during the map screen, so the
    // CPU addresses in the master table are in the $A000+ range.
    let init_ptr = rom_data::read_word(rom, INIT_INDEX_MASTER + world_idx * 2);
    let init_file = rom_data::PRG012_FILE_BASE + (init_ptr as usize - 0xA000);

    // All worlds allocate 4 InitIndex bytes (gap between InitIndex and
    // ByRowType pointers is always 4), regardless of actual screen count.
    let num_screens = 4;

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

    // Rebuild InitIndex: one byte per screen = offset of first entry on that screen.
    // Screens with no entries point past the end (= n) matching vanilla convention.
    for s in 0..num_screens {
        let offset = entries.iter().position(|e| e.screen == s as u8).unwrap_or(n);
        rom.write_byte(init_file + s, offset as u8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The centre flag scrolls half a screen right, so it only goes where
    /// there is map to show: never on a world's last screen, so never in W1,
    /// whose one screen is its last; never in W5 or W8, which snap per screen.
    #[test]
    fn the_camera_never_scrolls_past_the_last_screen() {
        // A right-edge arrival (column 14, x = 224 > 208).
        assert_eq!(scroll_nibble(0, 0, 14), 0, "W1 has one screen");
        assert_eq!(scroll_nibble(1, 0, 14), 0x8, "W2 screen 0 has screen 1 to its right");
        assert_eq!(scroll_nibble(1, 1, 14), 1, "W2 screen 1 is its last");
        assert_eq!(scroll_nibble(2, 1, 14), 0x9, "W3 screen 1 has screen 2 to its right");
        assert_eq!(scroll_nibble(2, 2, 14), 2, "W3 screen 2 is its last");
        assert_eq!(scroll_nibble(4, 0, 14), 0, "W5 snaps");
        assert_eq!(scroll_nibble(7, 2, 14), 2, "W8 snaps");
        // A left-edge arrival still centres back onto the screen before.
        assert_eq!(scroll_nibble(1, 1, 0), 0x8);
        assert_eq!(scroll_nibble(0, 0, 0), 0, "nothing left of W1's screen");
        // Mid-screen never moves.
        for wi in [0, 1, 2, 3, 5, 6] {
            assert_eq!(scroll_nibble(wi, 0, 6), 0);
        }
    }
}
