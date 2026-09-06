# Fortress FX table redesign

**Status:** design only, nothing implemented. Written 2026-09-06 on
`experiment/world-maze` so a fresh session can pick it up cold.

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

### Most of that is cached arithmetic, not data

| Stored | Actually a function of |
|---|---|
| `VAddrH/L` (34) | `0x2880 + row*64 + (col%16)*2` |
| `MapCompIdx` (34) | column is the map column; bit is `Map_CompleteBit[row.min(7)]`, a table the engine already has and already indexes in `Map_MarkLevelComplete` |
| `Patterns` (68) | the replacement tile — our `fx_patterns_for` collapses it to **three cases** (water bridge / vertical lock / gap-sky) |
| `MapTileReplace` (17) | the current tile, via `Map_Removable_Tiles` -> `Map_RemoveTo_Tiles` |

That is 153 of the 187 bytes of slot data redundant with the target position.

---

## The design

**One position-keyed table replaces both the FX slots and the foreign-lock
table.**

```
row = key_world_row_col : target_world_row_col        4 bytes
```

Same encoding the foreign-lock rows already use: `(row << 3) | world` in one
byte, column in the next (6 bits).

Read it from **`Map_MarkLevelComplete`** (PRG011), which already computes
`(world, row, column)` from the player's position for its own completion write —
`Y` holds the column and `X` holds `Temp_Var13`, the row. Scan for rows whose key
matches; for each hit:

- **target world == current world** — set the completion bit, swap the tile in
  `Tile_Mem`, and queue the animation (subject to the existing off-screen gate).
- **target world != current world** — set the bit in the packed store
  (`completion_bits`) and skip the animation. Nobody is there to see it.

Local and cross-world stop being two mechanisms. They differ by one comparison.

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

### Budget

236 bytes freed in PRG010 (largest current gap: 64), 68 spent on a 17-row table,
so roughly 168 back before the derivation code — which trades table lookups for a
shift-and-add and a three-way branch. Also frees the 128-byte PRG011 allocation.

---

## Flexibility: the two questions asked

### Can a non-fortress break something?

**Yes, and it falls out for free.** The fortress-ness is incidental once the key
is a position. `Map_MarkLevelComplete` runs for **every** completed map cell —
levels, toad houses, spade panels, fortresses alike — so any of them can be a
key just by having a row. Beating a level could bust a lock.

Two things to handle:

- The animation is driven by the map state machine reaching
  `MO_DoFortressFX` (dispatch entry 8) with `Map_DoFortressFX` set. Today only
  the orb sets it. The scan would set it instead, on any matching completion.
- The crumble **sound** is chosen in the completion path by tile type
  (`PRG011_AA9C`), so a non-fortress trigger gets the ordinary level-clear sound
  unless that is also changed. Probably desirable to leave alone.

### Can new tiles be used for breaking?

**Yes, but this is the one part that is not free**, and the constraint is
persistence rather than the FX table.

A busted lock survives a map reload only because
`Map_Reload_with_Completions` re-derives it from `Map_Removable_Tiles` ->
`Map_RemoveTo_Tiles` plus the completion bit. **A target tile that is not in
those tables would revert on the next map load.** So storing a replacement byte
in the row does not buy flexibility — the reload would undo it.

Therefore a new breakable tile means expanding both parallel tables. They are
packed tight and cannot grow in place:

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
- anything else found by grepping those two constants.

**Consequence to think about before doing it:** a tile added to
`Map_Removable_Tiles` starts **claiming a completion bit**. That grows the packed
store's stencil, interacts with `PLANE_RESERVE`, and brings the row-7/8 shared-bit
rule (#212) into play for the new tile. It is not a free knob.

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
  the whole map (`$84A0`), so a routine may live in either.

---

## Suggested staging

The risky half is the addressing change; the data change is provable.

1. **Derive the data, keep the index.** Replace `VAddrH/L`, `MapCompIdx`,
   `Patterns` and `MapTileReplace` with computation, leaving the
   world+ordinal addressing alone. A/B against current ROMs: the animation and
   the completion bits must come out byte-identical. Pure win, easily reverted.
2. **Switch to position keys.** Move the scan into `Map_MarkLevelComplete`,
   delete `FortressFX_W*` and `FortressFXBase_ByWorld`, retire the Boom-Boom
   Y-byte parameter, and fold `foreign_locks` into the same table.
3. **Then** consider expanding `Map_Removable_Tiles` for new breakable tiles,
   as its own change with its own stencil census.

Every new 6502 array gets an `asm::check` in the same commit (CLAUDE.md), and
anything touching the overworld needs the two deep censuses re-run:
`CENSUS_SEEDS=500 all_world_targets_reachable` and
`CENSUS_SEEDS=1000 test_route_census`.

---

## Verified vs assumed

**Verified** by reading the disassembly and the ROM this session: the whole
Boom-Boom -> orb -> ordinal -> slot chain; the 236-byte table block and its
offsets; that `MO_DoFortressFX` writes the completion bit for **both** players
and then swaps the tile; that `Map_Removable_Tiles`/`Map_RemoveTo_Tiles` are
parallel 8-entry tables with `Map_Completable_Tiles` immediately after; that
`MO_DoLevelClear` sets no completion bit; that `Map_MarkLevelComplete` is called
from the generic completion path with the row and column already in `X` and `Y`.

**Assumed, verify before relying on it:** that the enemy-stream loader puts the
Y byte's high nibble into `Objects_YHi` — inferred from our own writer's encoding
and vanilla's "this was used as a parameter" comment, not read in the loader
itself. It only matters for step 2, which retires that parameter anyway.
