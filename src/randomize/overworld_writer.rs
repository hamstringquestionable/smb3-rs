/// Overworld writer: pure ROM patching for overworld placement decisions.
///
/// This module handles the mechanical ROM writes that apply placement decisions
/// made by `overworld_builder`. It has no RNG and no decision logic — just
/// explicit data in, ROM bytes out.

use crate::rom::Rom;

use super::overworld_builder::{Placement, PlacedWorld, TileKind};
use super::overworld_helpers;
use super::pipe_helpers;
use super::rom_data::{
    self, FX_MAP_COMP_IDX, FX_PATTERNS, FX_VADDR_H, FX_VADDR_L,
    Grid, MAP_COMPLETE_BITS, WORLDS,
};

// ---------------------------------------------------------------------------
// Write: apply a PlacedWorld to ROM
// ---------------------------------------------------------------------------

/// Write a placed world to ROM: tile grid, level entries, and positions.
pub(super) fn write_world(rom: &mut Rom, placed: &PlacedWorld) {
    let world_idx = placed.world_idx;
    write_tile_grid(rom, world_idx, &placed.grid);

    let world = &WORLDS[world_idx];
    let n = world.entry_count;
    let rt = world.rowtype_offset;
    let sc = rt + n;

    for p in &placed.placements {
        let level_entry = p.tile.level_entry.as_ref()
            .expect("placed tile must have level_entry");
        rom_data::write_entry(rom, world, p.tile.entry_idx, level_entry);

        // Write position into pointer table so entry sits at placement.pos
        let (row, col) = p.pos;
        let row_nib = (row + 2) as u8;
        let screen = (col / 16) as u8;
        let col_in_screen = (col % 16) as u8;
        let brt = rom.read_byte(rt + p.tile.entry_idx);
        rom.write_byte(rt + p.tile.entry_idx, (row_nib << 4) | (brt & 0x0F));
        rom.write_byte(sc + p.tile.entry_idx, (screen << 4) | col_in_screen);
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
// Write: fortress FX
// ---------------------------------------------------------------------------

/// Write fortress-specific FX data for a placed world.
pub(super) fn write_fortress_fx(
    rom: &mut Rom,
    placed: &PlacedWorld,
    fx_slot_base: usize,
) {
    let world_idx = placed.world_idx;

    let fortress_placements: Vec<&Placement> = placed.placements
        .iter()
        .filter(|p| matches!(p.tile.kind, TileKind::Fortress { .. }) && p.lock_pos.is_some())
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
        let (ob_row, ob_col) = p.lock_pos.unwrap();
        let replace_tile = p.lock_replace_tile.unwrap();

        // Patch Boom-Boom Y-byte
        if let TileKind::Fortress { boomboom_y_offset } = &p.tile.kind {
            let old_y = rom.read_byte(*boomboom_y_offset);
            let new_y = (ordinal << 4) | (old_y & 0x0F);
            rom.write_byte(*boomboom_y_offset, new_y);
        }

        let patterns = overworld_helpers::fx_patterns_for(replace_tile);

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
// Write: pipe destinations
// ---------------------------------------------------------------------------

/// Write pipe destination tables and re-sort the pointer table.
///
/// Positions are already written by `write_world` — this only updates the
/// pipe destination tables (MapXHi/MapX/MapY/MapScrlXHi) and re-sorts.
pub(super) fn write_pipe_destinations(
    rom: &mut Rom,
    placed: &PlacedWorld,
) {
    let world_idx = placed.world_idx;

    let pipe_placements: Vec<&Placement> = placed.placements
        .iter()
        .filter(|p| matches!(p.tile.kind, TileKind::Pipe { .. }))
        .collect();

    // Update destination tables for each pipe pair
    for pair in pipe_placements.chunks(2) {
        if pair.len() < 2 {
            break;
        }
        if let TileKind::Pipe { dest_idx } = &pair[0].tile.kind {
            pipe_helpers::write_pipe_dest(rom, *dest_idx, pair[0].pos, pair[1].pos);
        }
    }

    // Re-sort pointer table and sync map object positions
    pipe_helpers::resort_pointer_table(rom, world_idx);
    rom_data::sync_map_object_positions(rom, world_idx);
}
