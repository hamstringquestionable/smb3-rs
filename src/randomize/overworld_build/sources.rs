//! World sources: places a `WorldState` can come from. All three are LOADERS
//! — they package existing data into the builder's state and never redo the work of
//! the shared phases (catalog, pickup) they read from.
//!
//! - [`from_pickup`] is the build input: one world as the shared pickup
//!   phase hands it over, ready for placement phases to run on.
//! - [`from_vanilla`] reads a world straight from the unmodified ROM. Vanilla
//!   is *known ground truth* — if a metric reads vanilla W1 and reports
//!   something we know is wrong, the metric is broken, not the world.
//! - [`from_built`] wraps the shipping builder's output — the baseline every
//!   old-vs-new comparisons were made against.

use super::*;

/// Roll the per-seed level/fort/floor allotment EXACTLY like the shipping builder,
/// by calling its machinery verbatim: fortresses via
/// `redistribute_fortresses` (W8 always keeps 4; W1-W7 roll 1-3 each with
/// the 13-fort total conserved), then levels via `distribute_levels` over
/// `prepare_capacities` (capacity^0.5-weighted shares of the 62 vanilla
/// levels, random leftover topping up random worlds), then the C1 floors via
/// `deal_c1_floors` (a permuted multiset summing to 8 x `C1_FLOOR`). One roll
/// covers all 8 worlds — the caller writes the result onto each
/// `WorldState`'s budgets before scheduling. `from_pickup` alone stays vanilla-frozen; variance is
/// the caller's choice, made here.
pub(crate) fn allot_budgets(
    rom: &Rom,
    catalog: &NodeCatalog,
    pickup: &PickupResult,
    flags: &BuildFlags,
    mut rng: &mut dyn RngCore,
) -> ([usize; 8], [usize; 8], [u32; 8]) {
    let fort_counts = redistribute_fortresses(&mut rng);
    let CapacityPrep { capacities, .. } = prepare_capacities(
        rom,
        catalog,
        pickup,
        &fort_counts,
        flags.eights_are_wild,
        flags.shuffle_toad_houses,
        flags.shuffle_hammer_bros,
    );
    let level_counts =
        distribute_levels(&capacities, VANILLA_LEVEL_COUNT, LEVEL_SPREAD_EXPONENT, &mut rng);
    let c1_floors = deal_c1_floors(&mut rng);
    (level_counts, fort_counts, c1_floors)
}

/// Wrap a finished `BuiltWorld` back into a `WorldState`. Start/target are
/// re-derived from the grid the same way the builder derived them. `fixed`
/// is empty: a finished world has nothing left to place.
#[cfg(test)]
pub(crate) fn from_built(built: &BuiltWorld) -> WorldState {
    WorldState {
        world_idx: built.world_idx,
        grid: built.grid.clone(),
        slots: built.slots.clone(),
        locks: built.locks.clone(),
        pipe_pairs: built.pipe_pairs.clone(),
        start: rom_data::find_start(&built.grid),
        target: find_target(&built.grid, built.world_idx),
        fixed: HashSet::new(),
        hammer_gated: HashSet::new(),
        pipe_budget: VANILLA_PIPE_PAIRS[built.world_idx],
        level_budget: built
            .slots
            .iter()
            .filter(|s| s.kind == SlotKind::Level)
            .count(),
        fort_budget: built
            .slots
            .iter()
            .filter(|s| s.kind == SlotKind::Fortress)
            .count(),
        c1_floor: C1_FLOOR,
        ptr_slots: 0,
        hb_sprite_pins: Vec::new(),
        log: Vec::new(),
    }
}

/// The build entry point's input: one world as the shared pickup phase
/// hands it over — cleared grid (blank path tiles), with the start/airship/
/// Bowser tiles stamped back at their catalog positions (pickup blanks them,
/// but they are terrain, not placements).
///
/// `flags` mirrors the build configuration: toad-house / hammer-bro shuffle
/// decide whether pinned houses and floating sprites stay `fixed`, and
/// `eights_are_wild` is stamped onto the grid so the walker sees the W8
/// canoe edges. The caller owns consistency: the catalog must already be
/// SAS-swapped when testing start↔airship swap (pick_swaps runs before
/// pickup, as in the real pipeline), and `pickup` must have been built with
/// PickupFlags matching `flags`.
pub(crate) fn from_pickup(
    rom: &Rom,
    catalog: &NodeCatalog,
    pickup: &PickupResult,
    world_idx: usize,
    flags: &BuildFlags,
) -> WorldState {
    let mut grid = pickup.worlds[world_idx].grid.clone();
    grid.eights_are_wild = flags.eights_are_wild;
    for entry in catalog.entries.iter().filter(|e| e.world_idx == world_idx) {
        if matches!(entry.kind, NodeKind::Start | NodeKind::Airship | NodeKind::Bowser) {
            grid.set(entry.grid_pos.0, entry.grid_pos.1, entry.tile);
        }
    }
    // SAS-swapped worlds: the castle's top half ($C8) travels with the
    // relocated airship tile, and the cell above the relocated start gets a
    // plausible backdrop (W5 skips the castle-top write — the cell above its
    // vanilla start is the only approach path). Stamping happens here, on
    // the grid every phase builds on, so the non-blank $C8 also keeps
    // placement off that cell.
    crate::randomize::start_airship_swap::swap_tiles_above(&mut grid, world_idx, catalog);
    let start = rom_data::find_start(&grid);
    let target = find_target(&grid, world_idx);
    let fixed = fixed_positions_for_world(
        rom,
        catalog,
        world_idx,
        flags.shuffle_toad_houses,
        flags.shuffle_hammer_bros,
    );
    let level_budget = catalog
        .entries
        .iter()
        .filter(|e| e.world_idx == world_idx && matches!(e.kind, NodeKind::Level))
        .count();
    let fort_budget = catalog
        .entries
        .iter()
        .filter(|e| e.world_idx == world_idx && matches!(e.kind, NodeKind::Fortress { .. }))
        .count();
    let hb_sprite_pins = if flags.shuffle_hammer_bros {
        Vec::new()
    } else {
        rom_data::read_hb_sprite_positions(rom, world_idx)
    };
    let hammer_gated: HashSet<Pos> = HAMMER_GATED_POCKETS
        .iter()
        .filter(|(wi, _)| *wi == world_idx)
        .map(|(_, pos)| *pos)
        .collect();
    WorldState {
        world_idx,
        grid,
        slots: Vec::new(),
        locks: Vec::new(),
        pipe_pairs: Vec::new(),
        start,
        target,
        fixed,
        hammer_gated,
        pipe_budget: VANILLA_PIPE_PAIRS[world_idx],
        level_budget,
        fort_budget,
        c1_floor: C1_FLOOR,
        ptr_slots: pickup.worlds[world_idx].pool_indices.len(),
        hb_sprite_pins,
        log: Vec::new(),
    }
}

/// Lone blanks sealed off by breakable hammer rocks: `(world_idx, position)`.
/// W3 (8,6) is the vanilla spade between two rocks — once the spade is picked
/// up, the cell is a one-blank island that connectivity would otherwise
/// bridge with a pipe every seed. (W6's rock at (2,13) guards a vanilla PIPE
/// at (2,14) — deliberately NOT listed, bridging it is vanilla-faithful.)
const HAMMER_GATED_POCKETS: &[(usize, Pos)] = &[
    (2, (8, 6)), // W3 spade pocket between two rocks near start
];

/// Read one vanilla world from the ROM: tiles from the map grid, slots and
/// pipe pairs from the node catalog, locks from the fortress-FX tables.
///
/// The grid is normalized to the builder convention the route scorer
/// expects: lock tiles are RESTORED to their open path tile on the grid, and
/// the closed state lives only in the `locks` overlay.
#[cfg(test)]
pub(crate) fn from_vanilla(rom: &Rom, catalog: &NodeCatalog, world_idx: usize) -> WorldState {
    let mut grid = rom_data::read_tile_grid(rom, world_idx);
    let slots = vanilla_slots(rom, catalog, world_idx);
    let fort_count = slots
        .iter()
        .filter(|s| s.kind == SlotKind::Fortress)
        .count();
    let locks = vanilla_locks(rom, &grid, world_idx, fort_count);
    for lock in &locks {
        grid.set(lock.pos.0, lock.pos.1, lock.replace_tile);
    }
    let start = rom_data::find_start(&grid);
    let target = find_target(&grid, world_idx);
    let level_budget = slots.iter().filter(|s| s.kind == SlotKind::Level).count();
    let fort_budget = slots.iter().filter(|s| s.kind == SlotKind::Fortress).count();
    WorldState {
        world_idx,
        grid,
        slots,
        locks,
        pipe_pairs: vanilla_pipe_pairs(catalog, world_idx),
        start,
        target,
        fixed: HashSet::new(),
        hammer_gated: HashSet::new(),
        pipe_budget: VANILLA_PIPE_PAIRS[world_idx],
        level_budget,
        fort_budget,
        c1_floor: C1_FLOOR,
        ptr_slots: 0,
        hb_sprite_pins: Vec::new(),
        log: Vec::new(),
    }
}

/// Vanilla slot list for one world, from the node catalog's classification.
///
/// Start / Airship / Bowser are not slots (they become `start` / `target`);
/// MapObject entries (e.g. W7's path-blocking piranha plants) have no slot
/// equivalent in the builder model and are skipped — a known fidelity gap the
/// vanilla measurements must be read with.
#[cfg(test)]
fn vanilla_slots(rom: &Rom, catalog: &NodeCatalog, world_idx: usize) -> Vec<SlotAssignment> {
    let mut slots = Vec::new();
    for entry in catalog.entries.iter().filter(|e| e.world_idx == world_idx) {
        let (kind, section) = match &entry.kind {
            NodeKind::Level => (SlotKind::Level, 0),
            NodeKind::Fortress { boomboom_y_offset } => {
                // The Boom-Boom Y-byte's upper nibble is the fortress's
                // 1-based FX ordinal within its world; sections are 0-based.
                let ordinal = (rom.read_byte(*boomboom_y_offset) >> 4) as usize;
                (SlotKind::Fortress, ordinal.saturating_sub(1))
            }
            NodeKind::Pipe { .. } => (SlotKind::Pipe, 0),
            NodeKind::ToadHouse => (SlotKind::ToadHouse, 0),
            NodeKind::BonusGame => (SlotKind::BonusGame, 0),
            NodeKind::HammerBro => (SlotKind::HammerBro, 0),
            NodeKind::Start | NodeKind::Airship | NodeKind::Bowser | NodeKind::MapObject => {
                continue;
            }
        };
        slots.push(SlotAssignment {
            pos: entry.grid_pos,
            kind,
            section,
            is_hand_trap: false,
            is_troll_pipe: false,
        });
    }
    slots
}

/// Vanilla teleport pipe pairs: catalog `Pipe` entries share a `dest_idx` per
/// pair; the A-side is the first element of the edge.
#[cfg(test)]
fn vanilla_pipe_pairs(catalog: &NodeCatalog, world_idx: usize) -> Vec<TeleportEdge> {
    let mut by_dest: HashMap<usize, (Option<Pos>, Option<Pos>)> = HashMap::new();
    for entry in catalog.entries.iter().filter(|e| e.world_idx == world_idx) {
        if let NodeKind::Pipe { dest_idx, is_a_side } = entry.kind {
            let pair = by_dest.entry(dest_idx).or_default();
            if is_a_side {
                pair.0 = Some(entry.grid_pos);
            } else {
                pair.1 = Some(entry.grid_pos);
            }
        }
    }
    let mut pairs: Vec<TeleportEdge> = by_dest
        .into_values()
        .filter_map(|(a, b)| Some((a?, b?)))
        .collect();
    // HashMap iteration order is arbitrary; sort for a deterministic state.
    pairs.sort();
    pairs
}

/// Vanilla lock overlay for one world, decoded from the fortress-FX tables.
///
/// The FX world table holds up to 4 FX slot numbers per world, in fortress
/// ordinal order — vanilla pairs every fortress with exactly one FX slot, so
/// the world's first `fort_count` entries are the live ones (a trailing 0x00
/// is "unused", but 0 is also W1's real first slot — counting via the
/// catalog's fortress count sidesteps the ambiguity). Position decode inverts
/// the writer (`fortress_fx.rs`): row byte = (row+2)<<4; loc byte =
/// (col_in_screen<<4) | screen.
#[cfg(test)]
fn vanilla_locks(rom: &Rom, grid: &Grid, world_idx: usize, fort_count: usize) -> Vec<LockAssignment> {
    let mut locks = Vec::new();
    for ordinal in 0..fort_count.min(4) {
        let slot = rom.read_byte(rom_data::FX_WORLD_TABLE + world_idx * 4 + ordinal) as usize;
        let row = (rom.read_byte(rom_data::FX_MAP_LOC_ROW + slot) >> 4) as usize - 2;
        let loc = rom.read_byte(rom_data::FX_MAP_LOC + slot);
        let col = (loc & 0x0F) as usize * 16 + (loc >> 4) as usize;
        locks.push(LockAssignment {
            pos: (row, col),
            // The vanilla grid carries the CLOSED tile; the caller restores
            // the open tile onto the grid after reading these.
            gap_tile: grid.get(row, col),
            replace_tile: rom.read_byte(rom_data::FX_MAP_TILE_REPLACE + slot),
            fort_section: ordinal,
            secret_exit_safe: false,
        });
    }
    locks
}
