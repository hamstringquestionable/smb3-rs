//! Phase 2 of the overworld builder rewrite: Clear/Pick-up.
//!
//! Consumes a `NodeCatalog` (Phase 1) and produces cleared grids plus a shuffle
//! pool of level-like entries. No RNG, no ROM writes — purely deterministic.
//!
//! Steps per world:
//! 1. Read the tile grid from ROM.
//! 2. Pre-open vanilla FX gap tiles (making the grid fully connected).
//! 3. Collect level-like catalog entries into the shuffle pool.
//! 4. Blank their grid positions with theme-appropriate node tiles.

use crate::rom::Rom;

use super::node_catalog::{CatalogEntry, NodeCatalog, NodeKind};
use super::rom_data::{self, FxSlot, Grid, VALID_BLANK_TILES, VALID_HORZ, VALID_VERT};

// ---------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------

/// A node picked up from the grid, ready for the shuffle pool.
///
/// References a `CatalogEntry` by index; other fields are snapshots of the
/// catalog entry's vanilla routing for convenience.
#[derive(Clone, Debug)]
pub(crate) struct PoolEntry {
    /// Index into `NodeCatalog.entries`.
    pub catalog_idx: usize,
    /// Vanilla world_idx (or `usize::MAX` for synthetic beta entries).
    #[allow(dead_code)] // read in tests
    pub world_idx: usize,
    /// Vanilla pointer table slot.
    pub entry_idx: usize,
}

/// One world's cleared grid plus tracking info for the Build phase.
#[derive(Clone)]
pub(crate) struct ClearedWorld {
    #[allow(dead_code)] // read in tests
    pub world_idx: usize,
    /// Grid with FX gaps pre-opened and pool entries blanked to `TILE_EMPTY_NODE`.
    pub grid: Grid,
    /// Vanilla grid positions of the entries that were picked up (parallel to `pool_indices`).
    #[allow(dead_code)] // read in tests
    pub pickup_positions: Vec<(usize, usize)>,
    /// Indices into `PickupResult.pool` for this world's picked-up entries.
    pub pool_indices: Vec<usize>,
}

/// Complete Phase 2 output: cleared grids + global shuffle pool.
pub(crate) struct PickupResult {
    /// Per-world cleared grids (indexed 0..8).
    pub worlds: Vec<ClearedWorld>,
    /// Global pool of all level-like entries across all worlds.
    pub pool: Vec<PoolEntry>,
}

/// Per-feature flags controlling which catalog entry kinds the pickup phase
/// pulls into the shuffle pool. Each flag corresponds to a pickup-time option;
/// when false, those entries stay at their vanilla positions.
#[derive(Copy, Clone)]
pub(crate) struct PickupFlags {
    pub shuffle_spade_games: bool,
    pub shuffle_toad_houses: bool,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Execute Phase 2: read grids, open FX gaps, collect the shuffle pool, blank
/// picked-up positions.
pub(crate) fn pick_up(rom: &Rom, catalog: &NodeCatalog, flags: PickupFlags) -> PickupResult {
    pick_up_filtered(rom, catalog, flags, default_pickup_pred)
}

/// Default pickup predicate: includes level-like nodes, hammer bros, and —
/// per flag — bonus games and toad houses. Airships, Bowser, and the W3 boat
/// dock tile are excluded.
fn default_pickup_pred(entry: &CatalogEntry, flags: PickupFlags) -> bool {
    // Airships and Bowser stay at vanilla pointer table entries — the
    // autoscroll patch targets their hardcoded entry_idx offsets, and
    // blanking their grid positions would create extra build-phase slots
    // without matching available_slots in the writer.
    if matches!(entry.kind, NodeKind::Airship | NodeKind::Bowser) {
        return false;
    }
    // W3 boat dock tile ($4B) must stay — the boat docks here, and
    // replacing the tile (e.g. with a pipe) breaks boat boarding.
    if entry.tile == 0x4B {
        return false;
    }
    if entry.kind.is_level_like() || matches!(entry.kind, NodeKind::HammerBro) {
        return true;
    }
    if flags.shuffle_spade_games && matches!(entry.kind, NodeKind::BonusGame) {
        return true;
    }
    // ToadHouse pickup preserves each entry's vanilla obj_ptr (one of 7
    // reward variants), so the reward identity stays attached.
    if flags.shuffle_toad_houses && matches!(entry.kind, NodeKind::ToadHouse) {
        return true;
    }
    false
}

/// Like `pick_up`, but only collects entries whose `CatalogEntry` satisfies `pred`.
pub(super) fn pick_up_filtered(
    rom: &Rom,
    catalog: &NodeCatalog,
    flags: PickupFlags,
    pred: fn(&CatalogEntry, PickupFlags) -> bool,
) -> PickupResult {
    let mut pool: Vec<PoolEntry> = Vec::new();
    let mut worlds = Vec::with_capacity(8);

    let fx_slots = rom_data::read_fx_slots(rom);
    let fx_assignments = rom_data::read_world_fx_assignments(rom);

    for (wi, world_fx) in fx_assignments.iter().enumerate() {
        worlds.push(pick_up_world(
            rom, catalog, wi, &mut pool, flags, pred, &fx_slots, world_fx,
        ));
    }

    // Synthetic beta entries (world_idx == usize::MAX) have no vanilla grid
    // cell to blank. Push them directly into the pool so they're available
    // for the build phase to place on any world's map.
    for (ci, entry) in catalog.entries.iter().enumerate() {
        if entry.world_idx != usize::MAX {
            continue;
        }
        if !pred(entry, flags) {
            continue;
        }
        pool.push(PoolEntry {
            catalog_idx: ci,
            world_idx: usize::MAX,
            entry_idx: usize::MAX,
        });
    }

    PickupResult { worlds, pool }
}

// ---------------------------------------------------------------------------
// Per-world pick-up
// ---------------------------------------------------------------------------

// Reason: only `(fx_slots, world_fx)` bundle naturally, and at 2 fields a
// struct adds more noise than it removes. The remaining args are
// individually meaningful inputs to per-world pickup.
#[allow(clippy::too_many_arguments)]
fn pick_up_world(
    rom: &Rom,
    catalog: &NodeCatalog,
    world_idx: usize,
    pool: &mut Vec<PoolEntry>,
    flags: PickupFlags,
    pred: fn(&CatalogEntry, PickupFlags) -> bool,
    fx_slots: &[FxSlot],
    world_fx: &[u8],
) -> ClearedWorld {
    let mut grid = rom_data::read_tile_grid(rom, world_idx);

    // Pre-open all vanilla FX gap tiles so the grid is fully connected.
    open_fx_gaps(&mut grid, fx_slots, world_fx);

    let mut pickup_positions = Vec::new();
    let mut pool_indices = Vec::new();

    for (ci, entry) in catalog.entries.iter().enumerate() {
        if entry.world_idx != world_idx || !pred(entry, flags) {
            continue;
        }

        let (row, col) = entry.grid_pos;
        let pool_idx = pool.len();

        pool.push(PoolEntry {
            catalog_idx: ci,
            world_idx: entry.world_idx,
            entry_idx: entry.entry_idx,
        });

        pickup_positions.push((row, col));
        pool_indices.push(pool_idx);

        if row < grid.rows && col < grid.cols {
            grid.set(row, col, blank_tile_for(&grid, world_idx, row, col));
        }
    }

    ClearedWorld {
        world_idx,
        grid,
        pickup_positions,
        pool_indices,
    }
}

// ---------------------------------------------------------------------------
// Blank tile selection
// ---------------------------------------------------------------------------

/// Position-specific overrides where the neighbor-based heuristic picks the
/// wrong tile. `(world_idx, row, col, tile)`
const BLANK_TILE_OVERRIDES: &[(usize, usize, usize, u8)] = &[
    (2, 8, 6, 0x47), // W3 spade near start — heuristic picks 0x44 (no neighbors), needs 0x47 for BFS
    (4, 6, 20, 0xD9), // W5 spade in sky region — neighbors are non-path sky bg, heuristic falls to land
];

/// Positions that should use island-themed blank tiles (0xAE/0xAF/0xB5/0xB6).
/// All other positions default to standard land tiles (0x44-0x4A), except sky
/// positions which are auto-detected from neighbors (0xD* tiles).
const ISLAND_POSITIONS: &[(usize, usize, usize)] = &[
    // W3 — narrow island strips
    (2, 2, 4),  // 3-2
    (2, 4, 4),  // hammer
    (2, 6, 4),  // 3-1
    (2, 4, 12), // hammer
    (2, 6, 12), // 3-5
    (3, 6, 20), // W4 4-4
    (6, 5, 10), // W7 7-4
];

/// Blank-tile theme tuple: `(horiz, vert, both, none)` covering the four
/// combinations of valid-path neighbors.
const THEME_STANDARD: (u8, u8, u8, u8) = (0x47, 0x48, 0x4A, 0x44);
const THEME_SKY: (u8, u8, u8, u8) = (0xDC, 0xDD, 0xDE, 0xD9);
const THEME_ISLAND: (u8, u8, u8, u8) = (0xAE, 0xB5, 0xAF, 0xB6);

/// Pick the right blank node tile based on neighboring path directions and
/// the world/screen visual theme. If the tile is already a valid blank, it
/// is returned unchanged to preserve the vanilla path connectivity.
pub(super) fn blank_tile_for(grid: &Grid, world_idx: usize, row: usize, col: usize) -> u8 {
    let current = grid.get(row, col);
    if VALID_BLANK_TILES.contains(&current) {
        return current;
    }

    if let Some(&(_, _, _, tile)) = BLANK_TILE_OVERRIDES
        .iter()
        .find(|&&(w, r, c, _)| w == world_idx && r == row && c == col)
    {
        return tile;
    }

    blank_tile_from_neighbors(grid, world_idx, row, col)
}

/// Pick a blank tile purely from neighbor analysis, ignoring per-position
/// overrides. Used for dynamic positions (e.g. W8 army sprites) that aren't at
/// vanilla fixed spots.
pub(super) fn blank_tile_from_neighbors(grid: &Grid, world_idx: usize, row: usize, col: usize) -> u8 {
    let h_tile = if col > 0 { Some(grid.get(row, col - 1)) } else { None };
    let v_tile = if row > 0 { Some(grid.get(row - 1, col)) } else { None };

    let has_h = h_tile.is_some_and(|t| VALID_HORZ.contains(&t));
    let has_v = v_tile.is_some_and(|t| VALID_VERT.contains(&t));

    let force_island = ISLAND_POSITIONS.contains(&(world_idx, row, col));
    let neighbor = has_h.then(|| h_tile.unwrap()).or_else(|| has_v.then(|| v_tile.unwrap()));

    let (h, v, hv, none) = if force_island {
        THEME_ISLAND
    } else {
        match neighbor.map(|t| t >> 4) {
            Some(0xD) => THEME_SKY,
            _ => THEME_STANDARD,
        }
    };

    match (has_h, has_v) {
        (true, true) => hv,
        (true, false) => h,
        (false, true) => v,
        (false, false) => none,
    }
}

// ---------------------------------------------------------------------------
// FX gap opener
// ---------------------------------------------------------------------------

/// Replace vanilla FX gap tiles with their underlying path tiles, making the
/// grid fully connected before placement.
fn open_fx_gaps(grid: &mut Grid, fx_slots: &[FxSlot], world_fx: &[u8]) {
    for &slot_idx in world_fx {
        let slot = &fx_slots[slot_idx as usize];
        if slot.grid_row < grid.rows && slot.grid_col < grid.cols {
            grid.set(slot.grid_row, slot.grid_col, slot.replace_tile);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn load_rom() -> Option<Rom> {
        let data = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&data).ok()
    }

    #[test]
    fn test_pool_count() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = NodeCatalog::build(&rom, false);
        let result = pick_up(&rom, &catalog, PickupFlags { shuffle_spade_games: true, shuffle_toad_houses: true });

        // 62 levels + 17 fortresses + 48 pipes + 151 hammer bros + 19 bonus games + 22 toad houses = 319
        // (Airships and Bowser excluded — their pointer table entries stay vanilla
        // so the autoscroll patch's hardcoded offsets remain valid.)
        // (166 HammerBro catalog entries minus 12 with non-level pointers like toad house/bonus game)
        // (3 W3 HammerBro entries on tile $4B (boat dock) excluded — tile must stay for boat boarding)
        // (19 BonusGame entries picked up when shuffle_spade_games is true)
        // (22 ToadHouse entries picked up when shuffle_toad_houses is true)
        assert_eq!(result.pool.len(), 319, "pool should have 319 entries (level-like + hammer bros + bonus games + toad houses, no airship/bowser/boat-dock)");
    }

    #[test]
    fn test_blanked_positions() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = NodeCatalog::build(&rom, false);
        let result = pick_up(&rom, &catalog, PickupFlags { shuffle_spade_games: true, shuffle_toad_houses: true });

        for (pi, pe) in result.pool.iter().enumerate() {
            let entry = &catalog.entries[pe.catalog_idx];
            let (row, col) = entry.grid_pos;
            let cw = &result.worlds[entry.world_idx];

            if row < cw.grid.rows && col < cw.grid.cols {
                let tile = cw.grid.get(row, col);
                let valid_blank = VALID_BLANK_TILES.contains(&tile);
                assert!(
                    valid_blank,
                    "pool[{pi}] ({}) at ({row},{col}) should be blanked, got ${tile:02X}",
                    entry.name,
                );
            }
        }
    }

    #[test]
    fn test_no_fx_gaps_remain() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = NodeCatalog::build(&rom, false);
        let result = pick_up(&rom, &catalog, PickupFlags { shuffle_spade_games: true, shuffle_toad_houses: true });

        let fx_slots = rom_data::read_fx_slots(&rom);
        let fx_assignments = rom_data::read_world_fx_assignments(&rom);

        for (wi, world_fx) in fx_assignments.iter().enumerate() {
            let grid = &result.worlds[wi].grid;

            for (si, slot) in fx_slots.iter().enumerate() {
                if !world_fx.contains(&(si as u8)) {
                    continue;
                }
                if slot.grid_row < grid.rows && slot.grid_col < grid.cols {
                    let tile = grid.get(slot.grid_row, slot.grid_col);
                    assert!(
                        tile != 0x54 && tile != 0x56 && tile != 0x9D && tile != 0xE4,
                        "W{} FX slot {si} at ({},{}) still has gap tile ${tile:02X}",
                        wi + 1, slot.grid_row, slot.grid_col,
                    );
                }
            }
        }
    }

    #[test]
    fn test_start_tiles_preserved() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = NodeCatalog::build(&rom, false);
        let result = pick_up(&rom, &catalog, PickupFlags { shuffle_spade_games: true, shuffle_toad_houses: true });

        for entry in &catalog.entries {
            if !matches!(entry.kind, NodeKind::Start) {
                continue;
            }
            let (row, col) = entry.grid_pos;
            let cw = &result.worlds[entry.world_idx];
            assert_eq!(
                cw.grid.get(row, col),
                rom_data::TILE_START,
                "W{} start at ({row},{col}) should be preserved",
                entry.world_idx + 1,
            );
        }
    }

    #[test]
    fn test_pool_indices_consistent() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = NodeCatalog::build(&rom, false);
        let result = pick_up(&rom, &catalog, PickupFlags { shuffle_spade_games: true, shuffle_toad_houses: true });

        for cw in &result.worlds {
            for &pi in &cw.pool_indices {
                let pe = &result.pool[pi];
                assert_eq!(
                    pe.world_idx, cw.world_idx,
                    "pool[{pi}] world_idx {} != ClearedWorld {}",
                    pe.world_idx, cw.world_idx,
                );
            }
            // pool_indices and pickup_positions should be parallel
            assert_eq!(
                cw.pool_indices.len(),
                cw.pickup_positions.len(),
                "W{}: pool_indices and pickup_positions length mismatch",
                cw.world_idx + 1,
            );
        }
    }

    #[test]
    #[ignore]
    fn test_print_pickup() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = NodeCatalog::build(&rom, false);
        let result = pick_up(&rom, &catalog, PickupFlags { shuffle_spade_games: true, shuffle_toad_houses: true });

        for cw in &result.worlds {
            eprintln!("\n=== World {} ({} picked up) ===", cw.world_idx + 1, cw.pool_indices.len());
            for (i, &pi) in cw.pool_indices.iter().enumerate() {
                let pe = &result.pool[pi];
                let entry = &catalog.entries[pe.catalog_idx];
                let (r, c) = cw.pickup_positions[i];
                eprintln!(
                    "  [{:2}] {:<8} ({},{})  tile=${:02X}  {:?}",
                    entry.entry_idx, entry.name, r, c, entry.tile, entry.kind,
                );
            }
        }

        eprintln!("\n=== Pool Summary ===");
        let mut counts = std::collections::HashMap::new();
        for pe in &result.pool {
            let entry = &catalog.entries[pe.catalog_idx];
            let label = match &entry.kind {
                NodeKind::Level => "Level",
                NodeKind::Fortress { .. } => "Fortress",
                NodeKind::Pipe { .. } => "Pipe",
                NodeKind::Airship => "Airship",
                NodeKind::Bowser => "Bowser",
                _ => "Other",
            };
            *counts.entry(label).or_insert(0usize) += 1;
        }
        for (kind, count) in &counts {
            eprintln!("  {kind:<12} {count}");
        }
        eprintln!("  Total:       {}", result.pool.len());
    }

    /// Helper: write cleared grids into a ROM copy and save to disk.
    fn dump_filtered_rom(rom: &Rom, catalog: &NodeCatalog, pred: fn(&CatalogEntry, PickupFlags) -> bool, filename: &str) {
        let result = pick_up_filtered(rom, catalog, PickupFlags { shuffle_spade_games: true, shuffle_toad_houses: true }, pred);
        let mut data = rom.data.clone();
        for cw in &result.worlds {
            for r in 0..cw.grid.rows {
                for c in 0..cw.grid.cols {
                    let offset = rom_data::map_tile_offset(cw.world_idx, r, c);
                    data[offset] = cw.grid.get(r, c);
                }
            }
        }
        std::fs::write(filename, &data).unwrap();
        eprintln!("Wrote {filename} ({} bytes, {} picked up)", data.len(), result.pool.len());
    }

    #[test]
    #[ignore]
    fn test_compare_overrides_vs_heuristic() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };

        let fx_slots = rom_data::read_fx_slots(&rom);
        let fx_assignments = rom_data::read_world_fx_assignments(&rom);
        let mut mismatches = 0;
        for &(wi, row, col, override_tile) in BLANK_TILE_OVERRIDES {
            let mut grid = rom_data::read_tile_grid(&rom, wi);
            open_fx_gaps(&mut grid, &fx_slots, &fx_assignments[wi]);

            let heuristic_tile = blank_tile_from_neighbors(&grid, wi, row, col);
            let vanilla_tile = grid.get(row, col);

            if override_tile == heuristic_tile {
                eprintln!(
                    "  SAME  W{} ({},{})  override=${:02X}  heuristic=${:02X}  vanilla=${:02X}",
                    wi + 1, row, col, override_tile, heuristic_tile, vanilla_tile,
                );
            } else {
                eprintln!(
                    "  DIFF  W{} ({},{})  override=${:02X}  heuristic=${:02X}  vanilla=${:02X}",
                    wi + 1, row, col, override_tile, heuristic_tile, vanilla_tile,
                );
                mismatches += 1;
            }
        }
        eprintln!("\n{} overrides, {} differ from heuristic", BLANK_TILE_OVERRIDES.len(), mismatches);
    }

    #[test]
    fn test_w5_spade_pickup_uses_sky_blanks() {
        // After pickup, W5 spade positions in the sky region should get
        // sky-palette blanks (not standard land 0x44). (4, 30) hits the V
        // case via 0xE8 in VALID_VERT; (6, 20) has no path neighbors at all
        // so an override pins it to the sky "none" tile.
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = NodeCatalog::build(&rom, false);
        let result = pick_up(&rom, &catalog, PickupFlags {
            shuffle_spade_games: true,
            shuffle_toad_houses: true,
        });
        let w5 = &result.worlds[4];
        assert_eq!(w5.grid.get(6, 20), 0xD9, "W5 (6,20) override should produce sky none-tile");
        assert_eq!(w5.grid.get(4, 30), 0xDD, "W5 (4,30) should produce sky v-tile via 0xE8 in VALID_VERT");
    }

    #[test]
    #[ignore]
    fn test_dump_cleared_roms() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = NodeCatalog::build(&rom, false);

        dump_filtered_rom(&rom, &catalog, |e, _| e.kind.is_level_like(), "cleared_all.nes");
        dump_filtered_rom(&rom, &catalog, |e, _| matches!(e.kind, NodeKind::Level), "cleared_levels.nes");
        dump_filtered_rom(&rom, &catalog, |e, _| matches!(e.kind, NodeKind::Fortress { .. }), "cleared_fortresses.nes");
        dump_filtered_rom(&rom, &catalog, |e, _| matches!(e.kind, NodeKind::Pipe { .. }), "cleared_pipes.nes");
    }
}
