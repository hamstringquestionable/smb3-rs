//! Ending credits: align the per-world "end picture" montage with the
//! randomized world progression order.
//!
//! After Bowser, SMB3 plays a montage (`Ending2_DoEndPic` in PRG024) that
//! shows a hand-drawn scene for each of the 8 worlds, iterating the counter
//! `Ending2_CurWorld` from 0 to 7. Every per-world asset is looked up in a
//! parallel 8-entry table indexed by that counter:
//!
//! - `EndPicByWorld_H/L` — pointer to the world's compressed nametable picture
//! - `EndPic_VRAMStart_H/L` — where on screen the picture is drawn
//! - `Ending2_EndPicSpriteList{H,L,Len}` — the world's foreground sprites
//! - `Ending2_EndPicPatTable2..5` — CHR pattern-table banks for the picture
//! - `PRG024_BE29/BE31` — start/end of the series of graphics-load commands
//!   ($2A..$4C) that stream in the picture tiles
//! - the palette, fired as `Graphics_Queue = CurWorld + $4D` (commands
//!   $4D..$54 in `Video_Upd_Table2`, `EndSeq_World1Pal..World8Pal`)
//!
//! Because the counter *is* the index into every one of these tables, showing
//! the worlds in progression order is a pure permutation: to display internal
//! world `order[p]` at montage position `p`, set `new[p] = old[order[p]]` for
//! each table. No code changes are needed — only data is rewritten.
//!
//! The one thing that *isn't* a permuted pointer is the on-screen "WORLD n"
//! caption: it's a plain background tile baked into each scene's graphics-load
//! command data (drawn outside the mini-map frame), so a pure reorder would
//! leave every scene captioned with its original world number. After permuting,
//! we rewrite each scene's caption digit to its montage position so the world
//! shown first reads "WORLD 1", the second "WORLD 2", and so on.
//!
//! This follows the project's "decide then write" split: the orchestrator
//! decides `order` (from the [`super::world_order`] shuffle), this module
//! performs the mechanical ROM writes.

use rand::Rng;

use crate::rom::Rom;
use super::rom_data::{self, Grid, ROWS, TILE_BOWSER};

// All offsets are file offsets into the PRG024/PRG025 title-screen/endings
// banks (file 0x30010 / 0x32010). Each names an 8-entry table indexed by world
// 0-7. Offsets verified by signature search against SMB3 USA Rev 1.

const BE_SERIES_START: usize = 0x31E39; // PRG024_BE29: series queue start command
const BE_SERIES_END: usize = 0x31E41; //   PRG024_BE31: series queue end command
const PAT_TABLE_2: usize = 0x31F6E; //     Ending2_EndPicPatTable2
const PAT_TABLE_3: usize = 0x31F76; //     Ending2_EndPicPatTable3
const PAT_TABLE_4: usize = 0x31F7E; //     Ending2_EndPicPatTable4
const PAT_TABLE_5: usize = 0x31F86; //     Ending2_EndPicPatTable5
const SPRITE_LIST_H: usize = 0x31F8E; //   Ending2_EndPicSpriteListH
const SPRITE_LIST_L: usize = 0x31F96; //   Ending2_EndPicSpriteListL
const SPRITE_LIST_LEN: usize = 0x31F9E; // Ending2_EndPicSpriteListLen
const ENDPIC_PTR_H: usize = 0x32126; //    EndPicByWorld_H
const ENDPIC_PTR_L: usize = 0x3212E; //    EndPicByWorld_L
const VRAM_START_H: usize = 0x325DA; //    EndPic_VRAMStart_H
const VRAM_START_L: usize = 0x325E2; //    EndPic_VRAMStart_L

/// The thirteen 1-byte-per-world tables that share the montage permutation.
const BYTE_TABLES: [usize; 13] = [
    BE_SERIES_START,
    BE_SERIES_END,
    PAT_TABLE_2,
    PAT_TABLE_3,
    PAT_TABLE_4,
    PAT_TABLE_5,
    SPRITE_LIST_H,
    SPRITE_LIST_L,
    SPRITE_LIST_LEN,
    ENDPIC_PTR_H,
    ENDPIC_PTR_L,
    VRAM_START_H,
    VRAM_START_L,
];

/// Palette pointer table: `Video_Upd_Table2` commands $4D..$54
/// (`EndSeq_World1Pal..World8Pal`), 2 bytes (one little-endian pointer) per
/// world. The palette is fired as `Graphics_Queue = CurWorld + $4D`, so
/// command $4D+p indexes entry `p` here.
const PALETTE_PTR_TABLE: usize = 0x32684;

/// Ending-font digit glyphs: the world-number tile for digit `n` is
/// `DIGIT_TILE_BASE + n`, i.e. $77 = "1" … $7E = "8". Confirmed against the
/// vanilla "WORLD n" captions.
const DIGIT_TILE_BASE: u8 = 0x76;

/// File offset(s) of the "WORLD n" caption digit tile in each world's ending
/// scene, one entry per internal world 0-7. Unlike everything else the montage
/// draws, the caption is a background tile in the scene's graphics-load command
/// data (PRG025), drawn *outside* the mini-map frame — so neither the reorder
/// nor [`render_world_maps`] touches it, and each scene keeps its original
/// number unless we rewrite it. World 5's scene streams its caption twice
/// (initial draw + redraw), so it has two offsets; every other world has one.
/// Offsets verified by signature search for the "WORLD " prefix
/// (`DE F4 EF F1 E3 5C`) against SMB3 USA Rev 1.
const CAPTION_DIGIT_OFFSETS: [&[usize]; 8] = [
    &[0x32D9B],          // World 1
    &[0x32E00],          // World 2
    &[0x32E5E],          // World 3
    &[0x32EAC],          // World 4
    &[0x32EDE, 0x32F18], // World 5 (caption drawn twice)
    &[0x32F39],          // World 6
    &[0x32FE1],          // World 7
    &[0x3305F],          // World 8
];

/// Reorder the ending montage so that position `p` displays internal world
/// `order[p]`. `order` must be a permutation of `0..=7`.
///
/// The identity order is a no-op, so calling this with world-order
/// randomization disabled leaves the ROM byte-identical.
pub fn reorder_world_pictures(rom: &mut Rom, order: &[u8; 8]) {
    debug_assert!(is_permutation(order), "credits order must be a permutation of 0..=7");

    if *order == [0, 1, 2, 3, 4, 5, 6, 7] {
        return;
    }

    for &base in BYTE_TABLES.iter() {
        let mut old = [0u8; 8];
        old.copy_from_slice(rom.read_range(base, 8));
        for (p, &world) in order.iter().enumerate() {
            rom.write_byte(base + p, old[world as usize]);
        }
    }

    // Palette pointers are 2 bytes each.
    let mut old_pal = [0u8; 16];
    old_pal.copy_from_slice(rom.read_range(PALETTE_PTR_TABLE, 16));
    for (p, &world) in order.iter().enumerate() {
        let src = world as usize * 2;
        rom.write_range(PALETTE_PTR_TABLE + p * 2, &old_pal[src..src + 2]);
    }

    // Renumber each scene's "WORLD n" caption to its montage position: the world
    // shown at slot `p` now reads "WORLD p+1". The caption is a static tile the
    // reorder doesn't move, so without this the first-shown world would keep its
    // original number. `p` ranges 0..=7, so the digit is always a single glyph.
    for (p, &world) in order.iter().enumerate() {
        let digit_tile = DIGIT_TILE_BASE + (p as u8 + 1);
        for &off in CAPTION_DIGIT_OFFSETS[world as usize] {
            rom.write_byte(off, digit_tile);
        }
    }
}

/// Build the full 8-world montage order from a play-order progression.
///
/// `progression` is the world sequence the player actually traversed (from
/// [`super::world_order::randomize`]); it may be shorter than 8 when fewer
/// worlds are enabled. Visited worlds come first in play order; any worlds not
/// in the progression are appended in ascending order so the result is always
/// a permutation of `0..=7` and every montage slot shows a real picture.
pub fn order_from_progression(progression: &[u8]) -> [u8; 8] {
    let mut order = [0u8; 8];
    let mut seen = [false; 8];
    let mut n = 0;
    for &w in progression {
        if w < 8 && !seen[w as usize] {
            order[n] = w;
            seen[w as usize] = true;
            n += 1;
        }
    }
    for w in 0..8u8 {
        if !seen[w as usize] {
            order[n] = w;
            n += 1;
        }
    }
    order
}

// --- Rebuilding the pictures from the randomized maps -----------------------
//
// Each ending mini-map is a top-down view of the world drawn as a 16×12 tile
// frame (14×10 interior) in the `EndPic_WorldN` buffer, colored by the per-world
// ending palette + attribute table. It is a near **1:1 grid of the overworld
// map**: the playable overworld is 14 columns wide (columns 0 and `cols-1` are
// the `$02` border), the mini-map interior is also 14 wide, and each map
// metatile maps to one little "mini-tile" at the same cell — the vertical-path
// columns line up exactly. So we redraw the interior straight from the
// *randomized* grid through a metatile→mini-tile lookup ([`MINI_TILE_LUT`],
// derived from the vanilla maps), preserving the map's structure and native look
// while moving the levels/fortresses/pipes to where the builder placed them. The
// frame, sprites, CHR banks, palette, and attribute table are untouched.

/// The eight `EndPic_WorldN` buffers are packed back-to-back in this file
/// region (W1 start through the byte before `EndPic_VRAMStart`). We regenerate
/// all eight and **repack** them here, then repoint `EndPicByWorld_H/L`, so
/// individual buffers may grow or shrink as long as the total fits. Verified
/// against SMB3 USA Rev 1.
const BUFFER_REGION_START: usize = 0x32136;
const BUFFER_REGION_SIZE: usize = 0x4A4; // 1188 bytes across all 8 buffers
/// CPU base of PRG025 (the bank mapped at $C000 during the ending), used to turn
/// a repacked file offset back into the pointer the montage reads.
const PRG025_CPU_BASE: u16 = 0xC000;
const PRG025_FILE_BASE: usize = 0x32010;

const FRAME_W: usize = 16;
const FRAME_H: usize = 12;
const INTERIOR_W: usize = 14;
/// Decompressed picture length the montage loader expects (0xC1); 16×12 frame
/// tiles plus one trailing filler byte the draw loop never reads.
const FRAME_LEN: usize = 0xC1;

// Frame border tiles (identical across all eight vanilla mini-maps).
const CORNER_TL: u8 = 0x70;
const CORNER_TR: u8 = 0x72;
const CORNER_BL: u8 = 0x74;
const CORNER_BR: u8 = 0x75;
const EDGE_TOP: u8 = 0x71; //  top/bottom horizontal edge
const EDGE_SIDE: u8 = 0x73; // left/right vertical edge

/// Overworld metatile → ending mini-map tile. Built by tallying, across all
/// eight vanilla maps, the mini-tile the game draws at each 1:1-aligned cell for
/// a given map metatile (most common wins). Before that tally, every walkable
/// path tile (`VALID_HORZ`/`VALID_VERT`) is seeded to a plain path mini-tile
/// (`0x04`/`0x05`) so path tiles absent from the sampled pages render as paths
/// rather than the `0x3F` terrain/tree fallback. Regenerate with the
/// `credits_render.py`-style tooling if the map tile set changes.
///
/// KNOWN GAP: still a flat table, so context-dependent metatiles (e.g. `0x62`,
/// drawn as brown wall in one row and tan battlement in another) collapse to one
/// mini-tile. A per-world / row-aware pass would tighten the remaining icons.
#[rustfmt::skip]
const MINI_TILE_LUT: [u8; 256] = [
    0x3F, 0x3F, 0x3F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3F, 0x3F, 0x3F,
    0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F,
    0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F,
    0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F,
    0x3F, 0x3F, 0x55, 0x66, 0x03, 0x04, 0x05, 0x06, 0x63, 0x04, 0x07, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F,
    0x0A, 0x65, 0x3F, 0x65, 0x0B, 0x3F, 0x0B, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F,
    0x3F, 0x0F, 0x1C, 0x2A, 0x2E, 0x3F, 0x3F, 0x62, 0x66, 0x0C, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F,
    0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F,
    0x3F, 0x3F, 0x11, 0x12, 0x13, 0x14, 0x6A, 0x6B, 0x17, 0x20, 0x21, 0x3F, 0x23, 0x67, 0x25, 0x26,
    0x27, 0x30, 0x31, 0x3F, 0x33, 0x34, 0x35, 0x3F, 0x37, 0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3F, 0x3F,
    0x3F, 0x3F, 0x44, 0x45, 0x46, 0x3F, 0x48, 0x49, 0x4A, 0x4B, 0x1A, 0x1B, 0x04, 0x26, 0x3F, 0x3F,
    0x05, 0x2D, 0x04, 0x1E, 0x3F, 0x4C, 0x3F, 0x4E, 0x04, 0x4F, 0x22, 0x2F, 0x1F, 0x52, 0x1D, 0x02,
    0x3F, 0x3F, 0x50, 0x51, 0x51, 0x24, 0x3F, 0x3F, 0x3D, 0x3E, 0x6C, 0x6D, 0x6E, 0x6F, 0x53, 0x3F,
    0x59, 0x3F, 0x53, 0x24, 0x24, 0x3F, 0x3F, 0x55, 0x2A, 0x03, 0x04, 0x05, 0x06, 0x63, 0x07, 0x3F,
    0x0A, 0x2B, 0x3F, 0x3F, 0x0B, 0x0E, 0x04, 0x3F, 0x10, 0x55, 0x60, 0x62, 0x3F, 0x3F, 0x3F, 0x3F,
    0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F,
];

/// Bottom-edge filler tile per world — the decorative foreground strip the
/// vanilla vignettes draw below the 9 map rows (water/ground/darkness). Fills
/// the mini-map's 10th interior row so the 9-row map isn't stretched.
const BOTTOM_FILL: [u8; 8] = [0x55, 0x66, 0x67, 0x55, 0x5C, 0x5C, 0x5C, 0x2A];

/// World 8 (Dark Land, the finale) is framed on Bowser's castle rather than a
/// random page, matching the vanilla vignette. Its castle tile maps to a 2×2
/// mini-icon that should sit at these interior columns (as the vanilla art
/// does), so the window is offset to put the castle there.
const DARK_LAND: usize = 7;
const CASTLE_ICON_COL: usize = 11;

/// Redraw every world's ending mini-map from its randomized overworld grid.
///
/// Like the vanilla vignettes, each mini-map shows a **single 16-column map
/// screen** drawn 1:1 — we pick one at random per world (`rng`). Runs after the
/// overworld writer has committed the final map tiles, and **before**
/// [`reorder_world_pictures`] (which permutes the pointers this rewrites). The 8
/// regenerated pictures are repacked into their shared region and
/// `EndPicByWorld_H/L` repointed. Everything else — the frame art, sprites, CHR
/// banks, palette, and attribute table — is untouched, so this cannot corrupt
/// the montage. If the regenerated set somehow overruns the region the montage
/// is left vanilla (no partial write).
pub fn render_world_maps<R: Rng>(rom: &mut Rom, rng: &mut R) {
    let pictures: Vec<Vec<u8>> = (0..8)
        .map(|w| {
            let grid = rom_data::read_tile_grid(rom, w);
            let base = window_base(&grid, w, rng);
            compress(&build_frame(&grid, base, w))
        })
        .collect();

    let total: usize = pictures.iter().map(Vec::len).sum();
    if total > BUFFER_REGION_SIZE {
        return;
    }

    let mut offset = BUFFER_REGION_START;
    for (world, pic) in pictures.iter().enumerate() {
        rom.write_range(offset, pic);
        let cpu = PRG025_CPU_BASE + (offset - PRG025_FILE_BASE) as u16;
        rom.write_byte(ENDPIC_PTR_H + world, (cpu >> 8) as u8);
        rom.write_byte(ENDPIC_PTR_L + world, (cpu & 0xFF) as u8);
        offset += pic.len();
    }
}

/// Choose the leftmost map column shown in a world's mini-map. Most worlds show
/// a random 16-column page (centered in the interior, clamped to the right
/// edge). Dark Land is framed on Bowser's castle so the finale always depicts
/// the castle, matching the vanilla vignette.
fn window_base<R: Rng>(grid: &Grid, world: usize, rng: &mut R) -> usize {
    let max_base = grid.cols - INTERIOR_W;
    if world == DARK_LAND {
        if let Some(castle_col) = find_tile(grid, TILE_BOWSER) {
            return castle_col.saturating_sub(CASTLE_ICON_COL).min(max_base);
        }
        return max_base; // no castle found: show the rightmost page
    }
    let screens = (grid.cols / 16).max(1);
    (rng.random_range(0..screens) * 16 + 1).min(max_base)
}

/// Column of the first cell holding `tile`, scanning row-major.
fn find_tile(grid: &Grid, tile: u8) -> Option<usize> {
    (0..ROWS)
        .flat_map(|r| (0..grid.cols).map(move |c| (r, c)))
        .find(|&(r, c)| grid.get(r, c) == tile)
        .map(|(_, c)| c)
}

/// The HANDTRAP overworld tile: the builder stamps it at a node slot in place of
/// a level number (`overworld_writer`), so on the map it's a node sitting on a
/// path. Its metatile ID is shared with a plain horizontal path variant (it's in
/// `VALID_HORZ`), so a flat [`MINI_TILE_LUT`] entry would draw it as a bare
/// horizontal path — instead we draw a node marker.
const HAND_TRAP_TILE: u8 = 0xE6;

/// Mini-tile drawn for a hand-trap node. The ending art has no dedicated
/// hand-trap icon, so we reuse the spade / bonus-game ring tile. The mini-maps
/// are purely cosmetic, so it's fine that hand-traps and spade games share a
/// marker; the ring's blank margins let the path connect through it, and reusing
/// an existing tile means no CHR is touched.
const HAND_TRAP_MINI: u8 = 0x10;

/// Mini-tile for one map cell: a [`MINI_TILE_LUT`] lookup, except a HANDTRAP node
/// draws the [`HAND_TRAP_MINI`] ring marker.
fn mini_tile_at(grid: &Grid, row: usize, col: usize) -> u8 {
    let tile = grid.get(row, col);
    if tile == HAND_TRAP_TILE {
        return HAND_TRAP_MINI;
    }
    MINI_TILE_LUT[tile as usize]
}

/// Build one world's 0xC1-byte picture: the shared frame border, a 14-column
/// window of the map (starting at column `base`) drawn 1:1 through
/// [`MINI_TILE_LUT`], and the world's decorative bottom-fill strip. The map's 9
/// rows go into interior rows 1..9; the 10th interior row is the world's
/// [`BOTTOM_FILL`] tile (not a stretched map row).
fn build_frame(grid: &Grid, base: usize, world: usize) -> Vec<u8> {
    let mut f = vec![EDGE_TOP; FRAME_LEN];
    // Corners and side borders.
    f[0] = CORNER_TL;
    f[FRAME_W - 1] = CORNER_TR;
    f[(FRAME_H - 1) * FRAME_W] = CORNER_BL;
    f[(FRAME_H - 1) * FRAME_W + FRAME_W - 1] = CORNER_BR;
    for r in 1..FRAME_H - 1 {
        f[r * FRAME_W] = EDGE_SIDE;
        f[r * FRAME_W + FRAME_W - 1] = EDGE_SIDE;
    }

    for r in 0..ROWS {
        for ic in 0..INTERIOR_W {
            let mc = (base + ic).min(grid.cols - 1);
            f[(r + 1) * FRAME_W + (ic + 1)] = mini_tile_at(grid, r, mc);
        }
    }
    // Decorative bottom strip fills the extra 10th interior row.
    let fill = BOTTOM_FILL[world];
    for ic in 0..INTERIOR_W {
        f[(ROWS + 1) * FRAME_W + (ic + 1)] = fill;
    }
    f
}

/// Compress a raw tile stream with the montage's bit-7 run scheme: a pair of
/// equal tiles becomes one `tile | 0x80` byte, a lone tile stays as itself.
/// All input tiles are < 0x80 (every ending-frame tile is).
fn compress(tiles: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(tiles.len());
    let mut i = 0;
    while i < tiles.len() {
        if i + 1 < tiles.len() && tiles[i] == tiles[i + 1] {
            out.push(tiles[i] | 0x80);
            i += 2;
        } else {
            out.push(tiles[i]);
            i += 1;
        }
    }
    out
}

fn is_permutation(order: &[u8; 8]) -> bool {
    let mut seen = [false; 8];
    for &w in order {
        if w >= 8 || seen[w as usize] {
            return false;
        }
        seen[w as usize] = true;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_rom() -> Rom {
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;
        // Seed each per-world table with a recognizable value: table byte for
        // world w = (table_id << 4) | w, so we can verify the permutation.
        for (id, &base) in BYTE_TABLES.iter().enumerate() {
            for w in 0..8u8 {
                data[base + w as usize] = ((id as u8) << 4) | w;
            }
        }
        // Palette pointers: two distinct bytes per world.
        for w in 0..8u8 {
            data[PALETTE_PTR_TABLE + w as usize * 2] = 0xA0 | w;
            data[PALETTE_PTR_TABLE + w as usize * 2 + 1] = 0xB0 | w;
        }
        // Caption digit tiles: seed each world's "WORLD n" glyph to its vanilla
        // number so the renumber can be checked against montage position.
        for (w, offs) in CAPTION_DIGIT_OFFSETS.iter().enumerate() {
            for &off in *offs {
                data[off] = DIGIT_TILE_BASE + (w as u8 + 1);
            }
        }
        Rom::from_bytes_lax(&data, true).unwrap()
    }

    #[test]
    fn identity_order_is_noop() {
        let mut rom = make_test_rom();
        let before: Vec<u8> = rom.read_range(0x31E39, 0x1000).to_vec();
        reorder_world_pictures(&mut rom, &[0, 1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(rom.read_range(0x31E39, 0x1000), before.as_slice());
    }

    #[test]
    fn permutes_every_byte_table() {
        let mut rom = make_test_rom();
        let order = [3u8, 5, 0, 7, 1, 6, 2, 4];
        reorder_world_pictures(&mut rom, &order);
        for (id, &base) in BYTE_TABLES.iter().enumerate() {
            for (p, &world) in order.iter().enumerate() {
                let expect = ((id as u8) << 4) | world;
                assert_eq!(
                    rom.read_byte(base + p),
                    expect,
                    "table {id} position {p} should hold world {world}'s byte",
                );
            }
        }
    }

    #[test]
    fn renumbers_captions_to_montage_position() {
        let mut rom = make_test_rom();
        let order = [3u8, 5, 0, 7, 1, 6, 2, 4];
        reorder_world_pictures(&mut rom, &order);
        // The world shown at slot `p` must read "WORLD p+1" at all of its
        // caption offsets, regardless of its original number.
        for (p, &world) in order.iter().enumerate() {
            let expect = DIGIT_TILE_BASE + (p as u8 + 1);
            for &off in CAPTION_DIGIT_OFFSETS[world as usize] {
                assert_eq!(
                    rom.read_byte(off),
                    expect,
                    "world {world} shown at slot {p} should read digit {}",
                    p + 1,
                );
            }
        }
    }

    #[test]
    fn identity_order_keeps_vanilla_caption_numbers() {
        let mut rom = make_test_rom();
        reorder_world_pictures(&mut rom, &[0, 1, 2, 3, 4, 5, 6, 7]);
        for (w, offs) in CAPTION_DIGIT_OFFSETS.iter().enumerate() {
            for &off in *offs {
                assert_eq!(rom.read_byte(off), DIGIT_TILE_BASE + (w as u8 + 1));
            }
        }
    }

    #[test]
    fn permutes_palette_pointers_as_words() {
        let mut rom = make_test_rom();
        let order = [7u8, 6, 5, 4, 3, 2, 1, 0];
        reorder_world_pictures(&mut rom, &order);
        for (p, &world) in order.iter().enumerate() {
            assert_eq!(rom.read_byte(PALETTE_PTR_TABLE + p * 2), 0xA0 | world);
            assert_eq!(rom.read_byte(PALETTE_PTR_TABLE + p * 2 + 1), 0xB0 | world);
        }
    }

    #[test]
    fn dark_land_finale_preserved_for_full_progression() {
        // world_order always makes Dark Land (7) the last visited world, so the
        // montage finale (position 7) must still be world 8's castle picture.
        let order = order_from_progression(&[2, 5, 0, 3, 1, 6, 4, 7]);
        assert_eq!(order[7], 7);
        assert!(is_permutation(&order));
    }

    #[test]
    fn handtrap_renders_as_node_marker() {
        let g = Grid { tiles: vec![vec![0x45, HAND_TRAP_TILE]], cols: 2, eights_are_wild: false };
        // A hand-trap draws the ring marker regardless of its neighbors.
        assert_eq!(mini_tile_at(&g, 0, 1), HAND_TRAP_MINI);
        // A non-hand-trap tile is a plain LUT lookup.
        assert_eq!(mini_tile_at(&g, 0, 0), MINI_TILE_LUT[0x45]);
    }

    #[test]
    fn compress_decompress_round_trips() {
        // Every ending-frame tile is < 0x80, so a pure round trip is lossless.
        let raw: Vec<u8> = (0..0xC1u32).map(|i| (i % 0x40) as u8).collect();
        let comp = compress(&raw);
        let mut out = Vec::new();
        for &b in &comp {
            if b & 0x80 != 0 {
                out.push(b & 0x7F);
                out.push(b & 0x7F);
            } else {
                out.push(b);
            }
        }
        assert_eq!(out, raw);
    }

    #[test]
    fn mini_tile_lut_maps_known_metatiles() {
        // Every entry is a valid < 0x80 mini-tile (bit 7 is the compression flag).
        assert!(MINI_TILE_LUT.iter().all(|&t| t < 0x80));
        // Background/unmapped metatiles fall back to terrain.
        assert_eq!(MINI_TILE_LUT[0xFF], 0x3F);
        // Known map tiles resolve to their mini-tiles: $46 vertical path → $05,
        // $45 horizontal path → $04, and the $67 fortress metatile maps to a
        // distinct marker tile rather than plain terrain.
        assert_eq!(MINI_TILE_LUT[0x46], 0x05);
        assert_eq!(MINI_TILE_LUT[0x45], 0x04);
        assert_ne!(MINI_TILE_LUT[0x67], 0x3F);
    }

    #[test]
    fn short_progression_appends_unvisited_worlds() {
        // world_count = 3 → visited [start.., 7]; the rest fill in ascending.
        let order = order_from_progression(&[4, 2, 5, 7]);
        assert_eq!(&order[..4], &[4, 2, 5, 7]);
        assert!(is_permutation(&order));
        // The four unvisited worlds trail in ascending order.
        assert_eq!(&order[4..], &[0, 1, 3, 6]);
    }
}
