/// Phase 3 of the overworld builder rewrite: Build.
///
/// Takes `PickupResult` (Phase 2) + `NodeCatalog` (Phase 1) + RNG and produces
/// slot assignments for each world. Does NOT assign specific pool entries or
/// write to ROM — that's Phase 4 (writer).
///
/// Algorithm:
/// 0. Redistribute fortresses across worlds (W8 keeps 4, W1-W7 get 1-3 each)
/// 1. Place pipes (connectivity first, then remaining to connect islands)
/// 2. BFS sectioning (order reachable blanks by distance, divide by fort count)
/// 3. Populate sections (1 fort per section, rest are levels)
/// 4. Lock placement (every fort gets 1 lock)

use std::collections::{HashMap, HashSet};

use rand::Rng;
use rand::seq::{IndexedRandom, SliceRandom};

use super::map_walker::walk_map;
use super::node_catalog::{NodeCatalog, NodeKind};
use super::overworld_helpers::{find_target, gap_tile_for, LOCKABLE_TILES};
use super::overworld_pickup::PickupResult;
use crate::rom::Rom;
use super::rom_data::{
    self, BACKGROUND_TILES, Grid, TILE_NODE, TILE_PIPE, TILE_FORTRESS,
    VALID_HORZ, VALID_VERT,
};

// ---------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------

/// What kind of node occupies a grid slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlotKind {
    Level,
    Fortress,
    Pipe,
    HammerBro,
}

/// A single slot assignment on the grid.
#[derive(Clone, Debug)]
pub struct SlotAssignment {
    pub pos: (usize, usize),
    pub kind: SlotKind,
    /// Which section (0-based) this slot belongs to.
    pub section: usize,
}

/// A lock/bridge placed on a path tile.
#[derive(Clone, Debug)]
pub(crate) struct LockAssignment {
    /// Path tile position where the lock goes.
    pub pos: (usize, usize),
    /// The blocking tile to write (0x54 vert lock, 0x56 horiz lock, 0xE4 sky lock, 0x9D water gap).
    pub gap_tile: u8,
    /// The original path tile (for FX restore).
    pub replace_tile: u8,
    /// Which fortress (section index) opens this lock.
    pub fort_section: usize,
    /// True if the world's target (airship/Bowser) is still reachable with
    /// this lock closed. These locks are safe for 1-F (secret exit doesn't
    /// trigger FX replacement).
    pub secret_exit_safe: bool,
}

/// Complete build result for one world.
#[derive(Clone, Debug)]
pub(crate) struct BuiltWorld {
    #[allow(dead_code)] // read in tests
    pub world_idx: usize,
    /// The grid with pipes placed (but no forts/levels/locks stamped yet).
    pub grid: Grid,
    /// Slot assignments for placeable nodes.
    pub slots: Vec<SlotAssignment>,
    /// Lock/bridge assignments.
    pub locks: Vec<LockAssignment>,
    /// Number of sections (= number of fortresses in this world).
    pub section_count: usize,
    /// Pipe pair positions placed in this world: Vec of (endpoint_a, endpoint_b).
    pub pipe_pairs: Vec<((usize, usize), (usize, usize))>,
}

/// Complete Phase 3 output.
pub(crate) struct BuildResult {
    pub worlds: Vec<BuiltWorld>,
    /// Fortress counts per world (decided in Step 0).
    #[allow(dead_code)] // read in tests
    pub fort_counts: [usize; 8],
}

// ---------------------------------------------------------------------------
// Vanilla pipe pair counts per world
// ---------------------------------------------------------------------------

/// Number of pipe pairs (not endpoints) per world in the vanilla ROM.
const VANILLA_PIPE_PAIRS: [usize; 8] = [
    0,  // W1
    1,  // W2
    3,  // W3
    2,  // W4
    2,  // W5 (includes spiral tower)
    2,  // W6
    8,  // W7
    6,  // W8
];

/// Fixed pipe endpoints per world: positions that must always be a pipe.
/// The partner endpoint is placed randomly. Each entry is (world_idx, position).
const FIXED_PIPE_ENDPOINTS: &[(usize, (usize, usize))] = &[
    (2, (6, 45)), // W3 rightmost node — always a pipe, partner randomized
];

/// Positions excluded from pipe placement. These are blank tiles that are
/// unreachable (surrounded by rocks/walls) and should never get a pipe.
const PIPE_EXCLUDED_POSITIONS: &[(usize, (usize, usize))] = &[
    (2, (8, 6)), // W3 between two rocks near start — HB only, not a pipe slot
];

/// Fortress score bonus positions per world. These isolated positions rarely
/// win fortress placement without a boost. Each entry is (world_idx, position).
const FORTRESS_BONUS_POSITIONS: &[(usize, (usize, usize))] = &[
    (2, (5, 26)), // W3 canoe island
    (2, (0, 34)), // W3 canoe island (toad house in vanilla)
    (2, (5, 28)), // W3 canoe island (spade in vanilla)
    (2, (3, 26)), // W3 canoe island (spade in vanilla)
    (2, (3, 28)), // W3 canoe island
];
const FORTRESS_BONUS: f64 = 0.5;

/// Total vanilla levels across all worlds (62 Level entries in the catalog).
const VANILLA_LEVEL_COUNT: usize = 62;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Execute Phase 3: build slot assignments for all 8 worlds.
pub(crate) fn build<R: Rng>(
    rom: &Rom,
    pickup: &PickupResult,
    catalog: &NodeCatalog,
    rng: &mut R,
) -> BuildResult {
    // Step 0: redistribute fortresses
    let fort_counts = redistribute_fortresses(rng);

    // Build patched grids once: clone pickup grids and restore airship/Bowser
    // tiles (blanked during pickup but kept at vanilla positions).
    let mut patched_grids: Vec<Grid> = Vec::with_capacity(8);
    for wi in 0..8 {
        let mut grid = pickup.worlds[wi].grid.clone();
        for entry in &catalog.entries {
            if entry.world_idx != wi {
                continue;
            }
            if matches!(entry.kind, NodeKind::Airship | NodeKind::Bowser) {
                let (r, c) = entry.grid_pos;
                if r < grid.rows && c < grid.cols {
                    grid.set(r, c, entry.tile);
                }
            }
        }
        patched_grids.push(grid);
    }

    // Pre-compute available level slots per world. Two constraints apply:
    // 1. Grid blanks: number of blank node tiles on the map (visual capacity).
    // 2. Pointer table slots: entries vacated during pickup (ROM capacity).
    // The tighter constraint wins — assigning more entries than pointer table
    // slots causes blank screens because write_pointer_entries runs out of
    // slots to write to.
    // Pre-compute fixed positions once per world (used by both capacity
    // calculation and build_world).
    let fixed_positions: Vec<HashSet<(usize, usize)>> = (0..8)
        .map(|wi| fixed_positions_for_world(rom, catalog, wi))
        .collect();

    let mut capacities = [0usize; 8];
    for wi in 0..8 {
        let pipe_endpoints = VANILLA_PIPE_PAIRS[wi] * 2;
        let blanks = find_blank_slots(&patched_grids[wi], &fixed_positions[wi]).len();
        let grid_capacity = blanks.saturating_sub(pipe_endpoints + fort_counts[wi]);

        // Cap by available pointer table slots from pickup.
        let ptr_slots = pickup.worlds[wi].pool_indices.len();
        let ptr_capacity = ptr_slots.saturating_sub(pipe_endpoints + fort_counts[wi]);

        capacities[wi] = grid_capacity.min(ptr_capacity);
    }

    // Distribute VANILLA_LEVEL_COUNT levels across worlds proportionally to capacity.
    let mut level_counts = distribute_levels(&capacities, VANILLA_LEVEL_COUNT, rng);

    // W6 (index 5): cap levels so levels + fortresses = vanilla total (13).
    // W6's dense map topology clumps badly with too many levels.
    let w6_max_levels = 13usize.saturating_sub(fort_counts[5]);
    if level_counts[5] > w6_max_levels {
        let surplus = level_counts[5] - w6_max_levels;
        level_counts[5] = w6_max_levels;

        // Redistribute surplus to other worlds with spare capacity.
        let mut remaining = surplus;
        let mut order: Vec<usize> = (0..8).filter(|&w| w != 5).collect();
        order.shuffle(rng);
        for &wi in &order {
            if remaining == 0 { break; }
            let spare = capacities[wi].saturating_sub(level_counts[wi]);
            let give = spare.min(remaining);
            level_counts[wi] += give;
            remaining -= give;
        }
    }

    let mut lock_counter: usize = 0;
    let mut worlds = Vec::with_capacity(8);
    for wi in 0..8 {
        // Max non-pipe slots = pointer table slots minus pipe endpoints.
        // This caps the total fort+level+HB entries to what the pointer table
        // can actually hold. Excess blank tiles stay as path nodes.
        let ptr_slots = pickup.worlds[wi].pool_indices.len();
        let pipe_endpoints = VANILLA_PIPE_PAIRS[wi] * 2;
        let max_non_pipe_slots = ptr_slots.saturating_sub(pipe_endpoints);

        let built = build_world(
            wi,
            rom,
            patched_grids[wi].clone(),
            &fixed_positions[wi],
            fort_counts[wi],
            level_counts[wi],
            VANILLA_PIPE_PAIRS[wi],
            max_non_pipe_slots,
            &mut lock_counter,
            rng,
        );
        worlds.push(built);
    }

    // Ensure at least one lock across all worlds is secret-exit-safe.
    // The 1-in-4 safe preference usually produces one, but if not,
    // retry lock placement in a random world with counter forced to
    // a safe slot.
    let has_safe = worlds.iter().any(|b| b.locks.iter().any(|l| l.secret_exit_safe));
    if !has_safe {
        let mut retry_order: Vec<usize> = (0..8).collect();
        retry_order.shuffle(rng);
        let mut retry_counter: usize = 3; // forces prefer_safe on first lock
        for &wi in &retry_order {
            let built = &worlds[wi];
            let start_pos = rom_data::find_start(&built.grid);
            let target_pos = find_target(&built.grid, wi);
            let new_locks = place_locks(
                &built.grid,
                &built.pipe_pairs,
                start_pos,
                target_pos,
                &built.slots,
                fort_counts[wi],
                &mut retry_counter,
                rng,
            );
            if new_locks.iter().any(|l| l.secret_exit_safe) {
                worlds[wi].locks = new_locks;
                break;
            }
        }
    }

    BuildResult { worlds, fort_counts }
}

/// Collect positions that must not be overwritten by level/fort/pipe placement.
///
/// This includes: airship, Bowser, toad house tiles (catalog-based), plus the
/// grid positions of floating map object sprites (hammer bro sprites, piranhas,
/// W8 hand traps) read from the ROM's map object tables.
///
/// HammerBro catalog entries are NOT excluded — those blank tiles are valid
/// placement slots. Only the actual floating sprite positions are excluded
/// because a numbered level tile under a floating sprite looks wrong.
fn fixed_positions_for_world(
    rom: &Rom,
    catalog: &NodeCatalog,
    world_idx: usize,
) -> HashSet<(usize, usize)> {
    let mut fixed = HashSet::new();

    // Airship, Bowser, toad house — stay at vanilla positions
    for entry in &catalog.entries {
        if entry.world_idx != world_idx {
            continue;
        }
        match entry.kind {
            NodeKind::Airship | NodeKind::Bowser | NodeKind::ToadHouse => {
                fixed.insert(entry.grid_pos);
            }
            _ => {}
        }
    }

    // Floating sprite positions from the map object tables
    for pos in rom_data::read_map_sprite_positions(rom, world_idx) {
        fixed.insert(pos);
    }

    fixed
}

/// Distribute `total` levels across worlds proportional to capacity.
/// Ensures every level is placed (sum of output == total).
/// World processing order is shuffled to avoid front-loading bias from
/// rounding.
fn distribute_levels<R: Rng>(capacities: &[usize; 8], total: usize, rng: &mut R) -> [usize; 8] {
    let total_cap: usize = capacities.iter().sum();
    let mut counts = [0usize; 8];

    if total_cap == 0 || total == 0 {
        return counts;
    }

    // Shuffle world processing order to spread rounding bias randomly.
    let mut order: Vec<usize> = (0..8).collect();
    order.shuffle(rng);

    // Proportional allocation
    let mut remaining = total;
    for &wi in &order {
        let share = (capacities[wi] as f64 / total_cap as f64 * total as f64).round() as usize;
        counts[wi] = share.min(capacities[wi]).min(remaining);
        remaining -= counts[wi];
    }

    // Distribute any leftover (rounding errors) to worlds with spare capacity
    for &wi in &order {
        if remaining == 0 {
            break;
        }
        let spare = capacities[wi] - counts[wi];
        let give = spare.min(remaining);
        counts[wi] += give;
        remaining -= give;
    }

    counts
}

// ---------------------------------------------------------------------------
// Step 0: Fortress redistribution
// ---------------------------------------------------------------------------

/// Distribute 13 fortresses across W1-W7 (each gets 1-3), W8 keeps 4.
fn redistribute_fortresses<R: Rng>(rng: &mut R) -> [usize; 8] {
    let mut counts = [0usize; 8];
    counts[7] = 4; // W8 always keeps 4

    // Start each of W1-W7 with 1 fortress (= 7 used), leaving 6 to distribute
    for c in counts[..7].iter_mut() {
        *c = 1;
    }
    let mut remaining = 6; // 13 - 7

    // Distribute remaining fortresses randomly, respecting max of 3 per world
    while remaining > 0 {
        let eligible: Vec<usize> = (0..7).filter(|&w| counts[w] < 3).collect();
        if eligible.is_empty() {
            break;
        }
        let &w = eligible.choose(rng).unwrap();
        counts[w] += 1;
        remaining -= 1;
    }

    counts
}

// ---------------------------------------------------------------------------
// Per-world build
// ---------------------------------------------------------------------------

fn build_world<R: Rng>(
    world_idx: usize,
    rom: &Rom,
    mut grid: Grid,
    fixed_positions: &HashSet<(usize, usize)>,
    fort_count: usize,
    level_count: usize,
    pipe_pair_count: usize,
    max_non_pipe_slots: usize,
    lock_counter: &mut usize,
    rng: &mut R,
) -> BuiltWorld {
    let start_pos = rom_data::find_start(&grid);
    let target_pos = find_target(&grid, world_idx);

    // Collect all blank node positions (potential placement slots).
    let blank_positions = find_blank_slots(&grid, fixed_positions);

    // Collect fixed pipe endpoints for this world.
    let fixed_pipe_eps: Vec<(usize, usize)> = FIXED_PIPE_ENDPOINTS
        .iter()
        .filter(|(wi, _)| *wi == world_idx)
        .map(|(_, pos)| *pos)
        .collect();

    // Exclude certain positions from pipe placement (unreachable blanks).
    let pipe_excluded: HashSet<(usize, usize)> = PIPE_EXCLUDED_POSITIONS
        .iter()
        .filter(|(wi, _)| *wi == world_idx)
        .map(|(_, pos)| *pos)
        .collect();
    let pipe_blanks: Vec<(usize, usize)> = blank_positions
        .iter()
        .copied()
        .filter(|p| !pipe_excluded.contains(p))
        .collect();

    // Step 1: Place pipes
    let pipe_pairs = place_pipes(
        &mut grid,
        &pipe_blanks,
        start_pos,
        target_pos,
        pipe_pair_count,
        &fixed_pipe_eps,
        rng,
    );

    // Collect positions used by pipes
    let pipe_positions: HashSet<(usize, usize)> = pipe_pairs
        .iter()
        .flat_map(|&(a, b)| vec![a, b])
        .collect();

    // Step 2: BFS sectioning
    let sections = bfs_section(
        &grid,
        &pipe_pairs,
        start_pos,
        &blank_positions,
        &pipe_positions,
        &fixed_positions,
        fort_count,
    );

    // Build BFS distance map for scoring — reflects actual walkable distance.
    let bfs_distances: HashMap<(usize, usize), usize> =
        bfs_ordered(&grid, &pipe_pairs, start_pos)
            .into_iter()
            .collect();

    // Reverse BFS from target (airship/Bowser) — used to compute path relevance
    // for level scoring. Positions on the main start→target trunk have low detour.
    let reverse_bfs: HashMap<(usize, usize), usize> = target_pos
        .map(|tp| walk_map(&grid, &pipe_pairs, Some(tp)).distances)
        .unwrap_or_default();
    let target_bfs_dist = target_pos.and_then(|tp| bfs_distances.get(&tp).copied());

    // Step 3: Populate sections
    let mut slots = populate_sections(&grid, &sections, fort_count, level_count, &pipe_positions, &bfs_distances, &reverse_bfs, target_bfs_dist, world_idx, rng);

    // Add mandatory HammerBro slots for HB sprite starting positions.
    // These were excluded from find_blank_slots (so levels/forts/pipes
    // aren't placed under sprites) but still need pointer table entries —
    // the sprite starts there and can be encountered immediately.
    let hb_sprite_pos_list: Vec<(usize, usize)> = {
        let existing: HashSet<(usize, usize)> = slots.iter().map(|s| s.pos).collect();
        rom_data::read_hb_sprite_positions(rom, world_idx)
            .into_iter()
            .filter(|pos| !existing.contains(pos))
            .collect()
    };
    let hb_sprite_positions: HashSet<(usize, usize)> = hb_sprite_pos_list.iter().copied().collect();
    for pos in &hb_sprite_pos_list {
        slots.push(SlotAssignment {
            pos: *pos,
            kind: SlotKind::HammerBro,
            section: 0,
        });
    }

    // Cap total slots to what the pointer table can hold. Forts and levels
    // are already within budget (capped during capacity calculation); any
    // excess is purely HammerBro slots from blank tiles. Drop the farthest
    // regular HB slots but never drop HB sprite positions — those are
    // mandatory.
    if slots.len() > max_non_pipe_slots {
        let mut kept: Vec<SlotAssignment> = Vec::with_capacity(max_non_pipe_slots);
        let mut hb_slots: Vec<SlotAssignment> = Vec::new();
        for s in slots {
            if s.kind != SlotKind::HammerBro || hb_sprite_positions.contains(&s.pos) {
                kept.push(s);
            } else {
                hb_slots.push(s);
            }
        }
        let hb_budget = max_non_pipe_slots.saturating_sub(kept.len());
        kept.extend(hb_slots.into_iter().take(hb_budget));
        slots = kept;
    }

    // Step 4: Lock placement
    let locks = place_locks(
        &grid,
        &pipe_pairs,
        start_pos,
        target_pos,
        &slots,
        fort_count,
        lock_counter,
        rng,
    );

    BuiltWorld {
        world_idx,
        grid,
        slots,
        locks,
        section_count: fort_count,
        pipe_pairs,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Find all blank node slots on the grid (positions with theme-blank tiles).
fn find_blank_slots(
    grid: &Grid,
    fixed_positions: &HashSet<(usize, usize)>,
) -> Vec<(usize, usize)> {
    let mut blanks = Vec::new();
    for r in 0..grid.rows {
        for c in 0..grid.cols {
            let pos = (r, c);
            if fixed_positions.contains(&pos) {
                continue;
            }
            if !rom_data::VALID_BLANK_TILES.contains(&grid.get(r, c)) {
                continue;
            }
            blanks.push(pos);
        }
    }
    blanks
}

/// Returns true if a tile would be "caught" by the game's
/// `Map_Reload_with_Completions` routine. Used to seed the completable set
/// and to prevent lock placement at row 7 when row 8 has a level.
///
/// The game checks (in order):
/// 1. Special tiles: $50, $E8, $E6, $BD, $E0
/// 2. Fortress: $67, $EB
/// 3. Page threshold: page0 >= $03, page1 >= $67, page2 >= $BF, page3 >= $E9
/// 4. Map_Removable_Tiles: $51, $52, $54, $67, $EB, $E4, $56, $9D
fn is_completion_unsafe(tile: u8) -> bool {
    const SPECIAL: [u8; 5] = [0x50, 0xE8, 0xE6, 0xBD, 0xE0];
    const REMOVABLE: [u8; 8] = [0x51, 0x52, 0x54, 0x67, 0xEB, 0xE4, 0x56, 0x9D];
    const THRESHOLDS: [u8; 4] = [0x03, 0x67, 0xBF, 0xE9];

    // 0x67/0xEB are also caught by the threshold check below, but kept
    // explicit here for readability — fortress tiles are the primary case.
    if SPECIAL.contains(&tile) || tile == 0x67 || tile == 0xEB {
        return true;
    }
    let page = (tile >> 6) as usize;
    if tile >= THRESHOLDS[page] {
        return true;
    }
    REMOVABLE.contains(&tile)
}

/// BFS from start, returning nodes in visit order with their distances.
/// BFS-ordered list of (position, distance) using the canonical `walk_map`.
///
/// This is the single source of truth for map traversal — all BFS-dependent
/// logic (sectioning, scoring, connectivity checks) must go through here or
/// `walk_map` directly to stay in sync with canoe edges, pipe teleports, etc.
pub(super) fn bfs_ordered(
    grid: &Grid,
    pipe_pairs: &[((usize, usize), (usize, usize))],
    start_pos: Option<(usize, usize)>,
) -> Vec<((usize, usize), usize)> {
    let result = walk_map(grid, pipe_pairs, start_pos);
    let mut ordered: Vec<((usize, usize), usize)> = result
        .distances
        .into_iter()
        .collect();
    // Sort by distance, then by position for determinism (HashMap has no order).
    ordered.sort_by_key(|&((r, c), d)| (d, r, c));
    ordered
}

// ---------------------------------------------------------------------------
// Step 1: Pipe placement
// ---------------------------------------------------------------------------

/// Split blank positions into (reachable, unreachable) relative to BFS walk,
/// excluding already-used positions.
fn split_blanks_by_reachability(
    blanks: &[(usize, usize)],
    reachable: &HashSet<(usize, usize)>,
    used: &HashSet<(usize, usize)>,
) -> (Vec<(usize, usize)>, Vec<(usize, usize)>) {
    let mut reach = Vec::new();
    let mut unreach = Vec::new();
    for &p in blanks {
        if used.contains(&p) {
            continue;
        }
        if reachable.contains(&p) {
            reach.push(p);
        } else {
            unreach.push(p);
        }
    }
    (reach, unreach)
}

fn place_pipes<R: Rng>(
    grid: &mut Grid,
    blank_positions: &[(usize, usize)],
    start_pos: Option<(usize, usize)>,
    target_pos: Option<(usize, usize)>,
    pair_count: usize,
    fixed_endpoints: &[(usize, usize)],
    rng: &mut R,
) -> Vec<((usize, usize), (usize, usize))> {
    if pair_count == 0 {
        return Vec::new();
    }

    let mut placed_pairs: Vec<((usize, usize), (usize, usize))> = Vec::new();
    let mut used_positions: HashSet<(usize, usize)> = HashSet::new();

    // Phase 0: fixed endpoints — place these first, partner on opposite side.
    // The fixed endpoint is typically on an island (e.g. W3 rightmost node).
    // The partner must be on the reachable mainland so the pipe actually
    // bridges the gap. If both ends land on the same island the pipe is
    // useless and the target becomes unreachable.
    for &fixed_pos in fixed_endpoints {
        if placed_pairs.len() >= pair_count {
            break;
        }
        grid.set(fixed_pos.0, fixed_pos.1, TILE_PIPE);
        used_positions.insert(fixed_pos);

        // BFS to find which blanks are reachable from start.
        let walk = walk_map(grid, &placed_pairs, start_pos);
        let fixed_is_reachable = walk.nodes.contains(&fixed_pos);

        // Pick partner from opposite side: if fixed is on an island,
        // partner must be reachable (and vice versa).
        let available: Vec<(usize, usize)> = blank_positions
            .iter()
            .copied()
            .filter(|p| !used_positions.contains(p))
            .filter(|p| walk.nodes.contains(p) != fixed_is_reachable)
            .collect();

        // Fall back to any available blank if no opposite-side candidates.
        let fallback: Vec<(usize, usize)> = if available.is_empty() {
            blank_positions
                .iter()
                .copied()
                .filter(|p| !used_positions.contains(p))
                .collect()
        } else {
            Vec::new()
        };
        let candidates = if available.is_empty() { &fallback } else { &available };

        if let Some(&partner) = candidates.choose(rng) {
            grid.set(partner.0, partner.1, TILE_PIPE);
            used_positions.insert(partner);
            placed_pairs.push((fixed_pos, partner));
        }
    }

    // Phase A+B: connect islands first (required for target reachability in A,
    // best-effort in B), then fill remaining pairs in reachable area.
    let target_reachable = |g: &Grid, pairs: &[((usize, usize), (usize, usize))]| -> bool {
        if let Some(tp) = target_pos {
            let walk = walk_map(g, pairs, start_pos);
            walk.nodes.contains(&tp)
        } else {
            true // no target = nothing to connect
        }
    };

    let mut must_connect_target = true;
    while placed_pairs.len() < pair_count {
        // In the must_connect_target phase, stop once target is reachable.
        if must_connect_target && target_reachable(grid, &placed_pairs) {
            must_connect_target = false;
        }

        let walk = walk_map(grid, &placed_pairs, start_pos);
        let (reachable_blanks, unreachable_blanks) =
            split_blanks_by_reachability(blank_positions, &walk.nodes, &used_positions);

        if !unreachable_blanks.is_empty() && !reachable_blanks.is_empty() {
            // Connect an island: scored selection for both endpoints.
            // Unreachable side: prefer nearer islands (manhattan from start)
            // to create progressive chains rather than jumping to the end.
            let start = start_pos.unwrap_or((0, 0));
            let b = {
                let mut scored: Vec<((usize, usize), f64)> = unreachable_blanks
                    .iter()
                    .map(|&pos| {
                        let start_dist = (pos.0.abs_diff(start.0) + pos.1.abs_diff(start.1)) as f64;
                        // Nearer to start = higher score (invert distance)
                        let proximity_score = (TARGET_MAX_DIST - start_dist.min(TARGET_MAX_DIST)) / TARGET_MAX_DIST * 5.0;
                        let target_pen = target_proximity_penalty(pos, target_pos);
                        proximity_score - target_pen
                    })
                    .zip(unreachable_blanks.iter().copied())
                    .map(|(score, pos)| (pos, score))
                    .collect();
                scored.sort_by(|x, y| y.1.partial_cmp(&x.1).unwrap_or(std::cmp::Ordering::Equal));
                let top_n = scored.len().min(5);
                scored[rng.random_range(..top_n)].0
            };

            // Reachable side: prefer positions far from start (BFS distance),
            // spread from existing pipes, and away from target.
            let a = {
                let mut scored: Vec<((usize, usize), f64)> = reachable_blanks
                    .iter()
                    .map(|&pos| {
                        let score = score_pipe_endpoint(
                            grid, pos, &used_positions, &walk.distances, target_pos,
                        );
                        (pos, score)
                    })
                    .collect();
                scored.sort_by(|x, y| y.1.partial_cmp(&x.1).unwrap_or(std::cmp::Ordering::Equal));
                let top_n = scored.len().min(5);
                scored[rng.random_range(..top_n)].0
            };

            grid.set(a.0, a.1, TILE_PIPE);
            grid.set(b.0, b.1, TILE_PIPE);
            used_positions.insert(a);
            used_positions.insert(b);
            placed_pairs.push((a, b));
        } else if must_connect_target {
            break; // can't connect anything more but target still unreachable
        } else {
            // No more islands — score candidate pairs and pick from top 5
            let available: Vec<(usize, usize)> = blank_positions
                .iter()
                .copied()
                .filter(|p| !used_positions.contains(p))
                .collect();

            if available.len() < 2 {
                break; // not enough slots
            }

            // Enumerate all candidate pairs and score them
            let mut candidates: Vec<((usize, usize), (usize, usize), f64)> = Vec::new();
            for i in 0..available.len() {
                for j in (i + 1)..available.len() {
                    let a = available[i];
                    let b = available[j];
                    let score = score_pipe_pair(
                        grid, a, b, &used_positions, &walk.distances, target_pos,
                    );
                    candidates.push((a, b, score));
                }
            }

            // Sort descending by score, pick randomly from top 5
            candidates.sort_by(|x, y| y.2.partial_cmp(&x.2).unwrap_or(std::cmp::Ordering::Equal));
            let top_n = candidates.len().min(5);
            let pick = rng.random_range(..top_n);
            let (a, b, _) = candidates[pick];

            grid.set(a.0, a.1, TILE_PIPE);
            grid.set(b.0, b.1, TILE_PIPE);
            used_positions.insert(a);
            used_positions.insert(b);
            placed_pairs.push((a, b));
        }
    }

    placed_pairs
}

// ---------------------------------------------------------------------------
// Step 2: BFS sectioning
// ---------------------------------------------------------------------------

/// Divide reachable blank slots into N sections by BFS distance from start.
fn bfs_section(
    grid: &Grid,
    pipe_pairs: &[((usize, usize), (usize, usize))],
    start_pos: Option<(usize, usize)>,
    blank_positions: &[(usize, usize)],
    pipe_positions: &HashSet<(usize, usize)>,
    fixed_positions: &HashSet<(usize, usize)>,
    section_count: usize,
) -> Vec<Vec<(usize, usize)>> {
    if section_count == 0 {
        return vec![blank_positions
            .iter()
            .copied()
            .filter(|p| !pipe_positions.contains(p))
            .collect()];
    }

    // BFS-order all reachable positions
    let ordered = bfs_ordered(grid, pipe_pairs, start_pos);

    // Filter to only blank slots that aren't used by pipes or fixed entries
    let assignable: Vec<(usize, usize)> = ordered
        .iter()
        .map(|&(pos, _)| pos)
        .filter(|p| {
            blank_positions.contains(p)
                && !pipe_positions.contains(p)
                && !fixed_positions.contains(p)
        })
        .collect();

    if assignable.is_empty() {
        return vec![assignable];
    }

    // Divide into roughly equal sections
    let per_section = assignable.len() / section_count;
    let extra = assignable.len() % section_count;

    let mut sections = Vec::with_capacity(section_count);
    let mut offset = 0;
    for i in 0..section_count {
        let size = per_section + if i < extra { 1 } else { 0 };
        sections.push(assignable[offset..offset + size].to_vec());
        offset += size;
    }

    sections
}

// ---------------------------------------------------------------------------
// Step 3: Populate sections
// ---------------------------------------------------------------------------

fn populate_sections<R: Rng>(
    grid: &Grid,
    sections: &[Vec<(usize, usize)>],
    fort_count: usize,
    level_count: usize,
    pipe_positions: &HashSet<(usize, usize)>,
    bfs_distances: &HashMap<(usize, usize), usize>,
    reverse_bfs: &HashMap<(usize, usize), usize>,
    target_bfs_dist: Option<usize>,
    world_idx: usize,
    rng: &mut R,
) -> Vec<SlotAssignment> {
    let mut slots = Vec::new();

    // Two separate sets:
    // 1. `completable` — all completion-unsafe tiles on the grid. Used for
    //    the row 7/8 hard constraint (game engine bug). Includes spades,
    //    airships, etc.
    // 2. `scored` — only levels and fortresses we've placed. Used by the
    //    scoring function to spread levels apart. Excludes spades, pipes,
    //    and other non-clumping tiles.
    let mut completable: HashSet<(usize, usize)> = HashSet::new();
    for r in 0..grid.rows {
        for c in 0..grid.cols {
            let t = grid.get(r, c);
            if is_completion_unsafe(t) {
                completable.insert((r, c));
            }
        }
    }
    let mut scored: HashSet<(usize, usize)> = HashSet::new();

    // Add pipe slots (not in sections, but tracked)
    for &pos in pipe_positions {
        slots.push(SlotAssignment {
            pos,
            kind: SlotKind::Pipe,
            section: 0, // pipes don't really belong to a section
        });
    }

    // Phase 1: Place one fortress per section.
    // Track which positions are fortresses so we exclude them from level candidates.
    let mut fort_positions: HashSet<(usize, usize)> = HashSet::new();

    for (si, section) in sections.iter().enumerate() {
        if section.is_empty() || si >= fort_count {
            continue;
        }

        // Score all candidates in this section, filtering row 7/8 conflicts.
        let mut candidates: Vec<((usize, usize), f64)> = section
            .iter()
            .filter(|pos| !is_row78_conflict(**pos, &completable))
            .map(|&pos| {
                (pos, score_fortress_candidate(grid, pos, &scored, bfs_distances, world_idx))
            })
            .collect();

        // Sort descending by score.
        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Pick randomly from top 5 (or fewer if section is small).
        // Fallback: if no candidates passed the row78 filter, pick any slot.
        let pos = if candidates.is_empty() {
            section[rng.random_range(..section.len())]
        } else {
            let top_n = candidates.len().min(5);
            candidates[rng.random_range(..top_n)].0
        };

        completable.insert(pos);
        scored.insert(pos);
        fort_positions.insert(pos);
        slots.push(SlotAssignment {
            pos,
            kind: SlotKind::Fortress,
            section: si,
        });
    }

    // Phase 2: Place levels globally across all sections using score-based
    // picking. Candidates are all non-fortress positions from every section.
    let mut global_candidates: Vec<((usize, usize), usize)> = Vec::new(); // (pos, section_idx)
    for (si, section) in sections.iter().enumerate() {
        for &pos in section {
            if !fort_positions.contains(&pos) {
                global_candidates.push((pos, si));
            }
        }
    }

    let mut level_positions: HashSet<(usize, usize)> = HashSet::new();

    for _ in 0..level_count {
        let best = global_candidates
            .iter()
            .filter(|(pos, _)| !level_positions.contains(pos))
            .filter(|(pos, _)| !is_row78_conflict(*pos, &completable))
            .max_by(|(a, _), (b, _)| {
                let sa = score_candidate(grid, *a, &scored, bfs_distances, reverse_bfs, target_bfs_dist);
                let sb = score_candidate(grid, *b, &scored, bfs_distances, reverse_bfs, target_bfs_dist);
                sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
            });

        match best {
            Some(&(pos, _)) => {
                level_positions.insert(pos);
                completable.insert(pos);
                scored.insert(pos);
            }
            None => break,
        }
    }

    // Phase 3: Emit remaining slots — levels and hammer bros.
    for (si, section) in sections.iter().enumerate() {
        for &pos in section {
            if fort_positions.contains(&pos) {
                continue; // already emitted in phase 1
            }
            if level_positions.contains(&pos) {
                slots.push(SlotAssignment {
                    pos,
                    kind: SlotKind::Level,
                    section: si,
                });
            } else {
                slots.push(SlotAssignment {
                    pos,
                    kind: SlotKind::HammerBro,
                    section: si,
                });
            }
        }
    }

    slots
}

/// Returns true if a node position has exactly one traversable exit direction.
/// Dead-end positions look better with a level or fortress than as blank tiles.
fn is_dead_end(grid: &Grid, pos: (usize, usize)) -> bool {
    let (r, c) = pos;
    let mut exits = 0;
    if c >= 2 && VALID_HORZ.contains(&grid.get(r, c - 1)) { exits += 1; }
    if c + 2 < grid.cols && VALID_HORZ.contains(&grid.get(r, c + 1)) { exits += 1; }
    if r >= 2 && VALID_VERT.contains(&grid.get(r - 1, c)) { exits += 1; }
    if r + 2 < grid.rows && VALID_VERT.contains(&grid.get(r + 1, c)) { exits += 1; }
    exits == 1
}

/// Returns true if placing a completable tile at `pos` would create a
/// row 7/8 completion-bit collision. This is a hard game engine constraint
/// (shared bit $01) that cannot be relaxed.
fn is_row78_conflict(
    pos: (usize, usize),
    completable: &HashSet<(usize, usize)>,
) -> bool {
    let (r, c) = pos;
    if r == 7 {
        completable.contains(&(8, c))
    } else if r == 8 {
        completable.contains(&(7, c))
    } else {
        false
    }
}

// ---------------------------------------------------------------------------
// Level placement scoring
// ---------------------------------------------------------------------------

/// Core scoring logic shared by level and fortress placement.
fn score_with_weights(
    grid: &Grid,
    pos: (usize, usize),
    completable: &HashSet<(usize, usize)>,
    bfs_distances: &HashMap<(usize, usize), usize>,
    dead_end_bonus_value: f64,
) -> f64 {
    let (r, c) = pos;
    let my_bfs = bfs_distances.get(&pos).copied().unwrap_or(0);

    const W_MANHATTAN: f64 = 1.0;    // visual/spatial spread
    const W_BFS: f64 = 1.5;          // traversal spread (weighted higher than grid distance)
    const W_DENSITY: f64 = 3.0;      // penalty per nearby completable tile
    const DENSITY_RADIUS: usize = 4; // combined manhattan+BFS distance threshold
    const SEP_CAP: f64 = 8.0;        // max separation contribution per metric

    let min_manhattan = completable
        .iter()
        .map(|&(cr, cc)| r.abs_diff(cr) + c.abs_diff(cc))
        .min()
        .unwrap_or(usize::MAX);
    let manhattan_score = (min_manhattan as f64).min(SEP_CAP) * W_MANHATTAN;

    let min_bfs_diff = completable
        .iter()
        .filter_map(|p| bfs_distances.get(p))
        .map(|&d| my_bfs.abs_diff(d))
        .min()
        .unwrap_or(usize::MAX);
    let bfs_score = (min_bfs_diff as f64).min(SEP_CAP) * W_BFS;

    let nearby = completable
        .iter()
        .filter(|&&(cr, cc)| {
            let manhattan = r.abs_diff(cr) + c.abs_diff(cc);
            let bfs_diff = bfs_distances
                .get(&(cr, cc))
                .map(|&d| my_bfs.abs_diff(d))
                .unwrap_or(manhattan);
            manhattan.max(bfs_diff) <= DENSITY_RADIUS
        })
        .count();
    let density_penalty = nearby as f64 * W_DENSITY;

    let dead_end_bonus = if is_dead_end(grid, pos) { dead_end_bonus_value } else { 0.0 };

    manhattan_score + bfs_score + dead_end_bonus - density_penalty
}

/// Path relevance: max detour (in BFS hops) that still earns a bonus.
const PATH_DETOUR_CAP: f64 = 6.0;
/// Path relevance weight. Max bonus = PATH_DETOUR_CAP * W_PATH = 3.0.
const W_PATH: f64 = 0.5;

/// Score a candidate position for level placement. Higher = better.
/// Includes a path relevance bonus: positions on the main start→target
/// route (low detour) score higher than side-branch positions.
fn score_candidate(
    grid: &Grid,
    pos: (usize, usize),
    completable: &HashSet<(usize, usize)>,
    bfs_distances: &HashMap<(usize, usize), usize>,
    reverse_bfs: &HashMap<(usize, usize), usize>,
    target_bfs_dist: Option<usize>,
) -> f64 {
    let base = score_with_weights(grid, pos, completable, bfs_distances, 0.5);

    // Path relevance: detour = dist(start→pos) + dist(pos→target) - dist(start→target).
    // Zero detour = perfectly on the shortest path. Higher detour = side branch.
    let path_bonus = match (target_bfs_dist, reverse_bfs.get(&pos)) {
        (Some(target_dist), Some(&rev_d)) => {
            let fwd_d = bfs_distances.get(&pos).copied().unwrap_or(0);
            let detour = (fwd_d + rev_d).saturating_sub(target_dist);
            (PATH_DETOUR_CAP - (detour as f64).min(PATH_DETOUR_CAP)) * W_PATH
        }
        _ => 0.0,
    };

    base + path_bonus
}

/// Score a candidate position for fortress placement. Higher = better.
/// Fortresses get a larger dead-end bonus (+5.0) since they naturally
/// belong at path termini, plus a bonus for designated island positions.
fn score_fortress_candidate(
    grid: &Grid,
    pos: (usize, usize),
    completable: &HashSet<(usize, usize)>,
    bfs_distances: &HashMap<(usize, usize), usize>,
    world_idx: usize,
) -> f64 {
    let base = score_with_weights(grid, pos, completable, bfs_distances, 5.0);
    let island_bonus = if FORTRESS_BONUS_POSITIONS.iter().any(|&(wi, p)| wi == world_idx && p == pos) {
        FORTRESS_BONUS
    } else {
        0.0
    };
    base + island_bonus
}

/// Target proximity penalty weight. Higher = more aggressively avoids placing
/// pipes near the airship/Bowser. Tweakable for tuning.
const W_TARGET_PROXIMITY: f64 = 5.0;
/// Max manhattan distance for target penalty normalization.
const TARGET_MAX_DIST: f64 = 20.0;
/// Cap on spread contribution for pipe scoring. Positions beyond this
/// effective spread all score the same, preventing far-away positions
/// from always dominating and creating more varied placement.
const PIPE_SPREAD_CAP: f64 = 8.0;

/// Compute target proximity penalty for a position. Positions near the
/// airship/Bowser get penalized; positions far away get no penalty.
fn target_proximity_penalty(pos: (usize, usize), target_pos: Option<(usize, usize)>) -> f64 {
    if let Some(tp) = target_pos {
        let dist = (pos.0.abs_diff(tp.0) + pos.1.abs_diff(tp.1)) as f64;
        W_TARGET_PROXIMITY * (TARGET_MAX_DIST - dist.min(TARGET_MAX_DIST)) / TARGET_MAX_DIST
    } else {
        0.0
    }
}

/// Score a single pipe endpoint. Higher = better.
/// Includes spread from existing pipes (capped), dead-end bonus, and target penalty.
fn score_pipe_endpoint(
    grid: &Grid,
    pos: (usize, usize),
    pipe_positions: &HashSet<(usize, usize)>,
    bfs_distances: &HashMap<(usize, usize), usize>,
    target_pos: Option<(usize, usize)>,
) -> f64 {
    let base = score_with_weights(grid, pos, pipe_positions, bfs_distances, 1.0);
    let capped = base.min(PIPE_SPREAD_CAP);
    capped - target_proximity_penalty(pos, target_pos)
}

/// Score a candidate pipe pair. Higher = better.
/// Rewards spread from already-placed pipes, separation between endpoints,
/// and penalizes proximity to the airship/Bowser target.
fn score_pipe_pair(
    grid: &Grid,
    a: (usize, usize),
    b: (usize, usize),
    pipe_positions: &HashSet<(usize, usize)>,
    bfs_distances: &HashMap<(usize, usize), usize>,
    target_pos: Option<(usize, usize)>,
) -> f64 {
    let spread_a = score_pipe_endpoint(grid, a, pipe_positions, bfs_distances, target_pos);
    let spread_b = score_pipe_endpoint(grid, b, pipe_positions, bfs_distances, target_pos);
    let separation = ((a.0.abs_diff(b.0) + a.1.abs_diff(b.1)) as f64 * 0.5).min(10.0);
    spread_a + spread_b + separation
}

// ---------------------------------------------------------------------------
// Step 4: Lock placement
// ---------------------------------------------------------------------------

fn place_locks<R: Rng>(
    grid: &Grid,
    pipe_pairs: &[((usize, usize), (usize, usize))],
    start_pos: Option<(usize, usize)>,
    target_pos: Option<(usize, usize)>,
    slots: &[SlotAssignment],
    fort_count: usize,
    lock_counter: &mut usize,
    rng: &mut R,
) -> Vec<LockAssignment> {
    let mut locks: Vec<LockAssignment> = Vec::new();
    let mut locked_tiles: HashSet<(usize, usize)> = HashSet::new();

    // Build a base grid with forts/levels stamped so BFS sees them as nodes.
    // This grid does NOT have any locks on it.
    let mut base_grid = grid.clone();
    for slot in slots {
        match slot.kind {
            SlotKind::Fortress => base_grid.set(slot.pos.0, slot.pos.1, TILE_FORTRESS),
            SlotKind::Level => {
                let tile = base_grid.get(slot.pos.0, slot.pos.1);
                if BACKGROUND_TILES.contains(&tile) {
                    base_grid.set(slot.pos.0, slot.pos.1, TILE_NODE);
                }
            }
            SlotKind::Pipe => {} // already stamped on grid
            SlotKind::HammerBro => {} // blank path tile, no stamp needed
        }
    }

    // Process each fortress in section order
    for section_idx in 0..fort_count {
        let fort_pos = match slots
            .iter()
            .find(|s| s.section == section_idx && s.kind == SlotKind::Fortress)
        {
            Some(s) => s.pos,
            None => continue,
        };

        // Build the "current state" grid: base grid + all previously placed locks
        // + all locks from earlier sections opened (simulating progression).
        // When checking section N's lock, sections 0..N-1 have been beaten,
        // so their locks are open. The new lock we're testing is the only closed one.
        let build_test_grid = |new_lock: Option<((usize, usize), u8)>| -> Grid {
            let mut g = base_grid.clone();
            // Place all previously committed locks
            for prev in &locks {
                if prev.fort_section < section_idx {
                    // Earlier section — fort beaten, lock opened (restore path tile)
                    g.set(prev.pos.0, prev.pos.1, prev.replace_tile);
                } else {
                    // Same or later section — lock still closed
                    g.set(prev.pos.0, prev.pos.1, prev.gap_tile);
                }
            }
            // Place the candidate lock
            if let Some((pos, gap)) = new_lock {
                g.set(pos.0, pos.1, gap);
            }
            g
        };

        // Find all lockable path tiles not yet used
        let reference_grid = build_test_grid(None);
        let mut candidates: Vec<(usize, usize)> = Vec::new();
        for r in 0..reference_grid.rows {
            for c in 0..reference_grid.cols {
                let tile = reference_grid.get(r, c);
                if LOCKABLE_TILES.contains(&tile) && !locked_tiles.contains(&(r, c)) {
                    // Row 7 and row 8 share Map_Completions bit ($01).
                    // A lock/bridge/gap is completion-unsafe — it would
                    // prevent the fallthrough between rows 7 and 8.
                    // Skip if the paired row has a completable slot.
                    if r == 7 || r == 8 {
                        let paired_row = if r == 7 { 8 } else { 7 };
                        let pair_completable = slots.iter().any(|s| {
                            s.pos == (paired_row, c)
                                && matches!(s.kind, SlotKind::Level | SlotKind::Fortress | SlotKind::Pipe)
                        });
                        if pair_completable {
                            continue;
                        }
                    }
                    candidates.push((r, c));
                }
            }
        }

        candidates.shuffle(rng);

        let prefer_safe = *lock_counter % 4 == 3;
        let mut best: Option<((usize, usize), u8, u8, i32, bool)> = None;
        let mut best_safe: Option<((usize, usize), u8, u8, i32)> = None;

        for &cand_pos in &candidates {
            let tile = reference_grid.get(cand_pos.0, cand_pos.1);
            let gap = gap_tile_for(tile);

            // Hard rule 1: with this lock placed (and earlier locks opened),
            // the current fortress must still be reachable from start.
            let test_grid = build_test_grid(Some((cand_pos, gap)));
            let walk = walk_map(&test_grid, pipe_pairs, start_pos);

            if !walk.nodes.contains(&fort_pos) {
                continue;
            }

            // Hard rule 2: this lock must not block any earlier fortress.
            // Check each earlier section's fort is reachable when its own
            // lock (and all locks before it) are open but this new lock is closed.
            let blocks_earlier = locks.iter().any(|prev_lock| {
                let prev_fort = slots.iter()
                    .find(|s| s.section == prev_lock.fort_section && s.kind == SlotKind::Fortress);
                if let Some(pf) = prev_fort {
                    // Build grid: open locks up to prev_lock's section, close the rest + candidate
                    let mut g = base_grid.clone();
                    for l in &locks {
                        if l.fort_section < prev_lock.fort_section {
                            g.set(l.pos.0, l.pos.1, l.replace_tile);
                        } else {
                            g.set(l.pos.0, l.pos.1, l.gap_tile);
                        }
                    }
                    // Also place the candidate lock
                    g.set(cand_pos.0, cand_pos.1, gap);
                    let w = walk_map(&g, pipe_pairs, start_pos);
                    !w.nodes.contains(&pf.pos)
                } else {
                    false
                }
            });
            if blocks_earlier {
                continue;
            }

            let mut score: i32 = 0;

            // Soft goal 1: blocks a later fortress
            let blocks_later_fort = slots.iter().any(|s| {
                s.kind == SlotKind::Fortress
                    && s.section > section_idx
                    && !walk.nodes.contains(&s.pos)
            });
            if blocks_later_fort {
                score += 100;
            }

            // Check if target is reachable with this lock closed (used for
            // scoring and 1-F secret exit safety).
            let target_reachable = target_pos
                .map(|tp| walk.nodes.contains(&tp))
                .unwrap_or(true);

            // A "safe" lock blocks nothing important: all fortresses and
            // the target remain reachable. Safe for 1-F secret exit since
            // leaving it closed can never cause a softlock.
            let safe = target_reachable && slots.iter().all(|s| {
                s.kind != SlotKind::Fortress || walk.nodes.contains(&s.pos)
            });

            // Soft goal 2: blocks target (airship/bowser)
            if !target_reachable {
                score += 50;
            }

            // Soft goal 3: longer detour to target = better lock.
            // Compare BFS distance to target with vs without this lock.
            // Only evaluated when goals 1+2 didn't fire, to avoid extra BFS.
            if score == 0 {
                if let Some(tp) = target_pos {
                    let locked_dist = walk.distances.get(&tp).copied();
                    let open_grid = build_test_grid(None);
                    let walk_open = walk_map(&open_grid, pipe_pairs, start_pos);
                    let open_dist = walk_open.distances.get(&tp).copied();
                    if let (Some(ld), Some(od)) = (locked_dist, open_dist) {
                        let detour = ld as i32 - od as i32;
                        if detour > 0 {
                            score += detour;
                        }
                    }
                }
            }

            // Track best overall and best safe separately.
            let dominated = match &best {
                Some((_, _, _, best_score, _)) => score > *best_score,
                None => true,
            };
            if dominated {
                best = Some((cand_pos, gap, tile, score, safe));
            }

            if safe {
                let safe_dominated = match &best_safe {
                    Some((_, _, _, best_score)) => score > *best_score,
                    None => true,
                };
                if safe_dominated {
                    best_safe = Some((cand_pos, gap, tile, score));
                }
            }
        }

        // Every 4th lock prefers the best safe candidate; fall back to best overall.
        let chosen = if prefer_safe {
            best_safe.map(|(pos, gap, replace, score)| (pos, gap, replace, score, true))
                .or(best)
        } else {
            best
        };

        if let Some((pos, gap, replace, _score, safe)) = chosen {
            locked_tiles.insert(pos);
            *lock_counter += 1;
            locks.push(LockAssignment {
                pos,
                gap_tile: gap,
                replace_tile: replace,
                fort_section: section_idx,
                secret_exit_safe: safe,
            });
        }
    }

    locks
}

// ---------------------------------------------------------------------------
// Debug ROM writer
// ---------------------------------------------------------------------------

/// Stamp build results onto the ROM tile grids for visual inspection.
///
/// Writes generic tiles for each slot type so the overworld maps can be
/// viewed in an emulator. The game will crash if you enter any level.
#[allow(dead_code)]
pub(super) fn debug_stamp_rom(rom: &mut crate::rom::Rom, result: &BuildResult) {
    for built in &result.worlds {
        let wi = built.world_idx;

        // First write the cleared grid (with pipes already placed)
        for r in 0..built.grid.rows {
            for c in 0..built.grid.cols {
                let offset = rom_data::map_tile_offset(wi, r, c);
                rom.data[offset] = built.grid.get(r, c);
            }
        }

        // Stamp slot assignments
        let mut level_num: u8 = 1;
        for slot in &built.slots {
            let tile = match slot.kind {
                SlotKind::Level => {
                    // Use numbered map tiles ($03-$0D = levels 1-11, then wrap)
                    let t = 0x02 + level_num.min(13);
                    level_num = level_num.wrapping_add(1);
                    t
                }
                SlotKind::Fortress => TILE_FORTRESS,
                SlotKind::Pipe => TILE_PIPE,
                SlotKind::HammerBro => continue, // keep existing blank path tile
            };
            let offset = rom_data::map_tile_offset(wi, slot.pos.0, slot.pos.1);
            rom.data[offset] = tile;
        }

        // Stamp locks
        for lock in &built.locks {
            let offset = rom_data::map_tile_offset(wi, lock.pos.0, lock.pos.1);
            rom.data[offset] = lock.gap_tile;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rom::Rom;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    fn load_rom() -> Option<Rom> {
        let data = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&data).ok()
    }

    #[test]
    fn test_fortress_redistribution() {
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        for _ in 0..100 {
            let counts = redistribute_fortresses(&mut rng);
            let total: usize = counts.iter().sum();
            assert_eq!(total, 17, "total fortresses must be 17");
            assert_eq!(counts[7], 4, "W8 must keep 4");
            for w in 0..7 {
                assert!(counts[w] >= 1 && counts[w] <= 3,
                    "W{} got {} forts, expected 1-3", w + 1, counts[w]);
            }
        }
    }

    #[test]
    fn test_build_all_worlds() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = NodeCatalog::build(&rom, false);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, true);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let result = build(&rom, &pickup, &catalog, &mut rng);

        assert_eq!(result.worlds.len(), 8);

        for built in &result.worlds {
            let wi = built.world_idx;
            let forts = built.slots.iter().filter(|s| s.kind == SlotKind::Fortress).count();
            let pipes = built.pipe_pairs.len();
            let locks = built.locks.len();

            assert_eq!(forts, result.fort_counts[wi],
                "W{}: fort slots {} != expected {}", wi + 1, forts, result.fort_counts[wi]);
            assert_eq!(pipes, VANILLA_PIPE_PAIRS[wi],
                "W{}: pipe pairs {} != expected {}", wi + 1, pipes, VANILLA_PIPE_PAIRS[wi]);
            assert!(locks <= result.fort_counts[wi],
                "W{}: locks {} > fort count {}", wi + 1, locks, result.fort_counts[wi]);
        }

        let total_levels: usize = result.worlds.iter()
            .map(|b| b.slots.iter().filter(|s| s.kind == SlotKind::Level).count())
            .sum();
        let total_forts: usize = result.worlds.iter()
            .map(|b| b.slots.iter().filter(|s| s.kind == SlotKind::Fortress).count())
            .sum();
        assert_eq!(total_levels, VANILLA_LEVEL_COUNT,
            "total levels {} != {}", total_levels, VANILLA_LEVEL_COUNT);
        assert_eq!(total_forts, 17, "total forts {} != 17", total_forts);
    }

    #[test]
    fn test_locks_dont_block_own_fort() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = NodeCatalog::build(&rom, false);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, true);

        for seed in 0..10 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let result = build(&rom, &pickup, &catalog, &mut rng);

            for built in &result.worlds {
                let start_pos = rom_data::find_start(&built.grid);

                // Build grid with all assignments stamped
                let mut test_grid = built.grid.clone();
                for slot in &built.slots {
                    match slot.kind {
                        SlotKind::Fortress => test_grid.set(slot.pos.0, slot.pos.1, TILE_FORTRESS),
                        SlotKind::Level | SlotKind::Pipe | SlotKind::HammerBro => {}
                    }
                }

                // For each lock, verify its fort is still reachable
                for lock in &built.locks {
                    // Place ALL locks
                    let mut locked_grid = test_grid.clone();
                    for l in &built.locks {
                        locked_grid.set(l.pos.0, l.pos.1, l.gap_tile);
                    }
                    // But open THIS lock (as if its fort was beaten)
                    locked_grid.set(lock.pos.0, lock.pos.1, lock.replace_tile);

                    // Open all locks from earlier sections too
                    for earlier in &built.locks {
                        if earlier.fort_section < lock.fort_section {
                            locked_grid.set(earlier.pos.0, earlier.pos.1, earlier.replace_tile);
                        }
                    }

                    let fort_pos = built.slots.iter()
                        .find(|s| s.section == lock.fort_section && s.kind == SlotKind::Fortress)
                        .map(|s| s.pos);

                    if let Some(fp) = fort_pos {
                        let walk = walk_map(&locked_grid, &built.pipe_pairs, start_pos);
                        assert!(walk.nodes.contains(&fp),
                            "Seed {seed} W{}: lock at {:?} blocks its own fort at {:?}",
                            built.world_idx + 1, lock.pos, fp);
                    }
                }
            }
        }
    }

    #[test]
    #[ignore]
    fn test_dump_debug_rom() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = NodeCatalog::build(&rom, false);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, true);

        for seed in [42, 123, 999] {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let result = build(&rom, &pickup, &catalog, &mut rng);

            let mut rom_copy = Rom::from_bytes(&rom.data).unwrap();
            debug_stamp_rom(&mut rom_copy, &result);

            let filename = format!("debug_build_seed{seed}.nes");
            std::fs::write(&filename, &rom_copy.data).unwrap();

            eprintln!("\n=== Seed {seed} ===");
            for built in &result.worlds {
                let forts = built.slots.iter().filter(|s| s.kind == SlotKind::Fortress).count();
                let levels = built.slots.iter().filter(|s| s.kind == SlotKind::Level).count();
                let pipes = built.pipe_pairs.len();
                let locks = built.locks.len();
                eprintln!(
                    "  W{}: {} forts, {} levels, {} pipe pairs, {} locks",
                    built.world_idx + 1, forts, levels, pipes, locks,
                );
            }
            eprintln!("  Wrote {filename}");
        }
    }

    #[test]
    #[ignore]
    fn test_print_build() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = NodeCatalog::build(&rom, false);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, true);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let result = build(&rom, &pickup, &catalog, &mut rng);

        for built in &result.worlds {
            eprintln!("\n=== World {} ({} sections) ===",
                built.world_idx + 1, built.section_count);

            for (si, section_slots) in (0..built.section_count).map(|si| {
                (si, built.slots.iter().filter(|s| s.section == si).collect::<Vec<_>>())
            }) {
                let fort = section_slots.iter().find(|s| s.kind == SlotKind::Fortress);
                let levels = section_slots.iter().filter(|s| s.kind == SlotKind::Level).count();
                let lock = built.locks.iter().find(|l| l.fort_section == si);

                eprintln!("  Section {si}: {} slots ({} levels, fort at {:?})",
                    section_slots.len(), levels,
                    fort.map(|f| f.pos));
                if let Some(l) = lock {
                    eprintln!("    Lock at {:?} (gap=${:02X}, restore=${:02X})",
                        l.pos, l.gap_tile, l.replace_tile);
                }
            }

            eprintln!("  Pipes: {} pairs", built.pipe_pairs.len());
            for (i, &(a, b)) in built.pipe_pairs.iter().enumerate() {
                eprintln!("    Pair {i}: ({},{}) ↔ ({},{})", a.0, a.1, b.0, b.1);
            }
        }
    }

    #[test]
    #[ignore]
    fn test_measure_shortfalls() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = NodeCatalog::build(&rom, false);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, true);

        let mut level_shortfalls = 0u32;
        let mut lock_shortfalls = 0u32;
        let seeds = 1000;

        for seed in 0..seeds {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let result = build(&rom, &pickup, &catalog, &mut rng);

            let total_levels: usize = result.worlds.iter()
                .map(|b| b.slots.iter().filter(|s| s.kind == SlotKind::Level).count())
                .sum();
            if total_levels < VANILLA_LEVEL_COUNT {
                level_shortfalls += 1;
                let deficit = VANILLA_LEVEL_COUNT - total_levels;
                // Show per-world breakdown
                let mut detail = String::new();
                for built in &result.worlds {
                    let levels = built.slots.iter().filter(|s| s.kind == SlotKind::Level).count();
                    let section_sizes: Vec<usize> = (0..built.section_count)
                        .map(|si| built.slots.iter().filter(|s| s.section == si).count())
                        .collect();
                    if levels < 3 {
                        detail.push_str(&format!(" W{}={levels}(sections={section_sizes:?})", built.world_idx + 1));
                    }
                }
                eprintln!("Seed {seed}: {total_levels}/{VANILLA_LEVEL_COUNT} (-{deficit}){detail}");
            }

            for built in &result.worlds {
                let expected_locks = result.fort_counts[built.world_idx];
                if built.locks.len() < expected_locks {
                    lock_shortfalls += 1;
                    // Find which section(s) are missing locks
                    let placed: HashSet<usize> = built.locks.iter().map(|l| l.fort_section).collect();
                    for si in 0..built.section_count {
                        if !placed.contains(&si) {
                            let section_size = built.slots.iter().filter(|s| s.section == si).count();
                            let fort = built.slots.iter().find(|s| s.section == si && s.kind == SlotKind::Fortress);
                            eprintln!("Seed {seed} W{} section {si}: NO LOCK, section_size={section_size}, fort={:?}, total_slots={}",
                                built.world_idx + 1, fort.map(|f| f.pos),
                                built.slots.len());
                        }
                    }
                }
            }
        }

        // Count seeds with at least one secret_exit_safe lock and
        // track which worlds have safe locks in failing seeds
        let mut safe_count = 0u32;
        let mut no_safe_details: Vec<(u64, [usize; 8])> = Vec::new();
        for seed in 0..seeds {
            let mut rng2 = ChaCha8Rng::seed_from_u64(seed);
            let result2 = build(&rom, &pickup, &catalog, &mut rng2);
            let has_safe = result2.worlds.iter().any(|b| {
                b.locks.iter().any(|l| l.secret_exit_safe)
            });
            if has_safe {
                safe_count += 1;
            } else {
                // For failing seeds, count locks per world to see which have room
                let mut lock_counts = [0usize; 8];
                for b in &result2.worlds {
                    lock_counts[b.world_idx] = b.locks.len();
                }
                no_safe_details.push((seed, lock_counts));
            }
        }

        eprintln!("\n=== {seeds} seeds ===");
        eprintln!("Level shortfalls: {level_shortfalls}/{seeds}");
        eprintln!("Lock shortfalls:  {lock_shortfalls}/{seeds} (world-level)");
        eprintln!("Seeds with >=1 secret_exit_safe lock: {safe_count}/{seeds}");
        if !no_safe_details.is_empty() {
            eprintln!("No-safe seeds (first 10):");
            for (seed, counts) in no_safe_details.iter().take(10) {
                eprintln!("  Seed {seed}: locks per world = {counts:?}");
            }
        }
    }

    #[test]
    #[ignore]
    fn test_w6_slot_distribution() {
        let rom = match load_rom() {
            Some(r) => r,
            None => {
                eprintln!("ROM not found, skipping");
                return;
            }
        };
        let catalog = NodeCatalog::build(&rom, false);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, true);

        for seed in 0..6u64 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let result = build(&rom, &pickup, &catalog, &mut rng);
            let built = &result.worlds[5]; // W6 (0-indexed)

            eprintln!("\n===== Seed {seed} — W6 =====");
            eprintln!("level_count received: {} (from distribute_levels)",
                built.slots.iter().filter(|s| s.kind == SlotKind::Level).count());
            eprintln!("fort_count: {}", result.fort_counts[5]);
            eprintln!("total slots: {}", built.slots.len());
            eprintln!("section_count: {}", built.section_count);
            eprintln!("pipe_pairs: {}", built.pipe_pairs.len());

            // Group by kind
            let mut fortresses = Vec::new();
            let mut levels = Vec::new();
            let mut hammer_bros = Vec::new();
            let mut pipes = Vec::new();
            for slot in &built.slots {
                match slot.kind {
                    SlotKind::Fortress => fortresses.push(slot),
                    SlotKind::Level => levels.push(slot),
                    SlotKind::HammerBro => hammer_bros.push(slot),
                    SlotKind::Pipe => pipes.push(slot),
                }
            }

            eprintln!("\nFortresses ({}):", fortresses.len());
            for s in &fortresses {
                eprintln!("  ({:2}, {:2})  section={}", s.pos.0, s.pos.1, s.section);
            }

            eprintln!("\nLevels ({}):", levels.len());
            for s in &levels {
                // Compute min Manhattan distance to nearest other Level slot
                let min_dist = levels.iter()
                    .filter(|o| o.pos != s.pos)
                    .map(|o| {
                        let dr = (s.pos.0 as isize - o.pos.0 as isize).unsigned_abs();
                        let dc = (s.pos.1 as isize - o.pos.1 as isize).unsigned_abs();
                        dr + dc
                    })
                    .min()
                    .unwrap_or(0);
                eprintln!("  ({:2}, {:2})  section={}  min_dist_to_level={}", s.pos.0, s.pos.1, s.section, min_dist);
            }

            eprintln!("\nHammerBros ({}):", hammer_bros.len());
            for s in &hammer_bros {
                eprintln!("  ({:2}, {:2})  section={}", s.pos.0, s.pos.1, s.section);
            }

            eprintln!("\nPipes ({}):", pipes.len());
            for s in &pipes {
                eprintln!("  ({:2}, {:2})  section={}", s.pos.0, s.pos.1, s.section);
            }

            eprintln!("\nLocks ({}):", built.locks.len());
            for l in &built.locks {
                eprintln!("  ({:2}, {:2})  gap=0x{:02X}  replace=0x{:02X}  fort_section={}  safe={}",
                    l.pos.0, l.pos.1, l.gap_tile, l.replace_tile, l.fort_section, l.secret_exit_safe);
            }
        }
    }

    #[test]
    #[ignore]
    fn test_dump_w7_blank_vs_bfs() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = NodeCatalog::build(&rom, false);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, true);
        let wi = 6; // W7

        let cw = &pickup.worlds[wi];
        eprintln!("\n=== W7 Pickup: {} pool entries ===", cw.pool_indices.len());

        let fixed = fixed_positions_for_world(&rom, &catalog, wi);
        eprintln!("Fixed positions: {} {:?}", fixed.len(), fixed);

        let blank_positions = find_blank_slots(&cw.grid, &fixed);
        eprintln!("Blank tiles on grid: {}", blank_positions.len());

        // Run the actual build for several seeds and check coverage
        for seed in 0..5u64 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let result = build(&rom, &pickup, &catalog, &mut rng);
            let built = &result.worlds[wi];

            // All positions that got a slot assignment
            let slot_positions: HashSet<(usize, usize)> = built.slots.iter().map(|s| s.pos).collect();
            // Add pipe positions
            let pipe_positions: HashSet<(usize, usize)> = built.pipe_pairs.iter()
                .flat_map(|&(a, b)| vec![a, b]).collect();
            let all_assigned: HashSet<(usize, usize)> = slot_positions.union(&pipe_positions).copied().collect();

            // Blank tiles with no assignment
            let uncovered: Vec<(usize, usize)> = blank_positions.iter()
                .filter(|p| !all_assigned.contains(p))
                .copied()
                .collect();

            let total_slots = built.slots.len() + pipe_positions.len();
            eprintln!("\n--- Seed {seed} ---");
            eprintln!("  Slots: {} (L={}, F={}, P={}, HB={})",
                total_slots,
                built.slots.iter().filter(|s| s.kind == SlotKind::Level).count(),
                built.slots.iter().filter(|s| s.kind == SlotKind::Fortress).count(),
                pipe_positions.len(),
                built.slots.iter().filter(|s| s.kind == SlotKind::HammerBro).count(),
            );
            eprintln!("  Pool entries (ptr slots): {}", cw.pool_indices.len());
            eprintln!("  max_non_pipe_slots: {}", cw.pool_indices.len() - VANILLA_PIPE_PAIRS[wi] * 2);
            eprintln!("  Blanks on grid: {}", blank_positions.len());
            eprintln!("  Assigned positions: {}", all_assigned.len());
            eprintln!("  Uncovered blanks: {}", uncovered.len());

            if !uncovered.is_empty() {
                for (r, c) in &uncovered {
                    eprintln!("    UNCOVERED: ({},{}) tile=${:02X}", r, c, cw.grid.get(*r, *c));
                    // Check if BFS can reach it with the placed pipes
                    let bfs_all = bfs_ordered(&built.grid, &built.pipe_pairs, rom_data::find_start(&built.grid));
                    let bfs_set: HashSet<(usize, usize)> = bfs_all.iter().map(|&(p, _)| p).collect();
                    eprintln!("      BFS reachable: {}", bfs_set.contains(&(*r, *c)));
                }
            }

            // Check for assignments NOT on blank tiles (double-covering or wrong pos)
            let non_blank_assignments: Vec<_> = all_assigned.iter()
                .filter(|p| !blank_positions.contains(p) && !pipe_positions.contains(p))
                .collect();
            if !non_blank_assignments.is_empty() {
                eprintln!("  Assignments on non-blank tiles:");
                for &&(r, c) in &non_blank_assignments {
                    eprintln!("    ({},{}) tile=${:02X}", r, c, cw.grid.get(r, c));
                }
            }
        }
    }

}
