use rand::seq::IndexedRandom;
use rand::Rng;

use crate::randomize::palette_variants::{
    VariantGroup,
    PLAINS_SLOT3_VARIANTS, SLOT0_MAP_VARIANTS, SLOT1_MAP_VARIANTS, SLOT2_VARIANTS,
    SLOT4_VARIANTS, SLOT5_VARIANTS, SLOT6_VARIANTS, SLOT7_VARIANTS, SLOT_TAIL_VARIANTS,
    SLICE1_WATER_VARIANTS, SLICE2_VARIANTS, SLICE3_GIANT_VARIANTS,
    SLICE4_HEAD_VARIANTS, SLICE4_POST_VARIANTS, SLICE4_TAIL_VARIANTS,
    POOL_VARIANTS, ROTATE_ONLY_QUARTETS,
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

/// All variant-group regions applied by `randomize_themed`, in write order.
const THEMED_REGIONS: &[&[VariantGroup]] = &[
    SLOT0_MAP_VARIANTS,
    SLOT1_MAP_VARIANTS,
    SLOT2_VARIANTS,
    PLAINS_SLOT3_VARIANTS,
    SLOT4_VARIANTS,
    SLOT5_VARIANTS,
    SLOT6_VARIANTS,
    SLOT7_VARIANTS,
    SLOT_TAIL_VARIANTS,
    POOL_VARIANTS,
    SLICE1_WATER_VARIANTS,
    SLICE2_VARIANTS,
    SLICE3_GIANT_VARIANTS,
    SLICE4_HEAD_VARIANTS,
    SLICE4_TAIL_VARIANTS,
    SLICE4_POST_VARIANTS,
];

/// A context-aware theme group: a set of palette regions that paint the same
/// screens, sharing one hue shift per roll, constrained to shifts that keep
/// the group's dominant colors plausible.
struct ThemeGroup {
    #[allow(dead_code)] // documentation + debugging aid
    name: &'static str,
    /// File-offset ranges (start, end) belonging to this group. A curated
    /// quartet or rotate-only quartet belongs to the group whose range
    /// contains its offset.
    ranges: &'static [(usize, usize)],
    /// Allowed hue shifts (0-11), rolled uniformly. All small (0, ±1, ±2 =
    /// at most ~60° around the wheel) so no context ever leaves its
    /// plausible color family. 11 = -1, 10 = -2 (mod 12).
    shifts: &'static [u8],
}

/// Context-aware theme groups. Regions that light up the same screens share
/// a group (plains BG in slot 3 and its slice-4 variants must shift
/// together, or one screen would split into two themes).
///
/// Shift sets are chosen from what each context's dominant hues tolerate on
/// the NES wheel (1→C: blue→violet→magenta→red→orange→yellow→green→cyan):
/// - plains/giant/water tolerate ±1 and -2 (spring / dusk / autumn / swamp
///   readings) but NOT +2 (magenta sky territory);
/// - warm contexts (desert, lava) and identity-ish contexts (maps, sprite
///   skin tones, the partially unmapped pool) stay within ±1.
const THEME_GROUPS: &[ThemeGroup] = &[
    ThemeGroup {
        name: "maps",
        ranges: &[(0x36BE4, 0x36C54)], // slots 0-1 (W6 + W7 overworld maps)
        shifts: &[0, 1, 11],
    },
    ThemeGroup {
        name: "sprites/text",
        ranges: &[(0x36C54, 0x36C8C)], // slot 2 (hammer bro sprites, HELP text)
        shifts: &[0, 1, 11],
    },
    ThemeGroup {
        name: "plains",
        // slot 3 (plains BG+HUD), slot 5 (plains enemies / W7-5 BG),
        // slice 4 head/tail/post (sky-land + plains variants)
        ranges: &[(0x36C8C, 0x36CC4), (0x36CFC, 0x36D34), (0x37600, 0x377E0), (0x37808, 0x37850)],
        shifts: &[0, 1, 11, 10],
    },
    ThemeGroup {
        name: "giant",
        ranges: &[(0x36CC4, 0x36CFC), (0x37400, 0x37600)], // slot 4 + slice 3
        shifts: &[0, 1, 11, 10],
    },
    ThemeGroup {
        name: "fortress",
        ranges: &[(0x36D34, 0x36DA6)], // slots 6-7 (fortress HUD + BG)
        shifts: &[0, 1, 11],
    },
    ThemeGroup {
        name: "lava/bowser",
        ranges: &[(0x36DA8, 0x36E20)], // slot tail (lava, rotodisc, bowser, donut)
        shifts: &[0, 1, 11],
    },
    ThemeGroup {
        name: "pool",
        ranges: &[(0x36E20, 0x37000)], // mixed pool (water sprites at 0x36F00)
        shifts: &[0, 1, 11],
    },
    ThemeGroup {
        name: "water",
        ranges: &[(0x37000, 0x37200)], // slice 1
        shifts: &[0, 1, 11, 10],
    },
    ThemeGroup {
        name: "desert/airship",
        ranges: &[(0x37200, 0x37400)], // slice 2 (desert + fortress + airship)
        shifts: &[0, 1, 11],
    },
];

/// Look up the theme-group index owning a file offset. Every curated offset
/// must belong to a group (enforced by test); unknown offsets get None and
/// are left unrotated.
fn theme_group_for(offset: usize) -> Option<usize> {
    THEME_GROUPS.iter().position(|g| {
        g.ranges.iter().any(|&(start, end)| (start..end).contains(&offset))
    })
}

/// Themed palette randomization across all tilesets.
///
/// Two layers, both aesthetically safe by construction:
///
/// 1. **Variant swap**: for each curated quartet position, pick ONE whole
///    4-byte variant from a list of pre-validated options (vanilla +
///    Recolored + hand-curated). Every emitted palette group was designed as
///    a coherent unit — no flat color-pool mixing, no independent byte picks.
///
/// 2. **Context-aware hue rotation**: each theme group (plains, water,
///    fortress, ...) rolls its own small hue shift from the group's allowed
///    set and applies it to every chromatic byte the group owns. The NES
///    color byte is `(luminance << 4) | hue`, so rotating the hue nibble
///    while preserving the luminance nibble keeps every brightness/contrast
///    relationship of the source palette intact — visibility is preserved by
///    construction. Shifts are capped at 2 steps (~60°) and constrained per
///    context, so water stays watery, lava stays warm, and skies never go
///    magenta — subtle seasonal variation instead of a whole-wheel spin.
///    Grays, blacks, whites (hue nibble 0/D/E/F) and non-color bytes
///    (> 0x3C, 0xFF skip markers) pass through untouched.
///
/// Coverage: every quartet Recolored changed across the themed-slot table
/// (slots 0-7 + tail), the palette pool at 0x36E20, and master-pool slices
/// 1-4 (skipping the level-layout pointer table at 0x377E0-0x37807).
/// Quartets Recolored kept at vanilla but which hold chromatic bytes are in
/// `ROTATE_ONLY_QUARTETS`: they never variant-swap, but they DO hue-rotate,
/// so a kept-vanilla green can't clash with rotated colors on the same screen.
pub fn randomize_themed<R: Rng>(rom: &mut Rom, rng: &mut R) {
    // Character palettes stay randomized too — independent of tileset palettes.
    randomize(rom, rng);

    // Roll one shift per theme group, in declaration order (deterministic
    // for a given RNG stream).
    let group_shifts: Vec<u8> = THEME_GROUPS
        .iter()
        .map(|g| *g.shifts.choose(rng).unwrap())
        .collect();
    let shift_for = |offset: usize| -> u8 {
        theme_group_for(offset).map_or(0, |gi| group_shifts[gi])
    };

    for region in THEMED_REGIONS {
        apply_variant_groups(rom, region, &shift_for, rng);
    }

    // Hue-rotate the kept-vanilla chromatic quartets in place.
    for &offset in ROTATE_ONLY_QUARTETS {
        let shift = shift_for(offset);
        for i in 0..4 {
            let b = rom.read_byte(offset + i);
            rom.write_byte(offset + i, rotate_hue(b, shift));
        }
    }
}

/// Rotate a NES color's hue around the 12-hue wheel, preserving luminance.
///
/// NES color byte layout: high nibble = luminance row (0-3), low nibble =
/// hue column (1-C; 0 = gray/white, D-F = blacks/forbidden). Only chromatic
/// bytes (row 0-3, hue 1-C) rotate; everything else — grays, blacks, the
/// 0xFF skip marker, and any non-color byte — passes through unchanged.
/// Output hue stays in 1-C, so rotation can never produce the problematic
/// 0x0D/0x0E/0x0F column or leave the base 64-color palette.
fn rotate_hue(byte: u8, shift: u8) -> u8 {
    let hue = byte & 0x0F;
    if byte > 0x3C || hue == 0 || hue > 0x0C {
        return byte;
    }
    let rotated = ((hue - 1 + shift) % 12) + 1;
    (byte & 0xF0) | rotated
}

/// For each curated position, pick one 4-byte variant at random, hue-rotate
/// its chromatic bytes by its theme group's shift, and write it.
fn apply_variant_groups<R: Rng>(
    rom: &mut Rom,
    groups: &[VariantGroup],
    shift_for: &dyn Fn(usize) -> u8,
    rng: &mut R,
) {
    for group in groups {
        let picked = group.variants.choose(rng).unwrap();
        let shift = shift_for(group.offset);
        let rotated = picked.map(|b| rotate_hue(b, shift));
        rom.write_range(group.offset, &rotated);
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
        Rom::from_bytes_lax(&data, true).unwrap()
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
        let mut v: Vec<&'static VariantGroup> = Vec::new();
        for slice in THEMED_REGIONS {
            v.extend(slice.iter());
        }
        v
    }

    #[test]
    fn rotate_hue_basics() {
        // Shift 0 and full-circle shift are identity for every byte value.
        for b in 0u8..=0xFF {
            assert_eq!(rotate_hue(b, 0), b, "shift 0 must be identity for {b:#04x}");
        }
        // Chromatic bytes: luminance nibble preserved, hue stays in 1-C,
        // 12-step cycle returns to start.
        for row in 0u8..4 {
            for hue in 1u8..=0x0C {
                let b = (row << 4) | hue;
                for shift in 0u8..12 {
                    let r = rotate_hue(b, shift);
                    assert_eq!(r & 0xF0, b & 0xF0, "luminance changed for {b:#04x}");
                    let rh = r & 0x0F;
                    assert!((1..=0x0C).contains(&rh), "hue {rh:#03x} out of range");
                }
                // applying 1-step rotation 12 times cycles back
                let mut cur = b;
                for _ in 0..12 {
                    cur = rotate_hue(cur, 1);
                }
                assert_eq!(cur, b, "12-cycle must return to {b:#04x}");
            }
        }
        // Non-chromatic bytes pass through at every shift: grays/whites
        // (hue 0), blacks/forbidden (hue D-F), and anything above 0x3C.
        for &b in &[0x00u8, 0x10, 0x20, 0x30, 0x0D, 0x0E, 0x0F, 0x1D, 0x2F, 0x3D, 0x3F, 0x40, 0x99, 0xAD, 0xFF] {
            for shift in 0u8..12 {
                assert_eq!(rotate_hue(b, shift), b, "{b:#04x} must pass through");
            }
        }
    }

    #[test]
    fn themed_emits_rotated_curated_variants_only() {
        // Every 4-byte write at a curated position must match one of the
        // pre-registered variants rotated by its theme group's shift — ONE
        // shift per group (coherent theme within each context, no per-quartet
        // rainbow), drawn from the group's allowed set, and no free-byte picks.
        for seed in [1u64, 42, 99, 777, 12345] {
            let mut rom = make_test_rom();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize_themed(&mut rom, &mut rng);

            for (gi, tg) in THEME_GROUPS.iter().enumerate() {
                let group_quartets: Vec<&'static VariantGroup> = all_variant_groups()
                    .into_iter()
                    .filter(|g| theme_group_for(g.offset) == Some(gi))
                    .collect();
                let shift_matches = |shift: u8| -> bool {
                    group_quartets.iter().all(|group| {
                        let written = rom.read_range(group.offset, 4);
                        group.variants.iter().any(|v| {
                            v.iter().zip(written).all(|(&vb, &wb)| rotate_hue(vb, shift) == wb)
                        })
                    })
                };
                assert!(
                    tg.shifts.iter().any(|&s| shift_matches(s)),
                    "seed {seed}, group '{}': no allowed shift explains all written quartets",
                    tg.name,
                );
            }
        }
    }

    #[test]
    fn every_curated_offset_belongs_to_one_theme_group() {
        // Every variant-group and rotate-only offset must fall inside exactly
        // one theme group's ranges — an unowned offset would silently skip
        // rotation and could clash with its rotated neighbors.
        for group in all_variant_groups() {
            let owners = THEME_GROUPS
                .iter()
                .filter(|tg| tg.ranges.iter().any(|&(s, e)| (s..e).contains(&group.offset)))
                .count();
            assert_eq!(
                owners, 1,
                "variant group at {:#08x} owned by {owners} theme groups (want 1)",
                group.offset
            );
        }
        for &offset in ROTATE_ONLY_QUARTETS {
            let owners = THEME_GROUPS
                .iter()
                .filter(|tg| tg.ranges.iter().any(|&(s, e)| (s..e).contains(&offset)))
                .count();
            assert_eq!(
                owners, 1,
                "rotate-only quartet at {offset:#08x} owned by {owners} theme groups (want 1)"
            );
        }
    }

    #[test]
    fn theme_shifts_are_subtle() {
        // Garishness guard: every allowed shift must be within 2 steps of
        // vanilla on the 12-hue wheel (0, ±1, ±2 — i.e. {0, 1, 2, 10, 11}).
        // A shift of 3+ steps sends skies magenta / grass purple.
        for tg in THEME_GROUPS {
            assert!(!tg.shifts.is_empty(), "group '{}' has no shifts", tg.name);
            assert!(
                tg.shifts.contains(&0),
                "group '{}' must always allow vanilla hues",
                tg.name
            );
            for &s in tg.shifts {
                assert!(
                    matches!(s, 0 | 1 | 2 | 10 | 11),
                    "group '{}' allows non-subtle shift {s}",
                    tg.name
                );
            }
        }
    }

    #[test]
    fn themed_does_not_touch_uncurated_positions() {
        // For every region covered, offsets not in any VariantGroup and not in
        // a rotate-only quartet must stay untouched. We stamp recognizable
        // canary bytes (all >= 0x40, so hue rotation passes them through) in
        // each covered range and check them after running the randomizer.
        const REGIONS: &[(usize, usize, u8)] = &[
            (0x36BE4, 0x36C1C, 0x40),  // slot 0 (W6 map)
            (0x36C1C, 0x36C54, 0x50),  // slot 1 (W7 map)
            (0x36C54, 0x36C8C, 0xA0),  // slot 2
            (0x36C8C, 0x36CC4, 0xC0),  // slot 3
            (0x36CC4, 0x36CFC, 0xD0),  // slot 4
            (0x36CFC, 0x36D34, 0xE0),  // slot 5
            (0x36D34, 0x36D6C, 0x90),  // slot 6
            (0x36D6C, 0x36DA6, 0x80),  // slot 7
            (0x36DA8, 0x36E20, 0x40),  // slot tail
            (0x36E20, 0x37000, 0x50),  // pool
            (0x37000, 0x37200, 0x70),  // slice 1
            (0x37200, 0x37400, 0x60),  // slice 2
            (0x37400, 0x37600, 0x50),  // slice 3
            (0x37600, 0x377E0, 0xB0),  // slice 4 head
            (0x37808, 0x37846, 0xC0),  // slice 4 tail
            (0x37844, 0x37850, 0x40),  // slice 4 post
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
                // Rotate-only quartets are read-rotate-written, but the canary
                // bytes are all >= 0x40, which rotate_hue passes through — so
                // even those positions must still hold their canary.
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
    fn rotate_only_quartets_are_disjoint_and_safe() {
        // Rotate-only quartets must not overlap any variant group (they'd
        // double-write) and must not touch the pointer-table crash trap.
        let curated_offsets: std::collections::HashSet<usize> = all_variant_groups()
            .iter()
            .flat_map(|g| (0..4).map(move |k| g.offset + k))
            .collect();

        for &offset in ROTATE_ONLY_QUARTETS {
            for k in 0..4 {
                assert!(
                    !curated_offsets.contains(&(offset + k)),
                    "rotate-only quartet {offset:#08x} overlaps a variant group"
                );
            }
            let overlaps_ptr = offset + 4 > 0x377E0 && offset < 0x37808;
            assert!(
                !overlaps_ptr,
                "rotate-only quartet {offset:#08x} overlaps pointer table"
            );
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
