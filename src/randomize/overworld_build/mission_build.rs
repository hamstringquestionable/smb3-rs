//! Mission-first builder — iteration 1, slice 1.
//!
//! Translate a mission `Embedding` (abstract node ids from the mission engine)
//! into the builder's own fortress slots and lock assignments — the format the
//! writer consumes. This is the glue between the proven mission engine and the
//! `BuildResult` contract; later slices place levels/filler and assemble the
//! whole build around it.

use super::types::{LockAssignment, SlotAssignment, SlotKind};
use crate::randomize::mission::{GridMap, MapView, Mission, embed};
use crate::randomize::overworld_helpers::gap_tile_for;
use crate::randomize::rom_data::{Grid, TeleportEdge};

/// Place this world's fortresses and locks mission-first: sample a chain of
/// `fort_count` forts, embed it on the cleared + piped grid, and translate the
/// result into builder slots + locks. `None` if the map can't host the mission
/// (rare — embed is ~100% on cleared grids).
// Reason: WIP — consumed by the mission_build orchestrator in the next slice;
// only exercised by tests until then.
#[allow(dead_code)]
pub(super) fn mission_forts_and_locks(
    grid: &Grid,
    pipes: &[TeleportEdge],
    world_idx: usize,
    fort_count: usize,
) -> Option<(Vec<SlotAssignment>, Vec<LockAssignment>)> {
    if fort_count == 0 {
        return Some((Vec::new(), Vec::new()));
    }

    let gm = GridMap::new(grid.clone(), pipes.to_vec(), world_idx)?;
    let mission = Mission::chain(fort_count);
    let placed = embed(&mission, &gm)?;

    let cols = grid.cols;
    let decode = |id: usize| (id / cols, id % cols);

    let mut forts = Vec::with_capacity(fort_count);
    let mut locks = Vec::with_capacity(fort_count);

    for fort in 0..fort_count {
        forts.push(SlotAssignment {
            pos: decode(placed.fort_pos[fort]),
            kind: SlotKind::Fortress,
            section: fort,
            is_hand_trap: false,
            is_troll_pipe: false,
        });

        let lock_id = placed.lock_pos[fort];
        let lock_pos = decode(lock_id);
        let original = grid.get(lock_pos.0, lock_pos.1);
        // With this lock alone closed, is the goal cut off? That's a
        // target-blocker; otherwise the secret exit stays safe.
        let blocks_target = gm.strands(lock_id, gm.goal());
        locks.push(LockAssignment {
            pos: lock_pos,
            gap_tile: gap_tile_for(original),
            replace_tile: original,
            fort_section: fort,
            secret_exit_safe: !blocks_target,
            blocks_target,
        });
    }

    Some((forts, locks))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::randomize::node_catalog::NodeCatalog;
    use crate::randomize::overworld_helpers::LOCKABLE_TILES;
    use crate::randomize::overworld_pickup::{PickupFlags, pick_up};
    use crate::rom::Rom;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;
    use std::collections::HashSet;

    fn load_rom() -> Option<Rom> {
        let data = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&data).ok()
    }

    /// Slice-1 check: the mission engine's fort/lock output translates into
    /// well-formed builder slots + locks on real cleared + piped grids.
    #[test]
    fn forts_and_locks_are_well_formed() {
        let Some(rom) = load_rom() else {
            return; // no ROM — skip
        };
        let catalog = NodeCatalog::build(&rom, false);
        let pickup = pick_up(
            &rom,
            &catalog,
            PickupFlags {
                shuffle_spade_games: false,
                shuffle_toad_houses: true,
                shuffle_hammer_bros: false,
            },
        );

        for seed in 0..10u64 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let result = super::super::build(
                &rom,
                &super::super::OverworldData { pickup: &pickup, catalog: &catalog },
                &mut rng,
                super::super::BuildFlags { shuffle_toad_houses: true, ..Default::default() },
            );

            for built in &result.worlds {
                let wi = built.world_idx;
                let fc = built.section_count;

                let (forts, locks) =
                    mission_forts_and_locks(&built.grid, &built.pipe_pairs, wi, fc)
                        .unwrap_or_else(|| panic!("W{} seed {seed}: mission failed to embed", wi + 1));

                assert_eq!(forts.len(), fc, "W{} fort count", wi + 1);
                assert_eq!(locks.len(), fc, "W{} lock count", wi + 1);

                let fort_positions: HashSet<_> = forts.iter().map(|f| f.pos).collect();
                assert_eq!(fort_positions.len(), fc, "W{} distinct forts", wi + 1);

                for (i, f) in forts.iter().enumerate() {
                    assert_eq!(f.kind, SlotKind::Fortress);
                    assert_eq!(f.section, i);
                }

                let lock_positions: HashSet<_> = locks.iter().map(|l| l.pos).collect();
                assert_eq!(lock_positions.len(), fc, "W{} distinct locks", wi + 1);
                for l in &locks {
                    assert!(
                        LOCKABLE_TILES.contains(&l.replace_tile),
                        "W{} lock on non-lockable tile {:#04x}",
                        wi + 1,
                        l.replace_tile
                    );
                    assert!(!fort_positions.contains(&l.pos), "W{} lock on a fort tile", wi + 1);
                }
            }
        }
    }
}
