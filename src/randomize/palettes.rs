use rand::seq::IndexedRandom;
use rand::Rng;

use crate::randomize::palette_variants::{
    VariantGroup,
    PLAINS_SLOT3_VARIANTS, SLOT2_VARIANTS, SLOT4_VARIANTS, SLOT5_VARIANTS,
    SLOT6_VARIANTS, SLOT7_VARIANTS, SLICE1_WATER_VARIANTS, SLICE2_VARIANTS,
    SLICE3_GIANT_VARIANTS, SLICE4_HEAD_VARIANTS, SLICE4_TAIL_VARIANTS,
};
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

/// Themed palette randomization across all tilesets.
///
/// Uses palette-group SWAP randomization: for each curated quartet position,
/// the randomizer picks ONE whole 4-byte variant from a list of pre-validated
/// options (vanilla + Recolored). This guarantees every emitted palette was
/// designed as a coherent unit — no flat color-pool mixing, no independent
/// byte picks, no risk of cross-palette clash.
///
/// Coverage: every quartet Recolored changed across the themed-slot table
/// (slots 2-7) and master-pool slices 1-4 (skipping the level-layout pointer
/// table at 0x377E0-0x37807). With two variants per position (vanilla,
/// Recolored), variety scales as 2^(changed positions). Additional variants
/// per position (hand-curated or from other palette hacks) can be appended to
/// each `VariantGroup`'s `variants` list without touching this code path.
///
/// Positions that Recolored didn't change are omitted from the variants tables —
/// they stay vanilla always.
pub fn randomize_themed<R: Rng>(rom: &mut Rom, rng: &mut R) {
    // Character palettes stay randomized too — independent of tileset palettes.
    randomize(rom, rng);

    apply_variant_groups(rom, SLOT2_VARIANTS, rng);
    apply_variant_groups(rom, PLAINS_SLOT3_VARIANTS, rng);
    apply_variant_groups(rom, SLOT4_VARIANTS, rng);
    apply_variant_groups(rom, SLOT5_VARIANTS, rng);
    apply_variant_groups(rom, SLOT6_VARIANTS, rng);
    apply_variant_groups(rom, SLOT7_VARIANTS, rng);
    apply_variant_groups(rom, SLICE1_WATER_VARIANTS, rng);
    apply_variant_groups(rom, SLICE2_VARIANTS, rng);
    apply_variant_groups(rom, SLICE3_GIANT_VARIANTS, rng);
    apply_variant_groups(rom, SLICE4_HEAD_VARIANTS, rng);
    apply_variant_groups(rom, SLICE4_TAIL_VARIANTS, rng);
}

/// For each curated position, pick one 4-byte variant at random and write it.
fn apply_variant_groups<R: Rng>(rom: &mut Rom, groups: &[VariantGroup], rng: &mut R) {
    for group in groups {
        let picked = group.variants.choose(rng).unwrap();
        rom.write_range(group.offset, picked);
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

    /// All variant constants applied by `randomize_themed` (except character
    /// palettes, which have their own test).
    fn all_variant_groups() -> Vec<&'static VariantGroup> {
        use crate::randomize::palette_variants::*;
        let mut v: Vec<&'static VariantGroup> = Vec::new();
        for slice in [
            SLOT2_VARIANTS, PLAINS_SLOT3_VARIANTS, SLOT4_VARIANTS, SLOT5_VARIANTS,
            SLOT6_VARIANTS, SLOT7_VARIANTS, SLICE1_WATER_VARIANTS, SLICE2_VARIANTS,
            SLICE3_GIANT_VARIANTS, SLICE4_HEAD_VARIANTS, SLICE4_TAIL_VARIANTS,
        ] {
            v.extend(slice.iter());
        }
        v
    }

    #[test]
    fn themed_emits_curated_variants_only() {
        // Every 4-byte write at a curated position must exactly match one of the
        // pre-registered variants — no random pool fallback, no free-byte picks.
        for seed in [1u64, 42, 99, 777, 12345] {
            let mut rom = make_test_rom();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize_themed(&mut rom, &mut rng);
            for group in all_variant_groups() {
                let written = rom.read_range(group.offset, 4);
                let matched = group.variants.iter().any(|v| v == written);
                assert!(
                    matched,
                    "seed {seed} at {:#08x}: wrote {:02x?}, must match one curated variant",
                    group.offset, written
                );
            }
        }
    }

    #[test]
    fn themed_does_not_touch_uncurated_positions() {
        // For every region covered, offsets not in any VariantGroup must stay
        // untouched. We stamp recognizable canary bytes in each covered range
        // and check them after running the randomizer.
        const REGIONS: &[(usize, usize, u8)] = &[
            (0x36C54, 0x36C8C, 0xA0),  // slot 2
            (0x36C8C, 0x36CC4, 0xC0),  // slot 3
            (0x36CC4, 0x36CFC, 0xD0),  // slot 4
            (0x36CFC, 0x36D34, 0xE0),  // slot 5
            (0x36D34, 0x36D6C, 0x90),  // slot 6
            (0x36D6C, 0x36DA6, 0x80),  // slot 7
            (0x37000, 0x37200, 0x70),  // slice 1
            (0x37200, 0x37400, 0x60),  // slice 2
            (0x37400, 0x37600, 0x50),  // slice 3
            (0x37600, 0x377E0, 0xB0),  // slice 4 head
            (0x37808, 0x37846, 0xC0),  // slice 4 tail
        ];

        let mut rom = make_test_rom();
        for &(start, end, base) in REGIONS {
            let canary: Vec<u8> =
                (0..(end - start)).map(|i| base | (i as u8 & 0x0F)).collect();
            rom.write_range(start, &canary);
        }

        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize_themed(&mut rom, &mut rng);

        let curated_offsets: std::collections::HashSet<usize> = all_variant_groups()
            .iter()
            .flat_map(|g| (0..4).map(move |k| g.offset + k))
            .collect();

        for &(start, end, base) in REGIONS {
            for off in start..end {
                if curated_offsets.contains(&off) {
                    continue;
                }
                assert_eq!(
                    rom.read_byte(off),
                    base | ((off - start) as u8 & 0x0F),
                    "uncurated offset {:#08x} (region {:#08x}-{:#08x}) was modified",
                    off,
                    start,
                    end,
                );
            }
        }
    }

    #[test]
    fn themed_does_not_touch_pointer_table() {
        // The 40-byte region 0x377E0-0x37807 is a level-layout pointer table;
        // painting it crashes the game. Themed randomizer must leave it alone.
        let mut rom = make_test_rom();
        let vanilla: Vec<u8> = (0..0x28).map(|i| 0xAB + (i as u8 & 0x0F)).collect();
        rom.write_range(0x377E0, &vanilla);

        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize_themed(&mut rom, &mut rng);

        assert_eq!(
            rom.read_range(0x377E0, 0x28),
            &vanilla[..],
            "pointer table 0x377E0-0x37807 must not be modified"
        );
    }

    #[test]
    fn no_variant_group_overlaps_pointer_table() {
        // Static sanity: no curated offset can fall inside the pointer-table
        // crash trap, even transitively (offset + 3 still < 0x377E0, or
        // offset >= 0x37808).
        for group in all_variant_groups() {
            let start = group.offset;
            let end = group.offset + 4;
            let overlaps = end > 0x377E0 && start < 0x37808;
            assert!(
                !overlaps,
                "VariantGroup at {:#08x} overlaps pointer table 0x377E0-0x37807",
                group.offset
            );
        }
    }
}
