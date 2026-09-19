# MiMaze — the item-gate layer

> **Status: design. Nothing is implemented.** Written 2026-09-19 against the
> censuses in `maze::tests` (`level_gate_census`, `key_placement_census`,
> `wall_site_census`, `canoe_gate_census`), all in the tree and reproducible.
>
> This is the **generator** side. `item_keys_design.md` is the ROM side — the
> found-mask and the dispenser hooks — and this design *depends* on it rather
> than standing beside it. See "The dispenser layer is not optional".
>
> **Corrected once already, on the first day.** The first draft had the layer
> gating map *positions* through a new map-side entry refusal, on the grounds
> that level identity is decided downstream. That was wrong in both halves, and
> the corrections are the design: identity is not out of reach, and no new
> enforcement is needed because vanilla already supplies it.

## The shape in one paragraph

After the maze is generated and before it is written, walk the finished map,
pick cells whose sealing would gate something worth gating, and ask the writer
to put a **level that genuinely requires an item** on each of them — 6-5 on a
cut that should need the leaf. The item's source is placed in an earlier
sphere. Nothing is placed on the map and nothing new enforces anything: the
level's own contents are the wall, and the dispenser layer is what stops the
player bringing a leaf from somewhere else.

## Why this works with no new map machinery

Three facts, each already true.

**1. A level tile is a wall until it is beaten.** `MO_NormalMoveEnter` will not
let the player walk *off* a tile at or above its palette page's threshold until
it is completed, and a level tile is above its threshold by definition — that
is what makes it enterable. So a level that cannot be beaten seals every route
through its cell.

**2. A level that requires a power-up carries its own source.** `vision.md`'s
fourth charter point: *"if a level requires a power-up, the level itself
provides a renewable (effectively unlimited) source of it"*. 6-5 needs flight
and holds the Q-leaf that grants it (`0x22D74`). That is why the level is
beatable entering small, and it is also the hook — **turn that source off and
the level becomes unbeatable.**

**3. The dispenser layer turns it off.** With power-up blocks gated on the
found-mask (`item_keys_design.md`), 6-5's leaf block pays a coin until the
player has found a leaf from an overworld source. 6-5 is then a wall keyed on
"leaf found", enforced by vanilla's own level design.

The walker already agrees with all of this, which is what made the measurement
possible: `walk::expand` rejects a destination cell that is background and
`base_grids` blanks a blocked cell to background, so
`spheres_with_blocked({that cell})` *is* "this level is gated and the key is
not found yet".

## The layer picks which level lands where

The first draft claimed it could not, because `overworld_writer::assign_pool`
binds pool entries to slots downstream. The binding is downstream; the
*constraint* need not be.

**The precedent is troll pipes.** `troll_pipes::mark_troll_pipes(&mut build)`
runs before the capture point and sets `SlotAssignment::is_troll_pipe`. The
writer honours it: `assign_pool` orders marked slots first and draws each one a
pool entry satisfying a predicate (`!holds_unique_item`), demoting the slot if
the pool cannot supply one. A model-side mark, a writer-side constraint
solve — exactly the shape this needs.

**And level identity is already a first-class thing there.** `CHEST_LEVELS` and
`is_chest_level(world_idx, entry_idx)` name specific levels, and `assign_pool`
reasons about them today. A `LEVEL_REQUIREMENTS: &[(world, entry, Item)]`
registry is the same kind of table, and `requires(pi) == Some(item)` is the
same kind of predicate.

So the layer's output for a gate is `(slot position, required item)`, and the
writer draws a level with that requirement onto that slot. Same machinery,
one more predicate.

## What can be a gate: the real supply

The geometry is not the constraint. `level_gate_census`, 100 seeds: **18.8
positions per seed** seal something when gated, min 11; 98.2% have a key site
in front. There is no shortage of places.

**The vocabulary is the constraint.** A level is gate-capable only if beating
it needs a power-up, and the levels known to satisfy that are the ones the
randomizer already had to protect — `powerups.rs`:

| Level | Item | Why |
|---|---|---|
| 6-5 | **leaf** | flight; its Q-leaf is `PROTECTED_OFFSETS[0]` |
| 7-7 | **star** | four Q-stars cross the muncher fields |
| 8-F | **mushroom** | must be big to break a block in sub-area 2 |
| 7-F1 | **mushroom + tanooki** | big → bricks → Big [?] → tanooki → flight |

**Four, and 7-F1 is a conjunction.** That is the budget until someone analyses
more levels, and it is the single most important number in this document: gates
per seed are bounded by **4**, not by the 18.8 the geometry offers. The scarce
resource is levels that genuinely require something, not places to put them.

Two ways the vocabulary grows, both out of scope here:

* **Analysis.** Any level whose completion needs its own power-up qualifies;
  the four above are only the ones the randomizer had to protect to avoid
  breaking them. Nobody has swept the other 58.
* **Manufacture.** `PROTECTED_OFFSETS` and `FLOWER_OR_LEAF_QBLOCK_OFFSETS` pin
  what those blocks dispense. Unpinned, the requirement becomes whatever byte2
  the roll put there — a per-seed requirement rather than an authored one, as
  `item_keys_design.md` notes. Cheap, and it changes the character of the mode.

## The dispenser layer is not optional

The first draft called it "a separate, independent layer" that this one stands
without. **It does not stand without it.** Without the dispenser gate, a leaf
from any other level satisfies 6-5, so 6-5 gates nothing and the whole design
is decoration.

The dependency is exact: the gate is "level L requires item I", and what makes
it bite is that **no block anywhere dispenses I until I is found**. That is the
LATP and Big [?] work in `item_keys_design.md`, and it is a prerequisite.

## The solver

`GlobalState::spheres()` is a fixpoint over one accumulator, beaten forts. This
needs a second — found items — and the shape is unchanged:

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

Monotone in both, so it terminates and assumes no play order — the same
argument the existing fixpoint rests on. A sphere is still a round, so
`Sphere::width`, `completion_cost` and the spoiler log keep working.

**This is the only new machinery in the layer.** Everything else is a mark, a
predicate, or a table.

## Choosing the gates

Generate and test, like the builder's shaping and for the same reason.

1. Rank gateable Level slots by what they seal. Prefer the band that seals
   something meaningful — the census puts roughly 8 per seed above a tenth of
   the content, with the rest sealing a node or two.
2. Assign the four requirement levels to four of them, and place each item's
   source **in front of its own cut** — the acyclicity rule, which the key
   census says is satisfiable for 98.2% of cuts.
3. Run the item-aware fixpoint. Solvable, every fort beatable, or redeal.
4. Emit the slot marks and the source assignments.

Budget is not a knob. It is at most four, and whether all four should always be
used is a measurement nobody has taken.

## The sphere rule

A gate's key must be collectable strictly before the gate is needed. The
fixpoint enforces it structurally — if the key is behind the gate, the round
never grows and the seed is rejected — so this is an invariant, not a scoring
term. 7-F1's conjunction needs *both* a mushroom and a tanooki source earlier,
and if 7-F1 holds a lock key, that lock's fortress must not be what gates
either one.

## What the writer receives

* **slot marks** — `(position, required item)`, honoured by `assign_pool` the
  way `is_troll_pipe` is
* **source assignments** — a Toad House's treasure type, a Hammer Bro's reward,
  a letter's reward; all now decided before anything reads them, since the item
  tables roll ahead of `overworld_pickup`
* **found-mask setters** at the three grant sites

## Invariants

1. **Never gate behind its own key.** Generalises `locks.rs:27`; across gates
   it is acyclicity of the key graph, enforced by the fixpoint rather than by a
   separate check.
2. **Order-free.** No assumption about the order the player takes gates in.
3. **Nothing sealed out.** Every fortress beatable, every world completable.
4. **Standard mode untouched**, `rom_identity` byte-identical with the mode off.
5. **Sequence breaks stay legal.** A player who carries a tanooki into 6-5 and
   beats it without the leaf has earned it. The generator must never try to
   prevent this — `item_keys_design.md` says so and means it.

## Known gaps

* **Until the solver exists, the ROM half can strand a run — so it stays out
  of the randomizer.** `randomize::item_keys` works and is playtested, but on
  its own it is unsafe: the found table starts empty, the player is
  permanently small until a mushroom turns up, and `PROTECTED_OFFSETS` records
  8-F as requiring big. Nothing guarantees a mushroom source is reachable
  before a fortress that needs one. That guarantee is the whole point of the
  fixpoint above, which is why the feature has no option, no flag key bit and
  no web control, and why wiring one before the layer lands would ship a
  softlock. A flag key bit in particular is permanent once released.
* **Four gates is thin for a Metroidvania.** The mode's depth is bounded by the
  requirement vocabulary until it is grown. Whether four is enough to feel like
  anything is the first question a prototype should answer.
* **Multi-gate composition is unmeasured.** Every census blocked one cell. With
  at most four gates the risk is smaller than it looked, but the fixpoint is
  still the only defence and nobody has run it.
* **Toad House items are not independently assignable.** 22 houses share a
  15-entry `ToadHouse_Item2Inventory` keyed by treasure *type* (the type is the
  house's own `obj_ptr` high byte, `prg030.asm:1351`), so two houses of one type
  cannot hold different items.
* **Legibility.** A gated level looks like an ordinary level, and the player
  learns by walking into it and failing. That is arguably correct for a
  Metroidvania — you learn the wall by hitting it — but it is undecided, and
  the marker-tile family in `item_keys_design.md` is the alternative.

## Related

* [item_keys_design.md](item_keys_design.md) — the ROM side, and a hard
  dependency, not a companion.
* [world_maze_design.md](world_maze_design.md) — the mode this extends.
* [vision.md](vision.md) — charter point 4 is what makes a requirement level a
  gate at all.
* [seed_stability.md](seed_stability.md) — the bar for landing any of it.
