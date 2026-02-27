/// Overworld helpers: low-level ROM write operations for fortress and lock placement.
///
/// This module handles the mechanical ROM writes for placing fortresses and their
/// associated locks/bridges on the overworld map. The randomization logic (choosing
/// WHERE fortresses and locks go) lives in `overworld.rs`; this module handles
/// HOW to write those decisions to the ROM.
///
/// Key responsibility: context-aware FX type selection. Given an obstacle position,
/// the helper reads the tile already there and derives the correct FX type (lock,
/// water bridge, sky bridge), pattern bytes, and gap tile automatically.

use crate::rom::Rom;

use super::rom_data::{
    self, FxSlot, LevelEntry, WORLDS,
    FX_WORLD_TABLE, FX_MAP_LOC_ROW, FX_MAP_LOC, FX_MAP_TILE_REPLACE,
};

// ---------------------------------------------------------------------------
// FX table ROM offsets (17 slots each)
// ---------------------------------------------------------------------------

const FX_VADDR_H: usize = 0x147CD;
const FX_VADDR_L: usize = 0x147DE;
const FX_MAP_COMP_IDX: usize = 0x147EF; // 17 x 2 bytes
const FX_PATTERNS: usize = 0x14811;     // 17 x 4 bytes

// ---------------------------------------------------------------------------
// Tile constants
// ---------------------------------------------------------------------------

/// Lock tile ID on the overworld map.
const TILE_LOCK: u8 = 0x54;

/// Gap tile IDs for different FX types.
const TILE_BRIDGE_GAP: u8 = 0x56;
const TILE_WATER_GAP: u8 = 0x9D;
const TILE_SKY_GAP: u8 = 0xE4;

/// Fortress map tile ID.
#[cfg(test)]
const TILE_FORTRESS: u8 = 0x67;

/// Airship dock tile ID.
const TILE_AIRSHIP: u8 = 0xC9;

/// Bowser's castle tile ID.
const TILE_BOWSER: u8 = 0xCC;

/// All path tiles that a lock/gap can be placed on.
pub(super) const LOCKABLE_TILES: &[u8] = &[
    0x45, // horizontal path
    0x46, // vertical path
    0xB3, // water bridge path
    0xDA, // sky bridge path
    0xAC, // horizontal path variant
    0xB7, // horizontal path variant
    0xB8, // horizontal path variant
    0xB9, // horizontal path variant
    0xE6, // horizontal path variant
    0xAA, // vertical path variant
    0xAB, // vertical path variant
    0xB0, // vertical path variant
    0xB1, // vertical drawbridge
    0xB2, // horizontal drawbridge
    0xDB, // vertical path variant
    0xBA, // vertical path variant
];

/// Map_Complete_Bits lookup table: maps grid row to completion bit.
/// Row 0 = $80, row 1 = $40, ..., row 7 = $01.
const MAP_COMPLETE_BITS: [u8; 8] = [0x80, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x01];

// ---------------------------------------------------------------------------
// FX type (private — the randomizer never sees this)
// ---------------------------------------------------------------------------

/// FX type determines the pattern bytes and gap tile used.
/// Derived automatically from the tile at the obstacle position.
///
/// Lock vs BridgeGap distinction is critical: the game's hardcoded
/// `Map_RemoveTo_Tiles` table maps $54 (lock) → $46 (vertical path) and
/// $56 (bridge gap) → $45 (horizontal path). Using the wrong gap tile
/// causes vertical/horizontal path corruption on map reload.
#[derive(Clone, Copy, Debug, PartialEq)]
#[allow(dead_code)]
enum FxType {
    Lock,        // FE C0 FE C0 — gap tile $54, replaced with $46 (vertical path)
    BridgeGap,   // FE FE E1 E1 — gap tile $56, replaced with $45 (horizontal path)
    WaterBridge, // D4 D6 D5 D7 — gap tile $9D
    SkyBridge,   // FE FE E1 E1 — gap tile $E4
}

impl FxType {
    fn gap_tile(self) -> u8 {
        match self {
            FxType::Lock => TILE_LOCK,
            FxType::BridgeGap => TILE_BRIDGE_GAP,
            FxType::WaterBridge => TILE_WATER_GAP,
            FxType::SkyBridge => TILE_SKY_GAP,
        }
    }
}

/// Determine the FX type and pattern bytes from the tile at the obstacle position.
/// Returns (FxType, pattern_bytes).
///
/// Vertical path tiles get FxType::Lock ($54 → $46 on reload).
/// Horizontal path tiles get FxType::BridgeGap ($56 → $45 on reload).
fn fx_type_for_tile(tile: u8) -> (FxType, [u8; 4]) {
    match tile {
        // Water bridge → water pattern
        0xB3 => (FxType::WaterBridge, [0xD4, 0xD6, 0xD5, 0xD7]),
        // Sky bridge → sky gap with horizontal pattern
        0xDA => (FxType::SkyBridge, [0xFE, 0xFE, 0xE1, 0xE1]),
        // Vertical path tiles → lock (gap $54, replaced with $46 vertical)
        0x46 | 0xAA | 0xAB | 0xB0 | 0xB1 | 0xDB | 0xBA =>
            (FxType::Lock, [0xFE, 0xC0, 0xFE, 0xC0]),
        // Horizontal path tiles → bridge gap (gap $56, replaced with $45 horizontal)
        _ => (FxType::BridgeGap, [0xFE, 0xFE, 0xE1, 0xE1]),
    }
}

// ---------------------------------------------------------------------------
// FX encoding helpers
// ---------------------------------------------------------------------------

/// Compute VRAM address for a map tile at (grid_row, col_in_screen).
fn fx_vram_addr(grid_row: usize, col_in_screen: usize) -> u16 {
    (0x2880 + grid_row * 64 + col_in_screen * 2) as u16
}

/// Encode FortressFX_MapLocation byte: upper nibble = column, lower nibble = screen.
fn fx_map_location(screen: usize, col_in_screen: usize) -> u8 {
    ((col_in_screen as u8) << 4) | (screen as u8)
}

/// Encode FortressFX_MapLocationRow byte: (grid_row + 2) << 4.
fn fx_map_location_row(grid_row: usize) -> u8 {
    ((grid_row + 2) as u8) << 4
}

/// Compute the Map_Completions (column, bit) pair for a position.
/// The game's Map_Complete_Bits LUT has 8 entries (rows 0-7);
/// row 8 is clamped to 7 as a safety measure.
fn fx_comp_idx(grid_row: usize, screen: usize, col_in_screen: usize) -> (u8, u8) {
    let col = (screen * 16 + col_in_screen) as u8;
    let bit = MAP_COMPLETE_BITS[grid_row.min(7)];
    (col, bit)
}

// ---------------------------------------------------------------------------
// Placement data structures (passed from randomizer)
// ---------------------------------------------------------------------------

/// A single fortress placement instruction from the randomizer.
pub(super) struct FortressPlacement {
    /// Level data (tileset, obj_ptr, lay_ptr)
    pub level_entry: LevelEntry,
    /// ROM offset for Boom-Boom Y-byte patching
    pub boomboom_y_offset: usize,
    /// Fortress map tile ($67, $EB, $AF, etc.)
    pub fort_tile: u8,

    /// Destination world index (0-based)
    pub dest_world: usize,
    /// Destination pointer table entry index
    pub dest_slot: usize,
    /// 1-based ordinal within destination world (for Y-byte and FX table)
    pub ordinal: u8,

    /// Grid position (row, col) of the fortress tile in destination world.
    /// Used for Map_Completions persistence (FortressFX_MapCompIdx).
    pub fortress_pos: (usize, usize),
    /// Grid position (row, col) for the obstacle (lock/gap) in destination world
    pub obstacle_pos: (usize, usize),
}

/// A displaced action level that needs to be written to a freed fortress slot.
pub(super) struct DisplacedLevel {
    pub level_entry: LevelEntry,
    pub tile: u8,
    pub dest_world: usize,
    pub dest_slot: usize,
}

// ---------------------------------------------------------------------------
// Core placement execution
// ---------------------------------------------------------------------------

/// Execute a batch of fortress placements for one world.
///
/// Writes level entries, fortress tiles, Boom-Boom Y-bytes, obstacle tiles,
/// FX slot data, and FX world table entries. Also handles displaced levels.
///
/// `fx_slot_base` is the first FX slot index to assign for this world's
/// fortresses. Slots are assigned sequentially: base, base+1, etc.
pub(super) fn execute_world_placements(
    rom: &mut Rom,
    placements: &[FortressPlacement],
    displaced: &[DisplacedLevel],
    fx_slot_base: usize,
) {
    let world_idx = if let Some(p) = placements.first() {
        p.dest_world
    } else {
        // No placements — just clear this world's FX entries and handle displaced
        for d in displaced {
            rom_data::write_entry(rom, &WORLDS[d.dest_world], d.dest_slot, &d.level_entry);
            set_entry_tile(rom, d.dest_world, d.dest_slot, d.tile);
        }
        return;
    };

    // Write FX world table entries for this world
    let fx_base = FX_WORLD_TABLE + world_idx * 4;
    for i in 0..4 {
        if i < placements.len() {
            rom.write_byte(fx_base + i, (fx_slot_base + i) as u8);
        } else {
            rom.write_byte(fx_base + i, 0x00);
        }
    }

    for (i, p) in placements.iter().enumerate() {
        let slot_idx = fx_slot_base + i;

        // 1. Write level entry to destination slot
        rom_data::write_entry(rom, &WORLDS[p.dest_world], p.dest_slot, &p.level_entry);

        // 2. Place fortress tile on the map
        set_entry_tile(rom, p.dest_world, p.dest_slot, p.fort_tile);

        // 3. Patch Boom-Boom Y-byte with new ordinal
        let old_y = rom.read_byte(p.boomboom_y_offset);
        let new_y = (p.ordinal << 4) | (old_y & 0x0F);
        rom.write_byte(p.boomboom_y_offset, new_y);

        // 4-6. Place obstacle and configure FX slot
        let (ob_row, ob_col) = p.obstacle_pos;
        let screen = ob_col / 16;
        let col_in_screen = ob_col % 16;

        // Read current tile to determine FX type
        let tile_offset = rom_data::map_tile_offset(p.dest_world, ob_row, ob_col);
        let original_tile = rom.read_byte(tile_offset);
        let (fx_type, patterns) = fx_type_for_tile(original_tile);

        // Place the obstacle tile (lock or gap)
        rom.write_byte(tile_offset, fx_type.gap_tile());

        // Write FX slot data
        let vram = fx_vram_addr(ob_row, col_in_screen);
        rom.write_byte(FX_VADDR_H + slot_idx, (vram >> 8) as u8);
        rom.write_byte(FX_VADDR_L + slot_idx, (vram & 0xFF) as u8);
        rom.write_byte(FX_MAP_LOC_ROW + slot_idx, fx_map_location_row(ob_row));
        rom.write_byte(FX_MAP_LOC + slot_idx, fx_map_location(screen, col_in_screen));
        rom.write_byte(FX_MAP_TILE_REPLACE + slot_idx, original_tile);

        // Map_Completions persistence — encodes the FORTRESS position, not the obstacle
        let (fort_row, fort_col) = p.fortress_pos;
        let fort_screen = fort_col / 16;
        let fort_col_in_screen = fort_col % 16;
        let (comp_col, comp_bit) = fx_comp_idx(fort_row, fort_screen, fort_col_in_screen);
        rom.write_byte(FX_MAP_COMP_IDX + slot_idx * 2, comp_col);
        rom.write_byte(FX_MAP_COMP_IDX + slot_idx * 2 + 1, comp_bit);

        // Pattern bytes
        let pat_off = FX_PATTERNS + slot_idx * 4;
        rom.write_byte(pat_off, patterns[0]);
        rom.write_byte(pat_off + 1, patterns[1]);
        rom.write_byte(pat_off + 2, patterns[2]);
        rom.write_byte(pat_off + 3, patterns[3]);
    }

    // Handle displaced levels — write them to their freed slots
    for d in displaced {
        rom_data::write_entry(rom, &WORLDS[d.dest_world], d.dest_slot, &d.level_entry);
        set_entry_tile(rom, d.dest_world, d.dest_slot, d.tile);
    }
}

// ---------------------------------------------------------------------------
// Pre-open existing locks
// ---------------------------------------------------------------------------

/// Pre-open all locks/bridges/gaps in a world by restoring replacement tiles
/// from the FX slot snapshot.
///
/// Must be called before placing new obstacles. Uses a snapshot taken before
/// any writes to avoid reading stale FX data.
pub(super) fn pre_open_fx_for_world(
    rom: &mut Rom,
    world_idx: usize,
    fx_slots_snapshot: &[FxSlot],
) {
    let grid = rom_data::read_tile_grid(rom, world_idx);

    for r in 0..grid.rows {
        for c in 0..grid.cols {
            let tile = grid.get(r, c);
            if tile != TILE_LOCK && tile != TILE_BRIDGE_GAP
                && tile != TILE_WATER_GAP && tile != TILE_SKY_GAP
            {
                continue;
            }
            if let Some(slot) = fx_slots_snapshot.iter().find(|s| s.grid_row == r && s.grid_col == c) {
                let offset = rom_data::map_tile_offset(world_idx, r, c);
                rom.write_byte(offset, slot.replace_tile);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Map tile helpers
// ---------------------------------------------------------------------------

/// Read the map tile at an entry's grid position.
pub(super) fn entry_tile(rom: &Rom, world_idx: usize, entry_idx: usize) -> u8 {
    let (row, col) = rom_data::entry_grid_position(rom, &WORLDS[world_idx], entry_idx);
    let off = rom_data::map_tile_offset(world_idx, row, col);
    rom.read_byte(off)
}

/// Write a map tile at an entry's grid position.
pub(super) fn set_entry_tile(rom: &mut Rom, world_idx: usize, entry_idx: usize, tile: u8) {
    let (row, col) = rom_data::entry_grid_position(rom, &WORLDS[world_idx], entry_idx);
    let off = rom_data::map_tile_offset(world_idx, row, col);
    rom.write_byte(off, tile);
}

// ---------------------------------------------------------------------------
// Read-only scan helpers
// ---------------------------------------------------------------------------

/// Get the airship or Bowser's castle grid position for a world.
/// Scans the map tile grid for the target tile.
pub(super) fn world_target_position(rom: &Rom, world_idx: usize) -> Option<(usize, usize)> {
    let grid = rom_data::read_tile_grid(rom, world_idx);
    let target_tile = if world_idx == 7 { TILE_BOWSER } else { TILE_AIRSHIP };
    for r in 0..grid.rows {
        for c in 0..grid.cols {
            if grid.get(r, c) == target_tile {
                return Some((r, c));
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rom::Rom;

    /// POC: Two fortresses in W1 — slot 0 = lock (existing), slot 1 = water bridge.
    #[test]
    fn test_poc_bridge_in_w1() {
        let rom_data = std::fs::read("Super Mario Bros. 3 (USA) (Rev 1).nes");
        if rom_data.is_err() {
            return; // Skip if ROM not available
        }
        let mut rom = Rom::from_bytes(&rom_data.unwrap()).unwrap();

        // Read W1 fortress (entry 11) and W2 fortress (entry 13)
        let w1_fort = rom_data::read_entry(&rom, &WORLDS[0], 11);
        let w2_fort = rom_data::read_entry(&rom, &WORLDS[1], 13);

        // Write fortresses into entries [0] and [2] (1-1 and 1-3)
        rom_data::write_entry(&mut rom, &WORLDS[0], 0, &w1_fort);
        rom_data::write_entry(&mut rom, &WORLDS[0], 2, &w2_fort);

        // Put fortress tiles on the map for entries [0] and [2]
        let (row0, col0) = rom_data::entry_grid_position(&rom, &WORLDS[0], 0);
        let (row2, col2) = rom_data::entry_grid_position(&rom, &WORLDS[0], 2);
        let off0 = rom_data::map_tile_offset(0, row0, col0);
        let off2 = rom_data::map_tile_offset(0, row2, col2);
        rom.write_byte(off0, TILE_FORTRESS);
        rom.write_byte(off2, TILE_FORTRESS);

        // Place water gap where B3 tile is at row 6, col 9
        let tile_offset = rom_data::map_tile_offset(0, 6, 9);
        let original_tile = rom.read_byte(tile_offset);
        assert_eq!(original_tile, 0xB3, "Expected B3 (water) under the bridge gap");

        let (fx_type, _patterns) = fx_type_for_tile(original_tile);
        assert_eq!(fx_type, FxType::WaterBridge);

        // Build placements
        let placements = vec![
            FortressPlacement {
                level_entry: w1_fort,
                boomboom_y_offset: 0x0D35F,
                fort_tile: TILE_FORTRESS,
                dest_world: 0,
                dest_slot: 0,
                ordinal: 1,
                fortress_pos: (row0, col0),
                obstacle_pos: (3, 4), // existing lock position
            },
            FortressPlacement {
                level_entry: w2_fort,
                boomboom_y_offset: 0x0D262,
                fort_tile: TILE_FORTRESS,
                dest_world: 0,
                dest_slot: 2,
                ordinal: 2,
                fortress_pos: (row2, col2),
                obstacle_pos: (6, 9), // water bridge position
            },
        ];

        execute_world_placements(&mut rom, &placements, &[], 0);

        // Verify the water gap tile was placed
        let gap_tile = rom.read_byte(rom_data::map_tile_offset(0, 6, 9));
        assert_eq!(gap_tile, TILE_WATER_GAP);

        // Verify FX slot 1 has water bridge patterns
        let pat_off = FX_PATTERNS + 1 * 4;
        assert_eq!(rom.read_byte(pat_off), 0xD4);
        assert_eq!(rom.read_byte(pat_off + 1), 0xD6);
        assert_eq!(rom.read_byte(pat_off + 2), 0xD5);
        assert_eq!(rom.read_byte(pat_off + 3), 0xD7);

        // Verify replacement tile is the original B3
        assert_eq!(rom.read_byte(FX_MAP_TILE_REPLACE + 1), 0xB3);

        // Write POC ROM for manual testing
        let out_path = "target/poc_bridge_w1.nes";
        std::fs::create_dir_all("target").ok();
        std::fs::write(out_path, &rom.data).unwrap();
        println!("Wrote POC ROM to {}", out_path);
    }
}
