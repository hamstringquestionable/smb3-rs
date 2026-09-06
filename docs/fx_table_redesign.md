# Fortress FX table redesign

**Status:** stage 0 landed (a Rust-only derivation proof); no ROM change yet.
Written 2026-09-06 on
`experiment/world-maze`, revised 2026-09-06 after a second design pass, so a
fresh session can pick it up cold.

**Read first:** `docs/world_maze_design.md` for the mode this serves, and
`src/randomize/overworld_writer/fortress_fx.rs` for the code being replaced.

---

## Why this exists

Locks and cross-world locks are currently two mechanisms that both open a lock,
keyed differently, and keeping them in step has already produced one bug that
survived the whole life of the mode (a re-keyed lock kept its local key as well,
so every cross-world lock was decoration — fixed 2026-09-06, guarded by
`every_maze_lock_has_exactly_one_key`).

Working *within* vanilla's FX scheme also costs us three limits we do not want:

- **4 locks per world.** `FortressFX_W1..W8` are four bytes each.
- **17 FX slots total**, and the Boom-Boom parameter that selects one is a
  **nibble**, so a flat index could address only 15.
- **Slot 0 is ambiguous.** `FX_WORLD_TABLE` writes `0x00` both for "slot 0" and
  "unused", so World 1's row reads `[0,0,0,0]` whether it holds one lock or
  none.

**The vanilla census saturates every one of those caps exactly.**
`FORTRESS_ENTRIES` is 17 entries and W8 holds 4 — 17 fortresses against 17 FX
slots, 4 against a 4-wide per-world row. There is no headroom at all: standard
mode can have exactly one lock per fortress, forever, and cannot key a lock to
anything that is not a fortress. That is the argument for doing this beyond the
maze.

---

## What is there today

Vanilla's chain, verified against the Southbird disassembly:

1. **`ObjInit_BoomBoom`** (PRG003) copies the enemy record's `Objects_YHi` into
   `Objects_Var4`, then forces the spawn Y-hi to `1`. The Y byte's **high
   nibble** is therefore a smuggled parameter, not a position; the low nibble is
   the real Y. Our writer stamps it as `(ordinal << 4) | (old_y & 0x0F)`.
2. **The `(?)` orb**, not Boom-Boom's death, hands it over:
   `LDA Objects_Var4,X / STA Map_DoFortressFX`. It is **1-based**; `0` means
   "no effect".
3. **`MO_DoFortressFX`** (PRG010, map dispatch entry 8) resolves it:

   ```asm
   DEC Map_DoFortressFX          ; -> 0-based per-world ordinal
   LDY World_Num
   LDA FortressFXBase_ByWorld,Y
   ADD Map_DoFortressFX
   TAY
   LDA FortressFX_W1,Y           ; -> the GLOBAL slot index 0..$10
   STA Map_DoFortressFX
   ```

4. The **slot** then indexes every data table.

### The block being replaced

One contiguous run, all of it ours to rewrite:

| Offset | Table | Bytes |
|---|---|---|
| `0x147CD` | `FortressFX_VAddrH` | 17 |
| `0x147DE` | `FortressFX_VAddrL` | 17 |
| `0x147EF` | `FortressFX_MapCompIdx` | 34 (17 x 2) |
| `0x14811` | `FortressFX_Patterns` | 68 (17 x 4) |
| `0x14855` | `FortressFX_MapLocationRow` | 17 |
| `0x14866` | `FortressFX_MapLocation` | 17 |
| `0x14877` | `FortressFX_MapTileReplace` | 17 |
| `0x14888` | `FortressFX_W1..W8` | 32 |
| `0x148A8` | `FortressFXBase_ByWorld` | 17 |
| | **total `0x147CD`..`0x148B9`** | **236** |

Plus `FS_MAZE_FOREIGN_LOCK` (PRG011, 128 reserved) which this absorbs.

**The block is sited where the code that uses it lives.** PRG010's file base is
`0x14010` at CPU `$C000`, so the run is CPU **`$C7BD..$C8A9`** — 236 contiguous
bytes ending three bytes before `MO_DoFortressFX` at `$C8AC`. PRG010's largest
current gap is 64 bytes, so this would be by far the biggest usable run in the
map bank, and it is adjacent to its consumer.

### Most of that is cached arithmetic, not data

| Stored | Actually a function of |
|---|---|
| `VAddrH/L` (34) | `0x2880 + row*64 + (col%16)*2` |
| `MapCompIdx` (34) | column is the map column; bit is `Map_CompleteBit[row.min(7)]`, a table the engine already has and already indexes in `Map_MarkLevelComplete` |
| `Patterns` (68) | the metatile quadrant tables, indexed by the replacement tile |
| `MapTileReplace` (17) | the current tile, via `Map_Removable_Tiles` -> `Map_RemoveTo_Tiles` |

That is 153 of the 187 bytes of slot data redundant with the target position.

**Stage 0 measured this** (2026-09-06, `overworld_writer::fortress_fx::derivation`).
The answer is "mostly, and the exceptions are worth knowing":

- **`VAddrH/L` and `MapCompIdx` (68 bytes) always derive** — vanilla and every
  randomized ROM, no exceptions. Stage 1 can delete both outright.
- **`Patterns` (68 bytes) always derive** from the metatile quadrants, with one
  vanilla slot excepted: `$0F`, W8's dark screen, stores `FF FF FF FF` ("all
  black square in the dark") rather than the quadrants of the tile it writes.
  That slot's patterns carry information the target cell does not. Our writer
  never reproduced it — `fx_patterns_for` has no darkness case — so nothing is
  lost by deriving them; the `World_8_Dark` gate in the screen check already
  covers that page.
- **`fx_patterns_for`'s three hand-written cases agree exactly** with the
  quadrants of the three plain tiles `Map_RemoveTo_Tiles` names. It is a
  hand-copy of a table the ROM already holds, and stage 1 deletes it.
- **`MapTileReplace` does *not* derive**, and this is the one real decision
  stage 1 has to make. See below. Vanilla has a single instance of its own:
  slot `$08` (W6, row 4 col 13) sits on plain ground — the corridor around it is
  `$42/$45/$47` and no `$DA` appears anywhere on that screen — yet it stores
  `$DA`, the *sky* path, which is what the adjacent slot `$07` legitimately
  stores for the game's one real sky lock (W5, cell `$E4`, cloud neighbours).
  Deriving fixes it. Nobody has seen it because `$45` and `$DA` share CHR
  quadrants and the effect writes no attribute byte, so the frame is right
  either way and the reload restores `$45`.

### The replacement tile is where the derivation actually breaks

`Map_Removable_Tiles` pairs each obstacle with the path its own terrain wants:
`$54 -> $46` and `$56 -> $45` for ground, `$E4 -> $DA` for sky, `$9D -> $B3` for
water. The sky pair exists exactly so a lock in the clouds reveals a cloud path
rather than a ground one.

The builder, however, stores the *original* tile it covered — a drawbridge, a
path variant, a sky path — so the stored byte is frequently a tile the engine's
table cannot name. Measured over 200 seeds: **562 of 3362 slots (16.7%) keep a
variant**; `$B7`, `$BA` and `$DB` are the common ones.

The patterns give the game away — they are the **plain** tile's quadrants in
every case, variant or not. So today's ROM already shows three different tiles
at one cell over time: the plain graphic during the effect, the variant once map
RAM is redrawn, and the plain tile again after the next map load, because
`Map_Reload_with_Completions` maps `$54` back to `$46` regardless. The variant
survives only until the player leaves the map.

Deriving the write from `Map_RemoveTo_Tiles` therefore does not lose a stable
behaviour — it makes all three agree on the plain tile, which is what a reload
produces anyway.

### Decision: stage 1 derives, and the terrain mismatch stays (2026-09-06)

The interesting case is sky. `smb3.asm` names the whole vocabulary:

```
TILE_LOCKVERT    = $54  ->  TILE_VERTPATH    = $46
TILE_LOCKHORZ    = $56  ->  TILE_HORZPATH    = $45
TILE_ALTLOCK     = $E4  ->  TILE_HORZPATHSKY = $DA
TILE_VERTPATHSKY = $DB  <-  nothing removes to it
```

`$DB` exists, so the *destination* is not the missing piece — the missing piece
is a **gap tile** for it. `$E4` is the only sky lock and it is hard-paired to
the horizontal sky path, because Nintendo never needed a vertical one. So when
the builder locks a `$DB` cell it writes `$54`, a ground lock in the clouds, and
the reload later reveals `$46`, a ground path in the clouds. Measured: **126 of
3362 slots over 200 seeds (3.7%)**.

That mismatch is **not created by this rework** — the reload has it today. What
stage 1 changes is that it becomes visible immediately rather than one map load
later, since the effect stops writing the correct `$DB` first.

**Decided: leave it.** Deriving is the right long-term shape precisely because
the gap tile *should* determine the terrain: once a sky-vertical gap tile
exists, derivation is automatically correct everywhere, whereas a per-entry tile
byte would let the effect and the reload drift apart again — the same
two-sources-of-truth shape this whole rework exists to remove. Expanding
`Map_Removable_Tiles` for that tile is a **future enhancement** (stage 3), not a
stage 1 obligation.

Two things it would need, recorded so the enhancement starts warm:

- a new metatile for the lock itself — `$54` and `$56` differ by which sides the
  path stubs attach to and `$E4` is the horizontal shape, so this is art, not
  just a table row;
- a new row in the removable tables, which the disassembly explicitly invites:
  `MRT_END ; marker to calculate size -- allows user expansion of
  Map_Removable_Tiles`, and the loop bound at `prg012.asm:359` assembles from
  that marker rather than being a hardcoded `7`.

A zero-ROM-cost interim exists if the mismatch ever looks worse than it reads —
have the builder decline to lock `$DB` cells. It costs lock sites in W5's sky,
so it moves route choice and would need the two deep censuses, which is why it
is not the default.

The census in `randomized_fx_tile_and_patterns_diverge_only_by_path_variant` is
the number to re-measure if any of this is revisited.

---

## The design

**One position-keyed table replaces both the FX slots and the foreign-lock
table.**

```
entry = key_world_row_col : target_world_row_col        4 bytes
```

Same encoding the foreign-lock entries already use: `(row << 3) | world` in one
byte, column in the next (6 bits). Call these **table entries**, never "rows" —
"row" is the map row inside the key.

Read it from **`Map_MarkLevelComplete`** (PRG011), which already computes
`(world, row, column)` from the player's position for its own completion write —
`Y` holds the column and `X` holds `Temp_Var13`, the row. Scan for entries whose
key matches; for each hit:

- **target world == current world** — arm `Map_DoFortressFX` and let map
  operation 8 do the rest (see below).
- **target world != current world** — set the bit in the packed store
  (`completion_bits`) and skip the animation. Nobody is there to see it.

Local and cross-world stop being two mechanisms. They differ by one comparison.

### The dispatch ordering is what makes this work

Verified in `prg010.asm:905`:

```
7 - MO_DoLevelClear     ; the completion effect (poof / panel flip)
8 - MO_DoFortressFX     ; the lock break
```

`MO_DoLevelClear` ends at `PRG011_AA58`, which swaps the tile to rubble, calls
`Map_MarkLevelComplete`, and then falls through to `INC Map_Operation` — landing
on op 8 the next frame. **Arming `Map_DoFortressFX` from inside
`Map_MarkLevelComplete` puts the write exactly one operation ahead of its
consumer.** No new dispatch hook, no timing gamble, and the in-level orb becomes
pure legacy. Op 8 never calls back into `Map_MarkLevelComplete` — it clears both
vars at `PRG010_C9C9` and advances — so there is no re-entry loop.

### Op 8 keeps ownership of the local completion bit

At `$C96C` op 8 gates on the bit and then sets it:

```asm
LDA Map_Completions,Y
AND <Temp_Var12
BNE PRG010_C9C9    ; already busted -> skip the whole effect
...
ORA <Temp_Var12    ; otherwise set it, for both players
```

The scan must therefore **not** set the bit for a local target. If it did, that
gate would fire and nothing would ever animate. Cross-world targets are the
opposite case: the scan sets their bit itself, because op 8 never runs for them.

### What the target becomes is not stored

The engine already carries the mapping, and `Map_Reload_with_Completions` already
uses it on every map load:

```asm
Map_Removable_Tiles:                            ; $A437, 8 entries
  .byte TILE_ROCKBREAKH, TILE_ROCKBREAKV, TILE_LOCKVERT, TILE_FORT,
        TILE_ALTFORT, TILE_ALTLOCK, TILE_LOCKHORZ, TILE_RIVERVERT
Map_RemoveTo_Tiles:                             ; $A43F, 8 entries
  .byte TILE_HORZPATH, TILE_VERTPATH, TILE_VERTPATH, TILE_FORTRUBBLE,
        TILE_ALTRUBBLE, TILE_HORZPATHSKY, TILE_HORZPATH, TILE_BRIDGE
```

Look up the current tile at the target, write the parallel entry. **This also
removes a bug class:** today the immediate change (`MapTileReplace`) and the
after-reload change (`Map_RemoveTo_Tiles`) are independent sources of truth that
happen to agree. Deriving both from one table means they cannot drift — the same
failure shape as a lock having two keys.

### But the sources are not banked in — mirror 48 bytes

`Map_Removable_Tiles`/`Map_RemoveTo_Tiles` (`$A437`/`$A43F`) and the metatile
quadrant tables (`PRG012_FILE_BASE`, UL/LL/UR/LR x 256 — see
`overworld_writer/metatiles.rs`) are **all in PRG012**. During map play `$A000`
is PRG011 and `$C000` is PRG010; **PRG012 is not mapped**. Neither lookup is
reachable from `MO_DoFortressFX` without a bank swap.

The fix is cheap and improves the design. Mirror into the freed PRG010 block:

| Mirror | Bytes |
|---|---|
| `Map_Removable_Tiles` | 8 |
| `Map_RemoveTo_Tiles` | 8 |
| 4 pattern bytes per removable-to tile | 32 |
| | **48** |

One index then drives the tile swap **and** the CHR patterns, collapsing two of
the three drift sources into one. 48 bytes replaces 85 (`Patterns` 68 +
`MapTileReplace` 17) and scales with the tile vocabulary rather than the lock
count.

The mirror is written at build time from PRG012's copy, with a Rust equality
assertion so the two cannot drift. A later expansion of the removable tables
updates one Rust constant and both copies follow.

### Budget

236 bytes freed in PRG010, spent on: the entry table (4 bytes per lock), the
48-byte mirror, the rewritten `MO_DoFortressFX`, and the rewritten screen check.
Also frees the 128-byte PRG011 allocation. The win is capability, not bytes —
budget roughly neutral.

Per `rom_space_discipline`: write the routine as tight as it goes, then reserve
generously inside the 236-byte block so a later change adds entries without
relocating. Note both numbers on the `FREE_SPACE_ALLOCATIONS` row.

---

## The screen check must move in the same commit

`patch_fortress_fx_screen_check` (82 bytes at `$D544`, allocation 112) reads
`$0745` (the resolved slot) and `$C856,Y` (`FortressFX_MapLocation`). It cannot
survive the addressing switch untouched, and it is the fiddliest code in the
subsystem (issue #131, W8 darkness, the +/-1 half-screen straddle).

### What gets simpler

- **The half-screen index becomes a shift.** The whole point of the
  `TXA / ASL / AND #$06 / ADC #$00` block is reconstructing
  `2*screen + (col_in_screen >= 8)` from the packed `loc` byte. With the raw
  column in hand that quantity *is* `col >> 3`, since
  `col = screen*16 + col_in_screen`. Three `LSR`s, and the 6-byte table lookup
  that fed it (`LDY $0745 / LDA $C856,Y`) goes away.
- **No world test is needed** — the cross-world branch returned before this.
- **`FX_MAP_LOC_ROW`'s "low nibble must be zero" rule disappears.** The engine
  ORs that nibble into the map-data write column at `$C99B`, which is what
  softlocked `d608bf1` when a poof-only flag was smuggled into bit 0. Under
  position keying the row byte is ours and nothing ORs it into a column.
- **It stops being a hook.** Since `MO_DoFortressFX` is being rewritten
  wholesale, the check becomes inline code in a routine we own — no `$C8E6`
  displacement, no `$0745` read.

Expect 6-10 bytes off the 82, plus the 6 bytes of lookup. The real value is that
nothing in it references the slot tables, which is what makes them deletable.

### What must NOT change: visibility is identity, not distance

The tempting simplification — "we have coordinates now, just compute distance
from Mario" — is wrong, and three in-house attempts (beta.6/7/8) already failed
by reasoning from position.

The VRAM address the effect writes is

```rust
let vram = (0x2880 + ob_row * 64 + col_in_screen * 2) as u16;
```

**There is no screen term.** Every screen's column 5 writes the same VRAM
address. The map is drawn into a nametable window that the screens alias onto,
so a lock 16 columns away does not merely fail a distance test — it computes an
address currently occupied by a *visible tile from a different screen*. Writing
it replaces the wrong cell on screen: a corrupted map, not a missed animation.

The predicate is therefore "does the nametable right now hold **this** screen at
that address" — an identity question, which the half-screen index answers. The
`+/-1` adjacency block (scroll straddle) and the `(col<<4) EOR $FD` edge filter
(columns 0 and 15 clipping across the boundary) are about the scroll, not the
addressing being replaced. **Keep both byte-for-byte**; let the rewrite touch
only the index computation above them. The `World_8_Dark` gate also survives
verbatim — it reads engine state, not FX tables.

---

## Scope

**This work is the FX rework and nothing else** (decided 2026-09-06). The output
ROM is behaviourally identical to today's: one lock per fortress, at most 4 per
world, fortress keys only. Deja vu on forts, more than 4 forts per world,
levels-as-keys and new obstacle tiles are all deferred.

Three things are **not** features and cannot be deferred:

1. **Absorbing `foreign_locks` is forced.** It displaces the
   `STA Map_Completions,Y` at `$BA86` inside `Map_MarkLevelComplete` — exactly
   where the position scan goes. Two scans fighting over one displaced
   instruction is not worth building. Maze cross-world locks ride through this
   change; `every_maze_lock_has_exactly_one_key` and
   `every_cross_world_lock_names_a_crumbling_fortress` are the regression net.
2. **Zeroing the Boom-Boom Y-nibbles.** Once the orb path is retired, a stale
   high nibble arms a stale slot. The writer must stamp `old_y & 0x0F` for all
   17 fortresses.
3. **Replacing the W8 bridge channel.** `apply_w8_bridges` writes
   `FX_MAP_TILE_REPLACE + 16` so `open_fx_gaps` reads it back during pickup and
   does not clobber the bridge tile — a table-mediated handshake between two
   modules. Deleting the table breaks it **quietly**: a wrong tile on W8's
   bridge row, not a crash. It needs a direct path instead.

One thing worth doing that ships no feature:

4. **Prove the multi-entry scan on the emulated CPU.** The rework's output
   exercises only the vanilla shape, so every path it exists to enable would be
   live-but-never-taken code. `foreign_locks` already has the pattern —
   `execute_the_row_key_against_the_engine` runs the engine's own bytes.
   Extending it to feed the scan an out-of-world entry and a >4-per-world layout
   costs test code only: no ROM bytes, no option, no builder change.

### The one-to-one invariant

**A fortress will never open two locks** (design decision, 2026-09-06). Combined
with the existing maze rule that a lock has exactly one key, the table is a
partial **one-to-one** map between completable cells:

- at most one entry per key;
- at most one entry per target.

Assert both at build time, in the shape of `every_maze_lock_has_exactly_one_key`.
That test exists because the mirror-image bug — a re-keyed lock quietly keeping
its local key as well — survived the entire life of the maze mode. Same failure
signature: a lock behaving wrongly, no crash, invisible unless you go looking.
Playtesting does not catch this class.

The invariant also makes the scan trivially correct whether or not it stops on
the first match, so it keeps `foreign_locks`' fall-through (cheaper by a byte)
and the difference is unobservable. The format still *permits* two entries
sharing a key; the builder never emits them, and the test says so.

---

## Completion space: three different things get called that

This came up as "map completion tile space is limited". Only one of the three is
scarce, and it is not the one that limits new obstacles.

| | Size | Scarce? |
|---|---|---|
| `Map_Completions` (live world, `$7D00`) | 64 bytes per player half = 64 columns x 8 row-bits = **512 cells** | **No.** The map is 9 x 64 with rows 7/8 sharing a bit, so every cell already has one. Nothing to expand. |
| `completion_bits` packed store (maze only) | `PLANE_RESERVE` = **48 bytes** for all 8 worlds end to end; measured worst case 42 over 150 builds | **Yes**, ~6 bytes of headroom. Exists only because the maze needs completions for worlds the player is not standing in. Standard mode never touches it. |
| `Map_Removable_Tiles` / `Map_RemoveTo_Tiles` | **8 entries each** | **Yes** — and this is the real limit on new obstacle types. |

So "new obstacles keyed to specific things" is a **tile-table** project, not a
bit-budget project. Standard mode can have as many new obstacle types as an
8 -> N expansion allows with no bit pressure at all; the same expansion in maze
mode eats into those 6 spare bytes and needs a stencil census.

### What expanding the removable tables costs

They are packed tight and cannot grow in place:

```
$A437  Map_Removable_Tiles    8
$A43F  Map_RemoveTo_Tiles     8
$A447  Map_Completable_Tiles  5      <- immediately after
```

Expansion means relocating both parallel tables into PRG012 free space (716
free, largest gap 336) and patching every reader:

- vanilla's own loop bound, assembled as `LDX #(MRT_END-Map_Removable_Tiles-1)`
  — a literal `LDX #$07` in the binary (PRG012, disasm line ~359);
- `completion_bits`' stencil routine, which hardcodes both table addresses and
  their counts (`MAP_REMOVABLE_TILES`, `MAP_COMPLETABLE_TILES` in
  `completion_bits.rs`);
- the 48-byte PRG010 mirror this design introduces;
- anything else found by grepping those two constants.

A tile added to `Map_Removable_Tiles` starts **claiming a completion bit**, which
grows the packed store's stencil, interacts with `PLANE_RESERVE`, and brings the
row-7/8 shared-bit rule (#212) into play for the new tile. It is not a free knob.

---

## What position keying unlocks (deferred, recorded so the format serves it)

- **Deja vu on forts.** Blocked today by a real collision, not by neglect:
  `BOOMBOOM_Y_OFFSETS` is a 17-entry array parallel to `FORTRESS_ENTRIES` — one
  Y-byte **per pointer-table entry**, not per map cell. `write_fortress_fx`
  resolves it through `pickup.pool[fort_a.pool_idx].catalog_idx`, so two map
  cells holding the same fortress write the same ROM byte twice. The second
  write wins and the first copy's lock becomes permanently unopenable.
  **Implementing fort deja vu before this rework would ship a silent softlock.**
- **More than 4 forts per world.** Three stacked caps: the 4-wide
  `FX_WORLD_TABLE` row, the 17 global slots, and the 17-card fort deck. This
  removes the first two; fort deja vu handles the third. Not checked: whether
  the builder independently caps forts per world.
- **Levels as keys.** Free — `Map_MarkLevelComplete` runs for every completed
  map cell, so a level, toad house or spade panel can hold a key. Open design
  question: a lock keyed to a level three screens away reads as a dead end
  rather than a puzzle. See `map_legibility_charter`.

---

## Suggested staging

1. **Stage 0 — prove the derivation in Rust. Done** (2026-09-06). Four tests in
   `overworld_writer::fortress_fx::derivation`, no ROM bytes touched:
   `rust_gap_tile_mapping_agrees_with_the_engines_removable_tables` checks the
   Rust copy of the mapping against the engine's own tables;
   `vanilla_fx_slot_data_is_derivable_from_its_target_cell` pins vanilla's two
   exceptions as exact strings; `fx_position_bytes_are_always_derived_from_the_target_cell`
   asserts the 68 arithmetic bytes over vanilla plus N randomized ROMs; and
   `randomized_fx_tile_and_patterns_diverge_only_by_path_variant` pins the shape
   of the one divergence and prints its census (`CENSUS_SEEDS` raises the seed
   count, default 8). Findings are folded into "Most of that is cached
   arithmetic" above.
2. **Stage 1 — one ROM change.** The entry table and 48-byte mirror in the freed
   `$C7BD` block, the scan in `Map_MarkLevelComplete`, a rewritten
   `MO_DoFortressFX`, a rewritten screen check, `foreign_locks` absorbed, the
   Boom-Boom nibbles zeroed, the W8 bridge channel replaced, and the one-to-one
   invariant asserted.

   *The original plan had an intermediate "derive the data, keep the slot
   addressing" step. Dropped: it carries real 6502 risk for no player-visible
   gain, and the derivation code would be written twice — once indexed by slot,
   once by position. The branch is the safety net; the intermediate ROM state is
   not worth its own commit.*
3. **Stage 2 — builder capabilities** (out of scope here): fort deja vu, the
   per-world fort cap, levels-as-keys.
4. **Stage 3 — expand `Map_Removable_Tiles`** for new obstacle types, with its
   own stencil census. Its first customer is the sky vertical lock (see
   "Decision: stage 1 derives" above) — a terrain mismatch that predates this
   rework and was explicitly left in place.

**Ordering rule that matters: nothing from stage 2 before stage 1 lands**, or
fort deja vu ships the Y-byte clobber.

Every new 6502 array gets an `asm::check` in the same commit (CLAUDE.md), and
anything touching the overworld needs the two deep censuses re-run:
`CENSUS_SEEDS=500 all_world_targets_reachable` and
`CENSUS_SEEDS=1000 test_route_census`.

---

## Hard constraints to carry forward

- **Rows 7 and 8 share completion bit `$01`** and the reload reads row 7 first
  (#212). `Map_CompleteBit[row.min(7)]` is the same arithmetic either way — the
  constraint just moves from the writer into the derivation.
- **Map objects are not map cells.** Hammer Bros, tanks and battleships are the
  nine sprite slots; they set **no completion bit at all** (`MO_DoLevelClear`
  just empties the slot with a poof). They can be neither key nor target under
  this design. Making tanks work is the separate `map_objects.rs` hook question
  — that module already hooks the exact `STA Map_Objects_IDs,Y` at `$ABB7`.
- **The off-screen gate and W8 darkness** (`patch_fortress_fx_screen_check`,
  issue #131) are the fiddliest part of the routine and must survive. The
  poof-only exit still updates map RAM and `Map_Completions`; only the VRAM write
  is suppressed.
- **PRG010 is the map bank** at `$C000` and PRG011 at `$A000`, both mapped for
  the whole map (`$84A0`), so a routine may live in either — but **PRG012 is
  not**, which is why the 48-byte mirror exists.
- **The crumble sound is chosen by tile type** in the completion path
  (`PRG011_AA9C`), so a non-fortress key would get the ordinary level-clear
  sound unless that is also changed. Probably desirable to leave alone.

### Rust-side readers of the tables being deleted

- `overworld_pickup::open_fx_gaps` (**production**) reads vanilla's FX slots to
  open lock gaps before placement. It reads the *source* ROM state, so it
  survives — but see forced inclusion 3 for the `qol` handshake it depends on.
- `qol::overworld_map::apply_w8_bridges` writes `FX_MAP_TILE_REPLACE + 16`.
- `overworld_build::sources::vanilla_locks` (**test only**) inverts the writer's
  position encoding.
- `completion_bits::vanilla_fx_bits_are_owned` (**test**) reads
  `FX_MAP_COMP_IDX` for all 17 slots.
- `rom_data::access::read_fx_slots` and `overworld_writer/tests.rs:552`.

---

## Verified vs assumed

**Verified** by reading the disassembly and the ROM: the whole Boom-Boom -> orb
-> ordinal -> slot chain; the 236-byte table block, its offsets and its CPU
address range; the map operation table (`prg010.asm:905`) placing
`MO_DoLevelClear` at 7 and `MO_DoFortressFX` at 8, and `PRG011_AA58` calling
`Map_MarkLevelComplete` before `INC Map_Operation`; that op 8 gates on and then
writes the completion bit for **both** players before swapping the tile; that
`Map_Removable_Tiles`/`Map_RemoveTo_Tiles` are parallel 8-entry tables with
`Map_Completable_Tiles` immediately after; that all of those plus the metatile
quadrant tables live in PRG012 and are unmapped during map play; that the FX
VRAM address has no screen term (so screens alias onto one nametable window);
that `MO_DoLevelClear` sets no completion bit for map objects; that
`Map_MarkLevelComplete` is called with the row and column already in `X` and
`Y`; that `BOOMBOOM_Y_OFFSETS` is per pointer-table entry; the completion-space
figures (`HALF_LEN` 64, `PLANE_RESERVE` 48 with worst measured 42 over 150
builds).

**Assumed, verify before relying on it:**

- That the enemy-stream loader puts the Y byte's high nibble into `Objects_YHi`
  — inferred from our own writer's encoding and vanilla's "this was used as a
  parameter" comment, not read in the loader itself. It only matters up to the
  point the parameter is retired.
- The exact straddle geometry of the screen check — why `+/-1` and not `+/-2`,
  why the edge-filter bounds are `[$10, $E8)`. Taken from the recorded Fred
  cross-check (21 Fred-generated ROMs carry the same 80 bytes), not re-derived.
  This is why those two blocks are to be kept byte-for-byte rather than
  rewritten.
