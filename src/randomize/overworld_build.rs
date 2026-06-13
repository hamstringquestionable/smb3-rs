//! Phase 3 of the overworld builder rewrite: Build.
//!
//! Takes `PickupResult` (Phase 2) + `NodeCatalog` (Phase 1) + RNG and produces
//! slot assignments for each world. Does NOT assign specific pool entries or
//! write to ROM — that's Phase 4 (writer).
//!
//! Algorithm:
//! 0. Redistribute fortresses across worlds (W8 keeps 4, W1-W7 get 1-3 each)
//! 1. Place pipes (connectivity first, then remaining to connect islands)
//! 2. BFS sectioning (order reachable blanks by distance, divide by fort count)
//! 3. Populate sections (1 fort per section, rest are levels)
//! 4. Lock placement (every fort gets 1 lock)

use std::collections::{HashMap, HashSet};

use rand::Rng;
use rand::seq::{IndexedRandom, SliceRandom};

use super::map_walker::walk_map;
use super::node_catalog::{NodeCatalog, NodeKind};
use super::overworld_helpers::{find_target, gap_tile_for, LOCKABLE_TILES};
use super::overworld_pickup::PickupResult;
use crate::rom::Rom;
use super::rom_data::{
    self, BACKGROUND_TILES, Grid, Pos, TILE_BONUS_GAME, TILE_FORTRESS, TILE_NODE, TILE_PIPE,
    TILE_TOAD_HOUSE, TeleportEdge,
    VALID_HORZ, VALID_VERT,
};

// ---------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------

/// Read-only Phase 1 + 2 outputs that build and writer phases consume together.
/// Both fields are produced by earlier phases and never mutated downstream —
/// bundling them avoids threading two parallel references through every helper.
pub(crate) struct OverworldData<'a> {
    pub pickup: &'a PickupResult,
    pub catalog: &'a NodeCatalog,
}

/// What kind of node occupies a grid slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlotKind {
    Level,
    Fortress,
    Pipe,
    HammerBro,
    BonusGame,
    ToadHouse,
}

/// A single slot assignment on the grid.
#[derive(Clone, Debug)]
pub struct SlotAssignment {
    pub pos: (usize, usize),
    pub kind: SlotKind,
    /// Which section (0-based) this slot belongs to.
    pub section: usize,
    /// When true, the writer stamps a HANDTRAP tile (0xE6) at this slot
    /// instead of a level-number tile. Only set on `SlotKind::Level` slots.
    pub is_hand_trap: bool,
    /// When true, the writer stamps a PIPE tile (0xBC) at this slot instead
    /// of a level-number tile. Only set on `SlotKind::Level` slots. The
    /// slot's level pointer entry is unchanged; pressing A on the pipe-look
    /// tile drops the player into the underlying level (uniform Map_Op = $10
    /// dispatch — no pipe-transit state).
    pub is_troll_pipe: bool,
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
    /// True if this lock makes the target unreachable when closed. Used to
    /// suppress redundant target-blocking bonuses for subsequent locks in
    /// the same world (avoids piling multiple locks against the airship).
    pub blocks_target: bool,
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
    pub pipe_pairs: Vec<TeleportEdge>,
}

/// Complete Phase 3 output.
#[derive(Clone)]
pub(crate) struct BuildResult {
    pub worlds: Vec<BuiltWorld>,
    /// Fortress counts per world (decided in Step 0).
    #[allow(dead_code)] // read in tests
    pub fort_counts: [usize; 8],
}

/// Sample a candidate weighted by softmax(score / temperature). Higher
/// temperature flattens the distribution (more random); lower temperature
/// concentrates probability on top-scoring candidates. Returns `None` if empty.
fn pick_softmax_by_score<T, R: Rng>(
    candidates: Vec<(T, f64)>,
    temperature: f64,
    rng: &mut R,
) -> Option<T> {
    if candidates.is_empty() {
        return None;
    }
    // Subtract max for numerical stability.
    let max_score = candidates
        .iter()
        .map(|(_, s)| *s)
        .fold(f64::NEG_INFINITY, f64::max);
    let weights: Vec<f64> = candidates
        .iter()
        .map(|(_, s)| ((s - max_score) / temperature).exp())
        .collect();
    let total: f64 = weights.iter().sum();
    let mut roll = rng.random_range(0.0..total);
    for (i, w) in weights.iter().enumerate() {
        roll -= w;
        if roll <= 0.0 {
            return Some(candidates.into_iter().nth(i).unwrap().0);
        }
    }
    // Floating point edge case — return last.
    Some(candidates.into_iter().last().unwrap().0)
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
    data: &OverworldData,
    rng: &mut R,
    shuffle_toad_houses: bool,
) -> BuildResult {
    let pickup = data.pickup;
    let catalog = data.catalog;
    // Step 0: redistribute fortresses
    let fort_counts = redistribute_fortresses(rng);

    // Build patched grids once: clone pickup grids and restore Airship/Bowser
    // tiles (blanked during pickup but kept at vanilla positions). The Start
    // tile is also restored here so that worlds where the start ↔ airship
    // swap fired have the two tiles in their new (swapped) positions before
    // BFS/lock placement runs. For un-swapped worlds the Start restore is a
    // no-op rewrite of the same byte at its vanilla position.
    let mut patched_grids: Vec<Grid> = Vec::with_capacity(8);
    for wi in 0..8 {
        let mut grid = pickup.worlds[wi].grid.clone();
        for entry in &catalog.entries {
            if entry.world_idx != wi {
                continue;
            }
            if matches!(
                entry.kind,
                NodeKind::Airship | NodeKind::Bowser | NodeKind::Start
            ) {
                let (r, c) = entry.grid_pos;
                if r < grid.rows && c < grid.cols {
                    grid.set(r, c, entry.tile);
                }
            }
        }
        super::start_airship_swap::swap_tiles_above(&mut grid, wi, catalog);
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
        .map(|wi| fixed_positions_for_world(rom, catalog, wi, shuffle_toad_houses))
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

    let mut worlds = Vec::with_capacity(8);
    for wi in 0..8 {
        // Max non-pipe slots = pointer table slots minus pipe endpoints.
        // This caps the total fort+level+HB entries to what the pointer table
        // can actually hold. Excess blank tiles stay as path nodes.
        let ptr_slots = pickup.worlds[wi].pool_indices.len();
        let pipe_endpoints = VANILLA_PIPE_PAIRS[wi] * 2;
        let max_non_pipe_slots = ptr_slots.saturating_sub(pipe_endpoints);

        let counts = WorldSlotCounts {
            fort_count: fort_counts[wi],
            level_count: level_counts[wi],
            pipe_pair_count: VANILLA_PIPE_PAIRS[wi],
            max_non_pipe_slots,
            force_safe: false,
        };
        let built = build_world(
            wi,
            rom,
            patched_grids[wi].clone(),
            &fixed_positions[wi],
            &counts,
            rng,
        );
        worlds.push(built);
    }

    // Ensure at least one lock across all worlds is secret-exit-safe.
    // The score-based prefer_safe usually produces one (triggers when
    // best_score < 5), but if not, retry with force_safe=true.
    let has_safe = worlds.iter().any(|b| b.locks.iter().any(|l| l.secret_exit_safe));
    if !has_safe {
        let mut retry_order: Vec<usize> = (0..8).collect();
        retry_order.shuffle(rng);
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
                true, // force_safe
                rng,
            );
            if new_locks.iter().any(|l| l.secret_exit_safe) {
                worlds[wi].locks = new_locks;
                break;
            }
        }
    }

    // Toad houses promote first so the smaller, less flexible 22-entry budget
    // lands before spades scramble for the remaining HammerBro slots.
    promote_hb_slots(
        rom, &mut worlds, data, rng,
        |k| matches!(k, NodeKind::ToadHouse), SlotKind::ToadHouse, None,
    );
    promote_hb_slots(
        rom, &mut worlds, data, rng,
        |k| matches!(k, NodeKind::BonusGame), SlotKind::BonusGame, Some(SPADE_BUDGET),
    );

    BuildResult { worlds, fort_counts }
}

// ---------------------------------------------------------------------------
// Step 5: HammerBro slot promotion (Toad Houses + spade games)
// ---------------------------------------------------------------------------

const SPADE_BUDGET: usize = 19;

/// Promote HammerBro slots to a target `SlotKind`, distributing picked-up pool
/// entries of the matching `NodeKind` across worlds in proportion to each
/// world's available HammerBro slot count.
///
/// Runs after lock placement so reachability constraints are already satisfied;
/// no-ops when the pickup pool contains no matching entries. `budget_cap`
/// limits how many entries are placed (spades cap at SPADE_BUDGET; toad houses
/// place every entry).
fn promote_hb_slots<R: Rng>(
    rom: &Rom,
    worlds: &mut [BuiltWorld],
    data: &OverworldData,
    rng: &mut R,
    matches_source: impl Fn(&NodeKind) -> bool,
    target_kind: SlotKind,
    budget_cap: Option<usize>,
) {
    let mut source_count = data
        .pickup
        .pool
        .iter()
        .filter(|pe| matches_source(&data.catalog.entries[pe.catalog_idx].kind))
        .count();
    if let Some(cap) = budget_cap {
        source_count = source_count.min(cap);
    }
    if source_count == 0 {
        return;
    }

    let mut candidates_by_world: Vec<Vec<(usize, usize)>> = Vec::with_capacity(worlds.len());
    for (wi, w) in worlds.iter().enumerate() {
        let sprite_positions: HashSet<(usize, usize)> = rom_data::read_hb_sprite_positions(rom, wi)
            .into_iter()
            .collect();

        let completable = completable_positions(&w.grid, &w.slots);

        let mut cands: Vec<(usize, usize)> = w
            .slots
            .iter()
            .filter(|s| s.kind == SlotKind::HammerBro)
            .map(|s| s.pos)
            .filter(|p| !sprite_positions.contains(p))
            .filter(|p| !is_row78_conflict(*p, &completable))
            .collect();
        cands.shuffle(rng);
        candidates_by_world.push(cands);
    }

    let total_cands: usize = candidates_by_world.iter().map(|c| c.len()).sum();
    if total_cands == 0 {
        return;
    }

    let target = source_count.min(total_cands);
    let mut budget = vec![0usize; worlds.len()];
    for wi in 0..worlds.len() {
        let frac = candidates_by_world[wi].len() as f64 / total_cands as f64;
        budget[wi] = ((frac * target as f64).round() as usize).min(candidates_by_world[wi].len());
    }

    // Rounding may leave us over/under. Trim or pad until we match `target`.
    let mut allocated: usize = budget.iter().sum();
    while allocated > target {
        let wi = budget
            .iter()
            .enumerate()
            .filter(|&(_, b)| *b > 0)
            .max_by_key(|&(_, b)| *b)
            .map(|(i, _)| i)
            .unwrap();
        budget[wi] -= 1;
        allocated -= 1;
    }
    while allocated < target {
        let wi = (0..worlds.len())
            .filter(|&i| candidates_by_world[i].len() > budget[i])
            .max_by_key(|&i| candidates_by_world[i].len() - budget[i]);
        match wi {
            Some(wi) => {
                budget[wi] += 1;
                allocated += 1;
            }
            None => break,
        }
    }

    for (wi, w) in worlds.iter_mut().enumerate() {
        let chosen: HashSet<(usize, usize)> = candidates_by_world[wi]
            .iter()
            .take(budget[wi])
            .copied()
            .collect();
        for slot in w.slots.iter_mut() {
            if slot.kind == SlotKind::HammerBro && chosen.contains(&slot.pos) {
                slot.kind = target_kind.clone();
            }
        }
    }
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
    shuffle_toad_houses: bool,
) -> HashSet<(usize, usize)> {
    let mut fixed = HashSet::new();

    // Airship, Bowser stay at vanilla positions unconditionally. Toad Houses
    // stay pinned only when shuffle_toad_houses is off; when on, the build
    // phase places them at promoted HammerBro slots.
    for entry in &catalog.entries {
        if entry.world_idx != world_idx {
            continue;
        }
        match entry.kind {
            NodeKind::Airship | NodeKind::Bowser => {
                fixed.insert(entry.grid_pos);
            }
            NodeKind::ToadHouse if !shuffle_toad_houses => {
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

/// Per-world numeric budgets passed into `build_world`. All five fields are
/// computed in `build()` from pickup capacity, vanilla pipe counts, and the
/// redistributed fortress counts.
struct WorldSlotCounts {
    fort_count: usize,
    level_count: usize,
    pipe_pair_count: usize,
    max_non_pipe_slots: usize,
    force_safe: bool,
}

fn build_world<R: Rng>(
    world_idx: usize,
    rom: &Rom,
    mut grid: Grid,
    fixed_positions: &HashSet<(usize, usize)>,
    counts: &WorldSlotCounts,
    rng: &mut R,
) -> BuiltWorld {
    let fort_count = counts.fort_count;
    let level_count = counts.level_count;
    let pipe_pair_count = counts.pipe_pair_count;
    let max_non_pipe_slots = counts.max_non_pipe_slots;
    let force_safe = counts.force_safe;
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
        fixed_positions,
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
            is_hand_trap: false,
            is_troll_pipe: false,
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
        force_safe,
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

    // 0x67/0xEB/0x6A are also caught by the threshold check below, but kept
    // explicit here for readability — fortress tiles are the primary case.
    if SPECIAL.contains(&tile) || tile == 0x67 || tile == 0xEB || tile == 0x6A {
        return true;
    }
    let page = (tile >> 6) as usize;
    if tile >= THRESHOLDS[page] {
        return true;
    }
    REMOVABLE.contains(&tile)
}

/// Collect positions whose tile/slot would be "caught" by the game's
/// completion-check routine — the input to `is_row78_conflict`. This covers
/// both completion-unsafe grid tiles and placed Level/Fortress/BonusGame slots
/// (which will be stamped as completion-unsafe tiles by the writer).
fn completable_positions(grid: &Grid, slots: &[SlotAssignment]) -> HashSet<(usize, usize)> {
    let mut set: HashSet<(usize, usize)> = HashSet::new();
    for r in 0..grid.rows {
        for c in 0..grid.cols {
            if is_completion_unsafe(grid.get(r, c)) {
                set.insert((r, c));
            }
        }
    }
    for s in slots {
        if matches!(
            s.kind,
            SlotKind::Level | SlotKind::Fortress | SlotKind::BonusGame | SlotKind::ToadHouse
        ) {
            set.insert(s.pos);
        }
    }
    set
}

/// BFS from start, returning nodes in visit order with their distances.
/// BFS-ordered list of (position, distance) using the canonical `walk_map`.
///
/// This is the single source of truth for map traversal — all BFS-dependent
/// logic (sectioning, scoring, connectivity checks) must go through here or
/// `walk_map` directly to stay in sync with canoe edges, pipe teleports, etc.
pub(super) fn bfs_ordered(
    grid: &Grid,
    pipe_pairs: &[TeleportEdge],
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
    blanks: &[Pos],
    reachable: &HashSet<Pos>,
    used: &HashSet<Pos>,
) -> (Vec<Pos>, Vec<Pos>) {
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
) -> Vec<TeleportEdge> {
    if pair_count == 0 {
        return Vec::new();
    }

    // Hard exclusion: forbid pipe endpoints adjacent (≤1 walking hop) to
    // start or target. Diagnostic on 1000-seed sweeps showed 100% of
    // "trivial bypass" (0 forts + 0 levels) playthroughs were caused by
    // pipes sitting next to start, next to target, or both — eliminating
    // both ends of that pattern eliminates the failure mode. Fixed
    // endpoints (W3 boat dock) are exempt: their position is dictated by
    // ROM data, not chosen by the builder.
    let no_pipe_zone: HashSet<(usize, usize)> = {
        let mut zone: HashSet<(usize, usize)> = HashSet::new();
        for anchor in [start_pos, target_pos].into_iter().flatten() {
            let walk = walk_map(grid, &[], Some(anchor));
            for (&pos, &d) in &walk.distances {
                if d <= 1 {
                    zone.insert(pos);
                }
            }
        }
        // Fixed endpoints stay placeable even inside the zone.
        for &fp in fixed_endpoints {
            zone.remove(&fp);
        }
        zone
    };
    // Shadow the parameter with the filtered set so every candidate site
    // below (phase 0 partner, island connections, no-more-islands pairs)
    // automatically respects the zone.
    let blank_positions: Vec<(usize, usize)> = blank_positions
        .iter()
        .copied()
        .filter(|p| !no_pipe_zone.contains(p))
        .collect();
    let blank_positions = blank_positions.as_slice();

    let mut placed_pairs: Vec<TeleportEdge> = Vec::new();
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
    let target_reachable = |g: &Grid, pairs: &[TeleportEdge]| -> bool {
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
            let b_scored: Vec<((usize, usize), f64)> = unreachable_blanks
                .iter()
                .map(|&pos| {
                    let start_dist = (pos.0.abs_diff(start.0) + pos.1.abs_diff(start.1)) as f64;
                    // Nearer to start = higher score (invert distance)
                    let proximity_score = (TARGET_MAX_DIST - start_dist.min(TARGET_MAX_DIST)) / TARGET_MAX_DIST * 5.0;
                    let target_pen = target_proximity_penalty(pos, target_pos);
                    (pos, proximity_score - target_pen)
                })
                .collect();
            let b = pick_softmax_by_score(b_scored, PIPE_SOFTMAX_T, rng).unwrap();

            // Reachable side: prefer positions far from start (BFS distance),
            // spread from existing pipes, and away from target.
            let a_scored: Vec<((usize, usize), f64)> = reachable_blanks
                .iter()
                .map(|&pos| {
                    let score = score_pipe_endpoint(
                        grid, pos, &used_positions, &walk.distances, target_pos,
                    );
                    (pos, score)
                })
                .collect();
            let a = pick_softmax_by_score(a_scored, PIPE_SOFTMAX_T, rng).unwrap();

            grid.set(a.0, a.1, TILE_PIPE);
            grid.set(b.0, b.1, TILE_PIPE);
            used_positions.insert(a);
            used_positions.insert(b);
            placed_pairs.push((a, b));
        } else if must_connect_target {
            break; // can't connect anything more but target still unreachable
        } else {
            // No more islands — score candidate pairs and pick from top N
            let available: Vec<(usize, usize)> = blank_positions
                .iter()
                .copied()
                .filter(|p| !used_positions.contains(p))
                .collect();

            if available.len() < 2 {
                break; // not enough slots
            }

            // Enumerate all candidate pairs and score them
            let mut candidates: Vec<(TeleportEdge, f64)> = Vec::new();
            for i in 0..available.len() {
                for j in (i + 1)..available.len() {
                    let a = available[i];
                    let b = available[j];
                    let score = score_pipe_pair(
                        grid, a, b, &used_positions, &walk.distances, target_pos,
                    );
                    candidates.push(((a, b), score));
                }
            }

            let (a, b) = pick_softmax_by_score(candidates, PIPE_SOFTMAX_T, rng).unwrap();

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
    pipe_pairs: &[TeleportEdge],
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

// Reason: 10 args is over clippy's 7-arg default. Candidate bundles
// investigated (`BfsCtx` for the 3 distance args, reusing `WorldSlotCounts`
// for the 2 budget args) — none reveals a concept beyond what the inline
// arg names already convey. Each arg is a distinct input (geometry,
// sections, budgets, pipe positions, BFS data, world, RNG) and bundling
// them would add indirection without clarity.
#[allow(clippy::too_many_arguments)]
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
    // 2. `placed_levels_and_forts` — only levels and fortresses we've placed.
    //    Used by the scoring function to spread levels apart. Excludes spades,
    //    pipes, and other non-clumping tiles.
    let mut completable = completable_positions(grid, &[]);
    let mut placed_levels_and_forts: HashSet<(usize, usize)> = HashSet::new();

    // Add pipe slots (not in sections, but tracked)
    for &pos in pipe_positions {
        slots.push(SlotAssignment {
            pos,
            kind: SlotKind::Pipe,
            section: 0, // pipes don't really belong to a section
            is_hand_trap: false,
            is_troll_pipe: false,
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
        let candidates: Vec<((usize, usize), f64)> = section
            .iter()
            .filter(|pos| !is_row78_conflict(**pos, &completable))
            .map(|&pos| {
                (pos, score_fortress_candidate(grid, pos, &placed_levels_and_forts, bfs_distances, world_idx))
            })
            .collect();

        // Sample by softmax; fallback to any section slot if none passed the row78 filter.
        let pos = pick_softmax_by_score(candidates, FORTRESS_SOFTMAX_T, rng)
            .unwrap_or_else(|| section[rng.random_range(..section.len())]);

        completable.insert(pos);
        placed_levels_and_forts.insert(pos);
        fort_positions.insert(pos);
        slots.push(SlotAssignment {
            pos,
            kind: SlotKind::Fortress,
            section: si,
            is_hand_trap: false,
            is_troll_pipe: false,
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
                let sa = score_candidate(grid, *a, &placed_levels_and_forts, bfs_distances, reverse_bfs, target_bfs_dist);
                let sb = score_candidate(grid, *b, &placed_levels_and_forts, bfs_distances, reverse_bfs, target_bfs_dist);
                sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
            });

        match best {
            Some(&(pos, _)) => {
                level_positions.insert(pos);
                completable.insert(pos);
                placed_levels_and_forts.insert(pos);
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
                    is_hand_trap: false,
                    is_troll_pipe: false,
                });
            } else {
                slots.push(SlotAssignment {
                    pos,
                    kind: SlotKind::HammerBro,
                    section: si,
                    is_hand_trap: false,
                    is_troll_pipe: false,
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
/// Path relevance weight. Max bonus = PATH_DETOUR_CAP * W_PATH = 9.0.
/// Tuned via test_level_placement_quality: 0.5 was decorative (no bias);
/// 3.0 dominated and clumped levels on the route at the expense of spread
/// and dead-ends. 1.5 produces a meaningful route bias without breaking
/// the spread or density terms.
const W_PATH: f64 = 1.5;

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
const W_TARGET_PROXIMITY: f64 = 4.0;
/// Max manhattan distance for target penalty normalization.
const TARGET_MAX_DIST: f64 = 20.0;
/// Cap on the manhattan + BFS spread reward for pipe scoring. Positions
/// beyond this effective spread all score the same, preventing very-far
/// positions from always dominating. Applied to the spread term only —
/// dead-end bonus and density penalty bypass the cap so they always count.
const PIPE_SPREAD_CAP: f64 = 7.0;

/// Softmax temperature for pipe placement. Higher = more random, lower =
/// more concentrated on top-scoring candidates. Tuned for typical pipe
/// score range of ~[-8, +12].
const PIPE_SOFTMAX_T: f64 = 4.0;

/// Softmax temperature for fortress placement. Score range is similar to
/// pipes (~[-12, +15] including the +5 dead-end bonus).
const FORTRESS_SOFTMAX_T: f64 = 4.0;

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
///
/// Spread reward (distance from nearest existing pipe) is capped at
/// PIPE_SPREAD_CAP. Dead-end bonus, density penalty, and target penalty
/// are applied outside the cap so they always influence the score.
///
/// When `pipe_positions` is empty (first pair) the spread term is 0 — every
/// candidate ties on spread, so picking is driven by dead-end + target only.
fn score_pipe_endpoint(
    grid: &Grid,
    pos: (usize, usize),
    pipe_positions: &HashSet<(usize, usize)>,
    bfs_distances: &HashMap<(usize, usize), usize>,
    target_pos: Option<(usize, usize)>,
) -> f64 {
    const W_MANHATTAN: f64 = 1.0;
    const W_BFS: f64 = 1.5;
    const W_DENSITY: f64 = 3.0;
    const DENSITY_RADIUS: usize = 4;
    const SEP_CAP: f64 = 8.0;
    const DEAD_END_BONUS: f64 = 1.0;

    let (r, c) = pos;
    let my_bfs = bfs_distances.get(&pos).copied().unwrap_or(0);

    let spread = if pipe_positions.is_empty() {
        0.0
    } else {
        let min_manhattan = pipe_positions
            .iter()
            .map(|&(cr, cc)| r.abs_diff(cr) + c.abs_diff(cc))
            .min()
            .unwrap();
        let min_bfs_diff = pipe_positions
            .iter()
            .filter_map(|p| bfs_distances.get(p))
            .map(|&d| my_bfs.abs_diff(d))
            .min()
            .unwrap_or(min_manhattan);
        let m = (min_manhattan as f64).min(SEP_CAP) * W_MANHATTAN;
        let b = (min_bfs_diff as f64).min(SEP_CAP) * W_BFS;
        (m + b).min(PIPE_SPREAD_CAP)
    };

    let nearby = pipe_positions
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

    let dead_end_bonus = if is_dead_end(grid, pos) { DEAD_END_BONUS } else { 0.0 };

    spread + dead_end_bonus - density_penalty - target_proximity_penalty(pos, target_pos)
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

// Reason: every argument represents a distinct, independent input to lock
// placement (geometry, slot list, count, safety flag, RNG). No subset
// clusters into a meaningful concept — bundling would be a clippy bandage,
// not a real abstraction.
#[allow(clippy::too_many_arguments)]
fn place_locks<R: Rng>(
    grid: &Grid,
    pipe_pairs: &[TeleportEdge],
    start_pos: Option<(usize, usize)>,
    target_pos: Option<(usize, usize)>,
    slots: &[SlotAssignment],
    fort_count: usize,
    force_safe: bool,
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
            SlotKind::BonusGame => base_grid.set(slot.pos.0, slot.pos.1, TILE_BONUS_GAME),
            SlotKind::ToadHouse => base_grid.set(slot.pos.0, slot.pos.1, TILE_TOAD_HOUSE),
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
                                && matches!(s.kind, SlotKind::Level | SlotKind::Fortress | SlotKind::Pipe | SlotKind::BonusGame | SlotKind::ToadHouse)
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

        // Prefer safe when forced (retry path) or when the best candidate
        // is weak anyway (score < 5) — don't sacrifice a high-scoring lock.
        // Evaluated after scoring all candidates, see below.
        // (pos, gap_tile, replace_tile, score, safe, blocks_target)
        type LockCandidate = (Pos, u8, u8, i32, bool, bool);
        // Subset of LockCandidate without the safe/blocks_target flags.
        type SafeLockCandidate = (Pos, u8, u8, i32);

        let mut best: Option<LockCandidate> = None;
        let mut best_safe: Option<SafeLockCandidate> = None;

        // Open grid (no candidate lock) is constant for all candidates in this
        // section — hoist the BFS to avoid redundant walks per candidate.
        let open_grid = build_test_grid(None);
        let open_node_count = walk_map(&open_grid, pipe_pairs, start_pos).nodes.len() as i32;

        // If a previous lock in this world already blocks the target, suppress
        // the target-blocking bonus to avoid stacking multiple locks against
        // the airship/Bowser.
        let target_already_locked = locks.iter().any(|l| l.blocks_target);

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

            // Check if target is reachable with this lock closed (used for
            // secret exit safety).
            let target_reachable = target_pos
                .map(|tp| walk.nodes.contains(&tp))
                .unwrap_or(true);

            // A "safe" lock blocks nothing important: all fortresses and
            // the target remain reachable. Safe for 1-F secret exit since
            // leaving it closed can never cause a softlock.
            let safe = target_reachable && slots.iter().all(|s| {
                s.kind != SlotKind::Fortress || walk.nodes.contains(&s.pos)
            });

            // Score by gated node count: how many nodes become unreachable
            // when this lock is closed? Prefers chokepoints that gate large
            // portions of the map over locks adjacent to the airship (which
            // only gate ~1 node).
            let gated = open_node_count - walk.nodes.len() as i32;

            let mut score: i32 = gated;

            // Bonus: blocks a later fortress (strong progression signal)
            let blocks_later_fort = slots.iter().any(|s| {
                s.kind == SlotKind::Fortress
                    && s.section > section_idx
                    && !walk.nodes.contains(&s.pos)
            });
            if blocks_later_fort {
                score += 100;
            }

            // Bonus: blocks the target (airship/bowser) — only credited to
            // the first such lock in the world; subsequent target-blockers
            // would just pile up next to the airship.
            if !target_reachable && !target_already_locked {
                score += 10;
            }

            // Spread penalty: discourage placing this lock close to any
            // already-placed lock in the world. Falls off linearly with
            // Manhattan distance, zero past 8 tiles.
            if let Some(min_dist) = locks
                .iter()
                .map(|l| cand_pos.0.abs_diff(l.pos.0) + cand_pos.1.abs_diff(l.pos.1))
                .min()
            {
                score -= (8i32 - min_dist as i32).max(0) * 2;
            }

            // Slight preference for bridge tiles — water gaps look better
            // than locks on regular path tiles.
            if tile == 0xB3 {
                score += 1;
            }

            // Track best overall and best safe separately.
            let dominated = match &best {
                Some((_, _, _, best_score, _, _)) => score > *best_score,
                None => true,
            };
            if dominated {
                best = Some((cand_pos, gap, tile, score, safe, !target_reachable));
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

        // Prefer safe when forced (retry) or when best score is low —
        // no point picking an impactful lock if there are none.
        let best_score = best.map(|(_, _, _, s, _, _)| s).unwrap_or(0);
        let prefer_safe = force_safe || best_score < 5;

        let chosen = if prefer_safe {
            best_safe.map(|(pos, gap, replace, score)| (pos, gap, replace, score, true, false))
                .or(best)
        } else {
            best
        };

        if let Some((pos, gap, replace, _score, safe, blocks_target)) = chosen {
            locked_tiles.insert(pos);
            locks.push(LockAssignment {
                pos,
                gap_tile: gap,
                replace_tile: replace,
                fort_section: section_idx,
                secret_exit_safe: safe,
                blocks_target,
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
                SlotKind::BonusGame => TILE_BONUS_GAME,
                SlotKind::ToadHouse => TILE_TOAD_HOUSE,
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
// Required-progression analyzer
// ---------------------------------------------------------------------------
//
// Per-world question: how many distinct level + fortress entries must the
// player clear to reach the airship (or W8's Bowser)?
//
// Movement model: level / fortress tiles are barriers — the player can stand
// on one but cannot transit past it until they've cleared it. Pipes,
// hammer-bros, toad houses, and bonus games are free transit. Locks on path
// tiles are barriers until the fortress whose section opens them is cleared.
// Pipes are free teleports (the player will always prefer the shortcut).
//
// Solved as minimum-vertex-weight shortest path on the state-augmented graph
// `(position, opened_section_mask)`. Per-world section counts are small
// (≤5), so the mask space stays tiny.
//
// Exposed via `analyze_required_progression` so the WASM single-seed dump can
// reuse the same routine.

/// What occupies a grid position visited along the required path.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)] // variants are inspected only by the dump helper today.
pub(crate) enum PathNodeKind {
    Start,
    Level,
    Fortress { section: usize },
    Pipe,
    HammerBro,
    ToadHouse,
    BonusGame,
    Target,
    /// Position has no slot (e.g., a stray node tile). Should be rare.
    Unclassified,
}

#[derive(Clone, Debug, Default)]
#[allow(dead_code)] // read by tests today; consumed by the WASM single-seed dump later.
pub(crate) struct RequiredProgression {
    /// Distinct fortress slots the player must clear (excludes the objective
    /// itself if it happens to live at a fortress tile).
    pub forts_required: usize,
    /// Distinct level slots the player must clear (excludes the objective).
    pub levels_required: usize,
    /// True when the airship/Bowser was reachable (always true on well-formed
    /// maps — false here would indicate a builder bug).
    pub reachable: bool,
    /// Ordered list of (position, kind) starting at start, ending at target.
    pub path: Vec<((usize, usize), PathNodeKind)>,
    /// Locks crossed during traversal, in path order: (lock_path_tile, fort_section).
    pub locks_crossed: Vec<((usize, usize), usize)>,
    /// Which section's lock the hammer pre-opened, if any. `None` means the
    /// hammer was not used (or the analysis was no-hammer).
    pub hammer_broke_section: Option<usize>,
}

/// Compute the minimum number of fortress/level entries the player must clear
/// to reach the world objective.
///
/// When `hammer` is true: the player has one hammer that can break exactly
/// ONE overworld lock for free. We try every individual lock-break and pick
/// the option that minimises total clears (including "don't use hammer").
#[allow(dead_code)] // read by tests today; consumed by the WASM single-seed dump later.
pub(crate) fn analyze_required_progression(
    built: &BuiltWorld,
    hammer: bool,
) -> RequiredProgression {
    if !hammer {
        return analyze_with_pre_opened(built, None);
    }
    // Try (no break) ∪ {break each section}. Minimise total fort+level clears.
    let mut best = analyze_with_pre_opened(built, None);
    let mut best_cost = if best.reachable {
        best.forts_required + best.levels_required
    } else {
        usize::MAX
    };
    for section in 0..built.section_count {
        let mut candidate = analyze_with_pre_opened(built, Some(section));
        if !candidate.reachable {
            continue;
        }
        let cost = candidate.forts_required + candidate.levels_required;
        if cost < best_cost {
            best_cost = cost;
            candidate.hammer_broke_section = Some(section);
            best = candidate;
        }
    }
    best
}

/// Inner Dijkstra: returns the minimum-cost progression with `hammered_section`
/// pre-opened (if `Some`) or no locks pre-opened (`None`).
#[allow(dead_code)]
fn analyze_with_pre_opened(
    built: &BuiltWorld,
    hammered_section: Option<usize>,
) -> RequiredProgression {
    let initial_mask: u32 = match hammered_section {
        Some(s) => 1u32 << s,
        None => 0,
    };
    analyze_with_pre_opened_mask(built, initial_mask)
}

/// Same as `analyze_with_pre_opened` but takes an arbitrary opened-section
/// mask. Useful for the all-locks-open sanity check in the dump.
#[allow(dead_code)]
fn analyze_with_pre_opened_mask(
    built: &BuiltWorld,
    initial_mask: u32,
) -> RequiredProgression {
    // 1. Stamp slots onto a working grid so walk_map sees them as nodes.
    //    Skip locks — we model them as conditional edges instead.
    let mut grid = built.grid.clone();
    for slot in &built.slots {
        match slot.kind {
            SlotKind::Fortress => grid.set(slot.pos.0, slot.pos.1, TILE_FORTRESS),
            SlotKind::Level
                if BACKGROUND_TILES.contains(&grid.get(slot.pos.0, slot.pos.1)) =>
            {
                grid.set(slot.pos.0, slot.pos.1, TILE_NODE);
            }
            SlotKind::BonusGame => grid.set(slot.pos.0, slot.pos.1, TILE_BONUS_GAME),
            SlotKind::ToadHouse => grid.set(slot.pos.0, slot.pos.1, TILE_TOAD_HOUSE),
            _ => {}
        }
    }

    let start = match rom_data::find_start(&grid) {
        Some(s) => s,
        None => return RequiredProgression::default(),
    };
    let target = match find_target(&grid, built.world_idx) {
        Some(t) => t,
        None => return RequiredProgression::default(),
    };

    let walk = walk_map(&grid, &built.pipe_pairs, Some(start));

    // 2. Per-position slot info (skip the target; it's accounted for separately).
    let mut kind_at: HashMap<(usize, usize), &SlotKind> = HashMap::new();
    let mut section_at: HashMap<(usize, usize), usize> = HashMap::new();
    for slot in &built.slots {
        kind_at.insert(slot.pos, &slot.kind);
        section_at.insert(slot.pos, slot.section);
    }

    // 3. Lock lookup keyed on path-tile position.
    let mut lock_section: HashMap<(usize, usize), usize> = HashMap::new();
    for lock in &built.locks {
        lock_section.insert(lock.pos, lock.fort_section);
    }

    // 3b. Canoe edges for this world. There's one boat that starts at the
    //     mainland dock (the `a` side of each `(a, b)` tuple — all share the
    //     same mainland in vanilla). The boat moves WITH the player when they
    //     ride it: a canoe edge (X, Y) is only usable when the boat sits at
    //     X, and after the ride the boat is at Y. Walking/piping to an island
    //     without the boat leaves you stranded (no canoe edge usable from
    //     that island).
    let canoe_edges: Vec<((usize, usize), (usize, usize))> = rom_data::CANOE_EDGES
        .iter()
        .filter(|&&(w, _)| w == built.world_idx)
        .map(|&(_, edge)| edge)
        .collect();
    let canoe_pair_set: HashSet<((usize, usize), (usize, usize))> = canoe_edges
        .iter()
        .flat_map(|&(a, b)| [(a, b), (b, a)])
        .collect();
    let initial_boat: Option<(usize, usize)> = canoe_edges.first().map(|&(a, _)| a);

    // 4. Dijkstra over (position, mask, boat_pos). Cost = node entries so far.
    //    Entering a fortress flips its section bit in the mask; riding a
    //    canoe moves the boat to the destination.
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;

    /// (position, opened-section-mask, boat-position-or-None)
    type SearchState = ((usize, usize), u32, Option<(usize, usize)>);
    type HeapEntry = Reverse<(usize, (usize, usize), u32, Option<(usize, usize)>)>;

    let mut dist: HashMap<SearchState, usize> = HashMap::new();
    let mut prev: HashMap<SearchState, SearchState> = HashMap::new();
    let mut heap: BinaryHeap<HeapEntry> = BinaryHeap::new();

    let initial: SearchState = (start, initial_mask, initial_boat);
    dist.insert(initial, 0);
    heap.push(Reverse((0, start, initial_mask, initial_boat)));

    let mut goal_state: Option<SearchState> = None;

    let entry_cost = |dest: (usize, usize)| -> (usize, bool) {
        // Returns (cost, is_fortress). is_fortress used by caller to update mask.
        if dest == target {
            return (1, false);
        }
        match kind_at.get(&dest) {
            Some(SlotKind::Fortress) => (1, true),
            Some(SlotKind::Level) => (1, false),
            _ => (0, false),
        }
    };

    while let Some(Reverse((cost, pos, mask, boat))) = heap.pop() {
        let state = (pos, mask, boat);
        if cost > *dist.get(&state).unwrap_or(&usize::MAX) {
            continue;
        }
        if std::env::var("TRACE_DIJKSTRA").is_ok() {
            eprintln!("    visit {pos:?} cost={cost} mask={mask:b} boat={boat:?}");
        }
        if pos == target {
            goal_state = Some(state);
            break;
        }

        // Walk / pipe edges from walk_map. Skip canoe edges — those are
        // handled below with explicit boat-state tracking.
        if let Some(edges) = walk.edges.get(&pos) {
            for edge in edges {
                if edge.path_pos.is_none() && canoe_pair_set.contains(&(pos, edge.dest)) {
                    continue;
                }
                // Lock-bearing path tile? Requires its section to be open.
                if let Some(path_pos) = edge.path_pos
                    && let Some(&section) = lock_section.get(&path_pos)
                    && mask & (1u32 << section) == 0
                {
                    continue;
                }
                let dest = edge.dest;
                let (edge_cost, is_fort) = entry_cost(dest);
                let new_mask = if is_fort {
                    mask | (1u32 << section_at[&dest])
                } else {
                    mask
                };
                let key = (dest, new_mask, boat);
                let new_cost = cost + edge_cost;
                if new_cost < *dist.get(&key).unwrap_or(&usize::MAX) {
                    dist.insert(key, new_cost);
                    prev.insert(key, state);
                    heap.push(Reverse((new_cost, dest, new_mask, boat)));
                }
            }
        }

        // Canoe edges: usable only if the boat sits at the current position.
        if boat == Some(pos) {
            for &(a, b) in &canoe_edges {
                let dest = if a == pos {
                    b
                } else if b == pos {
                    a
                } else {
                    continue;
                };
                let (edge_cost, is_fort) = entry_cost(dest);
                let new_mask = if is_fort {
                    mask | (1u32 << section_at[&dest])
                } else {
                    mask
                };
                let new_boat = Some(dest);
                let key = (dest, new_mask, new_boat);
                let new_cost = cost + edge_cost;
                if new_cost < *dist.get(&key).unwrap_or(&usize::MAX) {
                    dist.insert(key, new_cost);
                    prev.insert(key, state);
                    heap.push(Reverse((new_cost, dest, new_mask, new_boat)));
                }
            }
        }
    }

    // 5. Reconstruct the path back from goal. Tally distinct fort/level
    //    positions (start and target excluded from counts), and record which
    //    locks were crossed (lookup edge.path_pos used per hop).
    let Some(final_state) = goal_state else {
        return RequiredProgression::default();
    };

    let kind_for = |pos: (usize, usize)| -> PathNodeKind {
        if pos == start {
            return PathNodeKind::Start;
        }
        if pos == target {
            return PathNodeKind::Target;
        }
        match kind_at.get(&pos) {
            Some(SlotKind::Fortress) => PathNodeKind::Fortress {
                section: section_at[&pos],
            },
            Some(SlotKind::Level) => PathNodeKind::Level,
            Some(SlotKind::Pipe) => PathNodeKind::Pipe,
            Some(SlotKind::HammerBro) => PathNodeKind::HammerBro,
            Some(SlotKind::ToadHouse) => PathNodeKind::ToadHouse,
            Some(SlotKind::BonusGame) => PathNodeKind::BonusGame,
            None => PathNodeKind::Unclassified,
        }
    };

    let mut chain: Vec<SearchState> = vec![final_state];
    let mut cur = final_state;
    while let Some(&prev_state) = prev.get(&cur) {
        chain.push(prev_state);
        cur = prev_state;
    }
    chain.reverse();

    let mut path: Vec<((usize, usize), PathNodeKind)> = Vec::with_capacity(chain.len());
    let mut locks_crossed: Vec<((usize, usize), usize)> = Vec::new();
    let mut forts: HashSet<(usize, usize)> = HashSet::new();
    let mut levels: HashSet<(usize, usize)> = HashSet::new();

    for (i, state) in chain.iter().enumerate() {
        let pos = state.0;
        path.push((pos, kind_for(pos)));
        if i > 0 {
            let prev_pos = chain[i - 1].0;
            if let Some(edges) = walk.edges.get(&prev_pos)
                && let Some(edge) = edges.iter().find(|e| e.dest == pos)
                && let Some(path_pos) = edge.path_pos
                && let Some(&section) = lock_section.get(&path_pos)
            {
                locks_crossed.push((path_pos, section));
            }
        }
        if pos == start || pos == target {
            continue;
        }
        match kind_at.get(&pos) {
            Some(SlotKind::Fortress) => {
                forts.insert(pos);
            }
            Some(SlotKind::Level) => {
                levels.insert(pos);
            }
            _ => {}
        }
    }

    RequiredProgression {
        forts_required: forts.len(),
        levels_required: levels.len(),
        reachable: true,
        path,
        locks_crossed,
        hammer_broke_section: None,
    }
}

/// Pretty-print a `RequiredProgression` result for one world. Use for
/// verification + as a reference for the WASM single-seed dump.
#[allow(dead_code)]
pub(crate) fn dump_required_progression(built: &BuiltWorld) {
    let no_hammer = analyze_required_progression(built, false);
    let with_hammer = analyze_required_progression(built, true);
    // Sanity check: with EVERY lock pre-opened, is the target reachable?
    // If not, the unreachability is a real builder/topology issue. If yes
    // but the 1-lock-hammer path also fails, the issue is lock chain depth.
    let all_open_mask = (1u32 << built.section_count).wrapping_sub(1);
    let all_open = analyze_with_pre_opened_mask(built, all_open_mask);

    let start = rom_data::find_start(&built.grid);
    let target = find_target(&built.grid, built.world_idx);

    let canoes: Vec<((usize, usize), (usize, usize))> = rom_data::CANOE_EDGES
        .iter()
        .filter(|&&(w, _)| w == built.world_idx)
        .map(|&(_, edge)| edge)
        .collect();

    eprintln!("\n--- W{} ---", built.world_idx + 1);
    eprintln!(
        "  start={:?}  target={:?}  sections={}  locks={}  pipes={}{}",
        start,
        target,
        built.section_count,
        built.locks.len(),
        built.pipe_pairs.len(),
        if canoes.is_empty() {
            String::new()
        } else {
            format!("  canoes={}", canoes.len())
        },
    );

    // Inventory of fortress positions per section, so the lock annotations
    // make sense to the reader.
    let mut forts_by_section: Vec<(usize, (usize, usize))> = built
        .slots
        .iter()
        .filter(|s| s.kind == SlotKind::Fortress)
        .map(|s| (s.section, s.pos))
        .collect();
    forts_by_section.sort();
    eprintln!("  fortresses:");
    for (sec, pos) in &forts_by_section {
        eprintln!("    section {sec}: ({}, {})", pos.0, pos.1);
    }
    eprintln!("  locks:");
    for lock in &built.locks {
        eprintln!(
            "    ({}, {}) opened by section {}",
            lock.pos.0, lock.pos.1, lock.fort_section,
        );
    }
    eprintln!("  pipe pairs:");
    for &(a, b) in &built.pipe_pairs {
        eprintln!("    ({},{}) <-> ({},{})", a.0, a.1, b.0, b.1);
    }
    if !canoes.is_empty() {
        eprintln!("  canoe routes (boat starts at the first endpoint):");
        for (a, b) in &canoes {
            eprintln!("    ({},{}) -> ({},{}) (and reverse, while boat is at far side)", a.0, a.1, b.0, b.1);
        }
    }

    let pipe_set: EdgeSet = built.pipe_pairs
        .iter()
        .flat_map(|&(a, b)| [(a, b), (b, a)])
        .collect();
    let canoe_set: EdgeSet = canoes
        .iter()
        .flat_map(|&(a, b)| [(a, b), (b, a)])
        .collect();

    print_progression("Without hammer", &no_hammer, &pipe_set, &canoe_set);
    print_progression("With hammer (1 lock max)", &with_hammer, &pipe_set, &canoe_set);
    eprintln!(
        "  [Sanity: all locks pre-opened]  reachable={}  forts={}  levels={}",
        all_open.reachable, all_open.forts_required, all_open.levels_required,
    );

    match with_hammer.hammer_broke_section {
        Some(s) => eprintln!("  Hammer used on: lock for section {s}"),
        None => eprintln!("  Hammer used on: (nothing — hammer didn't help)"),
    }
    let fort_delta = no_hammer.forts_required as isize - with_hammer.forts_required as isize;
    let level_delta = no_hammer.levels_required as isize - with_hammer.levels_required as isize;
    let total_delta = fort_delta + level_delta;
    eprintln!(
        "  Hammer net: {fort_delta:+} fort(s), {level_delta:+} level(s)  =  {total_delta:+} total clears",
    );
}

/// Set of directed teleport edges (pipe-pair / canoe-pair, both orientations).
#[allow(dead_code)]
type EdgeSet = HashSet<((usize, usize), (usize, usize))>;

#[allow(dead_code)]
fn print_progression(
    label: &str,
    p: &RequiredProgression,
    pipe_set: &EdgeSet,
    canoe_set: &EdgeSet,
) {
    eprintln!(
        "\n  [{label}]  required: {} fort(s) + {} level(s)  (+ objective)",
        p.forts_required, p.levels_required,
    );
    if !p.reachable {
        eprintln!("    TARGET UNREACHABLE");
        return;
    }
    let mut lock_iter = p.locks_crossed.iter().peekable();
    for (i, (pos, kind)) in p.path.iter().enumerate() {
        let tag = match kind {
            PathNodeKind::Start => "START".to_string(),
            PathNodeKind::Level => "LEVEL".to_string(),
            PathNodeKind::Fortress { section } => format!("FORT (section {section})"),
            PathNodeKind::Pipe => "PIPE (transit)".to_string(),
            PathNodeKind::HammerBro => "HAMMERBRO (transit)".to_string(),
            PathNodeKind::ToadHouse => "TOAD (transit)".to_string(),
            PathNodeKind::BonusGame => "BONUS (transit)".to_string(),
            PathNodeKind::Target => "TARGET (airship/Bowser)".to_string(),
            PathNodeKind::Unclassified => "transit tile".to_string(),
        };
        // Classify the hop: pipe teleport, canoe, or walk.
        let via = if i > 0 {
            let prev = p.path[i - 1].0;
            let edge = (prev, *pos);
            if pipe_set.contains(&edge) {
                " [via PIPE]"
            } else if canoe_set.contains(&edge) {
                " [via CANOE]"
            } else {
                ""
            }
        } else {
            ""
        };
        eprintln!("    {i:2}. ({:2},{:2})  {tag}{via}", pos.0, pos.1);
        // After printing the step, if the next lock_crossed entry came from
        // this hop, surface it underneath.
        if let Some(&&(lock_pos, sec)) = lock_iter.peek()
            && i > 0
        {
            // The lock was on the edge into this node; print under this line.
            let prev = p.path[i - 1].0;
            // Path tile sits between prev and pos for a normal walk.
            let between_r = (prev.0 + pos.0) / 2;
            let between_c = (prev.1 + pos.1) / 2;
            if (between_r, between_c) == lock_pos {
                eprintln!(
                    "         ↳ crossed lock at ({},{}) (opened by section {sec})",
                    lock_pos.0, lock_pos.1,
                );
                lock_iter.next();
            }
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
        let data = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&data).ok()
    }

    /// Apply the QoL patches that the real pipeline runs before the overworld
    /// builder (`randomizer.rs` ~L657-670). These mutate the world-map grid —
    /// rocks blocking pipe shortcuts, W3 drawbridge tiles, big-Q rooms — so
    /// the catalog must see the post-patch state, not vanilla.
    fn apply_qol_for_overworld(rom: &Rom) -> Rom {
        let mut out = rom.clone();
        super::super::qol::fix_w3_drawbridges(&mut out);
        super::super::qol::remove_rocks(&mut out);
        super::super::qol::fix_big_q_block_rooms(&mut out);
        out
    }

    /// Build `(catalog, pickup)` for one seed. When the `SAS` env var is set,
    /// applies per-seed start↔airship swap before pickup runs, matching the
    /// real pipeline in `randomizer.rs` when `swap_start_airship` is on.
    fn build_catalog_pickup(rom: &Rom, seed: u64) -> (NodeCatalog, PickupResult) {
        let mut catalog = NodeCatalog::build(rom, false);
        if std::env::var("SAS").is_ok() {
            let mut swap_rng = ChaCha8Rng::seed_from_u64(seed);
            super::super::start_airship_swap::pick_swaps(&mut catalog, &mut swap_rng);
        }
        let pickup = super::super::overworld_pickup::pick_up(
            rom,
            &catalog,
            super::super::overworld_pickup::PickupFlags {
                shuffle_spade_games: true,
                shuffle_toad_houses: true,
            },
        );
        (catalog, pickup)
    }

    #[test]
    fn test_fortress_redistribution() {
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        for _ in 0..100 {
            let counts = redistribute_fortresses(&mut rng);
            let total: usize = counts.iter().sum();
            assert_eq!(total, 17, "total fortresses must be 17");
            assert_eq!(counts[7], 4, "W8 must keep 4");
            for (w, &count) in counts[..7].iter().enumerate() {
                assert!((1..=3).contains(&count),
                    "W{} got {count} forts, expected 1-3", w + 1);
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
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, super::super::overworld_pickup::PickupFlags { shuffle_spade_games: true, shuffle_toad_houses: true });
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let result = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, true);

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
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, super::super::overworld_pickup::PickupFlags { shuffle_spade_games: true, shuffle_toad_houses: true });

        for seed in 0..10 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let result = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, true);

            for built in &result.worlds {
                let start_pos = rom_data::find_start(&built.grid);

                // Build grid with all assignments stamped
                let mut test_grid = built.grid.clone();
                for slot in &built.slots {
                    match slot.kind {
                        SlotKind::Fortress => test_grid.set(slot.pos.0, slot.pos.1, TILE_FORTRESS),
                        SlotKind::BonusGame => test_grid.set(slot.pos.0, slot.pos.1, TILE_BONUS_GAME),
                        SlotKind::ToadHouse => test_grid.set(slot.pos.0, slot.pos.1, TILE_TOAD_HOUSE),
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
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, super::super::overworld_pickup::PickupFlags { shuffle_spade_games: true, shuffle_toad_houses: true });

        for seed in [42, 123, 999] {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let result = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, true);

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
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, super::super::overworld_pickup::PickupFlags { shuffle_spade_games: true, shuffle_toad_houses: true });
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let result = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, true);

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
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, super::super::overworld_pickup::PickupFlags { shuffle_spade_games: true, shuffle_toad_houses: true });

        let mut level_shortfalls = 0u32;
        let mut lock_shortfalls = 0u32;
        let seeds = 1000;

        for seed in 0..seeds {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let result = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, true);

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
            let result2 = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng2, true);
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
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, super::super::overworld_pickup::PickupFlags { shuffle_spade_games: true, shuffle_toad_houses: true });

        for seed in 0..6u64 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let result = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, true);
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
            let mut bonus_games = Vec::new();
            let mut toad_houses = Vec::new();
            for slot in &built.slots {
                match slot.kind {
                    SlotKind::Fortress => fortresses.push(slot),
                    SlotKind::Level => levels.push(slot),
                    SlotKind::HammerBro => hammer_bros.push(slot),
                    SlotKind::Pipe => pipes.push(slot),
                    SlotKind::BonusGame => bonus_games.push(slot),
                    SlotKind::ToadHouse => toad_houses.push(slot),
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

            eprintln!("\nBonus Games ({}):", bonus_games.len());
            for s in &bonus_games {
                eprintln!("  ({:2}, {:2})  section={}", s.pos.0, s.pos.1, s.section);
            }

            eprintln!("\nToad Houses ({}):", toad_houses.len());
            for s in &toad_houses {
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
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, super::super::overworld_pickup::PickupFlags { shuffle_spade_games: true, shuffle_toad_houses: true });
        let wi = 6; // W7

        let cw = &pickup.worlds[wi];
        eprintln!("\n=== W7 Pickup: {} pool entries ===", cw.pool_indices.len());

        let fixed = fixed_positions_for_world(&rom, &catalog, wi, true);
        eprintln!("Fixed positions: {} {:?}", fixed.len(), fixed);

        let blank_positions = find_blank_slots(&cw.grid, &fixed);
        eprintln!("Blank tiles on grid: {}", blank_positions.len());

        // Run the actual build for several seeds and check coverage
        for seed in 0..5u64 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let result = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, true);
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

    #[test]
    #[ignore]
    fn test_lock_scoring_detail() {
        let rom = match load_rom() {
            Some(r) => r,
            None => {
                eprintln!("ROM not found, skipping");
                return;
            }
        };
        let catalog = NodeCatalog::build(&rom, false);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, super::super::overworld_pickup::PickupFlags { shuffle_spade_games: true, shuffle_toad_houses: true });

        for seed in [42u64, 123, 999] {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let result = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, true);

            eprintln!("\n{}", "=".repeat(60));
            eprintln!("=== Seed {seed} ===");

            for built in &result.worlds {
                let wi = built.world_idx;
                let target_pos = find_target(&built.grid, wi);
                let start_pos = rom_data::find_start(&built.grid);

                let forts: Vec<_> = built.slots.iter()
                    .filter(|s| s.kind == SlotKind::Fortress)
                    .collect();
                let levels: Vec<_> = built.slots.iter()
                    .filter(|s| s.kind == SlotKind::Level)
                    .collect();

                eprintln!("\n  W{}: {} forts, {} levels, {} pipes, {} locks, target={:?}",
                    wi + 1, forts.len(), levels.len(), built.pipe_pairs.len(),
                    built.locks.len(), target_pos);

                // Build stamped grid (forts + levels, no locks)
                let mut base_grid = built.grid.clone();
                for slot in &built.slots {
                    match slot.kind {
                        SlotKind::Fortress => base_grid.set(slot.pos.0, slot.pos.1, TILE_FORTRESS),
                        SlotKind::Level
                            if BACKGROUND_TILES.contains(&base_grid.get(slot.pos.0, slot.pos.1)) =>
                        {
                            base_grid.set(slot.pos.0, slot.pos.1, TILE_NODE);
                        }
                        _ => {}
                    }
                }

                for lock in &built.locks {
                    // Open grid: no locks
                    let walk_open = walk_map(&base_grid, &built.pipe_pairs, start_pos);

                    // Locked grid: this lock closed
                    let mut locked_grid = base_grid.clone();
                    locked_grid.set(lock.pos.0, lock.pos.1, lock.gap_tile);
                    let walk_locked = walk_map(&locked_grid, &built.pipe_pairs, start_pos);

                    let gated_count = walk_open.nodes.len() as i32 - walk_locked.nodes.len() as i32;

                    // What specifically gets gated?
                    let gated_forts: Vec<_> = forts.iter()
                        .filter(|f| walk_open.nodes.contains(&f.pos) && !walk_locked.nodes.contains(&f.pos))
                        .collect();
                    let gated_levels: Vec<_> = levels.iter()
                        .filter(|l| walk_open.nodes.contains(&l.pos) && !walk_locked.nodes.contains(&l.pos))
                        .collect();
                    let gates_target = target_pos
                        .map(|tp| walk_open.nodes.contains(&tp) && !walk_locked.nodes.contains(&tp))
                        .unwrap_or(false);

                    // BFS distance from lock to target (via adjacent nodes)
                    let target_dist = if let Some(tp) = target_pos {
                        let walk_from_target = walk_map(&base_grid, &built.pipe_pairs, Some(tp));
                        let (lr, lc) = lock.pos;
                        [(-1i16, 0i16), (1, 0), (0, -1), (0, 1)].iter()
                            .filter_map(|&(dr, dc)| {
                                let nr = lr as i16 + dr;
                                let nc = lc as i16 + dc;
                                if nr < 0 || nc < 0 { return None; }
                                walk_from_target.distances.get(&(nr as usize, nc as usize)).copied()
                            })
                            .min()
                    } else {
                        None
                    };

                    eprintln!("    Lock ({:2},{:2}) sect={} safe={:<5} gated={:<3} dist_to_target={:<4} gates: {} forts, {} levels{}",
                        lock.pos.0, lock.pos.1,
                        lock.fort_section,
                        lock.secret_exit_safe,
                        gated_count,
                        target_dist.map(|d| d.to_string()).unwrap_or("-".into()),
                        gated_forts.len(),
                        gated_levels.len(),
                        if gates_target { ", TARGET" } else { "" },
                    );
                }
            }
        }
    }

    /// Dump all lock candidates and their scores for a specific world.
    /// Usage: change seed/target_wi below, then run with --nocapture.
    #[test]
    #[ignore]
    fn test_lock_candidates_dump() {
        let rom = match load_rom() {
            Some(r) => r,
            None => { eprintln!("ROM not found"); return; }
        };
        let catalog = NodeCatalog::build(&rom, false);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, super::super::overworld_pickup::PickupFlags { shuffle_spade_games: true, shuffle_toad_houses: true });

        let seed = 42u64;
        let target_wi = 6; // 0-indexed: W7 = 6

        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let result = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, true);
        let built = &result.worlds[target_wi];

        let start_pos = rom_data::find_start(&built.grid);
        let target_pos = find_target(&built.grid, target_wi);

        // Build base grid with forts/levels stamped (no locks)
        let mut base_grid = built.grid.clone();
        for slot in &built.slots {
            match slot.kind {
                SlotKind::Fortress => base_grid.set(slot.pos.0, slot.pos.1, TILE_FORTRESS),
                SlotKind::Level
                    if BACKGROUND_TILES.contains(&base_grid.get(slot.pos.0, slot.pos.1)) =>
                {
                    base_grid.set(slot.pos.0, slot.pos.1, TILE_NODE);
                }
                _ => {}
            }
        }

        eprintln!("\n=== Seed {seed}, W{} — Lock Candidate Dump ===", target_wi + 1);
        eprintln!("Target: {:?}, Start: {:?}", target_pos, start_pos);
        eprintln!("Forts: {}, Sections: {}", result.fort_counts[target_wi], built.section_count);

        // For each section, enumerate all lockable tiles and score them
        for section_idx in 0..built.section_count {
            let fort_pos = match built.slots.iter()
                .find(|s| s.section == section_idx && s.kind == SlotKind::Fortress)
            {
                Some(s) => s.pos,
                None => continue,
            };

            eprintln!("\n  Section {section_idx} (fort at {:?}):", fort_pos);

            // Open grid for this section: earlier locks open, no current lock
            let walk_open = walk_map(&base_grid, &built.pipe_pairs, start_pos);
            let open_node_count = walk_open.nodes.len();

            // Find all lockable path tiles
            // (pos, gated, safe, score, blocks_later_fort, blocks_target)
            type LockDebugCandidate = (Pos, i32, bool, i32, bool, bool);
            let mut candidates: Vec<LockDebugCandidate> = Vec::new();

            for r in 0..base_grid.rows {
                for c in 0..base_grid.cols {
                    let tile = base_grid.get(r, c);
                    if !LOCKABLE_TILES.contains(&tile) { continue; }

                    let gap = gap_tile_for(tile);
                    let mut test_grid = base_grid.clone();
                    test_grid.set(r, c, gap);
                    let walk = walk_map(&test_grid, &built.pipe_pairs, start_pos);

                    // Hard rule: fort must be reachable
                    if !walk.nodes.contains(&fort_pos) { continue; }

                    let gated = open_node_count as i32 - walk.nodes.len() as i32;
                    let target_reachable = target_pos
                        .map(|tp| walk.nodes.contains(&tp))
                        .unwrap_or(true);
                    let safe = target_reachable && built.slots.iter().all(|s| {
                        s.kind != SlotKind::Fortress || walk.nodes.contains(&s.pos)
                    });
                    let blocks_later_fort = built.slots.iter().any(|s| {
                        s.kind == SlotKind::Fortress
                            && s.section > section_idx
                            && !walk.nodes.contains(&s.pos)
                    });
                    let mut score = gated;
                    if blocks_later_fort { score += 100; }

                    candidates.push(((r, c), gated, safe, score, blocks_later_fort, !target_reachable));
                }
            }

            // Sort by score descending
            candidates.sort_by_key(|c| std::cmp::Reverse(c.3));

            let chosen = built.locks.iter().find(|l| l.fort_section == section_idx);
            eprintln!("    {} candidates pass hard rules, chosen={:?}",
                candidates.len(),
                chosen.map(|l| l.pos));

            for (pos, gated, safe, score, blf, bt) in &candidates {
                let marker = if chosen.map(|l| l.pos == *pos).unwrap_or(false) { " <-- CHOSEN" } else { "" };
                eprintln!("    ({:2},{:2}) gated={:<3} score={:<4} safe={:<5} blk_fort={:<5} blk_target={}{marker}",
                    pos.0, pos.1, gated, score, safe, blf, bt);
            }
        }
    }

    #[test]
    #[ignore]
    fn test_lock_airship_distance() {
        let rom = match load_rom() {
            Some(r) => r,
            None => {
                eprintln!("ROM not found, skipping");
                return;
            }
        };
        let rom = apply_qol_for_overworld(&rom);

        let seeds: u64 = std::env::var("LOCK_SEEDS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(100);
        // BFS distance histogram: index = distance, value = count
        let mut histogram = [0u32; 30];
        let mut total_locks = 0u32;
        let mut no_target_locks = 0u32;
        // Per-world stats: (sum_of_distances, count)
        let mut per_world: [(u64, u32); 8] = [(0, 0); 8];
        // Track locks at distance <= 2 per seed for flagging
        let mut close_lock_seeds = 0u32;
        // Inter-lock Manhattan distance (only for worlds with 2+ locks)
        let mut inter_hist = [0u32; 40];
        let mut total_pairs = 0u32;
        let mut per_world_pairs: [(u64, u32); 8] = [(0, 0); 8];
        let mut close_pair_seeds = 0u32;

        for seed in 0..seeds {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let (catalog, pickup) = build_catalog_pickup(&rom, seed);
            let result = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, true);
            let mut seed_has_close = false;
            let mut seed_has_close_pair = false;

            for built in &result.worlds {
                let wi = built.world_idx;
                let target_pos = find_target(&built.grid, wi);

                // Inter-lock Manhattan distance (works regardless of target).
                if built.locks.len() >= 2 {
                    for i in 0..built.locks.len() {
                        for j in (i + 1)..built.locks.len() {
                            let (ar, ac) = built.locks[i].pos;
                            let (br, bc) = built.locks[j].pos;
                            let d = ar.abs_diff(br) + ac.abs_diff(bc);
                            let idx = d.min(inter_hist.len() - 1);
                            inter_hist[idx] += 1;
                            total_pairs += 1;
                            per_world_pairs[wi].0 += d as u64;
                            per_world_pairs[wi].1 += 1;
                            if d <= 3 {
                                seed_has_close_pair = true;
                            }
                        }
                    }
                }

                if target_pos.is_none() {
                    no_target_locks += built.locks.len() as u32;
                    continue;
                }
                let tp = target_pos.unwrap();

                // Build a fully-stamped grid with all locks open so BFS
                // reflects the walkable map. walk_map uses node-to-node
                // hops (nodes are 2 tiles apart), so lock path tiles
                // won't appear in distances. Instead, BFS from the target
                // and measure to the node(s) adjacent to each lock.
                let mut stamped = built.grid.clone();
                for slot in &built.slots {
                    match slot.kind {
                        SlotKind::Fortress => stamped.set(slot.pos.0, slot.pos.1, TILE_FORTRESS),
                        SlotKind::Level
                            if BACKGROUND_TILES.contains(&stamped.get(slot.pos.0, slot.pos.1)) =>
                        {
                            stamped.set(slot.pos.0, slot.pos.1, TILE_NODE);
                        }
                        _ => {}
                    }
                }

                // BFS from target — distances to every reachable node
                let walk_from_target = walk_map(&stamped, &built.pipe_pairs, Some(tp));

                for lock in &built.locks {
                    total_locks += 1;

                    // Lock is on a path tile between two nodes. Find the
                    // closest adjacent node (in BFS hops from target).
                    let (lr, lc) = lock.pos;
                    let adjacent_nodes: Vec<(usize, usize)> = [(-1i16, 0i16), (1, 0), (0, -1), (0, 1)]
                        .iter()
                        .filter_map(|&(dr, dc)| {
                            let nr = lr as i16 + dr;
                            let nc = lc as i16 + dc;
                            if nr < 0 || nr >= stamped.rows as i16 || nc < 0 || nc >= stamped.cols as i16 {
                                return None;
                            }
                            let pos = (nr as usize, nc as usize);
                            // Only count positions that are actual BFS nodes
                            if walk_from_target.distances.contains_key(&pos) {
                                Some(pos)
                            } else {
                                None
                            }
                        })
                        .collect();

                    // Use the minimum distance among adjacent nodes
                    // (the side closer to the target).
                    let min_dist = adjacent_nodes.iter()
                        .filter_map(|pos| walk_from_target.distances.get(pos))
                        .min()
                        .copied();

                    if let Some(dist) = min_dist {
                        let idx = dist.min(histogram.len() - 1);
                        histogram[idx] += 1;
                        per_world[wi].0 += dist as u64;
                        per_world[wi].1 += 1;

                        if dist <= 2 {
                            seed_has_close = true;
                        }
                    } else {
                        no_target_locks += 1;
                    }
                }
            }
            if seed_has_close {
                close_lock_seeds += 1;
            }
            if seed_has_close_pair {
                close_pair_seeds += 1;
            }
        }

        eprintln!("\n=== Lock-to-Airship BFS Distance ({seeds} seeds, {total_locks} locks) ===\n");

        // Histogram
        eprintln!("Distance | Count | Bar");
        eprintln!("---------+-------+----");
        let max_dist_with_data = histogram.iter().rposition(|&c| c > 0).unwrap_or(0);
        for (d, &count) in histogram[..=max_dist_with_data].iter().enumerate() {
            let bar = "#".repeat((count as usize).min(60));
            eprintln!("{d:>5}    | {count:<5} | {bar}");
        }

        // Summary stats
        let total_dist: u64 = histogram.iter().enumerate().map(|(d, &c)| d as u64 * c as u64).sum();
        let mean = total_dist as f64 / total_locks.max(1) as f64;
        let close = histogram[0] + histogram[1] + histogram[2];
        let close_pct = close as f64 / total_locks.max(1) as f64 * 100.0;

        eprintln!("\nMean distance:         {mean:.1}");
        eprintln!("Locks at dist <= 2:    {close}/{total_locks} ({close_pct:.1}%)");
        eprintln!("Seeds with any <= 2:   {close_lock_seeds}/{seeds}");
        if no_target_locks > 0 {
            eprintln!("Locks without target:  {no_target_locks}");
        }

        eprintln!("\nPer-world averages:");
        for (wi, &(sum, count)) in per_world.iter().enumerate() {
            if count > 0 {
                let avg = sum as f64 / count as f64;
                eprintln!("  W{}: {avg:.1} avg ({count} locks)", wi + 1);
            }
        }

        eprintln!("\n=== Inter-Lock Manhattan Distance ({total_pairs} pairs) ===\n");
        eprintln!("Distance | Count | Bar");
        eprintln!("---------+-------+----");
        let max_inter = inter_hist.iter().rposition(|&c| c > 0).unwrap_or(0);
        for (d, &count) in inter_hist[..=max_inter].iter().enumerate() {
            let bar = "#".repeat((count as usize).min(60));
            eprintln!("{d:>5}    | {count:<5} | {bar}");
        }
        let inter_total: u64 = inter_hist.iter().enumerate().map(|(d, &c)| d as u64 * c as u64).sum();
        let inter_mean = inter_total as f64 / total_pairs.max(1) as f64;
        let close_pairs: u32 = inter_hist[..=3].iter().sum();
        let close_pair_pct = close_pairs as f64 / total_pairs.max(1) as f64 * 100.0;
        eprintln!("\nMean pair distance:    {inter_mean:.1}");
        eprintln!("Pairs at dist <= 3:    {close_pairs}/{total_pairs} ({close_pair_pct:.1}%)");
        eprintln!("Seeds with any pair <=3: {close_pair_seeds}/{seeds}");
        eprintln!("\nPer-world pair averages:");
        for (wi, &(sum, count)) in per_world_pairs.iter().enumerate() {
            if count > 0 {
                let avg = sum as f64 / count as f64;
                eprintln!("  W{}: {avg:.1} avg ({count} pairs)", wi + 1);
            }
        }
    }

    /// Distribution analyzer for pipe placement.
    ///
    /// Runs the builder for N seeds and reports, per world:
    ///   - endpoint frequency (how often each position appears as a pipe end)
    ///   - unordered-pair frequency
    ///   - Shannon entropy of the endpoint distribution (bits)
    ///   - top-5 most-picked endpoints and pairs
    ///
    /// Use the entropy number to compare scoring tweaks: higher = more variety.
    /// Run with: cargo test --release test_pipe_distribution -- --ignored --nocapture
    #[test]
    #[ignore]
    fn test_pipe_distribution() {
        let rom = match load_rom() {
            Some(r) => r,
            None => {
                eprintln!("ROM not found, skipping");
                return;
            }
        };
        let rom = apply_qol_for_overworld(&rom);

        let seeds: u64 = std::env::var("PIPE_SEEDS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1000);

        // Per-world tallies
        let mut endpoint_counts: [HashMap<(usize, usize), u32>; 8] = Default::default();
        let mut pair_counts: [HashMap<TeleportEdge, u32>; 8] = Default::default();
        let mut total_endpoints = [0u32; 8];
        let mut total_pairs = [0u32; 8];

        for seed in 0..seeds {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let (catalog, pickup) = build_catalog_pickup(&rom, seed);
            let result = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, true);

            for built in &result.worlds {
                let wi = built.world_idx;
                for &(a, b) in &built.pipe_pairs {
                    *endpoint_counts[wi].entry(a).or_insert(0) += 1;
                    *endpoint_counts[wi].entry(b).or_insert(0) += 1;
                    total_endpoints[wi] += 2;

                    // Normalize unordered pair (smaller first)
                    let pair = if a <= b { (a, b) } else { (b, a) };
                    *pair_counts[wi].entry(pair).or_insert(0) += 1;
                    total_pairs[wi] += 1;
                }
            }
        }

        eprintln!("\n=== Pipe Distribution over {seeds} seeds ===");

        for wi in 0..8 {
            let expected_pairs = VANILLA_PIPE_PAIRS[wi];
            if expected_pairs == 0 {
                continue;
            }

            let endpoints = &endpoint_counts[wi];
            let pairs = &pair_counts[wi];
            let total_ep = total_endpoints[wi] as f64;
            let total_pr = total_pairs[wi] as f64;

            // Shannon entropy (bits) over endpoint distribution
            let entropy: f64 = endpoints
                .values()
                .map(|&c| {
                    let p = c as f64 / total_ep;
                    -p * p.log2()
                })
                .sum();
            // Max entropy if uniform over all observed endpoints
            let max_entropy = (endpoints.len() as f64).log2();

            // Same for pairs
            let pair_entropy: f64 = pairs
                .values()
                .map(|&c| {
                    let p = c as f64 / total_pr;
                    -p * p.log2()
                })
                .sum();
            let pair_max_entropy = (pairs.len() as f64).log2();

            eprintln!(
                "\n--- W{} ({} pair{}/seed) ---",
                wi + 1,
                expected_pairs,
                if expected_pairs == 1 { "" } else { "s" },
            );
            eprintln!(
                "  Endpoints: {} unique  |  entropy {:.2} / {:.2} bits ({:.0}%)",
                endpoints.len(),
                entropy,
                max_entropy,
                if max_entropy > 0.0 { entropy / max_entropy * 100.0 } else { 0.0 },
            );
            eprintln!(
                "  Pairs:     {} unique  |  entropy {:.2} / {:.2} bits ({:.0}%)",
                pairs.len(),
                pair_entropy,
                pair_max_entropy,
                if pair_max_entropy > 0.0 { pair_entropy / pair_max_entropy * 100.0 } else { 0.0 },
            );

            let mut ep_sorted: Vec<_> = endpoints.iter().collect();
            ep_sorted.sort_by(|a, b| b.1.cmp(a.1));
            eprintln!("  Top endpoints:");
            for (pos, count) in ep_sorted.iter().take(5) {
                let count = **count;
                let pct = count as f64 / total_ep * 100.0;
                let bar = "#".repeat((pct as usize).min(40));
                eprintln!(
                    "    ({:2},{:2})  {:>5} ({:5.1}%)  {bar}",
                    pos.0, pos.1, count, pct,
                );
            }

            let mut pr_sorted: Vec<_> = pairs.iter().collect();
            pr_sorted.sort_by(|a, b| b.1.cmp(a.1));
            eprintln!("  Top pairs:");
            for (pair, count) in pr_sorted.iter().take(5) {
                let count = **count;
                let pct = count as f64 / total_pr * 100.0;
                let bar = "#".repeat((pct as usize).min(40));
                eprintln!(
                    "    ({:2},{:2}) <-> ({:2},{:2})  {:>5} ({:5.1}%)  {bar}",
                    pair.0.0, pair.0.1, pair.1.0, pair.1.1, count, pct,
                );
            }
        }

        // One-line summary line for easy before/after diffing
        eprintln!("\n=== Endpoint entropy summary (bits) ===");
        let summary: Vec<String> = (0..8)
            .filter(|&wi| VANILLA_PIPE_PAIRS[wi] > 0)
            .map(|wi| {
                let total_ep = total_endpoints[wi] as f64;
                let entropy: f64 = endpoint_counts[wi]
                    .values()
                    .map(|&c| {
                        let p = c as f64 / total_ep;
                        -p * p.log2()
                    })
                    .sum();
                format!("W{}={entropy:.2}", wi + 1)
            })
            .collect();
        eprintln!("  {}", summary.join("  "));
    }

    /// Distribution analyzer for fortress placement.
    ///
    /// Runs the builder for N seeds and reports, per world:
    ///   - unique fortress positions and Shannon entropy (bits)
    ///   - top-5 most-picked positions
    ///   - per-section breakdown (each section places exactly one fortress)
    ///
    /// Use the entropy number to compare scoring tweaks: higher = more variety.
    /// Run with: cargo test --release test_fortress_distribution -- --ignored --nocapture
    /// Override seed count with FORT_SEEDS=N.
    #[test]
    #[ignore]
    fn test_fortress_distribution() {
        let rom = match load_rom() {
            Some(r) => r,
            None => {
                eprintln!("ROM not found, skipping");
                return;
            }
        };
        let rom = apply_qol_for_overworld(&rom);

        let seeds: u64 = std::env::var("FORT_SEEDS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1000);

        // Per-world tallies
        let mut world_counts: [HashMap<(usize, usize), u32>; 8] = Default::default();
        let mut world_total = [0u32; 8];
        // Per-section tallies: [world][section] -> position frequency
        let mut section_counts: [Vec<HashMap<(usize, usize), u32>>; 8] = Default::default();
        let mut section_total: [Vec<u32>; 8] = Default::default();

        for seed in 0..seeds {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let (catalog, pickup) = build_catalog_pickup(&rom, seed);
            let result = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, true);

            for built in &result.worlds {
                let wi = built.world_idx;

                // Grow per-section storage to match observed section_count.
                if section_counts[wi].len() < built.section_count {
                    section_counts[wi].resize(built.section_count, HashMap::new());
                    section_total[wi].resize(built.section_count, 0);
                }

                for slot in &built.slots {
                    if slot.kind != SlotKind::Fortress {
                        continue;
                    }
                    *world_counts[wi].entry(slot.pos).or_insert(0) += 1;
                    world_total[wi] += 1;

                    if slot.section < section_counts[wi].len() {
                        *section_counts[wi][slot.section].entry(slot.pos).or_insert(0) += 1;
                        section_total[wi][slot.section] += 1;
                    }
                }
            }
        }

        eprintln!("\n=== Fortress Distribution over {seeds} seeds ===");

        for wi in 0..8 {
            let counts = &world_counts[wi];
            let total = world_total[wi];
            if total == 0 {
                continue;
            }
            let total_f = total as f64;

            let entropy: f64 = counts
                .values()
                .map(|&c| {
                    let p = c as f64 / total_f;
                    -p * p.log2()
                })
                .sum();
            let max_entropy = (counts.len() as f64).log2();
            let forts_per_seed = total as f64 / seeds as f64;

            eprintln!(
                "\n--- W{} ({:.0} fort{}/seed) ---",
                wi + 1,
                forts_per_seed,
                if forts_per_seed == 1.0 { "" } else { "s" },
            );
            eprintln!(
                "  Positions: {} unique  |  entropy {:.2} / {:.2} bits ({:.0}%)",
                counts.len(),
                entropy,
                max_entropy,
                if max_entropy > 0.0 { entropy / max_entropy * 100.0 } else { 0.0 },
            );

            let mut sorted: Vec<_> = counts.iter().collect();
            sorted.sort_by(|a, b| b.1.cmp(a.1));
            eprintln!("  Top positions:");
            for (pos, count) in sorted.iter().take(5) {
                let count = **count;
                let pct = count as f64 / total_f * 100.0;
                let bar = "#".repeat((pct as usize).min(40));
                eprintln!(
                    "    ({:2},{:2})  {:>5} ({:5.1}%)  {bar}",
                    pos.0, pos.1, count, pct,
                );
            }

            // Per-section breakdown
            for (si, sec_counts) in section_counts[wi].iter().enumerate() {
                if sec_counts.is_empty() {
                    continue;
                }
                let sec_total = section_total[wi][si] as f64;
                let sec_entropy: f64 = sec_counts
                    .values()
                    .map(|&c| {
                        let p = c as f64 / sec_total;
                        -p * p.log2()
                    })
                    .sum();
                let sec_max = (sec_counts.len() as f64).log2();
                let mut sec_sorted: Vec<_> = sec_counts.iter().collect();
                sec_sorted.sort_by(|a, b| b.1.cmp(a.1));
                let top: Vec<String> = sec_sorted
                    .iter()
                    .take(3)
                    .map(|(p, c)| {
                        let pct = **c as f64 / sec_total * 100.0;
                        format!("({},{})={:.0}%", p.0, p.1, pct)
                    })
                    .collect();
                eprintln!(
                    "    Section {si}: {} unique, entropy {:.2}/{:.2} bits, top: {}",
                    sec_counts.len(),
                    sec_entropy,
                    sec_max,
                    top.join("  "),
                );
            }
        }

        eprintln!("\n=== Fortress entropy summary (bits) ===");
        let summary: Vec<String> = (0..8)
            .filter(|&wi| world_total[wi] > 0)
            .map(|wi| {
                let total_f = world_total[wi] as f64;
                let entropy: f64 = world_counts[wi]
                    .values()
                    .map(|&c| {
                        let p = c as f64 / total_f;
                        -p * p.log2()
                    })
                    .sum();
                format!("W{}={entropy:.2}", wi + 1)
            })
            .collect();
        eprintln!("  {}", summary.join("  "));

        // Sanity: 17 fortresses per seed total
        let grand_total: u32 = world_total.iter().sum();
        let expected = 17 * seeds as u32;
        eprintln!("\nGrand total: {grand_total} fortresses across {seeds} seeds (expected {expected})");
        assert_eq!(grand_total, expected, "fortress count invariant broken");
    }

    /// Quality analyzer for level placement.
    ///
    /// Level placement is deterministic given pipes+forts, so a position-entropy
    /// test would mostly just measure upstream randomness. Instead, this measures
    /// whether the scoring achieves its stated goals:
    ///
    ///   - Spread: avg pairwise distance between placed levels, density-rule
    ///     violations (pairs within combined radius 4). Anti-clumping is the
    ///     primary anti-degeneracy signal.
    ///   - Path bonus: avg detour from start→target shortest path for placed
    ///     levels vs all candidates. Negative bias = levels biased toward main
    ///     route, the intended design goal.
    ///   - Dead-end bonus: % of dead-end candidates that became levels vs the
    ///     random baseline. Treated as a tiebreaker — it should win where it
    ///     doesn't conflict with path bias, but not override it.
    ///
    /// Run with: cargo test --release test_level_placement_quality -- --ignored --nocapture
    /// Override seed count with LEVEL_SEEDS=N.
    #[test]
    #[ignore]
    fn test_level_placement_quality() {
        let rom = match load_rom() {
            Some(r) => r,
            None => {
                eprintln!("ROM not found, skipping");
                return;
            }
        };
        let rom = apply_qol_for_overworld(&rom);

        let seeds: u64 = std::env::var("LEVEL_SEEDS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1000);

        // Per-world aggregates
        let mut total_pairwise_dist = [0u64; 8];
        let mut total_pairs = [0u64; 8];
        let mut density_violations = [0u64; 8];
        let mut dead_ends_picked = [0u64; 8];
        let mut total_dead_end_candidates = [0u64; 8];
        let mut levels_picked = [0u64; 8];
        let mut total_candidates = [0u64; 8];
        let mut total_level_detour = [0u64; 8];
        let mut total_levels_for_detour = [0u64; 8];
        let mut total_candidate_detour = [0u64; 8];
        let mut total_candidates_for_detour = [0u64; 8];

        for seed in 0..seeds {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let (catalog, pickup) = build_catalog_pickup(&rom, seed);
            let result = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, true);

            for built in &result.worlds {
                let wi = built.world_idx;
                let start_pos = rom_data::find_start(&built.grid);
                let target_pos = find_target(&built.grid, wi);

                let levels: Vec<(usize, usize)> = built.slots.iter()
                    .filter(|s| s.kind == SlotKind::Level)
                    .map(|s| s.pos)
                    .collect();

                // Candidate pool seen by level placement: all non-fort, non-pipe
                // section positions. In the final state, these became Level or
                // HammerBro slots.
                let candidates: Vec<(usize, usize)> = built.slots.iter()
                    .filter(|s| matches!(s.kind, SlotKind::Level | SlotKind::HammerBro))
                    .map(|s| s.pos)
                    .collect();

                // BFS from start (matches what scoring used).
                let walk = walk_map(&built.grid, &built.pipe_pairs, start_pos);
                let bfs_distances = &walk.distances;

                // === Spread: avg pairwise distance between placed levels ===
                for i in 0..levels.len() {
                    for j in (i + 1)..levels.len() {
                        let manhattan = levels[i].0.abs_diff(levels[j].0)
                            + levels[i].1.abs_diff(levels[j].1);
                        total_pairwise_dist[wi] += manhattan as u64;
                        total_pairs[wi] += 1;

                        // Density rule: max(manhattan, |bfs_diff|) <= 4
                        let bfs_diff = match (bfs_distances.get(&levels[i]), bfs_distances.get(&levels[j])) {
                            (Some(&a), Some(&b)) => a.abs_diff(b),
                            _ => manhattan,
                        };
                        if manhattan.max(bfs_diff) <= 4 {
                            density_violations[wi] += 1;
                        }
                    }
                }

                // === Dead-end utilization ===
                for &pos in &candidates {
                    if is_dead_end(&built.grid, pos) {
                        total_dead_end_candidates[wi] += 1;
                        if levels.contains(&pos) {
                            dead_ends_picked[wi] += 1;
                        }
                    }
                }
                levels_picked[wi] += levels.len() as u64;
                total_candidates[wi] += candidates.len() as u64;

                // === Path detour: levels vs candidate baseline ===
                // Only positions reachable in BOTH directions count — unreachable
                // positions can't have a meaningful detour relative to a route
                // they're not on.
                if let Some(tp) = target_pos {
                    let reverse_walk = walk_map(&built.grid, &built.pipe_pairs, Some(tp));
                    if let Some(&td) = bfs_distances.get(&tp) {
                        for &pos in &levels {
                            if let (Some(&fwd), Some(&rev)) = (
                                bfs_distances.get(&pos),
                                reverse_walk.distances.get(&pos),
                            ) {
                                let detour = (fwd + rev).saturating_sub(td);
                                total_level_detour[wi] += detour as u64;
                                total_levels_for_detour[wi] += 1;
                            }
                        }
                        for &pos in &candidates {
                            if let (Some(&fwd), Some(&rev)) = (
                                bfs_distances.get(&pos),
                                reverse_walk.distances.get(&pos),
                            ) {
                                let detour = (fwd + rev).saturating_sub(td);
                                total_candidate_detour[wi] += detour as u64;
                                total_candidates_for_detour[wi] += 1;
                            }
                        }
                    }
                }
            }
        }

        eprintln!("\n=== Level Placement Quality over {seeds} seeds ===");

        for wi in 0..8 {
            if total_candidates[wi] == 0 {
                continue;
            }

            eprintln!("\n--- W{} ---", wi + 1);

            // Spread
            if total_pairs[wi] > 0 {
                let avg_pair = total_pairwise_dist[wi] as f64 / total_pairs[wi] as f64;
                let dens_pct = density_violations[wi] as f64 / total_pairs[wi] as f64 * 100.0;
                eprintln!("  Spread:");
                eprintln!("    Avg pairwise level distance: {avg_pair:.1} tiles");
                eprintln!(
                    "    Density violations (combined radius <=4): {} / {} pairs ({:.1}%)",
                    density_violations[wi], total_pairs[wi], dens_pct,
                );
            }

            // Dead-end bonus
            let dead_end_util = if total_dead_end_candidates[wi] > 0 {
                dead_ends_picked[wi] as f64 / total_dead_end_candidates[wi] as f64 * 100.0
            } else { 0.0 };
            let random_baseline = levels_picked[wi] as f64 / total_candidates[wi] as f64 * 100.0;
            let lift = dead_end_util - random_baseline;
            eprintln!("  Dead-end bonus (+0.5):");
            eprintln!(
                "    Dead-end candidates: {} ({:.1}% of all candidates)",
                total_dead_end_candidates[wi],
                total_dead_end_candidates[wi] as f64 / total_candidates[wi] as f64 * 100.0,
            );
            eprintln!("    Picked as level:     {dead_end_util:.1}%");
            eprintln!("    Random baseline:     {random_baseline:.1}%");
            eprintln!(
                "    Lift: {lift:+.1} pp  ({})",
                if lift.abs() < 2.0 { "negligible" }
                else if lift > 0.0 { "bias toward dead-ends" }
                else { "bias against dead-ends" },
            );

            // Path bonus
            if total_levels_for_detour[wi] > 0 && total_candidates_for_detour[wi] > 0 {
                let avg_lvl = total_level_detour[wi] as f64 / total_levels_for_detour[wi] as f64;
                let avg_cand = total_candidate_detour[wi] as f64 / total_candidates_for_detour[wi] as f64;
                let bias = avg_lvl - avg_cand;
                eprintln!("  Path bonus (max = PATH_DETOUR_CAP * W_PATH):");
                eprintln!("    Avg detour for placed levels: {avg_lvl:.2} hops");
                eprintln!("    Avg detour for all candidates: {avg_cand:.2} hops");
                eprintln!(
                    "    Bias: {bias:+.2} hops  ({})",
                    if bias.abs() < 0.3 { "negligible" }
                    else if bias < 0.0 { "toward main route" }
                    else { "off main route" },
                );
            }
        }

        // One-line summary for diffing
        eprintln!("\n=== Summary (avg pairwise distance / dead-end lift / path bias) ===");
        for wi in 0..8 {
            if total_candidates[wi] == 0 { continue; }
            let avg_pair = if total_pairs[wi] > 0 {
                total_pairwise_dist[wi] as f64 / total_pairs[wi] as f64
            } else { 0.0 };
            let dead_end_util = if total_dead_end_candidates[wi] > 0 {
                dead_ends_picked[wi] as f64 / total_dead_end_candidates[wi] as f64 * 100.0
            } else { 0.0 };
            let random_baseline = levels_picked[wi] as f64 / total_candidates[wi] as f64 * 100.0;
            let lift = dead_end_util - random_baseline;
            let path_bias = if total_levels_for_detour[wi] > 0 && total_candidates_for_detour[wi] > 0 {
                let avg_lvl = total_level_detour[wi] as f64 / total_levels_for_detour[wi] as f64;
                let avg_cand = total_candidate_detour[wi] as f64 / total_candidates_for_detour[wi] as f64;
                avg_lvl - avg_cand
            } else { 0.0 };
            eprintln!(
                "  W{}: dist={avg_pair:5.1}  dead-end-lift={lift:+5.1}pp  path-bias={path_bias:+5.2}",
                wi + 1,
            );
        }
    }

    /// Required-progression analyzer.
    ///
    /// Per world, computes the minimum number of fortress + level entries
    /// the player must clear to reach the airship/Bowser. Locks block path
    /// tiles until the fortress whose section opens them is cleared; pipes
    /// are taken whenever they shorten the route. Also reports a "hammer
    /// mode" where all locks start open, isolating fortresses that were
    /// only required because of lock gating.
    ///
    /// Run with: cargo test --release test_required_progression -- --ignored --nocapture
    /// Override seed count with PROG_SEEDS=N.
    /// Toggle start↔airship swap with SAS=1.
    #[test]
    #[ignore]
    fn test_required_progression() {
        let rom = match load_rom() {
            Some(r) => r,
            None => {
                eprintln!("ROM not found, skipping");
                return;
            }
        };
        let rom = apply_qol_for_overworld(&rom);

        let seeds: u64 = std::env::var("PROG_SEEDS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1000);

        // Per-world tallies: sums for mean, plus min/max.
        let mut sum_forts = [0u64; 8];
        let mut sum_levels = [0u64; 8];
        let mut sum_h_forts = [0u64; 8];
        let mut sum_h_levels = [0u64; 8];
        let mut min_forts = [usize::MAX; 8];
        let mut max_forts = [0usize; 8];
        let mut min_levels = [usize::MAX; 8];
        let mut max_levels = [0usize; 8];
        let mut min_h_forts = [usize::MAX; 8];
        let mut max_h_forts = [0usize; 8];
        let mut unreachable = [0u32; 8];
        let mut unreachable_seeds: [Vec<u64>; 8] = Default::default();
        // "Trivial bypass" = hammerless playthrough requires 0 forts AND 0
        // levels (player walks/pipes straight to the airship). Tracked per
        // world plus classified by whether the path uses a pipe right after
        // start (pipe_start), right before target (pipe_target), both, or
        // neither — diagnostic that pinpoints the failure mode.
        let mut zero_zero = [0u32; 8];
        let mut zero_zero_seeds: [Vec<u64>; 8] = Default::default();
        let mut bypass_both = [0u32; 8];
        let mut bypass_start = [0u32; 8];
        let mut bypass_target = [0u32; 8];
        let mut bypass_other = [0u32; 8];

        for seed in 0..seeds {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let (catalog, pickup) = build_catalog_pickup(&rom, seed);
            let result = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, true);

            for built in &result.worlds {
                let wi = built.world_idx;
                let no_hammer = analyze_required_progression(built, false);
                let with_hammer = analyze_required_progression(built, true);

                if !no_hammer.reachable {
                    unreachable[wi] += 1;
                    if unreachable_seeds[wi].len() < 5 {
                        unreachable_seeds[wi].push(seed);
                    }
                    continue;
                }
                if no_hammer.forts_required == 0 && no_hammer.levels_required == 0 {
                    zero_zero[wi] += 1;
                    if zero_zero_seeds[wi].len() < 5 {
                        zero_zero_seeds[wi].push(seed);
                    }
                    let path = &no_hammer.path;
                    let pipe_after_start = path.get(1).is_some_and(|(_, k)| matches!(k, PathNodeKind::Pipe));
                    let pipe_before_target = path.len() >= 2
                        && matches!(path[path.len() - 2].1, PathNodeKind::Pipe);
                    match (pipe_after_start, pipe_before_target) {
                        (true, true)  => bypass_both[wi]  += 1,
                        (true, false) => bypass_start[wi] += 1,
                        (false, true) => bypass_target[wi] += 1,
                        (false, false)=> bypass_other[wi] += 1,
                    }
                }
                sum_forts[wi] += no_hammer.forts_required as u64;
                sum_levels[wi] += no_hammer.levels_required as u64;
                sum_h_forts[wi] += with_hammer.forts_required as u64;
                sum_h_levels[wi] += with_hammer.levels_required as u64;

                min_forts[wi] = min_forts[wi].min(no_hammer.forts_required);
                max_forts[wi] = max_forts[wi].max(no_hammer.forts_required);
                min_levels[wi] = min_levels[wi].min(no_hammer.levels_required);
                max_levels[wi] = max_levels[wi].max(no_hammer.levels_required);
                min_h_forts[wi] = min_h_forts[wi].min(with_hammer.forts_required);
                max_h_forts[wi] = max_h_forts[wi].max(with_hammer.forts_required);
            }
        }

        let sas_label = if std::env::var("SAS").is_ok() { " [SAS=1]" } else { "" };
        eprintln!("\n=== Required Progression to Airship ({seeds} seeds{sas_label}) ===");
        eprintln!();
        eprintln!(
            "{:<4} {:>8} {:>8}  {:>8} {:>8}   {:>8} {:>8}  {:>8}",
            "", "forts", "(range)", "levels", "(range)", "h-forts", "(range)", "saves",
        );

        let mut grand_forts = 0u64;
        let mut grand_levels = 0u64;
        let mut grand_h_forts = 0u64;
        let mut grand_h_levels = 0u64;

        for wi in 0..8 {
            let seeds_ok = (seeds as u32 - unreachable[wi]) as f64;
            if seeds_ok == 0.0 {
                eprintln!("  W{}: (no reachable seeds)", wi + 1);
                continue;
            }
            let avg_f = sum_forts[wi] as f64 / seeds_ok;
            let avg_l = sum_levels[wi] as f64 / seeds_ok;
            let avg_hf = sum_h_forts[wi] as f64 / seeds_ok;
            let saves = avg_f - avg_hf;

            grand_forts += sum_forts[wi];
            grand_levels += sum_levels[wi];
            grand_h_forts += sum_h_forts[wi];
            grand_h_levels += sum_h_levels[wi];

            eprintln!(
                "  W{}  {:>6.2}   {}-{:<3}  {:>6.2}   {}-{:<3}    {:>6.2}   {}-{:<3}   {:>5.2}",
                wi + 1,
                avg_f, min_forts[wi], max_forts[wi],
                avg_l, min_levels[wi], max_levels[wi],
                avg_hf, min_h_forts[wi], max_h_forts[wi],
                saves,
            );
        }

        let avg_total_forts = grand_forts as f64 / seeds as f64;
        let avg_total_levels = grand_levels as f64 / seeds as f64;
        let avg_total_h_forts = grand_h_forts as f64 / seeds as f64;
        let avg_total_h_levels = grand_h_levels as f64 / seeds as f64;
        eprintln!();
        eprintln!("  Per-seed totals (excludes the 8 objectives):");
        eprintln!(
            "    Without hammer: {:.2} forts + {:.2} levels  =  {:.2} clears",
            avg_total_forts, avg_total_levels, avg_total_forts + avg_total_levels,
        );
        eprintln!(
            "    With hammer:    {:.2} forts + {:.2} levels  =  {:.2} clears  (saves {:.2})",
            avg_total_h_forts, avg_total_h_levels,
            avg_total_h_forts + avg_total_h_levels,
            (avg_total_forts + avg_total_levels) - (avg_total_h_forts + avg_total_h_levels),
        );

        let total_unreach: u32 = unreachable.iter().sum();
        if total_unreach > 0 {
            eprintln!("\n  WARNING: {total_unreach} unreachable-target case(s) — builder bug?");
            for (wi, &count) in unreachable.iter().enumerate() {
                if count > 0 {
                    let pct = count as f64 / seeds as f64 * 100.0;
                    let seed_examples: Vec<String> = unreachable_seeds[wi]
                        .iter()
                        .map(u64::to_string)
                        .collect();
                    eprintln!(
                        "    W{}: {count}/{seeds} ({pct:.1}%)  example seeds: {}",
                        wi + 1,
                        seed_examples.join(", "),
                    );
                }
            }
        }

        let total_zero_zero: u32 = zero_zero.iter().sum();
        let total_world_seeds = seeds as u32 * 8;
        let overall_pct = total_zero_zero as f64 / total_world_seeds as f64 * 100.0;
        eprintln!(
            "\n  Trivial-bypass (0 forts + 0 levels) — overall {total_zero_zero}/{total_world_seeds} ({overall_pct:.2}%):"
        );
        for (wi, &count) in zero_zero.iter().enumerate() {
            let pct = count as f64 / seeds as f64 * 100.0;
            let examples = if zero_zero_seeds[wi].is_empty() {
                String::new()
            } else {
                let s: Vec<String> = zero_zero_seeds[wi].iter().map(u64::to_string).collect();
                format!("  example seeds: {}", s.join(", "))
            };
            eprintln!("    W{}: {count}/{seeds} ({pct:.1}%){examples}", wi + 1);
        }
        let tb = bypass_both.iter().sum::<u32>();
        let ts = bypass_start.iter().sum::<u32>();
        let tt = bypass_target.iter().sum::<u32>();
        let to_ = bypass_other.iter().sum::<u32>();
        if total_zero_zero > 0 {
            eprintln!(
                "  Bypass classification — pipe-both: {tb}, pipe-start-only: {ts}, pipe-target-only: {tt}, neither: {to_}",
            );
        }
    }

    /// Single-seed dump of the required-progression analysis. Intended for
    /// verification by eye — prints the fortress/lock inventory and the
    /// step-by-step path Dijkstra picked, both without and with hammer.
    ///
    /// Run with:
    ///   DUMP_SEED=0 DUMP_WORLD=4 cargo test --release \
    ///     test_dump_required_progression -- --ignored --nocapture
    /// Omit DUMP_WORLD to print all 8 worlds.
    #[test]
    #[ignore]
    fn test_dump_required_progression() {
        use crate::Options;

        let rom_bytes = match std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") {
            Ok(b) => b,
            Err(_) => {
                eprintln!("ROM not found, skipping");
                return;
            }
        };

        let seed: u64 = std::env::var("DUMP_SEED")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let world_filter: Option<usize> = std::env::var("DUMP_WORLD")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .map(|w| w.saturating_sub(1));

        // PROBE=1: print the vanilla world grid (post-QoL) for the chosen
        // DUMP_WORLD, then exit. Used to inspect map topology without any
        // build randomization.
        if std::env::var("PROBE").is_ok() {
            let rom = Rom::from_bytes(&rom_bytes).unwrap();
            let rom = apply_qol_for_overworld(&rom);
            let wi = world_filter.unwrap_or(2); // default W3
            let grid = rom_data::read_tile_grid(&rom, wi);
            eprintln!("=== Vanilla W{} grid (post-QoL) ===", wi + 1);
            for r in 0..grid.rows {
                eprint!("  r{r:1}:");
                for c in 0..grid.cols {
                    eprint!(" {:02X}", grid.get(r, c));
                }
                eprintln!();
            }
            return;
        }

        // STANDALONE=1 bypasses the full pipeline and runs the builder
        // directly off a fresh `seed_from_u64(seed)` RNG, matching what the
        // distribution analyzer (test_required_progression) sees. Use this
        // to reproduce unreachable-target findings reported by that test.
        if std::env::var("STANDALONE").is_ok() {
            let rom = match Rom::from_bytes(&rom_bytes) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("ROM parse failed: {e}");
                    return;
                }
            };
            let rom = apply_qol_for_overworld(&rom);
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let (catalog, pickup) = build_catalog_pickup(&rom, seed);
            let result = build(
                &rom,
                &OverworldData { pickup: &pickup, catalog: &catalog },
                &mut rng,
                true,
            );
            let sas_label = if std::env::var("SAS").is_ok() {
                " [SAS=1]"
            } else {
                ""
            };
            eprintln!("=== Required Progression dump (seed={seed}{sas_label}, STANDALONE) ===");
            for built in &result.worlds {
                if let Some(w) = world_filter
                    && built.world_idx != w
                {
                    continue;
                }
                dump_required_progression(built);
                // GRID=1: also print the post-build grid for visual inspection.
                if std::env::var("GRID").is_ok() {
                    eprintln!("\n  Post-build grid:");
                    for r in 0..built.grid.rows {
                        eprint!("    r{r:1}:");
                        for c in 0..built.grid.cols {
                            eprint!(" {:02X}", built.grid.get(r, c));
                        }
                        eprintln!();
                    }
                    if let (Some(start), Some(target)) = (
                        rom_data::find_start(&built.grid),
                        find_target(&built.grid, built.world_idx),
                    ) {
                        let probe = |grid: &Grid, label: &str, pos: (usize, usize)| {
                            let r = pos.0 as i32 - 1;
                            let c = pos.1 as i32 - 1;
                            let dirs = [
                                ("N", r, pos.1 as i32),
                                ("S", pos.0 as i32 + 1, pos.1 as i32),
                                ("W", pos.0 as i32, c),
                                ("E", pos.0 as i32, pos.1 as i32 + 1),
                            ];
                            eprintln!("  {label}={pos:?} tile=0x{:02X}", grid.get(pos.0, pos.1));
                            for (d, rr, cc) in dirs {
                                if rr < 0 || cc < 0 || rr as usize >= grid.rows || cc as usize >= grid.cols {
                                    eprintln!("    {d} ({rr},{cc}): off-grid");
                                } else {
                                    eprintln!("    {d} ({rr},{cc}): 0x{:02X}", grid.get(rr as usize, cc as usize));
                                }
                            }
                        };
                        probe(&built.grid, "start", start);
                        probe(&built.grid, "target", target);

                        // What does walk_map see as reachable from start?
                        let walk = walk_map(&built.grid, &built.pipe_pairs, Some(start));
                        let mut reachable: Vec<(usize, usize)> = walk.nodes.iter().copied().collect();
                        reachable.sort();
                        eprintln!("\n  walk_map reachable from start ({} nodes):", reachable.len());
                        for pos in &reachable {
                            eprintln!("    {pos:?} tile=0x{:02X}", built.grid.get(pos.0, pos.1));
                        }
                    }
                }
            }
            return;
        }

        // Build Options from either a FLAGS=SMB3R-... key (preferred — covers
        // every randomizer toggle) or fall back to `Options::default()` plus
        // an `SAS=1` override. This matches what the user would pass to the
        // CLI/web, so the RNG sequence reaching the overworld builder is the
        // one a real playthrough sees.
        let mut options = match std::env::var("FLAGS") {
            Ok(key) => match crate::Options::from_flag_key(&key) {
                Ok(o) => o,
                Err(e) => {
                    eprintln!("Invalid FLAGS key: {e}");
                    return;
                }
            },
            Err(_) => Options::default(),
        };
        if std::env::var("SAS").is_ok() {
            options.swap_start_airship = true;
        }
        // Palettes (both character-only and themed) use a fresh OS RNG, so
        // they introduce noise that breaks reproducibility without affecting
        // the topology this analyzer cares about. Force both off so identical
        // (seed, flags) inputs produce identical ROM bytes.
        options.palettes = false;
        options.palette_themed = false;

        let (rom, result) = match crate::randomize_rom_with_overworld_capture(
            &rom_bytes, seed, &options, None,
        ) {
            Ok(pair) => pair,
            Err(e) => {
                eprintln!("randomize_rom_with_overworld_capture failed: {e}");
                return;
            }
        };

        let sas_label = if options.swap_start_airship { " [SAS=1]" } else { "" };
        let flag_key = options.to_flag_key();
        eprintln!("=== Required Progression dump (seed={seed}{sas_label}) ===");
        eprintln!("Flags: {flag_key}");

        for built in &result.worlds {
            if let Some(w) = world_filter
                && built.world_idx != w
            {
                continue;
            }
            dump_required_progression(built);
        }

        // Save the fully-randomized ROM (matches the real playthrough state).
        let sas_tag = if options.swap_start_airship { "_sas" } else { "" };
        let filename = format!("progression_seed{seed}{sas_tag}.nes");
        std::fs::write(&filename, rom.output_bytes()).unwrap();
        eprintln!("\nWrote {filename}");
    }
}
