use rand::Rng;
use rand::seq::SliceRandom;

use crate::rom::Rom;

/// Per-world level table location in ROM.
struct WorldTables {
    /// File offset of the ByRowType sub-table for this world.
    rowtype_offset: usize,
    /// Number of entries (map positions) in this world.
    entry_count: usize,
}

/// All 8 worlds' table locations, derived from ROM analysis.
/// Each world has 4 contiguous sub-tables:
///   ByRowType (N bytes), ByScrCol (N bytes), ObjSets (N words), LevelLayouts (N words)
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
/// Level layout data lives in these banks.
const PAGE_A000_BY_TILESET: [usize; 19] = [
    11, 15, 21, 16, 17, 19, 18, 18, 18, 20, 23, 19, 17, 19, 13, 26, 26, 26, 9,
];

/// Data that travels with a level when shuffled.
///
/// Only the tileset (lower nibble of ByRowType) travels with the level.
/// The upper nibble of ByRowType is the map row position — part of the
/// game's lookup key (ByRowType + ByScrCol) — and must stay at its
/// original slot so the game matches the correct entry when Mario
/// steps on a map tile.
#[derive(Clone)]
struct LevelEntry {
    tileset: u8,
    obj_lo: u8,
    obj_hi: u8,
    lay_lo: u8,
    lay_hi: u8,
}

/// Convert a layout CPU address ($A000-$BFFF) + tileset to a ROM file offset.
fn layout_file_offset(cpu_addr: u16, tileset: u8) -> Option<usize> {
    if tileset as usize >= PAGE_A000_BY_TILESET.len() || cpu_addr < 0xA000 {
        return None;
    }
    let bank = PAGE_A000_BY_TILESET[tileset as usize];
    Some(bank * 0x2000 + 0x10 + (cpu_addr as usize - 0xA000))
}

/// Read the screen count from a level's 9-byte header.
/// Header byte 4, bits 3-0 = (num_screens - 1).
fn level_screen_count(rom: &Rom, layout_offset: usize) -> u8 {
    (rom.read_byte(layout_offset + 4) & 0x0F) + 1
}

/// Returns true if this map entry's pointer range indicates a real level
/// (not a fortress, toad house, bonus game, hand trap, or pipe junction).
fn is_level_pointer(obj_ptr: u16, lay_ptr: u16) -> bool {
    obj_ptr >= 0xC000 && obj_ptr < 0xD000 && lay_ptr != 0x0000
}

/// Compute sub-table file offsets for a world.
fn table_offsets(world: &WorldTables) -> (usize, usize, usize) {
    let n = world.entry_count;
    let scrcol = world.rowtype_offset + n;
    let objsets = scrcol + n;
    let layouts = objsets + n * 2;
    (scrcol, objsets, layouts)
}

/// Read a 16-bit little-endian word from ROM.
fn read_word(rom: &Rom, offset: usize) -> u16 {
    let lo = rom.read_byte(offset) as u16;
    let hi = rom.read_byte(offset + 1) as u16;
    (hi << 8) | lo
}

/// Read a LevelEntry from ROM for a given world and entry index.
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

/// Write a LevelEntry back to ROM for a given world and entry index.
/// Only the tileset (lower nibble of ByRowType) is updated — the upper
/// nibble (map row position) is preserved so the game's lookup key
/// remains correct for this map tile.
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

/// Collect the indices of entries that are real action levels for a given world.
/// Excludes fortresses, toad houses, bonus games, hammer bros, pipe connectors, etc.
///
/// An entry is a real action level if:
/// 1. Its obj pointer is in $C000-$CFFF (not fortress/special)
/// 2. Its layout pointer is non-zero
/// 3. Its (obj, lay) pair is unique within the world (excludes hammer bros)
/// 4. Its layout has 3+ screens (excludes pipe connectors and small arenas)
fn collect_shuffleable(rom: &Rom, world: &WorldTables) -> Vec<usize> {
    let (_scrcol, objsets, layouts) = table_offsets(world);

    // First pass: count (obj, lay) pair occurrences to detect duplicates
    let mut pair_counts = std::collections::HashMap::new();
    for i in 0..world.entry_count {
        let obj_ptr = read_word(rom, objsets + i * 2);
        let lay_ptr = read_word(rom, layouts + i * 2);
        if is_level_pointer(obj_ptr, lay_ptr) {
            *pair_counts.entry((obj_ptr, lay_ptr)).or_insert(0u32) += 1;
        }
    }

    // Second pass: collect entries that pass all filters
    let mut indices = Vec::new();
    for i in 0..world.entry_count {
        let obj_ptr = read_word(rom, objsets + i * 2);
        let lay_ptr = read_word(rom, layouts + i * 2);
        if !is_level_pointer(obj_ptr, lay_ptr) {
            continue;
        }

        // Exclude duplicate (obj, lay) pairs (hammer bros, etc.)
        if pair_counts[&(obj_ptr, lay_ptr)] > 1 {
            continue;
        }

        // Exclude short levels (pipe connectors, small arenas)
        let tileset = rom.read_byte(world.rowtype_offset + i) & 0x0F;
        if let Some(lay_offset) = layout_file_offset(lay_ptr, tileset) {
            if level_screen_count(rom, lay_offset) < 3 {
                continue;
            }
        } else {
            continue; // Can't resolve layout — skip
        }

        indices.push(i);
    }
    indices
}

/// Shuffle a group of entries identified by (world_idx, entry_idx) pairs.
fn shuffle_group<R: Rng>(rom: &mut Rom, rng: &mut R, indices: &[(usize, usize)]) {
    if indices.len() <= 1 {
        return;
    }

    let mut entries: Vec<LevelEntry> = indices
        .iter()
        .map(|&(w, i)| read_entry(rom, &WORLDS[w], i))
        .collect();

    entries.as_mut_slice().shuffle(rng);

    for (slot, &(w, idx)) in indices.iter().enumerate() {
        write_entry(rom, &WORLDS[w], idx, &entries[slot]);
    }
}

/// Shuffle levels within each world independently.
/// All shuffleable levels within a world are shuffled together regardless
/// of tileset — the ByRowType byte (which contains the tileset) is swapped
/// along with the level data so the game's lookup key stays consistent.
pub fn randomize_intra<R: Rng>(rom: &mut Rom, rng: &mut R) {
    for (w, world) in WORLDS.iter().enumerate() {
        let shuffleable = collect_shuffleable(rom, world);
        let indices: Vec<(usize, usize)> = shuffleable.iter().map(|&i| (w, i)).collect();
        shuffle_group(rom, rng, &indices);
    }
}

/// Shuffle levels across all worlds.
/// All shuffleable levels across all worlds are collected into a single
/// pool and shuffled together. The ByRowType byte (including tileset)
/// travels with each level so the game's lookup key stays consistent.
pub fn randomize_cross<R: Rng>(rom: &mut Rom, rng: &mut R) {
    let mut all_indices: Vec<(usize, usize)> = Vec::new();

    for (w, world) in WORLDS.iter().enumerate() {
        let shuffleable = collect_shuffleable(rom, world);
        for i in shuffleable {
            all_indices.push((w, i));
        }
    }

    shuffle_group(rom, rng, &all_indices);
}


#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha8Rng;
    use rand::SeedableRng;

    fn make_test_rom() -> Rom {
        let mut data = vec![0u8; 393232];
        // iNES header
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;

        // Populate World 1 tables with test data
        let w = &WORLDS[0];
        let n = w.entry_count;
        let (_scrcol, objsets, layouts) = table_offsets(w);

        // Set ByRowType: tileset=1 (Plains), upper nibble=2
        // Tileset 1 -> PAGE_A000_BY_TILESET[1] = bank 15
        // Layout CPU $B000 -> file offset = 15 * 0x2000 + 0x10 + ($B000 - $A000)
        //                   = 0x1E010 + 0x1000 = 0x1F010
        for i in 0..n {
            data[w.rowtype_offset + i] = 0x21; // upper nibble=2, tileset=1 (Plains)
        }

        // Set all ObjSets and Layouts to shuffleable values by default
        for i in 0..n {
            let obj_off = objsets + i * 2;
            let lay_off = layouts + i * 2;
            // Unique obj/lay per entry so we can verify shuffle
            let obj_val: u16 = 0xC000 + (i as u16) * 0x10;
            let lay_val: u16 = 0xB000 + (i as u16) * 0x10;
            data[obj_off] = (obj_val & 0xFF) as u8;
            data[obj_off + 1] = ((obj_val >> 8) & 0xFF) as u8;
            data[lay_off] = (lay_val & 0xFF) as u8;
            data[lay_off + 1] = ((lay_val >> 8) & 0xFF) as u8;

            // Write a fake level header at the layout file offset so
            // level_screen_count returns >= 3 (making it shuffleable).
            // Header byte 4 bits 3-0 = (screens - 1), so 0x07 = 8 screens.
            let bank = PAGE_A000_BY_TILESET[1]; // tileset 1
            let file_off = bank * 0x2000 + 0x10 + (lay_val as usize - 0xA000);
            if file_off + 9 < data.len() {
                data[file_off + 4] = 0x07; // 8 screens
            }
        }

        // Make entry 9 a toad house (non-shuffleable)
        let obj_off9 = objsets + 9 * 2;
        data[obj_off9] = 0x00;
        data[obj_off9 + 1] = 0x07; // obj = 0x0700

        // Make entry 11 a fortress (non-shuffleable)
        let obj_off11 = objsets + 11 * 2;
        data[obj_off11] = 0x00;
        data[obj_off11 + 1] = 0xD0; // obj = 0xD000

        // Make entry 12 a bonus/special (non-shuffleable)
        let obj_off12 = objsets + 12 * 2;
        let lay_off12 = layouts + 12 * 2;
        data[obj_off12] = 0x01;
        data[obj_off12 + 1] = 0x00; // obj = 0x0001
        data[lay_off12] = 0x00;
        data[lay_off12 + 1] = 0x00; // lay = 0x0000

        Rom::from_bytes(&data).unwrap()
    }

    #[test]
    fn test_non_shuffleable_entries_preserved() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let w = &WORLDS[0];
        let (_scrcol, objsets, layouts) = table_offsets(w);

        // Record non-shuffleable entries
        let toad_obj = read_word(&rom, objsets + 9 * 2);
        let fortress_obj = read_word(&rom, objsets + 11 * 2);
        let bonus_obj = read_word(&rom, objsets + 12 * 2);
        let bonus_lay = read_word(&rom, layouts + 12 * 2);

        randomize_intra(&mut rom, &mut rng);

        // Verify non-shuffleable entries unchanged
        assert_eq!(read_word(&rom, objsets + 9 * 2), toad_obj, "Toad house should be unchanged");
        assert_eq!(read_word(&rom, objsets + 11 * 2), fortress_obj, "Fortress should be unchanged");
        assert_eq!(read_word(&rom, objsets + 12 * 2), bonus_obj, "Bonus should be unchanged");
        assert_eq!(read_word(&rom, layouts + 12 * 2), bonus_lay, "Bonus layout should be unchanged");
    }

    #[test]
    fn test_hammer_bros_excluded() {
        let mut rom = make_test_rom();
        let w = &WORLDS[0];
        let (_scrcol, objsets, layouts) = table_offsets(w);

        // Make entries 13 and 14 share the same (obj, lay) pair = hammer bros
        let obj_off13 = objsets + 13 * 2;
        let obj_off14 = objsets + 14 * 2;
        let lay_off13 = layouts + 13 * 2;
        let lay_off14 = layouts + 14 * 2;
        // Both point to obj=0xC640 lay=0xB3E7
        rom.write_byte(obj_off13, 0x40); rom.write_byte(obj_off13 + 1, 0xC6);
        rom.write_byte(obj_off14, 0x40); rom.write_byte(obj_off14 + 1, 0xC6);
        rom.write_byte(lay_off13, 0xE7); rom.write_byte(lay_off13 + 1, 0xB3);
        rom.write_byte(lay_off14, 0xE7); rom.write_byte(lay_off14 + 1, 0xB3);

        let indices = collect_shuffleable(&rom, w);
        assert!(!indices.contains(&13), "Hammer bro entry 13 should be excluded");
        assert!(!indices.contains(&14), "Hammer bro entry 14 should be excluded");
    }

    #[test]
    fn test_pipe_connectors_excluded() {
        let mut rom = make_test_rom();
        let w = &WORLDS[0];
        let (_scrcol, _objsets, layouts) = table_offsets(w);

        // Make entry 15 a 1-screen level (pipe connector)
        let lay_val = read_word(&rom, layouts + 15 * 2);
        let tileset = 1u8;
        let bank = PAGE_A000_BY_TILESET[tileset as usize];
        let file_off = bank * 0x2000 + 0x10 + (lay_val as usize - 0xA000);
        // Set screen count to 1: header byte 4 = 0x00 (bits 3-0 = 0, so 1 screen)
        rom.write_byte(file_off + 4, 0x00);

        let indices = collect_shuffleable(&rom, w);
        assert!(!indices.contains(&15), "1-screen pipe connector should be excluded");
    }

    #[test]
    fn test_intra_world_shuffle_changes_data() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let w = &WORLDS[0];
        let (_scrcol, objsets, _layouts) = table_offsets(w);

        // Record original shuffleable entries
        let original: Vec<u16> = (0..w.entry_count)
            .map(|i| read_word(&rom, objsets + i * 2))
            .collect();

        randomize_intra(&mut rom, &mut rng);

        let shuffled: Vec<u16> = (0..w.entry_count)
            .map(|i| read_word(&rom, objsets + i * 2))
            .collect();

        // At least some shuffleable entries should have changed
        let changed = original.iter().zip(shuffled.iter())
            .enumerate()
            .filter(|&(i, (a, b))| a != b && i != 9 && i != 11 && i != 12)
            .count();
        assert!(changed > 0, "Shuffle should change at least some entries");
    }

    #[test]
    fn test_deterministic() {
        let mut rom1 = make_test_rom();
        let mut rom2 = make_test_rom();
        let mut rng1 = ChaCha8Rng::seed_from_u64(99);
        let mut rng2 = ChaCha8Rng::seed_from_u64(99);

        randomize_intra(&mut rom1, &mut rng1);
        randomize_intra(&mut rom2, &mut rng2);

        let w = &WORLDS[0];
        let (_scrcol, objsets, layouts_off) = table_offsets(w);
        let len = w.entry_count * 2;
        assert_eq!(rom1.read_range(objsets, len), rom2.read_range(objsets, len));
        assert_eq!(rom1.read_range(layouts_off, len), rom2.read_range(layouts_off, len));
    }

    #[test]
    fn test_byrowtype_upper_nibble_preserved_and_tileset_travels() {
        let mut rom = make_test_rom();
        let w = &WORLDS[0];
        let (_scrcol, objsets, layouts) = table_offsets(w);

        // Give entries 0-4 tileset 1 with varying upper nibbles,
        // entries 5-7 tileset 3 with varying upper nibbles
        for i in 0..5 {
            let upper = ((i as u8 + 2) << 4) & 0xF0; // rows 2,3,4,5,6
            rom.write_byte(w.rowtype_offset + i, upper | 0x01); // ts=1
        }
        for i in 5..9 {
            let upper = ((i as u8 + 2) << 4) & 0xF0; // rows 7,8,9,A
            rom.write_byte(w.rowtype_offset + i, upper | 0x03); // ts=3
            // Write layout headers in the TS3 bank so screen count check passes
            let lay_val = read_word(&rom, layouts + i * 2);
            let bank = PAGE_A000_BY_TILESET[3];
            let file_off = bank * 0x2000 + 0x10 + (lay_val as usize - 0xA000);
            if file_off + 9 < 393232 {
                rom.write_byte(file_off + 4, 0x07); // 8 screens
            }
        }

        // Record original upper nibbles and (obj -> tileset) mapping
        let shuffleable = collect_shuffleable(&rom, w);
        let original_upper: Vec<u8> = shuffleable
            .iter()
            .map(|&i| rom.read_byte(w.rowtype_offset + i) & 0xF0)
            .collect();
        let mut obj_to_tileset: std::collections::HashMap<u16, u8> =
            std::collections::HashMap::new();
        for &i in &shuffleable {
            let obj = read_word(&rom, objsets + i * 2);
            let ts = rom.read_byte(w.rowtype_offset + i) & 0x0F;
            obj_to_tileset.insert(obj, ts);
        }

        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize_intra(&mut rom, &mut rng);

        // Verify upper nibble (row position) is preserved at each slot
        for (slot, &i) in shuffleable.iter().enumerate() {
            let brt = rom.read_byte(w.rowtype_offset + i);
            let upper = brt & 0xF0;
            assert_eq!(upper, original_upper[slot],
                "Entry {i}: upper nibble changed from 0x{:02X} to 0x{:02X}",
                original_upper[slot], upper);
        }

        // Verify tileset (lower nibble) traveled with the obj pointer
        for &i in &shuffleable {
            let obj = read_word(&rom, objsets + i * 2);
            let ts = rom.read_byte(w.rowtype_offset + i) & 0x0F;
            assert_eq!(ts, obj_to_tileset[&obj],
                "Entry {i}: tileset {ts} doesn't match original tileset {} for obj 0x{obj:04X}",
                obj_to_tileset[&obj]);
        }
    }

    #[test]
    fn test_cross_tileset_shuffle_allowed() {
        let mut rom = make_test_rom();
        let w = &WORLDS[0];
        let (_scrcol, _objsets, layouts) = table_offsets(w);

        // Give entries 0-4 tileset 1, entries 5-7 tileset 3
        for i in 0..5 {
            rom.write_byte(w.rowtype_offset + i, 0x21); // ts=1
        }
        for i in 5..9 {
            rom.write_byte(w.rowtype_offset + i, 0x23); // ts=3
            // Write layout headers in the TS3 bank so screen count check passes
            let lay_val = read_word(&rom, layouts + i * 2);
            let bank = PAGE_A000_BY_TILESET[3];
            let file_off = bank * 0x2000 + 0x10 + (lay_val as usize - 0xA000);
            if file_off + 9 < 393232 {
                rom.write_byte(file_off + 4, 0x07); // 8 screens
            }
        }

        // Record original tileset assignments for shuffleable entries
        let shuffleable = collect_shuffleable(&rom, w);
        let original_tilesets: Vec<u8> = shuffleable
            .iter()
            .map(|&i| rom.read_byte(w.rowtype_offset + i) & 0x0F)
            .collect();

        // Shuffle many times to see if cross-tileset swaps happen
        let mut cross_tileset_swap = false;
        for seed in 0..20u64 {
            let mut rom2 = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize_intra(&mut rom2, &mut rng);

            let new_tilesets: Vec<u8> = shuffleable
                .iter()
                .map(|&i| rom2.read_byte(w.rowtype_offset + i) & 0x0F)
                .collect();

            if original_tilesets != new_tilesets {
                cross_tileset_swap = true;
                break;
            }
        }
        assert!(cross_tileset_swap,
            "Expected cross-tileset shuffling to occur in at least one of 20 seeds");
    }
}
