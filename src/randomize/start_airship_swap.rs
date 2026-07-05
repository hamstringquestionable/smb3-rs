//! Per-world Start ↔ Airship swap.
//!
//! Flips a coin per W1-W7. W8 has no airship sprite to move. On heads, Mario's
//! start tile (0xE5) and the airship/objective tile (0xC9) trade places. The
//! catalog is the source of truth — `pick_swaps` mutates the affected entries'
//! `grid_pos` values so the rest of the overworld pipeline (build, writer)
//! sees the swapped positions naturally.
//!
//! W5 is the one world whose vanilla start is approached from *above* (path
//! tile at (5, 2)), so the usual castle-top relocation would stamp the
//! non-walkable `$C8` over that path cell and sever the only approach to the
//! new airship. For W5 only, `swap_tiles_above` skips the castle-top write
//! and leaves the path tile intact — the relocated airship loses its
//! decorative top half, but the world stays playable.
//!
//! The engine-side scaffolding (per-world camera + Mario-position tables, a
//! Map_Init seed helper and a game-over finalize helper, their JSR patches, and
//! Map_Object slot-1 sprite moves) is committed once at the tail of the writer
//! via `write_engine_scaffolding`.
//!
//! Background and POC derivation: see `docs/start_airship_swap_findings.md`.

use rand::Rng;

use crate::rom::Rom;

use super::node_catalog::{NodeCatalog, NodeKind};
use super::pipe_helpers::grid_pos_to_dest_nibbles;
use super::rom_data::{
    self, AIRSHIP_OBJ_SLOT, FS_SAS_GAMEOVER_FINALIZE, FS_SAS_SCRH_TABLE, FS_SAS_SCRL_TABLE,
    FS_SAS_SEED_HELPER, FS_SAS_X_TABLE, FS_SAS_XHI_TABLE, GAMEOVER_FINALIZE_SITE, Grid,
    MAP_INIT_SCROLL_SITE, MAP_TILE_GRIDS, MAP_Y_STARTS_OFF, WORLDS,
};

// ---------------------------------------------------------------------------
// Phase 1: catalog mutation
// ---------------------------------------------------------------------------

/// Per W1-W7 (W8 skipped — no airship sprite), flip a coin. On heads, swap
/// that world's Start and Airship entry `grid_pos` values in the catalog, and
/// record the swap on `catalog.start_airship_swapped[wi]`. Downstream phases
/// pick up the new positions automatically.
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
    // half) normally travels with the airship — the "two tiles required for
    // the castle." For W5 the new airship lands at the vanilla start `(6, 2)`
    // and the cell above it is a walkable path tile; stamping `$C8` there
    // would block the only approach. Skip the castle-top write for W5 and
    // accept the cosmetic loss (relocated airship is just the bottom half).
    //
    // The tile above the vanilla START is a single-tile backdrop and for
    // W4/W5/W7 it happens to be a water square. Carried verbatim to the new
    // start position, that water square dangles above the relocated start
    // tile in the middle of land/sky, which looks wrong. Substitute a
    // per-world background tile in those cases.
    let above_old_airship = grid.get(ar - 1, ac);
    let above_for_new_start = match above_start_override(world_idx) {
        Some(t) => t,
        None => grid.get(sr - 1, sc),
    };
    grid.set(ar - 1, ac, above_for_new_start);
    if world_idx != 4 {
        grid.set(sr - 1, sc, above_old_airship);
    }
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
///   * One PRG031 Map_Init seed helper (position + primary/secondary scroll backups)
///     plus a PRG011 game-over finalize helper (stamps World_Map_X/XHi + scroll)
///   * `Map_Init` scroll-store replaced with `JSR seed_helper`
///   * `GameOver_TwirlToStart` finalize store replaced with `JSR finalize` so
///     the spiral lands on the per-world start instead of vanilla column 2
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
        // Camera framing. Most worlds smooth-scroll and rest cleanly on half-page
        // stops (Scroll_ColumnL a multiple of 8): frame Mario half a screen back
        // (page-aligning would pin him at the left auto-pan margin and show the far
        // edge of his page). W5 (Sky) is the exception — it presents two *static*
        // screens (ground / sky) with no smooth scroll, so any non-page-aligned
        // scroll is invalid; snap to the start's whole screen instead. (W8 is the
        // other edge-scroll-skipped world but is never swapped, so its col-2 start
        // collapses to 0 regardless.) Page-0 / unswapped starts collapse to column 0
        // in either branch (identical to vanilla).
        let cols = MAP_TILE_GRIDS[wi].columns as i32;
        let col = if wi == 4 {
            (sc as i32 / 16) * 16 // W5: page-align to the start's static screen
        } else {
            (((sc as i32 - 8) + 4).max(0) / 8) * 8 // round-to-8, floored at 0
        };
        let col = col.clamp(0, (cols - 16).max(0));
        scrl_tbl[wi] = if col & 8 != 0 { 0x80 } else { 0x00 };
        scrh_tbl[wi] = (col >> 4) as u8;
    }

    rom.set_tag("start_airship_swap/tables");
    rom.write_range(MAP_Y_STARTS_OFF, &y_tbl);
    rom.write_range(FS_SAS_X_TABLE, &x_tbl);
    rom.write_range(FS_SAS_XHI_TABLE, &xhi_tbl);
    rom.write_range(FS_SAS_SCRL_TABLE, &scrl_tbl);
    rom.write_range(FS_SAS_SCRH_TABLE, &scrh_tbl);

    rom.set_tag("start_airship_swap/helper");
    let x_tbl_cpu = file_to_prg031_cpu(FS_SAS_X_TABLE);
    let xhi_tbl_cpu = file_to_prg031_cpu(FS_SAS_XHI_TABLE);
    let scrl_tbl_cpu = file_to_prg031_cpu(FS_SAS_SCRL_TABLE);
    let scrh_tbl_cpu = file_to_prg031_cpu(FS_SAS_SCRH_TABLE);
    // Single Map_Init seed subroutine (Y = World_Num, X = Player index — both live
    // at the loop's scroll-store hook). Overwrites Mario's start position and BOTH
    // scroll backups from the four tables:
    //   X   -> Map_Entered_X ($797A) + Map_Previous_X ($7982)
    //   XHi -> Map_Entered_XHi ($7978) + Map_Previous_XHi ($7980)
    //   ScrL-> Map_Prev_XOff ($0722) + Map_Prev_XOff2 ($7986)
    //   ScrH-> Map_Prev_XHi  ($0724) + Map_Prev_XHi2  ($7988)
    // The secondary backups ($7986/$7988) are what the death-with-lives "skid from
    // afar" restores the camera from; leaving them at the vanilla page-0 value
    // strands Mario off-page after dying in a swapped world. The finalize helper
    // below mirrors this exact store order for the game-over path.
    let seed_helper = [
        0xB9, (x_tbl_cpu & 0xFF) as u8, (x_tbl_cpu >> 8) as u8,       // LDA X_TABLE,Y
        0x9D, 0x7A, 0x79,                                            // STA Map_Entered_X,X   ($797A)
        0x9D, 0x82, 0x79,                                            // STA Map_Previous_X,X  ($7982)
        0xB9, (xhi_tbl_cpu & 0xFF) as u8, (xhi_tbl_cpu >> 8) as u8,   // LDA XHI_TABLE,Y
        0x9D, 0x78, 0x79,                                            // STA Map_Entered_XHi,X  ($7978)
        0x9D, 0x80, 0x79,                                            // STA Map_Previous_XHi,X ($7980)
        0xB9, (scrl_tbl_cpu & 0xFF) as u8, (scrl_tbl_cpu >> 8) as u8, // LDA SCRL_TABLE,Y
        0x9D, 0x22, 0x07,                                            // STA Map_Prev_XOff,X  ($0722)
        0x9D, 0x86, 0x79,                                            // STA Map_Prev_XOff2,X ($7986)
        0xB9, (scrh_tbl_cpu & 0xFF) as u8, (scrh_tbl_cpu >> 8) as u8, // LDA SCRH_TABLE,Y
        0x9D, 0x24, 0x07,                                            // STA Map_Prev_XHi,X  ($0724)
        0x9D, 0x88, 0x79,                                            // STA Map_Prev_XHi2,X ($7988)
        0x60,                                                        // RTS
    ];
    rom.write_range(FS_SAS_SEED_HELPER, &seed_helper);

    rom.set_tag("start_airship_swap/map_init");
    let seed_helper_cpu = file_to_prg031_cpu(FS_SAS_SEED_HELPER);
    // Replace the vanilla `STA $0724,X` (the last store before DEX) with
    // `JSR seed_helper`. The vanilla `LDA #$20 / STA $797A / STA $7982` X-low store
    // earlier in the same iteration is left intact — the helper re-stamps those
    // bytes here, so the table values win before any draw.
    rom.write_range(
        MAP_INIT_SCROLL_SITE,
        &[0x20, (seed_helper_cpu & 0xFF) as u8, (seed_helper_cpu >> 8) as u8],
    );

    // GameOver_TwirlToStart spirals Mario back to the start via a per-frame X/Y
    // delta, then at finalize copies World_Map_X/XHi/Y into Map_Previous_*. The
    // delta is low-byte/within-screen only (and uses a second hardcoded column-2
    // for the skid *direction*), so it can't be retargeted to a swapped start on
    // a different column/screen by patching the delta. Instead we let the vanilla
    // animation play and STAMP the correct position at finalize.
    //
    // Hook: replace `STA Map_Prev_XHi2,X` (the last store before the World_Map →
    // Map_Previous copies; A = 0 there) with `JSR finalize`. The helper overwrites
    // World_Map_X ($79,X) / World_Map_XHi ($77,X), the camera scroll ($0722/$0724,X)
    // and both secondary scroll backups ($7986/$7988,X) from the FS_SAS_* tables.
    // The vanilla copies that follow then propagate the corrected X/XHi into
    // Map_Previous_X/XHi, so the continue lands on the real start tile. Y is
    // World_Num (re-loaded in the helper); X is Player_Current (live at the site).
    // The displaced `STA $7988` (A = 0) is dropped: the helper stamps $7988 with
    // the start screen instead, which is exactly what the death-with-lives afar
    // skid later restores the camera page from. Nothing between the hook and the
    // vanilla copies reads $7988.
    //
    // It is NOT enough to fix only the per-player scroll backup ($0722/$0724,X).
    // The vanilla twirl-to-start assumes the start is at page-0 hard-left and
    // flies the camera there (GameOver_TwirlFromAfar scrolls Horz_Scroll down to
    // 0). So at the twirl landing the live scroll ZP `Horz_Scroll`/`Horz_Scroll_Hi`
    // ($FD/$12) still point at page 0, regardless of where the swapped start
    // actually is. The game-over continue path then copies the live scroll into
    // Map_Prev_XOff/XHi (PRG030_92B6) and re-enters the world (PRG030_8634),
    // which reloads Horz_Scroll FROM Map_Prev and does a full nametable redraw —
    // so a stale page-0 live scroll wins, drawing the map on page 0 while Mario
    // is placed on the real (≥1) start page → off-map softlock whenever the
    // Game Over happened on a different overworld page than the start tile.
    // Fix: also stamp the live scroll ZP `Horz_Scroll` ($FD) / `Horz_Scroll_Hi`
    // ($12) here (global, not per-player) so the subsequent re-enter redraws the
    // nametable on the start framing. SCRL/SCRH are the half-page framing scroll
    // (identical to the Map_Init seeds); for unswapped / page-0 worlds they are 0,
    // so these stores are no-ops. Store order mirrors `seed_helper`.
    rom.set_tag("start_airship_swap/gameover_finalize");
    let finalize_helper = [
        0xAC, 0x27, 0x07,                                            // LDY World_Num ($0727)
        0xB9, (x_tbl_cpu & 0xFF) as u8, (x_tbl_cpu >> 8) as u8,      // LDA FS_SAS_X_TABLE,Y
        0x95, 0x79,                                                  // STA World_Map_X,X   ($79,X)
        0xB9, (xhi_tbl_cpu & 0xFF) as u8, (xhi_tbl_cpu >> 8) as u8,  // LDA FS_SAS_XHI_TABLE,Y
        0x95, 0x77,                                                  // STA World_Map_XHi,X ($77,X)
        0xB9, (scrl_tbl_cpu & 0xFF) as u8, (scrl_tbl_cpu >> 8) as u8,// LDA FS_SAS_SCRL_TABLE,Y
        0x9D, 0x22, 0x07,                                            // STA Map_Prev_XOff,X  ($0722)
        0x85, 0xFD,                                                  // STA Horz_Scroll      ($FD, live scroll low)
        0x9D, 0x86, 0x79,                                            // STA Map_Prev_XOff2,X ($7986)
        0xB9, (scrh_tbl_cpu & 0xFF) as u8, (scrh_tbl_cpu >> 8) as u8,// LDA FS_SAS_SCRH_TABLE,Y
        0x9D, 0x24, 0x07,                                            // STA Map_Prev_XHi,X   ($0724)
        0x85, 0x12,                                                  // STA Horz_Scroll_Hi   ($12, live scroll page)
        0x9D, 0x88, 0x79,                                            // STA Map_Prev_XHi2,X  ($7988)
        0x60,                                                        // RTS
    ];
    rom.write_range(FS_SAS_GAMEOVER_FINALIZE, &finalize_helper);
    let finalize_cpu = file_to_prg011_cpu(FS_SAS_GAMEOVER_FINALIZE);
    rom.write_range(
        GAMEOVER_FINALIZE_SITE,
        &[0x20, (finalize_cpu & 0xFF) as u8, (finalize_cpu >> 8) as u8],
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

/// PRG011 file offset → CPU address. PRG011 is mapped at $A000 during the map
/// (it owns Map_Init / GameOver_TwirlToStart), so a JSR from the game-over hook
/// to a helper in the same bank is bank-local.
fn file_to_prg011_cpu(file_off: usize) -> u16 {
    (0xA000 + (file_off - 0x16010)) as u16
}
