//! Double-digit metatile + frozen-metatile ROM patches.

use super::*;

/// File offset where the world-map CHR data begins.
const CHR_BASE: usize = 0x40010;

/// Patch metatile LL quadrant for double-digit level tiles (0x0D–0x15).
///
/// Vanilla tiles 0x0D–0x15 have a blank LL (CHR 0xBE = solid fill). We write
/// a custom CHR tile with a "1" tens digit into an unused slot, then point
/// the LL quadrant of tiles 0x0D–0x15 at it.
///
/// CHR tile 0xFD (page 0x17, local 0x3D) holds the letter 'Z' in vanilla.
/// The only place 'Z' appears on the world map is the "Warp Zone" screen,
/// which is reachable only by using a warp whistle. With the default
/// `--no-whistles` configuration the Warp Zone is unreachable, so the 'Z'
/// glyph never renders and we can safely repurpose its CHR slot.
///
/// Future improvement: rename the screen to "Warp World" (or any Z-free
/// 4-letter alt like "Warp Land" / "Warp Pipe") and the 'Z' tile becomes
/// permanently free, even with `--keep-whistles`. This requires locating
/// the screen's text data first — neither ASCII "Zone" nor a linear-alphabet
/// tile encoding [Z, O, N, E] = [0xFD, ?, ?, ?] was found by simple search,
/// so the popup builds the string by code or uses an interleaved encoding.
/// See memory/double_digit_chr_tile.md for the full investigation log.
///
/// Earlier picks failed: 0xCB is the LR of metatile 0x0B (vanilla "level 9"
/// digit), and 0xCC is the vertical-bar tile used by the popup window border
/// kit ("MARIO x N" / "WORLD N" overlay). Most other tiles in pages 0x16/0x17
/// are popup-font letters/digits.
///
/// CHR page 0x17 covers tile IDs 0xC0–0xFF and is stable (no MMC3 mid-frame
/// bank swapping); pages 0x16/0x17 are loaded only as the world-map BG bank
/// (R1 = 0x16) and never as a sprite or level CHR source.
pub(crate) fn patch_double_digit_metatiles(rom: &mut Rom) {
    // Metatile quadrant tables at PRG012 base: UL(256) LL(256) UR(256) LR(256).
    const METATILE_LL_BASE: usize = rom_data::PRG012_FILE_BASE + 256; // 0x18110

    // Overwrite CHR tile 0xFD with our custom "1" digit.
    // CHR page 0x17 covers tile IDs 0xC0–0xFF; tile 0xFD = local index 0x3D.
    const CHR_PAGE_17: usize = CHR_BASE + 0x17 * 0x400;
    const TILE_FD_OFFSET: usize = CHR_PAGE_17 + 0x3D * 16;
    // Arrow shape (cols 2–5) + "1" serif (col 6 row 1) + right border (col 7 = color 2).
    #[rustfmt::skip]
    const DIGIT_1_LL: [u8; 16] = [
        0x7E, 0x7C, 0x7E, 0x7E, 0x7E, 0x7E, 0x7F, 0x00, // plane 0
        0xA1, 0xB3, 0xB9, 0xBD, 0xB9, 0xB1, 0x80, 0xFF, // plane 1
    ];
    rom.write_range(TILE_FD_OFFSET, &DIGIT_1_LL);

    // Point LL of tiles 0x0D–0x15 (levels 10–19) at CHR tile 0xFD.
    for tile_id in 0x0Du8..=0x15 {
        rom.write_byte(METATILE_LL_BASE + tile_id as usize, 0xFD);
    }
}

/// Freeze metatile 0x6A's CHR animation so it can serve as a static fortress tile.
///
/// The overworld NMI handler rotates MMC3 R0 (2KB BG bank) through pages
/// (0x14+0x15), (0x70+0x71), (0x72+0x73), (0x74+0x75) to animate tiles $00-$7F.
/// Metatile 0x6A's quadrants (CHR 0x64-0x67) fall in this animated range, so
/// it visibly swaps between frames.
///
/// Copy the base (page 0x15) pixel data for CHR tiles 0x64-0x67 into the
/// same positions in pages 0x71, 0x73, 0x75 so every frame renders identically.
/// Metatile 0x6A is the only metatile referencing CHR 0x64-0x67, so no other
/// tile is affected.
pub(crate) fn patch_metatile_6a_freeze(rom: &mut Rom) {
    const BASE_PAGE: usize = 0x15;
    const ANIM_PAGES: [usize; 3] = [0x71, 0x73, 0x75];
    // Tiles 0x64-0x67 live in page 0x15 at local indices 0x24-0x27.
    for local_idx in 0x24..=0x27usize {
        let base_off = CHR_BASE + BASE_PAGE * 0x400 + local_idx * 16;
        let base_tile: [u8; 16] = core::array::from_fn(|i| rom.read_byte(base_off + i));
        for page in ANIM_PAGES {
            let off = CHR_BASE + page * 0x400 + local_idx * 16;
            rom.write_range(off, &base_tile);
        }
    }
}
