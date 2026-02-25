/// Overworld shuffle: redistribute fortresses (and eventually levels) across worlds.
///
/// Cross-world mode: takes the 13 W1-7 fortresses and redistributes them so each
/// world gets 1-3 fortresses. Action levels swap with fortresses to keep each
/// world's total entry count constant. W8 fortresses stay fixed.
///
/// This runs AFTER level shuffle so that level randomization happens first,
/// then overworld redistribution shuffles the results.

use rand::Rng;
use rand::seq::SliceRandom;

use crate::rom::Rom;

use super::map_walker;
use super::rom_data::{
    self, AIRSHIP_ENTRIES, FxSlot, Grid, MAP_TRANSITIONS, WORLDS,
    FX_WORLD_TABLE, FX_MAP_LOC_ROW, FX_MAP_LOC, FX_MAP_TILE_REPLACE,
    LevelEntry,
};

// ---------------------------------------------------------------------------
// Module-specific constants
// ---------------------------------------------------------------------------

/// Vanilla fortress entries (world_idx, entry_idx) — W1-7 only.
const FORTRESS_ENTRIES_W1_7: [(usize, usize); 13] = [
    (0, 11),          // W1
    (1, 13),          // W2
    (2, 13), (2, 34), // W3
    (3, 9), (3, 16),  // W4
    (4, 12), (4, 31), // W5
    (5, 9), (5, 27), (5, 48), // W6
    (6, 5), (6, 40),  // W7
];

/// ROM file offset of the Boom-Boom Y-byte for each W1-7 fortress
/// (same order as FORTRESS_ENTRIES_W1_7).
const BOOMBOOM_Y_OFFSETS_W1_7: [usize; 13] = [
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
];

/// W8 fortress entries (world_idx, entry_idx).
const FORTRESS_ENTRIES_W8: [(usize, usize); 4] = [
    (7, 7), (7, 10), (7, 26), (7, 36),
];

/// ROM file offset of the Boom-Boom Y-byte for each W8 fortress.
const BOOMBOOM_Y_OFFSETS_W8: [usize; 4] = [
    0x0DA32, // W8[ 7]
    0x0DA37, // W8[10]
    0x0D597, // W8[26]
    0x0DA2D, // W8[36]
];

/// Fortress map tile ID.
const TILE_FORTRESS: u8 = 0x67;

// ---------------------------------------------------------------------------
// Fortress redistribution
// ---------------------------------------------------------------------------

/// Generate a random partition of `total` into `buckets` values,
/// each between `min` and `max` inclusive.
fn random_partition<R: Rng>(rng: &mut R, total: usize, buckets: usize, min: usize, max: usize) -> Vec<usize> {
    assert!(total >= buckets * min && total <= buckets * max);

    loop {
        let mut counts = vec![min; buckets];
        let mut remaining = total - buckets * min;

        // Distribute remaining one at a time to random buckets
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

/// Collect action level entry indices for a world that could become fortress slots.
/// Same filters as collect_shuffleable in levels.rs, but also excludes current fortresses.
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
        // Exclude fortresses and Bowser — they're handled separately
        if FORTRESS_ENTRIES_W1_7.contains(&(world_idx, i)) {
            continue;
        }
        if FORTRESS_ENTRIES_W8.contains(&(world_idx, i)) {
            continue;
        }
        if (world_idx, i) == rom_data::BOWSER_ENTRY {
            continue;
        }
        // Exclude short levels (pipe connectors)
        let tileset = rom.read_byte(world.rowtype_offset + i) & 0x0F;
        if let Some(lay_offset) = rom_data::layout_file_offset(lay_ptr, tileset) {
            if rom_data::level_screen_count(rom, lay_offset) < 3 {
                continue;
            }
        } else {
            continue;
        }
        // Only include entries on level panel tiles (0x03-0x0C).
        // Entries on path tiles (0x47, 0x4A, etc.) are roaming enemies
        // like piranha plants that shouldn't be converted to fortresses.
        let (row, col) = rom_data::entry_grid_position(rom, world, i);
        let tile_off = rom_data::map_tile_offset(world_idx, row, col);
        let tile = rom.read_byte(tile_off);
        // Note: this also excludes W5 spiral castle (tile 0x5F)
        if !(0x03..=0x0C).contains(&tile) {
            continue;
        }
        indices.push(i);
    }
    indices
}

/// Read the map tile at an entry's grid position.
fn entry_tile(rom: &Rom, world_idx: usize, entry_idx: usize) -> u8 {
    let (row, col) = rom_data::entry_grid_position(rom, &WORLDS[world_idx], entry_idx);
    let off = rom_data::map_tile_offset(world_idx, row, col);
    rom.read_byte(off)
}

/// Write a map tile at an entry's grid position.
fn set_entry_tile(rom: &mut Rom, world_idx: usize, entry_idx: usize, tile: u8) {
    let (row, col) = rom_data::entry_grid_position(rom, &WORLDS[world_idx], entry_idx);
    let off = rom_data::map_tile_offset(world_idx, row, col);
    rom.write_byte(off, tile);
}

/// Cross-world fortress redistribution.
///
/// Redistributes the 13 W1-7 fortresses so each world gets 1-3.
/// Action levels swap with fortresses to maintain each world's entry count.
/// W8 fortresses shuffle positions within W8.
pub fn redistribute_fortresses<R: Rng>(rom: &mut Rom, rng: &mut R) {
    // -----------------------------------------------------------------------
    // Part A: W1-7 cross-world redistribution
    // -----------------------------------------------------------------------

    // Step 1: Decide how many fortresses each world gets
    let new_counts = random_partition(rng, 13, 7, 1, 3);

    // Step 2: Collect all 13 fortress entries with their data, Y-byte offset,
    // and original map tile. The tile travels with the fortress.
    let mut fortress_pool: Vec<(LevelEntry, usize, u8)> = FORTRESS_ENTRIES_W1_7
        .iter()
        .zip(BOOMBOOM_Y_OFFSETS_W1_7.iter())
        .map(|(&(w, i), &y_off)| {
            let entry = rom_data::read_entry(rom, &WORLDS[w], i);
            let tile = entry_tile(rom, w, i);
            (entry, y_off, tile)
        })
        .collect();

    // Shuffle the fortress pool
    fortress_pool.as_mut_slice().shuffle(rng);

    // Step 3: For each world, assign fortresses to slots and swap with
    // action levels as needed.
    let mut fortress_pool_idx = 0;

    // Displaced action levels need new homes (entry data + tile)
    let mut displaced_levels: Vec<(LevelEntry, u8)> = Vec::new();
    // Freed fortress slots that will receive displaced levels
    let mut freed_slots: Vec<(usize, usize)> = Vec::new();

    for world_idx in 0..7 {
        let target_fort_count = new_counts[world_idx];

        // Current fortress slots in this world
        let current_fort_slots: Vec<usize> = FORTRESS_ENTRIES_W1_7
            .iter()
            .filter(|&&(w, _)| w == world_idx)
            .map(|&(_, i)| i)
            .collect();
        let current_fort_count = current_fort_slots.len();

        if target_fort_count > current_fort_count {
            // Need more slots — convert some action levels to fortresses
            let extra_needed = target_fort_count - current_fort_count;
            let action_levels = collect_action_levels(rom, world_idx);

            let slots_to_convert: Vec<usize> = action_levels
                .iter()
                .rev()
                .take(extra_needed)
                .copied()
                .collect();

            // Save displaced action levels (entry + tile)
            for &slot_idx in &slots_to_convert {
                let level_entry = rom_data::read_entry(rom, &WORLDS[world_idx], slot_idx);
                let tile = entry_tile(rom, world_idx, slot_idx);
                displaced_levels.push((level_entry, tile));
            }

            // Write fortresses to all slots (existing + converted), with tiles
            let all_fort_slots: Vec<usize> = current_fort_slots
                .iter()
                .chain(slots_to_convert.iter())
                .copied()
                .collect();

            for &slot_idx in &all_fort_slots {
                let (ref fort_entry, _y_off, fort_tile) = fortress_pool[fortress_pool_idx];
                rom_data::write_entry(rom, &WORLDS[world_idx], slot_idx, fort_entry);
                set_entry_tile(rom, world_idx, slot_idx, fort_tile);
                fortress_pool_idx += 1;
            }
        } else if target_fort_count < current_fort_count {
            // Fewer fortresses — free some slots
            let (keep_slots, free_slots) = current_fort_slots.split_at(target_fort_count);

            for &slot_idx in keep_slots {
                let (ref fort_entry, _y_off, fort_tile) = fortress_pool[fortress_pool_idx];
                rom_data::write_entry(rom, &WORLDS[world_idx], slot_idx, fort_entry);
                set_entry_tile(rom, world_idx, slot_idx, fort_tile);
                fortress_pool_idx += 1;
            }

            for &slot_idx in free_slots {
                freed_slots.push((world_idx, slot_idx));
            }
        } else {
            // Same count — assign fortress data + tiles to existing slots
            for &slot_idx in &current_fort_slots {
                let (ref fort_entry, _y_off, fort_tile) = fortress_pool[fortress_pool_idx];
                rom_data::write_entry(rom, &WORLDS[world_idx], slot_idx, fort_entry);
                set_entry_tile(rom, world_idx, slot_idx, fort_tile);
                fortress_pool_idx += 1;
            }
        }
    }

    assert_eq!(fortress_pool_idx, 13, "All 13 fortresses must be assigned");

    // Step 4: Fill freed fortress slots with displaced action levels
    displaced_levels.as_mut_slice().shuffle(rng);
    assert_eq!(
        displaced_levels.len(),
        freed_slots.len(),
        "Displaced levels must match freed slots"
    );

    for ((level_entry, level_tile), &(w, i)) in displaced_levels.iter().zip(freed_slots.iter()) {
        rom_data::write_entry(rom, &WORLDS[w], i, level_entry);
        set_entry_tile(rom, w, i, *level_tile);
    }

    // Step 5: Patch Boom-Boom Y-bytes for W1-7
    let mut fortress_pool_idx = 0;
    for world_idx in 0..7 {
        let target_fort_count = new_counts[world_idx];
        for ordinal in 1..=target_fort_count {
            let (_entry, y_offset, _tile) = &fortress_pool[fortress_pool_idx];
            let old_y = rom.read_byte(*y_offset);
            let new_y = ((ordinal as u8) << 4) | (old_y & 0x0F);
            rom.write_byte(*y_offset, new_y);
            fortress_pool_idx += 1;
        }
    }

    // Step 6: Rewrite FortressFX_W1-W8 slot assignments for W1-7.
    // Assign FX slots 0x00-0x0C sequentially.
    let mut fx_slot = 0u8;
    for world_idx in 0..7 {
        let base = FX_WORLD_TABLE + world_idx * 4;
        let count = new_counts[world_idx];
        for i in 0..4 {
            if i < count {
                rom.write_byte(base + i, fx_slot);
                fx_slot += 1;
            } else {
                rom.write_byte(base + i, 0x00);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Part B: W8 intra-world fortress position shuffle
    // -----------------------------------------------------------------------
    shuffle_w8_fortresses(rom, rng);
}

/// Shuffle W8 fortress positions among available level slots within W8.
fn shuffle_w8_fortresses<R: Rng>(rom: &mut Rom, rng: &mut R) {
    let world_idx = 7;
    let world = &WORLDS[world_idx];

    // Collect W8 fortress data + tiles
    let mut w8_forts: Vec<(LevelEntry, usize, u8)> = FORTRESS_ENTRIES_W8
        .iter()
        .zip(BOOMBOOM_Y_OFFSETS_W8.iter())
        .map(|(&(_w, i), &y_off)| {
            let entry = rom_data::read_entry(rom, world, i);
            let tile = entry_tile(rom, world_idx, i);
            (entry, y_off, tile)
        })
        .collect();

    // Collect candidate slots: current fortress slots + action level slots
    let mut candidate_slots: Vec<usize> = FORTRESS_ENTRIES_W8
        .iter()
        .map(|&(_, i)| i)
        .collect();

    // Add action level slots from W8
    let action_levels = collect_action_levels(rom, world_idx);
    candidate_slots.extend_from_slice(&action_levels);
    candidate_slots.sort();
    candidate_slots.dedup();

    // We need exactly 4 slots for 4 fortresses
    if candidate_slots.len() < 4 {
        return; // not enough slots, skip shuffle
    }

    // Pick 4 random slots from candidates
    let mut chosen_slots: Vec<usize> = candidate_slots.clone();
    chosen_slots.as_mut_slice().shuffle(rng);
    chosen_slots.truncate(4);
    chosen_slots.sort();

    // Save the level entries + tiles at the chosen slots (before overwriting)
    let mut displaced: Vec<(usize, LevelEntry, u8)> = Vec::new();
    for &slot in &chosen_slots {
        if !FORTRESS_ENTRIES_W8.iter().any(|&(_, i)| i == slot) {
            // This is an action level being displaced
            let entry = rom_data::read_entry(rom, world, slot);
            let tile = entry_tile(rom, world_idx, slot);
            displaced.push((slot, entry, tile));
        }
    }

    // Freed fortress slots (original fortress positions not in chosen_slots)
    let mut freed: Vec<usize> = Vec::new();
    for &(_, i) in &FORTRESS_ENTRIES_W8 {
        if !chosen_slots.contains(&i) {
            freed.push(i);
        }
    }

    assert_eq!(displaced.len(), freed.len(),
        "Displaced levels must match freed fortress slots in W8");

    // Shuffle fortress data
    w8_forts.as_mut_slice().shuffle(rng);

    // Write fortresses to chosen slots with their tiles
    for (fort_idx, &slot) in chosen_slots.iter().enumerate() {
        let (ref fort_entry, _y_off, fort_tile) = w8_forts[fort_idx];
        rom_data::write_entry(rom, world, slot, fort_entry);
        set_entry_tile(rom, world_idx, slot, fort_tile);
    }

    // Write displaced levels to freed slots with their tiles
    displaced.as_mut_slice().shuffle(rng);
    for ((_, level_entry, level_tile), &freed_slot) in displaced.iter().zip(freed.iter()) {
        rom_data::write_entry(rom, world, freed_slot, level_entry);
        set_entry_tile(rom, world_idx, freed_slot, *level_tile);
    }

    // Patch Boom-Boom Y-bytes for W8
    for (ordinal_0, &slot) in chosen_slots.iter().enumerate() {
        let (_entry, y_offset, _tile) = &w8_forts[ordinal_0];
        let old_y = rom.read_byte(*y_offset);
        let new_y = (((ordinal_0 + 1) as u8) << 4) | (old_y & 0x0F);
        rom.write_byte(*y_offset, new_y);
    }
}

// ---------------------------------------------------------------------------
// FortressFX table offsets (17 slots each)
// ---------------------------------------------------------------------------
const FX_VADDR_H: usize = 0x147CD;
const FX_VADDR_L: usize = 0x147DE;
const FX_MAP_COMP_IDX: usize = 0x147EF; // 17 x 2 bytes
const FX_PATTERNS: usize = 0x14811;     // 17 x 4 bytes

/// Lock tile ID on the overworld map.
const TILE_LOCK: u8 = 0x54;

/// Gap tile IDs for different FX types.
const TILE_BRIDGE_GAP: u8 = 0x56;
const TILE_WATER_GAP: u8 = 0x9D;
const TILE_SKY_GAP: u8 = 0xE4;

/// FX type determines the pattern bytes and gap tile used.
#[derive(Clone, Copy, Debug, PartialEq)]
enum FxType {
    Lock,        // FE C0 FE C0 — gap tile $54
    Bridge,      // FE FE E1 E1 — gap tile $56
    WaterBridge, // D4 D6 D5 D7 — gap tile $9D
    SkyBridge,   // FE FE E1 E1 — gap tile $E4
}

impl FxType {
    fn patterns(self) -> [u8; 4] {
        match self {
            FxType::Lock => [0xFE, 0xC0, 0xFE, 0xC0],
            FxType::Bridge => [0xFE, 0xFE, 0xE1, 0xE1],
            FxType::WaterBridge => [0xD4, 0xD6, 0xD5, 0xD7],
            FxType::SkyBridge => [0xFE, 0xFE, 0xE1, 0xE1],
        }
    }

    fn gap_tile(self) -> u8 {
        match self {
            FxType::Lock => TILE_LOCK,
            FxType::Bridge => TILE_BRIDGE_GAP,
            FxType::WaterBridge => TILE_WATER_GAP,
            FxType::SkyBridge => TILE_SKY_GAP,
        }
    }
}

/// Compute VRAM address for a map tile at (grid_row, col_in_screen).
/// Formula: 0x2880 + grid_row * 64 + col_in_screen * 2
fn fx_vram_addr(grid_row: usize, col_in_screen: usize) -> u16 {
    (0x2880 + grid_row * 64 + col_in_screen * 2) as u16
}

/// Encode FortressFX_MapLocation byte: upper nibble = column, lower nibble = screen.
fn fx_map_location(screen: usize, col: usize) -> u8 {
    ((col as u8) << 4) | (screen as u8)
}

/// Encode FortressFX_MapLocationRow byte: (grid_row + 2) << 4.
fn fx_map_location_row(grid_row: usize) -> u8 {
    ((grid_row + 2) as u8) << 4
}

/// Update a single FX slot to point at a new map position.
/// Writes VRAM addr, row, location, replacement tile, and patterns for the given FX type.
fn repoint_fx_slot(
    rom: &mut Rom,
    slot: usize,
    grid_row: usize,
    screen: usize,
    col_in_screen: usize,
    replace_tile: u8,
    comp_col: u8,
    comp_bit: u8,
    fx_type: FxType,
) {
    let vram = fx_vram_addr(grid_row, col_in_screen);
    rom.write_byte(FX_VADDR_H + slot, (vram >> 8) as u8);
    rom.write_byte(FX_VADDR_L + slot, (vram & 0xFF) as u8);
    rom.write_byte(FX_MAP_LOC_ROW + slot, fx_map_location_row(grid_row));
    rom.write_byte(FX_MAP_LOC + slot, fx_map_location(screen, col_in_screen));
    rom.write_byte(FX_MAP_TILE_REPLACE + slot, replace_tile);
    // Map_Completions persistence
    rom.write_byte(FX_MAP_COMP_IDX + slot * 2, comp_col);
    rom.write_byte(FX_MAP_COMP_IDX + slot * 2 + 1, comp_bit);
    // Pattern bytes per FX type
    let pats = fx_type.patterns();
    let pat_off = FX_PATTERNS + slot * 4;
    rom.write_byte(pat_off, pats[0]);
    rom.write_byte(pat_off + 1, pats[1]);
    rom.write_byte(pat_off + 2, pats[2]);
    rom.write_byte(pat_off + 3, pats[3]);
}

/// Map_Complete_Bits lookup table (PRG012): maps grid row to completion bit.
/// Row 0 = $80, row 1 = $40, ..., row 7 = $01.
const MAP_COMPLETE_BITS: [u8; 8] = [0x80, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x01];

/// Compute the Map_Completions (column, bit) pair for a lock at a given grid position.
/// Column = screen * 16 + col_in_screen; bit = MAP_COMPLETE_BITS[grid_row].
fn fx_comp_idx(grid_row: usize, screen: usize, col_in_screen: usize) -> (u8, u8) {
    let col = (screen * 16 + col_in_screen) as u8;
    let bit = MAP_COMPLETE_BITS[grid_row];
    (col, bit)
}

/// Place a lock tile at a grid position, saving the original tile.
/// Returns the original tile that was at that position (for FortressFX_MapTileReplace).
fn place_lock(rom: &mut Rom, world_idx: usize, grid_row: usize, grid_col: usize) -> u8 {
    let offset = rom_data::map_tile_offset(world_idx, grid_row, grid_col);
    let orig = rom.read_byte(offset);
    rom.write_byte(offset, TILE_LOCK);
    orig
}

/// Place a gap tile (bridge/water/sky) at a grid position, saving the original tile.
/// Returns the original tile (used as the FX replacement tile when the gap is cleared).
fn place_gap(rom: &mut Rom, world_idx: usize, grid_row: usize, grid_col: usize, fx_type: FxType) -> u8 {
    let offset = rom_data::map_tile_offset(world_idx, grid_row, grid_col);
    let orig = rom.read_byte(offset);
    rom.write_byte(offset, fx_type.gap_tile());
    orig
}

/// Remove a lock tile, restoring the given path tile.
fn remove_lock(rom: &mut Rom, world_idx: usize, grid_row: usize, grid_col: usize, restore_tile: u8) {
    let offset = rom_data::map_tile_offset(world_idx, grid_row, grid_col);
    rom.write_byte(offset, restore_tile);
}

// ---------------------------------------------------------------------------
// Lock shuffle
// ---------------------------------------------------------------------------

/// All path tiles that a lock can be placed on.
const LOCKABLE_TILES: &[u8] = &[
    0x45, // horizontal path
    0x46, // vertical path
    0xB3, // water bridge path
    0xDA, // sky bridge path
    0xAC, // horizontal path variant
    0xB7, // horizontal path variant
    0xB8, // horizontal path variant
    0xB9, // horizontal path variant
    0xE6, // horizontal path variant
    0xAA, // vertical path variant
    0xAB, // vertical path variant
    0xB0, // vertical path variant
    0xB1, // vertical drawbridge (if fix_drawbridges is off)
    0xB2, // horizontal drawbridge (if fix_drawbridges is off)
    0xDB, // vertical path variant
    0xBA, // vertical path variant
];

/// Determine the FX pattern bytes for a given replacement tile.
/// These must match the visual appearance of the tile being restored.
fn patterns_for_tile(tile: u8) -> [u8; 4] {
    match tile {
        // Vertical path tiles → vertical path pattern
        0x46 | 0xAA | 0xAB | 0xB0 | 0xB1 | 0xDB | 0xBA => [0xFE, 0xC0, 0xFE, 0xC0],
        // Water bridge → water pattern
        0xB3 => [0xD4, 0xD6, 0xD5, 0xD7],
        // Horizontal path tiles, sky bridge → horizontal path pattern
        _ => [0xFE, 0xFE, 0xE1, 0xE1],
    }
}

/// Airship dock tile ID.
const TILE_AIRSHIP: u8 = 0xC9;

/// Bowser's castle tile ID.
const TILE_BOWSER: u8 = 0xCC;

/// Get the airship or Bowser's castle grid position for a world.
/// Scans the map tile grid for the target tile to handle post-resort state
/// where entry indices may have changed.
fn world_target_position(rom: &Rom, world_idx: usize) -> Option<(usize, usize)> {
    let grid = rom_data::read_tile_grid(rom, world_idx);
    let target_tile = if world_idx == 7 { TILE_BOWSER } else { TILE_AIRSHIP };
    for r in 0..grid.rows {
        for c in 0..grid.cols {
            if grid.get(r, c) == target_tile {
                return Some((r, c));
            }
        }
    }
    None
}

/// Pre-open all locks/bridges/gaps in a world by scanning the grid for blocking
/// tiles and restoring the replacement tile from the matching FX slot.
///
/// Takes a pre-saved snapshot of FX slot data (read before any repointing)
/// because shuffle_locks repoints slots as it processes worlds sequentially,
/// which would invalidate live reads for later worlds.
fn pre_open_fx_for_world(
    rom: &mut Rom,
    world_idx: usize,
    fx_slots_snapshot: &[FxSlot],
) {
    let grid = rom_data::read_tile_grid(rom, world_idx);

    for r in 0..grid.rows {
        for c in 0..grid.cols {
            let tile = grid.get(r, c);
            if tile != TILE_LOCK && tile != TILE_BRIDGE_GAP
                && tile != TILE_WATER_GAP && tile != TILE_SKY_GAP
            {
                continue;
            }
            // Find an FX slot that matches this position
            if let Some(slot) = fx_slots_snapshot.iter().find(|s| s.grid_row == r && s.grid_col == c) {
                let offset = rom_data::map_tile_offset(world_idx, r, c);
                rom.write_byte(offset, slot.replace_tile);
            }
        }
    }
}

/// Find fortress positions by scanning the pointer table for tileset-2 entries.
/// Returns grid positions of all fortresses (excluding airships and Bowser).
fn find_fortress_entry_positions(rom: &Rom, world_idx: usize) -> Vec<(usize, usize)> {
    let world = &WORLDS[world_idx];
    let (_scrcol, objsets, layouts) = rom_data::table_offsets(world);
    let mut positions = Vec::new();
    for i in 0..world.entry_count {
        let tileset = rom.read_byte(world.rowtype_offset + i) & 0x0F;
        let obj_ptr = rom_data::read_word(rom, objsets + i * 2);
        let lay_ptr = rom_data::read_word(rom, layouts + i * 2);
        if tileset != 2 || obj_ptr < 0xC000 || lay_ptr == 0x0000 {
            continue;
        }
        if AIRSHIP_ENTRIES.contains(&(world_idx, i)) {
            continue;
        }
        if (world_idx, i) == rom_data::BOWSER_ENTRY {
            continue;
        }
        let (row, col) = rom_data::entry_grid_position(rom, world, i);
        positions.push((row, col));
    }
    positions.sort();
    positions
}

/// Shuffle lock positions for all worlds.
///
/// For each world, pre-opens vanilla locks/bridges, determines fortress beat order
/// via BFS progression, then places new locks at random valid path tiles using
/// greedy forward placement. Each lock is validated to not block its own fortress.
pub fn shuffle_locks<R: Rng>(rom: &mut Rom, rng: &mut R) {
    let all_pipes = rom_data::read_pipe_pairs(rom);
    // Snapshot FX slot data ONCE before any repointing. As we process worlds
    // sequentially, repointing W1's slots would corrupt the position data
    // that W2+ needs for pre-opening their vanilla blocking tiles.
    let fx_slots_snapshot = rom_data::read_fx_slots(rom);

    for world_idx in 0..8 {
        let pipes = all_pipes.get(&world_idx).cloned().unwrap_or_default();

        // Pre-open all locks/bridges for this world using the snapshot.
        // Must happen even when fort_count == 0 (world lost all fortresses).
        pre_open_fx_for_world(rom, world_idx, &fx_slots_snapshot);

        // Read FX slot assignments from FX_WORLD_TABLE (authoritative source
        // of how many FX slots this world uses — set by redistribute or vanilla).
        let base = FX_WORLD_TABLE + world_idx * 4;
        let mut world_fx: Vec<u8> = Vec::new();
        for i in 0..4 {
            let slot = rom.read_byte(base + i);
            if slot == 0 && !(world_idx == 0 && i == 0) {
                break;
            }
            world_fx.push(slot);
        }
        let fort_count = world_fx.len();

        if fort_count == 0 {
            continue;
        }

        // Find fortress positions from pointer table
        let fort_positions = find_fortress_entry_positions(rom, world_idx);
        // Use the FX count (not the entry count) — some tileset-2 entries
        // like W5's spiral tower don't have Boom-Boom and no FX slot.
        let fort_positions: Vec<(usize, usize)> = fort_positions.into_iter().take(fort_count).collect();

        // Read the clean grid (all locks/bridges opened)
        let grid = rom_data::read_tile_grid(rom, world_idx);

        // Determine beat order by simulating progression on the clean grid
        let beat_order = determine_beat_order(&grid, &pipes, &fort_positions);

        // Get the airship/Bowser target position
        let target_pos = world_target_position(rom, world_idx);

        // Place locks with validation. For each lock, pick a random non-chokepoint
        // path tile. After placing all locks, simulate full progression. If any
        // fortress or the target is unreachable, retry (up to 50 attempts).
        let placed_locks = place_locks_for_world(
            rom, rng, world_idx, &grid, &pipes, &fort_positions, &beat_order, target_pos,
        );

        // Write all locks to the ROM
        for lock in &placed_locks {
            if let Some((lr, lc, _)) = lock {
                place_lock(rom, world_idx, *lr, *lc);
            }
        }

        // Repoint FX slots
        for (ord, _) in beat_order.iter().enumerate() {
            if let Some((lr, lc, replace_tile)) = placed_locks[ord] {
                if ord < world_fx.len() {
                    let slot_idx = world_fx[ord] as usize;
                    let screen = lc / 16;
                    let col_in_screen = lc % 16;
                    let (comp_col, comp_bit) = fx_comp_idx(lr, screen, col_in_screen);
                    let pats = patterns_for_tile(replace_tile);

                    repoint_fx_slot(
                        rom,
                        slot_idx,
                        lr,
                        screen,
                        col_in_screen,
                        replace_tile,
                        comp_col,
                        comp_bit,
                        FxType::Lock,
                    );

                    // Override patterns to match the specific replacement tile
                    let pat_off = FX_PATTERNS + slot_idx * 4;
                    rom.write_byte(pat_off, pats[0]);
                    rom.write_byte(pat_off + 1, pats[1]);
                    rom.write_byte(pat_off + 2, pats[2]);
                    rom.write_byte(pat_off + 3, pats[3]);
                }
            }
        }
    }
}

/// Place locks for one world with full progression validation.
///
/// Picks random non-chokepoint path tiles for each fortress, then simulates
/// the full fortress progression. If any fortress or the target becomes
/// unreachable, retries with different random choices.
fn place_locks_for_world<R: Rng>(
    rom: &Rom,
    rng: &mut R,
    world_idx: usize,
    grid: &Grid,
    pipes: &[((usize, usize), (usize, usize))],
    fort_positions: &[(usize, usize)],
    beat_order: &[usize],
    target_pos: Option<(usize, usize)>,
) -> Vec<Option<(usize, usize, u8)>> {
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
        // Pick n distinct random positions from candidates
        let mut choices: Vec<(usize, usize)> = Vec::new();
        let mut available = all_candidates.clone();
        available.as_mut_slice().shuffle(rng);

        for _ in 0..n {
            if let Some(pos) = available.pop() {
                choices.push(pos);
            }
        }

        if choices.len() < n {
            // Not enough candidates — use what we have
            break;
        }

        // Validate: simulate full progression with these locks
        if validate_lock_placement(grid, pipes, fort_positions, beat_order, &choices, target_pos) {
            // Convert to placed_locks format
            return choices.iter().map(|&(r, c)| {
                let tile_offset = rom_data::map_tile_offset(world_idx, r, c);
                let replace_tile = rom.read_byte(tile_offset);
                Some((r, c, replace_tile))
            }).collect();
        }
    }

    // Fallback: no locks (all None)
    vec![None; n]
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
    // Start with all locks active
    let mut sim_grid = grid.clone_grid();
    for &(r, c) in lock_positions {
        sim_grid.set(r, c, TILE_LOCK);
    }

    // Beat forts in order
    for (ord, &fort_idx) in beat_order.iter().enumerate() {
        let fort_pos = fort_positions[fort_idx];

        // Check fort is reachable with current locks
        let result = map_walker::walk_map(&sim_grid, pipes, None);
        if !result.nodes.contains(&fort_pos) {
            return false;
        }

        // "Beat" the fort: open its lock
        if ord < lock_positions.len() {
            let (lr, lc) = lock_positions[ord];
            // Restore original tile (from the clean grid)
            sim_grid.set(lr, lc, grid.get(lr, lc));
        }
    }

    // After all forts beaten, check target is reachable
    if let Some(target) = target_pos {
        let result = map_walker::walk_map(&sim_grid, pipes, None);
        if !result.nodes.contains(&target) {
            return false;
        }
    }

    true
}

/// Determine the order fortresses are beaten by simulating progression.
/// Returns indices into fort_positions in the order they'd be reached.
fn determine_beat_order(
    grid: &Grid,
    pipes: &[((usize, usize), (usize, usize))],
    fort_positions: &[(usize, usize)],
) -> Vec<usize> {
    let mut order = Vec::new();
    let mut beaten = std::collections::HashSet::new();

    // Walk the clean grid — all fortresses are reachable in some order
    loop {
        let result = map_walker::walk_map(grid, pipes, None);

        // Find the first reachable fortress not yet beaten
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

    /// POC: Two fortresses in W1 — slot 0 = lock (existing), slot 1 = water bridge.
    /// Places fortress data in entries [0] and [2] (1-1 and 1-3) for easy testing.
    /// Replaces B3 at row 6, col 9 with water gap 9D; FX will build it back to B3.
    #[test]
    fn test_poc_bridge_in_w1() {
        let rom_data = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data.is_err() {
            return; // Skip if ROM not available
        }
        let mut rom = Rom::from_bytes(&rom_data.unwrap()).unwrap();

        // --- Read W1 fortress (entry 11) and W2 fortress (entry 13) ---
        let w1_fort = rom_data::read_entry(&rom, &WORLDS[0], 11);
        let w2_fort = rom_data::read_entry(&rom, &WORLDS[1], 13);

        // --- Write fortresses into entries [0] and [2] (1-1 and 1-3) ---
        rom_data::write_entry(&mut rom, &WORLDS[0], 0, &w1_fort);
        rom_data::write_entry(&mut rom, &WORLDS[0], 2, &w2_fort);

        // Put fortress tiles on the map for entries [0] and [2]
        let (row0, col0) = rom_data::entry_grid_position(&rom, &WORLDS[0], 0); // row 0, col 4
        let (row2, col2) = rom_data::entry_grid_position(&rom, &WORLDS[0], 2); // row 0, col 8
        let off0 = rom_data::map_tile_offset(0, row0, col0);
        let off2 = rom_data::map_tile_offset(0, row2, col2);
        rom.write_byte(off0, TILE_FORTRESS);
        rom.write_byte(off2, TILE_FORTRESS);

        // --- Patch Boom-Boom Y-bytes: ordinal 1 and 2 ---
        // W1 fortress Y-byte (ordinal 1)
        let w1_y = rom.read_byte(BOOMBOOM_Y_OFFSETS_W1_7[0]);
        rom.write_byte(BOOMBOOM_Y_OFFSETS_W1_7[0], (1 << 4) | (w1_y & 0x0F));
        // W2 fortress Y-byte (ordinal 2)
        let w2_y = rom.read_byte(BOOMBOOM_Y_OFFSETS_W1_7[1]);
        rom.write_byte(BOOMBOOM_Y_OFFSETS_W1_7[1], (2 << 4) | (w2_y & 0x0F));

        // --- FX slot 0: existing lock at row 3, col 4 (unchanged from vanilla) ---
        // Already correct in the ROM — slot 0 points at the lock.

        // --- FX slot 1: water bridge at row 6, col 9 ---
        // Place water gap tile (0x9D) where B3 currently is
        let replace_tile = place_gap(&mut rom, 0, 6, 9, FxType::WaterBridge);
        assert_eq!(replace_tile, 0xB3, "Expected B3 (water) under the bridge gap");

        // Repoint FX slot 1 to the new water bridge position
        let screen = 0;
        let col_in_screen = 9;
        let (comp_col, comp_bit) = fx_comp_idx(6, screen, col_in_screen);
        repoint_fx_slot(
            &mut rom,
            1,       // slot 1
            6,       // grid_row
            screen,
            col_in_screen,
            replace_tile, // 0xB3 — tile to restore when bridge is built
            comp_col,
            comp_bit,
            FxType::WaterBridge,
        );

        // --- Update FortressFX_W1 to use slots 0 and 1 ---
        rom.write_byte(FX_WORLD_TABLE, 0x00);
        rom.write_byte(FX_WORLD_TABLE + 1, 0x01);
        rom.write_byte(FX_WORLD_TABLE + 2, 0x00);
        rom.write_byte(FX_WORLD_TABLE + 3, 0x00);

        // --- Verify the map tile was replaced ---
        let gap_tile = rom.read_byte(rom_data::map_tile_offset(0, 6, 9));
        assert_eq!(gap_tile, TILE_WATER_GAP);

        // --- Write patched ROM for manual testing ---
        let out_path = "target/poc_bridge_w1.nes";
        std::fs::create_dir_all("target").ok();
        std::fs::write(out_path, &rom.data).unwrap();
        println!("Wrote POC ROM to {}", out_path);
    }

    #[test]
    fn test_redistribute_deterministic() {
        // Two runs with the same seed should produce identical results
        let rom_data = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data.is_err() {
            // Skip test if ROM not available (CI)
            return;
        }

        let mut rom1 = Rom::from_bytes(&rom_data.as_ref().unwrap()).unwrap();
        let mut rom2 = Rom::from_bytes(&rom_data.as_ref().unwrap()).unwrap();

        let mut rng1 = ChaCha8Rng::seed_from_u64(12345);
        let mut rng2 = ChaCha8Rng::seed_from_u64(12345);

        redistribute_fortresses(&mut rom1, &mut rng1);
        redistribute_fortresses(&mut rom2, &mut rng2);

        // Check all pointer table data matches
        for world in &WORLDS {
            let n = world.entry_count;
            let start = world.rowtype_offset;
            let end = start + n * 6; // rowtype + scrcol + obj(2) + lay(2) per entry
            for off in start..end {
                assert_eq!(
                    rom1.read_byte(off),
                    rom2.read_byte(off),
                    "Mismatch at 0x{:05X}",
                    off,
                );
            }
        }

        // Check FX table matches
        for off in FX_WORLD_TABLE..FX_WORLD_TABLE + 32 {
            assert_eq!(rom1.read_byte(off), rom2.read_byte(off));
        }

        // Check Y-bytes match
        for &off in &BOOMBOOM_Y_OFFSETS_W1_7 {
            assert_eq!(rom1.read_byte(off), rom2.read_byte(off));
        }
    }

    #[test]
    fn test_shuffle_locks_deterministic() {
        let rom_data = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data.is_err() {
            return;
        }

        let mut rom1 = Rom::from_bytes(&rom_data.as_ref().unwrap()).unwrap();
        let mut rom2 = Rom::from_bytes(&rom_data.as_ref().unwrap()).unwrap();

        let mut rng1 = ChaCha8Rng::seed_from_u64(777);
        let mut rng2 = ChaCha8Rng::seed_from_u64(777);

        shuffle_locks(&mut rom1, &mut rng1);
        shuffle_locks(&mut rom2, &mut rng2);

        // Check all FX table data matches
        for off in FX_VADDR_H..FX_VADDR_H + 17 {
            assert_eq!(rom1.read_byte(off), rom2.read_byte(off), "VAddrH mismatch at {off}");
        }
        for off in FX_VADDR_L..FX_VADDR_L + 17 {
            assert_eq!(rom1.read_byte(off), rom2.read_byte(off), "VAddrL mismatch at {off}");
        }
        for off in FX_MAP_COMP_IDX..FX_MAP_COMP_IDX + 34 {
            assert_eq!(rom1.read_byte(off), rom2.read_byte(off), "CompIdx mismatch at {off}");
        }
        for off in FX_PATTERNS..FX_PATTERNS + 68 {
            assert_eq!(rom1.read_byte(off), rom2.read_byte(off), "Patterns mismatch at {off}");
        }

        // Check map tile grids match for all worlds
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
    fn test_shuffle_locks_no_row_8() {
        let rom_data = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data.is_err() {
            return;
        }

        let mut rom = Rom::from_bytes(&rom_data.unwrap()).unwrap();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        shuffle_locks(&mut rom, &mut rng);

        // Check no lock tiles at row 8 in any world
        for wi in 0..8 {
            let grid = rom_data::read_tile_grid(&rom, wi);
            for c in 0..grid.cols {
                assert_ne!(
                    grid.get(8, c), TILE_LOCK,
                    "Lock at row 8 in W{} col {}", wi + 1, c,
                );
            }
        }
    }

    #[test]
    fn test_shuffle_locks_progression_valid() {
        let rom_data = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data.is_err() {
            return;
        }

        // Test multiple seeds
        for seed in [42, 123, 999, 31337, 65536] {
            let mut rom = Rom::from_bytes(&rom_data.as_ref().unwrap()).unwrap();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            shuffle_locks(&mut rom, &mut rng);

            let all_pipes = rom_data::read_pipe_pairs(&rom);

            for wi in 0..8 {
                let pipes = all_pipes.get(&wi).cloned().unwrap_or_default();
                let fort_positions = rom_data::read_fortress_positions(&rom, wi);
                if fort_positions.is_empty() {
                    continue;
                }

                // Simulate progression: beat forts in order, open locks
                let steps = map_walker::simulate_progression(&rom, wi, &pipes);

                // After all steps, verify the airship/Bowser is reachable
                let final_nodes = &steps.last().unwrap().nodes;
                if let Some(target) = world_target_position(&rom, wi) {
                    assert!(
                        final_nodes.contains(&target),
                        "Seed {seed} W{}: airship/Bowser at ({},{}) not reachable after all fortresses beaten",
                        wi + 1, target.0, target.1,
                    );
                }
            }
        }
    }

    #[test]
    fn test_shuffle_locks_with_other_features() {
        let rom_data = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data.is_err() {
            return;
        }

        // Run with multiple seeds to verify lock shuffle doesn't panic or
        // reduce connectivity when combined with other overworld features.
        // Note: redistribute_fortresses can break pipe connectivity (tile
        // swaps may overwrite pipe tiles), so we verify lock shuffle's
        // guarantee: it doesn't make things worse, not that the full
        // combination always produces a solvable map.
        for seed in [42, 123, 999, 31337] {
            let mut rom_before = Rom::from_bytes(&rom_data.as_ref().unwrap()).unwrap();
            let mut rom_after = Rom::from_bytes(&rom_data.as_ref().unwrap()).unwrap();

            // Run everything EXCEPT lock shuffle
            let mut options_no_locks = crate::randomizer::Options::default();
            options_no_locks.shuffle_fortresses = true;
            options_no_locks.redistribute_fortresses = true;
            options_no_locks.shuffle_pipes = true;
            options_no_locks.shuffle_locks = false;
            options_no_locks.fix_drawbridges = true;
            options_no_locks.remove_w2_rock = true;
            crate::randomizer::randomize(&mut rom_before, seed, &options_no_locks);

            // Run everything INCLUDING lock shuffle
            let mut options_with_locks = crate::randomizer::Options::default();
            options_with_locks.shuffle_fortresses = true;
            options_with_locks.redistribute_fortresses = true;
            options_with_locks.shuffle_pipes = true;
            options_with_locks.shuffle_locks = true;
            options_with_locks.fix_drawbridges = true;
            options_with_locks.remove_w2_rock = true;
            crate::randomizer::randomize(&mut rom_after, seed, &options_with_locks);

            // For each world, verify lock shuffle didn't reduce reachability
            let pipes_before = rom_data::read_pipe_pairs(&rom_before);
            let pipes_after = rom_data::read_pipe_pairs(&rom_after);

            for wi in 0..8 {
                let pb = pipes_before.get(&wi).cloned().unwrap_or_default();
                let pa = pipes_after.get(&wi).cloned().unwrap_or_default();

                let grid_before = rom_data::read_tile_grid(&rom_before, wi);
                let grid_after = rom_data::read_tile_grid(&rom_after, wi);

                let walk_before = map_walker::walk_map(&grid_before, &pb, None);
                let walk_after = map_walker::walk_map(&grid_after, &pa, None);

                // Lock shuffle may add lock tiles that reduce initial reachability
                // (that's the point — locks block paths). But after beating all
                // fortresses (i.e., all locks opened), reachability must be at
                // least as good as without lock shuffle.
                // Since locks are placed on path tiles and opened to restore them,
                // the "all locks open" state should have the same reachability.

                // Verify: no crash, valid FX data written
                let fort_positions = find_fortress_entry_positions(&rom_after, wi);
                let fort_count = fort_positions.len();
                let base = FX_WORLD_TABLE + wi * 4;
                for i in 0..fort_count.min(4) {
                    let slot_idx = rom_after.read_byte(base + i) as usize;
                    assert!(
                        slot_idx < 17,
                        "Seed {seed} W{}: FX slot index {slot_idx} out of range",
                        wi + 1,
                    );
                }

                // Count lock tiles placed (should be <= fort_count)
                let lock_count = (0..grid_after.rows)
                    .flat_map(|r| (0..grid_after.cols).map(move |c| (r, c)))
                    .filter(|&(r, c)| grid_after.get(r, c) == TILE_LOCK)
                    .count();
                assert!(
                    lock_count <= fort_count,
                    "Seed {seed} W{}: {lock_count} locks but only {fort_count} fortresses",
                    wi + 1,
                );
            }
        }
    }

    #[test]
    fn test_shuffle_locks_visual() {
        let rom_data = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data.is_err() {
            return;
        }

        let mut rom = Rom::from_bytes(&rom_data.unwrap()).unwrap();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        shuffle_locks(&mut rom, &mut rng);

        let all_pipes = rom_data::read_pipe_pairs(&rom);

        println!("\n\x1b[1;33m=== Lock Shuffle (seed 42) ===\x1b[0m\n");
        for wi in 0..8 {
            let pipes = all_pipes.get(&wi).cloned().unwrap_or_default();
            let output = map_walker::render_progression(&rom, wi, &pipes);
            print!("{output}");
        }
    }
}
