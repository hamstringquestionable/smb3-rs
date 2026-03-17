/// Phase 2 of the overworld builder rewrite: Clear/Pick-up.
///
/// Consumes a `NodeCatalog` (Phase 1) and produces cleared grids plus a shuffle
/// pool of level-like entries. No RNG, no ROM writes — purely deterministic.
///
/// Steps per world:
/// 1. Read the tile grid from ROM.
/// 2. Pre-open vanilla FX gap tiles (making the grid fully connected).
/// 3. Collect level-like catalog entries into the shuffle pool.
/// 4. Blank their grid positions with theme-appropriate node tiles.

use crate::rom::Rom;

use super::node_catalog::{CatalogEntry, NodeCatalog, NodeKind};
use super::rom_data::{self, Grid, VALID_HORZ, VALID_VERT};

// ---------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------

/// A node picked up from the grid, ready for the shuffle pool.
///
/// References a `CatalogEntry` by index for immutable data (kind, name, tile,
/// level_entry). Carries mutable routing fields that the Build phase can update
/// during cross-world redistribution.
#[derive(Clone, Debug)]
pub(crate) struct PoolEntry {
    /// Index into `NodeCatalog.entries`.
    pub catalog_idx: usize,
    /// Current destination world (may change during redistribution).
    #[allow(dead_code)] // read in tests
    pub world_idx: usize,
    /// Current pointer table slot in the destination world.
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

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Execute Phase 2: read grids, open FX gaps, collect the shuffle pool, blank
/// picked-up positions.
pub(crate) fn pick_up(rom: &Rom, catalog: &NodeCatalog) -> PickupResult {
    pick_up_filtered(rom, catalog, |entry| {
        // Airships and Bowser stay at vanilla pointer table entries.
        // The autoscroll patch targets their hardcoded entry_idx offsets,
        // and blanking their grid positions would create extra build-phase
        // slots without matching available_slots in the writer.
        if matches!(entry.kind, NodeKind::Airship | NodeKind::Bowser) {
            return false;
        }
        // Level, Fortress, Pipe — shufflable gameplay nodes
        if entry.kind.is_level_like() {
            return true;
        }
        // HammerBro — roaming encounters that guard real levels
        matches!(entry.kind, NodeKind::HammerBro)
    })
}

/// Like `pick_up`, but only collects entries whose `CatalogEntry` satisfies `pred`.
pub(super) fn pick_up_filtered(
    rom: &Rom,
    catalog: &NodeCatalog,
    pred: fn(&CatalogEntry) -> bool,
) -> PickupResult {
    let mut pool: Vec<PoolEntry> = Vec::new();
    let mut worlds = Vec::with_capacity(8);

    for wi in 0..8 {
        worlds.push(pick_up_world(rom, catalog, wi, &mut pool, pred));
    }

    PickupResult { worlds, pool }
}

// ---------------------------------------------------------------------------
// Per-world pick-up
// ---------------------------------------------------------------------------

fn pick_up_world(
    rom: &Rom,
    catalog: &NodeCatalog,
    world_idx: usize,
    pool: &mut Vec<PoolEntry>,
    pred: fn(&CatalogEntry) -> bool,
) -> ClearedWorld {
    let mut grid = rom_data::read_tile_grid(rom, world_idx);

    // Pre-open all vanilla FX gap tiles so the grid is fully connected.
    open_fx_gaps(rom, &mut grid, world_idx);

    let mut pickup_positions = Vec::new();
    let mut pool_indices = Vec::new();

    for (ci, entry) in catalog.entries.iter().enumerate() {
        if entry.world_idx != world_idx || !pred(entry) {
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

/// Per-screen theme tiles: `(h_tile, v_tile, hv_tile, none_tile)`.
///
/// Different map regions use visually distinct node tiles to match the world
/// theme. Derived from the vanilla ROM's existing hammer-bro / blank-node
/// tile usage patterns.
///
/// Index = world_idx, inner index = screen number.
const SCREEN_THEMES: &[&[(u8, u8, u8, u8)]] = &[
    // W1: 1 screen
    &[(0x47, 0x48, 0x47, 0x44)],
    // W2: 2 screens
    &[(0x47, 0x48, 0x4A, 0x44), (0x47, 0x48, 0x4A, 0x44)],
    // W3: 3 screens (island theme on screen 0)
    &[(0x47, 0xB5, 0x4A, 0x44), (0x47, 0x48, 0x47, 0x44), (0x47, 0x48, 0x4A, 0x44)],
    // W4: 2 screens (island theme on screen 0)
    &[(0x47, 0xB5, 0x4A, 0x44), (0x47, 0x48, 0x4A, 0x44)],
    // W5: 2 screens (sky theme on screen 1)
    &[(0x47, 0x48, 0x44, 0x44), (0xDC, 0xD9, 0xDE, 0xD9)],
    // W6: 3 screens
    &[(0x47, 0x48, 0x4A, 0x44), (0x47, 0x48, 0x4A, 0x44), (0x47, 0x48, 0x4A, 0x44)],
    // W7: 2 screens
    &[(0x47, 0x44, 0x47, 0x44), (0x47, 0x48, 0x47, 0x44)],
    // W8: 4 screens
    &[
        (0x47, 0x44, 0x4A, 0x44),
        (0x47, 0x44, 0x47, 0x44),
        (0x47, 0x48, 0x4A, 0x44),
        (0x47, 0x48, 0x4A, 0x44),
    ],
];

/// Position-specific overrides for cases where the per-screen theme table
/// does not match the vanilla map's visual style (e.g. one-off decorative
/// tiles, direction ambiguity at junctions).
/// `(world_idx, row, col, tile)`
const BLANK_TILE_OVERRIDES: &[(usize, usize, usize, u8)] = &[
    (0, 0,  4, 0x44), (0, 8,  4, 0x48), (0, 8,  8, 0x4A),
    (1, 0,  8, 0x47), (1, 2, 12, 0x48), (1, 4,  8, 0x48),
    (1, 4, 18, 0x44), (1, 6,  8, 0x47),
    (2, 2, 12, 0x48), (2, 2, 16, 0x48), (2, 2, 20, 0x47), (2, 2, 22, 0x4A),
    (2, 4,  8, 0x4A), (2, 4, 20, 0xAE), (2, 6, 12, 0xB5),
    (3, 2, 18, 0x44), (3, 2, 28, 0x47), (3, 4,  8, 0x44),
    (3, 6,  2, 0x48), (3, 6, 20, 0xAF), (3, 6, 28, 0x4A),
    (4, 0,  4, 0x47), (4, 4,  4, 0x47), (4, 6, 24, 0xDC),
    (4, 8, 18, 0xD9),
    (5, 0, 22, 0x44), (5, 2, 14, 0x47), (5, 2, 32, 0x44),
    (5, 4, 24, 0x48), (5, 4, 28, 0x48), (5, 4, 34, 0x44),
    (5, 6, 28, 0x47),
    (6, 1, 15, 0x44), (6, 1, 22, 0x44), (6, 3,  2, 0x48), (6, 3,  6, 0x48),
    (6, 3, 11, 0xB6), (6, 3, 26, 0x4A), (6, 5,  3, 0x44),
    (6, 5, 10, 0xAE), (6, 5, 22, 0xB6), (6, 7,  3, 0x48),
    (6, 7,  8, 0x48), (6, 7, 14, 0x48), (6, 7, 24, 0x44),
    (7, 3, 18, 0x44), (7, 3, 24, 0x44), (7, 3, 34, 0x44),
    (7, 5,  8, 0xAF), (7, 5, 36, 0x48), (7, 5, 50, 0x44),
    (7, 7, 34, 0x48),
];

/// All valid blank/path node tiles. If a position already has one of these,
/// `blank_tile_for` leaves it unchanged.
const VALID_BLANK_TILES: &[u8] = &[
    0x44, 0x47, 0x48, 0x4A, 0x4B, // standard
    0xAE, 0xAF, 0xB5, 0xB6,       // island
    0xD9, 0xDC, 0xDE,             // sky
];

/// Pick the right blank node tile based on neighboring path directions and
/// the world/screen visual theme. If the tile is already a valid blank, it
/// is returned unchanged to preserve the vanilla path connectivity.
pub(super) fn blank_tile_for(grid: &Grid, world_idx: usize, row: usize, col: usize) -> u8 {
    // If the tile is already a valid blank, keep it as-is.
    let current = grid.get(row, col);
    if VALID_BLANK_TILES.contains(&current) {
        return current;
    }

    // Check position-specific overrides first.
    if let Some(&(_, _, _, tile)) = BLANK_TILE_OVERRIDES
        .iter()
        .find(|&&(w, r, c, _)| w == world_idx && r == row && c == col)
    {
        return tile;
    }

    blank_tile_from_neighbors(grid, world_idx, row, col)
}

/// Like `blank_tile_for` but skips position overrides. Used for dynamic
/// positions (e.g. W8 army sprites) that aren't at vanilla fixed spots.
pub(super) fn blank_tile_for_dynamic(grid: &Grid, world_idx: usize, row: usize, col: usize) -> u8 {
    blank_tile_from_neighbors(grid, world_idx, row, col)
}

fn blank_tile_from_neighbors(grid: &Grid, world_idx: usize, row: usize, col: usize) -> u8 {
    let has_h = col > 0 && VALID_HORZ.contains(&grid.get(row, col - 1));
    let has_v = row > 0 && VALID_VERT.contains(&grid.get(row - 1, col));

    let screen = col / 16;
    let (h, v, hv, none) = SCREEN_THEMES[world_idx][screen];

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
fn open_fx_gaps(rom: &Rom, grid: &mut Grid, world_idx: usize) {
    let fx_slots = rom_data::read_fx_slots(rom);
    let fx_assignments = rom_data::read_world_fx_assignments(rom);

    for &slot_idx in &fx_assignments[world_idx] {
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
        let catalog = NodeCatalog::build(&rom);
        let result = pick_up(&rom, &catalog);

        // 62 levels + 17 fortresses + 48 pipes + 154 hammer bros = 281
        // (Airships and Bowser excluded — their pointer table entries stay vanilla
        // so the autoscroll patch's hardcoded offsets remain valid.)
        // (166 HammerBro catalog entries minus 12 with non-level pointers like toad house/bonus game)
        assert_eq!(result.pool.len(), 281, "pool should have 281 entries (level-like + real hammer bros, no airship/bowser)");
    }

    #[test]
    fn test_blanked_positions() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = NodeCatalog::build(&rom);
        let result = pick_up(&rom, &catalog);

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
        let catalog = NodeCatalog::build(&rom);
        let result = pick_up(&rom, &catalog);

        let fx_slots = rom_data::read_fx_slots(&rom);
        let fx_assignments = rom_data::read_world_fx_assignments(&rom);

        for wi in 0..8 {
            let world_fx = &fx_assignments[wi];
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
        let catalog = NodeCatalog::build(&rom);
        let result = pick_up(&rom, &catalog);

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
        let catalog = NodeCatalog::build(&rom);
        let result = pick_up(&rom, &catalog);

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
        let catalog = NodeCatalog::build(&rom);
        let result = pick_up(&rom, &catalog);

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
    fn dump_filtered_rom(rom: &Rom, catalog: &NodeCatalog, pred: fn(&CatalogEntry) -> bool, filename: &str) {
        let result = pick_up_filtered(rom, catalog, pred);
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
    fn test_dump_cleared_roms() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = NodeCatalog::build(&rom);

        dump_filtered_rom(&rom, &catalog, |e| e.kind.is_level_like(), "cleared_all.nes");
        dump_filtered_rom(&rom, &catalog, |e| matches!(e.kind, NodeKind::Level), "cleared_levels.nes");
        dump_filtered_rom(&rom, &catalog, |e| matches!(e.kind, NodeKind::Fortress { .. }), "cleared_fortresses.nes");
        dump_filtered_rom(&rom, &catalog, |e| matches!(e.kind, NodeKind::Pipe { .. }), "cleared_pipes.nes");
    }
}
