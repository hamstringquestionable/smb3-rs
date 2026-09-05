//! The world-graph pass: the spine, the pad budget, and which world gets which
//! pad role.
//!
//! This is the layer that knows a world graph exists. Everything below it — the
//! per-world builder, the role pools in [`roles`](super::roles) — is handed a
//! count and a set of roles and never learns why.
//!
//! ## The allocation order, and why
//!
//! The 16 arrival ids are spent in strict priority order, because the first
//! claim is a safety obligation and the rest are flavour:
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

use super::roles::{PadRole, WorldTerrain, classify};
use super::{GlobalState, MazeEdge};

/// Global cap on telepads. Each pad TILE owns one arrival row, and there are
/// `PORTAL_MAX` rows; a two-way link is therefore two pads and two ids.
pub(crate) const PAD_BUDGET: usize = super::super::world_persist::PORTAL_MAX;

/// Hard cap on pads in one world. Above this a world stops being a place and
/// starts being a switchboard.
pub(crate) const PADS_PER_WORLD_MAX: usize = 3;

/// The generator's dials. Every one of these has to earn its keep against the
/// null-model baselines before it stays — see `docs/world_maze_design.md`,
/// "The null-model baselines".
#[derive(Clone, Copy, Debug)]
pub(crate) struct Knobs {
    /// How often a pad lands in a world other than its own. 0.0 makes every
    /// pad a same-world hop (position keying gives those for free); 1.0 makes
    /// every pad a crossing. **This is the maze knob** — it is what decides
    /// whether the eight worlds are a graph or eight rooms with local
    /// shortcuts.
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
// Reason: production consumes only `edge`. The three alongside it are the
// granted-vs-requested distribution the charter asks for as a census output,
// and the census is their only reader.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct PlacedPad {
    pub edge: MazeEdge,
    pub requested: PadRole,
    pub granted: PadRole,
    /// True when the landing is in a different world from the tile.
    pub foreign: bool,
}

/// Run the world-graph pass over a maze that has no pads yet.
///
/// Returns the pads in the order they were claimed, so the caller can see the
/// budget being spent on safety before flavour.
pub(crate) fn plan_pads<R: Rng>(state: &GlobalState, knobs: &Knobs, rng: &mut R) -> Vec<PlacedPad> {
    let terrain: Vec<WorldTerrain> = state
        .worlds
        .iter()
        .map(|w| classify(w, &state.locks, &state.reserved_in(w.world_idx)))
        .collect();

    let mut out: Vec<PlacedPad> = Vec::new();
    let mut budget = PAD_BUDGET;
    let mut per_world = [0usize; 8];
    let mut used_tiles: Vec<(usize, super::super::rom_data::Pos)> = Vec::new();

    // 1. Safety first. A world whose start region cannot be escaped with what
    //    the player carries on arrival gets a hub pad before anything else
    //    claims an id.
    let mut needy: Vec<usize> =
        (0..state.worlds.len()).filter(|&wi| !state.start_region_escapable(wi)).collect();
    needy.shuffle(rng);
    for wi in needy {
        if budget == 0 {
            break;
        }
        if let Some(pad) = place_one(state, &terrain, wi, PadRole::Hub, knobs, &mut used_tiles, rng)
        {
            out.push(pad);
            per_world[wi] += 1;
            budget -= 1;
        }
    }

    // 2 and 3. The rest of the budget, dealt round-robin over a shuffled world
    //    order so no world is systematically starved. Shortcut is asked for
    //    first and falls back to Free, which is what "a request, not a
    //    contract" means in code.
    let mut order: Vec<usize> = (0..state.worlds.len()).collect();
    order.shuffle(rng);
    let mut want: Vec<usize> =
        order.iter().map(|&wi| roll_count(rng).saturating_sub(per_world[wi])).collect();
    while budget > 0 && want.iter().any(|&n| n > 0) {
        let mut spent_this_round = false;
        for (slot, &wi) in order.iter().enumerate() {
            if budget == 0 || want[slot] == 0 || per_world[wi] >= PADS_PER_WORLD_MAX {
                continue;
            }
            let role = if rng.random_bool(0.5) { PadRole::Shortcut } else { PadRole::Free };
            if let Some(pad) = place_one(state, &terrain, wi, role, knobs, &mut used_tiles, rng) {
                out.push(pad);
                per_world[wi] += 1;
                budget -= 1;
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

/// Place one pad in `world` for `role`, or `None` when the terrain cannot host
/// it. Falls back from the role's own pool to any free site, and records which
/// role the chosen tile actually satisfies.
fn place_one<R: Rng>(
    state: &GlobalState,
    terrain: &[WorldTerrain],
    world: usize,
    role: PadRole,
    knobs: &Knobs,
    used: &mut Vec<(usize, super::super::rom_data::Pos)>,
    rng: &mut R,
) -> Option<PlacedPad> {
    let t = &terrain[world];
    let free = |sites: &[super::super::rom_data::Pos]| -> Vec<super::super::rom_data::Pos> {
        sites.iter().copied().filter(|&p| !used.contains(&(world, p))).collect()
    };
    let mut pool = free(t.sites_for(role));
    if pool.is_empty() {
        pool = free(&t.all_sites());
    }
    let &from = pool.choose(rng)?;

    // Where it lands. The one knob that decides whether the eight worlds are a
    // graph: a same-world hop is a shortcut, a crossing is an edge.
    let cross = rng.random_bool(knobs.foreign_landing_bias.clamp(0.0, 1.0));
    let dest_world = if cross {
        let others: Vec<usize> = (0..state.worlds.len()).filter(|&o| o != world).collect();
        *others.choose(rng)?
    } else {
        world
    };
    // A gated landing is the achievable version of the charter's island pad:
    // the pad still takes you somewhere you could not have walked to, the gate
    // is a lock rather than terrain.
    let dt = &terrain[dest_world];
    let landing_pool = if dt.gated_landings.is_empty() { &dt.landings } else { &dt.gated_landings };
    let &to = landing_pool.choose(rng)?;

    used.push((world, from));
    Some(PlacedPad {
        edge: MazeEdge::Pad { from: (world, from), to: (dest_world, to) },
        requested: role,
        granted: t.role_of_site(from),
        foreign: dest_world != world,
    })
}

/// Hub pads for worlds the fill left unable to escape their own start region.
///
/// Separate from [`plan_pads`] because it runs after the key assignment, on
/// whatever budget survived it. A world that gets nothing here is reported, not
/// silently shipped.
pub(crate) fn rescue_pads<R: Rng>(
    state: &GlobalState,
    worlds: &[usize],
    knobs: &Knobs,
    rng: &mut R,
) -> Vec<PlacedPad> {
    let terrain: Vec<WorldTerrain> = state
        .worlds
        .iter()
        .map(|w| classify(w, &state.locks, &state.reserved_in(w.world_idx)))
        .collect();
    let mut used: Vec<(usize, super::super::rom_data::Pos)> =
        state.pad_edges().iter().map(|&(from, _)| from).collect();
    let mut budget = PAD_BUDGET.saturating_sub(state.pad_edges().len());

    let mut out = Vec::new();
    for &wi in worlds {
        if budget == 0 {
            break;
        }
        if let Some(pad) = place_one(state, &terrain, wi, PadRole::Hub, knobs, &mut used, rng) {
            out.push(pad);
            budget -= 1;
        }
    }
    out
}
