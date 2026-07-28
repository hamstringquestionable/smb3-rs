//! Compound choice-shaping move: the **gated shortcut** (issue #125).
//!
//! Every measured sub-pass before this one is *single-object* — it places a
//! fort, OR a lock, OR a pipe, blind to the others. The interesting
//! choice-structures are *compound*: a gated shortcut is a pipe **and** a lock
//! **and** an off-path fort, working as one unit. This module places that one
//! compound move as a joint pipe+lock step, measured atomically.
//!
//! ## What it builds
//!
//! On a LINEAR world (one in-band route), a spare pipe that skips content
//! creates a *dominating* shortcut — a strictly cheaper route that the
//! spare-pipe pass would veto (a free skip is not a choice). This pass keeps
//! that pipe and makes it *fair*: it relocates an existing **off-route fort's
//! lock** onto the shortcut's exclusive stretch, so the shortcut now costs the
//! player that fort's +5. Two comparable routes = a real decision:
//!
//! ```text
//!   main route:      start ─ level ─ level ─ level ─ goal      (cost C0)
//!   gated shortcut:  start ─ beat F ─[lock]─ pipe ══> goal      (cost ~C0)
//! ```
//!
//! ## Why relocation, not a new lock
//!
//! The ROM budget is fixed on both axes a naive additive move would grow: the
//! per-world pipe count is capped by the catalog's pipe pool, and the fortress
//! FX table holds at most 4 locks/world with exactly ONE lock per fort section
//! (the writer folds each lock into its fort's FX batch). A census of 500 seeds
//! found ~0% of linear worlds have a *lockless* off-route fort — every fort is
//! already locked — so a gated shortcut cannot add a lock. It instead **moves**
//! an off-route fort's existing lock from its original chokepoint onto the
//! shortcut: lock count stays == fort count, the FX writer is untouched, and
//! the fort that was gating dead nodes now gates a real choice.
//!
//! ## Safety
//!
//! The move is applied to a clone and kept only if (a) the world stays
//! completable (a monotone fort-beating fixpoint reaches the goal) and (b) the
//! measured in-band route count strictly rises. Adding a pipe can only add
//! reachability; the sole new stranding risk is the relocated lock's tile,
//! which the completability fixpoint checks directly.

use super::*;

use std::sync::atomic::{AtomicU64, Ordering};

use super::route_choice::{
    DEFAULT_SLACK, SHAPING_SLACK, analyze_route_choice, in_band_count, measure_counts,
    walk_edge_mids,
};
use super::types::{BuiltWorld, SlotKind, stamp_slots};

/// Metrics for "how often the compound move comes into play" (issue #125 /
/// census `test_compound_moves`). `ELIGIBLE` = linear worlds that entered the
/// search with the ingredients present; `APPLIED` = worlds where a gated
/// shortcut was actually placed. Relaxed atomics: a single add per built world,
/// negligible in production, read only by the census after a build sweep.
pub(crate) static COMPOUND_ELIGIBLE: AtomicU64 = AtomicU64::new(0);
pub(crate) static COMPOUND_APPLIED: AtomicU64 = AtomicU64::new(0);

/// Reset the metric counters (census calls this before a sweep).
#[cfg(test)]
pub(crate) fn reset_metrics() {
    COMPOUND_ELIGIBLE.store(0, Ordering::Relaxed);
    COMPOUND_APPLIED.store(0, Ordering::Relaxed);
}

/// Candidate off-route forts examined per world, shortcut pipes per fort-free
/// scan, and golden lock sites per shortcut. All small: the pass runs only on
/// linear worlds but still measures per candidate, so the budget stays inside
/// the sub-100ms WASM ceiling. (Census `test_build_time` guards this.)
const MAX_PIPES: usize = 4;
const MAX_FORTS: usize = 2;
const MAX_SITES: usize = 2;

/// Try to place one gated-shortcut compound move on `built`, consuming at most
/// one pipe from the `spare_budget` the spare-pipe pass would otherwise use.
/// Returns the number of pipe pairs consumed (0 or 1). Runs after locks and the
/// C1 floor, before spare pipes, so it sees the final fort/lock landscape and
/// leaves the remaining pipe budget to the spare pass.
pub(super) fn place_gated_shortcut<R: Rng>(
    built: &mut BuiltWorld,
    spare_budget: usize,
    reserved: &HashSet<Pos>,
    start_pos: Option<Pos>,
    target_pos: Option<Pos>,
    rng: &mut R,
) -> usize {
    if spare_budget == 0 {
        return 0;
    }
    // Only linear worlds need the move; a choiceful world is already done.
    let rc = analyze_route_choice(built, SHAPING_SLACK);
    let base_in_band = in_band_count(&rc);
    if !rc.reachable || base_in_band >= 2 {
        return 0;
    }
    let Some(cheap) = rc.routes.first() else { return 0 };
    let cheap_nodes: HashSet<Pos> = cheap.path.iter().copied().collect();
    let cheap_mids = walk_edge_mids(&cheap.path);
    // Whether the goal is already open at start (no fort gates it). Relocating a
    // fort's lock off its chokepoint can un-gate the goal; if it was gated, we
    // won't accept a move that opens it (keeps the goal-open rate flat).
    let goal_open_before = goal_open(built, start_pos, target_pos);

    // Off-route forts whose lock we could relocate: a fortress not on the cheap
    // route (so beating it is a genuine extra cost), carrying an existing lock
    // that is NOT the world's goal gate (moving the goal gate could open the
    // goal at start). Shuffled so the choice of which fort to spend varies.
    let locked_sections: HashSet<usize> = built
        .locks
        .iter()
        .filter(|l| !l.blocks_target)
        .map(|l| l.fort_section)
        .collect();
    let mut forts: Vec<(usize, Pos)> = built
        .slots
        .iter()
        .filter(|s| s.kind == SlotKind::Fortress && !cheap_nodes.contains(&s.pos))
        .filter(|s| locked_sections.contains(&s.section))
        .map(|s| (s.section, s.pos))
        .collect();
    if forts.is_empty() {
        return 0;
    }
    forts.shuffle(rng);
    forts.truncate(MAX_FORTS);

    // Convertible pipe endpoints: HammerBro fillers (never levels/forts), not a
    // reserved sprite tile — same stock the spare-pipe pass draws from.
    let dist = walk_map(&built.grid, &built.pipe_pairs, start_pos, built.world_idx).distances;
    let endpoints: Vec<Pos> = built
        .slots
        .iter()
        .filter(|s| s.kind == SlotKind::HammerBro && !reserved.contains(&s.pos))
        .map(|s| s.pos)
        .filter(|p| dist.contains_key(p))
        .collect();
    if endpoints.len() < 2 {
        return 0;
    }
    let level_d: Vec<usize> = built
        .slots
        .iter()
        .filter(|s| s.kind == SlotKind::Level)
        .filter_map(|s| dist.get(&s.pos).copied())
        .collect();

    // Rank candidate shortcut pipes by how much content they skip (levels whose
    // route-distance sits strictly between the endpoints), widest first — the
    // pairs most likely to create a dominating shortcut worth gating.
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
    if pairs.is_empty() {
        return 0;
    }
    // We got here with the ingredients present — this world is a real candidate.
    COMPOUND_ELIGIBLE.fetch_add(1, Ordering::Relaxed);
    pairs.sort_by_key(|&(a, b, skipped, jump)| {
        (std::cmp::Reverse(skipped), std::cmp::Reverse(jump), a, b)
    });
    pairs.truncate(MAX_PIPES);

    // Search: for each shortcut pipe, derive the golden lock sites (tiles the
    // shortcut crosses but the old cheap route does not), then gate one with an
    // off-route fort's relocated lock. Keep the move that most raises the
    // in-band route count.
    let mut best: Option<(BuiltWorld, usize, usize)> = None; // (world, in_band, section)
    for (a, b, _, _) in pairs {
        let mut piped = built.clone();
        add_pipe(&mut piped, a, b);
        let rc_p = analyze_route_choice(&piped, SHAPING_SLACK);
        let Some(shortcut) = rc_p.routes.first() else { continue };
        // Golden sites: the shortcut's exclusive walk-edge mid-tiles vs the old
        // cheap route. Sorted for determinism; capped.
        let shortcut_nodes: HashSet<Pos> = shortcut.path.iter().copied().collect();
        let mut sites: Vec<Pos> = walk_edge_mids(&shortcut.path)
            .difference(&cheap_mids)
            .copied()
            .filter(|&t| t.0 != 7 && t.0 != 8) // conservative: skip row-7/8 completion-bit tiles
            .filter(|&t| lockable_tile_at(&piped.grid, t))
            // Never stack onto another fort's lock tile: two locks on one tile
            // would collide in the route model's lock map and the FX writer.
            .filter(|&t| !piped.locks.iter().any(|l| l.pos == t))
            .collect();
        if sites.is_empty() {
            continue;
        }
        sites.sort_unstable();
        sites.truncate(MAX_SITES);

        for &(section, fort_pos) in &forts {
            // A fort already on the shortcut is paid for there — its lock would
            // be decorative. Skip.
            if shortcut_nodes.contains(&fort_pos) {
                continue;
            }
            for &site in &sites {
                let mut trial = piped.clone();
                if !relocate_lock(&mut trial, section, site) {
                    continue;
                }
                // Measure choice first, then gate the (more expensive)
                // completability fixpoint on the candidate actually being a new
                // best — most trials never reach it.
                let after = in_band_count(&measure_counts(&trial, DEFAULT_SLACK));
                if after > base_in_band
                    && best.as_ref().is_none_or(|&(_, n, _)| after > n)
                    && (goal_open_before || !goal_open(&trial, start_pos, target_pos))
                    && completable(&trial, start_pos, target_pos)
                {
                    best = Some((trial, after, section));
                }
            }
        }
    }

    if let Some((mut world, _, section)) = best {
        // Recompute the relocated lock's secret-exit/target flags against the
        // final layout (the search left them stale — see relocate_lock).
        finalize_lock_flags(&mut world, section, start_pos, target_pos);
        *built = world;
        COMPOUND_APPLIED.fetch_add(1, Ordering::Relaxed);
        1
    } else {
        0
    }
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
