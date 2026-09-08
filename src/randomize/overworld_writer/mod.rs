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
use super::rom_data::{self, FORTRESS_1F_OBJ_PTR, Grid, TILE_BONUS_GAME, TILE_PIPE, WORLDS};

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
) -> WrittenOverworld {
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

    let mut grids: Vec<Grid> = Vec::with_capacity(8);
    for (wi, wa) in assignments.iter().enumerate() {
        let built = &build.worlds[wi];
        let sprite_mask = &sprite_masks[wi];

        grids.push(write_tile_grid(rom, built, wa, data, sprite_mask, flags.hints, rng));
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

    WrittenOverworld { grids }
}

/// **Every `(fortress, lock)` pair the build placed, one per lock.**
///
/// A fact about the *build*, not about the writing, which is why it takes no
/// `WrittenOverworld`: the pairing is decided before the writer runs, and in
/// the world maze `maze::stamp_into` has already rewritten it — possibly across
/// worlds. It is *input* to [`super::lock_keys::apply`], which owns every byte
/// the console reads; nothing here writes ROM.
pub(crate) fn lock_entries(build: &BuildResult) -> Vec<LockEntry> {
    collect_lock_entries(build)
}

/// Do the writer's grids still describe the bytes in the ROM?
///
/// The whole risk of handing the map over rather than re-reading it. Names the
/// first disagreeing cell, because "they differ" is not a debuggable message.
fn grids_agree_with_rom(grids: &[Grid], rom: &Rom) -> Result<(), String> {
    for (wi, grid) in grids.iter().enumerate() {
        for r in 0..grid.rows() {
            for c in 0..grid.cols {
                let on_rom = rom.read_byte(rom_data::map_tile_offset(wi, r, c));
                if grid.get(r, c) != on_rom {
                    return Err(format!(
                        "the writer's map and the ROM disagree at W{} ({r},{c}): the writer says \
                         {:#04X}, the ROM says {on_rom:#04X}. Something wrote a map tile straight \
                         to the ROM after `write_overworld` — put it in the build instead, or the \
                         packed completion store will be sized from a map that no longer exists.",
                        wi + 1,
                        grid.get(r, c),
                    ));
                }
            }
        }
    }
    Ok(())
}

/// What the writer wrote: the map it committed, and the slot assignments behind
/// it.
///
/// This is the writer's record of the ROM it produced, and it is the answer to
/// "what does the finished map look like" for everything downstream. Nothing
/// here writes ROM; the pairing is *input* to [`super::lock_keys::apply`],
/// which owns every byte the console reads.
///
/// It was called `LockPairing` when locks were its only consumer.
pub(crate) struct WrittenOverworld {
    /// **The map as committed, world by world.**
    ///
    /// The writer is the last thing that touches a map grid, so this is the
    /// finished article — what the console will read out of PRG012.
    ///
    /// It exists because three modules need it and none of them were given it:
    /// `completion_bits` counts the cells that can be marked done, to size each
    /// world's slice of the packed store; `world_travel` finds each world's
    /// START cell; `lock_keys` looks up an away lock's completion bit. All
    /// three used to read it back out of the ROM a cell at a time — which
    /// worked, but made "run after every grid write" an unwritten rule whose
    /// violation is silent (the slices shift, and completion marks land in the
    /// wrong world). Handing the map over makes that rule a signature instead.
    ///
    /// `grids_match_the_rom` is the guard on the other half of the trade: two
    /// copies of the map now exist, and they must not drift.
    grids: Vec<Grid>,
}

impl WrittenOverworld {
    /// The finished map for each world, in world order, checked against `rom`.
    ///
    /// Handing the map around means two copies of it now exist, and the failure
    /// mode if they drift is silent — a world's packed-store slice comes out the
    /// wrong size and completion marks land in the neighbouring world. Reading
    /// the ROM back could not drift; this check is what makes not doing that a
    /// fair trade. It costs one grid comparison in debug builds and nothing in
    /// release, and `grids_match_the_rom` runs the same check over a full
    /// randomize.
    ///
    /// Every caller already holds the ROM, so there is deliberately no
    /// unchecked accessor to reach for.
    pub(crate) fn grids(&self, rom: &Rom) -> &[Grid] {
        debug_assert!(
            grids_agree_with_rom(&self.grids, rom).is_ok(),
            "{}",
            grids_agree_with_rom(&self.grids, rom).unwrap_err()
        );
        &self.grids
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
