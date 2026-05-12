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

/// Sprite data table immediately after the routine — from rom_data::FS_SEED_HASH_DATA.
const DATA_OFFSET: usize = super::rom_data::FS_SEED_HASH_DATA;
const DATA_CPU_LO: u8 = 0x2D;
const DATA_CPU_HI: u8 = 0xE9;

/// Skip the title screen intro cutscene by setting Title_State = 6 (IntroSkip)
/// during init, after the zero-page clear. Title_State is at zero-page $DE.
/// We hook STA $0736 at file 0x308E2 → JSR $E955 (free space after sprite data).
const INTRO_SKIP_HOOK_OFFSET: usize = 0x308E2;
const INTRO_SKIP_ROUTINE_OFFSET: usize = super::rom_data::FS_INTRO_SKIP;

/// Sprite positions: vertical column in top-left corner, inset from edge.
const X_LEFT: u8 = 16;
const X_RIGHT: u8 = 24;
const Y_START: u8 = 64;
const Y_SPACING: u8 = 24;

/// Compute 5 icon indices and a palette choice from seed + flag bytes.
fn compute_hash(seed: u64, options: &Options) -> ([usize; HASH_LENGTH], u8) {
    let flag_bytes = options.to_flag_bytes();
    let mut h = seed;
    for &b in &flag_bytes {
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

/// Write seed hash sprites to the title screen.
///
/// Places 5 icons (each 16x16, made of two 8x16 sprites) vertically
/// in the top-left corner. The icons and palette are deterministically
/// chosen from the seed and options so players can verify they share
/// the same settings.
pub fn write_seed_hash(rom: &mut Rom, seed: u64, options: &Options) {
    let (icons, palette) = compute_hash(seed, options);

    // Build sprite data: 5 icons x 2 sprites x 4 bytes = 40 bytes.
    // The copy routine iterates X downward in groups of 8, mapping:
    //   data[32..39] -> OAM[0..7],  data[24..31] -> OAM[32..39],
    //   data[16..23] -> OAM[64..71], data[8..15] -> OAM[96..103],
    //   data[0..7]   -> OAM[128..135]
    // We place icon 0 (topmost) in the highest data group so it lands
    // in the lowest OAM slot (highest sprite priority).
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
        0xBD, DATA_CPU_LO, DATA_CPU_HI,     // LDA $E92D,X
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
    rom.write_range(HOOK_OFFSET, &[0x4C, 0x14, 0xE9]);

    // Skip intro cutscene: hook STA $0736 in title screen init to also set
    // Title_State ($DE) = 6 (IntroSkip). This loads all graphics quickly and
    // jumps straight to the 1P/2P menu, ensuring consistent CHR banks for our
    // hash sprites.
    //
    // Replace: 8D 36 07 (STA $0736) → 20 55 E9 (JSR $E955)
    // At $E955: STA $0736 / LDA #$06 / STA $DE / RTS
    rom.write_range(INTRO_SKIP_HOOK_OFFSET, &[0x20, 0x55, 0xE9]);
    #[rustfmt::skip]
    rom.write_range(INTRO_SKIP_ROUTINE_OFFSET, &[
        0x8D, 0x36, 0x07, // STA $0736  (original instruction)
        0xA9, 0x06,       // LDA #$06
        0x85, 0xDE,       // STA $DE    (Title_State = IntroSkip)
        0x60,             // RTS
    ]);
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

        let mut sprite_data = [0u8; HASH_LENGTH * 8];
        for i in 0..HASH_LENGTH {
            let group = HASH_LENGTH - 1 - i;
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

        // Icon 0 is in group 4 (bytes 32-39), should have Y_START
        assert_eq!(sprite_data[32], Y_START);
        assert_eq!(sprite_data[32 + 3], X_LEFT);
        assert_eq!(sprite_data[32 + 7], X_RIGHT);

        // Icon 4 is in group 0 (bytes 0-7), should have Y_START + 4*Y_SPACING
        assert_eq!(sprite_data[0], Y_START + 4 * Y_SPACING);
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
