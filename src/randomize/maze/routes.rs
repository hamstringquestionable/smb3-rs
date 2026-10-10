//! **The route census**: how long a maze is to play, and how much of its route
//! is a choice. Test-only, like every census here.
//!
//! A route is a minimal set of levels, forts and airships that finishes the
//! game. [`RouteModel`] answers whether a set wins; [`census`] runs random and
//! guided players over it and tries to prove the shortest by bounded search;
//! [`stage_tiers`] grades each stage on the best route by how avoidable it is.
//!
//! Two callers: `maze_route_census` in `tests.rs` (one CSV line, for batch
//! studies) and [`routes_report`] here (a readable report for one share link):
//!
//! ```text
//! ROUTE_LINK='https://hamstringquestionable.github.io/smb3-rs/beta/?seed=…&flags=…' \
//!   cargo test --release --lib routes_report -- --ignored --nocapture
//! ```
//!
//! It measures the code checked out. To measure what a site runs, check out
//! its branch first (`/beta/` is `beta/next`, the live site is `main`).

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use rand::seq::SliceRandom;

use super::GlobalState;
use super::walk::{MazePos, walk_maze};
use crate::randomize::overworld::build::SlotKind;
use crate::randomize::rom_data::{self, Grid};

/// One thing a route can clear: a level, a fortress, an airship, or (with
/// hammers) a rock. A route is the SET of these it clears, so two orders of
/// the same set are one route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RouteItem {
    Level(MazePos),
    Fort(super::FortRef, MazePos),
    Airship(MazePos),
    Rock(MazePos),
}

impl RouteItem {
    fn pos(self) -> MazePos {
        match self {
            RouteItem::Level(p) | RouteItem::Fort(_, p) | RouteItem::Airship(p) => p,
            RouteItem::Rock(p) => p,
        }
    }

    /// Levels, forts and airships are played; a rock is only broken.
    fn played(self) -> bool {
        !matches!(self, RouteItem::Rock(_))
    }
}

/// Breakable rocks and the path a hammer leaves (`route_choice`'s table).
const ROUTE_ROCKS: [(u8, u8); 2] = [(0x51, 0x45), (0x52, 0x46)];

/// One maze, as the route census sees it: what can be cleared, whether a set of
/// cleared things finishes the game, and an optimistic estimate of what is
/// still to play.
///
/// **A set wins** when, with every level, fort and airship NOT in it walled
/// off, the walk from the start reaches the castle holding K wand airships. A
/// fort opens its lock, an airship grants its wand and a rock breaks only once
/// the walk actually reaches it — being in the set is not enough, because
/// shrinking a set can cut one off. That is the generator's own fixpoint rule,
/// and [`RouteModel::confirm`] cross-checks a route against it.
///
/// Winning is monotone (more cleared is never less reachable), so a set from
/// which no single item can be dropped is a **minimal** route.
pub(crate) struct RouteModel<'a> {
    state: &'a GlobalState,
    k: usize,
    hammers: usize,
    items: Vec<RouteItem>,
    wand_tiles: HashSet<MazePos>,
    links: Vec<(MazePos, MazePos)>,
    /// Slots stamped, nothing walled: what the path tiles between cells are.
    open_grids: Vec<Grid>,
    /// The floor's own estimate (`metrics::ShortestBound`), so the census and
    /// the generator can never disagree about it.
    bound: super::metrics::ShortestBound,
    /// Flat cell index → the played item standing there.
    played_cell: std::collections::HashMap<usize, usize>,
}

impl<'a> RouteModel<'a> {
    pub(crate) fn new(state: &'a GlobalState, k: u8, hammers: usize) -> Self {
        let mut items: Vec<RouteItem> = Vec::new();
        for w in &state.worlds {
            let wi = w.world_idx;
            for s in &w.slots {
                match s.kind {
                    SlotKind::Level => items.push(RouteItem::Level((wi, s.pos))),
                    SlotKind::Fortress => items.push(RouteItem::Fort(
                        super::FortRef { world: wi, section: s.section },
                        (wi, s.pos),
                    )),
                    _ => {}
                }
            }
            if let Some(t) = w.target
                && (wi, t) != state.goal
            {
                items.push(RouteItem::Airship((wi, t)));
            }
            if hammers > 0 {
                for r in 0..w.grid.rows() {
                    for c in 0..w.grid.cols {
                        if ROUTE_ROCKS.iter().any(|&(rock, _)| rock == w.grid.get(r, c)) {
                            items.push(RouteItem::Rock((wi, (r, c))));
                        }
                    }
                }
            }
        }
        assert!(items.len() <= 128, "{} items do not fit a u128 set", items.len());
        let open_grids = state.base_grids(&HashSet::new());
        let links = state.links();

        let bound = super::metrics::ShortestBound::new(state);
        let played_cell = items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.played())
            .map(|(i, item)| (bound.cell(item.pos()), i))
            .collect();

        RouteModel {
            state,
            k: usize::from(k),
            hammers,
            wand_tiles: state.wand_tiles().into_iter().collect(),
            items,
            links,
            open_grids,
            bound,
            played_cell,
        }
    }

    fn n(&self) -> usize {
        self.items.len()
    }

    fn has(set: u128, i: usize) -> bool {
        set & (1u128 << i) != 0
    }

    /// Levels + forts + airships: what a route makes the player play.
    fn size(&self, set: u128) -> usize {
        (0..self.n()).filter(|&i| Self::has(set, i) && self.items[i].played()).count()
    }

    /// Walk with exactly `set` cleared: the reach, the shut lock cells, and
    /// whether it finishes the game.
    fn walk(&self, set: u128) -> (super::walk::MazeReach, Vec<HashSet<rom_data::Pos>>, bool) {
        let walled: HashSet<MazePos> = (0..self.n())
            .filter(|&i| !Self::has(set, i) && self.items[i].played())
            .map(|i| self.items[i].pos())
            .collect();
        let base = self.state.base_grids(&walled);
        let mut open = HashSet::new();
        let mut broken: HashSet<usize> = HashSet::new();
        loop {
            let mut grids = base.clone();
            for &i in &broken {
                let (w, (r, c)) = self.items[i].pos();
                let rock = grids[w].get(r, c);
                let path = ROUTE_ROCKS.iter().find(|&&(t, _)| t == rock).map(|&(_, p)| p);
                grids[w].set(r, c, path.expect("a rock item stands on a rock"));
            }
            let shut = self.state.shut_locks(&open);
            let view = self.state.view(&grids, &shut, &HashSet::new());
            let reach = walk_maze(&view, &self.links, self.state.start);
            let mut grew = false;
            for i in (0..self.n()).filter(|&i| Self::has(set, i)) {
                match self.items[i] {
                    RouteItem::Fort(f, p) if !open.contains(&f) && reach.contains(p) => {
                        open.insert(f);
                        grew = true;
                    }
                    RouteItem::Rock(_) if !broken.contains(&i) && self.beside(i, &reach) => {
                        broken.insert(i);
                        grew = true;
                    }
                    _ => {}
                }
            }
            if grew {
                continue;
            }
            let wands = (0..self.n())
                .filter(|&i| Self::has(set, i))
                .filter(|&i| {
                    matches!(self.items[i], RouteItem::Airship(p)
                        if self.wand_tiles.contains(&p) && reach.contains(p))
                })
                .count();
            let wins = reach.contains(self.state.goal) && wands >= self.k;
            return (reach, shut, wins);
        }
    }

    fn wins(&self, set: u128) -> bool {
        self.walk(set).2
    }

    /// A rock with a reached cell one or two steps from it.
    fn beside(&self, i: usize, reach: &super::walk::MazeReach) -> bool {
        let (w, (r, c)) = self.items[i].pos();
        let g = &self.open_grids[w];
        [(0i32, 1i32), (0, -1), (1, 0), (-1, 0)].iter().any(|&(dr, dc)| {
            (1..=2).any(|step| {
                let (nr, nc) = (r as i32 + step * dr, c as i32 + step * dc);
                (0..g.rows() as i32).contains(&nr)
                    && (0..g.cols as i32).contains(&nc)
                    && reach.contains((w, (nr as usize, nc as usize)))
            })
        })
    }

    /// Can the player step onto item `i` from what `reach` holds? The walker's
    /// own move: a 2-tile step from a reached cell across an open path tile,
    /// never out of an airship or castle tile.
    fn enterable(
        &self,
        i: usize,
        reach: &super::walk::MazeReach,
        shut: &[HashSet<rom_data::Pos>],
    ) -> bool {
        if matches!(self.items[i], RouteItem::Rock(_)) {
            return self.beside(i, reach);
        }
        let (w, (r, c)) = self.items[i].pos();
        let g = &self.open_grids[w];
        [(0i32, 1i32, true), (0, -1, true), (1, 0, false), (-1, 0, false)].iter().any(
            |&(dr, dc, horz)| {
                let at = |step: i32| {
                    let (nr, nc) = (r as i32 + step * dr, c as i32 + step * dc);
                    ((0..g.rows() as i32).contains(&nr) && (0..g.cols as i32).contains(&nc))
                        .then_some((nr as usize, nc as usize))
                };
                let (Some(path), Some(from)) = (at(1), at(2)) else { return false };
                let valid = if horz { rom_data::VALID_HORZ } else { rom_data::VALID_VERT };
                let from_tile = g.get(from.0, from.1);
                let sink = (from_tile == rom_data::TILE_AIRSHIP
                    || from_tile == rom_data::TILE_BOWSER)
                    && (w, from) != self.state.start;
                reach.contains((w, from))
                    && !sink
                    && valid.contains(&g.get(path.0, path.1))
                    && !shut[w].contains(&path)
            },
        )
    }

    /// The items a player holding `set` could clear next.
    fn next_items(&self, set: u128) -> Vec<usize> {
        let (reach, shut, _) = self.walk(set);
        let rocks_used =
            (0..self.n()).filter(|&i| Self::has(set, i) && !self.items[i].played()).count();
        (0..self.n())
            .filter(|&i| !Self::has(set, i))
            .filter(|&i| self.items[i].played() || rocks_used < self.hammers)
            .filter(|&i| self.enterable(i, &reach, &shut))
            .collect()
    }

    /// The optimistic estimate of what is still to play from `set` — the
    /// content floor's own measure (`metrics::ShortestBound`), asked with the
    /// set's played items as already cleared.
    pub(crate) fn estimate(&self, set: u128) -> Option<usize> {
        self.bound.estimate(|cell| self.played_cell.get(&cell).is_some_and(|&i| Self::has(set, i)))
    }

    /// One play-through, shrunk to a minimal route. `guided` prefers the next
    /// item that leaves the least still to play; otherwise every enterable item
    /// is equally likely. Items in `banned` are never played. `None` if the
    /// player got stuck.
    fn play<R: rand::Rng>(&self, rng: &mut R, guided: bool, banned: u128) -> Option<u128> {
        use rand::seq::IndexedRandom;
        let mut set = 0u128;
        while !self.wins(set) {
            let mut next = self.next_items(set);
            next.retain(|&i| !Self::has(banned, i));
            let pick = if guided && rng.random_bool(0.8) {
                let scored: Vec<(usize, usize)> = next
                    .iter()
                    .filter_map(|&i| {
                        let grown = set | (1u128 << i);
                        self.estimate(grown).map(|h| (i, self.size(grown) + h))
                    })
                    .collect();
                let best = scored.iter().map(|&(_, s)| s).min();
                let top: Vec<usize> =
                    scored.iter().filter(|&&(_, s)| Some(s) == best).map(|&(i, _)| i).collect();
                top.choose(rng).copied()
            } else {
                next.choose(rng).copied()
            };
            set |= 1u128 << pick?;
        }
        Some(self.shrink(set, rng))
    }

    /// Drop items in random order while the set still wins, until none can go.
    fn shrink<R: rand::Rng>(&self, mut set: u128, rng: &mut R) -> u128 {
        loop {
            let mut order: Vec<usize> = (0..self.n()).filter(|&i| Self::has(set, i)).collect();
            order.shuffle(rng);
            let before = set;
            for i in order {
                let without = set & !(1u128 << i);
                if self.wins(without) {
                    set = without;
                }
            }
            if set == before {
                return set;
            }
        }
    }

    /// **The smallest winning set of size at most `budget`, exactly**, or
    /// `None` if there is none. Breadth-first by set size, so the first win
    /// found is the smallest; a set the estimate puts over budget is cut.
    /// The second value is false when `cap` sets or `limit` time ran out first,
    /// so the answer is unproven.
    fn shortest_within(
        &self,
        budget: usize,
        cap: usize,
        limit: std::time::Duration,
    ) -> (Option<u128>, bool) {
        let clock = std::time::Instant::now();
        let mut seen: HashSet<u128> = HashSet::from([0]);
        let mut layer: Vec<u128> = vec![0];
        let mut explored = 0usize;
        while !layer.is_empty() {
            let mut next: Vec<u128> = Vec::new();
            for &set in &layer {
                explored += 1;
                if explored > cap || clock.elapsed() > limit {
                    return (None, false);
                }
                if self.wins(set) {
                    return (Some(set), true);
                }
                for i in self.next_items(set) {
                    let grown = set | (1u128 << i);
                    if !seen.insert(grown) {
                        continue;
                    }
                    if self.estimate(grown).is_none_or(|h| self.size(grown) + h > budget) {
                        continue;
                    }
                    next.push(grown);
                }
            }
            layer = next;
        }
        (None, true)
    }

    /// The generator's own fixpoint agrees this set finishes the game.
    fn confirm(&self, set: u128) -> bool {
        let walled: HashSet<MazePos> = (0..self.n())
            .filter(|&i| !Self::has(set, i) && self.items[i].played())
            .map(|i| self.items[i].pos())
            .collect();
        self.state.spheres_with_blocked(&walled).goal_sphere.is_some()
    }
    /// The item indices in `set`, ascending.
    fn members(&self, set: u128) -> Vec<usize> {
        (0..self.n()).filter(|&i| Self::has(set, i)).collect()
    }

    /// `set` in an order the walk allows: each item is enterable once the ones
    /// before it are cleared, staying in the current world while it can.
    fn play_order(&self, set: u128) -> Vec<usize> {
        let mut have = 0u128;
        let mut out = Vec::new();
        let mut world = self.state.start.0;
        while have != set {
            let next: Vec<usize> =
                self.next_items(have).into_iter().filter(|&i| Self::has(set, i)).collect();
            let Some(&first) = next.first() else { break };
            let pick =
                next.iter().copied().find(|&i| self.items[i].pos().0 == world).unwrap_or(first);
            world = self.items[pick].pos().0;
            have |= 1u128 << pick;
            out.push(pick);
        }
        out
    }
}

/// How hard [`census`] looks.
pub(crate) struct CensusParams {
    /// Uniformly random players, each shrunk to a minimal route.
    pub(crate) samples: usize,
    /// Players who prefer the move that leaves least to play.
    pub(crate) guided: usize,
    /// Starting hammers: each breaks one rock.
    pub(crate) hammers: usize,
    /// The exact search gives up after this many sets...
    pub(crate) exact_cap: usize,
    /// ...or this long, and the shortest is then unproven.
    pub(crate) exact_limit: Duration,
    /// Worker threads for the players; 0 means every core this process may use.
    /// The answer does not depend on it — only how long it takes.
    pub(crate) threads: usize,
}

/// One stage a route can play, as a report shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StageKind {
    Level,
    Fort,
    Airship,
    Rock,
}

/// What [`census`] found on one maze.
pub(crate) struct Census {
    /// Every stage a route can contain, by item index.
    pub(crate) stages: Vec<(StageKind, MazePos)>,
    /// The number the game shows for each world (index = world), 1-based
    /// along the airship chain from the start; `None` off the chain.
    shown: [Option<usize>; 8],
    goal_world: usize,
    /// Wands the castle asks for.
    wands: u8,
    /// Every telepad, as `(from, to)` — each pair appears both ways.
    pads: Vec<(MazePos, MazePos)>,
    /// The floor's own lower bound on the shortest route.
    pub(crate) lower_bound: Option<usize>,
    /// The best route the guided players found.
    pub(crate) guided_best: Option<usize>,
    /// The shortest route found, in an order the walk allows.
    pub(crate) shortest: Option<Vec<usize>>,
    /// The exact search finished, so nothing shorter exists.
    pub(crate) proven: bool,
    /// Distinct routes the random players finished on, with how many found each.
    pub(crate) routes: Vec<(Vec<usize>, usize)>,
    /// Random players who got stuck.
    pub(crate) stuck: usize,
    samples: usize,
    guided: usize,
    pub(crate) secs: f64,
}

impl Census {
    /// Stages a route makes the player play (rocks are only broken).
    pub(crate) fn played(&self, route: &[usize]) -> usize {
        route.iter().filter(|&&i| self.stages[i].0 != StageKind::Rock).count()
    }
}

/// Run `players` players on worker threads and return what each finished on,
/// in player order.
///
/// **Each player's dice are its own**: ChaCha8 seeded with `seed`, on stream
/// `stream_base + i`. So the result is the same however many threads run it
/// and in whatever order they finish — the thread count is a speed knob only.
fn play_all(
    model: &RouteModel,
    seed: u64,
    stream_base: u64,
    players: usize,
    guided: bool,
    banned: u128,
    threads: usize,
) -> Vec<Option<u128>> {
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let next = AtomicUsize::new(0);
    let mut out = vec![None; players];
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads.clamp(1, players.max(1)))
            .map(|_| {
                scope.spawn(|| {
                    let mut mine = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= players {
                            return mine;
                        }
                        let mut rng = ChaCha8Rng::seed_from_u64(seed);
                        rng.set_stream(stream_base + i as u64);
                        mine.push((i, model.play(&mut rng, guided, banned)));
                    }
                })
            })
            .collect();
        for w in workers {
            for (i, r) in w.join().expect("a census worker panicked") {
                out[i] = r;
            }
        }
    });
    out
}

/// Run the census on `state`. Players draw their dice from `seed` (see
/// [`play_all`]), so a seed always yields the same report on any machine.
///
/// Panics if the generator's own fixpoint rejects the shortest route found —
/// that would make every figure here wrong.
pub(crate) fn census(state: &GlobalState, p: &CensusParams, seed: u64) -> Census {
    let clock = Instant::now();
    let k = state.wands_required;
    let model = RouteModel::new(state, k, p.hammers);
    let threads = thread_count(p.threads);

    // Smallest first, then the lowest set: a fixed winner whatever the order.
    let mut best: Option<u128> = None;
    let better = |r: u128, best: &mut Option<u128>| {
        if best.is_none_or(|b| (model.size(r), r) < (model.size(b), b)) {
            *best = Some(r);
        }
    };
    // Guided players on streams from 2^32 up, clear of the random ones.
    for r in play_all(&model, seed, 1 << 32, p.guided, true, 0, threads).into_iter().flatten() {
        better(r, &mut best);
    }
    let guided_best = best.map(|b| model.size(b));

    let mut counts: HashMap<u128, usize> = HashMap::new();
    let mut stuck = 0;
    for r in play_all(&model, seed, 0, p.samples, false, 0, threads) {
        match r {
            Some(r) => {
                *counts.entry(r).or_default() += 1;
                better(r, &mut best);
            }
            None => stuck += 1,
        }
    }

    let (shortest, proven) = match best {
        Some(b) => {
            let (smaller, done) =
                model.shortest_within(model.size(b) - 1, p.exact_cap, p.exact_limit);
            (Some(smaller.unwrap_or(b)), done)
        }
        None => (None, false),
    };
    if let Some(s) = shortest {
        assert!(model.confirm(s), "the generator's fixpoint rejects a route the census found");
    }

    let mut routes: Vec<(Vec<usize>, usize)> =
        counts.into_iter().map(|(set, n)| (model.members(set), n)).collect();
    routes.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    // The game numbers worlds along the airship chain: start = 1.
    let mut shown = [None; 8];
    let mut world = Some(state.start.0);
    let mut n = 1;
    while let Some(w) = world.filter(|&w| shown[w].is_none()) {
        shown[w] = Some(n);
        n += 1;
        world = state.edges.iter().find_map(|e| match *e {
            super::MazeEdge::Airship { from_world, to_world } if from_world == w => Some(to_world),
            _ => None,
        });
    }

    let stages = model
        .items
        .iter()
        .map(|&item| {
            let kind = match item {
                RouteItem::Level(_) => StageKind::Level,
                RouteItem::Fort(..) => StageKind::Fort,
                RouteItem::Airship(_) => StageKind::Airship,
                RouteItem::Rock(_) => StageKind::Rock,
            };
            (kind, item.pos())
        })
        .collect();

    Census {
        stages,
        shown,
        goal_world: state.goal.0,
        wands: k,
        pads: state.pad_edges(),
        lower_bound: model.estimate(0),
        guided_best,
        shortest: shortest.map(|s| model.play_order(s)),
        proven,
        routes,
        stuck,
        samples: p.samples,
        guided: p.guided,
        secs: clock.elapsed().as_secs_f64(),
    }
}

/// `0` means every core this process may use.
fn thread_count(threads: usize) -> usize {
    match threads {
        0 => std::thread::available_parallelism().map_or(1, |n| n.get()),
        n => n,
    }
}

/// How avoidable one stage on the best route is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tier {
    /// No winning route skips it. Exact.
    Required,
    /// Skipping it costs more than [`BAND_PERCENT`] of the route's length, or
    /// no player found a way round it at all.
    Effective,
    /// Skipping it costs something, but no more than [`BAND_PERCENT`].
    Optional,
    /// An equally short route skips it.
    Free,
}

/// Where "optional" ends and "effectively required" begins, as a share of the
/// route's length — a share, not a count, because two extra stages matter far
/// more on a 13-stage seed than on a 30-stage one.
const BAND_PERCENT: usize = 10;

/// The best route, each stage on it tiered by how avoidable it is.
struct Tiers {
    /// The best route, in an order the walk allows. Longer than nothing the
    /// census found: classifying can turn up a shorter route, which replaces it.
    route: Vec<usize>,
    /// Per stage of `route`, same order: its tier, and how many more stages
    /// the best route found without it plays (`None` when required, or when
    /// no player found a way round it).
    tiers: Vec<(Tier, Option<usize>)>,
    /// Classifying found a shorter route than the census did.
    improved: bool,
}

/// Tier every stage on `best`.
///
/// **Required is exact**: the walk with everything else cleared cannot win
/// without the stage. The other tiers compare the best route found *without*
/// the stage — 20 guided and 100 random players with it walled off — against
/// `best`, so they are estimates and share their accuracy with the census's
/// own best route. A player who beats `best` replaces it and the pass starts
/// over, so the baseline only ever improves.
fn stage_tiers(state: &GlobalState, best: &[usize], seed: u64, threads: usize) -> Tiers {
    let model = RouteModel::new(state, state.wands_required, 0);
    let threads = thread_count(threads);
    let all: u128 = (0..model.n()).fold(0, |s, i| s | 1u128 << i);
    let mut best: u128 = best.iter().fold(0, |s, &i| s | 1u128 << i);
    let mut improved = false;
    'restart: loop {
        let len = model.size(best);
        let route = model.play_order(best);
        let mut tiers = Vec::with_capacity(route.len());
        for &i in &route {
            if !model.wins(all & !(1u128 << i)) {
                tiers.push((Tier::Required, None));
                continue;
            }
            // Streams per stage, clear of the census's own (0.. and 2^32..).
            let base = (2 + i as u64) << 32;
            let found = play_all(&model, seed, base, 20, true, 1u128 << i, threads)
                .into_iter()
                .chain(play_all(&model, seed, base + (1 << 31), 100, false, 1u128 << i, threads))
                .flatten()
                .min_by_key(|&r| (model.size(r), r));
            match found {
                Some(r) if model.size(r) < len => {
                    best = r;
                    improved = true;
                    continue 'restart;
                }
                Some(r) => {
                    let extra = model.size(r) - len;
                    let tier = if extra == 0 {
                        Tier::Free
                    } else if extra * 100 <= len * BAND_PERCENT {
                        Tier::Optional
                    } else {
                        Tier::Effective
                    };
                    tiers.push((tier, Some(extra)));
                }
                None => tiers.push((Tier::Effective, None)),
            }
        }
        return Tiers { route, tiers, improved };
    }
}

// ---------------------------------------------------------------------------
// The report
// ---------------------------------------------------------------------------

/// The original map each world index draws, for telling renumbered worlds apart.
const MAP_NAMES: [&str; 8] = [
    "Grass Land",
    "Desert Land",
    "Water Land",
    "Giant Land",
    "Sky Land",
    "Ice Land",
    "Pipe Land",
    "Dark Land",
];

/// `seed` and `flags` out of a share link (`…/smb3-rs/beta/?seed=…&flags=…`).
fn parse_link(url: &str) -> Option<(u64, String)> {
    let (_, query) = url.split_once('?')?;
    let param = |name: &str| {
        query.split(['&', '#']).find_map(|kv| kv.strip_prefix(name)?.strip_prefix('='))
    };
    Some((param("seed")?.parse().ok()?, param("flags")?.to_string()))
}

/// "World 3 (Ice Land map)", or the map alone for a world off the chain.
fn world_name(c: &Census, w: usize) -> String {
    match c.shown[w] {
        Some(n) => format!("World {n} ({} map)", MAP_NAMES[w]),
        None => format!("{} map (off the airship chain)", MAP_NAMES[w]),
    }
}

fn kind_name(k: StageKind) -> &'static str {
    match k {
        StageKind::Level => "level",
        StageKind::Fort => "fortress",
        StageKind::Airship => "airship",
        StageKind::Rock => "rock (hammer)",
    }
}

fn tier_letter(t: Tier) -> &'static str {
    match t {
        Tier::Required => "R",
        Tier::Effective => "E",
        Tier::Optional => "O",
        Tier::Free => "F",
    }
}

/// A world's sort key: its shown number, off-chain worlds last.
fn order_key(c: &Census, w: usize) -> usize {
    c.shown[w].unwrap_or(100 + w)
}

fn report(c: &Census, t: &Tiers) -> String {
    let mut out = Vec::new();
    out.push(format!("Wands required: {}", c.wands));
    out.push(format!(
        "Players: {} random ({} stuck, {} distinct routes), {} guided · {:.0} s",
        c.samples,
        c.stuck,
        c.routes.len(),
        c.guided,
        c.secs
    ));

    // --- Length ------------------------------------------------------------
    let len = c.played(&t.route);
    let air = t.route.iter().filter(|&&i| c.stages[i].0 == StageKind::Airship).count();
    let how = match (c.proven, t.improved) {
        (_, true) => "best found, while grading stages",
        (true, false) => "proven shortest",
        (false, false) => "best found; not proven",
    };
    out.push(String::new());
    out.push(format!("Seed length: {len} stages ({how})"));
    out.push(format!(
        "  {} levels/forts + {} airships · lower bound {}",
        len - air,
        air,
        c.lower_bound.map_or("-".into(), |b| b.to_string())
    ));

    // --- Tiers -------------------------------------------------------------
    let pct = |n: usize| 100.0 * n as f64 / len.max(1) as f64;
    let beyond = len * BAND_PERCENT / 100 + 1;
    out.push(String::new());
    out.push("How avoidable each stage on that route is:".into());
    for (tier, name, meaning) in [
        (Tier::Required, "required", "no winning route skips it".to_string()),
        (
            Tier::Effective,
            "effectively required",
            format!("skipping it costs more than +{BAND_PERCENT}% ({beyond}+ stages)"),
        ),
        (Tier::Optional, "optional", format!("skipping it costs up to +{BAND_PERCENT}%")),
        (Tier::Free, "free", "an equally short route skips it".to_string()),
    ] {
        let n = t.tiers.iter().filter(|(x, _)| *x == tier).count();
        out.push(format!("  {name:<21}{n:>3}  {:>3.0}%   {meaning}", pct(n)));
    }

    // --- The route, marked -------------------------------------------------
    out.push(String::new());
    out.push("The route, in one order the game allows:".into());
    let mut i = 0;
    while i < t.route.len() {
        let w = c.stages[t.route[i]].1.0;
        let mut marks = Vec::new();
        while i < t.route.len() && c.stages[t.route[i]].1.0 == w {
            let cost = match t.tiers[i] {
                (Tier::Effective | Tier::Optional, Some(x)) => format!("+{x}"),
                _ => String::new(),
            };
            let kind = kind_name(c.stages[t.route[i]].0);
            marks.push(format!("{kind} {}{cost}", tier_letter(t.tiers[i].0)));
            i += 1;
        }
        out.push(format!("  {}: {}", world_name(c, w), marks.join(" · ")));
    }
    out.push(format!("  then the castle in {}", world_name(c, c.goal_world)));
    out.push(
        "  R required · E effectively required · O optional · F free · +n = stages it costs to skip"
            .into(),
    );

    // --- Telepads ----------------------------------------------------------
    out.push(String::new());
    out.push(format!("Telepads: {} ({} pairs)", c.pads.len(), c.pads.len() / 2));
    let mut pairs: Vec<(usize, usize)> = c
        .pads
        .iter()
        .filter(|(a, b)| a <= b)
        .map(|(a, b)| {
            let (x, y) = (order_key(c, a.0), order_key(c, b.0));
            (x.min(y), x.max(y))
        })
        .collect();
    pairs.sort_unstable();
    let name = |k: usize| if k < 100 { format!("World {k}") } else { format!("map {}", k - 99) };
    for (a, b) in pairs {
        out.push(format!("  {} <-> {}", name(a), name(b)));
    }
    let mut without: Vec<String> = (0..8)
        .filter(|&w| !c.pads.iter().any(|(a, _)| a.0 == w))
        .filter_map(|w| c.shown[w])
        .map(|n| n.to_string())
        .collect();
    without.sort_unstable();
    if !without.is_empty() {
        out.push(format!("  worlds with none: {}", without.join(", ")));
    }
    out.join("\n")
}

/// **The route report for one share link** — see the module docs for the
/// command. Prints the seed's length and how avoidable each stage on its
/// best route is.
#[test]
#[ignore]
fn routes_report() {
    let Ok(link) = std::env::var("ROUTE_LINK") else {
        eprintln!("set ROUTE_LINK to a share link");
        return;
    };
    let (seed, flags) = parse_link(&link).expect("ROUTE_LINK has seed= and flags=");
    let options = crate::Options::from_flag_key(&flags).expect("the flag key decodes");
    assert!(options.world_maze, "the flag key has World Maze off");
    let Ok(bytes) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") else {
        eprintln!("no ROM at roms/");
        return;
    };
    crate::randomize_rom_with_overworld_capture(&bytes, seed, &options, None)
        .expect("the pipeline runs");
    let state = super::LAST_MAZE
        .with(|slot| slot.borrow_mut().take())
        .expect("the pipeline generated a maze");
    let params = CensusParams {
        samples: 5000,
        guided: 20,
        hammers: 0,
        exact_cap: 200_000,
        exact_limit: Duration::from_secs(5),
        threads: 0,
    };
    // The same dice `maze_route_census` uses, so the two agree on a seed.
    let dice = seed ^ 0x2007_00E5;
    let c = census(&state, &params, dice);
    let best = c.shortest.as_ref().expect("a player finished");
    let t = stage_tiers(&state, best, dice, 0);
    println!("\nseed {seed}\nflags {flags}\n\n{}", report(&c, &t));
    if options.item_gates {
        println!("\nNote: item gates are on; the census does not model them.");
    }
}

#[test]
fn parses_share_links() {
    let url = "https://hamstringquestionable.github.io/smb3-rs/beta/?seed=7457942570878277&flags=SMB3R-3PTZZZVZ7YGABAKAMVX7PVZF80";
    assert_eq!(
        parse_link(url),
        Some((7457942570878277, "SMB3R-3PTZZZVZ7YGABAKAMVX7PVZF80".into()))
    );
    assert_eq!(parse_link("https://x/?flags=K&seed=1"), Some((1, "K".into())));
    assert_eq!(parse_link("https://x/beta/"), None);
    assert_eq!(parse_link("https://x/?seed=abc&flags=K"), None);
}
