//! Pipe endpoint placement.

use super::*;

use super::scoring::{PIPE_SOFTMAX_T, TARGET_MAX_DIST, pick_softmax_by_score};
use super::sections::split_blanks_by_reachability;
use super::types::{SlotAssignment, SlotKind};

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

    // No-pipe exclusion zone, split by anchor. Diagnostic on 1000-seed sweeps
    // showed 100% of "trivial bypass" (0 forts + 0 levels) playthroughs were
    // caused by pipes within one walking hop of start or target, so both are
    // barred by default. The halves are kept separate so the START side can be
    // lifted when connectivity demands it (completability outranks the
    // anti-skip rule); the TARGET side is never lifted, since a pipe next to
    // the airship is the skip we actually care about. Fixed endpoints (W3 boat
    // dock) are exempt: their position is dictated by ROM data, not chosen by
    // the builder.
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
        let available: Vec<(usize, usize)> = strict
            .iter()
            .copied()
            .filter(|p| !used_positions.contains(p))
            .filter(|p| walk.nodes.contains(p) != fixed_is_reachable)
            .collect();

        // Fall back to any available blank if no opposite-side candidates.
        let fallback: Vec<(usize, usize)> = if available.is_empty() {
            strict
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
    let mut active: &[(usize, usize)] = &strict;
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

        // Guarantee fallback: on the LAST connectivity pipe with the objective
        // still stranded, restrict the island side to the objective's own
        // walk-component so this pipe definitely reaches it. Pipes are
        // teleports (no distance limit), so one pipe always suffices — hence
        // reserving just the final pipe is enough to guarantee reachability.
        // On every earlier pipe we instead grow outward (below), so the goal
        // is reached through intermediate islands, not a direct start→goal
        // jump. (The old code applied this filter on EVERY connectivity pipe,
        // which forced the first pipe straight to the goal island — a 100%
        // start→goal express rate that collapsed multi-island worlds.)
        let pipes_left = pair_count - placed_pairs.len();
        if must_connect_target
            && pipes_left <= 1
            && let Some(t) = target_pos
        {
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
            // Build outward: bridge the reachable frontier to the NEAREST
            // unreachable island, connecting its closest two blanks. This
            // grows the pipe network as a chain from start (start → i1 → i2 →
            // … → goal) instead of teleporting straight to the goal island.
            // Island side: prefer the blank nearest to the current frontier.
            let b_scored: Vec<((usize, usize), f64)> = unreachable_blanks
                .iter()
                .map(|&b| {
                    let frontier_dist = reachable_blanks
                        .iter()
                        .map(|&a| (a.0.abs_diff(b.0) + a.1.abs_diff(b.1)) as f64)
                        .fold(f64::INFINITY, f64::min);
                    // Nearer to the reachable frontier = higher score.
                    let proximity =
                        (TARGET_MAX_DIST - frontier_dist.min(TARGET_MAX_DIST)) / TARGET_MAX_DIST * 5.0;
                    (b, proximity)
                })
                .collect();
            let b = pick_softmax_by_score(b_scored, PIPE_SOFTMAX_T, rng).unwrap();

            // Reachable side: the frontier blank nearest to b (shortest bridge),
            // so the pipe extends the frontier to that island rather than
            // reaching back across the map.
            let a_scored: Vec<((usize, usize), f64)> = reachable_blanks
                .iter()
                .map(|&a| {
                    let d = (a.0.abs_diff(b.0) + a.1.abs_diff(b.1)) as f64;
                    (a, -d) // nearer to b = higher
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
            // All islands are connected and the target is reachable. The
            // remaining pipe budget is placed later by `place_spare_pipes`,
            // after levels exist, so each spare pipe can be aimed to skip a
            // level instead of being scored on spatial spread alone. Stop the
            // connectivity phase here.
            break;
        }
    }

    placed_pairs
}

/// Place `spare_needed` spare pipe pairs after `populate_sections`, converting
/// the lowest-value filler (HammerBro) slots into pipe endpoints. Unlike the
/// connectivity phase this runs with levels already placed, so each pair is
/// scored by how many level slots it lets the player skip: a pipe from a
/// near-start node to a far node teleports past every level on the stretch
/// between them (approximated by route-distance band). Endpoints are drawn
/// only from HammerBro slots — never levels or forts — and both are already
/// reachable, so a spare pipe can never strand content; it only shortcuts.
/// Pairs that skip nothing are still placed (a fall-back keeps the world at
/// its fixed vanilla pipe count).
// Reason: each argument is a distinct placement input (grid, slots to convert,
// the pipe list to extend, budget, reserved sprite tiles, start anchor, world,
// RNG). They don't form a cohesive concept, so bundling adds indirection
// without clarity — same call shape as `place_pipes`.
#[allow(clippy::too_many_arguments)]
pub(super) fn place_spare_pipes<R: Rng>(
    grid: &mut Grid,
    slots: &mut [SlotAssignment],
    pipe_pairs: &mut Vec<TeleportEdge>,
    spare_needed: usize,
    reserved: &HashSet<(usize, usize)>,
    start_pos: Option<(usize, usize)>,
    world_idx: usize,
    rng: &mut R,
) {
    if spare_needed == 0 {
        return;
    }

    // Convertible endpoints: HammerBro filler slots, minus any reserved
    // (mandatory HB sprite) positions that must keep their sprite. Kept as an
    // ordered Vec (slot order) — candidate order feeds softmax sampling, so it
    // must be deterministic across runs, unlike HashSet iteration.
    let mut available: Vec<(usize, usize)> = slots
        .iter()
        .filter(|s| s.kind == SlotKind::HammerBro && !reserved.contains(&s.pos))
        .map(|s| s.pos)
        .collect();

    let level_positions: Vec<(usize, usize)> = slots
        .iter()
        .filter(|s| s.kind == SlotKind::Level)
        .map(|s| s.pos)
        .collect();

    for _ in 0..spare_needed {
        if available.len() < 2 {
            break;
        }
        // Recompute distances each round — a placed spare pipe (a teleport)
        // changes the route, so later pairs score against the updated map.
        let dist = walk_map(grid, pipe_pairs, start_pos, world_idx).distances;
        let level_d: Vec<usize> = level_positions
            .iter()
            .filter_map(|p| dist.get(p).copied())
            .collect();

        let avail = &available;
        let mut candidates: Vec<(TeleportEdge, f64)> = Vec::new();
        for i in 0..avail.len() {
            for j in (i + 1)..avail.len() {
                let a = avail[i];
                let b = avail[j];
                let (da, db) = match (dist.get(&a), dist.get(&b)) {
                    (Some(&da), Some(&db)) => (da, db),
                    _ => continue,
                };
                let (lo, hi) = (da.min(db), da.max(db));
                if hi - lo < 2 {
                    continue; // too small a jump to be a real shortcut
                }
                // Levels whose route distance sits strictly between the two
                // endpoints are the ones the teleport lets the player skip.
                let skipped = level_d.iter().filter(|&&d| d > lo && d < hi).count();
                let score = skipped as f64 * 10.0 + (hi - lo) as f64;
                candidates.push(((a, b), score));
            }
        }

        // Prefer a level-skipping pair; if none qualifies, fall back to the
        // most distance-separated pair so the world still reaches its vanilla
        // pipe count.
        let chosen = pick_softmax_by_score(candidates, PIPE_SOFTMAX_T, rng).or_else(|| {
            let mut best: Option<(TeleportEdge, usize)> = None;
            for i in 0..avail.len() {
                for j in (i + 1)..avail.len() {
                    let (a, b) = (avail[i], avail[j]);
                    if let (Some(&da), Some(&db)) = (dist.get(&a), dist.get(&b)) {
                        let jump = da.abs_diff(db);
                        if best.is_none_or(|(_, bj)| jump > bj) {
                            best = Some(((a, b), jump));
                        }
                    }
                }
            }
            best.map(|(pair, _)| pair)
        });

        let (a, b) = match chosen {
            Some(pair) => pair,
            None => break,
        };

        grid.set(a.0, a.1, TILE_PIPE);
        grid.set(b.0, b.1, TILE_PIPE);
        available.retain(|&p| p != a && p != b);
        pipe_pairs.push((a, b));
        for s in slots.iter_mut() {
            if s.pos == a || s.pos == b {
                s.kind = SlotKind::Pipe;
            }
        }
    }
}
