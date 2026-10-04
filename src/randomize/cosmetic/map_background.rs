//! The world map the web app draws behind its page.
//!
//! The app shows one world's map, read from the player's own ROM, as its page
//! background. Where that map lives (the screen-major grid, the metatile
//! tables, which palette set each world uses) is all ROM knowledge, so it is
//! resolved here and the page is handed nothing but CHR tile numbers and NES
//! colours to draw.
//!
//! The map is the ROM as supplied, never a randomized one: the page draws it
//! before anything has been generated.

use serde::Serialize;

use crate::randomize::rom_data::{MAP_TILE_GRIDS, PRG012_FILE_BASE, ROWS, read_tile_grid};
use crate::rom::Rom;

/// `Map_Tile_ColorSets` (`prg012.asm:125`, `$A41D`): which row of
/// [`PALSET_MAPS`] each world's map tiles use.
const MAP_TILE_COLOR_SETS: usize = PRG012_FILE_BASE + 0x41D;

/// `PalSet_Maps` (`prg027.asm:1458`): the world-map palette sets, 16 bytes
/// (four palettes, one per palette page) per set.
const PALSET_MAPS: usize = 0x36BE2;

/// The metatile quadrant tables at PRG012 `$A000`: four 256-byte tables,
/// indexed by tile byte, in the order upper-left, lower-left, upper-right,
/// lower-right.
const METATILE_QUADRANTS: usize = PRG012_FILE_BASE;

/// The first CHR tile of BG pages `$14`-`$17`, the map's background art. A
/// quadrant byte `q` is CHR tile `MAP_BG_CHR + q`; tiles `$00`-`$7F` are
/// animated through other pages at run time, and this is their first frame.
const MAP_BG_CHR: u16 = 0x14 * 64;

/// One world's map, ready to draw: an 8×8 tile grid of CHR tile numbers, each
/// with its palette page, plus the world's four map palettes.
#[derive(Serialize, Debug)]
pub(crate) struct MapBackground {
    /// Width in 8×8 tiles: two per map square.
    cols: usize,
    /// Absolute CHR tile number per 8×8 tile, row-major.
    tiles: Vec<u16>,
    /// Palette page (0-3) per 8×8 tile, row-major: the map square's tile byte's
    /// high two bits.
    pages: Vec<u8>,
    /// The world's four map palettes as NES colour indices; entry 0 of each is
    /// the shared backdrop.
    palettes: [[u8; 4]; 4],
}

/// How many worlds have a map to draw.
// Reason: the page's only caller is `wasm.rs`, which a native build never
// compiles, so there it looks dead.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn world_count() -> usize {
    MAP_TILE_GRIDS.len()
}

/// World `world`'s map (0-based) as [`MapBackground`].
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn map_background(rom: &Rom, world: usize) -> MapBackground {
    let grid = read_tile_grid(rom, world);
    let cols = grid.cols * 2;
    let mut tiles = vec![0u16; cols * ROWS * 2];
    let mut pages = vec![0u8; cols * ROWS * 2];
    for row in 0..ROWS {
        for col in 0..grid.cols {
            let tile = grid.get(row, col) as usize;
            // (dx, dy) of each quadrant table, in table order.
            for (q, (dx, dy)) in [(0, 0), (0, 1), (1, 0), (1, 1)].into_iter().enumerate() {
                let quadrant = rom.read_byte(METATILE_QUADRANTS + q * 256 + tile);
                let i = (row * 2 + dy) * cols + col * 2 + dx;
                tiles[i] = MAP_BG_CHR + quadrant as u16;
                pages[i] = (tile >> 6) as u8;
            }
        }
    }

    let set = rom.read_byte(MAP_TILE_COLOR_SETS + world) as usize;
    let mut palettes = [[0u8; 4]; 4];
    for (page, palette) in palettes.iter_mut().enumerate() {
        palette.copy_from_slice(rom.read_range(PALSET_MAPS + set * 16 + page * 4, 4));
    }

    MapBackground { cols, tiles, pages, palettes }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROM_PATH: &str = "roms/Super Mario Bros. 3 (USA) (Rev 1).nes";

    fn rom() -> Option<Rom> {
        let bytes = std::fs::read(ROM_PATH).ok()?;
        Some(Rom::from_bytes_lax(&bytes, false).unwrap())
    }

    /// The two table offsets read the bytes the disassembly lists there.
    #[test]
    fn offsets_land_on_their_tables() {
        let Some(rom) = rom() else {
            eprintln!("SKIP: requires the ROM, which is not included in the repo");
            return;
        };
        // prg012.asm:125
        assert_eq!(
            rom.read_range(MAP_TILE_COLOR_SETS, 9),
            &[0x00, 0x01, 0x00, 0x03, 0x04, 0x05, 0x06, 0x07, 0x02]
        );
        // prg027.asm:1460, the World 1/3 set
        assert_eq!(
            rom.read_range(PALSET_MAPS, 16),
            &[
                0x0F, 0x0F, 0x30, 0x3C, 0x0F, 0x36, 0x27, 0x37, 0x0F, 0x21, 0x2A, 0x37, 0x0F, 0x30,
                0x16, 0x37
            ]
        );
    }

    /// World 1's fortress square comes out as the CHR tiles the web app's own
    /// fortress icon names (`icons.js`, `HINT_TILES`), in page 1 with World 1's
    /// colours.
    #[test]
    fn world1_fortress_matches_the_icon_tiles() {
        let Some(rom) = rom() else {
            eprintln!("SKIP: requires the ROM, which is not included in the repo");
            return;
        };
        let map = map_background(&rom, 0);
        assert_eq!(map.cols, MAP_TILE_GRIDS[0].columns * 2);
        assert_eq!(map.tiles.len(), map.cols * ROWS * 2);
        // `$67` sits at World 1 (4,6) (`rom_map.py --tile 0x67`).
        let (row, col) = (4, 6);
        let at = |dx: usize, dy: usize| (row * 2 + dy) * map.cols + col * 2 + dx;
        assert_eq!(
            [map.tiles[at(0, 0)], map.tiles[at(1, 0)], map.tiles[at(0, 1)], map.tiles[at(1, 1)]],
            [1476, 1478, 1477, 1479]
        );
        assert_eq!(map.pages[at(0, 0)], 1);
        assert_eq!(map.palettes[1], [0x0F, 0x36, 0x27, 0x37]);
    }

    /// Every world reads, and World 8 uses its own palette set.
    #[test]
    fn every_world_reads() {
        let Some(rom) = rom() else {
            eprintln!("SKIP: requires the ROM, which is not included in the repo");
            return;
        };
        assert_eq!(world_count(), MAP_TILE_GRIDS.len());
        for (world, info) in MAP_TILE_GRIDS.iter().enumerate() {
            let map = map_background(&rom, world);
            assert_eq!(map.cols, info.columns * 2, "W{}", world + 1);
            assert!(map.pages.iter().all(|&p| p < 4));
        }
        assert_eq!(map_background(&rom, 7).palettes[3], [0x0F, 0x35, 0x25, 0x17]);
    }
}
