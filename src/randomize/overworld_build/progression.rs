//! Required-progression analysis and diagnostic dumps.

// Reason: this whole module is exercised only by the test suite today
// (reserved for a future WASM single-seed dump), so in non-test builds
// everything here is dead code.
#![allow(dead_code)]

use super::*;

use super::types::{BuiltWorld, SlotKind, stamp_slots};

/// What occupies a grid position visited along the required path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PathNodeKind {
    Start,
    Level,
    Fortress { section: usize },
    Pipe,
    HammerBro,
    ToadHouse,
    BonusGame,
    Target,
    /// Position has no slot (e.g., a stray node tile). Should be rare.
    Unclassified,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RequiredProgression {
    /// Distinct fortress slots the player must clear (excludes the objective
    /// itself if it happens to live at a fortress tile).
    pub forts_required: usize,
    /// Distinct level slots the player must clear (excludes the objective).
    pub levels_required: usize,
    /// True when the airship/Bowser was reachable (always true on well-formed
    /// maps — false here would indicate a builder bug).
    pub reachable: bool,
    /// Ordered list of (position, kind) starting at start, ending at target.
    pub path: Vec<((usize, usize), PathNodeKind)>,
    /// Locks crossed during traversal, in path order: (lock_path_tile, fort_section).
    pub locks_crossed: Vec<((usize, usize), usize)>,
    /// Which section's lock the hammer pre-opened, if any. `None` means the
    /// hammer was not used (or the analysis was no-hammer).
    pub hammer_broke_section: Option<usize>,
    /// Longest run of back-to-back forced *level* plays along the required
    /// route with no other activity between them. A fortress, pipe transit,
    /// hammer-bro fight, or lock-poof resets the run; plain walking and
    /// toad-house/spade panels (rarely entered) do not. This is the "levels
    /// stacked with nothing to do between them" linearity signal.
    pub level_streak: usize,
    /// Trailing level run reaching the objective — how many forced levels sit
    /// right in front of the airship/Bowser with no fort/pipe/HB/lock between
    /// the run and the goal. `0` means an action (or lock) gates the final
    /// approach. A high value is the "clear path, just 2+ levels on the goal"
    /// complaint.
    pub goal_stack: usize,
}

/// Compute the minimum number of fortress/level entries the player must clear
/// to reach the world objective.
///
/// When `hammer` is true: the player has one hammer that can break exactly
/// ONE overworld lock for free. We try every individual lock-break and pick
/// the option that minimises total clears (including "don't use hammer").
pub(crate) fn analyze_required_progression(
    built: &BuiltWorld,
    hammer: bool,
) -> RequiredProgression {
    if !hammer {
        return analyze_with_pre_opened(built, None);
    }
    // Try (no break) ∪ {break each section}. Minimise total fort+level clears.
    let mut best = analyze_with_pre_opened(built, None);
    let mut best_cost = if best.reachable {
        best.forts_required + best.levels_required
    } else {
        usize::MAX
    };
    for section in 0..built.section_count {
        let mut candidate = analyze_with_pre_opened(built, Some(section));
        if !candidate.reachable {
            continue;
        }
        let cost = candidate.forts_required + candidate.levels_required;
        if cost < best_cost {
            best_cost = cost;
            candidate.hammer_broke_section = Some(section);
            best = candidate;
        }
    }
    best
}

/// Inner Dijkstra: returns the minimum-cost progression with `hammered_section`
/// pre-opened (if `Some`) or no locks pre-opened (`None`).
pub(super) fn analyze_with_pre_opened(
    built: &BuiltWorld,
    hammered_section: Option<usize>,
) -> RequiredProgression {
    let initial_mask: u32 = match hammered_section {
        Some(s) => 1u32 << s,
        None => 0,
    };
    analyze_with_pre_opened_mask(built, initial_mask)
}

/// Same as `analyze_with_pre_opened` but takes an arbitrary opened-section
/// mask. Useful for the all-locks-open sanity check in the dump.
pub(super) fn analyze_with_pre_opened_mask(
    built: &BuiltWorld,
    initial_mask: u32,
) -> RequiredProgression {
    // 1. Stamp slots onto a working grid so walk_map sees them as nodes.
    //    Skip locks — we model them as conditional edges instead.
    let mut grid = built.grid.clone();
    stamp_slots(&mut grid, &built.slots);

    let start = match rom_data::find_start(&grid) {
        Some(s) => s,
        None => return RequiredProgression::default(),
    };
    let target = match find_target(&grid, built.world_idx) {
        Some(t) => t,
        None => return RequiredProgression::default(),
    };

    let walk = walk_map(&grid, &built.pipe_pairs, Some(start), built.world_idx);

    // 2. Per-position slot info (skip the target; it's accounted for separately).
    let mut kind_at: HashMap<(usize, usize), &SlotKind> = HashMap::new();
    let mut section_at: HashMap<(usize, usize), usize> = HashMap::new();
    for slot in &built.slots {
        kind_at.insert(slot.pos, &slot.kind);
        section_at.insert(slot.pos, slot.section);
    }

    // 3. Lock lookup keyed on path-tile position.
    let mut lock_section: HashMap<(usize, usize), usize> = HashMap::new();
    for lock in &built.locks {
        lock_section.insert(lock.pos, lock.fort_section);
    }

    // 3b. Canoe edges for this world. There's one boat that starts at the
    //     mainland dock (the `a` side of each `(a, b)` tuple — all share the
    //     same mainland in vanilla). The boat moves WITH the player when they
    //     ride it: a canoe edge (X, Y) is only usable when the boat sits at
    //     X, and after the ride the boat is at Y. Walking/piping to an island
    //     without the boat leaves you stranded (no canoe edge usable from
    //     that island).
    let canoe_edges: Vec<((usize, usize), (usize, usize))> =
        rom_data::active_canoe_edges(built.world_idx, built.grid.eights_are_wild);
    let canoe_pair_set: HashSet<((usize, usize), (usize, usize))> = canoe_edges
        .iter()
        .flat_map(|&(a, b)| [(a, b), (b, a)])
        .collect();
    let initial_boat: Option<(usize, usize)> = canoe_edges.first().map(|&(a, _)| a);

    // 4. Dijkstra over (position, mask, boat_pos). Cost = node entries so far.
    //    Entering a fortress flips its section bit in the mask; riding a
    //    canoe moves the boat to the destination.
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;

    /// (position, opened-section-mask, boat-position-or-None)
    type SearchState = ((usize, usize), u32, Option<(usize, usize)>);
    type HeapEntry = Reverse<(usize, (usize, usize), u32, Option<(usize, usize)>)>;

    let mut dist: HashMap<SearchState, usize> = HashMap::new();
    let mut prev: HashMap<SearchState, SearchState> = HashMap::new();
    let mut heap: BinaryHeap<HeapEntry> = BinaryHeap::new();

    let initial: SearchState = (start, initial_mask, initial_boat);
    dist.insert(initial, 0);
    heap.push(Reverse((0, start, initial_mask, initial_boat)));

    let mut goal_state: Option<SearchState> = None;

    let entry_cost = |dest: (usize, usize)| -> (usize, bool) {
        // Returns (cost, is_fortress). is_fortress used by caller to update mask.
        if dest == target {
            return (1, false);
        }
        match kind_at.get(&dest) {
            Some(SlotKind::Fortress) => (1, true),
            Some(SlotKind::Level) => (1, false),
            _ => (0, false),
        }
    };

    while let Some(Reverse((cost, pos, mask, boat))) = heap.pop() {
        let state = (pos, mask, boat);
        if cost > *dist.get(&state).unwrap_or(&usize::MAX) {
            continue;
        }
        if std::env::var("TRACE_DIJKSTRA").is_ok() {
            eprintln!("    visit {pos:?} cost={cost} mask={mask:b} boat={boat:?}");
        }
        if pos == target {
            goal_state = Some(state);
            break;
        }

        // Relax one edge from the current state: entering a fortress flips
        // its section bit; standard Dijkstra decrease-key + heap push.
        let mut relax = |dest: (usize, usize), boat_after: Option<(usize, usize)>| {
            let (edge_cost, is_fort) = entry_cost(dest);
            let new_mask = if is_fort {
                mask | (1u32 << section_at[&dest])
            } else {
                mask
            };
            let key = (dest, new_mask, boat_after);
            let new_cost = cost + edge_cost;
            if new_cost < *dist.get(&key).unwrap_or(&usize::MAX) {
                dist.insert(key, new_cost);
                prev.insert(key, state);
                heap.push(Reverse((new_cost, dest, new_mask, boat_after)));
            }
        };

        // Walk / pipe edges from walk_map. Skip canoe edges — those are
        // handled below with explicit boat-state tracking.
        if let Some(edges) = walk.edges.get(&pos) {
            for edge in edges {
                if edge.path_pos.is_none() && canoe_pair_set.contains(&(pos, edge.dest)) {
                    continue;
                }
                // Lock-bearing path tile? Requires its section to be open.
                if let Some(path_pos) = edge.path_pos
                    && let Some(&section) = lock_section.get(&path_pos)
                    && mask & (1u32 << section) == 0
                {
                    continue;
                }
                relax(edge.dest, boat);
            }
        }

        // Canoe edges: usable only if the boat sits at the current position.
        // Riding moves the boat with the player to the destination.
        if boat == Some(pos) {
            for &(a, b) in &canoe_edges {
                let dest = if a == pos {
                    b
                } else if b == pos {
                    a
                } else {
                    continue;
                };
                relax(dest, Some(dest));
            }
        }
    }

    // 5. Reconstruct the path back from goal. Tally distinct fort/level
    //    positions (start and target excluded from counts), and record which
    //    locks were crossed (lookup edge.path_pos used per hop).
    let Some(final_state) = goal_state else {
        return RequiredProgression::default();
    };

    let kind_for = |pos: (usize, usize)| -> PathNodeKind {
        if pos == start {
            return PathNodeKind::Start;
        }
        if pos == target {
            return PathNodeKind::Target;
        }
        match kind_at.get(&pos) {
            Some(SlotKind::Fortress) => PathNodeKind::Fortress {
                section: section_at[&pos],
            },
            Some(SlotKind::Level) => PathNodeKind::Level,
            Some(SlotKind::Pipe) => PathNodeKind::Pipe,
            Some(SlotKind::HammerBro) => PathNodeKind::HammerBro,
            Some(SlotKind::ToadHouse) => PathNodeKind::ToadHouse,
            Some(SlotKind::BonusGame) => PathNodeKind::BonusGame,
            None => PathNodeKind::Unclassified,
        }
    };

    let mut chain: Vec<SearchState> = vec![final_state];
    let mut cur = final_state;
    while let Some(&prev_state) = prev.get(&cur) {
        chain.push(prev_state);
        cur = prev_state;
    }
    chain.reverse();

    let mut path: Vec<((usize, usize), PathNodeKind)> = Vec::with_capacity(chain.len());
    let mut locks_crossed: Vec<((usize, usize), usize)> = Vec::new();
    let mut forts: HashSet<(usize, usize)> = HashSet::new();
    let mut levels: HashSet<(usize, usize)> = HashSet::new();

    // Streak accounting: `run` = current back-to-back forced-level count.
    // An "activity" (fort / pipe / hammer-bro / crossed lock) resets it;
    // toad-house/spade/walk tiles pass through without resetting.
    let mut level_streak = 0usize;
    let mut goal_stack = 0usize;
    let mut run = 0usize;

    for (i, state) in chain.iter().enumerate() {
        let pos = state.0;
        path.push((pos, kind_for(pos)));
        if i > 0 {
            let prev_pos = chain[i - 1].0;
            if let Some(edges) = walk.edges.get(&prev_pos)
                && let Some(edge) = edges.iter().find(|e| e.dest == pos)
                && let Some(path_pos) = edge.path_pos
                && let Some(&section) = lock_section.get(&path_pos)
            {
                locks_crossed.push((path_pos, section));
                // A lock crossed on the way into this node is an activity
                // between the previous node and this one.
                run = 0;
            }
        }
        if pos == target {
            // Whatever level run reached the objective uninterrupted.
            goal_stack = run;
            continue;
        }
        if pos == start {
            continue;
        }
        match kind_at.get(&pos) {
            Some(SlotKind::Fortress) => {
                forts.insert(pos);
                run = 0;
            }
            Some(SlotKind::Level) => {
                levels.insert(pos);
                run += 1;
                level_streak = level_streak.max(run);
            }
            // A pipe ride is a deterministic activity, so it breaks a run.
            Some(SlotKind::Pipe) => {
                run = 0;
            }
            // Everything else passes through without breaking the run:
            //   - HammerBro: a hammer-bro *node* is not entered by standing
            //     on it. The fight is triggered by the wandering sprite,
            //     whose position is dynamic and unmodeled here, so a node
            //     with no sprite on it is just empty path. Do NOT credit it
            //     as an activity between levels.
            //   - ToadHouse / BonusGame: rarely entered by players.
            //   - Unclassified / stray transit tiles.
            _ => {}
        }
    }

    RequiredProgression {
        forts_required: forts.len(),
        levels_required: levels.len(),
        reachable: true,
        path,
        locks_crossed,
        hammer_broke_section: None,
        level_streak,
        goal_stack,
    }
}

/// Count of graph-adjacent Level–Level node pairs, independent of the
/// required route: two level slots joined by a single walk edge (not a
/// pipe/canoe teleport). A high count means the map physically stacks
/// levels side by side — the "level physically next to level" signal,
/// distinct from the route-based `level_streak`.
pub(crate) fn level_adjacency_pairs(built: &BuiltWorld) -> usize {
    let mut grid = built.grid.clone();
    stamp_slots(&mut grid, &built.slots);
    let Some(start) = rom_data::find_start(&grid) else {
        return 0;
    };
    let walk = walk_map(&grid, &built.pipe_pairs, Some(start), built.world_idx);
    let level_pos: HashSet<(usize, usize)> = built
        .slots
        .iter()
        .filter(|s| s.kind == SlotKind::Level)
        .map(|s| s.pos)
        .collect();
    let mut pairs: HashSet<((usize, usize), (usize, usize))> = HashSet::new();
    for (&a, edges) in &walk.edges {
        if !level_pos.contains(&a) {
            continue;
        }
        for edge in edges {
            // path_pos.is_some() ⇒ a walk edge (physical adjacency), not a
            // pipe/canoe teleport.
            if edge.path_pos.is_some() && level_pos.contains(&edge.dest) {
                let key = if a <= edge.dest {
                    (a, edge.dest)
                } else {
                    (edge.dest, a)
                };
                pairs.insert(key);
            }
        }
    }
    pairs.len()
}

/// What role a single pipe pair plays on the required route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PipeClass {
    /// Removing this pipe strands the objective — it's mandatory access to an
    /// island / rock-sectioned area (function 1: connectivity).
    Connectivity,
    /// Removing it keeps the objective reachable and doesn't change min-clears,
    /// but at least one endpoint is an island (not walk-reachable without
    /// pipes). This is a *redundant island route* — it gives the player an
    /// alternate path through the island network, so it's variety, not waste.
    IslandRouting,
    /// Removing it keeps the objective reachable, doesn't change min-clears,
    /// AND both endpoints are walk-reachable mainland. A loop over ground the
    /// player could already walk, skipping nothing — the true waste to minimize.
    DeadLoop,
    /// Removing it RAISES min-clears by `n` — a real shortcut that skips `n`
    /// forced clears (function 2), regardless of mainland/island.
    Shortcut(usize),
}

/// Classify every pipe pair by leave-one-out: remove it (leaving the others
/// and all canoes intact) and re-run the required-progression analysis. The
/// mainland/island split uses walk-only reachability from start (no pipes) so
/// intentional island-hopping routes aren't miscounted as dead-loop waste.
///
/// Caveat: leave-one-out reads a lateral loop offering an equal-length
/// alternative as delta-0 (it doesn't lower the MINIMUM clears). For a
/// mainland loop that's exactly the waste we're after; the island case is
/// separated out as `IslandRouting`.
pub(crate) fn classify_pipes(built: &BuiltWorld) -> Vec<PipeClass> {
    let base = analyze_required_progression(built, false);
    let base_clears = base.forts_required + base.levels_required;

    // Mainland = nodes walk-reachable from start with NO pipes. An endpoint
    // outside this set is only reachable by pipe (an island).
    let mut grid = built.grid.clone();
    stamp_slots(&mut grid, &built.slots);
    let mainland: HashSet<(usize, usize)> = rom_data::find_start(&grid)
        .map(|s| walk_map(&grid, &[], Some(s), built.world_idx).nodes)
        .unwrap_or_default();

    let mut out = Vec::with_capacity(built.pipe_pairs.len());
    for i in 0..built.pipe_pairs.len() {
        let (a, b) = built.pipe_pairs[i];
        let mut without = built.clone();
        without.pipe_pairs.remove(i);
        let r = analyze_required_progression(&without, false);
        out.push(if !r.reachable {
            PipeClass::Connectivity
        } else {
            let delta = (r.forts_required + r.levels_required).saturating_sub(base_clears);
            if delta > 0 {
                PipeClass::Shortcut(delta)
            } else if mainland.contains(&a) && mainland.contains(&b) {
                PipeClass::DeadLoop
            } else {
                PipeClass::IslandRouting
            }
        });
    }
    out
}

/// Whether a single pipe directly bridges the start walk-component to the
/// goal walk-component — a start→goal "express" pipe that short-circuits any
/// intermediate island hops.
///
/// `None` when start and goal already share a walk component (no bridging
/// pipe needed, e.g. W1/W2) or the target is missing. Otherwise `Some(true)`
/// if such a direct pipe exists.
///
/// Interpretation is world-dependent: for few-island worlds (W3/W4/W5) a
/// direct link is natural or required (~100%). For many-island worlds
/// (W7/W8) it collapses the intended island-hopping journey into one jump
/// and should be rare.
pub(crate) fn start_goal_express_pipe(built: &BuiltWorld) -> Option<bool> {
    let mut grid = built.grid.clone();
    stamp_slots(&mut grid, &built.slots);
    let start = rom_data::find_start(&grid)?;
    let target = find_target(&grid, built.world_idx)?;
    let start_comp = walk_map(&grid, &[], Some(start), built.world_idx).nodes;
    if start_comp.contains(&target) {
        return None; // start and goal already on the same island
    }
    let goal_comp = walk_map(&grid, &[], Some(target), built.world_idx).nodes;
    let direct = built.pipe_pairs.iter().any(|&(a, b)| {
        (start_comp.contains(&a) && goal_comp.contains(&b))
            || (goal_comp.contains(&a) && start_comp.contains(&b))
    });
    Some(direct)
}

/// Rough island count: the number of distinct walk-only components (no pipes)
/// spanned by the world's start, target, and pipe endpoints. Context for the
/// express-pipe rate — a world with 2 islands "should" bridge start↔goal
/// directly; a world with 5 should route through the middle ones.
pub(crate) fn island_count(built: &BuiltWorld) -> usize {
    let mut grid = built.grid.clone();
    stamp_slots(&mut grid, &built.slots);
    let mut key: Vec<(usize, usize)> = built
        .pipe_pairs
        .iter()
        .flat_map(|&(a, b)| [a, b])
        .collect();
    if let Some(s) = rom_data::find_start(&grid) {
        key.push(s);
    }
    if let Some(t) = find_target(&grid, built.world_idx) {
        key.push(t);
    }
    let mut seen: HashSet<(usize, usize)> = HashSet::new();
    let mut comps = 0;
    for n in key {
        if seen.contains(&n) {
            continue;
        }
        let comp = walk_map(&grid, &[], Some(n), built.world_idx).nodes;
        comps += 1;
        seen.extend(comp);
    }
    comps
}

/// How many forced clears a lock-breaking hammer lets the player skip:
/// (no-hammer clears) − (one-hammer clears). This is the "breaking a
/// rock/lock opens a shortcut past levels" value, assuming the
/// hammer-breaks-locks mechanic is in play.
pub(crate) fn hammer_skip(built: &BuiltWorld) -> usize {
    let nh = analyze_required_progression(built, false);
    let wh = analyze_required_progression(built, true);
    let nh_c = nh.forts_required + nh.levels_required;
    let wh_c = wh.forts_required + wh.levels_required;
    nh_c.saturating_sub(wh_c)
}

/// Pretty-print a `RequiredProgression` result for one world. Use for
/// verification + as a reference for the WASM single-seed dump.
pub(crate) fn dump_required_progression(built: &BuiltWorld) {
    let no_hammer = analyze_required_progression(built, false);
    let with_hammer = analyze_required_progression(built, true);
    // Sanity check: with EVERY lock pre-opened, is the target reachable?
    // If not, the unreachability is a real builder/topology issue. If yes
    // but the 1-lock-hammer path also fails, the issue is lock chain depth.
    let all_open_mask = (1u32 << built.section_count).wrapping_sub(1);
    let all_open = analyze_with_pre_opened_mask(built, all_open_mask);

    let start = rom_data::find_start(&built.grid);
    let target = find_target(&built.grid, built.world_idx);

    let canoes: Vec<((usize, usize), (usize, usize))> =
        rom_data::active_canoe_edges(built.world_idx, built.grid.eights_are_wild);

    eprintln!("\n--- W{} ---", built.world_idx + 1);
    eprintln!(
        "  start={:?}  target={:?}  sections={}  locks={}  pipes={}{}",
        start,
        target,
        built.section_count,
        built.locks.len(),
        built.pipe_pairs.len(),
        if canoes.is_empty() {
            String::new()
        } else {
            format!("  canoes={}", canoes.len())
        },
    );

    // Inventory of fortress positions per section, so the lock annotations
    // make sense to the reader.
    let mut forts_by_section: Vec<(usize, (usize, usize))> = built
        .slots
        .iter()
        .filter(|s| s.kind == SlotKind::Fortress)
        .map(|s| (s.section, s.pos))
        .collect();
    forts_by_section.sort();
    eprintln!("  fortresses:");
    for (sec, pos) in &forts_by_section {
        eprintln!("    section {sec}: ({}, {})", pos.0, pos.1);
    }
    eprintln!("  locks:");
    for lock in &built.locks {
        eprintln!(
            "    ({}, {}) opened by section {}",
            lock.pos.0, lock.pos.1, lock.fort_section,
        );
    }
    eprintln!("  pipe pairs:");
    for &(a, b) in &built.pipe_pairs {
        eprintln!("    ({},{}) <-> ({},{})", a.0, a.1, b.0, b.1);
    }
    if !canoes.is_empty() {
        eprintln!("  canoe routes (boat starts at the first endpoint):");
        for (a, b) in &canoes {
            eprintln!("    ({},{}) -> ({},{}) (and reverse, while boat is at far side)", a.0, a.1, b.0, b.1);
        }
    }

    let pipe_set: EdgeSet = built.pipe_pairs
        .iter()
        .flat_map(|&(a, b)| [(a, b), (b, a)])
        .collect();
    let canoe_set: EdgeSet = canoes
        .iter()
        .flat_map(|&(a, b)| [(a, b), (b, a)])
        .collect();

    print_progression("Without hammer", &no_hammer, &pipe_set, &canoe_set);
    print_progression("With hammer (1 lock max)", &with_hammer, &pipe_set, &canoe_set);
    eprintln!(
        "  [Sanity: all locks pre-opened]  reachable={}  forts={}  levels={}",
        all_open.reachable, all_open.forts_required, all_open.levels_required,
    );

    match with_hammer.hammer_broke_section {
        Some(s) => eprintln!("  Hammer used on: lock for section {s}"),
        None => eprintln!("  Hammer used on: (nothing — hammer didn't help)"),
    }
    let fort_delta = no_hammer.forts_required as isize - with_hammer.forts_required as isize;
    let level_delta = no_hammer.levels_required as isize - with_hammer.levels_required as isize;
    let total_delta = fort_delta + level_delta;
    eprintln!(
        "  Hammer net: {fort_delta:+} fort(s), {level_delta:+} level(s)  =  {total_delta:+} total clears",
    );
}

/// Set of directed teleport edges (pipe-pair / canoe-pair, both orientations).
type EdgeSet = HashSet<((usize, usize), (usize, usize))>;

pub(super) fn print_progression(
    label: &str,
    p: &RequiredProgression,
    pipe_set: &EdgeSet,
    canoe_set: &EdgeSet,
) {
    eprintln!(
        "\n  [{label}]  required: {} fort(s) + {} level(s)  (+ objective)",
        p.forts_required, p.levels_required,
    );
    if !p.reachable {
        eprintln!("    TARGET UNREACHABLE");
        return;
    }
    eprintln!(
        "    level streak: {}  |  goal stack: {}",
        p.level_streak, p.goal_stack,
    );
    let mut lock_iter = p.locks_crossed.iter().peekable();
    for (i, (pos, kind)) in p.path.iter().enumerate() {
        let tag = match kind {
            PathNodeKind::Start => "START".to_string(),
            PathNodeKind::Level => "LEVEL".to_string(),
            PathNodeKind::Fortress { section } => format!("FORT (section {section})"),
            PathNodeKind::Pipe => "PIPE (transit)".to_string(),
            PathNodeKind::HammerBro => "HAMMERBRO (transit)".to_string(),
            PathNodeKind::ToadHouse => "TOAD (transit)".to_string(),
            PathNodeKind::BonusGame => "BONUS (transit)".to_string(),
            PathNodeKind::Target => "TARGET (airship/Bowser)".to_string(),
            PathNodeKind::Unclassified => "transit tile".to_string(),
        };
        // Classify the hop: pipe teleport, canoe, or walk.
        let via = if i > 0 {
            let prev = p.path[i - 1].0;
            let edge = (prev, *pos);
            if pipe_set.contains(&edge) {
                " [via PIPE]"
            } else if canoe_set.contains(&edge) {
                " [via CANOE]"
            } else {
                ""
            }
        } else {
            ""
        };
        eprintln!("    {i:2}. ({:2},{:2})  {tag}{via}", pos.0, pos.1);
        // After printing the step, if the next lock_crossed entry came from
        // this hop, surface it underneath.
        if let Some(&&(lock_pos, sec)) = lock_iter.peek()
            && i > 0
        {
            // The lock was on the edge into this node; print under this line.
            let prev = p.path[i - 1].0;
            // Path tile sits between prev and pos for a normal walk.
            let between_r = (prev.0 + pos.0) / 2;
            let between_c = (prev.1 + pos.1) / 2;
            if (between_r, between_c) == lock_pos {
                eprintln!(
                    "         ↳ crossed lock at ({},{}) (opened by section {sec})",
                    lock_pos.0, lock_pos.1,
                );
                lock_iter.next();
            }
        }
    }
}
