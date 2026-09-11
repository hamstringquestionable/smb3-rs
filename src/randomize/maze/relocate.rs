//! Move a fortress into another world, and a level back the other way.
//!
//! ## What it is for
//!
//! The per-world builder places **one lock per fortress, in that fortress's own
//! world** (`overworld_build::locks`), so every world shows the player exactly
//! as many fortresses as it shows locks. The key-assignment fill already breaks
//! the *dependency* — 76% of locks end up opened by a fortress somewhere else —
//! but it never moves a node, so the **counts** still match and the map still
//! reads as eight self-contained worlds with the keys shuffled between them.
//!
//! This pass breaks the counts. It exchanges a fortress slot with a level slot
//! in another world, which leaves a world holding more locks than fortresses
//! and another holding more fortresses than locks. The fort/lock bijection is
//! untouched — every fortress still opens exactly one lock — so what is lost is
//! only the *deduction* "count the locks and you know the fort count", and the
//! hint tiles already say what that deduction used to (`LockHint` on the
//! fortress, the world number on the lock at `HintMode::Full`).
//!
//! It is a **perception** change and is deliberately small: one or two
//! exchanges a seed, which is enough to stop the 1:1 reading without turning
//! the fortress distribution into noise.
//!
//! ## Why the same-sphere rule makes this exact
//!
//! The move preserves the fixpoint **by construction**, the same discipline the
//! constructive fill uses — there is no accept test and no retry, because there
//! is nothing that can fail.
//!
//! [`GlobalState::spheres`] is a fixpoint over *positions*: round 0's reach
//! depends on terrain and every lock shut, so it cannot depend on what occupies
//! a slot. Say fortress `F` sits at `a` and a level sits at `b`, and both are
//! first reached in round `s`. Exchange them, **carrying `F`'s identity to
//! `b`** — its `FortRef` moves, so the lock it opens follows it. Then by
//! induction on the round:
//!
//! * reach in round 0 is unchanged (it reads terrain, not slots);
//! * if reach is unchanged through round `r`, the set of fortresses beaten in
//!   round `r` is unchanged too — `F` was beaten at `s` before and is beaten at
//!   `s` now, because `b` is reached at `s`, and no other fortress moved;
//! * so the same locks open, so reach in round `r + 1` is unchanged.
//!
//! Every sphere, the spoiler log, `solvable`, and which locks are sealable come
//! out identical. What *does* change is where the player has to walk to collect
//! a key, which is the point.
//!
//! Two relaxations follow from the same monotonicity and are **not** taken
//! here, recorded so the next reader does not have to re-derive them:
//!
//! * **Into an earlier sphere** is also free — the key arrives sooner and
//!   reachability only grows — but it collapses spheres and makes the maze
//!   shallower, which is the wrong direction.
//! * **Into a later sphere** is where actual maze depth would come from, and it
//!   is the only one that needs an accept test (one `spheres().solvable` per
//!   proposal, cheap beside the fill's ~136). It is a different feature with a
//!   different argument to make; this one is about what the map says.
//!
//! ## Where it runs
//!
//! Inside `generate`'s deal closure, **after the pads and before the fill**.
//! Pads change reachability, so the sphere index has to be read with them in
//! place; and the fill has to see the final fortress positions, since it draws
//! each key from the fortresses reachable at that moment. Running here also
//! keeps the fill's own fallback honest — it reverts to the builder's local
//! pairing, and the re-keying below keeps that pairing solvable.

use std::collections::HashMap;

use rand::Rng;
use rand::seq::IndexedRandom;

use super::super::overworld_build::SlotKind;
use super::super::rom_data::W8_IDX;
use super::walk::MazePos;
use super::{FortRef, GlobalState};

/// What one exchange moved, for the census and the spoiler log.
// Reason: production needs only that the exchange happened; all three fields
// are the census's and the invariant test's readout of WHICH fortress went
// where, and those are their only readers.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct Relocation {
    /// Where the fortress was, and which world it left.
    pub from: MazePos,
    /// Where it went, and the world that gained it.
    pub to: MazePos,
    /// The sphere both ends sit in — equal by construction.
    pub sphere: usize,
}

/// What the pass did.
#[derive(Clone, Debug, Default)]
pub(crate) struct RelocateReport {
    /// Exchanges rolled for this seed: [`MIN_SWAPS`]..=[`MAX_SWAPS`].
    pub wanted: usize,
    pub moves: Vec<Relocation>,
    /// Eligible (fortress, level) pairs before the first exchange — the supply.
    /// Zero means the pass could do nothing, which a census should notice
    /// rather than read as "it chose not to".
    // Reason: a census output, and the census is its only reader.
    #[allow(dead_code)]
    pub pairs: usize,
}

impl RelocateReport {
    pub(crate) fn done(&self) -> usize {
        self.moves.len()
    }
}

/// Fewest exchanges a seed makes.
pub(crate) const MIN_SWAPS: usize = 1;

/// Most exchanges a seed makes.
///
/// Small on purpose. Each exchange decouples the lock and fortress counts of
/// **two** worlds, so two already touches half the map; past that the fortress
/// distribution stops looking like a map someone laid out and starts looking
/// like a shuffle, which is the thing `overworld_build` spends its whole
/// shaping loop avoiding.
pub(crate) const MAX_SWAPS: usize = 2;

/// The next free fortress id in a world.
///
/// Ids only have to be unique within their world — they pair a fortress with
/// its lock and carry no ordering (`overworld_build::forts`) — so counting up
/// from the highest in use is enough. It leaves holes in the numbering when a
/// world loses a fortress, which the writer handles: `assign_pool` walks
/// `0..section_count` and *looks up* the slot rather than assuming one exists.
fn next_fort_id(state: &GlobalState, world: usize) -> usize {
    state.worlds[world]
        .slots
        .iter()
        .filter(|s| s.kind == SlotKind::Fortress)
        .map(|s| s.section + 1)
        .max()
        .unwrap_or(0)
}

/// Exchange one fortress with one level, carrying the fortress's identity.
///
/// The two slot records swap worlds *and* positions, so every flag a level slot
/// carries (`is_troll_pipe`, `is_hand_trap`) travels with the level rather than
/// being applied to whatever lands on its old cell.
fn exchange(state: &mut GlobalState, fort: MazePos, level: MazePos) {
    let (fw, fpos) = fort;
    let (lw, lpos) = level;
    debug_assert_ne!(fw, lw, "an exchange inside one world moves no fortress between worlds");

    let fi = state.worlds[fw]
        .slots
        .iter()
        .position(|s| s.kind == SlotKind::Fortress && s.pos == fpos)
        .expect("the fortress came from this world's slot list");
    let mut fort_slot = state.worlds[fw].slots.remove(fi);

    let li = state.worlds[lw]
        .slots
        .iter()
        .position(|s| s.kind == SlotKind::Level && s.pos == lpos)
        .expect("the level came from this world's slot list");
    let mut level_slot = state.worlds[lw].slots.remove(li);

    // Re-key BEFORE the fortress takes its new id, so `was` still names it.
    let was = FortRef { world: fw, section: fort_slot.section };
    let now = FortRef { world: lw, section: next_fort_id(state, lw) };
    for lock in state.locks.iter_mut() {
        if lock.fort == Some(was) {
            lock.fort = Some(now);
        }
    }

    fort_slot.pos = lpos;
    fort_slot.section = now.section;
    level_slot.pos = fpos;
    // A level slot's `section` is always 0 and means nothing (every phase but
    // `forts` writes it that way), so nothing has to be renumbered here.
    state.worlds[lw].slots.push(fort_slot);
    state.worlds[fw].slots.push(level_slot);
}

/// Move one or two fortresses into other worlds, keeping the fixpoint exact.
///
/// Consumes RNG, so it belongs inside the deal — a redeal gets a different
/// distribution along with a different maze.
pub(crate) fn relocate_forts<R: Rng>(state: &mut GlobalState, rng: &mut R) -> RelocateReport {
    // The sphere index is invariant under every exchange this pass makes (see
    // the module docs), so it is read once and reused rather than recomputed
    // between exchanges.
    let spheres = state.spheres();
    let sphere_of: HashMap<MazePos, usize> = spheres
        .spheres
        .iter()
        .enumerate()
        .flat_map(|(i, s)| s.reached.iter().map(move |&p| (p, i)))
        .collect();

    // Two worlds are excluded on both sides.
    //
    // **Off-spine worlds**, because their content is bonus — `spheres` does not
    // count their fortresses against solvability — so moving a required
    // fortress into one would make it unrequired, and pulling one out would
    // hand the maze a fortress it never had to beat.
    //
    // **World 8**, because this is a perception change and W8 is the one world
    // where it would not be one. Its four locks include the dealt bridge spans
    // to the castle (`capacity::BRIDGES_OUT_WEIGHTS` tunes how many are out,
    // and each one drags another fortress onto the mandatory path), and the
    // wand gate stands on the same approach. Moving a fortress off that
    // corridor, or onto it, changes the endgame rather than how the map reads.
    // W8 keeping 4 and 4 on every seed is a tell the mode accepts.
    let sited = |state: &GlobalState, kind: SlotKind| -> Vec<(MazePos, usize)> {
        state
            .worlds
            .iter()
            .filter(|w| state.in_maze[w.world_idx] && w.world_idx != W8_IDX)
            .flat_map(|w| {
                w.slots.iter().filter(|s| s.kind == kind).map(move |s| (w.world_idx, s.pos))
            })
            .filter_map(|p| sphere_of.get(&p).map(|&s| (p, s)))
            .collect()
    };

    let mut forts = sited(state, SlotKind::Fortress);
    let mut levels = sited(state, SlotKind::Level);

    let pairs =
        |forts: &[(MazePos, usize)], levels: &[(MazePos, usize)]| -> Vec<(MazePos, MazePos)> {
            forts
                .iter()
                .flat_map(|&((fw, fpos), fs)| {
                    levels
                        .iter()
                        .filter(move |&&((lw, _), ls)| lw != fw && ls == fs)
                        .map(move |&(lpos, _)| ((fw, fpos), lpos))
                })
                .collect()
        };

    let mut candidates = pairs(&forts, &levels);
    let mut report = RelocateReport {
        wanted: rng.random_range(MIN_SWAPS..=MAX_SWAPS),
        pairs: candidates.len(),
        moves: Vec::new(),
    };

    while report.done() < report.wanted {
        let Some(&(fort, level)) = candidates.choose(rng) else {
            break;
        };
        let sphere = sphere_of[&fort];
        exchange(state, fort, level);
        // Both cells are spent: the fortress is where the level was and vice
        // versa, so leaving either in the pools would let the second exchange
        // undo the first.
        forts.retain(|&(p, _)| p != fort);
        levels.retain(|&(p, _)| p != level);
        report.moves.push(Relocation { from: fort, to: level, sphere });
        candidates = pairs(&forts, &levels);
    }

    report
}

/// Every fortress id in a world, sorted.
///
/// Ids have to stay unique within a world — the writer looks a fortress up by
/// `(world, id)` — and a relocation is the only thing that ever mints a new one,
/// so the test that checks it lives beside the code that could break it.
#[cfg(test)]
pub(crate) fn fort_ids(state: &GlobalState, world: usize) -> Vec<usize> {
    let mut ids: Vec<usize> = state.worlds[world]
        .slots
        .iter()
        .filter(|s| s.kind == SlotKind::Fortress)
        .map(|s| s.section)
        .collect();
    ids.sort_unstable();
    ids
}
