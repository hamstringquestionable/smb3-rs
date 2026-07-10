//! Phase 4 of the overworld pipeline: assign pool entries to map slots and write
//! every derived ROM table. Steps live in submodules (types, assign, grid, pointers, fortress_fx, metatiles, sprites);
//! this module holds the public entry point and wires the steps together.

use std::collections::{HashMap, HashSet, VecDeque};

use rand::Rng;
use rand::seq::{IndexedRandom, SliceRandom};

use crate::rom::Rom;
use crate::PiranhaMode;

use super::node_catalog::NodeKind;
use super::overworld_build::{bfs_ordered, BuildResult, BuiltWorld, OverworldData, SlotKind};
use super::overworld_helpers;
use super::pipe_helpers;
use super::rom_data::{
    self, FORTRESS_1F_OBJ_PTR, FX_MAP_COMP_IDX, FX_PATTERNS, FX_VADDR_H, FX_VADDR_L,
    MAP_COMPLETE_BITS, TILE_BONUS_GAME, TILE_PIPE, WORLDS,
};

mod types;
mod assign;
mod grid;
mod pointers;
mod fortress_fx;
mod metatiles;
mod sprites;

use types::*;
use assign::*;
use grid::*;
use pointers::*;
use fortress_fx::*;
use sprites::*;

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
) {
    let assignments = assign_pool(rom, build, data, rng, flags);

    // Compute W8 army sprite target positions before writing tiles,
    // so write_tile_grid can stamp connectivity-aware blank tiles under the sprites.
    let w8_sprite_positions = pick_w8_sprite_positions(&assignments[7], rng);
    let w8_sprite_pos_set: HashSet<(usize, usize)> =
        w8_sprite_positions.iter().map(|&(_, pos)| pos).collect();

    // Piranha plant sprite placements (piranha shuffle). Decided before the
    // tile pass for the same reason as the army sprites: the level slot under
    // a plant gets a connectivity-aware path node instead of a number tile.
    let plant_positions = pick_plant_positions(
        rom, build, data, &assignments, &w8_sprite_pos_set, flags, rng,
    );

    // Per-world sprite-covered positions for the tile pass.
    let mut sprite_masks: Vec<HashSet<(usize, usize)>> = vec![HashSet::new(); 8];
    sprite_masks[7].extend(w8_sprite_pos_set.iter().copied());
    for &(wi, pos) in &plant_positions {
        sprite_masks[wi].insert(pos);
    }

    // Cycling HB level pool for fallback pointer table entries (same interleaving).
    let hb_fallback_levels = interleave_hb_by_obj_ptr(data.catalog.unique_hammer_bro_levels(), rng);
    let mut hb_fallback_iter = hb_fallback_levels.iter().cycle().cloned();

    let mut fx_slot = 0usize;
    for (wi, wa) in assignments.iter().enumerate() {
        let built = &build.worlds[wi];
        let sprite_mask = &sprite_masks[wi];

        write_tile_grid(rom, built, wa, data, sprite_mask, rng);
        write_pointer_entries(rom, wi, built, wa, data, &mut hb_fallback_iter);
        write_fortress_fx(rom, wi, built, wa, data, &mut fx_slot);
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
    if flags.shuffle_hammer_bros {
        write_hb_sprites(rom, build, rng);
    }
    patch_fortress_fx_screen_check(rom);

    // Apply engine-side scaffolding for the per-world start ↔ airship swap.
    // No-op when the option was off (no worlds got flagged in pick_swaps).
    if data.catalog.start_airship_swapped.iter().any(|&b| b) {
        super::start_airship_swap::write_engine_scaffolding(rom, data.catalog);
    }
}

/// Feature/mode flags consumed by the writer. `cross_world` is the standard
/// mode and defaults on; feature flags default off. Construct exhaustively in
/// production so a new flag forces a conscious wire-up; in tests use
/// `WriteFlags { ..Default::default() }` so adding a flag leaves them untouched.
#[derive(Copy, Clone)]
pub(crate) struct WriteFlags {
    pub cross_world: bool,
    pub shuffle_hammer_bros: bool,
    pub piranha: PiranhaMode,
}

impl Default for WriteFlags {
    fn default() -> Self {
        Self { cross_world: true, shuffle_hammer_bros: false, piranha: PiranhaMode::Off }
    }
}
