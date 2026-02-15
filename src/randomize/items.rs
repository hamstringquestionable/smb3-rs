use rand::Rng;
use rand::seq::IndexedRandom;

use crate::rom::Rom;

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
];

const WARP_WHISTLE: u8 = 0x0C;

/// Full item pool including warp whistle (used when remove_whistles is false).
const GOOD_ITEMS_WITH_WHISTLE: &[u8] = &[
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0C,
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

    // Toad House chests: randomize all 21 bytes.
    let mut th = rom.read_range(TOAD_HOUSE_ITEMS_OFFSET, TOAD_HOUSE_ITEMS_LEN).to_vec();
    for byte in &mut th {
        *byte = *pool.choose(rng).unwrap();
    }
    rom.write_range(TOAD_HOUSE_ITEMS_OFFSET, &th);

    // In-level treasure chests: randomize each D6 Y-byte.
    for &offset in TREASURE_CHEST_OFFSETS {
        rom.write_byte(offset, *pool.choose(rng).unwrap());
    }
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

            for i in 0..TOAD_HOUSE_ITEMS_LEN {
                if rom.read_byte(TOAD_HOUSE_ITEMS_OFFSET + i) == WARP_WHISTLE {
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
}
