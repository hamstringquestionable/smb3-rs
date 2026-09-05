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

/// What a pad is for. The role constrains where a pad TILE goes; which OTHER
/// pad it is paired with is a separate decision (see
/// `graph::Knobs::foreign_landing_bias`).
///
/// **There is no `Island` role, and that was a measurement, not an oversight.**
/// The charter's headline shape was a pad standing in a region no walk reaches.
/// `maze_terrain_pools_census` found finished worlds contained **no such
/// region** — 0 island sites in 800 world-seeds, because `Connectivity` bridges
/// every island with a pipe and `HammerBroFill` then claims every reachable
/// blank — so the role would have been a lever with an empty pool. What it
/// would cost, and the shape that replaces it, are in
/// `docs/world_maze_design.md` under "The island pad, and why v1 does not have
/// one".
///
/// **Re-measured 2026-09-05, the pool is no longer empty**: 128 sites in 800
/// world-seeds, mean 0.16 per world, and lopsided — 100 of them in World 3, 22
/// in World 4, 6 in World 7, none anywhere else. The census prints it. Nothing
/// draws from the pool ([`WorldTerrain::all_sites`] offers hub and gated only),
/// so this breaks nothing; it means the evidence the role was declined on has
/// moved and the decision is worth revisiting rather than assuming.
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
    /// Pad tiles no walk reaches at all, even with every lock open — the pool
    /// the charter's island pad would have drawn from. Once measured empty,
    /// now 0.16 per world and concentrated in W3/W4 (see [`PadRole`]).
    ///
    /// **The placer must never draw from it**: a pad site nothing can step on
    /// is a wasted arrival id, and half a pair that can never be used from one
    /// end. Test-only, because `maze_terrain_pools_census` is its only reader.
    #[cfg(test)]
    pub island_sites: Vec<Pos>,
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
    /// census.
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
    pub(crate) fn sites_for(&self, role: PadRole) -> Vec<Pos> {
        match role {
            PadRole::Hub => self.hub_sites.clone(),
            PadRole::Shortcut => self.gated_sites.clone(),
            // "No constraint" has to mean it. This returned `hub_sites` until
            // 2026-09-05, which made `Free` a second name for `Hub` — the
            // census showed the consequence plainly: of every pad ever placed,
            // **not one** was granted `Free`, because a hub site was always
            // available to satisfy it first. An owned `Vec` costs an allocation
            // per placement and buys a variant that does what it says.
            PadRole::Free => self.all_sites(),
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

    WorldTerrain {
        hub_sites,
        gated_sites,
        #[cfg(test)]
        island_sites,
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
/// not reach — measured empty, and kept as the evidence that they are.
///
/// The row-7/8 rule is applied here even though the pad tile no longer needs
/// it — see the comment on `barred` below.
pub(crate) fn pad_sites(w: &WorldState, stamped: &Grid, reserved: &HashSet<Pos>) -> Vec<Pos> {
    let taken: HashSet<Pos> = w.slots.iter().map(|s| s.pos).collect();
    // Rows 7 and 8 share completion bit `$01`, and the reload reads row 7
    // first: a COMPLETABLE tile at (7,c) swallows the bit that content at
    // (8,c) needs, and that content then never shows beaten (#212).
    //
    // `TILE_TELEPAD` is in neither `Map_Completable_Tiles` nor
    // `Map_Removable_Tiles`, so it claims no bit and the rule does not bind on
    // it — that is a property of the byte, not of the pad. The filter stays
    // anyway, and deliberately: it costs a handful of sites out of a pool that
    // averages dozens, and it is the only thing standing between a future tile
    // change and a silent regression that shows up as one level in one world
    // refusing to look beaten. `the_pad_tile_is_in_no_registry` is the other
    // half of that guard.
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
