# World Maze — design charter

*Shipped to `beta/next` 2026-09-07 (PRs #215/#216/#217) — the
`experiment/world-maze` branch is gone. **Never playtested end to end.**
Supersedes the parked `experiment/mega-map` fold. Sections written before
2026-09-06 predate the fortress-FX rework; where that matters they carry their
own SUPERSEDED banner — read those before trusting a mechanism described here.*

A mode in which the eight world maps stop being a sequence and become the rooms
of one Metroidvania. Every world keeps its own palette, music, king, tileset and
grid — so every "keyed to eight worlds" assumption in the codebase stays true —
and the worlds are linked by **telepads**, by the **airship/castle** one-way
edge, and by **cross-world locks**.

**The mode is built and playable** (2026-09-05). `--world-maze` /
`--maze-wands` are real options, flag-key encoded, and they work in the browser
build as well as the CLI. Phase 1 (persistence, arrivals, telepads) was
hardware-confirmed; phase 2 — the generator, the whistle, the wand gate and
cross-world locks — is complete, executed on an emulated 2A03 and verified end
to end against finished ROMs.

**What has NOT happened is a playtest.** Nobody has played the assembled mode on
hardware or in an emulator. This branch's own history is the reason to say so
plainly: two of the telepad bugs were found only by playing, and one of them
(the arrival trampoline) was invisible to every per-caller test because it was a
shared prerequisite filed under one caller.

This document is the design record. It keeps the decisions that were *wrong* as
well as the ones that held, because most of what follows was settled by a
measurement that contradicted the plan.

**Where phase 1 lives:** `world_persist.rs` (the `$84A0` hooks, the arrival
tables, `PAD_ENTER` and its position key) and
`completion_bits.rs` (the packed per-world store, the derived stencil, the pack
and expand routines). Both carry the mechanism detail in module rustdoc; this
document does not repeat it. The engine-level facts they rest on are in
`smb3_rom_reference.md` under "World transitions and per-world map state" and
"World-map graphics: CHR banks and unused metatiles". Playtest ROMs come from
`testrom`.

**Where phase 2 lives.** The generator is `src/randomize/maze/`:

| File | What |
|---|---|
| `mod.rs` | `GlobalState`, the global fixpoint, `generate` and its solvability fallback |
| `walk.rs` | the two cross-world walkers — reachability, and the level-cost Dijkstra |
| `graph.rs` | the world-graph pass: spine, pad budget, roles, the `Knobs` |
| `roles.rs` | what a pad is for, and which terrain can host it |
| `fill.rs` | the key assignment: a constructive forward fill, with the swap search as fallback |
| `metrics.rs` | how many levels a run beats, and how many it cannot avoid |
| `writer.rs` | converters to the specs the ROM side takes |

The ROM side is `world_persist.rs` (arrivals, `PAD_ENTER`), `completion_bits.rs`
(the packed store, the new-game signal), `maze_state.rs` (the SRAM map),
`world_travel.rs` (whistle fast travel, the visited marker), `wand_gate.rs` and
`lock_keys.rs` (every lock in the game, home and away, in one position-keyed
table — it absorbed what an earlier `foreign_locks.rs` did; see
`docs/fx_table_redesign.md`). Ordering between them is stated once, in
`randomizer::randomize_inner`.

## Settled decisions

- **Gate/key vocabulary already exists**: lock/fortress, rock/hammer, water
  gap/canoe, pipe (free edge), airship (one-way), castle/wands.
- **Locks do not have to sit in the fortress's own world.** The engine already
  has a data-only exit for an off-screen lock (`patch_fortress_fx_screen_check`),
  which is the same case one level up. Cross-world locks are the mode's headline
  feature, not an extra.
- **Goal is K of 7 wands, then the castle.** K is the difficulty dial and K=0 is
  a pure maze, so it is one mode rather than two. K is also the counterweight to
  the pads: without it a telepad chain to World 8 trivialises the game.
- **Hammers are shortcuts, never keys.** Formal invariant: the maze must be
  solvable with zero hammers — checkable by running the fixpoint with
  `has_hammer = false`. Avoids the consumable soft-lock.
- **Warp whistle is fast travel gated to visited worlds**, so it can never reach
  anywhere new and needs no modelling in the solver.
- **`world_order` loses its permutation meaning and keeps its table.** What
  survives is "which world does clearing this world's airship send you to", plus
  the display renumbering, so the player always knows how far along the spine
  they are. The maze **forces `world_order` on**, because that table *is* the
  spine and the wand counter chains through the routine it installs.
- **`world_count` keeps exactly the meaning it already has**, and it turned out
  to need no new definition. `world_order::randomize` with `world_count < 7`
  returns a shorter order; the worlds it leaves out are still built, still on
  the ROM and still full of content, but no airship leads into them. In maze
  mode that makes them **optional bonus content reachable only by telepad** —
  which is as close as the terrain gets to the pad-only island the charter
  wanted, and it costs nothing to support. Solvability is scoped to the worlds
  the spine names (`GlobalState::in_maze`), exactly as a shorter game means
  fewer worlds in standard mode. `a_short_spine_still_finishes` pins it.
- **`maze_wands` (K) is a player-facing option**; the two generator biases are
  not. See "Knobs".

## Topology

**The airship/castle spine.** Clearing a world's airship moves the player to the
next world in the renumbered order — `1→2→…→8`, one-way. That is one Hamiltonian
path through the worlds, and it is the reason a maze cannot become a set of
disconnected islands.

The target is the fixed dock/castle tile (`TILE_AIRSHIP` `$C9`, `TILE_BOWSER`
`$CC`), not a marching map object. Neither tile appears in
`Map_Removable_Tiles` or `Map_Completable_Tiles`, so **nothing in the engine ever
marks them** — the spine edge is repeatable, which is what keeps reachability
monotone and lets a fixpoint serve as the solver.

**Telepads are the chords.** 0–3 per world, biased to 1. They may lead anywhere,
including into a region reachable *only* by pad — a pad-only island holding a
fortress for a required lock is a deliberate and desirable shape.

**Pads are PAIRS.** Two pad tiles, one link, traversable both ways: step on A,
arrive at B; step on B, arrive at A. There is no such thing as an unpaired pad —
an odd leftover site is not placed at all.

**The budget is 16 ids, so 8 pairs.** `PORTAL_MAX = 16` arrival rows, one per
pad *tile*, shared with (now unused) pipe portals. Each half of a pair owns a
row, so a pair costs two. The cap is both the id encoding and the table bytes
(six arrival tables plus three key tables). Raising it is possible in PRG011 at
9 bytes per row, but the design targets 16.

> **A note on "one-way", because this wording cost a rewrite.** An arrival row is
> one-way *as a mechanism* — it maps one pad tile to one destination — and that
> is all the sentence "a two-way link costs two ids" ever meant. It is not a
> statement about the feature. **Pads are bidirectional pairs; airships are the
> one-way edges.** The "one-way is what stops the graph collapsing into a
> corridor" line under Topology is about the airship spine and nothing else.

**Cross-world locks.** Forward locks (fort in an earlier spine world, lock in a
later one) are always safe. Backward locks are the interesting ones — they force
a revisit — and are safe exactly when the pad web provides a return path, which
the global fixpoint decides.

## The generator

`WorldState::completable_sealed` is already the solver: close all locks, walk,
beat every reachable fort, open its locks, repeat. Each iteration is a sphere.
Over eight grids with cross-world edges it becomes both the global verifier and,
run incrementally, the generator.

### The decomposition

> **REVERSED 2026-09-07 — read this before the paragraph below.** The swap
> search is no longer the producer; the constructive forward fill is, with the
> swap search kept only as the fallback for gates the fill could not place
> (`fill.rs`'s `assign_keys` calls `constructive()` first). The reason the
> reversal happened is in `fill.rs`'s own doc comment: **a swap has nothing to
> aim with.** Measured, the constructive fill reaches 72% cross-world locks
> neutral / 73% preferring cross, against the swap search's worse spread — and
> the bridge gain came from the algorithm, not the aiming. The structural
> objection below turned out to be answerable rather than fatal: a stalled fill
> keeps the builder's assignment for what it could not place, which is the same
> safety net the swap search started from. See also "Measured 2026-09-06: the
> key-assignment fill rests on a false premise" later in this document.

**The forward fill was replaced by a swap search, and the reason is structural.**
The forward fill's first step needs a fortress inside the start region with
every lock closed — and the per-world builder deliberately puts forts *off* the
forced path, so the start region frequently holds none. A stalled forward fill
has to fall back to an arbitrary assignment for the remaining gates, which is
the retry loop it was meant to avoid. `maze/fill.rs` instead starts from the
builder's own assignment — every lock opened by a fort in its own world, already
known completable — and swaps the forts of two locks, keeping a swap only when
the maze is still solvable *and* no world's start region became a trap. It
cannot fail (the worst case is the assignment it started from) and it preserves
the fort/lock bijection for free, because a swap exchanges two forts between two
locks.

The obstacle to solvability-by-construction is that `locks.rs` today does two
jobs in one pass: it **places** a lock *and* **pairs** it with a fort, because
the placement guard (`completable() && completable_sealed(li)`) can only be
evaluated once the pairing is known. In a maze the first job is per-world and
the second cannot be. So:

- **Terrain and slots — per world, unchanged.** Grid shaping, pipe web, blank
  pools, and *where* content sits: level slots, fort slots, lock positions, pad
  positions.
- **Key assignment — global, new.** Which placed fort opens which placed lock,
  and which locks are locked at all.

The second pass is an assignment problem over positions that already exist,
which is why it needs no retry: start with every lock open, run the fixpoint for
sphere 0, then repeatedly close a lock and assign it a fort **from the currently
reachable set**. Closing a lock can still strand content, but that is one cheap
fixpoint check per step, not a rebuild. The pass never moves content between
worlds — levels stay in their own world, forts stay where placed — so the level
deck model is untouched.

**Standard mode is not touched in v1.** There is no clean seam to extract from
`locks.rs`, and sharing the code would move the overworld baseline and put the
change on the hook for the deep censuses (500-seed reachability, 1000-seed
route). The maze gets its own assignment pass; the duplication is honest,
because the two modes want different things — standard wants "a lock that cuts
this world well", the maze wants "a lock whose fort is anywhere in the reachable
set". Unify later only if a census says the maze's assigner subsumes the other.

### Two gate kinds

1. **Progression gates** — forward fill, ordered, produces a tree. Keys are
   always placed inside the already-reachable set.
2. **Free gates** — extra pads and locks whose forts are already reachable.
   They cannot break solvability, and they are where loops, shortcuts and decoys
   come from. Preserves vanilla's "a lock is a recovery shortcut" intent.

Build a bounded number of progression gates and dump the rest as free gates.
**Bound the fill by sphere count**, with a gate cap as a safety valve — sphere
count is the player-facing quantity.

### Where a pad LANDS: on its partner, and nowhere else

Two playtest reports, one cause. First a pad dropped the player on a blank
cell; then one dropped them on a pipe in another world; then they walked into a
spade panel expecting a pad and got the card game.

The first cut let a pad land on any placed slot, which was wrong twice over.
`stamp_slots` leaves `SlotKind::HammerBro` slots as plain path tiles, so **45%
of landings were on a cell the map draws nothing on** — but even fixing that to
"land on visible content" was fixing the symptom. **A pad's destination is its
partner pad**, always, and then the player can always see they have arrived and
can always go back.

The rules, all measured:

- **Every pad is half of a pair**, mutually pointing. No pad points at itself,
  no pad is unpaired, and the count is even and at most 16.
- **A same-world pair spans at least 8 grid cells** (4 map moves). The minimum
  measured span before this rule was **0** — a pad that teleported the player
  onto the tile they were standing on. A pair that cannot reach 8 inside one
  world becomes a crossing instead.
- **Landing on a pad does not re-trigger it.** All three branches reaching
  `PRG010_CEA7` sit downstream of an A-button *edge* test, so the hook fires
  when the player commits to a tile, not when they arrive on one. This is what
  makes pad-to-pad safe at all, and it is re-confirmed rather than assumed.

Cost to the shape of the maze, 60 seeds/arm: spheres 5.96 → 6.27, sphere width
2.85 → 2.71, pads per seed 9.30 → 8.63 (the dropped odd site), 97% of pairs
cross worlds. Nothing moved enough to change what the mode is.

### The pad has its own tile — `$DF`, and NOT a byte above the threshold

A pad used to be a spade panel (`$E8`), inherited from the POC where pads *were*
existing spade panels. Once the generator started stamping its own, that stopped
being free: on one played ROM there were **28 spade tiles of which only 9 were
pads**, World 2 had three and all three were card games, and nothing on screen
told them apart. Two out of three "spades" were a disappointment.

Pads are now `TILE_ALTSPIRAL` `$DF`. Three things had to hold at once:

1. **Enterable**, or the hook never fires — via **membership in
   `Map_EnterSpecialTiles`**, exactly as `$E8` reaches it.
2. **NOT at or above its page's `Tile_Attributes_TS0` threshold.** This is the
   trap, and it is worth stating loudly because the obvious choice walks into
   it: a page-3 byte `>= $E9` *is* enterable by threshold — and the same
   threshold is the engine's **level gate**. `MO_NormalMoveEnter` will not let
   the player walk *off* such a tile until it is completed, allowing only a
   reversal back the way they came ("you're not allowed to take the path until
   you complete that level!", `prg010.asm`). A pad can never be completed, because
   diverting at enter time means `MO_DoLevelClear` never runs. **A pad on a byte
   `>= $E9` would permanently sever every path it stood on** — invisible on a
   dead-end cell, fatal on a corridor.
3. **No CHR written.** The four quadrants point at existing patterns
   `$80`/`$82`/`$81`/`$83`, which close into a bright rectangular ring on black:
   a lit hatch, the inverse of every filled badge the map otherwise uses. Those
   are the same four tiles the wand-gate skull tried to *overwrite* — pointing
   at them is free, writing them was the bug.

Two constraints picked the art. Quadrants below `$80` **animate** (`Map_DoAnimations`
swaps patterns `$00`-`$7F`), which is why `TILE_LARGEFORT` is documented as
going visually corrupt; and **World 6's palette page 3 has colour 1 and colour 3
both `$30`**, so anything drawn on colour 3 is invisible there. A colour-1 ring
on colour-0 black is legible in all eight worlds.

Two bonuses fell out. `$DF` is **not** in `Map_Completable_Tiles`, so a pad
takes no packed-store bit — the worst case drops from 43 of 48 to **41 of 48**,
and "a pad stays a pad" becomes structural rather than observational. And it is
in `Map_Object_Forbid_LandingTiles`, so a marching Hammer Bro cannot land on one.

### Pad roles are the per-world interface

The per-world builder never learns that a world graph exists. It is handed a
count and a set of roles:

- **Hub pad** — in the start region, zero-key reachable. How a world satisfies
  invariant 3 when its airship path is gated.
- ~~**Island pad**~~ — see "The island pad, and why v1 does not have one" below.
- **Shortcut pad** — reachable, but behind a lock. A shortcut that opens later.

All three are queries against machinery that exists: `islands.rs` knows island
regions, `forced_positions` knows cut vertices, `locks.rs` knows what is behind
a lock.

Two consequences:

- **Pads are placed after the per-world builder finishes**, not before it.
  `roles::classify` sorts a *finished* world — `pad_sites` filters against
  slots that are already standing. Placing them earlier is precisely what an
  island pad would require, and is most of what the next section costs: a pad
  arrival is the only thing that could make an island region eligible to hold
  content, and content is dealt before any pad exists.
- **The role allocator needs a fallback.** Not every world's terrain has an
  island — W1 does not. So a role is a request, not a contract, and the
  distribution of granted roles is a census output.

### The island pad, and why v1 does not have one

**Corrected 2026-09-05, because the first version of this section got its own
evidence wrong.** It said "0 island landings in 800 world-seeds" and concluded
the terrain offered nothing to build on. The number was real but it measured the
wrong thing, and the conclusion happened to survive for a different reason.

What is actually true, measured over the same 800 world-seeds
(`maze_terrain_pools_census`):

```
island sites   128 total, mean 0.16, by world [0, 0, 100, 22, 0, 0, 6, 0]
```

So cells no walk reaches **do** exist — 100 in World 3, 22 in World 4, 6 in
World 7, which are the canoe-and-water worlds. `Connectivity` bridges islands
with pipes up to a budget, and what is left over is this. A pad tile could stand
on one.

**And pairing made it mechanically possible for the first time.** When pads were
independent one-way edges an island pad was incoherent — you could arrive
somewhere with no way back. A pair puts one half on the island and one on
reachable ground, and the island is enterable and leavable by construction.

**It still is not worth building, and this is the honest reason:** an island
holds no *content*. `HammerBroFill` claims every reachable blank and the content
phases run before any pad exists, so an unreachable region is an unreachable
region full of nothing. Landing there gets the player an empty cell and a pad
home. To make it the charter's "pad-only island holding a fortress for a
required lock", the pad has to be placed **inside** the per-world pass, before
`Levels` and `Forts`, with `Connectivity` told to leave that island unbridged
and `Locks`/`Shaping` — which guard on `completable()` — taught to believe in
the pad as an edge.

That is the real cost, and it was always the real cost; the earlier "the pool is
empty" framing made it look cheaper to dismiss than it is. `WorldTerrain::island_sites`
is kept and measured so the number stays visible, and `all_sites()` excludes it
so the placer can never draw one by accident.

### Boundary conditions

**The per-world builder still runs.** What changes is `start → target` becoming
**`entries → exits`**: entries are the tiles you can arrive on (pad arrivals,
the world's start tile), exits the tiles you must be able to leave from (pads
out, the airship, the castle). Everything per-world — grid theme, pipe web,
blank pools, level deck, hammer bros, toad houses, spades, the march graph — is
unchanged.

Its guarantee drops to two local obligations, and nothing else:

1. The start region has an ungated exit (invariant 3).
2. Every content slot is reachable from *some* entry, with locks held open.

Global solvability is no longer a per-world property and must not be checked as
one. That is what lets a world be genuinely incomplete on its own, which is the
whole point of the mode.

### Knobs

There are **two**, and both are defaults-are-null: at their default value each
does nothing, so a census with defaults reproduces the null-model baseline and
any movement is attributable to a single dial.

**`fort_distance_bias`** is the main one. `-1.0` keeps a lock's fort near it
(local, vanilla-ish); `0.0` accepts any solvable reassignment, which is the
null; `+1.0` pushes keys as far along the spine as the graph allows, which means
into other worlds. Magnitude is a probability rather than a switch, so the dial
is continuous.

**`foreign_landing_bias`** decides how often a pad lands in a world other than
its own. `0.0` makes every pad a same-world hop (position keying gives those for
free); `1.0` makes every pad a crossing. This is what decides whether the eight
worlds are a graph or eight rooms with local shortcuts.

**`maze_wands` (K) is the one that reached the option screen**, and the sweep is
why. It is not a length dial, it is a **floor** (`maze_wand_gate_sweep`, 40
seeds per K, zero unwinnable at every K):

| K | mean levels beaten | min | median | max |
|---|---|---|---|---|
| 0 | 26.6 | **1** | 29 | 42 |
| 1 | 26.8 | 7 | 29 | 42 |
| 2 | 27.3 | 13 | 28 | 42 |
| **3** | **27.8** | **16** | **28** | 42 |
| 5 | 29.8 | 16 | 29 | 42 |
| 7 | 36.4 | 24 | 38 | 45 |

At K=0 a telepad chain can drop the player beside the castle and some seeds
finish in a **single level** — the charter's "a telepad chain to World 8
trivialises the game", at its worst. K=3 raises the floor to 16 while the
*median run is unchanged*: the degenerate tail costs nothing to remove. The
floor then plateaus at 16 through K=5 and only moves again at 6-7, by which
point the median has climbed to 33 and 38. **3 is where the dial stops buying
and starts charging**, and it is the default.

The two generator biases stay internal constants. They move the mean by about
one level where K moves the floor by fifteen, so by the same discipline they
have not earned a control the player has to understand.

**Two knobs that were considered and rejected**, both on the discipline that a
knob has to be earned:

- **Lock density** — "what fraction of lock sites are live gates". It cannot
  exist: every fortress must have exactly one lock (a lock breaking is the only
  feedback that says which fort did it, and a world's lock count is how the
  player deduces its fort count), so the assignment is a bijection and the count
  is not a free parameter.
- **Key depth within a sphere** — the charter's proposed secondary dial. It is
  subsumed by `fort_distance_bias`: spine distance already orders keys from
  "same room" to "three worlds back", and a second dial over the same axis would
  need a census to tell the two apart before it earned its name.

### Choice survives at two scales

- **Local** — within a world, the existing route machinery works with
  entry/exit substituted. A world with two pads and an airship has *more*
  natural route choice than vanilla, because it has multiple exits. C1's
  definition survives; its calibrated baselines do not.
- **Global** — **sphere width**, the number of distinct gates openable in a
  given sphere. Width 1 is a corridor, width >= 2 is a real decision. This is
  the direct analogue of the existing "at least 2 routes in the choice band",
  one level up, and the spoiler log should report it per sphere.

New instruments to earn by census: sphere count, sphere-0 openness, sphere
width, bottleneck count, backtrack cost. The **per-sphere spoiler log** is the
primary debugging tool and should be generated *during* the fill, as a
first-class output like `BuildResult`, not reconstructed afterwards.

**The standing bill:** two sets of censuses forever, paid every time either
overworld mode is touched.


### How long is a maze game

`maze_game_length_census`, 60 seeds at the shipping K=3. The unit is content
beaten — a level tile cannot be walked past, so "how far is the castle" is
naturally measured in levels, and a fortress is a level you also have to play.

| | mean | min | median | max |
|---|---|---|---|---|
| **beaten on a completion** | **26.6** | 11 | 27 | 42 |
| strictly required | 12.9 | 2 | 12 | 21 |

Of 62 levels + 17 fortresses, **a run plays about 43% of the game and about 21%
of it is unavoidable**; ~11 of the fortresses beaten are detours the lock/key
structure forced rather than content on the way.

"Strictly required" is the cut-vertex sense: blocking that level makes the game
unwinnable. It is much smaller than what a run beats, and the gap is the size of
the choice the maze offers — a low required-count with a high played-count is a
map with real alternatives, not a short one. The two are measured by different
instruments on purpose (`required_levels` re-runs the global fixpoint 62 times
per seed; `completion_cost` simulates a player who beats exactly what they must),
so a bug in one does not move the other.

### How the baselines get made

No knob below has a measured value, and none can until something exists to
measure. The way out is the one the current builder used: **measure a null
model first, then add one lever at a time.**

**The null model's instrument is gone; its numbers are the record.** The
uniform placer in `maze/pads.rs` was deleted once the real placer existed,
because it emitted *unpaired one-way* pads landing on arbitrary slots — a shape
the generator can no longer produce, and one the design doc itself records as a
playtest bug. A census over it measured a maze this code cannot emit, which is a
trap rather than a baseline. The table below stays: it is what every later lever
was argued against.

The null model was the crudest thing that is still a maze:

- eight per-world builds, unchanged (`overworld_build::build` as it stands);
- the spine assumed — world *i*'s airship goes to world *i+1*, which needs no
  ROM change to measure, only an edge the verifier believes in;
- pads placed **uniformly at random** on legal blanks, roles ignored, budget
  respected, destinations random;
- no fill and no assignment pass: every lock keeps a fort in its own world.

Run the verifier over N seeds and read off sphere count, sphere-width
distribution, how often invariant 3 holds by accident, and the solvable rate.
Those are the baselines. They are not a target — they are "what happens if we
connect eight worlds and change nothing else", and every knob afterwards is
argued as a delta against them.

The null model is *expected* to fail often. That failure profile is the
measurement: it says which levers are load-bearing rather than which sound
clever. If invariant 3 already holds in most seeds, the start-region rule is a
cheap guard rather than a placement phase; if sphere count is already 4 with no
fill, the progression-gate machinery is smaller than planned.

This mirrors `forts.rs`, which measured uniform placement (24% of forts forced)
before adding its single preference, and it is the ladder discipline: one lever,
measured, kept or discarded.

### The null-model baselines — MEASURED (2026-09-05, 200 seeds)

`maze_null_model_census` and `maze_pads_vs_spine_only`, over the same flag arms
the overworld census uses (50% base / 25% more-hammer-rocks / 25% 8s-are-wild,
start↔airship swap rolled per world):

| Quantity | Null model |
|---|---|
| solvable | **100%** |
| spheres | mean 6.01 (min 3, max 12) |
| sphere width | 0:17% 1:23% 2:17% 3:12% 4:10% 5:7% 6:5% 7:3% 8+:7% |
| sphere-0 openness | 18.7% of all content reachable with zero keys |
| goal sphere | mean 3.46 (max 11) |
| wands before goal | mean 5.76 of 7 |
| pads placed | mean 9.25, max 16 |
| **invariant 3** | **55.8%** (893 of 1600 world-seeds) |
| spheres, spine only (no pads) | mean 10.79 |
| pads shortened the game | **99.5%** of seeds |

Four findings, and they redraw the plan:

1. **Solvability is not the problem, and was never going to be.** The per-world
   builder already guarantees each world completable from its own start, and
   the spine deposits the player on exactly that start — so the null model is
   solvable by construction. `the_spine_alone_completes_the_maze` keeps that
   honest: a failure there means the maze walker or the fixpoint is wrong, not
   the map. The generator's job is *shape*, not *validity*.

2. **Invariant 3 as the charter worded it is a real placement obligation — but
   the charter worded it too strictly.** "An exit reachable with zero keys"
   holds barely half the time (55.4%), because the per-world guarantee is start
   → target *with locks openable* and the invariant asked for it *with every
   lock closed*. But a fortress inside the start region **is** a key the player
   can go and get, and the property that actually prevents a soft-lock —
   ungated exit **or** a fort that opens one — holds **100% of 800
   world-seeds**. The rule is a cheap guard; the pad budget stays free.

3. **Uniform pads are a pure shortcut, and they gut the game.** They cut the
   sphere count nearly in half (10.79 → 6.01) in 99.5% of seeds, and drop the
   wands collectable before the castle from 7 to 5.76. That is the charter's
   "a telepad chain to World 8 trivialises the game", now with a number on it.
   Roles are therefore load-bearing rather than a refinement: a pad web placed
   without them *removes* progression instead of adding it.

4. **The progression-gate machinery is smaller than planned.** Sphere count is
   already 6, not the 4 the doc guessed a fill would be needed to reach — the
   per-world lock chains supply it. What is missing is not depth but *width
   discipline*: 40% of spheres are width 0 or 1 (a corridor), and the tail runs
   to 8+ where a whole world opens at once.

### Build order — all five steps done

1. ~~**`GlobalState`, a directed cross-world walker, and the global fixpoint —
   as a pure verifier.**~~ **DONE** (2026-09-05) — `src/randomize/maze/`, with
   the cross-world walker beside `walk_reachable` in `map_walker.rs`. The
   spoiler log is `Spheres::spoiler`. Shipped with the crude uniform pad placer
   in `maze/pads.rs`, since **deleted** — see below.
2. ~~**Census the null model.**~~ **DONE** — the table above.
3. ~~**World-graph pass**~~ **DONE** — `maze/graph.rs`. Spine order, pad counts
   and roles per world, and the id budget spent in strict priority order:
   safety pads first, then shortcut, then free.
4. ~~**Pad placement** by role~~ **DONE** — `maze/roles.rs`. On finished worlds
   rather than in the per-world pass; see "The island pad" for why that is
   enough, and what it costs.
5. ~~**The key-assignment fill**~~ **DONE** — `maze/fill.rs`. Shipped first as a
   swap search, then reversed on 2026-09-07 to the charter's constructive
   forward fill, with the swap search kept as the fallback; see below.

### Step 1 in detail

**Its input already exists.** `build()` returns `BuildResult { worlds:
Vec<BuiltWorld> }`, and each `BuiltWorld` carries that world's `grid`, `slots`,
`locks`, `pipe_pairs` and `section_count`. `sources.rs` already constructs a
`WorldState` from one. So step 1 needs one `build()` call and eight state
constructions — no pipeline plumbing, no writer changes, and **no ROM writes at
all**.

What gets written:

- `GlobalState` wrapping the eight `WorldState`s plus the edge set — a struct
  and a constructor.
- **A global walker, written beside `walk_reachable` rather than replacing
  it.** It needs a world-indexed frontier and directed teleport edges; keeping
  it separate leaves standard mode's walker untouched, for the same reason the
  key-assignment pass does not disturb `locks.rs`.
- The global fixpoint: `completable_sealed`'s loop over eight grids instead of
  one, returning `Spheres` rather than a bool.
- The crude uniform pad placer, so a null model exists to point at.
- A census test in the existing `CENSUS_SEEDS` style.

Roughly 400-600 lines including tests; about half is the walker.

**Make the module `#[cfg(test)]` for now.** It is a measurement instrument and
nothing in the randomizer calls it. That also sidesteps the ungating trap: the
`cfg(not(wasm32))` gate on the existing maze modules exists precisely because
unreferenced code fails CI's wasm clippy pass, and `#[allow(dead_code)]` is what
CLAUDE.md forbids. The gate question only arises when the generator wires in.

**The one integration unknown:** whether eight `WorldState`s can be built
outside `build()`'s per-world loop without a small accessor — `WorldState` is
`pub(crate)` within `overworld_build` and its construction currently happens
inside that loop. A ten-minute problem, but it is the thing most likely to make
step 1 touch a file this document says it will not.

#### What step 1 actually cost (2026-09-05)

**The integration unknown was a non-problem.** `from_built` already wraps a
`BuiltWorld` back into a `WorldState`, so the maze is eight calls to it over
`BuildResult::worlds` — no accessor, no plumbing, and no file this document said
would be left alone was touched. The only change outside `src/randomize/maze/`
is the cross-world walker appended to `map_walker.rs`.

**The walker is a copy, and the copy is guarded.** `maze_reach_from` duplicates
`reach_from`'s 2-tile move expansion rather than sharing it, because sharing
means editing the walker every overworld census depends on. What stops the copy
drifting is `maze_walk_matches_the_per_world_walker`: over every world of every
census seed it asserts the maze walker, given the eight worlds and no links,
reproduces `walk_reachable` cell for cell — and reaches nothing at all in the
other seven. The oracle is the shipping walker, not a hand-written expectation.

Two semantics the walker had to get right, neither of which is in the sketch:

- **The target tile is a walk sink, but the spine edge leaves it.** `reach_from`
  refuses to expand out of `TILE_AIRSHIP` / `TILE_BOWSER` at all. The maze
  version skips only the *directional* moves and still fires outgoing links —
  an airship you cannot walk through is still an airship you can clear.
- **Canoes become a fixpoint, not a pre-pass.** `canoes_reachable` asks "can the
  player walk to a dock from the start, without canoes". Across worlds a pad can
  drop the player straight onto a dock in a world they have no other route into,
  so canoe activation is re-tested after each pass until it stops changing.
  Monotone, so it runs at most once per world with water.

## Invariant 3 is RETIRED (2026-09-07)

*"Every world's start-tile region must contain at least one exit reachable with
zero keys"* is no longer enforced, and `start_region_escapable` is no longer a
generation gate. Two independent reasons, and the second is why it is safe
rather than merely convenient:

1. **The constructive fill cannot strand anyone.** Every gate takes its key from
   a fortress already reachable *from the global start* at the moment it is
   placed, so every gate it writes is openable — strictly stronger than the
   rule. The rule also counts only a world's OWN fortresses as openers, so it
   rejected the mode's own formula: pad out, beat a fortress there, come back.
2. **The whistle is the escape hatch.** Never consumed, survives a game over
   (nothing on that path clears `Inventory_Items`), and the cycler always has
   the spine's first world to return to. A trapped start region costs a hop.

**What enforcing it cost, measured over 60 seeds:** 2.1 crossings per seed
forced back to local keys, 33% of seeds falling back to the swap search, and —
because `generate` answered one stranded world by discarding *everything* —
seeds shipping with no cross-world locks at all. Retired: cross-world locks
51% -> 76%, the World 8 bridge 37% -> 73%, nothing falling back.

**The replacement assertion is *sphere 0 must beat a fortress*** — the run has
to be able to begin. Three tests carry it.

**The whistle is now a safety property.** If the mode ever ships without one,
game over must return the player to the spine's first world rather than the one
they died in: set `World_Num` to the spine head on the continue path before the
map re-inits (vanilla lands them in place via `GameOver_AlignToStartY`). That is
also the transition signal the packed completion store already tests
(`World_Num != LIVE_WORLD`), so the persistence hooks fire correctly for free.

## Invariants

1. **Global completability.** The fixpoint reaches the castle and every fortress
   is beatable — no fort sealed behind its own lock, now across all worlds.
2. **Zero hammers.** Solvable with `has_hammer = false`.
3. **The start-region rule** — *a player who arrives in a world must be able to
   leave it again.* **Revised 2026-09-05, and weakened deliberately.**

   The charter asked for an exit reachable with **zero keys**. Measured, that
   holds only 55.8% of the time, and the target is ungated by terrain alone in
   just 7.8% of worlds — so as stated it is a placement obligation on every
   world, not a cheap guard.

   It is also stricter than safety needs. A fortress inside the start region is
   a key the player can go and get; what actually soft-locks is a start region
   with **no ungated exit AND no fortress that opens one**. `start_region_escapable`
   asks that instead, counting only the world's OWN forts (the worst case: a
   player arriving for the first time has beaten nothing here, and a lock the
   fill paired with a foreign fort can never be opened from the inside).

   The generator treats it as a hard invariant: hub pads are the first claim on
   the 16 arrival ids, the fill re-checks it on every swap, and anything still
   failing gets a rescue pad from whatever budget is left.

   **Measured, the correct form holds 800 of 800 world-seeds — 100%**, against
   55.4% for the charter's wording (`start_region_exit_rate`). So the
   start-region rule is a **cheap guard, not a placement phase**: the machinery
   stays because a safety property with no enforcement is a bug waiting for a
   rare seed, but it claims no pads in practice and the whole 16-id budget is
   free for maze shaping. The generator census confirms it from the other side —
   **0 hub pads requested** across every arm.

   This is the whole of the game-over and whistle safety story, and it is much
   narrower than "the pad island must be escapable". Dying on a pad-only island
   does not strand you there; it drops you at the world's start tile, and the
   island is re-enterable by its pad from the other side. The only thing that can
   soft-lock is a start-tile region with no way out.

   It is nearly free, because every world has an airship and the per-world
   builder already guarantees start → target. The maze version only strengthens
   that to "with all locks closed" — one ungated path. Everything else in the
   world may still be gated.

   The fill may satisfy it either way: leave the airship path ungated, or place
   a pad in the start region.

4. **Whistle: a world counts as visited only once its start tile is touched.**
   Game over, airship arrival and whistle travel all deposit the player on a
   start tile, so invariant 3 certifies all three destinations at once. A world
   only ever pad-hopped into an island of is correctly not whistle-able. Costs
   one bit per world plus a position compare on the map idle frame.

## Persistence and world transitions

The ROM-level mechanics — that a world map is never saved but recomputed from
PRG012 grid data plus the `Map_Completions` bitfield, that the bitfield walks
both halves, and who writes which half — are documented in
`smb3_rom_reference.md`, "World transitions and per-world map state". This
section records only what the maze adds.

### The store

Eight worlds of `Map_Completions` are packed one bit per completable cell into
`$7997`, two planes of `PLANE_RESERVE = 48` bytes (Mario's and the permanent
mirror; the split is a distinction the engine makes, not a copy).
`LIVE_WORLD` (`$7ABC`) names the world currently expanded over `$7D00`.

`LIVE_WORLD` exists because `World_Num` cannot answer "which world am I
leaving?" at the hook: **both of vanilla's paths into `PRG030_84A0` set
`World_Num` to the destination first** — `INC World_Num` on the airship,
`LDA Map_Warp_PrevWorld / STA World_Num` in the warp zone.

### The two hooks, and the bug they had — **FIXED 2026-09-05**

| Hook | Where | Fires when |
|---|---|---|
| pack (`WIPE_REPLACEMENT`) | the displaced wipe, early in `$84A0` | ~~`TRANSITION_FLAG` set~~ → `World_Num != LIVE_WORLD` |
| expand (`SWAP_AT_RELOAD`) | later in the same init | `World_Num != LIVE_WORLD` |
| new game (`NEW_GAME_INIT`) | the title screen's game-start init | always, once, when a game begins |

They used to be two different definitions of "a transition happened", and they
disagreed on exactly vanilla's own two world-change paths, neither of which
raised the flag.

Observed on `maze_J`: clearing an airship packs nothing (flag clear, pack
returns untouched, `LIVE_WORLD` still names the world being left), then the
expand hook sees the mismatch and lays the destination world's plane over
`$7D00`. **The outgoing world's progress is discarded having never been
packed.** The loss is one-directional and happens on every airship transition.

Note that the pack routine itself is already correct on this path — it packs
`LIVE_WORLD`, not `World_Num`. Only the trigger was missing.

### The fix: a new-game signal, then one transition test

Raising `TRANSITION_FLAG` at vanilla's two sites would work, but it costs bytes
in PRG030 (`FS_WORLD_ORDER` is 28 bytes and uses exactly 28) and preserves the
fragility — the bug was "we missed a path", and a flag guarantees there will be
another one.

Testing `World_Num != LIVE_WORLD` at the pack site cannot work on its own: with
the flag gone, a clear flag plus a world mismatch is ambiguous between a real
transition and a new game, because the title screen falls *through* into
`$84A0`. Disambiguating those is what the flag currently does.

So: **give the maze a real new-game signal, then both hooks can use the
compare.**

- The signal is the title screen's game-start init, `LDA #$00 / STA World_Num /
  STA Debug_Flag` at file `0x30CC2` (PRG024, CPU `$ACB2`). `world_order` already
  patches the `#$00` operand at `0x30CC3` and already NOPs the three bytes of
  `STA Debug_Flag` at `0x30CC7`. **A `JSR` is exactly three bytes**, so the call
  costs nothing new at the site.
- The routine lives in PRG025, which the title entry maps at `$C000` while
  PRG024 is at `$A000`, and which has ~2.7KB free. ~20 bytes: zero the packed
  store, zero `$7D00` (vanilla's wipe, performed in the one place it is
  correct), set `LIVE_WORLD = World_Num`.
- Both hooks then test `World_Num != LIVE_WORLD`. `TRANSITION_FLAG` is deleted,
  along with the requirement to have found every path that changes worlds.
  `PAD_ENTER` and the debug world jump hand back the bytes that raised it, and
  the airship path needs no PRG030 rent at all.

**As built (2026-09-05).** `NEW_GAME_INIT` is 25 bytes at `FS_NEW_GAME_INIT =
0x33FC8`, CPU `$DFB8` in PRG025, sited immediately *before* `FS_TITLE_MUTE`
rather than at the head of PRG025's free run — a bundled third-party title hack
writes from the first `$FF` it finds, which is `0x33529`, so the head of that
run is not a safe place to live. `WIPE_REPLACEMENT` shrank 33 → 31 bytes and
`PAD_ENTER` 114 → 111. `$7ABD`, which held `TRANSITION_FLAG`, is free SRAM
again.

**The routine had to do a fifth thing the plan did not list: zero `$7D00`.**
With compare-based hooks, the *first* `$84A0` of a new game compares **equal**,
so `SWAP_AT_RELOAD` never expands a plane over the live array either — and
vanilla's own wipe is gone, because that is the instruction the pack hook
replaced. Without the explicit zeroing a new game opens with the previous run's
completions lit in its starting world and packs them into the store at the first
transition.

**The test that would have caught the original bug now exists**:
`clearing_an_airship_packs_the_world_it_leaves` reads the engine's own `INC
World_Num` out of the ROM, executes it, then drives both hooks and asserts the
outgoing world's bit reached its packed plane. There was no test on the airship
path before, which is why a flag-versus-compare mismatch survived to a
playtest.

This also closes the standing "cold boot with dirty SRAM / game over into a new
game inherits the store" hole, since nothing can happen before Start is pressed.

Two preconditions, both already true:

- **Continue does not run the new-game init.** Vanilla Continue returns the
  player to the world they died in; that init sets `World_Num = 0`. If Continue
  ran it, vanilla would restart at World 1.
- **Attract mode is already disabled** on every build — `title_screen.rs` writes
  the demo-trigger operand to `#$00` unconditionally, not behind an option.

### Game over: force No Game Over Penalty on

MaCobra52's "No Game Over Penalty" (`qol/macobra.rs`) is currently an opt-in
option defaulting to false. **Maze mode should force it on.**

Its routine at `$BD40`/`$BD46` tests the map tile being considered for reset and
allows the wipe only for `$50`, `$E0` and `$E8` — the two toad houses and the
spade panel — skipping it for everything else. So with the patch on, a game over
makes toad houses and spade panels replayable and touches nothing else: level
clears, busted locks, cleared forts, bridges and removed rocks all survive.

That makes game-over semantics uniform across all eight worlds with no new code.
The live world keeps its progress because the wipe is skipped; the other seven
keep theirs because they sit packed in SRAM untouched. The asymmetry — losing
level clears only in the world you died in — exists only when the option is off.

Two couplings to remember:

- ~~**`$E8` is the telepad tile.**~~ **Resolved.** The patch clears completion for
  `$50`/`$E0`/`$E8` cells on game over, which used to be a live dependency
  between an imported IPS and the maze's tile choice. Pads are `$DF` now, which
  the patch skips — and skipping is harmless because `$DF` is not completable.
- **The patch's fourth write is at file `0x3D314` = CPU `$9304`**, about ten
  bytes from `PRG030_9314`, the game-over `AND` of the two completion halves that
  the packed-store design rests on. Two patches now edit the same short stretch
  of game-over code; check a seeded `--write-log` with both enabled.

## Cross-world locks

> **SUPERSEDED by the fortress-FX rework (2026-09-06), one day after this was
> written.** Everything below reasons about vanilla's 17 FX slots
> (`FX_MAP_COMP_IDX`, `FortressFX_W1_W8`, `FortressFXBase_ByWorld`) and about
> budgeting cross-world locks against them. That whole chain is deleted: one
> jump-table word now points map operation 8 at a from-scratch, position-keyed
> routine, so there are no slots left to run out of and no separate cross-world
> mechanism — `lock_keys.rs` holds every lock in the game, home and away, in one
> table. **The current mechanism is `docs/fx_table_redesign.md` § "The design".**
> Kept because the engine facts it establishes (what `Map_Reload_with_Completions`
> replays, what an FX slot actually buys) are still true of vanilla and are what
> the replacement was designed against.

`Map_Reload_with_Completions` rebuilds the map from ROM and replays the
completion bitfield, swapping tiles via `Map_Removable_Tiles`. The lock tiles
are in that table, so **setting the bit is sufficient** — the FX system is not
involved in persistence at all. An FX slot buys the *animation* plus the bit
write at fort-clear time, nothing more.

Therefore:

- **Same-world locks need nothing new.** They already work; that is why phase 1
  works.
- **A cross-world lock needs one thing: the bit routed to the right world's
  storage.** The FX writes into `$7D00`, which is whatever world is loaded, so
  beating a fort in world 3 to open a lock in world 5 would mark the cell in
  world 3's array — the same aliasing that put a completed marker on a W2
  pyramid during the POC.
- **A cross-world lock consumes no FX slot.** There is no animation to play in a
  world you are not standing in.

The mechanism: a **"foreign locks busted" bitmask** in SRAM (one bit per
cross-world lock; 3 bytes covers 24), set on the fort-clear path, and applied at
map load — for each foreign lock belonging to the world being entered whose bit
is set, set its completion bit in `$7D00` before
`Map_Reload_with_Completions` runs. Simpler than writing into a packed plane,
which would need the target world's stencil derived while that world is not
loaded.

**The FX slot budget therefore constrains same-world locks and bridges only.**
There are 17 slots in the ROM (`FX_MAP_COMP_IDX`, 17 x 2 bytes), shared across
all eight worlds and already partly spent on W1's and W8's bridges. Cross-world
locks sit outside that budget.

Note for anything that reaches for the FX table directly: the engine resolves a
fortress's slot as `FortressFX_W1_W8[FortressFXBase_ByWorld[World_Num] +
$0745]`, so **slot selection is indexed by the current world** — a fort can only
ever name its own world's slice. Routing the bit around the FX system avoids
having to change that.


### The hook that sets a foreign bit

`Map_MarkLevelComplete` (PRG011), fortress branch at **`PRG011_BA7C`**. The
engine reaches it only when the tile the player is standing on is
`TILE_FORTRUBBLE` or `TILE_ALTRUBBLE` — i.e. a fortress was just beaten — which
is exactly the event a foreign lock needs, and it is a different path from
`MO_DoFortressFX`, so nothing about same-world lock FX is disturbed.

**The fort's identity is its position, and the position is already decomposed
into completion-array coordinates when we arrive:**

| register | holds |
|---|---|
| `Y` | column index — `(World_Map_XHi << 4) \| (World_Map_X >> 4)`, plus `$40` if the current player is Luigi |
| `X` | `Temp_Var13`, the completion row index (0-7), resolved through `Map_CompleteY` |
| `World_Map_Tile` | the rubble tile, already swapped |

So a foreign-lock table keys on `(World_Num, column & $3F, row)` — the same
"where is the player standing" key `PAD_ENTER` already uses, and no fort-id
numbering has to be invented or kept in sync. Mask `Y` with `$3F` to drop the
player half.

The routine lives in PRG011, which is mapped by definition since the hook is in
it. (This paragraph used to quote "390 bytes free, largest gap 277" — a
2026-09-05 measurement that the allocations made since have overtaken. Never
hand-copy those figures; run `smb3-rs <rom> --free-space`.)

**Two things to verify when building it:**

- Whether a fortress cleared by a *secret exit* still reaches `BA7C`. If it
  does, foreign locks open on a secret exit where same-world locks do not —
  strictly better than vanilla, and it would make the secret-exit question moot
  for foreign locks, but it must not be assumed.
- The exact splice. The three instructions in the block are all 3 bytes
  (`LDA/ORA/STA Map_Completions,Y`), and `BA89`'s `RTS` is shared with the
  non-fortress and skid-back paths, so the hook has to land inside the fortress
  branch and preserve `A`, `X` and `Y`.

## The wand gate

The goal gate is **not a lock tile**. A lock that no fortress opens teaches the
player the wrong rule about every other lock on the map, which is squarely the
legibility charter's problem.

It is a **wall**, cloned from World 8's own masonry:

- `0xE2` is the Dark Land wall — palette page 3, CHR quadrants `6C 6D / 6E 6F`,
  **blocks all four directions already**, and is in no behavior registry. As a
  barrier it costs nothing: no CHR work, no registry work, and it reads as
  castle masonry.
- **Do not add `0xE2` itself to `Map_Removable_Tiles`.** `IS_COMPLETABLE` tests
  a tile against `Map_Completable_Tiles` and `Map_Removable_Tiles`, and that is
  exactly what the packed store's stencil counts. Making `0xE2` removable would
  turn all **155** of W8's wall cells into completable cells — roughly 20 bytes
  of plane for W8 alone, against `PLANE_RESERVE = 48` for all eight worlds with
  a measured worst case of 42. The store would overflow into the mirror.
- **Clone the byte instead.** Take one of the 117 tile bytes unused in every
  world grid, in palette page 3, and write `6C 6D 6E 6F` into its four quadrant
  entries (four bytes of metatile data). Pixel-identical to the Dark Land wall,
  distinct identity, stencil grows by exactly one cell.

That also answers the hammer objection better than a lock tile would: a byte
that is neither a rock nor a lock is broken by nothing at all, so
`hammer_breaks_locks` cannot touch it. (That flag stays a player choice — a
player who enables it and hammers past something has chosen that, and the
zero-hammer invariant is unaffected either way.)

> **NOT WHAT SHIPPED (both paragraphs below).** The wand gate never joined the
> removable/completable tables: `wand_gate.rs` stamps `WAND_GATE_TILE` (`$D5`)
> into the grid directly and lifts it from its own routine, hooked on map reload
> and fill-attr. Its tests pin exactly that — `the gate byte is removable` and
> `the gate byte is completable` are both *assert-not*. The table relocation
> that did land (2026-09-07) was for the hint-locks feature's sky variant, and
> it used a 32-entry stride, not the 9-byte copies planned here. Kept for the
> costing, which is still a fair account of what growing those tables takes.

**Cost of the ninth removable entry.** The tables are packed and adjacent —
`Map_Removable_Tiles` (8) at `0x18447`, `Map_RemoveTo_Tiles` (8) at `0x1844F`,
`Map_Completable_Tiles` (5) at `0x18457` — and the loop bound is an `LDX #7`
immediate. Growing in place is impossible; relocate both tables to 9-byte copies
and repoint three absolute operands (`CMP Map_Removable_Tiles,X` and `LDA
Map_RemoveTo_Tiles,X` in PRG012, plus our own `IS_COMPLETABLE`, which reads the
same table) and two `LDX` immediates. PRG012 has 908 free bytes with a 576-byte
gap at `0x19DD0`, and PRG012 is mapped whenever this code runs. (Those two
figures are a 2026-09-05 measurement and are now well out of date — see
`--free-space`.) The gap still
needs the unreferenced check before it is claimed.

**Opening it** is then the same mechanism as everything else: when the wand
counter reaches K, set the gate cell's completion bit (both halves, so it
survives a game over). No FX slot, no animation — the map reloads on the way
back from the airship level anyway.

**Hazard to hold the line on:** map rows 7 and 8 share completion bit `$01`.
Today a stray bit landing on a wall cell does nothing, because no wall is
removable. With a removable wall on the map it would blow a hole in it. The
builder's shared-bit rule (#212) predates any removable wall, so the gate cell's
row/column pairing has to be held to it explicitly.

## Data model

Sketch, not a signature list — the point is that the maze's state is a thin
wrapper over eight existing `WorldState`s plus a small edge set.

```rust
struct GlobalState {
    worlds: Vec<WorldState>,   // the existing per-world state, untouched
    edges:  Vec<MazeEdge>,     // pads and spine edges, all directed
    locks:  Vec<MazeLock>,     // global: a lock's fort may live anywhere
    start:  (usize, Pos),      // starting world's start tile
    goal:   (usize, Pos),      // the castle cell
    wands_required: u8,        // K
}

enum MazeEdge {
    // Repeatable, one-way. A two-way link is two Pads and two arrival ids.
    Pad { from: (usize, Pos), to: (usize, Pos) },
    // Repeatable, one-way, grants a wand. One per world, spine order.
    Airship { from_world: usize, to_world: usize },
}

struct MazeLock {
    world: usize,
    pos:   Pos,
    fort:  Option<FortRef>,    // None until the fill assigns it
}
// foreign == fort.world != lock.world; it is derived, not stored.

struct PadSlot {
    id:   u8,                  // 0..15 — the arrival row it owns
    pos:  Pos,
    role: PadRole,             // Hub | Island | Gated
    dest: (usize, Pos),
}
```

**Everything is monotone**, which is what makes a fixpoint a sound solver:
pads are permanent, forts are permanent, and the spine is repeatable because
`TILE_AIRSHIP` and `TILE_BOWSER` are in neither `Map_Removable_Tiles` nor
`Map_Completable_Tiles`, so nothing ever marks them.

**The walker changes in exactly two ways:** the frontier becomes
`(world, row, col)`, and teleport edges become *directed*. `pipe_pairs` are
already teleport edges in `walk_reachable`; a maze edge is the same thing with a
world index on the far end and no implied reverse.

The fixpoint's output is the spoiler log, not a bool:

```rust
struct Spheres {
    solvable: bool,
    spheres:  Vec<Sphere>,
}
struct Sphere {
    reached: Vec<(usize, Pos)>, // newly reachable this round
    opened:  Vec<usize>,        // lock indices whose forts became beatable
    width:   usize,             // gates openable on entry — the choice metric
}
```

## Acceptance criteria

What a build has to prove, and where. The rows marked **exists** landed with
the verifier (step 1) and run in `src/randomize/maze/tests.rs`.

| Check | Statement |
|---|---|
| `maze_is_solvable` | the global fixpoint reaches the goal **and** every fortress is beatable, over N seeds — **exists** as `the_spine_alone_completes_the_maze` / `every_placed_slot_is_reached` |
| `maze_needs_no_hammer` | the same with `has_hammer = false` — **free**: the map walker reads rocks as walls, so every run above already IS the zero-hammer run |
| `every_start_region_has_an_exit` | invariant 3, per world per seed, with all locks closed — **exists** as `start_region_exit_rate`, and currently *measures* 55.8% rather than asserting; it becomes an assert when a placement phase owns it |
| `wands_are_collectable` | at least K airships reachable without passing the goal gate — **exists**, and the guarantee test asserts a lazy player actually collects exactly K at K = 0, 3 and 7 |
| `a_maze_rom_puts_a_pad_tile_under_every_arrival_key` | **end-to-end**: decode every pad key row out of a finished ROM and demand a **telepad** tile there. The pad tables and the map are written by different modules, and one row of disagreement leaves either a pad tile that does nothing or a teleport out of a cell with no pad drawn on it — the exact failure that cost a playtest session. Mutation-tested: stamping one row low fails it |
| `the_pads_still_fit_the_packed_store` | the pad tile `$DF` is **not** completable, so pads no longer grow the packed planes at all: 41 of 48 at the worst measured seed, against 43 when pads were spade panels |
| `every_pad_is_half_of_a_pair` | every pad's destination is another pad's tile and that pad points back; none points at itself; the count is even and at most 16; every same-world pair spans at least 8 cells. Subsumes the old `pad_ids_fit` — **exists** |
| `packed_planes_fit_their_reserve` | **exists today**, and neither the wand gate nor the pads cost it anything any more — the gate does not join `Map_Removable_Tiles`, and `$DF` is not in `Map_Completable_Tiles`. Worst case **41 of 48**, seven bytes of margin. The failure mode is silent, which is why it is asserted: the planes would run into the arrival variables sitting immediately after them |
| `the_maze_leaves_every_pipe_alone` | **exists today** — pipe tables and world pointer tables stay byte-identical |

Census outputs, none of which have a measured baseline yet: sphere count,
sphere width distribution, sphere-0 openness, backtrack cost, granted-vs-
requested pad role distribution.

## Answered (2026-09-05)

- **Where the wand counter lives, and the hook that increments it.**
  `maze_state::WAND_COUNT` at SRAM `$7AC9`, in the `$7AC1-$7ADF` run the maze
  owns; bumped by a 16-byte routine in PRG030 chained through
  `world_order`'s `INC World_Num` replacement, which is the one site an airship
  clear always passes.
- **The player starts holding the whistle, and keeps it.** It is the mode's
  fast travel, so it is granted at the first frame rather than hidden: an
  earlier cut pinned one into a toad-house chest to guarantee it existed, which
  made the mode's core tool something to go and find. `completion_bits`'
  new-game init writes it into **inventory slot 0**, and `qol::starting_state`
  is told to start the player's own items at slot 1, so three requested items
  still fit and none of them is displaced.

  Slot 0 is not arbitrary. The inventory is a **compacted list**, not an
  addressed array: the engine gives every item the first free slot, and using
  one memmoves the tail down over it. Its panel handler reads slot 0 first and
  returns immediately when it is empty (`PRG026_A4A1`) — no cursor, no use — so
  a hole at the bottom is a dead panel, not a blank square. Two earlier cuts
  fell into this. Merging the whistle into the starting-items list ate one of
  the three the UI offers; moving it clear of them, to slot 3, meant that with
  the default of *no* starting items the whistle sat above three empty slots
  and could never be reached at all. The rule the two writers now share: fill
  from slot 0 up, never leave a gap, and the whistle takes the bottom because
  it is the one item the player is guaranteed to be holding.
- **The whistle is not consumed.** Vanilla's `Inv_UseItem_WarpWhistle` ends
  with `JSR Inv_UseItem_ShiftOver`, which deletes the item — correct for a
  one-shot warp, fatal for fast travel: one whistle would buy exactly one trip,
  and the mode's promise is that using it again takes you on to the next world.
  Those three bytes are NOPped in maze mode. It is safe to give away because a
  maze whistle **can never reach anywhere new** — the cycler only visits worlds
  whose `VISITED` byte is already set — so an unlimited whistle is unlimited
  *backtracking*, not a sequence break. That is the same argument
  `remove_whistles` rests on.
- **The whistle's picker** — no picker. It cycles: each use advances to the next
  world whose `VISITED` byte is set, wrapping, and with one visited world it is
  a no-op that puts you back on your own start tile. The warp zone is not reused
  and is now unreachable: the hook replaces `WWFX_WarpDoWind`'s
  `World_Num = 8` outright.
- **Does hammer-breakability key off `Map_Removable_Tiles`? No.** Vanilla's
  hammer tests `$51`/`$52` by range (`SUB #TILE_ROCKBREAKH / CMP #$02`), and the
  randomizer's `hammer_breaks_tiles` builds its own explicit table. **The wand
  gate is hammer-proof by construction**, flag on or off — it does not depend on
  a flag being off.
- **The `0x19DD0` gap in PRG012 — unreferenced check PASSED.** `prg012.asm` ends
  with "Rest of ROM bank was empty" after the unlabelled block at
  `$BC4A-$BDBF`; the run is `$FF` with no exceptions; an operand scan across
  PRG010/011/012/030/031 for absolute references into `$BDC0-$BFFF` found 83
  hits, every one a misaligned read inside a data table. The preceding 374 bytes
  at `$BC4A-$BDBF` are unclaimed but are NOT filler — they need their own check.
  Note the gap's head is **not** free in a randomized ROM: `FS_SEED_STAMP` (the
  flag key + seed stamp) sits at `0x19DF0` and had no registry row until now.
- **Does a secret-exit fortress clear reach `PRG011_BA7C`? Yes** — and that
  makes foreign locks behave *better* than same-world ones. On return with
  `Map_ReturnStatus == 0` the path is `PRG030_90C4` → `MO_Wait14Ticks` →
  `MO_DoLevelClear` → `Map_MarkLevelComplete`, and the fortress branch tests
  which map tile was beaten, not how the level ended. `Map_DoFortressFX` is set
  by Boom-Boom's defeat ball in PRG003, which is precisely what a secret exit
  skips. **So a foreign lock opens on a secret exit where an FX lock does not**,
  and `ensure_secret_exit_safe`'s reasoning does not apply to foreign locks.
  Read from the disassembly, not playtested.
- **The skull: there is none, and the attempt to draw one shipped a bug.**
  This is worth reading before anyone tries again.

  Every drawn pattern in map BG CHR pages `$14`-`$17` is terrain, panel art,
  masonry, the alphabet, digits or border fill, and no combination assembles a
  skull. So an attempt was made to *free* four tiles for one — `$80`-`$83` —
  on a three-legged argument: no metatile quadrant names them (all 1024 entries
  scanned), no `.byte` nametable stream on the map screen writes them (the three
  hits in the whole disassembly are the title logo and two level-font videos,
  all running a different pattern bank), and pages `$14`-`$17` belong to the map
  alone with the animation rotating `$00`-`$7F` only.

  **Every leg was true. The conclusion was wrong.** `$80`-`$83` are the four
  corners of the map's window boxes, drawn by a routine that *computes* the
  corner index — which no scan over declarative data can see. The first
  playtest came back with the corner deco shredded. Render the four tiles and
  it is obvious: `$80` turns right-and-down, `$81` left-and-down, `$82` and
  `$83` are the bottom pair.

  **The transferable lesson: a scan over declarative data cannot prove a tile
  unused, because code can compute a tile index.** Proving one free needs an
  emulator trace of what the map screen actually writes to the nametable, not a
  grep. The same caution applies to the "41 drawn but metatile-unreferenced
  tiles" figure recorded elsewhere — metatile-unreferenced is not unreferenced.

  The gate therefore wears `$D5`'s own vanilla art, the ornamental block, which
  reads as visibly *not* Dark Land wall. `wand_gate` now writes **no CHR at
  all**, and two tests hold that line: `the_box_corners_are_not_free_chr` pins
  the four corner patterns, and `the_gate_writes_no_chr` asserts the whole 128KB
  CHR region comes out untouched.

### Not every fortress can open a cross-world lock

> **SUPERSEDED by the fortress-FX rework (2026-09-06).** The constraint this
> section describes was a property of the old hook, and the code it names is
> gone — there is no `GlobalState::crumbling`, no `crumbling_forts`, no
> `FillReport::rejected_uncrumbling`, and no
> `every_cross_world_lock_names_a_crumbling_fortress` test. **A cross-world lock
> can be keyed to a tank now**, which the old hook forbade. Kept as the record
> of why the rework was worth doing.

A foreign lock fires from `Map_MarkLevelComplete`'s fortress branch, gated on
the tile under the player being `TILE_FORTRUBBLE` or `TILE_ALTRUBBLE`. Only
three tiles ever produce those — `Map_CompleteTile` indices 8 and 9, reached
from `TILE_FORT`, `TILE_LARGEFORT` and `TILE_ALTFORT`. Everything else completes
to a Mario/Luigi panel.

**World 8's tanks and battleships are the exception that matters.** They are map
object *sprites* floating over a cell the overworld writer deliberately blanks
to a path node — `write_tile_grid`'s sprite-mask pass, whose comment says so:
"W8 army sprites float on top of the grid; the underlying tile must be a plain
path node". Three of W8's four fortresses are like this in every seed.

A **same-world** lock does not care: it opens through `MO_DoFortressFX`, keyed
on an FX slot rather than on a tile. A **cross-world** lock keyed on one is
dead, and silently — the player beats the fortress and a lock in another world
simply never opens. Nothing in the game says why.

So `maze::crumbling_forts` reads the finished map (it must run *after*
`write_overworld`, or every fortress reads as blank) and `fill::assign_keys`
refuses any swap that would make such a fortress open a foreign lock. Rejecting
at assignment time rather than filtering the table later is the point: a lock
dropped from the table is a lock that never opens at all.

`every_cross_world_lock_names_a_crumbling_fortress` decodes the table out of a
finished ROM and checks each row's cell, because the generator cannot see map
tiles and the map tiles cannot see the assignment. It asserts sprite fortresses
are present too, so it cannot pass by the hazard disappearing; disabling the
gate fails it immediately.

## The state that must NOT persist

Two things looked like the Hammer Bro bug and are not. Both were investigated
after the first playtest and deliberately left alone; the first would have
broken the game.

### CORRECTION (2026-09-07): the HELP bubble is not load-bearing in a shipped ROM

The section below is true of **vanilla** and false of what we ship.
`autoscroll::disable_autoscroll` repoints every world's airship entry away from
the shared Toad-and-King object stream (`$D2AF`) to its own reworked level —
verified on a generated ROM, W1 `AF D2` -> `57 D7`. So the cutscene never loads,
`TAndK_WaitPlayerButtonA` never runs, the slot-0 token is never read, and nothing
writes the airship into slot 1. The spine edge works because the dock *is* the
airship level, not because the bubble came back.

`the_spine_edge_needs_the_help_bubble_back` asserted this only against vanilla
and so passed without ever looking at our output; it is now
`the_spine_edge_no_longer_needs_the_help_bubble` and checks both halves.

**The consequence is a budget.** Map-object slots 0 and 1 are free in a normal
seed — two per world — which is what makes a hovering marker over each lock
affordable. Measured (`randomizer::tests::map_object_slot_budget`, 30 seeds):
free slots go from ~2.8/0.0 to ~4.8/1.0 (W1-7/W8). It holds while autoscroll
removal is on, which is the default; `--keep-autoscroll` restores the vanilla
dependency, so any slot policy must key off that flag rather than assume.

### The king rescue, and why the HELP bubble has to come back

`TILE_AIRSHIP $C9` **does not enter the airship.** All seven `AIRSHIP_ENTRIES`
share one object stream, `$D2AF`, whose entire content is a single
`OBJ_TOADANDKING`. What happens next is decided by
`TAndK_WaitPlayerButtonA` (PRG024 `$A260`), and it branches on **slot 0**:

```text
LDA Map_Objects_IDs        ; the HELP bubble
BEQ standard_exit          ; gone -> dialog, back to the map
LDA #$03 / STA Level_JctCtl ; present -> "switch to airship"  <-- the spine edge
LDA #MAPOBJ_EMPTY  / STA Map_Objects_IDs
LDA #MAPOBJ_AIRSHIP / STA Map_Objects_IDs+1
```

So the HELP bubble **is** the switch that makes the dock tile the spine edge.
Persisting the rescue — keeping slot 0 clear across visits, which is what the
"the bubble came back, that looks wrong" complaint asks for — would make every
world's airship dock a **permanent dead end**. The airship would then be
reachable only by walking onto the marching object in slot 1, whose route comes
from `Map_Airship_Dest_YSets`/`XSets` in PRG011: **vanilla coordinates, which
nothing in the randomizer rewrites, on a map the randomizer redrew.** And
`maze/walk.rs` models the spine as an edge leaving `TILE_AIRSHIP`; it knows
nothing about a marching object. That is a solvability regression bought with a
cosmetic complaint.

`Map_Init` restoring the bubble is therefore **load-bearing**: it is what makes
the spine edge repeatable across visits, which is the property this document's
monotone-reachability argument rests on. The bubble is not claiming the king is
unrescued; it is saying "this world's airship edge is live", which is true, and
true of every world always.

`the_spine_edge_needs_the_help_bubble_back` pins it, and
`the_restore_can_only_ever_clear_an_id` decodes `RESTORE_OBJECTS` to prove the
module is *incapable* of conjuring or suppressing an airship.

**A pre-existing sharp edge found on the way**, which the maze mitigates rather
than causes: within one visit, dying on the airship leaves slot 0 already clear,
so re-entering `$C9` walks you straight back out and the vanilla-coordinate
marching airship is the only way in. Leaving the world and returning resets it —
which standard mode cannot do and the maze can.

### The four per-world flags — each declined on its own evidence

All four are cleared *before* the `$84CD` restore hook (`Map_WhiteHouse` and
`Map_CoinShip` inside `Map_Init` at `$84AD`; `Map_Got13Warp` and `Map_Anchored`
inline at `$84BB`/`$84BE`), so a restore there would work and a pack there could
not — the same shape as the objects. The machinery was not the obstacle; none of
them is worth the bytes:

- **`Map_Got13Warp` is not map state.** Its one reader is `ObjInit_WarpHide`
  (PRG001), which suppresses the hidden toad house **inside level 1-3**. Every
  other in-level item respawns on a replay, which this mode does constantly;
  persisting this one would make a single hidden door uniquely non-respawning.
- **`Map_Anchored` qualifies an object that does not exist.** It freezes the
  marching airship, and slot 1 is `MAPOBJ_EMPTY` on every world entry.
- **`Map_CoinShip` is already capped by the object fix.** `MapBonusChk_CoinShip`
  converts a slot holding `MAPOBJ_HAMMERBRO`; once a world's bros stay beaten,
  the scan finds nothing and no further coin ship can appear there.
- **`Map_WhiteHouse` is a slow faucet on an axis this mode does not ration.**
  Re-earning it needs a `Map_BonusType` arm, the coin count, and a world hop,
  for one item any toad house also gives — against ~30 bytes of PRG011 in a mode
  that already forces No Game Over Penalty and unlimited whistles.

## Measured 2026-09-06: the key-assignment fill rests on a false premise

Interrogating the cross-world lock system turned up one wrong fact holding up
one algorithm. Recorded here because the redesign it implies is larger than a
fix.

**`maze::fill` justified the swap search with a stall that cannot happen.** Its
header said the charter's constructive forward fill "needs a fortress inside the
start region with every lock closed — and the per-world builder deliberately
puts forts *off* the forced path, so the start region frequently holds none".

A lock is opened by beating its fortress, and reaching that fortress cannot
require opening the lock it opens, so the chain must bottom out at a fortress
reachable with everything shut. Measured: **480 of 480 worlds over 60 seeds**
have one, up to four. Off the *forced path* is not the same as behind a lock,
and the argument slid between them.

**And the fill does not stall under a policy that looks one step ahead:**

| | first-frontier | territory-ordered |
|---|---|---|
| stalled | 17/60 (28%) | **0/60** |
| mean keys to choose from | 4.17 | **6.04** |
| a cross-world key was available | 74% | **87%** |

**What the mistake costs is intent.** A swap search can only permute what it
inherits, so it has nothing to aim with — `Knobs::fort_distance_bias` defaults
to `0.0`, which the code calls a uniform random walk. A cross-world key is
available at 87% of a constructive fill's steps and nothing asks for one, so a
foreign lock is emergent rather than designed. Nothing measures whether one
actually **forces a crossing**, either: `FillReport` counts `foreign_locks` and
`foreign_span` (how far the key is), never "the player had to go there". A lock
whose fortress sits in a world the spine visits anyway gates nothing and is
counted as a success.

The formula the mode is after is the W8 multi-page shape one scale up: *a lock
in one world, no fortress reachable to open it, a telepad to another world, beat
a fortress there, come back.* A constructive fill produces that directly; a
random walk produces it by luck.

**What the swap search does buy is worth keeping**: it cannot fail, because it
starts from the per-world builder's assignment, which is known good. Any
replacement can keep that as a fallback.

### The deeper shape: the maze is a post-pass, not a build phase

Four of the five defects found on 2026-09-06 share one root — the maze mutates a
finished map rather than participating in building it:

| defect | root |
|---|---|
| `maze::writer` emitted only `is_foreign()` locks, so **33.1% of same-world locks were opened by the wrong fortress** (59/60 seeds) | two producers' views of one assignment, spliced |
| pad tiles and the wand gate written over grid cells the writer had committed | decide-after-write |
| `secret_exit_safe` is a per-world verdict computed before telepads exist | safety computed before the connectivity that decides it |
| the swap search itself | can only permute an assignment it inherited |

The proposed direction is to build all eight worlds as one problem — internal
connectivity (pipes) and external connectivity (pads) placed together, then
content, then locks against the graph that results — the way the standard-mode
builder already treats a single world. Pads become a connectivity phase instead
of a post-hoc addition, and a lock can be placed *because* a pad puts its key in
another world.

**Today's fortress-FX rework is what makes that possible.** The old per-world
caps (4 locks, 17 global slots) came from the Boom-Boom ordinal nibble; with
position keying a world can hold more locks than it has fortresses and take its
keys from anywhere. See `docs/fx_table_redesign.md`.

### 1-F, and what "safe" has to mean

1-F's secret exit hands out an item and skips the crystal ball: the fortress is
beaten and the lock stays shut. **That is a choice the mode keeps** — sometimes
the lock is worth more than the item — so the requirement is that it can never
end the run, not that it cannot happen.

`assign.rs` satisfies that in standard mode by parking 1-F on a lock the builder
marked `secret_exit_safe`, now pinned by
`one_f_lands_on_a_lock_that_can_stay_shut` (40/40). In maze mode the fill
permutes the pairing and the flag is a *per-world* verdict, so the question has
to be re-asked of the whole graph.

The right predicate is **weaker than `Spheres::solvable`**: castle reachable and
at least K airship docks reachable, with the lock held shut for the whole
fixpoint. `solvable` additionally demands every fortress be beatable, but a
fortress stranded behind a lock the player *chose* not to open is fine — they can
go back. Using the strict test would reject nearly everything.

A worry that did **not** survive measurement: a required telepad landing behind
the sealed lock. Pads are emitted as two directed halves, so a pad behind a
sealed lock still works as an entrance and extra connectivity only adds
reachability. **0 over-promises at every K.**

What does move is the size of the sealable pool, because the airship clause is
K-sensitive:

| K | locks sealable, of 680 |
|---|---|
| 0 | 554 (81%) |
| 3 (default) | 551 (81%) |
| 7 | **389 (57%)** |

So the check has to be made at the shipping K, and the risk concentrates at high
wand counts — which is also the hardest setting. Nothing asks today.

## Still open

- ~~**Per-world flags `$84A0` resets**~~ — **investigated and declined**, all
  four, see "The state that must NOT persist" below.
- **Whether K should scale with `world_count`** rather than being flat. It is
  currently clamped to the airships the spine offers, which is the safe half of
  the answer, not the interesting one.
- **The water gap as a real key** (repurposing the anchor into a boat snap).
  Parked: canoe edges gate on dock walk-reachability in the walker and that is
  load-bearing, so a portable boat changes walker semantics rather than adding
  an item.
- **Same-world locks through the foreign-lock table.** It would free FX slots at
  the cost of the crumble animation, but the 4-byte rows do not fit a store that
  would then need both the packed `(byte, mask)` and the live
  `(column, row bit)`. A bigger allocation and a change of ownership in the
  FX-assignment code.
- **Hardware playtest of the assembled mode.** Every piece is executed on an
  emulated 2A03 and the ROM is structurally verified, but the whole thing has
  not been played.

## Testing

Playtest ROMs must be built with `--no-walk --keep-locks --keep-gaps`, or the
engine records no completions at all and the test is vacuous.
