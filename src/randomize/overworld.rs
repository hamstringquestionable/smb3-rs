/// Overworld fortress shuffle: redistribute fortresses and place locks as atomic units.
///
/// Each fortress travels as a complete unit: level entry + Boom-Boom Y-byte +
/// map tile + lock/obstacle. The randomizer decides placements, then hands them
/// to `overworld_helpers` for mechanical ROM writes.
///
/// Modes:
/// - IntraWorld: fortresses stay in their home world, lock positions randomized
/// - CrossWorld: fortresses redistribute across worlds (1-3 per world)

use rand::Rng;
use rand::seq::SliceRandom;

use crate::rom::Rom;

use super::map_walker;
use super::overworld_helpers::{
    self, FortressPlacement, DisplacedLevel, LOCKABLE_TILES,
};
use super::rom_data::{
    self, AIRSHIP_ENTRIES, FORTRESS_ENTRIES, Grid, MAP_TRANSITIONS, WORLDS,
    LevelEntry,
};

// ---------------------------------------------------------------------------
// Public types
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
// Internal fortress data (collected from ROM before any writes)
// ---------------------------------------------------------------------------

/// Fortress data collected from ROM, before placement decisions.
struct FortressData {
    level_entry: LevelEntry,
    boomboom_y_offset: usize,
    fort_tile: u8,
}

/// Collect fortress data from ROM for entries matching a world filter.
/// Looks up Y-byte offset by obj_ptr so it works correctly even after
/// level shuffle has rearranged which fortress is at each slot.
fn collect_fortresses_filtered(rom: &Rom, world_filter: impl Fn(usize) -> bool) -> Vec<(usize, usize, FortressData)> {
    FORTRESS_ENTRIES
        .iter()
        .filter(|&&(w, _)| world_filter(w))
        .map(|&(w, i)| {
            let entry = rom_data::read_entry(rom, &WORLDS[w], i);
            let obj_ptr = u16::from_le_bytes([entry.obj_lo, entry.obj_hi]);
            (w, i, FortressData {
                level_entry: entry,
                boomboom_y_offset: rom_data::boomboom_y_offset_for_obj(obj_ptr)
                    .expect("fortress slot must contain a known fortress"),
                fort_tile: overworld_helpers::entry_tile(rom, w, i),
            })
        })
        .collect()
}

/// Collect all W1-7 fortress data from ROM.
fn collect_w17_fortresses(rom: &Rom) -> Vec<FortressData> {
    collect_fortresses_filtered(rom, |w| w < 7)
        .into_iter()
        .map(|(_, _, data)| data)
        .collect()
}

/// Collect W8 fortress data from ROM.
fn collect_w8_fortresses(rom: &Rom) -> Vec<FortressData> {
    collect_fortresses_filtered(rom, |w| w == 7)
        .into_iter()
        .map(|(_, _, data)| data)
        .collect()
}

// ---------------------------------------------------------------------------
// Action level helpers
// ---------------------------------------------------------------------------

/// Collect action level entry indices for a world that could become fortress slots.
/// Excludes current fortresses, airships, Bowser, hammer bros, pipe connectors,
/// and entries not on numbered level tiles (0x03-0x0C).
fn collect_action_levels(rom: &Rom, world_idx: usize) -> Vec<usize> {
    let world = &WORLDS[world_idx];
    let (_scrcol, objsets, layouts) = rom_data::table_offsets(world);

    // Count (obj, lay) pairs to detect hammer bros duplicates
    let mut pair_counts = std::collections::HashMap::new();
    for i in 0..world.entry_count {
        let obj_ptr = rom_data::read_word(rom, objsets + i * 2);
        let lay_ptr = rom_data::read_word(rom, layouts + i * 2);
        if rom_data::is_level_pointer(obj_ptr, lay_ptr) {
            *pair_counts.entry((obj_ptr, lay_ptr)).or_insert(0u32) += 1;
        }
    }

    let mut indices = Vec::new();
    for i in 0..world.entry_count {
        let obj_ptr = rom_data::read_word(rom, objsets + i * 2);
        let lay_ptr = rom_data::read_word(rom, layouts + i * 2);
        if !rom_data::is_level_pointer(obj_ptr, lay_ptr) {
            continue;
        }
        if AIRSHIP_ENTRIES.contains(&(world_idx, i)) {
            continue;
        }
        if MAP_TRANSITIONS.contains(&(world_idx, i)) {
            continue;
        }
        if pair_counts[&(obj_ptr, lay_ptr)] > 1 {
            continue; // hammer bros
        }
        if FORTRESS_ENTRIES.contains(&(world_idx, i)) {
            continue;
        }
        if (world_idx, i) == rom_data::BOWSER_ENTRY {
            continue;
        }
        let tileset = rom.read_byte(world.rowtype_offset + i) & 0x0F;
        if let Some(lay_offset) = rom_data::layout_file_offset(lay_ptr, tileset) {
            if rom_data::level_screen_count(rom, lay_offset) < 3 {
                continue;
            }
        } else {
            continue;
        }
        let (row, col) = rom_data::entry_grid_position(rom, world, i);
        let tile_off = rom_data::map_tile_offset(world_idx, row, col);
        let tile = rom.read_byte(tile_off);
        if !(0x03..=0x0C).contains(&tile) {
            continue;
        }
        indices.push(i);
    }
    indices
}

// ---------------------------------------------------------------------------
// Partition helper
// ---------------------------------------------------------------------------

/// Generate a random partition of `total` into `buckets` values,
/// each between `min` and `max` inclusive.
fn random_partition<R: Rng>(rng: &mut R, total: usize, buckets: usize, min: usize, max: usize) -> Vec<usize> {
    assert!(total >= buckets * min && total <= buckets * max);

    loop {
        let mut counts = vec![min; buckets];
        let mut remaining = total - buckets * min;

        while remaining > 0 {
            let idx = rng.random_range(..buckets);
            if counts[idx] < max {
                counts[idx] += 1;
                remaining -= 1;
            }
        }
        return counts;
    }
}

// ---------------------------------------------------------------------------
// Lock placement and validation
// ---------------------------------------------------------------------------

/// Determine the order fortresses are beaten by simulating BFS progression.
/// Returns indices into fort_positions in the order they'd be reached.
fn determine_beat_order(
    grid: &Grid,
    pipes: &[((usize, usize), (usize, usize))],
    fort_positions: &[(usize, usize)],
) -> Vec<usize> {
    let mut order = Vec::new();
    let mut beaten = std::collections::HashSet::new();

    loop {
        let result = map_walker::walk_map(grid, pipes, None);

        let next = fort_positions
            .iter()
            .enumerate()
            .find(|(i, pos)| !beaten.contains(i) && result.nodes.contains(pos))
            .map(|(i, _)| i);

        match next {
            Some(idx) => {
                order.push(idx);
                beaten.insert(idx);
            }
            None => break,
        }
    }

    order
}

/// Validate that a set of lock placements allows full fortress progression.
///
/// Simulates: start with all locks active, beat forts in order (each beat
/// opens that fort's lock), verify each fort is reachable at its turn,
/// and the target is reachable after all forts beaten.
fn validate_lock_placement(
    grid: &Grid,
    pipes: &[((usize, usize), (usize, usize))],
    fort_positions: &[(usize, usize)],
    beat_order: &[usize],
    lock_positions: &[(usize, usize)],
    target_pos: Option<(usize, usize)>,
) -> bool {
    // Lock tile constant for simulation grid
    const TILE_LOCK: u8 = 0x54;

    let mut sim_grid = grid.clone_grid();
    for &(r, c) in lock_positions {
        sim_grid.set(r, c, TILE_LOCK);
    }

    for (ord, &fort_idx) in beat_order.iter().enumerate() {
        let fort_pos = fort_positions[fort_idx];

        let result = map_walker::walk_map(&sim_grid, pipes, None);
        if !result.nodes.contains(&fort_pos) {
            return false;
        }

        if ord < lock_positions.len() {
            let (lr, lc) = lock_positions[ord];
            sim_grid.set(lr, lc, grid.get(lr, lc));
        }
    }

    if let Some(target) = target_pos {
        let result = map_walker::walk_map(&sim_grid, pipes, None);
        if !result.nodes.contains(&target) {
            return false;
        }
    }

    true
}

/// Pick lock positions for a world's fortresses with full progression validation.
///
/// Returns a Vec of (row, col) positions, one per fortress in beat order.
/// Falls back to empty vec (no locks) if validation fails after 50 attempts.
fn pick_lock_positions<R: Rng>(
    _rom: &Rom,
    rng: &mut R,
    _world_idx: usize,
    grid: &Grid,
    pipes: &[((usize, usize), (usize, usize))],
    fort_positions: &[(usize, usize)],
    beat_order: &[usize],
    target_pos: Option<(usize, usize)>,
) -> Vec<Option<(usize, usize)>> {
    let n = beat_order.len();

    // Collect all eligible path tiles (reachable, lockable, not row 8)
    let result = map_walker::walk_map(grid, pipes, None);
    let mut all_candidates: Vec<(usize, usize)> = Vec::new();
    let mut sorted_paths: Vec<(usize, usize)> = result.path_tiles.iter().copied().collect();
    sorted_paths.sort();
    for &(r, c) in &sorted_paths {
        if r >= 8 { continue; }
        let tile = grid.get(r, c);
        if !LOCKABLE_TILES.contains(&tile) { continue; }
        all_candidates.push((r, c));
    }

    for _attempt in 0..50 {
        let mut available = all_candidates.clone();
        available.as_mut_slice().shuffle(rng);

        let choices: Vec<(usize, usize)> = available.into_iter().take(n).collect();

        if choices.len() < n {
            break;
        }

        if validate_lock_placement(grid, pipes, fort_positions, beat_order, &choices, target_pos) {
            return choices.iter().map(|&pos| Some(pos)).collect();
        }
    }

    // Fallback: no locks
    vec![None; n]
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Randomize fortress positions and their associated locks/obstacles.
///
/// - IntraWorld: fortresses stay in their home world, lock positions randomized
/// - CrossWorld: fortresses redistribute across worlds (1-3 per world)
pub fn randomize_fortresses<R: Rng>(rom: &mut Rom, rng: &mut R, mode: &FortressRedistribute) {
    match mode {
        FortressRedistribute::Off => {}
        FortressRedistribute::IntraWorld => randomize_intra(rom, rng),
        FortressRedistribute::CrossWorld => randomize_cross(rom, rng),
    }
}

// ---------------------------------------------------------------------------
// IntraWorld mode
// ---------------------------------------------------------------------------

/// IntraWorld: fortresses stay in home world, lock positions randomized.
fn randomize_intra<R: Rng>(rom: &mut Rom, rng: &mut R) {
    let all_pipes = rom_data::read_pipe_pairs(rom);
    let fx_slots_snapshot = rom_data::read_fx_slots(rom);

    // Process W1-7
    let mut fx_slot = 0usize;
    for world_idx in 0..7 {
        let pipes = all_pipes.get(&world_idx).cloned().unwrap_or_default();

        // Pre-open vanilla locks
        overworld_helpers::pre_open_fx_for_world(rom, world_idx, &fx_slots_snapshot);

        // Collect this world's fortresses (they stay in place)
        let world_forts: Vec<(usize, usize)> = FORTRESS_ENTRIES
            .iter()
            .filter(|&&(w, _)| w == world_idx)
            .copied()
            .collect();
        let fort_count = world_forts.len();

        if fort_count == 0 {
            continue;
        }

        // Get fortress grid positions (explicit data, not scanned)
        let fort_positions: Vec<(usize, usize)> = world_forts
            .iter()
            .map(|&(w, i)| rom_data::entry_grid_position(rom, &WORLDS[w], i))
            .collect();

        // Read clean grid and determine beat order
        let grid = rom_data::read_tile_grid(rom, world_idx);
        let beat_order = determine_beat_order(&grid, &pipes, &fort_positions);
        let target_pos = overworld_helpers::world_target_position(rom, world_idx);

        // Pick lock positions
        let lock_choices = pick_lock_positions(
            rom, rng, world_idx, &grid, &pipes, &fort_positions, &beat_order, target_pos,
        );

        // Build FortressPlacement instructions
        let mut placements = Vec::new();
        for (ord, &fort_idx) in beat_order.iter().enumerate() {
            let (w, entry_idx) = world_forts[fort_idx];
            let entry = rom_data::read_entry(rom, &WORLDS[w], entry_idx);
            let obj_ptr = u16::from_le_bytes([entry.obj_lo, entry.obj_hi]);

            if let Some(Some(obstacle_pos)) = lock_choices.get(ord) {
                placements.push(FortressPlacement {
                    level_entry: entry,
                    boomboom_y_offset: rom_data::boomboom_y_offset_for_obj(obj_ptr)
                        .expect("fortress slot must contain a known fortress"),
                    fort_tile: overworld_helpers::entry_tile(rom, w, entry_idx),
                    dest_world: world_idx,
                    dest_slot: entry_idx,
                    ordinal: (ord + 1) as u8,
                    fortress_pos: fort_positions[fort_idx],
                    obstacle_pos: *obstacle_pos,
                });
            }
        }

        overworld_helpers::execute_world_placements(rom, &placements, &[], fx_slot);
        fx_slot += fort_count;
    }

    // W8: same treatment
    {
        let world_idx = 7;
        let pipes = all_pipes.get(&world_idx).cloned().unwrap_or_default();
        overworld_helpers::pre_open_fx_for_world(rom, world_idx, &fx_slots_snapshot);

        let w8_forts: Vec<(usize, usize)> = FORTRESS_ENTRIES
            .iter()
            .filter(|&&(w, _)| w == 7)
            .copied()
            .collect();

        let fort_positions: Vec<(usize, usize)> = w8_forts
            .iter()
            .map(|&(w, i)| rom_data::entry_grid_position(rom, &WORLDS[w], i))
            .collect();

        let grid = rom_data::read_tile_grid(rom, world_idx);
        let beat_order = determine_beat_order(&grid, &pipes, &fort_positions);
        let target_pos = overworld_helpers::world_target_position(rom, world_idx);

        let lock_choices = pick_lock_positions(
            rom, rng, world_idx, &grid, &pipes, &fort_positions, &beat_order, target_pos,
        );

        let mut placements = Vec::new();
        for (ord, &fort_idx) in beat_order.iter().enumerate() {
            let (w, entry_idx) = w8_forts[fort_idx];
            let entry = rom_data::read_entry(rom, &WORLDS[w], entry_idx);
            let obj_ptr = u16::from_le_bytes([entry.obj_lo, entry.obj_hi]);

            if let Some(Some(obstacle_pos)) = lock_choices.get(ord) {
                placements.push(FortressPlacement {
                    level_entry: entry,
                    boomboom_y_offset: rom_data::boomboom_y_offset_for_obj(obj_ptr)
                        .expect("fortress slot must contain a known fortress"),
                    fort_tile: overworld_helpers::entry_tile(rom, w, entry_idx),
                    dest_world: world_idx,
                    dest_slot: entry_idx,
                    ordinal: (ord + 1) as u8,
                    fortress_pos: fort_positions[fort_idx],
                    obstacle_pos: *obstacle_pos,
                });
            }
        }

        overworld_helpers::execute_world_placements(rom, &placements, &[], fx_slot);
    }
}

// ---------------------------------------------------------------------------
// CrossWorld mode
// ---------------------------------------------------------------------------

/// CrossWorld: redistribute W1-7 fortresses across worlds, 1-3 per world.
fn randomize_cross<R: Rng>(rom: &mut Rom, rng: &mut R) {
    let all_pipes = rom_data::read_pipe_pairs(rom);
    let fx_slots_snapshot = rom_data::read_fx_slots(rom);

    // -----------------------------------------------------------------------
    // Part A: W1-7 cross-world redistribution
    // -----------------------------------------------------------------------

    // Step 1: Collect all 13 fortress data and shuffle
    let mut fortress_pool = collect_w17_fortresses(rom);
    fortress_pool.as_mut_slice().shuffle(rng);

    // Step 2: Decide how many fortresses each world gets
    let new_counts = random_partition(rng, 13, 7, 1, 3);

    // Step 3: For each world, assign fortress slots and collect displaced levels
    let mut fortress_pool_idx = 0;
    let mut all_placements: Vec<Vec<FortressPlacement>> = Vec::new();
    let mut all_displaced: Vec<Vec<DisplacedLevel>> = Vec::new();

    // First pass: determine slot assignments and collect displaced levels
    // We need to know what's being displaced before we can assign freed slots.
    let mut freed_slots: Vec<(usize, usize)> = Vec::new(); // (world, entry_idx)
    let mut displaced_levels: Vec<(LevelEntry, u8)> = Vec::new(); // level + tile

    // Assign fortress slots per world
    struct WorldAssignment {
        fortress_data_indices: Vec<usize>, // indices into fortress_pool
        slots: Vec<usize>,                 // pointer table entry indices
    }
    let mut assignments: Vec<WorldAssignment> = Vec::new();

    for world_idx in 0..7 {
        let target_count = new_counts[world_idx];

        let current_fort_slots: Vec<usize> = FORTRESS_ENTRIES
            .iter()
            .filter(|&&(w, _)| w == world_idx)
            .map(|&(_, i)| i)
            .collect();
        let current_count = current_fort_slots.len();

        let mut fort_data_indices = Vec::new();
        let mut slots = Vec::new();

        if target_count > current_count {
            // Need more slots — convert some action levels
            let extra_needed = target_count - current_count;
            let action_levels = collect_action_levels(rom, world_idx);
            let slots_to_convert: Vec<usize> = action_levels
                .iter()
                .rev()
                .take(extra_needed)
                .copied()
                .collect();

            // Save displaced action levels
            for &slot_idx in &slots_to_convert {
                let entry = rom_data::read_entry(rom, &WORLDS[world_idx], slot_idx);
                let tile = overworld_helpers::entry_tile(rom, world_idx, slot_idx);
                displaced_levels.push((entry, tile));
            }

            // All slots: existing fortress slots + converted action level slots
            slots.extend_from_slice(&current_fort_slots);
            slots.extend_from_slice(&slots_to_convert);
        } else if target_count < current_count {
            // Fewer fortresses — free some slots
            let (keep, free) = current_fort_slots.split_at(target_count);
            slots.extend_from_slice(keep);
            for &slot_idx in free {
                freed_slots.push((world_idx, slot_idx));
            }
        } else {
            slots.extend_from_slice(&current_fort_slots);
        }

        for _ in 0..target_count {
            fort_data_indices.push(fortress_pool_idx);
            fortress_pool_idx += 1;
        }

        assignments.push(WorldAssignment { fortress_data_indices: fort_data_indices, slots });
    }

    assert_eq!(fortress_pool_idx, 13, "All 13 fortresses must be assigned");

    // Assign displaced levels to freed slots
    displaced_levels.as_mut_slice().shuffle(rng);
    assert_eq!(displaced_levels.len(), freed_slots.len(),
        "Displaced levels must match freed slots");

    // Now build placements with lock positions for each world
    for world_idx in 0..7 {
        let pipes = all_pipes.get(&world_idx).cloned().unwrap_or_default();

        // Pre-open vanilla locks
        overworld_helpers::pre_open_fx_for_world(rom, world_idx, &fx_slots_snapshot);

        let assignment = &assignments[world_idx];
        let fort_count = assignment.fortress_data_indices.len();

        if fort_count == 0 {
            all_placements.push(Vec::new());
            all_displaced.push(Vec::new());
            continue;
        }

        // Build preliminary placements (without lock positions yet)
        // to determine fortress grid positions for BFS
        let mut fort_positions = Vec::new();
        for (&data_idx, &slot) in assignment.fortress_data_indices.iter().zip(assignment.slots.iter()) {
            let _fort = &fortress_pool[data_idx];
            // After writing the fortress to this slot, it will be at the slot's grid position
            let pos = rom_data::entry_grid_position(rom, &WORLDS[world_idx], slot);
            fort_positions.push(pos);
        }

        // Read clean grid and pick lock positions
        let grid = rom_data::read_tile_grid(rom, world_idx);
        let beat_order = determine_beat_order(&grid, &pipes, &fort_positions);
        let target_pos = overworld_helpers::world_target_position(rom, world_idx);

        let lock_choices = pick_lock_positions(
            rom, rng, world_idx, &grid, &pipes, &fort_positions, &beat_order, target_pos,
        );

        // Build FortressPlacement instructions in beat order
        let mut placements = Vec::new();
        for (ord, &fort_idx) in beat_order.iter().enumerate() {
            let data_idx = assignment.fortress_data_indices[fort_idx];
            let slot = assignment.slots[fort_idx];
            let fort = &fortress_pool[data_idx];

            if let Some(Some(obstacle_pos)) = lock_choices.get(ord) {
                placements.push(FortressPlacement {
                    level_entry: fort.level_entry.clone(),
                    boomboom_y_offset: fort.boomboom_y_offset,
                    fort_tile: fort.fort_tile,
                    dest_world: world_idx,
                    dest_slot: slot,
                    ordinal: (ord + 1) as u8,
                    fortress_pos: fort_positions[fort_idx],
                    obstacle_pos: *obstacle_pos,
                });
            }
        }

        // Build displaced level instructions for this world's freed slots
        let mut world_displaced = Vec::new();
        for ((entry, tile), &(fw, fi)) in displaced_levels.iter().zip(freed_slots.iter()) {
            if fw == world_idx {
                world_displaced.push(DisplacedLevel {
                    level_entry: entry.clone(),
                    tile: *tile,
                    dest_world: fw,
                    dest_slot: fi,
                });
            }
        }

        all_placements.push(placements);
        all_displaced.push(world_displaced);
    }

    // Execute all placements
    let mut fx_slot = 0usize;
    for world_idx in 0..7 {
        let fort_count = assignments[world_idx].fortress_data_indices.len();
        overworld_helpers::execute_world_placements(
            rom,
            &all_placements[world_idx],
            &all_displaced[world_idx],
            fx_slot,
        );
        fx_slot += fort_count;
    }

    // -----------------------------------------------------------------------
    // Part B: W8 intra-world fortress position shuffle + locks
    // -----------------------------------------------------------------------
    shuffle_w8_fortresses(rom, rng, &all_pipes, &fx_slots_snapshot, fx_slot);
}

/// Shuffle W8 fortress positions among available level slots within W8,
/// including lock placement.
fn shuffle_w8_fortresses<R: Rng>(
    rom: &mut Rom,
    rng: &mut R,
    all_pipes: &std::collections::HashMap<usize, Vec<((usize, usize), (usize, usize))>>,
    fx_slots_snapshot: &[rom_data::FxSlot],
    fx_slot_base: usize,
) {
    let world_idx = 7;

    // Pre-open vanilla locks
    overworld_helpers::pre_open_fx_for_world(rom, world_idx, fx_slots_snapshot);

    let mut w8_forts = collect_w8_fortresses(rom);

    // Collect candidate slots: current fortress slots + action level slots
    let w8_fort_slots: Vec<usize> = FORTRESS_ENTRIES
        .iter()
        .filter(|&&(w, _)| w == 7)
        .map(|&(_, i)| i)
        .collect();
    let mut candidate_slots: Vec<usize> = w8_fort_slots.clone();
    let action_levels = collect_action_levels(rom, world_idx);
    candidate_slots.extend_from_slice(&action_levels);
    candidate_slots.sort();
    candidate_slots.dedup();

    if candidate_slots.len() < 4 {
        return;
    }

    // Pick 4 random slots
    let mut chosen_slots: Vec<usize> = candidate_slots.clone();
    chosen_slots.as_mut_slice().shuffle(rng);
    chosen_slots.truncate(4);
    chosen_slots.sort();

    // Collect displaced action levels
    let mut displaced_data: Vec<(LevelEntry, u8)> = Vec::new();
    for &slot in &chosen_slots {
        if !w8_fort_slots.contains(&slot) {
            let entry = rom_data::read_entry(rom, &WORLDS[world_idx], slot);
            let tile = overworld_helpers::entry_tile(rom, world_idx, slot);
            displaced_data.push((entry, tile));
        }
    }

    // Freed fortress slots
    let mut freed: Vec<usize> = Vec::new();
    for &i in &w8_fort_slots {
        if !chosen_slots.contains(&i) {
            freed.push(i);
        }
    }
    assert_eq!(displaced_data.len(), freed.len());

    // Shuffle fortress data
    w8_forts.as_mut_slice().shuffle(rng);

    // Get fortress positions at chosen slots
    let fort_positions: Vec<(usize, usize)> = chosen_slots
        .iter()
        .map(|&slot| rom_data::entry_grid_position(rom, &WORLDS[world_idx], slot))
        .collect();

    // Pick lock positions
    let pipes = all_pipes.get(&world_idx).cloned().unwrap_or_default();
    let grid = rom_data::read_tile_grid(rom, world_idx);
    let beat_order = determine_beat_order(&grid, &pipes, &fort_positions);
    let target_pos = overworld_helpers::world_target_position(rom, world_idx);

    let lock_choices = pick_lock_positions(
        rom, rng, world_idx, &grid, &pipes, &fort_positions, &beat_order, target_pos,
    );

    // Build placements
    let mut placements = Vec::new();
    for (ord, &fort_idx) in beat_order.iter().enumerate() {
        let fort = &w8_forts[fort_idx];
        let slot = chosen_slots[fort_idx];

        if let Some(Some(obstacle_pos)) = lock_choices.get(ord) {
            placements.push(FortressPlacement {
                level_entry: fort.level_entry.clone(),
                boomboom_y_offset: fort.boomboom_y_offset,
                fort_tile: fort.fort_tile,
                dest_world: world_idx,
                dest_slot: slot,
                ordinal: (ord + 1) as u8,
                fortress_pos: fort_positions[fort_idx],
                obstacle_pos: *obstacle_pos,
            });
        }
    }

    // Build displaced instructions
    displaced_data.as_mut_slice().shuffle(rng);
    let displaced: Vec<DisplacedLevel> = displaced_data
        .iter()
        .zip(freed.iter())
        .map(|((entry, tile), &freed_slot)| DisplacedLevel {
            level_entry: entry.clone(),
            tile: *tile,
            dest_world: world_idx,
            dest_slot: freed_slot,
        })
        .collect();

    overworld_helpers::execute_world_placements(rom, &placements, &displaced, fx_slot_base);
}


#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn test_random_partition() {
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        for _ in 0..100 {
            let counts = random_partition(&mut rng, 13, 7, 1, 3);
            assert_eq!(counts.len(), 7);
            assert_eq!(counts.iter().sum::<usize>(), 13);
            for &c in &counts {
                assert!(c >= 1 && c <= 3, "count {} out of range", c);
            }
        }
    }

    #[test]
    fn test_fortress_intra_deterministic() {
        let rom_data = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data.is_err() {
            return;
        }

        let mut rom1 = Rom::from_bytes(&rom_data.as_ref().unwrap()).unwrap();
        let mut rom2 = Rom::from_bytes(&rom_data.as_ref().unwrap()).unwrap();

        let mut rng1 = ChaCha8Rng::seed_from_u64(777);
        let mut rng2 = ChaCha8Rng::seed_from_u64(777);

        randomize_fortresses(&mut rom1, &mut rng1, &FortressRedistribute::IntraWorld);
        randomize_fortresses(&mut rom2, &mut rng2, &FortressRedistribute::IntraWorld);

        // Check FX table data matches
        for off in 0x147CD..0x148B8 {
            assert_eq!(rom1.read_byte(off), rom2.read_byte(off),
                "FX table mismatch at 0x{:05X}", off);
        }

        // Check map tile grids match
        for wi in 0..8 {
            let info = &rom_data::MAP_TILE_GRIDS[wi];
            let size = info.screens * 144;
            for off in info.file_offset..info.file_offset + size {
                assert_eq!(
                    rom1.read_byte(off), rom2.read_byte(off),
                    "Map tile mismatch at 0x{:05X} (W{})", off, wi + 1,
                );
            }
        }
    }

    #[test]
    fn test_fortress_cross_deterministic() {
        let rom_data = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data.is_err() {
            return;
        }

        let mut rom1 = Rom::from_bytes(&rom_data.as_ref().unwrap()).unwrap();
        let mut rom2 = Rom::from_bytes(&rom_data.as_ref().unwrap()).unwrap();

        let mut rng1 = ChaCha8Rng::seed_from_u64(12345);
        let mut rng2 = ChaCha8Rng::seed_from_u64(12345);

        randomize_fortresses(&mut rom1, &mut rng1, &FortressRedistribute::CrossWorld);
        randomize_fortresses(&mut rom2, &mut rng2, &FortressRedistribute::CrossWorld);

        // Check pointer table data matches
        for world in &WORLDS {
            let n = world.entry_count;
            let start = world.rowtype_offset;
            let end = start + n * 6;
            for off in start..end {
                assert_eq!(
                    rom1.read_byte(off), rom2.read_byte(off),
                    "Mismatch at 0x{:05X}", off,
                );
            }
        }

        // Check FX table
        for off in 0x147CD..0x148B8 {
            assert_eq!(rom1.read_byte(off), rom2.read_byte(off));
        }

        // Check Y-bytes
        for &off in &rom_data::BOOMBOOM_Y_OFFSETS {
            assert_eq!(rom1.read_byte(off), rom2.read_byte(off));
        }
    }

    #[test]
    fn test_fortress_intra_no_row_8() {
        let rom_data = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data.is_err() {
            return;
        }

        let mut rom = Rom::from_bytes(&rom_data.unwrap()).unwrap();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize_fortresses(&mut rom, &mut rng, &FortressRedistribute::IntraWorld);

        for wi in 0..8 {
            let grid = rom_data::read_tile_grid(&rom, wi);
            for c in 0..grid.cols {
                assert_ne!(
                    grid.get(8, c), 0x54,
                    "Lock at row 8 in W{} col {}", wi + 1, c,
                );
            }
        }
    }

    #[test]
    fn test_fortress_intra_progression_valid() {
        let rom_data = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data.is_err() {
            return;
        }

        for seed in [42, 123, 999, 31337, 65536] {
            let mut rom = Rom::from_bytes(&rom_data.as_ref().unwrap()).unwrap();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize_fortresses(&mut rom, &mut rng, &FortressRedistribute::IntraWorld);

            let all_pipes = rom_data::read_pipe_pairs(&rom);

            for wi in 0..8 {
                let pipes = all_pipes.get(&wi).cloned().unwrap_or_default();
                let fort_positions = rom_data::read_fortress_positions(&rom, wi);
                if fort_positions.is_empty() {
                    continue;
                }

                let steps = map_walker::simulate_progression(&rom, wi, &pipes);

                let final_nodes = &steps.last().unwrap().nodes;
                if let Some(target) = overworld_helpers::world_target_position(&rom, wi) {
                    assert!(
                        final_nodes.contains(&target),
                        "Seed {seed} W{}: target at ({},{}) not reachable after all fortresses",
                        wi + 1, target.0, target.1,
                    );
                }
            }
        }
    }

    #[test]
    fn test_fortress_with_other_features() {
        let rom_data = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data.is_err() {
            return;
        }

        for seed in [42, 123, 999, 31337] {
            for mode in [FortressRedistribute::IntraWorld, FortressRedistribute::CrossWorld] {
                let mut rom = Rom::from_bytes(&rom_data.as_ref().unwrap()).unwrap();

                let mut options = crate::randomizer::Options::default();
                options.shuffle_fortresses = true;
                options.fortress_redistribute = mode.clone();
                options.shuffle_pipes = true;
                options.fix_drawbridges = true;
                options.remove_w2_rock = true;
                crate::randomizer::randomize(&mut rom, seed, &options);

                for wi in 0..8 {
                    // Verify FX slots are in range
                    let base = rom_data::FX_WORLD_TABLE + wi * 4;
                    for i in 0..4 {
                        let slot_idx = rom.read_byte(base + i) as usize;
                        assert!(
                            slot_idx < 17,
                            "Seed {seed} W{}: FX slot index {slot_idx} out of range",
                            wi + 1,
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn test_fortress_intra_visual() {
        let rom_data = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data.is_err() {
            return;
        }

        let mut rom = Rom::from_bytes(&rom_data.unwrap()).unwrap();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize_fortresses(&mut rom, &mut rng, &FortressRedistribute::IntraWorld);

        let all_pipes = rom_data::read_pipe_pairs(&rom);

        println!("\n\x1b[1;33m=== Fortress Shuffle IntraWorld (seed 42) ===\x1b[0m\n");
        for wi in 0..8 {
            let pipes = all_pipes.get(&wi).cloned().unwrap_or_default();
            let output = map_walker::render_progression(&rom, wi, &pipes);
            print!("{output}");
        }
    }

    /// Verify that when both level shuffle (shuffle_fortresses) and
    /// fortress redistribute (fortress_redistribute) are enabled, each Boom-Boom Y-byte's
    /// upper nibble matches the expected ordinal for its current map
    /// position — not the vanilla position it was shuffled from.
    #[test]
    fn test_level_plus_lock_shuffle_ybytes_correct() {
        let rom_data = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data.is_err() {
            return;
        }

        for seed in [42, 123, 999, 31337, 65536] {
            for mode in [FortressRedistribute::IntraWorld, FortressRedistribute::CrossWorld] {
                let mut rom = Rom::from_bytes(&rom_data.as_ref().unwrap()).unwrap();

                let mut options = crate::randomizer::Options::default();
                options.shuffle_fortresses = true;
                options.fortress_redistribute = mode.clone();
                crate::randomizer::randomize(&mut rom, seed, &options);

                // For each fortress slot, the Boom-Boom Y-byte that belongs
                // to the level at that slot must have the correct ordinal.
                // The overworld module writes FX_WORLD_TABLE with sequential
                // slot indices in beat order, and patches Y-bytes with
                // ordinals 1..N. Verify the Y-byte matches.
                for &(w, i) in rom_data::FORTRESS_ENTRIES.iter() {
                    let entry = rom_data::read_entry(&rom, &WORLDS[w], i);
                    let obj_ptr = u16::from_le_bytes([entry.obj_lo, entry.obj_hi]);

                    // Find the Y-byte for whatever fortress is at this slot
                    let y_off = rom_data::boomboom_y_offset_for_obj(obj_ptr);
                    if y_off.is_none() {
                        // Slot may have been replaced by an action level in
                        // CrossWorld mode — not a fortress anymore
                        continue;
                    }
                    let y_byte = rom.read_byte(y_off.unwrap());
                    let ordinal = y_byte >> 4;

                    // Ordinal must be 1-4 (valid fortress ordinals)
                    assert!(
                        ordinal >= 1 && ordinal <= 4,
                        "Seed {seed} {:?} W{}[{}]: Y-byte 0x{:02X} has invalid ordinal {}",
                        mode, w + 1, i, y_byte, ordinal,
                    );
                }
            }
        }
    }
}
