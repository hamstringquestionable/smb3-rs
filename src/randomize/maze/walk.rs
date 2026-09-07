//! The cross-world walkers.
//!
//! Two of them, sharing one move expansion: [`walk_maze`] answers "what can be
//! reached", [`walk_maze_cost`] answers "at what price" — the price being the
//! number of levels and fortresses that have to be beaten on the way, which is
//! how the mode answers "how many levels does it take to finish this game".
//!
//! They are written HERE rather than in `map_walker.rs` on purpose. Standard
//! mode's `walk_reachable` is load-bearing for every overworld census, and the
//! maze needs two things it does not have: a world-indexed frontier and
//! DIRECTED teleport edges. Sharing the code would mean editing the walker
//! every one of those censuses depends on.
//!
//! The cost of keeping them apart is a copy of `reach_from`'s 2-tile move
//! expansion. What stops the copy drifting is
//! `maze_walk_matches_the_per_world_walker`: over every world of every census
//! seed it asserts [`walk_maze`], given the eight worlds and no links,
//! reproduces `walk_reachable` cell for cell. The oracle is the shipping
//! walker, never a hand-written expectation.
//!
//! ## The two semantics the per-world walker does not have
//!
//! * **The target tile is a walk sink, but a link LEAVES it.** `reach_from`
//!   refuses to expand out of `TILE_AIRSHIP` / `TILE_BOWSER` at all. Here only
//!   the directional moves are skipped; outgoing links still fire. An airship
//!   you cannot walk through is still an airship you can clear.
//! * **Canoes are a fixpoint, not a pre-pass.** `canoes_reachable` asks "can
//!   the player walk to a dock from the start, without canoes". Across worlds a
//!   pad can drop them straight onto a dock in a world they have no other route
//!   into, so activation is re-tested after each pass until it stops changing.
//!   Enabling a canoe only ever grows the reachable set, so it is monotone.

#[cfg(test)]
use std::collections::BinaryHeap;
use std::collections::{HashMap, VecDeque};

use super::super::rom_data::{
    self, BACKGROUND_TILES, Grid, Pos, TILE_AIRSHIP, TILE_BOWSER, TeleportEdge, VALID_HORZ,
    VALID_VERT,
};

/// A cell addressed across all eight grids.
pub(crate) type MazePos = (usize, Pos);

/// Movement directions: `(delta_row, delta_col, is_horizontal)` — the same
/// four `map_walker::DIRECTIONS` uses.
const DIRECTIONS: [(i8, i8, bool); 4] =
    [(0, 1, true), (0, -1, true), (1, 0, false), (-1, 0, false)];

/// Position → teleport destinations.
type TeleportLookup = HashMap<Pos, Vec<Pos>>;

fn teleport_lookup(pairs: &[TeleportEdge]) -> TeleportLookup {
    let mut lookup = TeleportLookup::new();
    for &(a, b) in pairs {
        lookup.entry(a).or_default().push(b);
        lookup.entry(b).or_default().push(a);
    }
    lookup
}

/// One world as the maze walkers see it: the grid to walk and the intra-world
/// teleport (pipe) edges on it. Deliberately not a `WorldState` — the walker
/// has no business knowing about slots, locks or budgets, and the caller
/// stamps whatever it wants seen onto the grid before calling.
pub(crate) struct MazeWorld<'a> {
    pub grid: &'a Grid,
    pub pipe_pairs: &'a [TeleportEdge],
}

/// Reachability across every world at once.
pub(crate) struct MazeReach {
    per_world: Vec<Vec<bool>>,
    cols: Vec<usize>,
    /// Which worlds' canoes ended up usable — handed to [`walk_maze_cost`] so
    /// it does not have to re-derive the fixpoint, which is the only reader.
    #[cfg_attr(not(test), allow(dead_code))]
    canoe_on: Vec<bool>,
}

impl MazeReach {
    pub(crate) fn contains(&self, (world, (r, c)): MazePos) -> bool {
        self.per_world[world][r * self.cols[world] + c]
    }

    /// Reachable cells in one world. Test-only: it exists so
    /// `maze_walk_matches_the_per_world_walker` can assert that walking one
    /// world with no links reaches nothing in the other seven. Also the
    /// constructive fill's territory measure — how much a gate reveals.
    pub(crate) fn world_len(&self, world: usize) -> usize {
        self.per_world[world].iter().filter(|&&b| b).count()
    }
}

/// Minimum cost to reach each cell, in whatever unit the cost function
/// charges. Unreachable cells are absent.
///
/// Test-only, with [`walk_maze_cost`]: pricing a maze in levels is what the
/// censuses do, and a shipped run never asks.
#[cfg(test)]
pub(crate) struct MazeCost {
    per_world: Vec<Vec<u32>>,
    prev: Vec<Vec<Option<MazePos>>>,
    cols: Vec<usize>,
}

#[cfg(test)]
impl MazeCost {
    pub(crate) fn get(&self, (world, (r, c)): MazePos) -> Option<u32> {
        match self.per_world[world][r * self.cols[world] + c] {
            u32::MAX => None,
            n => Some(n),
        }
    }

    /// The cheapest route to `target`, start first. Empty when it is
    /// unreachable. Used to charge each level on a play-through once: the
    /// caller marks everything on the route cleared before pricing the next
    /// leg.
    pub(crate) fn path_to(&self, target: MazePos) -> Vec<MazePos> {
        if self.get(target).is_none() {
            return Vec::new();
        }
        let mut out = vec![target];
        let mut at = target;
        while let Some(p) = self.prev[at.0][at.1.0 * self.cols[at.0] + at.1.1] {
            out.push(p);
            at = p;
        }
        out.reverse();
        out
    }
}

/// Directed links, indexed for lookup.
type LinkLookup = HashMap<MazePos, Vec<MazePos>>;

fn link_lookup(links: &[(MazePos, MazePos)]) -> LinkLookup {
    let mut out = LinkLookup::new();
    for &(from, to) in links {
        out.entry(from).or_default().push(to);
    }
    out
}

/// Canoe edges for one world, or nothing when the boat is not usable yet.
fn canoe_lookups(worlds: &[MazeWorld], canoe_on: &[bool]) -> Vec<TeleportLookup> {
    worlds
        .iter()
        .enumerate()
        .map(|(wi, w)| {
            if canoe_on[wi] {
                teleport_lookup(&rom_data::active_canoe_edges(wi, w.grid.eights_are_wild))
            } else {
                TeleportLookup::new()
            }
        })
        .collect()
}

/// The move expansion both walkers share, and the only copy of `reach_from`'s
/// traversal in the maze code.
///
/// Calls `visit` with every cell one 2-tile move or one intra-world teleport
/// away from `pos`. The target tile is a sink: nothing is visited from it. The
/// caller applies cross-world links separately, because those fire from a sink
/// too.
fn expand(
    grid: &Grid,
    pos: Pos,
    pipes: &TeleportLookup,
    canoes: &TeleportLookup,
    is_start: bool,
    mut visit: impl FnMut(Pos),
) {
    let (r, c) = pos;
    let tile_here = grid.get(r, c);
    if (tile_here == TILE_AIRSHIP || tile_here == TILE_BOWSER) && !is_start {
        return;
    }

    for &(dr, dc, is_horz) in &DIRECTIONS {
        let pr = r as i16 + dr as i16;
        let pc = c as i16 + dc as i16;
        if pr < 0 || pr >= grid.rows() as i16 || pc < 0 || pc >= grid.cols as i16 {
            continue;
        }
        let path_tile = grid.get(pr as usize, pc as usize);
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
        visit((nr, nc));
    }

    for lookup in [pipes, canoes] {
        if let Some(dests) = lookup.get(&pos) {
            for &dest in dests {
                visit(dest);
            }
        }
    }
}

/// One reachability pass with a fixed canoe state.
fn reach_pass(
    worlds: &[MazeWorld],
    pipes: &[TeleportLookup],
    canoe_on: &[bool],
    links: &LinkLookup,
    start: MazePos,
    cols: &[usize],
) -> Vec<Vec<bool>> {
    let canoes = canoe_lookups(worlds, canoe_on);
    let mut seen: Vec<Vec<bool>> =
        worlds.iter().map(|w| vec![false; w.grid.rows() * w.grid.cols]).collect();
    let mut queue = VecDeque::new();
    seen[start.0][start.1.0 * cols[start.0] + start.1.1] = true;
    queue.push_back(start);

    while let Some((wi, pos)) = queue.pop_front() {
        let mut push = |dw: usize, dpos: Pos| {
            let i = dpos.0 * cols[dw] + dpos.1;
            if !seen[dw][i] {
                seen[dw][i] = true;
                queue.push_back((dw, dpos));
            }
        };
        expand(worlds[wi].grid, pos, &pipes[wi], &canoes[wi], (wi, pos) == start, |p| push(wi, p));
        if let Some(dests) = links.get(&(wi, pos)) {
            for &(dw, dpos) in dests {
                push(dw, dpos);
            }
        }
    }
    seen
}

/// Walk every world at once from a single starting cell.
///
/// **`worlds` is indexed by world index** — always all eight, in ROM order.
/// The canoe tables are keyed on world index (the coordinates are not
/// world-unique), so a partial or reordered slice would silently walk the
/// wrong water.
///
/// `links` are DIRECTED `(from, to)` teleports: a telepad is one-way by
/// construction, and the airship spine is one-way by design. That is the only
/// structural difference from a pipe pair, which `walk_reachable` already
/// models as a bidirectional teleport.
pub(crate) fn walk_maze(
    worlds: &[MazeWorld],
    links: &[(MazePos, MazePos)],
    start: MazePos,
) -> MazeReach {
    let lookup = link_lookup(links);
    let pipes: Vec<TeleportLookup> = worlds.iter().map(|w| teleport_lookup(w.pipe_pairs)).collect();
    let cols: Vec<usize> = worlds.iter().map(|w| w.grid.cols).collect();

    // The canoe fixpoint. A pad can drop the player straight onto a dock in a
    // world they have no other route into, so each pass may switch a boat on
    // that the previous one could not reach; enabling one only ever grows the
    // reachable set, so this converges in at most one round per world.
    let mut canoe_on = vec![false; worlds.len()];
    loop {
        let per_world = reach_pass(worlds, &pipes, &canoe_on, &lookup, start, &cols);
        let mut changed = false;
        for (wi, world) in worlds.iter().enumerate() {
            if canoe_on[wi] {
                continue;
            }
            let docks = rom_data::active_canoe_edges(wi, world.grid.eights_are_wild);
            if docks.iter().any(|&((r, c), _)| per_world[wi][r * cols[wi] + c]) {
                canoe_on[wi] = true;
                changed = true;
            }
        }
        if !changed {
            return MazeReach { per_world, cols, canoe_on };
        }
    }
}

/// The same walk, priced. `cost(cell)` is what stepping ONTO that cell costs —
/// 1 for a level or fortress the player has to beat to pass, 0 for everything
/// they can simply walk over.
///
/// Dijkstra rather than BFS because the graph is mostly zero-weight; the grids
/// are 9x64 so the heap is never the expensive part. Canoe state is taken from
/// a prior [`walk_maze`] so the fixpoint is not paid twice.
#[cfg(test)]
pub(crate) fn walk_maze_cost(
    worlds: &[MazeWorld],
    links: &[(MazePos, MazePos)],
    start: MazePos,
    reach: &MazeReach,
    cost: impl Fn(MazePos) -> u32,
) -> MazeCost {
    let lookup = link_lookup(links);
    let pipes: Vec<TeleportLookup> = worlds.iter().map(|w| teleport_lookup(w.pipe_pairs)).collect();
    let canoes = canoe_lookups(worlds, &reach.canoe_on);
    let cols: Vec<usize> = worlds.iter().map(|w| w.grid.cols).collect();
    let mut best: Vec<Vec<u32>> =
        worlds.iter().map(|w| vec![u32::MAX; w.grid.rows() * w.grid.cols]).collect();
    let mut prev: Vec<Vec<Option<MazePos>>> =
        worlds.iter().map(|w| vec![None; w.grid.rows() * w.grid.cols]).collect();

    // `Reverse` so the BinaryHeap pops the cheapest first.
    let mut heap = BinaryHeap::new();
    let start_cost = cost(start);
    best[start.0][start.1.0 * cols[start.0] + start.1.1] = start_cost;
    heap.push(std::cmp::Reverse((start_cost, start)));

    while let Some(std::cmp::Reverse((c, (wi, pos)))) = heap.pop() {
        if c > best[wi][pos.0 * cols[wi] + pos.1] {
            continue;
        }
        let mut relax = |dw: usize, dpos: Pos| {
            let next = c + cost((dw, dpos));
            let i = dpos.0 * cols[dw] + dpos.1;
            if next < best[dw][i] {
                best[dw][i] = next;
                prev[dw][i] = Some((wi, pos));
                heap.push(std::cmp::Reverse((next, (dw, dpos))));
            }
        };
        expand(worlds[wi].grid, pos, &pipes[wi], &canoes[wi], (wi, pos) == start, |p| relax(wi, p));
        if let Some(dests) = lookup.get(&(wi, pos)) {
            for &(dw, dpos) in dests {
                relax(dw, dpos);
            }
        }
    }

    MazeCost { per_world: best, prev, cols }
}
