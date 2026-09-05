//! Turning a generated maze into ROM writes.
//!
//! Everything the maze needs on the ROM side already exists as a mechanism —
//! the arrival tables, `PAD_ENTER` and its position key, the packed completion
//! store. This module is the adapter: it converts a [`GlobalState`] into the
//! specs those mechanisms take, and stamps the one tile the generator decides
//! that nothing else writes.
//!
//! **It does not orchestrate.** The maze's ROM side is several modules deep —
//! pad tiles, the wand gate, foreign locks, the packed completion store, the
//! whistle — and their order matters (everything that changes a map grid has to
//! precede the store, which derives its stencil from the finished grids). That
//! order is stated once, in `randomizer::randomize_inner`, where every other
//! feature's ordering already lives. This module only converts.

use crate::rom::Rom;

use super::super::foreign_locks::ForeignLock;
use super::super::overworld_build::SlotKind;
use super::super::rom_data::{self, PRG012_FILE_BASE, TELEPAD_QUADRANTS, TILE_TELEPAD};
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

/// Stamp the pad tiles onto the ROM's map grids, and compose the metatile they
/// wear.
///
/// The tile is [`TILE_TELEPAD`] (`$DF`), and it is a byte of the pad's own
/// rather than the spade panel it used to share. The playtest that forced the
/// change is in that constant's docs, along with every registry membership the
/// byte needs; the short version is that a player looking at 28 spade panels
/// cannot tell which nine of them teleport.
///
/// Composing the metatile is four bytes of quadrant table and **no CHR at
/// all** — [`TELEPAD_QUADRANTS`] points at patterns the map already draws. The
/// write is idempotent and world-independent (one metatile table serves all
/// eight grids), so it happens once here rather than per pad.
///
/// The cell keeps whatever pointer-table entry it had. That entry becomes
/// unreachable, which is why [`super::roles::pad_sites`] only ever offers
/// Hammer Bro filler slots and bare blanks: the least valuable content on the
/// map, and in the filler case no content at all.
pub(crate) fn stamp_pad_tiles(rom: &mut Rom, state: &GlobalState) {
    let pads = state.pad_edges();
    if pads.is_empty() {
        return;
    }
    for (plane, &pattern) in TELEPAD_QUADRANTS.iter().enumerate() {
        rom.write_byte(PRG012_FILE_BASE + plane * 256 + TILE_TELEPAD as usize, pattern);
    }
    for ((world, (row, col)), _) in pads {
        rom.write_byte(rom_data::map_tile_offset(world, row, col), TILE_TELEPAD);
    }
}

/// Every lock whose fortress lives in another world, in the shape the ROM side
/// takes.
///
/// [`ForeignLock`](super::super::foreign_locks::ForeignLock) is flat
/// `(world, position)` pairs rather than the maze's own types, and that is the
/// right shape rather than a lossy one: the hook keys on the player's map
/// position — `Y` already holds the completion column and `X` the row when
/// `Map_MarkLevelComplete`'s fortress branch is reached — so no fort-id
/// numbering has to be invented on either side.
///
/// Same-world locks are left out: they already work through the fortress FX
/// path, and a foreign-lock row for one would set its bit twice.
pub(crate) fn foreign_locks(state: &GlobalState) -> Vec<ForeignLock> {
    state
        .locks
        .iter()
        .filter(|l| l.is_foreign())
        .filter_map(|l| {
            let fort = l.fort?;
            let pos = state.worlds[fort.world]
                .slots
                .iter()
                .find(|s| s.section == fort.section && s.kind == SlotKind::Fortress)?
                .pos;
            Some(ForeignLock {
                fort_world: fort.world,
                fort_pos: pos,
                lock_world: l.world,
                lock_pos: l.pos,
            })
        })
        .collect()
}
