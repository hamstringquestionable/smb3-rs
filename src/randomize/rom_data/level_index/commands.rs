//! An area's generator commands, decoded as far as the index needs: where
//! each one is, whether it is a junction, and which ones place powerup blocks.
//!
//! A command is `b0 b1 b2 [b3]`: `b0` bits 5-7 are the generator group and
//! bits 0-4 the row, `b1` is screen (high nibble) and column (low). `b2` with a
//! zero high nibble is a fixed-size shape; otherwise a variable-size one whose
//! width the tileset decides.

use crate::randomize::rom_data::LevelDataRegion;
use crate::rom::Rom;

use super::Area;
use super::areas::{command_size, region_for_tileset};

/// Group 1 fixed shapes: the powerup blocks, the same in every tileset.
const GROUP_POWER_BLOCK: u8 = 1;
/// Group 2 fixed shapes 1-6: note and wood blocks, in tilesets where those
/// shapes are blocks at all ([`LevelDataRegion::randomize_note_wood`]).
const GROUP_NOTE_WOOD: u8 = 2;

/// Group 1 shape names, by shape (`b2`).
const POWER_BLOCK_NAMES: [&str; 16] = [
    "Q-block flower",
    "Q-block leaf",
    "Q-block star",
    "Q-block coin/star",
    "Q-block coin",
    "muncher",
    "brick flower",
    "brick leaf",
    "brick star",
    "brick coin/star",
    "brick 10 coins",
    "brick 1-up",
    "brick vine",
    "brick P-switch",
    "invisible coin",
    "invisible 1-up",
];

/// Group 2 shapes 1-6, where they are note and wood blocks.
const NOTE_WOOD_NAMES: [&str; 6] =
    ["note flower", "note leaf", "note star", "wood flower", "wood leaf", "wood star"];

/// One generator command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Command {
    /// File offset of `b0`.
    pub(crate) offset: usize,
    pub(crate) bytes: [u8; 4],
    /// 3, or 4 when the tileset reads an extra byte.
    pub(crate) len: usize,
}

impl Command {
    /// A junction start (`b0 >= $E0`), not a generator.
    pub(crate) fn is_junction(&self) -> bool {
        self.bytes[0] >= 0xE0
    }

    pub(crate) fn group(&self) -> u8 {
        self.bytes[0] >> 5
    }

    pub(crate) fn row(&self) -> u8 {
        self.bytes[0] & 0x1F
    }

    pub(crate) fn screen(&self) -> u8 {
        self.bytes[1] >> 4
    }

    pub(crate) fn column(&self) -> u8 {
        self.bytes[1] & 0x0F
    }

    /// A fixed-size shape's index within its group.
    pub(crate) fn fixed_shape(&self) -> Option<u8> {
        (!self.is_junction() && self.bytes[2] & 0xF0 == 0).then_some(self.bytes[2])
    }
}

/// A command that places one powerup block, and the byte that picks what.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PowerBlock {
    pub(crate) command: Command,
    pub(crate) name: &'static str,
}

impl PowerBlock {
    /// File offset of the shape byte (`b2`), the one a randomizer rewrites.
    pub(crate) fn shape_offset(&self) -> usize {
        self.command.offset + 2
    }
}

/// Every command in `area`, in stream order.
pub(crate) fn read_commands(rom: &Rom, area: &Area) -> Vec<Command> {
    let region = region_for_tileset(area.tileset).expect("a traced area has a region");
    let mut out = Vec::new();
    let mut i = area.header.offset + super::LEVEL_HEADER_SIZE;
    while i < area.terminator {
        let len = command_size(region, rom.read_byte(i), rom.read_byte(i + 2));
        let mut bytes = [0; 4];
        bytes[..len].copy_from_slice(rom.read_range(i, len));
        out.push(Command { offset: i, bytes, len });
        i += len;
    }
    out
}

/// The powerup blocks among `commands`, read with `area`'s tileset.
pub(crate) fn power_blocks(area: &Area, commands: &[Command]) -> Vec<PowerBlock> {
    let region: &LevelDataRegion = region_for_tileset(area.tileset).expect("traced");
    commands
        .iter()
        .filter_map(|&command| {
            let shape = command.fixed_shape()? as usize;
            let name = match command.group() {
                GROUP_POWER_BLOCK => POWER_BLOCK_NAMES[shape],
                GROUP_NOTE_WOOD if region.randomize_note_wood && (1..=6).contains(&shape) => {
                    NOTE_WOOD_NAMES[shape - 1]
                }
                _ => return None,
            };
            Some(PowerBlock { command, name })
        })
        .collect()
}
