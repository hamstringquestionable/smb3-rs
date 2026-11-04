//! Following a level from its pointer table entry through every area a
//! junction can switch to.
//!
//! An entry names the main area (layout, objects, tileset). Each header's
//! alt pointers name the next area, which `Level_JctInit` (PRG026) loads when
//! Mario takes a pipe or door: layout and objects from bytes 0-3, tileset from
//! byte 6. Area N's objects therefore come from area N-1's header, not its own.

use crate::randomize::rom_data::{
    LEVEL_DATA_REGIONS, LevelDataRegion, LevelEntry, PAGE_A000_BY_TILESET, layout_file_offset,
};
use crate::rom::Rom;

use super::{LEVEL_HEADER_SIZE, LevelHeader};

/// One area of a level: its header, the objects and tileset it is loaded
/// with, and the extent of its generator commands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Area {
    pub(crate) header: LevelHeader,
    pub(crate) layout_ptr: u16,
    pub(crate) objects_ptr: u16,
    pub(crate) tileset: u8,
    /// File offset of the `$FF` that ends the commands. They run from
    /// `header.offset + 9` up to here.
    pub(crate) terminator: usize,
}

/// Why a level's area chain stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChainEnd {
    /// The last area's alt layout is `$0000`: its junctions leave the level.
    NoAlt,
    /// The last area's alt pointer names an area already in the chain (the
    /// usual two-way pipe), by its index.
    LoopsTo(usize),
    /// The alt pointer is non-zero but does not lead to a level area. The
    /// area's exits never follow it (3-9's bonus room names tileset 0).
    Dead(DeadAlt),
}

/// What is wrong with a dead alt pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeadAlt {
    /// Tileset 0 is the world map's, or the value is past the bank table.
    Tileset(u8),
    /// The tileset's bank holds no level data.
    NoRegion(u8),
    /// The bytes there are not a level header.
    NotAHeader(usize),
    /// No `$FF` ends the commands before the bank does.
    Unterminated(usize),
}

/// The region whose level data lives in `tileset`'s `$A000` bank, which
/// decides how wide each command is.
pub(super) fn region_for_tileset(tileset: u8) -> Option<&'static LevelDataRegion> {
    let bank = *PAGE_A000_BY_TILESET.get(tileset as usize)?;
    LEVEL_DATA_REGIONS.iter().find(|r| (r.start - 0x10) / 0x2000 == bank)
}

/// Read the area that `layout`/`objects`/`tileset` load, or say why they
/// do not name one.
pub(crate) fn read_area(
    rom: &Rom,
    layout: u16,
    objects: u16,
    tileset: u8,
) -> Result<Area, DeadAlt> {
    if tileset == 0 {
        return Err(DeadAlt::Tileset(tileset));
    }
    let offset = layout_file_offset(layout, tileset).ok_or(DeadAlt::Tileset(tileset))?;
    let region = region_for_tileset(tileset).ok_or(DeadAlt::NoRegion(tileset))?;
    let header = LevelHeader::read(rom, offset);
    if !header.is_plausible() {
        return Err(DeadAlt::NotAHeader(offset));
    }
    // Step by command size to the `$FF`, never past the end of the bank.
    let bank_end = (offset - 0x10) / 0x2000 * 0x2000 + 0x2010;
    let mut i = offset + LEVEL_HEADER_SIZE;
    loop {
        if i >= bank_end {
            return Err(DeadAlt::Unterminated(offset));
        }
        if rom.read_byte(i) == 0xFF {
            break;
        }
        if i + 3 > bank_end {
            return Err(DeadAlt::Unterminated(offset));
        }
        i += region.command_size(rom.read_byte(i), rom.read_byte(i + 2));
    }
    Ok(Area { header, layout_ptr: layout, objects_ptr: objects, tileset, terminator: i })
}

/// A level traced from its entry: every area, main first, and why it stops.
pub(crate) type Trace = (Vec<Area>, ChainEnd);

/// Every area `entry` reaches, main area first, and why the chain stops.
/// `Err` when the entry itself does not name a level area.
pub(crate) fn trace(rom: &Rom, entry: &LevelEntry) -> Result<Trace, DeadAlt> {
    let layout = u16::from_le_bytes([entry.lay_lo, entry.lay_hi]);
    let objects = u16::from_le_bytes([entry.obj_lo, entry.obj_hi]);
    let mut areas = vec![read_area(rom, layout, objects, entry.tileset)?];
    loop {
        let last = &areas[areas.len() - 1].header;
        if last.alt_layout() == 0 {
            return Ok((areas, ChainEnd::NoAlt));
        }
        let next = match read_area(rom, last.alt_layout(), last.alt_objects(), last.alt_tileset()) {
            Ok(next) => next,
            Err(dead) => return Ok((areas, ChainEnd::Dead(dead))),
        };
        if let Some(i) = areas.iter().position(|a| a.header.offset == next.header.offset) {
            return Ok((areas, ChainEnd::LoopsTo(i)));
        }
        areas.push(next);
    }
}
