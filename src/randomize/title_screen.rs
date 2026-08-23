use crate::randomizer::Options;
use crate::rom::Rom;

/// Tile pairs for each icon: (left_tile, right_tile, right_extra_attributes).
/// `right_extra_attributes` is OR'd with the palette bits — 0x40 means h-flip
/// (symmetric icon using the same tile mirrored), 0x00 means a distinct R tile.
const ICON_TILES: [(u8, u8, u8); 20] = [
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
    (0x75, 0x77, 0x00), // shoe
    (0x55, 0x57, 0x00), // flame
    (0xB9, 0xBB, 0x00), // note on a panel
    (0xB5, 0xB5, 0x40), // window block (mirrored)
    (0x89, 0x89, 0x00), // "00" (half doubled, not mirrored)
];

const NUM_ICONS: usize = ICON_TILES.len();
const HASH_LENGTH: usize = 5;

/// Sprite palette the icons are drawn in. Fixed, and deliberately not part of
/// the hash.
///
/// It used to be a sixth digit picking between palettes 0 and 2, which doubled
/// the output space — but palette 0 is Mario's, and every player re-skin in the
/// web app's visual patch catalog rewrites it. The same seed then showed red on
/// vanilla, green under Luigi and cyan under Toad, so a player comparing title
/// screens couldn't tell a recoloured palette from a different seed. Unlike a
/// re-skinned *shape*, which reads as "that's a skin" and can be discounted, a
/// colour carries no such signal — it was pure payload, and unverifiable.
///
/// Widening `ICON_TILES` from 15 to 20 more than replaces what dropping the
/// digit cost: 20^5 = 3,200,000 against the old 15^5 * 2 = 1,518,750. Slot 2 is
/// not the bros', so no re-skin has reason to touch it.
const HASH_PALETTE: u8 = 0x02;

// Slots 0 and 1 are the bros', and every player re-skin rewrites them. Guard it
// at compile time rather than in a test — this is a fact about which slot is
// safe, not behaviour that needs exercising.
const _: () = assert!(HASH_PALETTE >= 2, "hash palette must not be the bros'");

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

/// Mute toggle: press B on the 1P/2P menu to stop the menu music, B again to
/// bring it back.
///
/// `Title_Do1P2PMenu` (PRG024) calls `Title_3Glow` once per frame at CPU $AC83,
/// between drawing the cursor sprite and testing Start. That `JSR $AA7D` becomes
/// `JSR mute_routine`, and the routine tail-jumps to `Title_3Glow` — so the glow
/// still runs and its `RTS` returns to the menu exactly as before.
///
/// The routine lives in PRG025, which the title entry point in PRG030 maps to
/// $C000 alongside PRG024 at $A000. Putting it there rather than in PRG031
/// spends none of the always-mapped space (30 bytes is its largest gap), and the
/// routine has no reason to run anywhere but the title screen.
const MUTE_HOOK_OFFSET: usize = 0x30C93;
const TITLE_3GLOW_CPU: u16 = 0xAA7D;
const MUTE_ROUTINE_OFFSET: usize = super::rom_data::FS_TITLE_MUTE;
/// PRG025 file 0x32010 is CPU $C000 while the title screen runs.
const MUTE_ROUTINE_CPU: u16 = (0xC000 + MUTE_ROUTINE_OFFSET - 0x32010) as u16; // $D519

/// Assemble the 22-byte mute toggle. `music` is the seeded track it restores.
///
/// ```text
///   BIT $18        ; Pad_Input — bit 6 is B, and BIT drops it straight into V
///   BVC done       ; B not newly pressed this frame: nothing to do
///   LDX #$01       ; assume unmute: $04F4 + 1 = $04F5 (Sound_QMusic2)
///   LDA #music
///   LDY $04E5      ; SndCur_Music2 — a Set 2 track is playing iff non-zero
///   BEQ store      ; silent already, so the B press means unmute
///   LDA #$80       ; MUS1_STOPMUSIC
///   DEX            ; X = 0 → $04F4 (Sound_QMusic1)
/// store:
///   STA $04F4,X
/// done:
///   JMP $AA7D      ; Title_3Glow, the call this routine displaced
/// ```
///
/// Two size choices worth keeping: `BIT` tests B in 2 bytes where `LDA`/`AND`
/// takes 4 (B is bit 6, which is exactly the bit `BIT` copies into V), and the
/// two music queues are adjacent, so one `STA $04F4,X` with X as the 0/1
/// selector replaces a second store and the branch that would skip it.
///
/// The engine's own `SndCur_Music2` is the mute flag — `MUS1_STOPMUSIC` runs
/// through `Music_StopAll`, which zeroes it — so no RAM byte is spent tracking
/// state, and nothing can drift out of sync with what is actually audible.
#[rustfmt::skip]
fn mute_routine(music: u8) -> [u8; 22] {
    [
        0x24, 0x18,             // BIT $18       (Pad_Input)
        0x50, 0x0F,             // BVC done
        0xA2, 0x01,             // LDX #$01
        0xA9, music,            // LDA #music
        0xAC, 0xE5, 0x04,       // LDY $04E5     (SndCur_Music2)
        0xF0, 0x03,             // BEQ store
        0xA9, 0x80,             // LDA #$80      (MUS1_STOPMUSIC)
        0xCA,                   // DEX
        0x9D, 0xF4, 0x04,       // STA $04F4,X   (Sound_QMusic1 / Sound_QMusic2)
        0x4C, TITLE_3GLOW_CPU as u8, (TITLE_3GLOW_CPU >> 8) as u8, // JMP Title_3Glow
    ]
}

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

/// Compute the 5 icon indices from seed + flag bytes — five base-`NUM_ICONS`
/// digits read off one accumulator.
///
/// The randomizer version (`CARGO_PKG_VERSION`) is folded in so two builds with
/// different randomization logic never collide on the same icons for a shared
/// seed + options. CI (`version-guard` in `.github/workflows/ci.yml`) requires a
/// version bump on every merge to `main`, so any output-affecting change lands
/// under a distinct version and thus a distinct hash.
fn compute_hash(seed: u64, options: &Options) -> [usize; HASH_LENGTH] {
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
    icons
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
    let icons = compute_hash(seed, options);
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
    let base = TITLE_SPRITE_PALETTE_OFFSET + HASH_PALETTE as usize * 4;
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
fn build_sprite_data(icons: &[usize; HASH_LENGTH]) -> [u8; HASH_LENGTH * 8] {
    let mut sprite_data = [0u8; HASH_LENGTH * 8];
    for i in 0..HASH_LENGTH {
        let group = HASH_LENGTH - 1 - i; // icon 0 -> group 4 (bytes 32-39)
        let y = Y_START + i as u8 * Y_SPACING;
        let (tile_l, tile_r, extra_attr_r) = ICON_TILES[icons[i]];
        let base = group * 8;
        sprite_data[base] = y;
        sprite_data[base + 1] = tile_l;
        sprite_data[base + 2] = HASH_PALETTE;
        sprite_data[base + 3] = X_LEFT;
        sprite_data[base + 4] = y;
        sprite_data[base + 5] = tile_r;
        sprite_data[base + 6] = HASH_PALETTE | extra_attr_r;
        sprite_data[base + 7] = X_RIGHT;
    }
    sprite_data
}

pub fn write_seed_hash(rom: &mut Rom, seed: u64, options: &Options) {
    let icons = compute_hash(seed, options);
    let sprite_data = build_sprite_data(&icons);

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

    // B on the 1P/2P menu toggles the menu music. The routine replaces the
    // per-frame `JSR Title_3Glow` and tail-jumps back into it, so the '3' keeps
    // glowing and Start still advances to the map. The demo-disable above is
    // what makes the mute stick: with $E1 held at 0 the menu never resets, so
    // the intro-skip routine never re-queues the track behind the player's back.
    rom.write_range(MUTE_ROUTINE_OFFSET, &mute_routine(pick_menu_music(seed)));
    rom.write_range(
        MUTE_HOOK_OFFSET,
        &[0x20, MUTE_ROUTINE_CPU as u8, (MUTE_ROUTINE_CPU >> 8) as u8],
    );
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
        assert_ne!(a, b);
    }

    #[test]
    fn hash_differs_by_options() {
        let opts_a = Options { ground: crate::randomizer::EnemyMode::Off, ..Default::default() };
        let opts_b = Options { ground: crate::randomizer::EnemyMode::Wild, ..Default::default() };
        let a = compute_hash(42, &opts_a);
        let b = compute_hash(42, &opts_b);
        assert_ne!(a, b);
    }

    #[test]
    fn hash_values_in_range() {
        let opts = Options::default();
        for seed in 0..100u64 {
            let icons = compute_hash(seed, &opts);
            for &v in &icons {
                assert!(v < NUM_ICONS, "icon index {v} out of range");
            }
        }
    }

    #[test]
    fn sprite_data_positions() {
        let opts = Options::default();
        let icons = compute_hash(42, &opts);
        let sprite_data = build_sprite_data(&icons);

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
        let icons = compute_hash(42, &opts);
        let expected = build_sprite_data(&icons);
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
    fn mute_routine_restores_the_seeded_track() {
        // The unmute path writes the same track the intro-skip routine queues,
        // so muting and unmuting cannot land the player on a different song.
        for seed in [0u64, 1, 42, 12345, u64::MAX] {
            let routine = mute_routine(pick_menu_music(seed));
            assert_eq!(routine[7], pick_menu_music(seed));
            assert_eq!(intro_skip_music_bytes(seed)[5], routine[7]);
        }
    }

    #[test]
    fn write_seed_hash_installs_the_mute_hook() {
        let opts = Options::default();
        let mut rom = crate::randomize::qol::test_support::make_test_rom();
        write_seed_hash(&mut rom, 42, &opts);

        // JSR $D519 in place of the vanilla JSR Title_3Glow.
        assert_eq!(rom.read_range(MUTE_HOOK_OFFSET, 3), &[0x20, 0x19, 0xD5]);
        assert_eq!(
            rom.read_range(MUTE_ROUTINE_OFFSET, 22),
            &mute_routine(pick_menu_music(42))[..]
        );
    }

    #[test]
    fn mute_hook_site_matches_vanilla() {
        // Guards the offset itself: 0x30C93 is CPU $AC83, the per-frame
        // `JSR Title_3Glow` inside Title_Do1P2PMenu. A drifted offset would
        // splice a JSR into the middle of some other instruction.
        let Some(v) = vanilla() else {
            eprintln!("SKIP: requires the ROM, which is not included in the repo");
            return;
        };
        assert_eq!(
            &v[MUTE_HOOK_OFFSET..MUTE_HOOK_OFFSET + 3],
            &[0x20, TITLE_3GLOW_CPU as u8, (TITLE_3GLOW_CPU >> 8) as u8],
        );
        // ...followed by the Start test the menu does every frame.
        assert_eq!(&v[MUTE_HOOK_OFFSET + 3..MUTE_HOOK_OFFSET + 7], &[0xA5, 0x18, 0x29, 0x10]);
    }

    fn vanilla() -> Option<Vec<u8>> {
        std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()
    }

    #[test]
    fn every_icon_is_reachable() {
        // A digit is `h % NUM_ICONS`, so every index must map to a real row —
        // and the table's length is what NUM_ICONS is derived from, so this
        // catches a row added without the count following.
        let opts = Options::default();
        let mut seen = [false; NUM_ICONS];
        for seed in 0..200_000u64 {
            for i in compute_hash(seed, &opts) {
                seen[i] = true;
            }
            if seen.iter().all(|&s| s) {
                return;
            }
        }
        let missing: Vec<_> = seen
            .iter()
            .enumerate()
            .filter(|&(_, &s)| !s)
            .map(|(i, _)| i)
            .collect();
        panic!("icons never selected across 200k seeds: {missing:?}");
    }
}

#[cfg(test)]
mod asm_checks {
    use super::*;
    use crate::randomize::rom_data::asm;

    #[test]
    fn mute_routine_is_well_formed() {
        asm::check(&mute_routine(0x10))
            .allocation(MUTE_ROUTINE_OFFSET)
            .origin(MUTE_ROUTINE_CPU)
            .assert_ok();
    }

    /// The hook overwrites `JSR Title_3Glow` — one whole 3-byte instruction —
    /// and must aim at the routine's own origin. Split out because comparing
    /// against vanilla needs the ROM, which CI does not have.
    #[test]
    fn mute_hook_displaces_whole_instructions() {
        let Some(v) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok() else {
            eprintln!("SKIP: requires the ROM, which is not included in the repo");
            return;
        };
        let patch = [0x20, MUTE_ROUTINE_CPU as u8, (MUTE_ROUTINE_CPU >> 8) as u8];
        asm::check(&mute_routine(0x10))
            .origin(MUTE_ROUTINE_CPU)
            .hook(&v, MUTE_HOOK_OFFSET, &patch)
            .assert_ok();
    }

    /// Run the mute routine on an emulated CPU and report what it queued.
    ///
    /// The routine is self-contained up to its tail `JMP Title_3Glow` — it reads
    /// `Pad_Input` and `SndCur_Music2` and writes one of the two music queues —
    /// so the whole thing can be executed rather than merely decoded. Stops at
    /// the `JMP`, which would otherwise run off into unloaded memory.
    fn run_mute(pad: u8, playing: u8, music: u8) -> (u8, u8) {
        use mos6502::cpu::CPU;
        use mos6502::instruction::{Instruction, Ricoh2a03};
        use mos6502::memory::{Bus, Memory};
        use mos6502::Variant;

        let code = mute_routine(music);
        let mut mem = Memory::new();
        mem.set_bytes(MUTE_ROUTINE_CPU, &code);
        mem.set_byte(0x0018, pad); // Pad_Input
        mem.set_byte(0x04E5, playing); // SndCur_Music2
        mem.set_byte(0x04F4, 0x00); // Sound_QMusic1, cleared every frame
        mem.set_byte(0x04F5, 0x00); // Sound_QMusic2, likewise
        let mut cpu = CPU::new(mem, Ricoh2a03);
        cpu.registers.program_counter = MUTE_ROUTINE_CPU;

        for _ in 0..code.len() {
            let op = cpu.memory.get_byte(cpu.registers.program_counter);
            if matches!(Ricoh2a03::decode(op), Some((Instruction::JMP, _))) {
                return (cpu.memory.get_byte(0x04F4), cpu.memory.get_byte(0x04F5));
            }
            cpu.single_step();
        }
        panic!("mute routine ran past {} instructions without reaching its JMP", code.len());
    }

    /// The routine's actual behaviour, over every input it can see: 256 pad
    /// states x 256 "what's playing" values. Decoding cannot tell whether `BIT`
    /// really lands B in V, or whether `LDX #$01`/`DEX` really selects between
    /// two adjacent queues — executing it can.
    #[test]
    fn mute_routine_queues_the_right_thing() {
        const MUSIC: u8 = 0x30;
        for playing in 0..=u8::MAX {
            for pad in 0..=u8::MAX {
                let (fanfare, theme) = run_mute(pad, playing, MUSIC);
                let want = match (pad & 0x40 != 0, playing != 0) {
                    (false, _) => (0x00, 0x00),  // B not pressed: queue nothing
                    (true, true) => (0x80, 0x00), // audible: MUS1_STOPMUSIC
                    (true, false) => (0x00, MUSIC), // silent: restore the track
                };
                assert_eq!(
                    (fanfare, theme),
                    want,
                    "pad {pad:#04X}, SndCur_Music2 {playing:#04X}",
                );
            }
        }
    }

    /// Pressing B twice returns to the starting state, which is what makes it a
    /// toggle rather than a one-way mute. The engine's own `SndCur_Music2` is
    /// the state, so the second press has to see it as `Music_StopAll` left it.
    #[test]
    fn two_presses_restore_the_track() {
        const MUSIC: u8 = 0x60;
        const B: u8 = 0x40;
        // Playing -> the press queues a stop; Music_StopAll then zeroes
        // SndCur_Music2, which is what the next press reads.
        assert_eq!(run_mute(B, MUSIC, MUSIC), (0x80, 0x00));
        assert_eq!(run_mute(B, 0x00, MUSIC), (0x00, MUSIC));
    }

    /// The seed-hash sprite copy routine and the intro-skip routine predate the
    /// `asm::check` rule; check them here now that the module exists.
    #[test]
    fn seed_hash_routines_are_well_formed() {
        let mut rom = crate::randomize::qol::test_support::make_test_rom();
        write_seed_hash(&mut rom, 42, &Options::default());

        asm::check(rom.read_range(ROUTINE_OFFSET, 25))
            .allocation(ROUTINE_OFFSET)
            .origin(ROUTINE_CPU)
            .assert_ok();
        asm::check(rom.read_range(INTRO_SKIP_ROUTINE_OFFSET, 13))
            .allocation(INTRO_SKIP_ROUTINE_OFFSET)
            .origin(INTRO_SKIP_CPU)
            .assert_ok();
    }
}
