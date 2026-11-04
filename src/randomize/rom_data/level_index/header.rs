//! The 9-byte level header, decoded the way `LevelLoad` (PRG030 `$97B7`)
//! reads it. Bit meanings are taken from that routine, not from older notes.

/// Bytes in a level header; the commands start right after it.
pub(crate) const LEVEL_HEADER_SIZE: usize = 9;

/// `GamePlay_TimeStart` (PRG030 `$97A8`): the clock's hundreds digit, by
/// [`LevelHeader::timer_index`]. 0 means no clock.
const GAMEPLAY_TIME_START: usize = 0x3D7B8;

/// `GamePlay_BGM` (PRG030 `$97AC`, read by `LDA $97AC,X` at file `0x3D8DD`):
/// the song queued for each [`LevelHeader::music`] index.
const GAMEPLAY_BGM_LEN: usize = 11;

/// The disassembly's names for `GamePlay_BGM`'s entries, by index.
const BGM_NAMES: [&str; GAMEPLAY_BGM_LEN] = [
    "overworld",
    "underground",
    "underwater",
    "fortress",
    "boss",
    "airship",
    "battle",
    "toad house",
    "athletic",
    "throne room",
    "sky",
];

use crate::rom::Rom;

/// One level header and where it sits in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LevelHeader {
    /// File offset of byte 0.
    pub(crate) offset: usize,
    pub(crate) bytes: [u8; LEVEL_HEADER_SIZE],
}

impl LevelHeader {
    pub(crate) fn read(rom: &Rom, offset: usize) -> Self {
        let mut bytes = [0; LEVEL_HEADER_SIZE];
        bytes.copy_from_slice(rom.read_range(offset, LEVEL_HEADER_SIZE));
        LevelHeader { offset, bytes }
    }

    /// Bytes 0-1: `Level_AltLayout`, the layout a junction switches to.
    pub(crate) fn alt_layout(&self) -> u16 {
        u16::from_le_bytes([self.bytes[0], self.bytes[1]])
    }

    /// Bytes 2-3: `Level_AltObjects`, the enemy set a junction switches to.
    pub(crate) fn alt_objects(&self) -> u16 {
        u16::from_le_bytes([self.bytes[2], self.bytes[3]])
    }

    /// Byte 4 bits 0-3: last screen index, so the width is this plus one.
    pub(crate) fn screens(&self) -> u8 {
        (self.bytes[4] & 0x0F) + 1
    }

    /// Byte 6 bit 4: `Level_7Vertical`.
    pub(crate) fn vertical(&self) -> bool {
        self.bytes[6] & 0x10 != 0
    }

    /// Byte 6 bits 0-3: `Level_AltTileset`, the tileset a junction switches to.
    pub(crate) fn alt_tileset(&self) -> u8 {
        self.bytes[6] & 0x0F
    }

    /// Byte 8 bits 6-7: index into `GamePlay_TimeStart` (300/400/200/none).
    pub(crate) fn timer_index(&self) -> u8 {
        self.bytes[8] >> 6
    }

    /// Byte 8 bits 0-3: index into `GamePlay_BGM`.
    pub(crate) fn music(&self) -> u8 {
        self.bytes[8] & 0x0F
    }

    /// The song's name, for an index `GamePlay_BGM` defines.
    pub(crate) fn music_name(&self) -> Option<&'static str> {
        BGM_NAMES.get(self.music() as usize).copied()
    }

    /// File offset of byte 8, which holds the song and the timer.
    pub(crate) fn music_offset(&self) -> usize {
        self.offset + 8
    }

    /// The level clock in seconds, read through this ROM's own table, or
    /// `None` for no clock.
    pub(crate) fn time(&self, rom: &Rom) -> Option<u16> {
        let hundreds = rom.read_byte(GAMEPLAY_TIME_START + self.timer_index() as usize);
        (hundreds != 0).then(|| hundreds as u16 * 100)
    }

    /// True when the bits `LevelLoad` never reads are clear and the song
    /// index is one `GamePlay_BGM` defines. Every header a vanilla pointer
    /// reaches passes; bytes that are not a header usually fail, which is how
    /// a dead alt pointer gets caught.
    pub(crate) fn is_plausible(&self) -> bool {
        self.bytes[8] & 0x30 == 0 && (self.music() as usize) < GAMEPLAY_BGM_LEN
    }
}
