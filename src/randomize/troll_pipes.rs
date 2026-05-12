use rand::Rng;
use rand::seq::IndexedRandom;

use super::overworld_build::{BuildResult, SlotKind};

/// Mark exactly one regular-level slot per world W2-W8 as a troll pipe.
/// W1 is excluded so the player has at least one safe world to learn the
/// game's vanilla appearance before encountering disguised level slots.
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
        if let Some(&pick) = candidates.choose(rng) {
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
    fn marks_one_pipe_per_world_w2_w8() {
        let Ok(bytes) = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes") else {
            return; // Base ROM not present (e.g. CI) — skip.
        };
        let rom = Rom::from_bytes(&bytes).unwrap();
        let mut counts = [0usize; 8];
        for seed in 0u64..16 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let catalog = node_catalog::NodeCatalog::build(&rom, false);
            let pickup = overworld_pickup::pick_up(&rom, &catalog, overworld_pickup::PickupFlags { shuffle_spade_games: true, shuffle_toad_houses: true });
            let mut build = overworld_build::build(&rom, &pickup, &catalog, &mut rng, true);
            troll_pipes::mark_troll_pipes(&mut build, &mut rng);
            for w in &build.worlds {
                let n = w.slots.iter().filter(|s| s.is_troll_pipe).count();
                if w.world_idx == 0 {
                    assert_eq!(n, 0, "W1 should never have a troll pipe (seed {seed})");
                } else {
                    assert_eq!(n, 1, "W{} should have exactly 1 troll pipe (seed {seed})", w.world_idx + 1);
                    counts[w.world_idx] += 1;
                }
            }
        }
        assert_eq!(counts[0], 0);
        for (w, &count) in counts.iter().enumerate().skip(1) {
            assert_eq!(count, 16, "W{} should be marked 16 times across 16 seeds", w + 1);
        }
    }
}
