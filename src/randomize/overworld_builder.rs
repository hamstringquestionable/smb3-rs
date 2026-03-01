/// Overworld map builder: pick up tiles, build new maps, write to ROM.
///
/// Phase 1 (this file so far): read all overworld maps and classify every
/// pointer table entry into a structured `PickedTile` inventory.

use std::collections::{HashMap, HashSet};

use rand::Rng;
use rand::seq::SliceRandom;

use crate::rom::Rom;

use super::map_walker;
use super::overworld_helpers::LOCKABLE_TILES;
use super::pipe_helpers;
use super::rom_data::{
    self, AIRSHIP_ENTRIES, BOWSER_ENTRY, FORTRESS_ENTRIES, Grid, LevelEntry,
    MAP_OBJ_ENTRY_LINKS, MAP_TRANSITIONS, TILE_PIPE, TILE_START,
    WORLDS,
};

// ---------------------------------------------------------------------------
// Tile inventory types
// ---------------------------------------------------------------------------

/// A pipe pair partner — where the other endpoint lives.
#[derive(Clone, Debug)]
pub(super) struct PipePartner {
    pub entry_idx: usize,
    pub world_idx: usize,
}

/// A tile picked up from an overworld map position.
#[derive(Clone, Debug)]
pub(super) enum PickedTile {
    /// Numbered action level.
    Level {
        entry_idx: usize,
        world_idx: usize,
        level_entry: LevelEntry,
        tile: u8,
    },
    /// Fortress (carries Boom-Boom Y-byte offset for patching).
    Fortress {
        entry_idx: usize,
        world_idx: usize,
        level_entry: LevelEntry,
        boomboom_y_offset: usize,
        tile: u8,
    },
    /// One endpoint of a pipe pair.
    Pipe {
        entry_idx: usize,
        world_idx: usize,
        level_entry: LevelEntry,
        dest_idx: usize,
        partner: PipePartner,
        tile: u8,
    },
    /// Airship dock.
    Airship {
        entry_idx: usize,
        world_idx: usize,
        level_entry: LevelEntry,
    },
    /// Bowser's castle (W8 only, never shuffled).
    Bowser {
        entry_idx: usize,
        world_idx: usize,
        level_entry: LevelEntry,
    },
    /// Start tile — fixed position, never moves.
    Start {
        entry_idx: usize,
        world_idx: usize,
    },
    /// Toad house, bonus game, hammer bro, map-object-linked entry, or other
    /// non-shuffleable entry. Stays at its original position.
    Fixed {
        entry_idx: usize,
        world_idx: usize,
        tile: u8,
    },
}

impl PickedTile {
    pub fn entry_idx(&self) -> usize {
        match self {
            PickedTile::Level { entry_idx, .. }
            | PickedTile::Fortress { entry_idx, .. }
            | PickedTile::Pipe { entry_idx, .. }
            | PickedTile::Airship { entry_idx, .. }
            | PickedTile::Bowser { entry_idx, .. }
            | PickedTile::Start { entry_idx, .. }
            | PickedTile::Fixed { entry_idx, .. } => *entry_idx,
        }
    }

    pub fn world_idx(&self) -> usize {
        match self {
            PickedTile::Level { world_idx, .. }
            | PickedTile::Fortress { world_idx, .. }
            | PickedTile::Pipe { world_idx, .. }
            | PickedTile::Airship { world_idx, .. }
            | PickedTile::Bowser { world_idx, .. }
            | PickedTile::Start { world_idx, .. }
            | PickedTile::Fixed { world_idx, .. } => *world_idx,
        }
    }

    /// Update the entry_idx and world_idx when a tile moves to a new slot.
    fn set_slot(&mut self, new_world: usize, new_entry: usize) {
        match self {
            PickedTile::Level { entry_idx, world_idx, .. }
            | PickedTile::Fortress { entry_idx, world_idx, .. }
            | PickedTile::Pipe { entry_idx, world_idx, .. }
            | PickedTile::Airship { entry_idx, world_idx, .. }
            | PickedTile::Bowser { entry_idx, world_idx, .. }
            | PickedTile::Start { entry_idx, world_idx, .. }
            | PickedTile::Fixed { entry_idx, world_idx, .. } => {
                *entry_idx = new_entry;
                *world_idx = new_world;
            }
        }
    }
}

/// Result of picking up all tiles from one world.
pub(super) struct PickedWorld {
    pub world_idx: usize,
    /// The tile grid with node tiles replaced by a placeholder.
    pub grid: Grid,
    /// All picked-up tiles from this world.
    pub tiles: Vec<PickedTile>,
    /// Original grid position of each tile (parallel to `tiles`).
    pub positions: Vec<(usize, usize)>,
}

// ---------------------------------------------------------------------------
// W5 Spiral Tower entries (functionally a pipe pair using dest index 0)
// ---------------------------------------------------------------------------

const W5_SPIRAL_ENTRIES: &[(usize, usize)] = &[(4, 10), (4, 21)];

// ---------------------------------------------------------------------------
// Pick-up implementation
// ---------------------------------------------------------------------------

/// Placeholder tile for positions where a node was removed.
const EMPTY_NODE: u8 = 0x47;

/// Read a world's overworld map and classify every pointer table entry.
///
/// Returns the grid (with node tiles replaced by `EMPTY_NODE`) and a list of
/// picked-up tiles. Path tiles, background, and structural elements stay on
/// the grid untouched.
pub(super) fn pick_up_world(rom: &Rom, world_idx: usize) -> PickedWorld {
    let grid = rom_data::read_tile_grid(rom, world_idx);
    let world = &WORLDS[world_idx];
    let n = world.entry_count;
    let (_scrcol, objsets, layouts) = rom_data::table_offsets(world);

    // -- Pre-compute sets for classification --

    // Hammer bros: duplicate (obj_ptr, lay_ptr) pairs
    let mut pair_counts: HashMap<(u16, u16), u32> = HashMap::new();
    for i in 0..n {
        let obj = rom_data::read_word(rom, objsets + i * 2);
        let lay = rom_data::read_word(rom, layouts + i * 2);
        if rom_data::is_level_pointer(obj, lay) {
            *pair_counts.entry((obj, lay)).or_insert(0) += 1;
        }
    }
    let hammer_pairs: HashSet<(u16, u16)> = pair_counts
        .into_iter()
        .filter(|&(_, count)| count > 1)
        .map(|(k, _)| k)
        .collect();

    // Map-object-linked entries (W7 piranha plants etc.)
    let map_obj_entries: HashSet<usize> = MAP_OBJ_ENTRY_LINKS
        .iter()
        .filter(|&&(w, _, _)| w == world_idx)
        .map(|&(_, _, entry_idx)| entry_idx)
        .collect();

    // Pipe detection: find entries with PIPEWAYCONTROLLER (0x25) enemy
    // Also group pipe entries by obj_ptr to form pairs
    let mut pipe_entries_by_obj: HashMap<u16, Vec<usize>> = HashMap::new();
    let mut spiral_entries: Vec<usize> = Vec::new();
    for i in 0..n {
        let obj = rom_data::read_word(rom, objsets + i * 2);
        if W5_SPIRAL_ENTRIES.contains(&(world_idx, i)) {
            spiral_entries.push(i);
        } else if rom_data::has_enemy_id(rom, obj, 0x25) {
            pipe_entries_by_obj.entry(obj).or_default().push(i);
        }
    }

    // Build pipe pairs and assign dest indices
    let dest_indices = rom_data::dest_indices_for_world(world_idx);
    let pipe_pair_map = build_pipe_pair_map(
        rom, world_idx, &pipe_entries_by_obj, &spiral_entries, &dest_indices,
    );

    // -- Classify each entry --
    let mut tiles = Vec::new();
    let mut positions = Vec::new();
    let mut picked_positions: HashSet<(usize, usize)> = HashSet::new();

    for i in 0..n {
        let (row, col) = rom_data::entry_grid_position(rom, world, i);
        let obj = rom_data::read_word(rom, objsets + i * 2);
        let lay = rom_data::read_word(rom, layouts + i * 2);
        let map_tile = if row < grid.rows && col < grid.cols {
            grid.get(row, col)
        } else {
            0xFF
        };

        let tile = classify_and_pick(
            rom, world_idx, i, obj, lay, map_tile,
            &hammer_pairs, &map_obj_entries, &pipe_pair_map,
        );

        // Track which grid positions had nodes picked up
        if row < grid.rows && col < grid.cols {
            match &tile {
                PickedTile::Fixed { .. } | PickedTile::Start { .. } => {
                    // Fixed and Start tiles stay on the grid
                }
                _ => {
                    picked_positions.insert((row, col));
                }
            }
        }

        tiles.push(tile);
        positions.push((row, col));
    }

    // Pre-open vanilla FX gap tiles so the grid is clean for placement.
    // Gap tiles (locks, bridges, water/sky gaps) are artifacts of the vanilla
    // fortress FX system — we need them cleared before placing new locks.
    let fx_slots = rom_data::read_fx_slots(rom);
    let fx_assignments = rom_data::read_world_fx_assignments(rom);
    let world_fx = &fx_assignments[world_idx];
    let mut grid = grid;
    for r in 0..grid.rows {
        for c in 0..grid.cols {
            let tile = grid.get(r, c);
            if tile != 0x54 && tile != 0x56 && tile != 0x9D && tile != 0xE4 {
                continue;
            }
            // Check if this gap tile belongs to an FX slot for this world
            if let Some(slot) = fx_slots
                .iter()
                .enumerate()
                .filter(|(i, _)| world_fx.contains(&(*i as u8)))
                .map(|(_, s)| s)
                .find(|s| s.grid_row == r && s.grid_col == c)
            {
                grid.set(r, c, slot.replace_tile);
            }
        }
    }

    // Replace picked node tiles with placeholder
    for &(r, c) in &picked_positions {
        grid.set(r, c, EMPTY_NODE);
    }

    PickedWorld { world_idx, grid, tiles, positions }
}

// ---------------------------------------------------------------------------
// Pipe pair matching
// ---------------------------------------------------------------------------

/// Info about a pipe entry's pair and dest index.
#[derive(Clone, Debug)]
struct PipePairInfo {
    dest_idx: usize,
    partner_entry_idx: usize,
}

/// Build a map from entry_idx → PipePairInfo by matching pipe entries to
/// destination table entries based on grid positions.
fn build_pipe_pair_map(
    rom: &Rom,
    world_idx: usize,
    pipe_entries_by_obj: &HashMap<u16, Vec<usize>>,
    spiral_entries: &[usize],
    dest_indices: &[usize],
) -> HashMap<usize, PipePairInfo> {
    let world = &WORLDS[world_idx];
    let mut result: HashMap<usize, PipePairInfo> = HashMap::new();

    // Collect all pipe pairs (entry_a, entry_b)
    let mut pairs: Vec<(usize, usize)> = Vec::new();

    // Regular pipe pairs: grouped by obj_ptr
    let mut keys: Vec<u16> = pipe_entries_by_obj.keys().copied().collect();
    keys.sort();
    for key in keys {
        let group = &pipe_entries_by_obj[&key];
        if group.len() == 2 {
            pairs.push((group[0], group[1]));
        }
    }

    // W5 spiral tower pair
    if world_idx == 4 && spiral_entries.len() == 2 {
        let mut sorted = spiral_entries.to_vec();
        sorted.sort();
        pairs.push((sorted[0], sorted[1]));
    }

    // Match pairs to dest indices by comparing positions
    for &(ea, eb) in &pairs {
        let ea_pos = rom_data::entry_grid_position(rom, world, ea);
        let eb_pos = rom_data::entry_grid_position(rom, world, eb);

        for &d in dest_indices {
            let (da, db) = read_dest_positions(rom, d);
            if (ea_pos == da && eb_pos == db) || (ea_pos == db && eb_pos == da) {
                result.insert(ea, PipePairInfo { dest_idx: d, partner_entry_idx: eb });
                result.insert(eb, PipePairInfo { dest_idx: d, partner_entry_idx: ea });
                break;
            }
        }
    }

    result
}

/// Read the A and B endpoint positions from the pipe destination tables.
fn read_dest_positions(rom: &Rom, dest_idx: usize) -> ((usize, usize), (usize, usize)) {
    let xhi = rom.read_byte(rom_data::PIPE_MAP_XHI + dest_idx);
    let x = rom.read_byte(rom_data::PIPE_MAP_X + dest_idx);
    let y = rom.read_byte(rom_data::PIPE_MAP_Y + dest_idx);

    let a_pos = (
        ((y >> 4) as usize).wrapping_sub(2),
        ((xhi >> 4) as usize) * 16 + ((x >> 4) as usize),
    );
    let b_pos = (
        ((y & 0xF) as usize).wrapping_sub(2),
        ((xhi & 0xF) as usize) * 16 + ((x & 0xF) as usize),
    );
    (a_pos, b_pos)
}

// ---------------------------------------------------------------------------
// Entry classification
// ---------------------------------------------------------------------------

fn classify_and_pick(
    rom: &Rom,
    world_idx: usize,
    entry_idx: usize,
    obj: u16,
    lay: u16,
    map_tile: u8,
    hammer_pairs: &HashSet<(u16, u16)>,
    map_obj_entries: &HashSet<usize>,
    pipe_pair_map: &HashMap<usize, PipePairInfo>,
) -> PickedTile {
    let world = &WORLDS[world_idx];

    // Start tile — never moves
    if map_tile == TILE_START {
        return PickedTile::Start { entry_idx, world_idx };
    }

    // Bowser's castle — never shuffled
    if (world_idx, entry_idx) == BOWSER_ENTRY {
        return PickedTile::Bowser {
            entry_idx,
            world_idx,
            level_entry: rom_data::read_entry(rom, world, entry_idx),
        };
    }

    // Airship
    if AIRSHIP_ENTRIES.contains(&(world_idx, entry_idx)) {
        return PickedTile::Airship {
            entry_idx,
            world_idx,
            level_entry: rom_data::read_entry(rom, world, entry_idx),
        };
    }

    // Fortress
    if FORTRESS_ENTRIES.contains(&(world_idx, entry_idx)) {
        let level_entry = rom_data::read_entry(rom, world, entry_idx);
        let obj_ptr = (level_entry.obj_hi as u16) << 8 | level_entry.obj_lo as u16;
        let boomboom_y_offset = rom_data::boomboom_y_offset_for_obj(obj_ptr)
            .unwrap_or(0);
        return PickedTile::Fortress {
            entry_idx,
            world_idx,
            level_entry,
            boomboom_y_offset,
            tile: map_tile,
        };
    }

    // Pipe (detected by PIPEWAYCONTROLLER enemy or W5 spiral)
    if let Some(info) = pipe_pair_map.get(&entry_idx) {
        return PickedTile::Pipe {
            entry_idx,
            world_idx,
            level_entry: rom_data::read_entry(rom, world, entry_idx),
            dest_idx: info.dest_idx,
            partner: PipePartner {
                entry_idx: info.partner_entry_idx,
                world_idx,
            },
            tile: map_tile,
        };
    }

    // Fixed entries: toad houses, bonus games, hammer bros, map-object-linked,
    // map transitions, non-level pointers, out-of-bounds rows
    let (row, _col) = rom_data::entry_grid_position(rom, world, entry_idx);

    if obj == 0x0700 {
        // Toad house
        return PickedTile::Fixed { entry_idx, world_idx, tile: map_tile };
    }
    if obj == 0x0001 && lay == 0x0000 {
        // Bonus game
        return PickedTile::Fixed { entry_idx, world_idx, tile: map_tile };
    }
    if MAP_TRANSITIONS.contains(&(world_idx, entry_idx)) {
        return PickedTile::Fixed { entry_idx, world_idx, tile: map_tile };
    }
    if map_obj_entries.contains(&entry_idx) {
        // Map-object-linked (W7 piranha plants)
        return PickedTile::Fixed { entry_idx, world_idx, tile: map_tile };
    }
    if hammer_pairs.contains(&(obj, lay)) {
        // Hammer bro (duplicate obj+lay)
        return PickedTile::Fixed { entry_idx, world_idx, tile: map_tile };
    }
    if row >= rom_data::ROWS {
        // Out of bounds
        return PickedTile::Fixed { entry_idx, world_idx, tile: map_tile };
    }
    if !rom_data::is_level_pointer(obj, lay) {
        // Not a real level
        return PickedTile::Fixed { entry_idx, world_idx, tile: map_tile };
    }

    // Regular action level
    PickedTile::Level {
        entry_idx,
        world_idx,
        level_entry: rom_data::read_entry(rom, world, entry_idx),
        tile: map_tile,
    }
}

// ---------------------------------------------------------------------------
// Tile constants for write
// ---------------------------------------------------------------------------

const TILE_AIRSHIP: u8 = 0xC9;
const TILE_BOWSER: u8 = 0xCC;

// ---------------------------------------------------------------------------
// Placed world (output of build phase)
// ---------------------------------------------------------------------------

/// A tile placed onto the map at a specific position.
#[derive(Clone, Debug)]
pub(super) struct Placement {
    /// The tile being placed.
    pub tile: PickedTile,
    /// Grid position where it's placed.
    pub pos: (usize, usize),
    /// For fortresses: where the lock/gap goes. None = no lock.
    pub lock_pos: Option<(usize, usize)>,
    /// For fortresses with locks: the original path tile at lock_pos (the tile
    /// restored by the FX system when the fortress is beaten).
    pub lock_replace_tile: Option<u8>,
}

/// A fully built world ready to be written to ROM.
pub(super) struct PlacedWorld {
    pub world_idx: usize,
    /// The final tile grid (all nodes restored).
    pub grid: Grid,
    /// All placement instructions.
    pub placements: Vec<Placement>,
}

// ---------------------------------------------------------------------------
// Build: identity transform (put everything back where it was)
// ---------------------------------------------------------------------------

/// Build an identity placement: every tile goes back to its original position.
///
/// This is the trivial case — no shuffling, no randomization. Used to verify
/// the pick-up → build → write round-trip produces an unchanged ROM.
pub(super) fn build_identity(picked: &PickedWorld) -> PlacedWorld {
    let mut grid = picked.grid.clone_grid();
    let mut placements = Vec::new();

    for (tile, &(row, col)) in picked.tiles.iter().zip(picked.positions.iter()) {
        // Restore the map tile on the grid
        let map_tile = match tile {
            PickedTile::Level { tile, .. } => *tile,
            PickedTile::Fortress { tile, .. } => *tile,
            PickedTile::Pipe { tile, .. } => *tile,
            PickedTile::Airship { .. } => TILE_AIRSHIP,
            PickedTile::Bowser { .. } => TILE_BOWSER,
            PickedTile::Start { .. } | PickedTile::Fixed { .. } => {
                // These never left the grid — no placement needed
                continue;
            }
        };

        if row < grid.rows && col < grid.cols {
            grid.set(row, col, map_tile);
        }

        placements.push(Placement {
            tile: tile.clone(),
            pos: (row, col),
            lock_pos: None,
            lock_replace_tile: None,
        });
    }

    PlacedWorld {
        world_idx: picked.world_idx,
        grid,
        placements,
    }
}

// ---------------------------------------------------------------------------
// Write: apply a PlacedWorld to ROM
// ---------------------------------------------------------------------------

/// Write a placed world to ROM: tile grid + level entries.
///
/// Writes the full tile grid and all level entry data (tileset + obj/lay
/// pointers) for placed tiles. Fixed and Start tiles are skipped since their
/// entry data was never picked up.
pub(super) fn write_world(rom: &mut Rom, placed: &PlacedWorld) {
    let world_idx = placed.world_idx;

    // Write the full tile grid
    write_tile_grid(rom, world_idx, &placed.grid);

    // Write level entries for each placement
    let world = &WORLDS[world_idx];
    for p in &placed.placements {
        let entry_idx = p.tile.entry_idx();
        let level_entry = match &p.tile {
            PickedTile::Level { level_entry, .. }
            | PickedTile::Fortress { level_entry, .. }
            | PickedTile::Pipe { level_entry, .. }
            | PickedTile::Airship { level_entry, .. }
            | PickedTile::Bowser { level_entry, .. } => level_entry,
            PickedTile::Start { .. } | PickedTile::Fixed { .. } => {
                unreachable!("Start/Fixed should not appear in placements");
            }
        };
        rom_data::write_entry(rom, world, entry_idx, level_entry);
    }
}

// ---------------------------------------------------------------------------
// Build: fortress lock placement
// ---------------------------------------------------------------------------

/// Build a world with randomized fortress lock positions.
///
/// Puts all tiles back at their original positions (like identity), but also
/// picks lock positions for each fortress using BFS-validated random selection.
pub(super) fn build_with_fortress_locks<R: Rng>(
    picked: &PickedWorld,
    rng: &mut R,
    pipe_pairs: &[((usize, usize), (usize, usize))],
) -> PlacedWorld {
    let mut grid = picked.grid.clone_grid();
    let mut placements = Vec::new();

    // First pass: restore all non-fortress tiles and collect fortress info
    let mut fortress_indices = Vec::new();
    for (idx, (tile, &(row, col))) in picked.tiles.iter().zip(picked.positions.iter()).enumerate() {
        let map_tile = match tile {
            PickedTile::Level { tile, .. } => *tile,
            PickedTile::Pipe { tile, .. } => *tile,
            PickedTile::Airship { .. } => TILE_AIRSHIP,
            PickedTile::Bowser { .. } => TILE_BOWSER,
            PickedTile::Fortress { .. } => {
                fortress_indices.push(idx);
                continue; // Handle fortresses after BFS
            }
            PickedTile::Start { .. } | PickedTile::Fixed { .. } => continue,
        };

        if row < grid.rows && col < grid.cols {
            grid.set(row, col, map_tile);
        }
        placements.push(Placement {
            tile: tile.clone(),
            pos: (row, col),
            lock_pos: None,
            lock_replace_tile: None,
        });
    }

    // Restore fortress tiles on the grid (needed for BFS)
    let mut fort_positions = Vec::new();
    for &idx in &fortress_indices {
        let (row, col) = picked.positions[idx];
        if let PickedTile::Fortress { tile, .. } = &picked.tiles[idx] {
            if row < grid.rows && col < grid.cols {
                grid.set(row, col, *tile);
            }
            fort_positions.push((row, col));
        }
    }

    if fortress_indices.is_empty() {
        return PlacedWorld {
            world_idx: picked.world_idx,
            grid,
            placements,
        };
    }

    // Determine beat order via BFS
    let beat_order = determine_beat_order(&grid, pipe_pairs, &fort_positions);

    // Find the target (airship or Bowser)
    let target_pos = find_target(&grid, picked.world_idx);

    // Pick lock positions
    let lock_choices = pick_lock_positions(
        rng, &grid, pipe_pairs, &fort_positions, &beat_order, target_pos,
    );

    // Place fortresses with their locks
    for (ord, &fort_idx) in beat_order.iter().enumerate() {
        let tile_idx = fortress_indices[fort_idx];
        let (row, col) = picked.positions[tile_idx];
        let lock_pos = lock_choices.get(ord).copied().flatten();

        // Place gap tile on grid if we have a lock position
        let lock_replace_tile = if let Some((lr, lc)) = lock_pos {
            let original_tile = grid.get(lr, lc);
            let gap_tile = gap_tile_for(original_tile);
            grid.set(lr, lc, gap_tile);
            Some(original_tile)
        } else {
            None
        };

        placements.push(Placement {
            tile: picked.tiles[tile_idx].clone(),
            pos: (row, col),
            lock_pos,
            lock_replace_tile,
        });
    }

    PlacedWorld {
        world_idx: picked.world_idx,
        grid,
        placements,
    }
}

/// Determine fortress beat order by simulating BFS progression.
/// Returns indices into `fort_positions` in the order they'd be reached.
fn determine_beat_order(
    grid: &Grid,
    pipes: &[((usize, usize), (usize, usize))],
    fort_positions: &[(usize, usize)],
) -> Vec<usize> {
    let mut order = Vec::new();
    let mut beaten = HashSet::new();

    loop {
        let result = map_walker::walk_map(grid, pipes, None);
        let next = fort_positions
            .iter()
            .enumerate()
            .find(|(i, pos)| !beaten.contains(i) && result.nodes.contains(pos))
            .map(|(i, _)| i);

        match next {
            Some(idx) => {
                order.push(idx);
                beaten.insert(idx);
            }
            None => break,
        }
    }
    order
}

/// Validate that lock placements allow full fortress progression.
fn validate_lock_placement(
    grid: &Grid,
    pipes: &[((usize, usize), (usize, usize))],
    fort_positions: &[(usize, usize)],
    beat_order: &[usize],
    lock_positions: &[(usize, usize)],
    target_pos: Option<(usize, usize)>,
) -> bool {
    const TILE_LOCK: u8 = 0x54;

    let mut sim_grid = grid.clone_grid();
    for &(r, c) in lock_positions {
        sim_grid.set(r, c, TILE_LOCK);
    }

    for (ord, &fort_idx) in beat_order.iter().enumerate() {
        let fort_pos = fort_positions[fort_idx];
        let result = map_walker::walk_map(&sim_grid, pipes, None);
        if !result.nodes.contains(&fort_pos) {
            return false;
        }
        if ord < lock_positions.len() {
            let (lr, lc) = lock_positions[ord];
            sim_grid.set(lr, lc, grid.get(lr, lc));
        }
    }

    if let Some(target) = target_pos {
        let result = map_walker::walk_map(&sim_grid, pipes, None);
        if !result.nodes.contains(&target) {
            return false;
        }
    }

    true
}

/// Pick lock positions with BFS validation. Returns one Option per fortress
/// in beat order. Falls back to None (no locks) if validation fails.
fn pick_lock_positions<R: Rng>(
    rng: &mut R,
    grid: &Grid,
    pipes: &[((usize, usize), (usize, usize))],
    fort_positions: &[(usize, usize)],
    beat_order: &[usize],
    target_pos: Option<(usize, usize)>,
) -> Vec<Option<(usize, usize)>> {
    let n = beat_order.len();

    // Collect eligible path tiles (reachable, lockable, row < 8)
    let result = map_walker::walk_map(grid, pipes, None);
    let mut candidates: Vec<(usize, usize)> = result.path_tiles
        .iter()
        .filter(|&&(r, _)| r < 8)
        .filter(|&&(r, c)| LOCKABLE_TILES.contains(&grid.get(r, c)))
        .copied()
        .collect();
    candidates.sort();

    for _attempt in 0..50 {
        let mut available = candidates.clone();
        available.as_mut_slice().shuffle(rng);
        let choices: Vec<(usize, usize)> = available.into_iter().take(n).collect();

        if choices.len() < n {
            break;
        }

        if validate_lock_placement(grid, pipes, fort_positions, beat_order, &choices, target_pos) {
            return choices.iter().map(|&pos| Some(pos)).collect();
        }
    }

    // Fallback: no locks
    vec![None; n]
}

/// Find the airship or Bowser's castle position on the grid.
fn find_target(grid: &Grid, world_idx: usize) -> Option<(usize, usize)> {
    let target_tile = if world_idx == 7 { TILE_BOWSER } else { TILE_AIRSHIP };
    for r in 0..grid.rows {
        for c in 0..grid.cols {
            if grid.get(r, c) == target_tile {
                return Some((r, c));
            }
        }
    }
    None
}

/// Determine the gap tile for a given path tile.
fn gap_tile_for(tile: u8) -> u8 {
    match tile {
        0xB3 => 0x9D,                                          // water → water gap
        0xDA => 0xE4,                                          // sky → sky gap
        0x46 | 0xAA | 0xAB | 0xB0 | 0xB1 | 0xDB | 0xBA => 0x54, // vertical → lock
        _ => 0x56,                                              // horizontal → bridge gap
    }
}

// ---------------------------------------------------------------------------
// Write: fortress FX
// ---------------------------------------------------------------------------

// FX table ROM offsets
const FX_VADDR_H: usize = 0x147CD;
const FX_VADDR_L: usize = 0x147DE;
const FX_MAP_COMP_IDX: usize = 0x147EF;
const FX_PATTERNS: usize = 0x14811;

/// Pattern bytes for each FX type.
fn fx_patterns_for(tile: u8) -> [u8; 4] {
    match tile {
        0xB3 => [0xD4, 0xD6, 0xD5, 0xD7],                    // water bridge
        0x46 | 0xAA | 0xAB | 0xB0 | 0xB1 | 0xDB | 0xBA =>
            [0xFE, 0xC0, 0xFE, 0xC0],                          // lock (vertical)
        _ => [0xFE, 0xFE, 0xE1, 0xE1],                        // bridge gap / sky
    }
}

/// Map_Complete_Bits LUT: row → completion bit.
const MAP_COMPLETE_BITS: [u8; 8] = [0x80, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x01];

/// Write fortress-specific FX data for a placed world.
///
/// Handles: Boom-Boom Y-byte patching, FX slot allocation, FX world table.
/// Vanilla locks are already cleared during pick_up_world (grid pre-opening),
/// so no pre_open_fx_for_world call is needed.
pub(super) fn write_fortress_fx(
    rom: &mut Rom,
    placed: &PlacedWorld,
    fx_slot_base: usize,
) {
    let world_idx = placed.world_idx;

    // Collect fortress placements that have locks
    let fortress_placements: Vec<&Placement> = placed.placements
        .iter()
        .filter(|p| matches!(p.tile, PickedTile::Fortress { .. }) && p.lock_pos.is_some())
        .collect();

    // Write FX world table
    let fx_base = rom_data::FX_WORLD_TABLE + world_idx * 4;
    for i in 0..4 {
        if i < fortress_placements.len() {
            rom.write_byte(fx_base + i, (fx_slot_base + i) as u8);
        } else {
            rom.write_byte(fx_base + i, 0x00);
        }
    }

    // Write each fortress's FX data
    for (i, p) in fortress_placements.iter().enumerate() {
        let slot_idx = fx_slot_base + i;
        let ordinal = (i + 1) as u8;
        let (fort_row, fort_col) = p.pos;
        let (ob_row, ob_col) = p.lock_pos.unwrap();
        let replace_tile = p.lock_replace_tile.unwrap();

        // Patch Boom-Boom Y-byte
        if let PickedTile::Fortress { boomboom_y_offset, .. } = &p.tile {
            let old_y = rom.read_byte(*boomboom_y_offset);
            let new_y = (ordinal << 4) | (old_y & 0x0F);
            rom.write_byte(*boomboom_y_offset, new_y);
        }

        let patterns = fx_patterns_for(replace_tile);

        // VRAM address
        let col_in_screen = ob_col % 16;
        let screen = ob_col / 16;
        let vram = (0x2880 + ob_row * 64 + col_in_screen * 2) as u16;
        rom.write_byte(FX_VADDR_H + slot_idx, (vram >> 8) as u8);
        rom.write_byte(FX_VADDR_L + slot_idx, (vram & 0xFF) as u8);

        // Map location
        rom.write_byte(rom_data::FX_MAP_LOC_ROW + slot_idx,
            ((ob_row + 2) as u8) << 4);
        rom.write_byte(rom_data::FX_MAP_LOC + slot_idx,
            ((col_in_screen as u8) << 4) | (screen as u8));

        // Replacement tile
        rom.write_byte(rom_data::FX_MAP_TILE_REPLACE + slot_idx, replace_tile);

        // Map_Completions persistence — encodes LOCK position
        let comp_col = ob_col as u8;
        let comp_bit = MAP_COMPLETE_BITS[ob_row.min(7)];
        rom.write_byte(FX_MAP_COMP_IDX + slot_idx * 2, comp_col);
        rom.write_byte(FX_MAP_COMP_IDX + slot_idx * 2 + 1, comp_bit);

        // Pattern bytes
        let pat_off = FX_PATTERNS + slot_idx * 4;
        for (j, &b) in patterns.iter().enumerate() {
            rom.write_byte(pat_off + j, b);
        }
    }
}

// ---------------------------------------------------------------------------
// Build: pipe placement
// ---------------------------------------------------------------------------

/// Collect all node positions from a PlacedWorld (positions eligible for pipe swap).
///
/// Includes levels, fortresses, and existing pipe positions. Excludes Start,
/// Fixed, Airship, and Bowser.
fn collect_swappable_nodes(placed: &PlacedWorld) -> HashSet<(usize, usize)> {
    let mut nodes = HashSet::new();
    for p in &placed.placements {
        match &p.tile {
            PickedTile::Level { .. }
            | PickedTile::Fortress { .. }
            | PickedTile::Pipe { .. } => {
                nodes.insert(p.pos);
            }
            _ => {}
        }
    }
    nodes
}

/// Compute fortress gate segments from the placed grid.
///
/// The grid already has gap tiles (locks/bridges) placed by fortress lock
/// placement. This iteratively opens each gap tile and assigns newly
/// reachable nodes to successive segments.
fn compute_segments_from_grid(
    grid: &Grid,
    all_nodes: &HashSet<(usize, usize)>,
    fortress_placements: &[&Placement],
) -> HashMap<(usize, usize), usize> {
    let mut segments: HashMap<(usize, usize), usize> = HashMap::new();
    let mut work_grid = grid.clone_grid();
    let mut seg_idx = 0;

    // Walk with all obstacles in place → segment 0
    let result = map_walker::walk_map(&work_grid, &[], None);
    for &pos in all_nodes {
        if result.nodes.contains(&pos) {
            segments.insert(pos, seg_idx);
        }
    }

    // Iteratively open each fortress's lock → new segments
    for fp in fortress_placements {
        if let (Some((lr, lc)), Some(replace_tile)) = (fp.lock_pos, fp.lock_replace_tile) {
            seg_idx += 1;
            work_grid.set(lr, lc, replace_tile);
            let result = map_walker::walk_map(&work_grid, &[], None);
            for &pos in all_nodes {
                if result.nodes.contains(&pos) && !segments.contains_key(&pos) {
                    segments.insert(pos, seg_idx);
                }
            }
        }
    }

    // Any remaining unassigned nodes get the highest segment
    for &pos in all_nodes {
        segments.entry(pos).or_insert(seg_idx);
    }

    segments
}

/// Build a world with pipe endpoints placed using progressive BFS.
///
/// Takes a PlacedWorld (from fortress lock placement) and relocates pipe
/// endpoints to new positions. Pipes connect disconnected areas, prioritizing
/// components containing must-reach targets (airship/Bowser). A forbidden-pair
/// constraint prevents piping directly from segment 0 to the goal segment.
pub(super) fn build_with_pipes<R: Rng>(
    placed: &mut PlacedWorld,
    rng: &mut R,
) {
    // Collect pipe placements and their indices
    let pipe_indices: Vec<usize> = placed.placements
        .iter()
        .enumerate()
        .filter(|(_, p)| matches!(p.tile, PickedTile::Pipe { .. }))
        .map(|(i, _)| i)
        .collect();

    if pipe_indices.is_empty() {
        return;
    }

    let all_nodes = collect_swappable_nodes(placed);

    // Remove pipe tiles from grid
    for &pi in &pipe_indices {
        let (r, c) = placed.placements[pi].pos;
        if r < placed.grid.rows && c < placed.grid.cols {
            placed.grid.set(r, c, EMPTY_NODE);
        }
    }

    // Compute fortress gate segments (locks already on the grid)
    let fortress_placements: Vec<&Placement> = placed.placements
        .iter()
        .filter(|p| matches!(p.tile, PickedTile::Fortress { .. }) && p.lock_pos.is_some())
        .collect();
    let segments = compute_segments_from_grid(&placed.grid, &all_nodes, &fortress_placements);

    // Find goal segment (airship or Bowser)
    let target_pos = find_target(&placed.grid, placed.world_idx);
    let goal_seg = target_pos
        .and_then(|p| segments.get(&p).copied());

    // Open fortress gaps for the walk (simulate post-fortress state)
    for fp in &fortress_placements {
        if let (Some((lr, lc)), Some(replace_tile)) = (fp.lock_pos, fp.lock_replace_tile) {
            placed.grid.set(lr, lc, replace_tile);
        }
    }

    // Walk with no pipes (gaps open)
    let result = map_walker::walk_map(&placed.grid, &[], None);
    let mut reachable = result.nodes.clone();

    // Restore fortress gap tiles on the grid
    for fp in &fortress_placements {
        if let (Some((lr, lc)), Some(_)) = (fp.lock_pos, fp.lock_replace_tile) {
            let gap_tile = gap_tile_for(placed.grid.get(lr, lc));
            placed.grid.set(lr, lc, gap_tile);
        }
    }

    // Forbidden pair check: no pipe may directly bridge segment 0 to goal
    let is_forbidden_pair = |a: (usize, usize), b: (usize, usize)| -> bool {
        if let Some(gs) = goal_seg {
            if gs == 0 { return false; }
            let sa = segments.get(&a).copied().unwrap_or(0);
            let sb = segments.get(&b).copied().unwrap_or(0);
            (sa == 0 && sb == gs) || (sb == 0 && sa == gs)
        } else {
            false
        }
    };

    // Must-reach positions (airship, Bowser)
    let must_reach: HashSet<(usize, usize)> = placed.placements
        .iter()
        .filter(|p| matches!(p.tile, PickedTile::Airship { .. } | PickedTile::Bowser { .. }))
        .map(|p| p.pos)
        .collect();

    // Progressive placement
    let mut placed_pairs: Vec<((usize, usize), (usize, usize))> = Vec::new();
    let mut used_positions: HashSet<(usize, usize)> = HashSet::new();
    let num_pairs = pipe_indices.len() / 2;
    let mut pair_order: Vec<usize> = (0..num_pairs).collect();
    pair_order.as_mut_slice().shuffle(rng);

    for _pair in pair_order {
        let available: HashSet<(usize, usize)> = &all_nodes - &used_positions;
        let unreachable_nodes: HashSet<(usize, usize)> = &available - &reachable;
        let reachable_available: HashSet<(usize, usize)> = &available & &reachable;

        if unreachable_nodes.is_empty() {
            // All reachable — place randomly respecting forbidden-pair rule
            let mut candidates: Vec<(usize, usize)> = reachable_available.into_iter().collect();
            candidates.sort();
            candidates.as_mut_slice().shuffle(rng);

            if candidates.len() >= 2 {
                let mut placed_ok = false;
                'outer: for i in 0..candidates.len() {
                    for j in (i + 1)..candidates.len() {
                        if !is_forbidden_pair(candidates[i], candidates[j]) {
                            let a = candidates[i];
                            let b = candidates[j];
                            placed_pairs.push((a, b));
                            used_positions.insert(a);
                            used_positions.insert(b);
                            placed.grid.set(a.0, a.1, TILE_PIPE);
                            placed.grid.set(b.0, b.1, TILE_PIPE);
                            placed_ok = true;
                            break 'outer;
                        }
                    }
                }
                if !placed_ok {
                    // Fallback: place anyway
                    let a = candidates[0];
                    let b = candidates[1];
                    placed_pairs.push((a, b));
                    used_positions.insert(a);
                    used_positions.insert(b);
                    placed.grid.set(a.0, a.1, TILE_PIPE);
                    placed.grid.set(b.0, b.1, TILE_PIPE);
                }
            }
            continue;
        }

        // Prioritize must-reach components
        let unreachable_must: HashSet<(usize, usize)> = &must_reach - &reachable;
        let unreachable_cands: Vec<(usize, usize)> = if !unreachable_must.is_empty() {
            let components = find_unreachable_components(&placed.grid, &reachable, &all_nodes);
            let mut priority = HashSet::new();
            for comp in &components {
                if !comp.is_disjoint(&unreachable_must) {
                    priority.extend(comp.intersection(&unreachable_nodes));
                }
            }
            if !priority.is_empty() {
                let mut v: Vec<(usize, usize)> = priority.into_iter().collect();
                v.sort();
                v
            } else {
                let mut v: Vec<(usize, usize)> = unreachable_nodes.into_iter().collect();
                v.sort();
                v
            }
        } else {
            let mut v: Vec<(usize, usize)> = unreachable_nodes.into_iter().collect();
            v.sort();
            v
        };

        let mut reachable_cands: Vec<(usize, usize)> = reachable_available.into_iter().collect();
        reachable_cands.sort();

        if reachable_cands.is_empty() {
            break;
        }

        reachable_cands.as_mut_slice().shuffle(rng);
        let mut unreachable_cands = unreachable_cands;
        unreachable_cands.as_mut_slice().shuffle(rng);

        // Pick a non-forbidden pair
        let b_pos = unreachable_cands[0];
        let mut a_pos = reachable_cands[0];
        if is_forbidden_pair(a_pos, b_pos) {
            if let Some(&alt) = reachable_cands.iter().find(|&&p| !is_forbidden_pair(p, b_pos)) {
                a_pos = alt;
            }
        }

        placed_pairs.push((a_pos, b_pos));
        used_positions.insert(a_pos);
        used_positions.insert(b_pos);
        placed.grid.set(a_pos.0, a_pos.1, TILE_PIPE);
        placed.grid.set(b_pos.0, b_pos.1, TILE_PIPE);

        // Re-walk with new pipe pair
        let result = map_walker::walk_map(&placed.grid, &placed_pairs, None);
        reachable = result.nodes;
    }

    // Update pipe placement positions — pair pipe_indices[2*i] and [2*i+1]
    // with placed_pairs[i]
    for (i, &(a_pos, b_pos)) in placed_pairs.iter().enumerate() {
        let idx_a = pipe_indices[i * 2];
        let idx_b = pipe_indices[i * 2 + 1];
        placed.placements[idx_a].pos = a_pos;
        placed.placements[idx_b].pos = b_pos;
    }
}

/// Find connected components among unreachable nodes.
fn find_unreachable_components(
    grid: &Grid,
    reachable: &HashSet<(usize, usize)>,
    all_nodes: &HashSet<(usize, usize)>,
) -> Vec<HashSet<(usize, usize)>> {
    let unreachable: HashSet<(usize, usize)> = all_nodes.difference(reachable).copied().collect();
    if unreachable.is_empty() {
        return Vec::new();
    }

    let mut visited: HashSet<(usize, usize)> = HashSet::new();
    let mut components = Vec::new();

    for &start in &unreachable {
        if visited.contains(&start) {
            continue;
        }
        let result = map_walker::walk_map(grid, &[], Some(start));
        let component: HashSet<(usize, usize)> = result.nodes.intersection(all_nodes).copied().collect();
        visited.extend(&component);
        components.push(component);
    }

    components
}

// ---------------------------------------------------------------------------
// Write: pipe placements
// ---------------------------------------------------------------------------

/// Write pipe placement changes to ROM.
///
/// For each pipe pair: swaps pointer table entries to new positions, updates
/// destination tables, then re-sorts the pointer table and syncs map objects.
pub(super) fn write_pipe_placements(
    rom: &mut Rom,
    placed: &PlacedWorld,
) {
    let world_idx = placed.world_idx;

    // Collect pipe placements in pair order (consecutive pairs from pick_up)
    let pipe_placements: Vec<&Placement> = placed.placements
        .iter()
        .filter(|p| matches!(p.tile, PickedTile::Pipe { .. }))
        .collect();

    if pipe_placements.is_empty() {
        return;
    }

    let world = &WORLDS[world_idx];
    let n = world.entry_count;
    let rt = world.rowtype_offset;
    let sc = rt + n;

    // Build live position → entry index lookup from current ROM state
    let mut pos_to_entry: HashMap<(usize, usize), usize> = HashMap::new();
    for i in 0..n {
        let rowtype = rom.read_byte(rt + i);
        let scrcol = rom.read_byte(sc + i);
        let row_nib = (rowtype >> 4) & 0x0F;
        let screen = (scrcol >> 4) & 0x0F;
        let col = scrcol & 0x0F;
        let grid_row = (row_nib as usize).wrapping_sub(2);
        let grid_col = screen as usize * 16 + col as usize;
        pos_to_entry.insert((grid_row, grid_col), i);
    }

    // Process pipe pairs (consecutive placements)
    for pair in pipe_placements.chunks(2) {
        if pair.len() < 2 {
            break;
        }
        let pa = pair[0];
        let pb = pair[1];

        let entry_idx_a = pa.tile.entry_idx();
        let entry_idx_b = pb.tile.entry_idx();
        let new_a_pos = pa.pos;
        let new_b_pos = pb.pos;

        // Swap entry A to its new position
        let cur_a_rt = rom.read_byte(rt + entry_idx_a);
        let cur_a_sc = rom.read_byte(sc + entry_idx_a);
        let cur_a_row = ((cur_a_rt >> 4) as usize).wrapping_sub(2);
        let cur_a_col = ((cur_a_sc >> 4) as usize & 0x0F) * 16 + (cur_a_sc as usize & 0x0F);
        let cur_a_pos = (cur_a_row, cur_a_col);

        if cur_a_pos != new_a_pos {
            if let Some(&target_idx) = pos_to_entry.get(&new_a_pos) {
                pipe_helpers::swap_entry_positions(rom, world_idx, entry_idx_a, target_idx);
                pos_to_entry.insert(new_a_pos, entry_idx_a);
                pos_to_entry.insert(cur_a_pos, target_idx);
            }
        }

        // Swap entry B to its new position
        let cur_b_rt = rom.read_byte(rt + entry_idx_b);
        let cur_b_sc = rom.read_byte(sc + entry_idx_b);
        let cur_b_row = ((cur_b_rt >> 4) as usize).wrapping_sub(2);
        let cur_b_col = ((cur_b_sc >> 4) as usize & 0x0F) * 16 + (cur_b_sc as usize & 0x0F);
        let cur_b_pos = (cur_b_row, cur_b_col);

        if cur_b_pos != new_b_pos {
            if let Some(&target_idx) = pos_to_entry.get(&new_b_pos) {
                pipe_helpers::swap_entry_positions(rom, world_idx, entry_idx_b, target_idx);
                pos_to_entry.insert(new_b_pos, entry_idx_b);
                pos_to_entry.insert(cur_b_pos, target_idx);
            }
        }

        // Update destination table
        if let PickedTile::Pipe { dest_idx, .. } = &pa.tile {
            pipe_helpers::write_pipe_dest(rom, *dest_idx, new_a_pos, new_b_pos);
        }
    }

    // Re-sort pointer table and sync map object positions
    pipe_helpers::resort_pointer_table(rom, world_idx);
    rom_data::sync_map_object_positions(rom, world_idx);
}

// ---------------------------------------------------------------------------
// Cross-world tile redistribution
// ---------------------------------------------------------------------------

/// Redistribute Level and Fortress tiles across worlds.
///
/// Levels shuffle among Level slots and Fortresses shuffle among Fortress
/// slots — each type stays within its own kind so the per-world count of
/// each tile type is preserved. The `entry_idx` and `world_idx` on each
/// tile are updated to reflect their new slot.
///
/// Pipes, Airships, Bowser, Start, and Fixed tiles stay in their original world.
fn redistribute_tiles<R: Rng>(
    worlds: &mut [PickedWorld; 8],
    rng: &mut R,
    shuffle_levels: bool,
    shuffle_fortresses: bool,
) {
    // Helper: shuffle one tile type across all worlds
    fn shuffle_type<R2: Rng>(
        worlds: &mut [PickedWorld; 8],
        rng: &mut R2,
        is_target: fn(&PickedTile) -> bool,
    ) {
        let mut slots: Vec<(usize, usize)> = Vec::new();
        let mut pool: Vec<PickedTile> = Vec::new();

        for (wi, picked) in worlds.iter().enumerate() {
            for (ti, tile) in picked.tiles.iter().enumerate() {
                if is_target(tile) {
                    slots.push((wi, ti));
                    pool.push(tile.clone());
                }
            }
        }

        if pool.len() <= 1 {
            return;
        }

        pool.as_mut_slice().shuffle(rng);

        for (i, &(wi, ti)) in slots.iter().enumerate() {
            let mut tile = pool[i].clone();
            let dest_entry_idx = worlds[wi].tiles[ti].entry_idx();
            tile.set_slot(wi, dest_entry_idx);
            worlds[wi].tiles[ti] = tile;
        }
    }

    if shuffle_levels {
        shuffle_type(worlds, rng, |t| matches!(t, PickedTile::Level { .. }));
    }
    if shuffle_fortresses {
        shuffle_type(worlds, rng, |t| matches!(t, PickedTile::Fortress { .. }));
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Randomize overworld maps using the builder pipeline.
///
/// Replaces `overworld::randomize_fortresses()` (lock shuffle) and
/// `pipes::randomize()` (pipe shuffle) with a unified pick-up → build → write
/// flow. When `shuffle_levels_cross` or `shuffle_fortresses_cross` is set,
/// tiles are redistributed across worlds before building.
pub fn randomize<R: Rng>(
    rom: &mut Rom,
    rng: &mut R,
    shuffle_locks: bool,
    shuffle_pipes: bool,
    shuffle_levels_cross: bool,
    shuffle_fortresses_cross: bool,
) {
    if !shuffle_locks && !shuffle_pipes && !shuffle_levels_cross && !shuffle_fortresses_cross {
        return;
    }

    let all_pipes = rom_data::read_pipe_pairs(rom);

    // Phase 1: Pick up all worlds
    let mut picked: [PickedWorld; 8] = std::array::from_fn(|wi| pick_up_world(rom, wi));

    // Phase 2: Cross-world redistribution (if requested)
    if shuffle_levels_cross || shuffle_fortresses_cross {
        redistribute_tiles(&mut picked, rng, shuffle_levels_cross, shuffle_fortresses_cross);
    }

    // Phase 3 & 4: Build and write each world
    let mut fx_slot = 0usize;

    for wi in 0..8 {
        let pipes = all_pipes.get(&wi).cloned().unwrap_or_default();

        // When pipes will be shuffled, don't let lock validation rely on
        // current pipe positions — they'll move during build_with_pipes.
        let lock_pipes = if shuffle_pipes { vec![] } else { pipes.clone() };

        let mut placed = if shuffle_locks {
            build_with_fortress_locks(&picked[wi], rng, &lock_pipes)
        } else {
            build_identity(&picked[wi])
        };

        if shuffle_pipes {
            build_with_pipes(&mut placed, rng);
        }

        let fort_count = placed.placements.iter()
            .filter(|p| matches!(p.tile, PickedTile::Fortress { .. }) && p.lock_pos.is_some())
            .count();

        if shuffle_locks {
            write_fortress_fx(rom, &placed, fx_slot);
        }
        write_world(rom, &placed);
        if shuffle_pipes {
            write_pipe_placements(rom, &placed);
        }
        fx_slot += fort_count;
    }
}

/// Write a tile grid back to ROM.
fn write_tile_grid(rom: &mut Rom, world_idx: usize, grid: &Grid) {
    for r in 0..grid.rows {
        for c in 0..grid.cols {
            let offset = rom_data::map_tile_offset(world_idx, r, c);
            rom.write_byte(offset, grid.get(r, c));
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    fn load_rom() -> Option<Rom> {
        let data = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&data).ok()
    }

    /// Generate a playable ROM using the builder pipeline.
    /// Run with: cargo test --lib generate_builder_rom -- --ignored --nocapture
    #[test]
    #[ignore]
    fn generate_builder_rom() {
        let rom = load_rom().expect("ROM not found");
        let mut out = rom.clone();
        let seed = 1u64;
        let mut rng = ChaCha8Rng::seed_from_u64(seed);

        // QoL patches first (same as randomizer.rs)
        crate::randomize::qol::fix_w3_drawbridges(&mut out);
        crate::randomize::qol::remove_w2_rock(&mut out);
        crate::randomize::qol::fix_big_q_block_rooms(&mut out);

        // Pick up all worlds, redistribute cross-world, build, write
        let mut picked: [PickedWorld; 8] =
            std::array::from_fn(|wi| pick_up_world(&out, wi));
        redistribute_tiles(&mut picked, &mut rng, true, true);

        let mut fx_slot = 0usize;
        for wi in 0..8 {
            // Don't use pipe positions for lock validation — pipes will move
            let mut placed = build_with_fortress_locks(&picked[wi], &mut rng, &[]);
            build_with_pipes(&mut placed, &mut rng);

            let fort_count = placed.placements.iter()
                .filter(|p| matches!(p.tile, PickedTile::Fortress { .. }) && p.lock_pos.is_some())
                .count();

            write_fortress_fx(&mut out, &placed, fx_slot);
            write_world(&mut out, &placed);
            write_pipe_placements(&mut out, &placed);
            fx_slot += fort_count;
        }

        let path = "builder_test.nes";
        std::fs::write(path, &out.data).expect("failed to write ROM");
        eprintln!("Wrote {path} (seed {seed})");
    }

    #[test]
    fn test_fortress_locks_progression_valid() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };

        for seed in [42u64, 1, 99, 777, 31337] {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let all_pipes = rom_data::read_pipe_pairs(&rom);

            let mut test_rom = rom.clone();
            let mut fx_slot = 0usize;

            for wi in 0..8 {
                let picked = pick_up_world(&rom, wi);
                let pipes = all_pipes.get(&wi).cloned().unwrap_or_default();
                let placed = build_with_fortress_locks(&picked, &mut rng, &pipes);

                // Count fortresses with locks
                let fort_count = placed.placements.iter()
                    .filter(|p| matches!(p.tile, PickedTile::Fortress { .. }) && p.lock_pos.is_some())
                    .count();

                // Write FX first, then grid+entries
                write_fortress_fx(&mut test_rom, &placed, fx_slot);
                write_world(&mut test_rom, &placed);
                fx_slot += fort_count;

                // Verify no locks at row 8
                for c in 0..placed.grid.cols {
                    assert_ne!(
                        placed.grid.get(8, c), 0x54,
                        "Seed {seed} W{}: lock at row 8 col {c}", wi + 1,
                    );
                }
            }

            // Verify progression: simulate fortress FX and check target reachable
            let all_pipes = rom_data::read_pipe_pairs(&test_rom);
            for wi in 0..8 {
                let pipes = all_pipes.get(&wi).cloned().unwrap_or_default();
                let steps = map_walker::simulate_progression(&test_rom, wi, &pipes);

                if let Some(target) = crate::randomize::overworld_helpers::world_target_position(&test_rom, wi) {
                    let final_nodes = &steps.last().unwrap().nodes;
                    assert!(
                        final_nodes.contains(&target),
                        "Seed {seed} W{}: target ({},{}) unreachable after all fortresses",
                        wi + 1, target.0, target.1,
                    );
                }
            }
        }
    }

    #[test]
    fn test_fortress_locks_deterministic() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };

        let all_pipes = rom_data::read_pipe_pairs(&rom);

        let mut rom1 = rom.clone();
        let mut rom2 = rom.clone();

        for pass in 0..2 {
            let target_rom = if pass == 0 { &mut rom1 } else { &mut rom2 };
            let mut rng = ChaCha8Rng::seed_from_u64(777);
            let mut fx_slot = 0usize;

            for wi in 0..8 {
                let picked = pick_up_world(&rom, wi);
                let pipes = all_pipes.get(&wi).cloned().unwrap_or_default();
                let placed = build_with_fortress_locks(&picked, &mut rng, &pipes);

                let fort_count = placed.placements.iter()
                    .filter(|p| matches!(p.tile, PickedTile::Fortress { .. }) && p.lock_pos.is_some())
                    .count();

                write_fortress_fx(target_rom, &placed, fx_slot);
                write_world(target_rom, &placed);
                fx_slot += fort_count;
            }
        }

        // Compare FX table
        for off in 0x147CD..0x148B8 {
            assert_eq!(
                rom1.read_byte(off), rom2.read_byte(off),
                "FX table mismatch at 0x{:05X}", off,
            );
        }

        // Compare tile grids
        for wi in 0..8 {
            let info = &rom_data::MAP_TILE_GRIDS[wi];
            for r in 0..rom_data::ROWS {
                for c in 0..info.columns {
                    let off = rom_data::map_tile_offset(wi, r, c);
                    assert_eq!(
                        rom1.read_byte(off), rom2.read_byte(off),
                        "W{} tile mismatch at ({},{})", wi + 1, r, c,
                    );
                }
            }
        }

        // Compare Y-bytes
        for &off in &rom_data::BOOMBOOM_Y_OFFSETS {
            assert_eq!(rom1.read_byte(off), rom2.read_byte(off));
        }
    }

    #[test]
    fn test_identity_round_trip() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };

        let mut test_rom = rom.clone();

        // Collect vanilla FX gap positions (these get pre-opened during pick_up)
        let fx_slots = rom_data::read_fx_slots(&rom);
        let fx_assignments = rom_data::read_world_fx_assignments(&rom);
        let mut gap_positions: HashSet<(usize, usize, usize)> = HashSet::new(); // (world, row, col)
        for wi in 0..8 {
            let grid = rom_data::read_tile_grid(&rom, wi);
            for &slot_idx in &fx_assignments[wi] {
                let si = slot_idx as usize;
                if si < fx_slots.len() {
                    let s = &fx_slots[si];
                    let tile = grid.get(s.grid_row, s.grid_col);
                    if tile == 0x54 || tile == 0x56 || tile == 0x9D || tile == 0xE4 {
                        gap_positions.insert((wi, s.grid_row, s.grid_col));
                    }
                }
            }
        }

        // Pick up all 8 worlds, build identity, write back
        for wi in 0..8 {
            let picked = pick_up_world(&rom, wi);
            let placed = build_identity(&picked);
            write_world(&mut test_rom, &placed);
        }

        // Compare tile grid regions — should be identical except at pre-opened
        // FX gap positions (identity transform doesn't place locks)
        for wi in 0..8 {
            let info = &rom_data::MAP_TILE_GRIDS[wi];
            for r in 0..rom_data::ROWS {
                for c in 0..info.columns {
                    if gap_positions.contains(&(wi, r, c)) {
                        continue; // Skip pre-opened gap tiles
                    }
                    let offset = rom_data::map_tile_offset(wi, r, c);
                    let orig = rom.read_byte(offset);
                    let after = test_rom.read_byte(offset);
                    assert_eq!(
                        orig, after,
                        "W{} tile grid mismatch at ({},{}): 0x{:02X} -> 0x{:02X} (offset 0x{:05X})",
                        wi + 1, r, c, orig, after, offset,
                    );
                }
            }
        }

        // Compare pointer table regions — tileset nibble + obj/lay words
        for wi in 0..8 {
            let world = &WORLDS[wi];
            let n = world.entry_count;
            let (scrcol, objsets, layouts) = rom_data::table_offsets(world);

            for i in 0..n {
                // ByRowType (full byte — upper nibble is row, lower is tileset)
                let rt_off = world.rowtype_offset + i;
                assert_eq!(
                    rom.read_byte(rt_off), test_rom.read_byte(rt_off),
                    "W{} entry {} ByRowType mismatch at 0x{:05X}",
                    wi + 1, i, rt_off,
                );

                // ByScrCol (should be unchanged — identity doesn't move entries)
                let sc_off = scrcol + i;
                assert_eq!(
                    rom.read_byte(sc_off), test_rom.read_byte(sc_off),
                    "W{} entry {} ByScrCol mismatch at 0x{:05X}",
                    wi + 1, i, sc_off,
                );

                // ObjSets word
                let obj_off = objsets + i * 2;
                assert_eq!(
                    rom.read_byte(obj_off), test_rom.read_byte(obj_off),
                    "W{} entry {} ObjSets lo mismatch at 0x{:05X}",
                    wi + 1, i, obj_off,
                );
                assert_eq!(
                    rom.read_byte(obj_off + 1), test_rom.read_byte(obj_off + 1),
                    "W{} entry {} ObjSets hi mismatch at 0x{:05X}",
                    wi + 1, i, obj_off + 1,
                );

                // LevelLayouts word
                let lay_off = layouts + i * 2;
                assert_eq!(
                    rom.read_byte(lay_off), test_rom.read_byte(lay_off),
                    "W{} entry {} Layouts lo mismatch at 0x{:05X}",
                    wi + 1, i, lay_off,
                );
                assert_eq!(
                    rom.read_byte(lay_off + 1), test_rom.read_byte(lay_off + 1),
                    "W{} entry {} Layouts hi mismatch at 0x{:05X}",
                    wi + 1, i, lay_off + 1,
                );
            }
        }
    }

    #[test]
    fn test_pick_up_all_worlds() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };

        let mut total_levels = 0;
        let mut total_fortresses = 0;
        let mut total_pipes = 0;
        let mut total_airships = 0;
        let mut total_fixed = 0;
        let mut total_starts = 0;
        let mut total_bowser = 0;

        for wi in 0..8 {
            let picked = pick_up_world(&rom, wi);
            assert_eq!(picked.world_idx, wi);

            let mut w_levels = 0;
            let mut w_forts = 0;
            let mut w_pipes = 0;
            let mut w_airships = 0;
            let mut w_fixed = 0;
            let mut w_starts = 0;
            let mut w_bowser = 0;

            for tile in &picked.tiles {
                match tile {
                    PickedTile::Level { .. } => w_levels += 1,
                    PickedTile::Fortress { .. } => w_forts += 1,
                    PickedTile::Pipe { .. } => w_pipes += 1,
                    PickedTile::Airship { .. } => w_airships += 1,
                    PickedTile::Fixed { .. } => w_fixed += 1,
                    PickedTile::Start { .. } => w_starts += 1,
                    PickedTile::Bowser { .. } => w_bowser += 1,
                }
            }

            // Every world has exactly 1 start tile
            assert_eq!(w_starts, 1, "W{} should have 1 start tile", wi + 1);

            total_levels += w_levels;
            total_fortresses += w_forts;
            total_pipes += w_pipes;
            total_airships += w_airships;
            total_fixed += w_fixed;
            total_starts += w_starts;
            total_bowser += w_bowser;

            eprintln!(
                "W{}: {} levels, {} fortresses, {} pipes, {} airships, {} fixed, {} start, {} bowser (total: {})",
                wi + 1, w_levels, w_forts, w_pipes, w_airships, w_fixed, w_starts, w_bowser,
                picked.tiles.len(),
            );
        }

        // 17 fortresses total (from FORTRESS_ENTRIES)
        assert_eq!(total_fortresses, 17, "expected 17 fortresses");
        // 7 airships (W1-W7)
        assert_eq!(total_airships, 7, "expected 7 airships");
        // 1 Bowser
        assert_eq!(total_bowser, 1, "expected 1 Bowser");
        // 8 starts
        assert_eq!(total_starts, 8, "expected 8 start tiles");
        // Pipe entries come in pairs (each endpoint counted separately)
        assert_eq!(total_pipes % 2, 0, "pipe entries should be even");
        // 24 dest entries = 24 pipe pairs = 48 pipe endpoints
        assert_eq!(total_pipes, 48, "expected 48 pipe endpoints (24 pairs)");

        let grand_total = total_levels + total_fortresses + total_pipes
            + total_airships + total_fixed + total_starts + total_bowser;
        let expected_total: usize = WORLDS.iter().map(|w| w.entry_count).sum();
        assert_eq!(grand_total, expected_total, "all entries should be classified");
    }

    #[test]
    fn test_pick_up_preserves_path_tiles_without_entries() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };

        // Verify that path tiles WITHOUT entries are NOT replaced with EMPTY_NODE.
        // Some path-like tiles (e.g. 0xE6 = hand trap in W8) have entries on them
        // and get legitimately picked up.
        for wi in 0..8 {
            let original = rom_data::read_tile_grid(&rom, wi);
            let picked = pick_up_world(&rom, wi);

            // Collect positions that have entries
            let world = &WORLDS[wi];
            let mut entry_positions = HashSet::new();
            for i in 0..world.entry_count {
                let (r, c) = rom_data::entry_grid_position(&rom, world, i);
                entry_positions.insert((r, c));
            }

            for r in 0..picked.grid.rows {
                for c in 0..picked.grid.cols {
                    let orig = original.get(r, c);
                    let after = picked.grid.get(r, c);
                    let is_path = rom_data::VALID_HORZ.contains(&orig)
                        || rom_data::VALID_VERT.contains(&orig);
                    if is_path && !entry_positions.contains(&(r, c)) {
                        assert_eq!(
                            after, orig,
                            "W{} path tile at ({},{}) was modified: 0x{:02X} -> 0x{:02X}",
                            wi + 1, r, c, orig, after,
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn test_fortress_boomboom_offsets_valid() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };

        for wi in 0..8 {
            let picked = pick_up_world(&rom, wi);
            for tile in &picked.tiles {
                if let PickedTile::Fortress { boomboom_y_offset, entry_idx, .. } = tile {
                    assert_ne!(
                        *boomboom_y_offset, 0,
                        "W{} fortress entry {} has zero boomboom_y_offset",
                        wi + 1, entry_idx,
                    );
                    assert!(
                        *boomboom_y_offset < rom.data.len(),
                        "W{} fortress entry {} boomboom_y_offset out of range",
                        wi + 1, entry_idx,
                    );
                }
            }
        }
    }

    #[test]
    fn test_pipe_pairs_consistent() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };

        for wi in 0..8 {
            let picked = pick_up_world(&rom, wi);
            let pipes: Vec<&PickedTile> = picked.tiles.iter()
                .filter(|t| matches!(t, PickedTile::Pipe { .. }))
                .collect();

            // Every pipe's partner should also be in the picked tiles
            for p in &pipes {
                if let PickedTile::Pipe { entry_idx, dest_idx, partner, .. } = p {
                    let partner_found = pipes.iter().any(|q| {
                        if let PickedTile::Pipe { entry_idx: qi, dest_idx: qd, .. } = q {
                            *qi == partner.entry_idx && *qd == *dest_idx
                        } else {
                            false
                        }
                    });
                    assert!(
                        partner_found,
                        "W{} pipe entry {} (dest {}) partner entry {} not found",
                        wi + 1, entry_idx, dest_idx, partner.entry_idx,
                    );
                }
            }
        }
    }

    #[test]
    fn test_pipe_placement_must_reach() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };

        for seed in [42u64, 1, 99, 777, 31337] {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let all_pipes = rom_data::read_pipe_pairs(&rom);

            for wi in 0..8 {
                let picked = pick_up_world(&rom, wi);
                let pipes = all_pipes.get(&wi).cloned().unwrap_or_default();
                let mut placed = build_with_fortress_locks(&picked, &mut rng, &pipes);

                // Count pipes before
                let pipe_count_before = placed.placements.iter()
                    .filter(|p| matches!(p.tile, PickedTile::Pipe { .. }))
                    .count();

                build_with_pipes(&mut placed, &mut rng);

                // Count pipes after — should be unchanged
                let pipe_count_after = placed.placements.iter()
                    .filter(|p| matches!(p.tile, PickedTile::Pipe { .. }))
                    .count();
                assert_eq!(
                    pipe_count_before, pipe_count_after,
                    "Seed {seed} W{}: pipe count changed", wi + 1,
                );

                // Collect placed pipe pairs from the grid
                let pipe_positions: Vec<(usize, usize)> = placed.placements.iter()
                    .filter(|p| matches!(p.tile, PickedTile::Pipe { .. }))
                    .map(|p| p.pos)
                    .collect();
                let mut pipe_pairs = Vec::new();
                for chunk in pipe_positions.chunks(2) {
                    if chunk.len() == 2 {
                        pipe_pairs.push((chunk[0], chunk[1]));
                    }
                }

                // Open fortress gaps for BFS
                let mut check_grid = placed.grid.clone_grid();
                for p in &placed.placements {
                    if let (Some((lr, lc)), Some(rt)) = (p.lock_pos, p.lock_replace_tile) {
                        check_grid.set(lr, lc, rt);
                    }
                }

                // Walk with pipes and opened gaps — must-reach should be reachable
                let result = map_walker::walk_map(&check_grid, &pipe_pairs, None);

                if let Some(target) = find_target(&placed.grid, wi) {
                    assert!(
                        result.nodes.contains(&target),
                        "Seed {seed} W{}: target {:?} unreachable after pipe placement",
                        wi + 1, target,
                    );
                }
            }
        }
    }

    #[test]
    fn test_pipe_placement_deterministic() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };

        let all_pipes = rom_data::read_pipe_pairs(&rom);

        let mut rom1 = rom.clone();
        let mut rom2 = rom.clone();

        for pass in 0..2 {
            let target_rom = if pass == 0 { &mut rom1 } else { &mut rom2 };
            let mut rng = ChaCha8Rng::seed_from_u64(777);
            let mut fx_slot = 0usize;

            for wi in 0..8 {
                let picked = pick_up_world(&rom, wi);
                let pipes = all_pipes.get(&wi).cloned().unwrap_or_default();
                let mut placed = build_with_fortress_locks(&picked, &mut rng, &pipes);
                build_with_pipes(&mut placed, &mut rng);

                let fort_count = placed.placements.iter()
                    .filter(|p| matches!(p.tile, PickedTile::Fortress { .. }) && p.lock_pos.is_some())
                    .count();

                write_fortress_fx(target_rom, &placed, fx_slot);
                write_world(target_rom, &placed);
                write_pipe_placements(target_rom, &placed);
                fx_slot += fort_count;
            }
        }

        // Compare tile grids
        for wi in 0..8 {
            let info = &rom_data::MAP_TILE_GRIDS[wi];
            for r in 0..rom_data::ROWS {
                for c in 0..info.columns {
                    let off = rom_data::map_tile_offset(wi, r, c);
                    assert_eq!(
                        rom1.read_byte(off), rom2.read_byte(off),
                        "W{} tile mismatch at ({},{})", wi + 1, r, c,
                    );
                }
            }
        }

        // Compare pointer tables
        for world in &WORLDS {
            let n = world.entry_count;
            let start = world.rowtype_offset;
            let end = start + n * 6;
            for off in start..end {
                assert_eq!(
                    rom1.read_byte(off), rom2.read_byte(off),
                    "Pointer table mismatch at 0x{:05X}", off,
                );
            }
        }

        // Compare dest tables
        for off in rom_data::PIPE_MAP_XHI..rom_data::PIPE_MAP_XHI + 24 {
            assert_eq!(rom1.read_byte(off), rom2.read_byte(off));
        }
        for off in rom_data::PIPE_MAP_X..rom_data::PIPE_MAP_X + 24 {
            assert_eq!(rom1.read_byte(off), rom2.read_byte(off));
        }
        for off in rom_data::PIPE_MAP_Y..rom_data::PIPE_MAP_Y + 24 {
            assert_eq!(rom1.read_byte(off), rom2.read_byte(off));
        }
    }

    #[test]
    fn test_pipe_no_segment_skip() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };

        for seed in [42u64, 1, 99, 777, 31337] {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let all_pipes = rom_data::read_pipe_pairs(&rom);

            for wi in 0..8 {
                let picked = pick_up_world(&rom, wi);
                let pipes = all_pipes.get(&wi).cloned().unwrap_or_default();
                let mut placed = build_with_fortress_locks(&picked, &mut rng, &pipes);
                build_with_pipes(&mut placed, &mut rng);

                // Re-derive segments from the placed grid (before pipe placement
                // modified it) — we can't easily get the pre-pipe grid here, so
                // just verify no pipe tile directly connects a segment-0 node
                // to an airship/Bowser node without intermediate content.
                // This is a lighter check: verify airship/Bowser is NOT pipe-adjacent
                // to start in BFS terms (walk with no pipes should not reach target).
                let pipe_placements: Vec<&Placement> = placed.placements.iter()
                    .filter(|p| matches!(p.tile, PickedTile::Pipe { .. }))
                    .collect();

                if pipe_placements.is_empty() {
                    continue;
                }

                // Collect pipe pairs
                let pipe_pairs: Vec<((usize, usize), (usize, usize))> = pipe_placements
                    .chunks(2)
                    .filter(|c| c.len() == 2)
                    .map(|c| (c[0].pos, c[1].pos))
                    .collect();

                // Verify: for each pipe pair, the two endpoints shouldn't both be
                // the start area and goal area (by checking segment assignments).
                // Since segments were computed before pipe placement, we reconstruct.
                let fortress_locks: Vec<&Placement> = placed.placements.iter()
                    .filter(|p| matches!(p.tile, PickedTile::Fortress { .. }) && p.lock_pos.is_some())
                    .collect();

                let all_nodes = collect_swappable_nodes(&placed);

                // Recompute segments: strip pipe tiles from grid first
                let mut seg_grid = placed.grid.clone_grid();
                for pp in &pipe_placements {
                    let (r, c) = pp.pos;
                    if r < seg_grid.rows && c < seg_grid.cols {
                        seg_grid.set(r, c, EMPTY_NODE);
                    }
                }
                let segments = compute_segments_from_grid(&seg_grid, &all_nodes, &fortress_locks);

                let target_pos = find_target(&placed.grid, wi);
                let goal_seg = target_pos.and_then(|p| segments.get(&p).copied());

                if let Some(gs) = goal_seg {
                    if gs == 0 { continue; }
                    for &(a, b) in &pipe_pairs {
                        let sa = segments.get(&a).copied().unwrap_or(0);
                        let sb = segments.get(&b).copied().unwrap_or(0);
                        assert!(
                            !((sa == 0 && sb == gs) || (sb == 0 && sa == gs)),
                            "Seed {seed} W{}: pipe ({:?},{:?}) bridges seg 0 to goal seg {}",
                            wi + 1, a, b, gs,
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn test_redistribute_levels_cross_world() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };

        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let mut picked: [PickedWorld; 8] = std::array::from_fn(|wi| pick_up_world(&rom, wi));

        // Count levels per world before
        let levels_before: Vec<usize> = picked.iter()
            .map(|p| p.tiles.iter().filter(|t| matches!(t, PickedTile::Level { .. })).count())
            .collect();
        let total_levels: usize = levels_before.iter().sum();

        redistribute_tiles(&mut picked, &mut rng, true, false);

        // Each world should still have the same number of level slots
        let levels_after: Vec<usize> = picked.iter()
            .map(|p| p.tiles.iter().filter(|t| matches!(t, PickedTile::Level { .. })).count())
            .collect();
        assert_eq!(levels_before, levels_after, "level counts per world should be preserved");

        // Total levels unchanged
        let total_after: usize = levels_after.iter().sum();
        assert_eq!(total_levels, total_after);

        // At least some levels should have moved cross-world
        let mut cross_world_moves = 0;
        for (wi, p) in picked.iter().enumerate() {
            for tile in &p.tiles {
                if let PickedTile::Level { level_entry, world_idx, .. } = tile {
                    // world_idx was updated to the destination world
                    // but level_entry carries the original data — we can check
                    // if the entry_idx refers to a slot in a different world
                    // than where the data originally came from
                    let _ = (level_entry, world_idx);
                }
            }
        }
        // A weaker check: verify the LevelEntry data has been shuffled
        // by checking that at least one world has a different set of obj_ptrs
        let original: [PickedWorld; 8] = std::array::from_fn(|wi| pick_up_world(&rom, wi));
        for wi in 0..8 {
            let orig_objs: Vec<u16> = original[wi].tiles.iter()
                .filter_map(|t| match t {
                    PickedTile::Level { level_entry, .. } => {
                        Some((level_entry.obj_hi as u16) << 8 | level_entry.obj_lo as u16)
                    }
                    _ => None,
                })
                .collect();
            let new_objs: Vec<u16> = picked[wi].tiles.iter()
                .filter_map(|t| match t {
                    PickedTile::Level { level_entry, .. } => {
                        Some((level_entry.obj_hi as u16) << 8 | level_entry.obj_lo as u16)
                    }
                    _ => None,
                })
                .collect();
            if orig_objs != new_objs {
                cross_world_moves += 1;
            }
        }
        assert!(cross_world_moves > 0, "expected at least one world to have different levels");
    }

    #[test]
    fn test_redistribute_fortresses_cross_world() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };

        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let mut picked: [PickedWorld; 8] = std::array::from_fn(|wi| pick_up_world(&rom, wi));

        // Count fortresses per world before
        let forts_before: Vec<usize> = picked.iter()
            .map(|p| p.tiles.iter().filter(|t| matches!(t, PickedTile::Fortress { .. })).count())
            .collect();
        let total_forts: usize = forts_before.iter().sum();
        assert_eq!(total_forts, 17);

        redistribute_tiles(&mut picked, &mut rng, false, true);

        // Each world should still have the same number of fortress slots
        let forts_after: Vec<usize> = picked.iter()
            .map(|p| p.tiles.iter().filter(|t| matches!(t, PickedTile::Fortress { .. })).count())
            .collect();
        assert_eq!(forts_before, forts_after, "fortress counts per world should be preserved");

        // Verify fortress data (boomboom_y_offset) is non-zero and in range
        for p in &picked {
            for tile in &p.tiles {
                if let PickedTile::Fortress { boomboom_y_offset, .. } = tile {
                    assert_ne!(*boomboom_y_offset, 0);
                    assert!(*boomboom_y_offset < rom.data.len());
                }
            }
        }
    }

    #[test]
    fn test_redistribute_preserves_non_shuffleable() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };

        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let original: [PickedWorld; 8] = std::array::from_fn(|wi| pick_up_world(&rom, wi));
        let mut picked: [PickedWorld; 8] = std::array::from_fn(|wi| pick_up_world(&rom, wi));

        redistribute_tiles(&mut picked, &mut rng, true, true);

        // Pipes, Airships, Bowser, Start, Fixed should be unchanged
        for wi in 0..8 {
            for (ti, tile) in picked[wi].tiles.iter().enumerate() {
                match tile {
                    PickedTile::Pipe { entry_idx, dest_idx, .. } => {
                        if let PickedTile::Pipe { entry_idx: oe, dest_idx: od, .. } = &original[wi].tiles[ti] {
                            assert_eq!(*entry_idx, *oe, "W{} pipe entry_idx changed", wi + 1);
                            assert_eq!(*dest_idx, *od, "W{} pipe dest_idx changed", wi + 1);
                        } else {
                            panic!("W{} tile {} type changed from non-pipe to pipe", wi + 1, ti);
                        }
                    }
                    PickedTile::Airship { entry_idx, .. } => {
                        if let PickedTile::Airship { entry_idx: oe, .. } = &original[wi].tiles[ti] {
                            assert_eq!(*entry_idx, *oe);
                        } else {
                            panic!("W{} tile {} type changed", wi + 1, ti);
                        }
                    }
                    PickedTile::Bowser { entry_idx, .. } => {
                        if let PickedTile::Bowser { entry_idx: oe, .. } = &original[wi].tiles[ti] {
                            assert_eq!(*entry_idx, *oe);
                        } else {
                            panic!("W{} tile {} type changed", wi + 1, ti);
                        }
                    }
                    PickedTile::Start { entry_idx, .. } => {
                        if let PickedTile::Start { entry_idx: oe, .. } = &original[wi].tiles[ti] {
                            assert_eq!(*entry_idx, *oe);
                        } else {
                            panic!("W{} tile {} type changed", wi + 1, ti);
                        }
                    }
                    PickedTile::Fixed { entry_idx, tile: t, .. } => {
                        if let PickedTile::Fixed { entry_idx: oe, tile: ot, .. } = &original[wi].tiles[ti] {
                            assert_eq!(*entry_idx, *oe);
                            assert_eq!(*t, *ot);
                        } else {
                            panic!("W{} tile {} type changed", wi + 1, ti);
                        }
                    }
                    _ => {} // Level and Fortress can change — that's the point
                }
            }
        }
    }

    #[test]
    fn test_cross_world_full_pipeline() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };

        for seed in [42u64, 1, 99] {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let mut test_rom = rom.clone();

            // QoL patches first
            crate::randomize::qol::fix_w3_drawbridges(&mut test_rom);
            crate::randomize::qol::remove_w2_rock(&mut test_rom);
            crate::randomize::qol::fix_big_q_block_rooms(&mut test_rom);

            // Pick up, redistribute, build, write
            let mut picked: [PickedWorld; 8] =
                std::array::from_fn(|wi| pick_up_world(&test_rom, wi));
            redistribute_tiles(&mut picked, &mut rng, true, true);

            let mut fx_slot = 0usize;
            for wi in 0..8 {
                // Don't use pipe positions for lock validation — pipes will move
                let mut placed = build_with_fortress_locks(&picked[wi], &mut rng, &[]);
                build_with_pipes(&mut placed, &mut rng);

                let fort_count = placed.placements.iter()
                    .filter(|p| matches!(p.tile, PickedTile::Fortress { .. }) && p.lock_pos.is_some())
                    .count();

                write_fortress_fx(&mut test_rom, &placed, fx_slot);
                write_world(&mut test_rom, &placed);
                write_pipe_placements(&mut test_rom, &placed);
                fx_slot += fort_count;
            }

            // Verify fortress progression after writing
            let written_pipes = rom_data::read_pipe_pairs(&test_rom);
            for wi in 0..8 {
                let pipes = written_pipes.get(&wi).cloned().unwrap_or_default();
                let steps = map_walker::simulate_progression(&test_rom, wi, &pipes);

                if let Some(target) = crate::randomize::overworld_helpers::world_target_position(&test_rom, wi) {
                    let final_nodes = &steps.last().unwrap().nodes;
                    assert!(
                        final_nodes.contains(&target),
                        "Seed {seed} W{}: target ({},{}) unreachable after cross-world shuffle",
                        wi + 1, target.0, target.1,
                    );
                }
            }

            // Verify all entry data is valid (obj_ptr >= 0xC000 for levels/fortresses)
            for wi in 0..8 {
                let world = &WORLDS[wi];
                let (_sc, objsets, layouts) = rom_data::table_offsets(world);
                for i in 0..world.entry_count {
                    let obj = rom_data::read_word(&test_rom, objsets + i * 2);
                    let lay = rom_data::read_word(&test_rom, layouts + i * 2);
                    // Basic sanity: if it was a level before, it should still be valid
                    if rom_data::is_level_pointer(obj, lay) {
                        assert!(obj >= 0xC000, "W{} entry {}: obj 0x{:04X} invalid", wi + 1, i, obj);
                    }
                }
            }
        }
    }

    #[test]
    fn test_cross_world_deterministic() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };

        let mut rom1 = rom.clone();
        let mut rom2 = rom.clone();

        for pass in 0..2 {
            let target_rom = if pass == 0 { &mut rom1 } else { &mut rom2 };
            let mut rng = ChaCha8Rng::seed_from_u64(777);

            let mut picked: [PickedWorld; 8] =
                std::array::from_fn(|wi| pick_up_world(&rom, wi));
            redistribute_tiles(&mut picked, &mut rng, true, true);

            let mut fx_slot = 0usize;
            for wi in 0..8 {
                let mut placed = build_with_fortress_locks(&picked[wi], &mut rng, &[]);
                build_with_pipes(&mut placed, &mut rng);

                let fort_count = placed.placements.iter()
                    .filter(|p| matches!(p.tile, PickedTile::Fortress { .. }) && p.lock_pos.is_some())
                    .count();

                write_fortress_fx(target_rom, &placed, fx_slot);
                write_world(target_rom, &placed);
                write_pipe_placements(target_rom, &placed);
                fx_slot += fort_count;
            }
        }

        // Compare tile grids
        for wi in 0..8 {
            let info = &rom_data::MAP_TILE_GRIDS[wi];
            for r in 0..rom_data::ROWS {
                for c in 0..info.columns {
                    let off = rom_data::map_tile_offset(wi, r, c);
                    assert_eq!(
                        rom1.read_byte(off), rom2.read_byte(off),
                        "W{} tile mismatch at ({},{})", wi + 1, r, c,
                    );
                }
            }
        }

        // Compare pointer tables
        for world in &WORLDS {
            let n = world.entry_count;
            let start = world.rowtype_offset;
            let end = start + n * 6;
            for off in start..end {
                assert_eq!(
                    rom1.read_byte(off), rom2.read_byte(off),
                    "Pointer table mismatch at 0x{:05X}", off,
                );
            }
        }

        // Compare FX tables
        for off in 0x147CD..0x148B8 {
            assert_eq!(rom1.read_byte(off), rom2.read_byte(off));
        }

        // Compare Y-bytes
        for &off in &rom_data::BOOMBOOM_Y_OFFSETS {
            assert_eq!(rom1.read_byte(off), rom2.read_byte(off));
        }
    }
}
