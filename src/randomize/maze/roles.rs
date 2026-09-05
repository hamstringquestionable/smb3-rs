//! Pad roles: what a telepad is FOR, and where a world can host one.
//!
//! The per-world builder never learns that a world graph exists. It is handed
//! a count and a set of roles, and every role is a query against machinery that
//! already exists — the walk with locks open, the walk with locks closed, and
//! the difference between them.
//!
//! A role is a **request, not a contract**: not every world's terrain can host
//! every role (W1 has no island), so the placer records what it granted and the
//! granted-vs-requested distribution is a census output.

use std::collections::HashSet;

use super::super::map_walker::walk_reachable;
use super::super::overworld_build::{SlotKind, WorldState, stamp_slots};
use super::super::rom_data::{self, Grid, Pos};
use super::MazeLock;

/// What a pad is for. The role constrains where the pad TILE goes; where it
/// LANDS is a separate decision (see `graph::Knobs::foreign_landing_bias`).
///
/// **There is no `Island` role, and that is a measurement, not an oversight.**
/// The charter's headline shape was a pad landing in a region no walk reaches.
/// `maze_terrain_pools_census` says finished worlds contain **no such region**:
/// 0 island landings in 800 world-seeds, because `Connectivity` bridges every
/// island with a pipe and `HammerBroFill` then claims every reachable blank.
/// Building the role anyway would be a lever with an empty pool. What it would
/// cost, and the shape that replaces it, are in `docs/world_maze_design.md`
/// under "The island pad, and why v1 does not have one".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PadRole {
    /// Tile in the start region, reachable with zero keys. This is how a world
    /// satisfies invariant 3 when its airship path is gated — which is almost
    /// always, since the target is ungated by terrain alone only 7.8% of the
    /// time.
    Hub,
    /// Tile behind a lock — reachable, but only later. A shortcut that opens as
    /// the run progresses.
    Shortcut,
    /// No constraint. The null model's pad, kept as the baseline every other
    /// role is measured against.
    Free,
}

/// One world's terrain, sorted into the pools the roles draw from.
pub(crate) struct WorldTerrain {
    /// Pad tiles in the start region: reachable with every lock closed.
    pub hub_sites: Vec<Pos>,
    /// Pad tiles reachable only once some lock opens.
    pub gated_sites: Vec<Pos>,
    /// Pad tiles no walk reaches at all, even with every lock open. Measured
    /// empty on finished terrain; kept because it is the evidence for that,
    /// and because it must stay empty — a pad site nothing can step on is a
    /// wasted arrival id. Test-only: `maze_terrain_pools_census` is the only
    /// reader, and the placer must never draw from it.
    #[cfg(test)]
    pub island_sites: Vec<Pos>,
    /// Cells a pad may deposit the player on that no walk reaches. Same story:
    /// measured empty, kept as the assertion that it is.
    #[cfg(test)]
    pub island_landings: Vec<Pos>,
    /// Cells a pad may deposit the player on anywhere in the world.
    pub landings: Vec<Pos>,
    /// Landings the player cannot walk to until some lock opens. This is the
    /// achievable version of the island pad: the pad still takes you somewhere
    /// you could not have got to, the gate is just a lock rather than terrain.
    pub gated_landings: Vec<Pos>,
    /// Whether the world's own target is reachable from the start with every
    /// lock closed — invariant 3's first and cheapest satisfier, and measured
    /// to happen only 7.8% of the time. Test-only: the census reads it, the
    /// placer does not (it asks `start_region_escapable`, which is the property
    /// that actually matters).
    #[cfg(test)]
    pub target_ungated: bool,
}

impl WorldTerrain {
    /// Every pad tile the world can host, whatever its role.
    pub(crate) fn all_sites(&self) -> Vec<Pos> {
        let mut out = self.hub_sites.clone();
        out.extend(&self.gated_sites);
        out
    }

    /// Which role a tile actually satisfies, for the granted-vs-requested
    /// census. Island is decided by the landing, not the tile, so it is not
    /// answerable here.
    pub(crate) fn role_of_site(&self, pos: Pos) -> PadRole {
        if self.hub_sites.contains(&pos) {
            PadRole::Hub
        } else if self.gated_sites.contains(&pos) {
            PadRole::Shortcut
        } else {
            PadRole::Free
        }
    }

    /// The pool a role draws its tile from, or `None` when the terrain cannot
    /// grant it. A role is a request, not a contract.
    pub(crate) fn sites_for(&self, role: PadRole) -> &[Pos] {
        match role {
            PadRole::Hub => &self.hub_sites,
            PadRole::Shortcut => &self.gated_sites,
            PadRole::Free => &self.hub_sites,
        }
    }
}

/// Sort one finished world into its role pools.
///
/// Three walks decide everything: with locks CLOSED (the start region), with
/// locks OPEN (everything the world can offer on its own), and the difference.
/// A cell in neither is an island — no key opens the way there, so only a pad
/// can.
pub(crate) fn classify(
    w: &WorldState,
    locks: &[MazeLock],
    reserved: &HashSet<Pos>,
) -> WorldTerrain {
    let mut base = w.grid.clone();
    stamp_slots(&mut base, &w.slots);

    let mut closed = base.clone();
    let mut open = base.clone();
    for lock in locks.iter().filter(|l| l.world == w.world_idx) {
        closed.set(lock.pos.0, lock.pos.1, lock.gap_tile);
        open.set(lock.pos.0, lock.pos.1, lock.replace_tile);
    }
    let sealed = walk_reachable(&closed, &w.pipe_pairs, w.start, w.world_idx);
    let unsealed = walk_reachable(&open, &w.pipe_pairs, w.start, w.world_idx);

    let sites = pad_sites(w, &base, reserved);
    let mut hub_sites = Vec::new();
    let mut gated_sites = Vec::new();
    #[cfg(test)]
    let mut island_sites = Vec::new();
    for pos in sites {
        if sealed.contains(pos) {
            hub_sites.push(pos);
        } else if unsealed.contains(pos) {
            gated_sites.push(pos);
        } else {
            // Unreachable by any walk. Measured empty on every finished world,
            // and the placer must never take one — a pad tile nothing can step
            // on spends an arrival id for nothing.
            #[cfg(test)]
            island_sites.push(pos);
        }
    }

    let landings = landing_candidates(w);
    #[cfg(test)]
    let island_landings: Vec<Pos> =
        landings.iter().copied().filter(|&p| !unsealed.contains(p)).collect();
    let gated_landings: Vec<Pos> =
        landings.iter().copied().filter(|&p| !sealed.contains(p) && unsealed.contains(p)).collect();

    WorldTerrain {
        hub_sites,
        gated_sites,
        #[cfg(test)]
        island_sites,
        #[cfg(test)]
        island_landings,
        landings,
        gated_landings,
        #[cfg(test)]
        target_ungated: w.target.is_some_and(|t| sealed.contains(t)),
    }
}

/// Where a pad TILE may go in a finished world.
///
/// Two pools. **HammerBro slots** are the leftover blank pool by construction —
/// `HammerBroFill` claims every reachable blank the other phases did not — so a
/// pad there is "an ordinary walkable cell holding the least valuable content".
/// **Blanks the fill never claimed** are the cells the world's own walk does
/// not reach, which is where island pads come from.
///
/// A pad tile takes no completion bit — the hardware playtest confirmed a pad
/// on a spade panel stays a spade panel, because diverting at enter time means
/// `MO_DoLevelClear` never runs — so the row-7/8 rule does not apply to it.
pub(crate) fn pad_sites(w: &WorldState, stamped: &Grid, reserved: &HashSet<Pos>) -> Vec<Pos> {
    let taken: HashSet<Pos> = w.slots.iter().map(|s| s.pos).collect();
    // A pad tile is a spade panel, which is in `Map_Completable_Tiles`. The
    // engine never marks it (the hardware playtest settled that: diverting at
    // enter time means `MO_DoLevelClear` never runs) — but the map RELOAD does
    // not know that. Rows 7 and 8 share one completion bit and the reload reads
    // row 7 first, so a pad stamped at (7,c) swallows the bit that content at
    // (8,c) needs, and that content would never show beaten. Same rule, same
    // reason, as every other placement: see `WorldState::row78_barred`.
    let barred = w.row78_barred();
    let mut out: Vec<Pos> = w
        .slots
        .iter()
        .filter(|s| s.kind == SlotKind::HammerBro)
        .map(|s| s.pos)
        .filter(|p| !barred.contains(p) && !reserved.contains(p))
        .collect();
    for r in 0..stamped.rows() {
        for c in 0..stamped.cols {
            let pos = (r, c);
            if rom_data::VALID_BLANK_TILES.contains(&stamped.get(r, c))
                && !taken.contains(&pos)
                && !w.fixed.contains(&pos)
                && !barred.contains(&pos)
                && !reserved.contains(&pos)
                && Some(pos) != w.start
                && Some(pos) != w.target
            {
                out.push(pos);
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Where a pad may DEPOSIT the player: any placed slot, or the world's start
/// tile. All of these sit on the node lattice, which the 2-tile movement model
/// requires — an arrival on a path tile would leave the player off-grid.
pub(crate) fn landing_candidates(w: &WorldState) -> Vec<Pos> {
    let mut out: Vec<Pos> = w.slots.iter().map(|s| s.pos).collect();
    out.extend(w.start);
    out.sort_unstable();
    out.dedup();
    out
}
