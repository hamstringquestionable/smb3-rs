//! World sources: places a `WorldState` can come from. All three are LOADERS
//! — they package existing data into v2's state and never redo the work of
//! the shared phases (catalog, pickup) they read from.
//!
//! - [`from_pickup`] is the v2 build input: one world as the shared pickup
//!   phase hands it over, ready for placement phases to run on.
//! - [`from_vanilla`] reads a world straight from the unmodified ROM. Vanilla
//!   is *known ground truth* — if a metric reads vanilla W1 and reports
//!   something we know is wrong, the metric is broken, not the world.
//! - [`from_built`] wraps the shipping builder's output — the baseline every
//!   v2 comparison is made against.

use super::*;

/// Wrap a shipping-builder world as a v2 `WorldState`. Start/target are
/// re-derived from the grid the same way the builder derived them. `fixed`
/// is empty: a finished world has nothing left to place.
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
        pipe_budget: VANILLA_PIPE_PAIRS[built.world_idx],
        log: Vec::new(),
    }
}

/// The v2 build entry point's input: one world as the shared pickup phase
/// hands it over — cleared grid (blank path tiles), with the start/airship/
/// Bowser tiles stamped back at their catalog positions (pickup blanks them,
/// but they are terrain, not placements).
///
/// Uses the SIMPLEST configuration while v2 finds its feet: no start↔airship
/// swap, no shuffled toad houses / hammer bros / spades — so pinned houses
/// and floating sprites land in `fixed`.
pub(crate) fn from_pickup(
    rom: &Rom,
    catalog: &NodeCatalog,
    pickup: &PickupResult,
    world_idx: usize,
) -> WorldState {
    let mut grid = pickup.worlds[world_idx].grid.clone();
    for entry in catalog.entries.iter().filter(|e| e.world_idx == world_idx) {
        if matches!(entry.kind, NodeKind::Start | NodeKind::Airship | NodeKind::Bowser) {
            grid.set(entry.grid_pos.0, entry.grid_pos.1, entry.tile);
        }
    }
    let start = rom_data::find_start(&grid);
    let target = find_target(&grid, world_idx);
    let fixed = fixed_positions_for_world(rom, catalog, world_idx, false, false);
    WorldState {
        world_idx,
        grid,
        slots: Vec::new(),
        locks: Vec::new(),
        pipe_pairs: Vec::new(),
        start,
        target,
        fixed,
        pipe_budget: VANILLA_PIPE_PAIRS[world_idx],
        log: Vec::new(),
    }
}

/// Read one vanilla world from the ROM: tiles from the map grid, slots and
/// pipe pairs from the node catalog, locks from the fortress-FX tables.
///
/// The grid is normalized to the builder convention the route scorer
/// expects: lock tiles are RESTORED to their open path tile on the grid, and
/// the closed state lives only in the `locks` overlay.
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
    WorldState {
        world_idx,
        grid,
        slots,
        locks,
        pipe_pairs: vanilla_pipe_pairs(catalog, world_idx),
        start,
        target,
        fixed: HashSet::new(),
        pipe_budget: VANILLA_PIPE_PAIRS[world_idx],
        log: Vec::new(),
    }
}

/// Vanilla slot list for one world, from the node catalog's classification.
///
/// Start / Airship / Bowser are not slots (they become `start` / `target`);
/// MapObject entries (e.g. W7's path-blocking piranha plants) have no slot
/// equivalent in the builder model and are skipped — a known fidelity gap the
/// vanilla measurements must be read with.
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
            blocks_target: false,
        });
    }
    locks
}
