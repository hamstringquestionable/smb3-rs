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

/// Data that travels with a level when shuffled.
#[derive(Clone)]
struct LevelEntry {
    obj_lo: u8,
    obj_hi: u8,
    lay_lo: u8,
    lay_hi: u8,
    tileset: u8, // lower nibble of ByRowType
}

/// Returns true if this map entry is a regular action level that can be shuffled.
/// Excludes fortresses, toad houses, bonus games, hand traps, pipe junctions, etc.
fn is_shuffleable(obj_ptr: u16, lay_ptr: u16) -> bool {
    // Must be a real level pointer (banked at $C000+)
    // Must not be a fortress (obj >= $D000, fortress enemy data in higher banks)
    // Must not be empty/special (lay = $0000)
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
        obj_lo: rom.read_byte(obj_off),
        obj_hi: rom.read_byte(obj_off + 1),
        lay_lo: rom.read_byte(lay_off),
        lay_hi: rom.read_byte(lay_off + 1),
        tileset: rom.read_byte(world.rowtype_offset + idx) & 0x0F,
    }
}

/// Write a LevelEntry back to ROM for a given world and entry index.
fn write_entry(rom: &mut Rom, world: &WorldTables, idx: usize, entry: &LevelEntry) {
    let (_scrcol, objsets, layouts) = table_offsets(world);

    let obj_off = objsets + idx * 2;
    let lay_off = layouts + idx * 2;

    rom.write_byte(obj_off, entry.obj_lo);
    rom.write_byte(obj_off + 1, entry.obj_hi);
    rom.write_byte(lay_off, entry.lay_lo);
    rom.write_byte(lay_off + 1, entry.lay_hi);

    // Update tileset in ByRowType: preserve upper nibble, replace lower nibble
    let rowtype_off = world.rowtype_offset + idx;
    let old = rom.read_byte(rowtype_off);
    rom.write_byte(rowtype_off, (old & 0xF0) | (entry.tileset & 0x0F));
}

/// Shuffle levels within each world independently.
pub fn randomize_intra<R: Rng>(rom: &mut Rom, rng: &mut R) {
    for world in &WORLDS {
        let (_scrcol, objsets, layouts) = table_offsets(world);

        // Find shuffleable entry indices
        let mut shuffleable_indices: Vec<usize> = Vec::new();
        for i in 0..world.entry_count {
            let obj_ptr = read_word(rom, objsets + i * 2);
            let lay_ptr = read_word(rom, layouts + i * 2);
            if is_shuffleable(obj_ptr, lay_ptr) {
                shuffleable_indices.push(i);
            }
        }

        if shuffleable_indices.len() <= 1 {
            continue;
        }

        // Extract entries
        let mut entries: Vec<LevelEntry> = shuffleable_indices
            .iter()
            .map(|&i| read_entry(rom, world, i))
            .collect();

        // Shuffle
        entries.as_mut_slice().shuffle(rng);

        // Write back
        for (slot, &idx) in shuffleable_indices.iter().enumerate() {
            write_entry(rom, world, idx, &entries[slot]);
        }
    }
}

/// Shuffle levels across all worlds.
pub fn randomize_cross<R: Rng>(rom: &mut Rom, rng: &mut R) {
    // Collect all shuffleable entries across all worlds with their locations
    let mut all_indices: Vec<(usize, usize)> = Vec::new(); // (world_idx, entry_idx)

    for (w, world) in WORLDS.iter().enumerate() {
        let (_scrcol, objsets, layouts) = table_offsets(world);
        for i in 0..world.entry_count {
            let obj_ptr = read_word(rom, objsets + i * 2);
            let lay_ptr = read_word(rom, layouts + i * 2);
            if is_shuffleable(obj_ptr, lay_ptr) {
                all_indices.push((w, i));
            }
        }
    }

    if all_indices.len() <= 1 {
        return;
    }

    // Extract all entries
    let mut entries: Vec<LevelEntry> = all_indices
        .iter()
        .map(|&(w, i)| read_entry(rom, &WORLDS[w], i))
        .collect();

    // Shuffle
    entries.as_mut_slice().shuffle(rng);

    // Write back
    for (slot, &(w, idx)) in all_indices.iter().enumerate() {
        write_entry(rom, &WORLDS[w], idx, &entries[slot]);
    }
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

        // Set ByRowType: mix of tilesets
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
    fn test_tileset_follows_level() {
        let mut rom = make_test_rom();

        // Give entry 0 tileset 1 (Plains) and entry 1 tileset 3 (Hilly)
        let w = &WORLDS[0];
        rom.write_byte(w.rowtype_offset, 0x21); // upper=2, ts=1
        rom.write_byte(w.rowtype_offset + 1, 0x23); // upper=2, ts=3

        let (_scrcol, objsets, _layouts) = table_offsets(w);
        let entry0_obj = read_word(&rom, objsets);
        let entry1_obj = read_word(&rom, objsets + 2);

        // Force a known shuffle by using a seed that swaps entries 0 and 1
        // We just need to verify that after shuffle, wherever entry0's obj pointer
        // ends up, its tileset nibble follows it
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize_intra(&mut rom, &mut rng);

        // Find where entry0's obj pointer ended up
        for i in 0..w.entry_count {
            let obj = read_word(&rom, objsets + i * 2);
            if obj == entry0_obj {
                let ts = rom.read_byte(w.rowtype_offset + i) & 0x0F;
                assert_eq!(ts, 1, "Plains tileset should follow its level data");
            }
            if obj == entry1_obj {
                let ts = rom.read_byte(w.rowtype_offset + i) & 0x0F;
                assert_eq!(ts, 3, "Hilly tileset should follow its level data");
            }
        }
    }

    #[test]
    fn test_upper_nibble_preserved() {
        let mut rom = make_test_rom();
        let w = &WORLDS[0];

        // Set distinctive upper nibbles
        for i in 0..w.entry_count {
            let old = rom.read_byte(w.rowtype_offset + i);
            rom.write_byte(w.rowtype_offset + i, (((i as u8) & 0x0F) << 4) | (old & 0x0F));
        }

        // Record upper nibbles
        let original_upper: Vec<u8> = (0..w.entry_count)
            .map(|i| rom.read_byte(w.rowtype_offset + i) & 0xF0)
            .collect();

        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize_intra(&mut rom, &mut rng);

        // Upper nibbles should be unchanged at every position
        for i in 0..w.entry_count {
            let upper = rom.read_byte(w.rowtype_offset + i) & 0xF0;
            assert_eq!(upper, original_upper[i], "Upper nibble at entry {i} should be preserved");
        }
    }
}
