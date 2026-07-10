//! W8 army sprites and Hammer-Bro map-sprite writes.

use super::*;

/// W8 army sprite definitions: (map_object_slot, is_fortress).
/// Tank goes on a level slot, the other 3 go on fortress slots.
const W8_ARMY_SPRITES: &[(usize, bool)] = &[
    (2, false), // Tank sprite (ID $0E) → level slot
    (3, true),  // Navy/Battleship sprite (ID $0D) → fortress slot
    (4, true),  // Air Force sprite (ID $0F) → fortress slot
    (5, true),  // Super Tank sprite (ID $0E) → fortress slot
];

/// Hammer Bro map-sprite type ids. Cosmetic on the overworld (the battle is the
/// node under the sprite); assigned at random per encounter.
const HB_SPRITE_IDS: [u8; 4] = [0x03, 0x04, 0x05, 0x06];

/// Decide where each W8 army sprite goes. Returns (sprite_slot, grid_pos).
pub(super) fn pick_w8_sprite_positions<R: Rng>(
    wa_w8: &WorldAssignments,
    rng: &mut R,
) -> Vec<(usize, (usize, usize))> {
    let mut fort_positions: Vec<(usize, usize)> = wa_w8
        .fortress
        .iter()
        .map(|a| a.pos)
        .collect();
    fort_positions.as_mut_slice().shuffle(rng);

    let mut level_positions: Vec<(usize, usize)> = wa_w8
        .level
        .iter()
        .map(|a| a.pos)
        .collect();
    level_positions.as_mut_slice().shuffle(rng);

    let mut fort_iter = fort_positions.into_iter();
    let mut level_iter = level_positions.into_iter();

    let mut result = Vec::new();
    for &(sprite_slot, is_fortress) in W8_ARMY_SPRITES {
        let pos = if is_fortress {
            fort_iter.next()
        } else {
            level_iter.next()
        };
        if let Some(p) = pos {
            result.push((sprite_slot, p));
        }
    }
    result
}

/// Write W8 army sprite positions to the map object tables.
pub(super) fn write_w8_sprites(rom: &mut Rom, positions: &[(usize, (usize, usize))]) {
    for &(sprite_slot, (row, col)) in positions {
        rom_data::write_map_sprite_position(rom, 7, sprite_slot, row, col);
    }
}

/// True when the slot at `pos` carries a special tile (hand trap, live troll
/// pipe) that a plant sprite must not cover — the plant's auto-enter would
/// clobber the special entry behavior.
fn is_special_level_slot(built: &BuiltWorld, wa: &WorldAssignments, pos: (usize, usize)) -> bool {
    built.slots.iter().any(|s| {
        s.pos == pos
            && (s.is_hand_trap
                || (s.is_troll_pipe && !wa.demoted_troll_pipes.contains(&pos)))
    })
}

/// How many plant sprites this world can still host: empty-able map-object
/// slots minus the Hammer Bros that will occupy some and the slots kept free
/// for runtime bonus spawns (white toad house, coin ship).
fn plant_budget(rom: &Rom, built: &BuiltWorld, wi: usize, shuffle_hammer_bros: bool) -> usize {
    let eligible = rom_data::eligible_hb_map_slots(rom, wi).len();
    let hb_used = if shuffle_hammer_bros {
        built.hb_sprites.len()
    } else {
        rom_data::read_hb_sprite_positions(rom, wi).len()
    };
    eligible.saturating_sub(hb_used + super::super::overworld_build::RESERVED_DYNAMIC_SLOTS)
}

/// Decide where piranha plant sprites go (piranha shuffle).
///
/// `On`: the two released plant levels keep their sprites — one plant on
/// each slot the pool assignment gave them, whatever world that is.
/// `Wild`: the levels travel as plain numbered levels; instead one random
/// plain level slot per world gets a plant dropped on top of it.
///
/// Best-effort in both modes: a placement is skipped when the world has no
/// spare map-object slot (after Hammer Bros + the dynamic-spawn reserve) or
/// the slot carries a special tile. Skipped plants simply leave the level as
/// a normal numbered tile.
pub(super) fn pick_plant_positions<R: Rng>(
    rom: &Rom,
    build: &BuildResult,
    data: &OverworldData,
    assignments: &[WorldAssignments],
    w8_sprite_pos_set: &HashSet<(usize, usize)>,
    flags: WriteFlags,
    rng: &mut R,
) -> Vec<(usize, (usize, usize))> {
    let mut plants = Vec::new();
    if flags.piranha == PiranhaMode::Off {
        return plants;
    }

    let mut budgets: Vec<usize> = (0..8)
        .map(|wi| plant_budget(rom, &build.worlds[wi], wi, flags.shuffle_hammer_bros))
        .collect();

    // A level slot can host a plant if it isn't a hand trap / live troll pipe
    // and isn't already covered by a W8 army sprite.
    let hostable = |wi: usize, pos: (usize, usize)| {
        let covered = is_special_level_slot(&build.worlds[wi], &assignments[wi], pos)
            || (wi == 7 && w8_sprite_pos_set.contains(&pos));
        !covered
    };

    match flags.piranha {
        PiranhaMode::On => {
            // Pool indices of the released plant levels (vanilla W7 entries
            // linked to map-object sprites).
            let plant_pool: HashSet<usize> = data.pickup.pool.iter().enumerate()
                .filter(|(_, pe)| {
                    rom_data::MAP_OBJ_ENTRY_LINKS.iter()
                        .any(|&(w, _, e)| pe.world_idx == w && pe.entry_idx == e)
                })
                .map(|(pi, _)| pi)
                .collect();
            for (wi, wa) in assignments.iter().enumerate() {
                for a in &wa.level {
                    if plant_pool.contains(&a.pool_idx)
                        && hostable(wi, a.pos)
                        && budgets[wi] > 0
                    {
                        budgets[wi] -= 1;
                        plants.push((wi, a.pos));
                    }
                }
            }
        }
        PiranhaMode::Wild => {
            for (wi, wa) in assignments.iter().enumerate() {
                if budgets[wi] == 0 {
                    continue;
                }
                let candidates: Vec<(usize, usize)> = wa.level.iter()
                    .map(|a| a.pos)
                    .filter(|&pos| hostable(wi, pos))
                    .collect();
                if let Some(&pos) = candidates.choose(rng) {
                    plants.push((wi, pos));
                }
            }
        }
        PiranhaMode::Off => unreachable!(),
    }

    plants
}

/// Write the piranha plant sprites into map-object slots. Claims the
/// highest-index empty slot per plant (leaving low slots for the Hammer-Bro
/// writer, which fills from the bottom) with no reward — the plant levels
/// reward through their own chest (`piranha_rooms`). Must run before
/// `write_hb_sprites` so HB slot eligibility sees the claimed slots.
pub(super) fn write_plant_sprites(rom: &mut Rom, plants: &[(usize, (usize, usize))]) {
    for &(wi, (row, col)) in plants {
        let Some(slot) = rom_data::last_empty_map_obj_slot(rom, wi) else {
            continue; // budget said yes but no slot — should not happen
        };
        rom_data::write_map_sprite(
            rom, wi, slot, row, col,
            super::super::piranha_rooms::PLANT_SPRITE_ID,
        );
        rom.write_byte(rom_data::map_obj_reward_offset(wi, slot), 0);
    }
}

/// Write the redistributed Hammer Bro sprites decided in the build phase. Clears
/// every vanilla HB map-object entry first (freeing its slot + reward), then
/// stamps each world's chosen sprites into eligible map-object slots with a
/// random type id and the reward picked up from the vanilla encounters.
pub(super) fn write_hb_sprites<R: Rng>(rom: &mut Rom, build: &BuildResult, rng: &mut R) {
    for wi in 0..8 {
        rom_data::clear_hb_sprites(rom, wi);
    }
    for (wi, built) in build.worlds.iter().enumerate() {
        let slots = rom_data::eligible_hb_map_slots(rom, wi);
        for (sprite, &map_slot) in built.hb_sprites.iter().zip(slots.iter()) {
            let (row, col) = sprite.grid_pos;
            let id = *HB_SPRITE_IDS.choose(rng).unwrap();
            rom_data::write_hb_sprite(rom, wi, map_slot, row, col, id, sprite.reward);
        }
    }
}
