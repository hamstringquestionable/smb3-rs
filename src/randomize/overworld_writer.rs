/// Phase 4 of the overworld builder rewrite: Write.
///
/// Takes `BuildResult` (Phase 3) + `PickupResult` (Phase 2) + `NodeCatalog`
/// (Phase 1) + RNG, assigns concrete pool entries to slots, and writes all
/// ROM data: tile grids, pointer tables, FX tables, pipe destinations, and
/// W8 army sprite positions.

use std::collections::{HashMap, HashSet};

use rand::Rng;
use rand::seq::SliceRandom;

use crate::rom::Rom;

use super::node_catalog::{NodeCatalog, NodeKind};
use super::overworld_build::{bfs_ordered, BuildResult, BuiltWorld, SlotKind};
use super::overworld_helpers;
use super::overworld_pickup::PickupResult;
use super::pipe_helpers;
use super::rom_data::{
    self, FX_MAP_COMP_IDX, FX_PATTERNS, FX_VADDR_H, FX_VADDR_L,
    MAP_COMPLETE_BITS, TILE_FORTRESS, TILE_PIPE, WORLDS,
};



// ---------------------------------------------------------------------------
// W8 army sprite slots
// ---------------------------------------------------------------------------

/// W8 army sprite definitions: (map_object_slot, is_fortress).
/// Tank goes on a level slot, the other 3 go on fortress slots.
const W8_ARMY_SPRITES: &[(usize, bool)] = &[
    (2, false), // Tank sprite (ID $0E) → level slot
    (3, true),  // Navy/Battleship sprite (ID $0D) → fortress slot
    (4, true),  // Air Force sprite (ID $0F) → fortress slot
    (5, true),  // Super Tank sprite (ID $0E) → fortress slot
];

// ---------------------------------------------------------------------------
// Assignment types
// ---------------------------------------------------------------------------

/// A concrete assignment of a pool entry to a grid position.
#[derive(Clone, Debug)]
struct Assignment {
    /// Index into `pickup.pool`.
    pool_idx: usize,
    /// Target grid position.
    pos: (usize, usize),
}

/// Pipe pair assignment: two pool entries, a dest_idx, and two positions.
#[derive(Clone, Debug)]
struct PipeAssignment {
    pool_idx_a: usize,
    pool_idx_b: usize,
    dest_idx: usize,
    pos_a: (usize, usize),
    pos_b: (usize, usize),
}

/// Hammer bro assignment: carries its own LevelEntry from the cycling pool.
#[derive(Clone, Debug)]
struct HammerBroAssignment {
    /// Index into `pickup.pool` (provides entry_idx for the pointer table slot).
    #[allow(dead_code)] // read in tests
    pool_idx: usize,
    /// Target grid position.
    pos: (usize, usize),
    /// Level data from the cycling hammer bro level pool.
    level_entry: rom_data::LevelEntry,
}

/// All assignments for one world.
struct WorldAssignments {
    /// Fortress assignments, ordered by section (for FX ordinal computation).
    fortress: Vec<Assignment>,
    /// Level assignments.
    level: Vec<Assignment>,
    /// Pipe pair assignments.
    pipes: Vec<PipeAssignment>,
    /// Airship assignment (W1-W7 only).
    airship: Option<Assignment>,
    /// Bowser assignment (W8 only).
    bowser: Option<Assignment>,
    /// Hammer bro assignments (remaining blank slots).
    hammer_bro: Vec<HammerBroAssignment>,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Execute Phase 4: assign pool entries to slots and write all ROM data.
pub(crate) fn write_overworld<R: Rng>(
    rom: &mut Rom,
    build: &BuildResult,
    pickup: &PickupResult,
    catalog: &NodeCatalog,
    rng: &mut R,
    cross_world: bool,
) {
    let assignments = assign_pool(rom, build, pickup, catalog, rng, cross_world);

    // Compute W8 army sprite target positions before writing tiles,
    // so write_tile_grid can stamp connectivity-aware blank tiles under the sprites.
    let w8_sprite_positions = pick_w8_sprite_positions(&assignments[7], rng);
    let w8_sprite_pos_set: HashSet<(usize, usize)> =
        w8_sprite_positions.iter().map(|&(_, pos)| pos).collect();

    let mut fx_slot = 0usize;
    for wi in 0..8 {
        let built = &build.worlds[wi];
        let wa = &assignments[wi];
        let sprite_mask = if wi == 7 { &w8_sprite_pos_set } else { &HashSet::new() };

        write_tile_grid(rom, built, wa, pickup, catalog, sprite_mask);
        write_pointer_entries(rom, wi, wa, pickup, catalog);
        write_fortress_fx(rom, wi, built, wa, pickup, catalog, &mut fx_slot);
        write_pipe_dests(rom, wa);
        pipe_helpers::resort_pointer_table(rom, wi);
        rom_data::sync_map_object_positions(rom, wi);
    }

    write_w8_sprites(rom, &w8_sprite_positions);
}

// ---------------------------------------------------------------------------
// Step 1: Pool assignment
// ---------------------------------------------------------------------------

fn assign_pool<R: Rng>(
    rom: &Rom,
    build: &BuildResult,
    pickup: &PickupResult,
    catalog: &NodeCatalog,
    rng: &mut R,
    cross_world: bool,
) -> Vec<WorldAssignments> {
    // Partition pool by kind.
    let mut fort_pool: Vec<usize> = Vec::new();
    let mut level_pool: Vec<usize> = Vec::new();
    let mut airship_pool: Vec<usize> = Vec::new();
    let mut bowser_idx: Option<usize> = None;
    let mut pipe_groups: HashMap<usize, HashMap<usize, Vec<usize>>> = HashMap::new();
    let mut hb_pool_by_world: HashMap<usize, Vec<usize>> = HashMap::new();

    for (pi, pe) in pickup.pool.iter().enumerate() {
        let entry = &catalog.entries[pe.catalog_idx];
        match &entry.kind {
            NodeKind::Fortress { .. } => fort_pool.push(pi),
            NodeKind::Level => level_pool.push(pi),
            NodeKind::Airship => airship_pool.push(pi),
            NodeKind::Bowser => bowser_idx = Some(pi),
            NodeKind::Pipe { dest_idx } => {
                pipe_groups
                    .entry(entry.world_idx)
                    .or_default()
                    .entry(*dest_idx)
                    .or_default()
                    .push(pi);
            }
            NodeKind::HammerBro => {
                hb_pool_by_world
                    .entry(entry.world_idx)
                    .or_default()
                    .push(pi);
            }
            _ => {}
        }
    }

    // Build cycling hammer bro level pool (unique real levels, shuffled).
    let mut hb_levels = catalog.unique_hammer_bro_levels();
    hb_levels.as_mut_slice().shuffle(rng);
    let mut hb_level_iter = hb_levels.iter().cycle().cloned();

    fort_pool.as_mut_slice().shuffle(rng);
    level_pool.as_mut_slice().shuffle(rng);
    airship_pool.as_mut_slice().shuffle(rng);

    // For intra-world mode, partition fort/level pools by origin world.
    let mut fort_by_world: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut level_by_world: HashMap<usize, Vec<usize>> = HashMap::new();
    if !cross_world {
        for &pi in &fort_pool {
            let wi = catalog.entries[pickup.pool[pi].catalog_idx].world_idx;
            fort_by_world.entry(wi).or_default().push(pi);
        }
        for &pi in &level_pool {
            let wi = catalog.entries[pickup.pool[pi].catalog_idx].world_idx;
            level_by_world.entry(wi).or_default().push(pi);
        }
    }

    let mut fort_iter = fort_pool.into_iter();
    let mut level_iter = level_pool.into_iter();

    let mut assignments: Vec<WorldAssignments> = Vec::with_capacity(8);

    for wi in 0..8 {
        let built = &build.worlds[wi];

        // --- Fortress assignments (ordered by section for FX) ---
        let mut fortress = Vec::new();
        for section in 0..built.section_count {
            if let Some(slot) = built.slots.iter().find(|s| {
                s.kind == SlotKind::Fortress && s.section == section
            }) {
                let pi = if cross_world {
                    fort_iter.next().expect("fortress pool exhausted")
                } else {
                    fort_by_world
                        .get_mut(&wi)
                        .and_then(|v| v.pop())
                        .expect("intra-world fortress pool exhausted")
                };
                fortress.push(Assignment { pool_idx: pi, pos: slot.pos });
            }
        }

        // --- Level assignments ---
        let mut level = Vec::new();
        for slot in &built.slots {
            if slot.kind != SlotKind::Level {
                continue;
            }
            let pi = if cross_world {
                level_iter.next().expect("level pool exhausted")
            } else {
                level_by_world
                    .get_mut(&wi)
                    .and_then(|v| v.pop())
                    .expect("intra-world level pool exhausted")
            };
            level.push(Assignment { pool_idx: pi, pos: slot.pos });
        }

        // --- Pipe assignments ---
        // Each dest_idx has two pool entries: the A-side (left pipe in transit
        // level, layout byte5 bit 6 = 0) and the B-side (right pipe, bit 6 = 1).
        // The dest table upper nibble = A position, lower = B position.  The
        // game picks the nibble based on Mario's exit side in the transit level,
        // so pool_idx_a/pos_a must be the A-side entry or the pipe self-references.
        let mut pipes = Vec::new();
        if let Some(world_pipes) = pipe_groups.get_mut(&wi) {
            let mut groups: Vec<(usize, Vec<usize>)> = world_pipes.drain().collect();
            groups.sort_by_key(|(dest_idx, _)| *dest_idx);
            groups.as_mut_slice().shuffle(rng);

            for (pair_idx, (dest_idx, group)) in groups.into_iter().enumerate() {
                if pair_idx >= built.pipe_pairs.len() || group.len() < 2 {
                    break;
                }
                let (pos_a, pos_b) = built.pipe_pairs[pair_idx];

                // Determine which group entry is the A-side by reading layout
                // byte5 bit 6 from the ROM.  A-side has bit 6 = 0.
                let (idx_a, idx_b) = pipe_ab_order(&group, pickup, catalog, rom);
                pipes.push(PipeAssignment {
                    pool_idx_a: idx_a,
                    pool_idx_b: idx_b,
                    dest_idx,
                    pos_a,
                    pos_b,
                });
            }
        }

        // --- Airship (W1-W7) ---
        let airship = if wi < 7 {
            let airship_pos = catalog.entries.iter()
                .find(|e| e.world_idx == wi && matches!(e.kind, NodeKind::Airship))
                .map(|e| e.grid_pos);
            airship_pos.and_then(|pos| {
                airship_pool.pop().map(|pi| Assignment { pool_idx: pi, pos })
            })
        } else {
            None
        };

        // --- Bowser (W8 only) ---
        let bowser = if wi == 7 {
            bowser_idx.map(|pi| {
                let pos = catalog.entries[pickup.pool[pi].catalog_idx].grid_pos;
                Assignment { pool_idx: pi, pos }
            })
        } else {
            None
        };

        // --- Hammer bro assignments (remaining blank slots) ---
        let mut hammer_bro = Vec::new();
        let mut hb_world_pool = hb_pool_by_world.remove(&wi).unwrap_or_default();
        hb_world_pool.as_mut_slice().shuffle(rng);
        let mut hb_iter = hb_world_pool.into_iter();

        for slot in &built.slots {
            if slot.kind != SlotKind::HammerBro {
                continue;
            }
            if let Some(pi) = hb_iter.next() {
                let le = hb_level_iter.next().unwrap();
                hammer_bro.push(HammerBroAssignment {
                    pool_idx: pi,
                    pos: slot.pos,
                    level_entry: le,
                });
            }
        }

        // Any remaining HB pool entries that didn't get a slot (e.g., BFS
        // unreachable positions) still need pointer table assignments. Place
        // them at their vanilla grid positions.
        for pi in hb_iter {
            let ce = &catalog.entries[pickup.pool[pi].catalog_idx];
            let le = hb_level_iter.next().unwrap();
            hammer_bro.push(HammerBroAssignment {
                pool_idx: pi,
                pos: ce.grid_pos,
                level_entry: le,
            });
        }

        assignments.push(WorldAssignments {
            fortress,
            level,
            pipes,
            airship,
            bowser,
            hammer_bro,
        });
    }

    assignments
}

// ---------------------------------------------------------------------------
// Step 2: Write tile grids
// ---------------------------------------------------------------------------

fn write_tile_grid(
    rom: &mut Rom,
    built: &BuiltWorld,
    wa: &WorldAssignments,
    pickup: &PickupResult,
    catalog: &NodeCatalog,
    sprite_mask: &HashSet<(usize, usize)>,
) {
    let wi = built.world_idx;
    let mut grid = built.grid.clone();

    // Stamp fortress tiles.
    for a in &wa.fortress {
        grid.set(a.pos.0, a.pos.1, TILE_FORTRESS);
    }

    // Stamp pipe tiles (handle spiral castle $5F).
    for pa in &wa.pipes {
        let tile_a = catalog.entries[pickup.pool[pa.pool_idx_a].catalog_idx].tile;
        let tile_b = catalog.entries[pickup.pool[pa.pool_idx_b].catalog_idx].tile;
        grid.set(pa.pos_a.0, pa.pos_a.1, if tile_a == 0x5F { 0x5F } else { TILE_PIPE });
        grid.set(pa.pos_b.0, pa.pos_b.1, if tile_b == 0x5F { 0x5F } else { TILE_PIPE });
    }

    // Stamp airship tile.
    if let Some(a) = &wa.airship {
        let tile = catalog.entries[pickup.pool[a.pool_idx].catalog_idx].tile;
        grid.set(a.pos.0, a.pos.1, tile);
    }

    // Stamp bowser tile.
    if let Some(a) = &wa.bowser {
        let tile = catalog.entries[pickup.pool[a.pool_idx].catalog_idx].tile;
        grid.set(a.pos.0, a.pos.1, tile);
    }

    // Stamp level tiles in BFS order from start.
    let level_pos_set: HashMap<(usize, usize), usize> = wa
        .level
        .iter()
        .enumerate()
        .map(|(i, a)| (a.pos, i))
        .collect();

    let start_pos = rom_data::find_start(&grid);
    let bfs = bfs_ordered(&grid, &built.pipe_pairs, start_pos);

    let mut level_num: u8 = 1;
    let mut assigned: Vec<bool> = vec![false; wa.level.len()];

    for &(pos, _dist) in &bfs {
        if let Some(&la_idx) = level_pos_set.get(&pos) {
            if !assigned[la_idx] {
                let tile = 0x02 + level_num.min(13);
                grid.set(pos.0, pos.1, tile);
                assigned[la_idx] = true;
                level_num += 1;
            }
        }
    }

    // Any level slots not reached by BFS (safety fallback).
    for (i, a) in wa.level.iter().enumerate() {
        if !assigned[i] {
            let tile = 0x02 + level_num.min(13);
            grid.set(a.pos.0, a.pos.1, tile);
            level_num += 1;
        }
    }

    // Stamp lock gap tiles.
    for lock in &built.locks {
        grid.set(lock.pos.0, lock.pos.1, lock.gap_tile);
    }

    // Overwrite sprite-covered positions with connectivity-aware path nodes.
    // W8 army sprites float on top of the grid; the underlying tile must be
    // a plain path node, not a fortress/level tile. Skip BLANK_TILE_OVERRIDES
    // since sprite positions are dynamic (not the vanilla fixed positions).
    for &pos in sprite_mask {
        let tile = super::overworld_pickup::blank_tile_for_dynamic(&grid, wi, pos.0, pos.1);
        grid.set(pos.0, pos.1, tile);
    }

    // Write grid to ROM.
    for r in 0..grid.rows {
        for c in 0..grid.cols {
            let offset = rom_data::map_tile_offset(wi, r, c);
            rom.write_byte(offset, grid.get(r, c));
        }
    }
}

// ---------------------------------------------------------------------------
// Step 3: Write pointer table entries
// ---------------------------------------------------------------------------

fn write_pointer_entries(
    rom: &mut Rom,
    world_idx: usize,
    wa: &WorldAssignments,
    pickup: &PickupResult,
    catalog: &NodeCatalog,
) {
    let world = &WORLDS[world_idx];
    let n = world.entry_count;
    let rt = world.rowtype_offset;
    let sc = rt + n;

    // Reusable entry_idx values: pointer table slots vacated during Phase 2 pickup.
    let cw = &pickup.worlds[world_idx];
    let available_slots: Vec<usize> = cw
        .pool_indices
        .iter()
        .map(|&pi| pickup.pool[pi].entry_idx)
        .collect();

    // Collect all assignments as (pool_idx, pos) for level-like entries.
    let mut all: Vec<(usize, (usize, usize))> = Vec::new();

    for a in &wa.fortress {
        all.push((a.pool_idx, a.pos));
    }
    for a in &wa.level {
        all.push((a.pool_idx, a.pos));
    }
    for pa in &wa.pipes {
        all.push((pa.pool_idx_a, pa.pos_a));
        all.push((pa.pool_idx_b, pa.pos_b));
    }
    if let Some(a) = &wa.airship {
        all.push((a.pool_idx, a.pos));
    }
    if let Some(a) = &wa.bowser {
        all.push((a.pool_idx, a.pos));
    }

    let mut slot_i = 0;

    // Write level-like entries (fortress, level, pipe, airship, bowser).
    for &(pool_idx, pos) in &all {
        if slot_i >= available_slots.len() {
            break;
        }
        let entry_idx = available_slots[slot_i];
        slot_i += 1;

        let pe = &pickup.pool[pool_idx];
        let ce = &catalog.entries[pe.catalog_idx];
        let level_entry = ce
            .level_entry
            .as_ref()
            .expect("assigned pool entry must have level_entry");

        rom_data::write_entry(rom, world, entry_idx, level_entry);

        let (row, col) = pos;
        let row_nib = (row + 2) as u8;
        let screen = (col / 16) as u8;
        let col_in_screen = (col % 16) as u8;

        rom.write_byte(rt + entry_idx, (row_nib << 4) | (level_entry.tileset & 0x0F));
        rom.write_byte(sc + entry_idx, (screen << 4) | col_in_screen);
    }

    // Write hammer bro entries (carry their own LevelEntry).
    for hb in &wa.hammer_bro {
        if slot_i >= available_slots.len() {
            break;
        }
        let entry_idx = available_slots[slot_i];
        slot_i += 1;

        rom_data::write_entry(rom, world, entry_idx, &hb.level_entry);

        let (row, col) = hb.pos;
        let row_nib = (row + 2) as u8;
        let screen = (col / 16) as u8;
        let col_in_screen = (col % 16) as u8;

        rom.write_byte(rt + entry_idx, (row_nib << 4) | (hb.level_entry.tileset & 0x0F));
        rom.write_byte(sc + entry_idx, (screen << 4) | col_in_screen);
    }
}

// ---------------------------------------------------------------------------
// Step 4: Write FX tables
// ---------------------------------------------------------------------------

fn write_fortress_fx(
    rom: &mut Rom,
    world_idx: usize,
    built: &BuiltWorld,
    wa: &WorldAssignments,
    pickup: &PickupResult,
    catalog: &NodeCatalog,
    fx_slot: &mut usize,
) {
    // Pair each lock with its fortress assignment (matched by section).
    let locked_forts: Vec<_> = built
        .locks
        .iter()
        .filter_map(|lock| {
            wa.fortress.iter().enumerate().find(|(fi, _)| {
                // Fortress assignments are ordered by section in assign_pool.
                // Assignment index fi == fort_section for this world.
                *fi == lock.fort_section
            }).map(|(_, fa)| (lock, fa))
        })
        .collect();

    // Write FX world table (up to 4 slots per world).
    let fx_base = rom_data::FX_WORLD_TABLE + world_idx * 4;
    for i in 0..4 {
        if i < locked_forts.len() {
            rom.write_byte(fx_base + i, (*fx_slot + i) as u8);
        } else {
            rom.write_byte(fx_base + i, 0x00);
        }
    }

    for (ordinal_0, (lock, fort_a)) in locked_forts.iter().enumerate() {
        let slot = *fx_slot;
        *fx_slot += 1;

        let ordinal = (ordinal_0 + 1) as u8;

        // Look up boomboom_y_offset from the assigned fortress pool entry.
        let ce = &catalog.entries[pickup.pool[fort_a.pool_idx].catalog_idx];
        let boomboom_y_offset = match &ce.kind {
            NodeKind::Fortress { boomboom_y_offset } => *boomboom_y_offset,
            _ => panic!("fortress assignment must reference a Fortress catalog entry"),
        };

        // Patch Boom-Boom Y-byte.
        let old_y = rom.read_byte(boomboom_y_offset);
        rom.write_byte(boomboom_y_offset, (ordinal << 4) | (old_y & 0x0F));

        // Lock position.
        let (ob_row, ob_col) = lock.pos;
        let col_in_screen = ob_col % 16;
        let screen = ob_col / 16;

        // FX pattern bytes.
        let patterns = overworld_helpers::fx_patterns_for(lock.replace_tile);

        // VRAM address.
        let vram = (0x2880 + ob_row * 64 + col_in_screen * 2) as u16;
        rom.write_byte(FX_VADDR_H + slot, (vram >> 8) as u8);
        rom.write_byte(FX_VADDR_L + slot, (vram & 0xFF) as u8);

        // Map location.
        rom.write_byte(
            rom_data::FX_MAP_LOC_ROW + slot,
            ((ob_row + 2) as u8) << 4,
        );
        rom.write_byte(
            rom_data::FX_MAP_LOC + slot,
            ((col_in_screen as u8) << 4) | (screen as u8),
        );

        // Replacement tile.
        rom.write_byte(rom_data::FX_MAP_TILE_REPLACE + slot, lock.replace_tile);

        // Map_Completions persistence — encodes lock position.
        let comp_col = ob_col as u8;
        let comp_bit = MAP_COMPLETE_BITS[ob_row.min(7)];
        rom.write_byte(FX_MAP_COMP_IDX + slot * 2, comp_col);
        rom.write_byte(FX_MAP_COMP_IDX + slot * 2 + 1, comp_bit);

        // Pattern bytes.
        let pat_off = FX_PATTERNS + slot * 4;
        for (j, &b) in patterns.iter().enumerate() {
            rom.write_byte(pat_off + j, b);
        }
    }
}

// ---------------------------------------------------------------------------
// Pipe A/B side detection
// ---------------------------------------------------------------------------

/// Given the two pool indices for a pipe dest_idx, return (A-side, B-side).
/// The A-side entry has layout byte5 bit 6 = 0 (left-to-right transit level).
/// Falls back to original order if the layout can't be read.
fn pipe_ab_order(
    group: &[usize],
    pickup: &PickupResult,
    catalog: &NodeCatalog,
    rom: &Rom,
) -> (usize, usize) {
    let idx0 = group[0];
    let idx1 = group[1];

    // Read layout byte5 for group[0] to check bit 6.
    let pe = &pickup.pool[idx0];
    let ce = &catalog.entries[pe.catalog_idx];
    if let Some(le) = &ce.level_entry {
        let lay_ptr = u16::from_le_bytes([le.lay_lo, le.lay_hi]);
        if let Some(file_off) = rom_data::layout_file_offset(lay_ptr, le.tileset) {
            let byte5 = rom.read_byte(file_off + 5);
            if byte5 & 0x40 == 0 {
                // group[0] is A-side
                return (idx0, idx1);
            } else {
                // group[0] is B-side, swap
                return (idx1, idx0);
            }
        }
    }
    // Fallback: preserve original order
    (idx0, idx1)
}

// ---------------------------------------------------------------------------
// Step 5: Write pipe destination tables
// ---------------------------------------------------------------------------

fn write_pipe_dests(rom: &mut Rom, wa: &WorldAssignments) {
    for pa in &wa.pipes {
        pipe_helpers::write_pipe_dest(rom, pa.dest_idx, pa.pos_a, pa.pos_b);
    }
}

// ---------------------------------------------------------------------------
// Step 6: Move W8 army sprites
// ---------------------------------------------------------------------------

/// Decide where each W8 army sprite goes. Returns (sprite_slot, grid_pos).
fn pick_w8_sprite_positions<R: Rng>(
    wa_w8: &WorldAssignments,
    rng: &mut R,
) -> Vec<(usize, (usize, usize))> {
    let mut fort_positions: Vec<(usize, usize)> = wa_w8
        .fortress
        .iter()
        .map(|a| a.pos)
        .collect();
    fort_positions.as_mut_slice().shuffle(rng);

    let mut level_positions: Vec<(usize, usize)> = wa_w8
        .level
        .iter()
        .map(|a| a.pos)
        .collect();
    level_positions.as_mut_slice().shuffle(rng);

    let mut fort_iter = fort_positions.into_iter();
    let mut level_iter = level_positions.into_iter();

    let mut result = Vec::new();
    for &(sprite_slot, is_fortress) in W8_ARMY_SPRITES {
        let pos = if is_fortress {
            fort_iter.next()
        } else {
            level_iter.next()
        };
        if let Some(p) = pos {
            result.push((sprite_slot, p));
        }
    }
    result
}

/// Write W8 army sprite positions to the map object tables.
fn write_w8_sprites(rom: &mut Rom, positions: &[(usize, (usize, usize))]) {
    for &(sprite_slot, (row, col)) in positions {
        rom_data::write_map_sprite_position(rom, 7, sprite_slot, row, col);
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
    fn test_pool_assignment_exhaustive() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = super::super::node_catalog::NodeCatalog::build(&rom);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog);
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let build = super::super::overworld_build::build(&rom, &pickup, &catalog, &mut rng);

        let mut rng2 = ChaCha8Rng::seed_from_u64(99);
        let assignments = assign_pool(&rom, &build, &pickup, &catalog, &mut rng2, true);

        // Collect all assigned pool indices.
        let mut used: Vec<usize> = Vec::new();
        for wa in &assignments {
            for a in &wa.fortress {
                used.push(a.pool_idx);
            }
            for a in &wa.level {
                used.push(a.pool_idx);
            }
            for pa in &wa.pipes {
                used.push(pa.pool_idx_a);
                used.push(pa.pool_idx_b);
            }
            if let Some(a) = &wa.airship {
                used.push(a.pool_idx);
            }
            if let Some(a) = &wa.bowser {
                used.push(a.pool_idx);
            }
            for hb in &wa.hammer_bro {
                used.push(hb.pool_idx);
            }
        }

        used.sort();
        used.dedup();
        assert_eq!(
            used.len(),
            pickup.pool.len(),
            "every pool entry must be assigned exactly once: assigned {} of {}",
            used.len(),
            pickup.pool.len(),
        );
    }

    #[test]
    fn test_write_deterministic() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = super::super::node_catalog::NodeCatalog::build(&rom);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog);

        let mut rom1 = rom.clone();
        let mut rom2 = rom.clone();

        for pass in 0..2 {
            let target = if pass == 0 { &mut rom1 } else { &mut rom2 };
            let mut rng = ChaCha8Rng::seed_from_u64(42);
            let build = super::super::overworld_build::build(&rom, &pickup, &catalog, &mut rng);
            write_overworld(target, &build, &pickup, &catalog, &mut rng, true);
        }

        assert_eq!(rom1.data, rom2.data, "same seed must produce identical output");
    }

    #[test]
    fn test_w8_sprites_moved() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = super::super::node_catalog::NodeCatalog::build(&rom);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog);
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let build = super::super::overworld_build::build(&rom, &pickup, &catalog, &mut rng);

        let mut test_rom = rom.clone();
        write_overworld(&mut test_rom, &build, &pickup, &catalog, &mut rng, true);

        // Read W8 sprite positions after write.
        let positions = rom_data::read_map_sprite_positions(&test_rom, 7);

        // The army sprites (slots 2-5) should be at slot positions, not vanilla.
        // We can't predict exact positions (random), but they should be valid
        // grid positions within the W8 map.
        for &(row, col) in &positions {
            assert!(row < 9, "W8 sprite row {row} out of range");
            assert!(col < 64, "W8 sprite col {col} out of range");
        }
    }

    #[test]
    fn test_fx_slots_valid() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = super::super::node_catalog::NodeCatalog::build(&rom);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog);
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let build = super::super::overworld_build::build(&rom, &pickup, &catalog, &mut rng);

        let mut test_rom = rom.clone();
        write_overworld(&mut test_rom, &build, &pickup, &catalog, &mut rng, true);

        // Count total locked fortresses across all worlds.
        let total_locks: usize = build.worlds.iter().map(|b| b.locks.len()).sum();

        // Read FX world tables — count non-zero entries.
        let mut fx_count = 0;
        for wi in 0..8 {
            let fx_base = rom_data::FX_WORLD_TABLE + wi * 4;
            for i in 0..4 {
                let slot_idx = test_rom.read_byte(fx_base + i);
                if slot_idx != 0 || (i == 0 && build.worlds[wi].locks.len() > 0) {
                    // Slot 0 is valid (could be index 0), so check lock count.
                    if i < build.worlds[wi].locks.len() {
                        fx_count += 1;
                    }
                }
            }
        }

        assert_eq!(
            fx_count, total_locks,
            "FX slot count {fx_count} != total locks {total_locks}",
        );
    }

    #[test]
    fn test_pointer_table_sorted() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = super::super::node_catalog::NodeCatalog::build(&rom);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog);
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let build = super::super::overworld_build::build(&rom, &pickup, &catalog, &mut rng);

        let mut test_rom = rom.clone();
        write_overworld(&mut test_rom, &build, &pickup, &catalog, &mut rng, true);

        // Verify each world's pointer table is sorted by (screen, row, col).
        for wi in 0..8 {
            let world = &WORLDS[wi];
            let n = world.entry_count;
            let rt = world.rowtype_offset;
            let sc = rt + n;

            let mut prev = (0u8, 0u8, 0u8);
            for i in 0..n {
                let rowtype = test_rom.read_byte(rt + i);
                let scrcol = test_rom.read_byte(sc + i);
                let screen = (scrcol >> 4) & 0x0F;
                let row_nib = (rowtype >> 4) & 0x0F;
                let col = scrcol & 0x0F;
                let key = (screen, row_nib, col);

                assert!(
                    key >= prev,
                    "W{} entry {i} not sorted: ({},{},{}) < ({},{},{})",
                    wi + 1, key.0, key.1, key.2, prev.0, prev.1, prev.2,
                );
                prev = key;
            }
        }
    }

    /// Generate a full ROM for manual/emulator testing.
    #[test]
    #[ignore]
    fn test_generate_rom() {
        let rom = match load_rom() {
            Some(r) => r,
            None => {
                eprintln!("ROM not found, skipping");
                return;
            }
        };
        let catalog = super::super::node_catalog::NodeCatalog::build(&rom);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog);

        for seed in [42u64, 123, 999] {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let build = super::super::overworld_build::build(&rom, &pickup, &catalog, &mut rng);

            let mut out = rom.clone();

            // Apply QoL patches that the builder expects.
            super::super::qol::fix_w3_drawbridges(&mut out);
            super::super::qol::remove_w2_rock(&mut out);
            super::super::qol::fix_big_q_block_rooms(&mut out);

            write_overworld(&mut out, &build, &pickup, &catalog, &mut rng, true);

            let filename = format!("writer_test_seed{seed}.nes");
            std::fs::write(&filename, &out.data).unwrap();
            eprintln!("Wrote {filename}");
        }
    }
}
