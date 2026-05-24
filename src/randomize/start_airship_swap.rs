//! Per-world Start ↔ Airship swap.
//!
//! Flips a coin per W1-W7 (W8 has no airship sprite). On heads, Mario's start
//! tile (0xE5) and the airship/objective tile (0xC9) trade places. The catalog
//! is the source of truth — `pick_swaps` mutates the affected entries'
//! `grid_pos` values so the rest of the overworld pipeline (build, writer) sees
//! the swapped positions naturally.
//!
//! The engine-side scaffolding (per-world camera + Mario-position tables, the
//! two helper routines, Map_Init JSR patches, and Map_Object slot-1 sprite
//! moves) is committed once at the tail of the writer via
//! `write_engine_scaffolding`.
//!
//! Background and POC derivation: see `docs/start_airship_swap_findings.md`.

use rand::Rng;

use crate::rom::Rom;

use super::node_catalog::{NodeCatalog, NodeKind};
use super::pipe_helpers::grid_pos_to_dest_nibbles;
use super::rom_data::{
    self, AIRSHIP_OBJ_SLOT, FS_SAS_SCRH_TABLE, FS_SAS_SCRL_TABLE, FS_SAS_X_HELPER,
    FS_SAS_X_TABLE, FS_SAS_XHI_HELPER, FS_SAS_XHI_TABLE, Grid, MAP_INIT_SCROLL_SITE,
    MAP_INIT_X_LOW_SITE, MAP_Y_STARTS_OFF, WORLDS,
};

// ---------------------------------------------------------------------------
// Phase 1: catalog mutation
// ---------------------------------------------------------------------------

/// Per W1-W7 (W8 skipped), flip a coin. On heads, swap that world's Start
/// and Airship entry `grid_pos` values in the catalog, and record the swap on
/// `catalog.start_airship_swapped[wi]`. Downstream phases pick up the new
/// positions automatically.
pub(crate) fn pick_swaps<R: Rng>(catalog: &mut NodeCatalog, rng: &mut R) {
    for wi in 0..7 {
        if !rng.random::<bool>() {
            continue;
        }
        let start_idx = catalog
            .entries
            .iter()
            .position(|e| e.world_idx == wi && matches!(e.kind, NodeKind::Start));
        let air_idx = catalog
            .entries
            .iter()
            .position(|e| e.world_idx == wi && matches!(e.kind, NodeKind::Airship));
        if let (Some(si), Some(ai)) = (start_idx, air_idx) {
            let s_pos = catalog.entries[si].grid_pos;
            let a_pos = catalog.entries[ai].grid_pos;
            catalog.entries[si].grid_pos = a_pos;
            catalog.entries[ai].grid_pos = s_pos;
            catalog.start_airship_swapped[wi] = true;
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 2: per-world grid tweaks (called from the build phase)
// ---------------------------------------------------------------------------

/// For a swapped world, swap the tiles directly ABOVE the original airship and
/// original start positions. The airship sprite is two tiles tall, so without
/// this the visible top half stays anchored above the vanilla airship
/// location. After mutation, `catalog.airship.grid_pos` is the OLD start
/// position and `catalog.start.grid_pos` is the OLD airship position — we read
/// the rows directly above each to perform the matching tile-above swap.
///
/// The base tile bytes (0xC9 / 0xE5) are already handled by the build phase's
/// grid restore loop, since we also restore `NodeKind::Start` there now.
pub(super) fn swap_tiles_above(grid: &mut Grid, world_idx: usize, catalog: &NodeCatalog) {
    if !catalog.start_airship_swapped[world_idx] {
        return;
    }
    // After `pick_swaps`, the Airship entry holds the OLD start position and
    // the Start entry holds the OLD airship position.
    let Some(airship_entry) = catalog
        .world(world_idx)
        .find(|e| matches!(e.kind, NodeKind::Airship))
    else { return };
    let Some(start_entry) = catalog
        .world(world_idx)
        .find(|e| matches!(e.kind, NodeKind::Start))
    else { return };
    let (sr, sc) = airship_entry.grid_pos;
    let (ar, ac) = start_entry.grid_pos;
    if sr == 0 || ar == 0 {
        return;
    }
    // The tile directly above the vanilla airship (`0xC8`, the castle's top
    // half) always travels with the airship — that's the "two tiles required
    // for the castle." The tile above the vanilla START is a single-tile
    // backdrop and for W4/W5/W7 it happens to be a water square. Carried
    // verbatim to the new start position, that water square dangles above
    // the relocated start tile in the middle of land/sky, which looks wrong.
    // Substitute a per-world background tile in those cases.
    let above_old_airship = grid.get(ar - 1, ac);
    let above_for_new_start = match above_start_override(world_idx) {
        Some(t) => t,
        None => grid.get(sr - 1, sc),
    };
    grid.set(ar - 1, ac, above_for_new_start);
    grid.set(sr - 1, sc, above_old_airship);
}

/// World-specific replacement for the tile that ends up directly above the
/// relocated start position. Worlds where the vanilla above-start tile is
/// water override to a generic land/sky blank.
fn above_start_override(world_idx: usize) -> Option<u8> {
    match world_idx {
        3 | 6 => Some(0x42), // W4 / W7 — land path blank
        4 => Some(0xD7),     // W5 — sky blank
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Phase 3a: per-world pointer-table entry updates (called from the writer
// before pipe_helpers::resort_pointer_table)
// ---------------------------------------------------------------------------

/// For a swapped world, rewrite the Airship and Start entries' rowtype/scrcol
/// bytes so their pointer-table positions match the new grid coordinates. The
/// writer's main pass deliberately leaves these two entries alone (they're not
/// in the pickup pool), so without this step the engine looks up the wrong
/// entry when the player walks onto the swapped tiles.
///
/// Must run BEFORE `pipe_helpers::resort_pointer_table` for the same world so
/// the resort places the rewritten entries in their correct screen/row order.
pub(super) fn write_swapped_world_entries(rom: &mut Rom, world_idx: usize, catalog: &NodeCatalog) {
    if !catalog.start_airship_swapped[world_idx] {
        return;
    }
    let world = &WORLDS[world_idx];
    let rt_off = world.rowtype_offset;
    let sc_off = rt_off + world.entry_count;

    for entry in catalog
        .world(world_idx)
        .filter(|e| matches!(e.kind, NodeKind::Airship | NodeKind::Start))
    {
        let (row, col) = entry.grid_pos;
        let (screen, col_in_screen, row_nib) = grid_pos_to_dest_nibbles(row, col);

        // Preserve the low nibble of the existing rowtype byte (tileset code).
        let old_rt = rom.read_byte(rt_off + entry.entry_idx);
        let new_rt = (row_nib << 4) | (old_rt & 0x0F);
        rom.write_byte(rt_off + entry.entry_idx, new_rt);
        rom.write_byte(sc_off + entry.entry_idx, (screen << 4) | col_in_screen);
    }
}

// ---------------------------------------------------------------------------
// Phase 3b: engine scaffolding (called from the writer)
// ---------------------------------------------------------------------------

/// Write the engine-side scaffolding so the SMB3 engine spawns Mario at the
/// catalog's Start position per world and the airship sprite at the catalog's
/// Airship position. Always runs when the option is enabled — unswapped
/// worlds get vanilla-equivalent values, so the patches are no-ops there.
///
/// Writes:
///   * `Map_Y_Starts` (vanilla 8-byte table at `MAP_Y_STARTS_OFF`)
///   * Four per-world tables in PRG031 free space (X, XHi, ScrL, ScrH)
///   * Two helper subroutines (X-low setter, XHi + camera-scroll setter)
///   * `Map_Init` inline patches replaced with `JSR helper`
///   * Per-swapped-world `Map_Object` slot-1 sprite position move
pub(crate) fn write_engine_scaffolding(rom: &mut Rom, catalog: &NodeCatalog) {
    let mut y_tbl = [0u8; 8];
    let mut x_tbl = [0u8; 8];
    let mut xhi_tbl = [0u8; 8];
    let mut scrl_tbl = [0u8; 8];
    let mut scrh_tbl = [0u8; 8];

    for wi in 0..8 {
        let Some(start_entry) = catalog
            .world(wi)
            .find(|e| matches!(e.kind, NodeKind::Start))
        else { continue };
        let (sr, sc) = start_entry.grid_pos;
        y_tbl[wi] = ((sr as u8) * 0x10).wrapping_add(0x20);
        x_tbl[wi] = ((sc % 16) as u8) * 0x10;
        xhi_tbl[wi] = (sc / 16) as u8;
        // Page-aligned camera: $0722 = 0 (viewport at left of loaded slice),
        // $0724 = Mario's screen index so Scroll_Update_Ranges loads cols
        // (screen*16)..(screen*16+15).
        scrl_tbl[wi] = 0;
        scrh_tbl[wi] = (sc / 16) as u8;
    }

    rom.set_tag("start_airship_swap/tables");
    rom.write_range(MAP_Y_STARTS_OFF, &y_tbl);
    rom.write_range(FS_SAS_X_TABLE, &x_tbl);
    rom.write_range(FS_SAS_XHI_TABLE, &xhi_tbl);
    rom.write_range(FS_SAS_SCRL_TABLE, &scrl_tbl);
    rom.write_range(FS_SAS_SCRH_TABLE, &scrh_tbl);

    rom.set_tag("start_airship_swap/helpers");
    let x_tbl_cpu = file_to_prg031_cpu(FS_SAS_X_TABLE);
    let x_helper = [
        0xB9, (x_tbl_cpu & 0xFF) as u8, (x_tbl_cpu >> 8) as u8, // LDA Map_X_Starts,Y
        0x9D, 0x7A, 0x79,                                       // STA Map_Entered_X,X
        0x9D, 0x82, 0x79,                                       // STA $7982,X (mirror)
        0x60,                                                   // RTS
    ];
    rom.write_range(FS_SAS_X_HELPER, &x_helper);

    let xhi_tbl_cpu = file_to_prg031_cpu(FS_SAS_XHI_TABLE);
    let scrl_tbl_cpu = file_to_prg031_cpu(FS_SAS_SCRL_TABLE);
    let scrh_tbl_cpu = file_to_prg031_cpu(FS_SAS_SCRH_TABLE);
    let xhi_helper = [
        0xB9, (xhi_tbl_cpu & 0xFF) as u8, (xhi_tbl_cpu >> 8) as u8,   // LDA Map_XHi_Starts,Y
        0x9D, 0x78, 0x79,                                             // STA Map_Entered_XHi,X
        0xB9, (scrl_tbl_cpu & 0xFF) as u8, (scrl_tbl_cpu >> 8) as u8, // LDA Map_ScrL_Starts,Y
        0x9D, 0x22, 0x07,                                             // STA Map_Prev_XOff,X ($0722)
        0xB9, (scrh_tbl_cpu & 0xFF) as u8, (scrh_tbl_cpu >> 8) as u8, // LDA Map_ScrH_Starts,Y
        0x9D, 0x24, 0x07,                                             // STA Map_Prev_XHi,X  ($0724)
        0x60,                                                         // RTS
    ];
    rom.write_range(FS_SAS_XHI_HELPER, &xhi_helper);

    rom.set_tag("start_airship_swap/map_init");
    let x_helper_cpu = file_to_prg031_cpu(FS_SAS_X_HELPER);
    let xhi_helper_cpu = file_to_prg031_cpu(FS_SAS_XHI_HELPER);
    // Replace the 8-byte inline X-low immediate-store block with `JSR helper`
    // followed by five NOPs to keep the surrounding flow intact.
    rom.write_range(
        MAP_INIT_X_LOW_SITE,
        &[
            0x20, (x_helper_cpu & 0xFF) as u8, (x_helper_cpu >> 8) as u8,
            0xEA, 0xEA, 0xEA, 0xEA, 0xEA,
        ],
    );
    // Replace `STA $0724,X` (the final store before DEX) with `JSR xhi_helper`.
    // The helper writes $7978/$0722/$0724 as its tail, so its values win against
    // the inline zero-store at `$0722` (left intact a few cycles earlier).
    rom.write_range(
        MAP_INIT_SCROLL_SITE,
        &[0x20, (xhi_helper_cpu & 0xFF) as u8, (xhi_helper_cpu >> 8) as u8],
    );

    rom.set_tag("start_airship_swap/slot1");
    for wi in 0..7 {
        if !catalog.start_airship_swapped[wi] {
            continue;
        }
        if let Some(air_entry) = catalog
            .world(wi)
            .find(|e| matches!(e.kind, NodeKind::Airship))
        {
            let (r, c) = air_entry.grid_pos;
            rom_data::write_map_sprite_position(rom, wi, AIRSHIP_OBJ_SLOT, r, c);
        }
    }
}

/// PRG031 file offset → CPU address. PRG031 is always mapped at $E000.
fn file_to_prg031_cpu(file_off: usize) -> u16 {
    (0xE000 + (file_off - 0x3E010)) as u16
}
