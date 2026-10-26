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

/// Lay the W1-W8 pointer tables out again with `counts[wi]` rows each.
///
/// Vanilla packs every world's tables back to back (InitIndex, then the
/// row-type, screen-column, object and layout tables), W1 to W8, with the
/// Warp Zone's after them. This rewrites that run in the same bytes and
/// repoints the five master tables. World `wi` keeps the old rows `keep[wi]`
/// as its first rows, in that order; the rows after them are zeroed for the
/// caller to fill. InitIndex is left for [`resort_pointer_table`] to rebuild.
///
/// Panics if the rows do not fit before the Warp Zone's tables.
pub(crate) fn relayout_pointer_tables(rom: &mut Rom, keep: &[Vec<usize>], counts: &[usize; 8]) {
    let start = rom_data::world_tables(rom, 0).rowtype_offset - 4;
    let end = rom_data::prg_bank_cpu_to_file(12, rom_data::read_word(rom, INIT_INDEX_MASTER + 16));
    let total: usize = counts.iter().map(|n| 4 + n * 6).sum();
    assert!(start + total <= end, "{total} bytes of pointer tables overrun the Warp Zone's");

    // Every kept row's six bytes, read before any are overwritten.
    let rows: Vec<Vec<[u8; 6]>> = (0..8)
        .map(|wi| {
            let world = rom_data::world_tables(rom, wi);
            let (sc, obj, lay) = rom_data::table_offsets(&world);
            let at = |i: usize| {
                [
                    rom.read_byte(world.rowtype_offset + i),
                    rom.read_byte(sc + i),
                    rom.read_byte(obj + i * 2),
                    rom.read_byte(obj + i * 2 + 1),
                    rom.read_byte(lay + i * 2),
                    rom.read_byte(lay + i * 2 + 1),
                ]
            };
            assert!(keep[wi].len() <= counts[wi], "W{} keeps more rows than it has", wi + 1);
            keep[wi].iter().map(|&i| at(i)).collect()
        })
        .collect();

    rom.write_range(start, &vec![0; total]);
    let mut at = start;
    for wi in 0..8 {
        let n = counts[wi];
        let world = rom_data::WorldTables { rowtype_offset: at + 4, entry_count: n };
        let (sc, obj, lay) = rom_data::table_offsets(&world);
        for (master, file) in [
            (INIT_INDEX_MASTER, at),
            (rom_data::ROWTYPE_MASTER, world.rowtype_offset),
            (rom_data::SCRCOL_MASTER, sc),
            (rom_data::OBJSETS_MASTER, obj),
            (rom_data::LAYOUTS_MASTER, lay),
        ] {
            let cpu = rom_data::prg_bank_file_to_cpu(12, file);
            rom.write_range(master + wi * 2, &cpu.to_le_bytes());
        }
        for (i, r) in rows[wi].iter().enumerate() {
            rom.write_byte(world.rowtype_offset + i, r[0]);
            rom.write_byte(sc + i, r[1]);
            rom.write_range(obj + i * 2, &r[2..4]);
            rom.write_range(lay + i * 2, &r[4..6]);
        }
        at += 4 + n * 6;
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

    /// The row `Map_PrepareLevel` loads at `pos`: from the screen's InitIndex,
    /// on to the first row on the player's map row, then on to the first with
    /// the player's screen-column. `None` if the scan leaves the world's rows.
    fn engine_lookup(rom: &Rom, wi: usize, pos: Pos) -> Option<usize> {
        let world = rom_data::world_tables(rom, wi);
        let (sc, _, _) = rom_data::table_offsets(&world);
        let init_cpu = rom_data::read_word(rom, INIT_INDEX_MASTER + wi * 2);
        let init = rom_data::prg_bank_cpu_to_file(12, init_cpu);
        let (screen, col, row_nib) = grid_pos_to_dest_nibbles(pos.0, pos.1);
        let n = world.entry_count;
        let mut y = rom.read_byte(init + screen as usize) as usize;
        while y < n && rom.read_byte(world.rowtype_offset + y) >> 4 != row_nib {
            y += 1;
        }
        while y < n && rom.read_byte(sc + y) != (screen << 4) | col {
            y += 1;
        }
        (y < n).then_some(y)
    }

    /// Worlds grown and shrunk by a relayout still load the same level on
    /// every kept tile, the free rows come out empty, and the Warp Zone's
    /// tables after the run are not touched.
    #[test]
    fn relaid_tables_load_the_same_levels() {
        let Ok(bytes) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") else {
            eprintln!("SKIP: requires the ROM, which is not included in the repo");
            return;
        };
        let vanilla = Rom::from_bytes(&bytes).unwrap();
        let old: Vec<usize> =
            (0..8).map(|wi| rom_data::world_tables(&vanilla, wi).entry_count).collect();
        let counts =
            [old[0] + 3, old[1] - 3, old[2], old[3], old[4], old[5] - 2, old[6], old[7] + 2];
        let keep: Vec<Vec<usize>> =
            (0..8).map(|wi| (0..old[wi].min(counts[wi])).collect()).collect();

        // The Warp Zone (master slot 8): its object and layout masters share
        // one table, so its tables end at its layouts.
        let w9 = |table: usize| {
            rom_data::prg_bank_cpu_to_file(12, rom_data::read_word(&vanilla, table + 16))
        };
        let w9_init = w9(INIT_INDEX_MASTER);
        let w9_end = w9(rom_data::LAYOUTS_MASTER)
            + (w9(rom_data::SCRCOL_MASTER) - w9(rom_data::ROWTYPE_MASTER)) * 2;
        let w9_masters: Vec<u16> = (0..5)
            .map(|m| rom_data::read_word(&vanilla, INIT_INDEX_MASTER + m * 18 + 16))
            .collect();

        let mut rom = vanilla.clone();
        relayout_pointer_tables(&mut rom, &keep, &counts);
        for wi in 0..8 {
            resort_pointer_table(&mut rom, wi);
        }

        assert_eq!(
            rom.read_range(w9_init, w9_end - w9_init),
            vanilla.read_range(w9_init, w9_end - w9_init)
        );
        let masters: Vec<u16> =
            (0..5).map(|m| rom_data::read_word(&rom, INIT_INDEX_MASTER + m * 18 + 16)).collect();
        assert_eq!(masters, w9_masters, "the Warp Zone was repointed");
        for wi in 0..8 {
            let world = rom_data::world_tables(&rom, wi);
            assert_eq!(world.entry_count, counts[wi], "W{}", wi + 1);
            let van = rom_data::world_tables(&vanilla, wi);
            for &i in &keep[wi] {
                let pos = rom_data::entry_grid_position(&vanilla, &van, i);
                if pos.0 >= rom_data::ROWS {
                    continue; // parked off the map: never loaded
                }
                let want = engine_lookup(&vanilla, wi, pos)
                    .map(|r| rom_data::read_entry(&vanilla, &van, r));
                let got =
                    engine_lookup(&rom, wi, pos).map(|r| rom_data::read_entry(&rom, &world, r));
                assert_eq!(got, want, "W{} {pos:?}", wi + 1);
            }
            let empty =
                (0..counts[wi]).filter(|&r| rom.read_byte(world.rowtype_offset + r) == 0).count();
            assert!(empty >= counts[wi] - keep[wi].len(), "W{}: free rows not empty", wi + 1);
        }
    }
}
