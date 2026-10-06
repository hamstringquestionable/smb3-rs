//! Quality-of-life patches: standalone, mostly independent ROM edits applied
//! per-option by the randomizer. Each submodule owns one cohesive feature area.

mod beta;
pub(crate) mod big_q;
mod bro_arena;
mod bro_timer;
mod canoe;
mod canoe_summon;
mod cards;
mod hammer_breaks;
mod lakitu;
mod level_clock;
mod macobra;
mod map_warp;
mod overworld_map;
mod starting_state;
// Retires the 2-player Vs Challenge, whose trigger is unsound once the two
// players can be in different worlds and which bypassed the map's
// tile-enterability rules. Unconditional, and it frees 339 bytes of PRG030.
pub(crate) mod two_player_vs;

pub(crate) use beta::fix_beta_stages;
pub(crate) use big_q::fix_big_q_block_rooms;
pub(crate) use bro_arena::rebuild_desert_bro_arena;
pub(crate) use bro_timer::apply_bro_battle_timer;
pub(crate) use canoe::fix_canoe_softlock;
#[cfg(test)]
pub(crate) use canoe_summon::a_press_hook_installed;
pub(crate) use canoe_summon::{
    apply_canoe_summon, remove_canoe_summon_hook, write_canoe_summon_routine,
};
pub(crate) use cards::card_speed_clear;
pub(crate) use hammer_breaks::hammer_breaks_tiles;
pub(crate) use lakitu::apply_lakitu_stays_down;
pub(crate) use level_clock::apply_real_time_clock;
pub(crate) use macobra::{
    apply_early_sun, apply_fast_mushroom_house, apply_faster_frog, apply_faster_tail_speed,
    apply_fireball_hearts, apply_infinite_mushroom_houses, apply_japanese_damage,
    apply_limit_bro_movement, apply_macobra_patches, apply_mariomon, apply_modern_powerups,
    apply_no_game_over_penalty, apply_remove_flashing,
};
pub(crate) use map_warp::apply_map_warp;
pub(crate) use overworld_map::{W8_BRIDGE_COLS, W8_BRIDGE_ROW};
pub(crate) use overworld_map::{
    apply_w1_shortcut, apply_w8_bridges, apply_w8_canoe_and_paths, fix_w3_drawbridges,
    make_hammer_rocks, remove_n_cards, remove_rocks,
};
pub(crate) use starting_state::{set_starting_lives, write_starting_items};

#[cfg(test)]
pub(crate) mod test_support {
    use crate::rom::Rom;

    /// Minimal valid ROM for qol unit tests.
    pub(crate) fn make_test_rom() -> Rom {
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;
        data[0x308E1] = 0x04; // STARTING_LIVES_OFFSET
        Rom::from_bytes_lax(&data, true).unwrap()
    }
}
