//! Mission-owned connectivity pipe placement.
//!
//! Fork of `pipes::place_pipes` carrying the mission builder's topology rules.
//! The core mechanic: `walk_map` only traverses a pipe whose entrance it can
//! WALK to, so a lock in front of an entrance strands the whole island behind
//! it. That makes island gateability a property this pass can construct:
//!
//! - **Choked entrances.** When bridging to an island, the reachable-side
//!   endpoint is preferred from tiles that some lockable tile already strands
//!   (the entrance sits behind a potential lock). A chain mission then finds
//!   chained, individually-gateable islands; a fork mission finds a one-lock
//!   island home for its terminal group. The embed search already exploits
//!   stranded entrances (its strand cache walks through pipes) — this pass
//!   guarantees such geometry exists instead of leaving it to luck.
//! - **No start→goal express.** The pipe that connects the GOAL's component
//!   prefers a source endpoint OUTSIDE the start island, so the goal never
//!   hangs one teleport off the starting island when the map allows depth.
//!
//! Both rules are hard preferences with explicit fallbacks — completability
//! always outranks topology quality, exactly like the shared pass.

use super::*;

use super::pipes::{FIXED_PIPE_ENDPOINTS, PIPE_EXCLUDED_POSITIONS};
use super::plan::PipeScoring;
use super::scoring::pick_softmax_by_score;
use super::sections::split_blanks_by_reachability;

/// Per-lock strand sets in the current walk state: for every lockable path
/// tile, the set of nodes that fall out of the walk when it alone closes.
/// This is the pipe pass's view of "what could a future lock gate here".
fn lock_strands(
    grid: &Grid,
    pipes: &[TeleportEdge],
    start_pos: Option<Pos>,
    world_idx: usize,
) -> Vec<HashSet<Pos>> {
    let base = walk_map(grid, pipes, start_pos, world_idx);
    let mut out = Vec::new();
    for &t in &base.path_tiles {
        if !LOCKABLE_TILES.contains(&grid.get(t.0, t.1)) {
            continue;
        }
        let mut g = grid.clone();
        g.set(t.0, t.1, gap_tile_for(g.get(t.0, t.1)));
        let closed = walk_map(&g, pipes, start_pos, world_idx).nodes;
        let strand: HashSet<Pos> =
            base.nodes.iter().filter(|n| !closed.contains(*n)).copied().collect();
        if !strand.is_empty() {
            out.push(strand);
        }
    }
    out
}

/// Mission-first connectivity pipe placement. Same contract as
/// `pipes::place_pipes`: stamps `TILE_PIPE` on the grid and returns the
/// placed teleport pairs (connectivity only — the spare-pipe budget is spent
/// later, after locks exist).
// Reason: same call shape as `place_pipes` — each arg is a distinct
// placement input; a bundling struct would add indirection, not clarity.
#[allow(clippy::too_many_arguments)]
pub(super) fn mission_place_pipes<R: Rng>(
    grid: &mut Grid,
    blank_positions: &[Pos],
    start_pos: Option<Pos>,
    target_pos: Option<Pos>,
    pair_count: usize,
    fixed_endpoints: &[Pos],
    knobs: &PipeScoring,
    world_idx: usize,
    rng: &mut R,
) -> Vec<TeleportEdge> {
    if pair_count == 0 {
        return Vec::new();
    }

    // The start's own walking island, before any pipe exists — used by the
    // no-express rule (the goal-connecting pipe prefers a source outside it).
    let start_island: HashSet<Pos> = walk_map(grid, &[], start_pos, world_idx).nodes;

    // No-pipe exclusion zones around start and target, same rationale and
    // shape as the shared pass (see `place_pipes`): the start side may be
    // lifted when connectivity demands it, the target side never is.
    let zone_within_1_hop = |anchor: Option<Pos>| -> HashSet<Pos> {
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
    for &fp in fixed_endpoints {
        start_zone.remove(&fp);
        target_zone.remove(&fp);
    }

    let strict: Vec<Pos> = blank_positions
        .iter()
        .copied()
        .filter(|p| !start_zone.contains(p) && !target_zone.contains(p))
        .collect();
    let relaxed: Vec<Pos> = blank_positions
        .iter()
        .copied()
        .filter(|p| !target_zone.contains(p))
        .collect();

    let mut placed_pairs: Vec<TeleportEdge> = Vec::new();
    let mut used_positions: HashSet<Pos> = HashSet::new();

    // Phase 0: fixed endpoints (W3 dock) — identical to the shared pass, plus
    // the choked-entrance preference on the partner side.
    for &fixed_pos in fixed_endpoints {
        if placed_pairs.len() >= pair_count {
            break;
        }
        grid.set(fixed_pos.0, fixed_pos.1, TILE_PIPE);
        used_positions.insert(fixed_pos);

        let walk = walk_map(grid, &placed_pairs, start_pos, world_idx);
        let fixed_is_reachable = walk.nodes.contains(&fixed_pos);

        let available: Vec<Pos> = strict
            .iter()
            .copied()
            .filter(|p| !used_positions.contains(p))
            .filter(|p| walk.nodes.contains(p) != fixed_is_reachable)
            .collect();
        let fallback: Vec<Pos> = if available.is_empty() {
            strict
                .iter()
                .copied()
                .filter(|p| !used_positions.contains(p))
                .collect()
        } else {
            Vec::new()
        };
        let candidates = if available.is_empty() { &fallback } else { &available };

        // Prefer a gateable partner; fall back to any.
        let strandable: HashSet<Pos> = lock_strands(grid, &placed_pairs, start_pos, world_idx)
            .into_iter()
            .flatten()
            .collect();
        let choked: Vec<Pos> = candidates
            .iter()
            .copied()
            .filter(|p| strandable.contains(p))
            .collect();
        let pool = if choked.is_empty() { candidates } else { &choked };

        if let Some(&partner) = pool.choose(rng) {
            grid.set(partner.0, partner.1, TILE_PIPE);
            used_positions.insert(partner);
            placed_pairs.push((fixed_pos, partner));
        }
    }

    let target_reachable = |g: &Grid, pairs: &[TeleportEdge]| -> bool {
        if let Some(tp) = target_pos {
            walk_map(g, pairs, start_pos, world_idx).nodes.contains(&tp)
        } else {
            true
        }
    };

    let mut active: &[Pos] = &strict;
    let mut lifted = false;

    let mut must_connect_target = true;
    while placed_pairs.len() < pair_count {
        if must_connect_target && target_reachable(grid, &placed_pairs) {
            must_connect_target = false;
        }

        let walk = walk_map(grid, &placed_pairs, start_pos, world_idx);
        let (reachable_blanks, mut unreachable_blanks) =
            split_blanks_by_reachability(active, &walk.nodes, &used_positions);

        // Last-connectivity-pipe guarantee: aim the island side at the
        // target's own component so the goal definitely connects (identical
        // to the shared pass).
        let pipes_left = pair_count - placed_pairs.len();
        let mut connecting_target_comp = false;
        if must_connect_target
            && let Some(t) = target_pos
        {
            let target_comp = walk_map(grid, &placed_pairs, Some(t), world_idx).nodes;
            if pipes_left <= 1 {
                let toward_target: Vec<Pos> = unreachable_blanks
                    .iter()
                    .copied()
                    .filter(|b| target_comp.contains(b))
                    .collect();
                if !toward_target.is_empty() {
                    unreachable_blanks = toward_target;
                    connecting_target_comp = true;
                }
            } else if unreachable_blanks.iter().all(|b| target_comp.contains(b)) {
                // Only the goal's component is left to bridge.
                connecting_target_comp = true;
            }
        }

        if !unreachable_blanks.is_empty() && !reachable_blanks.is_empty() {
            // Local-gate rule for the goal's island: an island-side endpoint
            // inside the goal's component must leave a lockable tile between
            // itself and the goal, so a future GoalGate lock can strand just
            // the goal's side of that island — NOT the entire downstream
            // chain (an upstream choke technically strands the goal too, but
            // its strand swallows most fort slots, which starves GoalGate's
            // "strands no fort" requirement). `good_b` = island nodes that
            // some single lockable tile separates from the goal, walking
            // island-internally (no pipes). Fallback: if the rule would empty
            // the pool, allow any endpoint — completability first.
            let target_island: Option<HashSet<Pos>> = target_pos
                .filter(|tp| must_connect_target && !walk.nodes.contains(tp))
                .map(|tp| walk_map(grid, &[], Some(tp), world_idx).nodes);
            let good_b: HashSet<Pos> = match &target_island {
                Some(island) if unreachable_blanks.iter().any(|b| island.contains(b)) => {
                    let tp = target_pos.unwrap();
                    let island_walk = walk_map(grid, &[], Some(tp), world_idx);
                    let mut out = HashSet::new();
                    for &t in &island_walk.path_tiles {
                        if !LOCKABLE_TILES.contains(&grid.get(t.0, t.1)) {
                            continue;
                        }
                        let mut g = grid.clone();
                        g.set(t.0, t.1, gap_tile_for(g.get(t.0, t.1)));
                        let still = walk_map(&g, &[], Some(tp), world_idx).nodes;
                        out.extend(island.iter().filter(|n| !still.contains(*n)).copied());
                    }
                    out
                }
                _ => HashSet::new(),
            };
            let locally_gateable = |b: &Pos| match &target_island {
                Some(island) if island.contains(b) => good_b.contains(b),
                _ => true, // not the goal's island — no local rule
            };
            let gated_pool: Vec<Pos> = unreachable_blanks
                .iter()
                .copied()
                .filter(locally_gateable)
                .collect();
            let b_pool: &Vec<Pos> = if gated_pool.is_empty() { &unreachable_blanks } else { &gated_pool };

            // Island side: nearest island first (chain growth, as shared).
            let b_scored: Vec<(Pos, f64)> = b_pool
                .iter()
                .map(|&b| {
                    let frontier_dist = reachable_blanks
                        .iter()
                        .map(|&a| (a.0.abs_diff(b.0) + a.1.abs_diff(b.1)) as f64)
                        .fold(f64::INFINITY, f64::min);
                    let proximity = (knobs.frontier_max_dist
                        - frontier_dist.min(knobs.frontier_max_dist))
                        / knobs.frontier_max_dist
                        * knobs.frontier_weight;
                    (b, proximity)
                })
                .collect();
            let b = pick_softmax_by_score(b_scored, knobs.softmax_t, rng).unwrap();

            // Reachable side: shortest bridge, filtered by the mission
            // topology rules in strict-to-loose order. Completability never
            // depends on the filters — the unfiltered pool is the last rung.
            //
            // Rule 1 (gate preservation): prefer sources OUTSIDE the goal's
            // best gate — the smallest current strand containing the goal.
            // Islands attached inside a gater's strand get all their fort
            // slots stranded along with the goal, which starves GoalGate's
            // "strands no fort" requirement (measured on SAS W7: every
            // 2-fort shape failed because only ~1 slot survived outside).
            // Rule 2 (choked entrance): prefer sources some lock strands, so
            // the island itself stays gateable for chain links / fork homes.
            let strands = lock_strands(grid, &placed_pairs, start_pos, world_idx);
            let strandable: HashSet<Pos> = strands.iter().flatten().copied().collect();
            let best_goal_strand: Option<&HashSet<Pos>> = target_pos.and_then(|tp| {
                strands.iter().filter(|s| s.contains(&tp)).min_by_key(|s| s.len())
            });
            let ok_strand = |p: &Pos| best_goal_strand.is_none_or(|s| !s.contains(p));
            let choke = |p: &Pos| strandable.contains(p);
            let base_pool: Vec<Pos> = reachable_blanks.clone();
            let pools: [Vec<Pos>; 4] = [
                base_pool.iter().copied().filter(|p| ok_strand(p) && choke(p)).collect(),
                base_pool.iter().copied().filter(ok_strand).collect(),
                base_pool.iter().copied().filter(choke).collect(),
                base_pool.clone(),
            ];
            let mut pool = pools.iter().find(|p| !p.is_empty()).unwrap();

            // No-express tiebreak: when this pipe reaches the goal's
            // component, avoid sourcing it from the start island if the
            // chosen pool allows it (the goal shouldn't hang one teleport
            // off the starting island when the map offers depth).
            let indirect_pool: Vec<Pos>;
            if connecting_target_comp {
                indirect_pool = pool
                    .iter()
                    .copied()
                    .filter(|p| !start_island.contains(p))
                    .collect();
                if !indirect_pool.is_empty() {
                    pool = &indirect_pool;
                }
            }

            let a_scored: Vec<(Pos, f64)> = pool
                .iter()
                .map(|&a| {
                    let d = (a.0.abs_diff(b.0) + a.1.abs_diff(b.1)) as f64;
                    (a, -d)
                })
                .collect();
            let a = pick_softmax_by_score(a_scored, knobs.softmax_t, rng).unwrap();

            grid.set(a.0, a.1, TILE_PIPE);
            grid.set(b.0, b.1, TILE_PIPE);
            used_positions.insert(a);
            used_positions.insert(b);
            placed_pairs.push((a, b));
        } else if must_connect_target {
            if !lifted {
                lifted = true;
                active = &relaxed;
                continue;
            }
            break;
        } else {
            // Connectivity satisfied; the rest of the budget becomes spare
            // pipes after locks exist.
            break;
        }
    }

    placed_pairs
}

/// Fixed pipe endpoints for a world (shared constant, filtered here so the
/// mission builder needs no knowledge of the table shape).
pub(super) fn fixed_pipe_endpoints(world_idx: usize) -> Vec<Pos> {
    FIXED_PIPE_ENDPOINTS
        .iter()
        .filter(|(wi, _)| *wi == world_idx)
        .map(|(_, pos)| *pos)
        .collect()
}

/// Positions excluded from pipe placement for a world (shared constant).
pub(super) fn pipe_excluded_positions(world_idx: usize) -> HashSet<Pos> {
    PIPE_EXCLUDED_POSITIONS
        .iter()
        .filter(|(wi, _)| *wi == world_idx)
        .map(|(_, pos)| *pos)
        .collect()
}
