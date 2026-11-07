//! An area's enemy/object segment: a page byte, then `[id, x, y]` entries
//! until `$FF` (see `docs/smb3_rom_reference.md`, "Enemy Data").

use crate::randomize::rom_data::{ENEMY_DATA_END, enemy_ptr_to_file_offset};
use crate::rom::Rom;

/// Toad Houses reuse the objects pointer as a house ID and treasure, and the
/// engine swaps in its own object set (PRG030, the `Level_Tileset = 7` check).
const TILESET_TOAD_HOUSE: u8 = 7;

/// One object entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Object {
    /// File offset of the ID byte; X and Y follow it.
    pub(crate) offset: usize,
    pub(crate) id: u8,
    /// Tile column; the screen is `x >> 4` in a horizontal level.
    pub(crate) x: u8,
    /// Tile row.
    pub(crate) y: u8,
}

/// The objects an area loads with, or `None` when its pointer is not into
/// the object data (Toad Houses, bonus rooms with no list).
pub(crate) fn read_objects(rom: &Rom, objects_ptr: u16, tileset: u8) -> Option<Vec<Object>> {
    if tileset == TILESET_TOAD_HOUSE || !(0xC000..0xE00D).contains(&objects_ptr) {
        return None;
    }
    let mut i = enemy_ptr_to_file_offset(objects_ptr) + 1; // skip the page byte
    let mut out = Vec::new();
    while i + 2 < ENEMY_DATA_END && rom.read_byte(i) != 0xFF {
        let id = rom.read_byte(i);
        out.push(Object { offset: i, id, x: rom.read_byte(i + 1), y: rom.read_byte(i + 2) });
        i += 3;
    }
    Some(out)
}
