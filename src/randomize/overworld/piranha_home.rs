//! Piranha Shuffle **Off** on generated maps: the two plant levels stay in
//! World 7.
//!
//! On vanilla maps Off means the plants keep their own World 7 cells. A
//! generated World 7 has none of those, so Off is played as On — the plant
//! levels join the deck and each plant sprite follows its level — with the two
//! levels **pinned** to World 7 level slots, so the deal puts them there. The
//! three settings then mean the same thing on every map: Off keeps the plants
//! in World 7, On lets them travel, Wild scatters one per world.
//!
//! The pin is honoured: nothing downstream can take a plant level out of the
//! deck. Both are `CHEST_LEVELS`, so Deja Vu deals each exactly once, and
//! neither is on Friendlier Levels' block list. A plant can still fail to get
//! its *sprite* — World 7's map-object slots are finite — in which case the
//! level is a plain numbered tile in World 7, the same as On does.

use rand::Rng;
use rand::seq::SliceRandom;

use crate::randomize::overworld::build::{BuildResult, SlotKind};
use crate::randomize::rom_data::MAP_OBJ_ENTRY_LINKS;

/// Salt for the slot pick's substream: "PIRANHA7". Its own stream, so pinning
/// the plants moves no other draw in the seed.
pub(crate) const PIRANHA_HOME_SALT: u64 = 0x5049_5241_4E48_4137;

/// World 7's index.
const HOME_WORLD: usize = 6;

/// Pin both plant levels to distinct World 7 level slots, chosen uniformly from
/// the ones that can wear a plant: a plain level slot, not a hand trap, troll
/// pipe or obelisk, and not already pinned by another pass. Returns how many
/// were pinned (2, unless World 7 has fewer eligible slots).
pub(crate) fn pin_plants_home<R: Rng>(build: &mut BuildResult, rng: &mut R) -> usize {
    let world = &mut build.worlds[HOME_WORLD];
    let mut eligible: Vec<usize> = world
        .slots
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            s.kind == SlotKind::Level
                && !s.is_hand_trap
                && !s.is_troll_pipe
                && !s.is_pyramid
                && s.pin.is_none()
        })
        .map(|(i, _)| i)
        .collect();
    eligible.shuffle(rng);
    let mut pinned = 0;
    for (&slot, &(w, _, entry)) in eligible.iter().zip(MAP_OBJ_ENTRY_LINKS) {
        world.slots[slot].pin = Some((w, entry));
        pinned += 1;
    }
    pinned
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Options, PiranhaMode};

    fn vanilla() -> Option<crate::rom::Rom> {
        let bytes = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        crate::rom::Rom::from_bytes(&bytes).ok()
    }

    /// **The feature on finished ROMs.** Generated maps with Piranha Shuffle
    /// Off: both plant levels are pinned into World 7, and every plant sprite
    /// the ROM carries stands in World 7 on a pinned cell — none anywhere
    /// else. Also reports how often World 7 ran out of sprite slots, which is
    /// the one way a pinned plant level goes without its plant.
    #[test]
    fn off_on_generated_maps_keeps_the_plants_in_world_7() {
        use crate::randomize::rom_data::read_plant_sprite_positions;
        use std::collections::HashSet;
        let Some(base) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let (mut sprites, mut seeds) = (0usize, 0usize);
        for seed in 1..=12u64 {
            for maze in [false, true] {
                let options = Options {
                    generated_overworld: true,
                    world_maze: maze,
                    piranha_shuffle: PiranhaMode::Off,
                    palettes: false,
                    ..Default::default()
                };
                let mut rom = base.clone();
                let mut build = None;
                crate::pipeline::randomize_with_overworld_capture(
                    &mut rom, seed, &options, &mut build,
                );
                let build = build.expect("captured");
                let pins: HashSet<(usize, usize)> = build.worlds[HOME_WORLD]
                    .slots
                    .iter()
                    .filter(|s| {
                        s.pin.is_some_and(|p| {
                            MAP_OBJ_ENTRY_LINKS.iter().any(|&(w, _, e)| p == (w, e))
                        })
                    })
                    .map(|s| s.pos)
                    .collect();
                assert_eq!(pins.len(), 2, "seed {seed} maze {maze}: plants not pinned into W7");
                for wi in 0..8 {
                    for pos in read_plant_sprite_positions(&rom, wi) {
                        assert_eq!(
                            wi,
                            HOME_WORLD,
                            "seed {seed} maze {maze}: a plant in W{}",
                            wi + 1
                        );
                        assert!(
                            pins.contains(&pos),
                            "seed {seed} maze {maze}: plant off a pinned cell"
                        );
                        sprites += 1;
                    }
                }
                seeds += 1;
            }
        }
        eprintln!("plant sprites placed: {sprites} of {} pinned levels", 2 * seeds);
        assert!(sprites > seeds, "most pinned plant levels should get their sprite");
    }

    /// Off on vanilla maps is untouched: the run does not pin anything.
    #[test]
    fn off_on_vanilla_maps_pins_nothing() {
        let Some(mut rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let options = Options { palettes: false, ..Default::default() };
        let mut build = None;
        crate::pipeline::randomize_with_overworld_capture(&mut rom, 1, &options, &mut build);
        let build = build.expect("captured");
        assert!(build.worlds.iter().flat_map(|w| &w.slots).all(|s| s.pin.is_none()));
    }
}
