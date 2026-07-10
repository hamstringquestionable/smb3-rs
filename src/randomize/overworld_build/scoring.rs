//! Candidate scoring: softmax sampling and the per-placement weight functions.

use super::*;

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

pub(super) const FORTRESS_BONUS: f64 = 0.5;

/// Total vanilla levels across all worlds (62 Level entries in the catalog).
pub(super) const VANILLA_LEVEL_COUNT: usize = 62;

/// Exponent applied to each world's capacity when distributing levels. `1.0`
/// is pure capacity-proportional (rich worlds run away with levels); `0.0` is
/// uniform. A sub-linear value compresses the spread toward the middle —
/// pulling the high-capacity worlds (Desert, Ice) down and filling the
/// emptier ones — without forcing uniformity. Tuned by feel, not exposed to
/// players. See `distribute_levels`.
pub(super) const LEVEL_SPREAD_EXPONENT: f64 = 0.5;

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

// Shared spread/density weights (used by level, fortress, and pipe scoring).
const W_MANHATTAN: f64 = 1.0;    // visual/spatial spread
const W_BFS: f64 = 1.5;          // traversal spread (weighted higher than grid distance)
const W_DENSITY: f64 = 3.0;      // penalty per nearby occupied tile
const DENSITY_RADIUS: usize = 4; // combined manhattan+BFS distance threshold
const SEP_CAP: f64 = 8.0;        // max separation contribution per metric

/// Shared spread/density quantities of `pos` relative to a set of reference
/// positions: (min manhattan distance, min BFS-distance difference, count of
/// reference positions within DENSITY_RADIUS). The min values are
/// `usize::MAX` / `None` when they can't be computed (empty set / no BFS
/// data) — callers apply their own fallbacks.
fn spread_and_density(
    pos: (usize, usize),
    others: &HashSet<(usize, usize)>,
    bfs_distances: &HashMap<(usize, usize), usize>,
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
            manhattan.max(bfs_diff) <= DENSITY_RADIUS
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
    dead_end_bonus_value: f64,
) -> f64 {
    let (min_manhattan, min_bfs_diff, nearby) =
        spread_and_density(pos, completable, bfs_distances);
    let min_bfs_diff = min_bfs_diff.unwrap_or(usize::MAX);

    let manhattan_score = (min_manhattan as f64).min(SEP_CAP) * W_MANHATTAN;
    let bfs_score = (min_bfs_diff as f64).min(SEP_CAP) * W_BFS;
    let density_penalty = nearby as f64 * W_DENSITY;

    let dead_end_bonus = if is_dead_end(grid, pos) { dead_end_bonus_value } else { 0.0 };

    manhattan_score + bfs_score + dead_end_bonus - density_penalty
}

/// Path relevance: max detour (in BFS hops) that still earns a bonus.
pub(super) const PATH_DETOUR_CAP: f64 = 6.0;

/// Path relevance weight. Max bonus = PATH_DETOUR_CAP * W_PATH = 9.0.
/// Tuned via test_level_placement_quality: 0.5 was decorative (no bias);
/// 3.0 dominated and clumped levels on the route at the expense of spread
/// and dead-ends. 1.5 produces a meaningful route bias without breaking
/// the spread or density terms.
pub(super) const W_PATH: f64 = 1.5;

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
) -> f64 {
    let base = score_with_weights(grid, pos, completable, bfs_distances, 0.5);

    // Path relevance: detour = dist(start→pos) + dist(pos→target) - dist(start→target).
    // Zero detour = perfectly on the shortest path. Higher detour = side branch.
    let path_bonus = match (target_bfs_dist, reverse_bfs.get(&pos)) {
        (Some(target_dist), Some(&rev_d)) => {
            let fwd_d = bfs_distances.get(&pos).copied().unwrap_or(0);
            let detour = (fwd_d + rev_d).saturating_sub(target_dist);
            (PATH_DETOUR_CAP - (detour as f64).min(PATH_DETOUR_CAP)) * W_PATH
        }
        _ => 0.0,
    };

    base + path_bonus
}

/// Score a candidate position for fortress placement. Higher = better.
/// Fortresses get a larger dead-end bonus (+5.0) since they naturally
/// belong at path termini, plus a bonus for designated island positions.
pub(super) fn score_fortress_candidate(
    grid: &Grid,
    pos: (usize, usize),
    completable: &HashSet<(usize, usize)>,
    bfs_distances: &HashMap<(usize, usize), usize>,
    world_idx: usize,
) -> f64 {
    let base = score_with_weights(grid, pos, completable, bfs_distances, 5.0);
    let island_bonus = if FORTRESS_BONUS_POSITIONS.iter().any(|&(wi, p)| wi == world_idx && p == pos) {
        FORTRESS_BONUS
    } else {
        0.0
    };
    base + island_bonus
}

/// Target proximity penalty weight. Higher = more aggressively avoids placing
/// pipes near the airship/Bowser. Tweakable for tuning.
pub(super) const W_TARGET_PROXIMITY: f64 = 4.0;

/// Max manhattan distance for target penalty normalization.
pub(super) const TARGET_MAX_DIST: f64 = 20.0;

/// Cap on the manhattan + BFS spread reward for pipe scoring. Positions
/// beyond this effective spread all score the same, preventing very-far
/// positions from always dominating. Applied to the spread term only —
/// dead-end bonus and density penalty bypass the cap so they always count.
pub(super) const PIPE_SPREAD_CAP: f64 = 7.0;

/// Softmax temperature for pipe placement. Higher = more random, lower =
/// more concentrated on top-scoring candidates. Tuned for typical pipe
/// score range of ~[-8, +12].
pub(super) const PIPE_SOFTMAX_T: f64 = 4.0;

/// Softmax temperature for fortress placement. Score range is similar to
/// pipes (~[-12, +15] including the +5 dead-end bonus).
pub(super) const FORTRESS_SOFTMAX_T: f64 = 4.0;

/// Compute target proximity penalty for a position. Positions near the
/// airship/Bowser get penalized; positions far away get no penalty.
pub(super) fn target_proximity_penalty(pos: (usize, usize), target_pos: Option<(usize, usize)>) -> f64 {
    if let Some(tp) = target_pos {
        let dist = (pos.0.abs_diff(tp.0) + pos.1.abs_diff(tp.1)) as f64;
        W_TARGET_PROXIMITY * (TARGET_MAX_DIST - dist.min(TARGET_MAX_DIST)) / TARGET_MAX_DIST
    } else {
        0.0
    }
}

/// Score a single pipe endpoint. Higher = better.
///
/// Spread reward (distance from nearest existing pipe) is capped at
/// PIPE_SPREAD_CAP. Dead-end bonus, density penalty, and target penalty
/// are applied outside the cap so they always influence the score.
///
/// When `pipe_positions` is empty (first pair) the spread term is 0 — every
/// candidate ties on spread, so picking is driven by dead-end + target only.
pub(super) fn score_pipe_endpoint(
    grid: &Grid,
    pos: (usize, usize),
    pipe_positions: &HashSet<(usize, usize)>,
    bfs_distances: &HashMap<(usize, usize), usize>,
    target_pos: Option<(usize, usize)>,
) -> f64 {
    const DEAD_END_BONUS: f64 = 1.0;

    let (min_manhattan, min_bfs_diff, nearby) =
        spread_and_density(pos, pipe_positions, bfs_distances);

    let spread = if pipe_positions.is_empty() {
        0.0
    } else {
        let min_bfs_diff = min_bfs_diff.unwrap_or(min_manhattan);
        let m = (min_manhattan as f64).min(SEP_CAP) * W_MANHATTAN;
        let b = (min_bfs_diff as f64).min(SEP_CAP) * W_BFS;
        (m + b).min(PIPE_SPREAD_CAP)
    };

    let density_penalty = nearby as f64 * W_DENSITY;

    let dead_end_bonus = if is_dead_end(grid, pos) { DEAD_END_BONUS } else { 0.0 };

    spread + dead_end_bonus - density_penalty - target_proximity_penalty(pos, target_pos)
}

/// Score a candidate pipe pair. Higher = better.
/// Rewards spread from already-placed pipes, separation between endpoints,
/// and penalizes proximity to the airship/Bowser target.
pub(super) fn score_pipe_pair(
    grid: &Grid,
    a: (usize, usize),
    b: (usize, usize),
    pipe_positions: &HashSet<(usize, usize)>,
    bfs_distances: &HashMap<(usize, usize), usize>,
    target_pos: Option<(usize, usize)>,
) -> f64 {
    let spread_a = score_pipe_endpoint(grid, a, pipe_positions, bfs_distances, target_pos);
    let spread_b = score_pipe_endpoint(grid, b, pipe_positions, bfs_distances, target_pos);
    let separation = ((a.0.abs_diff(b.0) + a.1.abs_diff(b.1)) as f64 * 0.5).min(10.0);
    spread_a + spread_b + separation
}
