//! World maze — the generator, and the fixpoint that decides whether what it
//! generated is winnable.
//!
//! `docs/world_maze_design.md` is the design record. [`generate`] is the entry
//! point a shipped run calls; everything else here is what it is built out of.
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
//! ## The null model is history, not code
//!
//! Build-order step 1 shipped a uniform pad placer so the charter's null-model
//! baselines could be measured ("what happens if we connect eight worlds and
//! change nothing else"). Those numbers are in the design doc and the placer is
//! **deleted**: it emitted unpaired one-way pads landing on arbitrary slots, a
//! shape [`graph`] can no longer produce, so a census over it measured a maze
//! this module cannot emit. The baseline every later lever was argued against
//! is the table in the doc, not a second placer kept alive to reproduce it.

use rand::Rng;

use std::collections::HashSet;

use super::map_walker::walk_reachable;
use super::overworld_build::{BuildResult, SlotKind, WorldState, from_built, stamp_slots};
use super::rom_data::{self, Grid, Pos};
use walk::{MazePos, MazeWorld, walk_maze};

pub(crate) mod fill;
pub(crate) mod graph;
/// How long a generated maze is, in levels. A measurement instrument: the
/// censuses are its only readers, and a shipped run has nowhere to put the
/// answer.
#[cfg(test)]
pub(crate) mod metrics;
pub(crate) mod roles;
#[cfg(test)]
mod tests;
pub(crate) mod walk;
pub(crate) mod writer;

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
    /// The fort that opens it.
    ///
    /// `None` means **the lock is not installed** — the tile is left as open
    /// path. It is not "a lock nothing opens": a permanently sealed gate is a
    /// thing the generator must never produce, and making the empty case the
    /// harmless one means a half-finished fill degrades to a more open maze
    /// rather than an unwinnable one.
    pub fort: Option<FortRef>,
}

/// The airship order every null-model measurement uses: worlds in ROM order.
/// The last entry must hold Bowser's castle, which is the goal.
///
/// Test-only. A shipped run takes its spine from `world_order`'s table, which
/// is the whole reason the mode forces that option on; this constant exists so
/// a census can pin the spine and vary one thing at a time.
///
/// A spine need not name all eight worlds. `world_order` with `world_count < 7`
/// returns a shorter order, and the worlds it leaves out are still built, still
/// full of content, and still on the map — they simply have no airship edge
/// into them, so **the only way in is a telepad**. That is not a degradation of
/// the mode, it is the mode: a world you can only reach by pad is the closest
/// thing the terrain allows to the charter's pad-only island.
#[cfg(test)]
pub(crate) const IDENTITY_SPINE: [usize; 8] = [0, 1, 2, 3, 4, 5, 6, 7];

/// Wands the castle demands by default, of the 7 airships.
///
/// **Measured** (`maze_wand_gate_sweep`, 40 seeds per K). K is not a length
/// dial — it is a **floor** guarantee, and the sweep says so plainly:
///
/// | K | mean levels beaten | min | median |
/// |---|---|---|---|
/// | 0 | 26.6 | **1** | 29 |
/// | 3 | 27.8 | **16** | 28 |
/// | 5 | 29.8 | 16 | 29 |
/// | 7 | 36.4 | 24 | 38 |
///
/// At K=0 a pad chain can drop the player beside the castle and some seeds
/// finish in a **single level**. K=3 raises that floor to 16 while the median
/// run is *unchanged* (29 → 28) — the degenerate tail costs nothing to remove.
/// The floor then plateaus at 16 through K=5 and only moves again at 6-7, by
/// which point the median has climbed to 33 and 38. So 3 is where the dial
/// stops buying and starts charging.
pub(crate) const DEFAULT_WANDS_REQUIRED: u8 = 3;

/// Eight worlds and the edges between them. A thin wrapper: the per-world
/// state is the existing [`WorldState`], untouched.
pub(crate) struct GlobalState {
    pub worlds: Vec<WorldState>,
    pub edges: Vec<MazeEdge>,
    pub locks: Vec<MazeLock>,
    pub start: MazePos,
    pub goal: MazePos,
    /// Which worlds the spine names. **A world off the spine is not part of
    /// the game**, exactly as `world_count` means in standard mode: it is still
    /// built, still on the ROM and still full of content, but nothing requires
    /// it and its fortresses are not counted against solvability. If a telepad
    /// happens to land there, that content is a bonus.
    pub in_maze: [bool; 8],
    /// Cells no pad may stand on: the wandering Hammer Bro sprites' home
    /// tiles. `TILE_TELEPAD` is in `Map_Object_Forbid_LandingTiles`, so a
    /// marching bro cannot land on a pad — but its home cell comes from the
    /// sprite table rather than from a march, so a pad stamped there would
    /// start the game with a sprite parked on it. `BuiltWorld::hb_sprites` is
    /// the only place these are known — `from_built` does not carry them onto
    /// the `WorldState`.
    pub reserved: HashSet<MazePos>,
    /// K of 7 — the difficulty dial, and [`Self::spheres`] gates on it: the
    /// goal does not count as reached until K wands are collectable, which is
    /// exact because the gate cell is the only way into the castle.
    pub wands_required: u8,
}

impl GlobalState {
    /// Wrap a finished eight-world build as a maze, with no pads yet.
    ///
    /// Locks keep the fort the per-world builder paired them with — that is
    /// the null model: no key-assignment fill, so every lock stays local.
    pub(crate) fn from_build(result: &BuildResult, spine: &[usize], wands_required: u8) -> Self {
        assert!(spine.len() >= 2, "a spine needs a start and a castle");
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
        let last = *spine.last().expect("checked above");
        let start = (first, worlds[first].start.expect("the spine's first world needs a START"));
        let goal = (last, worlds[last].target.expect("the spine's last world needs a target"));

        let reserved = result
            .worlds
            .iter()
            .flat_map(|b| b.hb_sprites.iter().map(|s| (b.world_idx, s.grid_pos)))
            .collect();

        let mut in_maze = [false; 8];
        for &w in spine {
            in_maze[w] = true;
        }

        GlobalState { worlds, edges, locks, start, goal, in_maze, reserved, wands_required }
    }

    /// Add telepads. Each pad tile owns one arrival row, so the count is
    /// bounded by [`graph::PAD_BUDGET`].
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
    pub(crate) fn links(&self) -> Vec<(MazePos, MazePos)> {
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
    pub(crate) fn wand_tiles(&self) -> Vec<MazePos> {
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
        for w in self.worlds.iter().filter(|w| self.in_maze[w.world_idx]) {
            out.extend(w.slots.iter().map(|s| (w.world_idx, s.pos)));
            out.extend(w.target.map(|p| (w.world_idx, p)));
        }
        out.extend(self.pad_edges().into_iter().map(|(from, _)| from));
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Every fortress the game requires, as `(who it is, where it is)`.
    ///
    /// Scoped to the worlds the spine names: a fort in a world nothing requires
    /// is bonus content, and counting it would make every short spine report
    /// itself unwinnable.
    fn forts(&self) -> Vec<(FortRef, Pos)> {
        self.worlds
            .iter()
            .filter(|w| self.in_maze[w.world_idx])
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
    ///
    /// `blocked` cells are walled off AFTER stamping, which is what makes
    /// "would the game still be winnable without this level?" a one-line
    /// question (see [`metrics::required_levels`]). Any `BACKGROUND_TILES`
    /// member reads as a wall to the walker; the value never reaches the ROM.
    pub(crate) fn base_grids(&self, blocked: &HashSet<MazePos>) -> Vec<Grid> {
        self.worlds
            .iter()
            .map(|w| {
                let mut g = w.grid.clone();
                stamp_slots(&mut g, &w.slots);
                for &(_, pos) in blocked.iter().filter(|(wi, _)| *wi == w.world_idx) {
                    g.set(pos.0, pos.1, rom_data::BACKGROUND_TILES[0]);
                }
                g
            })
            .collect()
    }

    /// [`Self::base_grids`] with every lock stamped as `open` leaves it. The
    /// fixpoint and [`metrics::completion_cost`] both step through the same
    /// sequence of these, one per fort set, and they have to agree about what
    /// a given set of beaten forts makes walkable.
    pub(crate) fn locked_grids(&self, bases: &[Grid], open: &HashSet<FortRef>) -> Vec<Grid> {
        bases
            .iter()
            .enumerate()
            .map(|(wi, base)| {
                let mut g = base.clone();
                for lock in self.locks.iter().filter(|l| l.world == wi) {
                    // An uninstalled lock (`fort: None`) is open path.
                    let opens = lock.fort.is_none_or(|f| open.contains(&f));
                    let tile = if opens { lock.replace_tile } else { lock.gap_tile };
                    g.set(lock.pos.0, lock.pos.1, tile);
                }
                g
            })
            .collect()
    }

    /// The eight grids as the maze walkers take them.
    pub(crate) fn view<'a>(&'a self, grids: &'a [Grid]) -> Vec<MazeWorld<'a>> {
        grids
            .iter()
            .zip(&self.worlds)
            .map(|(grid, w)| MazeWorld { grid, pipe_pairs: &w.pipe_pairs })
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
        self.spheres_with_blocked(&HashSet::new())
    }

    /// The fixpoint with some cells walled off — the counterfactual
    /// [`metrics::required_levels`] asks 62 times per seed.
    pub(crate) fn spheres_with_blocked(&self, blocked: &HashSet<MazePos>) -> Spheres {
        let bases = self.base_grids(blocked);
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
            let grids = self.locked_grids(&bases, &open);
            let reach = walk_maze(&self.view(&grids), &links, self.start);

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
            // The wand gate. A cloned wall tile sits on the unique chokepoint
            // of World 8's bridge and lifts when K wands are held, so the
            // castle is *walkable to* long before it is *enterable*. Modelling
            // it as "the goal does not count as reached until K" is exact,
            // because the gate cell is the only way in — nothing else lies
            // beyond it. See `docs/world_maze_design.md`, "The wand gate".
            if goal_sphere.is_none()
                && reach.contains(self.goal)
                && wands >= self.wands_required as usize
            {
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
    #[cfg(test)]
    pub(crate) fn wands_are_collectable(&self, spheres: &Spheres) -> bool {
        spheres.wands_at_goal >= self.wands_required as usize
    }

    /// This world's reserved cells, as plain positions.
    pub(crate) fn reserved_in(&self, world: usize) -> HashSet<Pos> {
        self.reserved.iter().filter(|(w, _)| *w == world).map(|&(_, p)| p).collect()
    }

    /// **The safety invariant that replaces the charter's invariant 3.**
    ///
    /// Can the player get out of `world`'s start region with nothing but what
    /// they carry on arrival? Airship arrival, game over and whistle travel all
    /// deposit the player on a start tile, so a start region they can leave
    /// certifies all three.
    ///
    /// It is deliberately weaker than [`Self::start_region_has_exit`], and the
    /// difference is the point. The charter asked for an exit reachable with
    /// **zero keys**; that is stricter than safety needs, because a fortress
    /// inside the start region is a key the player can go and get. What
    /// actually soft-locks is a start region with no ungated exit AND no
    /// fortress that opens one. Measured, the strict form holds ~56% of the
    /// time and this one far more often, which is the difference between a
    /// placement phase and a cheap guard.
    ///
    /// **Only this world's own forts count**, which is the worst case and so
    /// the safe one: a lock the key-assignment fill paired with a foreign fort
    /// can never be opened from inside, and a player arriving for the first
    /// time has beaten nothing here.
    pub(crate) fn start_region_escapable(&self, world: usize) -> bool {
        let w = &self.worlds[world];
        let mut base = w.grid.clone();
        stamp_slots(&mut base, &w.slots);

        let mut exits: Vec<Pos> = self
            .pad_edges()
            .iter()
            .filter(|((pw, _), _)| *pw == world)
            .map(|&((_, pos), _)| pos)
            .collect();
        exits.extend(w.target);
        if exits.is_empty() {
            return false;
        }

        let forts: Vec<(usize, Pos)> = w
            .slots
            .iter()
            .filter(|s| s.kind == SlotKind::Fortress)
            .map(|s| (s.section, s.pos))
            .collect();
        let mut open: HashSet<usize> = HashSet::new();
        loop {
            let mut g = base.clone();
            for lock in self.locks.iter().filter(|l| l.world == world) {
                let opens = lock.fort.is_none_or(|f| f.world == world && open.contains(&f.section));
                let tile = if opens { lock.replace_tile } else { lock.gap_tile };
                g.set(lock.pos.0, lock.pos.1, tile);
            }
            let reach = walk_reachable(&g, &w.pipe_pairs, w.start, world);
            if exits.iter().any(|&e| reach.contains(e)) {
                return true;
            }
            let newly: Vec<usize> = forts
                .iter()
                .filter(|(id, pos)| !open.contains(id) && reach.contains(*pos))
                .map(|&(id, _)| id)
                .collect();
            if newly.is_empty() {
                return false;
            }
            open.extend(newly);
        }
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
    #[cfg(test)]
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
// Reason: production reads only `Spheres::solvable`, for the fallback guard in
// `generate`. Everything else is the spoiler log and the census surface, whose
// only reader is the test harness — the same shape, and the same reason, as
// `overworld_build::PhaseReport`.
#[allow(dead_code)]
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
// Reason: see `Sphere` above — `solvable` is the only field a shipped run
// reads, and the rest is what makes a failure diagnosable.
#[allow(dead_code)]
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
    ///
    /// Test-only: the censuses and every failing assertion print it, and
    /// nothing in a shipped run has anywhere to put it.
    #[cfg(test)]
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

/// What one generated maze came out as — the numbers a census reads and the
/// spoiler log a playtest reads.
// Reason: production reads only `spheres`, for the solvability guard in
// `generate`; the rest is the census and spoiler surface and the test harness
// is its only reader. Same shape, and the same reason, as
// `overworld_build::PhaseReport`.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct GenReport {
    pub spheres: Spheres,
    pub fill: fill::FillReport,
    pub pads: Vec<graph::PlacedPad>,
    /// Worlds whose start region cannot be escaped with what the player
    /// carries on arrival. **Must be empty**: each one is a seed that can
    /// strand a player.
    pub unsafe_worlds: Vec<usize>,
}

/// Build a maze from eight finished worlds.
///
/// The order is not arbitrary. Pads are planned first, because the fill's
/// safety test asks whether a world's start region has an exit and a pad is
/// one. The fill then moves keys around, re-testing that safety on every swap.
/// Anything still unsafe afterwards gets a hub pad out of whatever budget is
/// left — the last claim on the 16 ids, because by then it is the only one
/// that can make a seed unplayable.
pub(crate) fn generate<R: Rng>(
    result: &BuildResult,
    spine: &[usize],
    wands_required: u8,
    knobs: &graph::Knobs,
    rng: &mut R,
) -> (GlobalState, GenReport) {
    let mut state = GlobalState::from_build(result, spine, wands_required);

    let pads = graph::plan_pads(&state, knobs, rng);
    state.add_pads(pads.iter().map(|p| p.edge).collect());

    let fill = fill::assign_keys(&mut state, spine, knobs, rng);

    // Last-resort safety. `plan_pads` already gave a hub pad to every world
    // that needed one; a world still failing here either had no free site or
    // ran out of budget, and both are census outputs rather than errors.
    let mut unsafe_worlds: Vec<usize> =
        (0..state.worlds.len()).filter(|&wi| !state.start_region_escapable(wi)).collect();
    if !unsafe_worlds.is_empty() {
        let rescue = graph::rescue_pads(&state, &unsafe_worlds, knobs, rng);
        state.add_pads(rescue.iter().map(|p| p.edge).collect());
        unsafe_worlds.retain(|&wi| !state.start_region_escapable(wi));
    }

    let mut spheres = state.spheres();

    // Defence in depth, and the one guard this mode cannot do without.
    //
    // Every accept test in the fill already checks solvability and start-region
    // safety, so this should be unreachable. But "should be unreachable" is not
    // something a player can cash, and an unwinnable seed is the single failure
    // a maze cannot recover from — there is no way to notice it except by
    // playing to the wall. If it ever fires, fall back to the shape that is
    // solvable BY CONSTRUCTION: the per-world builder's own local locks, no
    // pads, the spine alone. `the_spine_alone_completes_the_maze` is what makes
    // that a guarantee rather than a hope.
    if !spheres.solvable || !unsafe_worlds.is_empty() {
        state = GlobalState::from_build(result, spine, wands_required);
        spheres = state.spheres();
        // RECOMPUTE, do not clear. The fallback drops the pads, and a hub pad
        // is the only rescue an unsafe start region has — so the fallback can
        // make escapability WORSE, and clearing the list here would report a
        // clean bill for the one case that needed reporting. This branch is
        // unreachable today (0 hub pads requested in every census arm, 100%
        // escapable), which is exactly why it must not lie if it ever fires.
        unsafe_worlds =
            (0..state.worlds.len()).filter(|&wi| !state.start_region_escapable(wi)).collect();
    }

    (state, GenReport { spheres, fill, pads, unsafe_worlds })
}
