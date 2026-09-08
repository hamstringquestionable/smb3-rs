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

/// Fortress wearing the alternate colour: the lock it opens is in another
/// world. `Map_Removable_Tiles` turns it into rubble `$E3`, same as the
/// default's `$60`.
const TILE_FORT_AWAY_LOCK: u8 = 0xEB;

/// Fortress whose lock is in World 8 — the ones that open the way to the
/// castle. In neither tile registry, so it comes back wearing the completion
/// marker rather than rubble; it still claims a completion bit.
const TILE_FORT_W8_LOCK: u8 = 0x6A;

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

/// Say on each fortress where the lock it opens is.
///
/// The lock hint answers "is this lock's key nearby"; this answers the same
/// question from the other end, at the moment the player is deciding whether a
/// fortress is worth the detour. Three states, and the ROM already has three
/// fortress tiles:
///
/// | tile | means | on beat |
/// |---|---|---|
/// | `$67` | the lock is in this world | rubble `$60` |
/// | `$EB` | the lock is in another world | rubble `$E3` |
/// | `$6A` | the lock is in **World 8** | the M/L completion marker |
///
/// Own world wins, then World 8, then elsewhere — so a World 8 fortress opening
/// a World 8 lock reads `$67`, not `$6A`. Measured over 60 seeds the split is
/// 24% / 59% / 17%, with every world seeing a mix.
///
/// **This costs nothing.** All three tiles exist, all three are already stamped
/// — `overworld_writer::grid` picks among them at random for cosmetic variety —
/// so the only change is that the choice now means something. No new tile, no
/// `Map_Removable_Tiles` entry, no CHR, no 6502.
///
/// `$6A` is the odd one and it is deliberate: it is in neither
/// `Map_Removable_Tiles` nor `Map_Completable_Tiles`, so it never becomes
/// rubble and comes back wearing the completion marker instead. It still claims
/// a completion bit — `overworld_build::is_completion_unsafe` names it, and the
/// threshold check would catch it regardless — so the packed store allocates it
/// one and the beat persists. Different look, same bookkeeping, which is what
/// makes it usable as a third state at all.
///
/// Runs after `write_overworld`, which stamped the random pick, and before
/// `world_persist` derives its stencil. The stencil is unaffected either way:
/// all three tiles claim a bit — and a cell this skips is left exactly as the
/// writer left it, so it cannot drift either.
pub(crate) fn stamp_fort_tiles(rom: &mut Rom, state: &GlobalState) {
    for lock in &state.locks {
        let Some(fort) = lock.fort else { continue };
        let Some(pos) = state.worlds[fort.world]
            .slots
            .iter()
            .find(|s| s.section == fort.section && s.kind == SlotKind::Fortress)
            .map(|s| s.pos)
        else {
            continue; // `lock_keys` is the one that panics on this
        };
        // **Recolour a fortress tile; never create one.** World 8's army
        // sprites are placed on fortress positions
        // (`overworld_writer::sprites::pick_w8_sprite_positions` draws from
        // `wa.fortress`), and the writer deliberately blanks the cell under a
        // sprite so the tank or battleship reads as the content there. Stamping
        // a fortress tile back would undo that and put a fortress under the
        // sprite. Those forts simply go unlabelled — the sprite is the visual,
        // and there is nowhere to say it.
        let offset = rom_data::map_tile_offset(fort.world, pos.0, pos.1);
        if !rom_data::FORTRESS_TILES.contains(&rom.read_byte(offset)) {
            continue;
        }
        let tile = if lock.world == fort.world {
            rom_data::TILE_FORTRESS
        } else if lock.world == rom_data::W8_IDX {
            TILE_FORT_W8_LOCK
        } else {
            TILE_FORT_AWAY_LOCK
        };
        rom.write_byte(offset, tile);
    }
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
