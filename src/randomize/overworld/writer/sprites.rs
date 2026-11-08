//! Which map-object sprites go where: World 8's army, piranha plants, and
//! the Hammer Bros. Decisions are made here; the slots themselves are
//! `overworld::sprite_slots`, which writes them.

use super::*;
use crate::randomize::overworld::map_source::WorldMap;
use crate::randomize::overworld::sprite_slots::{SpriteKind, WorldSprites};

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
///
/// The tank never lands on Wandom's pyramid: the cell under a sprite is
/// blanked to a path node, which would erase the one tile that says "beat me
/// to learn K". Seeds without a pyramid filter nothing, so their shuffle is
/// unchanged.
pub(super) fn pick_w8_sprite_positions<R: Rng>(
    wa_w8: &WorldAssignments,
    built_w8: &BuiltWorld,
    rng: &mut R,
) -> Vec<(usize, (usize, usize))> {
    let mut fort_positions: Vec<(usize, usize)> = wa_w8.fortress.iter().map(|a| a.pos).collect();
    fort_positions.as_mut_slice().shuffle(rng);

    let is_pyramid =
        |pos: (usize, usize)| built_w8.slots.iter().any(|s| s.pos == pos && s.is_pyramid);
    let mut level_positions: Vec<(usize, usize)> =
        wa_w8.level.iter().map(|a| a.pos).filter(|&pos| !is_pyramid(pos)).collect();
    level_positions.as_mut_slice().shuffle(rng);

    let mut fort_iter = fort_positions.into_iter();
    let mut level_iter = level_positions.into_iter();

    let mut result = Vec::new();
    for &(sprite_slot, is_fortress) in W8_ARMY_SPRITES {
        let pos = if is_fortress { fort_iter.next() } else { level_iter.next() };
        if let Some(p) = pos {
            result.push((sprite_slot, p));
        }
    }
    result
}

/// Move World 8's army sprites to their picked cells.
pub(super) fn move_w8_sprites(w8: &mut WorldSprites, positions: &[(usize, (usize, usize))]) {
    for &(sprite_slot, pos) in positions {
        w8.move_to(sprite_slot, pos);
    }
}

/// True when the slot at `pos` carries a special tile (hand trap, live troll
/// pipe, Wandom's pyramid) that a plant sprite must not cover — the plant's
/// auto-enter would clobber the special entry behavior, and the cell under a
/// plant is blanked to a path node, which would erase the pyramid.
fn is_special_level_slot(built: &BuiltWorld, wa: &WorldAssignments, pos: (usize, usize)) -> bool {
    built.slots.iter().any(|s| {
        s.pos == pos
            && (s.is_hand_trap
                || s.is_pyramid
                || (s.is_troll_pipe && !wa.demoted_troll_pipes.contains(&pos)))
    })
}

/// How many plant sprites this world can still host: empty-able map-object
/// slots minus the Hammer Bros that will occupy some and the slots kept free
/// for runtime bonus spawns (white toad house, coin ship).
fn plant_budget(built: &BuiltWorld, map: &WorldMap, shuffle_hammer_bros: bool) -> usize {
    let hb_used = if shuffle_hammer_bros { built.hb_sprites.len() } else { map.hb_sprites.len() };
    built.sprites.room().saturating_sub(hb_used)
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
        .map(|wi| {
            plant_budget(&build.worlds[wi], &data.catalog.maps[wi], flags.shuffle_hammer_bros)
        })
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
            let plant_pool: HashSet<usize> = data
                .pickup
                .pool
                .iter()
                .enumerate()
                .filter(|(_, pe)| {
                    rom_data::MAP_OBJ_ENTRY_LINKS
                        .iter()
                        .any(|&(w, _, e)| pe.world_idx == w && pe.entry_idx == e)
                })
                .map(|(pi, _)| pi)
                .collect();
            for (wi, wa) in assignments.iter().enumerate() {
                for a in &wa.level {
                    if plant_pool.contains(&a.pool_idx) && hostable(wi, a.pos) && budgets[wi] > 0 {
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
                let candidates: Vec<(usize, usize)> =
                    wa.level.iter().map(|a| a.pos).filter(|&pos| hostable(wi, pos)).collect();
                if let Some(&pos) = candidates.choose(rng) {
                    plants.push((wi, pos));
                }
            }
        }
        PiranhaMode::Off => unreachable!(),
    }

    plants
}

/// Put the piranha plant sprites in their worlds' slots: the highest empty slot
/// per plant (the low slots stay for the Hammer Bros, which fill from the
/// bottom), with no reward — the plant levels reward through their own chest
/// (`piranha_rooms`). Before [`deal_hb_sprites`], so the bros see these slots
/// taken.
pub(super) fn place_plants(sprites: &mut [WorldSprites], plants: &[(usize, (usize, usize))]) {
    for &(wi, pos) in plants {
        // The budget said yes, so a slot exists; skipping is the safe miss.
        let _ = sprites[wi].place_high(
            crate::randomize::levels::piranha_rooms::PLANT_SPRITE_ID,
            pos,
            0,
        );
    }
}

/// Deal the Hammer Bros decided in the build phase: every vanilla bro leaves
/// its slot first, then each world's chosen sprites take its eligible slots
/// from the bottom, with a random type id and the reward picked up from the
/// vanilla encounters.
pub(super) fn deal_hb_sprites<R: Rng>(
    sprites: &mut [WorldSprites],
    build: &BuildResult,
    rng: &mut R,
) {
    for world in sprites.iter_mut() {
        world.clear(SpriteKind::HammerBro);
    }
    for (world, built) in sprites.iter_mut().zip(&build.worlds) {
        let slots = world.hb_eligible();
        for (sprite, &slot) in built.hb_sprites.iter().zip(slots.iter()) {
            let id = *HB_SPRITE_IDS.choose(rng).unwrap();
            world.set(slot, id, sprite.grid_pos, sprite.reward);
        }
    }
}
