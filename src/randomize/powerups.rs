use rand::Rng;

use crate::rom::Rom;

/// ROM offset for bumped block attribute data (8 bytes: 0x02611–0x02618).
/// These define what item type each "?" block produces.
///
/// Known item IDs in SMB3:
/// 0x00 = Mushroom (becomes Fire Flower if already big)
/// 0x01 = Super Leaf (Raccoon power)
/// 0x02 = Fire Flower
/// 0x03 = Frog Suit
/// 0x04 = Tanooki Suit
/// 0x05 = Hammer Suit
/// 0x06 = Jugem's Cloud
/// 0x07 = P-Wing
/// 0x08 = Starman
const BLOCK_ATTR_START: usize = 0x02611;
const BLOCK_ATTR_LEN: usize = 8;

/// Valid power-up item IDs that are safe to place in blocks.
const VALID_POWERUP_IDS: &[u8] = &[
    0x00, // Mushroom / Fire Flower
    0x01, // Super Leaf
    0x02, // Fire Flower
    0x03, // Frog Suit
    0x04, // Tanooki Suit
    0x05, // Hammer Suit
    0x08, // Starman
];

/// Randomize the power-up items that come from "?" blocks.
pub fn randomize<R: Rng>(rom: &mut Rom, rng: &mut R) {
    let mut items = rom.read_range(BLOCK_ATTR_START, BLOCK_ATTR_LEN).to_vec();

    for item in &mut items {
        *item = VALID_POWERUP_IDS[rng.random_range(..VALID_POWERUP_IDS.len())];
    }

    rom.write_range(BLOCK_ATTR_START, &items);
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
        data[4] = 16; // PRG pages
        data[5] = 16; // CHR pages
        data[6] = 0x40; // mapper 4
        // Put known values in the powerup area
        for i in 0..BLOCK_ATTR_LEN {
            data[BLOCK_ATTR_START + i] = 0x00;
        }
        Rom::from_bytes(&data).unwrap()
    }

    #[test]
    fn test_powerups_randomized() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize(&mut rom, &mut rng);

        let result = rom.read_range(BLOCK_ATTR_START, BLOCK_ATTR_LEN);
        // All values should be valid powerup IDs
        for &byte in result {
            assert!(VALID_POWERUP_IDS.contains(&byte), "Invalid powerup ID: {byte:#04x}");
        }
    }

    #[test]
    fn test_deterministic_with_seed() {
        let mut rom1 = make_test_rom();
        let mut rom2 = make_test_rom();
        let mut rng1 = ChaCha8Rng::seed_from_u64(123);
        let mut rng2 = ChaCha8Rng::seed_from_u64(123);

        randomize(&mut rom1, &mut rng1);
        randomize(&mut rom2, &mut rng2);

        assert_eq!(
            rom1.read_range(BLOCK_ATTR_START, BLOCK_ATTR_LEN),
            rom2.read_range(BLOCK_ATTR_START, BLOCK_ATTR_LEN),
        );
    }
}
