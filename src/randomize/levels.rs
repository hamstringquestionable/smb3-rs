use rand::Rng;
use rand::seq::SliceRandom;

use crate::rom::Rom;

use super::rom_data::{
    self, AIRSHIP_ENTRIES, FORTRESS_ENTRIES, MAP_TRANSITIONS, TILE_SPIRAL, WORLDS,
    LevelEntry, WorldTables,
    entry_grid_position, is_level_pointer, layout_file_offset, level_screen_count,
    map_tile_offset, read_entry, read_word, write_entry,
};

/// Collect the indices of entries that are real action levels for a given world.
/// Excludes fortresses, boss levels, toad houses, bonus games, hammer bros,
/// pipe connectors, airships, etc.
///
/// An entry is a real action level if:
/// 1. Its obj pointer >= $C000 and layout pointer is non-zero
/// 2. Its (obj, lay) pair is unique within the world (excludes hammer bros)
/// 3. Its enemy data does not contain boss enemies (excludes fortresses/bosses)
/// 4. Its layout has 3+ screens (excludes pipe connectors and small arenas)
/// 5. It is not an airship entry (autoscroll patch overwrites these slots)
/// 6. It is not a map transition entry (structural map region transition)
fn collect_shuffleable(rom: &Rom, world_idx: usize, world: &WorldTables) -> Vec<usize> {
    let (_scrcol, objsets, layouts) = rom_data::table_offsets(world);

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

        // Exclude airship entries (autoscroll patch overwrites these slots)
        if AIRSHIP_ENTRIES.contains(&(world_idx, i)) {
            continue;
        }

        // Exclude map transition entries (structural map region transitions)
        if MAP_TRANSITIONS.contains(&(world_idx, i)) {
            continue;
        }

        // Exclude spiral castle (W5 screen connector, not a playable level)
        let (row, col) = entry_grid_position(rom, world, i);
        if rom.read_byte(map_tile_offset(world_idx, row, col)) == TILE_SPIRAL {
            continue;
        }

        // Exclude duplicate (obj, lay) pairs (hammer bros, etc.)
        if pair_counts[&(obj_ptr, lay_ptr)] > 1 {
            continue;
        }

        // Exclude fortress and boss levels
        if FORTRESS_ENTRIES.contains(&(world_idx, i)) {
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
        let shuffleable = collect_shuffleable(rom, w, world);
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
        let shuffleable = collect_shuffleable(rom, w, world);
        for i in shuffleable {
            all_indices.push((w, i));
        }
    }

    shuffle_group(rom, rng, &all_indices);
}

/// Collect fortress entries: all 17 levels containing Boom-Boom boss fights.
/// Uses the hardcoded FORTRESS_ENTRIES constant derived from rom_map.py analysis.
/// Excludes Bowser's castle (W8[40]) which must stay at its map position.
fn collect_fortresses(_rom: &Rom) -> Vec<(usize, usize)> {
    FORTRESS_ENTRIES.to_vec()
}

/// ROM file offset of the Boom-Boom Y-byte for each fortress (ordered same
/// as FORTRESS_ENTRIES). The Y-byte's upper nibble encodes the fortress
/// ordinal (1-based) used as Map_DoFortressFX, and the lower nibble is
/// Boom-Boom's spawn Y position. All offsets are in PRG006 enemy data.
const BOOMBOOM_Y_OFFSETS: [usize; 17] = [
    0x0D35F, // W1[11]  Y=$17
    0x0D262, // W2[13]  Y=$18
    0x0D3D3, // W3[13]  Y=$17
    0x0D3A1, // W3[34]  Y=$28
    0x0D536, // W4[ 9]  Y=$27
    0x0D55F, // W4[16]  Y=$17
    0x0D40F, // W5[12]  Y=$17
    0x0D2C7, // W5[31]  Y=$27
    0x0D4E1, // W6[ 9]  Y=$17
    0x0CAE1, // W6[27]  Y=$24
    0x0D4B0, // W6[48]  Y=$37
    0x0D4FA, // W7[ 5]  Y=$17
    0x0D47E, // W7[40]  Y=$27
    0x0DA32, // W8[ 7]  Y=$17
    0x0DA37, // W8[10]  Y=$27
    0x0D597, // W8[26]  Y=$38
    0x0DA2D, // W8[36]  Y=$47
];

/// Original fortress ordinal (Map_DoFortressFX value) for each position
/// in FORTRESS_ENTRIES. Extracted from the original Y-byte upper nibbles.
/// Note: W4 is [2, 1] not [1, 2] — entry W4[9] is the game's "second"
/// fortress despite appearing earlier in the pointer table.
const POSITION_ORDINALS: [u8; 17] = [
    1,    // W1[11]
    1,    // W2[13]
    1, 2, // W3[13], W3[34]
    2, 1, // W4[ 9], W4[16]  — reversed!
    1, 2, // W5[12], W5[31]
    1, 2, 3, // W6[ 9], W6[27], W6[48]
    1, 2, // W7[ 5], W7[40]
    1, 2, 3, 4, // W8[ 7], W8[10], W8[26], W8[36]
];

/// Shuffle fortresses across all worlds. Any fortress can appear in any
/// fortress map slot (except Bowser's castle which stays fixed).
///
/// After shuffling, patches each Boom-Boom's Y-byte upper nibble so it
/// produces the correct Map_DoFortressFX value for its new map position.
/// The upper nibble is the 1-based fortress ordinal within the world;
/// the lower nibble (Boom-Boom's spawn Y position) is preserved.
pub fn randomize_fortresses<R: Rng>(rom: &mut Rom, rng: &mut R) {
    let indices = collect_fortresses(rom);

    // Record original obj_ptr for each fortress so we can identify
    // which original level ended up at each position after shuffling.
    let original_obj_ptrs: Vec<u16> = indices
        .iter()
        .map(|&(w, i)| {
            let entry = read_entry(rom, &WORLDS[w], i);
            u16::from_le_bytes([entry.obj_lo, entry.obj_hi])
        })
        .collect();

    shuffle_group(rom, rng, &indices);

    // For each fortress position, find which original fortress landed
    // there (by matching obj_ptr) and patch its Boom-Boom Y-byte.
    for (position_idx, &(w, i)) in indices.iter().enumerate() {
        let shuffled = read_entry(rom, &WORLDS[w], i);
        let shuffled_obj = u16::from_le_bytes([shuffled.obj_lo, shuffled.obj_hi]);

        // Find which original fortress this is
        let orig_idx = original_obj_ptrs
            .iter()
            .position(|&p| p == shuffled_obj)
            .expect("shuffled fortress must match an original");

        // Look up the required ordinal for this position
        let ordinal_in_world = POSITION_ORDINALS[position_idx];

        // Patch the Boom-Boom Y-byte: replace upper nibble, keep lower
        let y_offset = BOOMBOOM_Y_OFFSETS[orig_idx];
        let old_y = rom.read_byte(y_offset);
        let new_y = (ordinal_in_world << 4) | (old_y & 0x0F);
        rom.write_byte(y_offset, new_y);
    }
}

/// Shuffle airships across worlds 1-7. Each world's airship map tile
/// can load any of the 7 airship levels.
///
/// Note: when autoscroll is disabled, the autoscroll patch overwrites
/// airship pointer entries with world-specific redesigned data after
/// this shuffle runs, so airship shuffle only has a visible effect
/// when autoscroll is kept enabled.
pub fn randomize_airships<R: Rng>(rom: &mut Rom, rng: &mut R) {
    let indices: Vec<(usize, usize)> = AIRSHIP_ENTRIES.to_vec();
    shuffle_group(rom, rng, &indices);
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::randomize::rom_data::PAGE_A000_BY_TILESET;
    use rand_chacha::ChaCha8Rng;
    use rand::SeedableRng;

    /// File offset of the start of enemy/object data in PRG006 (test helper).
    const ENEMY_DATA_BASE: usize = 0x0C010;
    /// CPU base address for enemy data bank (test helper).
    const ENEMY_DATA_CPU_BASE: u16 = 0xC000;
    /// Bowser's castle entry (test helper).
    const BOWSER_CASTLE: (usize, usize) = (7, 40);

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
        let (_scrcol, objsets, layouts) = rom_data::table_offsets(w);

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

        // Entry 11 is a fortress — excluded via FORTRESS_ENTRIES constant (W1[11])

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
        let (_scrcol, objsets, layouts) = rom_data::table_offsets(w);

        // Record non-shuffleable entries
        let toad_obj = read_word(&rom, objsets + 9 * 2);
        let toad_lay = read_word(&rom, layouts + 9 * 2);
        let fortress_obj = read_word(&rom, objsets + 11 * 2);
        let fortress_lay = read_word(&rom, layouts + 11 * 2);
        let bonus_obj = read_word(&rom, objsets + 12 * 2);
        let bonus_lay = read_word(&rom, layouts + 12 * 2);

        randomize_intra(&mut rom, &mut rng);

        // Verify non-shuffleable entries unchanged
        assert_eq!(read_word(&rom, objsets + 9 * 2), toad_obj, "Toad house obj should be unchanged");
        assert_eq!(read_word(&rom, layouts + 9 * 2), toad_lay, "Toad house lay should be unchanged");
        assert_eq!(read_word(&rom, objsets + 11 * 2), fortress_obj, "Fortress obj should be unchanged");
        assert_eq!(read_word(&rom, layouts + 11 * 2), fortress_lay, "Fortress lay should be unchanged");
        assert_eq!(read_word(&rom, objsets + 12 * 2), bonus_obj, "Bonus obj should be unchanged");
        assert_eq!(read_word(&rom, layouts + 12 * 2), bonus_lay, "Bonus lay should be unchanged");
    }

    #[test]
    fn test_hammer_bros_excluded() {
        let mut rom = make_test_rom();
        let w = &WORLDS[0];
        let (_scrcol, objsets, layouts) = rom_data::table_offsets(w);

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

        let indices = collect_shuffleable(&rom, 0, w);
        assert!(!indices.contains(&13), "Hammer bro entry 13 should be excluded");
        assert!(!indices.contains(&14), "Hammer bro entry 14 should be excluded");
    }

    #[test]
    fn test_pipe_connectors_excluded() {
        let mut rom = make_test_rom();
        let w = &WORLDS[0];
        let (_scrcol, _objsets, layouts) = rom_data::table_offsets(w);

        // Make entry 15 a 1-screen level (pipe connector)
        let lay_val = read_word(&rom, layouts + 15 * 2);
        let tileset = 1u8;
        let bank = PAGE_A000_BY_TILESET[tileset as usize];
        let file_off = bank * 0x2000 + 0x10 + (lay_val as usize - 0xA000);
        // Set screen count to 1: header byte 4 = 0x00 (bits 3-0 = 0, so 1 screen)
        rom.write_byte(file_off + 4, 0x00);

        let indices = collect_shuffleable(&rom, 0, w);
        assert!(!indices.contains(&15), "1-screen pipe connector should be excluded");
    }

    #[test]
    fn test_intra_world_shuffle_changes_data() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let w = &WORLDS[0];
        let (_scrcol, objsets, _layouts) = rom_data::table_offsets(w);

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
        let (_scrcol, objsets, layouts_off) = rom_data::table_offsets(w);
        let len = w.entry_count * 2;
        assert_eq!(rom1.read_range(objsets, len), rom2.read_range(objsets, len));
        assert_eq!(rom1.read_range(layouts_off, len), rom2.read_range(layouts_off, len));
    }

    #[test]
    fn test_byrowtype_upper_nibble_preserved_and_tileset_travels() {
        let mut rom = make_test_rom();
        let w = &WORLDS[0];
        let (_scrcol, objsets, layouts) = rom_data::table_offsets(w);

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
        let shuffleable = collect_shuffleable(&rom, 0, w);
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
        let (_scrcol, _objsets, layouts) = rom_data::table_offsets(w);

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
        let shuffleable = collect_shuffleable(&rom, 0, w);
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

    /// Helper: set up a fortress entry with a Boom-Boom boss at a given
    /// world/index with a unique obj pointer.
    fn setup_fortress(data: &mut [u8], world_idx: usize, entry_idx: usize, obj_val: u16, lay_val: u16) {
        let w = &WORLDS[world_idx];
        let n = w.entry_count;
        let scrcol = w.rowtype_offset + n;
        let objsets = scrcol + n;
        let layouts = objsets + n * 2;

        let obj_off = objsets + entry_idx * 2;
        let lay_off = layouts + entry_idx * 2;
        data[obj_off] = (obj_val & 0xFF) as u8;
        data[obj_off + 1] = ((obj_val >> 8) & 0xFF) as u8;
        data[lay_off] = (lay_val & 0xFF) as u8;
        data[lay_off + 1] = ((lay_val >> 8) & 0xFF) as u8;

        // Set tileset 2 (fortress) in ByRowType, preserve upper nibble
        let old_brt = data[w.rowtype_offset + entry_idx];
        data[w.rowtype_offset + entry_idx] = (old_brt & 0xF0) | 0x02;

        // Write enemy data with Boom-Boom
        let enemy_off = ENEMY_DATA_BASE + (obj_val as usize - ENEMY_DATA_CPU_BASE as usize);
        data[enemy_off] = 0x01;     // page flag
        data[enemy_off + 1] = 0x4B; // OBJ_BOOMBOOMJUMP
        data[enemy_off + 2] = 0x50;
        data[enemy_off + 3] = 0x18;
        data[enemy_off + 4] = 0xFF; // terminator
    }

    fn make_fortress_test_rom() -> Rom {
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;

        // Initialize all worlds with valid but non-level entries by default
        for w_idx in 0..8 {
            let w = &WORLDS[w_idx];
            let n = w.entry_count;
            let scrcol = w.rowtype_offset + n;
            let objsets = scrcol + n;
            let layouts = objsets + n * 2;

            for i in 0..n {
                // Default: special entry (obj=0x0300, won't be detected)
                let obj_off = objsets + i * 2;
                let lay_off = layouts + i * 2;
                data[obj_off] = 0x00;
                data[obj_off + 1] = 0x03;
                data[lay_off] = 0x00;
                data[lay_off + 1] = 0x00;
                data[w.rowtype_offset + i] = ((i as u8) << 4) | 0x01;
            }
        }

        // Set up all 17 fortress entries with unique obj/lay pointers
        for (i, &(w_idx, entry_idx)) in FORTRESS_ENTRIES.iter().enumerate() {
            let obj_val: u16 = 0xC100 + (i as u16) * 0x10;
            let lay_val: u16 = 0xA100 + (i as u16) * 0x10;
            setup_fortress(&mut data, w_idx, entry_idx, obj_val, lay_val);
        }

        // Write Boom-Boom Y-byte values at all BOOMBOOM_Y_OFFSETS.
        // Each gets its original ordinal as upper nibble and a unique lower
        // nibble (0x7 + fortress_index) so we can verify preservation.
        // Original ordinals per position: W1=1, W2=1, W3=1,2, W4=2,1,
        // W5=1,2, W6=1,2,3, W7=1,2, W8=1,2,3,4
        let original_ordinals: [u8; 17] = [
            1, 1, 1, 2, 2, 1, 1, 2, 1, 2, 3, 1, 2, 1, 2, 3, 4,
        ];
        for (i, &offset) in BOOMBOOM_Y_OFFSETS.iter().enumerate() {
            let lower = i as u8 & 0x0F; // unique lower nibble per fortress
            let y_byte = (original_ordinals[i] << 4) | lower;
            data[offset] = y_byte;
        }

        // Set up Bowser's castle at W8[40] — should NOT be shuffled
        setup_fortress(&mut data, 7, 40, 0xC400, 0xA400);
        // Use Bowser boss ID instead of Boom-Boom
        let bowser_off = ENEMY_DATA_BASE + (0xC400u16 as usize - ENEMY_DATA_CPU_BASE as usize);
        data[bowser_off + 1] = 0x18; // OBJ_BOSS_BOWSER

        // Set up airship entries at the known indices with unique lay pointers
        for &(w_idx, entry_idx) in AIRSHIP_ENTRIES.iter() {
            let w = &WORLDS[w_idx];
            let n = w.entry_count;
            let scrcol = w.rowtype_offset + n;
            let objsets = scrcol + n;
            let layouts = objsets + n * 2;

            let obj_off = objsets + entry_idx * 2;
            let lay_off = layouts + entry_idx * 2;
            // All airships share obj=0xD2AF
            data[obj_off] = 0xAF;
            data[obj_off + 1] = 0xD2;
            // Unique lay per airship
            let lay_val: u16 = 0xA800 + (w_idx as u16) * 0x10;
            data[lay_off] = (lay_val & 0xFF) as u8;
            data[lay_off + 1] = ((lay_val >> 8) & 0xFF) as u8;
            // Set tileset 2
            data[w.rowtype_offset + entry_idx] = (data[w.rowtype_offset + entry_idx] & 0xF0) | 0x02;

            // Write enemy data at obj=0xD2AF (shared, only needs to be done once)
            let enemy_off = ENEMY_DATA_BASE + (0xD2AFusize - ENEMY_DATA_CPU_BASE as usize);
            data[enemy_off] = 0x01;
            data[enemy_off + 1] = 0xD5; // Koopaling battle object (not a boss ID)
            data[enemy_off + 2] = 0x10;
            data[enemy_off + 3] = 0x08;
            data[enemy_off + 4] = 0xFF;
        }

        Rom::from_bytes(&data).unwrap()
    }

    #[test]
    fn test_fortress_shuffle() {
        let mut rom = make_fortress_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        // collect_fortresses returns the hardcoded FORTRESS_ENTRIES (17 entries)
        let fortresses = collect_fortresses(&rom);
        assert_eq!(fortresses.len(), 17, "Expected 17 fortress entries");
        assert!(!fortresses.contains(&BOWSER_CASTLE), "Bowser should be excluded");

        // Record original obj pointers and Y-byte lower nibbles
        let original_objs: Vec<u16> = fortresses.iter().map(|&(w, i)| {
            let world = &WORLDS[w];
            let (_scrcol, objsets, _layouts) = rom_data::table_offsets(world);
            read_word(&rom, objsets + i * 2)
        }).collect();
        let original_y_lowers: Vec<u8> = BOOMBOOM_Y_OFFSETS.iter()
            .map(|&off| rom.read_byte(off) & 0x0F)
            .collect();

        // Record Bowser's original data
        let bowser_w = &WORLDS[BOWSER_CASTLE.0];
        let (_sc, bowser_objsets, bowser_layouts) = rom_data::table_offsets(bowser_w);
        let bowser_obj = read_word(&rom, bowser_objsets + BOWSER_CASTLE.1 * 2);
        let bowser_lay = read_word(&rom, bowser_layouts + BOWSER_CASTLE.1 * 2);

        randomize_fortresses(&mut rom, &mut rng);

        // Bowser must be unchanged
        assert_eq!(read_word(&rom, bowser_objsets + BOWSER_CASTLE.1 * 2), bowser_obj);
        assert_eq!(read_word(&rom, bowser_layouts + BOWSER_CASTLE.1 * 2), bowser_lay);

        // Fortress entries should still contain the same set of obj pointers (just shuffled)
        let mut shuffled: Vec<u16> = fortresses.iter().map(|&(w, i)| {
            let world = &WORLDS[w];
            let (_scrcol, objsets, _layouts) = rom_data::table_offsets(world);
            read_word(&rom, objsets + i * 2)
        }).collect();
        let mut orig_sorted = original_objs.clone();
        orig_sorted.sort();
        shuffled.sort();
        assert_eq!(orig_sorted, shuffled, "Fortress obj pointers should be a permutation");

        // Verify Boom-Boom Y-byte patching: each position's Y-byte upper
        // nibble must match its ordinal within the destination world.
        for (position_idx, &(w, i)) in fortresses.iter().enumerate() {
            let world = &WORLDS[w];
            let (_scrcol, objsets, _layouts) = rom_data::table_offsets(world);
            let shuffled_obj = read_word(&rom, objsets + i * 2);

            // Find which original fortress landed here
            let orig_idx = original_objs.iter()
                .position(|&p| p == shuffled_obj)
                .unwrap();

            let y_byte = rom.read_byte(BOOMBOOM_Y_OFFSETS[orig_idx]);
            let upper = y_byte >> 4;
            let lower = y_byte & 0x0F;

            // Upper nibble = position's ordinal in its world
            let expected_ordinal = POSITION_ORDINALS[position_idx];
            assert_eq!(upper, expected_ordinal,
                "Position {} (W{}): Y-byte upper nibble {} != expected ordinal {}",
                position_idx, w + 1, upper, expected_ordinal);

            // Lower nibble must be preserved from the original fortress
            assert_eq!(lower, original_y_lowers[orig_idx],
                "Fortress {}: Y-byte lower nibble 0x{:X} != original 0x{:X}",
                orig_idx, lower, original_y_lowers[orig_idx]);
        }
    }

    #[test]
    fn test_airship_shuffle() {
        let mut rom = make_fortress_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        // Record original lay pointers for airships
        let original_lays: Vec<u16> = AIRSHIP_ENTRIES.iter().map(|&(w, i)| {
            let world = &WORLDS[w];
            let (_scrcol, _objsets, layouts) = rom_data::table_offsets(world);
            read_word(&rom, layouts + i * 2)
        }).collect();

        randomize_airships(&mut rom, &mut rng);

        let shuffled_lays: Vec<u16> = AIRSHIP_ENTRIES.iter().map(|&(w, i)| {
            let world = &WORLDS[w];
            let (_scrcol, _objsets, layouts) = rom_data::table_offsets(world);
            read_word(&rom, layouts + i * 2)
        }).collect();

        // Should be a permutation of originals
        let mut orig_sorted = original_lays.clone();
        let mut shuf_sorted = shuffled_lays.clone();
        orig_sorted.sort();
        shuf_sorted.sort();
        assert_eq!(orig_sorted, shuf_sorted, "Airship lay pointers should be a permutation");
    }

    #[test]
    fn test_fortress_shuffle_deterministic() {
        let mut rom1 = make_fortress_test_rom();
        let mut rom2 = make_fortress_test_rom();
        let mut rng1 = ChaCha8Rng::seed_from_u64(77);
        let mut rng2 = ChaCha8Rng::seed_from_u64(77);

        randomize_fortresses(&mut rom1, &mut rng1);
        randomize_fortresses(&mut rom2, &mut rng2);

        for &(w, i) in collect_fortresses(&make_fortress_test_rom()).iter() {
            let world = &WORLDS[w];
            let (_scrcol, objsets, layouts) = rom_data::table_offsets(world);
            assert_eq!(
                read_word(&rom1, objsets + i * 2),
                read_word(&rom2, objsets + i * 2),
            );
            assert_eq!(
                read_word(&rom1, layouts + i * 2),
                read_word(&rom2, layouts + i * 2),
            );
        }

        // Boom-Boom Y-bytes must also be identical
        for &offset in BOOMBOOM_Y_OFFSETS.iter() {
            assert_eq!(
                rom1.read_byte(offset),
                rom2.read_byte(offset),
                "Y-byte at 0x{:05X} differs between runs",
                offset,
            );
        }
    }
}
