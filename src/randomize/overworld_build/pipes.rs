//! Pipe endpoint placement.

use super::*;

/// Number of pipe pairs (not endpoints) per world in the vanilla ROM.
pub(super) const VANILLA_PIPE_PAIRS: [usize; 8] = [
    0,  // W1
    1,  // W2
    3,  // W3
    2,  // W4
    2,  // W5 (includes spiral tower)
    2,  // W6
    8,  // W7
    6,  // W8
];

/// Fixed pipe endpoints per world: positions that must always be a pipe.
/// The partner endpoint is placed randomly. Each entry is (world_idx, position).
pub(super) const FIXED_PIPE_ENDPOINTS: &[(usize, (usize, usize))] = &[
    (2, (6, 45)), // W3 rightmost node — always a pipe, partner randomized
];

/// Positions excluded from pipe placement. These are blank tiles that are
/// unreachable (surrounded by rocks/walls) and should never get a pipe.
pub(super) const PIPE_EXCLUDED_POSITIONS: &[(usize, (usize, usize))] = &[
    (2, (8, 6)), // W3 between two rocks near start — HB only, not a pipe slot
];

// Reason: each argument is a distinct pipe-placement input (grid, candidate
// blanks, start/target anchors, pair budget, fixed endpoints, world, RNG).
// They don't form a cohesive concept, so bundling would add indirection
// without clarity.
#[allow(clippy::too_many_arguments)]
pub(super) fn place_pipes<R: Rng>(
    grid: &mut Grid,
    blank_positions: &[(usize, usize)],
    start_pos: Option<(usize, usize)>,
    target_pos: Option<(usize, usize)>,
    pair_count: usize,
    fixed_endpoints: &[(usize, usize)],
    world_idx: usize,
    rng: &mut R,
) -> Vec<TeleportEdge> {
    if pair_count == 0 {
        return Vec::new();
    }

    // Hard exclusion: forbid pipe endpoints adjacent (≤1 walking hop) to
    // start or target. Diagnostic on 1000-seed sweeps showed 100% of
    // "trivial bypass" (0 forts + 0 levels) playthroughs were caused by
    // pipes sitting next to start, next to target, or both — eliminating
    // both ends of that pattern eliminates the failure mode. Fixed
    // endpoints (W3 boat dock) are exempt: their position is dictated by
    // ROM data, not chosen by the builder.
    // No-pipe exclusion zone, split by anchor. A pipe within one walking hop of
    // start or target trivially skips the world, so both are barred by default.
    // The halves are kept separate so the START side can be lifted when
    // connectivity demands it (completability outranks the anti-skip rule); the
    // TARGET side is never lifted, since a pipe next to the airship is the skip
    // we actually care about.
    let zone_within_1_hop = |anchor: Option<(usize, usize)>| -> HashSet<(usize, usize)> {
        let mut z = HashSet::new();
        if let Some(a) = anchor {
            for (&pos, &d) in &walk_map(grid, &[], Some(a), world_idx).distances {
                if d <= 1 {
                    z.insert(pos);
                }
            }
        }
        z
    };
    let mut start_zone = zone_within_1_hop(start_pos);
    let mut target_zone = zone_within_1_hop(target_pos);
    // Fixed endpoints stay placeable even inside either zone.
    for &fp in fixed_endpoints {
        start_zone.remove(&fp);
        target_zone.remove(&fp);
    }

    // Strict pool (default) excludes both zones; the relaxed pool restores the
    // start side. Phase 0 (fixed endpoints) and the loop start on `strict`.
    let strict: Vec<(usize, usize)> = blank_positions
        .iter()
        .copied()
        .filter(|p| !start_zone.contains(p) && !target_zone.contains(p))
        .collect();
    let relaxed: Vec<(usize, usize)> = blank_positions
        .iter()
        .copied()
        .filter(|p| !target_zone.contains(p))
        .collect();
    let blank_positions = strict.as_slice();

    let mut placed_pairs: Vec<TeleportEdge> = Vec::new();
    let mut used_positions: HashSet<(usize, usize)> = HashSet::new();

    // Phase 0: fixed endpoints — place these first, partner on opposite side.
    // The fixed endpoint is typically on an island (e.g. W3 rightmost node).
    // The partner must be on the reachable mainland so the pipe actually
    // bridges the gap. If both ends land on the same island the pipe is
    // useless and the target becomes unreachable.
    for &fixed_pos in fixed_endpoints {
        if placed_pairs.len() >= pair_count {
            break;
        }
        grid.set(fixed_pos.0, fixed_pos.1, TILE_PIPE);
        used_positions.insert(fixed_pos);

        // BFS to find which blanks are reachable from start.
        let walk = walk_map(grid, &placed_pairs, start_pos, world_idx);
        let fixed_is_reachable = walk.nodes.contains(&fixed_pos);

        // Pick partner from opposite side: if fixed is on an island,
        // partner must be reachable (and vice versa).
        let available: Vec<(usize, usize)> = blank_positions
            .iter()
            .copied()
            .filter(|p| !used_positions.contains(p))
            .filter(|p| walk.nodes.contains(p) != fixed_is_reachable)
            .collect();

        // Fall back to any available blank if no opposite-side candidates.
        let fallback: Vec<(usize, usize)> = if available.is_empty() {
            blank_positions
                .iter()
                .copied()
                .filter(|p| !used_positions.contains(p))
                .collect()
        } else {
            Vec::new()
        };
        let candidates = if available.is_empty() { &fallback } else { &available };

        // The fixed-endpoint partner is picked from the opposite side (island ↔
        // mainland) above; the must_connect_target loop below then places the
        // remaining pairs with a target-component filter, so a sub-optimal
        // partner here is recovered rather than stranding the airship. (This
        // replaced an earlier W3/SAS-specific `preferred` reachability filter,
        // now subsumed by the general island-connect logic.)
        if let Some(&partner) = candidates.choose(rng) {
            grid.set(partner.0, partner.1, TILE_PIPE);
            used_positions.insert(partner);
            placed_pairs.push((fixed_pos, partner));
        }
    }

    // Phase A+B: connect islands first (required for target reachability in A,
    // best-effort in B), then fill remaining pairs in reachable area.
    let target_reachable = |g: &Grid, pairs: &[TeleportEdge]| -> bool {
        if let Some(tp) = target_pos {
            let walk = walk_map(g, pairs, start_pos, world_idx);
            walk.nodes.contains(&tp)
        } else {
            true // no target = nothing to connect
        }
    };

    // `active` is the candidate pool the loop draws from. It starts strict and
    // is lifted to `relaxed` at most once, only when the loop would otherwise
    // give up with the target still unreachable.
    let mut active: &[(usize, usize)] = blank_positions;
    let mut lifted = false;

    let mut must_connect_target = true;
    while placed_pairs.len() < pair_count {
        // In the must_connect_target phase, stop once target is reachable.
        if must_connect_target && target_reachable(grid, &placed_pairs) {
            must_connect_target = false;
        }

        let walk = walk_map(grid, &placed_pairs, start_pos, world_idx);
        let (reachable_blanks, mut unreachable_blanks) =
            split_blanks_by_reachability(active, &walk.nodes, &used_positions);

        // While we still must reach the target, prefer bridging to an island
        // that actually leads there: keep only unreachable blanks that share a
        // walk-component with the target. This generalizes the W3 fixed-endpoint
        // `preferred` filter to every island connection, so RNG can't squander a
        // pipe on a dead tile that connects nothing (e.g. W4's stranded (6,24)).
        // Falls back to the unfiltered set when no candidate reaches the target.
        if must_connect_target && let Some(t) = target_pos {
            let target_comp = walk_map(grid, &placed_pairs, Some(t), world_idx).nodes;
            let toward_target: Vec<(usize, usize)> = unreachable_blanks
                .iter()
                .copied()
                .filter(|b| target_comp.contains(b))
                .collect();
            if !toward_target.is_empty() {
                unreachable_blanks = toward_target;
            }
        }

        if !unreachable_blanks.is_empty() && !reachable_blanks.is_empty() {
            // Connect an island: scored selection for both endpoints.
            // Unreachable side: prefer nearer islands (manhattan from start)
            // to create progressive chains rather than jumping to the end.
            let start = start_pos.unwrap_or((0, 0));
            let b_scored: Vec<((usize, usize), f64)> = unreachable_blanks
                .iter()
                .map(|&pos| {
                    let start_dist = (pos.0.abs_diff(start.0) + pos.1.abs_diff(start.1)) as f64;
                    // Nearer to start = higher score (invert distance)
                    let proximity_score = (TARGET_MAX_DIST - start_dist.min(TARGET_MAX_DIST)) / TARGET_MAX_DIST * 5.0;
                    let target_pen = target_proximity_penalty(pos, target_pos);
                    (pos, proximity_score - target_pen)
                })
                .collect();
            let b = pick_softmax_by_score(b_scored, PIPE_SOFTMAX_T, rng).unwrap();

            // Reachable side: prefer positions far from start (BFS distance),
            // spread from existing pipes, and away from target.
            let a_scored: Vec<((usize, usize), f64)> = reachable_blanks
                .iter()
                .map(|&pos| {
                    let score = score_pipe_endpoint(
                        grid, pos, &used_positions, &walk.distances, target_pos,
                    );
                    (pos, score)
                })
                .collect();
            let a = pick_softmax_by_score(a_scored, PIPE_SOFTMAX_T, rng).unwrap();

            grid.set(a.0, a.1, TILE_PIPE);
            grid.set(b.0, b.1, TILE_PIPE);
            used_positions.insert(a);
            used_positions.insert(b);
            placed_pairs.push((a, b));
        } else if must_connect_target {
            // Can't connect anything more from the strict pool, but the target
            // is still stranded. Completability beats the anti-skip rule: lift
            // the start-side no-pipe zone once and retry, which exposes the
            // start-adjacent blanks as fresh anchors. Only give up if even the
            // relaxed pool leaves the target unreachable.
            if !lifted {
                lifted = true;
                active = &relaxed;
                continue;
            }
            break;
        } else {
            // No more islands — score candidate pairs and pick from top N
            let available: Vec<(usize, usize)> = active
                .iter()
                .copied()
                .filter(|p| !used_positions.contains(p))
                .collect();

            if available.len() < 2 {
                break; // not enough slots
            }

            // Enumerate all candidate pairs and score them
            let mut candidates: Vec<(TeleportEdge, f64)> = Vec::new();
            for i in 0..available.len() {
                for j in (i + 1)..available.len() {
                    let a = available[i];
                    let b = available[j];
                    let score = score_pipe_pair(
                        grid, a, b, &used_positions, &walk.distances, target_pos,
                    );
                    candidates.push(((a, b), score));
                }
            }

            let (a, b) = pick_softmax_by_score(candidates, PIPE_SOFTMAX_T, rng).unwrap();

            grid.set(a.0, a.1, TILE_PIPE);
            grid.set(b.0, b.1, TILE_PIPE);
            used_positions.insert(a);
            used_positions.insert(b);
            placed_pairs.push((a, b));
        }
    }

    placed_pairs
}
