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

use super::super::rom_data::Pos;
use super::roles::{PadRole, WorldTerrain, classify};
use super::{GlobalState, MazeEdge};

/// Global cap on telepads. Each pad TILE owns one arrival row, and there are
/// `PORTAL_MAX` rows; a pair is two tiles, so this is **eight pairs**.
pub(crate) const PAD_BUDGET: usize = super::super::world_persist::PORTAL_MAX;

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
    pub foreign_landing_bias: f64,
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
    pub fort_distance_bias: f64,
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
    pub edge: MazeEdge,
    pub requested: PadRole,
    pub granted: PadRole,
    /// True when the partner is in a different world.
    pub foreign: bool,
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
pub(crate) const SAME_WORLD_MIN_SPAN: usize = 8;

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
    pair_up(sites, knobs, rng)
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
        if let Some(site) = claim_one(terrain, wi, PadRole::Hub, &out, None, rng) {
            out.push(site);
        }
    }

    let mut order: Vec<usize> = (0..state.worlds.len()).collect();
    order.shuffle(rng);
    let mut want: Vec<usize> = order
        .iter()
        .map(|&wi| roll_count(rng).saturating_sub(out.iter().filter(|s| s.world == wi).count()))
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
            if let Some(site) = claim_one(terrain, wi, role, &out, None, rng) {
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

/// Pads per world: 0..=3, biased to 1 (the charter's distribution).
const PAD_COUNT_WEIGHTS: [u32; 4] = [2, 5, 2, 1];

/// Weighted 0..=3.
pub(crate) fn roll_count<R: Rng>(rng: &mut R) -> usize {
    let total: u32 = PAD_COUNT_WEIGHTS.iter().sum();
    let mut roll = rng.random_range(..total);
    for (n, &w) in PAD_COUNT_WEIGHTS.iter().enumerate() {
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
/// the chosen tile actually satisfies. `far_from` is the partner constraint: a
/// site in the same world as an already-claimed half has to be
/// [`SAME_WORLD_MIN_SPAN`] away from it.
fn claim_one<R: Rng>(
    terrain: &[WorldTerrain],
    world: usize,
    role: PadRole,
    claimed: &[Site],
    far_from: Option<Pos>,
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
            .filter(|&p| far_from.is_none_or(|q| span(p, q) >= SAME_WORLD_MIN_SPAN))
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
fn pair_up<R: Rng>(sites: Vec<Site>, knobs: &Knobs, rng: &mut R) -> Vec<PlacedPad> {
    let mut pool = sites;
    pool.shuffle(rng);
    let mut out = Vec::new();
    while let Some(a) = pool.pop() {
        let cross = rng.random_bool(knobs.foreign_landing_bias.clamp(0.0, 1.0));
        // No legal partner left: `a` is dropped rather than shipped pointing
        // nowhere.
        let Some(i) = choose_partner(&pool, &a, cross, rng) else { continue };
        let b = pool.remove(i);
        out.push(half(a, b));
        out.push(half(b, a));
    }
    out
}

/// Index into `pool` of a site `a` may be paired with, preferring the rolled
/// kind and settling for the other.
fn choose_partner<R: Rng>(pool: &[Site], a: &Site, cross: bool, rng: &mut R) -> Option<usize> {
    let legal = |b: &Site| b.world != a.world || span(a.pos, b.pos) >= SAME_WORLD_MIN_SPAN;
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

/// Hub pads for worlds the fill left unable to escape their own start region.
///
/// Separate from [`plan_pads`] because it runs after the key assignment, on
/// whatever budget survived it. A rescue costs **two** ids, not one, because a
/// pad has to have a partner to be a pad at all — so a world with only one id
/// left gets nothing, and is reported rather than silently shipped.
pub(crate) fn rescue_pads<R: Rng>(
    state: &GlobalState,
    worlds: &[usize],
    knobs: &Knobs,
    rng: &mut R,
) -> Vec<PlacedPad> {
    let terrain = classify_all(state);
    // Everything already standing counts against the budget and the per-world
    // cap, whichever pass placed it.
    let mut claimed: Vec<Site> = state
        .pad_edges()
        .iter()
        .map(|&((world, pos), _)| Site {
            world,
            pos,
            requested: PadRole::Free,
            granted: PadRole::Free,
        })
        .collect();

    let mut out = Vec::new();
    for &wi in worlds {
        if claimed.len() + 2 > PAD_BUDGET {
            break;
        }
        let Some(hub) = claim_one(&terrain, wi, PadRole::Hub, &claimed, None, rng) else {
            continue;
        };
        claimed.push(hub);
        match claim_partner(state, &terrain, &hub, knobs, &claimed, rng) {
            Some(partner) => {
                claimed.push(partner);
                out.push(half(hub, partner));
                out.push(half(partner, hub));
            }
            // Nowhere to point: hand the site back rather than ship a pad that
            // leads nowhere.
            None => {
                claimed.pop();
            }
        }
    }
    out
}

/// A partner tile for a rescue hub, in a world chosen the way
/// [`Knobs::foreign_landing_bias`] asks and falling through to any world that
/// can host one.
fn claim_partner<R: Rng>(
    state: &GlobalState,
    terrain: &[WorldTerrain],
    hub: &Site,
    knobs: &Knobs,
    claimed: &[Site],
    rng: &mut R,
) -> Option<Site> {
    let cross = rng.random_bool(knobs.foreign_landing_bias.clamp(0.0, 1.0));
    let mut order: Vec<usize> = (0..state.worlds.len()).filter(|&o| o != hub.world).collect();
    order.shuffle(rng);
    if cross {
        order.push(hub.world);
    } else {
        order.insert(0, hub.world);
    }
    order.into_iter().find_map(|wi| {
        let far = (wi == hub.world).then_some(hub.pos);
        claim_one(terrain, wi, PadRole::Free, claimed, far, rng)
    })
}
