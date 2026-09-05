//! World maze — the global verifier (design phase 2, build-order step 1).
//!
//! `docs/world_maze_design.md` is the design record; this module is its first
//! executable piece. Nothing here writes to the ROM, and nothing in the
//! randomizer calls it — it is a **measurement instrument**, which is why the
//! whole module is `#[cfg(test)]`. That also sidesteps the ungating trap the
//! phase-1 modules hit: unreferenced code fails CI's wasm clippy pass, and
//! `#[allow(dead_code)]` is not an answer this project accepts. The gate
//! question arrives with the generator, not before it.
//!
//! ## What it does
//!
//! [`GlobalState`] wraps eight finished [`WorldState`]s plus a directed edge
//! set (telepads, and the airship spine). [`GlobalState::spheres`] runs
//! `WorldState::completable_sealed` one level up — close every lock, walk all
//! eight grids at once, beat every fort the walk reaches, open the locks those
//! forts hold, repeat — and returns the per-sphere spoiler log rather than a
//! bool.
//!
//! Everything the fixpoint rests on is **monotone**: pads are permanent, forts
//! are permanent, and the spine is repeatable because neither `TILE_AIRSHIP`
//! nor `TILE_BOWSER` appears in `Map_Removable_Tiles` or
//! `Map_Completable_Tiles`, so the engine never marks them.
//!
//! The map walker reads rocks as walls, so this **is** the zero-hammer run
//! (invariant 2) and no separate mode is needed for it.
//!
//! ## The null model
//!
//! [`pads`] holds the crude uniform pad placer. Together with an unmodified
//! eight-world [`build`](crate::randomize::overworld_build::build) and the
//! identity spine it makes the null model the charter asks for: "what happens
//! if we connect eight worlds and change nothing else". Its census is the
//! baseline every later lever is argued as a delta against.

use std::collections::HashSet;

use super::map_walker::{MazePos, MazeWorld, walk_maze, walk_reachable};
use super::overworld_build::{BuildResult, SlotKind, WorldState, from_built, stamp_slots};
use super::rom_data::{Grid, Pos};

pub(crate) mod pads;
mod tests;

/// A directed edge the engine can traverse repeatedly.
#[derive(Clone, Copy, Debug)]
pub(crate) enum MazeEdge {
    /// A telepad: step on `from`, arrive at `to`. One-way by construction —
    /// a two-way link is two pads and two arrival ids.
    Pad { from: MazePos, to: MazePos },
    /// The spine: clearing `from_world`'s airship deposits the player on
    /// `to_world`'s start tile.
    Airship { from_world: usize, to_world: usize },
}

/// Which fortress opens a lock. `section` is the fort's per-world section
/// index — the same key `LockAssignment::fort_section` uses, so no new fort
/// numbering has to be invented (and the ROM-side foreign-lock hook keys on
/// position, not on an id, for the same reason).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct FortRef {
    pub world: usize,
    pub section: usize,
}

/// A lock in the maze. Unlike `LockAssignment` its fort may live in any world.
#[derive(Clone, Debug)]
pub(crate) struct MazeLock {
    pub world: usize,
    pub pos: Pos,
    pub gap_tile: u8,
    pub replace_tile: u8,
    /// The fort that opens it, or `None` until the (not yet written)
    /// key-assignment fill assigns one. An unassigned lock never opens.
    pub fort: Option<FortRef>,
}

impl MazeLock {
    /// A lock whose fort lives in another world — the mode's headline shape.
    /// Derived, never stored.
    pub(crate) fn is_foreign(&self) -> bool {
        self.fort.is_some_and(|f| f.world != self.world)
    }
}

/// The airship order every null-model measurement uses: worlds in ROM order.
/// The last entry must hold Bowser's castle, which is the goal.
pub(crate) const IDENTITY_SPINE: [usize; 8] = [0, 1, 2, 3, 4, 5, 6, 7];

/// Eight worlds and the edges between them. A thin wrapper: the per-world
/// state is the existing [`WorldState`], untouched.
pub(crate) struct GlobalState {
    pub worlds: Vec<WorldState>,
    pub edges: Vec<MazeEdge>,
    pub locks: Vec<MazeLock>,
    pub start: MazePos,
    pub goal: MazePos,
    /// K of 7 — the difficulty dial. Recorded, not yet enforced: the wand gate
    /// is a cloned wall tile that does not exist yet, so [`Spheres`] REPORTS
    /// how many wands are collectable before the goal instead of gating on it.
    pub wands_required: u8,
}

impl GlobalState {
    /// Wrap a finished eight-world build as a maze, with no pads yet.
    ///
    /// Locks keep the fort the per-world builder paired them with — that is
    /// the null model: no key-assignment fill, so every lock stays local.
    pub(crate) fn from_build(result: &BuildResult, spine: &[usize; 8], wands_required: u8) -> Self {
        let worlds: Vec<WorldState> = result.worlds.iter().map(from_built).collect();
        for (i, w) in worlds.iter().enumerate() {
            assert_eq!(w.world_idx, i, "BuildResult worlds must be in world order");
        }

        let locks = worlds
            .iter()
            .flat_map(|w| {
                w.locks.iter().map(|l| MazeLock {
                    world: w.world_idx,
                    pos: l.pos,
                    gap_tile: l.gap_tile,
                    replace_tile: l.replace_tile,
                    fort: Some(FortRef { world: w.world_idx, section: l.fort_section }),
                })
            })
            .collect();

        let edges = spine
            .windows(2)
            .map(|p| MazeEdge::Airship { from_world: p[0], to_world: p[1] })
            .collect();

        let first = spine[0];
        let last = spine[7];
        let start = (first, worlds[first].start.expect("the spine's first world needs a START"));
        let goal = (last, worlds[last].target.expect("the spine's last world needs a target"));

        GlobalState { worlds, edges, locks, start, goal, wands_required }
    }

    /// Add telepads. Each pad tile owns one arrival row, so the count is
    /// bounded by [`pads::PAD_BUDGET`].
    pub(crate) fn add_pads(&mut self, pads: Vec<MazeEdge>) {
        self.edges.extend(pads);
    }

    /// Every pad in the maze, as `(from, to)`.
    pub(crate) fn pad_edges(&self) -> Vec<(MazePos, MazePos)> {
        self.edges
            .iter()
            .filter_map(|e| match *e {
                MazeEdge::Pad { from, to } => Some((from, to)),
                MazeEdge::Airship { .. } => None,
            })
            .collect()
    }

    /// Every edge as a directed teleport the walker understands.
    ///
    /// The spine resolves here rather than at construction because it is
    /// stated in worlds, not cells: it leaves the source world's target tile
    /// and lands on the destination's start tile.
    fn links(&self) -> Vec<(MazePos, MazePos)> {
        let mut out = Vec::new();
        for edge in &self.edges {
            match *edge {
                MazeEdge::Pad { from, to } => out.push((from, to)),
                MazeEdge::Airship { from_world, to_world } => {
                    if let (Some(from), Some(to)) =
                        (self.worlds[from_world].target, self.worlds[to_world].start)
                    {
                        out.push(((from_world, from), (to_world, to)));
                    }
                }
            }
        }
        out
    }

    /// The airship tiles that grant a wand: every spine world except the last,
    /// which holds the castle.
    fn wand_tiles(&self) -> Vec<MazePos> {
        self.edges
            .iter()
            .filter_map(|e| match *e {
                MazeEdge::Airship { from_world, .. } => {
                    self.worlds[from_world].target.map(|p| (from_world, p))
                }
                MazeEdge::Pad { .. } => None,
            })
            .collect()
    }

    /// Every cell a spoiler log cares about: placed content, world targets,
    /// and pad tiles. Plain path tiles are reachability plumbing, not events.
    fn content(&self) -> Vec<MazePos> {
        let mut out: Vec<MazePos> = Vec::new();
        for w in &self.worlds {
            out.extend(w.slots.iter().map(|s| (w.world_idx, s.pos)));
            out.extend(w.target.map(|p| (w.world_idx, p)));
        }
        out.extend(self.pad_edges().into_iter().map(|(from, _)| from));
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Every fortress in the maze, as `(who it is, where it is)`.
    fn forts(&self) -> Vec<(FortRef, Pos)> {
        self.worlds
            .iter()
            .flat_map(|w| {
                w.slots
                    .iter()
                    .filter(|s| s.kind == SlotKind::Fortress)
                    .map(|s| (FortRef { world: w.world_idx, section: s.section }, s.pos))
            })
            .collect()
    }

    /// Each world's grid with its slots stamped and no locks — the base every
    /// round of the fixpoint re-stamps its lock state onto.
    fn base_grids(&self) -> Vec<Grid> {
        self.worlds
            .iter()
            .map(|w| {
                let mut g = w.grid.clone();
                stamp_slots(&mut g, &w.slots);
                g
            })
            .collect()
    }

    /// The global fixpoint, and the mode's solver.
    ///
    /// Close every lock, walk all eight grids at once, beat every fort the
    /// walk reaches, open the locks those forts hold, repeat. Each round is a
    /// sphere. Because reachability is monotone the loop terminates in at most
    /// one round per fortress, and no assumption is made about the order the
    /// player beats forts in.
    pub(crate) fn spheres(&self) -> Spheres {
        let bases = self.base_grids();
        let links = self.links();
        let forts = self.forts();
        let content = self.content();
        let wand_tiles = self.wand_tiles();

        let mut open: HashSet<FortRef> = HashSet::new();
        let mut seen: HashSet<MazePos> = HashSet::new();
        let mut spheres: Vec<Sphere> = Vec::new();
        let mut goal_sphere = None;
        let mut wands_at_goal = 0;

        loop {
            let grids: Vec<Grid> = bases
                .iter()
                .enumerate()
                .map(|(wi, base)| {
                    let mut g = base.clone();
                    for lock in self.locks.iter().filter(|l| l.world == wi) {
                        let opens = lock.fort.is_some_and(|f| open.contains(&f));
                        g.set(
                            lock.pos.0,
                            lock.pos.1,
                            if opens { lock.replace_tile } else { lock.gap_tile },
                        );
                    }
                    g
                })
                .collect();
            let view: Vec<MazeWorld> = grids
                .iter()
                .zip(&self.worlds)
                .map(|(grid, w)| MazeWorld { grid, pipe_pairs: &w.pipe_pairs })
                .collect();
            let reach = walk_maze(&view, &links, self.start);

            let reached: Vec<MazePos> = content
                .iter()
                .copied()
                .filter(|&p| !seen.contains(&p) && reach.contains(p))
                .collect();
            seen.extend(reached.iter().copied());

            let beaten: Vec<FortRef> = forts
                .iter()
                .filter(|(f, pos)| !open.contains(f) && reach.contains((f.world, *pos)))
                .map(|&(f, _)| f)
                .collect();
            let opened: Vec<usize> = self
                .locks
                .iter()
                .enumerate()
                .filter(|(_, l)| l.fort.is_some_and(|f| beaten.contains(&f)))
                .map(|(i, _)| i)
                .collect();
            open.extend(beaten.iter().copied());

            let wands = wand_tiles.iter().filter(|&&p| reach.contains(p)).count();
            if goal_sphere.is_none() && reach.contains(self.goal) {
                goal_sphere = Some(spheres.len());
                wands_at_goal = wands;
            }

            // Every fort holds exactly one lock (the builder enforces it), so
            // "gates openable on entry" and "forts beatable on entry" are the
            // same number. `opened` is the one that stays right if that ever
            // stops holding.
            spheres.push(Sphere {
                reached,
                beaten: beaten.clone(),
                width: opened.len(),
                opened,
                wands,
            });

            // No new fort means no new key, and reachability is monotone —
            // nothing can grow again.
            if beaten.is_empty() {
                break;
            }
        }

        let unbeaten: Vec<FortRef> =
            forts.iter().map(|&(f, _)| f).filter(|f| !open.contains(f)).collect();
        Spheres {
            solvable: goal_sphere.is_some() && unbeaten.is_empty(),
            spheres,
            unbeaten,
            goal_sphere,
            wands_at_goal,
        }
    }

    /// `wands_are_collectable`: is a K-of-7 gate on the castle satisfiable —
    /// are at least `wands_required` airships clearable before the castle
    /// comes into reach? K = 0 (a pure maze) makes it vacuous, which is the
    /// null model's setting.
    pub(crate) fn wands_are_collectable(&self, spheres: &Spheres) -> bool {
        spheres.wands_at_goal >= self.wands_required as usize
    }

    /// Invariant 3, for one world: with **every lock closed**, does the start
    /// tile's region hold an exit — this world's own target, or a pad out of
    /// it?
    ///
    /// This is the whole of the game-over and whistle safety story. Game over,
    /// airship arrival and whistle travel all deposit the player on a start
    /// tile, so a start region with an ungated exit certifies all three. Dying
    /// on a pad-only island strands nobody: it drops you at the start tile,
    /// and the island is re-enterable by its pad from the other side.
    ///
    /// Deliberately a per-world walk, not a maze walk: what is being asked is
    /// whether the player can get OUT of where they were just deposited, using
    /// nothing they have not already got.
    pub(crate) fn start_region_has_exit(&self, world: usize) -> bool {
        let w = &self.worlds[world];
        let mut g = w.grid.clone();
        stamp_slots(&mut g, &w.slots);
        for lock in self.locks.iter().filter(|l| l.world == world) {
            g.set(lock.pos.0, lock.pos.1, lock.gap_tile);
        }
        let reach = walk_reachable(&g, &w.pipe_pairs, w.start, world);
        if w.target.is_some_and(|t| reach.contains(t)) {
            return true;
        }
        self.pad_edges().iter().any(|&((pw, pos), _)| pw == world && reach.contains(pos))
    }
}

/// One round of the fixpoint.
#[derive(Clone, Debug)]
pub(crate) struct Sphere {
    /// Content that became reachable this round (see [`GlobalState::content`]).
    pub reached: Vec<MazePos>,
    /// Fortresses beatable on entry to this sphere.
    pub beaten: Vec<FortRef>,
    /// Indices into `GlobalState::locks` that those fortresses open.
    pub opened: Vec<usize>,
    /// Gates openable on entry — the global choice metric. Width 1 is a
    /// corridor; width >= 2 is a real decision. The direct analogue, one level
    /// up, of the per-world builder's "at least 2 routes in the choice band".
    pub width: usize,
    /// Wands collectable by the end of this sphere (cumulative).
    pub wands: usize,
}

/// The fixpoint's output: a spoiler log, not a bool.
#[derive(Clone, Debug)]
pub(crate) struct Spheres {
    /// The castle is reachable AND every fortress is beatable. A fort the
    /// fixpoint never reaches is content sealed out of the game, which fails
    /// invariant 1 just as squarely as an unreachable castle.
    pub solvable: bool,
    pub spheres: Vec<Sphere>,
    pub unbeaten: Vec<FortRef>,
    /// The sphere in which the castle first became reachable.
    pub goal_sphere: Option<usize>,
    /// Wands collectable before the castle came into reach — what a K-of-7
    /// gate would have to be satisfied by.
    pub wands_at_goal: usize,
}

impl Spheres {
    /// The per-sphere spoiler log, the mode's primary debugging instrument.
    pub(crate) fn spoiler(&self) -> String {
        let mut out = String::new();
        for (i, s) in self.spheres.iter().enumerate() {
            let forts: Vec<String> =
                s.beaten.iter().map(|f| format!("W{}.{}", f.world + 1, f.section)).collect();
            out.push_str(&format!(
                "  sphere {i}: width {:<2} +{:<3} content  {} lock(s) open  wands {}  forts [{}]\n",
                s.width,
                s.reached.len(),
                s.opened.len(),
                s.wands,
                forts.join(" ")
            ));
        }
        match self.goal_sphere {
            Some(g) => out.push_str(&format!(
                "  goal reached in sphere {g} with {} wands\n",
                self.wands_at_goal
            )),
            None => out.push_str("  GOAL UNREACHABLE\n"),
        }
        if !self.unbeaten.is_empty() {
            let names: Vec<String> =
                self.unbeaten.iter().map(|f| format!("W{}.{}", f.world + 1, f.section)).collect();
            out.push_str(&format!("  UNBEATABLE FORTS: {}\n", names.join(" ")));
        }
        out
    }
}
