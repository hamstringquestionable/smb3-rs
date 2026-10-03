//! Island decomposition and roles.
//!
//! An "island" is a connected component of the walk network with NO pipe
//! edges — the fixed terrain pockets the pipe web stitches together. The
//! decomposition is computed fresh from the state whenever needed, so SAS
//! swaps and map-editing QOL flags come out right for free.
//!
//! Roles (user design, 2026-08-01, from the W7 anatomy review): the start
//! and target islands have inherent roles; the rest classify by size —
//! tiny islands are utility flavor (a troll pipe, a fort/toad-house slot),
//! mid islands are corridors (a level or two, then pipe out), and big
//! islands are where ROUTING belongs: mouths spread far apart so traversal
//! crosses the interior, forks with room for level-bearing arms. W7 is the
//! richest customer (islands of 1/2/4/6/8/9/13 nodes — pinned in
//! `test_builder_island_roles`), but the rule is generic: W5's twin
//! 19/20 islands, W8's 17-node hub, W4's mainland all classify sensibly,
//! and single-island worlds no-op.

use super::*;

/// What an island is FOR. Size thresholds are structural classification
/// (not tuning weights): <= 2 utility, 3-5 corridor, >= 6 routing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum IslandRole {
    /// Holds the start. Mouth placement stays uniform (the anchor bar
    /// already keeps pipes off the player's doorstep).
    Entry,
    /// Holds the target. Big enough to want spread mouths (vanilla W7's
    /// final island has two entry doors).
    Final,
    /// 1-2 nodes: troll pipe, fort / toad-house / hammer-bro slot.
    Utility,
    /// 3-5 nodes: push the player through a level or two, then pipe out.
    Corridor,
    /// 6+ nodes: the choice real estate — spread mouths, fork material.
    Routing,
}

pub(super) struct Island {
    // Reason: only the anatomy pinning test (cfg(test)) reads the size
    // today; it stays on the struct as the island's defining datum.
    #[allow(dead_code)]
    pub size: usize,
    pub role: IslandRole,
}

impl Island {
    /// Spread-mouths preference applies: traversal should cross the
    /// island instead of entering and leaving through the same corner.
    pub(super) fn wants_spread_mouths(&self) -> bool {
        matches!(self.role, IslandRole::Routing | IslandRole::Corridor | IslandRole::Final)
    }
}

/// Pocket decomposition: connected components of the walk network with NO
/// pipe edges. Canoe edges still count when the canonical walker says they
/// do (dock walk-reachable from the seed) — canoes are terrain, not budget.
/// Covers every position the builder places on: blanks, slots, pipe mouths,
/// start, target. Returns position -> island id and the island count.
/// Start seeds first (the mainland pocket claims its canoe islands before
/// an island seed can claim itself), then scan order — seed-stable.
pub(super) fn pocket_map(state: &WorldState) -> (HashMap<Pos, usize>, usize) {
    let mut seeds: Vec<Pos> = blank_positions(state);
    seeds.extend(state.slots.iter().map(|s| s.pos));
    for &(a, b) in &state.pipe_pairs {
        seeds.push(a);
        seeds.push(b);
    }
    seeds.extend(state.target);
    seeds.sort_unstable();
    seeds.dedup();
    if let Some(start) = state.start {
        seeds.retain(|&p| p != start);
        seeds.insert(0, start);
    }
    let mut pocket: HashMap<Pos, usize> = HashMap::new();
    let mut count = 0;
    for &seed in &seeds {
        if pocket.contains_key(&seed) {
            continue;
        }
        let reach = walk_reachable(&state.grid, &[], Some(seed), state.world_idx);
        for &q in &seeds {
            if !pocket.contains_key(&q) && reach.contains(q) {
                pocket.insert(q, count);
            }
        }
        count += 1;
    }
    (pocket, count)
}

/// All placeable blank tiles on the grid (pickup's blank-tile contract),
/// including fixed ones.
pub(super) fn blank_positions(state: &WorldState) -> Vec<Pos> {
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

/// The pocket pairs already joined by a direct pipe, normalized (lo, hi).
/// Intra-pocket pairs appear as (p, p) — harmless to callers, which only
/// test cross-pocket candidates.
pub(super) fn linked_pocket_pairs(
    pocket: &HashMap<Pos, usize>,
    pipe_pairs: &[TeleportEdge],
) -> HashSet<(usize, usize)> {
    pipe_pairs
        .iter()
        .filter_map(|&(a, b)| {
            let (&pa, &pb) = (pocket.get(&a)?, pocket.get(&b)?);
            Some((pa.min(pb), pa.max(pb)))
        })
        .collect()
}

/// Classify every island. Start precedence over target for the (single
/// island) world where one component holds both — nothing consumes roles
/// there anyway.
pub(super) fn classify(
    state: &WorldState,
    pocket: &HashMap<Pos, usize>,
    count: usize,
) -> Vec<Island> {
    let mut sizes: Vec<usize> = vec![0; count];
    for &id in pocket.values() {
        sizes[id] += 1;
    }
    sizes
        .into_iter()
        .enumerate()
        .map(|(id, size)| {
            let has = |p: Option<Pos>| p.is_some_and(|p| pocket.get(&p) == Some(&id));
            let role = if has(state.start) {
                IslandRole::Entry
            } else if has(state.target) {
                IslandRole::Final
            } else if size <= 2 {
                IslandRole::Utility
            } else if size <= 5 {
                IslandRole::Corridor
            } else {
                IslandRole::Routing
            };
            Island { size, role }
        })
        .collect()
}

/// Existing pipe mouths on island `id`.
pub(super) fn island_mouths(
    pocket: &HashMap<Pos, usize>,
    pipe_pairs: &[TeleportEdge],
    id: usize,
) -> Vec<Pos> {
    pipe_pairs.iter().flat_map(|&(a, b)| [a, b]).filter(|p| pocket.get(p) == Some(&id)).collect()
}

/// Spread-mouths refinement: `picked` was chosen uniformly (that keeps the
/// cross-island distribution as-is); if its island wants spread mouths and
/// already has some, move the pick to the candidate ON THE SAME ISLAND
/// farthest (in-island walk distance, no pipes) from the nearest existing
/// mouth. Ties keep `picked` when it is among the best, else resolve by
/// candidate order (the caller's list is deterministic). Everything else —
/// mouthless islands, Entry/Utility roles, foreign candidates — returns
/// `picked` unchanged.
pub(super) fn spread_mouth(
    state: &WorldState,
    pocket: &HashMap<Pos, usize>,
    islands: &[Island],
    candidates: &[Pos],
    picked: Pos,
) -> Pos {
    let Some(&id) = pocket.get(&picked) else { return picked };
    if !islands[id].wants_spread_mouths() {
        return picked;
    }
    let mouths = island_mouths(pocket, &state.pipe_pairs, id);
    if mouths.is_empty() {
        return picked;
    }
    // In-island distance: BFS from each mouth with no pipe edges; a
    // candidate's score is the distance to its NEAREST mouth.
    let dist_maps: Vec<HashMap<Pos, usize>> = mouths
        .iter()
        .map(|&m| walk_map(&state.grid, &[], Some(m), state.world_idx).distances)
        .collect();
    let score = |p: Pos| -> usize {
        dist_maps.iter().map(|d| d.get(&p).copied().unwrap_or(0)).min().unwrap_or(0)
    };
    let mut best = picked;
    let mut best_score = score(picked);
    for &c in candidates {
        if pocket.get(&c) == Some(&id) && score(c) > best_score {
            best = c;
            best_score = score(c);
        }
    }
    best
}
