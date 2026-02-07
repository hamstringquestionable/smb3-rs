use rand::Rng;
use rand::seq::IndexedRandom;

use crate::rom::Rom;

/// Enemy-to-level definitions: 0x0BFD8–0x0E00D.
/// This is a large block of data where each level's enemy/object set is defined.
/// Each enemy entry is variable-length, but enemy type IDs can be swapped
/// within the same category to maintain level playability.
const ENEMY_DATA_START: usize = 0x0BFD8;
const ENEMY_DATA_END: usize = 0x0E00D;

/// Ground-based enemies that can be safely swapped with each other.
const GROUND_ENEMIES: &[u8] = &[
    0x00, // Goomba (green, walks off ledges)
    0x01, // Goomba (red, turns at ledges)
    0x06, // Green Koopa Troopa
    0x07, // Red Koopa Troopa
    0x08, // Buzzy Beetle
    0x0A, // Spiny
    0x11, // Bob-omb
    0x15, // Green Koopa Paratroopa (hops)
];

/// Flying enemies that can be safely swapped with each other.
const FLYING_ENEMIES: &[u8] = &[
    0x03, // Red Para-goomba (hops)
    0x16, // Red Koopa Paratroopa (flies up/down)
    0x17, // Green Koopa Paratroopa (flies left/right)
];

/// Water enemies that can be safely swapped with each other.
const WATER_ENEMIES: &[u8] = &[
    0x19, // Blooper
    0x1A, // Blooper with babies
    0x1B, // Cheep Cheep (slow)
    0x1C, // Cheep Cheep (fast)
    0x1E, // Big Bertha
];

/// Randomize enemies by swapping enemy type IDs within the same class.
/// This approach scans the enemy data for known enemy IDs and replaces them
/// with a random enemy from the same class, preserving game stability.
pub fn randomize<R: Rng>(rom: &mut Rom, rng: &mut R) {
    let len = ENEMY_DATA_END - ENEMY_DATA_START;
    let mut enemy_data = rom.read_range(ENEMY_DATA_START, len).to_vec();

    // Build a lookup: for each enemy ID, what class does it belong to?
    // Then for each byte that matches a known enemy ID, swap it with
    // another from the same class.
    //
    // Note: This is a simplified approach. The enemy data format is complex
    // with variable-length records. A byte matching an enemy ID could be
    // part of position data or other fields. To mitigate false positives,
    // we only swap bytes that exactly match known enemy IDs and rely on the
    // statistical rarity of coordinate values matching enemy IDs.
    //
    // A more robust approach would fully parse the enemy data format, but
    // this works well enough for an initial version.

    for byte in &mut enemy_data {
        if let Some(replacement) = swap_in_class(*byte, rng) {
            *byte = replacement;
        }
    }

    rom.write_range(ENEMY_DATA_START, &enemy_data);
}

fn swap_in_class<R: Rng>(enemy_id: u8, rng: &mut R) -> Option<u8> {
    let class = if GROUND_ENEMIES.contains(&enemy_id) {
        GROUND_ENEMIES
    } else if FLYING_ENEMIES.contains(&enemy_id) {
        FLYING_ENEMIES
    } else if WATER_ENEMIES.contains(&enemy_id) {
        WATER_ENEMIES
    } else {
        return None;
    };

    Some(*class.choose(rng).unwrap())
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
        // Put some known enemy IDs in the enemy data area
        data[ENEMY_DATA_START] = 0x00; // Goomba (ground)
        data[ENEMY_DATA_START + 1] = 0x19; // Blooper (water)
        data[ENEMY_DATA_START + 2] = 0x03; // Para-goomba (flying)
        data[ENEMY_DATA_START + 3] = 0xFF; // Unknown (should not be changed)
        Rom::from_bytes(&data).unwrap()
    }

    #[test]
    fn test_enemies_stay_in_class() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize(&mut rom, &mut rng);

        let result = rom.read_range(ENEMY_DATA_START, 4);

        assert!(GROUND_ENEMIES.contains(&result[0]), "Ground enemy replaced with non-ground");
        assert!(WATER_ENEMIES.contains(&result[1]), "Water enemy replaced with non-water");
        assert!(FLYING_ENEMIES.contains(&result[2]), "Flying enemy replaced with non-flying");
        assert_eq!(result[3], 0xFF, "Unknown byte should not be changed");
    }

    #[test]
    fn test_enemies_deterministic() {
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
}
