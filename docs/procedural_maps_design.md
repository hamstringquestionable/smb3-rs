# Procedural Overworld Maps — design### What the readers read now

*As built.* The catalog owns the eight maps it was built from
(`NodeCatalog::maps`), so every later phase reaches them through the catalog
it already holds.

| Reader | Used to read (ROM or constant) | Reads now |
|---|---|---|
| catalog (`classify`, `pipes`) | grid, `entry_grid_position` | `WorldMap.grid`, `.entry_pos` |
| pickup | `read_tile_grid`, FX table | `.grid`, `.gaps` |
| `build/sources.rs` | HB sprites, `HAMMER_GATED_POCKETS` | `.hb_sprites`, `.hammer_gated` |
| `build/capacity.rs` | HB and other sprite positions | `.hb_sprites`, `.sprites` |
| `writer/assign.rs`, `writer/sprites.rs` | HB sprite positions | `.hb_sprites` |

Still read from the ROM on purpose: what each entry *is* (the catalog), and
map-object slot ids (slot eligibility belongs to the sprite writers).

*Started 2026-10-10. **Milestone 1 (the interface and the vanilla source) is
built; nothing past it is.** Branch `feature/procedural-maps`. Where a claim below rests on reading the code rather
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

*That is the design sketch. The built type (`overworld/map_source/mod.rs`) is
narrower for now: `entry_pos` indexed by entry, `sprites` plus `hb_sprites` as
positions, and no `start` or `tile_quirks` field yet. Those arrive with the
generator, which is the first thing that needs them.*

It holds **positions and tiles only**. What each entry *is* (which level, fort,
Toad House) still comes from the catalog reading the ROM's pointer data. A
source decides *where* things go, never *what* they are, which is why the
budgets carry over for free.

### Vanilla source

*As built (milestone 1).* `overworld/map_source/vanilla/` holds the
vanilla-specific map knowledge that used to be spread across `pickup.rs`,
`build/sources.rs` and `qol/overworld_map.rs`:

- `read` takes the grid, entry positions and sprite positions from the ROM.
- `load` is `read` plus the map edits in `fixes.rs` (rock removal, extra
  hammer rocks, the W1 shortcut, the W8 bridges / water / canoe docks, the W3
  drawbridge tiles), behind their flags. **They are applied in memory and
  never written to the ROM**; the writer puts each world's grid in place once.
  Settled deliberately over "write the ROM, read it back": one path for the
  map, written once, with no "must run before the builder reads the map"
  ordering rule.
- `gaps` come from the FX table, computed **after** the edits, because the W8
  bridge edit covers an FX cell that must then not count as a gap.
- `hammer_gated` comes from `HAMMER_GATED_POCKETS`.
- `BLANK_TILE_OVERRIDES` and `ISLAND_POSITIONS` moved here too, but pickup
  still reads them directly. Gating them off for a generated map waits for the
  generator, where it can be tested.

**One map write stays a ROM write: the W8 canoe sprite** (`qol::place_w8_canoe`,
in the `map_fixes` stage). Map sprites have several writers, so the source reads
the sprite back rather than owning it. It moves when the writer gains the
duty below.

Test harnesses that build from a ROM call `vanilla::stamp_fixes`, which writes
the edited grids into the harness ROM. Test-only; the pipeline never does it.

**Engine patches stay where they are.** `map_fixes` also patches code, e.g.
`fix_w3_drawbridges` NOPs the `World3_Bridge` toggle (`prg010.asm` ~1949) as
well as replacing the four drawbridge tiles. Only the tile half describes the
map, so only it moves. The NOP stays in `qol::fix_w3_drawbridges` and applies
whichever source ran; a generated W3 has no drawbridges either.

### Generated source

Builds the same struct from scratch, within each world's budgets: region
graph → layout → tiles (see "The generator"). It reports its own `gaps` and
`hammer_gated`, and should normally report no `tile_quirks`, because it draws
every tile correctly in the first place.

### What the readers read now

*As built.* The catalog owns the eight maps it was built from
(`NodeCatalog::maps`), so every later phase reaches them through the catalog
it already holds.

| Reader | Used to read (ROM or constant) | Reads now |
|---|---|---|
| catalog (`classify`, `pipes`) | grid, `entry_grid_position` | `WorldMap.grid`, `.entry_pos` |
| pickup | `read_tile_grid`, FX table | `.grid`, `.gaps` |
| `build/sources.rs` | HB sprites, `HAMMER_GATED_POCKETS` | `.hb_sprites`, `.hammer_gated` |
| `build/capacity.rs` | HB and other sprite positions | `.hb_sprites`, `.sprites` |
| `writer/assign.rs`, `writer/sprites.rs` | HB sprite positions | `.hb_sprites` |

Still read from the ROM on purpose: what each entry *is* (the catalog), and
map-object slot ids (slot eligibility belongs to the sprite writers).

The writer gains one duty: write **every** entry's position, every sprite
position and the start from the struct, not only the ones that moved. On the
vanilla source those are the bytes already in the ROM, so output is unchanged.
Start ↔ Airship swap already does this for two entries
(`write_swapped_world_entries`, then the every-seed `resort_pointer_table`),
which is the precedent that it is safe. **Not built yet**: on the vanilla source
it would write identical bytes, so nothing can test it before a source moves
things. It belongs with the generator.

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

The renderer's vanilla oracle test (see "How we know it works") is where we
show we can tell them apart.

**Canoe routes stay vanilla until milestone 5.** Which docks the canoe joins is
a fixed per-world table today (`rom_data::CANOE_EDGES`, read through
`active_canoe_edges`). Generating docks and canoe paths is part of the W3
milestone; the dock pairs then join `WorldMap`, so each source supplies its
own, the way `gaps` works.

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
- **Renderer oracle:** describe vanilla abstractly (nodes, edges, the three
  kinds of water), let the renderer draw tiles from that description, and
  compare against vanilla's real tiles. This separates renderer bugs from
  generator bugs: once it passes, a generated map that looks wrong is the
  generator's fault. It covers what a renderer must get right (path and
  junction tiles, blank nodes, water and shorelines), **not** scenery, which
  is hand-placed art no rule reproduces. Written alongside the renderer, in
  the style of `test_theme_matches_every_vanilla_blank`, which already does
  this for vanilla's 155 blanks.

  *Not* a raw round-trip: `WorldMap` stores raw tiles, so reading vanilla in
  and writing it back is identical by construction, and milestone 1 already
  proved it. An earlier draft listed that round-trip as its own milestone; it
  was dropped on 2026-10-11 for that reason.
- **Generated source:** never stranded
  (`CENSUS_SEEDS=500 cargo test --release --lib all_world_targets_reachable`),
  and **route choice at least as good as vanilla layouts**
  (`test_route_census`), read per world, not just overall. W1 and W7 are the
  sensitive columns. The C1 floors were tuned on vanilla layouts and may need
  re-measuring; any change to them is earned by a census.
- **Playtest**, via a `testrom` flag. The parity trap and the oasis bug were
  both found by playing.

## Milestones

1. **The interface and the vanilla source.** *Done 2026-10-10.* `WorldMap`,
   the vanilla source, readers moved onto it, map edits applied in memory.
   Proven byte-identical by `rom_identity` (100 seeds over 11 arms, including
   one with `more hammer rocks` and `8s are Wild` on).
2. **Inventory.** *Done 2026-10-11*; results in "Vanilla's shape" below.
   Each world's tile vocabulary and shape statistics, as the generator's
   target.
   **Groundwork after the inventory** (*done 2026-10-11*, byte-identical):
   pipe pairing reads the ROM again, since which entries pair is identity
   and a generated map moves pipes; the vanilla blank-tile overrides and
   island cells apply only to grids with `Grid::vanilla_layout` set; and the
   writer writes every kept entry whose map position differs from the ROM's
   (`write_kept_entry_positions`, which replaced the swap-only
   `write_swapped_world_entries`). **Left for the first generated world**,
   because nothing exercises them before one exists: choosing the source per
   world; and Hammer Bro sprites (require Hammer Bro shuffle on, or have the
   generator place them). The airship needs no sprite: in W1–W7 there is no
   airship map sprite to move, because the randomizer disables the airship
   cutscene and autoscroller (per the project owner, 2026-10-11).
3. **W1 generated: paths and rocks.** In this order: the layout generator,
   judged on **ASCII layouts** from an ignored test before anything touches
   tiles or the ROM; then the renderer and its vanilla oracle test; then a
   `testrom` flag and the censuses. (W1 has no pipe pairs.)

   *Status 2026-10-11:* layout generator and a first, scenery-free
   renderer are in (`map_source::generated`): frame and entrance stub from
   vanilla, `$B4` ground, `$45`/`$46` paths, `$51`/`$52` rocks, each entry on
   a node showing its own tile, castle top above the airship. Behind the
   hidden `generated_worlds` option (not in the flag key; the web always
   sends 0), set by `testrom --generate-world 1`; needs Hammer Bro shuffle on.
   `generated_w1_is_completable`: 500 of 500 seeds completable, 20 run the
   whole pipeline. **Playtested 2026-10-11 by the project owner (seeds 1 and
   2): works.** Not yet: the renderer oracle test, the route census.
   **Decorations come next, as their own step, before anything
   ships** (project owner).

   **The waypoint, and how it generalizes.** Before the tree grows, a winding
   path runs from the start to a waypoint, so a layout cannot clump around
   the start. For W1 (start top-left) the waypoint lands in a bottom-right
   staircase: the last column on the top row, one more column in per row
   down. That is roughly "far from the start, measured diagonally" (row plus
   column distance), which is the form to switch to when either of these
   arrives, since it needs no notion of right or bottom:
   - **a start that moves**: the far region moves with it;
   - **a wider world** (W3, W6: 3 screens; W8: 4): one waypoint per screen,
     chained, each far from the one before, so no screen is left bare.
   Not identical to today's rule (pure distance would also allow the last
   three spots of the top row), so switching moves W1's output; do it when
   one of the two cases needs it (project owner, 2026-10-11).

   **Decorations: the plan** (2026-10-11, not built). Decorations are
   generated the same way as the layout: shapes are grown, then drawn, and
   the drawing rules are learned from vanilla rather than hand-written.

   - **D1. Empty regions.** After paths and nodes are drawn, the empty cells
     (not node, path or frame) fall into separate regions. Each region is
     filled according to its size and shape: tiny ones (1–2 cells, usually
     wedged between paths) stay ground; medium ones get a cluster; large,
     open ones are candidates for a feature (a lake or a forest).
   - **D2. Clusters.** Single-tile decorations grown as a blob inside a
     region: one seed cell, then neighbours added at random up to a size the
     region allows. No edges to draw, so they always look coherent. Each
     world's decoration tiles are a small hand-written theme table, checked
     against the inventory's scenery counts. `$53` (decorative rock, which
     looks like a breakable one) never touches a path.
   - **D3. Edge rules learned from vanilla.** Lakes and shores are like paths:
     each cell's tile depends on which neighbours are water. Read every water
     and shore cell in vanilla's maps, record "neighbour pattern → tile", and
     check each pattern always gives the same tile (the oracle, in the style
     of the blank-tile test). If it holds, any water shape can be drawn by
     marking the water cells and looking tiles up; a pattern vanilla never
     shows is one the generator does not grow. The same works for any other
     edged feature vanilla has (hills, forest edges), if their patterns are
     consistent.
   - **D4. Lakes and forests.** In a large region, grow a blob (a random
     flood fill from a seed cell, stopping at a size limit and at the cells
     next to paths), then draw it with D3's table. A forest is the same with
     tree tiles; if vanilla's trees have no edge pattern, it is a D2 cluster
     instead.
   - **D5. Rivers and water gaps.** A river is decided with the layout, not
     after it: a band of water cells across the map that avoids nodes. Where
     it crosses a link, the path tile becomes a bridge. **Water gaps then
     come from the builder, not the generator:** a lock the builder places
     on a bridge `$B3` is drawn as water `$9D` until its fortress rebuilds
     it, exactly as W8's bridge row works. Constraint found 2026-10-11: only
     the horizontal bridge `$B3` has a water-gap form; a vertical bridge
     `$BA` locks as a land lock `$54`. So rivers cross links horizontally
     (a river running north–south), or a vertical crossing's bridge is kept
     out of the lock candidates.
   - **D6. Judging it.** ASCII cannot show whether scenery is coherent.
     Judged on real tiles: `testrom` ROMs played or screenshotted (headless
     Mesen), and possibly a PNG render of a map from the ROM's own CHR.

   **The mini tiles `$57`–`$5E`** are W5's miniature ground, drawn for its
   view down from the sky (one each at W5 row 0; `$5E` unused). A possible
   texture for variety, to be tried on real tiles, not a default.

   Order: D1 + D2 first (a decorated, playable W1), then D3 as an experiment
   that decides whether D4 and D5 can draw water, then D4, then D5.
4. **A world with pipes** (W2 or W4).
5. **W3: water and canoe.** Islands are the biggest new mechanic, so they get
   their own milestone.
6. **The rest**, each world's special mechanics as its own question: W5's sky
   page, W7's pipe-heavy budget, W8's darkness, army sprites and anchors.
7. **The flag.** Flag-key bits and a key version bump, web option.

## Vanilla's shape (the generator's targets)

*Measured 2026-10-11 by `map_source::inventory`
(`cargo test --release --lib map_inventory -- --ignored --nocapture`), on the
vanilla source with default flags, gaps and breakable rocks open. The same
function measures a generated `WorldMap`, so a generated world is compared
with these figures by re-running it, not by reading this table.*

| World | Nodes | Per screen | Walk links | Dead ends | Corridors | Junctions | Loops | Walk pieces | Start → target |
|---|---|---|---|---|---|---|---|---|---|
| W1 | 21 | 21.0 | 22 | 3 | 13 | 5 | 2 | 21 | 9 |
| W2 | 47 | 23.5 | 49 | 7 | 30 | 10 | 3 | 47 | 15 |
| W3 | 52 | 17.3 | 52 | 16 | 21 | 15 | 4 | 42, 5, 3, 2 | 15 |
| W4 | 34 | 17.0 | 34 | 10 | 15 | 9 | 2 | 30, 4 | 15 |
| W5 | 42 | 21.0 | 43 | 8 | 24 | 10 | 3 | 22, 20 | 18 |
| W6 | 57 | 19.0 | 61 | 10 | 32 | 15 | 5 | 57 | 23 |
| W7 | 46 | 23.0 | 40 | 16 | 23 | 6 | 1 | 14, 10, 9, 6, 4, 2, 1 | 13 |
| W8 | 41 | 10.2 | 36 | 13 | 25 | 3 | 0 | 17, 8, 7, 6, 3 | 18 |

Dead ends, corridors and junctions count a node's walk links (1, 2, 3+); W7
has one node reached only by pipe. "Walk pieces" are the groups that walking
alone connects; more than one means islands joined by pipe or canoe.
Start → target is in hops, using pipes and canoe.

What it says:

- **Every node has a pointer entry; the spare ones hold Hammer Bro levels.**
  Vanilla fills each node that has no level, fort or other feature with a
  Hammer Bro level, which is why some entries sit on blank-looking tiles. So
  a generated world's node count is its pointer-entry count: spare nodes are
  where Hammer Bro levels go, and fewer nodes than entries starves capacity.
- **Vanilla is close to a tree.** Walk links barely exceed nodes, most nodes
  are corridors, and there are 0-5 loops per world. Loops are where route
  choice comes from, so **this column is a floor to beat, not a target to
  match**: "fresh routes" means more of them than vanilla has.
- **Islands are how W3, W4, W5, W7 and W8 get their structure.** W7 is seven
  pipe-joined pieces and W8 five. A generator that only draws connected maps
  would lose that, so pieces belong in the region graph from the start.
- **Water is three things, and one tile plays two of them.** `$9D` is a gap
  (a lock drawn as water) at one FX cell each in W4, W5 and W6, and plain scenery water
  elsewhere in the same worlds. The renderer cannot tell them apart by tile;
  only the region graph knows which water is a gap.

Tile vocabulary by role (vanilla, all worlds; the test prints per-world
counts):

| Role | Tiles |
|---|---|
| Horizontal path | `45` `49`, island `AC` `B7`, bridges `B3` `B8` `B9`, sky `DA`, `E6` (W8) |
| Vertical path | `46`, island `AA` `AB` `B0`, bridge `BA`, sky `DB` |
| Gap / lock | `54` `56` (locks), `9D` (water), `E4` (sky) |
| Breakable rock | `51` (opens horizontal), `52` (opens vertical) |
| Background | `02` `B4` |

Everything else is scenery: from 9 distinct tiles (36 cells) in W1 to 39 in
W3, and up to 499 cells in W8.

**Found while measuring.** The map walker counts the scenery cell at column 0,
left of each start, as a node: the start's entrance path stub runs off the
left edge and the walker does not check the cell it lands on. Harmless to the
builder (nothing is placed there), so it is left alone; the inventory counts
only entry and blank cells.

## Open questions

- Is the per-world pipe-pair count a hard ROM limit?
- Should a theme be able to move to another world slot (a desert map in slot
  5)? Needs the palette/tileset question answered first.
- Starts as anchors in v1: keep for good, or generate later?
- **Nodes beyond the pointer-entry count.** A generated world uses exactly one
  node per pointer entry (W1: 21 of 35 lattice spots). Measured 2026-10-11:
  finished `8s are Wild` seeds already have **8 blank nodes with no pointer
  entry in W8** on every seed checked (5 of 5), and that mode ships, so a plain
  blank node with no entry looks safe to stand on — which contradicts the
  writer comment that walking onto a tile with no entry crashes. The risk the
  project owner names is the **Hammer Bro march**: a bro landing on such a node
  pulls the player into it. A march veto, like the ones that already keep bros
  off other tiles, would cover it. Until that is checked in the emulator,
  generated worlds keep the entry limit. Two more ways to cover more of the
  screen without entries: extra blank nodes (this), and longer path runs where
  the player stops on path tiles (vanilla W8's bridge row does this).
