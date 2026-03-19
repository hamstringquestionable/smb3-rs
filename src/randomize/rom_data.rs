/// Shared ROM constants, data structures, and helpers for SMB3 randomization.
///
/// This module holds all the shared knowledge about the ROM layout — constants,
/// lookup tables, data structures, and low-level read/write helpers — used by
/// multiple randomization modules. The BFS map walker lives in `map_walker.rs`.


use crate::rom::Rom;

// ---------------------------------------------------------------------------
// Public types (re-exported by randomizer.rs)
// ---------------------------------------------------------------------------

/// Fortress redistribute mode.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FortressRedistribute {
    Off,
    IntraWorld,
    CrossWorld,
}

impl Default for FortressRedistribute {
    fn default() -> Self {
        FortressRedistribute::Off
    }
}

// ---------------------------------------------------------------------------
// Tile constants
// ---------------------------------------------------------------------------

/// Valid horizontal path tiles (Map_Object_Valid_Left/Right in PRG010).
pub(super) const VALID_HORZ: &[u8] = &[0x45, 0x49, 0xB2, 0xB3, 0xAC, 0xB7, 0xB8, 0xDA, 0xB9, 0xE6];

/// Valid vertical path tiles (Map_Object_Valid_Down/Up in PRG010).
pub(super) const VALID_VERT: &[u8] = &[0x46, 0xB1, 0xAA, 0xAB, 0xB0, 0xDB, 0xBA];

/// Background / non-walkable tiles.
pub(super) const BACKGROUND_TILES: &[u8] = &[0xB4, 0xFF, 0x02];

/// Valid blank node tiles — positions with these tiles are available for
/// level/fort/pipe/HB placement. Used by both pickup (Phase 2) and build
/// (Phase 3) to ensure consistent blank detection.
pub(super) const VALID_BLANK_TILES: &[u8] = &[
    0x44, 0x47, 0x48, 0x4A,        // standard
    0xAE, 0xAF, 0xB5, 0xB6,        // island
    0xD9, 0xDC, 0xDD, 0xDE,        // sky
];

/// Start tile ID.
pub(super) const TILE_START: u8 = 0xE5;

/// Pipe tile ID.
pub(super) const TILE_PIPE: u8 = 0xBC;

/// W5 Spiral Tower tile ID (functionally a pipe connecting screen 0 ↔ screen 1).
pub(super) const TILE_SPIRAL: u8 = 0x5F;

/// Fortress map tile ID (used in test code across multiple modules).
#[allow(dead_code)]
pub(super) const TILE_FORTRESS: u8 = 0x67;

/// Airship dock tile ID.
pub(super) const TILE_AIRSHIP: u8 = 0xC9;

/// Bowser's castle tile ID.
pub(super) const TILE_BOWSER: u8 = 0xCC;

/// Placeholder stamped on the BFS grid to mark a position as non-background.
/// The actual value is irrelevant — it just needs to be outside BACKGROUND_TILES
/// so walk_map treats the position as a reachable node.
pub(super) const TILE_NODE: u8 = 0x47;

/// Number of rows in every overworld map.
pub(super) const ROWS: usize = 9;

// ---------------------------------------------------------------------------
// ROM offset constants
// ---------------------------------------------------------------------------

// Pipe destination tables (PRG002)
pub(super) const PIPE_MAP_XHI: usize = 0x046AA;
pub(super) const PIPE_MAP_X: usize = 0x046C2;
pub(super) const PIPE_MAP_Y: usize = 0x046DA;
pub(super) const PIPE_MAP_SCRL_XHI: usize = 0x046F2;

// FX table offsets (17 slots)
pub(super) const FX_VADDR_H: usize = 0x147CD;
pub(super) const FX_VADDR_L: usize = 0x147DE;
pub(super) const FX_MAP_COMP_IDX: usize = 0x147EF; // 17 x 2 bytes
pub(super) const FX_PATTERNS: usize = 0x14811;     // 17 x 4 bytes
pub(super) const FX_MAP_LOC_ROW: usize = 0x14855;
pub(super) const FX_MAP_LOC: usize = 0x14866;
pub(super) const FX_MAP_TILE_REPLACE: usize = 0x14877;
pub(super) const FX_WORLD_TABLE: usize = 0x14888;

/// Map_Complete_Bits lookup table: maps grid row to completion bit.
/// Row 0 = $80, row 1 = $40, ..., row 7 = $01.
pub(super) const MAP_COMPLETE_BITS: [u8; 8] = [0x80, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x01];

// ---------------------------------------------------------------------------
// Entry lookup tables
// ---------------------------------------------------------------------------

/// Destination byte → world index (0-based). Only paired pipe destinations.
pub(super) const DEST_TO_WORLD: &[(u8, usize)] = &[
    (0x00, 4),  // W5 (spiral tower)
    (0x01, 1),  // W2
    (0x02, 5), (0x03, 5),  // W6
    (0x04, 6), (0x05, 6), (0x06, 6), (0x07, 6),  // W7
    (0x08, 6), (0x09, 6), (0x0A, 6), (0x0B, 6),  // W7
    (0x0C, 7), (0x0D, 7), (0x0E, 7), (0x0F, 7), (0x10, 7), (0x11, 7),  // W8
    (0x12, 2), (0x13, 2), (0x14, 2),  // W3
    (0x15, 3), (0x16, 3),  // W4
    (0x17, 4),  // W5
];

/// Per-world map tile grid info.
pub(super) struct MapGridInfo {
    pub file_offset: usize,
    pub columns: usize,
    #[allow(dead_code)]
    pub screens: usize,
}

pub(super) const MAP_TILE_GRIDS: [MapGridInfo; 8] = [
    MapGridInfo { file_offset: 0x185BA, columns: 16, screens: 1 },  // W1
    MapGridInfo { file_offset: 0x1864B, columns: 32, screens: 2 },  // W2
    MapGridInfo { file_offset: 0x1876C, columns: 48, screens: 3 },  // W3
    MapGridInfo { file_offset: 0x1891D, columns: 32, screens: 2 },  // W4
    MapGridInfo { file_offset: 0x18A3E, columns: 32, screens: 2 },  // W5
    MapGridInfo { file_offset: 0x18B5F, columns: 48, screens: 3 },  // W6
    MapGridInfo { file_offset: 0x18D10, columns: 32, screens: 2 },  // W7
    MapGridInfo { file_offset: 0x18E31, columns: 64, screens: 4 },  // W8
];

/// Pointer table locations per world.
pub(super) struct WorldTables {
    pub rowtype_offset: usize,
    pub entry_count: usize,
}

pub(super) const WORLDS: [WorldTables; 8] = [
    WorldTables { rowtype_offset: 0x19438, entry_count: 21 },
    WorldTables { rowtype_offset: 0x194BA, entry_count: 47 },
    WorldTables { rowtype_offset: 0x195D8, entry_count: 52 },
    WorldTables { rowtype_offset: 0x19714, entry_count: 34 },
    WorldTables { rowtype_offset: 0x197E4, entry_count: 42 },
    WorldTables { rowtype_offset: 0x198E4, entry_count: 57 },
    WorldTables { rowtype_offset: 0x19A3E, entry_count: 46 },
    WorldTables { rowtype_offset: 0x19B56, entry_count: 41 },
];

/// Known fortress entries (world_idx, entry_idx).
pub(super) const FORTRESS_ENTRIES: &[(usize, usize)] = &[
    (0, 11),
    (1, 13),
    (2, 13), (2, 34),
    (3, 9), (3, 16),
    (4, 12), (4, 31),
    (5, 9), (5, 27), (5, 48),
    (6, 5), (6, 40),
    (7, 7), (7, 10), (7, 26), (7, 36),
];

/// ROM file offset of the Boom-Boom Y-byte for each fortress (same order as
/// FORTRESS_ENTRIES). The Y-byte upper nibble encodes the fortress ordinal
/// (1-based Map_DoFortressFX value); the lower nibble is spawn Y position.
pub(super) const BOOMBOOM_Y_OFFSETS: [usize; 17] = [
    0x0D35F, // W1[11]
    0x0D262, // W2[13]
    0x0D3D3, // W3[13]
    0x0D3A1, // W3[34]
    0x0D536, // W4[ 9]
    0x0D55F, // W4[16]
    0x0D40F, // W5[12]
    0x0D2C7, // W5[31]
    0x0D4E1, // W6[ 9]
    0x0CAE1, // W6[27]
    0x0D4B0, // W6[48]
    0x0D4FA, // W7[ 5]
    0x0D47E, // W7[40]
    0x0DA32, // W8[ 7]
    0x0DA37, // W8[10]
    0x0D597, // W8[26]
    0x0DA2D, // W8[36]
];

/// The 1-F fortress obj_ptr. This fortress level has a secret exit that
/// bypasses the Boom-Boom boss (no crystal ball → no FX trigger → lock
/// stays closed). Must be placed in a slot whose lock is secret_exit_safe.
pub(super) const FORTRESS_1F_OBJ_PTR: u16 = 0xD32B;

/// Vanilla fortress obj_ptrs (same order as FORTRESS_ENTRIES).
/// The obj_ptr identifies the fortress level's enemy data stream in PRG006.
/// After level shuffle, the obj_ptr at a slot still points to the same enemy
/// data — only the pointer table entries move, not the data itself.
pub(super) const VANILLA_FORTRESS_OBJ_PTRS: [u16; 17] = [
    0xD32B, // W1[11]
    0xD222, // W2[13]
    0xD393, // W3[13]
    0xD362, // W3[34]
    0xD508, // W4[ 9]
    0xD528, // W4[16]
    0xD3D0, // W5[12]
    0xD2B4, // W5[31]
    0xD4B0, // W6[ 9]
    0xCAAB, // W6[27]
    0xD470, // W6[48]
    0xD4E4, // W7[ 5]
    0xD41B, // W7[40]
    0xD8CC, // W8[ 7]
    0xD867, // W8[10]
    0xD551, // W8[26]
    0xD91C, // W8[36]
];

/// Given an obj_ptr found at a fortress slot, return the Boom-Boom Y-byte
/// ROM file offset for that fortress's enemy data.
pub(super) fn boomboom_y_offset_for_obj(obj_ptr: u16) -> Option<usize> {
    VANILLA_FORTRESS_OBJ_PTRS
        .iter()
        .zip(BOOMBOOM_Y_OFFSETS.iter())
        .find(|&(&op, _)| op == obj_ptr)
        .map(|(_, &y)| y)
}

/// Known airship entries (world_idx, entry_idx).
pub(super) const AIRSHIP_ENTRIES: &[(usize, usize)] = &[
    (0, 17), (1, 36), (2, 49), (3, 6), (4, 35), (5, 53), (6, 43),
];

/// Bowser's castle entry.
pub(super) const BOWSER_ENTRY: (usize, usize) = (7, 40);

/// Known toad house obj_ptrs. The standard format is $0700; the variant
/// formats ($0300-$0900) select different reward pools/game types but all
/// load a toad house screen. All share lay=$AD60.
pub(super) const TOAD_HOUSE_OBJ_PTRS: &[u16] = &[
    0x0300, 0x0400, 0x0500, 0x0600, 0x0700, 0x0800, 0x0900,
];

/// Known hammer bro level obj_ptrs. Each world's hammer bro encounters point
/// to one of these object streams. Multiple pointer table entries share the
/// same obj_ptr (with varying layouts/tilesets).
/// W8's 0xC03D is excluded — it uses a full action level layout (7-7), not
/// a short HB battle.
pub(super) const HAMMER_BRO_OBJ_PTRS: &[u16] = &[
    0xC72B, // W1
    0xD14D, // W2
    0xD142, // W2 (variant)
    0xC640, // W3, W5, W6, W7
    0xD0EA, // W4
];

/// Map transition entries.
pub(super) const MAP_TRANSITIONS: &[(usize, usize)] = &[];

// ---------------------------------------------------------------------------
// Overworld map object tables (PRG011)
// ---------------------------------------------------------------------------

/// Master pointer table for Map_List_Object_Ys (8 words, one per world).
const MAP_OBJ_YS_MASTER: usize = 0x16020;
/// Master pointer table for Map_List_Object_XHis.
const MAP_OBJ_XHIS_MASTER: usize = 0x16030;
/// Master pointer table for Map_List_Object_XLos.
const MAP_OBJ_XLOS_MASTER: usize = 0x16040;

/// Map object → pointer table entry linkage.
/// (world_idx, object_slot, pointer_table_entry_idx)
/// W7 piranha plants: stationary overworld sprites whose positions must
/// stay in sync with their pointer table entries after pipe shuffling.
pub(super) const MAP_OBJ_ENTRY_LINKS: &[(usize, usize, usize)] = &[
    (6, 2, 11), // W7 piranha plant 1
    (6, 3, 45), // W7 piranha plant 2
];

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// Mutable overworld tile grid.
#[derive(Clone, Debug)]
pub(crate) struct Grid {
    pub tiles: Vec<Vec<u8>>,
    pub rows: usize,
    pub cols: usize,
}

impl Grid {
    pub fn get(&self, row: usize, col: usize) -> u8 {
        self.tiles[row][col]
    }

    pub fn set(&mut self, row: usize, col: usize, tile: u8) {
        self.tiles[row][col] = tile;
    }

}

/// Data that travels with a level when shuffled.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct LevelEntry {
    pub tileset: u8,
    pub obj_lo: u8,
    pub obj_hi: u8,
    pub lay_lo: u8,
    pub lay_hi: u8,
}

/// An FX slot (lock/bridge position and replacement tile).
pub(super) struct FxSlot {
    pub grid_row: usize,
    pub grid_col: usize,
    pub replace_tile: u8,
}

// ---------------------------------------------------------------------------
// ROM helpers
// ---------------------------------------------------------------------------

/// Read a 16-bit little-endian word from ROM.
pub(super) fn read_word(rom: &Rom, offset: usize) -> u16 {
    let lo = rom.read_byte(offset) as u16;
    let hi = rom.read_byte(offset + 1) as u16;
    (hi << 8) | lo
}

/// Compute sub-table file offsets for a world's pointer tables.
/// Returns (scrcol_offset, objsets_offset, layouts_offset).
pub(super) fn table_offsets(world: &WorldTables) -> (usize, usize, usize) {
    let n = world.entry_count;
    let scrcol = world.rowtype_offset + n;
    let objsets = scrcol + n;
    let layouts = objsets + n * 2;
    (scrcol, objsets, layouts)
}

/// Get the (grid_row, grid_col) for a pointer table entry.
pub(super) fn entry_grid_position(rom: &Rom, world: &WorldTables, idx: usize) -> (usize, usize) {
    let row_nibble = (rom.read_byte(world.rowtype_offset + idx) >> 4) & 0x0F;
    let scrcol = rom.read_byte(world.rowtype_offset + world.entry_count + idx);
    let screen = (scrcol >> 4) & 0x0F;
    let column = scrcol & 0x0F;
    let grid_row = (row_nibble as usize).wrapping_sub(2);
    let grid_col = screen as usize * 16 + column as usize;
    (grid_row, grid_col)
}

/// Compute the ROM file offset of a map tile at (row, col).
pub(super) fn map_tile_offset(world_idx: usize, row: usize, col: usize) -> usize {
    let info = &MAP_TILE_GRIDS[world_idx];
    let screen = col / 16;
    let col_in_screen = col % 16;
    info.file_offset + screen * 144 + row * 16 + col_in_screen
}

// ---------------------------------------------------------------------------
// Level entry helpers
// ---------------------------------------------------------------------------

/// PRG bank loaded at CPU $A000-$BFFF for each tileset (0-18).
pub(super) const PAGE_A000_BY_TILESET: [usize; 19] = [
    11, 15, 21, 16, 17, 19, 18, 18, 18, 20, 23, 19, 17, 19, 13, 26, 26, 26, 9,
];

/// Returns true if this map entry has a real level pointer (not a toad house,
/// bonus game, hand trap, or pipe junction).
pub(super) fn is_level_pointer(obj_ptr: u16, lay_ptr: u16) -> bool {
    obj_ptr >= 0xC000 && lay_ptr != 0x0000
}

/// Convert a layout CPU address ($A000-$BFFF) + tileset to a ROM file offset.
pub(super) fn layout_file_offset(cpu_addr: u16, tileset: u8) -> Option<usize> {
    if tileset as usize >= PAGE_A000_BY_TILESET.len() || cpu_addr < 0xA000 {
        return None;
    }
    let bank = PAGE_A000_BY_TILESET[tileset as usize];
    Some(bank * 0x2000 + 0x10 + (cpu_addr as usize - 0xA000))
}

/// ROM file offset of PRG006 enemy/object data base (CPU $C000).
const ENEMY_DATA_FILE_BASE: usize = 0x0C010;

/// Check whether the first enemy data segment at `obj_ptr` contains `target_id`.
///
/// Enemy data format: 1-byte page flag, then 3-byte entries `[id, x, y]`,
/// terminated by `0xFF`. Only the first segment is scanned.
pub(super) fn has_enemy_id(rom: &Rom, obj_ptr: u16, target_id: u8) -> bool {
    if obj_ptr < 0xC000 {
        return false;
    }
    let file_off = ENEMY_DATA_FILE_BASE + (obj_ptr as usize - 0xC000);
    if file_off + 1 >= rom.data.len() {
        return false;
    }
    let mut pos = file_off + 1; // skip page flag byte
    while pos + 2 < rom.data.len() {
        if rom.data[pos] == 0xFF {
            break;
        }
        if rom.data[pos] == target_id {
            return true;
        }
        pos += 3;
    }
    false
}

/// Read the screen count from a level's 9-byte header.
/// Header byte 4, bits 3-0 = (num_screens - 1).
pub(super) fn level_screen_count(rom: &Rom, layout_offset: usize) -> u8 {
    (rom.read_byte(layout_offset + 4) & 0x0F) + 1
}

/// Read a LevelEntry from ROM for a given world and entry index.
pub(super) fn read_entry(rom: &Rom, world: &WorldTables, idx: usize) -> LevelEntry {
    let (_scrcol, objsets, layouts) = table_offsets(world);
    let obj_off = objsets + idx * 2;
    let lay_off = layouts + idx * 2;

    LevelEntry {
        tileset: rom.read_byte(world.rowtype_offset + idx) & 0x0F,
        obj_lo: rom.read_byte(obj_off),
        obj_hi: rom.read_byte(obj_off + 1),
        lay_lo: rom.read_byte(lay_off),
        lay_hi: rom.read_byte(lay_off + 1),
    }
}

/// Write a LevelEntry back to ROM for a given world and entry index.
/// Only the tileset (lower nibble of ByRowType) is updated — the upper
/// nibble (map row position) is preserved.
pub(super) fn write_entry(rom: &mut Rom, world: &WorldTables, idx: usize, entry: &LevelEntry) {
    let (_scrcol, objsets, layouts) = table_offsets(world);
    let obj_off = objsets + idx * 2;
    let lay_off = layouts + idx * 2;

    let old_brt = rom.read_byte(world.rowtype_offset + idx);
    let new_brt = (old_brt & 0xF0) | (entry.tileset & 0x0F);
    rom.write_byte(world.rowtype_offset + idx, new_brt);

    rom.write_byte(obj_off, entry.obj_lo);
    rom.write_byte(obj_off + 1, entry.obj_hi);
    rom.write_byte(lay_off, entry.lay_lo);
    rom.write_byte(lay_off + 1, entry.lay_hi);
}

// ---------------------------------------------------------------------------
// Grid reading
// ---------------------------------------------------------------------------

/// Read a world's tile grid from ROM as a mutable Grid.
pub(super) fn read_tile_grid(rom: &Rom, world_idx: usize) -> Grid {
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

    Grid { tiles, rows: ROWS, cols }
}

/// Find the START tile position in a grid.
pub(super) fn find_start(grid: &Grid) -> Option<(usize, usize)> {
    for r in 0..grid.rows {
        for c in 0..grid.cols {
            if grid.get(r, c) == TILE_START {
                return Some((r, c));
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Pipe data reading
// ---------------------------------------------------------------------------

/// Get destination table indices that belong to a given world.
pub(super) fn dest_indices_for_world(world_idx: usize) -> Vec<usize> {
    DEST_TO_WORLD
        .iter()
        .filter(|&&(_, w)| w == world_idx)
        .map(|&(d, _)| d as usize)
        .collect()
}


/// Read all pipe pairs from ROM destination tables, grouped by world.
/// Returns a map: world_idx → Vec of ((row_a, col_a), (row_b, col_b)).
#[cfg(test)]
pub(super) fn read_pipe_pairs(rom: &Rom) -> std::collections::HashMap<usize, Vec<((usize, usize), (usize, usize))>> {
    let mut pipes_by_world: std::collections::HashMap<usize, Vec<_>> = std::collections::HashMap::new();

    for &(dest, world_idx) in DEST_TO_WORLD {
        let d = dest as usize;
        let xhi = rom.read_byte(PIPE_MAP_XHI + d);
        let x = rom.read_byte(PIPE_MAP_X + d);
        let y = rom.read_byte(PIPE_MAP_Y + d);

        let a_scr = ((xhi >> 4) & 0x0F) as usize;
        let b_scr = (xhi & 0x0F) as usize;
        let a_col = ((x >> 4) & 0x0F) as usize;
        let b_col = (x & 0x0F) as usize;
        let a_row_nib = ((y >> 4) & 0x0F) as usize;
        let b_row_nib = (y & 0x0F) as usize;

        let a_pos = (a_row_nib.wrapping_sub(2), a_scr * 16 + a_col);
        let b_pos = (b_row_nib.wrapping_sub(2), b_scr * 16 + b_col);

        pipes_by_world.entry(world_idx).or_default().push((a_pos, b_pos));
    }

    pipes_by_world
}

// ---------------------------------------------------------------------------
// FX helpers
// ---------------------------------------------------------------------------

/// Read all 17 FX slots from ROM.
pub(super) fn read_fx_slots(rom: &Rom) -> Vec<FxSlot> {
    let mut slots = Vec::with_capacity(17);
    for i in 0..17 {
        let loc_row = rom.read_byte(FX_MAP_LOC_ROW + i);
        let loc = rom.read_byte(FX_MAP_LOC + i);
        let replace_tile = rom.read_byte(FX_MAP_TILE_REPLACE + i);

        let grid_row = ((loc_row >> 4) as usize).wrapping_sub(2);
        let col_in_screen = ((loc >> 4) & 0x0F) as usize;
        let screen = (loc & 0x0F) as usize;

        slots.push(FxSlot {
            grid_row,
            grid_col: screen * 16 + col_in_screen,
            replace_tile,
        });
    }
    slots
}

/// Read FortressFX_W1-W8: which FX slots each world uses.
/// Returns array of 8 Vecs, one per world.
///
/// Each world has 4 bytes in the table, but only the first N are meaningful
/// where N = number of fortresses in that world. The rest are zero-padded.
/// We use the fortress count from FORTRESS_ENTRIES to know how many to read.
pub(super) fn read_world_fx_assignments(rom: &Rom) -> [Vec<u8>; 8] {
    let mut assignments: [Vec<u8>; 8] = Default::default();
    for wi in 0..8 {
        let fort_count = FORTRESS_ENTRIES.iter().filter(|&&(w, _)| w == wi).count();
        let base = FX_WORLD_TABLE + wi * 4;
        for i in 0..fort_count.min(4) {
            assignments[wi].push(rom.read_byte(base + i));
        }
    }
    assignments
}

/// Read grid positions of fortress entries for a world.
#[cfg(test)]
pub(super) fn read_fortress_positions(rom: &Rom, world_idx: usize) -> Vec<(usize, usize)> {
    let world = &WORLDS[world_idx];
    FORTRESS_ENTRIES
        .iter()
        .filter(|&&(w, _)| w == world_idx)
        .map(|&(_, ei)| entry_grid_position(rom, world, ei))
        .collect()
}

// ---------------------------------------------------------------------------
// Map object position sync
// ---------------------------------------------------------------------------

/// Resolve a master pointer table entry to a ROM file offset for a given slot.
/// The master table holds 8 CPU-address words ($A010 bank); each points to a
/// 9-byte per-world sub-table.
fn map_obj_slot_offset(rom: &Rom, master_table: usize, world_idx: usize, slot: usize) -> usize {
    let cpu = read_word(rom, master_table + world_idx * 2) as usize;
    // PRG011 is bank 11 → file offset = 11 * 0x2000 + 0x10 + (cpu - 0xA000)
    0x16010 + (cpu - 0xA000) + slot
}


/// Write a map object sprite's position to the map object tables.
///
/// Converts a grid position to pixel coordinates and writes to the Y/XHi/XLo
/// tables for the given world and slot.
pub(super) fn write_map_sprite_position(
    rom: &mut Rom,
    world_idx: usize,
    slot: usize,
    grid_row: usize,
    grid_col: usize,
) {
    let y = ((grid_row + 2) * 16) as u8;
    let xhi = (grid_col / 16) as u8;
    let xlo = ((grid_col % 16) * 16) as u8;

    let y_off = map_obj_slot_offset(rom, MAP_OBJ_YS_MASTER, world_idx, slot);
    let xhi_off = map_obj_slot_offset(rom, MAP_OBJ_XHIS_MASTER, world_idx, slot);
    let xlo_off = map_obj_slot_offset(rom, MAP_OBJ_XLOS_MASTER, world_idx, slot);

    rom.write_byte(y_off, y);
    rom.write_byte(xhi_off, xhi);
    rom.write_byte(xlo_off, xlo);
}

/// Read the grid positions of all active floating sprites for a world.
///
/// Each world has up to 9 map object slots. A slot with ID $FF is unused.
/// For active slots, we convert pixel coordinates back to grid positions.
/// These are the positions where floating sprites sit (hammer bros, piranhas,
/// W8 hand traps, etc.) and should not have level/fort tiles placed under them.
pub(super) fn read_map_sprite_positions(rom: &Rom, world_idx: usize) -> Vec<(usize, usize)> {
    const MAP_OBJ_IDS_MASTER: usize = 0x16050;
    let mut positions = Vec::new();

    for slot in 0..9 {
        let id_off = map_obj_slot_offset(rom, MAP_OBJ_IDS_MASTER, world_idx, slot);
        let id = rom.read_byte(id_off);
        if id == 0xFF {
            continue; // unused slot
        }

        let y_off = map_obj_slot_offset(rom, MAP_OBJ_YS_MASTER, world_idx, slot);
        let xhi_off = map_obj_slot_offset(rom, MAP_OBJ_XHIS_MASTER, world_idx, slot);
        let xlo_off = map_obj_slot_offset(rom, MAP_OBJ_XLOS_MASTER, world_idx, slot);

        let y = rom.read_byte(y_off) as usize;
        let xhi = rom.read_byte(xhi_off) as usize;
        let xlo = rom.read_byte(xlo_off) as usize;

        // Reverse of Grid→pixel: Y=(row+2)*16, XHi=col/16, XLo=(col%16)*16
        if y < 32 {
            continue; // invalid (row would be negative)
        }
        let row = (y / 16).saturating_sub(2);
        let col = xhi * 16 + xlo / 16;

        positions.push((row, col));
    }

    positions
}

/// Read grid positions of hammer bro sprites only (IDs 0x03–0x06).
///
/// These positions need HB level pointer entries even though they are excluded
/// from level/fort/pipe placement by `fixed_positions`.
pub(super) fn read_hb_sprite_positions(rom: &Rom, world_idx: usize) -> Vec<(usize, usize)> {
    const MAP_OBJ_IDS_MASTER: usize = 0x16050;
    let mut positions = Vec::new();

    for slot in 0..9 {
        let id_off = map_obj_slot_offset(rom, MAP_OBJ_IDS_MASTER, world_idx, slot);
        let id = rom.read_byte(id_off);
        if !(0x03..=0x06).contains(&id) {
            continue;
        }

        let y_off = map_obj_slot_offset(rom, MAP_OBJ_YS_MASTER, world_idx, slot);
        let xhi_off = map_obj_slot_offset(rom, MAP_OBJ_XHIS_MASTER, world_idx, slot);
        let xlo_off = map_obj_slot_offset(rom, MAP_OBJ_XLOS_MASTER, world_idx, slot);

        let y = rom.read_byte(y_off) as usize;
        let xhi = rom.read_byte(xhi_off) as usize;
        let xlo = rom.read_byte(xlo_off) as usize;

        if y < 32 {
            continue;
        }
        let row = (y / 16).saturating_sub(2);
        let col = xhi * 16 + xlo / 16;

        positions.push((row, col));
    }

    positions
}
