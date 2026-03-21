use rand::Rng;
use rand::seq::IndexedRandom;

use crate::rom::Rom;

use super::rom_data::LEVEL_DATA_REGIONS;

/// Tilesets where group 2 note/wood block powerups can be safely randomized.
///
/// Each note/wood variant produces a distinct tile ID (notes: 0x2F/0x30/0x31,
/// wood: 0x73/0x74/0x75). In tilesets that have note/wood block graphics,
/// all variants look the same (note block or wood block). But in tilesets
/// like Dungeon (TS2), Desert (TS9), and Ship (TS10), these tile ID slots
/// are reused for tileset-specific decorations, so swapping between variants
/// changes the block's visual appearance (extra/missing tiles).
///
/// Regions where note/wood randomization is DISABLED are identified by their
/// start offset (Dungeon 0x2A7F7, Desert 0x28F3F, Ship 0x2EC07).
const NOTE_WOOD_DISABLED_REGIONS: &[usize] = &[0x28F3F, 0x2A7F7, 0x2EC07];

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
const GEN_GROUP_EXTENDED: u8 = 0x40; // group 2 (notes, wood blocks)

/// ? block powerup shapes (flower=0, leaf=1, star=2).
const QBLOCK_SHAPES: &[u8] = &[0x00, 0x01, 0x02];

/// Brick powerup shapes (flower=6, leaf=7, star=8, 1-up=11).
const BRICK_SHAPES: &[u8] = &[0x06, 0x07, 0x08, 0x0B];

/// Note block powerup shapes (flower=1, leaf=2, star=3).
const NOTE_SHAPES: &[u8] = &[0x01, 0x02, 0x03];

/// Wood block powerup shapes (flower=4, leaf=5, star=6).
const WOOD_SHAPES: &[u8] = &[0x04, 0x05, 0x06];

/// Level header size in bytes (skipped after each 0xFF terminator).
const LEVEL_HEADER_SIZE: usize = 9;

/// File offsets of byte2 values that must not be randomized.
///
/// 6-5 (Ice level): single Q-leaf block — leaf is required for flight.
///
/// 7-7 (Muncher level): four Q-star blocks that must stay star — stars are
/// required to cross muncher fields. Found by brute-scanning the sub-area
/// at 0x23D48–0x23F1F for group 1 fixed-size byte2=0x02 patterns.
const PROTECTED_OFFSETS: &[usize] = &[
    0x22D74, // 6-5 Q-leaf byte2 — leaf required for flight in ice level
    0x23D7F, // 7-7 Q-star byte2 (screen 1)
    0x23DB0, // 7-7 Q-star byte2 (screen 2)
    0x23E1F, // 7-7 Q-star byte2 (screen 5)
    0x23EA0, // 7-7 Q-star byte2 (screen 8)
];

/// Randomize per-level powerup block types by scanning all level data regions
/// for generator commands that place powerup blocks, and swapping the shape
/// index (byte2) to a random type within the same category.
///
/// Group 1 (0x20): ? blocks swap among {flower, leaf, star}, bricks swap among
/// {flower, leaf, star, 1-up}. Group 2 (0x40): note blocks swap among
/// {flower, leaf, star}, wood blocks swap among {flower, leaf, star} — but only
/// in tileset regions where note/wood variants share the same visual appearance.
/// In Dungeon/Desert/Ship tilesets the note/wood tile IDs render as different
/// decorations, so swapping would corrupt the level visuals.
/// Protected offsets (like 7-7's star blocks) are never modified.
pub fn randomize<R: Rng>(rom: &mut Rom, rng: &mut R) {
    for region in LEVEL_DATA_REGIONS {
        let len = region.end - region.start;
        let mut data = rom.read_range(region.start, len).to_vec();
        let note_wood_ok = !NOTE_WOOD_DISABLED_REGIONS.contains(&region.start);

        // Each region begins with a 9-byte level header, then generator
        // commands terminated by 0xFF. After each 0xFF the next level's
        // 9-byte header follows (unless we've reached the end of the region).
        //
        // Most commands are 3 bytes, but some variable-size generators read
        // a 4th byte from the stream. We must detect these to stay aligned.
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

            if is_fixed {
                let shape = b2 & 0x0F;
                let file_offset = region.start + i + 2;

                if group == GEN_GROUP_POWERBLOCK {
                    if QBLOCK_SHAPES.contains(&shape) && !PROTECTED_OFFSETS.contains(&file_offset) {
                        data[i + 2] = *QBLOCK_SHAPES.choose(rng).unwrap();
                    } else if BRICK_SHAPES.contains(&shape) && !PROTECTED_OFFSETS.contains(&file_offset) {
                        data[i + 2] = *BRICK_SHAPES.choose(rng).unwrap();
                    }
                } else if note_wood_ok && group == GEN_GROUP_EXTENDED {
                    if NOTE_SHAPES.contains(&shape) {
                        data[i + 2] = *NOTE_SHAPES.choose(rng).unwrap();
                    } else if WOOD_SHAPES.contains(&shape) {
                        data[i + 2] = *WOOD_SHAPES.choose(rng).unwrap();
                    }
                }
            }

            // Determine command size: 3 bytes normally, 4 if this is a
            // variable-size dispatch that reads an extra byte.
            let mut cmd_size = 3;
            if !is_fixed {
                let grp = (b0 >> 5) as usize;
                let dispatch = grp * 15 + ((b2 >> 4) as usize) - 1;
                if region.extra_byte_dispatches.contains(&(dispatch as u8)) {
                    cmd_size = 4;
                }
            }
            i += cmd_size;
        }

        rom.write_range(region.start, &data);
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
            // wood block leaf: byte0=0x57 (grp=2, y=7, hi=1), byte1=0x28, byte2=0x05
            0x57, 0x28, 0x05,
            // note block star: byte0=0x46 (grp=2, y=6), byte1=0x37, byte2=0x03
            0x46, 0x37, 0x03,
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
    fn test_4byte_command_alignment() {
        // Verifies that a 4-byte GroundRun command doesn't misalign the parser,
        // causing subsequent powerup blocks to be missed or corrupted.
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;

        // Plains region (TS1): GroundRun (dispatch 11) reads a 4th byte.
        let start = 0x1E512;
        let level = &[
            // 9-byte header
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            // GroundRun: byte0=0x1A (grp=0, y=10, hi=1), byte1=0x00, byte2=0xC0
            // dispatch = 0*15 + (0xC0>>4) - 1 = 11 → GroundRun → 4 bytes
            0x1A, 0x00, 0xC0,
            0x26, // extra byte (ground width)
            // QBLOCKLEAF: byte0=0x33 (grp=1, y=3, hi=1), byte1=0x0F, byte2=0x01
            0x33, 0x0F, 0x01,
            0xFF, // terminator
        ];
        data[start..start + level.len()].copy_from_slice(level);

        let mut rom = Rom::from_bytes(&data).unwrap();
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        // The QBLOCKLEAF byte2 is at start + 9 + 4 + 2 = start + 15
        let leaf_offset = start + 15;
        assert_eq!(rom.read_byte(leaf_offset), 0x01, "precondition: byte2 is leaf");

        randomize(&mut rom, &mut rng);

        // After randomization, byte2 should be one of {0x00, 0x01, 0x02}
        let result = rom.read_byte(leaf_offset);
        assert!(
            QBLOCK_SHAPES.contains(&result),
            "QBLOCKLEAF after GroundRun was not randomized (got 0x{result:02X}), \
             parser likely misaligned on 4-byte command"
        );

        // Also verify the GroundRun extra byte was NOT corrupted
        assert_eq!(rom.read_byte(start + 12), 0x26, "GroundRun extra byte was corrupted");
    }

    #[test]
    fn test_wood_note_blocks_randomized() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize(&mut rom, &mut rng);

        // Wood block leaf is at: header(9) + 3 grp1 cmds(9) + 3 non-power cmds(9) + 2 = byte2
        let wood_offset = 0x1E512 + 9 + 9 + 9 + 2;
        let b2_wood = rom.read_byte(wood_offset);
        assert!(WOOD_SHAPES.contains(&b2_wood), "Wood block became 0x{b2_wood:02X}");

        // Note block star is 3 bytes later
        let note_offset = wood_offset + 3;
        let b2_note = rom.read_byte(note_offset);
        assert!(NOTE_SHAPES.contains(&b2_note), "Note block became 0x{b2_note:02X}");
    }

    #[test]
    fn test_deterministic() {
        let mut rom1 = make_test_rom();
        let mut rom2 = make_test_rom();
        let mut rng1 = ChaCha8Rng::seed_from_u64(123);
        let mut rng2 = ChaCha8Rng::seed_from_u64(123);

        randomize(&mut rom1, &mut rng1);
        randomize(&mut rom2, &mut rng2);

        for region in LEVEL_DATA_REGIONS {
            let len = region.end - region.start;
            assert_eq!(
                rom1.read_range(region.start, len),
                rom2.read_range(region.start, len),
            );
        }
    }
}
