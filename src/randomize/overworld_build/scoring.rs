//! Candidate scoring: softmax sampling and the per-placement weight functions.
//!
//! Every tunable weight lives in the `knobs` structs (`SpreadScoring`,
//! `LevelScoring`, `FortScoring`, `LockScoring`, `PipeScoring`) — this module
//! only holds the scoring *mechanics* and fixed geometry facts.

use super::*;

use super::knobs::{FortScoring, LevelScoring, SpreadScoring};

/// Sample a candidate weighted by softmax(score / temperature). Higher
/// temperature flattens the distribution (more random); lower temperature
/// concentrates probability on top-scoring candidates. Returns `None` if empty.
pub(super) fn pick_softmax_by_score<T, R: Rng>(
    candidates: Vec<(T, f64)>,
    temperature: f64,
    rng: &mut R,
) -> Option<T> {
    if candidates.is_empty() {
        return None;
    }
    // Subtract max for numerical stability.
    let max_score = candidates
        .iter()
        .map(|(_, s)| *s)
        .fold(f64::NEG_INFINITY, f64::max);
    let weights: Vec<f64> = candidates
        .iter()
        .map(|(_, s)| ((s - max_score) / temperature).exp())
        .collect();
    let total: f64 = weights.iter().sum();
    let mut roll = rng.random_range(0.0..total);
    for (i, w) in weights.iter().enumerate() {
        roll -= w;
        if roll <= 0.0 {
            return Some(candidates.into_iter().nth(i).unwrap().0);
        }
    }
    // Floating point edge case — return last.
    Some(candidates.into_iter().last().unwrap().0)
}

/// Fortress score bonus positions per world. These isolated positions rarely
/// win fortress placement without a boost. Each entry is (world_idx, position).
pub(super) const FORTRESS_BONUS_POSITIONS: &[(usize, (usize, usize))] = &[
    (2, (5, 26)), // W3 canoe island
    (2, (0, 34)), // W3 canoe island (toad house in vanilla)
    (2, (5, 28)), // W3 canoe island (spade in vanilla)
    (2, (3, 26)), // W3 canoe island (spade in vanilla)
    (2, (3, 28)), // W3 canoe island
];

/// Total vanilla levels across all worlds (62 Level entries in the catalog).
pub(crate) const VANILLA_LEVEL_COUNT: usize = 62;

/// Exponent applied to each world's capacity when distributing levels. `1.0`
/// is pure capacity-proportional (rich worlds run away with levels); `0.0` is
/// uniform. A sub-linear value compresses the spread toward the middle —
/// pulling the high-capacity worlds (Desert, Ice) down and filling the
/// emptier ones — without forcing uniformity. Tuned by feel, not exposed to
/// players. See `distribute_levels`.
pub(crate) const LEVEL_SPREAD_EXPONENT: f64 = 0.5;

/// Returns true if a node position has exactly one traversable exit direction.
/// Dead-end positions look better with a level or fortress than as blank tiles.
pub(super) fn is_dead_end(grid: &Grid, pos: (usize, usize)) -> bool {
    let (r, c) = pos;
    let mut exits = 0;
    if c >= 2 && VALID_HORZ.contains(&grid.get(r, c - 1)) { exits += 1; }
    if c + 2 < grid.cols && VALID_HORZ.contains(&grid.get(r, c + 1)) { exits += 1; }
    if r >= 2 && VALID_VERT.contains(&grid.get(r - 1, c)) { exits += 1; }
    if r + 2 < grid.rows() && VALID_VERT.contains(&grid.get(r + 1, c)) { exits += 1; }
    exits == 1
}

/// Returns true if placing a completable tile at `pos` would create a
/// row 7/8 completion-bit collision. This is a hard game engine constraint
/// (shared bit $01) that cannot be relaxed.
pub(super) fn is_row78_conflict(
    pos: (usize, usize),
    completable: &HashSet<(usize, usize)>,
) -> bool {
    let (r, c) = pos;
    if r == 7 {
        completable.contains(&(8, c))
    } else if r == 8 {
        completable.contains(&(7, c))
    } else {
        false
    }
}

/// Shared spread/density quantities of `pos` relative to a set of reference
/// positions: (min manhattan distance, min BFS-distance difference, count of
/// reference positions within `density_radius`). The min values are
/// `usize::MAX` / `None` when they can't be computed (empty set / no BFS
/// data) — callers apply their own fallbacks.
fn spread_and_density(
    pos: (usize, usize),
    others: &HashSet<(usize, usize)>,
    bfs_distances: &HashMap<(usize, usize), usize>,
    density_radius: usize,
) -> (usize, Option<usize>, usize) {
    let (r, c) = pos;
    let my_bfs = bfs_distances.get(&pos).copied().unwrap_or(0);

    let min_manhattan = others
        .iter()
        .map(|&(cr, cc)| r.abs_diff(cr) + c.abs_diff(cc))
        .min()
        .unwrap_or(usize::MAX);

    let min_bfs_diff = others
        .iter()
        .filter_map(|p| bfs_distances.get(p))
        .map(|&d| my_bfs.abs_diff(d))
        .min();

    let nearby = others
        .iter()
        .filter(|&&(cr, cc)| {
            let manhattan = r.abs_diff(cr) + c.abs_diff(cc);
            let bfs_diff = bfs_distances
                .get(&(cr, cc))
                .map(|&d| my_bfs.abs_diff(d))
                .unwrap_or(manhattan);
            manhattan.max(bfs_diff) <= density_radius
        })
        .count();

    (min_manhattan, min_bfs_diff, nearby)
}

/// Core scoring logic shared by level and fortress placement.
pub(super) fn score_with_weights(
    grid: &Grid,
    pos: (usize, usize),
    completable: &HashSet<(usize, usize)>,
    bfs_distances: &HashMap<(usize, usize), usize>,
    spread: &SpreadScoring,
    dead_end_bonus_value: f64,
) -> f64 {
    let (min_manhattan, min_bfs_diff, nearby) =
        spread_and_density(pos, completable, bfs_distances, spread.density_radius);
    let min_bfs_diff = min_bfs_diff.unwrap_or(usize::MAX);

    let manhattan_score = (min_manhattan as f64).min(spread.separation_cap) * spread.manhattan;
    let bfs_score = (min_bfs_diff as f64).min(spread.separation_cap) * spread.bfs;
    let density_penalty = nearby as f64 * spread.density_penalty;

    let dead_end_bonus = if is_dead_end(grid, pos) { dead_end_bonus_value } else { 0.0 };

    manhattan_score + bfs_score + dead_end_bonus - density_penalty
}

/// Score a candidate position for level placement. Higher = better.
/// Includes a path relevance bonus: positions on the main start→target
/// route (low detour) score higher than side-branch positions.
pub(super) fn score_candidate(
    grid: &Grid,
    pos: (usize, usize),
    completable: &HashSet<(usize, usize)>,
    bfs_distances: &HashMap<(usize, usize), usize>,
    reverse_bfs: &HashMap<(usize, usize), usize>,
    target_bfs_dist: Option<usize>,
    knobs: &LevelScoring,
) -> f64 {
    let base = score_with_weights(
        grid, pos, completable, bfs_distances, &knobs.spread, knobs.dead_end_bonus,
    );

    // Path relevance: detour = dist(start→pos) + dist(pos→target) - dist(start→target).
    // Zero detour = perfectly on the shortest path. Higher detour = side branch.
    let path_bonus = match (target_bfs_dist, reverse_bfs.get(&pos)) {
        (Some(target_dist), Some(&rev_d)) => {
            let fwd_d = bfs_distances.get(&pos).copied().unwrap_or(0);
            let detour = (fwd_d + rev_d).saturating_sub(target_dist);
            (knobs.path_detour_cap - (detour as f64).min(knobs.path_detour_cap))
                * knobs.path_bonus
        }
        _ => 0.0,
    };

    base + path_bonus
}

/// Score a candidate position for fortress placement. Higher = better.
/// Fortresses get a larger dead-end bonus since they naturally belong at
/// path termini, plus a bonus for designated island positions.
pub(super) fn score_fortress_candidate(
    grid: &Grid,
    pos: (usize, usize),
    completable: &HashSet<(usize, usize)>,
    bfs_distances: &HashMap<(usize, usize), usize>,
    world_idx: usize,
    knobs: &FortScoring,
) -> f64 {
    let base = score_with_weights(
        grid, pos, completable, bfs_distances, &knobs.spread, knobs.dead_end_bonus,
    );
    let island_bonus = if FORTRESS_BONUS_POSITIONS.iter().any(|&(wi, p)| wi == world_idx && p == pos) {
        knobs.island_bonus
    } else {
        0.0
    };
    base + island_bonus
}
