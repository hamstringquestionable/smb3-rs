# Mission-First Overworld Builder — Charter

Status: **draft for review.** This is the plain-English contract for the feature.
Every future slice is measured against it. No Rust here on purpose.

## What it is

A **drop-in replacement** for `overworld_build`. It takes the cleared, empty map
from the pickup phase and produces a **complete** overworld — every piece placed
— handed to the existing writer unchanged.

- **In:** the pickup phase's cleared per-world grids, the catalog, the seed's RNG,
  the run's flags, the per-world **pipe count**, and the world's **rock info**
  (which rocks were removed / optionally added).
- **Out:** the same `BuildResult` the writer already consumes. Nothing downstream
  changes.

If you deleted the old builder and dropped this in, the ROM would still come out —
just built mission-first.

## The four values (these win ties)

1. **Drop-in.** Same input, same output, same seams. The writer, the ROM format,
   and everything after are untouched. We never change the boundary.

2. **Mission-first, everywhere.** Every piece is placed to serve a per-world
   *mission* — the intended progression. No blind heuristic that ignores the
   progression. Pipes, forts, locks, levels — all of it answers to the mission.

3. **Grug-brain.** The simplest code that does the job. Concrete over generic,
   explicit over clever, readable and maintainable by one person with no AI. We
   add complexity ONLY when a metric proves we need it — never on spec.

4. **Proven with metrics.** Every step ships with a scoreboard number, the way
   fort realization went from 34–49% to 100%. We measure; we don't guess. A step
   isn't "done" until a metric says it is.

5. **Clear knobs.** Tuning is done on **what to build**, in plain units you can
   read and predict — counts, on/off, tile distances. NOT opaque scoring weights,
   softmax temperatures, or percentages tuned to hit some other percentage. If you
   can't look at a knob and say what the map will do, it's the wrong knob.

   *This is the payoff of mission-first:* the mission is a discrete decision ("a
   3-fort chain, 1 optional"), so you tune the decision, not the physics.

## What "complete" means — everything a build must produce

The current builder produces all of this; the replacement must too. For each,
"driven" = decided by the mission, "filler" = mechanical placement into whatever
slots are left.

| Piece | Role |
|---|---|
| Pipes | driven: a **fixed count per world** (a given), each pipe assigned a role — connectivity / shortcut / alternative / scout / dead. See the Pipes section. |
| Fortresses + locks | driven: realize the mission's progression. See the Fortresses & locks section. |
| Levels | driven: **fully mission-aligned** (pathing / pacing / agency). See the Levels section. |
| Hammer Bros | filler |
| Toad Houses | filler, with a light purpose: they can give players needed items if they are struggling, try to keep them even across worlds, fill space so the map doesn't look empty |
| Spades / bonus games | filler, same light purpose |

Nothing gets forgotten: the charter lists them so each has an owner.

## Even distribution — the unifying placement structure

Once the mission *shape* is placed (the forts, locks, and pipes the progression
needs), everything else — levels and filler — is spread as **evenly as possible**
across the map. This is a first-class positive rule, not a deferred nicety: a good
map has content spread across it, not clustered. The exact mechanism doesn't
matter much once the shape is fixed; "as even as we can" is the goal. Filler
(toad houses, spades) exists partly to fill gaps this leaves, so no corner of the
map reads as empty.

## The world plan — the real "mission"

A world's mission is bigger than "a chain of forts." It's the world's whole
**agency structure**: the mandatory level spine, where the shortcuts are and what
kind, where the level-vs-level choices are, and which forts are detours off the
path. Levels, pipes, rocks, and locks are all placed **together** to realize that
structure — the point of the whole thing is to hand the player *decisions*.

This is the ambitious part, and it will be **built in iterations** — a simple
version first (mandatory spine + one shortcut type), proven with metrics, then
richer. We don't build the whole agency engine in one go.

Note: `Mission` (the fort-role list we already have) is one *ingredient* of the
world plan, not the whole thing.

## Fortresses & locks

Forts are the part we understand best — the embed step that realizes them is built
and proven. This is the fullest section on purpose.

### The mechanic

A fort is a **key**; its lock is a **door**. Beating the fort opens its lock (a
gap or bridge tile becomes passable). This is the world's **hard** gating —
contrast levels, which are *soft* gating (just clear them). A lock only ever opens
by beating its fort.

### The placement rule

A fort on the mandatory *walking* path is a wasted lock: the player walks onto it,
plays it because it's in the way, clears it, and the lock opens without tension or
agency — it's just a level. So the rule:

> A fort is **physically optional** (off the mandatory walking path, a detour you
> choose to visit) but its **lock is not** — the lock gates real progress or a
> shortcut, so beating the fort still matters. You go *find* the fort to get the
> key.

This is the exact thing we measured as broken at the very start (forts stuck on
the trunk, ~53%, feeling pointless). *Never say never* — the builder may break
this occasionally for variety — but "walking-optional, lock-mandatory" is the
default the embed step aims for.

### What a lock gates

A fort's lock, once opened, can gate one of:

- **the next fort** — a chain link (beat this one to reach the next),
- **the goal** — the final gate onto the airship / Bowser,
- **a shortcut or nothing important** — a decoy / optional fort whose lock only
  opens a bypass or side content.

### The shapes (each is a different player experience)

- **Single gate.** One fort's key opens the goal; the others are optional decoys.
  "Which fort actually matters?"
- **Chain.** Beat a fort → its lock opens the way to the next fort → … → the last
  opens the goal. A required sequence.
- **Fork.** Several forts look equally plausible; one really opens the way forward,
  the rest are decoys — "pick the right fort." Two rules make a fork *work*: the
  branches must be **level-balanced** (an extra level on one branch and everyone
  takes the other — see Levels), and the real fort must have **no tell** (it must
  not be predictable — e.g. never always the farthest one, or "always pick the far
  fort" beats the guess).

### Count & optional forts

Fort count is a **given** per world (like pipes and rocks) or distributed — a knob,
not an emergent outcome. Some forts are **optional**: an optional fort simply has a
**shortcut around its lock** (a pipe/rock/lock bypass), so a player who finds the
bypass can skip it. "Optional fort" is not a new mechanism — it's a normal fort
plus a shortcut.

### Realization (why this is the proven part)

Because the embed step places forts *to realize the chosen shape* — rather than
scattering them and hoping a progression falls out — the intended shape forms
~**100%** of the time. That is the direct fix for the old builder, which realized
its intended chain links only **34–49%** of the time. Embed either finds a
placement that realizes the shape, or honestly reports the map can't host it (then
we pick a simpler shape) — never a silent, meaningless lock.

### Forts and route topology

Connectivity pipes create hub-vs-chain island layouts (see Pipes), and that
topology decides *what a lock can gate*: a fort guarding the one bridge onto a hub
island gates everything beyond it. So fort placement and pipe topology are
co-designed, not independent.

## Levels — the nuanced piece

Levels are the world's **pathing currency** and its **pacing**. They exist to give
the player decisions, not to fill space.

- **Steering.** Players take the route with fewer levels. So level placement
  faux-controls pathing: the intended way is lighter, heavier content sits where
  you'd rather players not go by default.
- **The mandatory spine.** Roughly **60% of a world's levels** sit on the must-play
  path — dialed *down* when the world has more required forts (levels + forts
  together are the "required effort" you're balancing). **At least one** level per
  world is truly mandatory (no shortcut around it).
- **Shortcuts are the joy.** A shortcut (pipe, hammerable rock, or lock/bridge)
  lets a player *skip* levels, and *finding* it is a core pleasure. Placement is
  **shape-driven** — levels go where they *make a planned shortcut work*:
  - *Pipe skip:* cluster 2 levels and leave a gap on the other side so a pipe
    hops them.
  - *Lock bypass:* stack 3 levels on the upper path and keep the lower path
    clear-ish, so a lock drops on the lower path — play the 3, or find the fort
    and open the lock.
- **Level-vs-level forks** are their own agency: choose level A or B; maybe A is a
  long death-trap and you learn to take B.
- **Fork balance.** For a real *fort* fork, the branches must cost about the same
  in levels — one extra blocking level and everyone takes the lighter branch, and
  the choice is gone.
- **Pacing.** At most ~4 levels back-to-back, and only rarely; usually a run is
  broken by a shortcut (`level → pipe skips 2 → level → level` beats three in a
  row).
- **Order** doesn't matter (a difficulty ramp is a later, flag-gated concern).

## Shortcuts — pipes, rocks, locks (given, not invented)

Shortcuts are how players earn agency, and they come from three sources that the
builder **receives as input** and works *around* — it does not invent them:

- **Pipes** — a fixed count per world (a given). Each is assigned a role
  (connectivity or shortcut).
- **Rocks** — handed to the builder: a few are removed to open maps up, and up to
  2 are optionally added as shortcut-gates. The builder gets this before it runs
  and keeps the alternate path around a rock clear-ish (or loads the *other* side
  with levels) so the shortcut is a real option.
- **Locks** — a fort's key opens a bridge/lock, which can itself be a shortcut
  around a run of levels.

Level placement and shortcut placement are one problem: you place levels *to make
the shortcuts meaningful*.

## Pipes — roles and philosophy

Pipe count is a **fixed per-world allotment, given to the builder** (W1 might get
0, W2 gets 1). It is *not* a builder decision — the builder gets the number and
the world shape and builds to it. (Counts may be randomized someday, but blind to
the builder.)

Pipes serve several roles, and one pipe can wear more than one hat:

- **Connectivity (primary).** Some maps literally can't be finished without a pipe
  — a region or the goal is unreachable on foot. This is the non-negotiable job.
- **Route-shaping.** Connectivity pipes decide the world's island topology — a
  **hub** island feeding several, or a **chain** of islands in a row — and that
  topology is a major lever on *how locks work* in the world. So connectivity and
  progression are linked, not separate.
- **Shortcut / alternative.** A **shortcut** is strictly better (skip a level, a
  cluster, or a fort) — once found, the player always takes it. An **alternative
  route** is a genuine either/or (the first path is long/hard, the pipe leads to a
  different set) — a real choice, not a freebie.
- **Scout.** A pipe that mostly reveals *information* — pops you to another part of
  the map so you can see a locked bridge / a fort and learn whether you must go
  back and beat something. Agency through knowledge.
- **Dead / loop.** Does little or nothing — loops you back near where you started.
  A *small* number of these are funny and fine. (Not "troll" — that name is taken.)
- **Intended route.** A pipe can *be* the expected way through an area, not an
  add-on — e.g. the pipe is how you reach the far side of a lock, and beating the
  fort changes that access. Pipes are part of the layout, not just bonuses.

### The philosophy (this shapes the whole approach)

- **Opportunities, not guarantees.** We don't hand-plan every shortcut. We leave
  the geometry *open* for shortcuts to occur, and let randomization produce them.
  A skip that shows up identically every world isn't fun.
- **Discovery is the point.** A shortcut is best when you *can't see both ends on
  one screen*. A pipe that visibly skips two levels right there gets taken by
  everyone and is boring; a pipe you only realize skips content after exploring is
  a reward. Prefer endpoints that aren't visible together.
- **Geometry limits agency, and that's OK.** We aim to leave choices open; we don't
  force them where the map won't allow.

## The pipeline (ordered, mission-first)

Per world (exact ordering will be refined — see Open Qs — since levels and
shortcuts are one coupled problem):

1. **Decide the world plan.** Shape (chain / fork / single-gate), fort count, which
   forts are optional, the mandatory-level target, and roughly where the shortcuts
   and choices go — given this world's pipe count and rock info.
2. **Find the islands.** Look at the cleared map's raw *walking* connectivity (no
   pipes yet) — what regions are cut off?
3. **Place the world's pipes (the given count).** First satisfy connectivity
   (reach islands / the goal), choosing island topology — hub vs chain — that
   **shapes the lock progression** and **preserves the chokepoints** the locks
   need. Spend any leftover pipes as shortcuts / alternatives / scouts, favoring
   endpoints you *can't see together* (discovery).
4. **Embed forts + locks.** Forts as walking-optional detours; locks gating real
   progress. (This part is built and proven at 100%.)
5. **Place levels, leaving room for shortcuts (coupled).** Lay the mandatory level
   spine (~60% target, ≥1 unskippable), keeping fork branches level-balanced, and
   leave geometry *open* so shortcuts (pipe / rock / lock) can occur — we create
   opportunities, we don't force every skip. Randomization fills them in.
6. **Filler, spread evenly.** Hammer bros, toad houses, spades into leftover slots
   via the even-distribution rule, filling empty corners.
7. **Assemble** the `BuildResult` for the writer.

We won't build all of this at once — start with steps 1–4 plus a minimal spine,
prove it, then add shortcut shapes and choices iteration by iteration.

## Metrics (how each step proves out)

Each step lands with a diagnostic, run on demand over many seeds:

- **Mission realization** — % of worlds where the sampled mission actually embeds
  (target ~100%). Already have it.
- **Connectivity** — % of worlds where the mission stays embeddable *after* pipe
  placement (proves step 3 doesn't over-connect).
- **Shortcut correctness** — a shortcut pipe bypasses exactly its optional fort and
  nothing else.
- **Completability** — every produced world is beatable start-to-goal.
- **Whatever the knobs claim** — if a knob says "3-fort chains," the metric confirms
  the maps have them.

## The knobs (plain units, contrasted with the old builder)

| We want (clear) | Not this (the old builder) |
|---|---|
| shape = chain / fork / single-gate | softmax_t = 4.0 |
| fort count = 3, optional forts = 1 | path_bonus = 0.75 |
| levels per world = 8 (vanilla-ish caps) | density_penalty = 3.0 |
| mandatory-level share ≈ 60% (less with more forts) | family weights = 55/35/10% |
| max levels in a row = 4 (rare) | dead_end_bonus = 5.0 |
| shortcuts per world = 2 | path_detour_cap = 6.0 |

You set what the world *is*. The builder either realizes it or reports it can't —
no fuzzy scoring in between. (A share like "≈60% mandatory" is fine here — it's a
*legible* target you can predict, unlike a weight tuned to hit some other number.)

## Non-goals / what stays mechanical (don't over-engineer)

- Hammer bros, toad houses, spades are **filler**. They carry no progression
  meaning; they drop into leftover slots (spread by the even-distribution rule
  above, and used to keep the map from looking empty). We do not invent mission
  roles for them.
- We do not chase aesthetic *perfection*. "Spread things evenly" is the whole
  aesthetic rule; we don't add fussier placement scoring unless a metric proves
  a real problem.

## Settled by the interviews (2026-07-25)

- Levels are **fully mission-aligned** (a pathing/pacing/agency tool), not filler.
- Forts are **walking-optional, lock-mandatory** detours (never say never).
- The real "mission" is the world's whole **agency structure**, built in iterations.
- Rocks and **pipe counts** are **given inputs**, not invented by the builder.
- Pipes: connectivity is primary and its **island topology shapes the locks**; other
  roles are shortcut / alternative / scout / dead. **Opportunities over guarantees** —
  leave geometry open, let randomization create discoverable shortcuts.
- Filler = hammer bros / toad houses / spades, spread evenly to fill space.

## Open questions (still to settle)

1. **Coupled levels + shortcuts (step 5).** Levels are placed *to make shortcuts
   work*, so they can't be a clean separate pass. What's the actual algorithm —
   place shortcut points first then levels around them, or co-solve? This plus…
2. **Minimal connectivity (step 3).** …placing "just enough" pipes while keeping
   chokepoints, are the two hardest new pieces.
3. **Expressing the plan in data.** How do "optional fort", "planned shortcut of
   size n", "level-fork here", "balanced branches" become concrete inputs the
   placement steps read? (This is the `Mission`-grows-into-a-world-plan work.)
4. **Fort counts.** A fixed knob per world, or distributed across worlds like today?
5. **First iteration scope.** What's the smallest end-to-end version (steps 1–4 +
   a minimal spine) that produces a complete, writable build to prove the seam?
6. **Route topology as a lock lever.** Connectivity pipes shape hub-vs-chain island
   layouts that change how locks work — how much does the plan *choose* this vs
   take whatever the given pipes/geometry produce?
