/// Overworld writer: pure ROM patching for overworld placement decisions.
///
/// This module handles the mechanical ROM writes that apply placement decisions
/// made by `overworld_builder`. It has no RNG and no decision logic — just
/// explicit data in, ROM bytes out.

use std::collections::HashMap;

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

/// Write a placed world to ROM: tile grid + level entries.
pub(super) fn write_world(rom: &mut Rom, placed: &PlacedWorld) {
    let world_idx = placed.world_idx;
    write_tile_grid(rom, world_idx, &placed.grid);

    let world = &WORLDS[world_idx];
    for p in &placed.placements {
        let level_entry = p.tile.level_entry.as_ref()
            .expect("placed tile must have level_entry");
        rom_data::write_entry(rom, world, p.tile.entry_idx, level_entry);
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
// Write: pipe placements
// ---------------------------------------------------------------------------

/// Write pipe placement changes to ROM.
pub(super) fn write_pipe_placements(
    rom: &mut Rom,
    placed: &PlacedWorld,
) {
    let world_idx = placed.world_idx;

    let pipe_placements: Vec<&Placement> = placed.placements
        .iter()
        .filter(|p| matches!(p.tile.kind, TileKind::Pipe { .. }))
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

        let entry_idx_a = pa.tile.entry_idx;
        let entry_idx_b = pb.tile.entry_idx;
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
        if let TileKind::Pipe { dest_idx } = &pa.tile.kind {
            pipe_helpers::write_pipe_dest(rom, *dest_idx, new_a_pos, new_b_pos);
        }
    }

    // Re-sort pointer table and sync map object positions
    pipe_helpers::resort_pointer_table(rom, world_idx);
    rom_data::sync_map_object_positions(rom, world_idx);
}
