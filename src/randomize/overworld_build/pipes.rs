//! Pipe endpoint placement.

use super::*;

use super::knobs::{FortSkipPolicy, PipeScoring};
use super::route_choice::{C1_FLOOR, DEFAULT_SLACK, RouteChoice, measure_counts};
use super::types::BuiltWorld;

/// Score adjustment per in-band route a spare pipe CREATES (measured by
/// `analyze_route_choice`). Sized to dominate the level-skip/jump score
/// (range ~0-50) — a pipe that hands the player a real route decision beats
/// any plain shortcut.
const CHOICE_PIPE_BONUS: f64 = 100.0;
/// Score penalty when a spare pipe DESTROYS in-band routes — a soft veto:
/// softmax practically never picks it while alternatives exist, but it stays
/// available so the world can still reach its vanilla pipe count.
const CHOICE_PIPE_VETO: f64 = -1000.0;
/// Candidate pairs measured per spare-pipe round. Bounds the per-round
/// Dijkstra budget.
const CHOICE_MEASURES_PER_ROUND: usize = 10;
use super::scoring::pick_softmax_by_score;
use super::sections::split_blanks_by_reachability;
use super::types::{LockAssignment, SlotAssignment, SlotKind, stamp_slots};

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

/// Worlds that keep the OLD nearest-frontier (shortest-bridge) island endpoint
/// instead of the junction-preference one (issue #121). Junction preference
/// helps every other world's route choice, but on a compact world whose
/// central (high walk-degree) island tiles sit next to the airship, landing
/// the mandatory island bridge on that junction builds a near-start→near-goal
/// express that dominates every walking route. Measured: W4 regresses 20%→39%
/// linear under junction endpoints while W5/W7/W8 all improve, and no
/// world-agnostic knob (bridge penalty, goal-distance bias) separated the two
/// cases — the split is geometric, so W4 simply opts out. See
/// `test_pipe_probe` for the per-seed mechanism.
const PROXIMITY_ENDPOINT_WORLDS: &[usize] = &[
    3, // W4 (Giant Land) — junction island tiles are goal-adjacent
];

/// Nearest-frontier island score (proximity opt-out only): Manhattan distance
/// at which the score saturates to zero, and the weight it scales to. Retained
/// from the pre-#121 scorer so opted-out worlds keep their exact behavior.
const FRONTIER_CAP: f64 = 20.0;
const FRONTIER_WEIGHT: f64 = 5.0;

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
    knobs: &PipeScoring,
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
            // Build outward: bridge the reachable frontier to the nearest
            // unreachable island. The ISLAND endpoint lands on a JUNCTION rather
            // than the closest dead-end tip (issue #121). A connectivity pipe is
            // placed before any level exists, so the old scorer took the island
            // blank nearest the frontier — usually a corridor tip — and the
            // island joined as a one-way spur (a spanning tree, the most linear
            // shape there is). Landing on a high walk-degree tile instead
            // integrates the island as a routable subgraph the later measured
            // passes can fork. A mild per-hop bridge penalty keeps the junction
            // bias from reaching across the map. Worlds in
            // PROXIMITY_ENDPOINT_WORLDS opt out (see that constant).
            let use_proximity = PROXIMITY_ENDPOINT_WORLDS.contains(&world_idx);
            let island_score = |b: (usize, usize), frontier_dist: f64| -> f64 {
                if use_proximity {
                    // Nearer the frontier = higher (shortest bridge), saturated.
                    (FRONTIER_CAP - frontier_dist.min(FRONTIER_CAP)) / FRONTIER_CAP
                        * FRONTIER_WEIGHT
                } else {
                    walk_degree(grid, b) as f64 * knobs.junction_weight
                        - frontier_dist * knobs.bridge_penalty
                }
            };
            let b_scored: Vec<((usize, usize), f64)> = unreachable_blanks
                .iter()
                .map(|&b| {
                    let frontier_dist = reachable_blanks
                        .iter()
                        .map(|&a| (a.0.abs_diff(b.0) + a.1.abs_diff(b.1)) as f64)
                        .fold(f64::INFINITY, f64::min);
                    (b, island_score(b, frontier_dist))
                })
                .collect();
            let b = pick_softmax_by_score(b_scored, knobs.softmax_t, rng).unwrap();

            // Frontier (mainland) side: the blank nearest b — a short bridge, so
            // the pipe extends the frontier to the island rather than reaching
            // back across the map. Junction preference here proved both worse
            // for choice and slower (distant central bridges cost the downstream
            // measure passes ~5ms/build), so the mainland mouth stays nearest-b
            // on every world.
            let a_scored: Vec<((usize, usize), f64)> = reachable_blanks
                .iter()
                .map(|&a| {
                    let d = (a.0.abs_diff(b.0) + a.1.abs_diff(b.1)) as f64;
                    (a, -d) // nearer to b = higher
                })
                .collect();
            let a = pick_softmax_by_score(a_scored, knobs.softmax_t, rng).unwrap();

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

/// Place `spare_needed` spare pipe pairs after `populate_sections` AND
/// `place_locks`, converting the lowest-value filler (HammerBro) slots into
/// pipe endpoints. Unlike the connectivity phase this runs with levels already
/// placed, so each pair is scored by how many level slots it lets the player
/// skip: a pipe from a near-start node to a far node teleports past every
/// level on the stretch between them (approximated by route-distance band).
/// Endpoints are drawn only from HammerBro slots — never levels or forts —
/// and both are already reachable, so a spare pipe can never strand content;
/// it only shortcuts. Pairs that skip nothing are still placed (a fall-back
/// keeps the world at its fixed vanilla pipe count).
///
/// Because locks are already placed, every pair is also checked against them:
/// a pair that would bypass exactly ONE mandatory fortress is the fort-skip
/// prize and is taken whenever one exists (at most once per world); a pair
/// that would bypass two or more fortresses, or make the goal reachable with
/// every lock still closed, is rejected outright.
// Reason: each argument is a distinct placement input (grid, slots to convert,
// the pipe list to extend, budget, reserved sprite tiles, locks, anchors,
// world, RNG). They don't form a cohesive concept, so bundling adds
// indirection without clarity — same call shape as `place_pipes`.
#[allow(clippy::too_many_arguments)]
pub(super) fn place_spare_pipes<R: Rng>(
    grid: &mut Grid,
    slots: &mut [SlotAssignment],
    pipe_pairs: &mut Vec<TeleportEdge>,
    spare_needed: usize,
    reserved: &HashSet<(usize, usize)>,
    locks: &[LockAssignment],
    knobs: &PipeScoring,
    start_pos: Option<(usize, usize)>,
    target_pos: Option<(usize, usize)>,
    world_idx: usize,
    rng: &mut R,
) {
    if spare_needed == 0 {
        return;
    }

    // Pipe-sparse worlds (vanilla budget <= 2 pairs) get the HARD choice
    // veto: a spare that collapses an in-band route is banned and selection
    // retries, instead of the soft score veto that can still lose to a weak
    // field. Measured A/B (1000 seeds): dramatically lifts choice on the
    // sparse worlds (W2 36→24% linear, W4 31→16, W5 24→15, W6 8→2; overall
    // 46→41%) and skips their pool measures, but the same veto REGRESSES
    // pipe-heavy worlds (W3/W7/W8) — there, shortcut webs are the choice
    // fabric and the measured bonus/soft-veto pool below does better. Gate
    // on the world's actual budget, not its index, so future budget changes
    // follow automatically.
    let hard_choice_veto = pipe_pairs.len() + spare_needed <= 2;

    // Rows 7 and 8 share a Map_Completions bit, and a pipe is
    // completion-relevant — the same engine bug `place_locks` guards against.
    // Locks no longer see future spare pipes, so the exclusion runs this way
    // instead: never convert a slot whose row-7/8 partner column holds a lock.
    let lock_row78_conflicts: HashSet<(usize, usize)> = locks
        .iter()
        .filter_map(|l| match l.pos.0 {
            7 => Some((8, l.pos.1)),
            8 => Some((7, l.pos.1)),
            _ => None,
        })
        .collect();

    // Convertible endpoints: HammerBro filler slots, minus any reserved
    // (mandatory HB sprite) positions that must keep their sprite. Kept as an
    // ordered Vec (slot order) — candidate order feeds softmax sampling, so it
    // must be deterministic across runs, unlike HashSet iteration.
    let mut available: Vec<(usize, usize)> = slots
        .iter()
        .filter(|s| {
            s.kind == SlotKind::HammerBro
                && !reserved.contains(&s.pos)
                && !lock_row78_conflicts.contains(&s.pos)
        })
        .map(|s| s.pos)
        .collect();

    let level_positions: Vec<(usize, usize)> = slots
        .iter()
        .filter(|s| s.kind == SlotKind::Level)
        .map(|s| s.pos)
        .collect();

    // At most one fort skip per world — the prize stays special.
    let mut fort_skip_placed = false;

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

        // Per-lock start-side / goal-side components with that lock closed
        // and every other lock open (the progression state in which beating
        // that fort is the player's obstacle). A candidate pipe whose
        // endpoints straddle the two components makes the fort optional.
        // Only locks that actually gate the goal in that state count —
        // decoy/safe locks yield None and are skipped.
        let mut stamped = grid.clone();
        stamp_slots(&mut stamped, slots);
        let split_components = |g: &Grid| -> Option<(HashSet<Pos>, HashSet<Pos>)> {
            let tp = target_pos?;
            let comp_start = walk_map(g, pipe_pairs, start_pos, world_idx).nodes;
            if comp_start.contains(&tp) {
                return None; // goal reachable in this state — nothing to bypass
            }
            let comp_target = walk_map(g, pipe_pairs, Some(tp), world_idx).nodes;
            Some((comp_start, comp_target))
        };
        let lock_components: Vec<(HashSet<Pos>, HashSet<Pos>)> = locks
            .iter()
            .filter_map(|l| {
                let mut g = stamped.clone();
                g.set(l.pos.0, l.pos.1, l.gap_tile);
                split_components(&g)
            })
            .collect();
        // All-locks-closed split: a pipe straddling THIS one makes the goal
        // reachable with zero forts beaten — always rejected.
        let all_closed_grid = {
            let mut g = stamped.clone();
            for l in locks {
                g.set(l.pos.0, l.pos.1, l.gap_tile);
            }
            g
        };
        let all_closed_components = split_components(&all_closed_grid);
        // Per-lock closed grids, kept for the exact verification below.
        let lock_grids: Vec<Grid> = locks
            .iter()
            .map(|l| {
                let mut g = stamped.clone();
                g.set(l.pos.0, l.pos.1, l.gap_tile);
                g
            })
            .collect();
        let straddles = |(cs, ct): &(HashSet<Pos>, HashSet<Pos>), a: Pos, b: Pos| {
            (cs.contains(&a) && ct.contains(&b)) || (cs.contains(&b) && ct.contains(&a))
        };

        let avail = &available;
        // Level-skip candidates: (pair, skipped levels, jump distance) for
        // every non-rejected pair that hops >=2 and bypasses no fort.
        let mut candidates: Vec<(TeleportEdge, usize, usize)> = Vec::new();
        // Fort-skip candidates: pairs that bypass exactly one mandatory fort.
        let mut fort_candidates: Vec<(TeleportEdge, f64)> = Vec::new();
        // All non-rejected pairs with their jump, for the last-resort pick.
        let mut allowed_pairs: Vec<(TeleportEdge, usize)> = Vec::new();
        for i in 0..avail.len() {
            for j in (i + 1)..avail.len() {
                let a = avail[i];
                let b = avail[j];
                let (da, db) = match (dist.get(&a), dist.get(&b)) {
                    (Some(&da), Some(&db)) => (da, db),
                    _ => continue,
                };
                let bypassed = lock_components.iter().filter(|c| straddles(c, a, b)).count();
                let opens_goal = all_closed_components
                    .as_ref()
                    .is_some_and(|c| straddles(c, a, b));
                let skip_allowed = knobs.fort_skip == FortSkipPolicy::AnyOneFort
                    && !fort_skip_placed;
                if opens_goal || bypassed >= 2 || (bypassed == 1 && !skip_allowed) {
                    continue;
                }
                let (lo, hi) = (da.min(db), da.max(db));
                // Levels whose route distance sits strictly between the two
                // endpoints are the ones the teleport lets the player skip.
                let skipped = level_d.iter().filter(|&&d| d > lo && d < hi).count();
                if bypassed == 1 {
                    fort_candidates.push((
                        (a, b),
                        skipped as f64 * knobs.level_skip_weight + (hi - lo) as f64 * knobs.jump_weight,
                    ));
                    continue;
                }
                if hi - lo >= 2 {
                    candidates.push(((a, b), skipped, hi - lo));
                }
                allowed_pairs.push(((a, b), hi - lo));
            }
        }

        // Choice-aware adjustment: measure how the top-scored candidate
        // pairs change the world's in-band route count. Creating a route is
        // rewarded heavily; collapsing one is soft-vetoed. This is where a
        // linear world earns its second route — the pipe is the only tool
        // that can fork single-corridor terrain.
        let section_count = slots.iter().filter(|s| s.kind == SlotKind::Fortress).count();
        let measure_with = |extra: Option<TeleportEdge>| -> RouteChoice {
            let mut pp = pipe_pairs.clone();
            pp.extend(extra);
            let world = BuiltWorld {
                world_idx,
                grid: grid.clone(),
                slots: slots.to_vec(),
                locks: locks.to_vec(),
                section_count,
                pipe_pairs: pp,
                hb_sprites: Vec::new(),
            };
            // Counts-only: this pass reads `best_cost` / `routes.len()` /
            // `reachable`, never a path.
            measure_counts(&world, DEFAULT_SLACK)
        };
        let rc_now = measure_with(None);
        // C1-floor guard target: a shortcut may not price the cheapest route
        // below the floor — or, on an already-deficient world, below where
        // it stands (`enforce_c1_floor` ran before this pass).
        let c1_target = rc_now.best_cost.min(C1_FLOOR);
        let in_band_now = rc_now.routes.len();
        let choice_adjust: HashMap<TeleportEdge, f64> = if hard_choice_veto {
            // Sparse world: the committed pair gets a real measured veto in
            // the verification loop instead — no pre-measured score pool.
            HashMap::new()
        } else {
            // Measurement pool, most-likely-choice-creators first. A pipe
            // creates an IN-BAND alternative mainly when it skips exactly one
            // level (net -2: the walk route stays 2 points behind, inside the
            // slack band) — big skips replace the best route instead of tying
            // it. So: 1-skip pairs (widest jumps first), then fort-skip
            // pairs, then the top plain scores (to veto choice-destroyers
            // among the likely winners).
            let mut pool: Vec<TeleportEdge> = Vec::new();
            let mut one_skips: Vec<(TeleportEdge, usize)> = candidates
                .iter()
                .filter(|&&(_, s, _)| s == 1)
                .map(|&(p, _, j)| (p, j))
                .collect();
            one_skips.sort_by_key(|&(p, j)| (std::cmp::Reverse(j), p));
            pool.extend(one_skips.into_iter().map(|(p, _)| p).take(6));
            let mut forts_by_score = fort_candidates.clone();
            forts_by_score.sort_by(|a, b| {
                b.1.partial_cmp(&a.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.0.cmp(&b.0))
            });
            pool.extend(forts_by_score.into_iter().map(|(p, _)| p).take(3));
            let mut by_score: Vec<(TeleportEdge, f64)> = candidates
                .iter()
                .map(|&(p, s, j)| {
                    (p, s as f64 * knobs.level_skip_weight + j as f64 * knobs.jump_weight)
                })
                .collect();
            by_score.sort_by(|a, b| {
                b.1.partial_cmp(&a.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.0.cmp(&b.0))
            });
            pool.extend(by_score.into_iter().map(|(p, _)| p).take(3));
            pool.dedup_by_key(|p| *p); // adjacent dups only; full dedup below via map insert
            pool.truncate(CHOICE_MEASURES_PER_ROUND);

            let mut adj_map: HashMap<TeleportEdge, f64> = HashMap::new();
            for pair in pool {
                if adj_map.contains_key(&pair) {
                    continue;
                }
                let delta = measure_with(Some(pair)).routes.len() as i64 - in_band_now as i64;
                let adj = if delta < 0 {
                    CHOICE_PIPE_VETO
                } else {
                    delta as f64 * CHOICE_PIPE_BONUS
                };
                adj_map.insert(pair, adj);
            }
            adj_map
        };
        let adjusted = |pair: TeleportEdge, base: f64| -> f64 {
            base + choice_adjust.get(&pair).copied().unwrap_or(0.0)
        };

        // Exact pre-walks for verification (candidate-independent, per round).
        let exact_state: Option<(bool, Vec<bool>)> = match (start_pos, target_pos) {
            (Some(sp), Some(tp)) => {
                let sealed = !walk_map(&all_closed_grid, pipe_pairs, Some(sp), world_idx)
                    .nodes
                    .contains(&tp);
                let lock_sealed: Vec<bool> = lock_grids
                    .iter()
                    .map(|g| !walk_map(g, pipe_pairs, Some(sp), world_idx).nodes.contains(&tp))
                    .collect();
                Some((sealed, lock_sealed))
            }
            _ => None,
        };

        // Selection + EXACT verification. The component-straddle model above
        // assumes a candidate pipe only merges its two endpoints' components.
        // The canoe systems (W3; W8 under `8s are Wild`) break that: walk_map
        // enables canoe edges only while the mainland dock is reachable, so
        // one pipe can open routes far from either endpoint (observed: W3
        // "goal open at start" worlds where a level-skip spare silently
        // opened the goal through the canoe network). Every winning pair is
        // re-checked with real walks; a pair the exact model rejects is
        // banned and selection re-runs without it.
        let mut banned: HashSet<TeleportEdge> = HashSet::new();
        // Hard-veto choice bans and floor bans are SOFT overall: tracked
        // separately so they can be lifted if they'd leave the world short
        // of its vanilla pipe budget — the budget outranks both. Choice
        // lifts first (the weaker rule); the C1 floor holds out longest.
        let mut choice_banned: Vec<TeleportEdge> = Vec::new();
        let mut choice_active = hard_choice_veto;
        // Bound the hard-veto retry churn: after this many banned pairs in
        // one round, fall back to the unmeasured pick (worst-case seeds
        // otherwise pay a measure per banned candidate).
        let mut choice_vetoes_left = 8usize;
        let mut floor_banned: Vec<TeleportEdge> = Vec::new();
        let mut floor_active = true;
        let mut verified_fort_skip = false;
        let chosen: Option<TeleportEdge> = loop {
            // A fort skip is the prize: take one whenever it exists (softmax
            // breaks ties toward the pair that also skips the most levels).
            let fc: Vec<(TeleportEdge, f64)> = fort_candidates
                .iter()
                .filter(|(p, _)| !banned.contains(p))
                .map(|&(p, s)| (p, adjusted(p, s)))
                .collect();
            let pick = if !fc.is_empty() {
                pick_softmax_by_score(fc, knobs.softmax_t, rng)
            } else {
                // Roll this pipe's skip cap from the weighted distribution.
                let cap = {
                    let total: f64 = knobs.spare_cap_weights.iter().sum();
                    let mut roll = rng.random_range(0.0..total);
                    let mut c = knobs.spare_cap_weights.len();
                    for (i, w) in knobs.spare_cap_weights.iter().enumerate() {
                        roll -= w;
                        if roll <= 0.0 {
                            c = i + 1;
                            break;
                        }
                    }
                    c
                };

                // Greedily take the biggest skip within the cap (softmax
                // breaks near ties). If every shortcut skips more than the
                // cap, take the smallest one (least overshoot). Fall back to
                // the most distance-separated pair if nothing qualified, so
                // the world still reaches its fixed vanilla pipe count.
                let within: Vec<(TeleportEdge, f64)> = candidates
                    .iter()
                    .filter(|(p, s, _)| {
                        !banned.contains(p)
                            && ((1..=cap).contains(s)
                                // A measured route-creating pair ignores the
                                // skip cap: handing the player a decision
                                // outranks capping shortcut size.
                                || choice_adjust.get(p).is_some_and(|&adj| adj > 0.0))
                    })
                    .map(|&(p, s, j)| {
                        let base = s as f64 * knobs.level_skip_weight + j as f64 * knobs.jump_weight;
                        (p, adjusted(p, base))
                    })
                    .collect();
                pick_softmax_by_score(within, knobs.softmax_t, rng)
                    .or_else(|| {
                        candidates
                            .iter()
                            .filter(|(p, s, _)| !banned.contains(p) && *s >= 1)
                            .min_by_key(|&&(_, s, _)| s)
                            .map(|&(p, _, _)| p)
                    })
                    .or_else(|| {
                        allowed_pairs
                            .iter()
                            .filter(|(p, _)| !banned.contains(p))
                            .max_by_key(|&&(_, j)| j)
                            .map(|&(p, _)| p)
                    })
            };
            let Some(pair) = pick else {
                if !choice_banned.is_empty() {
                    for p in choice_banned.drain(..) {
                        banned.remove(&p);
                    }
                    choice_active = false;
                    continue;
                }
                if floor_active && !floor_banned.is_empty() {
                    for p in floor_banned.drain(..) {
                        banned.remove(&p);
                    }
                    floor_active = false;
                    continue;
                }
                break None;
            };

            // Exact verification with real walks.
            let Some((sealed, lock_sealed)) = &exact_state else { break Some(pair) };
            let (Some(sp), Some(tp)) = (start_pos, target_pos) else { break Some(pair) };
            let mut trial: Vec<TeleportEdge> = pipe_pairs.clone();
            trial.push(pair);
            let opens_goal = *sealed
                && walk_map(&all_closed_grid, &trial, Some(sp), world_idx).nodes.contains(&tp);
            if opens_goal {
                banned.insert(pair);
                continue;
            }
            let bypass = lock_grids
                .iter()
                .zip(lock_sealed.iter())
                .filter(|(g, was_sealed)| {
                    **was_sealed
                        && walk_map(g, &trial, Some(sp), world_idx).nodes.contains(&tp)
                })
                .count();
            let skip_allowed =
                knobs.fort_skip == FortSkipPolicy::AnyOneFort && !fort_skip_placed;
            if bypass >= 2 || (bypass == 1 && !skip_allowed) {
                banned.insert(pair);
                continue;
            }
            // Measured winner checks (one route measure per tried pair):
            // on sparse worlds the shortcut may not collapse an in-band
            // route (hard choice veto), and everywhere it may not price the
            // world below the C1 floor (or cheapen an already-deficient
            // world further).
            if rc_now.reachable && (choice_active || floor_active) {
                let m = measure_with(Some(pair));
                if choice_active && m.routes.len() < in_band_now {
                    banned.insert(pair);
                    choice_banned.push(pair);
                    choice_vetoes_left -= 1;
                    if choice_vetoes_left == 0 {
                        // Measure budget spent: keep the known collapsers
                        // banned (the budget-lift below can still restore
                        // them), but stop paying for further measures.
                        choice_active = false;
                    }
                    continue;
                }
                if floor_active && m.best_cost < c1_target {
                    banned.insert(pair);
                    floor_banned.push(pair);
                    continue;
                }
            }
            verified_fort_skip = bypass == 1;
            break Some(pair);
        };

        let (a, b) = match chosen {
            Some(pair) => pair,
            None => break,
        };
        if verified_fort_skip {
            fort_skip_placed = true;
        }

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
