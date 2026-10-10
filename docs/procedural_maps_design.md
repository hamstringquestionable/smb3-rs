# Procedural Overworld Maps — design

*Design only, started 2026-10-10. **Nothing here is implemented.** Branch
`feature/procedural-maps`. Where a claim below rests on reading the code rather
than on a test or a playtest, it says so.*

## The point

**New routes through the game.** The overworld builder already decides
everything that sits *on* a map (levels, forts, locks, pipes, Hammer Bros,
route shape), but every seed is drawn on vanilla's paths. A racer who knows the
eight vanilla layouts knows the skeleton of every seed. This feature replaces
that skeleton: each world gets a freshly generated layout, with multiple
paths, islands reached by canoe, pipes, and rock shortcuts, drawn in that
world's own tile theme.

What it is **not**:

- Not a visual feature first. Decoration exists so a generated map reads as
  SMB3; the reason to ship is route variety.
- Not a change to *what* is on the map. Each world keeps its own budgets
  (pointer entries, pipe pairs, sprite slots), so level counts and the item
  economy are untouched.
- Not a change to world size. Every world keeps its vanilla screen count (see
  "Budgets"). Resizing grids is the mega-map's territory and its six
  "keyed to eight worlds" traps.

It ships as an **opt-in flag**. The default stays vanilla layouts.

## The architecture: one interface, two sources

Everything downstream of the map (pickup, build, writer, maze) reads **one
in-memory description of each world's map**, and two interchangeable sources
produce it.

```
ROM ──► [map source: vanilla OR generated] ──► WorldMap × 8
                                                  │
catalog (what each entry IS, from ROM) ───────────┤
                                                  ▼
                              pickup ─► build ─► writer ─► ROM
```

### `WorldMap`

A sketch, not a final type:

```rust
struct WorldMap {
    grid: Grid,                        // every tile: paths, nodes, water, rocks, scenery
    entries: Vec<(EntryIdx, Pos)>,     // where each pointer-table entry sits
    sprites: Vec<(SpriteSlot, Pos)>,   // Hammer Bros, W8 army, canoe, W7 piranhas
    start: Pos,
    gaps: Vec<GapCell>,                // walkable cells drawn as water until a fort opens them
    hammer_gated: Vec<Pos>,            // blanks reachable only by breaking a rock
    tile_quirks: Vec<(Pos, u8)>,       // forced tiles where the neighbour rule is wrong
}
```

It holds **positions and tiles only**. What each entry *is* (which level, fort,
Toad House) still comes from the catalog reading the ROM's pointer data. A
source decides *where* things go, never *what* they are, which is why the
budgets carry over for free.

### Vanilla source

Holds every piece of vanilla-specific map knowledge, which today is scattered
across `pickup.rs`, `build/sources.rs` and `qol/overworld_map.rs`:

- grid, entry positions, sprite positions and start, read from the ROM
- the **grid** edits `map_fixes` makes today (W1 shortcut rock, rock removal,
  W8 bridges / canoe paths / water, W3 drawbridge tiles), still behind their
  flags
- `gaps` from the FX table (what `open_fx_gaps` reads today)
- `hammer_gated` from `HAMMER_GATED_POCKETS`
- `tile_quirks` from `BLANK_TILE_OVERRIDES` and `ISLAND_POSITIONS`

**Engine patches stay where they are.** `map_fixes` also patches code, e.g.
`fix_w3_drawbridges` NOPs the `World3_Bridge` toggle (`prg010.asm` ~1949) as
well as replacing the four drawbridge tiles. Only the tile half describes the
map, so only it moves. The NOP applies whichever source ran; a generated W3
has no drawbridges either.

### Generated source

Builds the same struct from scratch, within each world's budgets: region
graph → layout → tiles (see "The generator"). It reports its own `gaps` and
`hammer_gated`, and should normally report no `tile_quirks`, because it draws
every tile correctly in the first place.

### What changes for the readers

| Today reads | Will read |
|---|---|
| `read_tile_grid` (pickup) | `WorldMap.grid` |
| `entry.grid_pos`, from the ROM pointer table (catalog) | `WorldMap.entries` |
| `read_map_sprite_positions` / `read_non_hb_sprite_positions` (capacity) | `WorldMap.sprites` |
| FX table in `open_fx_gaps` (pickup) | `WorldMap.gaps` |
| `HAMMER_GATED_POCKETS` (`build/sources.rs`) | `WorldMap.hammer_gated` |
| `BLANK_TILE_OVERRIDES` / `ISLAND_POSITIONS` (pickup) | `WorldMap.tile_quirks` |

The writer gains one duty: write **every** entry's position, every sprite
position and the start from the struct, not only the ones that moved. On the
vanilla source those are the bytes already in the ROM, so output is unchanged.
Start ↔ Airship swap already does this for two entries
(`write_swapped_world_entries`, then the every-seed `resort_pointer_table`),
which is the precedent that it is safe.

*This table is from one read of the code. Milestone 1 starts by listing every
reader with file and line; anything this table missed is found there.*

### What "off" means

The flag picks a source. There is no `if procgen` in pickup, the builder or the
writer; none of them can tell which source ran.

## Budgets

Each world keeps its vanilla screen count, pointer entries and pipe pairs. Grids
are 9 rows by `columns`, stored screen-major (`rom_data::map_tile_offset`).

| World | Screens | Columns | Pointer entries | Pipe pairs |
|---|---|---|---|---|
| W1 | 1 | 16 | 21 | 0 |
| W2 | 2 | 32 | 47 | 1 |
| W3 | 3 | 48 | 52 | 3 |
| W4 | 2 | 32 | 34 | 2 |
| W5 | 2 | 32 | 42 | 2 (incl. spiral tower) |
| W6 | 3 | 48 | 57 | 2 |
| W7 | 2 | 32 | 46 | 8 |
| W8 | 4 | 64 | 41 | 6 |

Sources: `MAP_TILE_GRIDS`, `WORLDS[].entry_count` (`rom_data/tables.rs`),
`VANILLA_PIPE_PAIRS` (`build/capacity.rs`). Vanilla geometry, so these do not
drift.

How counts flow today, unchanged by this feature (`build/capacity.rs`):

```
capacity = min(blank nodes − pipe ends − forts,
               pointer slots − pipe ends − forts)
```

Pointer slots are the hard limit. The grid term is measured. So the
generator's obligation is **enough nodes that the grid is never the binding
term**: roughly pointer slots plus headroom, so placement still has choice.
Pipes spend pointer slots, so a pipe-heavy theme leaves fewer levels in its
world.

**Open:** whether the pipe-pair count is a ROM-table limit or only a vanilla
default the builder chose to keep. W1 has zero, which would make W1 a
pipe-less world under generation too.

## The generator

Two levels, so route quality can be tuned on a small graph before any tile
art exists.

### Level 1: region graph (where route choice comes from)

A world is a handful of regions joined by typed connectors:

| Connector | Engine mechanism | Role in a route |
|---|---|---|
| Path | ordinary tiles | free movement |
| Choke | a cell a lock can sit on | the builder's shaping tool; forts open locks |
| Gap | water that a fort turns into a bridge | a lock drawn as water |
| Pipe pair | pipe destination table | jumps between regions |
| Water + docks | canoe map object | reaches islands |
| Rock | broken with a hammer | an **optional** shortcut |

Loops, parallel branches and islands are decided here. The number of distinct
routes can be counted on the graph and a weak graph rejected before drawing.

A rock may only be a shortcut. A required route never depends on one unless a
hammer is guaranteed, and that is the builder's call, not the generator's.

### Level 2: layout and tiles

Each region becomes nodes and paths on the grid:

- **Node parity.** A map move is two tiles (`Map_CheckDoMove`, `LDA #32`), so
  nodes sit on even rows in W1–W6 and odd rows in W7–W8, with one-tile path
  runs between them. Breaking this gives nodes that look connected and are
  dead in game (see the parity trap in the hand-drawn-map notes).
- **Path tiles are chosen from the left and up neighbours only**, the vanilla
  convention `blank_tile_from_neighbors` follows. The renderer must match it.
- **Themes.** Each world uses its own tile set and palette. Moving a theme to
  another world slot depends on whether palette and tile set can be separated
  from the slot; not investigated, and out of scope for v1.

### Water: three kinds

Vanilla water is one of three things, and the renderer must keep them apart:

1. **Scenery**: not on the walk graph at all. Most of vanilla's water is this.
2. **Gap**: a walkable cell drawn as water until a fort opens it. Reported in
   `gaps`; becomes the FX table.
3. **Canoe water**: crossed by the canoe between docks.

Proving the vanilla round-trip (milestone 2) is where we show we can tell them
apart.

## Anchors and rules

Fixed points the generator builds around, and rules it must respect:

- **Bowser's castle** (W8) is an anchor.
- **The W8 wand-gate cell (5,59)** is an anchor. Its position is baked into
  6502 (`GATE_TILE_MEM`, `maze/wand_gate.rs`), and it sits on the W8 bridge
  row. Keeping the cell fixed means the patch does not change.
- **Starts are anchors in v1.** That avoids rewriting `Map_Y_Starts` and the
  start-position tables; routes still change completely.
- **The airship is never approached from above.** The castle top `$C8` is
  stamped on the cell above it (`swap_tiles_above`). Vanilla W5 is the one
  world approached from above and is special-cased to skip the castle top;
  forbidding that approach removes the special case instead of inheriting it.
- **Rows 7 and 8 share one completion bit.** A completable cell on row 7 and
  one on the row-8 cell beside it collide, and terrain can be the second
  claimant (the W2 oasis). The builder already guards this
  (`WorldState::row78_barred`); the generator must not create terrain that
  claims the bit on a cell the builder will use.
- **Tile palette page sets completability.** Which tiles count as completable
  or walkable depends on the page's level-gate threshold. The tile vocabulary
  is chosen per theme with this in mind.
- **Hammer-gated pockets are output, not constants.** Every blank that only a
  rock opens is reported in `hammer_gated`, so connectivity does not spend a
  pipe on it.

Not yet checked, and listed so they are not forgotten:

- how the airship sprite and W8 army / W7 piranha sprite tables tie positions
  to entries (`MAP_OBJ_ENTRY_LINKS`)
- the credits mini-maps (`credits::MINI_TILE_LUT`), which sample the final map
- how Start ↔ Airship swap behaves on a generated world, given starts are
  anchors

## World Maze

The maze sits downstream of the map source, so a world that is completable in
standard play should work in the maze unchanged, provided the wand-gate anchor
holds. If it does not, that is a finding to discuss, not a reason to
special-case the generator.

## How we know it works

- **Vanilla source:** seeded output byte-identical before and after the
  refactor (`tests/rom_identity.rs`). This is byte identity used for its proper
  job: "I intended to change nothing, prove it".
- **Round-trip:** vanilla ROM → `WorldMap` → written back → identical bytes.
  Proves the struct captures everything a map is.
- **Generated source:** never stranded
  (`CENSUS_SEEDS=500 cargo test --release --lib all_world_targets_reachable`),
  and **route choice at least as good as vanilla layouts**
  (`test_route_census`), read per world, not just overall. W1 and W7 are the
  sensitive columns. The C1 floors were tuned on vanilla layouts and may need
  re-measuring; any change to them is earned by a census.
- **Playtest**, via a `testrom` flag. The parity trap and the oasis bug were
  both found by playing.

## Milestones

1. **The interface and the vanilla source.** List every reader (file and line),
   introduce `WorldMap`, move the readers onto it. Proven byte-identical. One
   PR into `beta/next`, useful on its own because it gathers the vanilla
   quirks into one module.
2. **Round-trip and inventory.** Decode vanilla into `WorldMap` and write it
   back byte-identical. Record each world's tile vocabulary (paths, the three
   water kinds, rocks, scenery) and vanilla's shape statistics (nodes per
   screen, branching, dead ends, loops) as the generator's target.
3. **W1 generated: paths and rocks.** A standalone binary printing ASCII maps
   first, then a `testrom` flag, then the censuses. (W1 has no pipe pairs.)
4. **A world with pipes** (W2 or W4).
5. **W3: water and canoe.** Islands are the biggest new mechanic, so they get
   their own milestone.
6. **The rest**, each world's special mechanics as its own question: W5's sky
   page, W7's pipe-heavy budget, W8's darkness, army sprites and anchors.
7. **The flag.** Flag-key bits and a key version bump, web option.

## Open questions

- Is the per-world pipe-pair count a hard ROM limit?
- Should a theme be able to move to another world slot (a desert map in slot
  5)? Needs the palette/tileset question answered first.
- Starts as anchors in v1: keep for good, or generate later?
