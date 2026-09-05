//! The key-assignment fill: which fortress opens which lock, across all eight
//! worlds.
//!
//! This is the mode's headline feature. A lock does not have to sit in its
//! fortress's world — the engine already has a data-only exit for an
//! off-screen lock (`patch_fortress_fx_screen_check`), and a cross-world lock
//! is that case one level up. What it buys is the thing that makes a maze a
//! maze: a gate you cannot open without going somewhere else first.
//!
//! ## Why a swap search and not the charter's forward fill
//!
//! The charter proposed a forward fill: close every lock, walk, and repeatedly
//! assign a frontier gate a key **from the already-reachable set**, so
//! solvability is by construction with no retry.
//!
//! That construction can stall, and on this map it stalls often. Its first step
//! needs a fortress inside the start region with every lock closed — and the
//! per-world builder deliberately puts forts *off* the forced path, so the
//! start region frequently holds none. A stalled forward fill has to fall back
//! to an arbitrary assignment for the remaining gates, which is exactly the
//! retry loop it was meant to avoid.
//!
//! So the fill starts from an assignment that is **already known good** — the
//! per-world builder's, where every lock is opened by a fort in its own world
//! and every world is completable — and moves from there by swapping the forts
//! of two locks, keeping a swap only when the maze is still solvable. Two
//! properties fall out:
//!
//! * **It cannot fail.** The worst case is that no swap is accepted and the
//!   result is the local assignment the builder already guaranteed.
//! * **The bijection is preserved for free.** A swap exchanges two forts
//!   between two locks, so "every fortress has exactly one lock" — the
//!   charter's map-legibility rule, and the reason a world's lock count tells
//!   the player its fort count — holds at every step without being checked.

use rand::Rng;
use rand::seq::SliceRandom;

use super::graph::Knobs;
use super::{FortRef, GlobalState, MazeLock};

/// Exchange the forts of two locks — the fill's only move, and its own undo.
/// The fort/lock bijection survives for free, because a swap trades two forts
/// between two locks rather than handing one out.
///
/// Takes the lock slice rather than the whole state so it stays disjoint from
/// the `state.worlds` borrow the distance objective holds.
fn swap_forts(locks: &mut [MazeLock], a: usize, b: usize) {
    let fa = locks[a].fort;
    locks[a].fort = locks[b].fort;
    locks[b].fort = fa;
}

/// A cross-world lock can only be opened by a fortress whose cell actually
/// turns to rubble: the hook is gated on that tile, and World 8's tanks are
/// sprites over a blanked cell.
fn opens_ok(st: &GlobalState, li: usize) -> bool {
    let lock = &st.locks[li];
    lock.fort.is_none_or(|f| f.world == lock.world || st.crumbling.contains(&f))
}

/// How many swaps the fill proposes per lock. Each proposal costs one global
/// fixpoint, so this is the fill's whole cost model: `locks * PROPOSALS_PER_LOCK`
/// fixpoints, ~17 * 8 = 136 on a normal seed.
const PROPOSALS_PER_LOCK: usize = 8;

/// What the fill did, for the census and the spoiler log.
#[derive(Clone, Debug, Default)]
pub(crate) struct FillReport {
    pub proposed: usize,
    /// Swaps rejected because they made the maze unsolvable.
    pub rejected_unsolvable: usize,
    /// Swaps rejected because they would have given a cross-world lock to a
    /// fortress whose cell never becomes rubble — a lock that could not fire.
    pub rejected_uncrumbling: usize,
    /// Swaps rejected because they moved the objective the wrong way.
    pub rejected_objective: usize,
    pub accepted: usize,
    /// Locks whose fort ended up in another world.
    pub foreign_locks: usize,
    /// Spine distance between a foreign lock and its fort, summed.
    pub foreign_span: usize,
}

/// Reassign which fortress opens which lock, biased by
/// [`Knobs::fort_distance_bias`].
///
/// The maze must already be solvable on entry; it is still solvable on exit.
pub(crate) fn assign_keys<R: Rng>(
    state: &mut GlobalState,
    spine: &[usize],
    knobs: &Knobs,
    rng: &mut R,
) -> FillReport {
    let mut report = FillReport::default();
    if state.locks.len() < 2 {
        return report;
    }

    // Spine position, so "far" means far along the player's route rather than
    // far in ROM order. A key three worlds back is a long walk; a key in the
    // next world is barely a detour.
    // A world off the spine sits at the far end of it: the only way in is a
    // telepad, so a key there is as far from anywhere as the graph gets.
    let mut spine_pos = [spine.len(); 8];
    for (i, &w) in spine.iter().enumerate() {
        spine_pos[w] = i;
    }
    let distance = |lock: &MazeLock, fort: FortRef| -> usize {
        if fort.world == lock.world {
            // Same world: a few points for grid separation, so a local key is
            // not entirely flat. Halved because two tiles apart on one grid is
            // nothing beside a world boundary.
            let pos = state.worlds[fort.world]
                .slots
                .iter()
                .find(|s| s.section == fort.section && s.kind == super::SlotKind::Fortress)
                .map(|s| s.pos)
                .unwrap_or(lock.pos);
            (pos.0.abs_diff(lock.pos.0) + pos.1.abs_diff(lock.pos.1)) / 2
        } else {
            // Crossing a world boundary dominates everything on one grid.
            100 + spine_pos[fort.world].abs_diff(spine_pos[lock.world]) * 10
        }
    };
    let total = |st: &GlobalState| -> usize {
        st.locks.iter().filter_map(|l| l.fort.map(|f| distance(l, f))).sum()
    };

    let bias = knobs.fort_distance_bias.clamp(-1.0, 1.0);
    let mut score = total(state);
    let mut order: Vec<usize> = (0..state.locks.len()).collect();

    for _ in 0..PROPOSALS_PER_LOCK {
        order.shuffle(rng);
        for w in order.chunks(2) {
            let [a, b] = w else { continue };
            let (a, b) = (*a, *b);
            if state.locks[a].fort == state.locks[b].fort {
                continue;
            }
            report.proposed += 1;
            swap_forts(&mut state.locks, a, b);

            let after = total(state);
            // bias 0 accepts any solvable swap — a random walk over the
            // solvable assignments, which is the null model. Otherwise the
            // sign decides which way the objective has to move.
            let wanted = if bias > 0.0 {
                after > score
            } else if bias < 0.0 {
                after < score
            } else {
                true
            };
            // Magnitude is a probability, so the dial is continuous rather
            // than a switch: |bias| 0.5 takes half the improving swaps. At
            // bias 0 every solvable swap is taken, which is the random walk.
            let p = if bias == 0.0 { 1.0 } else { bias.abs() };
            let take = wanted && rng.random_bool(p);
            if !take {
                swap_forts(&mut state.locks, a, b);
                report.rejected_objective += 1;
                continue;
            }
            // Rejecting an uncrumbling fortress here rather than filtering the
            // table later is deliberate — a foreign lock the ROM cannot fire is
            // not a cosmetic problem, it is a lock that never opens.
            if !opens_ok(state, a) || !opens_ok(state, b) {
                swap_forts(&mut state.locks, a, b);
                report.rejected_uncrumbling += 1;
                continue;
            }

            // Two hard gates, and both have to hold or the swap goes back.
            // Solvability is the obvious one. The second is the safety
            // invariant: a swap that hands a start-region lock to a foreign
            // fort makes that world a trap, because a player arriving there
            // for the first time cannot open it from the inside. Only the two
            // worlds whose locks moved can have changed, so this costs two
            // small per-world fixpoints rather than eight.
            let (wa, wb) = (state.locks[a].world, state.locks[b].world);
            let safe =
                state.start_region_escapable(wa) && (wb == wa || state.start_region_escapable(wb));
            if !safe || !state.spheres().solvable {
                swap_forts(&mut state.locks, a, b);
                report.rejected_unsolvable += 1;
                continue;
            }
            report.accepted += 1;
            score = after;
        }
    }

    for lock in &state.locks {
        if let Some(f) = lock.fort
            && f.world != lock.world
        {
            report.foreign_locks += 1;
            report.foreign_span += spine_pos[f.world].abs_diff(spine_pos[lock.world]);
        }
    }
    report
}
