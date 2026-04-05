use rand::Rng;
use rand::seq::IndexedRandom;

use crate::rom::Rom;
use super::rom_data::FS_MYSTERY_ANCHOR;

const ANCHOR: u8 = 0x0A;

/// Useful item pool for chest/reward randomization (Global Item IDs).
const GOOD_ITEMS: &[u8] = &[
    0x01, // Mushroom
    0x02, // Fire Flower
    0x03, // Leaf
    0x04, // Frog Suit
    0x05, // Tanooki Suit
    0x06, // Hammer Suit
    0x07, // Jugem's Cloud
    0x08, // P-Wing
    0x09, // Starman
    0x0B, // Hammer
    0x0D, // Music Box
];

/// Powerup-only pool for anchor replacement (excludes non-powerup items like
/// Cloud, P-Wing, Starman which don't change suit).
const POWERUP_ITEMS: &[u8] = &[
    0x01, // Mushroom
    0x02, // Fire Flower
    0x03, // Leaf
    0x04, // Frog Suit
    0x05, // Tanooki Suit
    0x06, // Hammer Suit
];

/// Toad House pool — powerups and combat items only (no map consumables).
const TOAD_HOUSE_ITEMS: &[u8] = &[
    0x01, // Mushroom
    0x02, // Fire Flower
    0x03, // Leaf
    0x04, // Frog Suit
    0x05, // Tanooki Suit
    0x06, // Hammer Suit
    0x08, // P-Wing
    0x09, // Starman
];

const WARP_WHISTLE: u8 = 0x0C;

/// Full item pool including warp whistle (used when remove_whistles is false).
const GOOD_ITEMS_WITH_WHISTLE: &[u8] = &[
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0B, 0x0C, 0x0D,
];



// Hammer Bros map items: 8 worlds x 9 object slots = 72 bytes.
// Non-zero entries are item rewards from defeating Hammer Bros.
const HAMMER_BROS_ITEMS_OFFSET: usize = 0x16190;
const HAMMER_BROS_ITEMS_LEN: usize = 72;

// Princess letter rewards: one item per world (worlds 1-7).
const PRINCESS_REWARDS_OFFSET: usize = 0x360DE;
const PRINCESS_REWARDS_LEN: usize = 7;

// Toad House chests: 7 houses x 3 items = 21 bytes.
const TOAD_HOUSE_ITEMS_OFFSET: usize = 0x3B14B;
const TOAD_HOUSE_ITEMS_LEN: usize = 21;

// In-level treasure chest item offsets (D6 OBJ_TREASURESET Y-byte).
const TREASURE_CHEST_OFFSETS: &[usize] = &[
    0x0C427, // Music Box chest
    0x0CE9F, // Cloud chest
    0x0D0E2, // Leaf chest
    0x0D36A, // Warp Whistle chest
    0x0DA3F, // Star chest
];

// Known warp whistle byte locations across all item tables.
const WHISTLE_OFFSETS: &[usize] = &[
    0x1619D, // Hammer Bros W2 obj[4]
    0x3B14B, // Toad House 0 slot 0
    0x0D36A, // In-level treasure D6 Y-byte
];

/// Randomize all chest and reward items: Hammer Bros drops, Princess letter
/// rewards, Toad House chests, and in-level treasure chests.
///
/// When `remove_whistles` is true, warp whistles are excluded from the item
/// pool so they never appear.
pub fn randomize<R: Rng>(rom: &mut Rom, rng: &mut R, remove_whistles: bool) {
    let pool = if remove_whistles {
        GOOD_ITEMS
    } else {
        GOOD_ITEMS_WITH_WHISTLE
    };

    // Hammer Bros map items: randomize non-zero entries only (zero = no item).
    let mut hb = rom.read_range(HAMMER_BROS_ITEMS_OFFSET, HAMMER_BROS_ITEMS_LEN).to_vec();
    for byte in &mut hb {
        if *byte != 0 {
            *byte = *pool.choose(rng).unwrap();
        }
    }
    rom.write_range(HAMMER_BROS_ITEMS_OFFSET, &hb);

    // Princess letter rewards: randomize non-zero entries (0x00 = no reward).
    let mut pr = rom.read_range(PRINCESS_REWARDS_OFFSET, PRINCESS_REWARDS_LEN).to_vec();
    for byte in &mut pr {
        if *byte != 0 {
            *byte = *pool.choose(rng).unwrap();
        }
    }
    rom.write_range(PRINCESS_REWARDS_OFFSET, &pr);

    // Toad House chests: use restricted pool (no cloud/hammer/music box/whistle).
    let mut th = rom.read_range(TOAD_HOUSE_ITEMS_OFFSET, TOAD_HOUSE_ITEMS_LEN).to_vec();
    for byte in &mut th {
        *byte = *TOAD_HOUSE_ITEMS.choose(rng).unwrap();
    }
    rom.write_range(TOAD_HOUSE_ITEMS_OFFSET, &th);

    // In-level treasure chests: randomize each D6 Y-byte.
    for &offset in TREASURE_CHEST_OFFSETS {
        rom.write_byte(offset, *pool.choose(rng).unwrap());
    }
}

/// Replace all anchor items (0x0A) in item tables with a single randomly
/// chosen powerup. Since the airship lock patch makes anchors unnecessary,
/// this turns every anchor pickup into the same powerup for a given seed
/// (e.g., all anchors become Hammer Suits). The sprite is not changed —
/// only the item ID in the data tables.
pub fn replace_anchors<R: Rng>(rom: &mut Rom, rng: &mut R) {
    let replacement = *POWERUP_ITEMS.choose(rng).unwrap();

    // Hammer Bros map items
    let mut hb = rom.read_range(HAMMER_BROS_ITEMS_OFFSET, HAMMER_BROS_ITEMS_LEN).to_vec();
    for byte in &mut hb {
        if *byte == ANCHOR {
            *byte = replacement;
        }
    }
    rom.write_range(HAMMER_BROS_ITEMS_OFFSET, &hb);

    // Princess letter rewards
    let mut pr = rom.read_range(PRINCESS_REWARDS_OFFSET, PRINCESS_REWARDS_LEN).to_vec();
    for byte in &mut pr {
        if *byte == ANCHOR {
            *byte = replacement;
        }
    }
    rom.write_range(PRINCESS_REWARDS_OFFSET, &pr);

    // Toad House chests
    let mut th = rom.read_range(TOAD_HOUSE_ITEMS_OFFSET, TOAD_HOUSE_ITEMS_LEN).to_vec();
    for byte in &mut th {
        if *byte == ANCHOR {
            *byte = replacement;
        }
    }
    rom.write_range(TOAD_HOUSE_ITEMS_OFFSET, &th);
}

/// Remove warp whistles without full item randomization. Replaces the 3 known
/// whistle locations with a random item from the good pool.
pub fn remove_whistles_only<R: Rng>(rom: &mut Rom, rng: &mut R) {
    for &offset in WHISTLE_OFFSETS {
        if rom.read_byte(offset) == WARP_WHISTLE {
            rom.write_byte(offset, *GOOD_ITEMS.choose(rng).unwrap());
        }
    }
}

/// Mystery anchor pool — all items that make sense when used from the map
/// inventory. Anchor items stay as 0x0A in data tables but the item-use
/// dispatch is patched so using an anchor triggers a random powerup effect.
const MYSTERY_ANCHOR_POOL: &[u8] = &[
    0x01, // Mushroom
    0x02, // Fire Flower
    0x03, // Super Leaf
    0x04, // Frog Suit
    0x05, // Tanooki Suit
    0x06, // Hammer Suit
    0x07, // Jugem's Cloud
    0x08, // P-Wing
    0x09, // Starman
    0x0B, // Hammer (rock breaker)
];

/// Patch the item-use dispatch so anchors secretly function as a random
/// powerup chosen at build time. The anchor sprite stays unchanged in the
/// inventory — only the effect changes when the player uses it.
///
/// Writes a 15-byte trampoline at free space in PRG031 and redirects the
/// anchor branch in the item dispatch to it.
pub fn write_mystery_anchor<R: Rng>(rom: &mut Rom, rng: &mut R) {
    let target = *MYSTERY_ANCHOR_POOL.choose(rng).unwrap();

    // Trampoline at FS_MYSTERY_ANCHOR (CPU $E240):
    //   A9 xx        LDA #<target_id>
    //   8D F5 07     STA $07F5
    //   C9 09        CMP #$09
    //   F0 03        BEQ +3          ; starman needs special handler
    //   4C 7B E3     JMP $E37B       ; normal item handler
    //   4C 1B E5     JMP $E51B       ; star/anchor handler (sets invincibility)
    let trampoline: [u8; 15] = [
        0xA9, target,       // LDA #target
        0x8D, 0xF5, 0x07,   // STA $07F5
        0xC9, 0x09,          // CMP #$09
        0xF0, 0x03,          // BEQ +3
        0x4C, 0x7B, 0xE3,   // JMP $E37B
        0x4C, 0x1B, 0xE5,   // JMP $E51B
    ];
    rom.write_range(FS_MYSTERY_ANCHOR, &trampoline);

    // Patch dispatch at file 0x3E500 (CPU $E4F0):
    // Original: C9 0A F0 27  (CMP #$0A; BEQ +$27)
    // Patched:  4C 40 E2 EA  (JMP $E240; NOP)
    rom.write_range(0x3E500, &[0x4C, 0x40, 0xE2, 0xEA]);
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

        // Hammer Bros items: W1 obj[2]=Star, W2 obj[4]=Whistle
        data[HAMMER_BROS_ITEMS_OFFSET + 2] = 0x09; // W1 obj[2] star
        data[HAMMER_BROS_ITEMS_OFFSET + 9 + 4] = WARP_WHISTLE; // W2 obj[4]

        // Princess rewards
        data[PRINCESS_REWARDS_OFFSET] = 0x08; // W1: P-Wing
        data[PRINCESS_REWARDS_OFFSET + 1] = 0x07; // W2: Cloud
        data[PRINCESS_REWARDS_OFFSET + 6] = 0x00; // W7: Nothing

        // Toad House chests
        data[TOAD_HOUSE_ITEMS_OFFSET] = WARP_WHISTLE; // House 0 slot 0
        data[TOAD_HOUSE_ITEMS_OFFSET + 1] = 0x08; // House 0 slot 1
        data[TOAD_HOUSE_ITEMS_OFFSET + 2] = 0x04; // House 0 slot 2

        // In-level treasure chests (place D6 objects so Y-byte is at the right offset)
        for &offset in TREASURE_CHEST_OFFSETS {
            data[offset - 2] = 0xD6; // D6 object ID
            data[offset - 1] = 0x32; // X byte (arbitrary)
            data[offset] = 0x09;     // Y byte (star)
        }
        // Make the whistle chest actually a whistle
        data[0x0D36A] = WARP_WHISTLE;

        Rom::from_bytes(&data).unwrap()
    }

    #[test]
    fn test_items_randomized() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize(&mut rom, &mut rng, true);

        // Toad House items should all be valid items
        for i in 0..TOAD_HOUSE_ITEMS_LEN {
            let b = rom.read_byte(TOAD_HOUSE_ITEMS_OFFSET + i);
            assert!(GOOD_ITEMS.contains(&b), "Toad House byte {i} = 0x{b:02X}");
        }

        // Treasure chest items should be valid
        for &offset in TREASURE_CHEST_OFFSETS {
            let b = rom.read_byte(offset);
            assert!(GOOD_ITEMS.contains(&b), "Treasure at 0x{offset:05X} = 0x{b:02X}");
        }
    }

    #[test]
    fn test_zero_slots_preserved() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize(&mut rom, &mut rng, true);

        // W8 Hammer Bros items (all zero) should stay zero
        for i in 0..9 {
            let offset = HAMMER_BROS_ITEMS_OFFSET + 7 * 9 + i;
            assert_eq!(rom.read_byte(offset), 0x00, "W8 obj[{i}] should be 0");
        }

        // Princess W7 (0x00) should stay zero
        assert_eq!(rom.read_byte(PRINCESS_REWARDS_OFFSET + 6), 0x00);
    }

    #[test]
    fn test_whistles_removed() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize(&mut rom, &mut rng, true);

        for &offset in WHISTLE_OFFSETS {
            let b = rom.read_byte(offset);
            assert_ne!(b, WARP_WHISTLE, "Whistle not removed at 0x{offset:05X}");
            assert!(GOOD_ITEMS.contains(&b), "Invalid item at 0x{offset:05X}: 0x{b:02X}");
        }
    }

    #[test]
    fn test_whistles_kept_when_allowed() {
        // With remove_whistles=false, the pool includes whistle.
        // Run many seeds to verify whistle can appear.
        let mut found_whistle = false;
        for seed in 0..100 {
            let mut rom = make_test_rom();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom, &mut rng, false);

            for i in 0..HAMMER_BROS_ITEMS_LEN {
                if rom.read_byte(HAMMER_BROS_ITEMS_OFFSET + i) == WARP_WHISTLE {
                    found_whistle = true;
                    break;
                }
            }
            if found_whistle {
                break;
            }
        }
        assert!(found_whistle, "Whistle never appeared in 100 seeds with remove_whistles=false");
    }

    #[test]
    fn test_remove_whistles_only() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        // Verify whistles exist before
        assert_eq!(rom.read_byte(0x1619D), WARP_WHISTLE);
        assert_eq!(rom.read_byte(0x3B14B), WARP_WHISTLE);
        assert_eq!(rom.read_byte(0x0D36A), WARP_WHISTLE);

        remove_whistles_only(&mut rom, &mut rng);

        // Whistles should be replaced
        for &offset in WHISTLE_OFFSETS {
            let b = rom.read_byte(offset);
            assert_ne!(b, WARP_WHISTLE, "Whistle not removed at 0x{offset:05X}");
            assert!(GOOD_ITEMS.contains(&b));
        }

        // Non-whistle items should be untouched
        assert_eq!(rom.read_byte(HAMMER_BROS_ITEMS_OFFSET + 2), 0x09); // W1 star
        assert_eq!(rom.read_byte(PRINCESS_REWARDS_OFFSET), 0x08); // W1 P-Wing
    }

    #[test]
    fn test_deterministic() {
        let mut rom1 = make_test_rom();
        let mut rom2 = make_test_rom();
        let mut rng1 = ChaCha8Rng::seed_from_u64(123);
        let mut rng2 = ChaCha8Rng::seed_from_u64(123);

        randomize(&mut rom1, &mut rng1, true);
        randomize(&mut rom2, &mut rng2, true);

        // Check all item regions are identical
        assert_eq!(
            rom1.read_range(HAMMER_BROS_ITEMS_OFFSET, HAMMER_BROS_ITEMS_LEN),
            rom2.read_range(HAMMER_BROS_ITEMS_OFFSET, HAMMER_BROS_ITEMS_LEN),
        );
        assert_eq!(
            rom1.read_range(PRINCESS_REWARDS_OFFSET, PRINCESS_REWARDS_LEN),
            rom2.read_range(PRINCESS_REWARDS_OFFSET, PRINCESS_REWARDS_LEN),
        );
        assert_eq!(
            rom1.read_range(TOAD_HOUSE_ITEMS_OFFSET, TOAD_HOUSE_ITEMS_LEN),
            rom2.read_range(TOAD_HOUSE_ITEMS_OFFSET, TOAD_HOUSE_ITEMS_LEN),
        );
        for &offset in TREASURE_CHEST_OFFSETS {
            assert_eq!(rom1.read_byte(offset), rom2.read_byte(offset));
        }
    }

    #[test]
    fn test_mystery_anchor_trampoline_written() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        write_mystery_anchor(&mut rom, &mut rng);

        // Trampoline starts with LDA #imm
        assert_eq!(rom.read_byte(FS_MYSTERY_ANCHOR), 0xA9);
        let target = rom.read_byte(FS_MYSTERY_ANCHOR + 1);
        assert!(MYSTERY_ANCHOR_POOL.contains(&target),
            "Target 0x{target:02X} not in mystery pool");

        // STA $07F5
        assert_eq!(rom.read_range(FS_MYSTERY_ANCHOR + 2, 3), &[0x8D, 0xF5, 0x07]);
        // CMP #$09; BEQ +3
        assert_eq!(rom.read_range(FS_MYSTERY_ANCHOR + 5, 4), &[0xC9, 0x09, 0xF0, 0x03]);
        // JMP $E37B (normal handler)
        assert_eq!(rom.read_range(FS_MYSTERY_ANCHOR + 9, 3), &[0x4C, 0x7B, 0xE3]);
        // JMP $E51B (star handler)
        assert_eq!(rom.read_range(FS_MYSTERY_ANCHOR + 12, 3), &[0x4C, 0x1B, 0xE5]);
    }

    #[test]
    fn test_mystery_anchor_dispatch_patched() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        write_mystery_anchor(&mut rom, &mut rng);

        // Dispatch at 0x3E500: JMP $E240; NOP
        assert_eq!(rom.read_range(0x3E500, 4), &[0x4C, 0x40, 0xE2, 0xEA]);
    }

    #[test]
    fn test_mystery_anchor_pool_coverage() {
        use std::collections::HashSet;
        let mut seen = HashSet::new();
        for seed in 0..500u64 {
            let mut rom = make_test_rom();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            write_mystery_anchor(&mut rom, &mut rng);
            seen.insert(rom.read_byte(FS_MYSTERY_ANCHOR + 1));
        }
        for &item in MYSTERY_ANCHOR_POOL {
            assert!(seen.contains(&item),
                "Item 0x{item:02X} never appeared in 500 seeds");
        }
    }

    #[test]
    fn test_mystery_anchor_deterministic() {
        let mut rom1 = make_test_rom();
        let mut rom2 = make_test_rom();
        let mut rng1 = ChaCha8Rng::seed_from_u64(77);
        let mut rng2 = ChaCha8Rng::seed_from_u64(77);
        write_mystery_anchor(&mut rom1, &mut rng1);
        write_mystery_anchor(&mut rom2, &mut rng2);

        assert_eq!(
            rom1.read_range(FS_MYSTERY_ANCHOR, 15),
            rom2.read_range(FS_MYSTERY_ANCHOR, 15),
            "Same seed should produce identical trampoline"
        );
    }

    #[test]
    fn test_mystery_anchor_leaves_item_tables_intact() {
        let mut rom = make_test_rom();
        // Place anchors in item tables
        rom.write_byte(HAMMER_BROS_ITEMS_OFFSET + 2, ANCHOR);
        rom.write_byte(PRINCESS_REWARDS_OFFSET, ANCHOR);
        rom.write_byte(TOAD_HOUSE_ITEMS_OFFSET + 1, ANCHOR);

        let mut rng = ChaCha8Rng::seed_from_u64(42);
        write_mystery_anchor(&mut rom, &mut rng);

        // Anchors should remain untouched in all tables
        assert_eq!(rom.read_byte(HAMMER_BROS_ITEMS_OFFSET + 2), ANCHOR);
        assert_eq!(rom.read_byte(PRINCESS_REWARDS_OFFSET), ANCHOR);
        assert_eq!(rom.read_byte(TOAD_HOUSE_ITEMS_OFFSET + 1), ANCHOR);
    }
}
