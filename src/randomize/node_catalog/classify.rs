//! Per-world classification: reads each pointer table entry from the ROM and
//! assigns it a `NodeKind`. Pure reads — no RNG, no writes.

use std::collections::{HashMap, HashSet};

use crate::rom::Rom;

use super::pipes::build_pipe_map;
use super::{NodeKind, RawClassifiedEntry};
use crate::randomize::rom_data::{
    self, AIRSHIP_ENTRIES, BOWSER_ENTRY, FORTRESS_ENTRIES, HAMMER_BRO_OBJ_PTRS,
    MAP_OBJ_ENTRY_LINKS, TILE_START, TOAD_HOUSE_OBJ_PTRS, WORLDS,
};

// ---------------------------------------------------------------------------
// W5 Spiral Tower entries (functionally a pipe pair using dest index 0)
// ---------------------------------------------------------------------------

const W5_SPIRAL_ENTRIES: &[(usize, usize)] = &[(4, 10), (4, 21)];

// ---------------------------------------------------------------------------
// Per-world classification
// ---------------------------------------------------------------------------

/// Classify all entries in a single world.
/// Returns: Vec of (entry_idx, kind, grid_pos, tile, level_entry).
pub(super) fn classify_world(
    rom: &Rom,
    world_idx: usize,
    grid: &rom_data::Grid,
) -> Vec<RawClassifiedEntry> {
    let world = &WORLDS[world_idx];
    let n = world.entry_count;
    let (_scrcol, objsets, layouts) = rom_data::table_offsets(world);

    // -- Pre-compute sets for classification --

    // Map-object-linked entries
    let map_obj_entries: HashSet<usize> = MAP_OBJ_ENTRY_LINKS
        .iter()
        .filter(|&&(w, _, _)| w == world_idx)
        .map(|&(_, _, entry_idx)| entry_idx)
        .collect();

    // Pipe detection: PIPEWAYCONTROLLER (0x25) enemy, grouped by obj_ptr
    let mut pipe_entries_by_obj: HashMap<u16, Vec<usize>> = HashMap::new();
    let mut spiral_entries: Vec<usize> = Vec::new();
    for i in 0..n {
        let obj = rom_data::read_word(rom, objsets + i * 2);
        if W5_SPIRAL_ENTRIES.contains(&(world_idx, i)) {
            spiral_entries.push(i);
        } else if rom_data::has_enemy_id(rom, obj, 0x25) {
            pipe_entries_by_obj.entry(obj).or_default().push(i);
        }
    }

    // Build pipe pair map: entry_idx → dest_idx
    let dest_indices = rom_data::dest_indices_for_world(world_idx);
    let pipe_map = build_pipe_map(rom, world_idx, &pipe_entries_by_obj, &spiral_entries, &dest_indices);

    // -- Classify each entry --
    let mut result = Vec::with_capacity(n);

    for i in 0..n {
        let (row, col) = rom_data::entry_grid_position(rom, world, i);
        let obj = rom_data::read_word(rom, objsets + i * 2);
        let lay = rom_data::read_word(rom, layouts + i * 2);
        let map_tile = if row < grid.rows() && col < grid.cols {
            grid.get(row, col)
        } else {
            0xFF
        };

        let kind = classify_entry(
            rom, world_idx, i, obj, lay, map_tile, row,
            &map_obj_entries, &pipe_map,
        );

        let level_entry = if matches!(kind, NodeKind::Start) {
            None
        } else {
            Some(rom_data::read_entry(rom, world, i))
        };

        result.push((i, kind, (row, col), map_tile, level_entry));
    }

    result
}

/// Classify a single entry into a NodeKind.
// Reason: the catalog builder threads these as individual locals during a
// single pass over the pointer tables; bundling into a struct would add
// construction friction at every call site without revealing a real concept.
#[allow(clippy::too_many_arguments)]
fn classify_entry(
    rom: &Rom,
    world_idx: usize,
    entry_idx: usize,
    obj: u16,
    lay: u16,
    map_tile: u8,
    row: usize,
    map_obj_entries: &HashSet<usize>,
    pipe_map: &HashMap<usize, (usize, bool)>,
) -> NodeKind {
    // 1. Start tile
    if map_tile == TILE_START {
        return NodeKind::Start;
    }

    // 2. Bowser's castle
    if (world_idx, entry_idx) == BOWSER_ENTRY {
        return NodeKind::Bowser;
    }

    // 3. Airship
    if AIRSHIP_ENTRIES.contains(&(world_idx, entry_idx)) {
        return NodeKind::Airship;
    }

    // 4. Fortress
    if FORTRESS_ENTRIES.contains(&(world_idx, entry_idx)) {
        let entry = rom_data::read_entry(rom, &WORLDS[world_idx], entry_idx);
        let obj_ptr = (entry.obj_hi as u16) << 8 | entry.obj_lo as u16;
        let boomboom_y_offset = rom_data::boomboom_y_offset_for_obj(obj_ptr).unwrap_or(0);
        return NodeKind::Fortress { boomboom_y_offset };
    }

    // 5. Pipe (PIPEWAYCONTROLLER or W5 spiral)
    if let Some(&(dest_idx, is_a_side)) = pipe_map.get(&entry_idx) {
        return NodeKind::Pipe { dest_idx, is_a_side };
    }

    // 6. Toad house (standard $0700 + variant reward formats)
    if TOAD_HOUSE_OBJ_PTRS.contains(&obj) {
        return NodeKind::ToadHouse;
    }

    // 7. Bonus game
    if obj == 0x0001 && lay == 0x0000 {
        return NodeKind::BonusGame;
    }

    // 8. Map object (W7 piranha plants, etc.)
    if map_obj_entries.contains(&entry_idx) {
        return NodeKind::MapObject;
    }

    // 9. Hammer bro (known hammer bro obj_ptrs)
    if HAMMER_BRO_OBJ_PTRS.contains(&obj) {
        return NodeKind::HammerBro;
    }

    // 10. Non-level entries (out of bounds, special pointers)
    if row >= rom_data::ROWS || !rom_data::is_level_pointer(obj, lay) {
        return NodeKind::HammerBro;
    }

    // 11. Regular action level
    NodeKind::Level
}
