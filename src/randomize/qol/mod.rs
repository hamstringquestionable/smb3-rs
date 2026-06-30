//! Quality-of-life patches: standalone, mostly independent ROM edits applied
//! per-option by the randomizer. Each submodule owns one cohesive feature area.

mod beta;
mod big_q;
mod canoe;
mod cards;
mod hammer_breaks;
mod macobra;
mod overworld_map;
mod starting_state;

pub use beta::fix_beta_stages;
pub use big_q::fix_big_q_block_rooms;
pub use canoe::fix_canoe_softlock;
pub use cards::card_speed_clear;
pub use hammer_breaks::hammer_breaks_tiles;
pub use macobra::{
    apply_early_sun, apply_fast_mushroom_house, apply_faster_frog, apply_faster_tail_speed,
    apply_infinite_mushroom_houses, apply_japanese_damage, apply_limit_bro_movement,
    apply_macobra_patches, apply_no_game_over_penalty,
};
pub use overworld_map::{
    apply_w8_bridges, apply_w8_canoe_and_paths, fix_w3_drawbridges, make_hammer_rocks,
    remove_n_cards, remove_rocks,
};
pub use starting_state::{set_starting_lives, write_starting_items};

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
