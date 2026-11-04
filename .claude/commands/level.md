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

Run the Rust lookup (`src/rominfo.rs`, read through `rom_data::level_index`):

```
nix-shell --run "cargo run --release --quiet --bin rominfo -- --level $ARGUMENTS"
```

Add `--rom <path>` to look inside a randomized ROM; names are then that ROM's own
(`1-1` is whatever level now sits in 1-1's slot). `--list` prints every name.

Present the full output to the user. The output includes:
- Level identity: kind, world, entry index, map position and tile, pointer table tileset/layout/objects
- Other entries that share the same main area (Toad Houses, Hammer Bro battles)
- Every area a junction reaches, main first: header bytes decoded (screens, music
  and its byte offset, time, alt pointers), command count and junctions
- Each area's powerup blocks: shape-byte offset, block, position, and whether the
  powerup randomizer protects or restricts it
- Each area's objects: offset, ID, disassembly name, position
- Why the area chain stops, and which bosses appear

The offsets are file offsets, directly usable in `src/randomize/` code: a powerup
block's offset is its shape byte (the one the randomizer rewrites), an object's is
its ID byte. Enemy classes are not shown: they depend on the enemy options.

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
