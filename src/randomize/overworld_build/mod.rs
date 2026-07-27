//! Phase 3 of the overworld pipeline: assign levels to map slots via BFS-ordered
//! placement, distribute fortresses/pipes/locks, and enforce connectivity.
//! Steps live in submodules (types, scoring, capacity, sections, pipes, locks, progression); this module holds the
//! `build` entry point that drives them.

use std::collections::{HashMap, HashSet};

use rand::Rng;
use rand::seq::{IndexedRandom, SliceRandom};

use super::map_walker::walk_map;
use super::node_catalog::{NodeCatalog, NodeKind};
use super::overworld_helpers::{find_target, gap_tile_for, LOCKABLE_TILES};
use super::overworld_pickup::PickupResult;
use crate::rom::Rom;
use super::rom_data::{
    self, BACKGROUND_TILES, Grid, Pos, TILE_BONUS_GAME, TILE_FORTRESS, TILE_NODE, TILE_PIPE,
    TILE_TOAD_HOUSE, TeleportEdge,
    VALID_HORZ, VALID_VERT,
};

mod types;
mod plan;
mod scoring;
mod capacity;
mod sections;
mod pipes;
mod locks;
mod progression;
mod route_choice;

use capacity::{
    SPADE_BUDGET, assign_hb_sprites, distribute_levels, prepare_capacities, promote_hb_slots,
    redistribute_fortresses,
};
use locks::place_locks;
use pipes::VANILLA_PIPE_PAIRS;
use plan::WorldPlan;
use scoring::{LEVEL_SPREAD_EXPONENT, VANILLA_LEVEL_COUNT};
use sections::build_world;
use types::{CapacityPrep, WorldSlotCounts};

// Public API consumed by the randomizer and the overworld writer.
pub use {types::SlotAssignment, types::SlotKind};
pub(crate) use sections::bfs_ordered;
pub(crate) use capacity::RESERVED_DYNAMIC_SLOTS;
pub(crate) use types::{BuildFlags, BuildResult, BuiltWorld, OverworldData};
// Progression analysis is exercised only by the test suite today (reserved for a
// future WASM single-seed dump), so surface it just for `tests`.
#[cfg(test)]
pub(crate) use progression::{
    analyze_required_progression, classify_pipes, dump_required_progression, hammer_skip,
    island_count, level_adjacency_pairs, start_goal_express_pipe, PipeClass,
};
// Choice-first route scorer (separate weighted model — see route_choice.rs).
#[cfg(test)]
pub(crate) use route_choice::{analyze_route_choice, dump_route_choice};

#[cfg(test)]
mod tests;

/// Execute Phase 3: build slot assignments for all 8 worlds.
pub(crate) fn build<R: Rng>(
    rom: &Rom,
    data: &OverworldData,
    rng: &mut R,
    flags: BuildFlags,
) -> BuildResult {
    let BuildFlags {
        shuffle_toad_houses,
        eights_are_wild,
        shuffle_hammer_bros,
    } = flags;
    let pickup = data.pickup;
    let catalog = data.catalog;
    // Step 0: redistribute fortresses
    let fort_counts = redistribute_fortresses(rng);

    // Pre-compute patched grids, fixed positions, and per-world level capacity.
    // (Shared with the distribution-tuning tests so they can't drift.)
    let CapacityPrep {
        patched_grids,
        fixed_positions,
        capacities,
    } = prepare_capacities(
        rom, catalog, pickup, &fort_counts,
        eights_are_wild, shuffle_toad_houses, shuffle_hammer_bros,
    );

    // Distribute VANILLA_LEVEL_COUNT levels across worlds by compressed capacity
    // (see LEVEL_SPREAD_EXPONENT). The compression keeps the densest worlds from
    // hoarding levels, so the old W6-specific clamp is no longer needed.
    let level_counts = distribute_levels(&capacities, VANILLA_LEVEL_COUNT, LEVEL_SPREAD_EXPONENT, rng);

    // Sample every world's plan up front, then enforce the seed-level
    // invariant (>=1 Safe role somewhere) before any placement happens —
    // cheaper and shape-preserving compared to the post-build force_safe
    // retry, which remains only as a backstop.
    let mut plans: Vec<WorldPlan> = (0..8)
        .map(|wi| WorldPlan::sample(fort_counts[wi], wi, rng))
        .collect();
    plan::ensure_seed_safe_role(&mut plans, rng);

    let mut worlds = Vec::with_capacity(8);
    for wi in 0..8 {
        // Max non-pipe slots = pointer table slots minus pipe endpoints.
        // This caps the total fort+level+HB entries to what the pointer table
        // can actually hold. Excess blank tiles stay as path nodes.
        let ptr_slots = pickup.worlds[wi].pool_indices.len();
        let pipe_endpoints = VANILLA_PIPE_PAIRS[wi] * 2;
        let max_non_pipe_slots = ptr_slots.saturating_sub(pipe_endpoints);

        let counts = WorldSlotCounts {
            fort_count: fort_counts[wi],
            level_count: level_counts[wi],
            pipe_pair_count: VANILLA_PIPE_PAIRS[wi],
            max_non_pipe_slots,
            force_safe: false,
        };
        // Reroll this world for real route choice — see `build_world_best_of_k`.
        // Intrinsic to the builder, not a flag: a good overworld hands the
        // player decisions. The seed-coordinated plan is tried first.
        let built = build_world_best_of_k(
            wi,
            rom,
            &patched_grids[wi],
            &fixed_positions[wi],
            &counts,
            &plans[wi],
            shuffle_hammer_bros,
            rng,
        );
        worlds.push(built);
    }

    // Ensure at least one lock across all worlds is secret-exit-safe.
    // The score-based prefer_safe usually produces one (triggers when
    // best_score < 5), but if not, retry with force_safe=true.
    let has_safe = worlds.iter().any(|b| b.locks.iter().any(|l| l.secret_exit_safe));
    if !has_safe {
        let mut retry_order: Vec<usize> = (0..8).collect();
        retry_order.shuffle(rng);
        for &wi in &retry_order {
            let built = &worlds[wi];
            let start_pos = rom_data::find_start(&built.grid);
            let target_pos = find_target(&built.grid, wi);
            // Retry aims only to surface a secret-exit-safe lock, so ask for
            // an all-Safe plan (matches force_safe) rather than the world's shape.
            let safe_plan = WorldPlan::all_safe(fort_counts[wi]);
            let new_locks = place_locks(
                &built.grid,
                &built.pipe_pairs,
                start_pos,
                target_pos,
                &built.slots,
                fort_counts[wi],
                &safe_plan,
                true, // force_safe
                wi,
                rng,
            );
            if new_locks.iter().any(|l| l.secret_exit_safe) {
                worlds[wi].locks = new_locks;
                // Keep the stored plan truthful for diagnostics — this world
                // is now all-Safe, whatever it originally sampled.
                worlds[wi].plan = safe_plan;
                break;
            }
        }
    }

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
    if shuffle_hammer_bros {
        assign_hb_sprites(rom, data.pickup, &mut worlds, rng);
    }

    BuildResult { worlds, fort_counts }
}

/// Reroll a world for route choice: build it from the seed-coordinated plan,
/// and if that comes out linear, re-sample the plan and rebuild — up to
/// `REROLL_LIMIT` builds total — returning the first version that offers real
/// route choice (≥2 roughly-equal routes). If none do, keep the coordinated
/// build (exactly what the builder would have produced without rerolling).
/// Early-stops, so choiceful worlds cost a single build. Deterministic: rerolls
/// draw from the seeded `rng` in order.
// Reason: too_many_arguments — this is `build_world`'s own parameter list; the
// reroll wraps it without adding state, so a struct would just indirect it.
#[allow(clippy::too_many_arguments)]
fn build_world_best_of_k<R: Rng>(
    world_idx: usize,
    rom: &Rom,
    grid: &Grid,
    fixed_positions: &HashSet<Pos>,
    counts: &WorldSlotCounts,
    first_plan: &WorldPlan,
    shuffle_hammer_bros: bool,
    rng: &mut R,
) -> BuiltWorld {
    let has_choice = |w: &BuiltWorld| {
        route_choice::analyze_route_choice(w, route_choice::DEFAULT_SLACK)
            .routes
            .len()
            >= 2
    };
    let build = |plan: &WorldPlan, rng: &mut R| {
        build_world(world_idx, rom, grid.clone(), fixed_positions, counts, plan, shuffle_hammer_bros, rng)
    };

    let base = build(first_plan, rng);
    if has_choice(&base) {
        return base;
    }
    for _ in 1..route_choice::REROLL_LIMIT {
        let plan = WorldPlan::sample(counts.fort_count, world_idx, rng);
        let candidate = build(&plan, rng);
        if has_choice(&candidate) {
            return candidate;
        }
    }
    base
}
