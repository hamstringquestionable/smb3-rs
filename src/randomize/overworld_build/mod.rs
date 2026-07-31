//! Phase 3 of the overworld pipeline: the overworld builder. Takes the
//! picked-up maps (blank path tiles) and places levels, fortresses, pipes,
//! locks, and hammer-bro fillers, producing the `BuildResult` the writer
//! stamps onto the ROM.
//!
//! ## Design charter (2026-07-30 session; the "choice-first rebuild")
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
//! - **Knob-free placement, measured judgment.** The placement phases are
//!   uniform-random; all judgment concentrates in the [`shaping`] loop and
//!   is justified by the census tables in the test suite.
//!
//! ## Pipeline
//!
//! Per world: [`Connectivity`] (bridge islands with pipe pairs) →
//! [`Levels`] → [`Forts`] → [`Locks`] → [`Shaping`] (the diagnosis-driven
//! improvement loop) → [`SparePipes`] (the full vanilla pipe budget is
//! always spent; the guard steers where), wrapped in
//! [`run_shaped_with_web_retries`] (a world that finishes below the C1
//! floor redeals its pipe web). Then across worlds: the
//! secret-exit-safety backstop, hammer-bro fill, toad-house / spade
//! promotion, and wandering-sprite redistribution.
//!
//! Module layout: [`state`] holds the core definitions (world state, phase
//! unit); [`sources`] loads worlds (pickup input, vanilla ROM reader,
//! adapter); [`metrics`] is the measuring stick every phase and census
//! reads through; [`route_choice`] is the underlying route scorer;
//! [`capacity`] owns budgets and the cross-world promotion post-passes;
//! [`progression`] is the older required-progression analyzer (censuses).

use std::collections::{HashMap, HashSet};

use rand::Rng;
use rand::RngCore;
use rand::seq::{IndexedRandom, SliceRandom};

use crate::rom::Rom;

use super::map_walker::{Reach, walk_map, walk_reachable};
use super::node_catalog::{NodeCatalog, NodeKind};
use super::overworld_helpers::{LOCKABLE_TILES, find_target, gap_tile_for};
use super::overworld_pickup::{PickupResult, blank_tile_for};
use super::rom_data::{
    self, BACKGROUND_TILES, Grid, Pos, TILE_BONUS_GAME, TILE_FORTRESS, TILE_NODE, TILE_PIPE,
    TILE_TOAD_HOUSE, TeleportEdge,
};

mod capacity;
mod connectivity;
mod forts;
mod hammer_bros;
mod levels;
mod locks;
mod metrics;
mod progression;
mod route_choice;
mod shaping;
mod sources;
mod spare_pipes;
mod state;
mod types;

#[cfg(test)]
mod builder_tests;
#[cfg(test)]
mod tests;

use capacity::{SPADE_BUDGET, assign_hb_sprites, promote_hb_slots};

// Public API consumed by the randomizer, the writer, and the feature
// modules that post-process the build (hands, troll pipes).
pub use {types::SlotAssignment, types::SlotKind};
pub(crate) use capacity::{
    RESERVED_DYNAMIC_SLOTS, VANILLA_PIPE_PAIRS, bfs_ordered, distribute_levels,
    fixed_positions_for_world, prepare_capacities, redistribute_fortresses,
};
pub(crate) use capacity::{LEVEL_SPREAD_EXPONENT, VANILLA_LEVEL_COUNT};
pub(crate) use route_choice::{C1_FLOOR, DEFAULT_SLACK, RouteChoice, SHAPING_SLACK, analyze_route_choice};
pub(crate) use types::{
    BuildFlags, BuildResult, BuiltWorld, CapacityPrep, LockAssignment, OverworldData, stamp_slots,
};

// The phase set and its harness surface.
pub(crate) use connectivity::Connectivity;
pub(crate) use forts::Forts;
pub(crate) use hammer_bros::HammerBroFill;
pub(crate) use levels::Levels;
pub(crate) use locks::{Locks, ensure_secret_exit_safe};
pub(crate) use metrics::measure_world;
pub(crate) use shaping::Shaping;
pub(crate) use sources::{allot_budgets, from_pickup};
pub(crate) use spare_pipes::SparePipes;
pub(crate) use state::{Phase, PhaseReport, WorldState, row78_partner, run_schedule};

// Test-only measurement surface: the census/probe harness in the test
// modules and the diagnostic dumps.
#[cfg(test)]
pub(crate) use progression::{
    PipeClass, analyze_required_progression, classify_pipes, dump_required_progression,
    hammer_skip, island_count, level_adjacency_pairs, start_goal_express_pipe,
};
#[cfg(test)]
pub(crate) use route_choice::dump_route_choice;
#[cfg(test)]
pub(crate) use sources::{from_built, from_vanilla};

/// Pipe-web redeals allowed beyond the first attempt when the finished
/// world ends below the C1 floor. Retries fire only on the few percent of
/// worlds that finish sub-floor, so the cost is a handful of extra shaped
/// runs per hundred seeds — census-watched, like everything else.
pub(crate) const WEB_RETRIES: usize = 4;

/// The full shaped pipeline for one world, with pipe-web redeals — the
/// answer to the sub-floor tail (2026-07-31 diagnosis). A world that
/// finishes below the C1 floor is almost always one unlucky connectivity
/// draw: a bridge mouth cheap from the start turns the goal into a 1-2 pipe
/// express, and no shaping move can price a trunk that short (the
/// SAS-swapped W4/W7 goals — a former start pocket, two tiny walk-arms, two
/// mandatory bridges — are where the draw goes bad ~1 seed in 9). No
/// placement bias and no repair move: restore the placement, pick up the
/// pipe web ONLY (levels and forts keep their spots — layout diversity is
/// the scarcest currency), and redeal connectivity + locks + shaping on a
/// fresh uniform web. Screen the outcome, not the proposal. The same
/// screen enforces the full fort roster (a removed fort is deleted
/// content). If every attempt fails, the best by (full forts, C1, routes
/// in band) is kept.
pub(crate) fn run_shaped_with_web_retries(state: &mut WorldState, rng: &mut dyn RngCore) {
    run_schedule(state, &[&Connectivity, &Levels, &Forts], rng);
    let placement = state.snapshot();
    run_schedule(state, &[&Locks, &Shaping, &SparePipes], rng);

    let mut best: Option<((bool, u32, usize), state::WorldSnapshot)> = None;
    let mut redeals = 0usize;
    loop {
        let m = measure_world(state);
        // Full fort roster is an invariant, not a preference: the Locks
        // phase removes a fort only when NO lock placement keeps the world
        // completable, and a removed fort is deleted content (its fortress
        // level leaves the game — measured 2 events per 80k worlds, both on
        // webs that also failed the floor). Such a web redeals like any
        // other degenerate web.
        let full_forts = state.fort_count() == state.fort_budget;
        if (m.c1 >= C1_FLOOR && full_forts) || redeals >= WEB_RETRIES {
            if let Some((key, snap)) = &best
                && (full_forts, m.c1, m.routes_in_band) < *key
            {
                state.restore(snap);
            }
            if redeals > 0 {
                let kept = measure_world(state);
                state.log.push(PhaseReport {
                    phase: "web_retry",
                    actions: vec![format!(
                        "{redeals} redeal(s): kept C1 {}, routes {}, forts {}/{}",
                        kept.c1,
                        kept.routes_in_band,
                        state.fort_count(),
                        state.fort_budget,
                    )],
                });
            }
            return;
        }
        let key = (full_forts, m.c1, m.routes_in_band);
        if best.as_ref().is_none_or(|(k, _)| key > *k) {
            best = Some((key, state.snapshot()));
        }
        redeals += 1;
        state.restore(&placement);
        state.pickup_pipes();
        run_schedule(state, &[&Connectivity, &Locks, &Shaping, &SparePipes], rng);
    }
}

/// Execute Phase 3: build slot assignments for all 8 worlds.
pub(crate) fn build<R: Rng>(
    rom: &Rom,
    data: &OverworldData,
    rng: &mut R,
    flags: BuildFlags,
) -> BuildResult {
    // Per-seed budgets: fortresses via redistribute_fortresses (W8 keeps 4;
    // W1-W7 roll 1-3 with the 13-fort total conserved), levels distributed
    // by compressed capacity shares.
    let (level_counts, fort_counts) = allot_budgets(rom, data.catalog, data.pickup, &flags, rng);

    let mut states: Vec<WorldState> = (0..8)
        .map(|wi| {
            let mut state = from_pickup(rom, data.catalog, data.pickup, wi, &flags);
            state.level_budget = level_counts[wi];
            state.fort_budget = fort_counts[wi];
            state
        })
        .collect();

    for state in &mut states {
        run_shaped_with_web_retries(state, rng);
    }

    // Cross-world invariant: at least one lock somewhere must be
    // secret-exit-safe — the writer parks the 1-F fortress level (whose
    // secret exit skips the lock-opening FX) on it.
    let _ = ensure_secret_exit_safe(&mut states, rng);

    // Every leftover reachable blank becomes a HammerBro slot: the pointer
    // table pool the writer fills, and the promotion stock below.
    for state in &mut states {
        run_schedule(state, &[&HammerBroFill], rng);
        renumber_fort_sections(state);
    }

    let mut worlds: Vec<BuiltWorld> = states.iter().map(|s| s.to_built()).collect();

    // Toad houses promote first so the smaller, less flexible 22-entry budget
    // lands before spades scramble for the remaining HammerBro slots.
    promote_hb_slots(
        rom, &mut worlds, data, rng,
        |k| matches!(k, NodeKind::ToadHouse), SlotKind::ToadHouse, None,
    );
    promote_hb_slots(
        rom, &mut worlds, data, rng,
        |k| matches!(k, NodeKind::BonusGame), SlotKind::BonusGame, Some(SPADE_BUDGET),
    );

    // Redistribute the wandering Hammer Bro sprites across all worlds (random
    // 1-3 per world, summing to the vanilla total). Runs last so the HammerBro
    // slots it selects from are final (Toad House / spade promotion already
    // consumed any it needed). Decided here; the writer stamps the ROM tables.
    if flags.shuffle_hammer_bros {
        assign_hb_sprites(rom, data.pickup, &mut worlds, rng);
    }

    BuildResult { worlds, fort_counts }
}

/// The writer pairs each lock to its fortress by INDEX: fort `section`
/// values must be a dense `0..section_count`. Placement assigns them
/// densely, but the Locks phase may remove an unlockable fort (rare),
/// leaving a hole — renumber in placement order and remap the locks.
fn renumber_fort_sections(state: &mut WorldState) {
    let remap: HashMap<usize, usize> = state
        .slots
        .iter()
        .filter(|s| s.kind == SlotKind::Fortress)
        .enumerate()
        .map(|(new, s)| (s.section, new))
        .collect();
    for slot in &mut state.slots {
        if slot.kind == SlotKind::Fortress {
            slot.section = remap[&slot.section];
        }
    }
    for lock in &mut state.locks {
        lock.fort_section = remap[&lock.fort_section];
    }
}
