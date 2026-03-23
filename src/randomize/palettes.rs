use rand::Rng;

use crate::rom::Rom;

/// Character sprite palette entries: [bg_mirror(0x00), body, highlight, outline(0x0F)].
/// Byte 0 must stay 0x00 — it mirrors $3F00 (universal background color) via the PPU.
/// Byte 3 is the outline/shadow color (0x0F). Only bytes 1-2 are randomized.
const PALETTE_RANGES: &[(usize, &str)] = &[
    (0x10539, "Small/Big/Raccoon Mario"),
    (0x1053D, "Small/Big/Raccoon Luigi"),
    (0x10541, "Fire Mario/Luigi"),
    (0x10549, "Frog Mario/Luigi"),
    (0x1054D, "Tanooki Mario/Luigi"),
    (0x10551, "Hammer Mario/Luigi"),
];

/// Valid NES colors that produce good visible results.
/// Excludes 0x0D (known to cause issues on some hardware),
/// 0x0E, 0x0F (black variants), and 0x20+ emphasis colors.
/// We stick to the base 64-color NES palette (0x00–0x3F) minus problematic entries.
const SAFE_COLORS: &[u8] = &[
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
    0x08, 0x09, 0x0A, 0x0B, 0x0C,
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17,
    0x18, 0x19, 0x1A, 0x1B, 0x1C,
    0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27,
    0x28, 0x29, 0x2A, 0x2B, 0x2C,
    0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37,
    0x38, 0x39, 0x3A, 0x3B, 0x3C,
];

/// Randomize character sprite palettes (Mario/Luigi power-up colors).
pub fn randomize<R: Rng>(rom: &mut Rom, rng: &mut R) {
    for &(offset, _name) in PALETTE_RANGES {
        // Randomize bytes 1-2 (body and highlight), preserve byte 0 (bg mirror) and 3 (outline)
        for i in 1..3 {
            rom.write_byte(offset + i, SAFE_COLORS[rng.random_range(..SAFE_COLORS.len())]);
        }
    }
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
        Rom::from_bytes(&data).unwrap()
    }

    #[test]
    fn test_palettes_use_safe_colors() {
        let mut rom = make_test_rom();
        // Set vanilla values
        rom.write_range(0x10539, &[0x00, 0x16, 0x36, 0x0F]);

        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize(&mut rom, &mut rng);

        for &(offset, name) in PALETTE_RANGES {
            for i in 1..3 {
                let byte = rom.read_byte(offset + i);
                assert!(
                    SAFE_COLORS.contains(&byte),
                    "Palette '{name}' at {offset:#06x}+{i} has unsafe color: {byte:#04x}"
                );
            }
        }

        // Verify byte 0 (bg mirror) and byte 3 (outline) are preserved
        assert_eq!(rom.read_byte(0x10539), 0x00, "Mario byte 0 must stay 0x00");
        assert_eq!(rom.read_byte(0x1053C), 0x0F, "Mario outline must stay 0x0F");
    }

    #[test]
    fn test_palettes_deterministic() {
        let mut rom1 = make_test_rom();
        let mut rom2 = make_test_rom();
        let mut rng1 = ChaCha8Rng::seed_from_u64(99);
        let mut rng2 = ChaCha8Rng::seed_from_u64(99);

        randomize(&mut rom1, &mut rng1);
        randomize(&mut rom2, &mut rng2);

        for &(offset, _) in PALETTE_RANGES {
            assert_eq!(
                rom1.read_range(offset, 4),
                rom2.read_range(offset, 4),
            );
        }
    }
}
