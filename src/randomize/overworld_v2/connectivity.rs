//! Connectivity phase — the first v2 placement phase, from scratch.
//!
//! Job: make the world traversable. Every placeable blank and the world goal
//! should be walk-reachable from start; terrain the walker can't cross
//! (water, gaps between screens) gets bridged by teleport pipe pairs — one
//! endpoint on the already-reachable mainland, one on the cut-off island.
//!
//! Deliberately KNOB-FREE: both endpoints are uniform-random picks from the
//! legal candidates. Where an endpoint *should* land (junction vs dead end,
//! near vs far) is a routing question, and the whole point of the v2
//! rediscovery is to let measurement justify every control before it exists.
//! The phase spends as few pipes as possible (one pair per island) and
//! leaves the rest of the world's pipe budget for the routing phase.
//!
//! One measured-and-earned placement bar: no endpoint within
//! `PIPE_ANCHOR_RADIUS` of the start or goal (census-diagnosed — an
//! anchor-adjacent pipe mouth is an ungateable express no later phase can
//! price back up), with a full-set fallback when an island has no clear
//! candidate, because bridging outranks the bar.
//!
//! Not hard rules, by design: a world can end this phase with unreachable
//! blanks (an island whose every blank is `fixed` has no legal endpoint, or
//! the pipe budget runs out first). The phase reports what it couldn't do;
//! the census measures how often.
//!
//! The one hard bound is `state.pipe_budget` — the world's chosen pipe
//! limit (see its doc). Real terrain always connects within it, so hitting
//! the bound means a bug, contained to this world's few pipes instead of
//! looping; it also guarantees the island loop terminates.

use super::*;

use rand::seq::IndexedRandom;

pub(crate) struct Connectivity;

impl Phase for Connectivity {
    fn name(&self) -> &'static str {
        "connectivity"
    }

    fn run(&self, state: &mut WorldState, rng: &mut dyn RngCore) -> PhaseReport {
        let mut actions = Vec::new();

        while state.pipe_pairs.len() < state.pipe_budget {
            let reach = walk_reachable(&state.grid, &state.pipe_pairs, state.start, state.world_idx);
            let blanks = blank_positions(state);

            let Some(island_seed) = next_island_seed(state, &reach, &blanks) else {
                break; // everything that can be reached is reached
            };

            // The island = whatever the walker can cover starting from the
            // seed. Endpoint candidacy uses the shared `legal_blanks` rules
            // (not fixed, not taken, row-7/8 partner respected — a pipe is
            // completable content too), split by which side of the cut each
            // blank sits on. Anchor-adjacent blanks are barred (an endpoint
            // next to start/goal is an ungateable express) — but bridging is
            // mandatory, so a side whose every candidate is near an anchor
            // falls back to the full set rather than stranding the island.
            let island_reach =
                walk_reachable(&state.grid, &state.pipe_pairs, Some(island_seed), state.world_idx);
            let legal = state.legal_blanks();
            let pick_side = |on_side: &dyn Fn(&Pos) -> bool| -> Vec<Pos> {
                let side: Vec<Pos> = legal.iter().copied().filter(|p| on_side(p)).collect();
                let clear: Vec<Pos> =
                    side.iter().copied().filter(|&p| !state.near_anchor(p)).collect();
                if clear.is_empty() { side } else { clear }
            };
            let island_candidates = pick_side(&|p| island_reach.contains(*p));
            let mainland_candidates = pick_side(&|p| reach.contains(*p));

            let (Some(&near), Some(&far)) =
                (mainland_candidates.choose(rng), island_candidates.choose(rng))
            else {
                actions.push(format!(
                    "stuck: island at {island_seed:?} has no legal endpoint ({} island / {} mainland candidates)",
                    island_candidates.len(),
                    mainland_candidates.len(),
                ));
                break;
            };

            state.add_pipe_pair(near, far);
            actions.push(format!("pipe {near:?} <-> {far:?} (island of {} blanks)", island_candidates.len()));
        }

        // Count what remains cut off — measured, not enforced.
        let reach = walk_reachable(&state.grid, &state.pipe_pairs, state.start, state.world_idx);
        let stranded = blank_positions(state)
            .iter()
            .filter(|p| !reach.contains(**p))
            .count();
        let goal_ok = state.target.is_none_or(|t| reach.contains(t));
        actions.push(format!("done: {stranded} blanks stranded, goal reachable: {goal_ok}"));

        PhaseReport { phase: self.name(), actions }
    }
}

/// All placeable blank tiles on the grid (pickup's blank-tile contract),
/// including fixed ones — coverage is judged over all of them; only
/// ENDPOINT candidacy excludes `fixed`.
fn blank_positions(state: &WorldState) -> Vec<Pos> {
    let mut blanks = Vec::new();
    for r in 0..state.grid.rows() {
        for c in 0..state.grid.cols {
            if rom_data::VALID_BLANK_TILES.contains(&state.grid.get(r, c)) {
                blanks.push((r, c));
            }
        }
    }
    blanks
}

/// Where to grow next: the goal's component first (a world you can't finish
/// is the worst kind of cut off), then the first stranded blank in scan
/// order. `None` when start is unknown or nothing is stranded.
fn next_island_seed(state: &WorldState, reach: &Reach, blanks: &[Pos]) -> Option<Pos> {
    state.start?;
    if let Some(target) = state.target
        && !reach.contains(target)
    {
        return Some(target);
    }
    blanks.iter().copied().find(|p| !reach.contains(*p))
}

