//! Choice-first route scorer — deliberately SEPARATE from `progression.rs`
//! (which stays on its plain min-clears model) so the two can be compared.
//!
//! Weighted set-cost model, in plain points:
//!   pipe = 1 (per use)   level = 3 (once)   fort = 5 (once)   rock = 8 (once)
//!
//! Each clearable (fort / level / rock) is charged ONCE, via a cleared-bitmask
//! carried in the search state — so walking back through a played level, a
//! beaten fort, or a broken rock is free ("played once, broken once"). A lock
//! costs nothing at its tile, but you can only cross it once its fort is
//! beaten, and beating the fort is where the 5 lands — so a lock implicitly
//! drags in its fort's cost.
//!
//! A route's cost is that weighted set-cost. Two routes are the SAME route if
//! they play the same set of levels. "Roughly equal" = within `slack` points
//! (default 3 = one level).
//!
//! Enumeration is COMPLETE and needs no bans or backward pass: the cleared-mask
//! already records exactly which levels a route played, so we run ONE Dijkstra,
//! keep going until the popped cost exceeds `best + slack`, and read off every
//! goal-state settled within budget — each is a route (mask → level-set). A
//! domination filter then drops any route that only plays an EXTRA level for no
//! benefit (a within-slack detour), so those don't masquerade as real choices.

use super::*;
use super::types::{BuiltWorld, SlotKind, stamp_slots};

use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap};

/// Weighted set-cost knobs, in plain points. Legible on purpose.
pub(crate) const COST_PIPE: u32 = 1;
pub(crate) const COST_LEVEL: u32 = 3;
pub(crate) const COST_FORT: u32 = 5;
pub(crate) const COST_ROCK: u32 = 8;
/// "Roughly equal" band, in points. 3 = one level of wobble.
pub(crate) const DEFAULT_SLACK: u32 = 3;

/// Wider measuring band used while SHAPING a world (`shape_forts`): routes up
/// to 13 points above best stay visible — two stacked forts (+10) plus the
/// choice band (+3) — so any near-miss corridor the fort budget could rescue
/// is on the table. Final acceptance still uses `DEFAULT_SLACK`.
pub(crate) const SHAPING_SLACK: u32 = 13;

/// Breakable overworld rocks and the path tile they open into
/// ($51→$45 horizontal, $52→$46 vertical). $53 is unbreakable (stays a wall).
const BREAKABLE_ROCKS: [(u8, u8); 2] = [(0x51, 0x45), (0x52, 0x46)];

/// One distinct route to the goal.
#[derive(Clone, Debug)]
pub(crate) struct ChoiceRoute {
    /// Weighted set-cost, in points.
    pub cost: u32,
    /// Distinct levels played — the route's identity (dedup key).
    pub levels: BTreeSet<Pos>,
    /// Distinct forts beaten / rocks broken (breakdown for display).
    // Reason: dead_code — read only by the cfg(test) renderers and census
    // tests; kept in the production struct so the breakdown is computed once,
    // next to the mask that defines it.
    #[allow(dead_code)]
    pub forts: u32,
    #[allow(dead_code)]
    pub rocks: u32,
    /// Node path start..goal, for rendering.
    pub path: Vec<Pos>,
}

/// Per-world choice summary.
#[derive(Clone, Debug, Default)]
pub(crate) struct RouteChoice {
    // Reason: dead_code — the summary fields below are read only by the
    // cfg(test) dump/census consumers today; production code derives its own
    // views (in-band count, shaping gap) from `routes` + `best_cost`.
    #[allow(dead_code)]
    pub reachable: bool,
    pub best_cost: u32,
    /// Distinct non-dominated routes within `slack` of best, cheapest first.
    pub routes: Vec<ChoiceRoute>,
    /// How many routes tie at `best_cost`.
    #[allow(dead_code)]
    pub tied_at_best: usize,
    /// Cheapest strictly-worse in-band route minus best (the "gap"); `None`
    /// when there is no in-band alternative (LINEAR).
    #[allow(dead_code)]
    pub runner_up_gap: Option<u32>,
    /// DOMINATED routes (a strict superset of some kept route's levels at no
    /// lower cost — pure detours), cheapest first, capped. Not choices — but
    /// they are the shaping pass's raw material: a fort on the kept route's
    /// exclusive path stretch re-prices it, un-nesting the cost relation and
    /// turning the detour into a real alternative.
    pub detours: Vec<ChoiceRoute>,
}

/// Cap on the dominated-detour list — plenty for shaping, keeps the struct
/// small on detour-rich maps.
const MAX_DETOURS: usize = 8;

/// Rescue targets for the shaping passes, most-promising first: out-of-band
/// parallel routes and dominated superset detours. Either way, shifting the
/// cheap route's cost by +`COST_FORT` (a fort on its exclusive stretch, or a
/// fort-gated lock on its exclusive edge) lands targets whose gap is closest
/// to `COST_FORT` in the choice band most often.
pub(super) fn rescue_targets(rc: &RouteChoice) -> Vec<ChoiceRoute> {
    if rc.routes.is_empty() {
        return Vec::new();
    }
    let band = rc.best_cost + DEFAULT_SLACK;
    let mut targets: Vec<ChoiceRoute> = rc
        .routes
        .iter()
        .filter(|r| r.cost > band)
        .chain(rc.detours.iter().filter(|r| r.cost > rc.best_cost))
        .cloned()
        .collect();
    targets.sort_by_key(|r| ((r.cost - rc.best_cost).abs_diff(COST_FORT), r.cost));
    targets.truncate(3);
    targets
}

/// The mid path-tiles of a route's WALK edges (consecutive path nodes two
/// apart; teleport hops have no mid tile). The cheap route's exclusive mids
/// vs a rescue target are the golden LOCK sites: gapping one forces the
/// fort's cost onto the shortcut without touching the target route.
pub(super) fn walk_edge_mids(path: &[Pos]) -> HashSet<Pos> {
    path.windows(2)
        .filter_map(|w| {
            let (a, b) = (w[0], w[1]);
            if a.0 == b.0 && a.1.abs_diff(b.1) == 2 {
                Some((a.0, (a.1 + b.1) / 2))
            } else if a.1 == b.1 && a.0.abs_diff(b.0) == 2 {
                Some(((a.0 + b.0) / 2, a.1))
            } else {
                None
            }
        })
        .collect()
}

/// Enumerate every distinct near-optimal route to the goal and summarise the
/// choice they offer.
pub(crate) fn analyze_route_choice(built: &BuiltWorld, slack: u32) -> RouteChoice {
    // Working grid: open breakable rocks so walk_map makes edges through them
    // (we charge 8 the first time one is crossed, below); then stamp nodes.
    // Rocks are kept in grid scan order: their mask bits (assigned below) must
    // be seed-stable, since heap tie-breaking includes the mask and placement
    // decisions will key on the returned routes.
    let mut grid = built.grid.clone();
    let mut rock_tiles: Vec<Pos> = Vec::new();
    for r in 0..grid.rows() {
        for c in 0..grid.cols {
            let t = grid.get(r, c);
            for (closed, open) in BREAKABLE_ROCKS {
                if t == closed {
                    grid.set(r, c, open);
                    rock_tiles.push((r, c));
                }
            }
        }
    }
    stamp_slots(&mut grid, &built.slots);

    let (Some(start), Some(target)) = (
        rom_data::find_start(&grid),
        find_target(&grid, built.world_idx),
    ) else {
        return RouteChoice::default();
    };
    let walk = walk_map(&grid, &built.pipe_pairs, Some(start), built.world_idx);

    // Slot kind + section per node; lock path-tiles → the fort section that
    // opens them.
    let mut kind_at: HashMap<Pos, &SlotKind> = HashMap::new();
    let mut section_at: HashMap<Pos, usize> = HashMap::new();
    for slot in &built.slots {
        kind_at.insert(slot.pos, &slot.kind);
        section_at.insert(slot.pos, slot.section);
    }
    let mut lock_section: HashMap<Pos, usize> = HashMap::new();
    for lock in &built.locks {
        lock_section.insert(lock.pos, lock.fort_section);
    }

    // Cleared-mask bit layout (u64): fort sections use bits 0..section_count,
    // then one bit per level, then one bit per rock. `level_pos` inverts the
    // level bits so we can read a goal mask back into a level-set.
    let mut level_bit: HashMap<Pos, u32> = HashMap::new();
    let mut level_pos: Vec<(u32, Pos)> = Vec::new();
    let mut next = built.section_count as u32;
    for slot in &built.slots {
        if matches!(slot.kind, SlotKind::Level) {
            level_bit.insert(slot.pos, next);
            level_pos.push((next, slot.pos));
            next += 1;
        }
    }
    let mut rock_bit: HashMap<Pos, u32> = HashMap::new();
    for &rp in &rock_tiles {
        rock_bit.insert(rp, next);
        next += 1;
    }
    debug_assert!(next <= 64, "too many clearables for a u64 mask ({next})");
    let fort_bits: u64 = (0..built.section_count).map(|s| 1u64 << s).sum();
    let rock_bits: u64 = rock_bit.values().map(|&b| 1u64 << b).sum();

    // Node-entry cost: charge a level/fort the FIRST time (bit flip), free
    // after. Pipes/other transit nodes are free at the node — a pipe RIDE is
    // charged on its teleport edge instead.
    let node_cost = |dest: Pos, mask: u64| -> (u32, u64) {
        if dest == target {
            return (0, mask);
        }
        match kind_at.get(&dest) {
            Some(SlotKind::Fortress) => {
                let bit = 1u64 << section_at[&dest];
                if mask & bit == 0 { (COST_FORT, mask | bit) } else { (0, mask) }
            }
            Some(SlotKind::Level) => {
                let bit = 1u64 << level_bit[&dest];
                if mask & bit == 0 { (COST_LEVEL, mask | bit) } else { (0, mask) }
            }
            _ => (0, mask),
        }
    };

    // Canoe (boat) — same model as progression.rs: one boat, rides move it.
    let canoe_edges = rom_data::active_canoe_edges(built.world_idx, built.grid.eights_are_wild);
    let canoe_pair_set: HashSet<(Pos, Pos)> =
        canoe_edges.iter().flat_map(|&(a, b)| [(a, b), (b, a)]).collect();
    let initial_boat = canoe_edges.first().map(|&(a, _)| a);

    // Dijkstra over (pos, cleared-mask, boat). We do NOT stop at the first
    // goal: once `best` is known (the first goal popped, by cost order), we
    // keep going until the popped cost exceeds `best + slack`, collecting every
    // goal-state in that band. Each is a distinct route (its mask says which
    // levels/forts/rocks it used).
    type State = (Pos, u64, Option<Pos>);
    // (cost, pos, cleared-mask, boat) — cost first so the heap orders by it.
    type HeapEntry = Reverse<(u32, Pos, u64, Option<Pos>)>;
    let mut dist: HashMap<State, u32> = HashMap::new();
    let mut prev: HashMap<State, State> = HashMap::new();
    let mut heap: BinaryHeap<HeapEntry> = BinaryHeap::new();

    let init: State = (start, 0, initial_boat);
    dist.insert(init, 0);
    heap.push(Reverse((0, start, 0, initial_boat)));

    let mut best: Option<u32> = None;
    let mut goals: Vec<(State, u32)> = Vec::new(); // (goal state, cost)

    while let Some(Reverse((cost, pos, mask, boat))) = heap.pop() {
        let state = (pos, mask, boat);
        if cost > *dist.get(&state).unwrap_or(&u32::MAX) {
            continue;
        }
        if let Some(b) = best
            && cost > b + slack
        {
            break; // everything left is out of band
        }
        if pos == target {
            best.get_or_insert(cost);
            goals.push((state, cost));
            continue; // target is a sink — never expand through it
        }

        // Relax an edge: `edge_extra` is the pipe/rock surcharge and `mask_in`
        // is the mask after any rock-break on the crossed path tile; then add
        // the destination node's own cost.
        let mut relax = |dest: Pos, boat_after: Option<Pos>, edge_extra: u32, mask_in: u64| {
            let (nc, nm) = node_cost(dest, mask_in);
            let new_cost = cost + edge_extra + nc;
            let key = (dest, nm, boat_after);
            if new_cost < *dist.get(&key).unwrap_or(&u32::MAX) {
                dist.insert(key, new_cost);
                prev.insert(key, state);
                heap.push(Reverse((new_cost, dest, nm, boat_after)));
            }
        };

        if let Some(edges) = walk.edges.get(&pos) {
            for edge in edges {
                match edge.path_pos {
                    Some(pp) => {
                        // Lock: crossable only once its fort's section is open.
                        if let Some(&sec) = lock_section.get(&pp)
                            && mask & (1u64 << sec) == 0
                        {
                            continue;
                        }
                        // Rock: break it (charge once) the first time crossed.
                        let mut edge_extra = 0;
                        let mut mask_after = mask;
                        if let Some(&rb) = rock_bit.get(&pp) {
                            let bit = 1u64 << rb;
                            if mask_after & bit == 0 {
                                edge_extra += COST_ROCK;
                                mask_after |= bit;
                            }
                        }
                        relax(edge.dest, boat, edge_extra, mask_after);
                    }
                    None => {
                        // Teleport. Canoe rides handled below; here it's a pipe.
                        if canoe_pair_set.contains(&(pos, edge.dest)) {
                            continue;
                        }
                        relax(edge.dest, boat, COST_PIPE, mask);
                    }
                }
            }
        }

        // Canoe rides: free, and only when the boat sits at the current node.
        if boat == Some(pos) {
            for &(a, b) in &canoe_edges {
                let dest = if a == pos {
                    b
                } else if b == pos {
                    a
                } else {
                    continue;
                };
                relax(dest, Some(dest), 0, mask);
            }
        }
    }

    let Some(best_cost) = best else {
        return RouteChoice::default();
    };

    // Read each in-band goal state back into a route; keep the cheapest per
    // distinct level-set (remembering the state so we can rebuild its path).
    let mut by_levels: HashMap<BTreeSet<Pos>, (u32, State)> = HashMap::new();
    for (state, cost) in goals {
        let mask = state.1;
        let levels: BTreeSet<Pos> = level_pos
            .iter()
            .filter(|(bit, _)| mask & (1u64 << bit) != 0)
            .map(|&(_, pos)| pos)
            .collect();
        by_levels
            .entry(levels)
            .and_modify(|e| {
                if cost < e.0 {
                    *e = (cost, state);
                }
            })
            .or_insert((cost, state));
    }

    // Rebuild the node path for a goal state by walking `prev` back to start.
    let reconstruct = |goal: State| -> Vec<Pos> {
        let mut nodes = vec![goal.0];
        let mut cur = goal;
        while let Some(&p) = prev.get(&cur) {
            nodes.push(p.0);
            cur = p;
        }
        nodes.reverse();
        nodes
    };

    let mut routes: Vec<ChoiceRoute> = by_levels
        .into_iter()
        .map(|(levels, (cost, state))| ChoiceRoute {
            cost,
            levels,
            forts: (state.1 & fort_bits).count_ones(),
            rocks: (state.1 & rock_bits).count_ones(),
            path: reconstruct(state),
        })
        .collect();

    // Domination filter: drop a route that plays a strict superset of another
    // route's levels at no lower cost — a within-slack detour, not a real
    // choice. (Level-sets are already distinct, so subset ⇒ strict.)
    let dominated: Vec<bool> = routes
        .iter()
        .map(|ri| {
            routes
                .iter()
                .any(|rj| rj.cost <= ri.cost && rj.levels != ri.levels && rj.levels.is_subset(&ri.levels))
        })
        .collect();
    let (mut kept, mut detours): (Vec<ChoiceRoute>, Vec<ChoiceRoute>) = {
        let mut kept = Vec::new();
        let mut detours = Vec::new();
        for (r, dom) in routes.drain(..).zip(dominated) {
            if dom {
                detours.push(r);
            } else {
                kept.push(r);
            }
        }
        (kept, detours)
    };

    let by_cost_then_levels = |a: &ChoiceRoute, b: &ChoiceRoute| {
        a.cost
            .cmp(&b.cost)
            .then_with(|| a.levels.iter().cmp(b.levels.iter()))
    };
    kept.sort_by(by_cost_then_levels);
    detours.sort_by(by_cost_then_levels);
    detours.truncate(MAX_DETOURS);
    let tied_at_best = kept.iter().filter(|r| r.cost == best_cost).count();
    let runner_up_gap = kept
        .iter()
        .map(|r| r.cost)
        .find(|&c| c > best_cost)
        .map(|c| c - best_cost);

    RouteChoice {
        reachable: true,
        best_cost,
        routes: kept,
        tied_at_best,
        runner_up_gap,
        detours,
    }
}

/// One-line route-choice verdict for a world (eyeball diagnostic).
#[cfg(test)]
pub(crate) fn dump_route_choice(built: &BuiltWorld, slack: u32) {
    let rc = analyze_route_choice(built, slack);
    if !rc.reachable {
        eprintln!("  W{}: UNREACHABLE", built.world_idx + 1);
        return;
    }
    let costs: Vec<u32> = rc.routes.iter().map(|r| r.cost).collect();
    let verdict = if rc.routes.len() == 1 {
        "LINEAR — single best route".to_string()
    } else if let Some(gap) = rc.runner_up_gap {
        format!("{} tied at best, runner-up +{gap}", rc.tied_at_best)
    } else {
        format!("CHOICE — {} routes tied at {}", rc.tied_at_best, rc.best_cost)
    };
    eprintln!(
        "  W{}: {} route(s), costs {:?}  |  {}",
        built.world_idx + 1,
        rc.routes.len(),
        costs,
        verdict,
    );
}

/// ASCII render of every route through a world — one grid per route, the path
/// drawn on the map — so the enumeration can be eyeballed against the geometry.
#[cfg(test)]
pub(crate) fn render_route_choice(built: &BuiltWorld, slack: u32) -> String {
    use std::fmt::Write as _;
    let rc = analyze_route_choice(built, slack);

    let mut grid = built.grid.clone();
    stamp_slots(&mut grid, &built.slots);
    let mut kind_at: HashMap<Pos, &SlotKind> = HashMap::new();
    for slot in &built.slots {
        kind_at.insert(slot.pos, &slot.kind);
    }
    let start = rom_data::find_start(&grid);
    let target = find_target(&grid, built.world_idx);
    let goal_ch = if built.world_idx == 7 { 'B' } else { 'A' };

    // Bounding box of the non-background tiles, so we don't print a sea of blanks.
    let (mut r0, mut r1, mut c0, mut c1) = (grid.rows(), 0usize, grid.cols, 0usize);
    for r in 0..grid.rows() {
        for c in 0..grid.cols {
            if !BACKGROUND_TILES.contains(&grid.get(r, c)) {
                r0 = r0.min(r);
                r1 = r1.max(r);
                c0 = c0.min(c);
                c1 = c1.max(c);
            }
        }
    }

    let mut out = String::new();
    let _ = writeln!(
        out,
        "\n=== W{} — {} route(s), slack {slack} ===",
        built.world_idx + 1,
        rc.routes.len(),
    );
    if !rc.reachable {
        out.push_str("  UNREACHABLE\n");
        return out;
    }

    for (i, route) in rc.routes.iter().enumerate() {
        // Classify the cells the route touches: nodes, the walk-tile between
        // consecutive nodes, and pipe/canoe teleport endpoints.
        let nodes: HashSet<Pos> = route.path.iter().copied().collect();
        let mut mids: HashSet<Pos> = HashSet::new();
        let mut pipes: HashSet<Pos> = HashSet::new();
        for w in route.path.windows(2) {
            let (a, b) = (w[0], w[1]);
            if a.0 == b.0 && a.1.abs_diff(b.1) == 2 {
                mids.insert((a.0, (a.1 + b.1) / 2));
            } else if a.1 == b.1 && a.0.abs_diff(b.0) == 2 {
                mids.insert(((a.0 + b.0) / 2, a.1));
            } else {
                pipes.insert(a);
                pipes.insert(b);
            }
        }
        // Number the levels in the order they're played.
        let mut order: HashMap<Pos, usize> = HashMap::new();
        for &p in &route.path {
            if matches!(kind_at.get(&p), Some(SlotKind::Level)) && !order.contains_key(&p) {
                order.insert(p, order.len() + 1);
            }
        }

        let _ = writeln!(
            out,
            "\n  route {}/{}: cost {} ({}L {}F {}R)",
            i + 1,
            rc.routes.len(),
            route.cost,
            route.levels.len(),
            route.forts,
            route.rocks,
        );
        for r in r0..=r1 {
            out.push_str("  ");
            for c in c0..=c1 {
                let p = (r, c);
                let ch = if Some(p) == start {
                    'S'
                } else if Some(p) == target {
                    goal_ch
                } else if nodes.contains(&p) && matches!(kind_at.get(&p), Some(SlotKind::Fortress)) {
                    'F'
                } else if let Some(&n) = order.get(&p) {
                    if n <= 9 {
                        (b'0' + n as u8) as char
                    } else {
                        (b'a' + (n - 10) as u8) as char
                    }
                } else if pipes.contains(&p) {
                    'P'
                } else if mids.contains(&p) {
                    '*'
                } else if nodes.contains(&p) {
                    'o'
                } else if BACKGROUND_TILES.contains(&grid.get(r, c)) {
                    ' '
                } else {
                    '.'
                };
                out.push(ch);
                out.push(' ');
            }
            out.push('\n');
        }
    }
    out.push_str("  S=start  A/B=goal  F=fort  <n>=level(play order)  P=pipe  *=path  o=node\n");
    out
}
