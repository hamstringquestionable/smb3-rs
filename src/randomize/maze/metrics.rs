//! What a finished maze costs a player, in the unit the player actually feels:
//! **levels beaten**.
//!
//! Nothing in the per-world route scorer answers this. `route_choice` prices a
//! route in abstract points to compare two routes on one map; the question here
//! is a whole-game number a player would recognise — "this seed takes about
//! twenty levels" — and it has to be answered across eight grids with the
//! lock/key dependency in the middle of it.
//!
//! * [`shortest_lower_bound`] — the shipping number: a lower bound on the
//!   shortest route, in levels, forts and airships played. The content floor
//!   judges every deal by it.
//! * `completion_cost` (test-only) — what a "lazy" play-through beats: always
//!   the cheapest fort next, needed or not. It was the floor until 2026-10;
//!   measured against real races it predicted almost nothing (r = 0.28 against
//!   winners' times), because most of what it counts is detours a real player
//!   does not take. Kept for the censuses.
//! * `required_levels` (test-only) — how many levels the player has **no
//!   choice** about, in the strict sense that removing one makes the game
//!   unwinnable. This is the mandatory core; everything else is a route
//!   decision.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

use super::walk::MazePos;
#[cfg(test)]
use super::walk::{walk_maze, walk_maze_cost};
use super::{FortRef, GlobalState};
use crate::randomize::overworld::build::SlotKind;
use crate::randomize::rom_data;

/// A play-through's price, in content beaten.
#[cfg(test)]
#[derive(Clone, Debug, Default)]
pub(crate) struct CompletionCost {
    /// Levels and fortresses beaten on the way to the castle.
    pub(crate) content: usize,
    /// How many of those were fortresses — the keys, as opposed to the road.
    pub(crate) forts: usize,
    /// Fortresses the run had to beat that were NOT on the direct route: the
    /// detours the lock/key structure forced.
    pub(crate) detours: usize,
    /// Airships the run had to clear purely to satisfy the wand gate.
    pub(crate) wand_detours: usize,
    /// False when the castle was never reached.
    pub(crate) reached: bool,
}

/// Simulate a player who beats exactly what they must.
///
/// A level tile in SMB3 cannot be walked past — the player has to clear it —
/// so "how far is the castle" is naturally measured in levels, and a Dijkstra
/// that charges 1 for stepping onto an uncleared level or fortress and 0 for
/// everything else answers it directly.
///
/// The lock/key dependency is what makes it more than one Dijkstra. The loop
/// is a lazy player: price the route to the castle with the keys currently
/// held; if it is not reachable, go and beat the **cheapest** fortress that is,
/// which opens its lock, and price it again. Every cell on a route taken is
/// marked cleared, so a level is charged once however many times it is walked
/// over afterwards — which is what the engine does.
///
/// It is an **upper bound** on the true minimum, because a lazy player takes
/// the cheapest next fort rather than the one that leads to the cheapest whole
/// run. Getting the exact minimum means searching over key orders, which is
/// exponential; the bound is tight enough to answer "about how long is this
/// game" and it is honest about which way it errs.
#[cfg(test)]
pub(crate) fn completion_cost(state: &GlobalState) -> CompletionCost {
    let bases = state.base_grids(&HashSet::new());
    let links = state.links();

    // Content cells, and which of them is a fortress.
    let mut charged: HashSet<MazePos> = HashSet::new();
    let mut fort_at: Vec<(FortRef, MazePos)> = Vec::new();
    for w in state.worlds.iter().filter(|w| state.in_maze[w.world_idx]) {
        for slot in &w.slots {
            match slot.kind {
                SlotKind::Level => {
                    charged.insert((w.world_idx, slot.pos));
                }
                SlotKind::Fortress => {
                    charged.insert((w.world_idx, slot.pos));
                    fort_at.push((
                        FortRef { world: w.world_idx, section: slot.section },
                        (w.world_idx, slot.pos),
                    ));
                }
                _ => {}
            }
        }
    }

    // Airships are wands, and a wand is a thing the player has to go and get.
    // Everything but the spine's last world (which holds the castle) has one.
    let wand_tiles: Vec<MazePos> = state.wand_tiles();

    let mut cleared: HashSet<MazePos> = HashSet::new();
    let mut beaten: HashSet<FortRef> = HashSet::new();
    let mut held: HashSet<MazePos> = HashSet::new();
    let mut out = CompletionCost::default();

    loop {
        let shut = state.shut_locks(&beaten);
        // No keys: exact while `state.gates` is empty, which it is for every
        // seed today. A gated maze wants this loop to accumulate found keys
        // the way it already accumulates beaten forts.
        let view = state.view(&bases, &shut, &HashSet::new());
        let reach = walk_maze(&view, &links, state.start);
        let cost = walk_maze_cost(&view, &links, state.start, &reach, |p| {
            u32::from(charged.contains(&p) && !cleared.contains(&p))
        });

        // Wands, when the gate still wants them and one is within reach. The
        // gate on World 8's bridge will not lift without K of them, so a run
        // that could walk to the castle still has to go and clear airships —
        // which is the whole reason it exists: without it a pad chain finishes
        // some seeds in ONE level (measured).
        //
        // "Within reach" is the load-bearing half. An airship that is still
        // behind a lock is not a dead end, it is a fortress away, so this
        // falls through to the fort loop below rather than giving up. Getting
        // that wrong made K >= 1 look unwinnable in two seeds out of three.
        if held.len() < state.wands_required as usize {
            let next = wand_tiles
                .iter()
                .filter(|p| !held.contains(p))
                .filter_map(|&p| cost.get(p).map(|c| (c, p)))
                .min_by_key(|&(c, p)| (c, p));
            if let Some((price, pos)) = next {
                out.content += price as usize;
                out.wand_detours += 1;
                for p in cost.path_to(pos) {
                    cleared.insert(p);
                }
                held.insert(pos);
                continue;
            }
        } else if let Some(price) = cost.get(state.goal) {
            out.content += price as usize;
            out.forts += cost
                .path_to(state.goal)
                .iter()
                .filter(|p| fort_at.iter().any(|(_, fp)| fp == *p) && !cleared.contains(p))
                .count();
            out.reached = true;
            return out;
        }

        // The castle is gated. Beat the cheapest fortress still standing.
        let next = fort_at
            .iter()
            .filter(|(f, _)| !beaten.contains(f))
            .filter_map(|&(f, pos)| cost.get(pos).map(|c| (c, f, pos)))
            .min_by_key(|&(c, f, _)| (c, f.world, f.section));
        let Some((price, fort, pos)) = next else {
            return out; // nothing left to open: unsolvable
        };
        out.content += price as usize;
        out.forts += 1;
        out.detours += 1;
        for p in cost.path_to(pos) {
            cleared.insert(p);
        }
        beaten.insert(fort);
    }
}

/// Levels the player has no choice about: blocking one makes the game
/// unwinnable.
///
/// The same cut-vertex question `WorldState::forced_forts` asks of fortresses,
/// asked of every level across the whole maze. A level tile is made
/// impassable — not merely uncleared, but a wall — and the global fixpoint is
/// re-run. If the castle can no longer be reached, or a fortress can no longer
/// be beaten, that level was mandatory.
///
/// Expensive: one global fixpoint per level, ~62 per seed. A census
/// instrument, not something to call in a build — the floor uses
/// [`shortest_lower_bound`] instead.
///
/// **Returns `None` when the maze was not winnable to begin with, and that
/// guard is not paranoia — it is a bug this instrument actually caused.** The
/// question asked per level is "block it; is the game still solvable?" On a
/// maze that was never solvable the answer is "no" for every level, so the
/// count comes back as *every level in the game* and reads as a map of pure
/// corridors. Two seeds reported 62 of 62 that way and the figure reached a
/// census table before anyone noticed the mazes behind it had unreachable
/// castles and nine unbeatable fortresses between them.
///
/// A lying instrument is worst exactly when something upstream is already
/// broken, which is when you are reading it most carefully.
#[cfg(test)]
pub(crate) fn required_levels(state: &GlobalState) -> Option<usize> {
    if !state.spheres().solvable {
        return None;
    }
    let levels: Vec<MazePos> = state
        .worlds
        .iter()
        .filter(|w| state.in_maze[w.world_idx])
        .flat_map(|w| {
            w.slots.iter().filter(|s| s.kind == SlotKind::Level).map(move |s| (w.world_idx, s.pos))
        })
        .collect();

    let mut count = 0;
    let mut blocked = HashSet::new();
    for &cell in &levels {
        blocked.clear();
        blocked.insert(cell);
        if !state.spheres_with_blocked(&blocked).solvable {
            count += 1;
        }
    }
    Some(count)
}

/// **A lower bound on the shortest route through a maze**, in levels, forts
/// and airships played — what [`super::CONTENT_FLOOR`] judges a deal by.
///
/// The exact shortest route cannot be computed per deal (`maze_route_census`
/// takes seconds to minutes per seed, and does not always finish). This is the
/// optimistic estimate that census prunes with: it is never more than the
/// truth, so "at least 10" is a guarantee, and it tracks the truth closely
/// enough to be useful — r = 0.78 against the exact shortest over 1,000 seeds
/// at K=3, 0.84 at K=0, and r = 0.63 against real race winners' times, where
/// the lazy play-through it replaced managed 0.28. About ten times cheaper
/// than that play-through, too (0.12 ms against 1.2 ms native).
///
/// **How.** One Dijkstra over every cell of every world, charging 1 for
/// entering a level, fort or airship not yet cleared. Locks are honoured
/// lightly: crossing one costs nothing itself, but nobody stands past it more
/// cheaply than its fort costs to reach, so a lock edge arrives at
/// `max(cost so far, cost of its fort)`, and the fort costs are re-solved
/// until they settle. The max rather than the sum is what keeps it optimistic
/// — the walk to a fort and the walk past its lock may share levels. Wands
/// still owed are a second target: the run must also reach the K-th cheapest
/// wand airship.
///
/// The move graph is the walker's — 2-tile steps over valid path tiles,
/// pipes, every canoe (whether or not its dock is reachable yet — optimistic
/// again), pads and the airship spine, and nothing out of an airship or castle
/// tile except the global start — built once per maze.
pub(crate) struct ShortestBound {
    // Only `cell` reads these; production asks by flat index already.
    #[cfg(test)]
    offset: Vec<usize>,
    #[cfg(test)]
    cols: Vec<usize>,
    edges: Vec<Vec<(usize, Option<FortRef>)>>,
    /// Cells that cost 1 to enter: every level, fort and airship.
    played: Vec<bool>,
    forts: Vec<(FortRef, usize)>,
    wands: Vec<usize>,
    start: usize,
    goal: usize,
    k: usize,
}

impl ShortestBound {
    pub(crate) fn new(state: &GlobalState) -> Self {
        let grids = state.base_grids(&HashSet::new());
        let cols: Vec<usize> = grids.iter().map(|g| g.cols).collect();
        let mut offset = Vec::with_capacity(grids.len());
        let mut cells = 0;
        for g in &grids {
            offset.push(cells);
            cells += g.rows() * g.cols;
        }
        let idx = |(w, (r, c)): MazePos| offset[w] + r * cols[w] + c;

        let lock_fort: HashMap<MazePos, FortRef> =
            state.locks.iter().filter_map(|l| l.fort.map(|f| ((l.world, l.pos), f))).collect();
        let mut edges: Vec<Vec<(usize, Option<FortRef>)>> = vec![Vec::new(); cells];
        for (w, g) in grids.iter().enumerate() {
            let inside = |r: i32, c: i32| {
                (0..g.rows() as i32).contains(&r) && (0..g.cols as i32).contains(&c)
            };
            for r in 0..g.rows() {
                for c in 0..g.cols {
                    let tile = g.get(r, c);
                    if (tile == rom_data::TILE_AIRSHIP || tile == rom_data::TILE_BOWSER)
                        && (w, (r, c)) != state.start
                    {
                        continue;
                    }
                    for (dr, dc, horz) in
                        [(0i32, 1i32, true), (0, -1, true), (1, 0, false), (-1, 0, false)]
                    {
                        let (pr, pc) = (r as i32 + dr, c as i32 + dc);
                        let (nr, nc) = (r as i32 + 2 * dr, c as i32 + 2 * dc);
                        if !inside(pr, pc) || !inside(nr, nc) {
                            continue;
                        }
                        let path = (pr as usize, pc as usize);
                        let land = (nr as usize, nc as usize);
                        let valid = if horz { rom_data::VALID_HORZ } else { rom_data::VALID_VERT };
                        if valid.contains(&g.get(path.0, path.1))
                            && !rom_data::BACKGROUND_TILES.contains(&g.get(land.0, land.1))
                        {
                            edges[idx((w, (r, c)))]
                                .push((idx((w, land)), lock_fort.get(&(w, path)).copied()));
                        }
                    }
                }
            }
            let canoes = rom_data::active_canoe_edges(w, g.eights_are_wild);
            for &(a, b) in state.worlds[w].pipe_pairs.iter().chain(canoes.iter()) {
                edges[idx((w, a))].push((idx((w, b)), None));
                edges[idx((w, b))].push((idx((w, a)), None));
            }
        }
        for (from, to) in state.links() {
            edges[idx(from)].push((idx(to), None));
        }

        let mut played = vec![false; cells];
        let mut forts = Vec::new();
        for w in &state.worlds {
            for s in &w.slots {
                match s.kind {
                    SlotKind::Level => played[idx((w.world_idx, s.pos))] = true,
                    SlotKind::Fortress => {
                        let cell = idx((w.world_idx, s.pos));
                        played[cell] = true;
                        forts.push((FortRef { world: w.world_idx, section: s.section }, cell));
                    }
                    _ => {}
                }
            }
            if let Some(t) = w.target
                && (w.world_idx, t) != state.goal
            {
                played[idx((w.world_idx, t))] = true;
            }
        }
        let wands = state.wand_tiles().into_iter().map(idx).collect();

        ShortestBound {
            start: idx(state.start),
            goal: idx(state.goal),
            k: usize::from(state.wands_required),
            #[cfg(test)]
            offset,
            #[cfg(test)]
            cols,
            edges,
            played,
            forts,
            wands,
        }
    }

    /// The flat index of a cell — the unit [`Self::estimate`]'s `cleared` is
    /// asked in.
    #[cfg(test)]
    pub(crate) fn cell(&self, (w, (r, c)): MazePos) -> usize {
        self.offset[w] + r * self.cols[w] + c
    }

    /// The fewest levels, forts and airships still to play, with `cleared`
    /// naming the cells already played (they cost nothing). `None` when the
    /// castle cannot be reached even optimistically.
    pub(crate) fn estimate(&self, cleared: impl Fn(usize) -> bool) -> Option<usize> {
        let enter = |cell: usize| u32::from(self.played[cell] && !cleared(cell));
        let mut fort_cost: HashMap<FortRef, u32> = self
            .forts
            .iter()
            .map(|&(f, cell)| (f, if cleared(cell) { 0 } else { u32::MAX }))
            .collect();
        loop {
            let mut dist = vec![u32::MAX; self.edges.len()];
            let mut heap = BinaryHeap::new();
            dist[self.start] = enter(self.start);
            heap.push(Reverse((dist[self.start], self.start)));
            while let Some(Reverse((d, u))) = heap.pop() {
                if d > dist[u] {
                    continue;
                }
                for &(v, lock) in &self.edges[u] {
                    let gate = lock.map_or(0, |f| fort_cost[&f]);
                    if gate == u32::MAX {
                        continue;
                    }
                    let next = d.max(gate) + enter(v);
                    if next < dist[v] {
                        dist[v] = next;
                        heap.push(Reverse((next, v)));
                    }
                }
            }
            // Opening a lock only ever lowers costs, so this converges.
            let mut changed = false;
            for &(f, cell) in &self.forts {
                if dist[cell] < fort_cost[&f] {
                    fort_cost.insert(f, dist[cell]);
                    changed = true;
                }
            }
            if changed {
                continue;
            }
            let mut need = dist[self.goal];
            if self.k > 0 {
                let mut owed: Vec<u32> =
                    self.wands.iter().map(|&c| if cleared(c) { 0 } else { dist[c] }).collect();
                owed.sort_unstable();
                need = need.max(*owed.get(self.k - 1)?);
            }
            return (need != u32::MAX).then_some(need as usize);
        }
    }
}

/// The floor's question for a finished maze: [`ShortestBound`] from scratch.
pub(crate) fn shortest_lower_bound(state: &GlobalState) -> Option<usize> {
    ShortestBound::new(state).estimate(|_| false)
}
