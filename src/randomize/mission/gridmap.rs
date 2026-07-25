//! Real-world-grid adapter: makes an SMB3 overworld map satisfy [`MapView`].
//!
//! Nodes are grid positions encoded as `row * cols + col`. Reachability is the
//! real `walk_map` (so pipes, canoes, and the 2-tile movement model are exactly
//! what the shipping builder sees), and closing a lock is `gap_tile_for` on that
//! tile — identical to how `place_locks` tests a candidate. This is the first
//! point where the mission engine touches real ROM data; it is still read-only
//! and produces no ROM writes.

use std::collections::{HashMap, HashSet};

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
    /// What each lockable tile gates (node ids stranded when it alone is
    /// closed), precomputed once. `embed` queries these thousands of times, so
    /// caching them turns each query from a `walk_map` into a lookup.
    strand_cache: HashMap<usize, HashSet<usize>>,
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

        // Precompute each lockable tile's strand set once. Closing a lock is
        // gap_tile_for on that tile; what it gates is the nodes that fall out of
        // the walk (difference form, so already-unreachable nodes aren't
        // miscredited).
        let mut strand_cache: HashMap<usize, HashSet<usize>> = HashMap::new();
        for &lock in &lockable {
            let (r, c) = (lock / cols, lock % cols);
            let mut g = grid.clone();
            g.set(r, c, gap_tile_for(g.get(r, c)));
            let closed: HashSet<usize> = walk_map(&g, &pipe_pairs, Some(start_pos), world_idx)
                .nodes
                .iter()
                .map(|&p| enc(p))
                .collect();
            let strand = open_reach.difference(&closed).copied().filter(|v| *v != lock).collect();
            strand_cache.insert(lock, strand);
        }

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
            strand_cache,
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

    // Served from the precomputed cache — the hot path during embed search.
    fn strand_set(&self, lock: usize) -> HashSet<usize> {
        self.strand_cache.get(&lock).cloned().unwrap_or_default()
    }
    fn strands(&self, lock: usize, target: usize) -> bool {
        self.strand_cache.get(&lock).is_some_and(|s| s.contains(&target))
    }
}

#[cfg(test)]
mod tests {
    use super::super::embed::embed;
    use super::super::{Mission, Role};
    use super::*;
    use crate::randomize::node_catalog::NodeCatalog;
    use crate::randomize::overworld_build::{BuildFlags, OverworldData, build};
    use crate::randomize::overworld_pickup::{PickupFlags, pick_up};
    use crate::rom::Rom;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

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

    /// The real measurement: run the actual builder to get each world's cleared,
    /// piped grid (no forts stamped), then embed a SingleGate mission on it.
    /// Unlike the vanilla probe, these grids have no pre-existing fortress locks,
    /// so the goal is openly reachable and a placed lock actually gates it.
    /// Run with: `cargo test --release --lib mission_cleared_grid_probe -- --nocapture`
    #[test]
    fn mission_cleared_grid_probe() {
        let Some(rom) = load_rom() else {
            return; // no ROM in CI — skip
        };

        const SEEDS: u64 = 30;
        // Catalog + pickup don't depend on the build RNG, so build them once.
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

        let mut gate_ok = [0u32; 8];
        let mut goal_reachable = [0u32; 8];
        let mut total = [0u32; 8];

        for seed in 0..SEEDS {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let result = build(
                &rom,
                &OverworldData { pickup: &pickup, catalog: &catalog },
                &mut rng,
                BuildFlags { shuffle_toad_houses: true, ..Default::default() },
            );
            for built in &result.worlds {
                let wi = built.world_idx;
                let Some(gm) = GridMap::new(built.grid.clone(), built.pipe_pairs.clone(), wi) else {
                    continue;
                };
                total[wi] += 1;
                if gm.open_reach.contains(&gm.goal) {
                    goal_reachable[wi] += 1;
                }
                let single_gate = Mission { roles: vec![Role::GoalGate] };
                if embed(&single_gate, &gm).is_some() {
                    gate_ok[wi] += 1;
                }
            }
        }

        eprintln!("\nSingleGate embed on cleared + piped builder grids ({SEEDS} seeds):\n");
        eprintln!("  {:<6} {:>10} {:>12}", "world", "goal-reach", "goal-gated");
        let (mut g, mut t) = (0u32, 0u32);
        for wi in 0..8 {
            eprintln!(
                "  W{:<5} {:>7}/{:<2} {:>9}/{:<2}",
                wi + 1,
                goal_reachable[wi],
                total[wi],
                gate_ok[wi],
                total[wi],
            );
            g += gate_ok[wi];
            t += total[wi];
        }
        eprintln!("\n  overall goal-gate embed: {g}/{t}");
    }
}
