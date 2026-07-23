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
mod scoring;
mod capacity;
mod sections;
mod pipes;
mod locks;
mod progression;

use capacity::{
    SPADE_BUDGET, assign_hb_sprites, distribute_levels, prepare_capacities, promote_hb_slots,
    redistribute_fortresses,
};
use locks::{LockRole, place_locks, sample_lock_plan};
use pipes::VANILLA_PIPE_PAIRS;
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
        // Sample this world's progression archetype → per-fort lock roles.
        let roles = sample_lock_plan(fort_counts[wi], wi, rng);
        let built = build_world(
            wi,
            rom,
            patched_grids[wi].clone(),
            &fixed_positions[wi],
            &counts,
            &roles,
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
            // all-Safe roles (matches force_safe) rather than the world's shape.
            let safe_roles = vec![LockRole::Safe; fort_counts[wi]];
            let new_locks = place_locks(
                &built.grid,
                &built.pipe_pairs,
                start_pos,
                target_pos,
                &built.slots,
                fort_counts[wi],
                &safe_roles,
                true, // force_safe
                wi,
                rng,
            );
            if new_locks.iter().any(|l| l.secret_exit_safe) {
                worlds[wi].locks = new_locks;
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
