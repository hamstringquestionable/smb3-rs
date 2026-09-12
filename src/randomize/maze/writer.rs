//! Turning a generated maze into the shapes the ROM side takes.
//!
//! Everything the maze needs on the ROM side already exists as a mechanism —
//! the arrival tables, `PAD_ENTER` and its position key, the packed completion
//! store. This module is the adapter: it converts a [`GlobalState`] into the
//! specs those mechanisms take.
//!
//! **It writes almost nothing itself.** The maze's map edits — pad tiles,
//! uninstalled locks, the fortress hint tiles — are model edits made by
//! [`super::stamp_into`] before `overworld_writer` runs, so they reach the ROM
//! in the writer's own single pass. What is left here is one metatile
//! definition, which is not a grid edit and belongs to no world.
//!
//! **It does not orchestrate.** The order the maze's ROM-side modules install
//! in is stated once, in `randomizer::randomize_inner`, where every other
//! feature's ordering already lives.

use crate::rom::Rom;

use super::super::rom_data::{PRG012_FILE_BASE, TELEPAD_QUADRANTS, TILE_TELEPAD};
use super::super::world_persist::Telepad;
use super::GlobalState;

/// Every pad, as the spec `world_persist::apply` takes.
///
/// Arrival ids are assigned by position in this list, which is what makes the
/// pad key tables and the arrival tables agree without a pairing table: row `i`
/// holds where pad `i` stands, and where pad `i` sends you.
pub(crate) fn telepad_specs(state: &GlobalState) -> Vec<Telepad> {
    state
        .pad_edges()
        .into_iter()
        .map(|((world, src_pos), (dest_world, dest_pos))| Telepad {
            world: world as u8,
            dest_world: dest_world as u8,
            dest_pos,
            src_pos,
        })
        .collect()
}

/// Compose the metatile the pads wear.
///
/// The tile is [`TILE_TELEPAD`] (`$DF`), a byte of the pad's own rather than
/// the spade panel it used to share. The playtest that forced the change is in
/// that constant's docs, along with every registry membership the byte needs;
/// the short version is that a player looking at 28 spade panels cannot tell
/// which nine of them teleport.
///
/// Composing it is four bytes of quadrant table and **no CHR at all** —
/// [`TELEPAD_QUADRANTS`] points at patterns the map already draws. One metatile
/// table serves all eight grids, so this happens once rather than per pad.
///
/// Placing the tile is [`super::stamp_into`]'s job, not this one's: it goes on
/// the grid as a model edit and the overworld writer stamps it like any other
/// tile.
pub(crate) fn install_pad_metatile(rom: &mut Rom, state: &GlobalState) {
    if state.pad_edges().is_empty() {
        return;
    }
    for (plane, &pattern) in TELEPAD_QUADRANTS.iter().enumerate() {
        rom.write_byte(PRG012_FILE_BASE + plane * 256 + TILE_TELEPAD as usize, pattern);
    }
}
