# Pipe Swap POC - Session Findings

## Summary
Successfully implemented a proof-of-concept for swapping level positions on the overworld map. The swap correctly exchanges both level data (obj/lay/tileset pointers) and visual map tiles between two entries.

## Key Learnings

### 1. Tile Visualization vs Game Rendering
- **Critical Discovery**: Tile IDs don't universally represent the same visual graphics across all worlds **this needs to be verified** 
- Each world has its own CHR bank that maps tile IDs to different graphics
- Example in World 2:
  - Tile `0xBC` renders as **pipe** graphics in-game (not a level panel)
  - Tile `0x69` renders as **pyramid** graphics in-game (not a pipe)
  - Tile `0x03` is the standard level "1" panel

### 2. Map Tile Grid Format (Confirmed Working)
- **Grid offset formula**: `base + screen*144 + row*16 + col_in_screen`
- Row-major storage: 144 bytes per screen (9 rows × 16 cols)
- W2 base: `0x1864B`, 2 screens (32 columns total)
- **Tile writes work correctly** - verified with fortress tile debug test

### 3. Level Data Swap Mechanics (Confirmed Working)
To swap two level positions on the map:
1. **Swap pointer table data**: obj_ptr, lay_ptr, tileset (via `read_entry`/`write_entry`)
2. **Swap map tiles**: exchange tile IDs at both grid positions
3. **Positions stay fixed**: ByRowType row nibble and ByScrCol stay in place
   - The game matches player position → entry index → level data
   - We swap the data at each index, not the indices themselves

### 4. Entry Numbering ≠ Level Numbering
- Pointer table indices (entry 2, entry 12, etc.) don't correspond to player-visible level numbers
- Entry 12 in W2 = "2-1" (first level on path)
- Entry 2 in W2 = "2-2" (second level)
- Must use map visualization to identify which entry is which level

### 5. Working POC Implementation
**File**: `src/randomize/overworld.rs` - `poc_pipe_swap()`

**Test case**: W2 entry 12 (first level, tile 0x03) ↔ entry 19 (pipe, tile 0xBC)
- Sets starting world to W2
- Swaps level data and map tiles
- Both tile graphics and level loading swap correctly

**Code structure**:
```rust
pub fn poc_pipe_swap(rom: &mut Rom) {
    // 1. Read both entries' level data
    let data_a = read_entry(rom, world, entry_a);
    let data_b = read_entry(rom, world, entry_b);
    
    // 2. Swap level data (obj, lay, tileset)
    write_entry(rom, world, entry_a, &data_b);
    write_entry(rom, world, entry_b, &data_a);
    
    // 3. Swap map tiles
    let (row_a, col_a) = entry_grid_position(rom, world, entry_a);
    let (row_b, col_b) = entry_grid_position(rom, world, entry_b);
    let tile_off_a = map_tile_offset(world_idx, row_a, col_a);
    let tile_off_b = map_tile_offset(world_idx, row_b, col_b);
    let tile_a = rom.read_byte(tile_off_a);
    let tile_b = rom.read_byte(tile_off_b);
    rom.write_byte(tile_off_a, tile_b);
    rom.write_byte(tile_off_b, tile_a);
}
```

### 6. Map Visualization Tool Updates
**File**: `tools/map_viz.py`

Added tile-based rendering for better accuracy:
- `P` = pipe (tile 0xBC with entry)
- `p` = pyramid (tiles 0x68/0x69, decorative or with entry)
- This overrides entry type rendering to match visual appearance

### 7. Next Steps for Full Pipe Shuffle

For intra-world pipe shuffling:
1. Identify all pipe entries per world (tile 0xBC in W2/W7, maybe others elsewhere)
2. Identify valid swap targets (levels on same or nearby rows)
3. Swap pipe entries with level entries using the POC pattern
4. **No special pipe destination handling needed** - pipes are just regular level entries
   - The `special` entries (obj=0x0300-0x0900) are different (warp pipe connectors)
   - Enterable pipes (0xBC tiles with level data) work like any other level

### 8. Autoscroll Patch Interaction
- Autoscroll module runs AFTER pipe swap and modifies some level pointers (airship redirects)
- Does NOT interfere with our swaps - verified via byte-level ROM inspection
- Entry 36 (airship) gets modified by autoscroll, but our swapped entries 12/19 preserve correctly

## Test Results
✅ Map tile writes work (fortress debug test)
✅ Level data swaps work (pyramid ↔ pyramid test)
✅ Pipe ↔ level swap works (entry 12 ↔ entry 19)
✅ Starting world override works (99 lives test)
✅ Tiles render correctly based on world-specific CHR banks

## Important Constants
```rust
// W2 pointer table
rowtype_base: 0x194BA
entry_count: 47

// W2 map tile grid
base: 0x1864B
screens: 2 (32 columns)
format: row-major, 144 bytes/screen

// Starting world
WORLD_INIT_OPERAND: 0x30CC3
```

## Known Issues / Limitations
- Entry numbering is not intuitive - requires map visualization to identify levels
- Tile appearance varies by world CHR bank - can't assume tile 0x69 is always a pyramid
- No automatic detection of which tiles are pipes vs levels - must be world-specific

## Files Modified This Session
- `src/randomize/overworld.rs` - POC implementation
- `tools/map_viz.py` - Tile rendering improvements
- `src/randomizer.rs` - Added `poc_pipe_swap` option
- `src/main.rs` - Added `--poc-pipe-swap` CLI flag
