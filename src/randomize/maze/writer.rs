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

use super::super::lock_keys::LockEntry;
use super::super::overworld_build::SlotKind;
use super::super::rom_data::{self, PRG012_FILE_BASE, TELEPAD_QUADRANTS, TILE_TELEPAD};
use super::super::world_persist::Telepad;
use super::GlobalState;

/// The map object a lock hint wears. The HELP bubble: static, non-marching,
/// entered by nothing, and present in every world already.
const MAPOBJ_HINT: u8 = 0x01;

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

/// Open every lock the maze decided not to install.
///
/// [`MazeLock::fort`](super::MazeLock::fort) documents `None` as "the lock is
/// not installed — the tile is left as open path", and calls that the harmless
/// case so a half-finished assignment degrades to a more open maze rather than
/// an unwinnable one. **That was not true until this existed.**
/// `overworld_writer::grid` stamps `gap_tile` from the *builder's* lock list,
/// unconditionally and before the maze decides anything, so an uninstalled lock
/// shipped as a gate with no key — a permanently sealed gate, which is the one
/// thing the generator must never produce.
///
/// Writing `replace_tile` is the whole of it: that byte is what the builder
/// recorded as "what this cell looks like once the lock is gone", and it is the
/// same byte `Map_RemoveTo_Tiles` would have swapped in.
///
/// Must run **before** `world_persist::apply`, like the other grid writers: the
/// packed completion store derives its stencil from the finished grids, and a
/// lock tile claims a bit that a path tile does not.
pub(crate) fn open_uninstalled_locks(rom: &mut Rom, state: &GlobalState) {
    for lock in state.locks.iter().filter(|l| l.fort.is_none()) {
        rom.write_byte(
            rom_data::map_tile_offset(lock.world, lock.pos.0, lock.pos.1),
            lock.replace_tile,
        );
    }
}

/// Park a hint sprite over every lock whose fortress is in the same world.
///
/// One bit, and deliberately only one: *is the key here or not*. Naming the
/// world would need a per-object tile, and the engine's draw path is ID-driven
/// (`LDA MapObject_Pat1-3,X`, X from the object ID), so a digit means hooking
/// the sprite draw at `$B64C` to override `Sprite_RAM` mid-render — a patch
/// rather than a table write. The bit is what removes the wasted search; the
/// digit only narrows a search the player was going to make anyway.
///
/// # Why a map object and not a tile
///
/// A tile variant would need a `Map_Removable_Tiles` entry, and that table has
/// eight, both tables are adjacent so the first cannot grow without moving the
/// second, and the loop bound is a baked `LDX #$07` — plus our own PRG011
/// mirror. A map object needs none of that; it needs a free slot.
///
/// # The slots
///
/// Nine per world, loaded from ROM by `Map_Init`. Slots 0 and 1 look reserved
/// and are not: `autoscroll::disable_autoscroll` repoints every airship entry
/// away from the Toad-and-King scene, so the HELP token is never read and the
/// airship is never written into slot 1. That is two slots per world, and it is
/// what makes World 8 — four locks, most of them foreign, and a map already
/// carrying two tanks, a battleship and an airship — affordable at all.
///
/// # Marking the LOCAL ones, and why that way round
///
/// A sprite means **the fortress that opens this lock is in this world**;
/// absence means it is somewhere else. Absence carries half the message, so the
/// marked set has to be the one that always fits — and it is the smaller one by
/// a wide margin. The constructive fill makes ~76% of locks cross-world, so
/// local locks average **0.4 per world** (1.0 in World 8) against ~1.4 foreign.
/// Measured over 30 seeds, marking the local ones is never short of slots;
/// marking the foreign ones falls short in 26 world-seeds of 240.
///
/// It is also the better signal of the two. A marked lock says "you can solve
/// this one here", which is something to act on; an unmarked one says "not
/// here", which is the common case and the one worth not wasting time on.
///
/// A local lock that went unmarked for want of a slot would be
/// indistinguishable from a foreign one, so absence would mean two things at
/// once — `lock_hint_slots_are_never_short` is what keeps that honest.
///
/// # What the sprite is allowed to be
///
/// Static, non-marching, non-interactive. HELP is the model and, for now, the
/// implementation: it is drawn, it never moves, and nothing enters it.
pub(crate) fn stamp_lock_hints(rom: &mut Rom, state: &GlobalState) -> usize {
    let mut placed = 0;
    for world in 0..state.worlds.len() {
        // Slots this world can spare, lowest first. Slot 0 holds the HELP
        // bubble, which is decoration now, so it counts as spare.
        let mut spare: Vec<usize> = (0..9)
            .filter(|&slot| {
                let id = rom.read_byte(rom_data::map_obj_slot_offset(
                    rom,
                    rom_data::MAP_OBJ_IDS_MASTER,
                    world,
                    slot,
                ));
                id == 0 || id == MAPOBJ_HINT
            })
            .collect();

        for lock in state.locks.iter().filter(|l| l.world == world) {
            let Some(fort) = lock.fort else { continue };
            if fort.world != lock.world {
                continue; // key is elsewhere; absence says so
            }
            let Some(slot) = spare.pop() else { break };
            rom_data::write_map_sprite(rom, world, slot, lock.pos.0, lock.pos.1, MAPOBJ_HINT);
            placed += 1;
        }
    }
    placed
}

/// **Every** lock in the maze, paired with the fortress that opens it, in the
/// shape the ROM side takes.
///
/// [`LockEntry`](super::super::lock_keys::LockEntry) is flat `(world, position)`
/// pairs rather than the maze's own types, and that is the right shape rather
/// than a lossy one: the hook keys on the player's map position — `Y` already
/// holds the completion column and `X` the row when `Map_MarkLevelComplete`'s
/// fortress branch is reached — so no fort-id numbering has to be invented on
/// either side.
///
/// **Same-world locks are included, and leaving them out was a bug.** This used
/// to emit only `is_foreign()` locks, on the reasoning that the rest "already
/// work through the fortress FX path". They did — but through the *overworld
/// builder's* pairing, not the maze's. [`fill`](super::fill) starts from the
/// builder's assignment and moves by swapping the forts of two locks, so a swap
/// that left both locks in their own worlds was simply discarded on the way to
/// the ROM: measured at 60 seeds, **33.1% of same-world locks (164 of 496, in 59
/// of 60 seeds) were opened by the wrong fortress**, and a fortress that kept a
/// stale local lock while gaining a foreign one opened two.
///
/// The fill's bijection — "every fortress has exactly one lock", the charter's
/// map-legibility rule — only reaches the cartridge if the whole assignment
/// travels together. `lock_keys::assert_one_key_per_lock` is what now says so.
///
/// A lock with no fort is not installed at all (see [`MazeLock::fort`]), so it
/// contributes no entry; today the fill never produces one.
pub(crate) fn lock_keys(state: &GlobalState) -> Vec<LockEntry> {
    state
        .locks
        .iter()
        .filter_map(|l| {
            // No fort means the lock was never installed — an open path tile,
            // not a sealed gate. That is the harmless case and it contributes
            // no entry.
            let fort = l.fort?;
            // A fort that does not resolve to a cell IS the harmful case: the
            // lock would reach the ROM with nothing to open it, which is an
            // unwinnable seed and silent if it were merely skipped.
            let pos = state.worlds[fort.world]
                .slots
                .iter()
                .find(|s| s.section == fort.section && s.kind == SlotKind::Fortress)
                .unwrap_or_else(|| {
                    panic!(
                        "lock at W{} {:?} names fortress section {} in W{}, which is not on \
                         the map — the lock would be permanently sealed",
                        l.world + 1,
                        l.pos,
                        fort.section,
                        fort.world + 1
                    )
                })
                .pos;
            Some(LockEntry {
                key_world: fort.world,
                key_pos: pos,
                target_world: l.world,
                target_pos: l.pos,
            })
        })
        .collect()
}
