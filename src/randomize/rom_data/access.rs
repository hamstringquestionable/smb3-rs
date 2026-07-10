//! Typed read/write helpers over the ROM: level entries, tile grids, pipe
//! pairs, FX slots, map sprites, and bank/offset math.

use super::*;

/// Read a 16-bit little-endian word from ROM.
pub(crate) fn read_word(rom: &Rom, offset: usize) -> u16 {
    let lo = rom.read_byte(offset) as u16;
    let hi = rom.read_byte(offset + 1) as u16;
    (hi << 8) | lo
}

/// Compute sub-table file offsets for a world's pointer tables.
/// Returns (scrcol_offset, objsets_offset, layouts_offset).
pub(crate) fn table_offsets(world: &WorldTables) -> (usize, usize, usize) {
    let n = world.entry_count;
    let scrcol = world.rowtype_offset + n;
    let objsets = scrcol + n;
    let layouts = objsets + n * 2;
    (scrcol, objsets, layouts)
}

/// Get the (grid_row, grid_col) for a pointer table entry.
pub(crate) fn entry_grid_position(rom: &Rom, world: &WorldTables, idx: usize) -> (usize, usize) {
    let row_nibble = (rom.read_byte(world.rowtype_offset + idx) >> 4) & 0x0F;
    let scrcol = rom.read_byte(world.rowtype_offset + world.entry_count + idx);
    let screen = (scrcol >> 4) & 0x0F;
    let column = scrcol & 0x0F;
    let grid_row = (row_nibble as usize).wrapping_sub(2);
    let grid_col = screen as usize * 16 + column as usize;
    (grid_row, grid_col)
}

/// Compute the ROM file offset of a map tile at (row, col).
pub(crate) fn map_tile_offset(world_idx: usize, row: usize, col: usize) -> usize {
    let info = &MAP_TILE_GRIDS[world_idx];
    let screen = col / 16;
    let col_in_screen = col % 16;
    info.file_offset + screen * 144 + row * 16 + col_in_screen
}

/// PRG bank loaded at CPU $A000-$BFFF for each tileset (0-18).
pub(crate) const PAGE_A000_BY_TILESET: [usize; 19] = [
    11, 15, 21, 16, 17, 19, 18, 18, 18, 20, 23, 19, 17, 19, 13, 26, 26, 26, 9,
];

/// Returns true if this map entry has a real level pointer (not a toad house,
/// bonus game, hand trap, or pipe junction).
pub(crate) fn is_level_pointer(obj_ptr: u16, lay_ptr: u16) -> bool {
    obj_ptr >= 0xC000 && lay_ptr != 0x0000
}

/// Convert a CPU address in a fixed PRG bank mapped to the $A000-$BFFF window
/// into its ROM file offset: `bank * 0x2000 + 0x10 + (cpu - 0xA000)`.
///
/// The `+ 0x10` is the iNES header. Forgetting it shifts the result by 16
/// bytes, which silently mis-aims any JSR/JMP operand derived from it — the
/// root cause of issue #14. Use this (and [`jsr_into_bank`]) instead of
/// open-coding the formula so the header offset lives in exactly one place.
pub(crate) fn prg_bank_cpu_to_file(bank: usize, cpu_addr: u16) -> usize {
    bank * 0x2000 + 0x10 + (cpu_addr as usize - 0xA000)
}

/// Inverse of [`prg_bank_cpu_to_file`]: file offset within an $A000-window
/// bank back to its CPU address.
pub(crate) const fn prg_bank_file_to_cpu(bank: usize, file_offset: usize) -> u16 {
    (0xA000 + (file_offset - bank * 0x2000 - 0x10)) as u16
}

/// Convert a file offset in PRG031 (the MMC3 fixed bank, always mapped at
/// $E000-$FFFF, file 0x3E010) to its CPU address.
pub(crate) const fn prg031_file_to_cpu(file_offset: usize) -> u16 {
    (0xE000 + (file_offset - 0x3E010)) as u16
}

/// Build a 3-byte `JSR <target>` where `target` is given as a *file offset*
/// inside `bank` (the $A000-window bank live when the hook runs). The operand
/// is computed from [`prg_bank_file_to_cpu`], so it can never drift from where
/// the target bytes are actually written. Prefer this over hand-writing the
/// `[0x20, lo, hi]` literal for any same-bank hook → free-space-helper call.
pub(crate) fn jsr_into_bank(bank: usize, target_file: usize) -> [u8; 3] {
    let cpu = prg_bank_file_to_cpu(bank, target_file);
    [0x20, (cpu & 0xFF) as u8, (cpu >> 8) as u8]
}

/// Convert a layout CPU address ($A000-$BFFF) + tileset to a ROM file offset.
pub(crate) fn layout_file_offset(cpu_addr: u16, tileset: u8) -> Option<usize> {
    if tileset as usize >= PAGE_A000_BY_TILESET.len() || cpu_addr < 0xA000 {
        return None;
    }
    let bank = PAGE_A000_BY_TILESET[tileset as usize];
    Some(prg_bank_cpu_to_file(bank, cpu_addr))
}

/// ROM file offset of PRG006 enemy/object data base (CPU $C000).
pub(crate) const ENEMY_DATA_FILE_BASE: usize = 0x0C010;

/// Translate a CPU enemy-data pointer (`$C000..=$E00D`) to its absolute file
/// offset.
pub(crate) fn enemy_ptr_to_file_offset(ep: u16) -> usize {
    ENEMY_DATA_FILE_BASE + (ep as usize - 0xC000)
}

/// Enemy/object data block: 0x0BFD8..0x0E00D (exclusive end).
/// Each level's enemy set is a sequence of segments separated by 0xFF.
/// Each segment starts with a 1-byte page flag, then zero or more 3-byte
/// entries [object_id, x_pos, y_pos], terminated by 0xFF.
pub const ENEMY_DATA_START: usize = 0x0BFD8;

pub const ENEMY_DATA_END: usize = 0x0E00D;

/// Bro enemies that work in tileset 10 (8-Tank sub-area).
/// Excludes HammerBro (0x81) which fails to spawn in ts=10.
pub(crate) const TANK_BRO_POOL: &[u8] = &[
    0x82, // OBJ_BOOMERANGBRO
    0x86, // OBJ_HEAVYBRO
    0x87, // OBJ_FIREBRO
];

/// Check whether the first enemy data segment at `obj_ptr` contains `target_id`.
///
/// Enemy data format: 1-byte page flag, then 3-byte entries `[id, x, y]`,
/// terminated by `0xFF`. Only the first segment is scanned.
pub(crate) fn has_enemy_id(rom: &Rom, obj_ptr: u16, target_id: u8) -> bool {
    if obj_ptr < 0xC000 {
        return false;
    }
    let file_off = enemy_ptr_to_file_offset(obj_ptr);
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

/// Read a LevelEntry from ROM for a given world and entry index.
pub(crate) fn read_entry(rom: &Rom, world: &WorldTables, idx: usize) -> LevelEntry {
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
pub(crate) fn write_entry(rom: &mut Rom, world: &WorldTables, idx: usize, entry: &LevelEntry) {
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

/// Get destination table indices that belong to a given world.
pub(crate) fn dest_indices_for_world(world_idx: usize) -> Vec<usize> {
    DEST_TO_WORLD
        .iter()
        .filter(|&&(_, w)| w == world_idx)
        .map(|&(d, _)| d as usize)
        .collect()
}

/// Read all pipe pairs from ROM destination tables, grouped by world.
/// Returns a map: world_idx → Vec of ((row_a, col_a), (row_b, col_b)).
#[cfg(test)]
pub(crate) fn read_pipe_pairs(rom: &Rom) -> std::collections::HashMap<usize, Vec<TeleportEdge>> {
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

/// Read all 17 FX slots from ROM.
pub(crate) fn read_fx_slots(rom: &Rom) -> Vec<FxSlot> {
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
pub(crate) fn read_world_fx_assignments(rom: &Rom) -> [Vec<u8>; 8] {
    let mut assignments: [Vec<u8>; 8] = Default::default();
    for (wi, assignment) in assignments.iter_mut().enumerate() {
        let fort_count = FORTRESS_ENTRIES.iter().filter(|&&(w, _)| w == wi).count();
        let base = FX_WORLD_TABLE + wi * 4;
        for i in 0..fort_count.min(4) {
            assignment.push(rom.read_byte(base + i));
        }
    }
    assignments
}

/// Resolve a master pointer table entry to a ROM file offset for a given slot.
/// The master table holds 8 CPU-address words ($A010 bank); each points to a
/// 9-byte per-world sub-table.
pub(crate) fn map_obj_slot_offset(rom: &Rom, master_table: usize, world_idx: usize, slot: usize) -> usize {
    let cpu = read_word(rom, master_table + world_idx * 2);
    // PRG011 is bank 11; the sub-table base is `cpu`, the entry is `slot` past it.
    prg_bank_cpu_to_file(11, cpu) + slot
}

/// Write a map object sprite's position to the map object tables.
///
/// Converts a grid position to pixel coordinates and writes to the Y/XHi/XLo
/// tables for the given world and slot.
pub(crate) fn write_map_sprite_position(
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

/// Place a map-object sprite: write its position (Y/XHi/XLo) and its type ID.
/// Used to add new sprites (e.g. the W8 canoe at an otherwise-empty slot).
pub(crate) fn write_map_sprite(
    rom: &mut Rom,
    world_idx: usize,
    slot: usize,
    grid_row: usize,
    grid_col: usize,
    id: u8,
) {
    write_map_sprite_position(rom, world_idx, slot, grid_row, grid_col);
    let id_off = map_obj_slot_offset(rom, MAP_OBJ_IDS_MASTER, world_idx, slot);
    rom.write_byte(id_off, id);
}

/// True if a map-object slot id is a Hammer Bro sprite (0x03–0x06).
pub(crate) fn is_hb_sprite_id(id: u8) -> bool {
    (0x03..=0x06).contains(&id)
}

/// Read the grid positions of map-object sprites whose slot id satisfies
/// `pred`. Pixel coordinates are converted back to grid positions (reverse of
/// Grid→pixel: Y=(row+2)*16, XHi=col/16, XLo=(col%16)*16); slots whose Y would
/// put the row below 0 are skipped as invalid.
fn read_sprite_positions(
    rom: &Rom,
    world_idx: usize,
    pred: impl Fn(u8) -> bool,
) -> Vec<(usize, usize)> {
    let mut positions = Vec::new();

    for slot in 0..9 {
        let id_off = map_obj_slot_offset(rom, MAP_OBJ_IDS_MASTER, world_idx, slot);
        if !pred(rom.read_byte(id_off)) {
            continue;
        }

        let y_off = map_obj_slot_offset(rom, MAP_OBJ_YS_MASTER, world_idx, slot);
        let xhi_off = map_obj_slot_offset(rom, MAP_OBJ_XHIS_MASTER, world_idx, slot);
        let xlo_off = map_obj_slot_offset(rom, MAP_OBJ_XLOS_MASTER, world_idx, slot);

        let y = rom.read_byte(y_off) as usize;
        let xhi = rom.read_byte(xhi_off) as usize;
        let xlo = rom.read_byte(xlo_off) as usize;

        if y < 32 {
            continue; // invalid (row would be negative)
        }
        let row = (y / 16).saturating_sub(2);
        let col = xhi * 16 + xlo / 16;

        positions.push((row, col));
    }

    positions
}

/// Read the grid positions of all active floating sprites for a world.
///
/// Each world has up to 9 map object slots. A slot with ID $FF is unused.
/// These are the positions where floating sprites sit (hammer bros, piranhas,
/// W8 hand traps, etc.) and should not have level/fort tiles placed under them.
pub(crate) fn read_map_sprite_positions(rom: &Rom, world_idx: usize) -> Vec<(usize, usize)> {
    read_sprite_positions(rom, world_idx, |id| id != 0xFF)
}

/// Read grid positions of hammer bro sprites only (IDs 0x03–0x06).
///
/// These positions need HB level pointer entries even though they are excluded
/// from level/fort/pipe placement by `fixed_positions`.
pub(crate) fn read_hb_sprite_positions(rom: &Rom, world_idx: usize) -> Vec<(usize, usize)> {
    read_sprite_positions(rom, world_idx, is_hb_sprite_id)
}

/// Map-object reward item table (Global Item IDs). A flat 9-bytes-per-world
/// block laid out parallel to the map-object slot tables: `reward[world*9 +
/// slot]` is the item awarded for clearing the encounter at that slot. The
/// reward is keyed to `(world, slot)`, not to the level under the sprite — see
/// `docs/smb3_rom_reference.md`.
pub(crate) const MAP_OBJ_REWARDS: usize = 0x16190;

/// File offset of the reward byte for map-object `slot` in `world_idx`.
pub(crate) fn map_obj_reward_offset(world_idx: usize, slot: usize) -> usize {
    MAP_OBJ_REWARDS + world_idx * 9 + slot
}

/// First map-object slot usable for sprite placement in a world. Slot 0
/// always holds a fixed non-HB marker (`id 0x01`) and is reserved; slot 1
/// (the airship sprite slot) is reserved in W1-W7 but usable in W8, which has
/// no airship.
pub(crate) fn first_usable_map_obj_slot(world_idx: usize) -> usize {
    if world_idx == W8_IDX { 1 } else { 2 }
}

/// Map-object slot indices that can host a redistributed Hammer Bro sprite in
/// this world (see [`first_usable_map_obj_slot`] for the reserved low slots).
/// A slot qualifies if it is empty (`0x00`) or currently holds a Hammer Bro
/// (`0x03-0x06`, which redistribution clears) — so the result is identical
/// before and after [`clear_hb_sprites`].
pub(crate) fn eligible_hb_map_slots(rom: &Rom, world_idx: usize) -> Vec<usize> {
    (first_usable_map_obj_slot(world_idx)..9)
        .filter(|&slot| {
            let id = rom.read_byte(map_obj_slot_offset(rom, MAP_OBJ_IDS_MASTER, world_idx, slot));
            id == 0x00 || is_hb_sprite_id(id)
        })
        .collect()
}

/// Collect the reward byte of every Hammer-Bro map-object sprite (id
/// `0x03-0x06`) across all worlds, in `(world, slot)` order. These travel with
/// the encounters when Hammer Bros are redistributed.
pub(crate) fn collect_hb_sprite_rewards(rom: &Rom) -> Vec<u8> {
    let mut rewards = Vec::new();
    for world_idx in 0..8 {
        for slot in 0..9 {
            let id = rom.read_byte(map_obj_slot_offset(rom, MAP_OBJ_IDS_MASTER, world_idx, slot));
            if is_hb_sprite_id(id) {
                rewards.push(rom.read_byte(map_obj_reward_offset(world_idx, slot)));
            }
        }
    }
    rewards
}

/// Clear every Hammer-Bro map-object sprite (id `0x03-0x06`) in a world: the
/// slot's id, position, and reward byte are zeroed so the tile is freed and the
/// sprite no longer spawns. Used before writing redistributed Hammer Bros.
pub(crate) fn clear_hb_sprites(rom: &mut Rom, world_idx: usize) {
    for slot in 0..9 {
        let id_off = map_obj_slot_offset(rom, MAP_OBJ_IDS_MASTER, world_idx, slot);
        if is_hb_sprite_id(rom.read_byte(id_off)) {
            clear_map_sprite(rom, world_idx, slot);
        }
    }
}

/// Highest-index empty map-object slot usable for a stationary sprite,
/// scanning from the top so the low slots stay free for the Hammer-Bro
/// writer (which fills eligible slots from the bottom) and the reserved
/// dynamic-spawn buffer.
pub(crate) fn last_empty_map_obj_slot(rom: &Rom, world_idx: usize) -> Option<usize> {
    (first_usable_map_obj_slot(world_idx)..9).rev().find(|&slot| {
        rom.read_byte(map_obj_slot_offset(rom, MAP_OBJ_IDS_MASTER, world_idx, slot)) == 0x00
    })
}

/// Clear a single map-object slot: id, position, and reward byte zeroed so
/// the sprite no longer spawns and the slot reads as empty (`0x00`).
pub(crate) fn clear_map_sprite(rom: &mut Rom, world_idx: usize, slot: usize) {
    rom.write_byte(map_obj_slot_offset(rom, MAP_OBJ_IDS_MASTER, world_idx, slot), 0);
    rom.write_byte(map_obj_slot_offset(rom, MAP_OBJ_YS_MASTER, world_idx, slot), 0);
    rom.write_byte(map_obj_slot_offset(rom, MAP_OBJ_XHIS_MASTER, world_idx, slot), 0);
    rom.write_byte(map_obj_slot_offset(rom, MAP_OBJ_XLOS_MASTER, world_idx, slot), 0);
    rom.write_byte(map_obj_reward_offset(world_idx, slot), 0);
}

/// Write a redistributed Hammer-Bro sprite into a specific map-object slot:
/// its type id, grid position, and reward byte.
pub(crate) fn write_hb_sprite(
    rom: &mut Rom,
    world_idx: usize,
    slot: usize,
    grid_row: usize,
    grid_col: usize,
    id: u8,
    reward: u8,
) {
    write_map_sprite(rom, world_idx, slot, grid_row, grid_col, id);
    rom.write_byte(map_obj_reward_offset(world_idx, slot), reward);
}

/// Read the non-Hammer-Bro floating sprite positions for a world (army, canoe,
/// piranhas, …). When Hammer Bros are redistributed, only these stay fixed and
/// must be protected from level/fort placement; the vanilla HB tiles are freed.
pub(crate) fn read_non_hb_sprite_positions(rom: &Rom, world_idx: usize) -> Vec<(usize, usize)> {
    let hb: std::collections::HashSet<(usize, usize)> =
        read_hb_sprite_positions(rom, world_idx).into_iter().collect();
    read_map_sprite_positions(rom, world_idx)
        .into_iter()
        .filter(|pos| !hb.contains(pos))
        .collect()
}
