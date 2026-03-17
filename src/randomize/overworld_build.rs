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
    let level_counts = distribute_levels(&capacities, VANILLA_LEVEL_COUNT, rng);

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
            rng,
        );
        worlds.push(built);
    }

    // Ensure at least one lock across all worlds is secret-exit-safe
    // (target reachable with lock closed). 1-F's secret exit doesn't
    // trigger FX replacement, so its lock must not block progression.
    let has_safe = worlds.iter().any(|b| b.locks.iter().any(|l| l.secret_exit_safe));
    if !has_safe {
        // Retry lock placement in shuffled world order until one produces a safe lock.
        // Pass empty sections — scoring loses the "blocks next section" signal but
        // hard rules and other soft goals still apply. Only ~4% of seeds need this.
        let mut retry_order: Vec<usize> = (0..8).collect();
        retry_order.shuffle(rng);
        let empty_sections: Vec<Vec<(usize, usize)>> = Vec::new();
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
                &empty_sections,
                true,
                fort_counts[wi],
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
    let mut slots = populate_sections(&grid, &sections, fort_count, level_count, &pipe_positions, rng);

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
        &sections,
        false, // need_secret_exit_safe handled at build() level
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
    rng: &mut R,
) -> Vec<SlotAssignment> {
    let mut slots = Vec::new();

    // Track all positions that will have completable tiles (numbered level,
    // fortress, or pipe).  When assigning a Level slot, we skip positions
    // adjacent to any completable — this avoids numbered tiles touching and
    // also prevents the row 7/8 Map_Completions bit collision.
    let mut completable: HashSet<(usize, usize)> = pipe_positions.clone();

    // Seed the completable set with existing tiles on the grid that the
    // game's Map_Reload_with_Completions routine would "catch". This
    // includes numbered levels, fortresses, pipes, airships, and any tile
    // that triggers the completion/replacement checks.
    for r in 0..grid.rows {
        for c in 0..grid.cols {
            let t = grid.get(r, c);
            if is_completion_unsafe(t) {
                completable.insert((r, c));
            }
        }
    }

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
            let idx = rng.random_range(..section.len());
            completable.insert(section[idx]);
            Some(idx)
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

        // Build list of non-fort slot indices.
        let non_fort: Vec<usize> = (0..section.len())
            .filter(|&i| Some(i) != fort_idx)
            .collect();

        // Assign levels using even spacing as a preferred order, but skip
        // positions adjacent to any existing completable tile.  Positions
        // that are skipped stay as HammerBro (blank path tile, no numbered
        // tile stamped).
        let mut level_slots: HashSet<usize> = HashSet::new();
        let target = section_levels.min(non_fort.len());

        if target > 0 && !non_fort.is_empty() {
            // Dead-end pass: prefer placing levels at path dead-ends so
            // they don't visually terminate with a blank tile.
            for &idx in &non_fort {
                if level_slots.len() >= target {
                    break;
                }
                let pos = section[idx];
                if is_dead_end(grid, pos) && !is_adjacent_to_completable(pos, &completable) {
                    level_slots.insert(idx);
                    completable.insert(pos);
                }
            }

            // Generate evenly-spaced preferred candidates.
            let spacing = non_fort.len() as f64 / target as f64;
            let mut preferred: Vec<usize> = Vec::with_capacity(target);
            for k in 0..target {
                let idx = (k as f64 * spacing + spacing / 2.0).floor() as usize;
                preferred.push(non_fort[idx.min(non_fort.len() - 1)]);
            }

            // First pass: try preferred positions.
            for &idx in &preferred {
                if level_slots.len() >= target {
                    break;
                }
                if level_slots.contains(&idx) {
                    continue;
                }
                let pos = section[idx];
                if !is_adjacent_to_completable(pos, &completable) {
                    level_slots.insert(idx);
                    completable.insert(pos);
                }
            }

            // Second pass: fill remaining from any non-fort slot.
            if level_slots.len() < target {
                for &idx in &non_fort {
                    if level_slots.len() >= target {
                        break;
                    }
                    if level_slots.contains(&idx) {
                        continue;
                    }
                    let pos = section[idx];
                    if !is_adjacent_to_completable(pos, &completable) {
                        level_slots.insert(idx);
                        completable.insert(pos);
                    }
                }
            }

            // Third pass: relax adjacency to avoid dropping levels, but
            // still enforce the row 7/8 hard constraint (shared completion
            // bit means adjacent completable tiles at rows 7 and 8 in the
            // same column would cause map reload bugs).
            if level_slots.len() < target {
                for &idx in &non_fort {
                    if level_slots.len() >= target {
                        break;
                    }
                    if level_slots.contains(&idx) {
                        continue;
                    }
                    let pos = section[idx];
                    if !is_row78_conflict(pos, &completable) {
                        level_slots.insert(idx);
                        completable.insert(pos);
                    }
                }
            }
        }

        let assigned = level_slots.len();

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
        levels_remaining = levels_remaining.saturating_sub(assigned);
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

/// Returns true if `pos` is orthogonally adjacent to any position in the set.
fn is_adjacent_to_completable(
    pos: (usize, usize),
    completable: &HashSet<(usize, usize)>,
) -> bool {
    let (r, c) = pos;
    let adjacent = [
        (r.wrapping_sub(1), c),
        (r + 1, c),
        (r, c.wrapping_sub(1)),
        (r, c + 1),
    ];
    adjacent.iter().any(|adj| completable.contains(adj))
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
// Step 4: Lock placement
// ---------------------------------------------------------------------------

fn place_locks<R: Rng>(
    grid: &Grid,
    pipe_pairs: &[((usize, usize), (usize, usize))],
    start_pos: Option<(usize, usize)>,
    target_pos: Option<(usize, usize)>,
    slots: &[SlotAssignment],
    sections: &[Vec<(usize, usize)>],
    need_secret_exit_safe: bool,
    fort_count: usize,
    rng: &mut R,
) -> Vec<LockAssignment> {
    let mut locks: Vec<LockAssignment> = Vec::new();
    let mut locked_tiles: HashSet<(usize, usize)> = HashSet::new();
    let mut has_safe_lock = false;

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

        let mut best: Option<((usize, usize), u8, u8, i32, bool)> = None;

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

            // Check if target is reachable with this lock closed (safe for
            // 1-F secret exit which doesn't trigger FX replacement).
            let target_reachable = match target_pos {
                Some(tp) => walk.nodes.contains(&tp),
                None => true,
            };

            // When we need a secret-exit-safe lock and don't have one yet,
            // prefer safe candidates over unsafe ones regardless of score.
            let dominated = match &best {
                Some((_, _, _, best_score, best_safe)) => {
                    if need_secret_exit_safe && !has_safe_lock {
                        // Safe beats unsafe; among same safety, higher score wins
                        match (target_reachable, *best_safe) {
                            (true, false) => true,
                            (false, true) => false,
                            _ => score > *best_score,
                        }
                    } else {
                        score > *best_score
                    }
                }
                None => true,
            };

            if dominated {
                best = Some((cand_pos, gap, tile, score, target_reachable));
            }
        }

        // Place the best lock that passes the hard rule.
        // If no candidate passes, skip this fort (no lock for it).
        if let Some((pos, gap, replace, _score, target_reachable)) = best {
            locked_tiles.insert(pos);
            if target_reachable {
                has_safe_lock = true;
            }
            locks.push(LockAssignment {
                pos,
                gap_tile: gap,
                replace_tile: replace,
                fort_section: section_idx,
                secret_exit_safe: target_reachable,
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

    #[test]
    #[ignore]
    fn test_measure_shortfalls() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = NodeCatalog::build(&rom);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog);

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
}
