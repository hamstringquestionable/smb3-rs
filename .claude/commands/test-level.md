Generate a test ROM with specific levels placed on early map tiles for quick playtesting.

## Usage
`/test-level <level-name> [level-name2 ...] [--flags FLAGS] [--seed SEED] [--world N]`

Examples:
- `/test-level 6-F1` — place 6-F1 on tile 1 in starting world
- `/test-level 6-F1 5-F1 BC` — place 6-F1 on tile 1, 5-F1 on tile 2, Bowser Castle on tile 3
- `/test-level BC --flags SMB3R-01FFFD14 --seed 12345` — specific flags/seed
- `/test-level BC --world 8` — start in W8 so BC is on its home map

## Instructions

1. **Build** the release binary if needed: `nix-shell -p gcc --run 'export PATH="$HOME/.cargo/bin:$PATH" && cargo build --release'`

2. **Generate** the ROM using the provided flags/seed (or defaults: `--no-enemies --no-palettes --no-chest-items --no-levels`, seed random). Always use `--patched-rom -o test_level.nes`.

3. **Identify levels** by name. Use these mappings to find vanilla obj_ptr/lay_ptr/tileset:
   - Format: `W-F1` = World fortress 1 (e.g., `6-F1`), `BC` = Bowser Castle
   - Look up the level in the vanilla pointer tables (W1=0x19438/21 entries, W2=0x194BA/47, W3=0x195D8/52, W4=0x19714/34, W5=0x197E4/42, W6=0x198E4/57, W7=0x19A3E/46, W8=0x19B56/41)
   - Fortress entries from `FORTRESS_ENTRIES` in `src/randomize/rom_data.rs`
   - Bowser Castle = W8 index 40 (last real entry)

4. **Find numbered level tiles** on the target world's map grid. Map grid offsets from `MAP_TILE_GRIDS` in `rom_data.rs`:
   - W1: 0x185BA (16 cols), W2: 0x1864B (32 cols), W3: 0x1876C (48 cols)
   - W4: 0x1891D (32 cols), W5: 0x18A3E (32 cols), W6: 0x18B5F (48 cols)
   - W7: 0x18D10 (32 cols), W8: 0x18E31 (64 cols)
   - All grids have 9 rows. Tiles 0x03-0x0F are numbered levels (tile - 2 = level number).

5. **Find pointer table entries** that correspond to those tile positions using the InitIndex/ByRowType/ByScrCol tables.

6. **Overwrite** the pointer table entry (ByRowType byte for tileset, ObjSets word, LevelLayouts word) with the target level's values.

7. **Set starting world**: write world index (0-7) to ROM offset `0x30CC3`.

8. **Clear obstacles** on the map to make target tiles quickly reachable. Use **path tile replacement** with the original grid snapshot:
   - **Snapshot** the original map grid BEFORE making any replacements
   - Replace: lock tiles ($54, $56, $E4), level tiles ($03-$0F), fortress tiles ($67, $AF, $47, $EB), hand traps ($E6)
   - Use the **original** (pre-replacement) grid for neighbor checks — NOT the modified grid
   - Check all 4 neighbors (left, right, up, down) in the original grid against:
     - `VALID_HORZ = {0x45, 0x49, 0xB2, 0xB3, 0xAC, 0xB7, 0xB8, 0xDA, 0xB9, 0xE6}`
     - `VALID_VERT = {0x46, 0xB1, 0xAA, 0xAB, 0xB0, 0xDB, 0xBA}`
   - Pick PATH tile: vert only → `$46`, horiz only → `$45`, both → `$46`, neither → search outward (up to 4 tiles) for nearest path tile direction
   - **Do NOT use node tiles** ($44, $47, $48, $4A) — those stop the player. Use path tiles ($45, $46) so the player walks through.
   - Clear map object sprites (hammer bros) by zeroing IDs in the map object table

9. **Save** as `test_level.nes` and report which tiles have which levels.

## Key ROM offsets
- Map object ID master pointer: 0x16050 (per-world, 9 slots each)
- Starting world byte: 0x30CC3
- Vanilla ROM: `Super Mario Bros. 3 (USA) (Rev 1).nes`
- Pointer table starts: W1=0x19438, W2=0x194BA, W3=0x195D8, W4=0x19714, W5=0x197E4, W6=0x198E4, W7=0x19A3E, W8=0x19B56
