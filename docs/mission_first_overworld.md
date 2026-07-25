# Mission-First Overworld Generation — Design

Status: **proposal** (not yet built). Supersedes the geometry-first fort/lock
placement in `overworld_build`.

## 1. The problem

The current builder is **geometry-first**: it places forts by softmax scoring,
places levels by scoring, places locks by scoring, and then the *progression
shape is whatever falls out*. `WorldPlan` samples a desired archetype (Chain /
SingleGate / Fork) up front, but placement never enforces it — `place_locks`
merely *tries* to realize the sampled roles and falls back when the geometry
won't host them.

Measured consequence (1000 seeds): the sampled shape is a **wish**, not a
guarantee. ChainLink roles realized ~34% of the time; the other ~66% silently
degraded to a plain chokepoint or a Safe lock. Two independent knobs were tried
this session — removing the BFS distance-band "sectioning" and deriving roles
from real geometry — and neither closed the gap, because both are still
*inferring* a structure from geometry that was never built to host one.

The root issue is direction. **You cannot reliably read a progression out of a
map you generated without one in mind.**

## 2. The principle: separate mission from space

This is a solved shape in the PCG literature and in the item-randomizer
community:

- **Mission vs space** (Joris Dormans, *Cyclic Generation of Level
  Structures*): design the abstract *mission* — the lock-and-key dependency
  structure — as a first-class object you fully control, then *embed* it into
  geometry. Space serves the mission.
- **Logic graphs** (OoT/ALTTP/Metroid randomizers): model the world as a
  directed dependency graph (locations gated by requirements) and place keys so
  the graph is a valid, completable DAG with the desired depth/branching.
  Linearity becomes a *parameter you set*, not a statistic you measure after.

We invert the pipeline: **decide the mission, then place forts and locks to
realize it by construction.**

## 3. The mission model (this game's mechanics)

SMB3 fort/lock mechanic, as implemented today:

- Each fortress owns exactly **one lock** (a gap tile on a path). Beating the
  fort opens its lock (restores the path tile).
- Traversing a closed lock is impossible; so to cross lock `L` owned by fort
  `F`, you must have beaten `F`.

So a world's mission is a **lock-and-key dependency DAG** over the forts plus a
Goal node. Each fort gets one role (identical vocabulary to today's `LockRole`,
but now *designed*, not inferred):

- `ChainLink { target: j }` — fort `i`'s lock gates fort `j` (beat `i` to reach
  `j`).
- `GoalGate` — fort `i`'s lock gates the airship/Bowser target, stranding no
  fort.
- `Safe` — fort `i`'s lock gates nothing important (decoy / optional).

A **mission** is a per-fort role vector that forms a valid DAG:

- No self-gate (a fort's lock never strands itself → softlock).
- No cycle (fort A gates B, B gates A → deadlock).
- Completable: an order exists in which every required fort is reachable when
  needed, and the Goal is reachable after the required set is beaten.

Archetypes are just families of valid missions:

| Archetype | Mission (n forts) |
|---|---|
| SingleGate | one `GoalGate`, rest `Safe` |
| Chain | `0→1→…→(n-1)`, last is `GoalGate` |
| Fork{k} | length-`(n-k)` chain prefix, then a terminal group: one `GoalGate` + `k-1` `Safe` decoys reachable in parallel |

## 4. Realizability: what the space must provide

A mission is **embeddable** in a world iff there is an assignment of fort
positions and lock tiles such that, for the base grid (forts/levels stamped,
connectivity pipes only):

- **ChainLink i→j**: closing fort `i`'s lock strands fort `j`; fort `i` stays
  reachable; no fort required *before* `i` is stranded. (Mirrors `place_locks`
  hard-rule 1 + blocks-earlier.)
- **GoalGate i**: closing fort `i`'s lock strands the target while stranding no
  fort.
- **Safe i**: fort `i` is reachable; a lockable tile exists that strands nothing
  important (or the fort simply carries no impactful lock).
- **Global**: every fort reachable in some valid beat-order; target reachable
  after the required set.

The primitive for testing this is the **strand-set**: for a candidate lock tile
`T`, close it alone and record which forts/target `walk_map` can no longer
reach. (This is exactly the strand-set analysis prototyped this session —
reusable, but now evaluated *over candidate fort positions*, not fixed ones.)

## 5. Placement as constrained search

Because positions are variables, embedding is a **constraint-satisfaction
search**, but a small one — per world: forts ≤ ~4, candidate blank positions
≤ ~40, lockable tiles ≤ ~50. Backtracking is entirely tractable.

```
embed(mission, world):
    # order forts by dependency (topological): required-earliest first
    for each fort i in dependency order:
        for each candidate position p (aesthetically ranked: dead-ends first):
            for each lockable tile T reachable-consistent with p:
                if strand_set(T) satisfies role[i] given already-placed forts:
                    commit (i→p, i→T); recurse
        backtrack if no (p, T) works
    return full assignment, or FAIL
```

Aesthetic scoring (spread, dead-end preference) becomes the **candidate
ordering** inside the search, not an independent pass — so forts still look
well-placed, but every placement is one that *realizes a gate*. A fort ends up
on a branch with its lock on the trunk chokepoint it gates → the "key you
detour to find" dynamic, by construction.

## 6. Mission sampling (variety without wishes)

The map's fixed topology bounds which missions embed (a single-corridor world
cannot host a 3-way fork). So sampling is **feasibility-aware**:

1. Cheaply probe the world's branch structure (how many independent
   gate-able regions exist).
2. Sample an archetype from the families the world can actually embed,
   weighted for desired variety.
3. `embed()` the sampled mission. On FAIL (rare, if probing is sound), degrade
   deterministically to the richest embeddable sub-mission (e.g. drop a fork arm
   to a decoy, or shorten the chain).

Result: the realized mission **is** the intended mission, ~100% of the time, and
"why did it do that?" has an answer — *because that mission was sampled and
embedded.*

## 7. Strategy: a standalone builder, swapped in when proven

We do **not** retrofit this into the current builder. Trying to interleave
mission-first logic with the existing geometry-first passes is what produced two
competing role systems this session. Instead:

Build a self-contained module — `src/randomize/mission/` — that:

- has **no ROM writes** and **no dependency on `overworld_build`**;
- consumes a **read-only map view** (candidate fort slots, lockable tiles, a
  reachability query — satisfiable by the existing `Grid` + `walk_map`, and by
  hand-written synthetic maps for unit tests);
- produces an abstract **`Embedding`** (`{ fort_positions, lock_positions,
  mission }`) — a *decision*, not ROM bytes.

The current builder stays fully intact and live the entire time. The mission
builder is developed and measured **in parallel**, against real world grids
(read-only) and synthetic test maps. Nothing in the shipping path changes.

Only once it produces good embeddings at ~100% realization across all
archetypes do we write a thin **adapter** that turns an `Embedding` into the
inputs `overworld_writer` already expects, and switch the pipeline over in a
single deliberate swap. At that point the geometry-first fort/lock code and the
up-front role machinery (`from_archetype` roles, `fork_roles`, `shuffle_goal`,
`ensure_seed_safe_role`, per-band sectioning) are deleted in one subtraction.

What is reused rather than rebuilt: `walk_map` (read-only reachability),
`LockRole`'s vocabulary, and `progression.rs`'s Dijkstra — the latter becomes a
pure **oracle** the verifier and tests call to confirm a realized mission
matches intent.

## 8. First slice (small enough to hold in one head)

One question, answered end-to-end: *given a real cleared world map and its
candidate slots, can we sample a mission and embed it at ~100% realization?*

In scope:
- `mission/map.rs` — the read-only map view + `strand_set(T)`.
- `mission/mission.rs` — `Mission` (role vector) + `sample_mission` for
  **SingleGate** and **Chain** only.
- `mission/embed.rs` — the backtracking `embed(mission, map) -> Option<Embedding>`.
- `mission/verify.rs` — confirm an `Embedding` realizes its `Mission`.
- Tests: synthetic tiny maps (deterministic correctness) + a read-only
  diagnostic that runs over the 8 real world grids and reports realization,
  directly comparable to this session's 34%/49% geometry-first numbers.

Explicitly **out** of the first slice (added only after the core is proven):
Fork archetype; ROM writing / writer integration; pipes (connectivity + fort-
skip), levels, hammer bros, toad houses; canoes / SAS / W8 specials; the swap
itself.

Success = SingleGate and Chain realize at ~100% on real maps. That proves the
architecture on the same data geometry-first capped out on, with zero risk to
the shipping builder.

## 9. Measurement

The session's `report_fort_lock_metrics` scoreboard (saved in scratchpad) is the
acceptance gate:

- Role realization → ~100% by construction (any shortfall is an `embed` bug,
  not silent degradation).
- Add a **linearity distribution** readout (mean required-fort chain depth per
  archetype) so we can confirm we're producing the variety we sample.

## 10. Open questions

1. **Feasibility probing (§6.1)** — how cheaply can we bound embeddable
   archetypes without running full `embed()`? A coarse "count independent
   branch regions off the start→goal trunk" may suffice.
2. **Search cost** — is naive backtracking fine at these sizes, or do we want
   candidate pruning (e.g. only lock tiles on the start→fort frontier)?
3. **Levels vs mission interaction** — levels are placed after forts today.
   Does mission-first fort placement want to reserve chokepoint tiles from the
   level pass, or is the current ordering fine?
4. **Fort-skip pipes** — `place_spare_pipes` can currently make one mandatory
   fort optional. Under mission-first this is a *deliberate mission edit* (turn a
   ChainLink into an optional shortcut). Model it in the mission, or keep it as a
   post-pass?
