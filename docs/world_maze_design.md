# World Maze — design charter

*Branch `experiment/world-maze`, off `beta/next`. Supersedes the parked
`experiment/mega-map` fold.*

A mode in which the eight world maps stop being a sequence and become the rooms
of one Metroidvania. Every world keeps its own palette, music, king, tileset and
grid — so every "keyed to eight worlds" assumption in the codebase stays true —
and the worlds are linked by **telepads**, by the **airship/castle** one-way
edge, and by **cross-world locks**.

Phase 1 (persistence, arrivals, telepads) is complete and hardware-confirmed.
This document is the design record for phase 2, the generator. Its **step 1 —
the global verifier and the null-model census — is done**; see "The null-model
baselines" for the numbers everything after it is argued against.

**Where phase 1 lives:** `world_persist.rs` (the `$84A0` hooks, the arrival
tables, `PAD_ENTER` and its position key, the debug world jump) and
`completion_bits.rs` (the packed per-world store, the derived stencil, the pack
and expand routines). Both carry the mechanism detail in module rustdoc; this
document does not repeat it. The engine-level facts they rest on are in
`smb3_rom_reference.md` under "World transitions and per-world map state" and
"World-map graphics: CHR banks and unused metatiles". Playtest ROMs come from
`testrom`.

**Where step 2's verifier lives:** `src/randomize/maze/` — `GlobalState` and
the global fixpoint in `mod.rs`, the null model's uniform pad placer in
`pads.rs`, the acceptance tests and censuses in `tests.rs`. The cross-world
walker sits beside `walk_reachable` in `map_walker.rs`. The whole thing is
`#[cfg(test)]`: it is a measurement instrument with no production caller, and
the gate comes off when the generator wires in.

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
  they are. `world_count` gains meaning (maze size).

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

**The budget is 16, globally.** `PORTAL_MAX = 16` arrival rows, one per pad
*tile*, shared with (now unused) pipe portals. A pad is one-way by construction,
so a two-way link costs two ids. The cap is both the id encoding and the table
bytes (six arrival tables plus three key tables). Raising it is possible in
PRG011 at 9 bytes per row, but the design targets 16.

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

### Pad roles are the per-world interface

The per-world builder never learns that a world graph exists. It is handed a
count and a set of roles:

- **Hub pad** — in the start region, zero-key reachable. How a world satisfies
  invariant 3 when its airship path is gated.
- **Island pad** — deliberately in a region no walk reaches: an island, past a
  water gap, behind a lock. The pad-only fortress, and the shape the mode is
  for.
- **Gated pad** — reachable, but behind a lock. A shortcut that opens later.

All three are queries against machinery that exists: `islands.rs` knows island
regions, `forced_positions` knows cut vertices, `locks.rs` knows what is behind
a lock.

Two consequences:

- **Pads are placed before forts and locks** in the per-world pass. A pad
  arrival is what makes an island region eligible to hold content at all.
- **The role allocator needs a fallback.** Not every world's terrain has an
  island — W1 does not. So a role is a request, not a contract, and the
  distribution of granted roles is a census output.

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

**Fort-selection distance bias** is the main dial. When the fill assigns a fort
to a lock, the candidate set is every reachable fort in every world: nearest
gives local, vanilla-ish locks; a different world gives a maze; furthest gives
maximum backtracking. One number, spanning the whole range. Resist adding a
second until a census demands it.

**Key depth within a sphere** is the secondary dial: placing a key in the oldest
reachable region gives long backtracks, the newest gives a corridor.

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


### How the baselines get made

No knob below has a measured value, and none can until something exists to
measure. The way out is the one the current builder used: **measure a null
model first, then add one lever at a time.**

The null model is the crudest thing that is still a maze:

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

2. **Invariant 3 is a real placement obligation, not a cheap guard.** It was
   expected to hold nearly always ("every world has an airship and the builder
   already guarantees start → target"). It holds barely half the time, because
   the per-world guarantee is start → target *with locks openable*, and
   invariant 3 asks for it *with every lock closed*. That gap is the whole
   measurement. It needs a placement phase — either an ungated airship approach
   or a hub pad in the start region.

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

### Build order

1. ~~**`GlobalState`, a directed cross-world walker, and the global fixpoint —
   as a pure verifier.**~~ **DONE** (2026-09-05) — `src/randomize/maze/`, with
   the cross-world walker beside `walk_reachable` in `map_walker.rs`. The
   spoiler log is `Spheres::spoiler`. Shipped with the crude uniform pad placer
   in `maze/pads.rs`.
2. ~~**Census the null model.**~~ **DONE** — the table above.
3. **World-graph pass** — spine order, pad counts and roles per world, id
   budget allocation against the 16.
4. **Pad placement** in the per-world pass, by role.
5. **The key-assignment fill**, with the distance bias.

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

## Invariants

1. **Global completability.** The fixpoint reaches the castle and every fortress
   is beatable — no fort sealed behind its own lock, now across all worlds.
2. **Zero hammers.** Solvable with `has_hammer = false`.
3. **The start-region rule.** *Every world's start-tile region must contain at
   least one exit reachable with zero keys* — its airship, or a pad out.

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

### The two hooks, and the bug they had

| Hook | Where | Fires when |
|---|---|---|
| pack (`WIPE_REPLACEMENT`) | the displaced wipe, early in `$84A0` | `TRANSITION_FLAG` set |
| expand (`SWAP_AT_RELOAD`) | later in the same init | `World_Num != LIVE_WORLD` |

Those are two different definitions of "a transition happened", and they
disagree on exactly vanilla's own two world-change paths, neither of which
raises the flag.

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
  `PAD_ENTER` and `WORLD_JUMP_CHECK` hand back the bytes that raised it, and the
  airship path needs no PRG030 rent at all.

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

- **`$E8` is the telepad tile.** The patch explicitly clears completion for `$E8`
  cells on game over. Harmless today, because diverting at enter time means
  `MO_DoLevelClear` never runs for a pad and nothing ever marks one — but this is
  a live dependency between an imported IPS and the maze's tile choice.
- **The patch's fourth write is at file `0x3D314` = CPU `$9304`**, about ten
  bytes from `PRG030_9314`, the game-over `AND` of the two completion halves that
  the packed-store design rests on. Two patches now edit the same short stretch
  of game-over code; check a seeded `--write-log` with both enabled.

## Cross-world locks

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
it (390 bytes free, largest gap 277).

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

**Cost of the ninth removable entry.** The tables are packed and adjacent —
`Map_Removable_Tiles` (8) at `0x18447`, `Map_RemoveTo_Tiles` (8) at `0x1844F`,
`Map_Completable_Tiles` (5) at `0x18457` — and the loop bound is an `LDX #7`
immediate. Growing in place is impossible; relocate both tables to 9-byte copies
and repoint three absolute operands (`CMP Map_Removable_Tiles,X` and `LDA
Map_RemoveTo_Tiles,X` in PRG012, plus our own `IS_COMPLETABLE`, which reads the
same table) and two `LDX` immediates. PRG012 has 908 free bytes with a 576-byte
gap at `0x19DD0`, and PRG012 is mapped whenever this code runs. The gap still
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
| `wands_are_collectable` | at least K airships reachable without passing the goal gate — **exists** as `GlobalState::wands_are_collectable`, asserted at K=7 on a spine-only maze |
| `pad_ids_fit` | pad count <= `PORTAL_MAX`, and every pad owns a distinct arrival row — **exists** |
| `packed_planes_fit_their_reserve` | **exists today** — must still pass once the wand-gate byte joins `Map_Removable_Tiles` |
| `the_maze_leaves_every_pipe_alone` | **exists today** — pipe tables and world pointer tables stay byte-identical |

Census outputs, none of which have a measured baseline yet: sphere count,
sphere width distribution, sphere-0 openness, backtrack cost, granted-vs-
requested pad role distribution.

## Open questions

- **Per-world flags `$84A0` resets** — `Map_Anchored`, `Map_WhiteHouse`,
  `Map_CoinShip`, `Map_Got13Warp`. Vanilla never returns to a world so it does
  not matter; a maze returns constantly. These need per-world persistence
  alongside the completion pack.
- **Whether K should scale with `world_count`** rather than being a flat number.
- **The water gap as a real key** (repurposing the anchor into a boat snap).
  Parked: canoe edges currently gate on dock walk-reachability in the walker,
  and that is load-bearing, so a portable boat changes walker semantics rather
  than just adding an item.
- **Where the wand counter lives**, and the hook that increments it on airship
  clear. One SRAM byte; the SRAM run at `$7997` has ~9 bytes left after the
  packed store, and `$7A73-$7ADF` is largely unspent.
- **The whistle's picker.** The visited bitmask is one byte and the "touched the
  start tile" test is a position compare on the map idle frame, but the player
  still needs to choose a world. Candidate: reuse the warp zone, whose three
  pipes already perform world transitions and already set `World_Num`.
- **Does hammer-breakability key off `Map_Removable_Tiles` membership, or off a
  separate rock list?** Decides whether the cloned wand-gate byte is hammer-proof
  by construction or only by the flag being off.
- **The `0x19DD0` gap in PRG012 has not had the unreferenced check.** Required
  before the relocated removable tables claim it.
- **Does a secret-exit fortress clear reach `PRG011_BA7C`?** Decides whether
  foreign locks open on a secret exit.
- **Parked: hunting the unused skull graphic.** The wiki's "skull meant for the
  world map" is not in the map BG CHR (pages `$14`-`$17`); of the 41 drawn but
  metatile-unreferenced tiles there, all are the alphabet, digits or fragments.
  If it exists in the final ROM it is likely a *sprite* — map objects come from
  pages `$20`-`$23`, and the object ID list has two unused entries
  (`MAPOBJ_UNK08`, `MAPOBJ_UNK0C`). A sprite cannot be a wall, so this is
  decoration at best; the wand gate does not depend on it.

## Testing

Playtest ROMs must be built with `--no-walk --keep-locks --keep-gaps`, or the
engine records no completions at all and the test is vacuous.
