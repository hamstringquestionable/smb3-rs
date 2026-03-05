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

use std::collections::{HashSet, VecDeque};

use rand::Rng;
use rand::seq::{IndexedRandom, SliceRandom};

use super::map_walker::walk_map;
use super::node_catalog::{NodeCatalog, NodeKind};
use super::overworld_helpers::{find_target, gap_tile_for, LOCKABLE_TILES};
use super::overworld_pickup::{ClearedWorld, PickupResult};
use crate::rom::Rom;
use super::rom_data::{
    self, BACKGROUND_TILES, Grid, TILE_PIPE, TILE_FORTRESS,
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
    /// The gap tile to write (0x54 lock, 0x56 bridge, 0x9D water, 0xE4 sky).
    pub gap_tile: u8,
    /// The original path tile (for FX restore).
    pub replace_tile: u8,
    /// Which fortress (section index) opens this lock.
    pub fort_section: usize,
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
    // Apply QoL map tile patches to the cleared grids so BFS sees the same
    // connectivity the player will. These modify path tiles only (not nodes),
    // so they don't affect blank slot counts.
    let mut patched_worlds: Vec<ClearedWorld> = pickup.worlds.clone();
    apply_qol_grid_patches(&mut patched_worlds);

    // Step 0: redistribute fortresses
    let fort_counts = redistribute_fortresses(rng);

    // Pre-compute available blank slots per world so we can distribute
    // levels proportionally. Restore airship/Bowser tiles first (they were
    // blanked in pickup) and exclude only toad houses + floating sprite
    // positions — all other blank tiles are valid placement slots.
    let mut capacities = [0usize; 8];
    for wi in 0..8 {
        let mut grid = patched_worlds[wi].grid.clone();
        // Restore airship/Bowser tiles
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
        let fixed = fixed_positions_for_world(rom, catalog, wi);
        let pipe_endpoints = VANILLA_PIPE_PAIRS[wi] * 2;
        let blanks = count_blank_tiles(&grid, &fixed);
        capacities[wi] = blanks.saturating_sub(pipe_endpoints + fort_counts[wi]);
    }

    // Distribute VANILLA_LEVEL_COUNT levels across worlds proportionally to capacity.
    let level_counts = distribute_levels(&capacities, VANILLA_LEVEL_COUNT);

    let mut worlds = Vec::with_capacity(8);
    for wi in 0..8 {
        let built = build_world(
            wi,
            rom,
            &patched_worlds[wi],
            catalog,
            fort_counts[wi],
            level_counts[wi],
            VANILLA_PIPE_PAIRS[wi],
            rng,
        );
        worlds.push(built);
    }

    BuildResult { worlds, fort_counts }
}

/// Apply QoL map tile patches that affect overworld connectivity.
///
/// These patches open paths that the player will always see (default-on QoL),
/// so the build phase must account for them when computing BFS reachability.
fn apply_qol_grid_patches(worlds: &mut [ClearedWorld]) {
    // W2 rock at grid (0, 21): $51 → $45 (horizontal path)
    let w2 = &mut worlds[1].grid;
    if w2.cols > 21 && w2.get(0, 21) == 0x51 {
        w2.set(0, 21, 0x45);
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

/// Count blank tiles on a grid, excluding fixed positions.
fn count_blank_tiles(grid: &Grid, fixed: &HashSet<(usize, usize)>) -> usize {
    let blank_tiles: &[u8] = &[
        0x44, 0x47, 0x48, 0x4A,
        0xAE, 0xAF, 0xB5, 0xB6,
        0xD9, 0xDC, 0xDE,
    ];
    let mut count = 0;
    for r in 0..grid.rows {
        for c in 0..grid.cols {
            if !fixed.contains(&(r, c)) && blank_tiles.contains(&grid.get(r, c)) {
                count += 1;
            }
        }
    }
    count
}

/// Distribute `total` levels across worlds proportional to capacity.
/// Ensures every level is placed (sum of output == total).
fn distribute_levels(capacities: &[usize; 8], total: usize) -> [usize; 8] {
    let total_cap: usize = capacities.iter().sum();
    let mut counts = [0usize; 8];

    if total_cap == 0 || total == 0 {
        return counts;
    }

    // Proportional allocation
    let mut remaining = total;
    for wi in 0..8 {
        let share = (capacities[wi] as f64 / total_cap as f64 * total as f64).round() as usize;
        counts[wi] = share.min(capacities[wi]).min(remaining);
        remaining -= counts[wi];
    }

    // Distribute any leftover (rounding errors) to worlds with spare capacity
    for wi in 0..8 {
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
    cleared: &ClearedWorld,
    catalog: &NodeCatalog,
    fort_count: usize,
    level_count: usize,
    pipe_pair_count: usize,
    rng: &mut R,
) -> BuiltWorld {
    let mut grid = cleared.grid.clone();

    // Restore airship and Bowser tiles — they were blanked during pickup but
    // stay at their vanilla positions.
    for entry in &catalog.entries {
        if entry.world_idx != world_idx {
            continue;
        }
        if matches!(entry.kind, NodeKind::Airship | NodeKind::Bowser) {
            let (r, c) = entry.grid_pos;
            if r < grid.rows && c < grid.cols {
                grid.set(r, c, entry.tile);
            }
        }
    }

    let start_pos = rom_data::find_start(&grid);
    let target_pos = find_target(&grid, world_idx);

    // Fixed positions: airship, Bowser, toad houses, and floating sprite
    // positions. HammerBro catalog entries are NOT excluded — those blank
    // tiles are valid placement slots for levels/forts/pipes.
    let fixed_positions = fixed_positions_for_world(rom, catalog, world_idx);

    // Collect all blank node positions (potential placement slots).
    let blank_positions = find_blank_slots(&grid, &fixed_positions);

    // Collect fixed pipe endpoints for this world.
    let fixed_pipe_eps: Vec<(usize, usize)> = FIXED_PIPE_ENDPOINTS
        .iter()
        .filter(|(wi, _)| *wi == world_idx)
        .map(|(_, pos)| *pos)
        .collect();

    // Step 1: Place pipes
    let pipe_pairs = place_pipes(
        &mut grid,
        &blank_positions,
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

    // Step 3: Populate sections
    let slots = populate_sections(&sections, fort_count, level_count, &pipe_positions, rng);

    // Step 4: Lock placement
    let locks = place_locks(
        &grid,
        &pipe_pairs,
        start_pos,
        target_pos,
        &slots,
        &sections,
        fort_count,
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
    let blank_tiles: &[u8] = &[
        0x44, 0x47, 0x48, 0x4A, 0x4B, // standard
        0xAE, 0xAF, 0xB5, 0xB6,       // island
        0xD9, 0xDC, 0xDE,             // sky
    ];

    let mut blanks = Vec::new();
    for r in 0..grid.rows {
        for c in 0..grid.cols {
            let pos = (r, c);
            if fixed_positions.contains(&pos) {
                continue;
            }
            if blank_tiles.contains(&grid.get(r, c)) {
                blanks.push(pos);
            }
        }
    }
    blanks
}

/// BFS from start, returning nodes in visit order with their distances.
pub(super) fn bfs_ordered(
    grid: &Grid,
    pipe_pairs: &[((usize, usize), (usize, usize))],
    start_pos: Option<(usize, usize)>,
) -> Vec<((usize, usize), usize)> {
    let start = match start_pos {
        Some(s) => s,
        None => return Vec::new(),
    };

    let mut pipe_lookup: std::collections::HashMap<(usize, usize), Vec<(usize, usize)>> =
        std::collections::HashMap::new();
    for &(a, b) in pipe_pairs {
        pipe_lookup.entry(a).or_default().push(b);
        pipe_lookup.entry(b).or_default().push(a);
    }

    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    let mut result = Vec::new();

    visited.insert(start);
    queue.push_back((start, 0usize));

    while let Some(((r, c), dist)) = queue.pop_front() {
        result.push(((r, c), dist));

        // Orthogonal 2-tile movement
        for &(dr, dc, is_horz) in &[(0i8, 1i8, true), (0, -1, true), (1, 0, false), (-1, 0, false)] {
            let pr = r as i16 + dr as i16;
            let pc = c as i16 + dc as i16;
            if pr < 0 || pr >= grid.rows as i16 || pc < 0 || pc >= grid.cols as i16 {
                continue;
            }
            let (pr, pc) = (pr as usize, pc as usize);

            let path_tile = grid.get(pr, pc);
            let valid = if is_horz { VALID_HORZ } else { VALID_VERT };
            if !valid.contains(&path_tile) {
                continue;
            }

            let nr = r as i16 + 2 * dr as i16;
            let nc = c as i16 + 2 * dc as i16;
            if nr < 0 || nr >= grid.rows as i16 || nc < 0 || nc >= grid.cols as i16 {
                continue;
            }
            let (nr, nc) = (nr as usize, nc as usize);

            let dest_tile = grid.get(nr, nc);
            if BACKGROUND_TILES.contains(&dest_tile) {
                continue;
            }

            if !visited.contains(&(nr, nc)) {
                visited.insert((nr, nc));
                queue.push_back(((nr, nc), dist + 1));
            }
        }

        // Pipe teleport edges
        if let Some(dests) = pipe_lookup.get(&(r, c)) {
            for &dest in dests {
                if !visited.contains(&dest) {
                    visited.insert(dest);
                    queue.push_back((dest, dist + 1));
                }
            }
        }
    }

    result
}

// ---------------------------------------------------------------------------
// Step 1: Pipe placement
// ---------------------------------------------------------------------------

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

    // Phase 0: fixed endpoints — place these first, partner chosen randomly
    for &fixed_pos in fixed_endpoints {
        if placed_pairs.len() >= pair_count {
            break;
        }
        grid.set(fixed_pos.0, fixed_pos.1, TILE_PIPE);
        used_positions.insert(fixed_pos);

        // Pick a random partner from available blank slots
        let available: Vec<(usize, usize)> = blank_positions
            .iter()
            .copied()
            .filter(|p| !used_positions.contains(p))
            .collect();

        if let Some(&partner) = available.choose(rng) {
            grid.set(partner.0, partner.1, TILE_PIPE);
            used_positions.insert(partner);
            placed_pairs.push((fixed_pos, partner));
        }
    }

    // Phase A: connectivity pipes — connect unreachable areas until target is reachable
    let target_reachable = |g: &Grid, pairs: &[((usize, usize), (usize, usize))]| -> bool {
        if let Some(tp) = target_pos {
            let walk = walk_map(g, pairs, start_pos);
            walk.nodes.contains(&tp)
        } else {
            true // no target = nothing to connect
        }
    };

    while placed_pairs.len() < pair_count && !target_reachable(grid, &placed_pairs) {
        let walk = walk_map(grid, &placed_pairs, start_pos);
        let reachable = &walk.nodes;

        // Find blank slots split into reachable and unreachable
        let reachable_blanks: Vec<(usize, usize)> = blank_positions
            .iter()
            .copied()
            .filter(|p| reachable.contains(p) && !used_positions.contains(p))
            .collect();
        let unreachable_blanks: Vec<(usize, usize)> = blank_positions
            .iter()
            .copied()
            .filter(|p| !reachable.contains(p) && !used_positions.contains(p))
            .collect();

        if reachable_blanks.is_empty() || unreachable_blanks.is_empty() {
            break; // can't connect anything more
        }

        let &a = reachable_blanks.choose(rng).unwrap();
        let &b = unreachable_blanks.choose(rng).unwrap();

        grid.set(a.0, a.1, TILE_PIPE);
        grid.set(b.0, b.1, TILE_PIPE);
        used_positions.insert(a);
        used_positions.insert(b);
        placed_pairs.push((a, b));
    }

    // Phase B: remaining pipes — try to connect more unreachable islands
    while placed_pairs.len() < pair_count {
        let walk = walk_map(grid, &placed_pairs, start_pos);
        let reachable = &walk.nodes;

        let reachable_blanks: Vec<(usize, usize)> = blank_positions
            .iter()
            .copied()
            .filter(|p| reachable.contains(p) && !used_positions.contains(p))
            .collect();
        let unreachable_blanks: Vec<(usize, usize)> = blank_positions
            .iter()
            .copied()
            .filter(|p| !reachable.contains(p) && !used_positions.contains(p))
            .collect();

        if !unreachable_blanks.is_empty() && !reachable_blanks.is_empty() {
            // Still islands to connect
            let &a = reachable_blanks.choose(rng).unwrap();
            let &b = unreachable_blanks.choose(rng).unwrap();
            grid.set(a.0, a.1, TILE_PIPE);
            grid.set(b.0, b.1, TILE_PIPE);
            used_positions.insert(a);
            used_positions.insert(b);
            placed_pairs.push((a, b));
        } else {
            // No more islands — place both endpoints in reachable area
            let available: Vec<(usize, usize)> = blank_positions
                .iter()
                .copied()
                .filter(|p| !used_positions.contains(p))
                .collect();

            if available.len() < 2 {
                break; // not enough slots
            }

            let mut chosen: Vec<(usize, usize)> = available.clone();
            chosen.shuffle(rng);
            let a = chosen[0];
            let b = chosen[1];

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

    if assignable.is_empty() || section_count == 0 {
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
    sections: &[Vec<(usize, usize)>],
    fort_count: usize,
    level_count: usize,
    pipe_positions: &HashSet<(usize, usize)>,
    rng: &mut R,
) -> Vec<SlotAssignment> {
    let mut slots = Vec::new();

    // Add pipe slots (not in sections, but tracked)
    for &pos in pipe_positions {
        slots.push(SlotAssignment {
            pos,
            kind: SlotKind::Pipe,
            section: 0, // pipes don't really belong to a section
        });
    }

    // Distribute level_count levels across sections proportionally.
    // Each section also gets 1 fort (if si < fort_count).
    let total_section_slots: usize = sections.iter().map(|s| s.len()).sum();
    let mut levels_remaining = level_count;

    for (si, section) in sections.iter().enumerate() {
        if section.is_empty() {
            continue;
        }

        // Pick a random position for the fortress (if we still need one)
        let has_fort = si < fort_count;
        let fort_idx = if has_fort {
            Some(rng.random_range(..section.len()))
        } else {
            None
        };

        // Compute how many levels this section gets (proportional to its size)
        let section_levels = if si == sections.len() - 1 {
            // Last section gets whatever remains
            levels_remaining
        } else {
            let share = (section.len() as f64 / total_section_slots.max(1) as f64
                * level_count as f64)
                .round() as usize;
            share.min(levels_remaining).min(
                section.len() - if has_fort { 1 } else { 0 },
            )
        };

        // Build list of non-fort slot indices, then pick evenly spaced ones for levels.
        let non_fort: Vec<usize> = (0..section.len())
            .filter(|&i| Some(i) != fort_idx)
            .collect();

        // Choose which non-fort slots get levels using even spacing.
        let mut level_slots: HashSet<usize> = HashSet::new();
        let actual_levels = section_levels.min(non_fort.len());
        if actual_levels > 0 && !non_fort.is_empty() {
            let spacing = non_fort.len() as f64 / actual_levels as f64;
            for k in 0..actual_levels {
                let idx = (k as f64 * spacing + spacing / 2.0).floor() as usize;
                level_slots.insert(non_fort[idx.min(non_fort.len() - 1)]);
            }
        }

        for (i, &pos) in section.iter().enumerate() {
            if Some(i) == fort_idx {
                slots.push(SlotAssignment {
                    pos,
                    kind: SlotKind::Fortress,
                    section: si,
                });
            } else if level_slots.contains(&i) {
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
        levels_remaining = levels_remaining.saturating_sub(actual_levels);
    }

    slots
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
    sections: &[Vec<(usize, usize)>],
    fort_count: usize,
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
                    base_grid.set(slot.pos.0, slot.pos.1, 0x47);
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
                    candidates.push((r, c));
                }
            }
        }

        candidates.shuffle(rng);

        let mut best: Option<((usize, usize), u8, u8, i32)> = None;

        for &cand_pos in &candidates {
            let tile = reference_grid.get(cand_pos.0, cand_pos.1);
            let gap = gap_tile_for(tile);

            // Hard rule: with this lock placed (and earlier locks opened),
            // the fortress must still be reachable from start.
            let test_grid = build_test_grid(Some((cand_pos, gap)));
            let walk = walk_map(&test_grid, pipe_pairs, start_pos);

            if !walk.nodes.contains(&fort_pos) {
                continue; // violates hard rule
            }

            let mut score: i32 = 0;

            // Soft goal 1: blocks next section
            if section_idx + 1 < sections.len() {
                let next_section_blocked = sections[section_idx + 1]
                    .iter()
                    .any(|p| !walk.nodes.contains(p));
                if next_section_blocked {
                    score += 100;
                }
            }

            // Soft goal 2: blocks target (airship/bowser)
            if let Some(tp) = target_pos {
                if !walk.nodes.contains(&tp) {
                    score += 50;
                }
            }

            // Soft goal 3: blocks anything compared to no lock
            if score == 0 {
                let open_grid = build_test_grid(None);
                let walk_open = walk_map(&open_grid, pipe_pairs, start_pos);
                if walk.nodes.len() < walk_open.nodes.len() {
                    score += 25;
                }
            }

            let dominated = match &best {
                Some((_, _, _, best_score)) => score > *best_score,
                None => true,
            };

            if dominated {
                best = Some((cand_pos, gap, tile, score));
            }
        }

        // Place the best lock that passes the hard rule.
        // If no candidate passes, skip this fort (no lock for it).
        if let Some((pos, gap, replace, _score)) = best {
            locked_tiles.insert(pos);
            locks.push(LockAssignment {
                pos,
                gap_tile: gap,
                replace_tile: replace,
                fort_section: section_idx,
            });
        }
    }

    // Post-placement validation: a lock placed for section N might be
    // invalidated by a lock placed later for section M > N. Remove any
    // lock that blocks its own fortress in the final all-locks-placed state.
    let mut valid_locks = Vec::new();
    for lock in &locks {
        let fort_pos = match slots
            .iter()
            .find(|s| s.section == lock.fort_section && s.kind == SlotKind::Fortress)
        {
            Some(s) => s.pos,
            None => {
                valid_locks.push(lock.clone());
                continue;
            }
        };

        // Build grid with all locks placed, but open this one and all earlier
        let mut check_grid = base_grid.clone();
        for l in &locks {
            if l.fort_section < lock.fort_section {
                // Earlier — beaten, lock open
                check_grid.set(l.pos.0, l.pos.1, l.replace_tile);
            } else if l.pos == lock.pos {
                // This lock — also open (testing if fort is reachable to be beaten)
                check_grid.set(l.pos.0, l.pos.1, l.replace_tile);
            } else {
                // Later or same section, different lock — closed
                check_grid.set(l.pos.0, l.pos.1, l.gap_tile);
            }
        }

        let walk = walk_map(&check_grid, pipe_pairs, start_pos);
        if walk.nodes.contains(&fort_pos) {
            valid_locks.push(lock.clone());
        }
        // If fort is unreachable, drop this lock entirely
    }

    valid_locks
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
        let catalog = NodeCatalog::build(&rom);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog);
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
        let catalog = NodeCatalog::build(&rom);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog);

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
        let catalog = NodeCatalog::build(&rom);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog);

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
        let catalog = NodeCatalog::build(&rom);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog);
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
}
