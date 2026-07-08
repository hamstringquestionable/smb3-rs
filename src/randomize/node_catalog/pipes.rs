//! Pipe-pair matching: groups pipe endpoints into pairs and works out which
//! endpoint is the A-side in the pipe destination tables.

use std::collections::HashMap;

use crate::rom::Rom;

use crate::randomize::rom_data::{self, PIPE_MAP_X, PIPE_MAP_XHI, PIPE_MAP_Y, WORLDS};

// ---------------------------------------------------------------------------
// Pipe pair matching
// ---------------------------------------------------------------------------

/// Build a map from entry_idx → (dest_idx, is_a_side) for all pipe entries.
///
/// The A-side is the entry whose dest table upper nibble encodes its position.
/// For regular pipe pairs (both share an obj_ptr and have PIPEWAYCONTROLLER),
/// A-side has layout byte5 bit 6 = 0.  For mixed pairs (one has PWC, one
/// doesn't — e.g. W5 spiral castle), the PWC entry is the A-side.
pub(super) fn build_pipe_map(
    rom: &Rom,
    world_idx: usize,
    pipe_entries_by_obj: &HashMap<u16, Vec<usize>>,
    spiral_entries: &[usize],
    dest_indices: &[usize],
) -> HashMap<usize, (usize, bool)> {
    let world = &WORLDS[world_idx];
    let mut result: HashMap<usize, (usize, bool)> = HashMap::new();

    // Collect all pipe pairs: (entry_a, entry_b)
    let mut pairs: Vec<(usize, usize)> = Vec::new();

    // Regular pipe pairs: grouped by obj_ptr
    let mut keys: Vec<u16> = pipe_entries_by_obj.keys().copied().collect();
    keys.sort();
    for key in keys {
        let group = &pipe_entries_by_obj[&key];
        if group.len() == 2 {
            pairs.push((group[0], group[1]));
        }
    }

    // W5 spiral tower pair
    if world_idx == 4 && spiral_entries.len() == 2 {
        let mut sorted = spiral_entries.to_vec();
        sorted.sort();
        pairs.push((sorted[0], sorted[1]));
    }

    // Match pairs to dest indices by comparing grid positions
    for &(ea, eb) in &pairs {
        let ea_pos = rom_data::entry_grid_position(rom, world, ea);
        let eb_pos = rom_data::entry_grid_position(rom, world, eb);

        for &d in dest_indices {
            let (da, db) = read_dest_positions(rom, d);
            if (ea_pos == da && eb_pos == db) || (ea_pos == db && eb_pos == da) {
                let (a_entry, b_entry) = classify_pipe_ab(rom, world, ea, eb);
                result.insert(a_entry, (d, true));
                result.insert(b_entry, (d, false));
                break;
            }
        }
    }

    result
}

/// Determine which of two pipe entries is the A-side (upper nibble in dest tables).
///
/// Mixed pairs (one has PIPEWAYCONTROLLER, one doesn't): PWC entry → A-side.
/// Regular pairs (both have PWC): layout byte5 bit 6 = 0 → A-side.
/// Fallback: (ea, eb) order preserved.
fn classify_pipe_ab(rom: &Rom, world: &rom_data::WorldTables, ea: usize, eb: usize) -> (usize, usize) {
    let le_a = rom_data::read_entry(rom, world, ea);
    let le_b = rom_data::read_entry(rom, world, eb);

    let obj_a = u16::from_le_bytes([le_a.obj_lo, le_a.obj_hi]);
    let obj_b = u16::from_le_bytes([le_b.obj_lo, le_b.obj_hi]);

    let has_pwc_a = rom_data::has_enemy_id(rom, obj_a, 0x25);
    let has_pwc_b = rom_data::has_enemy_id(rom, obj_b, 0x25);

    // Mixed pair: PWC entry is A-side.
    if has_pwc_a && !has_pwc_b {
        return (ea, eb);
    }
    if has_pwc_b && !has_pwc_a {
        return (eb, ea);
    }

    // Regular pair: use layout byte5 bit 6. A-side has bit 6 = 0.
    let lay_ptr_a = u16::from_le_bytes([le_a.lay_lo, le_a.lay_hi]);
    if let Some(file_off) = rom_data::layout_file_offset(lay_ptr_a, le_a.tileset) {
        let byte5 = rom.read_byte(file_off + 5);
        if byte5 & 0x40 == 0 {
            return (ea, eb);
        } else {
            return (eb, ea);
        }
    }

    // Fallback: preserve original order.
    (ea, eb)
}

/// Read the A and B endpoint positions from the pipe destination tables.
fn read_dest_positions(rom: &Rom, dest_idx: usize) -> ((usize, usize), (usize, usize)) {
    let xhi = rom.read_byte(PIPE_MAP_XHI + dest_idx);
    let x = rom.read_byte(PIPE_MAP_X + dest_idx);
    let y = rom.read_byte(PIPE_MAP_Y + dest_idx);

    let a_pos = (
        ((y >> 4) as usize).wrapping_sub(2),
        ((xhi >> 4) as usize) * 16 + ((x >> 4) as usize),
    );
    let b_pos = (
        ((y & 0xF) as usize).wrapping_sub(2),
        ((xhi & 0xF) as usize) * 16 + ((x & 0xF) as usize),
    );
    (a_pos, b_pos)
}
