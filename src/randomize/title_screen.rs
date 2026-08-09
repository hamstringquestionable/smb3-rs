use crate::randomizer::Options;
use crate::rom::Rom;

/// Tile pairs for each icon: (left_tile, right_tile, right_extra_attributes).
/// `right_extra_attributes` is OR'd with the palette bits — 0x40 means h-flip
/// (symmetric icon using the same tile mirrored), 0x00 means a distinct R tile.
const ICON_TILES: [(u8, u8, u8); 15] = [
    (0xF1, 0xF3, 0x00), // leaf (L/R pair)
    (0xF5, 0xF5, 0x40), // mirrored
    (0xF7, 0xF7, 0x40), // mirrored
    (0xF9, 0xF9, 0x40), // mirrored
    (0xFB, 0xFB, 0x40), // mirrored
    (0x0D, 0x0F, 0x00), // L/R pair
    (0x11, 0x11, 0x40), // mirrored
    (0x13, 0x13, 0x40), // mirrored (mushroom house)
    (0x17, 0x19, 0x00), // L/R pair
    (0x29, 0x2B, 0x00), // L/R pair
    (0x4B, 0x4D, 0x00), // L/R pair
    (0x59, 0x5B, 0x00), // L/R pair
    (0x6B, 0x6D, 0x00), // L/R pair
    (0x79, 0x79, 0x40), // mirrored
    (0xDB, 0xDB, 0x40), // mirrored
];

const NUM_ICONS: usize = ICON_TILES.len();
const HASH_LENGTH: usize = 5;

/// Sprite palettes to choose from: palette 0 (red) and palette 2 (orange/yellow).
const PALETTES: [u8; 2] = [0x00, 0x02];

/// Hook: replace JSR $B7D6 at CPU $97B1 with JMP $E914.
const HOOK_OFFSET: usize = 0x317B1;

/// PRG031 free space for the sprite copy routine — from rom_data::FS_SEED_HASH_ROUTINE.
const ROUTINE_OFFSET: usize = super::rom_data::FS_SEED_HASH_ROUTINE;
const ROUTINE_CPU: u16 = super::rom_data::prg031_file_to_cpu(ROUTINE_OFFSET); // $E914

/// Sprite data table immediately after the routine — from rom_data::FS_SEED_HASH_DATA.
const DATA_OFFSET: usize = super::rom_data::FS_SEED_HASH_DATA;
const DATA_CPU: u16 = super::rom_data::prg031_file_to_cpu(DATA_OFFSET); // $E92D

/// Skip the title screen intro cutscene by setting Title_State = 6 (IntroSkip)
/// during init, after the zero-page clear. Title_State is at zero-page $DE.
/// We hook STA $0736 at file 0x308E2 → JSR $E955 (free space after sprite data).
const INTRO_SKIP_HOOK_OFFSET: usize = 0x308E2;
const INTRO_SKIP_ROUTINE_OFFSET: usize = super::rom_data::FS_INTRO_SKIP;
const INTRO_SKIP_CPU: u16 = super::rom_data::prg031_file_to_cpu(INTRO_SKIP_ROUTINE_OFFSET); // $E955

/// Disable the attract-mode demo. When the 1P/2P menu sits idle, a countdown
/// (`$DF` × `$E0` frames) expires and the title handler at PRG024 CPU $8C4E does
/// `LDA #$FF / STA $E1`; a later check (`LDA $E1 / BEQ / JMP $A8AF` at CPU $8979)
/// reads that flag and jumps into the recorded demo playback. Setting the stored
/// value to 0 keeps `$E1` clear, so the `BEQ` is always taken and the demo never
/// starts — the menu just holds. The byte at 0x30C5F is the `#$FF` operand.
const DEMO_TRIGGER_OPERAND_OFFSET: usize = 0x30C5F;

/// Curated music tracks for the title menu. Values are written to the music
/// change trigger at $04F5 — each one is a looping theme that fits a static
/// menu screen (map themes 1–8 plus a handful of level / mushroom / hilly
/// loops). See docs/smb3_rom_reference.md "Music Values ($04F5)".
const MENU_MUSIC_TRACKS: [u8; 16] = [
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, // World 1–8 map themes
    0x10, // Plains
    0x20, // Underground
    0x30, // Water
    0x40, // Dungeon / Fortress
    0x60, // Doomship
    0x80, // Mushroom house
    0x90, // Hilly theme
    0x09, // World 9 / Coin Heaven
];

/// Pick a deterministic menu music track from the seed. Independent of
/// `compute_hash` so changes to the music list don't shift seed-verification
/// icons.
pub(super) fn pick_menu_music(seed: u64) -> u8 {
    let h = seed
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(0xBF58_476D_1CE4_E5B9);
    MENU_MUSIC_TRACKS[(h % MENU_MUSIC_TRACKS.len() as u64) as usize]
}

/// CHR page (1 KB) loaded into each sprite pattern slot on the title screen.
/// The reset path in PRG030 (`PRG030_845A`) sets MMC3 R2–R5 to $20/$21/$04/$7F
/// before handing control to the title handler, so an OAM tile ID of $00–$3F
/// reads from page $20, $40–$7F from $21, $80–$BF from $04, $C0–$FF from $7F.
const TITLE_SPRITE_CHR_PAGES: [usize; 4] = [0x20, 0x21, 0x04, 0x7F];

/// File offset of `Title_Load_Palette` (PRG025): a 32-byte upload to $3F00 —
/// 16 background colors followed by the 16 sprite colors, four per palette.
const TITLE_SPRITE_PALETTE_OFFSET: usize = 0x326AD + 16;

/// Sprite positions: vertical column in top-left corner, inset from edge.
const X_LEFT: u8 = 16;
const X_RIGHT: u8 = 24;
const Y_START: u8 = 64;
const Y_SPACING: u8 = 24;

/// Instruction bytes shared by the intro-skip routine (here) and the
/// starting-items trampoline (`qol::starting_state`): set Title_State ($DE) =
/// 6 (IntroSkip) and queue the seeded menu music via $04F5.
pub(super) fn intro_skip_music_bytes(seed: u64) -> [u8; 9] {
    let music = pick_menu_music(seed);
    [
        0xA9, 0x06,       // LDA #$06
        0x85, 0xDE,       // STA $DE    (Title_State = IntroSkip)
        0xA9, music,      // LDA #music
        0x8D, 0xF5, 0x04, // STA $04F5  (queue menu music)
    ]
}

/// Compute 5 icon indices and a palette choice from seed + flag bytes.
///
/// The randomizer version (`CARGO_PKG_VERSION`) is folded in so two builds with
/// different randomization logic never collide on the same icons for a shared
/// seed + options. CI (`version-guard` in `.github/workflows/ci.yml`) requires a
/// version bump on every merge to `main`, so any output-affecting change lands
/// under a distinct version and thus a distinct hash.
fn compute_hash(seed: u64, options: &Options) -> ([usize; HASH_LENGTH], u8) {
    let flag_bytes = options.to_flag_bytes();
    let mut h = seed;
    for &b in &flag_bytes {
        h = h.wrapping_mul(2_654_435_761).wrapping_add(b as u64);
    }
    for &b in env!("CARGO_PKG_VERSION").as_bytes() {
        h = h.wrapping_mul(2_654_435_761).wrapping_add(b as u64);
    }

    let mut icons = [0usize; HASH_LENGTH];
    for icon in &mut icons {
        *icon = (h % NUM_ICONS as u64) as usize;
        h /= NUM_ICONS as u64;
    }
    let palette = PALETTES[(h % PALETTES.len() as u64) as usize];
    (icons, palette)
}

/// One hash icon as a CHR renderer needs it: four 8x8 tile indices into CHR
/// ROM in `[top-left, top-right, bottom-left, bottom-right]` order, plus
/// whether the right half is the left half mirrored.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeedHashIcon {
    pub tiles: [usize; 4],
    pub flip_right: bool,
}

/// The five title-screen hash icons for a seed + options, resolved against a
/// specific ROM's title-screen CHR banks and palette. Lets the web app draw
/// the icons the player is about to see, from their own ROM's graphics.
#[derive(serde::Serialize)]
pub struct SeedHashPreview {
    pub icons: Vec<SeedHashIcon>,
    /// NES color indices of the chosen sprite palette. Entry 0 is the
    /// universal backdrop and renders transparent.
    pub palette: [u8; 4],
}

/// Resolve an 8x16-mode OAM tile ID to its absolute CHR ROM tile index.
/// Every `ICON_TILES` entry is odd (pattern table 1), so the top tile of the
/// pair is `tile & 0xFE` and the bottom is the one after it.
fn chr_tile_index(tile: u8) -> usize {
    TITLE_SPRITE_CHR_PAGES[(tile >> 6) as usize] * 64 + (tile & 0x3F) as usize
}

/// Describe the hash `write_seed_hash` would stamp for `seed` + `options`,
/// without writing anything. `rom` is only read for its title sprite palette.
pub fn seed_hash_preview(rom: &[u8], seed: u64, options: &Options) -> SeedHashPreview {
    let (icons, palette_attr) = compute_hash(seed, options);
    let icons = icons
        .iter()
        .map(|&i| {
            let (tile_l, tile_r, extra_attr_r) = ICON_TILES[i];
            let (top_l, top_r) = (tile_l & 0xFE, tile_r & 0xFE);
            SeedHashIcon {
                tiles: [
                    chr_tile_index(top_l),
                    chr_tile_index(top_r),
                    chr_tile_index(top_l + 1),
                    chr_tile_index(top_r + 1),
                ],
                flip_right: extra_attr_r & 0x40 != 0,
            }
        })
        .collect();

    // A ROM too short to hold the table (only reachable with validation off,
    // where the hash isn't written anyway) falls back to all-backdrop.
    let base = TITLE_SPRITE_PALETTE_OFFSET + (palette_attr & 0x03) as usize * 4;
    let mut palette = [0x0Fu8; 4];
    if let Some(colors) = rom.get(base..base + 4) {
        palette.copy_from_slice(colors);
    }

    SeedHashPreview { icons, palette }
}

/// Write seed hash sprites to the title screen.
///
/// Places 5 icons (each 16x16, made of two 8x16 sprites) vertically
/// in the top-left corner. The icons and palette are deterministically
/// chosen from the seed and options so players can verify they share
/// the same settings.
/// Build the 40-byte sprite data table: 5 icons x 2 sprites x 4 bytes.
/// The copy routine iterates X downward in groups of 8, mapping:
///   data[32..39] -> OAM[0..7],  data[24..31] -> OAM[32..39],
///   data[16..23] -> OAM[64..71], data[8..15] -> OAM[96..103],
///   data[0..7]   -> OAM[128..135]
/// We place icon 0 (topmost) in the highest data group so it lands
/// in the lowest OAM slot (highest sprite priority).
fn build_sprite_data(icons: &[usize; HASH_LENGTH], palette: u8) -> [u8; HASH_LENGTH * 8] {
    let mut sprite_data = [0u8; HASH_LENGTH * 8];
    for i in 0..HASH_LENGTH {
        let group = HASH_LENGTH - 1 - i; // icon 0 -> group 4 (bytes 32-39)
        let y = Y_START + i as u8 * Y_SPACING;
        let (tile_l, tile_r, extra_attr_r) = ICON_TILES[icons[i]];
        let base = group * 8;
        sprite_data[base] = y;
        sprite_data[base + 1] = tile_l;
        sprite_data[base + 2] = palette;
        sprite_data[base + 3] = X_LEFT;
        sprite_data[base + 4] = y;
        sprite_data[base + 5] = tile_r;
        sprite_data[base + 6] = palette | extra_attr_r;
        sprite_data[base + 7] = X_RIGHT;
    }
    sprite_data
}

pub fn write_seed_hash(rom: &mut Rom, seed: u64, options: &Options) {
    let (icons, palette) = compute_hash(seed, options);
    let sprite_data = build_sprite_data(&icons, palette);

    // ASM routine (25 bytes) at CPU $E914:
    //   LDY #$07
    //   LDX #$27          ; 40 bytes - 1
    // loop:
    //   LDA table,X       ; $E92D
    //   STA $0200,Y
    //   TXA
    //   AND #$07
    //   BNE +5
    //   TYA
    //   CLC
    //   ADC #$28          ; stride = 40 OAM bytes between groups
    //   TAY
    // skip:
    //   DEY
    //   DEX
    //   BPL loop
    //   RTS
    #[rustfmt::skip]
    let routine: [u8; 25] = [
        0xA0, 0x07,                         // LDY #$07
        0xA2, (HASH_LENGTH * 8 - 1) as u8,  // LDX #$27
        0xBD, DATA_CPU as u8, (DATA_CPU >> 8) as u8, // LDA $E92D,X
        0x99, 0x00, 0x02,                   // STA $0200,Y
        0x8A,                                // TXA
        0x29, 0x07,                          // AND #$07
        0xD0, 0x05,                          // BNE +5
        0x98,                                // TYA
        0x18,                                // CLC
        0x69, 0x28,                          // ADC #$28
        0xA8,                                // TAY
        0x88,                                // DEY
        0xCA,                                // DEX
        0x10, 0xEC,                          // BPL loop
        0x60,                                // RTS
    ];

    rom.write_range(ROUTINE_OFFSET, &routine);
    rom.write_range(DATA_OFFSET, &sprite_data);

    // Hook: replace JSR $B7D6 with JMP $E914 in the title screen sprite loop.
    rom.write_range(HOOK_OFFSET, &[0x4C, ROUTINE_CPU as u8, (ROUTINE_CPU >> 8) as u8]);

    // Skip intro cutscene: hook STA $0736 in title screen init to also set
    // Title_State ($DE) = 6 (IntroSkip). This loads all graphics quickly and
    // jumps straight to the 1P/2P menu, ensuring consistent CHR banks for our
    // hash sprites. Also queue a randomized menu music track via $04F5.
    //
    // Replace: 8D 36 07 (STA $0736) → 20 55 E9 (JSR $E955)
    // At $E955: STA $0736 / LDA #$06 / STA $DE / LDA #music / STA $04F5 / RTS
    //
    // NOTE: when starting items are enabled, `qol::write_starting_items` later
    // overwrites the whole lives-init block at 0x308E0 (including this hook)
    // with its own trampoline, which replays the same intro-skip + music bytes
    // (`intro_skip_music_bytes`); the routine below is then dead but harmless.
    rom.write_range(
        INTRO_SKIP_HOOK_OFFSET,
        &[0x20, INTRO_SKIP_CPU as u8, (INTRO_SKIP_CPU >> 8) as u8],
    );
    let mut intro_routine = vec![0x8D, 0x36, 0x07]; // STA $0736 (original instruction)
    intro_routine.extend_from_slice(&intro_skip_music_bytes(seed));
    intro_routine.push(0x60); // RTS
    rom.write_range(INTRO_SKIP_ROUTINE_OFFSET, &intro_routine);

    // Disable the attract-mode demo: LDA #$FF → LDA #$00 so the idle-timeout
    // never raises the demo-trigger flag $E1 (see DEMO_TRIGGER_OPERAND_OFFSET).
    rom.write_byte(DEMO_TRIGGER_OPERAND_OFFSET, 0x00);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_deterministic() {
        let opts = Options::default();
        let a = compute_hash(12345, &opts);
        let b = compute_hash(12345, &opts);
        assert_eq!(a, b);
    }

    #[test]
    fn hash_differs_by_seed() {
        let opts = Options::default();
        let a = compute_hash(1, &opts);
        let b = compute_hash(2, &opts);
        assert_ne!(a.0, b.0);
    }

    #[test]
    fn hash_differs_by_options() {
        let opts_a = Options { ground: crate::randomizer::EnemyMode::Off, ..Default::default() };
        let opts_b = Options { ground: crate::randomizer::EnemyMode::Wild, ..Default::default() };
        let a = compute_hash(42, &opts_a);
        let b = compute_hash(42, &opts_b);
        assert_ne!(a.0, b.0);
    }

    #[test]
    fn hash_values_in_range() {
        let opts = Options::default();
        for seed in 0..100u64 {
            let (icons, palette) = compute_hash(seed, &opts);
            for &v in &icons {
                assert!(v < NUM_ICONS, "icon index {v} out of range");
            }
            assert!(
                PALETTES.contains(&palette),
                "palette {palette} not in PALETTES"
            );
        }
    }

    #[test]
    fn sprite_data_positions() {
        let opts = Options::default();
        let (icons, palette) = compute_hash(42, &opts);
        let sprite_data = build_sprite_data(&icons, palette);

        // Icon 0 is in group 4 (bytes 32-39), should have Y_START
        assert_eq!(sprite_data[32], Y_START);
        assert_eq!(sprite_data[32 + 3], X_LEFT);
        assert_eq!(sprite_data[32 + 7], X_RIGHT);

        // Icon 4 is in group 0 (bytes 0-7), should have Y_START + 4*Y_SPACING
        assert_eq!(sprite_data[0], Y_START + 4 * Y_SPACING);
    }

    #[test]
    fn write_seed_hash_writes_sprite_data_to_rom() {
        let opts = Options::default();
        let mut rom = crate::randomize::qol::test_support::make_test_rom();
        rom.write_byte(DEMO_TRIGGER_OPERAND_OFFSET, 0xFF); // vanilla operand
        write_seed_hash(&mut rom, 42, &opts);

        // The data table in ROM must be exactly what build_sprite_data emits.
        let (icons, palette) = compute_hash(42, &opts);
        let expected = build_sprite_data(&icons, palette);
        assert_eq!(rom.read_range(DATA_OFFSET, expected.len()), &expected[..]);

        // Hook and intro-skip hook target the derived CPU addresses.
        assert_eq!(rom.read_range(HOOK_OFFSET, 3), &[0x4C, 0x14, 0xE9]);
        assert_eq!(rom.read_range(INTRO_SKIP_HOOK_OFFSET, 3), &[0x20, 0x55, 0xE9]);

        // Attract-mode demo trigger neutralized: LDA #$FF operand becomes #$00.
        assert_eq!(rom.read_byte(DEMO_TRIGGER_OPERAND_OFFSET), 0x00);
    }

    #[test]
    fn icon_tiles_are_all_pattern_table_1() {
        // `chr_tile_index` assumes every icon tile is odd (bit 0 = pattern
        // table 1), which is what makes `tile & 0xFE` the top half of the
        // 8x16 pair. An even entry would silently render from PT0.
        for (i, &(l, r, _)) in ICON_TILES.iter().enumerate() {
            assert_eq!(l & 1, 1, "icon {i} left tile {l:#04X} is not in PT1");
            assert_eq!(r & 1, 1, "icon {i} right tile {r:#04X} is not in PT1");
        }
    }

    #[test]
    fn preview_matches_written_sprite_data() {
        let opts = Options::default();
        let mut rom = crate::randomize::qol::test_support::make_test_rom();
        write_seed_hash(&mut rom, 42, &opts);
        let written = rom.read_range(DATA_OFFSET, HASH_LENGTH * 8).to_vec();

        let preview = seed_hash_preview(&rom.data, 42, &opts);
        assert_eq!(preview.icons.len(), HASH_LENGTH);

        for (i, icon) in preview.icons.iter().enumerate() {
            // Icon i sits in group HASH_LENGTH-1-i of the OAM data table.
            let base = (HASH_LENGTH - 1 - i) * 8;
            let (tile_l, tile_r) = (written[base + 1], written[base + 5]);
            assert_eq!(icon.tiles[0], chr_tile_index(tile_l & 0xFE));
            assert_eq!(icon.tiles[1], chr_tile_index(tile_r & 0xFE));
            assert_eq!(icon.tiles[2], icon.tiles[0] + 1);
            assert_eq!(icon.tiles[3], icon.tiles[1] + 1);
            assert_eq!(icon.flip_right, written[base + 6] & 0x40 != 0);
        }
    }

    #[test]
    fn preview_tile_indices_stay_inside_chr() {
        // 128 KB of CHR = 8192 tiles.
        let opts = Options::default();
        let rom = crate::randomize::qol::test_support::make_test_rom();
        for seed in 0..100u64 {
            for icon in seed_hash_preview(&rom.data, seed, &opts).icons {
                for t in icon.tiles {
                    assert!(t < 8192, "tile index {t} past the end of CHR");
                }
            }
        }
    }

    #[test]
    fn menu_music_deterministic() {
        assert_eq!(pick_menu_music(12345), pick_menu_music(12345));
    }

    #[test]
    fn menu_music_in_table() {
        for seed in 0..1000u64 {
            assert!(MENU_MUSIC_TRACKS.contains(&pick_menu_music(seed)));
        }
    }

    #[test]
    fn menu_music_varies_across_seeds() {
        let mut seen = std::collections::HashSet::new();
        for seed in 0..1000u64 {
            seen.insert(pick_menu_music(seed));
        }
        assert!(
            seen.len() >= MENU_MUSIC_TRACKS.len() / 2,
            "music selection too clumpy: only {} distinct tracks across 1000 seeds",
            seen.len()
        );
    }

    #[test]
    fn palette_varies_across_seeds() {
        let opts = Options::default();
        let mut saw_pal0 = false;
        let mut saw_pal2 = false;
        for seed in 0..1000u64 {
            let (_, palette) = compute_hash(seed, &opts);
            if palette == 0x00 {
                saw_pal0 = true;
            }
            if palette == 0x02 {
                saw_pal2 = true;
            }
            if saw_pal0 && saw_pal2 {
                break;
            }
        }
        assert!(saw_pal0, "palette 0 never selected in 1000 seeds");
        assert!(saw_pal2, "palette 2 never selected in 1000 seeds");
    }
}
