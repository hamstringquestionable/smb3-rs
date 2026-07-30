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
    BuiltWorld, DEFAULT_SLACK, LockAssignment, RouteChoice, SlotAssignment, SlotKind,
    VANILLA_PIPE_PAIRS, analyze_route_choice, fixed_positions_for_world, stamp_slots,
};
use super::overworld_helpers::{LOCKABLE_TILES, find_target, gap_tile_for};
use super::overworld_pickup::PickupResult;
use super::rom_data::{self, Grid, Pos, TILE_PIPE, TeleportEdge};

mod connectivity;
mod forts;
mod levels;
mod locks;
mod metrics;
mod sources;
mod spare_pipes;
mod state;

#[cfg(test)]
mod tests;

pub(crate) use connectivity::Connectivity;
pub(crate) use forts::Forts;
pub(crate) use levels::Levels;
pub(crate) use locks::Locks;
pub(crate) use metrics::measure_world;
pub(crate) use spare_pipes::SparePipes;
pub(crate) use sources::{from_built, from_pickup, from_vanilla};
pub(crate) use state::{Phase, PhaseReport, WorldState, row78_partner, run_schedule};
