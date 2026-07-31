//! The measuring stick: one summary of a world's route structure, computed
//! through the SAME scorer the shipping builder uses (`analyze_route_choice`
//! via `WorldState::to_built`), so vanilla, current-builder, and v2 worlds
//! are always compared like with like.

use super::*;

/// One world's measured route structure.
pub(crate) struct WorldMeasure {
    /// Goal reachable at all (any lock state).
    pub reachable: bool,
    /// Cost of the cheapest route ("C1", in scorer points: level 3 / fort 5 /
    /// pipe 1 / rock 8).
    pub c1: u32,
    /// Distinct non-dominated routes within the choice band of the cheapest.
    /// 1 = linear, 2+ = the player has a real decision.
    pub routes_in_band: usize,
    /// Dominated detour routes (pure supersets of a kept route) — not
    /// choices, but the raw material shaping passes work on.
    pub dominated_detours: usize,
    /// Mean count of levels an alternative route plays that the cheapest
    /// route does not — the "how different is the choice" number. 0.0 when
    /// linear.
    pub mean_exclusive_levels: f64,
    /// Goal reachable from start with EVERY lock closed — the world has no
    /// door-and-key moment on its trunk. NOT a problem by decree (2026-07-30):
    /// 5 of 8 vanilla worlds are goal-open, locks are milestones there, and
    /// an open goal behind a C1 ≥ floor route is fine. Only CHEAPNESS is a
    /// defect; this stays a census column, never a target.
    pub goal_open: bool,
    /// The full scorer result, for detailed printing/inspection.
    pub rc: RouteChoice,
}

/// Measure one world.
pub(crate) fn measure_world(state: &WorldState) -> WorldMeasure {
    let rc = analyze_route_choice(&state.to_built(), DEFAULT_SLACK);
    WorldMeasure {
        reachable: rc.reachable,
        c1: rc.best_cost,
        routes_in_band: rc.routes.len(),
        dominated_detours: rc.detours.len(),
        mean_exclusive_levels: mean_exclusive_levels(&rc),
        goal_open: goal_open(state),
        rc,
    }
}

/// Average, over the non-cheapest in-band routes, of how many levels each
/// plays that the cheapest route does not. Distinguishes a genuinely
/// different alternative (high) from a near-duplicate that swaps one stop
/// (low).
fn mean_exclusive_levels(rc: &RouteChoice) -> f64 {
    let Some(cheapest) = rc.routes.first() else {
        return 0.0;
    };
    let alternatives = &rc.routes[1..];
    if alternatives.is_empty() {
        return 0.0;
    }
    let total_exclusive: usize = alternatives
        .iter()
        .map(|route| route.levels.difference(&cheapest.levels).count())
        .sum();
    total_exclusive as f64 / alternatives.len() as f64
}

/// Whether the goal is reachable from start with every lock closed (no
/// fortress beaten yet).
fn goal_open(state: &WorldState) -> bool {
    let Some(target) = state.target else {
        return false;
    };
    let mut grid = state.grid.clone();
    stamp_slots(&mut grid, &state.slots);
    for lock in &state.locks {
        grid.set(lock.pos.0, lock.pos.1, lock.gap_tile);
    }
    walk_reachable(&grid, &state.pipe_pairs, state.start, state.world_idx).contains(target)
}
