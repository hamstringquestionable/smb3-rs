# Sprite slots — one ledger for every map object

**Status: phase A built (2026-11-08, `feature/sprite-budget`); phase B is
design.** Phase A is the byte-identical refactor: `overworld::sprite_slots` is
the only code that writes the map-object tables, proved byte-identical over 80
runs across every mode that touches a sprite. Phase B carries the behaviour
changes.

## The problem

Every world has nine map-object slots (`Map_Objects_*`, PRG011 tables at
`MAP_OBJ_*_MASTER`). Seven separate pieces of code write them today, each
choosing its own slot, and they stay out of each other's way only because of
the order they happen to run in:

| Claimant | Slot choice | Where it is decided |
|---|---|---|
| HELP bubble | 0 (ROM) | vanilla table; the maze reads it as the wand indicator (`maze::map_objects`) |
| Airship | 1 (position only) | `start_airship_swap::move_airship_sprites`, generated castles in `pipeline::stages` |
| Hammer Bros | lowest eligible | count capped in `build::capacity::assign_hb_sprites`, stamped by `writer::sprites::write_hb_sprites` |
| Piranha plants | highest empty | `writer::sprites::plant_budget` + `write_plant_sprites` |
| W8 military | hardcoded 2-5 | `writer::sprites::W8_ARMY_SPRITES` |
| W8 canoe (8s are Wild) | hardcoded 6 | `qol::place_w8_canoe` |
| Generated canoes | highest empty | `rom_data::place_canoe` |

The arithmetic for "how many slots are left" exists three times (the HB cap,
`plant_budget`, the highest-empty lookups) and none of them knows what the
others will claim. Adding anything — a reserved pair of plants, a marker sprite,
another canoe — means reasoning about all seven.

## Engine facts the design rests on

Verified against the disassembly 2026-11-08; see also
`docs/smb3_rom_reference.md`.

- **Slot 1 is free with autoscroll removal.** Its only movers are the airship
  object's own handler (`Map_Object_Airship`, op 4, dispatched by object id) and
  the retired king scene (`prg024:306`). With removal on — hardcoded on the web,
  off only via `--keep-autoscroll` — no object ever has the airship id.
- **Index 0 is skipped by exactly two engine loops**: the died-on-an-object check
  (`prg011:1161`, which only acts on the airship and on a coin ship, reverting it
  to a Hammer Bro) and the canoe search (`prg010:3490`). Every other loop —
  drawing, marching, entering, completion, `Map_Init` — covers slot 0.
- **Slot 0 is the HELP bubble in W1-W7**, which the maze uses as the "this
  world's wand is held" indicator. **World 8's is parked off the map** (y=`$10`,
  screen 5 of a 4-screen map) and nothing reads it: there is no king and no
  wand, and the maze's slot-0 clear is keyed on `WANDS_TABLE[7]`, never set.
- `RESERVED_DYNAMIC_SLOTS` (2) stay empty everywhere for runtime spawns (white
  mushroom houses).

## The model

One ledger per world, owned by the world: it starts on `WorldMap` (the map
source makes the first claims — canoes, castle/HELP) and is carried into
`BuiltWorld`, where the builder and writer add the rest.

```
WorldSlots
  capacity   which of slots 0-8 this run may use
  reserve    runtime-spawn slots kept empty
  claims     Vec<Claim>

Claim
  kind    Help | Airship | HammerBro { reward } | Plant | Canoe | Military(unit) | ...
  owner   &'static str — "w8_military", "piranha_home", ... (errors, write log)
  pos     the grid cell, decided by the claimant, possibly later than the claim
  slot    assigned by the allocator, never by the claimant
```

**Slot rules are the engine's, attached to the kind**, not to the claimant:

| Kind | Rule | Why |
|---|---|---|
| `Help` | slot 0 | the maze's wand indicator; the engine's king-scene token |
| `Airship` | slot 1, only with `--keep-autoscroll` | engine assumes index 1 |
| `Canoe` | not 0 | the canoe search skips index 0 |
| `HammerBro` | not 0 | the coin-ship revert skips index 0 |
| `Plant`, `Military` | any | stationary; every loop that matters covers 0 |

A new kind of sprite declares its rule and nothing else changes.

### Lifecycle: invariants first, then the rest

1. **Engine-fixed** — HELP (W1-W7); the airship with `--keep-autoscroll`.
2. **Reserve** — the runtime-spawn slots.
3. **Invariants** — must fit, or it is a bug and panics: W8's military, canoes,
   World 7's piranhas when Piranha Shuffle is Off on generated maps.
4. **Flexible** — Hammer Bros. The builder distributes them **only into the
   slots each world has left** (`free(world)`), conserving the total across
   worlds as today.
5. **Best-effort** — On/Wild plants, future decorations: `try_claim` returns
   `None` when the world is full.

### One allocator, one writer

At write time an allocator gives every claim a slot that satisfies its kind's
rule, in **today's order** (user, 2026-11-08: no reason to change it): Hammer
Bros from the bottom, plants and canoes from the top, military in 2-5. Then a
single `write_sprite_slots` stamps ids, positions and rewards for all eight
worlds. Nothing else writes the map-object tables.

## Invariants

- Per world, claims never exceed capacity minus the reserve.
- Every claim's slot satisfies its kind's rule.
- Every invariant claim was placed — World 7's plants included.
- Only the slot writer writes `MAP_OBJ_*` (a test, in the spirit of
  `tools/offset_dups.py`, or the write log's ownership audit).

## What phase A built

- **`WorldSprites`** — a world's nine slots as a snapshot of the ROM's tables
  plus edits, written back by diff (only changed bytes), so leftover position
  bytes in vanilla's empty slots are never disturbed.
- **On each `BuiltWorld`** as `sprites`, read at the start of the build. The
  builder deals Hammer Bros into `room()`; the writer edits a copy (military,
  plants, Hammer Bros) and writes it once.
- **The passes that run before the build** — the 8s are Wild canoe, generated
  canoes and castle/HELP positions, the vanilla plant clear, the start/airship
  slot-1 move — claim through `WorldSprites::edit` (read, edit, write back).
  This is the one departure from the plan above: they do not share one
  pipeline-wide ledger object, which would have threaded a new parameter
  through a dozen stages and every test that calls `build()`. They do share
  the rules and the only writer.
- **Removed**: every `rom_data` helper that wrote the tables
  (`write_map_sprite[_position]`, `place_canoe`, `clear_map_sprite`,
  `clear_hb_sprites`, `write_hb_sprite`, `eligible_hb_map_slots`,
  `last_empty_map_obj_slot`, `read_plant_sprite_positions`).
- **Guarded** by `sprite_slots::ownership`, a source scan that fails on any
  table write outside the module (it flags eight lines in the pre-refactor code).
- **Not yet moved**: the position *readers* the map source uses
  (`read_parked_sprite_positions`, `read_hb_sprite_positions`). They still scan
  the ROM, and `read_parked` counts any id but ``, empties with a stale
  position included — a quirk to keep or fix deliberately, not in a refactor.
  Kinds and their rules are in `SpriteKind`; phase A encodes today's capacity
  (`first_usable`: slot 2, slot 1 in World 8) rather than per-kind rules.

## Phases

**Phase A — refactor, byte-identical.** Today's capacity (slot 1 reserved in
W1-W7, World 8's slot 0 unused) and today's slot order. Every current writer
becomes a claimant; the allocator and single writer replace the seven call sites.
Proved by byte-comparing against `beta/next` with `--no-palettes` across modes
(standard, maze, generated, each piranha mode, 8s are Wild) and by
`tests/rom_identity.rs`.

**Phase B — behaviour, each change measured.**

1. Slot 1 becomes capacity in every world (autoscroll removal on).
2. One W8 military unit moves into World 8's slot 0, freeing a general slot.
   Verify on an emulator: enter it, lose, win.
3. World 7 reserves two slots for the plants when Off on generated maps.
4. `move_airship_sprites` is skipped with autoscroll removal on — it would
   otherwise move whatever now lives in slot 1 in a swapped world, since it
   runs after the Hammer Bro writer.

Then a before/after `map_object_slot_budget` census; most seeds move, because
slot 1 raises every world's Hammer Bro cap.

## Open questions

- **The "fake tank" in World 8.** Players report telling the tank without a
  level apart from the real one: when both are on one screen, one disappears
  first, or entirely. Two candidates, neither checked yet: (a) a military
  sprite that gets no fortress/level position keeps its vanilla position, which
  on a randomized map can be a cell with no level; (b) the NES's 8-sprites-per-
  scanline limit, where which sprite drops out follows draw order and so slot
  order. Investigate before phase B moves a unit into slot 0.
