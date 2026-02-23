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

// Re-use WorldTables and helpers from levels.rs — but since they're private,
// we duplicate the minimal set we need here.

struct WorldTables {
    rowtype_offset: usize,
    entry_count: usize,
}

const WORLDS: [WorldTables; 8] = [
    WorldTables { rowtype_offset: 0x19438, entry_count: 21 }, // World 1
    WorldTables { rowtype_offset: 0x194BA, entry_count: 47 }, // World 2
    WorldTables { rowtype_offset: 0x195D8, entry_count: 52 }, // World 3
    WorldTables { rowtype_offset: 0x19714, entry_count: 34 }, // World 4
    WorldTables { rowtype_offset: 0x197E4, entry_count: 42 }, // World 5
    WorldTables { rowtype_offset: 0x198E4, entry_count: 57 }, // World 6
    WorldTables { rowtype_offset: 0x19A3E, entry_count: 46 }, // World 7
    WorldTables { rowtype_offset: 0x19B56, entry_count: 41 }, // World 8
];

/// PRG bank loaded at CPU $A000-$BFFF for each tileset (0-18).
const PAGE_A000_BY_TILESET: [usize; 19] = [
    11, 15, 21, 16, 17, 19, 18, 18, 18, 20, 23, 19, 17, 19, 13, 26, 26, 26, 9,
];

/// Data that travels with a level when shuffled.
#[derive(Clone, Debug)]
struct LevelEntry {
    tileset: u8,
    obj_lo: u8,
    obj_hi: u8,
    lay_lo: u8,
    lay_hi: u8,
}

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

/// Airship entries — excluded from becoming fortress slots.
const AIRSHIP_ENTRIES: [(usize, usize); 7] = [
    (0, 17), (1, 36), (2, 49), (3, 6), (4, 35), (5, 53), (6, 43),
];

/// Map transition entries — excluded from becoming fortress slots.
const MAP_TRANSITION_ENTRIES: [(usize, usize); 1] = [
    (4, 5),
];

/// FortressFX_W1-W8 table: 32 bytes at 0x14888, 4 slots per world.
const FORTRESS_FX_WORLD_TABLE: usize = 0x14888;

/// Map tile grid file offsets and column counts per world.
/// Storage: row-major per screen, 144 bytes per screen (9 rows x 16 cols).
const MAP_TILE_GRIDS: [(usize, usize); 8] = [
    (0x185BA, 16),  // W1: 1 screen
    (0x1864B, 32),  // W2: 2 screens
    (0x1876C, 48),  // W3: 3 screens
    (0x1891D, 32),  // W4: 2 screens
    (0x18A3E, 32),  // W5: 2 screens
    (0x18B5F, 48),  // W6: 3 screens
    (0x18D10, 32),  // W7: 2 screens
    (0x18E31, 64),  // W8: 4 screens
];

const MAP_TILE_GRID_ROWS: usize = 9;

/// Fortress map tile ID.
const TILE_FORTRESS: u8 = 0x67;


// ---------------------------------------------------------------------------
// ROM helpers
// ---------------------------------------------------------------------------

fn table_offsets(world: &WorldTables) -> (usize, usize, usize) {
    let n = world.entry_count;
    let scrcol = world.rowtype_offset + n;
    let objsets = scrcol + n;
    let layouts = objsets + n * 2;
    (scrcol, objsets, layouts)
}

fn read_word(rom: &Rom, offset: usize) -> u16 {
    let lo = rom.read_byte(offset) as u16;
    let hi = rom.read_byte(offset + 1) as u16;
    (hi << 8) | lo
}

fn read_entry(rom: &Rom, world: &WorldTables, idx: usize) -> LevelEntry {
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

fn write_entry(rom: &mut Rom, world: &WorldTables, idx: usize, entry: &LevelEntry) {
    let (_scrcol, objsets, layouts) = table_offsets(world);
    let obj_off = objsets + idx * 2;
    let lay_off = layouts + idx * 2;

    // Preserve upper nibble (row), replace lower nibble (tileset)
    let old_brt = rom.read_byte(world.rowtype_offset + idx);
    let new_brt = (old_brt & 0xF0) | (entry.tileset & 0x0F);
    rom.write_byte(world.rowtype_offset + idx, new_brt);

    rom.write_byte(obj_off, entry.obj_lo);
    rom.write_byte(obj_off + 1, entry.obj_hi);
    rom.write_byte(lay_off, entry.lay_lo);
    rom.write_byte(lay_off + 1, entry.lay_hi);
}

/// Compute the ROM file offset of a map tile at (row, col) in a world's tile grid.
/// Row-major per screen: each screen is 144 bytes (9 rows x 16 cols).
fn map_tile_offset(world_idx: usize, row: usize, col: usize) -> usize {
    let (base, _cols) = MAP_TILE_GRIDS[world_idx];
    let screen = col / 16;
    let col_in_screen = col % 16;
    base + screen * 144 + row * 16 + col_in_screen
}

/// Get the (grid_row, grid_col) for a pointer table entry by reading its
/// ByRowType and ByScrCol bytes.
fn entry_grid_position(rom: &Rom, world: &WorldTables, idx: usize) -> (usize, usize) {
    let row_nibble = (rom.read_byte(world.rowtype_offset + idx) >> 4) & 0x0F;
    let scrcol = rom.read_byte(world.rowtype_offset + world.entry_count + idx);
    let screen = (scrcol >> 4) & 0x0F;
    let column = scrcol & 0x0F;
    let grid_row = (row_nibble as usize).wrapping_sub(2);
    let grid_col = screen as usize * 16 + column as usize;
    (grid_row, grid_col)
}

fn is_level_pointer(obj_ptr: u16, lay_ptr: u16) -> bool {
    obj_ptr >= 0xC000 && lay_ptr != 0x0000
}

fn layout_file_offset(cpu_addr: u16, tileset: u8) -> Option<usize> {
    if tileset as usize >= PAGE_A000_BY_TILESET.len() || cpu_addr < 0xA000 {
        return None;
    }
    let bank = PAGE_A000_BY_TILESET[tileset as usize];
    Some(bank * 0x2000 + 0x10 + (cpu_addr as usize - 0xA000))
}

fn level_screen_count(rom: &Rom, layout_offset: usize) -> u8 {
    (rom.read_byte(layout_offset + 4) & 0x0F) + 1
}

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
    let (_scrcol, objsets, layouts) = table_offsets(world);

    // Count (obj, lay) pairs to detect hammer bros duplicates
    let mut pair_counts = std::collections::HashMap::new();
    for i in 0..world.entry_count {
        let obj_ptr = read_word(rom, objsets + i * 2);
        let lay_ptr = read_word(rom, layouts + i * 2);
        if is_level_pointer(obj_ptr, lay_ptr) {
            *pair_counts.entry((obj_ptr, lay_ptr)).or_insert(0u32) += 1;
        }
    }

    let mut indices = Vec::new();
    for i in 0..world.entry_count {
        let obj_ptr = read_word(rom, objsets + i * 2);
        let lay_ptr = read_word(rom, layouts + i * 2);
        if !is_level_pointer(obj_ptr, lay_ptr) {
            continue;
        }
        if AIRSHIP_ENTRIES.contains(&(world_idx, i)) {
            continue;
        }
        if MAP_TRANSITION_ENTRIES.contains(&(world_idx, i)) {
            continue;
        }
        if pair_counts[&(obj_ptr, lay_ptr)] > 1 {
            continue; // hammer bros
        }
        // Exclude current fortresses — they're handled separately
        if FORTRESS_ENTRIES_W1_7.contains(&(world_idx, i)) {
            continue;
        }
        // Exclude short levels (pipe connectors)
        let tileset = rom.read_byte(world.rowtype_offset + i) & 0x0F;
        if let Some(lay_offset) = layout_file_offset(lay_ptr, tileset) {
            if level_screen_count(rom, lay_offset) < 3 {
                continue;
            }
        } else {
            continue;
        }
        // Only include entries on level panel tiles (0x03-0x0C).
        // Entries on path tiles (0x47, 0x4A, etc.) are roaming enemies
        // like piranha plants that shouldn't be converted to fortresses.
        let (row, col) = entry_grid_position(rom, world, i);
        let tile_off = map_tile_offset(world_idx, row, col);
        let tile = rom.read_byte(tile_off);
        if !(0x03..=0x0C).contains(&tile) {
            continue;
        }
        indices.push(i);
    }
    indices
}

/// Cross-world fortress redistribution.
///
/// Redistributes the 13 W1-7 fortresses so each world gets 1-3.
/// Action levels swap with fortresses to maintain each world's entry count.
/// W8 is untouched.
pub fn redistribute_fortresses<R: Rng>(rom: &mut Rom, rng: &mut R) {
    // Step 1: Decide how many fortresses each world gets
    let new_counts = random_partition(rng, 13, 7, 1, 3);

    // Step 2: Collect all 13 fortress LevelEntry data + their Y-byte offsets
    let mut fortress_pool: Vec<(LevelEntry, usize)> = FORTRESS_ENTRIES_W1_7
        .iter()
        .zip(BOOMBOOM_Y_OFFSETS_W1_7.iter())
        .map(|(&(w, i), &y_off)| {
            let entry = read_entry(rom, &WORLDS[w], i);
            (entry, y_off)
        })
        .collect();

    // Shuffle the fortress pool
    fortress_pool.as_mut_slice().shuffle(rng);

    // Step 3: For each world, figure out which slots are fortress vs action level
    // and perform the swaps
    let mut fortress_pool_idx = 0;

    // Collect displaced action level entries that need new homes
    let mut displaced_levels: Vec<LevelEntry> = Vec::new();
    // Collect freed fortress slots (world_idx, entry_idx)
    let mut freed_slots: Vec<(usize, usize)> = Vec::new();
    // Track all tile swaps: (world, slot) pairs where type changed.
    // "became_fortress" = was action level, now fortress
    // "became_level" = was fortress, now action level
    let mut became_fortress: Vec<(usize, usize)> = Vec::new();
    let mut became_level: Vec<(usize, usize)> = Vec::new();
    // Save original tiles before any writes
    let mut original_tiles: std::collections::HashMap<(usize, usize), u8> = std::collections::HashMap::new();
    // Pre-save tiles for all fortress and action level slots that might change
    for world_idx in 0..7 {
        for &(w, i) in &FORTRESS_ENTRIES_W1_7 {
            if w == world_idx {
                let (row, col) = entry_grid_position(rom, &WORLDS[w], i);
                let tile_off = map_tile_offset(w, row, col);
                original_tiles.insert((w, i), rom.read_byte(tile_off));
            }
        }
        let action_levels = collect_action_levels(rom, world_idx);
        for &i in &action_levels {
            let (row, col) = entry_grid_position(rom, &WORLDS[world_idx], i);
            let tile_off = map_tile_offset(world_idx, row, col);
            original_tiles.insert((world_idx, i), rom.read_byte(tile_off));
        }
    }

    for world_idx in 0..7 {
        let target_fort_count = new_counts[world_idx];

        // Current fortress slots in this world
        let current_fort_slots: Vec<usize> = FORTRESS_ENTRIES_W1_7
            .iter()
            .filter(|&&(w, _)| w == world_idx)
            .map(|&(_, i)| i)
            .collect();
        let current_fort_count = current_fort_slots.len();

        if target_fort_count == current_fort_count {
            // Same count — just assign fortress data from pool to existing slots
            for &slot_idx in &current_fort_slots {
                let (ref fort_entry, _y_off) = fortress_pool[fortress_pool_idx];
                write_entry(rom, &WORLDS[world_idx], slot_idx, fort_entry);
                fortress_pool_idx += 1;
            }
        } else if target_fort_count > current_fort_count {
            // Need more fortress slots — convert some action levels to fortresses
            let extra_needed = target_fort_count - current_fort_count;
            let action_levels = collect_action_levels(rom, world_idx);

            // Pick action level slots to convert (take from end to minimize disruption)
            let slots_to_convert: Vec<usize> = action_levels
                .iter()
                .rev()
                .take(extra_needed)
                .copied()
                .collect();

            // Save displaced action level data and track the conversion
            for &slot_idx in &slots_to_convert {
                displaced_levels.push(read_entry(rom, &WORLDS[world_idx], slot_idx));
                became_fortress.push((world_idx, slot_idx));
            }

            // Write fortress data to all fortress slots (existing + converted)
            let all_fort_slots: Vec<usize> = current_fort_slots
                .iter()
                .chain(slots_to_convert.iter())
                .copied()
                .collect();

            for &slot_idx in &all_fort_slots {
                let (ref fort_entry, _y_off) = fortress_pool[fortress_pool_idx];
                write_entry(rom, &WORLDS[world_idx], slot_idx, fort_entry);
                fortress_pool_idx += 1;
            }
        } else {
            // Fewer fortresses needed — free some fortress slots for action levels
            let to_free = current_fort_count - target_fort_count;

            // Keep the first target_fort_count slots, free the rest
            let (keep_slots, free_slots) = current_fort_slots.split_at(target_fort_count);

            // Write fortress data to kept slots
            for &slot_idx in keep_slots {
                let (ref fort_entry, _y_off) = fortress_pool[fortress_pool_idx];
                write_entry(rom, &WORLDS[world_idx], slot_idx, fort_entry);
                fortress_pool_idx += 1;
            }

            // Mark freed slots and track the conversion
            for &slot_idx in free_slots {
                freed_slots.push((world_idx, slot_idx));
                became_level.push((world_idx, slot_idx));
            }
            let _ = to_free;
        }
    }

    assert_eq!(fortress_pool_idx, 13, "All 13 fortresses must be assigned");

    // Step 4: Fill freed fortress slots with displaced action levels
    // Shuffle displaced levels for randomness
    displaced_levels.as_mut_slice().shuffle(rng);
    assert_eq!(
        displaced_levels.len(),
        freed_slots.len(),
        "Displaced levels must match freed slots"
    );

    for (level_entry, &(w, i)) in displaced_levels.iter().zip(freed_slots.iter()) {
        write_entry(rom, &WORLDS[w], i, level_entry);
    }

    // Step 4b: Swap map tiles for slots that changed type.
    // Pair up became_fortress and became_level slots and swap their
    // pre-saved original tiles.
    assert_eq!(became_fortress.len(), became_level.len());
    for (bf, bl) in became_fortress.iter().zip(became_level.iter()) {
        let fort_orig_tile = original_tiles[bl]; // fortress slot's original tile
        let level_orig_tile = original_tiles[bf]; // action level slot's original tile

        // Write the fortress's original tile where the action level was
        let (row, col) = entry_grid_position(rom, &WORLDS[bf.0], bf.1);
        let tile_off = map_tile_offset(bf.0, row, col);
        rom.write_byte(tile_off, fort_orig_tile);

        // Write the action level's original tile where the fortress was
        let (row, col) = entry_grid_position(rom, &WORLDS[bl.0], bl.1);
        let tile_off = map_tile_offset(bl.0, row, col);
        rom.write_byte(tile_off, level_orig_tile);
    }

    // Step 5: Patch Boom-Boom Y-bytes
    // For each fortress in its new position, set the upper nibble to the
    // correct ordinal (1-based position within that world).
    let mut fortress_pool_idx = 0;
    for world_idx in 0..7 {
        let target_fort_count = new_counts[world_idx];
        for ordinal in 1..=target_fort_count {
            let (_entry, y_offset) = &fortress_pool[fortress_pool_idx];
            let old_y = rom.read_byte(*y_offset);
            let new_y = ((ordinal as u8) << 4) | (old_y & 0x0F);
            rom.write_byte(*y_offset, new_y);
            fortress_pool_idx += 1;
        }
    }

    // Step 6: Rewrite FortressFX_W1-W8 slot assignments
    // Each world gets 4 bytes at FORTRESS_FX_WORLD_TABLE + world_idx * 4.
    // We assign FX slots 0x00-0x0C sequentially to the new fortress positions.
    let mut fx_slot = 0u8;
    for world_idx in 0..7 {
        let base = FORTRESS_FX_WORLD_TABLE + world_idx * 4;
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
    // W8 stays untouched (slots 0x0D-0x10), but update FortressFXBase_ByWorld
    // for W1-7 to reflect the new layout. Each world's base = sum of previous
    // worlds' 4-byte blocks = world_idx * 4 (unchanged since the table is
    // always 4 entries per world).
    // Actually the base values don't change — they're always 4 apart.
    // The existing values (00, 04, 08, 0C, 10, 14, 18, 1C) are correct.
}

// ---------------------------------------------------------------------------
// FortressFX table offsets (17 slots each)
// ---------------------------------------------------------------------------
const FX_VADDR_H: usize = 0x147CD;
const FX_VADDR_L: usize = 0x147DE;
const FX_MAP_COMP_IDX: usize = 0x147EF; // 17 x 2 bytes
const FX_PATTERNS: usize = 0x14811;     // 17 x 4 bytes
const FX_MAP_LOC_ROW: usize = 0x14855;
const FX_MAP_LOC: usize = 0x14866;
const FX_MAP_TILE_REPLACE: usize = 0x14877;

/// Lock tile ID on the overworld map.
const TILE_LOCK: u8 = 0x54;

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
/// Writes VRAM addr, row, location, replacement tile, and lock patterns.
fn repoint_fx_slot(
    rom: &mut Rom,
    slot: usize,
    grid_row: usize,
    screen: usize,
    col_in_screen: usize,
    replace_tile: u8,
    comp_col: u8,
    comp_bit: u8,
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
    // Lock patterns: FE C0 FE C0
    let pat_off = FX_PATTERNS + slot * 4;
    rom.write_byte(pat_off, 0xFE);
    rom.write_byte(pat_off + 1, 0xC0);
    rom.write_byte(pat_off + 2, 0xFE);
    rom.write_byte(pat_off + 3, 0xC0);
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
    let offset = map_tile_offset(world_idx, grid_row, grid_col);
    let orig = rom.read_byte(offset);
    rom.write_byte(offset, TILE_LOCK);
    orig
}

/// Remove a lock tile, restoring the given path tile.
fn remove_lock(rom: &mut Rom, world_idx: usize, grid_row: usize, grid_col: usize, restore_tile: u8) {
    let offset = map_tile_offset(world_idx, grid_row, grid_col);
    rom.write_byte(offset, restore_tile);
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
        for off in FORTRESS_FX_WORLD_TABLE..FORTRESS_FX_WORLD_TABLE + 32 {
            assert_eq!(rom1.read_byte(off), rom2.read_byte(off));
        }

        // Check Y-bytes match
        for &off in &BOOMBOOM_Y_OFFSETS_W1_7 {
            assert_eq!(rom1.read_byte(off), rom2.read_byte(off));
        }
    }
}
