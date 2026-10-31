//! The world-graph pass: the spine, the pad budget, and which world gets which
//! pad role.
//!
//! This is the layer that knows a world graph exists. Everything below it — the
//! per-world builder, the role pools in [`roles`](super::roles) — is handed a
//! count and a set of roles and never learns why.
//!
//! ## Pads come in pairs
//!
//! A telepad **pair** is one link the player can walk both ways: two pad tiles,
//! each pointing at the other. That is the mode's whole shape — a pad the
//! player cannot come back through is a trapdoor, and a maze made of trapdoors
//! is a corridor with extra steps. `testrom --telepad A:B` has built exactly
//! this since the POC.
//!
//! The charter's "a pad is one-way by construction, so a two-way link costs two
//! ids" is a statement about the *mechanism*, not the design: each pad TILE
//! owns one arrival row, so a pair spends two of the sixteen. `PORTAL_MAX = 16`
//! therefore means **at most eight pairs**. (The one-way edge in the maze is
//! the airship, and that is a different kind of edge entirely.)
//!
//! So this module does two things in order: **claim sites**, then **pair them
//! up**. An odd site left over at the end is not placed at all — a pad with
//! nothing to point at is a wasted arrival row and a dead end on the map.
//!
//! ## The allocation order, and why
//!
//! The ids are spent in strict priority order, because the first claim is a
//! safety obligation and the rest are flavour:
//!
//! 1. **Hub pads**, for every world whose start region cannot be escaped with
//!    what the player has on arrival. This is invariant 3, and it is the only
//!    claim that can make a seed unplayable if it goes unmet.
//! 2. **Shortcut pads**, behind a lock — the ones that make the graph a graph
//!    rather than a chain.
//! 3. **Free pads** with whatever is left.
//!
//! A role is a request, not a contract: `W1` has no island, some worlds have no
//! gated region, and the placer records what it actually granted so the
//! distribution is a census output rather than an assumption.

use rand::Rng;
use rand::seq::{IndexedRandom, SliceRandom};

use super::super::rom_data::{self, Pos};
use super::metrics::ShortestBound;
use super::roles::{PadRole, WorldTerrain, classify};
use super::{GlobalState, MazeEdge};

/// Global cap on telepads. Each pad TILE owns one arrival row, and there are
/// `PORTAL_MAX` rows; a pair is two tiles, so this is **eight pairs**.
pub(crate) const PAD_BUDGET: usize = crate::randomize::maze::world_persist::PORTAL_MAX;

/// Hard cap on pads in one world. Above this a world stops being a place and
/// starts being a switchboard.
pub(crate) const PADS_PER_WORLD_MAX: usize = 3;

/// The generator's dials. Every one of these has to earn its keep against the
/// null-model baselines before it stays — see `docs/world_maze_design.md`,
/// "The null-model baselines".
#[derive(Clone, Copy, Debug)]
pub(crate) struct Knobs {
    /// How often a **pair** spans two worlds rather than sitting inside one.
    /// 0.0 makes every link a same-world shortcut (position keying gives those
    /// for free); 1.0 makes every link a crossing. **This is the maze knob** —
    /// it is what decides whether the eight worlds are a graph or eight rooms
    /// with local shortcuts.
    pub(crate) foreign_landing_bias: f64,
    /// Which way the key-assignment fill pushes a lock's fort. `-1.0` keeps
    /// keys local (a lock's fort is near it, vanilla-ish); `0.0` accepts any
    /// solvable reassignment, which is the null model; `+1.0` pushes keys as
    /// far away as the graph allows, which means into other worlds — the
    /// backtracking maze.
    ///
    /// There is deliberately **no lock-density knob.** Every fortress must
    /// have exactly one lock (the charter's map-legibility rule: a lock
    /// breaking is the only feedback that says which fort did it), so the
    /// assignment is a bijection and "how many locks" is not a free parameter.
    pub(crate) fort_distance_bias: f64,
}

impl Default for Knobs {
    /// The null model: uniform everywhere. Every knob's default is the value
    /// that makes it do nothing, so a census with defaults reproduces the
    /// baseline and any movement is attributable to a single dial.
    fn default() -> Self {
        Knobs { foreign_landing_bias: 7.0 / 8.0, fort_distance_bias: 0.0 }
    }
}

/// One decided pad, with the role that was asked for beside the role the
/// terrain actually granted.
///
/// `edge` points at this pad's **partner**, and the partner's own `PlacedPad`
/// points back — the two are emitted together and never apart.
// Reason: production consumes only `edge`. The three alongside it are the
// granted-vs-requested distribution the charter asks for as a census output,
// and the census is their only reader.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct PlacedPad {
    pub(crate) edge: MazeEdge,
    pub(crate) requested: PadRole,
    pub(crate) granted: PadRole,
    /// True when the partner is in a different world.
    pub(crate) foreign: bool,
}

/// A claimed pad tile, before it knows what it is paired with.
#[derive(Clone, Copy, Debug)]
struct Site {
    world: usize,
    pos: Pos,
    requested: PadRole,
    granted: PadRole,
}

/// The shortest same-world link worth two arrival ids, in grid cells
/// (Manhattan).
///
/// A pad that drops the player four tiles from where they stood spends two of
/// sixteen arrival rows to save two moves, and reads to a player as a bug — the
/// report that opened this was exactly that shape, and the census had a
/// same-world hop of span **0**: a pad that teleported to its own tile.
///
/// Manhattan on the grid rather than a walk distance, deliberately. What makes
/// a hop degenerate is that the player can SEE where they came from, and that
/// is a picture, not a path. The map moves two cells at a time, so 8 is four
/// map moves — far enough to be off-screen-ish and to feel like travel.
const SAME_WORLD_MIN_SPAN: usize = 8;

/// The valley: World 8's screen 3, the bridge approach to Bowser's castle.
///
/// Read off the grid rather than off the spine — the maze renumbers worlds for
/// display, but the castle is on `W8_IDX`'s map whatever it is called.
fn in_valley(world: usize, pos: Pos) -> bool {
    world == rom_data::W8_IDX && pos.1 / 16 == 3
}

/// The last valley column a pad tile may take: the level at (5,50) and the
/// fortress cell at (5,52), the two nodes before the dealt bridge spans. A pad
/// any further right would skip the spans and land beside Bowser.
const VALLEY_PAD_LAST_COL: usize = 52;

/// Manhattan span between two cells of the same world's grid.
fn span(a: Pos, b: Pos) -> usize {
    a.0.abs_diff(b.0) + a.1.abs_diff(b.1)
}

/// Run the world-graph pass over a maze that has no pads yet.
///
/// Returns the pads in claim order, so the caller can see the budget being
/// spent on safety before flavour. The length is always even: pads are emitted
/// two at a time, as pairs.
pub(crate) fn plan_pads<R: Rng>(state: &GlobalState, knobs: &Knobs, rng: &mut R) -> Vec<PlacedPad> {
    let terrain = classify_all(state);
    let sites = claim_sites(state, &terrain, rng);
    pair_up(sites, state, knobs, rng)
}

/// Sort every world's terrain into the pools the roles draw from.
fn classify_all(state: &GlobalState) -> Vec<WorldTerrain> {
    state
        .worlds
        .iter()
        .map(|w| classify(w, &state.locks, &state.reserved_in(w.world_idx)))
        .collect()
}

/// Claim pad TILES — where a pad may stand, and what role it was asked for.
///
/// Safety first: a world whose start region cannot be escaped with what the
/// player carries on arrival gets a hub site before anything else claims one.
/// The rest of the budget is dealt round-robin over a shuffled world order so
/// no world is systematically starved.
fn claim_sites<R: Rng>(state: &GlobalState, terrain: &[WorldTerrain], rng: &mut R) -> Vec<Site> {
    let mut out: Vec<Site> = Vec::new();

    let mut needy: Vec<usize> =
        (0..state.worlds.len()).filter(|&wi| !state.start_region_escapable(wi)).collect();
    needy.shuffle(rng);
    for wi in needy {
        if out.len() == PAD_BUDGET {
            break;
        }
        if let Some(site) = claim_one(terrain, wi, PadRole::Hub, &out, rng) {
            out.push(site);
        }
    }

    let mut order: Vec<usize> = (0..state.worlds.len()).collect();
    order.shuffle(rng);
    let mut want: Vec<usize> = order
        .iter()
        .map(|&wi| {
            let rolled = roll_count(&PAD_WEIGHTS, rng);
            rolled.saturating_sub(out.iter().filter(|s| s.world == wi).count())
        })
        .collect();
    while out.len() < PAD_BUDGET && want.iter().any(|&n| n > 0) {
        let mut spent_this_round = false;
        for (slot, &wi) in order.iter().enumerate() {
            if out.len() == PAD_BUDGET || want[slot] == 0 {
                continue;
            }
            // Shortcut is asked for first and falls back to Free, which is what
            // "a request, not a contract" means in code.
            let role = if rng.random_bool(0.5) { PadRole::Shortcut } else { PadRole::Free };
            if let Some(site) = claim_one(terrain, wi, role, &out, rng) {
                out.push(site);
                spent_this_round = true;
            }
            want[slot] -= 1;
        }
        if !spent_this_round {
            break;
        }
    }

    out
}

/// Pad-tile roll weights for 0..=3 tiles, the same for every world: never
/// none, otherwise one, two or three at 2:4:4.
///
/// One roll serves every world because [`PAD_JUMP_CAP`] decides what a pad
/// costs the run, not where it stands. Before the cap, density had to be kept
/// out of the early worlds by hand (lighter rolls for the opening and for W8),
/// since pads there were the shortcuts that tripped the content floor; with
/// the cap pairing on depth, this roll alone lands at ~14.7 tiles a deal —
/// the count the retired post-fill rescue pass used to reach.
const PAD_WEIGHTS: [u32; 4] = [0, 2, 4, 4];

/// Weighted 0..=3.
fn roll_count<R: Rng>(weights: &[u32; 4], rng: &mut R) -> usize {
    let total: u32 = weights.iter().sum();
    let mut roll = rng.random_range(..total);
    for (n, &w) in weights.iter().enumerate() {
        if roll < w {
            return n;
        }
        roll -= w;
    }
    0
}

/// Claim one pad tile in `world` for `role`, or `None` when the terrain cannot
/// host it.
///
/// Falls back from the role's own pool to any free site, and records which role
/// the chosen tile actually satisfies.
fn claim_one<R: Rng>(
    terrain: &[WorldTerrain],
    world: usize,
    role: PadRole,
    claimed: &[Site],
    rng: &mut R,
) -> Option<Site> {
    if claimed.iter().filter(|s| s.world == world).count() >= PADS_PER_WORLD_MAX {
        return None;
    }
    let t = &terrain[world];
    let free = |sites: &[Pos]| -> Vec<Pos> {
        sites
            .iter()
            .copied()
            .filter(|&p| !claimed.iter().any(|s| s.world == world && s.pos == p))
            .filter(|&p| !in_valley(world, p) || p.1 <= VALLEY_PAD_LAST_COL)
            .collect()
    };
    let mut pool = free(&t.sites_for(role));
    if pool.is_empty() {
        pool = free(&t.all_sites());
    }
    let &pos = pool.choose(rng)?;
    Some(Site { world, pos, requested: role, granted: t.role_of_site(pos) })
}

/// The widest depth gap a pad pair may span, in levels: how much deeper into
/// the run one end is than the other, by [`ShortestBound::depths`] over the
/// maze so far (re-read after every pair, since each pad moves what is past
/// it).
///
/// **Why depth.** A pad joining two places at much the same depth saves the
/// run little; one joining an early place to a deep one is the shortcut that
/// puts a deal under the content floor. Over 10,000 K=0 deals, pads spanning
/// 8+ were a fifth of all pads and carried half of every level saved, and no
/// pad spanning 5 or less saved 6+. Which worlds a pad joins, or its role,
/// predicted far less.
///
/// **Why 9.** Places close in depth are mostly in the same world or the next
/// one along the airship chain, so a tight cap alone turns pads into
/// neighbour hops: at 8 with a random pick, 60% of pairs joined adjacent
/// worlds (uncapped: 24%), which a playtest felt. [`choose_partner`] now
/// prefers a partner two or more worlds away, and with that the cap could
/// widen by one. Measured (2,000 seeds, flat [`PAD_WEIGHTS`], vanilla /
/// generated): neighbour pairs 25% / 22%, K=0 redeals 19% / 25% (uncapped:
/// 39% / 43%), ~15 tiles a deal. A cycling deck of caps (12/10/8/6) measured
/// the same and was not worth the machinery.
const PAD_JUMP_CAP: u32 = 9;

/// The chance, per level past [`PAD_JUMP_CAP`], that a site with nothing in
/// reach keeps its nearest partner anyway: one past keeps it this often, two
/// past this squared, and so on. Otherwise the site is dropped.
///
/// This is the dial for the occasional big jump. At 0.7 (with a cap of 8)
/// about one seed in six shipped a pad past the cap, nearly all of them one or
/// two over; about a fifth of the deals that rolled one were redealt by the
/// floor, against a tenth overall, so most survive to the player.
const PAD_JUMP_KEEP: f64 = 0.7;

/// Turn claimed sites into pairs, and throw away anything left over.
///
/// Each site is offered the kind of partner [`Knobs::foreign_landing_bias`]
/// rolled — another world, or its own — and falls back to the other kind
/// rather than going unpaired. What it will not settle for is a partner much
/// deeper or shallower than itself: see [`PAD_JUMP_CAP`].
fn pair_up<R: Rng>(
    sites: Vec<Site>,
    state: &GlobalState,
    knobs: &Knobs,
    rng: &mut R,
) -> Vec<PlacedPad> {
    let spine_pos = spine_positions(state);
    // Depth is re-read after every pair: a new pad moves everything past it.
    let mut bound = ShortestBound::new(state);
    let mut depth = bound.depths();
    let mut pool = sites;
    pool.shuffle(rng);
    let mut out = Vec::new();
    while let Some(a) = pool.pop() {
        let cross = rng.random_bool(knobs.foreign_landing_bias.clamp(0.0, 1.0));
        // No legal partner left: `a` is dropped rather than shipped pointing
        // nowhere.
        let depth_of = |s: &Site| depth[bound.cell((s.world, s.pos))];
        let Some(i) = choose_partner(&pool, &a, cross, &depth_of, &spine_pos, rng) else {
            continue;
        };
        let b = pool.remove(i);
        bound.link((a.world, a.pos), (b.world, b.pos));
        depth = bound.depths();
        out.push(half(a, b));
        out.push(half(b, a));
    }
    out
}

/// Each world's place on the airship chain, followed from the start world. A
/// world off the chain gets the chain's length: it is reached only by pad, as
/// far from anywhere as the graph goes (the fill reads it the same way).
fn spine_positions(state: &GlobalState) -> [usize; 8] {
    let mut chain = vec![state.start.0];
    while let Some(next) = state.edges.iter().find_map(|e| match *e {
        MazeEdge::Airship { from_world, to_world } if from_world == *chain.last()? => {
            Some(to_world)
        }
        _ => None,
    }) {
        if chain.contains(&next) {
            break;
        }
        chain.push(next);
    }
    let mut pos = [chain.len(); 8];
    for (i, &w) in chain.iter().enumerate() {
        pos[w] = i;
    }
    pos
}

/// Index into `pool` of a site `a` may be paired with: within
/// [`PAD_JUMP_CAP`] of its depth if any is — a world two or more airships
/// away first, then the rolled kind, then any; failing that, the nearest
/// depth there is, on a [`PAD_JUMP_KEEP`] roll. `None` drops `a`.
///
/// `spine_pos` is each world's place on the airship chain; "two away" is
/// measured there, not in ROM order.
fn choose_partner<R: Rng>(
    pool: &[Site],
    a: &Site,
    cross: bool,
    depth_of: &dyn Fn(&Site) -> u32,
    spine_pos: &[usize; 8],
    rng: &mut R,
) -> Option<usize> {
    // Every `pick` below takes this, the fallbacks included: they relax the
    // *preferences* (the rolled cross/local kind, then the jump), never
    // legality.
    let legal = |b: &Site| b.world != a.world || span(a.pos, b.pos) >= SAME_WORLD_MIN_SPAN;
    let short = |b: &Site| depth_of(a).abs_diff(depth_of(b)) <= PAD_JUMP_CAP;
    let kind = |b: &Site| (b.world != a.world) == cross;
    let far = |b: &Site| spine_pos[a.world].abs_diff(spine_pos[b.world]) >= 2;
    let pick = |f: &dyn Fn(&Site) -> bool, rng: &mut R| -> Option<usize> {
        let candidates: Vec<usize> =
            pool.iter().enumerate().filter(|(_, b)| f(b)).map(|(i, _)| i).collect();
        candidates.choose(rng).copied()
    };
    pick(&|b| legal(b) && short(b) && far(b), rng)
        .or_else(|| pick(&|b| legal(b) && short(b) && kind(b), rng))
        .or_else(|| pick(&|b| legal(b) && short(b), rng))
        .or_else(|| {
            // Nothing within the cap: the nearest depth there is, kept only on
            // a roll that gets harder the further past the cap it reaches.
            // Losing the roll drops the site.
            let jump = |b: &Site| depth_of(a).abs_diff(depth_of(b));
            let nearest = pool.iter().filter(|b| legal(b)).map(jump).min()?;
            let over = nearest.saturating_sub(PAD_JUMP_CAP).min(64) as i32;
            if !rng.random_bool(PAD_JUMP_KEEP.powi(over)) {
                return None;
            }
            pick(&|b| legal(b) && jump(b) == nearest, rng)
        })
}

/// One half of a pair: the pad standing at `from`, aimed at its partner.
fn half(from: Site, to: Site) -> PlacedPad {
    PlacedPad {
        edge: MazeEdge::Pad { from: (from.world, from.pos), to: (to.world, to.pos) },
        requested: from.requested,
        granted: from.granted,
        foreign: to.world != from.world,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    /// The roll lands on exactly the counts its table weights: a zero weight
    /// is a count a world never rolls, and every other count turns up.
    #[test]
    fn pad_roll_follows_its_weights() {
        let mut rng = ChaCha8Rng::seed_from_u64(9);
        let mut hist = [0usize; 4];
        for _ in 0..40_000 {
            hist[roll_count(&PAD_WEIGHTS, &mut rng)] += 1;
        }
        for (n, (&w, &seen)) in PAD_WEIGHTS.iter().zip(&hist).enumerate() {
            assert_eq!(w > 0, seen > 0, "count {n} rolled {seen} times");
        }
    }
}
