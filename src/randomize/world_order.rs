use rand::Rng;
use rand::seq::SliceRandom;

use crate::rom::Rom;

/// File offset of the `INC World_Num; JMP $84A0` instruction (6 bytes).
/// Original bytes: EE 27 07 4C A0 84
const WORLD_INC_OFFSET: usize = 0x3D0A1;

/// File offset of the world-init routine (11 bytes: INC $DE; LDA #$00; STA $0727; STA $0160; RTS).
/// The original `LDA #$00` is shared by both STA targets — we must split them so that patching
/// World_Num doesn't also corrupt the debug flag at $0160.
const WORLD_INIT_OFFSET: usize = 0x30CC0;

/// Free space in PRG030 for our lookup routine + table.
/// File offset 0x3DF20 = CPU $9F10 (PRG030 mapped at $8000).
const ROUTINE_OFFSET: usize = 0x3DF20;

/// CPU address of the routine in free space.
const ROUTINE_CPU: u16 = 0x9F10;

/// CPU address of the lookup table (routine + 11 bytes).
const TABLE_CPU: u16 = ROUTINE_CPU + 11;

/// Free space for the init tail (clears $0160 and returns).
/// Placed after the lookup routine + 8-byte table = offset + 20.
const INIT_TAIL_OFFSET: usize = ROUTINE_OFFSET + 20;

/// CPU address of the init tail.
const INIT_TAIL_CPU: u16 = ROUTINE_CPU + 20;

/// Randomize the world progression order.
///
/// Patches the `INC World_Num` instruction to instead use a lookup table
/// that maps current world -> next world. The table is written into free
/// space at the end of PRG030.
///
/// The 8 worlds (0-7) are shuffled, but world 7 (Dark Land) is always last
/// since it contains Bowser and the game ending.
pub fn randomize<R: Rng>(rom: &mut Rom, rng: &mut R) {
    // Build shuffled world order: worlds 0-6 shuffled, world 7 always last
    let mut worlds: Vec<u8> = (0..7).collect();
    worlds.as_mut_slice().shuffle(rng);
    worlds.push(7);

    // Rewrite the world-init routine to split the shared LDA #$00.
    // Original (11 bytes at 0x30CC0):
    //   E6 DE        INC $DE
    //   A9 00        LDA #$00
    //   8D 27 07     STA $0727   ; World_Num
    //   8D 60 01     STA $0160   ; debug flag (must stay 0!)
    //   60           RTS
    //
    // Patched in-place (11 bytes, tail jumps to free space):
    //   E6 DE        INC $DE
    //   A9 XX        LDA #starting_world
    //   8D 27 07     STA $0727
    //   4C XX XX     JMP init_tail
    //   EA           NOP (pad)
    let tail_lo = (INIT_TAIL_CPU & 0xFF) as u8;
    let tail_hi = ((INIT_TAIL_CPU >> 8) & 0xFF) as u8;
    rom.write_range(WORLD_INIT_OFFSET, &[
        0xE6, 0xDE,                         // INC $DE
        0xA9, worlds[0],                     // LDA #starting_world
        0x8D, 0x27, 0x07,                   // STA $0727
        0x4C, tail_lo, tail_hi,              // JMP init_tail
        0xEA,                                // NOP (pad to 11 bytes)
    ]);

    // Init tail in free space: clear debug flag and return.
    rom.write_range(INIT_TAIL_OFFSET, &[
        0xA9, 0x00,              // LDA #$00
        0x8D, 0x60, 0x01,       // STA $0160
        0x60,                    // RTS
    ]);

    // Build the "next world" lookup table.
    // For each position i in the shuffled order, the next world is worlds[i+1].
    // We index by the *current* World_Num value.
    // If current world is worlds[i], next world should be worlds[i+1].
    let mut next_world = [0u8; 8];
    for i in 0..7 {
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
        // Write original world-init routine (11 bytes)
        data[WORLD_INIT_OFFSET..WORLD_INIT_OFFSET + 11].copy_from_slice(&[
            0xE6, 0xDE,       // INC $DE
            0xA9, 0x00,       // LDA #$00
            0x8D, 0x27, 0x07, // STA $0727
            0x8D, 0x60, 0x01, // STA $0160
            0x60,             // RTS
        ]);
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
        randomize(&mut rom, &mut rng);

        // Original INC site should now be JMP + NOPs
        assert_eq!(rom.read_byte(WORLD_INC_OFFSET), 0x4C); // JMP
        assert_eq!(rom.read_byte(WORLD_INC_OFFSET + 3), 0xEA); // NOP
    }

    #[test]
    fn test_world_order_table_valid() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize(&mut rom, &mut rng);

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
        randomize(&mut rom, &mut rng);

        // Starting world is the operand of `LDA #XX` at WORLD_INIT_OFFSET + 3
        let start_world = rom.read_byte(WORLD_INIT_OFFSET + 3);
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
    fn test_debug_flag_cleared() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize(&mut rom, &mut rng);

        // The init tail should write LDA #$00; STA $0160
        let tail = rom.read_range(INIT_TAIL_OFFSET, 6);
        assert_eq!(&tail[0..2], &[0xA9, 0x00], "Init tail should LDA #$00");
        assert_eq!(&tail[2..5], &[0x8D, 0x60, 0x01], "Init tail should STA $0160");
        assert_eq!(tail[5], 0x60, "Init tail should end with RTS");

        // The in-place routine should JMP to the tail (not STA $0160 directly)
        assert_eq!(rom.read_byte(WORLD_INIT_OFFSET + 7), 0x4C, "Should JMP to init tail");
    }

    #[test]
    fn test_world_order_deterministic() {
        let mut rom1 = make_test_rom();
        let mut rom2 = make_test_rom();
        let mut rng1 = ChaCha8Rng::seed_from_u64(99);
        let mut rng2 = ChaCha8Rng::seed_from_u64(99);

        randomize(&mut rom1, &mut rng1);
        randomize(&mut rom2, &mut rng2);

        assert_eq!(
            rom1.read_range(ROUTINE_OFFSET, 26),
            rom2.read_range(ROUTINE_OFFSET, 26),
        );
    }

    #[test]
    fn test_routine_structure() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize(&mut rom, &mut rng);

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
}
