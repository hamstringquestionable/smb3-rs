//! Unified choice-shaping phase (issue #125): one measured loop with a
//! **compound-move vocabulary**, replacing the siloed single-object measured
//! sub-passes. Runs after the knob-built terrain + forts + locks + spare pipes,
//! over the complete map, and reshapes a still-LINEAR world into a choiceful one
//! by adding cost to the cheap route's exclusive stretch — the tool a parameter,
//! per the issue's "the four passes already share one decision core."
//!
//! ## Move vocabulary
//!
//! - **Golden lock** (no pipe) — relocate an off-route fortress's lock onto a
//!   tile the cheap route crosses but a near-optimal also-ran does not, so the
//!   cheap route now costs that fort's +5 and the also-ran becomes a real
//!   alternative. The only choice tool available to pipe-less worlds (World 1
//!   has a zero pipe budget).
//! - **Gated shortcut** (pipe + lock) — add a content-skipping pipe (a
//!   dominating shortcut the spare-pipe pass would veto) and gate it the same
//!   way, so taking the shortcut means beating an off-path fort first.
//!
//! Both are the same core — *price a route against a balancing target by
//! relocating an off-both-routes fort's lock onto their exclusive stretch*
//! ([`try_gate`]) — differing only in whether a pipe first creates the priced
//! route. Each round measures every candidate and commits the single best.
//!
//! ## Why relocation, not a new lock
//!
//! The ROM budget is fixed on both axes a naive additive move would grow: the
//! per-world pipe count is capped by the catalog's pipe pool, and the fortress
//! FX table holds at most 4 locks/world with exactly ONE lock per fort section
//! (the writer folds each lock into its fort's FX batch). A census of 500 seeds
//! found ~0% of linear worlds have a *lockless* off-route fort — every fort is
//! already locked — so a gated shortcut cannot add a lock. It instead **moves**
//! an off-route fort's existing lock: lock count stays == fort count, the FX
//! writer is untouched, and a fort that was gating dead nodes gates a real
//! choice.
//!
//! ## Safety
//!
//! Every move is applied to a clone and committed only if (a) the in-band route
//! count strictly rises, (b) the world stays completable (a monotone
//! fort-beating fixpoint reaches the goal), and (c) it doesn't newly open the
//! goal at start. A pipe only adds reachability; the sole new stranding risk is
//! the relocated lock's tile, which the completability fixpoint checks directly.

use super::*;

use std::sync::atomic::{AtomicU64, Ordering};

use super::route_choice::{
    DEFAULT_SLACK, SHAPING_SLACK, analyze_route_choice, in_band_count, measure_counts,
    rescue_targets, walk_edge_mids,
};
use super::types::{BuiltWorld, SlotKind, stamp_slots};

/// Metrics for "how often the compound move comes into play" (issue #125 /
/// census `test_compound_moves`). `ELIGIBLE` = linear worlds that entered the
/// loop with an off-route locked fort to spend; `APPLIED` = worlds where at
/// least one choice-shaping move was committed. Relaxed atomics: a single add
/// per built world, negligible in production, read only by the census.
pub(crate) static COMPOUND_ELIGIBLE: AtomicU64 = AtomicU64::new(0);
pub(crate) static COMPOUND_APPLIED: AtomicU64 = AtomicU64::new(0);

/// Reset the metric counters (census calls this before a sweep).
#[cfg(test)]
pub(crate) fn reset_metrics() {
    COMPOUND_ELIGIBLE.store(0, Ordering::Relaxed);
    COMPOUND_APPLIED.store(0, Ordering::Relaxed);
}

/// Rounds of the shaping loop (a successful move usually makes the world
/// choiceful in one), candidate off-route forts examined, shortcut pipes
/// scanned, and golden lock sites per target. All small: the loop runs only on
/// linear worlds but still measures per candidate, so the budget stays inside
/// the sub-100ms WASM ceiling (guarded by `test_build_time`).
const MAX_ROUNDS: usize = 3;
const MAX_PIPES: usize = 4;
const MAX_FORTS: usize = 2;
const MAX_SITES: usize = 2;

/// Run the unified choice-shaping loop on `built`. `spare_budget` is how many
/// pipe pairs the gated-shortcut move may spend (the spare-pipe pass reserved
/// them); returns how many it used (the caller places the rest as ordinary
/// spares). Golden-lock moves spend no pipes, so this fires even at
/// `spare_budget == 0` (pipe-less worlds).
pub(super) fn shape_choice<R: Rng>(
    built: &mut BuiltWorld,
    spare_budget: usize,
    reserved: &HashSet<Pos>,
    start_pos: Option<Pos>,
    target_pos: Option<Pos>,
    rng: &mut R,
) -> usize {
    let mut pipes_used = 0;
    let mut counted_eligible = false;
    let mut counted_applied = false;

    for _ in 0..MAX_ROUNDS {
        let rc = analyze_route_choice(built, SHAPING_SLACK);
        let base = in_band_count(&rc);
        if !rc.reachable || base >= 2 {
            break; // choiceful — done
        }
        let Some(cheap) = rc.routes.first().cloned() else { break };
        // Forts we may spend: off the cheap route (beating one is real extra
        // cost), carrying a lock that isn't the world's goal gate.
        let forts = off_route_locked_forts(built, &cheap.path, rng);
        if forts.is_empty() {
            break; // nothing to gate with
        }
        if !counted_eligible {
            COMPOUND_ELIGIBLE.fetch_add(1, Ordering::Relaxed);
            counted_eligible = true;
        }
        let goal_open_before = goal_open(built, start_pos, target_pos);

        // Best move this round: (world, in_band, relocated section, used a pipe).
        let mut best: Option<(BuiltWorld, usize, usize, bool)> = None;
        let keep = |cand: Option<(BuiltWorld, usize, usize)>,
                        used_pipe: bool,
                        best: &mut Option<(BuiltWorld, usize, usize, bool)>| {
            if let Some((w, n, sec)) = cand
                && best.as_ref().is_none_or(|&(_, bn, _, _)| n > bn)
            {
                *best = Some((w, n, sec, used_pipe));
            }
        };

        // Golden-lock moves (no pipe): gate the cheap route against each rescue
        // target (an out-of-band parallel or a dominated detour).
        for target in rescue_targets(&rc) {
            let cand = try_gate(
                built, &cheap.path, &target.path, &forts, goal_open_before, base, start_pos,
                target_pos,
            );
            keep(cand, false, &mut best);
        }

        // Gated-shortcut moves (one pipe): add a content-skipping pipe, then
        // gate the shortcut it creates against the old cheap route.
        if spare_budget - pipes_used >= 1 {
            for (a, b) in shortcut_pipe_candidates(built, reserved, start_pos) {
                let mut piped = built.clone();
                add_pipe(&mut piped, a, b);
                let rc_p = analyze_route_choice(&piped, SHAPING_SLACK);
                let Some(shortcut) = rc_p.routes.first().cloned() else { continue };
                let piped_forts = off_route_locked_forts(&piped, &shortcut.path, rng);
                let cand = try_gate(
                    &piped, &shortcut.path, &cheap.path, &piped_forts, goal_open_before, base,
                    start_pos, target_pos,
                );
                keep(cand, true, &mut best);
            }
        }

        match best {
            Some((mut world, _, section, used_pipe)) => {
                finalize_lock_flags(&mut world, section, start_pos, target_pos);
                *built = world;
                if used_pipe {
                    pipes_used += 1;
                }
                if !counted_applied {
                    COMPOUND_APPLIED.fetch_add(1, Ordering::Relaxed);
                    counted_applied = true;
                }
            }
            None => break, // no improving move
        }
    }
    pipes_used
}

/// Off-route fortresses whose lock we may relocate: a fort NOT on `route`
/// (beating it is genuine extra cost) carrying a lock that is NOT the world's
/// goal gate (moving the gate could open the goal). Shuffled so which fort is
/// spent varies, then capped.
fn off_route_locked_forts<R: Rng>(
    built: &BuiltWorld,
    route: &[Pos],
    rng: &mut R,
) -> Vec<(usize, Pos)> {
    let on_route: HashSet<Pos> = route.iter().copied().collect();
    let locked: HashSet<usize> = built
        .locks
        .iter()
        .filter(|l| !l.blocks_target)
        .map(|l| l.fort_section)
        .collect();
    let mut forts: Vec<(usize, Pos)> = built
        .slots
        .iter()
        .filter(|s| s.kind == SlotKind::Fortress && !on_route.contains(&s.pos))
        .filter(|s| locked.contains(&s.section))
        .map(|s| (s.section, s.pos))
        .collect();
    forts.shuffle(rng);
    forts.truncate(MAX_FORTS);
    forts
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

/// The shared gating core: charge the `priced` route a fort's +5 by relocating
/// an off-both-routes fort's lock onto a **golden site** — a walk-edge mid-tile
/// the priced route crosses but the balancing `target` does not. Returns the
/// best resulting `(world, in_band, section)` that strictly beats `base`, stays
/// completable, and doesn't newly open the goal. `base_world` is what the lock
/// is relocated on (already carrying the pipe for a gated shortcut).
// Reason: eight distinct inputs (world, the two routes, fort pool, goal state,
// baseline, anchors) with no cohesive sub-bundle — grouping would obscure, not
// clarify. Same call shape as the other builder passes.
#[allow(clippy::too_many_arguments)]
fn try_gate(
    base_world: &BuiltWorld,
    priced: &[Pos],
    target: &[Pos],
    forts: &[(usize, Pos)],
    goal_open_before: bool,
    base: usize,
    start_pos: Option<Pos>,
    target_pos: Option<Pos>,
) -> Option<(BuiltWorld, usize, usize)> {
    let priced_nodes: HashSet<Pos> = priced.iter().copied().collect();
    let target_nodes: HashSet<Pos> = target.iter().copied().collect();
    // Golden sites: mid-tiles of the priced route's exclusive walk edges.
    let target_mids = walk_edge_mids(target);
    let mut sites: Vec<Pos> = walk_edge_mids(priced)
        .difference(&target_mids)
        .copied()
        .filter(|&t| t.0 != 7 && t.0 != 8) // conservative: skip row-7/8 completion-bit tiles
        .filter(|&t| lockable_tile_at(&base_world.grid, t))
        // Never stack onto another fort's lock tile (route-model + FX collision).
        .filter(|&t| !base_world.locks.iter().any(|l| l.pos == t))
        .collect();
    if sites.is_empty() {
        return None;
    }
    sites.sort_unstable();
    sites.truncate(MAX_SITES);

    let mut best: Option<(BuiltWorld, usize, usize)> = None;
    for &(section, fort_pos) in forts {
        // The gating fort must be off BOTH routes: on the priced route it would
        // be paid for anyway; on the target route both routes pay it and it
        // stops differentiating them.
        if priced_nodes.contains(&fort_pos) || target_nodes.contains(&fort_pos) {
            continue;
        }
        for &site in &sites {
            let mut trial = base_world.clone();
            if !relocate_lock(&mut trial, section, site) {
                continue;
            }
            // Cheap choice measure first, then the (more expensive) goal-open and
            // completability checks only on a would-be new best.
            let after = in_band_count(&measure_counts(&trial, DEFAULT_SLACK));
            if after > base
                && best.as_ref().is_none_or(|&(_, n, _)| after > n)
                && (goal_open_before || !goal_open(&trial, start_pos, target_pos))
                && completable(&trial, start_pos, target_pos)
            {
                best = Some((trial, after, section));
            }
        }
    }
    best
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

/// Whether `t` holds a lockable path tile on `grid`.
fn lockable_tile_at(grid: &Grid, t: Pos) -> bool {
    LOCKABLE_TILES.contains(&grid.get(t.0, t.1))
}

/// Move fort `section`'s existing lock onto tile `site` (the shortcut's gate).
/// Returns false if that section has no lock or `site` isn't lockable — the
/// caller skips the candidate.
fn relocate_lock(built: &mut BuiltWorld, section: usize, site: Pos) -> bool {
    let tile = built.grid.get(site.0, site.1);
    if !LOCKABLE_TILES.contains(&tile) {
        return false;
    }
    let Some(lock) = built.locks.iter_mut().find(|l| l.fort_section == section) else {
        return false;
    };
    lock.pos = site;
    lock.replace_tile = tile;
    lock.gap_tile = gap_tile_for(tile);
    // secret_exit_safe / blocks_target are recomputed for the winning move by
    // finalize_lock_flags (found by section); they carry the old lock's values
    // through the trial, which only the accepted move ever reads.
    true
}

/// Whether the goal is reachable from start with EVERY lock closed (no fort
/// beaten) — i.e. the goal is "open at start", not gated by any fort. A
/// relocation that flips this from false to true weakens the world's
/// door-and-key structure, so the accept gate rejects it.
fn goal_open(built: &BuiltWorld, start_pos: Option<Pos>, target_pos: Option<Pos>) -> bool {
    let Some(target) = target_pos else { return false };
    let mut g = built.grid.clone();
    stamp_slots(&mut g, &built.slots);
    for l in &built.locks {
        g.set(l.pos.0, l.pos.1, l.gap_tile);
    }
    walk_map(&g, &built.pipe_pairs, start_pos, built.world_idx)
        .nodes
        .contains(&target)
}

/// Monotone completability: beat every fort reachable in the current lock
/// state (opening its lock), repeat to a fixpoint, then check the goal is
/// reachable. Adding a pipe only grows reachability, so this catches the one
/// new failure mode — the relocated lock stranding a fort or the goal.
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
        let reachable = walk_map(&g, &built.pipe_pairs, start_pos, built.world_idx).nodes;
        let mut progressed = false;
        for &(section, pos) in &forts {
            if !open.contains(&section) && reachable.contains(&pos) {
                open.insert(section);
                progressed = true;
            }
        }
        if !progressed {
            return reachable.contains(&target);
        }
    }
}

/// Recompute the relocated lock's `secret_exit_safe` / `blocks_target` flags
/// against the final layout: this lock closed, every other lock open. Mirrors
/// the meaning `place_locks` assigns them, so the global secret-exit-safe
/// guarantee and the FX writer read consistent values. The relocated lock is
/// found by its fort `section` (one lock per section — unambiguous).
fn finalize_lock_flags(
    built: &mut BuiltWorld,
    section: usize,
    start_pos: Option<Pos>,
    target_pos: Option<Pos>,
) {
    let Some(idx) = built.locks.iter().position(|l| l.fort_section == section) else {
        return;
    };
    let mut g = built.grid.clone();
    stamp_slots(&mut g, &built.slots);
    for (i, l) in built.locks.iter().enumerate() {
        let tile = if i == idx { l.gap_tile } else { l.replace_tile };
        g.set(l.pos.0, l.pos.1, tile);
    }
    let reachable = walk_map(&g, &built.pipe_pairs, start_pos, built.world_idx).nodes;
    let target_reachable = target_pos.is_none_or(|t| reachable.contains(&t));
    let all_forts_reachable = built
        .slots
        .iter()
        .filter(|s| s.kind == SlotKind::Fortress)
        .all(|s| reachable.contains(&s.pos));
    let lock = &mut built.locks[idx];
    lock.secret_exit_safe = target_reachable && all_forts_reachable;
    lock.blocks_target = !target_reachable;
}
