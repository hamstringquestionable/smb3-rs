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
use super::roles::{PadRole, WorldTerrain, classify};
use super::{GlobalState, MazeEdge, MazePos};

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

/// May these two sites be paired, given that a pad in the valley is a door
/// onto the endgame?
///
/// **The shortcut is kept, and priced.** A pad that lands on Bowser's bridge is
/// the best thing the mode can hand a player who went looking for it, and the
/// worst thing it can hand one who tripped over it on the way past. The
/// difference is entirely in the *other* end, so that is what this constrains:
/// the far side must be a site the world's own locks gate ([`PadRole::Shortcut`]
/// — reachable only once some fortress falls) and it must be in a different
/// world. Finding the valley then costs a crossing and a fortress.
///
/// Symmetric because a pair is two-way: either half may be the valley one, and
/// a pad whose ends are *both* in the valley is refused outright (neither can
/// gate the other).
///
/// **Why gatedness and not depth.** The honest question is "how late in the run
/// is the far end", and that is [`super::Spheres`] — which does not exist yet.
/// Pads are planned before `fill` assigns keys (see `generate`), so the only
/// reachability facts available here are the ones `roles::classify` derives
/// from lock *positions*: the walk with every lock shut against the walk with
/// them open. That split is stable under the fill's swapping, which is what
/// makes it safe to lean on. Depth would have to be a clause in the deal-accept
/// predicate instead, paid for in redeals.
///
/// **Not in the opening, either.** A gated site in the start world is still
/// one fortress from the endgame, so the far end must also stay out of
/// [`Opening`]: the start world, the world its airship leads to, and any world
/// a pad links straight to the start world. `pads` is every pad already
/// standing, so the rule runs both ways — a valley partner may not land in a
/// world linked to the start, and a new link to the start world may not pull
/// an existing partner's world into the opening.
fn valley_earned(a: &Site, b: &Site, opening: Opening, pads: &[(MazePos, MazePos)]) -> bool {
    let ok = |valley: &Site, other: &Site| {
        !in_valley(valley.world, valley.pos)
            || (other.granted == PadRole::Shortcut
                && other.world != valley.world
                && !opening.contains(other.world, pads))
    };
    let partner = |w: usize| pads.iter().any(|&(from, (tw, tp))| from.0 == w && in_valley(tw, tp));
    let keeps = |x: &Site, y: &Site| x.world != opening.start || !partner(y.world);
    ok(a, b) && ok(b, a) && keeps(a, b) && keeps(b, a)
}

/// The worlds a player reaches before beating anything elsewhere — where a
/// valley pad's far end may not stand (see [`valley_earned`]).
#[derive(Clone, Copy, Debug)]
struct Opening {
    /// The spine's first world.
    start: usize,
    /// Where clearing the start world's airship leads.
    next: Option<usize>,
}

impl Opening {
    fn of(state: &GlobalState) -> Self {
        let start = state.start.0;
        let next = state.edges.iter().find_map(|e| match *e {
            MazeEdge::Airship { from_world, to_world } if from_world == start => Some(to_world),
            _ => None,
        });
        Self { start, next }
    }

    /// Is `world` in the opening, given the pads already standing? Pads come
    /// in pairs, so checking the start world's halves sees every link.
    fn contains(self, world: usize, pads: &[(MazePos, MazePos)]) -> bool {
        world == self.start
            || Some(world) == self.next
            || pads.iter().any(|&(from, to)| from.0 == self.start && to.0 == world)
    }
}

/// `(from, to)` for each placed pad.
fn pad_ends(placed: &[PlacedPad]) -> Vec<(MazePos, MazePos)> {
    placed
        .iter()
        .filter_map(|p| match p.edge {
            MazeEdge::Pad { from, to } => Some((from, to)),
            MazeEdge::Airship { .. } => None,
        })
        .collect()
}

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
    pair_up(sites, Opening::of(state), knobs, rng)
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

    let opening = Opening::of(state);
    let mut order: Vec<usize> = (0..state.worlds.len()).collect();
    order.shuffle(rng);
    let mut want: Vec<usize> = order
        .iter()
        .map(|&wi| {
            let rolled = roll_count(pad_weights(opening, wi), rng);
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

/// Pad-tile roll weights for 0..=3 tiles, by where a world sits.
///
/// **Where a pad stands decides what it costs the run**, so one roll for every
/// world could not hold the pad count, the redeal rate and the valley rate at
/// once. This roll is the whole pad count — the post-fill rescue pass that
/// used to add ~6 tiles a deal is gone (see `generate`) — and a uniform roll
/// dense enough to replace it pushed the content floor's redeals to 32-48% at
/// K=0, because planned pads shorten a run more than rescue pads did.
///
/// * [`OPENING_PAD_WEIGHTS`]: the start world and the one its airship leads
///   to. Pads there are the earliest shortcuts, so they are what trips the
///   floor — lightening only these cut K=0 redeals from ~22% to ~13%.
/// * [`W8_PAD_WEIGHTS`]: the valley's world. Its tile count is the valley-pad
///   rate (one tile lands in the valley ~30% of the time), so this is the
///   valley dial: the middle roll here put valley pads on ~25% of seeds.
/// * [`MIDDLE_PAD_WEIGHTS`]: the other five, which carry the count.
///
/// Measured at 200 seeds x K=0/3/7 (`valley_pad_census`), vanilla then
/// generated maps: ~12.4 tiles a deal (14.7 with rescue), K=0 redeals 14% /
/// 15% (30% / 34.5% with rescue), valley pads on 15% / 16% of seeds.
const OPENING_PAD_WEIGHTS: [u32; 4] = [3, 5, 1, 0];
/// See [`OPENING_PAD_WEIGHTS`].
const W8_PAD_WEIGHTS: [u32; 4] = [3, 5, 1, 0];
/// See [`OPENING_PAD_WEIGHTS`].
const MIDDLE_PAD_WEIGHTS: [u32; 4] = [0, 2, 4, 4];

/// Which roll world `wi` takes — see [`OPENING_PAD_WEIGHTS`].
fn pad_weights(opening: Opening, wi: usize) -> &'static [u32; 4] {
    if wi == opening.start || Some(wi) == opening.next {
        &OPENING_PAD_WEIGHTS
    } else if wi == rom_data::W8_IDX {
        &W8_PAD_WEIGHTS
    } else {
        &MIDDLE_PAD_WEIGHTS
    }
}

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
            .collect()
    };
    let mut pool = free(&t.sites_for(role));
    if pool.is_empty() {
        pool = free(&t.all_sites());
    }
    let &pos = pool.choose(rng)?;
    Some(Site { world, pos, requested: role, granted: t.role_of_site(pos) })
}

/// Turn claimed sites into pairs, and throw away anything left over.
///
/// Each site is offered the kind of partner [`Knobs::foreign_landing_bias`]
/// rolled — another world, or its own — and falls back to the other kind rather
/// than going unpaired, because a crossing is what the budget is for and an
/// unpaired site is nothing at all.
fn pair_up<R: Rng>(
    sites: Vec<Site>,
    opening: Opening,
    knobs: &Knobs,
    rng: &mut R,
) -> Vec<PlacedPad> {
    let mut pool = sites;
    pool.shuffle(rng);
    let mut out = Vec::new();
    while let Some(a) = pool.pop() {
        let cross = rng.random_bool(knobs.foreign_landing_bias.clamp(0.0, 1.0));
        // No legal partner left: `a` is dropped rather than shipped pointing
        // nowhere.
        let pads = pad_ends(&out);
        let Some(i) = choose_partner(&pool, &a, cross, opening, &pads, rng) else { continue };
        let b = pool.remove(i);
        out.push(half(a, b));
        out.push(half(b, a));
    }
    out
}

/// Index into `pool` of a site `a` may be paired with, preferring the rolled
/// kind and settling for the other.
fn choose_partner<R: Rng>(
    pool: &[Site],
    a: &Site,
    cross: bool,
    opening: Opening,
    pads: &[(MazePos, MazePos)],
    rng: &mut R,
) -> Option<usize> {
    // Both `pick`s below take this, the fallback included — deliberately. The
    // fallback exists to relax the rolled cross/local *preference*, never
    // legality, and an unearned valley pad is illegal rather than unpreferred.
    let legal = |b: &Site| {
        (b.world != a.world || span(a.pos, b.pos) >= SAME_WORLD_MIN_SPAN)
            && valley_earned(a, b, opening, pads)
    };
    let pick = |f: &dyn Fn(&Site) -> bool, rng: &mut R| -> Option<usize> {
        let candidates: Vec<usize> =
            pool.iter().enumerate().filter(|(_, b)| f(b)).map(|(i, _)| i).collect();
        candidates.choose(rng).copied()
    };
    pick(&|b: &Site| legal(b) && (b.world != a.world) == cross, rng).or_else(|| pick(&legal, rng))
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

    /// Each roll lands on exactly the counts its table weights: a zero weight
    /// is a count the world never gets, and every other count turns up.
    #[test]
    fn pad_roll_follows_its_weights() {
        let mut rng = ChaCha8Rng::seed_from_u64(9);
        for weights in [&OPENING_PAD_WEIGHTS, &W8_PAD_WEIGHTS, &MIDDLE_PAD_WEIGHTS] {
            let mut hist = [0usize; 4];
            for _ in 0..40_000 {
                hist[roll_count(weights, &mut rng)] += 1;
            }
            for (n, (&w, &seen)) in weights.iter().zip(&hist).enumerate() {
                assert_eq!(w > 0, seen > 0, "{weights:?}: count {n} rolled {seen} times");
            }
        }
    }
}
