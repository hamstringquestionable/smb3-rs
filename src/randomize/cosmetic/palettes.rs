use rand::Rng;
use rand::seq::IndexedRandom;

use crate::randomize::cosmetic::palette_variants::{
    AIRSHIP_VARIANTS, BONUS_PLAYER_VARIANTS, BONUS_VARIANTS, DESERT_VARIANTS, FORT_VARIANTS,
    GIANT_VARIANTS, HIGH_UP_VARIANTS, HILLS_UNDER_VARIANTS, ICE_VARIANTS, MAP_SUIT_VARIANTS,
    MAPS_VARIANTS, PIPE_MAZE_VARIANTS, PLAINS_VARIANTS, PLANT_VARIANTS, ROTATE_ONLY_QUARTETS,
    SKY_VARIANTS, TOAD_VARIANTS, TWO_P_VS_VARIANTS, VariantGroup, WATER_VARIANTS,
};
use crate::rom::Rom;

/// Character sprite palette entries: [bg_mirror(0x00), body, face, outline/accent].
/// Byte 0 must stay 0x00 — it mirrors $3F00 (universal background color) via the PPU.
/// Byte 2 is the face in every suit (see `FACE_BYTE`).
const PALETTE_RANGES: &[(usize, &str)] = &[
    (0x10539, "Small/Big/Raccoon Mario"),
    (0x1053D, "Small/Big/Raccoon Luigi"),
    (0x10541, "Fire Mario/Luigi"),
    (0x10549, "Frog Mario/Luigi"),
    (0x1054D, "Tanooki Mario/Luigi"),
    (0x10551, "Hammer Mario/Luigi"),
];

/// Randomize character sprite palettes (Mario/Luigi power-up colors).
///
/// When `player_color` is Some, that color anchors the wardrobe scheme;
/// otherwise a random chromatic anchor is rolled — same as clicking a random
/// swatch on the web grid. Either way the output goes through
/// `apply_player_scheme`, so every roll is a coherent wardrobe (natural face,
/// Luigi contrast, tinted Hammer suit) rather than independent byte picks.
pub(crate) fn randomize<R: Rng>(rom: &mut Rom, rng: &mut R, player_color: Option<u8>) {
    let anchor = player_color.filter(|&c| is_chromatic(c)).unwrap_or_else(|| {
        let row: u8 = rng.random_range(..4);
        let hue: u8 = rng.random_range(1..=0x0C);
        (row << 4) | hue
    });
    apply_player_scheme(rom, anchor);
}

/// Index of the face color within every character palette quartet. Pinned
/// during scheme generation so a recolored Mario keeps a natural face. Pinned
/// by POSITION, not by value: vanilla uses 0x36 for every suit's face except
/// the Hammer suit's darker 0x27, which a value pin missed — so the Hammer
/// suit's face rotated with the scheme while the suit itself didn't change.
const FACE_BYTE: usize = 2;

/// Vanilla Small/Big Mario body hue (red, column 6) — the rotation origin
/// for the player-color scheme.
const VANILLA_MARIO_HUE: u8 = 0x06;

/// Whether a byte is a rotatable NES color (luminance row 0-3, hue 1-C).
fn is_chromatic(b: u8) -> bool {
    b <= 0x3C && (1..=0x0C).contains(&(b & 0x0F))
}

/// Derive the whole character wardrobe from one player-picked color.
///
/// The vanilla palettes already encode the relationships that make the cast
/// read correctly: Luigi's green sits +4 hues from Mario's red, the Fire suit
/// keeps a red accent byte, and every
/// suit shares the skin-tone highlight. So the scheme is a single hue
/// rotation of the vanilla wardrobe by (picked hue - vanilla red): every
/// relative relationship survives, anchored on the pick. Deterministic — the
/// same pick always produces the same wardrobe.
///
/// The Hammer suit is the exception: it is white and black, which have no hue
/// to rotate, so it is tinted from the pick instead (`HAMMER_SUIT`).
///
/// Two pins keep it looking intentional:
/// - the `FACE_BYTE` never rotates (blue Mario has blue clothes, not a blue
///   face);
/// - Mario's body byte is set to the picked color EXACTLY (row included), so
///   what the player clicked is what Mario wears.
///
/// Picking Mario's current color reproduces the current wardrobe
/// byte-for-byte (vanilla red 0x16 on an unpatched ROM), except the Hammer
/// suit, which always takes the tint.
fn apply_player_scheme(rom: &mut Rom, anchor: u8) {
    debug_assert!(is_chromatic(anchor), "anchor {anchor:#04x} must be chromatic");
    // Rotation origin = the CURRENT Mario body hue, not a hard-coded vanilla
    // red: visual reskin patches (Luigi-35th, Peach, ...) apply before
    // randomization and restyle the wardrobe, and rotating from the actual
    // body hue keeps the reskin's body-to-suit relationships intact (picking
    // the character's current color is always a no-op). Falls back to
    // vanilla red if a patch made the body achromatic (Dr. Mario's white
    // coat), so the accents still rotate sensibly.
    let current_body = rom.read_byte(PALETTE_RANGES[0].0 + 1);
    let origin_hue =
        if is_chromatic(current_body) { current_body & 0x0F } else { VANILLA_MARIO_HUE };
    let delta = ((anchor & 0x0F) + 12 - origin_hue) % 12;

    for &(offset, _name) in PALETTE_RANGES {
        // Bytes 1 and 3: body, and the outline/accent byte (usually 0x0F
        // black, which passes through — but the Fire suit carries a red
        // accent there that should follow the scheme). Byte 0 stays 0x00 and
        // the face never moves.
        for i in (1..4).filter(|&i| i != FACE_BYTE) {
            let b = rom.read_byte(offset + i);
            rom.write_byte(offset + i, rotate_hue(b, delta));
        }
    }

    // Mario wears exactly what the player clicked.
    let (mario_offset, _) = PALETTE_RANGES[0];
    rom.write_byte(mario_offset + 1, anchor);

    // Hammer suit: white -> the pick's lightest row, black (which is also
    // the outline) -> its darkest row. The face stays put.
    let hue = anchor & 0x0F;
    rom.write_byte(HAMMER_SUIT + 1, 0x30 | hue);
    rom.write_byte(HAMMER_SUIT + 3, hue);
}

/// Hammer suit quartet, `[00, white 0x30, face, black 0x0F]` in vanilla.
/// Has no chromatic suit byte, so `apply_player_scheme` tints it from the
/// pick rather than rotating it.
const HAMMER_SUIT: usize = PALETTE_RANGES[5].0;

/// All variant-group regions applied by `randomize_themed`, in write order.
const THEMED_REGIONS: &[&[VariantGroup]] = &[
    MAPS_VARIANTS,
    PLAINS_VARIANTS,
    FORT_VARIANTS,
    HILLS_UNDER_VARIANTS,
    HIGH_UP_VARIANTS,
    PLANT_VARIANTS,
    WATER_VARIANTS,
    TOAD_VARIANTS,
    PIPE_MAZE_VARIANTS,
    DESERT_VARIANTS,
    AIRSHIP_VARIANTS,
    GIANT_VARIANTS,
    ICE_VARIANTS,
    SKY_VARIANTS,
    TWO_P_VS_VARIANTS,
    BONUS_VARIANTS,
    BONUS_PLAYER_VARIANTS,
    MAP_SUIT_VARIANTS,
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

/// Context-aware theme groups: one per `PalSet_*` set of PRG027 (see
/// `docs/smb3_rom_reference.md` → "Palette Sets"). A screen loads its BG and
/// sprite colors from exactly one set, so one shift per set means no screen
/// can split into two themes. Each set is 192 bytes from 0x36BE2.
///
/// Shift sets are chosen from what each context's dominant hues tolerate on
/// the NES wheel (1→C: blue→violet→magenta→red→orange→yellow→green→cyan):
/// - plains/giant/water tolerate ±1 and -2 (spring / dusk / autumn / swamp
///   readings) but NOT +2 (magenta sky territory);
/// - every other set stays within ±1.
const THEME_GROUPS: &[ThemeGroup] = &[
    ThemeGroup {
        name: "maps",
        // PalSet_Maps + the map player palette per suit (InitPals_Per_MapPUp)
        ranges: &[(0x36BE2, 0x36CA2), (0x3782B, 0x3784F)],
        shifts: &[0, 1, 11],
    },
    ThemeGroup { name: "plains", ranges: &[(0x36CA2, 0x36D62)], shifts: &[0, 1, 11, 10] },
    ThemeGroup { name: "fortress", ranges: &[(0x36D62, 0x36E22)], shifts: &[0, 1, 11] },
    ThemeGroup { name: "hills/underground", ranges: &[(0x36E22, 0x36EE2)], shifts: &[0, 1, 11] },
    ThemeGroup { name: "high-up", ranges: &[(0x36EE2, 0x36FA2)], shifts: &[0, 1, 11] },
    ThemeGroup { name: "plant", ranges: &[(0x36FA2, 0x37062)], shifts: &[0, 1, 11] },
    ThemeGroup { name: "water", ranges: &[(0x37062, 0x37122)], shifts: &[0, 1, 11, 10] },
    ThemeGroup { name: "toad house", ranges: &[(0x37122, 0x371E2)], shifts: &[0, 1, 11] },
    ThemeGroup { name: "pipe maze", ranges: &[(0x371E2, 0x372A2)], shifts: &[0, 1, 11] },
    ThemeGroup { name: "desert", ranges: &[(0x372A2, 0x37362)], shifts: &[0, 1, 11] },
    ThemeGroup { name: "airship", ranges: &[(0x37362, 0x37422)], shifts: &[0, 1, 11] },
    ThemeGroup { name: "giant", ranges: &[(0x37422, 0x374E2)], shifts: &[0, 1, 11, 10] },
    ThemeGroup { name: "ice", ranges: &[(0x374E2, 0x375A2)], shifts: &[0, 1, 11] },
    ThemeGroup { name: "sky", ranges: &[(0x375A2, 0x37662)], shifts: &[0, 1, 11] },
    ThemeGroup { name: "2p vs", ranges: &[(0x37662, 0x37722)], shifts: &[0, 1, 11] },
    ThemeGroup {
        name: "bonus",
        // PalSet_Bonus + the bonus games' player palettes (BonusGame_PlayerPal)
        ranges: &[(0x37722, 0x377E2), (0x37808, 0x37820)],
        shifts: &[0, 1, 11],
    },
];

/// Look up the theme-group index owning a file offset. Every curated offset
/// must belong to a group (enforced by test); unknown offsets get None and
/// are left unrotated.
fn theme_group_for(offset: usize) -> Option<usize> {
    THEME_GROUPS
        .iter()
        .position(|g| g.ranges.iter().any(|&(start, end)| (start..end).contains(&offset)))
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
/// Coverage: every sub-palette Recolored changed across the 16 `PalSet_*`
/// sets plus the bonus-game and map-suit player palettes, never touching
/// `Palette_By_Tileset` (0x377E2-0x37807) or the suit-index table at
/// 0x37822-0x3782A. Sub-palettes Recolored kept at vanilla but which hold
/// chromatic bytes are in `ROTATE_ONLY_QUARTETS`: they never variant-swap,
/// but they DO hue-rotate, so a kept-vanilla green can't clash with rotated
/// colors on the same screen.
pub(crate) fn randomize_themed<R: Rng>(rom: &mut Rom, rng: &mut R) {
    // World palettes only — the character wardrobe is `randomize()`'s job,
    // driven independently by the player-colors option.

    // Roll one shift per theme group, in declaration order (deterministic
    // for a given RNG stream).
    let group_shifts: Vec<u8> =
        THEME_GROUPS.iter().map(|g| *g.shifts.choose(rng).unwrap()).collect();
    let shift_for =
        |offset: usize| -> u8 { theme_group_for(offset).map_or(0, |gi| group_shifts[gi]) };

    for region in THEMED_REGIONS {
        apply_variant_groups(rom, region, shift_for, rng);
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
    shift_for: impl Fn(usize) -> u8,
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
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    fn make_test_rom() -> Rom {
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;
        Rom::from_bytes_lax(&data, true).unwrap()
    }

    /// Vanilla character palette quartets, as in the real ROM.
    const VANILLA_WARDROBE: &[(usize, [u8; 4])] = &[
        (0x10539, [0x00, 0x16, 0x36, 0x0F]), // Small/Big Mario
        (0x1053D, [0x00, 0x2A, 0x36, 0x0F]), // Small/Big Luigi
        (0x10541, [0x00, 0x27, 0x36, 0x16]), // Fire M/L
        (0x10549, [0x00, 0x2A, 0x36, 0x0F]), // Frog M/L
        (0x1054D, [0x00, 0x17, 0x36, 0x0F]), // Tanooki M/L
        (0x10551, [0x00, 0x30, 0x27, 0x0F]), // Hammer M/L
    ];

    fn make_wardrobe_rom() -> Rom {
        let mut rom = make_test_rom();
        for &(offset, quartet) in VANILLA_WARDROBE {
            rom.write_range(offset, &quartet);
        }
        rom
    }

    #[test]
    fn random_roll_is_a_coherent_scheme() {
        // With no player color, randomize() must still emit a wardrobe that
        // matches apply_player_scheme for SOME chromatic anchor — never the
        // old independent per-byte picks.
        for seed in [1u64, 42, 99, 777] {
            let mut rom = make_wardrobe_rom();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom, &mut rng, None);

            let mario_body = rom.read_byte(0x10539 + 1);
            assert!(
                is_chromatic(mario_body),
                "seed {seed}: anchor {mario_body:#04x} not chromatic"
            );

            // Rebuild the scheme from the observed anchor; it must match.
            let mut expected = make_wardrobe_rom();
            apply_player_scheme(&mut expected, mario_body);
            for &(offset, _) in VANILLA_WARDROBE {
                assert_eq!(
                    rom.read_range(offset, 4),
                    expected.read_range(offset, 4),
                    "seed {seed}: random roll diverged from scheme at {offset:#06x}"
                );
            }
        }
    }

    #[test]
    fn test_palettes_deterministic() {
        let mut rom1 = make_wardrobe_rom();
        let mut rom2 = make_wardrobe_rom();
        let mut rng1 = ChaCha8Rng::seed_from_u64(99);
        let mut rng2 = ChaCha8Rng::seed_from_u64(99);

        randomize(&mut rom1, &mut rng1, None);
        randomize(&mut rom2, &mut rng2, None);

        for &(offset, _) in PALETTE_RANGES {
            assert_eq!(rom1.read_range(offset, 4), rom2.read_range(offset, 4),);
        }
    }

    #[test]
    fn player_scheme_vanilla_anchor_is_identity() {
        // Picking Mario's vanilla red must reproduce the vanilla wardrobe
        // byte-for-byte — the free "classic" option.
        // The Hammer suit is the exception: it always takes the tint.
        let mut rom = make_wardrobe_rom();
        apply_player_scheme(&mut rom, 0x16);
        for &(offset, quartet) in VANILLA_WARDROBE.iter().filter(|&&(o, _)| o != HAMMER_SUIT) {
            assert_eq!(
                rom.read_range(offset, 4),
                &quartet,
                "vanilla anchor changed quartet at {offset:#06x}"
            );
        }
    }

    #[test]
    fn player_scheme_composes_with_visual_reskins() {
        // Visual reskin patches apply BEFORE randomization and restyle the
        // wardrobe (Luigi-35th sets Mario's body green 0x1A). The rotation
        // origin is the current body hue, so:
        // 1. picking the reskin's own color is a no-op;
        // 2. picking another color rotates the RESKINNED wardrobe uniformly.
        let reskin: &[(usize, [u8; 4])] = &[
            (0x10539, [0x00, 0x1A, 0x36, 0x0F]), // body green (Luigi-35th style)
            (0x1053D, [0x00, 0x16, 0x36, 0x0F]),
            (0x10541, [0x00, 0x27, 0x36, 0x1A]),
            (0x10549, [0x00, 0x2A, 0x36, 0x0F]),
            (0x1054D, [0x00, 0x17, 0x36, 0x0F]),
            (0x10551, [0x00, 0x30, 0x27, 0x0F]),
        ];
        let make_reskin_rom = || {
            let mut rom = make_test_rom();
            for &(offset, quartet) in reskin {
                rom.write_range(offset, &quartet);
            }
            rom
        };

        // Identity: picking the reskin's current body color changes nothing
        // (but the Hammer suit, which always takes the tint).
        let mut rom = make_reskin_rom();
        apply_player_scheme(&mut rom, 0x1A);
        for &(offset, quartet) in reskin.iter().filter(|&&(o, _)| o != HAMMER_SUIT) {
            assert_eq!(
                rom.read_range(offset, 4),
                &quartet,
                "current-color anchor must be a no-op at {offset:#06x}"
            );
        }

        // Uniform rotation from the reskin's hue: picking red (hue 6) from
        // green (hue A) is delta -4 for every chromatic non-skin byte.
        let mut rom = make_reskin_rom();
        apply_player_scheme(&mut rom, 0x16);
        let delta = 8; // picked hue 6 minus reskin body hue A, mod 12 (≡ -4)
        assert_eq!(rom.read_byte(0x10539 + 1), 0x16, "body must be the exact pick");
        assert_eq!(rom.read_byte(0x1053D + 1), rotate_hue(0x16, delta));
        assert_eq!(rom.read_byte(0x10541 + 3), rotate_hue(0x1A, delta), "accent follows");
        assert_eq!(rom.read_range(0x10551, 4), &[0x00, 0x36, 0x27, 0x06], "hammer tinted red");
    }

    #[test]
    fn player_scheme_structure() {
        // Anchor on blue (0x12): Mario wears exactly the pick, skin stays
        // skin, the Hammer suit is tinted blue, and everything chromatic
        // rotates by the same delta (blue is 4 hues counterclockwise of red).
        let mut rom = make_wardrobe_rom();
        apply_player_scheme(&mut rom, 0x12);

        // Mario: exact pick + pinned skin + black outline.
        assert_eq!(rom.read_range(0x10539, 4), &[0x00, 0x12, 0x36, 0x0F]);
        // Luigi: green 0x2A rotates by the same -4 delta to cyan-blue land,
        // preserving the vanilla brother contrast.
        let delta = 8; // picked hue 2 minus vanilla red 6, mod 12 (≡ -4)
        assert_eq!(rom.read_byte(0x1053D + 1), rotate_hue(0x2A, delta));
        // Fire suit: red accent byte follows the scheme (0x16 -> blue 0x12).
        assert_eq!(rom.read_byte(0x10541 + 3), 0x12);
        // Hammer suit: white -> light blue 0x32, black -> dark blue 0x02,
        // face untouched.
        assert_eq!(rom.read_range(0x10551, 4), &[0x00, 0x32, 0x27, 0x02]);
        // Face pinned in every suit — including the Hammer suit's 0x27,
        // which is not the usual 0x36 skin tone.
        for &(offset, quartet) in VANILLA_WARDROBE {
            assert_eq!(
                rom.read_byte(offset + FACE_BYTE),
                quartet[FACE_BYTE],
                "face rotated at {offset:#06x}"
            );
        }
    }

    #[test]
    fn player_scheme_every_chromatic_anchor_emits_valid_colors() {
        // For all 48 chromatic anchors: every emitted byte stays a valid NES
        // color (or structural 0x00/0x0F), and Mario's body is the anchor.
        for row in 0u8..4 {
            for hue in 1u8..=0x0C {
                let anchor = (row << 4) | hue;
                let mut rom = make_wardrobe_rom();
                apply_player_scheme(&mut rom, anchor);
                assert_eq!(rom.read_byte(0x10539 + 1), anchor, "Mario body != anchor");
                for &(offset, _) in VANILLA_WARDROBE {
                    for i in 0..4 {
                        let b = rom.read_byte(offset + i);
                        assert!(
                            b <= 0x3C && (b & 0x0F) != 0x0D || b == 0x0F || b == 0x00,
                            "anchor {anchor:#04x}: invalid byte {b:#04x} at {offset:#06x}+{i}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn randomize_with_player_color_uses_scheme() {
        // randomize() with a player color must produce the deterministic
        // scheme regardless of RNG state.
        let mut rom1 = make_wardrobe_rom();
        let mut rom2 = make_wardrobe_rom();
        let mut rng1 = ChaCha8Rng::seed_from_u64(1);
        let mut rng2 = ChaCha8Rng::seed_from_u64(999);
        randomize(&mut rom1, &mut rng1, Some(0x2A));
        randomize(&mut rom2, &mut rng2, Some(0x2A));
        for &(offset, _) in VANILLA_WARDROBE {
            assert_eq!(
                rom1.read_range(offset, 4),
                rom2.read_range(offset, 4),
                "player-color scheme must be RNG-independent"
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
                    assert!((1..=0x0C).contains(&rh), "hue {rh:#04x} out of range");
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
        for &b in &[
            0x00u8, 0x10, 0x20, 0x30, 0x0D, 0x0E, 0x0F, 0x1D, 0x2F, 0x3D, 0x3F, 0x40, 0x99, 0xAD,
            0xFF,
        ] {
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
            assert!(tg.shifts.contains(&0), "group '{}' must always allow vanilla hues", tg.name);
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
            (0x36BE2, 0x377E2, 0x40), // the 16 PalSet_* sets
            (0x37808, 0x37820, 0x50), // BonusGame_PlayerPal
            (0x3782B, 0x3784F, 0x60), // InitPals_Per_MapPUp
        ];

        let mut rom = make_test_rom();
        for &(start, end, base) in REGIONS {
            let canary: Vec<u8> = (0..(end - start)).map(|i| base | (i as u8 & 0x0F)).collect();
            rom.write_range(start, &canary);
        }

        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize_themed(&mut rom, &mut rng);

        let curated_offsets: std::collections::HashSet<usize> =
            all_variant_groups().iter().flat_map(|g| (0..4).map(move |k| g.offset + k)).collect();

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
        let curated_offsets: std::collections::HashSet<usize> =
            all_variant_groups().iter().flat_map(|g| (0..4).map(move |k| g.offset + k)).collect();

        for &offset in ROTATE_ONLY_QUARTETS {
            for k in 0..4 {
                assert!(
                    !curated_offsets.contains(&(offset + k)),
                    "rotate-only quartet {offset:#08x} overlaps a variant group"
                );
            }
            let overlaps_ptr = offset + 4 > 0x377E2 && offset < 0x37808;
            assert!(!overlaps_ptr, "rotate-only quartet {offset:#08x} overlaps pointer table");
        }
    }

    #[test]
    fn themed_does_not_touch_pointer_table() {
        // `Palette_By_Tileset` (0x377E2-0x37807, 19 words) is how
        // `Setup_PalData` finds every palette set; painting it sends the
        // loader to a garbage address and crashes the game.
        let mut rom = make_test_rom();
        let vanilla: Vec<u8> = (0..0x26).map(|i| 0xAB + (i as u8 & 0x0F)).collect();
        rom.write_range(0x377E2, &vanilla);

        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize_themed(&mut rom, &mut rng);

        assert_eq!(
            rom.read_range(0x377E2, 0x26),
            &vanilla[..],
            "pointer table 0x377E2-0x37807 must not be modified"
        );
    }

    #[test]
    fn themed_does_not_touch_map_suit_indices() {
        // 0x37820-0x3782A is `Map_PlayerPalFix` (Mario/Luigi map color) then
        // `InitPal_Per_MapPowerup`, which holds palette INDICES 00-08. Those
        // look chromatic to `rotate_hue`, so a rotated index would hand a suit
        // another suit's palette, or (P-Wing 08 -> 09) read past the table into
        // `Setup_PalData`'s code. Use the real bytes, not >= 0x40 canaries.
        const VANILLA: [u8; 11] =
            [0x16, 0x1A, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        for seed in 0u64..32 {
            let mut rom = make_test_rom();
            rom.write_range(0x37820, &VANILLA);
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize_themed(&mut rom, &mut rng);
            assert_eq!(rom.read_range(0x37820, 11), &VANILLA[..], "seed {seed}");
        }
    }

    #[test]
    fn no_variant_group_overlaps_pointer_table() {
        // Static sanity: no curated offset can fall inside the pointer-table
        // crash trap, even transitively (offset + 3 still < 0x377E2, or
        // offset >= 0x37808).
        for group in all_variant_groups() {
            let start = group.offset;
            let end = group.offset + 4;
            let overlaps = end > 0x377E2 && start < 0x37808;
            assert!(
                !overlaps,
                "VariantGroup at {:#08x} overlaps pointer table 0x377E2-0x37807",
                group.offset
            );
        }
    }

    #[test]
    fn curated_offsets_sit_on_the_sub_palette_grid() {
        // Every PalSet sub-palette starts at 0x36BE2 + 4n; the player palette
        // tables after the pointer table have their own grids (0x37808,
        // 0x3782B). A group off its grid straddles two sub-palettes, so
        // independent picks could mix vanilla and Recolored halves of one.
        let on_grid = |o: usize| match o {
            0x36BE2..0x377E2 => (o - 0x36BE2).is_multiple_of(4),
            0x37808..0x37820 => (o - 0x37808).is_multiple_of(4),
            0x3782B..0x3784F => (o - 0x3782B).is_multiple_of(4),
            _ => false,
        };
        for group in all_variant_groups() {
            assert!(on_grid(group.offset), "variant group {:#07x} off grid", group.offset);
        }
        for &offset in ROTATE_ONLY_QUARTETS {
            assert!(on_grid(offset), "rotate-only quartet {offset:#07x} off grid");
        }
    }
}
