# MiMaze — the item-gate layer

> **Status: design. Nothing is implemented.** Written 2026-09-19 against the
> measurements in `maze::tests`' `wall_site_census`, `key_placement_census`,
> `canoe_gate_census` and `level_gate_census`, all of which are in the tree and
> reproducible. Where a number appears here it came from one of those.
>
> This is the **generator** side. `item_keys_design.md` is the ROM side — the
> found-mask, the dispenser hooks, the carriers — and it stays the authority on
> everything in the cartridge. Read that first; this says how a seed decides
> what to gate.

## The shape in one paragraph

After the maze is generated and before it is written, walk the finished map and
choose a handful of **levels** to seal behind items the player has not found
yet, and a handful of **sources** to hand those items out. Nothing is placed:
the layer reads a map the existing builder already dealt and assigns meaning to
what is standing there. A gated level cannot be entered, and because vanilla
will not let the player walk off an uncompleted level tile, a level that cannot
be entered is a wall.

## Why a level is a wall

This is the whole reason the layer is cheap, so it is worth stating exactly.

`MO_NormalMoveEnter` refuses to let the player walk *off* a tile at or above its
palette page's threshold in `Tile_AttrTable+4` until that tile is completed, and
a level tile is above its threshold by definition — that is what makes it
enterable. So in vanilla, **every level is already a wall until you beat it**.
Gating entry therefore gates passage, with no new tile, no new map object and
no change to any level's contents.

The maze walker already agrees, which is what made the measurement possible:
`walk::expand` rejects a destination cell whose tile is in `BACKGROUND_TILES`,
and `GlobalState::base_grids` blanks a blocked cell to background. So
`spheres_with_blocked({that level's cell})` *is* "this level is gated and the
player has not found the key".

### What that buys, measured

`level_gate_census`, 100 seeds, K=3, of 62 levels:

| | |
|---|---|
| gateable levels per seed (sealing something) | **18.8**, min 11, max 29 |
| of those, sealing enough to make the game unwinnable | 8.5 |
| with a key site in front of them | **98.2%**, mean 36.7 sources |
| cut sizes | 1384 under a tenth, 165 a tenth to a half, 160 a half to nine tenths, 166 nearly all |

No seed had fewer than 11. The layer is choosing from a comfortable supply, not
scraping.

## Where it runs

`randomizer/mod.rs:405`, the existing `// --- OVERWORLD CAPTURE POINT ---`,
between `maze::stamp_into` and `write_overworld`. A third model pass in the
slot the maze already occupies — read the finished `GlobalState`, decide,
hand the decision to the writer.

Everything it needs is settled by then:

* **the map** — levels, forts, locks, pipes, telepads, the winnability fixpoint
* **the sources and what they hand out** — since the item tables now roll ahead
  of `overworld_pickup` (see `CHANGELOG`), `(position -> item)` is a fact in the
  model rather than something re-rolled afterwards
* **Hammer Bro homes and rewards** — `BuiltWorld::hb_sprites`, both fields

One thing is **not** settled and shapes the design: which *level* sits on a
given slot is decided by `overworld_writer::assign_pool`, downstream. The model
knows "a Level slot stands here", never "6-5 stands here". The layer therefore
gates **positions, not levels**, and never reasons about a level's contents.

## The model

Three sets, and the layer's job is to choose the second and third.

* **Items** — the key vocabulary. The Global Item IDs the sources already deal
  in: mushroom `$01`, fire `$02`, leaf `$03`, frog `$04`, tanooki `$05`, hammer
  suit `$06`, and room to grow. A bitmask; two SRAM bytes covers sixteen, and
  `maze_state.rs` has six free (`$7ADA-$7ADF`).
* **Gates** — `(position, required item)`. Positions are Level slots.
* **Key placements** — `(source position, item)`. Sources are the three kinds
  the model can address: Toad Houses (22 per seed), Hammer Bro encounters (15),
  Princess letters (8). **Not in-level chests** — those are levels, and the
  level binding is downstream.

### The found-mask

One or two bytes in `maze_state.rs`' run, so `completion_bits::NEW_GAME_INIT`
clears it for free by clearing the whole declared range. That clear is installed
only in maze mode, which is exactly this mode, so the game-wide cost
`item_keys_design.md` records does not apply here.

A bit is set where an item is granted: `ToadHouse_ChestPressB` (PRG029, 2568
bytes free), the Hammer Bro reward path, the letter award. Sticky, never
cleared — which is what keeps the fixpoint monotone.

### Enforcement: refuse the entry

The gate is a **map-side entry refusal**, not a tile swap:

* the level tile stays exactly what it is
* pressing A on a gated cell whose bit is clear does not enter
* the player cannot walk off the cell, because the level is not complete
* when the bit is set, the level behaves normally with no state to restore

That needs one position-keyed table and one hook. Both patterns are shipped:
`lock_keys.rs` is already a position-keyed table that replaced vanilla's
fortress-FX slots, and `world_persist::PAD_ENTER` already hooks the level-entry
path at `PRG010_CEA7` for telepads. Map-side code has PRG010/PRG011 mapped for
the whole map, so this does not touch the nearly-full always-mapped banks.

**A tile swap was the alternative and is deferred, not rejected.** Swapping the
level tile for a wall tile is more legible but costs a minted tile family, CHR,
and a restore path, and `ML_RANGE` deliberately put the free `$6B-$7F` tail
*outside* the M/L window so those bytes carry obstacles. See "Legibility".

## The solver

`GlobalState::spheres()` is a fixpoint over one accumulator — beaten forts.
This needs a second: found items. The shape is unchanged.

```
open_forts = {}, found = {}
loop:
    shut  = locks closed by open_forts
    gates = gated cells whose item is not in found
    reach = walk(grids with gates blanked, shut)
    beat every fort in reach          -> open_forts
    collect every source in reach     -> found
    until neither grew
```

Monotone in both accumulators, so it terminates in at most one round per
fort-plus-item and no assumption is made about play order — the same argument
the existing fixpoint rests on. A sphere is still a round, so `Sphere::width`,
`completion_cost` and the spoiler log keep working with the item dimension
folded in.

**This is the only genuinely new machinery in the layer.** Everything else is
recombination.

## Choosing the gates

Generate and test, which is what the builder already does for shaping and for
the same reason: the property wanted is emergent, and the placement is cheap to
retry.

1. Rank the gateable Level slots by cut size, keeping the band that seals
   something worth sealing. The census says roughly 8 per seed fall in the
   10–90% band, with the rest sealing a tail of one or two nodes.
2. Deal gates from that band, and for each, deal its key onto a source **in
   front of the cut** — the acyclicity rule, and the reason the key census
   exists. 98.2% of cuts have somewhere; the 1.8% that do not are skipped.
3. Run the item-aware fixpoint. Solvable and every fort still beatable, or
   redeal.
4. Keep the deal, hand it to the writer.

Budget is deliberately **not a knob yet**. Pick a count from the measurement,
ship it, and let a knob be earned if the census says the spread matters.

## What the writer does

Three writes, none of which touch a map grid:

* the gate table — `(world, row, col, item bit)` per gate, position-keyed the
  way `lock_keys` is
* the source assignments — a Toad House's treasure type, a Hammer Bro's reward
  byte, a letter's reward byte, all now decided before anything reads them
* the found-mask setters at the three grant sites

## Invariants

1. **Never gate behind its own key.** No gate's cut may contain the source of
   the item that opens it. Generalises `locks.rs:27`; across many gates it is
   acyclicity of the key graph, and the fixpoint in step 3 is what enforces it
   in practice rather than a separate check.
2. **Order-free.** The fixpoint assumes no play order, so a seed is winnable
   however the player routes.
3. **Nothing sealed out.** Every fortress still beatable, every world still
   completable — the existing invariant, now also over items.
4. **Standard mode untouched.** `rom_identity` byte-identical with the mode off.
5. **Sequence breaks stay legal.** A player who carries a suit in and beats a
   level the logic called gated has earned it. The generator must never try to
   prevent this.

## Known gaps

* **Multi-gate composition is unmeasured.** Every census blocked exactly one
  cell. Two individually-sound gates can compound; step 3's fixpoint is the
  defence, but how often a deal survives it is unknown until the solver exists.
  This is the one risk that could change the design.
* **Toad House items are not independently assignable.** 22 houses share a
  15-entry `ToadHouse_Item2Inventory` keyed by treasure *type* (the type is the
  house's own `obj_ptr` high byte, `prg030.asm:1351`). Two houses of the same
  type cannot hold different items. The effective source count is below 45 and
  nobody has measured by how much.
* **Legibility.** A gated level looks like a level. The player learns by
  pressing A, and "nothing happens" is the failure mode `item_keys_design.md`
  rejects for `Player_QueueSuit`. At minimum this needs a sound or a bubble;
  the marker-tile family is the real answer and is costed in that document.
* **The dispenser layer is not in this.** Gating power-up blocks on the
  found-mask — the LATP and Big [?] work in `item_keys_design.md` — is a
  separate, independent layer. This one stands without it, and it should be
  judged on its own before the two are combined.

## Related

* [item_keys_design.md](item_keys_design.md) — the ROM side, and the authority
  on the found-mask, the dispenser hooks and the carrier vocabulary.
* [world_maze_design.md](world_maze_design.md) — the mode this extends.
* [seed_stability.md](seed_stability.md) — the bar: standard byte-identical,
  maze census-equivalent per world.
