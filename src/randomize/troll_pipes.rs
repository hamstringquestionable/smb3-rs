use rand::Rng;
use rand::seq::IndexedRandom;

use super::overworld_build::{BuildResult, SlotKind};

/// Probability (in percent) that an eligible world actually gets its troll
/// pipe stamped. Each world W2-W8 rolls independently, so on average about
/// `7 * 0.75 ≈ 5.25` pipes appear instead of a guaranteed 7. A pipe-free
/// early world is then ambiguous ("maybe-off, or just an unlucky roll"),
/// which keeps the Maybe secret hidden for longer. Applies in both On and
/// Maybe modes — by the time this runs the mode is already resolved to "on",
/// so there is one rule and no special case.
const TROLL_PIPE_PERCENT: u32 = 75;

/// Mark at most one regular-level slot per world W2-W8 as a troll pipe.
/// W1 is excluded so the player has at least one safe world to learn the
/// game's vanilla appearance before encountering disguised level slots.
///
/// Each eligible world independently has a [`TROLL_PIPE_PERCENT`]% chance of
/// actually receiving a pipe; on the miss the world keeps a normal level
/// tile. The roll is always drawn (even when there is no candidate slot) so
/// downstream RNG stays aligned across worlds and the output is reproducible
/// per seed.
///
/// The writer reads `slot.is_troll_pipe` and stamps `0xBC` (PIPE tile)
/// instead of a level-number tile. The slot's level pointer entry is
/// unchanged. When the player presses A on the pipe-look tile, the
/// world-map dispatch matches `0xBC` in `Map_EnterSpecialTiles` at `$CDBF`
/// and falls into the same `Map_Operation = $10` "enter level" path used
/// by every level number tile — no pipe-transit state is set up. The
/// player drops into the regular level pointed at by the slot.
///
/// Unlike `hands_levels`, troll_pipes needs **no ROM patches** beyond the
/// tile stamp itself. Validated end-to-end by `poc_troll_pipe_1_1.nes`.
pub(crate) fn mark_troll_pipes<R: Rng>(build: &mut BuildResult, rng: &mut R) {
    for built in &mut build.worlds {
        // Skip W1 (world_idx 0).
        if built.world_idx == 0 {
            continue;
        }
        let candidates: Vec<usize> = built
            .slots
            .iter()
            .enumerate()
            .filter(|(_, s)| s.kind == SlotKind::Level && !s.is_hand_trap)
            .map(|(i, _)| i)
            .collect();
        // Draw the appearance roll unconditionally so RNG advances by the
        // same amount per world regardless of whether a candidate exists.
        let appears = rng.random_range(..100u32) < TROLL_PIPE_PERCENT;
        if let Some(&pick) = candidates.choose(rng)
            && appears
        {
            built.slots[pick].is_troll_pipe = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    use crate::randomize::{node_catalog, overworld_build, overworld_pickup, troll_pipes};
    use crate::rom::Rom;

    #[test]
    fn marks_at_most_one_pipe_per_world_w2_w8() {
        let Ok(bytes) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") else {
            return; // Base ROM not present (e.g. CI) — skip.
        };
        let rom = Rom::from_bytes(&bytes).unwrap();
        const SEEDS: u64 = 256;
        let mut marked = 0usize; // W2-W8 worlds that got a pipe
        let mut eligible = 0usize; // W2-W8 worlds total
        for seed in 0u64..SEEDS {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let catalog = node_catalog::NodeCatalog::build(&rom, false);
            let pickup = overworld_pickup::pick_up(&rom, &catalog, overworld_pickup::PickupFlags { shuffle_spade_games: true, shuffle_toad_houses: true });
            let data = overworld_build::OverworldData { pickup: &pickup, catalog: &catalog };
            let mut build = overworld_build::build(&rom, &data, &mut rng, true, false);
            troll_pipes::mark_troll_pipes(&mut build, &mut rng);
            for w in &build.worlds {
                let n = w.slots.iter().filter(|s| s.is_troll_pipe).count();
                if w.world_idx == 0 {
                    assert_eq!(n, 0, "W1 should never have a troll pipe (seed {seed})");
                } else {
                    assert!(n <= 1, "W{} should have at most 1 troll pipe (seed {seed})", w.world_idx + 1);
                    eligible += 1;
                    marked += n;
                }
            }
        }
        // Each eligible world rolls an independent 75% appearance chance, so
        // the observed marked fraction should sit near 0.75. Use a wide band
        // so the assertion is robust across RNG noise but still catches a
        // regression to 100% (always) or 0% (never).
        let frac = marked as f64 / eligible as f64;
        assert!(
            (0.68..0.82).contains(&frac),
            "marked fraction {frac:.3} not near the 0.75 target (marked {marked}/{eligible})"
        );
    }

    #[test]
    fn deterministic_per_seed() {
        let Ok(bytes) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") else {
            return; // Base ROM not present (e.g. CI) — skip.
        };
        let rom = Rom::from_bytes(&bytes).unwrap();
        let run = || {
            let mut rng = ChaCha8Rng::seed_from_u64(42);
            let catalog = node_catalog::NodeCatalog::build(&rom, false);
            let pickup = overworld_pickup::pick_up(&rom, &catalog, overworld_pickup::PickupFlags { shuffle_spade_games: true, shuffle_toad_houses: true });
            let data = overworld_build::OverworldData { pickup: &pickup, catalog: &catalog };
            let mut build = overworld_build::build(&rom, &data, &mut rng, true, false);
            troll_pipes::mark_troll_pipes(&mut build, &mut rng);
            build.worlds.iter()
                .map(|w| w.slots.iter().filter(|s| s.is_troll_pipe).count())
                .collect::<Vec<_>>()
        };
        assert_eq!(run(), run(), "same seed must produce identical troll-pipe marking");
    }
}
