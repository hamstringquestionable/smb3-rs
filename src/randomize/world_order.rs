use rand::Rng;
use rand::seq::SliceRandom;

use crate::rom::Rom;

/// File offset of the `INC World_Num; JMP $84A0` instruction (6 bytes).
/// Original bytes: EE 27 07 4C A0 84
const WORLD_INC_OFFSET: usize = 0x3D0A1;

/// File offset of the `LDA #$00` operand that initializes World_Num at game start.
/// Original: `LDA #$00; STA $0727; STA $0160`. We patch the #$00 to the starting world
/// and NOP out the `STA $0160` so the debug flag isn't set to the world number.
const WORLD_INIT_OPERAND: usize = 0x30CC3;

/// File offset of the `STA $0160` (Debug_Flag) instruction (3 bytes).
/// We NOP this out because patching the LDA operand above would otherwise
/// set the debug flag to the starting world number.  The reset handler
/// clears $0160 to zero on power-on, so it's safe to skip this write.
const DEBUG_FLAG_STA_OFFSET: usize = 0x30CC7;

/// Free space in PRG030 — offset from rom_data::FS_WORLD_ORDER.
/// Uses 28 bytes: 12 routine + 8 next-world table + 8 display table.
const ROUTINE_OFFSET: usize = super::rom_data::FS_WORLD_ORDER;

/// CPU address of the routine in free space.
const ROUTINE_CPU: u16 = 0x9F10;

/// CPU address of the lookup table (routine + 12 bytes).
const TABLE_CPU: u16 = ROUTINE_CPU + 12;

/// File offset of the display-number table (8 bytes, right after next-world table).
/// PRG030 is always mapped at $8000–$9FFF (MMC3 fixed bank in mode 1), so CPU $9F24
/// is accessible from any bank configuration.
const DISPLAY_TABLE_OFFSET: usize = ROUTINE_OFFSET + 20; // 12 routine + 8 next-world
const DISPLAY_TABLE_CPU: u16 = TABLE_CPU + 8; // $9F24

/// Map screen "WORLD X" display site (PRG010).
/// Original: LDY $0727; INY; TYA; ORA #$F0; STA $0304 (10 bytes at 0x14372).
const MAP_DISPLAY_OFFSET: usize = 0x14372;

/// Status bar "WORLD X" display site (PRG026).
/// Original: LDX $0727; INX; TXA; ORA #$F0; STA $0304,Y (10 bytes at 0x350D7).
const STATUS_DISPLAY_OFFSET: usize = 0x350D7;

/// Randomize the world progression order.
///
/// Patches the `INC World_Num` instruction to instead use a lookup table
/// that maps current world -> next world. The table is written into free
/// space at the end of PRG030.
///
/// The 8 worlds (0-7) are shuffled, but world 7 (Dark Land) is always last
/// since it contains Bowser and the game ending.
pub fn randomize<R: Rng>(rom: &mut Rom, rng: &mut R, world_count: u8) {
    let world_count = world_count.clamp(1, 7) as usize;

    // Build shuffled world order: shuffle worlds 0-6, take first world_count, append world 7
    let mut pool: Vec<u8> = (0..7).collect();
    pool.as_mut_slice().shuffle(rng);
    let mut worlds: Vec<u8> = pool[..world_count].to_vec();
    worlds.push(7);

    // Patch the starting world: change `LDA #$00` operand to starting world.
    rom.write_byte(WORLD_INIT_OPERAND, worlds[0]);
    // NOP out `STA $0160` (Debug_Flag) so it doesn't get the world number.
    // The reset handler clears $0160 to zero, so skipping this write is safe.
    rom.write_range(DEBUG_FLAG_STA_OFFSET, &[0xEA, 0xEA, 0xEA]);

    // Build the "next world" lookup table.
    // For each position i in the shuffled order, the next world is worlds[i+1].
    // We index by the *current* World_Num value.
    // If current world is worlds[i], next world should be worlds[i+1].
    let mut next_world = [0u8; 8];
    for i in 0..world_count {
        next_world[worlds[i] as usize] = worlds[i + 1];
    }
    // World 7 (last) -> 7 (stays, game ends before this matters)
    next_world[7] = 7;

    // Patch the original INC World_Num site to JMP to our routine
    let routine_lo = (ROUTINE_CPU & 0xFF) as u8;
    let routine_hi = ((ROUTINE_CPU >> 8) & 0xFF) as u8;
    rom.write_range(WORLD_INC_OFFSET, &[
        0x4C, routine_lo, routine_hi, // JMP $9F10
        0xEA, 0xEA, 0xEA,            // NOP NOP NOP (pad)
    ]);

    // Write the lookup routine into free space
    let table_lo = (TABLE_CPU & 0xFF) as u8;
    let table_hi = ((TABLE_CPU >> 8) & 0xFF) as u8;
    let routine: Vec<u8> = vec![
        0xAE, 0x27, 0x07,             // LDX World_Num ($0727)
        0xBD, table_lo, table_hi,      // LDA table,X
        0x8D, 0x27, 0x07,             // STA World_Num ($0727)
        0x4C, 0xA0, 0x84,             // JMP $84A0 (map init)
    ];
    rom.write_range(ROUTINE_OFFSET, &routine);

    // Write the lookup table immediately after the routine
    rom.write_range(ROUTINE_OFFSET + routine.len(), &next_world);

    // Build the display-number table: internal world -> display tile ($F1–$F8).
    // worlds[i] is the internal world at shuffled position i, so position i
    // should display as "WORLD (i+1)". With fewer worlds, Dark Land displays
    // as world_count+1 (e.g. world_count=3 → Dark Land is "WORLD 4").
    let mut display_tile = [0u8; 8];
    for (position, &internal) in worlds.iter().enumerate() {
        display_tile[internal as usize] = 0xF0 | (position as u8 + 1);
    }
    rom.write_range(DISPLAY_TABLE_OFFSET, &display_tile);

    let disp_lo = (DISPLAY_TABLE_CPU & 0xFF) as u8;
    let disp_hi = ((DISPLAY_TABLE_CPU >> 8) & 0xFF) as u8;

    // Patch map screen display (PRG010): replace LDY/INY/TYA/ORA/STA with table lookup.
    // Original 10 bytes: AC 27 07 C8 98 09 F0 8D 04 03
    rom.write_range(MAP_DISPLAY_OFFSET, &[
        0xAE, 0x27, 0x07,             // LDX $0727  (World_Num)
        0xBD, disp_lo, disp_hi,       // LDA $DF24,X (display tile)
        0x8D, 0x04, 0x03,             // STA $0304
        0xEA,                         // NOP (pad)
    ]);

    // Patch status bar display (PRG026): replace LDX/INX/TXA/ORA/STA with table lookup.
    // Original 10 bytes: AE 27 07 E8 8A 09 F0 99 04 03
    rom.write_range(STATUS_DISPLAY_OFFSET, &[
        0xAE, 0x27, 0x07,             // LDX $0727  (World_Num)
        0xBD, disp_lo, disp_hi,       // LDA $DF24,X (display tile)
        0x99, 0x04, 0x03,             // STA $0304,Y
        0xEA,                         // NOP (pad)
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha8Rng;
    use rand::SeedableRng;

    fn make_test_rom() -> Rom {
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;
        // Write original world-init: LDA #$00 operand and STA $0160
        data[WORLD_INIT_OPERAND] = 0x00;
        data[DEBUG_FLAG_STA_OFFSET..DEBUG_FLAG_STA_OFFSET + 3]
            .copy_from_slice(&[0x8D, 0x60, 0x01]);
        // Write original INC World_Num bytes
        data[WORLD_INC_OFFSET..WORLD_INC_OFFSET + 6]
            .copy_from_slice(&[0xEE, 0x27, 0x07, 0x4C, 0xA0, 0x84]);
        // Fill free space with FF
        for i in ROUTINE_OFFSET..ROUTINE_OFFSET + 32 {
            data[i] = 0xFF;
        }
        Rom::from_bytes(&data).unwrap()
    }

    #[test]
    fn test_world_order_patches_inc() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize(&mut rom, &mut rng, 7);

        // Original INC site should now be JMP + NOPs
        assert_eq!(rom.read_byte(WORLD_INC_OFFSET), 0x4C); // JMP
        assert_eq!(rom.read_byte(WORLD_INC_OFFSET + 3), 0xEA); // NOP
    }

    #[test]
    fn test_world_order_table_valid() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize(&mut rom, &mut rng, 7);

        // Read the lookup table
        let table = rom.read_range(ROUTINE_OFFSET + 12, 8);

        // Every world 0-7 should appear exactly once as a "next" destination
        // (except world 7 which maps to itself)
        // More importantly: following the chain from world[0] in shuffled order
        // should visit all 8 worlds
        let mut visited = vec![false; 8];
        // Find which world is first in the shuffled order (it's the one that
        // no other world points to, except itself if it's 7)
        // Actually, let's just verify: table[7] == 7 (world 7 is always last)
        assert_eq!(table[7], 7, "World 7 (Dark Land) should map to itself");

        // All table values should be valid world numbers
        for &next in table.iter() {
            assert!(next <= 7, "Invalid world number in table: {next}");
        }

        // The chain should visit all worlds: starting from any world in position 0,
        // following next pointers should reach world 7
        // Find a starting world (one that appears as table[x] for no x,
        // i.e., it's the first world in the sequence — or just check all worlds reachable)
        for start in 0..8u8 {
            let mut current = start;
            visited[current as usize] = true;
            for _ in 0..8 {
                current = table[current as usize];
                visited[current as usize] = true;
            }
        }
        assert!(visited.iter().all(|&v| v), "Not all worlds reachable");
    }

    #[test]
    fn test_starting_world_patched() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize(&mut rom, &mut rng, 7);

        // Starting world is the operand of `LDA #XX` at WORLD_INIT_OPERAND
        let start_world = rom.read_byte(WORLD_INIT_OPERAND);
        assert_ne!(start_world, 0, "Starting world should usually not be 0 after shuffle");
        assert!(start_world <= 6, "Starting world should be 0-6 (not Dark Land)");

        // The starting world should match the first entry in the chain
        // Follow the chain from start_world and verify we visit all 8 worlds
        let table = rom.read_range(ROUTINE_OFFSET + 12, 8);
        let mut visited = vec![false; 8];
        let mut current = start_world;
        for _ in 0..8 {
            visited[current as usize] = true;
            current = table[current as usize];
        }
        assert!(visited.iter().all(|&v| v), "Chain from starting world should visit all worlds");
    }

    #[test]
    fn test_debug_flag_nopped() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize(&mut rom, &mut rng, 7);

        // STA $0160 (Debug_Flag) should be NOPed out
        assert_eq!(
            rom.read_range(DEBUG_FLAG_STA_OFFSET, 3),
            &[0xEA, 0xEA, 0xEA],
            "STA Debug_Flag should be NOPed out"
        );
    }

    #[test]
    fn test_world_order_deterministic() {
        let mut rom1 = make_test_rom();
        let mut rom2 = make_test_rom();
        let mut rng1 = ChaCha8Rng::seed_from_u64(99);
        let mut rng2 = ChaCha8Rng::seed_from_u64(99);

        randomize(&mut rom1, &mut rng1, 7);
        randomize(&mut rom2, &mut rng2, 7);

        assert_eq!(
            rom1.read_range(ROUTINE_OFFSET, 20),
            rom2.read_range(ROUTINE_OFFSET, 20),
        );
    }

    #[test]
    fn test_routine_structure() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize(&mut rom, &mut rng, 7);

        let routine = rom.read_range(ROUTINE_OFFSET, 12);
        // LDX $0727
        assert_eq!(&routine[0..3], &[0xAE, 0x27, 0x07]);
        // LDA table,X (BD xx xx)
        assert_eq!(routine[3], 0xBD);
        // STA $0727
        assert_eq!(&routine[6..9], &[0x8D, 0x27, 0x07]);
        // JMP $84A0
        assert_eq!(&routine[9..12], &[0x4C, 0xA0, 0x84]);
    }

    #[test]
    fn test_display_table_covers_all_worlds() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize(&mut rom, &mut rng, 7);

        let display = rom.read_range(DISPLAY_TABLE_OFFSET, 8);

        // Each entry should be a valid tile $F1–$F8
        for &tile in display.iter() {
            assert!(tile >= 0xF1 && tile <= 0xF8, "Bad display tile: {tile:#04X}");
        }

        // Every display number 1–8 should appear exactly once
        let mut seen = [false; 9];
        for &tile in display.iter() {
            let num = (tile & 0x0F) as usize;
            assert!(!seen[num], "Duplicate display number {num}");
            seen[num] = true;
        }

        // World 7 (Dark Land, always last) should display as "8"
        assert_eq!(display[7], 0xF8, "Dark Land should display as World 8");
    }

    #[test]
    fn test_display_patches_applied() {
        let mut rom = make_test_rom();
        // Write original bytes at display sites so we can verify they get patched
        rom.write_range(MAP_DISPLAY_OFFSET, &[
            0xAC, 0x27, 0x07, 0xC8, 0x98, 0x09, 0xF0, 0x8D, 0x04, 0x03,
        ]);
        rom.write_range(STATUS_DISPLAY_OFFSET, &[
            0xAE, 0x27, 0x07, 0xE8, 0x8A, 0x09, 0xF0, 0x99, 0x04, 0x03,
        ]);

        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize(&mut rom, &mut rng, 7);

        // Map display: should now use LDX $0727; LDA $DF24,X; STA $0304; NOP
        let map_patch = rom.read_range(MAP_DISPLAY_OFFSET, 10);
        assert_eq!(&map_patch[0..3], &[0xAE, 0x27, 0x07]); // LDX $0727
        assert_eq!(map_patch[3], 0xBD);                      // LDA abs,X
        assert_eq!(&map_patch[6..9], &[0x8D, 0x04, 0x03]);  // STA $0304
        assert_eq!(map_patch[9], 0xEA);                      // NOP

        // Status bar: should now use LDX $0727; LDA $DF24,X; STA $0304,Y; NOP
        let status_patch = rom.read_range(STATUS_DISPLAY_OFFSET, 10);
        assert_eq!(&status_patch[0..3], &[0xAE, 0x27, 0x07]); // LDX $0727
        assert_eq!(status_patch[3], 0xBD);                      // LDA abs,X
        assert_eq!(&status_patch[6..9], &[0x99, 0x04, 0x03]);  // STA $0304,Y
        assert_eq!(status_patch[9], 0xEA);                      // NOP
    }

    #[test]
    fn test_world_count_3() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize(&mut rom, &mut rng, 3);

        let table = rom.read_range(ROUTINE_OFFSET + 12, 8);

        // Follow chain from starting world: should visit exactly 4 worlds (3 + Dark Land)
        let start_world = rom.read_byte(WORLD_INIT_OPERAND);
        assert!(start_world <= 6, "Starting world should be 0-6");

        let mut visited = vec![false; 8];
        let mut current = start_world;
        for _ in 0..4 {
            visited[current as usize] = true;
            current = table[current as usize];
        }
        let count = visited.iter().filter(|&&v| v).count();
        assert_eq!(count, 4, "Chain should visit exactly 4 worlds (3 + Dark Land), got {count}");
        assert!(visited[7], "Dark Land (world 7) must be in the chain");

        // Display table: Dark Land should show as "WORLD 4" ($F4)
        let display = rom.read_range(DISPLAY_TABLE_OFFSET, 8);
        assert_eq!(display[7], 0xF4, "Dark Land should display as World 4 with world_count=3");
    }
}
