//! Measured fortress shaping — the constructive half of choice-first.
//!
//! Runs after pipes + levels + HammerBro fillers exist (and after the
//! pointer-budget cap), so the world is already a complete routable graph
//! with zero forts. Forts are realized by CONVERTING HammerBro filler slots
//! to `SlotKind::Fortress` (the same mechanism the spare-pipe pass uses for
//! HB→Pipe), which keeps the pointer-table budget exactly balanced.
//!
//! Two phases, both driven by `analyze_route_choice` at `SHAPING_SLACK`
//! (wide band — near-miss corridors stay visible):
//!
//! - **Phase A — equalize.** While the world lacks 2 roughly-equal routes
//!   (`DEFAULT_SLACK` band) and a rescuable near-miss route exists, try a
//!   fort on the CHEAP route's exclusive stretch (nodes the near-miss route
//!   doesn't share). A fort there re-prices the cheap route +5, closing the
//!   gap. Candidates are ranked by the aesthetic fort score; each try is
//!   re-measured and reverted unless it grows the in-band route count or
//!   shrinks the gap.
//! - **Phase B — aesthetic remainder.** Remaining forts go down by the
//!   existing softmax over the fort score, with a choice-guard: a pick that
//!   SHRINKS the in-band route count is reverted and redrawn (bounded), so
//!   late forts don't wreck what phase A built. If every redraw degrades,
//!   the first pick lands anyway — a fort must land.
//!
//! The returned gate hint is a uniformly random placed fort: `place_locks`
//! hard-filters that fort's lock to goal-gating candidates. Uniform choice
//! kills the "the real fort is always the farthest one" tell that the
//! archetype era needed `shuffle_goal` for.

use super::*;

use super::knobs::FortScoring;
use super::route_choice::{DEFAULT_SLACK, RouteChoice, SHAPING_SLACK, analyze_route_choice};
use super::scoring::{is_row78_conflict, pick_softmax_by_score, score_fortress_candidate};
use super::sections::completable_positions;
use super::types::{BuiltWorld, SlotKind};

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

    // Phase A: equalize. Bounded — each iteration either places a fort or
    // breaks out.
    let mut iterations = 0;
    while placed.len() < fort_count && iterations < 2 * fort_count {
        iterations += 1;
        if in_band_count(&rc) >= 2 {
            break; // already choiceful — remaining forts are content
        }
        let Some(nm) = near_miss(&rc) else {
            break; // nothing a fort can rescue; spare pipes get a turn later
        };
        // Fort candidates: convertible HB slots on the cheap route's
        // exclusive stretch (nodes the near-miss route doesn't cross).
        let nm_nodes: HashSet<Pos> = nm.path.iter().copied().collect();
        let exclusive: Vec<Pos> = rc.routes[0]
            .path
            .iter()
            .copied()
            .filter(|p| !nm_nodes.contains(p))
            .collect();
        let mut cands = ranked_convertible(built, Some(&exclusive), reserved, bfs_distances, knobs);
        cands.truncate(MEASURED_TRIES);

        let before = (in_band_count(&rc), shaping_gap(&rc));
        let mut applied = false;
        for (pos, _) in cands {
            let Some(idx) = hb_slot_index(built, pos) else { continue };
            convert(built, idx, placed.len());
            let rc2 = measure(built);
            let after = (in_band_count(&rc2), shaping_gap(&rc2));
            if after.0 > before.0 || (after.0 == before.0 && after.1 < before.1) {
                rc = rc2;
                placed.push(pos);
                applied = true;
                break;
            }
            revert(built, idx);
        }
        if !applied {
            break; // forts can't close this gap
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
            let rc2 = measure(built);
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
                rc = measure(built);
                placed.push(pos);
            }
        }
    }

    if placed.is_empty() {
        return None;
    }
    Some(placed[rng.random_range(..placed.len())])
}

/// Routes within `DEFAULT_SLACK` of best — the count that defines "has
/// choice", measured inside the wider `SHAPING_SLACK` result.
fn in_band_count(rc: &RouteChoice) -> usize {
    rc.routes
        .iter()
        .filter(|r| r.cost <= rc.best_cost + DEFAULT_SLACK)
        .count()
}

/// Cheapest route ABOVE the choice band — the rescuable near-miss.
fn near_miss(rc: &RouteChoice) -> Option<&super::route_choice::ChoiceRoute> {
    rc.routes.iter().find(|r| r.cost > rc.best_cost + DEFAULT_SLACK)
}

/// Distance of the near-miss above best (`u32::MAX` when there is none) —
/// the quantity phase A drives down.
fn shaping_gap(rc: &RouteChoice) -> u32 {
    near_miss(rc).map_or(u32::MAX, |r| r.cost - rc.best_cost)
}

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

fn hb_slot_index(built: &BuiltWorld, pos: Pos) -> Option<usize> {
    built
        .slots
        .iter()
        .position(|s| s.pos == pos && s.kind == SlotKind::HammerBro)
}

fn convert(built: &mut BuiltWorld, idx: usize, ordinal: usize) {
    built.slots[idx].kind = SlotKind::Fortress;
    built.slots[idx].section = ordinal;
}

fn revert(built: &mut BuiltWorld, idx: usize) {
    built.slots[idx].kind = SlotKind::HammerBro;
    built.slots[idx].section = 0;
}
