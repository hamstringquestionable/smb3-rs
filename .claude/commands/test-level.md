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

2. **Generate** the ROM using `target/release/smb3-rs` with the provided flags/seed (or defaults: `--no-enemies --no-palettes --no-chest-items --no-levels`, seed random). Always use `--patched-rom -o test_level.nes`.

3. **Apply open-movement patches** from the practice ROM so the player can walk over level/lock/fortress tiles without entering or clearing them:
   `nix-shell -p python3 --run 'python3 tools/apply_ips_subset.py smb3practice_SE.ips test_level.nes 0x14010 0x18010'`
   This applies only the PRG010–011 records (~19 records, ~85 bytes). Do NOT apply the full IPS — its PRG006/PRG012 records would clobber the randomized enemy data and overworld map.

4. **Identify levels** by name. Use these mappings to find vanilla obj_ptr/lay_ptr/tileset:
   - Format: `W-F1` = World fortress 1 (e.g., `6-F1`), `BC` = Bowser Castle
   - Look up the level in the vanilla pointer tables (W1=0x19438/21 entries, W2=0x194BA/47, W3=0x195D8/52, W4=0x19714/34, W5=0x197E4/42, W6=0x198E4/57, W7=0x19A3E/46, W8=0x19B56/41)
   - Fortress entries from `FORTRESS_ENTRIES` in `src/randomize/rom_data.rs`
   - Bowser Castle = W8 index 40 (last real entry)

5. **Find numbered level tiles** on the target world's map grid. Map grid offsets from `MAP_TILE_GRIDS` in `rom_data.rs`:
   - W1: 0x185BA (16 cols), W2: 0x1864B (32 cols), W3: 0x1876C (48 cols)
   - W4: 0x1891D (32 cols), W5: 0x18A3E (32 cols), W6: 0x18B5F (48 cols)
   - W7: 0x18D10 (32 cols), W8: 0x18E31 (64 cols)
   - All grids have 9 rows. Tiles 0x03-0x0F are numbered levels (tile - 2 = level number).

6. **Find pointer table entries** that correspond to those tile positions using the InitIndex/ByRowType/ByScrCol tables.

7. **Overwrite** the pointer table entry (ByRowType byte for tileset, ObjSets word, LevelLayouts word) with the target level's values.

8. **Set starting world**: write world index (0-7) to ROM offset `0x30CC3`.

9. **Save** as `test_level.nes` and report which tiles have which levels. With the open-movement patches applied, no tile clearing is needed — Mario can walk freely across the entire overworld.

## Key ROM offsets
- Map object ID master pointer: 0x16050 (per-world, 9 slots each)
- Starting world byte: 0x30CC3
- Vanilla ROM: `Super Mario Bros. 3 (USA) (Rev 1).nes`
- Pointer table starts: W1=0x19438, W2=0x194BA, W3=0x195D8, W4=0x19714, W5=0x197E4, W6=0x198E4, W7=0x19A3E, W8=0x19B56
