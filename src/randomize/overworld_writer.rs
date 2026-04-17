/// Phase 4 of the overworld builder rewrite: Write.
///
/// Takes `BuildResult` (Phase 3) + `PickupResult` (Phase 2) + `NodeCatalog`
/// (Phase 1) + RNG, assigns concrete pool entries to slots, and writes all
/// ROM data: tile grids, pointer tables, FX tables, pipe destinations, and
/// W8 army sprite positions.

use std::collections::{HashMap, HashSet};

use rand::Rng;
use rand::seq::{IndexedRandom, SliceRandom};

use crate::rom::Rom;

use super::node_catalog::{NodeCatalog, NodeKind};
use super::overworld_build::{bfs_ordered, BuildResult, BuiltWorld, SlotKind};
use super::overworld_helpers;
use super::overworld_pickup::PickupResult;
use super::pipe_helpers;
use super::rom_data::{
    self, FORTRESS_1F_OBJ_PTR, FX_MAP_COMP_IDX, FX_PATTERNS, FX_VADDR_H, FX_VADDR_L,
    MAP_COMPLETE_BITS, TILE_PIPE, WORLDS,
};



// ---------------------------------------------------------------------------
// Hammer bro pool interleaving
// ---------------------------------------------------------------------------

/// Reorder HB level entries so unique `obj_ptr` values are evenly interleaved.
///
/// Without this, the cycling pool is dominated by entries sharing `obj_ptr`
/// 0xC640 (8 of 13 entries), causing most HB encounters to have identical
/// enemies. Interleaving ensures each unique enemy set appears once before
/// any repeats: round-robin through obj_ptr groups, picking a random layout
/// variant from each group per round.
fn interleave_hb_by_obj_ptr<R: Rng>(
    levels: Vec<rom_data::LevelEntry>,
    rng: &mut R,
) -> Vec<rom_data::LevelEntry> {
    if levels.is_empty() {
        return levels;
    }

    // Group by obj_ptr using BTreeMap for deterministic iteration order.
    let mut groups: std::collections::BTreeMap<u16, Vec<rom_data::LevelEntry>> =
        std::collections::BTreeMap::new();
    for le in levels {
        let obj = u16::from_le_bytes([le.obj_lo, le.obj_hi]);
        groups.entry(obj).or_default().push(le);
    }

    // Shuffle within each group and collect group keys in random order.
    let mut keys: Vec<u16> = groups.keys().copied().collect();
    keys.as_mut_slice().shuffle(rng);
    for group in groups.values_mut() {
        group.as_mut_slice().shuffle(rng);
    }

    // Round-robin: pick one from each group per round until all exhausted.
    let max_len = groups.values().map(|g| g.len()).max().unwrap_or(0);
    let mut result = Vec::new();
    for round in 0..max_len {
        for &key in &keys {
            let group = groups.get(&key).unwrap();
            if round < group.len() {
                result.push(group[round].clone());
            }
        }
    }

    result
}

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

    // Cycling HB level pool for fallback pointer table entries (same interleaving).
    let hb_fallback_levels = interleave_hb_by_obj_ptr(catalog.unique_hammer_bro_levels(), rng);
    let mut hb_fallback_iter = hb_fallback_levels.iter().cycle().cloned();

    let mut fx_slot = 0usize;
    for wi in 0..8 {
        let built = &build.worlds[wi];
        let wa = &assignments[wi];
        let sprite_mask = if wi == 7 { &w8_sprite_pos_set } else { &HashSet::new() };

        write_tile_grid(rom, built, wa, pickup, catalog, sprite_mask, rng);
        write_pointer_entries(rom, wi, built, wa, pickup, catalog, &mut hb_fallback_iter);
        write_fortress_fx(rom, wi, built, wa, pickup, catalog, &mut fx_slot);
        write_pipe_dests(rom, wi, wa);
        pipe_helpers::resort_pointer_table(rom, wi);
        // Do not sync map object sprite positions: the overworld builder never
        // moves MapObject entries (W7 piranhas), so vanilla sprite positions are
        // correct.  The sync function uses fixed indices that become invalid
        // after resort_pointer_table, causing sprites to jump to wrong tiles.
    }

    write_w8_sprites(rom, &w8_sprite_positions);
    patch_fortress_fx_screen_check(rom);
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
    // Pipe groups: world → dest_idx → Vec<(pool_idx, is_a_side)>.
    let mut pipe_groups: HashMap<usize, HashMap<usize, Vec<(usize, bool)>>> = HashMap::new();
    for (pi, pe) in pickup.pool.iter().enumerate() {
        let entry = &catalog.entries[pe.catalog_idx];
        match &entry.kind {
            NodeKind::Fortress { .. } => fort_pool.push(pi),
            NodeKind::Level => level_pool.push(pi),
            NodeKind::Airship => airship_pool.push(pi),
            NodeKind::Bowser => {
                debug_assert!(bowser_idx.is_none(), "duplicate Bowser in pickup pool");
                bowser_idx = Some(pi);
            }
            NodeKind::Pipe { dest_idx, is_a_side } => {
                pipe_groups
                    .entry(entry.world_idx)
                    .or_default()
                    .entry(*dest_idx)
                    .or_default()
                    .push((pi, *is_a_side));
            }
            _ => {} // HammerBro entries don't need a pool — see HB assignment below
        }
    }

    // Build cycling hammer bro level pool, interleaved by obj_ptr so each
    // unique enemy set appears once before any repeats.
    let hb_levels = interleave_hb_by_obj_ptr(catalog.unique_hammer_bro_levels(), rng);
    let mut hb_level_iter = hb_levels.iter().cycle().cloned();

    // Build per-obj_ptr groups for sprite position round-robin assignment.
    // This ensures each HB sprite encounter in a world gets a different
    // enemy set (different obj_ptr = different enemies).
    let mut hb_obj_groups: std::collections::BTreeMap<u16, Vec<rom_data::LevelEntry>> =
        std::collections::BTreeMap::new();
    for le in &hb_levels {
        let obj = u16::from_le_bytes([le.obj_lo, le.obj_hi]);
        hb_obj_groups.entry(obj).or_default().push(le.clone());
    }
    let mut hb_group_keys: Vec<u16> = hb_obj_groups.keys().copied().collect();
    hb_group_keys.as_mut_slice().shuffle(rng);
    for group in hb_obj_groups.values_mut() {
        group.as_mut_slice().shuffle(rng);
    }

    // --- Pre-assign the 1-F fortress to a secret-exit-safe slot ---
    //
    // The 1-F fortress level has a secret exit that bypasses Boom-Boom
    // (no crystal ball → no FX trigger → lock stays closed). It must
    // land in a slot whose lock is marked secret_exit_safe to avoid
    // softlocking the player.

    // Find the 1-F pool entry.
    let fort_1f_pos = fort_pool.iter().position(|&pi| {
        let ce = &catalog.entries[pickup.pool[pi].catalog_idx];
        ce.level_entry.as_ref().is_some_and(|le| {
            u16::from_le_bytes([le.obj_lo, le.obj_hi]) == FORTRESS_1F_OBJ_PTR
        })
    }).expect("1-F fortress not found in pool");
    let fort_1f_pi = fort_pool.remove(fort_1f_pos);

    // Collect all safe (world_idx, section) slots. In intra-world mode,
    // 1-F can only go to a safe slot in its origin world.
    let fort_1f_origin = catalog.entries[pickup.pool[fort_1f_pi].catalog_idx].world_idx;
    let mut safe_slots: Vec<(usize, usize)> = Vec::new();
    for wi in 0..8 {
        if !cross_world && wi != fort_1f_origin {
            continue;
        }
        for lock in &build.worlds[wi].locks {
            if lock.secret_exit_safe {
                safe_slots.push((wi, lock.fort_section));
            }
        }
    }
    // Pre-assign 1-F to a safe slot if one exists. In intra-world mode,
    // W1 may have no safe lock — that's fine, the player must use the
    // normal exit (beat Boom-Boom) to open the lock.
    let mut preassigned_forts: HashMap<(usize, usize), usize> = HashMap::new();
    if let Some(&(safe_wi, safe_section)) = safe_slots.choose(rng) {
        preassigned_forts.insert((safe_wi, safe_section), fort_1f_pi);
    } else {
        // No safe slot available — return 1-F to the regular pool.
        fort_pool.push(fort_1f_pi);
    }

    // Shuffle remaining fortress and level pools.
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
                // Check if this slot was pre-assigned (1-F safe placement).
                let pi = if let Some(pre) = preassigned_forts.remove(&(wi, section)) {
                    pre
                } else if cross_world {
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
            let mut groups: Vec<(usize, Vec<(usize, bool)>)> = world_pipes.drain().collect();
            groups.sort_by_key(|(dest_idx, _)| *dest_idx);
            groups.as_mut_slice().shuffle(rng);

            for (pair_idx, (dest_idx, group)) in groups.into_iter().enumerate() {
                if pair_idx >= built.pipe_pairs.len() || group.len() < 2 {
                    break;
                }
                let (pos_a, pos_b) = built.pipe_pairs[pair_idx];

                // Use the is_a_side flag precomputed during catalog building.
                let (idx_a, idx_b) = if group[0].1 {
                    (group[0].0, group[1].0)
                } else {
                    (group[1].0, group[0].0)
                };
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
        //
        // Every SlotKind::HammerBro position gets a cycling HB level, up to
        // the remaining pointer table capacity after level-like assignments.
        //
        // Sprite positions (actual encounters the player fights) get a
        // dedicated per-obj_ptr round-robin so each encounter in a world
        // has a different enemy set. Filler positions (blank tiles needing
        // valid pointer entries) use the normal cycling pool.
        let level_like_count = fortress.len() + level.len() + pipes.len() * 2;
        let remaining_slots = pickup.worlds[wi].pool_indices.len().saturating_sub(level_like_count);

        let sprite_positions: HashSet<(usize, usize)> =
            rom_data::read_hb_sprite_positions(rom, wi).into_iter().collect();

        let mut sprite_slots = Vec::new();
        let mut filler_slots = Vec::new();
        for slot in &built.slots {
            if slot.kind != SlotKind::HammerBro { continue; }
            if sprite_positions.contains(&slot.pos) {
                sprite_slots.push(slot.pos);
            } else {
                filler_slots.push(slot.pos);
            }
        }

        // Assign sprite slots from per-obj_ptr round-robin.
        let mut hammer_bro = Vec::new();
        let mut sprite_obj_idx = 0usize;
        for pos in &sprite_slots {
            if hammer_bro.len() >= remaining_slots { break; }
            let key = hb_group_keys[sprite_obj_idx % hb_group_keys.len()];
            let group = hb_obj_groups.get(&key).unwrap();
            let le = group[sprite_obj_idx / hb_group_keys.len() % group.len()].clone();
            sprite_obj_idx += 1;
            hammer_bro.push(HammerBroAssignment { pos: *pos, level_entry: le });
        }

        // Assign filler slots from normal cycling pool.
        for pos in &filler_slots {
            if hammer_bro.len() >= remaining_slots { break; }
            hammer_bro.push(HammerBroAssignment {
                pos: *pos,
                level_entry: hb_level_iter.next().unwrap(),
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

fn write_tile_grid<R: Rng>(
    rom: &mut Rom,
    built: &BuiltWorld,
    wa: &WorldAssignments,
    pickup: &PickupResult,
    catalog: &NodeCatalog,
    sprite_mask: &HashSet<(usize, usize)>,
    rng: &mut R,
) {
    let wi = built.world_idx;
    let mut grid = built.grid.clone();

    // Stamp fortress tiles. The game treats $67, $EB, and $6A as fortress
    // tiles (Map_Removable_Tiles + completion-unsafe), so pick per-fortress.
    // $6A's CHR animation is frozen by patch_metatile_6a_freeze.
    const FORTRESS_TILES: [u8; 3] = [0x67, 0xEB, 0x6A];
    for a in &wa.fortress {
        let tile = FORTRESS_TILES[rng.random_range(..FORTRESS_TILES.len())];
        grid.set(a.pos.0, a.pos.1, tile);
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

    // Level tile sequence: $03-$0B = levels 1-9 (vanilla numbered tiles),
    // $0C-$15 = levels 10-19 (double-digit tiles with custom "1" tens digit
    // patched by patch_double_digit_metatiles). $69 (pyramid) is a level-20+
    // fallback with no valid display.
    const LEVEL_TILES: [u8; 20] = [
        0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B,
        0x0C, 0x0D, 0x0E, 0x0F, 0x10, 0x11, 0x12, 0x13, 0x14,
        0x15, 0x69,
    ];

    let mut level_idx: usize = 0;
    let mut assigned: Vec<bool> = vec![false; wa.level.len()];

    for &(pos, _dist) in &bfs {
        if let Some(&la_idx) = level_pos_set.get(&pos) {
            if !assigned[la_idx] {
                let tile = LEVEL_TILES[level_idx.min(LEVEL_TILES.len() - 1)];
                grid.set(pos.0, pos.1, tile);
                assigned[la_idx] = true;
                level_idx += 1;
            }
        }
    }

    // Any level slots not reached by BFS (safety fallback).
    for (i, a) in wa.level.iter().enumerate() {
        if !assigned[i] {
            let tile = LEVEL_TILES[level_idx.min(LEVEL_TILES.len() - 1)];
            grid.set(a.pos.0, a.pos.1, tile);
            level_idx += 1;
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
    built: &BuiltWorld,
    wa: &WorldAssignments,
    pickup: &PickupResult,
    catalog: &NodeCatalog,
    hb_level_iter: &mut impl Iterator<Item = rom_data::LevelEntry>,
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
    // Airship and bowser are not picked up — their pointer table entries
    // stay vanilla so the autoscroll patch's hardcoded offsets remain valid.

    debug_assert!(
        all.len() + wa.hammer_bro.len() <= available_slots.len(),
        "W{}: slot overflow: need {} but only {} available",
        world_idx + 1,
        all.len() + wa.hammer_bro.len(),
        available_slots.len(),
    );

    let mut slot_i = 0;

    // Write level-like entries (fortress, level, pipe).
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

    // Fill any remaining unused pointer table slots with valid HB levels.
    // These are blank node tiles on the grid that weren't assigned slots
    // during the build phase (e.g., not BFS-reachable at build time).
    // Place them at actual blank positions so the player doesn't walk onto
    // a tile with no pointer entry (which crashes the game).
    if slot_i < available_slots.len() {
        // Collect positions already covered by assignments above.
        let mut covered: HashSet<(usize, usize)> = HashSet::new();
        for &(_, pos) in &all {
            covered.insert(pos);
        }
        for hb in &wa.hammer_bro {
            covered.insert(hb.pos);
        }

        // Find blank tile positions on the grid that have no entry.
        // Exclude positions of catalog entries that were never picked up
        // (airship, Bowser, toad house, map objects like piranhas, start).
        // These already have valid pointer table entries from vanilla, so
        // filling them wastes a slot that should go to a real uncovered blank.
        let already_has_entry: HashSet<(usize, usize)> = catalog.entries.iter()
            .filter(|e| e.world_idx == world_idx && !matches!(e.kind,
                NodeKind::Level | NodeKind::Fortress { .. }
                | NodeKind::Pipe { .. } | NodeKind::HammerBro
                | NodeKind::BonusGame))
            .map(|e| e.grid_pos)
            .collect();
        let mut uncovered_blanks: Vec<(usize, usize)> = Vec::new();
        for r in 0..built.grid.rows {
            for c in 0..built.grid.cols {
                if rom_data::VALID_BLANK_TILES.contains(&built.grid.get(r, c))
                    && !covered.contains(&(r, c))
                    && !already_has_entry.contains(&(r, c))
                {
                    uncovered_blanks.push((r, c));
                }
            }
        }
        let mut blank_iter = uncovered_blanks.into_iter();

        while slot_i < available_slots.len() {
            let entry_idx = available_slots[slot_i];
            slot_i += 1;
            let le = hb_level_iter.next().unwrap();
            rom_data::write_entry(rom, world, entry_idx, &le);

            if let Some((row, col)) = blank_iter.next() {
                // Place at actual blank tile position.
                let row_nib = (row + 2) as u8;
                let screen = (col / 16) as u8;
                let col_in_screen = (col % 16) as u8;
                rom.write_byte(rt + entry_idx, (row_nib << 4) | (le.tileset & 0x0F));
                rom.write_byte(sc + entry_idx, (screen << 4) | col_in_screen);
            } else {
                // No more blanks — park at unreachable position.
                rom.write_byte(rt + entry_idx, le.tileset & 0x0F); // row_nib=0 → grid_row=-2
                rom.write_byte(sc + entry_idx, 0x00);
            }
        }
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
        // Bit 0 of the row byte is the poof-only flag: when set, the screen
        // check skips the VRAM tile write (so the tile doesn't appear over a
        // blackout effect) but still plays poof sprites and updates map data.
        let on_blackout = world_idx == 7 && screen == 2; // W8 screen 2
        let row_byte = ((ob_row + 2) as u8) << 4 | (on_blackout as u8);
        rom.write_byte(rom_data::FX_MAP_LOC_ROW + slot, row_byte);
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
// Step 4b: FX screen-check patch (6502)
// ---------------------------------------------------------------------------

/// Patches the MO_DoFortressFX engine routine so the lock-breaking visual
/// animation (VRAM pattern write + poof sprites) is skipped when the lock is
/// not on the currently visible screen.
///
/// In vanilla the fortress and its lock are always on the same screen, so the
/// animation plays correctly.  When we shuffle fortress/lock positions, the
/// lock can end up on a different screen.  Because the VRAM write and sprite
/// positions are screen-relative, playing the animation on the wrong screen
/// causes a visual glitch (tile placed at wrong spot + poof on wrong screen).
///
/// The map-data replacement (tile + Map_Completions) is NOT screen-relative
/// and always works correctly, so we only need to skip the visual part.
///
/// The map viewport scrolls in 128-pixel half-screen steps.  ZP $12
/// (Map_Scroll_XHi) is the scroll page and $FD (Map_Scroll_X) is either
/// 0 or 128.  When $FD=128 the viewport straddles two grid screens, so
/// the lock is visible if its screen == $12 OR (screen == $12+1 AND $FD≥128).
///
/// Hook: replace 3 bytes at file 0x148F6 (CPU $C8E6):
///   vanilla: A9 01 85  (LDA #$01 / STA $20[hi])
///   patched: 4C 44 D5  (JMP $D544)
///
/// Custom code at file 0x15554 (CPU $D544, PRG010 free space):
///   Read lock screen from FortressFX_MapLocation[slot] & 0x0F.
///   Compare with $12 — if equal, animate.
///   Otherwise check if lock_screen == $12+1 AND $FD >= 128 — if so, animate.
///   Else skip animation (data-only update via JMP $C952).
/// Patch metatile LL quadrant for double-digit level tiles (0x0D–0x15).
///
/// Vanilla tiles 0x0D–0x15 have a blank LL (CHR 0xBE = solid fill). We write
/// a custom CHR tile with a "1" tens digit into an unreferenced slot, then
/// point the LL quadrant of tiles 0x0D–0x15 at it.
///
/// CHR tile 0xCC is unreferenced by every overworld metatile (vanilla 0xCB
/// was unsafe — it's the LR of metatile 0x0B, the vanilla "level 9" tile).
/// CHR page 0x17 covers tile IDs 0xC0–0xFF and is stable (no MMC3 mid-frame
/// bank swapping).
pub(crate) fn patch_double_digit_metatiles(rom: &mut Rom) {
    // Metatile quadrant tables at 0x18010: UL(256) LL(256) UR(256) LR(256).
    const METATILE_LL_BASE: usize = 0x18010 + 256; // 0x18110

    // Overwrite CHR tile 0xCC with our custom "1" digit.
    // CHR page 0x17 covers tile IDs 0xC0–0xFF; tile 0xCC = index 0x0C.
    const CHR_START: usize = 0x40010;
    const CHR_PAGE_17: usize = CHR_START + 0x17 * 0x400;
    const TILE_CC_OFFSET: usize = CHR_PAGE_17 + 0x0C * 16;
    // Arrow shape (cols 2–5) + "1" serif (col 6 row 1) + right border (col 7 = color 2).
    #[rustfmt::skip]
    const DIGIT_1_LL: [u8; 16] = [
        0x7E, 0x7C, 0x7E, 0x7E, 0x7E, 0x7E, 0x7F, 0x00, // plane 0
        0xA1, 0xB3, 0xB9, 0xBD, 0xB9, 0xB1, 0x80, 0xFF, // plane 1
    ];
    rom.write_range(TILE_CC_OFFSET, &DIGIT_1_LL);

    // Point LL of tiles 0x0D–0x15 (levels 10–19) at CHR tile 0xCC.
    for tile_id in 0x0Du8..=0x15 {
        rom.write_byte(METATILE_LL_BASE + tile_id as usize, 0xCC);
    }
}

/// Freeze metatile 0x6A's CHR animation so it can serve as a static fortress tile.
///
/// The overworld NMI handler rotates MMC3 R0 (2KB BG bank) through pages
/// (0x14+0x15), (0x70+0x71), (0x72+0x73), (0x74+0x75) to animate tiles $00-$7F.
/// Metatile 0x6A's quadrants (CHR 0x64-0x67) fall in this animated range, so
/// it visibly swaps between frames.
///
/// Copy the base (page 0x15) pixel data for CHR tiles 0x64-0x67 into the
/// same positions in pages 0x71, 0x73, 0x75 so every frame renders identically.
/// Metatile 0x6A is the only metatile referencing CHR 0x64-0x67, so no other
/// tile is affected.
pub(crate) fn patch_metatile_6a_freeze(rom: &mut Rom) {
    const CHR_BASE: usize = 0x40010;
    const BASE_PAGE: usize = 0x15;
    const ANIM_PAGES: [usize; 3] = [0x71, 0x73, 0x75];
    // Tiles 0x64-0x67 live in page 0x15 at local indices 0x24-0x27.
    for local_idx in 0x24..=0x27usize {
        let base_off = CHR_BASE + BASE_PAGE * 0x400 + local_idx * 16;
        let base_tile: [u8; 16] = core::array::from_fn(|i| rom.read_byte(base_off + i));
        for page in ANIM_PAGES {
            let off = CHR_BASE + page * 0x400 + local_idx * 16;
            rom.write_range(off, &base_tile);
        }
    }
}

fn patch_fortress_fx_screen_check(rom: &mut Rom) {
    // --- Hook at $C8E6 ---
    const HOOK_OFFSET: usize = 0x148F6; // file offset of CPU $C8E6
    rom.write_byte(HOOK_OFFSET, 0x4C);     // JMP
    rom.write_byte(HOOK_OFFSET + 1, 0x44); // lo($D544)
    rom.write_byte(HOOK_OFFSET + 2, 0xD5); // hi($D544)

    // --- Custom code at $D544 (file 0x15554) ---
    //
    // Same screen (lock_screen == $12): → animate
    //
    // Next screen (lock_screen == $12 + 1):
    //   $FD < 128 → skip (no half-screen straddle, next screen not visible)
    //   col_in_screen >= 8 → skip (right half of next screen not visible)
    //   else → animate
    //
    // Other → skip
    const CODE_OFFSET: usize = rom_data::FS_FX_SCREEN_CHECK;
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Load lock screen number.
        0xAC, 0x45, 0x07,       // +0:  LDY $0745         ; FX slot
        0xB9, 0x56, 0xC8,       // +3:  LDA $C856,Y       ; FortressFX_MapLocation
        0x29, 0x0F,             // +6:  AND #$0F           ; lock screen (0-3)
        // Check: lock_screen == $12?
        0xC5, 0x12,             // +8:  CMP $12            ; current scroll page
        0xF0, 0x14,             // +10: BEQ +20 (→ +32)    ; match → same-screen → animate
        // Check: lock_screen == $12 + 1?
        0x38,                   // +12: SEC
        0xE5, 0x12,             // +13: SBC $12            ; A = lock_screen - $12
        0xC9, 0x01,             // +15: CMP #$01           ; exactly 1 screen ahead?
        0xD0, 0x1E,             // +17: BNE +30 (→ +49)    ; no → skip
        // --- Next-screen path ---
        0xA5, 0xFD,             // +19: LDA $FD            ; Map_Scroll_X
        0xC9, 0x80,             // +21: CMP #$80           ; straddle? (>= 128)
        0x90, 0x18,             // +23: BCC +24 (→ +49)    ; no → skip
        0xB9, 0x56, 0xC8,       // +25: LDA $C856,Y       ; reload FortressFX_MapLocation
        0x29, 0x80,             // +28: AND #$80           ; bit 7 = col_in_screen >= 8
        0xD0, 0x11,             // +30: BNE +17 (→ +49)    ; col >= 8 → off right edge → skip
        // Falls through: col < 8 → animate
        // --- Animate: check poof-only flag in FX_MAP_LOC_ROW bit 0 ---
        // (also reached by BEQ at +10 for same-screen case)
        0xA9, 0x01,             // +32: LDA #$01
        0x85, 0x20,             // +34: STA $20
        0xB9, 0x45, 0xC8,       // +36: LDA $C845,Y       ; FX_MAP_LOC_ROW
        0x29, 0x01,             // +39: AND #$01           ; poof-only flag?
        0xD0, 0x03,             // +41: BNE +3 (→ +46)     ; yes → poof only
        0x4C, 0xEA, 0xC8,       // +43: JMP $C8EA          ; full animate (VRAM + data)
        // --- Poof only: sprites + data, no VRAM tile write ---
        0x4C, 0x52, 0xC9,       // +46: JMP $C952          ; data update ($20=1 → poof plays)
        // --- Skip: data-only update, no animation ---
        0xA9, 0x06,             // +49: LDA #$06
        0x85, 0x20,             // +51: STA $20
        0x4C, 0x52, 0xC9,       // +53: JMP $C952          ; → Map_Completions update
    ];
    for (i, &b) in code.iter().enumerate() {
        rom.write_byte(CODE_OFFSET + i, b);
    }
    // NOP-pad remaining bytes from the previous (longer) version of this patch
    // so stale opcodes can't execute if control somehow falls through.
    for i in code.len()..67 {
        rom.write_byte(CODE_OFFSET + i, 0xEA); // NOP
    }
}

// ---------------------------------------------------------------------------
// Step 5: Write pipe destination tables
// ---------------------------------------------------------------------------

fn write_pipe_dests(rom: &mut Rom, world_idx: usize, wa: &WorldAssignments) {
    for pa in &wa.pipes {
        pipe_helpers::write_pipe_dest(rom, pa.dest_idx, pa.pos_a, pa.pos_b, world_idx);
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
        let catalog = super::super::node_catalog::NodeCatalog::build(&rom, false);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, true);
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
        }

        // No pool entry assigned more than once.
        let total_used = used.len();
        used.sort();
        used.dedup();
        assert_eq!(
            used.len(),
            total_used,
            "duplicate pool assignments detected",
        );

        // Per-world assignment count must not exceed available pointer table slots.
        for (wi, wa) in assignments.iter().enumerate() {
            let level_like = wa.fortress.len() + wa.level.len() + wa.pipes.len() * 2;
            let total = level_like + wa.hammer_bro.len();
            let available = pickup.worlds[wi].pool_indices.len();
            assert!(
                total <= available,
                "W{}: {} assignments exceed {} available pointer table slots",
                wi + 1, total, available,
            );
        }
    }

    #[test]
    fn test_write_deterministic() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = super::super::node_catalog::NodeCatalog::build(&rom, false);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, true);

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
        let catalog = super::super::node_catalog::NodeCatalog::build(&rom, false);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, true);
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
        let catalog = super::super::node_catalog::NodeCatalog::build(&rom, false);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, true);
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
        let catalog = super::super::node_catalog::NodeCatalog::build(&rom, false);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, true);
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

    /// Every BFS-reachable blank tile must have a pointer table entry after
    /// writing. Uncovered blanks crash the game when the player walks onto them.
    #[test]
    fn test_no_uncovered_blank_nodes() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = super::super::node_catalog::NodeCatalog::build(&rom, false);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, true);

        for seed in [42u64, 123, 999, 7777, 31337] {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let build = super::super::overworld_build::build(&rom, &pickup, &catalog, &mut rng);

            let mut test_rom = rom.clone();
            super::super::qol::fix_w3_drawbridges(&mut test_rom);
            super::super::qol::remove_w2_rock(&mut test_rom);
            super::super::qol::remove_w3_boat_rock(&mut test_rom);
            super::super::qol::fix_big_q_block_rooms(&mut test_rom);
            write_overworld(&mut test_rom, &build, &pickup, &catalog, &mut rng, true);

            let pipes_by_world = rom_data::read_pipe_pairs(&test_rom);

            for wi in 0..8 {
                let grid = rom_data::read_tile_grid(&test_rom, wi);
                let pipe_pairs = pipes_by_world.get(&wi)
                    .cloned()
                    .unwrap_or_default();
                let walk = super::super::map_walker::walk_map(&grid, &pipe_pairs, None);

                // Collect positions that have pointer table entries.
                let world = &WORLDS[wi];
                let mut covered: HashSet<(usize, usize)> = HashSet::new();
                for i in 0..world.entry_count {
                    let pos = rom_data::entry_grid_position(&test_rom, world, i);
                    if pos.0 < grid.rows {
                        covered.insert(pos);
                    }
                }

                // Every reachable blank tile must be covered.
                for &node in &walk.nodes {
                    let (r, c) = node;
                    if r >= grid.rows || c >= grid.cols {
                        continue;
                    }
                    let tile = grid.get(r, c);
                    if !rom_data::VALID_BLANK_TILES.contains(&tile) {
                        continue;
                    }
                    assert!(
                        covered.contains(&node),
                        "seed {seed} W{}: uncovered blank tile ${tile:02X} at ({r},{c})",
                        wi + 1,
                    );
                }
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
        let catalog = super::super::node_catalog::NodeCatalog::build(&rom, false);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, true);

        for seed in [42u64, 123, 999] {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let build = super::super::overworld_build::build(&rom, &pickup, &catalog, &mut rng);

            let mut out = rom.clone();

            // Apply QoL patches that the builder expects.
            super::super::qol::fix_w3_drawbridges(&mut out);
            super::super::qol::remove_w2_rock(&mut out);
            super::super::qol::remove_w3_boat_rock(&mut out);
            super::super::qol::fix_big_q_block_rooms(&mut out);

            write_overworld(&mut out, &build, &pickup, &catalog, &mut rng, true);

            let filename = format!("writer_test_seed{seed}.nes");
            std::fs::write(&filename, &out.data).unwrap();
            eprintln!("Wrote {filename}");
        }
    }
}
