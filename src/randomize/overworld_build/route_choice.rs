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
//! they play the same set of levels AND break the same rocks — a rock break
//! is a resource spend (a hammer), so a rock shortcut and the walk around it
//! are distinct choices, not duplicates. "Roughly equal" = within `slack`
//! points (default 3 = one level).
//!
//! Enumeration is COMPLETE and needs no bans or backward pass: the cleared-mask
//! already records exactly which levels a route played, so we run ONE Dijkstra,
//! keep going until the popped cost exceeds `best + slack`, and read off every
//! goal-state settled within budget — each is a route (mask → level-set). A
//! domination filter then drops any route that only plays an EXTRA level for no
//! benefit (a within-slack detour), so those don't masquerade as real choices.

use super::*;
use super::types::{BuiltWorld, SlotKind, stamp_slots};
use crate::randomize::map_walker::WalkResult;

use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap};

/// Weighted set-cost knobs, in plain points. Legible on purpose.
pub(crate) const COST_PIPE: u32 = 1;
pub(crate) const COST_LEVEL: u32 = 3;
pub(crate) const COST_FORT: u32 = 5;
pub(crate) const COST_ROCK: u32 = 8;
/// "Roughly equal" band, in points. 3 = one level of wobble.
pub(crate) const DEFAULT_SLACK: u32 = 3;
/// Floor on the cheapest route's cost — the world must charge at least this
/// much to finish, however composed (levels / forts / rocks / pipes).
/// Enforced best-effort by `enforce_c1_floor` (moving off-route levels onto
/// the cheap route) and guarded by the spare-pipe pass (a shortcut may not
/// price the world below the floor). This is the "not skippable" guarantee;
/// the goal gate is the "door and key" one — they overlap but neither
/// implies the other (probe: ~13% of gated worlds sat below 14).
pub(crate) const C1_FLOOR: u32 = 14;

/// Wider measuring band used while SHAPING a world (`shape_forts`): routes
/// above best stay visible up to this many points, so near-miss corridors the
/// fort budget could rescue are on the table. Rescue targets cluster around
/// `COST_FORT` above best; was 13 (two stacked forts + band), and a census
/// A/B at 9 matched it on route/streak/C1 while shrinking the wide-measure
/// state space — deep near-misses simply never got rescued. Final acceptance
/// still uses `DEFAULT_SLACK`.
pub(crate) const SHAPING_SLACK: u32 = 9;

/// Breakable overworld rocks and the path tile they open into
/// ($51→$45 horizontal, $52→$46 vertical). $53 is unbreakable (stays a wall).
const BREAKABLE_ROCKS: [(u8, u8); 2] = [(0x51, 0x45), (0x52, 0x46)];

/// One distinct route to the goal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ChoiceRoute {
    /// Weighted set-cost, in points.
    pub cost: u32,
    /// Distinct levels played — with `rocks`, the route's identity (dedup key).
    pub levels: BTreeSet<Pos>,
    /// Distinct rocks broken — part of the identity: breaking a rock spends a
    /// hammer, so a rock route never collapses into (or dominates) the
    /// rock-free way around.
    pub rocks: BTreeSet<Pos>,
    /// Distinct forts beaten (breakdown for display).
    // Reason: dead_code — read only by the cfg(test) renderers and census
    // tests; kept in the production struct so the breakdown is computed once,
    // next to the mask that defines it.
    #[allow(dead_code)]
    pub forts: u32,
    /// Node path start..goal, for rendering.
    pub path: Vec<Pos>,
}

/// Per-world choice summary.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
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

/// Routes within `DEFAULT_SLACK` of best — the count that defines "has
/// choice", measured inside a wider (`SHAPING_SLACK`) result.
pub(super) fn in_band_count(rc: &RouteChoice) -> usize {
    rc.routes
        .iter()
        .filter(|r| r.cost <= rc.best_cost + DEFAULT_SLACK)
        .count()
}

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

/// Deterministic multiplicative hasher (the FxHash fold) for the Dijkstra's
/// hot maps — the default SipHash dominated the build-time profile. Fixed
/// seed, so hashes are identical on native and WASM; none of the maps using
/// it are ever iterated, so distribution affects speed only, never behavior.
#[derive(Default)]
struct FastHasher(u64);

impl std::hash::Hasher for FastHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.add(b as u64);
        }
    }
    fn write_u128(&mut self, v: u128) {
        self.add(v as u64);
        self.add((v >> 64) as u64);
    }
    fn write_u64(&mut self, v: u64) {
        self.add(v);
    }
    fn write_u32(&mut self, v: u32) {
        self.add(v as u64);
    }
    fn write_usize(&mut self, v: usize) {
        self.add(v as u64);
    }
}

impl FastHasher {
    fn add(&mut self, v: u64) {
        self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(0x517cc1b727220a95);
    }
}

type FastMap<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<FastHasher>>;
type FastSet<K> = HashSet<K, std::hash::BuildHasherDefault<FastHasher>>;

/// Flat open-addressing map for the Dijkstra's `dist` table (packed state →
/// cost, plus the predecessor state for path reconstruction): linear probing,
/// power-of-2 capacity, insert/improve/lookup only — no deletes. Slots are
/// generation-stamped so `reset()` is O(1) (no memset), and the whole table
/// lives in a thread-local scratch reused across the hundreds of measure
/// calls per world. Grows by rehash at 7/8 load.
struct DistMap {
    keys: Vec<u128>,
    vals: Vec<u32>,
    prevs: Vec<u128>,
    stamp: Vec<u32>,
    cur: u32,
    shift: u32,
    len: usize,
}

/// "No predecessor" marker — `pack` never produces it (the pad bits above
/// bit 96 are always zero). The start state carries it; reconstruction stops
/// there.
const NO_PREV: u128 = u128::MAX;

impl DistMap {
    /// Zero-alloc placeholder left in the thread-local while a call borrows
    /// the real table; never used as a map.
    fn placeholder() -> Self {
        DistMap {
            keys: Vec::new(),
            vals: Vec::new(),
            prevs: Vec::new(),
            stamp: Vec::new(),
            cur: 0,
            shift: 63,
            len: 0,
        }
    }

    fn with_pow2(p: u32) -> Self {
        let cap = 1usize << p;
        DistMap {
            keys: vec![0; cap],
            vals: vec![0; cap],
            prevs: vec![0; cap],
            stamp: vec![0; cap],
            cur: 1,
            shift: 64 - p,
            len: 0,
        }
    }

    fn reset(&mut self) {
        self.cur = self.cur.wrapping_add(1);
        self.len = 0;
        if self.cur == 0 {
            // u32 generation wrapped (needs 4 billion resets): hard-clear.
            self.stamp.fill(0);
            self.cur = 1;
        }
    }

    #[inline]
    fn slot(&self, key: u128) -> usize {
        let h = ((key as u64) ^ ((key >> 64) as u64)).wrapping_mul(0x517cc1b727220a95);
        (h >> self.shift) as usize
    }

    #[inline]
    fn get(&self, key: u128) -> Option<u32> {
        let mask = self.keys.len() - 1;
        let mut i = self.slot(key);
        loop {
            if self.stamp[i] != self.cur {
                return None;
            }
            if self.keys[i] == key {
                return Some(self.vals[i]);
            }
            i = (i + 1) & mask;
        }
    }

    /// Store `val` (and the predecessor state) if the key is absent or `val`
    /// beats the stored cost; returns whether it was stored (= the relax
    /// improved the state).
    #[inline]
    fn improve(&mut self, key: u128, val: u32, prev: u128) -> bool {
        if self.len * 8 >= self.keys.len() * 7 {
            self.grow();
        }
        let mask = self.keys.len() - 1;
        let mut i = self.slot(key);
        loop {
            if self.stamp[i] != self.cur {
                self.stamp[i] = self.cur;
                self.keys[i] = key;
                self.vals[i] = val;
                self.prevs[i] = prev;
                self.len += 1;
                return true;
            }
            if self.keys[i] == key {
                if val < self.vals[i] {
                    self.vals[i] = val;
                    self.prevs[i] = prev;
                    return true;
                }
                return false;
            }
            i = (i + 1) & mask;
        }
    }

    /// Predecessor state recorded for `key` (`NO_PREV` at the start state).
    fn prev_of(&self, key: u128) -> Option<u128> {
        let mask = self.keys.len() - 1;
        let mut i = self.slot(key);
        loop {
            if self.stamp[i] != self.cur {
                return None;
            }
            if self.keys[i] == key {
                let p = self.prevs[i];
                return (p != NO_PREV).then_some(p);
            }
            i = (i + 1) & mask;
        }
    }

    fn grow(&mut self) {
        let mut bigger = Self::with_pow2(self.keys.len().trailing_zeros() + 1);
        for i in 0..self.keys.len() {
            if self.stamp[i] == self.cur {
                bigger.improve(self.keys[i], self.vals[i], self.prevs[i]);
            }
        }
        *self = bigger;
    }
}

/// Min-heap of (cost, packed state) — cost first so the heap orders by it;
/// the packed state tie-breaks in the old tuple order (see `PackedState`).
type CostHeap = BinaryHeap<Reverse<(u32, PackedState)>>;

thread_local! {
    /// Per-thread Dijkstra scratch (dist table + heap), reused across calls
    /// so the hot loop never allocates.
    static SCRATCH: std::cell::RefCell<(DistMap, CostHeap)> =
        std::cell::RefCell::new((DistMap::with_pow2(13), BinaryHeap::with_capacity(1 << 10)));
}

/// Packed Dijkstra state — (pos, cleared-mask, boat) in one `u128`, laid out
/// so numeric order equals the old tuple order (row, then col, then mask,
/// then boat with `None` < `Some`, `Some` ordered by (row, col)). The heap
/// tie-breaks on the state after the cost, so the layout guarantees the pop
/// sequence — and therefore every measured decision — is byte-identical to
/// the unpacked representation.
///
/// Bits (MSB→LSB): row 88..96, col 80..88, mask 16..80, boat code 0..16.
type PackedState = u128;

fn boat_code(boat: Option<Pos>) -> u128 {
    match boat {
        None => 0,
        Some((r, c)) => 1 + (r as u128) * 64 + c as u128,
    }
}

fn pack(pos: Pos, mask: u64, boat: u128) -> PackedState {
    debug_assert!(pos.0 < 256 && pos.1 < 64 && boat < 1 << 16);
    ((pos.0 as u128) << 88) | ((pos.1 as u128) << 80) | ((mask as u128) << 16) | boat
}

fn unpack_pos(key: PackedState) -> Pos {
    (((key >> 88) & 0xFF) as usize, ((key >> 80) & 0xFF) as usize)
}

fn unpack_mask(key: PackedState) -> u64 {
    (key >> 16) as u64
}

/// Enumerate every distinct near-optimal route to the goal and summarise the
/// choice they offer.
pub(crate) fn analyze_route_choice(built: &BuiltWorld, slack: u32) -> RouteChoice {
    analyze_route_choice_inner(built, slack, true)
}

/// Counts-only measure for trial flips (flip a slot, read two numbers, flip
/// back — the bulk of all measures): identical `RouteChoice` except every
/// `path` is empty. The `prev` map (an insert on nearly every relax) and the
/// path reconstruction are skipped. Trials read only `best_cost` /
/// `in_band_count` / `routes.len()` / `shaping_gap`, none of which touch
/// paths — any call that reads a `path` must use `analyze_route_choice`.
pub(crate) fn measure_counts(built: &BuiltWorld, slack: u32) -> RouteChoice {
    analyze_route_choice_inner(built, slack, false)
}

/// The walk-invariant base of a route measurement — the `walk_map` BFS graph
/// plus the grid-derived pieces (rock tiles, canoe edges, start/target) that do
/// NOT change when a candidate flips a slot's KIND (Level / Fortress /
/// HammerBro filler are all walk-nodes) or toggles a LOCK (a lock is a
/// `built.locks` overlay, never stamped into the grid `walk_map` sees). Across
/// the candidates of ONE selection step — the bulk of all measures — this base
/// is constant, so it is compiled ONCE (`WalkGraph::compile`) and every
/// candidate `measure`d against it, hoisting the BFS out of the trial loop.
///
/// INVALIDATED by anything that changes the walk graph: editing grid path/node
/// tiles, or `pipe_pairs` (spare-pipe trials feed the pair list to the BFS). A
/// slot flip onto a *background* tile is likewise NOT walk-invariant — an HB
/// filler on a background tile is unreachable, so converting it to a Fortress
/// adds a node — which is why `measure` VERIFIES walk-invariance in debug
/// builds (see the guard there) rather than assuming it. Reuse is a correctness
/// contract on the caller: only reuse a base across kind/lock candidates.
pub(super) struct WalkGraph {
    rows: usize,
    start: Pos,
    target: Pos,
    walk: WalkResult,
    /// Breakable-rock path tiles, in grid scan order — their mask-bit order
    /// must be seed-stable (heap tie-breaking includes the mask). Grid-derived,
    /// so stable across kind/lock candidates.
    rock_tiles: Vec<Pos>,
    canoe_edges: Vec<(Pos, Pos)>,
    canoe_pair_set: FastSet<(Pos, Pos)>,
    initial_boat: u128,
}

/// Build the working grid a measure walks: clone, open breakable rocks so
/// `walk_map` makes edges through them (the 8-point charge lands on first
/// crossing, in the Dijkstra), then stamp the slot nodes. Returns the grid and
/// the opened-rock tiles in scan order.
fn stage_grid(built: &BuiltWorld) -> (Grid, Vec<Pos>) {
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
    (grid, rock_tiles)
}

impl WalkGraph {
    /// Compile the walk-invariant base for `built`. `None` when the world has
    /// no start or no target (a degenerate world — the caller returns an empty
    /// `RouteChoice`).
    pub(super) fn compile(built: &BuiltWorld) -> Option<WalkGraph> {
        let (grid, rock_tiles) = stage_grid(built);
        let start = rom_data::find_start(&grid)?;
        let target = find_target(&grid, built.world_idx)?;
        let walk = walk_map(&grid, &built.pipe_pairs, Some(start), built.world_idx);

        // Canoe (boat) — same model as progression.rs: one boat, rides move it.
        let canoe_edges = rom_data::active_canoe_edges(built.world_idx, built.grid.eights_are_wild);
        let canoe_pair_set: FastSet<(Pos, Pos)> =
            canoe_edges.iter().flat_map(|&(a, b)| [(a, b), (b, a)]).collect();
        let initial_boat = boat_code(canoe_edges.first().map(|&(a, _)| a));

        Some(WalkGraph {
            rows: grid.rows(),
            start,
            target,
            walk,
            rock_tiles,
            canoe_edges,
            canoe_pair_set,
            initial_boat,
        })
    }

    /// Whether reusing this base for `built` is sound — i.e. `built`'s walk
    /// graph still matches the compiled one. Recomputes the BFS, so it is the
    /// explicit *check* a caller runs when a candidate MIGHT change the graph
    /// (spare-pipe trials, or a slot flip that could land on a background tile);
    /// kind and lock candidates are invariant by construction. Also the oracle
    /// behind `measure`'s debug guard. Only compiled where it is used (the guard
    /// and the reuse tests), so release builds carry neither it nor a warning.
    #[cfg(any(debug_assertions, test))]
    pub(super) fn walk_invariant(&self, built: &BuiltWorld) -> bool {
        let (grid, _) = stage_grid(built);
        let fresh = walk_map(&grid, &built.pipe_pairs, Some(self.start), built.world_idx);
        fresh.nodes == self.walk.nodes && fresh.edges == self.walk.edges
    }

    /// Measure `built`'s route choice against this compiled base. `built` must
    /// share the grid tiles and `pipe_pairs` the base was compiled from (only
    /// its slot KINDS / sections and `locks` may differ) — the reuse contract on
    /// `WalkGraph`, verified in debug builds below.
    pub(super) fn measure(&self, built: &BuiltWorld, slack: u32, want_paths: bool) -> RouteChoice {
        // Walk-invariance guard: reusing this base for `built` is sound only if
        // `built`'s walk graph still matches the compiled one. Catches a stale
        // reuse across a graph-changing trial (spare pipe, or a slot flip onto a
        // background tile). Debug-only — release/census builds never run the
        // second BFS.
        #[cfg(debug_assertions)]
        debug_assert!(
            self.walk_invariant(built),
            "WalkGraph reused across a walk-graph-changing trial: the walk graph \
             differs from the compiled base (grid tiles or pipe_pairs changed, or \
             a slot flipped onto a background tile)",
        );

        let (rows, start, target, initial_boat) =
            (self.rows, self.start, self.target, self.initial_boat);
        let walk = &self.walk;
        let canoe_edges = &self.canoe_edges;
        let canoe_pair_set = &self.canoe_pair_set;
        let rock_tiles = &self.rock_tiles;

        // Slot kind + section per node; lock path-tiles → the fort section that
        // opens them. FastMap: these are hit on every edge relax.
        let mut kind_at: FastMap<Pos, &SlotKind> = FastMap::default();
        let mut section_at: FastMap<Pos, usize> = FastMap::default();
        for slot in &built.slots {
            kind_at.insert(slot.pos, &slot.kind);
            section_at.insert(slot.pos, slot.section);
        }
        let mut lock_section: FastMap<Pos, usize> = FastMap::default();
        for lock in &built.locks {
            lock_section.insert(lock.pos, lock.fort_section);
        }

        // Cleared-mask bit layout (u64): fort sections use bits 0..section_count,
        // then one bit per level, then one bit per rock. `level_pos` inverts the
        // level bits so we can read a goal mask back into a level-set.
        let mut level_bit: FastMap<Pos, u32> = FastMap::default();
        let mut level_pos: Vec<(u32, Pos)> = Vec::new();
        let mut next = built.section_count as u32;
        for slot in &built.slots {
            if matches!(slot.kind, SlotKind::Level) {
                level_bit.insert(slot.pos, next);
                level_pos.push((next, slot.pos));
                next += 1;
            }
        }
        let mut rock_bit: FastMap<Pos, u32> = FastMap::default();
        let mut rock_pos: Vec<(u32, Pos)> = Vec::new();
        for &rp in rock_tiles {
            rock_bit.insert(rp, next);
            rock_pos.push((next, rp));
            next += 1;
        }
        debug_assert!(next <= 64, "too many clearables for a u64 mask ({next})");
        let fort_bits: u64 = (0..built.section_count).map(|s| 1u64 << s).sum();

        // Node-entry cost: charge a level/fort the FIRST time (bit flip), free
        // after. Pipes/other transit nodes are free at the node — a pipe RIDE is
        // charged on its teleport edge instead. Resolved to a per-node
        // (cost, mask bit) at compile time; `relax` applies the first-time rule.
        let node_charge = |p: Pos| -> Option<(u32, u64)> {
            if p == target {
                return None;
            }
            match kind_at.get(&p) {
                Some(SlotKind::Fortress) => Some((COST_FORT, 1u64 << section_at[&p])),
                Some(SlotKind::Level) => Some((COST_LEVEL, 1u64 << level_bit[&p])),
                _ => None,
            }
        };

        // Compile the walk graph once: pre-resolve each edge's lock section, rock
        // bit, pipe surcharge, and destination charge, so the Dijkstra's inner
        // loop touches no map except `dist`. Edge order per node is preserved
        // (and canoe-pair teleports dropped here instead of per-visit), so relax
        // order — and therefore `prev` tie-breaking — is unchanged.
        struct CompiledEdge {
            dest: Pos,
            /// (cost, mask bit) charged on first entry to `dest`.
            dest_charge: Option<(u32, u64)>,
            /// Pipe-ride cost (teleport edges only).
            surcharge: u32,
            /// Path tile crossable only once this fort section's bit is set.
            lock_sec: Option<u32>,
            /// Rock bit charged and set on first crossing.
            rock: Option<u32>,
        }
        // Direct-indexed: node (r, c) → span into a flat edge list, row-major at
        // a fixed 64-column stride (grids are at most 64 wide) — the per-pop
        // adjacency lookup is then a plain load. Per-node edge order is
        // preserved; node order in the flat list is irrelevant.
        let mut adj_span: Vec<(u32, u32)> = vec![(0, 0); rows * 64];
        let mut adj_edges: Vec<CompiledEdge> = Vec::with_capacity(4 * walk.edges.len());
        for (&p, es) in &walk.edges {
            let span_start = adj_edges.len() as u32;
            adj_edges.extend(es.iter().filter_map(|e| match e.path_pos {
                Some(pp) => Some(CompiledEdge {
                    dest: e.dest,
                    dest_charge: node_charge(e.dest),
                    surcharge: 0,
                    lock_sec: lock_section.get(&pp).map(|&s| s as u32),
                    rock: rock_bit.get(&pp).copied(),
                }),
                None if canoe_pair_set.contains(&(p, e.dest)) => None,
                None => Some(CompiledEdge {
                    dest: e.dest,
                    dest_charge: node_charge(e.dest),
                    surcharge: COST_PIPE,
                    lock_sec: None,
                    rock: None,
                }),
            }));
            adj_span[p.0 * 64 + p.1] = (span_start, adj_edges.len() as u32 - span_start);
        }

        // Dijkstra over packed (pos, cleared-mask, boat) states. We do NOT stop
        // at the first goal: once `best` is known (the first goal popped, by cost
        // order), we keep going until the popped cost exceeds `best + slack`,
        // collecting every goal-state in that band. Each is a distinct route (its
        // mask says which levels/forts/rocks it used). dist + heap are borrowed
        // from the thread-local scratch — no allocation in the hot loop.
        let (mut dist, mut heap) =
            SCRATCH.with(|s| s.replace((DistMap::placeholder(), BinaryHeap::new())));
        dist.reset();
        heap.clear();

        let init = pack(start, 0, initial_boat);
        dist.improve(init, 0, NO_PREV);
        heap.push(Reverse((0, init)));

        let mut best: Option<u32> = None;
        let mut goals: Vec<(PackedState, u32)> = Vec::new(); // (goal state, cost)

        while let Some(Reverse((cost, state))) = heap.pop() {
            if cost > dist.get(state).unwrap_or(u32::MAX) {
                continue;
            }
            if let Some(b) = best
                && cost > b + slack
            {
                break; // everything left is out of band
            }
            let pos = unpack_pos(state);
            let mask = unpack_mask(state);
            let boat = state & 0xFFFF;
            if pos == target {
                best.get_or_insert(cost);
                goals.push((state, cost));
                continue; // target is a sink — never expand through it
            }

            // Relax an edge: `edge_extra` is the pipe/rock surcharge and `mask_in`
            // is the mask after any rock-break on the crossed path tile; then add
            // the destination node's own charge (first visit only).
            let mut relax = |dest: Pos,
                             dest_charge: Option<(u32, u64)>,
                             boat_after: u128,
                             edge_extra: u32,
                             mask_in: u64| {
                let (nc, nm) = match dest_charge {
                    Some((c, bit)) if mask_in & bit == 0 => (c, mask_in | bit),
                    _ => (0, mask_in),
                };
                let new_cost = cost + edge_extra + nc;
                let key = pack(dest, nm, boat_after);
                if dist.improve(key, new_cost, state) {
                    heap.push(Reverse((new_cost, key)));
                }
            };

            let (span_start, span_len) = adj_span[pos.0 * 64 + pos.1];
            for e in &adj_edges[span_start as usize..(span_start + span_len) as usize] {
                // Lock: crossable only once its fort's section is open.
                if let Some(sec) = e.lock_sec
                    && mask & (1u64 << sec) == 0
                {
                    continue;
                }
                // Rock: break it (charge once) the first time crossed.
                let mut edge_extra = e.surcharge;
                let mut mask_after = mask;
                if let Some(rb) = e.rock {
                    let bit = 1u64 << rb;
                    if mask_after & bit == 0 {
                        edge_extra += COST_ROCK;
                        mask_after |= bit;
                    }
                }
                relax(e.dest, e.dest_charge, boat, edge_extra, mask_after);
            }

            // Canoe rides: free, and only when the boat sits at the current node.
            if boat == boat_code(Some(pos)) {
                for &(a, b) in canoe_edges {
                    let dest = if a == pos {
                        b
                    } else if b == pos {
                        a
                    } else {
                        continue;
                    };
                    relax(dest, node_charge(dest), boat_code(Some(dest)), 0, mask);
                }
            }
        }

        let Some(best_cost) = best else {
            SCRATCH.with(|s| *s.borrow_mut() = (dist, heap));
            return RouteChoice::default();
        };

        // Read each in-band goal state back into a route; keep the cheapest per
        // distinct (level-set, rock-set) — the route identity (remembering the
        // state so we can rebuild its path).
        type Plays = (BTreeSet<Pos>, BTreeSet<Pos>);
        let mut by_plays: HashMap<Plays, (u32, PackedState)> = HashMap::new();
        for (state, cost) in goals {
            let mask = unpack_mask(state);
            let levels: BTreeSet<Pos> = level_pos
                .iter()
                .filter(|(bit, _)| mask & (1u64 << bit) != 0)
                .map(|&(_, pos)| pos)
                .collect();
            let rocks: BTreeSet<Pos> = rock_pos
                .iter()
                .filter(|(bit, _)| mask & (1u64 << bit) != 0)
                .map(|&(_, pos)| pos)
                .collect();
            by_plays
                .entry((levels, rocks))
                .and_modify(|e| {
                    if cost < e.0 {
                        *e = (cost, state);
                    }
                })
                .or_insert((cost, state));
        }

        // Rebuild the node path for a goal state by walking the recorded
        // predecessors back to start. Counts-only measures skip this.
        let reconstruct = |goal: PackedState| -> Vec<Pos> {
            if !want_paths {
                return Vec::new();
            }
            let mut nodes = vec![unpack_pos(goal)];
            let mut cur = goal;
            while let Some(p) = dist.prev_of(cur) {
                nodes.push(unpack_pos(p));
                cur = p;
            }
            nodes.reverse();
            nodes
        };

        let mut routes: Vec<ChoiceRoute> = by_plays
            .into_iter()
            .map(|((levels, rocks), (cost, state))| ChoiceRoute {
                cost,
                levels,
                rocks,
                forts: (unpack_mask(state) & fort_bits).count_ones(),
                path: reconstruct(state),
            })
            .collect();

        // Reconstruction done — return the scratch for the next call.
        SCRATCH.with(|s| *s.borrow_mut() = (dist, heap));

        // Domination filter: drop a route that plays a strict superset of
        // another route's plays (levels AND rocks broken) at no lower cost —
        // a within-slack detour, not a real choice. Subset is required on
        // BOTH sets: a rock break spends a hammer, so a rock shortcut never
        // dominates the rock-free walk around it (they are a resource-trade
        // choice, e.g. vanilla W2's rock vs its three-level detour).
        // (Identities are already distinct, so subset on both ⇒ strict.)
        let dominated: Vec<bool> = routes
            .iter()
            .map(|ri| {
                routes.iter().any(|rj| {
                    rj.cost <= ri.cost
                        && (rj.levels != ri.levels || rj.rocks != ri.rocks)
                        && rj.levels.is_subset(&ri.levels)
                        && rj.rocks.is_subset(&ri.rocks)
                })
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
                .then_with(|| a.rocks.iter().cmp(b.rocks.iter()))
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
}

/// Compile the walk-invariant base and measure `built` against it. The compile
/// and the measure are split (`WalkGraph::compile` / `WalkGraph::measure`) so a
/// selection step can hoist the compile and reuse it across its kind/lock
/// candidates; the single-shot public entry points keep the one-call form.
fn analyze_route_choice_inner(built: &BuiltWorld, slack: u32, want_paths: bool) -> RouteChoice {
    match WalkGraph::compile(built) {
        Some(g) => g.measure(built, slack, want_paths),
        None => RouteChoice::default(),
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
            route.rocks.len(),
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
