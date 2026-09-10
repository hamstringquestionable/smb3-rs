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

use std::collections::{HashMap, HashSet};

use super::map_walker::walk_reachable_blocked;
use super::overworld_build::{
    BuildResult, FortRef, LockHint, SlotKind, WorldState, from_built, stamp_slots,
};
use super::rom_data::{self, Grid, Pos};
use walk::{MazePos, MazeWorld, walk_maze};

pub(crate) mod fill;
pub(crate) mod graph;
/// How long a generated maze is, in levels.
///
/// It began as a measurement instrument and [`CONTENT_FLOOR`] promoted it:
/// [`generate`] now prices every deal with [`metrics::completion_cost`] and
/// redeals the short ones, so this runs on the shipping path. The rest of the
/// module is still census-only.
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

/// A lock in the maze. Unlike `LockAssignment` its fort may live in any world.
#[derive(Clone, Debug)]
pub(crate) struct MazeLock {
    pub world: usize,
    pub pos: Pos,
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

/// The shortest run the mode will ship, in levels and fortresses beaten.
///
/// **Measured** (`maze_content_floor_census`, 300 grids). Without a floor the
/// length of a run is very nearly unmanaged: at K=0 it ran from **2** to 48,
/// and the wand gate only lifts the bottom of that — K is a floor, not a
/// length dial, and K=0 is a setting players like.
///
/// The floor exists because of what the spread is made of. Redealing the maze
/// on the *same* eight worlds moves `content` far more than changing worlds
/// does: the between-grid share of the variance is **12%** (sd 2.95 between
/// grids against 8.08 within one), and the median grid's twenty deals spanned
/// **28 levels**. A grid whose first deal came out under 14 has a per-grid mean
/// of 20.3 against 21.8 overall — it is an ordinary grid that got a bad deal,
/// not a grid that cannot produce a long game. So a redeal is the right lever,
/// and it is aimed at the maze layer rather than at the terrain.
///
/// 14 is where the cost curve is still cheap and the grids still clear it
/// easily:
///
/// | floor | K=0 redeal % | mean deals | K=0 min → | median → |
/// |---|---|---|---|---|
/// | 12 | 15% | 1.18 | 12 | 23 → 24 |
/// | **14** | **20%** | **1.26** | **14** | **23 → 25** |
/// | 16 | 26% | 1.36 | 16 | 23 → 26 |
/// | 18 | 33% | 1.50 | 18 | 23 → 26 |
///
/// 18 is where the terrain starts to bite — 3 grids of 200 cleared it twice or
/// less in twenty deals, so [`MAX_DEALS`] would begin shipping under-floor
/// seeds. At 14 no grid of 200 ever failed to clear it.
///
///
/// **Re-measured 2026-09-10, after the route Dijkstra became a radix heap**
/// (PR #234). Moving tie-breaks moves maps, so the floor's cost had to be
/// re-checked rather than assumed: at 14 it came out slightly *cheaper* —
/// K=0 redeals 18% of seeds (mean 1.22 deals, worst 4) against 20% / 1.26 / 6
/// before, K=3 9%, and nothing ships under the floor. The sweep table above
/// still dates from before that change, so read its 12 / 16 / 18 rows as
/// relative rather than current.
/// The redeal deliberately does **not** condition on landing just above the
/// floor: a rejected deal is redrawn from the whole distribution, so it lands
/// at a typical length. That is why the floor moves K=0's minimum from 2 to 14
/// while the median moves only 23 → 25, and the maximum not at all.
pub(crate) const CONTENT_FLOOR: usize = 14;

/// How many deals [`generate`] will pay for before keeping the best it saw.
///
/// The cap is a cost bound, not a correctness one — the loop keeps the longest
/// deal it has seen, so a seed that never clears [`CONTENT_FLOOR`] ships the
/// best available rather than failing. Worst observed at the shipping floor was
/// 6 deals in 300 seeds; 8 leaves margin without letting a pathological grid
/// spend 100 ms of a WASM budget that is already tight (a deal costs ~9.5 ms in
/// the browser, measured).
pub(crate) const MAX_DEALS: usize = 8;

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
                    fort: Some(l.fort),
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

    /// Which path cells are shut, per world, given the forts beaten so far.
    ///
    /// The fixpoint and [`metrics::completion_cost`] both step through the same
    /// sequence of these, one per fort set, and they have to agree about what a
    /// given set of beaten forts makes walkable. The constructive fill steps
    /// through the same sequence a third time.
    pub(crate) fn shut_locks(&self, open: &HashSet<FortRef>) -> Vec<HashSet<Pos>> {
        self.shut_locks_sealed(open, None)
    }

    /// [`Self::locked_grids`] with one lock held shut whatever opens it — the
    /// counterfactual [`Self::winnable_with_lock_sealed`] asks.
    pub(crate) fn shut_locks_sealed(
        &self,
        open: &HashSet<FortRef>,
        sealed: Option<usize>,
    ) -> Vec<HashSet<Pos>> {
        (0..self.worlds.len())
            .map(|wi| {
                self.locks
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| l.world == wi)
                    .filter(|(li, lock)| {
                        // An uninstalled lock (`fort: None`) is open path.
                        let opens =
                            Some(*li) != sealed && lock.fort.is_none_or(|f| open.contains(&f));
                        !opens
                    })
                    .map(|(_, lock)| lock.pos)
                    .collect()
            })
            .collect()
    }

    /// The eight worlds as the maze walkers take them: a grid, its pipes, and
    /// which of its path cells are shut.
    pub(crate) fn view<'a>(
        &'a self,
        grids: &'a [Grid],
        shut: &'a [HashSet<Pos>],
    ) -> Vec<MazeWorld<'a>> {
        grids
            .iter()
            .zip(&self.worlds)
            .zip(shut)
            .map(|((grid, w), blocked)| MazeWorld { grid, pipe_pairs: &w.pipe_pairs, blocked })
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
        self.spheres_inner(blocked, None)
    }

    fn spheres_inner(&self, blocked: &HashSet<MazePos>, sealed: Option<usize>) -> Spheres {
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
            let shut = self.shut_locks_sealed(&open, sealed);
            let reach = walk_maze(&self.view(&bases, &shut), &links, self.start);

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

    /// **Can the player still reach the castle if `lock` is never opened?**
    ///
    /// This is the question 1-F's secret exit asks. That exit hands out an item
    /// and skips the crystal ball, so the fortress is beaten but the lock stays
    /// shut — a real choice, and one the mode keeps. What it must never be is a
    /// choice that ends the run.
    ///
    /// **Deliberately weaker than [`Spheres::solvable`]**, and the difference is
    /// the whole point. `solvable` also demands that *every* fortress be
    /// beatable, because content sealed out of the game is a bug. But a fortress
    /// stranded behind a lock the player *chose* not to open is not sealed out —
    /// they can go back and beat it. Using the strict test here would reject
    /// almost every assignment and send the fill thrashing.
    ///
    /// The wand gate is still honoured: `goal_sphere` is only set once `K`
    /// wands are collectable, so this asks "reachable *and* enterable".
    pub(crate) fn winnable_with_lock_sealed(&self, lock: usize) -> bool {
        self.spheres_inner(&HashSet::new(), Some(lock)).goal_sphere.is_some()
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
            let shut: HashSet<Pos> = self
                .locks
                .iter()
                .filter(|l| l.world == world)
                .filter(|lock| {
                    !lock.fort.is_none_or(|f| f.world == world && open.contains(&f.section))
                })
                .map(|lock| lock.pos)
                .collect();
            let reach = walk_reachable_blocked(&base, &w.pipe_pairs, w.start, world, &shut);
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
        let shut: HashSet<Pos> =
            self.locks.iter().filter(|l| l.world == world).map(|l| l.pos).collect();
        let reach = walk_reachable_blocked(&g, &w.pipe_pairs, w.start, world, &shut);
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
    /// How many locks can be left shut forever, against how many the writer
    /// asked for — see [`fill::keep_n_sealable`].
    pub sealable: fill::Sealable,
    pub fill: fill::FillReport,
    pub pads: Vec<graph::PlacedPad>,
    /// Worlds whose start region cannot be escaped with what the player
    /// carries on arrival. **Must be empty**: each one is a seed that can
    /// strand a player.
    pub unsafe_worlds: Vec<usize>,
    /// Deals this seed paid for, 1..=[`MAX_DEALS`]. Anything above 1 is the
    /// content floor rejecting a short maze.
    pub deals: usize,
    /// What the kept deal priced at, in levels and fortresses beaten. Below
    /// [`CONTENT_FLOOR`] only when [`MAX_DEALS`] ran out.
    pub content: usize,
}

/// One deal of the maze layer: everything [`generate`] draws in a single
/// attempt, plus what that attempt priced at.
///
/// It exists because the content floor deals more than once and has to choose
/// between attempts. Nothing outside `generate` sees one.
struct Deal {
    /// Levels and fortresses a play-through beats, or 0 when the castle was
    /// never reached. See [`metrics::completion_cost`].
    content: usize,
    state: GlobalState,
    fill: fill::FillReport,
    pads: Vec<graph::PlacedPad>,
    unsafe_worlds: Vec<usize>,
}

/// Build a maze from eight finished worlds.
///
/// The order is not arbitrary. Pads are planned first, because the fill's
/// safety test asks whether a world's start region has an exit and a pad is
/// one. The fill then moves keys around, re-testing that safety on every swap.
/// Anything still unsafe afterwards gets a hub pad out of whatever budget is
/// left — the last claim on the 16 ids, because by then it is the only one
/// that can make a seed unplayable.
///
/// `sealable_locks` is the writer's invariant, passed in rather than assumed:
/// how many locks must still be ones the player can decline to open, so a
/// secret-exit fortress level has somewhere safe to land. See
/// [`fill::keep_n_sealable`].
///
/// **That whole sequence is one deal, and a short deal is dealt again.** The
/// pads and the key assignment together move a run's length far more than the
/// eight worlds under them do, so a maze that prices below [`CONTENT_FLOOR`]
/// is redrawn rather than shipped. The loop keeps the longest deal it saw, so
/// it cannot fail; see [`CONTENT_FLOOR`] for the measurement that chose 14.
pub(crate) fn generate<R: Rng>(
    result: &BuildResult,
    spine: &[usize],
    wands_required: u8,
    knobs: &graph::Knobs,
    sealable_locks: usize,
    rng: &mut R,
) -> (GlobalState, GenReport) {
    // **One deal**: the pads, the key assignment, and the rescue pass.
    // Everything that consumes RNG lives in here, which is what makes a redeal
    // a different maze; nothing after the loop draws at all.
    let deal = |rng: &mut R| {
        let mut state = GlobalState::from_build(result, spine, wands_required);

        let pads = graph::plan_pads(&state, knobs, rng);
        state.add_pads(pads.iter().map(|p| p.edge).collect());

        let fill = fill::assign_keys(&mut state, spine, knobs, rng);

        // A world whose start region has no walk-out still gets offered a pad,
        // because an extra edge can only help — but it is **no longer a reason
        // to throw the assignment away**, and that change is the whole reason
        // the constructive fill is usable.
        //
        // The rule existed because game over, airship arrival and whistle
        // travel all deposit the player on a start tile. Two things retire it:
        //
        // * **The fill cannot strand anyone.** Every gate takes its key from a
        //   fortress already reachable from the global start at the moment it
        //   is placed, so every gate it writes is openable — which is strictly
        //   stronger than the rule. `start_region_escapable` counts only a
        //   world's OWN fortresses as openers, so it rejects the mode's own
        //   formula: pad out, beat a fortress there, come back.
        // * **The whistle is the escape hatch anyway.** It is never consumed,
        //   survives a game over (nothing on that path clears
        //   `Inventory_Items`), and the cycler always has the spine's first
        //   world to return to.
        //
        // Enforced, it cost 2.1 crossings a seed and sent a third of seeds to
        // the fallback; retired, cross-world locks go 51% -> 76% and the World
        // 8 bridge 37% -> 73%, with nothing falling back.
        let mut unsafe_worlds: Vec<usize> =
            (0..state.worlds.len()).filter(|&wi| !state.start_region_escapable(wi)).collect();
        if !unsafe_worlds.is_empty() {
            let rescue = graph::rescue_pads(&state, &unsafe_worlds, knobs, rng);
            state.add_pads(rescue.iter().map(|p| p.edge).collect());
            unsafe_worlds.retain(|&wi| !state.start_region_escapable(wi));
        }

        Deal { content: 0, state, fill, pads, unsafe_worlds }
    };

    // **The content floor.** Deal until the maze is long enough, keeping the
    // longest deal seen. See [`CONTENT_FLOOR`] for why a redeal is the right
    // lever (88% of the variance in run length is in this layer, not in the
    // eight worlds) and [`MAX_DEALS`] for why the loop is bounded.
    //
    // Keeping the best rather than the last is what makes this **unable to
    // fail**, the same discipline the fill uses: the worst case is the longest
    // maze of the eight dealt, never a short one shipped because the budget ran
    // out.
    //
    // Deliberately measured on `content` and not on a proxy. A pad landing
    // beside the castle, a lightly-gated goal and a cheap route are three
    // different ways to produce a two-level run, and every proxy for them that
    // was tried moved the *median* without moving the *minimum*.
    let mut best: Option<Deal> = None;
    let mut deals = 0usize;
    while deals < MAX_DEALS {
        deals += 1;
        let mut dealt = deal(rng);
        // A deal that never reaches the castle scores zero, so it can only win
        // if every deal did — and the solvability guard below then catches it.
        let cost = metrics::completion_cost(&dealt.state);
        dealt.content = if cost.reached { cost.content } else { 0 };
        let floor_met = dealt.content >= CONTENT_FLOOR;
        if best.as_ref().is_none_or(|seen| dealt.content > seen.content) {
            best = Some(dealt);
        }
        if floor_met {
            break;
        }
    }
    let Deal { content, mut state, fill, pads, mut unsafe_worlds } =
        best.expect("MAX_DEALS is non-zero, so at least one deal was kept");

    let mut spheres = state.spheres();

    // Defence in depth, and the one guard this mode cannot do without.
    //
    // Every accept test in the fill already checks solvability and start-region
    // safety, so this should be unreachable — measured 0 firings in 8000
    // generations (1000 seeds x K=0..7). But "should be unreachable" is not
    // something a player can cash, and an unwinnable seed is the single failure
    // a maze cannot recover from — there is no way to notice it except by
    // playing to the wall. If it ever fires, fall back to the shape that is
    // solvable BY CONSTRUCTION: the per-world builder's own local locks, no
    // pads, the spine alone. `the_spine_alone_completes_the_maze` is what makes
    // that a guarantee rather than a hope.
    //
    // It outranks the content floor: a long maze nobody can finish is worse
    // than a short one, so this runs after the loop and overrides whatever it
    // kept.
    if !spheres.solvable {
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

    // Last, and after the fallback above, so it judges the assignment that
    // actually ships. Consumes no RNG.
    //
    // **Outside the deal loop on purpose.** It is a third of the generator's
    // cost (one global fixpoint per lock, ~3.4 ms of 9.7) and a deal that is
    // about to be thrown away does not need repairing. Running it here instead
    // of per-deal is what makes a redeal ~7 ms rather than ~11.
    let sealable = fill::keep_n_sealable(&mut state, sealable_locks);
    // Opening a gate changes reachability, so the log has to describe the map
    // that ships rather than the one measured before the repair.
    if sealable.opened > 0 {
        spheres = state.spheres();
    }

    (state, GenReport { spheres, fill, pads, unsafe_worlds, sealable, deals, content })
}

/// Fold the maze's decisions back into the build, for the writer to write.
///
/// **This is why the maze runs before the writer.** Everything here used to be
/// a ROM patch laid over grids the writer had already committed, which is what
/// made the ordering in `randomize_inner` load-bearing and forced later steps
/// to read the cartridge back to discover what had happened. As model edits
/// they are picked up by `overworld_writer::grid`, which starts from
/// `built.grid.clone()` and stamps on top, so one write pass emits the finished
/// map and nothing has to re-derive it afterwards.
///
/// Three things travel:
///
/// * **Pad tiles.** The cell keeps its pointer-table entry; that entry becomes
///   unreachable, which is why [`roles::pad_sites`] only ever offers Hammer Bro
///   filler slots and bare blanks.
/// * **Uninstalled locks** ([`MazeLock::fort`] `== None`) are dropped, so the
///   writer never stamps a `gap_tile` there and the path tile underneath
///   stands. Previously the writer stamped the gate from the builder's list and
///   the maze had to paint over it — a gate with no key in the window between.
/// * **`secret_exit_safe`**, restamped with the maze-grade verdict. The
///   builder's is a per-world question and over-promises (19% of locks at K=3,
///   43% at K=7), because sealing a lock can strand an airship the wand count
///   needs.
///
/// Consumes no RNG.
pub(crate) fn stamp_into(build: &mut BuildResult, state: &GlobalState) {
    // The wand gate's masonry. `wand_gate::apply` installs the opener and its
    // hooks; the cell it stands on is a map tile like any other, and `W8`'s
    // builder reserved it (`WorldState::wand_gate_reserved`) so nothing else
    // claimed it.
    if state.wands_required > 0 {
        let (row, col) = rom_data::W8_WAND_GATE_POS;
        build.worlds[rom_data::W8_IDX].grid.set(row, col, rom_data::WAND_GATE_TILE);
    }

    for (world, built) in build.worlds.iter_mut().enumerate() {
        for ((pad_world, (row, col)), _) in state.pad_edges() {
            if pad_world == world {
                built.grid.set(row, col, rom_data::TILE_TELEPAD);
            }
        }

        // Uninstalled locks are dropped; the rest take the maze's fortress,
        // which may be in another world. This is the whole of what used to be
        // `maze::writer::lock_keys` — the pairing travels in the model now, so
        // the writer emits the rows for both modes.
        let installed: HashMap<Pos, FortRef> = state
            .locks
            .iter()
            .filter(|l| l.world == world)
            .filter_map(|l| l.fort.map(|f| (l.pos, f)))
            .collect();
        built.locks.retain(|l| installed.contains_key(&l.pos));
        for lock in &mut built.locks {
            lock.fort = installed[&lock.pos];
            // **Restamp the verdict, do not inherit it.** `secret_exit_safe` as
            // the builder left it asks a per-world question, and the maze asks a
            // bigger one: with this lock sealed forever, is the castle still
            // reachable *and* are K airship docks still reachable, across all
            // eight worlds. The per-world flag over-promises on 19% of locks at
            // K=3 and 43% at K=7, so shipping it unchanged hands the writer
            // slots that would strand the player who takes 1-F's secret exit.
            let li = state
                .locks
                .iter()
                .position(|l| l.world == world && l.pos == lock.pos)
                .expect("the lock came from this list");
            lock.secret_exit_safe = state.winnable_with_lock_sealed(li);
        }

        // Say on each fortress where the lock it opens is. Own world wins,
        // then World 8, then elsewhere — so a World 8 fortress opening a World
        // 8 lock reads OwnWorld, not World8. Whether the player is told, and
        // which tile says it, is the writer's call.
        for slot in built.slots.iter_mut().filter(|s| s.kind == SlotKind::Fortress) {
            let Some(lock) = state
                .locks
                .iter()
                .find(|l| l.fort == Some(FortRef { world, section: slot.section }))
            else {
                continue;
            };
            slot.lock_hint = if lock.world == world {
                LockHint::OwnWorld
            } else if lock.world == rom_data::W8_IDX {
                LockHint::World8
            } else {
                LockHint::Elsewhere
            };
        }
    }
}
