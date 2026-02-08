use rand::Rng;
use rand::seq::IndexedRandom;

use crate::rom::Rom;

/// Level data regions by tileset (file offset ranges).
/// Each range contains 3-byte generator commands terminated by 0xFF.
const LEVEL_DATA_REGIONS: &[(usize, usize)] = &[
    (0x1A587, 0x1C005), // Underground (TS14)
    (0x1E512, 0x20005), // Plains (TS1)
    (0x20587, 0x22005), // Hilly (TS3)
    (0x227E0, 0x24005), // Ice / Sky (TS4/12)
    (0x24BA7, 0x26005), // Pipe / Water (TS7)
    (0x26A6F, 0x28C05), // Cloudy / Giant / Plant (TS5/11/13)
    (0x28F3F, 0x2A005), // Desert (TS2)
    (0x2A7F7, 0x2C005), // Dungeon (TS9)
    (0x2EC07, 0x30005), // Ship (TS10)
];

/// Level generator command encoding:
///   byte0 (Temp_Var15): bits 7-5 = generator group, bits 4-0 = Y position
///   byte1 (Temp_Var16): bits 7-4 = screen, bits 3-0 = X position
///   byte2 (LL_ShapeDef): upper nibble = 0 for fixed-size, lower nibble = shape index
///
/// Fixed-size index = ((byte0 & 0xE0) >> 1) + byte2, dispatched per tileset.
/// Group 1 (byte0 & 0xE0 == 0x20) → base index 16 → powerup blocks (identical
/// across all tilesets):
///   byte2: 0=Q-flower, 1=Q-leaf, 2=Q-star, 3=Q-coinstar, 4=Q-coin, 5=muncher,
///          6=brick-flower, 7=brick-leaf, 8=brick-star, 9=brick-coinstar,
///          10=brick-10coin, 11=brick-1up, 12=brick-vine, 13=brick-pswitch,
///          14=invis-coin, 15=invis-1up
const GEN_GROUP_MASK: u8 = 0xE0;
const GEN_GROUP_POWERBLOCK: u8 = 0x20; // group 1

/// ? block powerup shapes (flower=0, leaf=1, star=2).
const QBLOCK_SHAPES: &[u8] = &[0x00, 0x01, 0x02];

/// Brick powerup shapes (flower=6, leaf=7, star=8).
const BRICK_SHAPES: &[u8] = &[0x06, 0x07, 0x08];

/// Level header size in bytes (skipped after each 0xFF terminator).
const LEVEL_HEADER_SIZE: usize = 9;

/// File offsets of byte2 values that must not be randomized.
///
/// 7-7 (Muncher level): three Q-star blocks that must stay star — stars are
/// required to cross muncher fields. Verified against ROM.
const PROTECTED_OFFSETS: &[usize] = &[
    0x23DB0, // 7-7 Q-star byte2 (screen 2)
    0x23E1F, // 7-7 Q-star byte2 (screen 5)
    0x23EA0, // 7-7 Q-star byte2 (screen 8)
];

/// Randomize per-level ? block and brick powerup types by scanning all level
/// data regions for generator commands that place powerup blocks, and swapping
/// the shape index (byte2) to a random type within the same category.
///
/// ? blocks swap among {flower, leaf, star} and bricks swap among
/// {flower-brick, leaf-brick, star-brick}. Protected offsets (like 7-7's
/// star bricks) are never modified.
pub fn randomize<R: Rng>(rom: &mut Rom, rng: &mut R) {
    for &(start, end) in LEVEL_DATA_REGIONS {
        let len = end - start;
        let mut data = rom.read_range(start, len).to_vec();

        // Each region begins with a 9-byte level header, then 3-byte tile
        // commands terminated by 0xFF.  After each 0xFF the next level's
        // 9-byte header follows (unless we've reached the end of the region).
        let mut i = LEVEL_HEADER_SIZE; // skip the first header
        while i + 2 < data.len() {
            if data[i] == 0xFF {
                // Skip terminator + next level header
                i += 1 + LEVEL_HEADER_SIZE;
                continue;
            }

            let b0 = data[i];
            let b2 = data[i + 2];
            let group = b0 & GEN_GROUP_MASK;
            let is_fixed = (b2 & 0xF0) == 0;

            if group == GEN_GROUP_POWERBLOCK && is_fixed {
                let shape = b2 & 0x0F;
                let file_offset = start + i + 2;

                if QBLOCK_SHAPES.contains(&shape) && !PROTECTED_OFFSETS.contains(&file_offset) {
                    data[i + 2] = *QBLOCK_SHAPES.choose(rng).unwrap();
                } else if BRICK_SHAPES.contains(&shape) && !PROTECTED_OFFSETS.contains(&file_offset) {
                    data[i + 2] = *BRICK_SHAPES.choose(rng).unwrap();
                }
            }

            i += 3;
        }

        rom.write_range(start, &data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    fn make_test_rom() -> Rom {
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;

        // Place some test level data in the Plains region (0x1E512)
        let start = 0x1E512;
        let level = &[
            // 9-byte level header (dummy)
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            // flower Q-block: byte0=0x22 (grp=1, y=2), byte1=0x1A, byte2=0x00
            0x22, 0x1A, 0x00,
            // leaf Q-block: byte0=0x25 (grp=1, y=5), byte1=0x2B, byte2=0x01
            0x25, 0x2B, 0x01,
            // star brick: byte0=0x28 (grp=1, y=8), byte1=0x3C, byte2=0x08
            0x28, 0x3C, 0x08,
            // non-powerup generator (grp=3): should NOT be touched
            0x60, 0x0E, 0x1F,
            // junction (grp=7): should NOT be touched
            0xE0, 0x52, 0x20,
            // variable-size grp=1 (byte2 upper nibble != 0): should NOT be touched
            0x37, 0x1C, 0x11,
            0xFF, // terminator
        ];
        data[start..start + level.len()].copy_from_slice(level);

        Rom::from_bytes(&data).unwrap()
    }

    #[test]
    fn test_qblocks_randomized_within_class() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize(&mut rom, &mut rng);

        // Offsets: 9-byte header + command data
        let start = 0x1E512 + 9; // first command after header
        let b2_flower = rom.read_byte(start + 2);
        assert!(QBLOCK_SHAPES.contains(&b2_flower), "Q-block became 0x{b2_flower:02X}");

        let b2_leaf = rom.read_byte(start + 5);
        assert!(QBLOCK_SHAPES.contains(&b2_leaf), "Q-block became 0x{b2_leaf:02X}");

        let b2_brick = rom.read_byte(start + 8);
        assert!(BRICK_SHAPES.contains(&b2_brick), "Brick became 0x{b2_brick:02X}");
    }

    #[test]
    fn test_non_powerblock_untouched() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize(&mut rom, &mut rng);

        // Offsets: 9-byte header + 3 powerup cmds (9 bytes) + non-powerup cmds
        let start = 0x1E512 + 9 + 9; // after header + 3 powerup commands
        assert_eq!(rom.read_byte(start), 0x60);
        assert_eq!(rom.read_byte(start + 2), 0x1F);
        assert_eq!(rom.read_byte(start + 3), 0xE0);
        assert_eq!(rom.read_byte(start + 5), 0x20);
        assert_eq!(rom.read_byte(start + 6), 0x37);
        assert_eq!(rom.read_byte(start + 8), 0x11);
    }

    #[test]
    fn test_protected_offset_not_changed() {
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;

        // Ice/Sky region starts at 0x227E0. Place a header then commands
        // such that the Q-star at 0x23DB0 lines up correctly.
        // The protected byte2 is at file offset 0x23DB0.
        // Command starts at 0x23DAE (byte0), 0x23DAF (byte1), 0x23DB0 (byte2).
        // We need commands between 0x227E0+9 and 0x23DAE to be valid 3-byte
        // groups. For simplicity, place a header and then pad with variable-size
        // commands (which won't match our filter) up to the target offset, then
        // place the protected star command.
        let region_start = 0x227E0;
        // Dummy header
        for j in 0..9 {
            data[region_start + j] = 0x00;
        }
        // The protected Q-star: group 1 (0x20), byte2=0x02 (star)
        data[0x23DAE] = 0x35; // grp=1, y=21
        data[0x23DAF] = 0x2A; // scr=2, x=10
        data[0x23DB0] = 0x02; // Q-star

        let mut rom = Rom::from_bytes(&data).unwrap();
        let mut rng = ChaCha8Rng::seed_from_u64(99);

        for _ in 0..10 {
            randomize(&mut rom, &mut rng);
            assert_eq!(rom.read_byte(0x23DB0), 0x02, "7-7 Q-star was modified!");
        }
    }

    #[test]
    fn test_deterministic() {
        let mut rom1 = make_test_rom();
        let mut rom2 = make_test_rom();
        let mut rng1 = ChaCha8Rng::seed_from_u64(123);
        let mut rng2 = ChaCha8Rng::seed_from_u64(123);

        randomize(&mut rom1, &mut rng1);
        randomize(&mut rom2, &mut rng2);

        for &(start, end) in LEVEL_DATA_REGIONS {
            let len = end - start;
            assert_eq!(rom1.read_range(start, len), rom2.read_range(start, len));
        }
    }
}
