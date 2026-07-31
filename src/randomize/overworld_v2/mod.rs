//! Overworld builder v2 — a from-scratch rebuild, developed in parallel with
//! the shipping builder (`overworld_build`), which keeps producing every real
//! seed until this one earns the job.
//!
//! ## Design charter (2026-07-30 session)
//!
//! - **Tool roles.** The lock is the shaper (a fort on the forced path makes
//!   its lock decorative — the player opens it regardless). Forts belong off
//!   the path. Pipes are a primary routing tool, not defensive filler. Level
//!   moves are a final tweak, not an early shaper.
//! - **Reorderable phases.** Every placement pass is a [`Phase`]: take the
//!   [`WorldState`], change it, report what you did. The pipeline is a plain
//!   list, so the schedule is data — reordering it (even per seed) is a
//!   caller decision, not a rewrite.
//! - **Soft preferences over hard rules.** Only true safety is hard
//!   (completability, secret-exit safety). Notable structures — an ungated
//!   shortcut, an open goal — are measured rates to be tuned rare, not banned:
//!   a rare surprise is a good gameplay moment.
//! - **Measured from day one.** [`metrics`] reads a world through the SAME
//!   route scorer the shipping builder uses, so vanilla maps (known ground
//!   truth), current-builder output (the baseline), and v2 output are all
//!   compared with one stick.
//!
//! Module layout: [`state`] holds the two core definitions (world state,
//! phase unit); [`sources`] loads worlds to measure (vanilla ROM reader,
//! current-builder adapter); [`metrics`] is the measuring stick. Placement
//! phases will arrive one at a time, each with its own module.

use std::collections::{HashMap, HashSet};

use rand::RngCore;

use crate::rom::Rom;

use super::map_walker::{Reach, walk_reachable};
use super::node_catalog::{NodeCatalog, NodeKind};
use super::overworld_build::{
    BuildFlags, BuiltWorld, C1_FLOOR, CapacityPrep, DEFAULT_SLACK, LEVEL_SPREAD_EXPONENT,
    LockAssignment, RouteChoice, SlotAssignment, SlotKind, VANILLA_LEVEL_COUNT,
    VANILLA_PIPE_PAIRS, analyze_route_choice, distribute_levels, fixed_positions_for_world,
    prepare_capacities, redistribute_fortresses, stamp_slots,
};
use super::overworld_helpers::{LOCKABLE_TILES, find_target, gap_tile_for};
use super::overworld_pickup::{PickupResult, blank_tile_for};
use super::rom_data::{self, Grid, Pos, TILE_PIPE, TeleportEdge};

mod connectivity;
mod forts;
mod levels;
mod locks;
mod metrics;
mod shaping;
mod sources;
mod spare_pipes;
mod state;

#[cfg(test)]
mod tests;

pub(crate) use connectivity::Connectivity;
pub(crate) use forts::Forts;
pub(crate) use levels::Levels;
pub(crate) use locks::{Locks, ensure_secret_exit_safe};
pub(crate) use metrics::measure_world;
pub(crate) use shaping::Shaping;
pub(crate) use spare_pipes::SparePipes;
pub(crate) use sources::{allot_budgets, from_built, from_pickup, from_vanilla};
pub(crate) use state::{Phase, PhaseReport, WorldState, row78_partner, run_schedule};

/// Pipe-web redeals allowed beyond the first attempt when the finished
/// world ends below the C1 floor. Retries fire only on the few percent of
/// worlds that finish sub-floor, so the cost is a handful of extra shaped
/// runs per hundred seeds — census-watched, like everything else.
pub(crate) const WEB_RETRIES: usize = 4;

/// The full shaped pipeline with pipe-web redeals — the answer to the
/// sub-floor tail (2026-07-31 diagnosis). A world that finishes below the
/// C1 floor is almost always one unlucky connectivity draw: a bridge mouth
/// cheap from the start turns the goal into a 1-2 pipe express, and no
/// shaping move can price a trunk that short (the SAS-swapped W4/W7 goals —
/// a former start pocket, two tiny walk-arms, two mandatory bridges — are
/// where the draw goes bad ~1 seed in 9). No placement bias and no repair
/// move: restore the placement, pick up the pipe web ONLY (levels and forts
/// keep their spots — layout diversity is the scarcest currency), and
/// redeal connectivity + locks + shaping on a fresh uniform web. Screen the
/// outcome, not the proposal. If every attempt ends sub-floor, the best by
/// (C1, routes in band) is kept.
pub(crate) fn run_shaped_with_web_retries(state: &mut WorldState, rng: &mut dyn RngCore) {
    run_schedule(state, &[&Connectivity, &Levels, &Forts], rng);
    let placement = state.snapshot();
    run_schedule(state, &[&Locks, &Shaping], rng);

    let mut best: Option<(u32, usize, state::WorldSnapshot)> = None;
    let mut redeals = 0usize;
    loop {
        let m = measure_world(state);
        if m.c1 >= C1_FLOOR || redeals >= WEB_RETRIES {
            if let Some((c1, routes, snap)) = &best
                && (m.c1, m.routes_in_band) < (*c1, *routes)
            {
                state.restore(snap);
            }
            if redeals > 0 {
                let kept = measure_world(state);
                state.log.push(PhaseReport {
                    phase: "web_retry",
                    actions: vec![format!(
                        "{redeals} redeal(s): kept C1 {}, routes {}",
                        kept.c1, kept.routes_in_band,
                    )],
                });
            }
            return;
        }
        if best
            .as_ref()
            .is_none_or(|(c1, routes, _)| (m.c1, m.routes_in_band) > (*c1, *routes))
        {
            best = Some((m.c1, m.routes_in_band, state.snapshot()));
        }
        redeals += 1;
        state.restore(&placement);
        state.pickup_pipes();
        run_schedule(state, &[&Connectivity, &Locks, &Shaping], rng);
    }
}
