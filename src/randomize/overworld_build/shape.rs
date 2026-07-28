//! Measured fortress shaping — the constructive half of choice-first.
//!
//! Runs after pipes + levels + HammerBro fillers exist (and after the
//! pointer-budget cap), so the world is already a complete routable graph
//! with zero forts. Forts are realized by CONVERTING HammerBro filler slots
//! to `SlotKind::Fortress` (the same mechanism the spare-pipe pass uses for
//! HB→Pipe), which keeps the pointer-table budget exactly balanced.
//!
//! Two phases, driven by `analyze_route_choice`. Planning measures run at
//! `SHAPING_SLACK` (wide band — near-miss corridors stay visible) with
//! paths; trial measures use the cheap `measure_counts` variant at
//! `SHAPING_SLACK` (the accept key reads `shaping_gap`):
//!
//! - **Rescue loop — equalize.** While the world lacks 2 roughly-equal
//!   routes and a rescuable near-miss exists, add cost to the CHEAP route's
//!   exclusive stretch (the nodes a near-miss route doesn't share). The
//!   "add cost" tool is a parameter — a [`RescueMove`] is either a FORT
//!   converted onto that stretch (+5) or a LEVEL rebalanced onto it (moving
//!   a near-miss route's exclusive level there). Each round measures both
//!   kinds and commits the single best, preferring the cheaper move (a free
//!   level swap over a spent fort) on a measured tie. (Folds the old Phase
//!   A0 level-rebalance and Phase A fort-placement, which ran the same
//!   diagnose→locate→add-cost steps, into one loop — issue #125.)
//! - **Phase B — aesthetic remainder.** Any forts the rescue loop didn't
//!   spend go down by the softmax over the fort score, with a choice-guard:
//!   a pick that SHRINKS the in-band route count is reverted and redrawn
//!   (bounded), so late forts don't wreck what the rescue loop built. If
//!   every redraw degrades, the first pick lands anyway — a fort must land.
//!
//! The returned gate hint is a uniformly random placed fort: `place_locks`
//! hard-filters that fort's lock to goal-gating candidates. Uniform choice
//! kills the "the real fort is always the farthest one" tell that the
//! archetype era needed `shuffle_goal` for.

use super::*;

use super::knobs::FortScoring;
use super::route_choice::{
    C1_FLOOR, ChoiceRoute, DEFAULT_SLACK, RouteChoice, SHAPING_SLACK, analyze_route_choice,
    in_band_count, measure_counts, rescue_targets,
};
use super::scoring::{is_row78_conflict, pick_softmax_by_score, score_fortress_candidate};
use super::sections::completable_positions;
use super::types::{BuiltWorld, SlotKind, hb_slot_index};

/// Max candidate conversions re-measured per shaping decision before giving
/// up on that decision (phase A: give the pipe pass a turn; phase B: accept
/// the first softmax pick anyway).
const MEASURED_TRIES: usize = 4;

/// Place `fort_count` fortresses on `built` (which must have no forts yet) by
/// converting HammerBro filler slots, using route measurement to create
/// route choice where the terrain allows it. Returns the gate-fort position
/// hint for `place_locks` (`None` when no fort was placed).
///
/// Sections are assigned in placement order here and renumbered to BFS rank
/// by the caller — `analyze_route_choice` rebuilds its mask layout per call,
/// so interim section numbers only need to be unique.
pub(super) fn shape_forts<R: Rng>(
    built: &mut BuiltWorld,
    fort_count: usize,
    reserved: &HashSet<Pos>,
    bfs_distances: &HashMap<Pos, usize>,
    knobs: &FortScoring,
    rng: &mut R,
) -> Option<Pos> {
    if fort_count == 0 {
        return None;
    }

    let mut placed: Vec<Pos> = Vec::new();
    let mut rc = measure(built);

    // --- Unified rescue loop (issue #125, phase 1) -----------------------
    // Phase A0 (rebalance a level onto the cheap route) and Phase A (price
    // the cheap route with a fort) used to be two sequential measured passes
    // running the SAME three steps — diagnose a rescue target from the
    // `RouteChoice`, locate the cheap route's exclusive stretch, add cost
    // there, keep it if the in-band count rises or the gap shrinks — and
    // differed ONLY in the tool. This one loop makes the tool a parameter:
    // each round generates a level move AND a fort move on the shared
    // exclusive stretch, measures each, and commits the single best. Level
    // moves are generated (and measured) first, so on a measured tie the
    // strict-better comparison keeps the cheaper move — the free level swap
    // over a spent fort — reproducing the old A0-before-A preference without
    // hard-coding an ordering.
    let max_rounds = 2 * fort_count + 2;
    for _ in 0..max_rounds {
        if in_band_count(&rc) >= 2 {
            break; // choiceful — remaining forts are content (Phase B)
        }
        let before = (in_band_count(&rc), shaping_gap(&rc));
        let cheap = rc.routes[0].clone();
        let cheap_nodes: HashSet<Pos> = cheap.path.iter().copied().collect();

        // Level moves: rescue an out-of-band PARALLEL (non-nested) route by
        // moving one of its exclusive levels onto the cheap route's stretch
        // (cheap-exclusive HB first for the full -6 swing, then off-route).
        let mut level_moves: Vec<RescueMove> = Vec::new();
        for target in rc.routes.iter().filter(|r| r.cost > rc.best_cost + DEFAULT_SLACK).take(2) {
            let target_nodes: HashSet<Pos> = target.path.iter().copied().collect();
            let sources: Vec<Pos> = target
                .levels
                .iter()
                .copied()
                .filter(|l| !cheap.levels.contains(l))
                .collect();
            let all_dests = ranked_convertible(built, None, reserved, bfs_distances, knobs);
            let mut dests: Vec<Pos> = all_dests
                .iter()
                .filter(|(p, _)| cheap_nodes.contains(p) && !target_nodes.contains(p))
                .map(|(p, _)| *p)
                .collect();
            dests.extend(
                all_dests
                    .iter()
                    .filter(|(p, _)| !cheap_nodes.contains(p) && !target_nodes.contains(p))
                    .map(|(p, _)| *p),
            );
            for &src in sources.iter().take(2) {
                for &dst in dests.iter().take(2) {
                    level_moves.push(RescueMove::Level { src, dst });
                }
            }
        }

        // Fort moves: rescue ANY target (out-of-band parallel or dominated
        // detour) by pricing the cheap route's exclusive stretch +5.
        let mut fort_moves: Vec<RescueMove> = Vec::new();
        if placed.len() < fort_count {
            for target in rescue_targets(&rc) {
                let target_nodes: HashSet<Pos> = target.path.iter().copied().collect();
                let exclusive: Vec<Pos> = cheap
                    .path
                    .iter()
                    .copied()
                    .filter(|p| !target_nodes.contains(p))
                    .collect();
                for (pos, _) in ranked_convertible(built, Some(&exclusive), reserved, bfs_distances, knobs) {
                    fort_moves.push(RescueMove::Fort { pos });
                }
            }
        }

        // Measure a bounded prefix of each kind (levels first), keep the
        // strictly-best improvement. `apply`/`revert` trial each on `built`.
        // `ordinal` is captured by copy so the closure doesn't borrow `placed`
        // (mutated on commit below).
        let ordinal = placed.len();
        let mut best: Option<(RescueMove, (usize, u32))> = None;
        let consider = |mv: RescueMove, built: &mut BuiltWorld, best: &mut Option<(RescueMove, (usize, u32))>| {
            apply_rescue(built, &mv, ordinal);
            let rc2 = measure_counts(built, SHAPING_SLACK);
            let after = (in_band_count(&rc2), shaping_gap(&rc2));
            revert_rescue(built, &mv);
            let improves = after.0 > before.0 || (after.0 == before.0 && after.1 < before.1);
            let beats = best
                .as_ref()
                .is_none_or(|(_, b)| after.0 > b.0 || (after.0 == b.0 && after.1 < b.1));
            if improves && beats {
                *best = Some((mv, after));
            }
        };
        for mv in level_moves.into_iter().take(MEASURED_TRIES) {
            consider(mv, built, &mut best);
        }
        for mv in fort_moves.into_iter().take(MEASURED_TRIES) {
            consider(mv, built, &mut best);
        }

        match best {
            Some((mv, _)) => {
                apply_rescue(built, &mv, placed.len());
                if let RescueMove::Fort { pos } = mv {
                    placed.push(pos);
                }
                rc = measure(built); // full re-measure (paths) for the next round
            }
            None => break, // nothing measured can rescue; pipes/locks get a later turn
        }
    }

    // Phase B: aesthetic remainder, choice-guarded.
    while placed.len() < fort_count {
        let pool = ranked_convertible(built, None, reserved, bfs_distances, knobs);
        if pool.is_empty() {
            break; // no convertible slot left — degrade fort count (rare; census counts)
        }
        let baseline = in_band_count(&rc);
        let mut remaining = pool;
        let mut first_pick: Option<Pos> = None;
        let mut accepted: Option<RouteChoice> = None;

        for _ in 0..MEASURED_TRIES {
            let Some(pos) = pick_softmax_by_score(remaining.clone(), knobs.softmax_t, rng) else {
                break;
            };
            first_pick.get_or_insert(pos);
            let Some(idx) = hb_slot_index(built, pos) else { break };
            convert(built, idx, placed.len());
            // Trial: only `in_band_count` is read, which is defined on the
            // `DEFAULT_SLACK` band — measure narrow, no paths. (In-band
            // routes and their domination status can't depend on
            // out-of-band routes: a dominator is never costlier than what
            // it dominates.)
            let rc2 = measure_counts(built, DEFAULT_SLACK);
            if in_band_count(&rc2) >= baseline {
                placed.push(pos);
                accepted = Some(rc2);
                break;
            }
            revert(built, idx);
            remaining.retain(|(p, _)| *p != pos);
        }

        match accepted {
            Some(rc2) => rc = rc2,
            None => {
                // Every measured pick shrank the in-band routes — accept the
                // first (highest-drawn) anyway; a fort must land.
                let Some(pos) = first_pick else { break };
                let Some(idx) = hb_slot_index(built, pos) else { break };
                convert(built, idx, placed.len());
                rc = measure_counts(built, DEFAULT_SLACK);
                placed.push(pos);
            }
        }
    }

    if placed.is_empty() {
        return None;
    }
    Some(placed[rng.random_range(..placed.len())])
}

/// Level moves attempted per world by the C1-floor repair pass, and measured
/// (src, dst) pairs per move. Only deficient worlds (~14% pre-floor) pay.
const FLOOR_MAX_MOVES: usize = 6;
const FLOOR_MOVE_MEASURES: usize = 12;

/// Raise the cheapest route's cost to `C1_FLOOR` where the content budget
/// allows: while C1 is below the floor, move a level the cheap route doesn't
/// play onto the cheap route's path, measure-verified. A Level↔HammerBro
/// swap never changes walkability — only route cost — so locks, the goal
/// gate, and reachability are structurally untouched. Runs after locks (the
/// cost landscape is final) and before spare pipes (whose own floor guard
/// keeps shortcuts from undoing this).
///
/// Candidate picks maximize (C1 capped at the floor, in-band route count) —
/// the cap stops the pass from overshooting the floor at choice's expense.
/// Best-effort: a world without movable off-route levels or blank on-route
/// nodes stays cheap (the census counts them).
pub(super) fn enforce_c1_floor(built: &mut BuiltWorld, reserved: &HashSet<Pos>) {
    // Row 7/8 completion-bit partners of committed locks: a level may not
    // land there (locks aren't stamped on the grid, so `completable_positions`
    // can't see them — same guard the spare-pipe pass runs).
    let lock_partners: HashSet<Pos> = built
        .locks
        .iter()
        .filter_map(|l| match l.pos.0 {
            7 => Some((8, l.pos.1)),
            8 => Some((7, l.pos.1)),
            _ => None,
        })
        .collect();

    for _ in 0..FLOOR_MAX_MOVES {
        // Everything this pass reads lives in the `DEFAULT_SLACK` band, so
        // it plans AND trials narrow; the plan needs `cheap.path`, trials
        // read only (capped C1, in-band count).
        let rc = analyze_route_choice(built, DEFAULT_SLACK);
        if !rc.reachable || rc.best_cost >= C1_FLOOR {
            return;
        }
        let Some(cheap) = rc.routes.first().cloned() else { return };
        let band = rc.best_cost + DEFAULT_SLACK;
        let in_band_levels: HashSet<Pos> = rc
            .routes
            .iter()
            .filter(|r| r.cost <= band)
            .flat_map(|r| r.levels.iter().copied())
            .collect();

        // Sources, most-decorative first: levels no in-band route plays
        // (moving them can't cheapen any in-band route), then levels in the
        // band but not on the cheap route (their route drops 3, the cheap
        // route gains 3 — the measure decides if the min still rises).
        let mut sources: Vec<Pos> = built
            .slots
            .iter()
            .filter(|s| s.kind == SlotKind::Level && !in_band_levels.contains(&s.pos))
            .map(|s| s.pos)
            .collect();
        sources.extend(
            built
                .slots
                .iter()
                .filter(|s| {
                    s.kind == SlotKind::Level
                        && in_band_levels.contains(&s.pos)
                        && !cheap.levels.contains(&s.pos)
                })
                .map(|s| s.pos),
        );

        // Destinations: blank (HammerBro) nodes on the cheap route's path,
        // in path order — cost added there lands on C1 directly.
        let completable = completable_positions(&built.grid, &built.slots);
        let dests: Vec<Pos> = cheap
            .path
            .iter()
            .copied()
            .filter(|p| !reserved.contains(p) && !lock_partners.contains(p))
            .filter(|p| !is_row78_conflict(*p, &completable))
            .filter(|p| hb_slot_index(built, *p).is_some())
            .collect();

        let before = (rc.best_cost.min(C1_FLOOR), in_band_count(&rc));
        let mut best: Option<((usize, usize), (u32, usize))> = None;
        let mut measures = 0;
        'moves: for &src in sources.iter().take(4) {
            let Some(src_idx) = built
                .slots
                .iter()
                .position(|s| s.pos == src && s.kind == SlotKind::Level)
            else {
                continue;
            };
            for &dst in dests.iter().take(4) {
                if measures >= FLOOR_MOVE_MEASURES {
                    break 'moves;
                }
                let Some(dst_idx) = hb_slot_index(built, dst) else { continue };
                built.slots[src_idx].kind = SlotKind::HammerBro;
                built.slots[dst_idx].kind = SlotKind::Level;
                measures += 1;
                let rc2 = measure_counts(built, DEFAULT_SLACK);
                let key = (rc2.best_cost.min(C1_FLOOR), in_band_count(&rc2));
                if key > before && best.as_ref().is_none_or(|(_, bk)| key > *bk) {
                    best = Some(((src_idx, dst_idx), key));
                }
                built.slots[src_idx].kind = SlotKind::Level;
                built.slots[dst_idx].kind = SlotKind::HammerBro;
            }
        }

        // Apply the winning move; if no move raised (capped C1, in-band),
        // the floor is out of this world's reach — stop.
        let Some(((src_idx, dst_idx), _)) = best else { return };
        built.slots[src_idx].kind = SlotKind::HammerBro;
        built.slots[dst_idx].kind = SlotKind::Level;
    }
}

/// Cheapest route ABOVE the choice band — the rescuable near-miss.
fn near_miss(rc: &RouteChoice) -> Option<&ChoiceRoute> {
    rc.routes.iter().find(|r| r.cost > rc.best_cost + DEFAULT_SLACK)
}

/// Distance of the near-miss above best (`u32::MAX` when there is none) —
/// the quantity phase A drives down.
fn shaping_gap(rc: &RouteChoice) -> u32 {
    near_miss(rc).map_or(u32::MAX, |r| r.cost - rc.best_cost)
}

/// Full planning measure: wide band, paths populated — the result the
/// candidate-derivation reads (`routes[0].path`, `rescue_targets`) come from.
fn measure(built: &BuiltWorld) -> RouteChoice {
    analyze_route_choice(built, SHAPING_SLACK)
}

/// Convertible HB slots (optionally restricted to `within`), scored by the
/// aesthetic fort score, best first. Deterministic: slot-vec order in, score
/// with position tie-break out.
fn ranked_convertible(
    built: &BuiltWorld,
    within: Option<&[Pos]>,
    reserved: &HashSet<Pos>,
    bfs_distances: &HashMap<Pos, usize>,
    knobs: &FortScoring,
) -> Vec<(Pos, f64)> {
    let completable = completable_positions(&built.grid, &built.slots);
    let content: HashSet<Pos> = built
        .slots
        .iter()
        .filter(|s| matches!(s.kind, SlotKind::Level | SlotKind::Fortress))
        .map(|s| s.pos)
        .collect();
    let mut out: Vec<(Pos, f64)> = built
        .slots
        .iter()
        .filter(|s| s.kind == SlotKind::HammerBro)
        .filter(|s| !reserved.contains(&s.pos))
        .filter(|s| within.is_none_or(|w| w.contains(&s.pos)))
        .filter(|s| !is_row78_conflict(s.pos, &completable))
        .map(|s| {
            let score = score_fortress_candidate(
                &built.grid,
                s.pos,
                &content,
                bfs_distances,
                built.world_idx,
                knobs,
            );
            (s.pos, score)
        })
        .collect();
    out.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    out
}

fn convert(built: &mut BuiltWorld, idx: usize, ordinal: usize) {
    built.slots[idx].kind = SlotKind::Fortress;
    built.slots[idx].section = ordinal;
}

fn revert(built: &mut BuiltWorld, idx: usize) {
    built.slots[idx].kind = SlotKind::HammerBro;
    built.slots[idx].section = 0;
}

/// One measured rescue move in the unified shaping loop (issue #125): either
/// rebalance a level onto the cheap route's exclusive stretch, or price that
/// stretch by converting a filler to a fort. The loop measures each candidate
/// and commits the best — the "tool as a parameter" that folds the old Phase A0
/// (level) and Phase A (fort) into one pass. The move vocabulary grows in later
/// phases (locks, pipes, gated shortcuts).
enum RescueMove {
    /// Move the level at `src` onto the HammerBro filler at `dst`.
    Level { src: Pos, dst: Pos },
    /// Convert the HammerBro filler at `pos` to a fortress.
    Fort { pos: Pos },
}

/// Apply `mv` to `built` in place (used for both trial and commit).
/// `fort_ordinal` is the interim section for a new fort — unique per placement,
/// renumbered to BFS rank later.
fn apply_rescue(built: &mut BuiltWorld, mv: &RescueMove, fort_ordinal: usize) {
    match *mv {
        RescueMove::Level { src, dst } => {
            if let Some(i) = built
                .slots
                .iter()
                .position(|s| s.pos == src && s.kind == SlotKind::Level)
            {
                built.slots[i].kind = SlotKind::HammerBro;
            }
            if let Some(j) = hb_slot_index(built, dst) {
                built.slots[j].kind = SlotKind::Level;
            }
        }
        RescueMove::Fort { pos } => {
            if let Some(i) = hb_slot_index(built, pos) {
                convert(built, i, fort_ordinal);
            }
        }
    }
}

/// Reverse [`apply_rescue`] (trial cleanup).
fn revert_rescue(built: &mut BuiltWorld, mv: &RescueMove) {
    match *mv {
        RescueMove::Level { src, dst } => {
            if let Some(j) = built
                .slots
                .iter()
                .position(|s| s.pos == dst && s.kind == SlotKind::Level)
            {
                built.slots[j].kind = SlotKind::HammerBro;
            }
            if let Some(i) = built
                .slots
                .iter()
                .position(|s| s.pos == src && s.kind == SlotKind::HammerBro)
            {
                built.slots[i].kind = SlotKind::Level;
            }
        }
        RescueMove::Fort { pos } => {
            if let Some(i) = built
                .slots
                .iter()
                .position(|s| s.pos == pos && s.kind == SlotKind::Fortress)
            {
                revert(built, i);
            }
        }
    }
}
