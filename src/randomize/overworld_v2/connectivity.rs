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
//! Not hard rules, by design: a world can end this phase with unreachable
//! blanks (an island whose every blank is `fixed` has no legal endpoint).
//! The phase reports what it couldn't do; the census measures how often.

use super::*;

use rand::seq::IndexedRandom;

/// Safety cap on pipes placed in one world — far above any real map's island
/// count; a backstop against a walker/terrain surprise, not a tuning value.
const MAX_CONNECTIVITY_PIPES: usize = 8;

pub(crate) struct Connectivity;

impl Phase for Connectivity {
    fn name(&self) -> &'static str {
        "connectivity"
    }

    fn run(&self, state: &mut WorldState, rng: &mut dyn RngCore) -> PhaseReport {
        let mut actions = Vec::new();

        for _ in 0..MAX_CONNECTIVITY_PIPES {
            let reach = walk_reachable(&state.grid, &state.pipe_pairs, state.start, state.world_idx);
            let blanks = blank_positions(state);

            let Some(island_seed) = next_island_seed(state, &reach, &blanks) else {
                break; // everything that can be reached is reached
            };

            // The island = whatever the walker can cover starting from the
            // seed. Its blanks (minus fixed) are the far-endpoint candidates.
            let island_reach =
                walk_reachable(&state.grid, &state.pipe_pairs, Some(island_seed), state.world_idx);
            let island_candidates: Vec<Pos> = blanks
                .iter()
                .copied()
                .filter(|p| island_reach.contains(*p) && !state.fixed.contains(p))
                .collect();
            let mainland_candidates: Vec<Pos> = blanks
                .iter()
                .copied()
                .filter(|p| reach.contains(*p) && !state.fixed.contains(p))
                .collect();

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

            place_pipe_pair(state, near, far);
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

/// Stamp a teleport pipe pair onto the state: both tiles become pipes, both
/// positions become Pipe slots, and the teleport edge joins the pair list.
fn place_pipe_pair(state: &mut WorldState, a: Pos, b: Pos) {
    for pos in [a, b] {
        state.grid.set(pos.0, pos.1, TILE_PIPE);
        state.slots.push(SlotAssignment {
            pos,
            kind: SlotKind::Pipe,
            section: 0,
            is_hand_trap: false,
            is_troll_pipe: false,
        });
    }
    state.pipe_pairs.push((a, b));
}
