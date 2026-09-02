# Mega map — experiment (branch `experiment/mega-map`)

**Status:** working and playable. The randomizer builds completable folded maps
with the normal option set. Branch-only, 11 commits, **not pushed**; the option
is deliberately absent from the flag key. Do not merge to `main` as-is.

```sh
# randomize onto a folded map
smb3-rs <rom> --seed N --mega-map --patched-rom -o out.nes
smb3-rs <rom> --flags <KEY> --mega-map --seed N --patched-rom -o out.nes

# playtest build (open movement clashes with randomizer patches; --no-walk)
testrom --randomize --flags <KEY> --mega --seed N --no-walk \
        --starting-items hammer,leaf,fire --output out.nes

# fold a vanilla base only, for engine-level testing
testrom --mega --output out.nes
```

**Palettes are not seed-derived** (OS randomness — see `NOT_ENCODED`), so two
builds of one seed differ in palette bytes. Add `--no-palettes` for a
byte-reproducible ROM; that is also what every refactor here was verified with.

## What it does

Folds the eight worlds into **three pipe-linked super-worlds**. Every page keeps
its vanilla layout; worlds are concatenated whole, in order, and the seams
*between* source worlds are joined by repurposed pipe pairs.

| Slot | Sources | Pages | Entries | Forts | Sprites |
|---|---|---|---|---|---|
| **6** (start) | W1 + W2 + W3 | 6 | 116 | 4 | 8 |
| **7** | W4 + W5 + W6 | 7 | 129 | 7 | 8 of 10 |
| **8** (Bowser) | W7 + W8 | 6 | 85 | 6 | 8 |

```
  mega map: 3 super-worlds, start in world 6, 10 surplus singletons removed
    link W1|W2 in world 6: pipe 0x12, (0, 12) <-> (0, 20)
    link W2|W3 in world 6: pipe 0x01, (0, 36) <-> (0, 50)
    link W4|W5 in world 7: pipe 0x17, (2, 28) <-> (0, 34)
    link W5|W6 in world 7: pipe 0x03, (0, 62) <-> (4, 68)
    link W7|W8 in world 8: pipe 0x11, (3, 28) <-> (5, 34)
    space: grids 2739/2744 B, blocks 2004/2072 B
    connectivity: every page reachable
```

## Why this shape

**Progression comes free.** `INC World_Num` carries 5 → 6 → 7 unpatched, and the
last group holds Bowser's castle in the slot the ending code expects. The
previous eight-page prototype had to NOP the world advance out and had no
progression at all; here the vanilla chain does the work, and the ending fires.

**Pipes cross parity; walks do not.** Map movement advances two tiles at a time,
so `row % 2` and `col % 2` are each invariant along a walk — and W1–W6 sit on
even rows while W7 and W8 sit on odd. Joining pages *on foot* therefore needs
them shifted into phase; joining them *by pipe* does not, because a teleport
edge has no parity. That one fact deleted, from the previous prototype:

- the row-shift machinery and the fortress FX row/bit rewriting that followed it
- the seam-carving path search (~200 lines)
- the "don't bulldoze a fortress" content-preservation problem the carving created
- the land-themed blank tiles the carving left on sky and island pages

**Only inter-world seams need links.** A source world's pages stay adjacent and
in order, so whatever joined its page 0 to page 1 in vanilla still joins them.
Five links, against 24 pipe pairs in the ROM.

## Slots: the totals are conserved, the partitions shrink

This is the reason three groups fit where the eight-page single world did not.
Nothing is duplicated — the same 19 pages, 340 entries and 17 fortresses are
simply split three ways instead of eight, so each group gets a *larger* share of
every per-world table.

| Resource | 8 worlds | 3 super-worlds | Headroom |
|---|---|---|---|
| Pointer blocks | 2072 B, **exactly** full | 2004 B | 68 B |
| Grid data | 2744 B to the warp zone | 2739 B | 5 B |
| Fortress FX rows | 8 × 4 = 32 B | 4 + 7 + 6 = 17 B | 15 B |
| Fortress FX slots | 17 | 17 forts, total unchanged | 0 |
| Pipe pairs | 24 | 5 spent on links | 19 |
| `Map_Completions` | 4 pages/world | 6, 7, 6 needed | cap is 8 |

**Fortress rows are not fixed at four.** `FortressFXBase_ByWorld` indexes into
them, and the disassembly says so outright: *"there's no need for this to be
precisely four in every world, but that's what they allocated."* A seven-fort
group simply gets a seven-byte row.

### The one thing that does not fit

**Map-object sprite slots.** The per-world list is nine long, so W4+W5+W6's ten
sprites do not fit however the reserved slots are counted.
`per_world_tables_fit_their_groups` asserts the drop count, so the day it
changes the test says so.

**Slot 0's HELP bubble is spent to make it eight rather than seven.** Vanilla
reserves two slots: slot 0 holds `MAPOBJ_HELP` (`$01`) and slot 1 the airship.
The bubble is decoration and nothing else — `PRG011_B657` returns from the
map-object interaction handler the moment it sees the id, so the object is
drawn, animated, and never interacted with. Under a fold, where three source
worlds compete for one nine-slot list, an animation is not worth a Hammer Bro.
Group 7's drop goes 3 → 2.

Only the fold spends it, so a normal seed keeps the bubble on every map. The
mechanism is deliberately not a flag: `is_reserved_map_obj_slot` asks whether
slot 0 *still holds the marker*, which is a question the ROM answers, so
nothing has to track who cleared it.

Slot 1 stays reserved, and the reason is worth writing down because it is
invisible in the data: **every vanilla world's slot 1 reads `$00`**. The
airship map object is populated at runtime, not from the table, so "empty" is
exactly what a reserved slot looks like here — the first cut of this change
freed slot 0 by lowering a `first_usable_map_obj_slot` bound, and the writer
promptly parked a Fire Bro on top of the airship. W8 has no airship and may use
the slot, which is the one case the old bound existed to express.

Widening past eight is still open and is not just a length change: the reward
table is addressed as `MAP_OBJ_REWARDS + world * 9 + slot`, with the stride
baked into the engine as well as into `rom_data`. Three lists of fourteen fit
comfortably in the 72 bytes the eight nine-slot lists occupy, and RAM allows
fourteen (`Map_Objects_*` are 14 bytes each), but both strides have to move
together. (`MAPOBJ_TOTALINIT = $08` is a max *index*, not a count —
`LDY #8 / DEY / BPL` initialises all nine slots.)

## Completions are per-world, which is what makes 6/7/6 pages possible

`Map_Completions` is 128 bytes at `$7D00`, one per map column, split Mario /
Luigi — "allows a MAX of 4 map screens" per the disassembly. For **one player**
it covers eight pages, because a column index for eight pages runs 0..127 and
lands inside the array, the redraw loop already counts to `$80`, and the
game-over clear already runs `LDY #$7F`. Four sites fold the upper half onto the
lower one assuming it is Luigi's; undoing that is **seven operand bytes plus one
six-byte splice**, no free space claimed.

| Site | Vanilla | Becomes |
|---|---|---|
| `Map_Reload_with_Completions` screen index | `AND #$30` | `AND #$70` |
| ...its Mario/Luigi marker select | `AND #$40` | `AND #$00` |
| `MO_DoFortressFX` mirror write | `$7D40,Y` ×2 | `$7D00,Y` ×2 |
| rock-break mirror write | `EOR #$40` | `EOR #$00` |
| game-over clear | AND other player, 64 cols | `LDA #$00`, 128 cols |

The `MO_DoFortressFX` mirror is not cosmetic: `Map_Completions+$40,Y` with a
column past 63 writes to `$7D80` and up, which is `Inventory_Items` — clearing a
fortress on pages 4–7 would have rewritten the player's inventory.

**And the budget is per-world.** `PRG030_84A0`, the world-map initialisation,
clears all 128 bytes, and its only callers are world changes (`INC World_Num`
and the warp-zone destination). Returning from a level enters at `PRG030_84D7`
and skips the clear. So eight pages is a budget *each* super-world gets in full.

The cost is two-player mode, because those columns **are** Luigi's.

## Links: repurpose, verify, and never trust a heuristic

A pipe pair is two pointer entries sharing one `obj_ptr` — the transit level —
matched to a destination slot by comparing entry positions against the positions
in the dest tables. So a *new* pair would need a transit level of its own (a
third entry on an existing `obj_ptr` breaks the "group of exactly two" pairing)
and two more pointer entries. Moving an existing pair costs neither.

Four things had to be right, and each was found by getting it wrong:

1. **Re-aim every pair's dest slot, not just the moved ones.** The fold changes
   every carried entry's page, so a pair left with vanilla page numbers silently
   stops being a pair. This stranded W8's pages 1–3: six internal pipes intact on
   the map, unmatched in the tables.
2. **A mouth must already be reachable.** Placing a pipe on a cell does not make
   the cell reachable. Unchecked mouths produced links that looked right in the
   tables and left pages 3-and-up dark in all three groups.
3. **Mouths usually have to displace something.** Vanilla maps are dense — W1's
   single page has 21 entries and not one spare blank node. So the pipe and a
   level **trade places**: the level keeps its `obj_ptr` and lands where the pipe
   was, which was a reachable node by construction.
4. **Which pair to spend cannot be guessed.** "Same page" seemed safe — a pair
   whose mouths share a page cannot be the only thing joining two pages. It is
   still wrong: W2's single pipe has both mouths on its first page and is the
   only thing joining two *regions* of it. Taking it stranded ten nodes.

So the choice is verified, not reasoned: each candidate is moved on a cloned
plan and kept only if the group's **connected-component count strictly
decreases**. A link joins two components; if the move also severs something, the
split cancels the join and the count comes back level.

## Start and goal: two things the tile does not carry

Both found by looking at the map in an emulator, and neither is visible in the
tile data.

**The spawn coordinate is a per-world table.** `Map_Y_Starts` is eight bytes
indexed by `World_Num` — "Map Y start positions, World 1-8 (X is always $20)" —
so a group living in slot 6 spawned Mario at **W6's** row while its start panel
sat at W1's. Vanilla's rows all differ (`$40 $A0 $A0 $40 $80 $60 $30 $50`), so
there is no row to inherit. `aim_spawn_at_the_kept_start` writes the row of the
start tile the fold actually kept: slots 6/7/8 become `$40 / $40 / $30`.

Only the row was wrong, and that is the tell: X and X-Hi are not in a table at
all, the engine hardcodes `$20` — column 2 of page 0. Every vanilla start
happens to sit there and the kept start is always the leftmost, so it holds; but
it is an engine assumption rather than data the fold controls, so it is checked
and errors out rather than being trusted. Moving a start off column 2 needs the
X / X-Hi / scroll tables `start_airship_swap` adds.

**The last group had two goals.** W7's airship castle (`0xC9`) *and* W8's Bowser
castle (`0xCC`) are both enterable, and clearing an airship castle runs
`INC World_Num` — so a player reaching W7's castle on page 1 would have finished
world 8 early and advanced into world 9, the warp zone. When Bowser is present
he is the goal and every airship castle in the group goes.

## Ordering

`testrom` applies the fold at **step 5a**: after `open_map` (which walks all
eight `MAP_TILE_GRIDS` rows, whose offsets land at the wrong stride once the
fold has run) and after the open-movement patch (whose practice-ROM records
include two bytes of map-object data the fold also rewrites).

Inside the module the mirror rule holds: `plan()` reads *everything* out of the
vanilla layout before a single byte is written, because the merged grids are laid
out from W1's slot and run over W2, W3 and W4's sources. This is the same trap
the world-merge experiment hit from the other direction — there the merge had to
run *first*; here it must run *last*, because this module reads the vanilla
tables rather than the merged ones.

## Tests

`cargo test --lib mega_map` — all skip when the ROM is absent.

| Test | What it holds |
|---|---|
| `carries_every_entry_but_the_surplus_singletons` | all 340 entries accounted for |
| `blocks_are_sorted_and_init_index_is_exact` | `(page,row,col)` order; `InitIndex[p]` is the *first* entry on `p` |
| `every_page_is_reachable` | all 19 pages walkable from their group's start |
| `nothing_becomes_less_reachable_than_vanilla` | the real invariant — see below |
| `every_fortress_survives_and_is_reachable` | all 17 forts exist and are walkable |
| `links_join_two_pages_with_real_pipes` | 5 links, mouths on different pages, both pipes |
| `per_world_tables_fit_their_groups` | every table budget, including the sprite overflow |
| `stays_inside_its_regions` | no write past the warp grid or the block region |
| `progression_chain_is_intact` | starts in world 6; groups ascend; last is world 8 |
| `spawn_lands_on_the_start_tile` | `Map_Y_Starts[slot]` names the kept start's row |
| `each_group_has_exactly_one_goal` | one airship castle per group; the last has Bowser and none |
| `completion_widening_refuses_an_unexpected_rom` | mutates all 8 patch sites, guard fires |

`nothing_becomes_less_reachable_than_vanilla` is the one that matters. "Every
page is reachable" can hold while half a page is stranded; this compares each
source world's own vanilla reachable set, remapped, against the merged one.

It was also wrong twice before it was right — first handing vanilla *all 24* dest
positions filtered only by "does it fit in the grid", which lets other worlds'
pipes in as phantom shortcuts and makes vanilla look better connected than it is.
Pipes must be filtered by `DEST_TO_WORLD`, not by coordinate range.

## Known gaps in the fold itself

Distinct from the "what next" list at the end — these are things the fold does
imperfectly rather than work not started.

- **Hammer Bro rewards travel with the sprite** (fixed). Position and id live
  in per-world sub-tables reached through a master pointer; the reward byte is
  a flat `MAP_OBJ_REWARDS + world * 9 + slot`. Carrying a sprite moved the
  first two and not the third, so a bro landing past slot 4 inherited the
  destination world's vanilla `$00` — an encounter that hands out nothing.
  Reported from play as "hammer bros giving out empty items". The pool
  `collect_hb_sprite_rewards` builds was polluted the same way, and now reads
  only live world slots: dead slots keep their vanilla sprite sub-tables (only
  the map/pointer masters get re-aimed), so `0..8` counted every carried
  encounter twice.
- **Airship sprite placement.** Each group keeps the destination world's own
  airship sprite (map-object slot 1), which sits at *that* world's vanilla
  castle rather than at the castle the fold kept (the rightmost). The castle
  tile and its pointer entry are right; the sprite is not.
- **Two sprites dropped** in the W4+W5+W6 group — eight usable map-object slots
  against ten, after slot 0's HELP bubble was reclaimed.
  `per_world_tables_fit_their_groups` asserts the number, so the day it changes
  the test says so.
- **The warp zone** is left in place, its destinations naming worlds 1-8.

## Phase 2 (done): the layout is read from the ROM, not hardcoded

`rom_data::MapLayout` describes any ROM's map layout by reading it back out of
the master pointer tables, rather than asserting the vanilla shape from
`WORLDS` and `MAP_TILE_GRIDS`. Everything those constants say is already in the
ROM: the masters give each world's block, and `entry_count` falls out of the gap
between the RowType and ScrCol pointers, which are adjacent by construction.

Two subtleties worth keeping:

- **Page count comes from the entries, not from the grid data.** Scanning for
  the grid's `0xFF` terminator is wrong, because `0xFF` is also a real map tile
  (the border) and a scan can stop early. Every page carries at least one entry
  in both vanilla and the fold, so the highest page any entry names is the last.
- **Liveness cannot be inferred.** A folded ROM aims its dead slots at a live
  world's tables rather than at garbage — safe precisely because they are never
  loaded, but it means the caller has to say which slots are real.

`layout_matches_the_vanilla_constants` pins the reader against the constants on
a vanilla ROM. That is what lets the 46 call sites migrate a few at a time
instead of in a flag day: while it holds, the two describe the same thing.
`mega_map::read_grid` is the first real consumer, and reading geometry back from
the ROM makes its verification stronger — it now checks what was *written*
rather than agreeing with its own intent.

### Also in Phase 2: the fold no longer needs locks removed first

`plan_grid` holds locks and water gaps **open** while planning links. Only for
planning; the grid written keeps every lock. A lock is a gate the player opens,
not a wall, so treating one as impassable measures the wrong map — it splits a
world into regions that are not really separate, and then no pipe can be spared
without appearing to strand something.

This was found by calling the fold from a test that did not pre-remove locks: it
failed with "no pipe pair it can spare to link W2 to W3". `testrom` removes
locks first and the randomizer does not, so the fold had a hidden dependency on
its only caller. Same rule the fortress-forcedness work settled on — hold locks
open, or you measure goal gates.

## Phase 3 (done): the randomizer runs on a folded map

`--mega-map` on the CLI (and `Options::mega_map`) folds the map **before the
catalog** and after every QoL map pass — the QoL patches carry fixed vanilla
coordinates, so they must land on the vanilla grids and be carried across, while
the catalog and everything after it must see the folded layout.

The whole pipeline runs: catalog, pickup, build, write. It produces a coherent
map — content spread across all six or seven pages, start on its panel, goal at
the far end. A vanilla run stays byte-identical (`de503b5a`, seed 12345
`--no-palettes`), which is how every step was checked.

`mega_map` is **not in the flag key**. The key is versioned and shared with the
seed bot, and a seed built with this is not comparable to one without, so it
wants its own decision about encoding once the shape settles rather than a bit
spent while it is an experiment.

### What the migration actually cost

Much less than the 46-reference estimate, because engine slots stay 0–7: arrays
keyed *by slot* stay `[T; 8]`, and only iteration and layout lookups changed.
`MapLayout` grew the `(world, entry)`-keyed tables — fortresses, airships,
Bowser, the spiral pair, sprite links, pipe destinations — and `mega_map` emits
an `EntryRemap` they are moved through. `BOOMBOOM_Y_OFFSETS` needed nothing: it
is keyed by `obj_ptr`, which the fold preserves.

Four vanilla assumptions surfaced only when a folded map hit them:

- **`adj_span` indexed at a hardcoded 64-column stride** ("grids are at most 64
  wide"). Now `COL_STRIDE = 128`, the engine's real ceiling — `Map_Completions`
  is one byte per column over 128 columns, so no map can be wider.
- **`pack()`'s debug assert** carried the same 64. The column field was always
  eight bits wide; only the bound was stale. It fires in tests and not in
  release, so a release run had been silently packing out-of-range columns.
- **`VANILLA_PIPE_PAIRS[slot]`** is indexed by slot but *describes vanilla
  worlds*, so a folded slot 6 was handed W7's eight pairs when it really has
  six. Now `pipe_budget()` counts them from the layout.
- **`VAN_FX` had 16 rows against `FORTRESS_ENTRIES`' 17** — hand-written
  duplication that silently dropped W8's fourth fortress, so its FX never
  fired. Derived from the table now: the FX slot index *is* the index into it.

`redistribute_fortresses` and `deal_c1_floors` are the two budgets keyed by
world *count* rather than by slot. The floor deal generalises cleanly (and is a
no-op at eight). Fortress redistribution does not: its 1–3 span and its "W8
keeps 4" are vanilla's shape, so off the vanilla eight it keeps the roster the
layout already describes and spends no RNG — moving forts between super-worlds
is a variety knob that needs a census of the new shape to justify.

### The pipe budget is fine — and the story of getting that wrong

A super-world inherits exactly the sum of its source worlds' pipe pairs (4 / 6 /
14), pinned by `pipe_budget_is_the_sum_of_the_merged_worlds`. That turns out to
be enough, because **merging does not create islands** — it puts existing ones
in one world. `island_budget_for_the_builder` measures content-bearing walk
components with every gate open and all pipes removed:

| group | islands | bridges | budget | slack |
|---|---|---|---|---|
| 6 (W1+W2+W3) | 4 = 1+1+2 | 3 | 4 | **+1** |
| 7 (W4+W5+W6) | 5 = 2+2+1 | 4 | 6 | **+2** |
| 8 (W7+W8) | 12 = 7+5 | 11 | 14 | **+3** |

Straight sums of the vanilla per-world counts, every group in credit.

This is worth recording because the first three attempts at it all said the
opposite — a 14-pair deficit that would have been a genuine design crisis. Every
one was the **same mistake**: measuring connectivity with gates *shut*. Locks
and breakable rocks split a map into pockets that are gated, not separate, and
counting those as islands roughly triples the number. It appeared three times in
different clothes:

1. `plan_grid` planning links against a locks-shut graph, so no pipe could be
   spared without appearing to strand something.
2. The island census measuring the fold with gates shut against a **vanilla
   control that held them open** — an asymmetry that produced the fake −14.
3. `mega_map_builds_completable_super_worlds` walking the written ROM, where
   locks are stamped closed. The builder's own convention is that a lock is
   stored as its open path tile with the closed state in a separate overlay,
   which is why `all_world_targets_reachable` walks `built.grid`.

The rule, now stated everywhere it applies: **a gate is not a wall.** Measure
connectivity with locks and breakable rocks open, or measure something else.

With the asymmetry removed, `mega_map_builds_completable_super_worlds` passes:
eight seeds, three super-worlds each, every goal reachable from its start.

## Flags

The whole option set applies. A kitchen sink — world order, start/airship swap,
8s are Wild, troll pipes, piranha shuffle, more hammer rocks, hammer breaks
locks and bridges, antechamber shuffle, big-Q rooms and blocks, beta stages,
wild injections, random fire flower, poison mushrooms, modern powerups, anchor
visuals — builds clean, and
`mega_map_builds_completable_super_worlds` runs four arms (plain, start/airship
swap, 8s-are-Wild + piranha, beta stages) asserting every goal stays reachable.

Three flag-level things had to be fixed, all of the same family — *something
keyed to eight worlds*:

- **`--flags` clobbered `--mega-map`.** The option is not in the key, so
  decoding reset it to false. It overlays a decoded key now, like
  `palette_themed` and the other non-encoded options.
- **`open_map` iterated the vanilla `MAP_TILE_GRIDS`**, so on an
  already-folded ROM removing locks would scribble path tiles through the
  merged grids at the wrong stride. It takes a `MapLayout`.
- **World order shuffled all eight worlds.** Found in play: start in world 6,
  clear it, arrive in world 4. The next-world table is indexed by `World_Num`,
  so it sent the player into a dead slot. It takes the live world list now,
  shuffling only the worlds before the goal. World order also gets the *last*
  say on the starting world — the fold runs later and stamps its own, so the
  orchestrator restores world order's choice afterwards, which lets it
  legitimately permute which super-world comes first.

Still unverified: **`--world-count`** ("worlds before Dark Land, 1-7") now
clamps to the two non-goal super-worlds, so values above 2 silently do nothing.
Coherent, but it probably wants a clearer meaning here.

### The start ↔ airship swap is suppressed under the fold

Found in play, and it is the fifth instance of the same family — *something
keyed to eight worlds*. `build_position_tables` frames the camera against
`MAP_TILE_GRIDS[slot].columns`, the **vanilla** world that used to live in the
slot (48 / 32 / 64), while the super-world actually there is 96 / 112 / 96 wide.
Its `col.clamp(0, cols - 16)` therefore pins the camera up to four pages left
of a swapped start. `W5_IDX`'s static-screen special case is keyed to slot 4,
which no folded map uses, so all three groups take the smooth-scroll branch
whether that suits them or not.

`--mega-map` turns the option off rather than shipping that, and
`start_airship_swap_is_suppressed_by_the_fold` pins both halves: the folded ROM
keeps vanilla's `STA $0724,X` at `MAP_INIT_SCROLL_SITE`, and an unfolded one
with the same option still patches it — so the test cannot pass by SAS having
quietly stopped working everywhere.

Skipping `pick_swaps` also skips its seven coin flips and shifts the stream.
That is fine: no folded seed is comparable to an unfolded one anyway.

The fix, when it is worth doing, is the one `open_map` already had: the four
`FS_SAS_*` tables take a `MapLayout` instead of the vanilla constants.

## Picking this back up

Everything is committed on `experiment/mega-map`; the tree is clean. Nothing is
pushed, and the ROMs under `roms/` are gitignored — all regenerable from the
commands at the top.

The next things, roughly in order of value:

1. **Play it.** The engine-level questions are settled (it pans, completions
   track, progression chains W6→W7→W8) but nobody has played a full folded seed
   through to Bowser.
2. **Per-region dressing.** Palette, music, bottom tile, king room and airship
   level are keyed to `World_Num`, so each super-world wears its *destination*
   slot's clothes — W6's, W7's, W8's — rather than one set per page. The
   `Map_Region` latch (a spare WRAM byte holding the current page, with the
   eight-wide dispatch tables re-indexed off it) is the fix, and it is mostly
   same-size operand swaps.
3. **Two sprites dropped** in the W4+W5+W6 group: eight usable map-object slots
   against ten sprites. Widening past eight needs the reward stride
   (`MAP_OBJ_REWARDS + world * 9 + slot`) moved in the engine *and* `rom_data`
   together.
4. **Re-measure the C1 floors.** The 11/14/17 band was derived for maps of
   16-64 columns; a super-world is 96-112. `deal_c1_floors` already takes its
   arity from the layout, but the *values* want a census of the new shape.
   `redistribute_fortresses` likewise keeps the layout's own roster off the
   vanilla eight rather than inventing a distribution.
5. **The warp zone** still names worlds 1-8, most of which no longer exist.
6. **Flag key encoding**, if this ever ships.

## Phase 3 notes: what integrating with the randomizer needed

Less is per-world than it feels. Already page-agnostic:

- `walk_map` — proven on 96- and 112-column maps here
- the level deck / `assign_pool` — already deals **globally**
- `redistribute_fortresses` — already cross-world
- `map_tile_offset` — screen-major, width-driven
- the island-role model in `islands.rs` — and **vanilla pages placed side by side
  are exactly islands**, which is what the builder's phase 1 ("connectivity pipes
  bridge islands") already exists to solve

Genuinely per-world: the `[T; 8]` tables and `0..8` loops, the dealt C1 floors
(11/14/17), the per-world route census, `world_order`, and the capacity model
(`VANILLA_PIPE_PAIRS[w]`, fortress counts).

The wedge is the one the world-merge branch identified and did not take: **make
the world count variable**. Phase 2 landed the foundation — `MapLayout` already
has `world_count()` and knows which slots are live. What remains is migrating
the 46 references to `WORLDS` / `MAP_TILE_GRIDS` across 17 files to take a
`&MapLayout` instead, so dead slots are genuinely absent rather than emptied. That branch's own conclusion was
"emptying world 1 makes that safe, not correct," and it is the same workstream as
the progression chain and `world_order`. Doing it once unblocks both experiments.

Do not rescale the C1 floors by arithmetic. Three worlds at ~3× the columns is a
different distribution, not a stretched one — get them building, then run
`test_route_census` at 1000 seeds and read what it says before touching a knob.
