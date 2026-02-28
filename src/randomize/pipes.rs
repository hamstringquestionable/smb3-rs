/// Pipe shuffle: randomize overworld pipe endpoint positions.
///
/// Progressive placement algorithm:
/// 1. Remove all pipe tiles from the overworld grid
/// 2. Walk (BFS) to find reachable area
/// 3. Place pipe pairs one at a time, connecting reachable ↔ unreachable areas
/// 4. Prioritize must-reach positions (airships, Bowser's castle)
/// 5. Patch ROM: swap pointer table entries, update destination tables, re-sort
///
/// See `tools/pipe_shuffle.py` for the original Python prototype.

use std::collections::{HashMap, HashSet};

use rand::Rng;
use rand::seq::SliceRandom;

use crate::rom::Rom;

use super::map_walker;
use super::pipe_helpers;
use super::rom_data::{
    self, AIRSHIP_ENTRIES, BOWSER_ENTRY, FORTRESS_ENTRIES, Grid,
    MAP_TRANSITIONS, PIPE_MAP_XHI, PIPE_MAP_X, PIPE_MAP_Y, ROWS, TILE_PIPE,
    WORLDS,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Junction tile used when removing pipe tiles from the grid.
const TILE_REPLACEMENT: u8 = 0x47;

/// W5 Spiral Tower entries (functionally a pipe pair using dest index 0).
const W5_SPIRAL_ENTRIES: &[(usize, usize)] = &[(4, 10), (4, 21)];

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

type Pos = (usize, usize);

/// A pointer table entry with position and level data.
#[derive(Clone, Debug)]
struct PipeEntry {
    index: usize,
    grid_row: usize,
    grid_col: usize,
    obj_ptr: u16,
    lay_ptr: u16,
    tileset: u8,
}

/// Entry classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EntryType {
    Fortress,
    Airship,
    Pipe,
    Level,
    Toad,
    Bonus,
    Transition,
    Other,
}

/// A map position eligible for pipe placement.
struct SwappablePos {
    grid_row: usize,
    grid_col: usize,
}

// ---------------------------------------------------------------------------
// Entry reading and classification
// ---------------------------------------------------------------------------

/// Read all pointer table entries for a world.
fn read_all_entries(rom: &Rom, world_idx: usize) -> Vec<PipeEntry> {
    let world = &WORLDS[world_idx];
    let (sc, obj, lay) = rom_data::table_offsets(world);
    let n = world.entry_count;

    let mut entries = Vec::with_capacity(n);
    for i in 0..n {
        let rowtype = rom.read_byte(world.rowtype_offset + i);
        let scrcol = rom.read_byte(sc + i);
        let row_nib = (rowtype >> 4) & 0x0F;
        let tileset = rowtype & 0x0F;
        let screen = (scrcol >> 4) & 0x0F;
        let col = scrcol & 0x0F;
        let obj_ptr = rom_data::read_word(rom, obj + i * 2);
        let lay_ptr = rom_data::read_word(rom, lay + i * 2);

        entries.push(PipeEntry {
            index: i,
            grid_row: (row_nib as usize).wrapping_sub(2),
            grid_col: screen as usize * 16 + col as usize,
            obj_ptr,
            lay_ptr,
            tileset,
        });
    }
    entries
}

/// Classify a pointer table entry.
fn classify_entry(world_idx: usize, entry: &PipeEntry) -> EntryType {
    let i = entry.index;

    if FORTRESS_ENTRIES.contains(&(world_idx, i)) {
        return EntryType::Fortress;
    }
    if AIRSHIP_ENTRIES.contains(&(world_idx, i)) {
        return EntryType::Airship;
    }
    if MAP_TRANSITIONS.contains(&(world_idx, i)) {
        return EntryType::Transition;
    }
    if entry.obj_ptr == 0x0700 {
        return EntryType::Toad;
    }
    if entry.obj_ptr == 0x0001 && entry.lay_ptr == 0x0000 {
        return EntryType::Bonus;
    }
    if W5_SPIRAL_ENTRIES.contains(&(world_idx, i)) {
        return EntryType::Pipe;
    }
    if entry.tileset == 14 {
        return EntryType::Pipe;
    }
    if entry.obj_ptr >= 0xC000 && entry.lay_ptr != 0x0000 {
        return EntryType::Level;
    }
    EntryType::Other
}

/// Get paired pipe entries for a world (entries sharing the same obj_ptr).
fn get_pipe_pairs(rom: &Rom, world_idx: usize) -> Vec<(PipeEntry, PipeEntry)> {
    let entries = read_all_entries(rom, world_idx);
    let pipe_entries: Vec<PipeEntry> = entries
        .into_iter()
        .filter(|e| classify_entry(world_idx, e) == EntryType::Pipe)
        .collect();

    // Group by obj_ptr
    let mut by_obj: HashMap<u16, Vec<PipeEntry>> = HashMap::new();
    for e in pipe_entries {
        by_obj.entry(e.obj_ptr).or_default().push(e);
    }

    let mut pairs = Vec::new();
    let mut unpaired: Vec<PipeEntry> = Vec::new();
    let mut keys: Vec<u16> = by_obj.keys().copied().collect();
    keys.sort();
    for key in keys {
        let group = by_obj.remove(&key).unwrap();
        if group.len() == 2 {
            let mut it = group.into_iter();
            pairs.push((it.next().unwrap(), it.next().unwrap()));
        } else {
            unpaired.extend(group);
        }
    }

    // Pair W5 spiral tower entries (different obj_ptrs, same dest mechanism).
    if world_idx == 4 {
        let mut spiral: Vec<PipeEntry> = unpaired
            .into_iter()
            .filter(|e| W5_SPIRAL_ENTRIES.contains(&(world_idx, e.index)))
            .collect();
        if spiral.len() == 2 {
            spiral.sort_by_key(|e| e.index);
            let mut it = spiral.into_iter();
            pairs.push((it.next().unwrap(), it.next().unwrap()));
        }
    }

    pairs
}

/// Get all positions eligible for pipe placement swaps.
fn get_swappable_positions(rom: &Rom, world_idx: usize, start_pos: Option<Pos>) -> Vec<SwappablePos> {
    let entries = read_all_entries(rom, world_idx);

    // Detect hammer bros (duplicate obj+lay pairs)
    let mut pair_counts: HashMap<(u16, u16), u32> = HashMap::new();
    for e in &entries {
        if e.obj_ptr >= 0xC000 && e.lay_ptr != 0x0000 {
            *pair_counts.entry((e.obj_ptr, e.lay_ptr)).or_insert(0) += 1;
        }
    }
    let hammer_pairs: HashSet<(u16, u16)> = pair_counts
        .into_iter()
        .filter(|&(_, v)| v > 1)
        .map(|(k, _)| k)
        .collect();

    // Collect map-object-linked entries (e.g. W7 piranha plants) for this world
    let map_obj_entries: HashSet<usize> = rom_data::MAP_OBJ_ENTRY_LINKS
        .iter()
        .filter(|&&(w, _, _)| w == world_idx)
        .map(|&(_, _, entry_idx)| entry_idx)
        .collect();

    let mut positions = Vec::new();
    for e in &entries {
        let etype = classify_entry(world_idx, e);
        if matches!(etype, EntryType::Airship | EntryType::Transition | EntryType::Bonus | EntryType::Other) {
            continue;
        }
        if hammer_pairs.contains(&(e.obj_ptr, e.lay_ptr)) {
            continue;
        }
        if map_obj_entries.contains(&e.index) {
            continue;
        }
        if e.grid_row >= ROWS {
            continue;
        }
        // Never place a pipe on the START tile
        if let Some(sp) = start_pos {
            if (e.grid_row, e.grid_col) == sp {
                continue;
            }
        }

        positions.push(SwappablePos {
            grid_row: e.grid_row,
            grid_col: e.grid_col,
        });
    }
    positions
}

/// Get positions that MUST be reachable: airships and Bowser's castle.
fn get_must_reach(rom: &Rom, world_idx: usize) -> HashSet<Pos> {
    let entries = read_all_entries(rom, world_idx);
    let mut must_reach = HashSet::new();

    for e in &entries {
        let key = (world_idx, e.index);
        if AIRSHIP_ENTRIES.contains(&key) || key == BOWSER_ENTRY {
            if e.grid_row < ROWS {
                must_reach.insert((e.grid_row, e.grid_col));
            }
        }
    }
    must_reach
}

// ---------------------------------------------------------------------------
// Gap opening
// ---------------------------------------------------------------------------

/// Open fortress-gated gaps in the grid using the FX table (simulate post-fortress state).
///
/// Reads the FX slots assigned to this world and replaces only the tiles that
/// are actually wired to fortress completion effects, using each slot's stored
/// replacement tile. This is more precise than scanning for tile IDs, which
/// could false-positive on decorative uses of the same tile values.
fn open_gaps(rom: &Rom, world_idx: usize, grid: &mut Grid) {
    let fx_slots = rom_data::read_fx_slots(rom);
    let fx_assignments = rom_data::read_world_fx_assignments(rom);
    let world_fx = &fx_assignments[world_idx];

    for &slot_idx in world_fx {
        let slot_idx = slot_idx as usize;
        if slot_idx >= fx_slots.len() {
            continue;
        }
        let slot = &fx_slots[slot_idx];
        if slot.grid_row < grid.rows && slot.grid_col < grid.cols {
            grid.set(slot.grid_row, slot.grid_col, slot.replace_tile);
        }
    }
}

// ---------------------------------------------------------------------------
// Progressive pipe placement
// ---------------------------------------------------------------------------

/// Find connected components among unreachable nodes (using BFS without pipes).
fn find_unreachable_components(
    grid: &Grid,
    reachable: &HashSet<Pos>,
    all_nodes: &HashSet<Pos>,
) -> Vec<HashSet<Pos>> {
    let unreachable: HashSet<Pos> = all_nodes.difference(reachable).copied().collect();
    if unreachable.is_empty() {
        return Vec::new();
    }

    let mut visited: HashSet<Pos> = HashSet::new();
    let mut components = Vec::new();

    for &start in &unreachable {
        if visited.contains(&start) {
            continue;
        }
        // BFS from this node using only grid paths (no pipes)
        let result = map_walker::walk_map(grid, &[], Some(start));
        let component: HashSet<Pos> = result.nodes.intersection(all_nodes).copied().collect();
        visited.extend(&component);
        components.push(component);
    }

    components
}

/// Core pipe placement algorithm.
///
/// Removes pipes, walks the grid, and progressively places pipe pairs
/// to connect disconnected areas. Prioritizes components containing
/// must-reach positions (airships, Bowser).
///
/// Returns the modified grid and placed pipe pair positions.
fn place_pipes_progressive<R: Rng>(
    rom: &Rom,
    world_idx: usize,
    rng: &mut R,
) -> (Grid, Vec<(Pos, Pos)>) {
    let mut grid = rom_data::read_tile_grid(rom, world_idx);
    let pipe_pairs_data = get_pipe_pairs(rom, world_idx);
    let start_pos = rom_data::find_start(&grid);
    let positions = get_swappable_positions(rom, world_idx, start_pos);
    let dest_indices = rom_data::dest_indices_for_world(world_idx);

    if pipe_pairs_data.is_empty() || dest_indices.is_empty() {
        return (grid, Vec::new());
    }

    // Step 0: Open fortress-gated gaps using the FX table
    open_gaps(rom, world_idx, &mut grid);

    // Step 1: Remove all pipe/spiral tiles from grid
    for pa_pb in &pipe_pairs_data {
        for p in [&pa_pb.0, &pa_pb.1] {
            grid.set(p.grid_row, p.grid_col, TILE_REPLACEMENT);
        }
    }

    // Collect all node positions
    let all_nodes: HashSet<Pos> = positions.iter().map(|p| (p.grid_row, p.grid_col)).collect();

    // Get must-reach positions
    let must_reach = get_must_reach(rom, world_idx);

    // Step 2: Walk with no pipes
    let result = map_walker::walk_map(&grid, &[], None);
    let mut reachable = result.nodes.clone();

    // Step 3: Progressively place pipe pairs
    let mut placed_pairs = Vec::new();
    let mut remaining: Vec<usize> = (0..pipe_pairs_data.len()).collect();
    remaining.as_mut_slice().shuffle(rng);

    let mut used_positions: HashSet<Pos> = HashSet::new();

    for _pair_idx in remaining {
        let available = &all_nodes - &used_positions;
        let unreachable_nodes: HashSet<Pos> = &available - &reachable;
        let reachable_available: HashSet<Pos> = &available & &reachable;

        if unreachable_nodes.is_empty() {
            // All reachable — place randomly
            let mut candidates: Vec<Pos> = reachable_available.into_iter().collect();
            candidates.sort();
            if candidates.len() >= 2 {
                candidates.as_mut_slice().shuffle(rng);
                let a_pos = candidates[0];
                let b_pos = candidates[1];
                placed_pairs.push((a_pos, b_pos));
                used_positions.insert(a_pos);
                used_positions.insert(b_pos);
                grid.set(a_pos.0, a_pos.1, TILE_PIPE);
                grid.set(b_pos.0, b_pos.1, TILE_PIPE);
            }
            continue;
        }

        // Prioritize must-reach components
        let unreachable_must = &must_reach - &reachable;
        let unreachable_candidates: Vec<Pos> = if !unreachable_must.is_empty() {
            let components = find_unreachable_components(&grid, &reachable, &all_nodes);
            let mut priority = HashSet::new();
            for comp in &components {
                if !comp.is_disjoint(&unreachable_must) {
                    priority.extend(comp.intersection(&unreachable_nodes));
                }
            }
            if !priority.is_empty() {
                let mut v: Vec<Pos> = priority.into_iter().collect();
                v.sort();
                v
            } else {
                let mut v: Vec<Pos> = unreachable_nodes.into_iter().collect();
                v.sort();
                v
            }
        } else {
            let mut v: Vec<Pos> = unreachable_nodes.into_iter().collect();
            v.sort();
            v
        };

        let mut reachable_candidates: Vec<Pos> = reachable_available.into_iter().collect();
        reachable_candidates.sort();
        let mut unreachable_cands = unreachable_candidates;

        if reachable_candidates.is_empty() {
            break;
        }

        reachable_candidates.as_mut_slice().shuffle(rng);
        unreachable_cands.as_mut_slice().shuffle(rng);

        let a_pos = reachable_candidates[0];
        let b_pos = unreachable_cands[0];

        placed_pairs.push((a_pos, b_pos));
        used_positions.insert(a_pos);
        used_positions.insert(b_pos);
        grid.set(a_pos.0, a_pos.1, TILE_PIPE);
        grid.set(b_pos.0, b_pos.1, TILE_PIPE);

        // Re-walk with new pipe pair
        let result = map_walker::walk_map(&grid, &placed_pairs, None);
        reachable = result.nodes;
    }

    (grid, placed_pairs)
}

// ---------------------------------------------------------------------------
// ROM patching
// ---------------------------------------------------------------------------

/// Match pipe pair entries to destination table indices by comparing positions.
fn match_pairs_to_dests(
    rom: &Rom,
    world_idx: usize,
    pipe_pairs_data: &[(PipeEntry, PipeEntry)],
) -> Vec<(usize, usize, usize)> {
    // Returns Vec of (dest_idx, pair_a_entry_index, pair_b_entry_index)
    // where A = upper nibble endpoint, B = lower nibble endpoint.
    let dests = rom_data::dest_indices_for_world(world_idx);
    let mut matches = Vec::new();

    for d in &dests {
        let xhi = rom.read_byte(PIPE_MAP_XHI + d);
        let x = rom.read_byte(PIPE_MAP_X + d);
        let y = rom.read_byte(PIPE_MAP_Y + d);

        let a_pos: Pos = (
            ((y >> 4) as usize).wrapping_sub(2),
            ((xhi >> 4) as usize) * 16 + ((x >> 4) as usize),
        );
        let b_pos: Pos = (
            ((y & 0xF) as usize).wrapping_sub(2),
            ((xhi & 0xF) as usize) * 16 + ((x & 0xF) as usize),
        );

        for (ea, eb) in pipe_pairs_data {
            let ea_pos = (ea.grid_row, ea.grid_col);
            let eb_pos = (eb.grid_row, eb.grid_col);
            if ea_pos == a_pos && eb_pos == b_pos {
                matches.push((*d, ea.index, eb.index));
                break;
            } else if ea_pos == b_pos && eb_pos == a_pos {
                matches.push((*d, eb.index, ea.index));
                break;
            }
        }
    }

    matches
}

/// Apply pipe shuffle to ROM: swap entries, update dest tables, re-sort.
fn apply_pipe_shuffle(
    rom: &mut Rom,
    world_idx: usize,
    pipe_pairs_data: &[(PipeEntry, PipeEntry)],
    placed_pairs: &[(Pos, Pos)],
) {
    let world = &WORLDS[world_idx];
    let n = world.entry_count;
    let rt = world.rowtype_offset;
    let sc = rt + n;

    // Match original pairs to dest indices
    let dest_matches = match_pairs_to_dests(rom, world_idx, pipe_pairs_data);

    if dest_matches.len() != placed_pairs.len() {
        return; // Safety: don't patch if mismatch
    }

    // Build live position → entry index lookup
    let mut pos_to_entry: HashMap<Pos, usize> = HashMap::new();
    for i in 0..n {
        let rowtype = rom.read_byte(rt + i);
        let scrcol = rom.read_byte(sc + i);
        let row_nib = (rowtype >> 4) & 0x0F;
        let screen = (scrcol >> 4) & 0x0F;
        let col = scrcol & 0x0F;
        let grid_row = (row_nib as usize).wrapping_sub(2);
        let grid_col = screen as usize * 16 + col as usize;
        pos_to_entry.insert((grid_row, grid_col), i);
    }

    for ((dest_idx, pipe_a_idx, pipe_b_idx), &(new_a_pos, new_b_pos)) in
        dest_matches.iter().zip(placed_pairs.iter())
    {
        let pipe_a_idx = *pipe_a_idx;
        let pipe_b_idx = *pipe_b_idx;

        // Find current position of pipe A
        let cur_a_rt = rom.read_byte(rt + pipe_a_idx);
        let cur_a_sc = rom.read_byte(sc + pipe_a_idx);
        let cur_a_row = ((cur_a_rt >> 4) as usize).wrapping_sub(2);
        let cur_a_col =
            ((cur_a_sc >> 4) as usize & 0x0F) * 16 + (cur_a_sc as usize & 0x0F);
        let cur_a_pos = (cur_a_row, cur_a_col);

        // Swap pipe A to new position
        if cur_a_pos != new_a_pos {
            if let Some(&target_idx) = pos_to_entry.get(&new_a_pos) {
                pipe_helpers::swap_entry_positions(rom, world_idx, pipe_a_idx, target_idx);
                pos_to_entry.insert(new_a_pos, pipe_a_idx);
                pos_to_entry.insert(cur_a_pos, target_idx);
            }
        }

        // Find current position of pipe B
        let cur_b_rt = rom.read_byte(rt + pipe_b_idx);
        let cur_b_sc = rom.read_byte(sc + pipe_b_idx);
        let cur_b_row = ((cur_b_rt >> 4) as usize).wrapping_sub(2);
        let cur_b_col =
            ((cur_b_sc >> 4) as usize & 0x0F) * 16 + (cur_b_sc as usize & 0x0F);
        let cur_b_pos = (cur_b_row, cur_b_col);

        // Swap pipe B to new position
        if cur_b_pos != new_b_pos {
            if let Some(&target_idx) = pos_to_entry.get(&new_b_pos) {
                pipe_helpers::swap_entry_positions(rom, world_idx, pipe_b_idx, target_idx);
                pos_to_entry.insert(new_b_pos, pipe_b_idx);
                pos_to_entry.insert(cur_b_pos, target_idx);
            }
        }

        // Update pipe destination tables
        pipe_helpers::write_pipe_dest(rom, *dest_idx, new_a_pos, new_b_pos);
    }

    // Re-sort the entire pointer table
    pipe_helpers::resort_pointer_table(rom, world_idx);
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Shuffle pipe endpoint positions across all worlds.
pub fn randomize<R: Rng>(rom: &mut Rom, rng: &mut R) {
    for world_idx in 0..8 {
        let pipe_pairs_data = get_pipe_pairs(rom, world_idx);
        if pipe_pairs_data.is_empty() {
            continue;
        }

        let (_grid, placed_pairs) = place_pipes_progressive(rom, world_idx, rng);

        if !placed_pairs.is_empty() {
            apply_pipe_shuffle(rom, world_idx, &pipe_pairs_data, &placed_pairs);
            // Sync floating overworld sprites (W7 piranha plants) to their
            // new pointer table positions after entry swaps.
            rom_data::sync_map_object_positions(rom, world_idx);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    fn load_rom() -> Option<Rom> {
        let data = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&data).ok()
    }

    #[test]
    fn test_pipe_pairs_detected() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };

        // Expected pipe pair counts per world (W5 = 2: 1 regular + 1 spiral tower)
        let expected = [0, 1, 3, 2, 2, 2, 8, 6];
        for (wi, &expected_count) in expected.iter().enumerate() {
            let pairs = get_pipe_pairs(&rom, wi);
            assert_eq!(
                pairs.len(),
                expected_count,
                "World {} pipe pairs: expected {}, got {}",
                wi + 1,
                expected_count,
                pairs.len()
            );
        }
    }

    #[test]
    fn test_deterministic() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };

        let mut rom1 = rom.clone();
        let mut rom2 = rom.clone();

        let mut rng1 = ChaCha8Rng::seed_from_u64(42);
        let mut rng2 = ChaCha8Rng::seed_from_u64(42);

        randomize(&mut rom1, &mut rng1);
        randomize(&mut rom2, &mut rng2);

        // Check all pointer table data matches
        for world in &WORLDS {
            let n = world.entry_count;
            let start = world.rowtype_offset;
            let end = start + n * 6;
            for off in start..end {
                assert_eq!(
                    rom1.read_byte(off),
                    rom2.read_byte(off),
                    "Mismatch at 0x{:05X}",
                    off,
                );
            }
        }

        // Check dest tables match
        for off in PIPE_MAP_XHI..PIPE_MAP_XHI + 24 {
            assert_eq!(rom1.read_byte(off), rom2.read_byte(off));
        }
        for off in PIPE_MAP_X..PIPE_MAP_X + 24 {
            assert_eq!(rom1.read_byte(off), rom2.read_byte(off));
        }
        for off in PIPE_MAP_Y..PIPE_MAP_Y + 24 {
            assert_eq!(rom1.read_byte(off), rom2.read_byte(off));
        }
    }

    #[test]
    fn test_must_reach_satisfied() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };

        for seed in [42u64, 1, 99, 777] {
            let mut test_rom = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);

            // Run pipe shuffle, then verify must-reach positions
            for world_idx in 0..8 {
                let pipe_pairs_data = get_pipe_pairs(&test_rom, world_idx);
                if pipe_pairs_data.is_empty() {
                    continue;
                }

                let (grid, placed_pairs) = place_pipes_progressive(&test_rom, world_idx, &mut rng);

                let must_reach = get_must_reach(&test_rom, world_idx);
                if must_reach.is_empty() {
                    continue;
                }

                let result = map_walker::walk_map(&grid, &placed_pairs, None);
                let unreachable_must: HashSet<Pos> =
                    must_reach.difference(&result.nodes).copied().collect();

                assert!(
                    unreachable_must.is_empty(),
                    "Seed {}: World {} has unreachable must-reach: {:?}",
                    seed,
                    world_idx + 1,
                    unreachable_must,
                );

                if !placed_pairs.is_empty() {
                    apply_pipe_shuffle(
                        &mut test_rom,
                        world_idx,
                        &pipe_pairs_data,
                        &placed_pairs,
                    );
                }
            }
        }
    }

    #[test]
    fn test_resort_preserves_data() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };

        // For each world, resort and verify all entry data is preserved (just reordered)
        for world_idx in 0..8 {
            let mut test_rom = rom.clone();
            let world = &WORLDS[world_idx];
            let n = world.entry_count;
            let rt = world.rowtype_offset;
            let sc = rt + n;
            let obj = sc + n;
            let lay = obj + n * 2;

            // Collect original entries as sets of (rowtype, scrcol, obj_word, lay_word)
            let mut original: Vec<(u8, u8, u16, u16)> = (0..n)
                .map(|i| {
                    (
                        test_rom.read_byte(rt + i),
                        test_rom.read_byte(sc + i),
                        rom_data::read_word(&test_rom, obj + i * 2),
                        rom_data::read_word(&test_rom, lay + i * 2),
                    )
                })
                .collect();

            pipe_helpers::resort_pointer_table(&mut test_rom, world_idx);

            let mut sorted: Vec<(u8, u8, u16, u16)> = (0..n)
                .map(|i| {
                    (
                        test_rom.read_byte(rt + i),
                        test_rom.read_byte(sc + i),
                        rom_data::read_word(&test_rom, obj + i * 2),
                        rom_data::read_word(&test_rom, lay + i * 2),
                    )
                })
                .collect();

            original.sort();
            sorted.sort();
            assert_eq!(
                original, sorted,
                "World {} resort lost or gained entries",
                world_idx + 1,
            );
        }
    }

    #[test]
    fn test_swap_preserves_tileset() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };

        let mut test_rom = rom.clone();
        // Use W2 (has various entry types)
        let world = &WORLDS[1];
        let rt = world.rowtype_offset;

        // Record tilesets of entries 0 and 1
        let ts_0 = test_rom.read_byte(rt) & 0x0F;
        let ts_1 = test_rom.read_byte(rt + 1) & 0x0F;

        pipe_helpers::swap_entry_positions(&mut test_rom, 1, 0, 1);

        // Tilesets should stay with their original entry (not swap)
        let new_ts_0 = test_rom.read_byte(rt) & 0x0F;
        let new_ts_1 = test_rom.read_byte(rt + 1) & 0x0F;
        assert_eq!(new_ts_0, ts_0, "Entry 0 tileset changed after swap");
        assert_eq!(new_ts_1, ts_1, "Entry 1 tileset changed after swap");

        // But row nibbles should have swapped
        let orig_row_0 = (rom.read_byte(rt) >> 4) & 0x0F;
        let orig_row_1 = (rom.read_byte(rt + 1) >> 4) & 0x0F;
        let new_row_0 = (test_rom.read_byte(rt) >> 4) & 0x0F;
        let new_row_1 = (test_rom.read_byte(rt + 1) >> 4) & 0x0F;
        assert_eq!(new_row_0, orig_row_1, "Entry 0 should have entry 1's row");
        assert_eq!(new_row_1, orig_row_0, "Entry 1 should have entry 0's row");
    }
}
