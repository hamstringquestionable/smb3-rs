# Mega map — experiment (branch `experiment/mega-map`)

**Status:** working prototype. Vanilla-base only, reachable through `testrom
--mega`; nothing is wired to a flag, the flag key, or the randomizer pipeline.
Do not merge to `main` as-is.

```sh
cargo build --bin testrom
./target/debug/testrom --mega --output mega.nes
```

## What it does

Folds **screen 0 of every world** into a single eight-screen map in world 0:
128 columns, 157 pointer entries, eight fortresses, one biome per screen, all
walkable end to end from the start tile.

Everything stays inside the space the eight maps already owned. No free space
is claimed, and no new routine is added — the only 6502 changes are in-place
operand edits over vanilla code.

```
  base: vanilla
  removed 62 locks + water gaps across all 8 maps
  mega map: 8 screens, 157 entries, 8 forts (0 reconnected, 0 stranded),
           7 pipe pairs kept, 8 pipe tiles demoted, 9 singletons blanked
    seam 0|1: row 2, 5 tile(s)      seam 4|5: row 2, 5 tile(s)
    seam 1|2: row 2, 2 tile(s)      seam 5|6: row 4, 1 tile(s)
    seam 2|3: row 4, 4 tile(s)      seam 6|7: row 4, 3 tile(s)
    seam 3|4: row 4, 4 tile(s)
```

## Why screen 0 of every world

Screen 0 of each world holds **exactly one fortress**, and the eight together
hold 157 pointer entries. That gives eight screens, eight forts, eight biomes,
and 157 entries — under the 255 a byte-wide `Map_ByXHi_InitIndex` can name.

Taking whole worlds instead is not possible: vanilla's nineteen screens need
304 completion columns and there are 128.

Because destination screen `i` is world `i`'s screen, the **region index and
the world index coincide**. That is deliberate — it is what makes the eight-wide
per-world dispatch tables (palette, music, bottom tile, king room, airship
level, airship travel) reusable as *per-region* tables later, with no table
resizing. See "Next" below.

## The eight-screen ceiling, and why it cost almost nothing

`Map_Completions` is 128 bytes at `$7D00`, one per map column, split
`$7D00-$7D3F` Mario / `$7D40-$7D7F` Luigi. The disassembly's own comment says
it "allows a MAX of 4 map screens".

For **one player** it allows eight, because a column index for eight screens
runs 0..127 and lands exactly inside the array — and the engine already walks
all 128 bytes: the redraw loop runs its column counter to `$80`, and the
game-over clear runs `LDY #$7F`. What stops it is four sites that *fold* the
upper half onto the lower one, assuming it belongs to the other player.

| Site | Vanilla | Becomes | Cost |
|---|---|---|---|
| `Map_Reload_with_Completions`, screen index | `AND #$30` | `AND #$70` | 1 byte |
| ...its Mario/Luigi marker select | `AND #$40` | `AND #$00` | 1 byte |
| `MO_DoFortressFX` mirror write | `$7D40,Y` ×2 | `$7D00,Y` ×2 | 2 bytes |
| rock-break mirror write | `EOR #$40` | `EOR #$00` | 1 byte |
| game-over clear | AND other player, 64 cols | `LDA #$00`, 128 cols | 2 + 6 bytes |

Total: **seven operand bytes and one six-byte splice, zero free space.** The
shift after `AND #$70` already produces a `Tile_Mem_Addr` index, and that table
carries fifteen screen entries, so nothing downstream needs widening.

The `MO_DoFortressFX` mirror is worse than cosmetic: `Map_Completions+$40,Y`
with a column past 63 writes to `$7D80` and up, which is `Inventory_Items` —
clearing a fortress on screens 4–7 would have rewritten the player's inventory.

**The cost is two-player mode.** Luigi's completion array *is* screens 4–7.
Nothing here disables 2P; the prototype is 1P-only by construction.

Tile RAM was never the constraint: `Tile_Mem` is `$6000-$794F` and
`Tile_Mem_Addr` holds fifteen screen entries.

## Node parity — the finding that mattered

Map movement advances **two tiles at a time** (node, path tile, node), so
`row % 2` and `col % 2` are each invariant along any walk. Every node a walk
can reach shares the start's parity class. A single vanilla world never
notices. Eight folded together do:

| Screen | Source | Node rows | Node columns |
|---|---|---|---|
| 0–5 | W1–W6 | all even | all even |
| 6 | W7 | all odd | **mixed** — 12 even, 11 odd |
| 7 | W8 | all odd | all even |

Two consequences, both fatal before they were fixed:

1. **W7 and W8 are half a step out of phase vertically.** No corridor of any
   shape can join them to the rest — not a longer one, not an L-shaped one,
   because the clash is invariant under every legal move. `row_shifts` moves
   those two screens down one row. It costs nothing: W7's deepest node goes
   from row 7 to row 8, W8's from 5 to 6, both still on the map. The fortress
   FX row byte and completion bit shift with them.

2. **W7's two column lattices are joined only by its pipes.** The first version
   demoted every pipe on the map, which does not merely inconvenience Pipe Land
   — it makes half of it unreachable by any path, fortress included.
   `carry_pipes` keeps the pairs whose *both* endpoints survive (seven of
   twenty-four) and retargets their screens and rows.

## Seams: a search, not a corridor

Each source screen was drawn to meet *its own* neighbour, so the boundaries are
arbitrary — a screen ending in water now abuts one starting in desert. Joining
them is `cheapest_route`: Dijkstra over the node lattice, where an edge costs
the number of tiles that must be written to make it passable, existing paths
cost nothing, and edges that would overwrite content are not offered.

That shape was arrived at by killing four simpler ones, each of which produced
a map that looked fine and was not:

- A corridor of `0x44` tiles. `0x44` *looks* like a path and is a blank **node**;
  the horizontal path tile is `0x45`.
- A corridor of any odd span — the far end lands out of phase and connects to
  nothing.
- A corridor anchored on the nearest node. Seam 1|2 landed cleanly on column 32
  and the walk still stopped there, because column 32 was an isolated cell.
  Reaching a screen is not reaching *into* it: the target is now the next
  screen's largest connected component.
- A corridor that assumes it worked. Seam 3|4's shortest candidate cut through
  the vertical path feeding its own left anchor; the carve landed, screen 4
  stayed dark, and the three seams after it starved. Another ran straight over
  W4's fortress and deleted it — the map still connected, and a fortress simply
  ceased to exist.

`connect_stranded_forts` then runs the same search for any fortress the seams
left isolated: W7's sits in the last column of its screen and was, in vanilla,
approached from W7's *second* screen, which the fold does not carry.

## Ordering

`testrom` applies the fold at **step 4a — after `open_map`, before the code-only
passes.** Every pass that reaches map tiles through `rom_data`'s per-world
constants must run first, because those constants describe the vanilla
eight-world layout and stop describing the ROM the moment the fold runs.
`open_map` walks all eight `MAP_TILE_GRIDS` rows, whose offsets land at the
wrong stride inside the merged grid; running it first also means the carried
screens arrive already unlocked, which `--mega` wants anyway.

The mirror-image rule holds *inside* the module: `row_shifts` and
`fort_positions` are computed against the untouched ROM up front, because
`build_pointer_block` writes the merged block over the source blocks of the
first four worlds. An earlier version re-derived a shift afterwards and read
its own output.

This is the same trap the world-merge experiment hit from the other direction
(see `world_merge_experiment.md`, "the rule this experiment kept violating").

## ROM layout

| Resource | Destination | Size | Space it fits in |
|---|---|---|---|
| Tile grid | `0x185BA` (W1's) | 8×144 + `FF` = 1153 B | 2888 B the eight grids own |
| Pointer block | `0x19434` (W1's) | 8 + 6×157 = 950 B | 2072 B the eight blocks own |

The pointer block **must** live at `0x19434`. PRG012's only other gap over 100
bytes is `0x19DD0`, and it is not free: the Big ? Block trampoline, the flag-key
stamp and the title-screen seed-hash icons all write there — the last of them
after the overworld writer runs.

`World_Map_Max_PanR` goes to `$70` (128 columns; vanilla's largest is `$30`).
`INC World_Num` is NOPed out, because only world 0 has a grid and a block now
and clearing the airship would otherwise advance into the middle of the merged
block.

## Tests

`cargo test --lib mega_map` — all skip when the ROM is absent.

| Test | What it holds |
|---|---|
| `carries_every_source_screen_entry` | 157 entries, under the InitIndex ceiling |
| `merged_block_is_sorted_and_in_bounds` | screen-sorted, in-grid, `InitIndex[s]` is the *first* entry on `s` |
| `every_screen_is_reachable_from_the_start` | all eight screens connect |
| `every_screens_fortress_survives_and_is_reachable` | all eight forts exist and are walkable |
| `carried_fortress_fx_is_aimed_at_its_new_screen` | FX screen, completion column, clean row nibble, world-0 FX row |
| `completion_widening_refuses_an_unexpected_rom` | mutates each of the eight patch sites and checks the guard fires |

Two `#[ignore]` diagnostics — `diagnose_seams` and `diagnose_pipes` — print the
walk frontier and the pipe read-back. They earned their place: every seam bug
above was found with them.

The reachability tests replicate `testrom`'s lock/gap removal before folding,
because a fortress behind its own lock is the vanilla contract, not a defect.

## Still open

1. **Progression.** `INC World_Num` is NOPed, so the map never advances and the
   airship leads nowhere. Replacing it with a wand/region-gate model is the
   piece that turns this from a map into a game.
2. **One region's worth of dressing for all eight.** Palette, music, bottom
   tile, king room, airship level and airship travel are still keyed to
   `World_Num`, which is pinned at 0 — so the whole continent renders in W1's
   colours and plays W1's music. The fix is the `Map_Region` latch described in
   the design discussion: a spare WRAM byte holding the current screen, with the
   eight-wide dispatch tables re-indexed off it. Because those tables are
   already eight entries wide and `Map_Region` is a plain absolute byte like
   `World_Num`, most sites are same-size operand swaps.
3. **Blank tiles are land-themed.** The seam routes write `0x44`/`0x45`, which
   is wrong on the sky and island screens.
   `overworld_pickup::blank_tile_from_neighbors` knows the per-screen rule but
   reads the vanilla per-world grids, so it cannot run after the fold.
4. **The goal.** `reconcile_singletons` keeps the rightmost castle, but most
   worlds' castles live on their *last* screen, which the fold does not carry —
   so the goal is wherever it happens to fall, not at the far end.
5. **Map objects (hammer bros) are not carried.** World 0 keeps W1's table,
   which is valid but leaves seven screens without encounters.
6. **Not flag-gated, and the randomizer is not wired to it.** `--mega` refuses a
   `--randomize` base and refuses `--place`, because both address the map
   through per-world constants the fold invalidates. Wiring the overworld
   builder to a single 128×9 graph is the large remaining workstream.
7. **Untested on hardware/emulator.** Everything above is verified by the Rust
   walker against the ROM image. The one thing the walker cannot answer is
   whether the map engine actually *pans* across eight screens — vanilla's
   widest scrolling map is three (W3, W6), and W8's four screens do not scroll.
   **This is the next thing to check, and it is cheap: load `mega.nes` and walk
   right.**
