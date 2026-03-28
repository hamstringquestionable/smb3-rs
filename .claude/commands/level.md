Look up a level's enemies, items, sub-areas, and ROM offsets by name.

## Usage
`/level <level-name>`

Examples:
- `/level 3-2` — look up World 3 level 2
- `/level 7F1` — World 7 fortress 1
- `/level 8B` — Bowser Castle
- `/level 5A` — World 5 airship
- `/level 8-Tank` — W8 tank level

## Instructions

Run the rom_map.py level lookup:

```
nix-shell -p python3 --run "python3 tools/rom_map.py --level $ARGUMENTS"
```

Present the full output to the user. The output includes:
- Level identity: world, entry index, type, tileset, obj_ptr, lay_ptr, grid position
- All enemies with ROM offsets, names, classes, screen/row/col positions
- All sub-areas with their enemies
- All powerup blocks with ROM offsets, item types, randomization class, protection status
- Boss presence (Boom-Boom, Koopaling, Bowser)

When the user references specific enemies or items from the output, the ROM offsets are directly usable for patching in `src/randomize/` code. The obj_ptr identifies the enemy data segment, and byte2_offset identifies individual powerup blocks.

## Name formats
- `N-M` — World N level M (e.g., 3-2, 1-1, 6-5)
- `NF` or `NFx` — World N fortress (e.g., 3F, 7F1, 7F2)
- `NA` — World N airship (e.g., 5A)
- `8B` — Bowser Castle
- `8-Tank`, `8-Navy`, `8-Air`, `8-STnk` — W8 special levels
- `8-Hnd1`, `8-Hnd2`, `8-Hnd3` — W8 hand traps
- `2-QS`, `2-Pyr` — W2 quicksand, pyramid
- `5-SC` — W5 spiral castle
- `7-P1`, `7-P2` — W7 piranha levels
