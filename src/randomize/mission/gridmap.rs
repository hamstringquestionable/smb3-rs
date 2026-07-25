//! Real-world-grid adapter: makes an SMB3 overworld map satisfy [`MapView`].
//!
//! Nodes are grid positions encoded as `row * cols + col`. Reachability is the
//! real `walk_map` (so pipes, canoes, and the 2-tile movement model are exactly
//! what the shipping builder sees), and closing a lock is `gap_tile_for` on that
//! tile — identical to how `place_locks` tests a candidate. This is the first
//! point where the mission engine touches real ROM data; it is still read-only
//! and produces no ROM writes.

use std::collections::HashSet;

use super::map::MapView;
use crate::randomize::map_walker::walk_map;
use crate::randomize::overworld_helpers::{LOCKABLE_TILES, find_target, gap_tile_for};
use crate::randomize::rom_data::{self, Grid, TeleportEdge};

/// A world's overworld map, adapted to [`MapView`].
pub(crate) struct GridMap {
    grid: Grid,
    pipe_pairs: Vec<TeleportEdge>,
    cols: usize,
    world_idx: usize,
    start: usize,
    goal: usize,
    fort_slots: Vec<usize>,
    lockable: Vec<usize>,
    /// Reachable node ids with nothing closed — cached, since it's queried on
    /// every `strand_set`/`strands` and never changes.
    open_reach: HashSet<usize>,
}

impl GridMap {
    /// Build the adapter for `world_idx` from a world's map grid. `None` if the
    /// world has no start or no goal tile. `pipe_pairs` are the teleport edges
    /// to honor (empty for a raw vanilla-geometry probe).
    pub(crate) fn new(
        grid: Grid,
        pipe_pairs: Vec<TeleportEdge>,
        world_idx: usize,
    ) -> Option<GridMap> {
        let cols = grid.cols;
        let enc = |p: (usize, usize)| p.0 * cols + p.1;

        let start_pos = rom_data::find_start(&grid)?;
        let goal_pos = find_target(&grid, world_idx)?;

        let walk = walk_map(&grid, &pipe_pairs, Some(start_pos), world_idx);

        // Fort slots are blank *nodes* (standable tiles). Lockable tiles are the
        // *path tiles* between nodes — the connective tissue a lock closes off —
        // which `walk_map` returns in a separate set from the nodes.
        let mut fort_slots = Vec::new();
        for &pos in &walk.nodes {
            if rom_data::VALID_BLANK_TILES.contains(&grid.get(pos.0, pos.1)) {
                fort_slots.push(enc(pos));
            }
        }
        let mut lockable = Vec::new();
        for &pos in &walk.path_tiles {
            if LOCKABLE_TILES.contains(&grid.get(pos.0, pos.1)) {
                lockable.push(enc(pos));
            }
        }
        // Deterministic order (walk_map's sets are unordered).
        fort_slots.sort_unstable();
        lockable.sort_unstable();

        let open_reach: HashSet<usize> = walk.nodes.iter().map(|&p| enc(p)).collect();

        Some(GridMap {
            grid,
            pipe_pairs,
            cols,
            world_idx,
            start: enc(start_pos),
            goal: enc(goal_pos),
            fort_slots,
            lockable,
            open_reach,
        })
    }

    fn decode(&self, id: usize) -> (usize, usize) {
        (id / self.cols, id % self.cols)
    }
}

impl MapView for GridMap {
    fn start(&self) -> usize {
        self.start
    }
    fn goal(&self) -> usize {
        self.goal
    }
    fn fort_slots(&self) -> &[usize] {
        &self.fort_slots
    }
    fn lockable(&self) -> &[usize] {
        &self.lockable
    }

    fn reachable_blocking(&self, blocked: &HashSet<usize>) -> HashSet<usize> {
        if blocked.is_empty() {
            return self.open_reach.clone();
        }
        let mut g = self.grid.clone();
        for &id in blocked {
            let (r, c) = self.decode(id);
            g.set(r, c, gap_tile_for(g.get(r, c)));
        }
        let start_pos = self.decode(self.start);
        walk_map(&g, &self.pipe_pairs, Some(start_pos), self.world_idx)
            .nodes
            .iter()
            .map(|&(r, c)| r * self.cols + c)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::embed::embed;
    use super::super::{Mission, Role};
    use super::*;
    use crate::rom::Rom;

    fn load_rom() -> Option<Rom> {
        let data = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&data).ok()
    }

    /// Diagnostic (not a quality assertion): run the adapter over all 8 real
    /// world maps and report structure + whether vanilla geometry can host a
    /// goal gate. NOTE: this is raw vanilla geometry, NOT the builder's cleared
    /// grids, so a low goal-gate count here reflects vanilla map layout, not
    /// mission-first quality — the cleared-grid, apples-to-apples measurement
    /// vs the 34-49% geometry-first baseline is a later slice.
    /// Run with: cargo test --lib mission_real_world_probe -- --nocapture
    #[test]
    fn mission_real_world_probe() {
        let Some(rom) = load_rom() else {
            return; // no ROM in CI — skip
        };

        eprintln!("\nMission adapter over real world maps (vanilla geometry, with pipes):\n");
        eprintln!(
            "  {:<6} {:>5} {:>6} {:>9} {:>10} {:>10}",
            "world", "nodes", "forts", "lockable", "goal-reach", "goal-gate?"
        );
        for wi in 0..8 {
            let grid = rom_data::read_tile_grid(&rom, wi);
            let pipes = rom_data::read_pipe_pairs(&rom).remove(&wi).unwrap_or_default();
            let gm = GridMap::new(grid, pipes, wi).expect("world should adapt");

            // Start is always a reachable node.
            assert!(gm.open_reach.contains(&gm.start));

            // Some vanilla goals are only reachable via pipes/canoe, which this
            // no-pipe probe doesn't supply — report reachability rather than
            // assume it.
            let goal_reachable = gm.open_reach.contains(&gm.goal);
            let single_gate = Mission { roles: vec![Role::GoalGate] };
            let gated = goal_reachable && embed(&single_gate, &gm).is_some();

            eprintln!(
                "  W{:<5} {:>5} {:>6} {:>9} {:>10} {:>10}",
                wi + 1,
                gm.open_reach.len(),
                gm.fort_slots.len(),
                gm.lockable.len(),
                if goal_reachable { "yes" } else { "no" },
                if gated { "yes" } else { "no" },
            );
        }
    }
}
