//! W8 army sprites and Hammer-Bro map-sprite writes.

use super::*;

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
