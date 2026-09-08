//! Phase 4 of the overworld pipeline: assign pool entries to map slots and write
//! every derived ROM table. Steps live in submodules (types, assign, grid, pointers, fortress_fx, metatiles, sprites);
//! this module holds the public entry point and wires the steps together.

use std::collections::{HashMap, HashSet, VecDeque};

use rand::Rng;
use rand::seq::{IndexedRandom, SliceRandom};

use crate::rom::Rom;
use crate::{DejaVuMode, PiranhaMode};

use super::lock_keys::LockEntry;
use super::node_catalog::NodeKind;
use super::overworld_build::{
    BuildResult, BuiltWorld, LockHint, OverworldData, SlotKind, VANILLA_LEVEL_COUNT, bfs_ordered,
};
use super::pipe_helpers;
use super::rom_data::{self, FORTRESS_1F_OBJ_PTR, TILE_BONUS_GAME, TILE_PIPE, WORLDS};

mod assign;
mod fortress_fx;
mod grid;
mod march_veto;
mod metatiles;
mod pointers;
mod sprites;
mod types;

use assign::{assign_pool, interleave_hb_by_obj_ptr};
use fortress_fx::collect_lock_entries;
use grid::write_tile_grid;
use pointers::{write_pipe_dests, write_pointer_entries};
use sprites::{
    pick_plant_positions, pick_w8_sprite_positions, write_hb_sprites, write_plant_sprites,
    write_w8_sprites,
};

use types::{Assignment, HammerBroAssignment, PipeAssignment, WorldAssignments};

// Public API consumed by the randomizer.
pub(crate) use metatiles::{patch_double_digit_metatiles, patch_metatile_6a_freeze};

#[cfg(test)]
mod tests;

/// Execute Phase 4: assign pool entries to slots and write all ROM data.
pub(crate) fn write_overworld<R: Rng>(
    rom: &mut Rom,
    build: &BuildResult,
    data: &OverworldData,
    rng: &mut R,
    flags: WriteFlags,
) -> LockPairing {
    let assignments = assign_pool(rom, build, data, rng, flags);

    // Compute W8 army sprite target positions before writing tiles,
    // so write_tile_grid can stamp connectivity-aware blank tiles under the sprites.
    let w8_sprite_positions = pick_w8_sprite_positions(&assignments[7], rng);
    let w8_sprite_pos_set: HashSet<(usize, usize)> =
        w8_sprite_positions.iter().map(|&(_, pos)| pos).collect();

    // Piranha plant sprite placements (piranha shuffle). Decided before the
    // tile pass for the same reason as the army sprites: the level slot under
    // a plant gets a connectivity-aware path node instead of a number tile.
    let plant_positions =
        pick_plant_positions(rom, build, data, &assignments, &w8_sprite_pos_set, flags, rng);

    // Per-world sprite-covered positions for the tile pass.
    let mut sprite_masks: Vec<HashSet<(usize, usize)>> = vec![HashSet::new(); 8];
    sprite_masks[7].extend(w8_sprite_pos_set.iter().copied());
    for &(wi, pos) in &plant_positions {
        sprite_masks[wi].insert(pos);
    }

    // Cycling HB level pool for fallback pointer table entries (same interleaving).
    let hb_fallback_levels = interleave_hb_by_obj_ptr(data.catalog.unique_hammer_bro_levels(), rng);
    let mut hb_fallback_iter = hb_fallback_levels.iter().cycle().cloned();

    for (wi, wa) in assignments.iter().enumerate() {
        let built = &build.worlds[wi];
        let sprite_mask = &sprite_masks[wi];

        write_tile_grid(rom, built, wa, data, sprite_mask, flags.hints, rng);
        write_pointer_entries(rom, wi, built, wa, data, &mut hb_fallback_iter);
        write_pipe_dests(rom, wi, wa);
        // For swapped worlds, rewrite the Airship + Start entry coordinates
        // (the main writer pass leaves both untouched) before the resort so
        // the engine's runtime lookup finds the right entry per tile.
        super::start_airship_swap::write_swapped_world_entries(rom, wi, data.catalog);
        pipe_helpers::resort_pointer_table(rom, wi);
        // Do not sync map object sprite positions: the overworld builder never
        // moves MapObject entries (W7 piranhas), so vanilla sprite positions are
        // correct.  The sync function uses fixed indices that become invalid
        // after resort_pointer_table, causing sprites to jump to wrong tiles.
    }

    write_w8_sprites(rom, &w8_sprite_positions);
    // Plants claim their map-object slots before the HB writer so its slot
    // eligibility scan sees them as occupied.
    write_plant_sprites(rom, &plant_positions);
    // Keep wandering map objects (Hammer Bros) off plant/army nodes — a bro
    // parked on one would replay the level after it's beaten. Also vetoes
    // hand-trap landings (subsumes the former bros_no_hands patch).
    // Sub-tagged: it claims free space, so the write log has to name it for
    // the free-space audit and collision reports.
    rom.push_tag("march_veto");
    march_veto::write_march_veto(rom, &w8_sprite_positions, &plant_positions);
    rom.pop_tag();
    if flags.shuffle_hammer_bros {
        write_hb_sprites(rom, build, rng);
    }
    // Apply engine-side scaffolding for the per-world start ↔ airship swap.
    // No-op when the option was off (no worlds got flagged in pick_swaps).
    if data.catalog.start_airship_swapped.iter().any(|&b| b) {
        super::start_airship_swap::write_engine_scaffolding(rom, data.catalog);
    }

    LockPairing { assignments }
}

/// A handle that answers "which fortress opens which lock" after the fact.
///
/// **Why the answer is not simply written during the pass.** The world maze
/// replaces this pairing wholesale, and it can only do so after
/// [`write_overworld`] has finished: the maze's fill reads the grids this very
/// pass lays down, so the decision cannot be made earlier without duplicating
/// the writer's sprite-mask logic. Holding the assignments and producing the
/// pairing on demand keeps that late decision cheap, and it takes no RNG, so
/// nothing downstream shifts.
///
/// The pairing is *input* to [`super::lock_keys::apply`], which owns every byte
/// the console reads. Nothing here writes ROM.
pub(crate) struct LockPairing {
    assignments: Vec<WorldAssignments>,
}

impl LockPairing {
    /// Every `(fortress, lock)` pair the build placed, one per lock.
    ///
    /// Injective in both directions: a lock names one fortress section, and no
    /// two locks in a world share a section. In maze mode this is only the
    /// *starting* assignment — `maze::fill` permutes it and
    /// `maze::writer::lock_keys` emits the result instead of this.
    ///
    /// There is deliberately no way to ask this where 1-F ended up. The maze
    /// used to need that, because it ran after the writer and had to protect
    /// the pairing the writer had already committed. It now runs *before* the
    /// writer and hands down `BuiltWorld::secret_exit_slots` instead, so which
    /// safe slot 1-F takes is the writer's business alone.
    pub(crate) fn lock_entries(&self, build: &BuildResult) -> Vec<LockEntry> {
        let mut out = Vec::new();
        for (wi, wa) in self.assignments.iter().enumerate() {
            collect_lock_entries(wi, &build.worlds[wi], wa, &mut out);
        }
        out
    }
}

/// Feature flags consumed by the writer, all defaulting off. Construct
/// exhaustively in production so a new flag forces a conscious wire-up; in
/// tests use `WriteFlags { ..Default::default() }` so adding a flag leaves
/// them untouched.
#[derive(Copy, Clone, Default)]
pub(crate) struct WriteFlags {
    pub shuffle_hammer_bros: bool,
    pub piranha: PiranhaMode,
    /// Deja Vu: how many times one level may appear on the map.
    pub deja_vu: DejaVuMode,
    /// Deja Vu, fortress half: redeal the fortress deck in the same mode.
    /// Read only when `deja_vu` is on.
    pub deja_vu_forts: bool,
    /// Friendlier Levels: drop `FRIENDLIER_BLOCKED_LEVELS` from the level deck
    /// and `FRIENDLIER_BLOCKED_FORTS` from the fortress deck, refilling both
    /// with duplicates of what remains.
    pub friendlier_levels: bool,
    /// Map hints: honour `SlotAssignment::lock_hint` when picking a fortress
    /// tile, instead of choosing among them for variety.
    pub hints: bool,
}
