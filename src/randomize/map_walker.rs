//! BFS map walker for overworld connectivity analysis.
//!
//! Traverses SMB3 overworld maps using the game's 2-tile movement model
//! (node → path tile → node). Supports pipe teleport edges, chokepoint
//! detection, and fortress progression simulation.
//!
//! Shared ROM constants, data structures, and helpers live in `rom_data.rs`.

use std::collections::{HashMap, HashSet, VecDeque};

#[cfg(test)]
use crate::rom::Rom;

use super::rom_data::{
    self, BACKGROUND_TILES, TILE_AIRSHIP, TILE_BOWSER, VALID_HORZ, VALID_VERT,
    Grid, TeleportEdge,
};

#[cfg(test)]
use super::rom_data::Pos;



/// Movement directions: (delta_row, delta_col, is_horizontal).
const DIRECTIONS: [(i8, i8, bool); 4] = [
    (0, 1, true),   // right
    (0, -1, true),  // left
    (1, 0, false),  // down
    (-1, 0, false), // up
];

// ---------------------------------------------------------------------------
// Data structures (walker-specific)
// ---------------------------------------------------------------------------

/// An edge in the walk graph.
/// Fields are populated during BFS and consumed by test-only visualization/analysis.
#[allow(dead_code)]
#[derive(PartialEq, Eq)]
pub(super) struct Edge {
    pub dest: (usize, usize),
    /// Path tile position (None for pipe teleport edges).
    pub path_pos: Option<(usize, usize)>,
}

/// Result of a BFS map walk.
pub(super) struct WalkResult {
    pub nodes: HashSet<(usize, usize)>,
    /// BFS distance (in hops) from start to each reachable node.
    pub distances: HashMap<(usize, usize), usize>,
    /// Edge graph — populated during BFS, consumed by test-only chokepoint analysis.
    #[allow(dead_code)]
    pub edges: HashMap<(usize, usize), Vec<Edge>>,
    #[allow(dead_code)]
    pub path_tiles: HashSet<(usize, usize)>,
}

// ---------------------------------------------------------------------------
// BFS map walker
// ---------------------------------------------------------------------------

/// Position → teleport destinations, built from bidirectional edge pairs.
type TeleportLookup = HashMap<(usize, usize), Vec<(usize, usize)>>;

fn teleport_lookup(pairs: &[TeleportEdge]) -> TeleportLookup {
    let mut lookup = TeleportLookup::new();
    for &(a, b) in pairs {
        lookup.entry(a).or_default().push(b);
        lookup.entry(b).or_default().push(a);
    }
    lookup
}

/// First-pass reachability check: can the player walk (using pipes only — no
/// canoes) to ANY canoe mainland dock from `start`? If yes, canoes become
/// usable for the main BFS. If no, canoes stay disabled.
///
/// Returns true when there are no canoes in the world (trivially "no dock to
/// fail to reach"), so non-W3 worlds short-circuit cheaply.
fn canoes_reachable(
    grid: &Grid,
    pipe_pairs: &[TeleportEdge],
    start: (usize, usize),
    world_idx: usize,
) -> bool {
    // Mainland docks are the `a` side of each active canoe edge for this world.
    // `active_canoe_edges` applies both the world filter (the coordinates are
    // not world-unique — W3's (6,20) also exists in W2/W4–W8) and the `8s are
    // Wild` gate (W8's docks only exist when the flag is on).
    let docks: Vec<(usize, usize)> = rom_data::active_canoe_edges(world_idx, grid.eights_are_wild)
        .into_iter()
        .map(|(a, _)| a)
        .collect();
    if docks.is_empty() {
        return true;
    }

    // Same BFS as the main walk, just with no canoe edges (a 9×64 grid, so
    // running it to completion instead of early-exiting at a dock is cheap).
    let no_canoes = reach_from(grid, start, &teleport_lookup(pipe_pairs), &TeleportLookup::new());
    docks.iter().any(|&d| no_canoes.contains(d))
}

/// BFS walk from a start position, returning reachable nodes, edges, and path tiles.
///
/// Movement model: player moves 2 tiles at a time. The intermediate tile must
/// be a valid path tile for the movement direction. Pipes create bidirectional
/// teleport edges. Canoes are stateful — see `canoes_reachable` and the
/// canoe_lookup construction below.
pub(super) fn walk_map(
    grid: &Grid,
    pipe_pairs: &[TeleportEdge],
    start_pos: Option<(usize, usize)>,
    world_idx: usize,
) -> WalkResult {
    let start = match start_pos.or_else(|| rom_data::find_start(grid)) {
        Some(s) => s,
        None => {
            return WalkResult {
                nodes: HashSet::new(),
                distances: HashMap::new(),
                edges: HashMap::new(),
                path_tiles: HashSet::new(),
            };
        }
    };

    let pipe_lookup = teleport_lookup(pipe_pairs);

    // Canoes: the boat starts at the mainland dock (the `a` side of each
    // `CANOE_EDGES` tuple). The player must be able to *walk* to that dock
    // before any canoe edge becomes usable — once they've boarded, they can
    // shuttle the boat between mainland and any island as many times as they
    // like, which makes the bidirectional model below a correct simplification
    // (each "free" canoe hop corresponds to a real round trip the player
    // could make once they hold the boat).
    //
    // If the player CAN'T reach the mainland dock, no canoe is ever usable;
    // we omit the edges entirely so the BFS reflects reality. This is the
    // structural fix for the SAS-W3 deadlock where the swap moves the start
    // into a region with no walking path to the dock.
    let canoe_lookup = if canoes_reachable(grid, pipe_pairs, start, world_idx) {
        teleport_lookup(&rom_data::active_canoe_edges(world_idx, grid.eights_are_wild))
    } else {
        TeleportLookup::new()
    };

    walk_from(grid, start, &pipe_lookup, &canoe_lookup)
}

/// The BFS core shared by `walk_map` and the canoe first pass: walk from
/// `start` expanding orthogonal 2-tile moves plus the given teleport edges.
fn walk_from(
    grid: &Grid,
    start: (usize, usize),
    pipe_lookup: &TeleportLookup,
    canoe_lookup: &TeleportLookup,
) -> WalkResult {
    let mut nodes = HashSet::new();
    let mut distances: HashMap<(usize, usize), usize> = HashMap::new();
    let mut edges: HashMap<(usize, usize), Vec<Edge>> = HashMap::new();
    let mut path_tiles = HashSet::new();
    let mut queue = VecDeque::new();

    nodes.insert(start);
    distances.insert(start, 0);
    queue.push_back(start);

    while let Some((r, c)) = queue.pop_front() {
        edges.entry((r, c)).or_default();

        // The airship/Bowser target blocks through-movement until completed —
        // and completing it ends the world — so the player can never pass
        // THROUGH it. Model it as enterable but not exitable (a sink): the
        // region behind the target is a separate island to every walk, which
        // is what the game actually plays like (and what lets the pipe
        // connectivity phase serve that region its own access). Exception:
        // when the walk STARTS on the target, expansion stays enabled — a
        // from-target walk asks "which nodes can reach me", and since every
        // other edge is symmetric, expanding out of the sink source keeps
        // that meaning intact.
        let tile_here = grid.get(r, c);
        if (tile_here == TILE_AIRSHIP || tile_here == TILE_BOWSER) && (r, c) != start {
            continue;
        }

        // Orthogonal movement: node → path tile → node (2 tiles)
        for &(dr, dc, is_horz) in &DIRECTIONS {
            let pr = r as i16 + dr as i16;
            let pc = c as i16 + dc as i16;
            if pr < 0 || pr >= grid.rows() as i16 || pc < 0 || pc >= grid.cols as i16 {
                continue;
            }
            let (pr, pc) = (pr as usize, pc as usize);

            let path_tile = grid.get(pr, pc);
            let valid = if is_horz { VALID_HORZ } else { VALID_VERT };
            if !valid.contains(&path_tile) {
                continue;
            }

            let nr = r as i16 + 2 * dr as i16;
            let nc = c as i16 + 2 * dc as i16;
            if nr < 0 || nr >= grid.rows() as i16 || nc < 0 || nc >= grid.cols as i16 {
                continue;
            }
            let (nr, nc) = (nr as usize, nc as usize);

            let dest_tile = grid.get(nr, nc);
            if BACKGROUND_TILES.contains(&dest_tile) {
                continue;
            }

            path_tiles.insert((pr, pc));
            edges.entry((r, c)).or_default().push(Edge {
                dest: (nr, nc),
                path_pos: Some((pr, pc)),
            });

            if !nodes.contains(&(nr, nc)) {
                nodes.insert((nr, nc));
                distances.insert((nr, nc), distances[&(r, c)] + 1);
                queue.push_back((nr, nc));
            }
        }

        // Teleport edges: pipes, then canoes (canoe edges are only present
        // when the mainland dock was reachable — see canoe_lookup above).
        for lookup in [pipe_lookup, canoe_lookup] {
            if let Some(dests) = lookup.get(&(r, c)) {
                for &dest in dests {
                    edges.entry((r, c)).or_default().push(Edge {
                        dest,
                        path_pos: None,
                    });
                    if !nodes.contains(&dest) {
                        nodes.insert(dest);
                        distances.insert(dest, distances[&(r, c)] + 1);
                        queue.push_back(dest);
                    }
                }
            }
        }
    }

    WalkResult { nodes, distances, edges, path_tiles }
}

// ---------------------------------------------------------------------------
// Reachability-only walk (flat bitset) — the hot path
// ---------------------------------------------------------------------------

/// Reachable-node set as a flat bitset (index `r * cols + c`). The many
/// reachability-only callers — `place_locks`' per-candidate hard-rule checks,
/// `canoes_reachable`, the gated shortcut's completability/goal-open guards —
/// need only "is this node reachable" and "how many", never the edge graph or
/// distances. Building those (a `HashMap<Pos, Vec<Edge>>` + distance map) per
/// call dominated the lock pass's cost; this returns the identical node SET
/// (BFS reachability is order-independent) with no per-call allocation beyond
/// one bit-vector.
pub(super) struct Reach {
    bits: Vec<u64>,
    cols: usize,
    count: usize,
}

impl Reach {
    fn new(rows: usize, cols: usize) -> Self {
        Reach {
            bits: vec![0; rows * cols / 64 + 1],
            cols,
            count: 0,
        }
    }

    /// Set the bit for `(r, c)`; returns true if it was newly set.
    #[inline]
    fn insert(&mut self, (r, c): (usize, usize)) -> bool {
        let i = r * self.cols + c;
        let (word, bit) = (i >> 6, i & 63);
        if (self.bits[word] >> bit) & 1 == 1 {
            return false;
        }
        self.bits[word] |= 1 << bit;
        self.count += 1;
        true
    }

    #[inline]
    pub(super) fn contains(&self, (r, c): (usize, usize)) -> bool {
        let i = r * self.cols + c;
        (self.bits[i >> 6] >> (i & 63)) & 1 == 1
    }

    #[inline]
    pub(super) fn len(&self) -> usize {
        self.count
    }
}

/// Reachability-only BFS core — mirrors [`walk_from`]'s traversal exactly but
/// tracks only the reachable set, so it yields the identical `nodes` set.
fn reach_from(
    grid: &Grid,
    start: (usize, usize),
    pipe_lookup: &TeleportLookup,
    canoe_lookup: &TeleportLookup,
) -> Reach {
    let mut reach = Reach::new(grid.rows(), grid.cols);
    let mut queue = VecDeque::new();
    reach.insert(start);
    queue.push_back(start);

    while let Some((r, c)) = queue.pop_front() {
        let tile_here = grid.get(r, c);
        if (tile_here == TILE_AIRSHIP || tile_here == TILE_BOWSER) && (r, c) != start {
            continue; // target is a sink (see walk_from)
        }

        for &(dr, dc, is_horz) in &DIRECTIONS {
            let pr = r as i16 + dr as i16;
            let pc = c as i16 + dc as i16;
            if pr < 0 || pr >= grid.rows() as i16 || pc < 0 || pc >= grid.cols as i16 {
                continue;
            }
            let (pr, pc) = (pr as usize, pc as usize);
            let path_tile = grid.get(pr, pc);
            let valid = if is_horz { VALID_HORZ } else { VALID_VERT };
            if !valid.contains(&path_tile) {
                continue;
            }
            let nr = r as i16 + 2 * dr as i16;
            let nc = c as i16 + 2 * dc as i16;
            if nr < 0 || nr >= grid.rows() as i16 || nc < 0 || nc >= grid.cols as i16 {
                continue;
            }
            let (nr, nc) = (nr as usize, nc as usize);
            if BACKGROUND_TILES.contains(&grid.get(nr, nc)) {
                continue;
            }
            if reach.insert((nr, nc)) {
                queue.push_back((nr, nc));
            }
        }

        for lookup in [pipe_lookup, canoe_lookup] {
            if let Some(dests) = lookup.get(&(r, c)) {
                for &dest in dests {
                    if reach.insert(dest) {
                        queue.push_back(dest);
                    }
                }
            }
        }
    }
    reach
}

/// Reachability-only counterpart of [`walk_map`]: same start resolution and
/// canoe gating, returns the reachable-node bitset. Use wherever only `.nodes`
/// is read.
pub(super) fn walk_reachable(
    grid: &Grid,
    pipe_pairs: &[TeleportEdge],
    start_pos: Option<(usize, usize)>,
    world_idx: usize,
) -> Reach {
    let start = match start_pos.or_else(|| rom_data::find_start(grid)) {
        Some(s) => s,
        None => return Reach::new(grid.rows(), grid.cols),
    };
    let pipe_lookup = teleport_lookup(pipe_pairs);
    let canoe_lookup = if canoes_reachable(grid, pipe_pairs, start, world_idx) {
        teleport_lookup(&rom_data::active_canoe_edges(world_idx, grid.eights_are_wild))
    } else {
        TeleportLookup::new()
    };
    reach_from(grid, start, &pipe_lookup, &canoe_lookup)
}

// ---------------------------------------------------------------------------
// Chokepoint detection
// ---------------------------------------------------------------------------

/// Find path tiles whose removal disconnects the node graph (articulation points).
///
/// Tests each path tile by removing it and checking if BFS still reaches all nodes.
#[cfg(test)]
pub(super) fn find_chokepoints(result: &WalkResult) -> HashSet<(usize, usize)> {
    if result.nodes.is_empty() {
        return HashSet::new();
    }

    // Build adjacency: node → list of (neighbor, path_pos_or_none)
    type AdjacencyEdge = (Pos, Option<Pos>);
    let mut adj: HashMap<Pos, Vec<AdjacencyEdge>> = HashMap::new();
    for (node, neighbors) in &result.edges {
        for edge in neighbors {
            adj.entry(*node).or_default().push((edge.dest, edge.path_pos));
        }
    }

    let start = *result.nodes.iter().next().unwrap();
    let mut chokepoints = HashSet::new();

    for &path_pos in &result.path_tiles {
        // BFS without using edges through this path tile
        let mut visited = HashSet::new();
        let mut q = VecDeque::new();
        visited.insert(start);
        q.push_back(start);

        while let Some(n) = q.pop_front() {
            if let Some(neighbors) = adj.get(&n) {
                for &(dest, pp) in neighbors {
                    if pp == Some(path_pos) {
                        continue;
                    }
                    if !visited.contains(&dest) {
                        visited.insert(dest);
                        q.push_back(dest);
                    }
                }
            }
        }

        if visited.len() < result.nodes.len() {
            chokepoints.insert(path_pos);
        }
    }

    chokepoints
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::rom_data;

    #[test]
    fn test_find_start_all_worlds() {
        let rom_data_bytes = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data_bytes.is_err() {
            return;
        }
        let rom = Rom::from_bytes(&rom_data_bytes.unwrap()).unwrap();

        for wi in 0..8 {
            let grid = rom_data::read_tile_grid(&rom, wi);
            let start = rom_data::find_start(&grid);
            assert!(
                start.is_some(),
                "World {} should have a START tile",
                wi + 1
            );
        }
    }

    #[test]
    fn test_walk_w1_reachable() {
        let rom_data_bytes = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data_bytes.is_err() {
            return;
        }
        let rom = Rom::from_bytes(&rom_data_bytes.unwrap()).unwrap();

        let grid = rom_data::read_tile_grid(&rom, 0);
        let pipes = rom_data::read_pipe_pairs(&rom);
        let w1_pipes = pipes.get(&0).cloned().unwrap_or_default();
        let result = walk_map(&grid, &w1_pipes, None, 0);

        // W1 has 21 entries, most are reachable from start (no pipes needed)
        assert!(
            result.nodes.len() >= 15,
            "W1 should have at least 15 reachable nodes, got {}",
            result.nodes.len()
        );
    }

    #[test]
    fn test_walk_w7_needs_pipes() {
        let rom_data_bytes = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data_bytes.is_err() {
            return;
        }
        let rom = Rom::from_bytes(&rom_data_bytes.unwrap()).unwrap();

        let grid = rom_data::read_tile_grid(&rom, 6);

        // Walk without pipes — should be very limited
        let result_no_pipes = walk_map(&grid, &[], None, 6);

        // Walk with pipes — should reach many more
        let pipes = rom_data::read_pipe_pairs(&rom);
        let w7_pipes = pipes.get(&6).cloned().unwrap_or_default();
        let result_with_pipes = walk_map(&grid, &w7_pipes, None, 6);

        assert!(
            result_with_pipes.nodes.len() > result_no_pipes.nodes.len(),
            "W7 with pipes ({}) should reach more than without ({})",
            result_with_pipes.nodes.len(),
            result_no_pipes.nodes.len()
        );
    }

    /// Regression test for the canoe-edge world-leak bug (PR #39).
    ///
    /// `CANOE_EDGES` coordinates are not world-unique, so before world filtering
    /// any world whose BFS reached a mainland-dock *coordinate* got the canoe
    /// teleport injected — fabricating connectivity. This proves each edge fires
    /// ONLY in its own world.
    ///
    /// Rather than inject a synthetic edge (the edge list is private), we use
    /// the real edges but evaluate them from a world that has none: same grid,
    /// same reachable dock coordinate, different `world_idx`. The island must be
    /// reachable in the edge's own world and unreachable from the other. Under
    /// the old unfiltered code the negative assertion would fail.
    #[test]
    fn test_canoe_edges_are_world_scoped() {
        // Gather every active canoe edge (all worlds, `8s are Wild` on so the
        // W8 edges are included) via the single accessor.
        let all_edges: Vec<(usize, (Pos, Pos))> = (0..8)
            .flat_map(|w| {
                rom_data::active_canoe_edges(w, true)
                    .into_iter()
                    .map(move |e| (w, e))
            })
            .collect();
        // A world with no canoe edges at all: walking it must never produce
        // canoe connectivity regardless of which coordinates are reachable.
        let canoe_worlds: HashSet<usize> = all_edges.iter().map(|&(w, _)| w).collect();
        let other = (0..8)
            .find(|w| !canoe_worlds.contains(w))
            .expect("at least one world should have no canoe edges");

        for (world, (a, b)) in all_edges {
            assert!(a.1 >= 2, "test assumes mainland dock col >= 2");

            // Background grid (0xB4) large enough for every dock coordinate,
            // with a 2-tile walk carved from `start` to the mainland dock `a`.
            // The island dock `b` is left isolated, so it can only be reached
            // via a canoe hop — never by walking. `eights_are_wild` on so the
            // W8 edges resolve.
            let mut grid = Grid {
                tiles: vec![vec![0xB4u8; 48]; 9],
                cols: 48,
                eights_are_wild: true,
            };
            let start = (a.0, a.1 - 2);
            grid.set(a.0, a.1 - 1, 0x45); // VALID_HORZ path tile
            grid.set(a.0, a.1, 0x45); // mainland dock node (non-background)

            // Positive: in the edge's own world the canoe makes `b` reachable.
            let own = walk_map(&grid, &[], Some(start), world);
            assert!(
                own.nodes.contains(&a),
                "W{} mainland dock {a:?} should be walk-reachable",
                world + 1
            );
            assert!(
                own.nodes.contains(&b),
                "W{} canoe {a:?}->{b:?} should reach the island in its own world",
                world + 1
            );

            // Negative: same grid + same reachable coordinate, but a world with
            // no canoe edges must NOT gain the island.
            let leaked = walk_map(&grid, &[], Some(start), other);
            assert!(
                leaked.nodes.contains(&a),
                "control: mainland coord {a:?} still walk-reachable in W{}",
                other + 1
            );
            assert!(
                !leaked.nodes.contains(&b),
                "LEAK: canoe {a:?}->{b:?} (world {world}) fired in world {other} — \
                 world filtering is broken"
            );
        }
    }

    #[test]
    fn test_dest_indices_for_world() {
        assert_eq!(rom_data::dest_indices_for_world(0).len(), 0); // W1: no pipes
        assert_eq!(rom_data::dest_indices_for_world(1).len(), 1); // W2: 1 pair
        assert_eq!(rom_data::dest_indices_for_world(4).len(), 2); // W5: 1 regular + 1 spiral tower
        assert_eq!(rom_data::dest_indices_for_world(6).len(), 8); // W7: 8 pairs
        assert_eq!(rom_data::dest_indices_for_world(7).len(), 6); // W8: 6 pairs
    }

    #[test]
    fn test_chokepoints_w1() {
        let rom_data_bytes = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data_bytes.is_err() {
            return;
        }
        let rom = Rom::from_bytes(&rom_data_bytes.unwrap()).unwrap();

        let grid = rom_data::read_tile_grid(&rom, 0);
        let result = walk_map(&grid, &[], None, 0);
        let chokepoints = find_chokepoints(&result);

        // W1 has a linear path structure with many chokepoints
        assert!(
            !chokepoints.is_empty(),
            "W1 should have chokepoints (linear map)"
        );
    }

}
