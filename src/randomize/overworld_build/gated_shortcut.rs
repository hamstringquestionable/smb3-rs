//! Compound choice-shaping move: the **gated shortcut** (issue #125), run after
//! the knob-built terrain + forts + locks + spare pipes, over the complete map.
//! On a still-LINEAR world (one in-band route) it adds a content-skipping pipe —
//! a dominating shortcut the spare-pipe pass would veto — and makes it *fair* by
//! forcing a fort's cost onto the shortcut, so taking it means beating that fort
//! first: a real "long way, or beat the fort and pipe" decision.
//!
//! ## The gating lock: re-run `place_locks`, don't relocate
//!
//! The key move is HOW the shortcut gets gated. The lock pass (`place_locks`)
//! already knows how to place a fort's lock on a route fork to create choice —
//! its per-section golden hunt does exactly that, with the full hard-rule / gate
//! / progression context. So the gated shortcut just **re-runs `place_locks`**
//! on the pipe-augmented world: the hunt sees the fork the pipe opened and gates
//! it, the lock *born* on the fork with full context.
//!
//! An earlier version hand-rolled a weaker "relocate one fort's lock onto the
//! fork afterward" and lost ~4pp of choice — a lock dragged onto a fork after
//! the fact can't match one the hunt placed there from the start (it inherits a
//! chokepoint it must give up, searches a tiny bounded set, and misses the
//! hard-rule context). Reusing the real hunt is both simpler and far stronger.
//!
//! ## Budget-safe + affordable
//!
//! Re-running `place_locks` re-places the SAME number of locks (one per fort),
//! so it never grows the FX budget (≤4 locks/world, one per fort section), and
//! the move spends at most the one pipe the spare-pipe pass reserved. Each
//! candidate pipe is measured on a clone and committed only if it strictly
//! raises the in-band route count, stays completable (a monotone fort-beating
//! fixpoint reaches the goal), and doesn't newly open the goal at start. The
//! re-run is kept cheap by the flat-bitset reachability walk (`walk_reachable`)
//! and a trimmed re-hunt choice-guard (see [`REHUNT_CHOICE_GUARD_TRIES`]).

use super::*;

use std::sync::atomic::{AtomicU64, Ordering};

use super::knobs::LockScoring;
use super::locks::place_locks;
use super::route_choice::{C1_FLOOR, DEFAULT_SLACK, in_band_count, measure_counts};
use super::types::{BuiltWorld, LockAssignment, SlotKind, stamp_slots};

/// Metrics for "how often the gated shortcut comes into play" (issue #125 /
/// census `test_gated_shortcut_moves`). `ELIGIBLE` = linear worlds that entered the
/// loop (had a fort to gate with); `APPLIED` = worlds where at least one
/// choice-shaping move was committed. Relaxed atomics: a single add per built
/// world, negligible in production, read only by the census.
pub(crate) static GATED_SHORTCUT_ELIGIBLE: AtomicU64 = AtomicU64::new(0);
pub(crate) static GATED_SHORTCUT_APPLIED: AtomicU64 = AtomicU64::new(0);

/// Reset the metric counters (census calls this before a sweep).
#[cfg(test)]
pub(crate) fn reset_metrics() {
    GATED_SHORTCUT_ELIGIBLE.store(0, Ordering::Relaxed);
    GATED_SHORTCUT_APPLIED.store(0, Ordering::Relaxed);
}

/// Rounds of the shaping loop (a successful move usually makes the world
/// choiceful in one) and shortcut pipes scanned per round. Small: each candidate
/// re-runs `place_locks` (heavy) and the loop runs only on linear worlds, so the
/// budget stays inside the sub-100ms WASM ceiling (guarded by `test_build_time`).
const MAX_ROUNDS: usize = 1;
const MAX_PIPES: usize = 2;

/// Run the unified choice-shaping loop on `built`. `spare_budget` is how many
/// pipe pairs the gated-shortcut move may spend (the spare-pipe pass reserved
/// them); returns how many it used (the caller places the rest as ordinary
/// spares). The re-hunt move spends no pipes, so this fires even at
/// `spare_budget == 0` (pipe-less worlds).
pub(super) fn place_gated_shortcut<R: Rng>(
    built: &mut BuiltWorld,
    spare_budget: usize,
    reserved: &HashSet<Pos>,
    lock_knobs: &LockScoring,
    start_pos: Option<Pos>,
    target_pos: Option<Pos>,
    rng: &mut R,
) -> usize {
    let mut pipes_used = 0;
    let mut counted_eligible = false;
    let mut counted_applied = false;

    for _ in 0..MAX_ROUNDS {
        // Counts-only linearity gate: the loop never reads route paths, so the
        // cheap narrow-band measure suffices (and every choiceful world pays
        // only this before breaking).
        let rc = measure_counts(built, DEFAULT_SLACK);
        let base = in_band_count(&rc);
        if !rc.reachable || base >= 2 {
            break; // choiceful — done
        }
        // C1-floor guard target: the shortcut's added pipe may not price the
        // cheapest route below the floor (or, on an already-deficient world,
        // below where it stands) — the same rule the spare-pipe pass enforces.
        let c1_target = rc.best_cost.min(C1_FLOOR);
        let fort_count = built.slots.iter().filter(|s| s.kind == SlotKind::Fortress).count();
        if fort_count == 0 {
            break; // no fort whose lock could gate a fork
        }
        if !counted_eligible {
            GATED_SHORTCUT_ELIGIBLE.fetch_add(1, Ordering::Relaxed);
            counted_eligible = true;
        }
        let goal_open_before = goal_open(built, start_pos, target_pos);
        // Keep the world's goal gate through a re-hunt (the C1-floor backstop /
        // secret-safe retry rely on it). None when the goal isn't gated.
        let gate_section = built.locks.iter().find(|l| l.blocks_target).map(|l| l.fort_section);

        // Best move this round: (world, in_band, used a pipe). A candidate is
        // kept only if it strictly beats the current best AND passes the goal /
        // completability guards (checked last — most candidates never reach it).
        let mut best: Option<(BuiltWorld, usize, bool)> = None;
        let consider = |world: BuiltWorld, used_pipe: bool, best: &mut Option<(BuiltWorld, usize, bool)>| {
            let rc2 = measure_counts(&world, DEFAULT_SLACK);
            let after = in_band_count(&rc2);
            if after > base
                && rc2.best_cost >= c1_target
                && best.as_ref().is_none_or(|(_, n, _)| after > *n)
                && (goal_open_before || !goal_open(&world, start_pos, target_pos))
                && completable(&world, start_pos, target_pos)
            {
                *best = Some((world, after, used_pipe));
            }
        };

        // Gated shortcut (one pipe): add a content-skipping pipe, then let the
        // re-hunt gate the shortcut fork it creates.
        if spare_budget - pipes_used >= 1 {
            for (a, b) in shortcut_pipe_candidates(built, reserved, start_pos) {
                let mut w = built.clone();
                add_pipe(&mut w, a, b);
                w.locks = rehunt_locks(&w, fort_count, lock_knobs, gate_section, start_pos, target_pos, rng);
                consider(w, true, &mut best);
            }
        }

        match best {
            Some((world, _, used_pipe)) => {
                *built = world;
                if used_pipe {
                    pipes_used += 1;
                }
                if !counted_applied {
                    GATED_SHORTCUT_APPLIED.fetch_add(1, Ordering::Relaxed);
                    counted_applied = true;
                }
            }
            None => break, // no improving move
        }
    }
    pipes_used
}

/// `choice_guard_tries` used inside a re-hunt. The full placement sweeps 8
/// top-scored candidates per section for its choice guard; the re-hunt only
/// needs the golden hunt (always measured) to gate the shortcut fork, so it
/// trims the guard sweep — the dominant per-section cost — to stay in budget.
const REHUNT_CHOICE_GUARD_TRIES: usize = 2;

/// Re-run the existing lock placement on `world` — the reused golden hunt. Same
/// fort count of locks out, so the FX budget is untouched; the hunt sees any
/// added pipe and gates the fork it opens. Runs with a trimmed choice-guard (see
/// [`REHUNT_CHOICE_GUARD_TRIES`]) to keep the re-run affordable.
fn rehunt_locks<R: Rng>(
    world: &BuiltWorld,
    fort_count: usize,
    lock_knobs: &LockScoring,
    gate_section: Option<usize>,
    start_pos: Option<Pos>,
    target_pos: Option<Pos>,
    rng: &mut R,
) -> Vec<LockAssignment> {
    let mut knobs = *lock_knobs;
    knobs.choice_guard_tries = REHUNT_CHOICE_GUARD_TRIES;
    place_locks(
        &world.grid,
        &world.pipe_pairs,
        start_pos,
        target_pos,
        &world.slots,
        fort_count,
        &knobs,
        gate_section,
        false, // force_safe
        world.world_idx,
        rng,
    )
}

/// Candidate shortcut pipes: HammerBro-filler endpoint pairs that skip content
/// (levels whose route-distance sits strictly between them), widest skip first
/// — the pairs most likely to create a dominating shortcut worth gating.
fn shortcut_pipe_candidates(
    built: &BuiltWorld,
    reserved: &HashSet<Pos>,
    start_pos: Option<Pos>,
) -> Vec<(Pos, Pos)> {
    let dist = walk_map(&built.grid, &built.pipe_pairs, start_pos, built.world_idx).distances;
    let endpoints: Vec<Pos> = built
        .slots
        .iter()
        .filter(|s| s.kind == SlotKind::HammerBro && !reserved.contains(&s.pos))
        .map(|s| s.pos)
        .filter(|p| dist.contains_key(p))
        .collect();
    let level_d: Vec<usize> = built
        .slots
        .iter()
        .filter(|s| s.kind == SlotKind::Level)
        .filter_map(|s| dist.get(&s.pos).copied())
        .collect();
    let mut pairs: Vec<(Pos, Pos, usize, usize)> = Vec::new();
    for i in 0..endpoints.len() {
        for j in (i + 1)..endpoints.len() {
            let (a, b) = (endpoints[i], endpoints[j]);
            let (da, db) = (dist[&a], dist[&b]);
            let (lo, hi) = (da.min(db), da.max(db));
            if hi - lo < 2 {
                continue;
            }
            let skipped = level_d.iter().filter(|&&d| d > lo && d < hi).count();
            if skipped == 0 {
                continue;
            }
            pairs.push((a, b, skipped, hi - lo));
        }
    }
    pairs.sort_by_key(|&(a, b, skipped, jump)| {
        (std::cmp::Reverse(skipped), std::cmp::Reverse(jump), a, b)
    });
    pairs.truncate(MAX_PIPES);
    pairs.into_iter().map(|(a, b, _, _)| (a, b)).collect()
}

/// Stamp a pipe pair `(a, b)` onto `built`: mark both endpoints as pipe tiles /
/// slots and add the teleport edge. Both must be HammerBro filler slots.
fn add_pipe(built: &mut BuiltWorld, a: Pos, b: Pos) {
    built.grid.set(a.0, a.1, TILE_PIPE);
    built.grid.set(b.0, b.1, TILE_PIPE);
    for s in built.slots.iter_mut() {
        if s.pos == a || s.pos == b {
            s.kind = SlotKind::Pipe;
        }
    }
    built.pipe_pairs.push((a, b));
}

/// Whether the goal is reachable from start with EVERY lock closed (no fort
/// beaten) — the goal is "open at start", not gated by any fort. A move that
/// flips this from false to true weakens the world's door-and-key structure, so
/// the accept gate rejects it.
fn goal_open(built: &BuiltWorld, start_pos: Option<Pos>, target_pos: Option<Pos>) -> bool {
    let Some(target) = target_pos else { return false };
    let mut g = built.grid.clone();
    stamp_slots(&mut g, &built.slots);
    for l in &built.locks {
        g.set(l.pos.0, l.pos.1, l.gap_tile);
    }
    walk_reachable(&g, &built.pipe_pairs, start_pos, built.world_idx).contains(target)
}

/// Monotone completability: beat every fort reachable in the current lock state
/// (opening its lock), repeat to a fixpoint, then check the goal is reachable.
/// `place_locks` already enforces this per section, but a re-hunt on a
/// pipe-augmented world is measured independently, so the guard stays.
fn completable(built: &BuiltWorld, start_pos: Option<Pos>, target_pos: Option<Pos>) -> bool {
    let Some(target) = target_pos else { return true };
    let mut base = built.grid.clone();
    stamp_slots(&mut base, &built.slots);
    let forts: Vec<(usize, Pos)> = built
        .slots
        .iter()
        .filter(|s| s.kind == SlotKind::Fortress)
        .map(|s| (s.section, s.pos))
        .collect();

    let mut open: HashSet<usize> = HashSet::new();
    loop {
        let mut g = base.clone();
        for l in &built.locks {
            let tile = if open.contains(&l.fort_section) {
                l.replace_tile
            } else {
                l.gap_tile
            };
            g.set(l.pos.0, l.pos.1, tile);
        }
        let reachable = walk_reachable(&g, &built.pipe_pairs, start_pos, built.world_idx);
        let mut progressed = false;
        for &(section, pos) in &forts {
            if !open.contains(&section) && reachable.contains(pos) {
                open.insert(section);
                progressed = true;
            }
        }
        if !progressed {
            return reachable.contains(target);
        }
    }
}
