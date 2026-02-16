use rand::Rng;
use rand::seq::IndexedRandom;

use crate::rom::Rom;

/// Enemy/object data block: 0x0BFD8–0x0E00D.
///
/// Format: each level's enemy set is a sequence of segments separated by 0xFF.
/// Each segment starts with a 1-byte page flag, then zero or more 3-byte
/// entries: [object_id, x_pos, y_pos], terminated by 0xFF.
///
/// We parse this structure properly and only randomize the object_id byte
/// of entries whose ID is in our explicit allowlist of swappable enemies.
const ENEMY_DATA_START: usize = 0x0BFD8;
const ENEMY_DATA_END: usize = 0x0E00D;

// Object IDs from the Southbird SMB3 disassembly (smb3.asm).
// Only IDs that are actual enemies safe to swap are included.
// Special objects (end-level card, pipes, platforms, bosses, powerups,
// autoscroll, event triggers, cannons, etc.) are NOT listed and will
// never be modified.

/// Ground-walking enemies (no shell). These can be freely swapped with each other.
const GROUND_ENEMIES: &[u8] = &[
    0x29, // OBJ_SPIKE
    0x2A, // OBJ_PATOOIE
    0x33, // OBJ_NIPPER (stationary)
    0x39, // OBJ_NIPPERHOPPING
    0x3F, // OBJ_DRYBONES
    0x40, // OBJ_BUSTERBEATLE
    0x55, // OBJ_BOBOMB
    0x6B, // OBJ_PILEDRIVER (micro goomba)
    0x71, // OBJ_SPINY
    0x72, // OBJ_GOOMBA
];

/// Shell-producing enemies — kept in their own class because some levels require
/// shells to progress. Swapping these with non-shell enemies could make levels unbeatable.
const SHELL_ENEMIES: &[u8] = &[
    0x6C, // OBJ_GREENTROOPA
    0x6D, // OBJ_REDTROOPA
    0x70, // OBJ_BUZZYBEATLE
];

/// Big enemies (Giant World variants). Swap only among themselves.
const BIG_ENEMIES: &[u8] = &[
    0x7A, // OBJ_BIGGREENTROOPA
    0x7B, // OBJ_BIGREDTROOPA
    0x7C, // OBJ_BIGGOOMBA
    0x7E, // OBJ_BIGGREENHOPPER
];

/// Flying/hopping enemies that can be swapped with each other.
const FLYING_ENEMIES: &[u8] = &[
    0x6E, // OBJ_PARATROOPAGREENHOP
    0x6F, // OBJ_FLYINGREDPARATROOPA
    0x73, // OBJ_PARAGOOMBA
    0x74, // OBJ_PARAGOOMBAWITHMICROS
    0x80, // OBJ_FLYINGGREENPARATROOPA
];

/// Water enemies that can be swapped with each other.
const WATER_ENEMIES: &[u8] = &[
    0x61, // OBJ_BLOOPERWITHKIDS
    0x62, // OBJ_BLOOPER
    0x63, // OBJ_BIGBERTHABIRTHER
    0x64, // OBJ_CHEEPCHEEPHOPPER
    0x6A, // OBJ_BLOOPERCHILDSHOOT
];

/// Hammer/Boomerang/Fire Bros — swap among themselves.
const BRO_ENEMIES: &[u8] = &[
    0x81, // OBJ_HAMMERBRO
    0x82, // OBJ_BOOMERANGBRO
    0x86, // OBJ_HEAVYBRO
    0x87, // OBJ_FIREBRO
];

/// Piranha plant variants — swap among themselves.
const PIRANHAS: &[u8] = &[
    0xA0, // OBJ_GREENPIRANHA
    0xA1, // OBJ_GREENPIRANHA_FLIPPED
    0xA2, // OBJ_REDPIRANHA
    0xA3, // OBJ_REDPIRANHA_FLIPPED
    0xA4, // OBJ_GREENPIRANHA_FIRE
    0xA5, // OBJ_GREENPIRANHA_FIREC
    0xA6, // OBJ_VENUSFIRETRAP
    0xA7, // OBJ_VENUSFIRETRAP_CEIL
];

/// Cheep cheep variants (overworld jumping types).
const CHEEPS: &[u8] = &[
    0x77, // OBJ_GREENCHEEP
    0x88, // OBJ_ORANGECHEEP
];

/// Big ? Block IDs — these can be swapped with each other to randomize
/// which suit/powerup the player gets from Big ? Blocks.
const BIG_Q_BLOCKS: &[u8] = &[
    0x94, // OBJ_BIGQBLOCK_3UP
    0x95, // OBJ_BIGQBLOCK_MUSHROOM
    0x96, // OBJ_BIGQBLOCK_FIREFLOWER
    0x97, // OBJ_BIGQBLOCK_SUPERLEAF
    0x98, // OBJ_BIGQBLOCK_TANOOKI
    0x99, // OBJ_BIGQBLOCK_FROG
    0x9A, // OBJ_BIGQBLOCK_HAMMER
];

/// File offset of the Tanooki Big ? Block in World 7-F1.
/// This block must NOT be randomized — flying/Tanooki is required to beat the level.
const W7F1_TANOOKI_OFFSET: usize = 0x0C336;

/// All swap classes collected for lookup.
const ALL_CLASSES: &[&[u8]] = &[
    GROUND_ENEMIES,
    SHELL_ENEMIES,
    BIG_ENEMIES,
    FLYING_ENEMIES,
    WATER_ENEMIES,
    BRO_ENEMIES,
    PIRANHAS,
    CHEEPS,
];

/// Find which class an enemy ID belongs to, if any.
fn find_class(id: u8) -> Option<&'static [u8]> {
    for class in ALL_CLASSES {
        if class.contains(&id) {
            return Some(class);
        }
    }
    None
}

/// Randomize enemies by parsing the structured object data and only swapping
/// object IDs that belong to a known enemy class. Position bytes and all
/// special objects (end-level cards, pipes, platforms, bosses, powerups,
/// autoscroll triggers, cannons, etc.) are never modified.
pub fn randomize<R: Rng>(rom: &mut Rom, rng: &mut R) {
    randomize_object_data(rom, rng, false);
}

/// Randomize Big ? Blocks by swapping their IDs among the set of Big ? Block
/// types. The Tanooki block in World 7-F1 is protected because flying is
/// required to beat that level.
pub fn randomize_big_q_blocks<R: Rng>(rom: &mut Rom, rng: &mut R) {
    randomize_object_data(rom, rng, true);
}

fn randomize_object_data<R: Rng>(rom: &mut Rom, rng: &mut R, big_q_only: bool) {
    let len = ENEMY_DATA_END - ENEMY_DATA_START;
    let mut data = rom.read_range(ENEMY_DATA_START, len).to_vec();

    let mut i = 0;
    while i < data.len() {
        // Skip 0xFF terminators
        if data[i] == 0xFF {
            i += 1;
            continue;
        }

        // First non-FF byte after a terminator is the page/flag byte
        let _page_flag = data[i];
        i += 1;

        // Now parse 3-byte entries until we hit 0xFF or end of data
        while i + 2 < data.len() && data[i] != 0xFF {
            let obj_id = data[i];
            let file_offset = ENEMY_DATA_START + i;

            if big_q_only {
                // Only randomize Big ? Blocks, skip 7-F1 Tanooki
                if BIG_Q_BLOCKS.contains(&obj_id) && file_offset != W7F1_TANOOKI_OFFSET {
                    data[i] = *BIG_Q_BLOCKS.choose(rng).unwrap();
                }
            } else {
                // Only swap if this ID belongs to a known enemy class
                if let Some(class) = find_class(obj_id) {
                    data[i] = *class.choose(rng).unwrap();
                }
            }

            // Advance past the 3-byte entry (id, x, y)
            i += 3;
        }
    }

    rom.write_range(ENEMY_DATA_START, &data);
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    fn make_test_rom() -> Rom {
        let mut data = vec![0u8; 393232];
        // iNES header
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16; // PRG pages
        data[5] = 16; // CHR pages
        data[6] = 0x40; // mapper flags

        // Set up a realistic enemy data segment at ENEMY_DATA_START:
        // FF terminator, then a segment with page flag + entries + FF
        let seg = &[
            0xFF, // leading terminator
            0x01, // page flag
            0x72, 0x0E, 0x19, // Goomba at (0x0E, 0x19)
            0x6C, 0x24, 0x16, // Green Troopa at (0x24, 0x16)
            0xA6, 0x16, 0x17, // Venus Fire Trap at (0x16, 0x17)
            0x41, 0xA8, 0x15, // End Level Card at (0xA8, 0x15) — must not change
            0xD3, 0x00, 0x50, // Autoscroll — must not change
            0xFF, // terminator
        ];
        let start = ENEMY_DATA_START;
        data[start..start + seg.len()].copy_from_slice(seg);

        Rom::from_bytes(&data).unwrap()
    }

    #[test]
    fn test_enemies_stay_in_class() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize(&mut rom, &mut rng);

        // Read back the segment (skip FF + page flag = offset 2)
        let base = ENEMY_DATA_START + 2;
        let result = rom.read_range(base, 15);

        // Goomba should be replaced with a ground enemy
        assert!(
            GROUND_ENEMIES.contains(&result[0]),
            "Goomba replaced with non-ground: 0x{:02X}",
            result[0]
        );
        // Position bytes must be unchanged
        assert_eq!(result[1], 0x0E);
        assert_eq!(result[2], 0x19);

        // Green Troopa should be replaced with a shell enemy
        assert!(
            SHELL_ENEMIES.contains(&result[3]),
            "Green Troopa replaced with non-shell enemy: 0x{:02X}",
            result[3]
        );
        assert_eq!(result[4], 0x24);
        assert_eq!(result[5], 0x16);

        // Venus Fire Trap should be replaced with a piranha
        assert!(
            PIRANHAS.contains(&result[6]),
            "Venus replaced with non-piranha: 0x{:02X}",
            result[6]
        );

        // End Level Card must NOT be changed
        assert_eq!(result[9], 0x41, "End Level Card was modified!");
        assert_eq!(result[10], 0xA8);
        assert_eq!(result[11], 0x15);

        // Autoscroll must NOT be changed
        assert_eq!(result[12], 0xD3, "Autoscroll was modified!");
    }

    #[test]
    fn test_deterministic() {
        let mut rom1 = make_test_rom();
        let mut rom2 = make_test_rom();
        let mut rng1 = ChaCha8Rng::seed_from_u64(77);
        let mut rng2 = ChaCha8Rng::seed_from_u64(77);

        randomize(&mut rom1, &mut rng1);
        randomize(&mut rom2, &mut rng2);

        let len = ENEMY_DATA_END - ENEMY_DATA_START;
        assert_eq!(
            rom1.read_range(ENEMY_DATA_START, len),
            rom2.read_range(ENEMY_DATA_START, len),
        );
    }

    fn make_bigq_test_rom() -> Rom {
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;

        // Segment with a regular Big ? Block (should be randomized)
        let seg1_start = ENEMY_DATA_START;
        let seg1 = &[
            0xFF,
            0x01, // page flag
            0x94, 0x18, 0x05, // BIGQBLOCK_3UP
            0x98, 0x16, 0x14, // BIGQBLOCK_TANOOKI
            0x41, 0xA8, 0x15, // ENDLEVELCARD (must not change)
            0xFF,
        ];
        data[seg1_start..seg1_start + seg1.len()].copy_from_slice(seg1);

        // Place the protected 7-F1 Tanooki at its exact file offset
        // W7F1_TANOOKI_OFFSET = 0x0C336, which is the ID byte of the entry.
        // We need: [FF] [page] [0x98, x, y] [0x41, x, y] [FF]
        // So page byte at 0x0C335, entry at 0x0C336
        let w7f1_seg_start = W7F1_TANOOKI_OFFSET - 2; // FF + page byte before the entry
        data[w7f1_seg_start] = 0xFF;
        data[w7f1_seg_start + 1] = 0x01; // page flag
        data[W7F1_TANOOKI_OFFSET] = 0x98; // BIGQBLOCK_TANOOKI
        data[W7F1_TANOOKI_OFFSET + 1] = 0x0A;
        data[W7F1_TANOOKI_OFFSET + 2] = 0x13;
        data[W7F1_TANOOKI_OFFSET + 3] = 0x41; // ENDLEVELCARD
        data[W7F1_TANOOKI_OFFSET + 4] = 0x48;
        data[W7F1_TANOOKI_OFFSET + 5] = 0x15;
        data[W7F1_TANOOKI_OFFSET + 6] = 0xFF;

        Rom::from_bytes(&data).unwrap()
    }

    #[test]
    fn test_big_q_blocks_randomized() {
        let mut rom = make_bigq_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize_big_q_blocks(&mut rom, &mut rng);

        // Regular Big ? Blocks should be randomized to some Big ? Block ID
        let base = ENEMY_DATA_START + 2; // skip FF + page
        let result = rom.read_range(base, 9);
        assert!(
            BIG_Q_BLOCKS.contains(&result[0]),
            "Big Q block not replaced with Big Q: 0x{:02X}",
            result[0]
        );
        assert!(
            BIG_Q_BLOCKS.contains(&result[3]),
            "Big Q block not replaced with Big Q: 0x{:02X}",
            result[3]
        );
        // End level card must not change
        assert_eq!(result[6], 0x41);
    }

    #[test]
    fn test_7f1_tanooki_protected() {
        let mut rom = make_bigq_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(99);
        randomize_big_q_blocks(&mut rom, &mut rng);

        // The 7-F1 Tanooki must remain 0x98
        let protected = rom.read_byte(W7F1_TANOOKI_OFFSET);
        assert_eq!(
            protected, 0x98,
            "7-F1 Tanooki was changed to 0x{:02X}!",
            protected
        );
    }
}
