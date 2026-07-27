# Choice-First Overworld Builder — Charter

Status: **first constructive implementation landed (2026-07-27).** The builder
now measures route structure and shapes forts/locks/pipes against it — see
"Implementation status" at the end of this document for what exists, the
mechanisms discovered along the way, and the measured numbers. The rest of the
charter remains the plain-English contract the work is measured against. No
Rust here on purpose.

> **A note on the name.** This started as a "mission-first" builder, and the code
> still carries that name (the `Mission` struct, the `--mission-overworld` flag).
> That name describes a *mechanism* — placing pieces to realize a per-world
> progression — not the goal. The goal is the one thing below: **generate choices
> for the player.** A mission is one tool we use to produce them. Read "mission"
> throughout as an ingredient, never the north star.

## What it is

A **drop-in replacement** for `overworld_build`. It takes the cleared, empty map
from the pickup phase and produces a **complete** overworld — every piece placed
— handed to the existing writer unchanged.

- **In:** the pickup phase's cleared per-world grids, the catalog, the seed's RNG,
  the run's flags, the per-world **pipe count**, and the world's **rock info**
  (which rocks were removed / optionally added).
- **Out:** the same `BuildResult` the writer already consumes. Nothing downstream
  changes.

If you deleted the old builder and dropped this in, the ROM would still come out.

## The point (the north star above everything else)

**The builder's job is to hand the player decisions.** A good overworld is a
sequence of route choices; a bad one is a hallway you walk down. Everything else
in this charter exists to produce those choices. The decisions look like:

- Is it worth spending a **hammer** (a limited resource) to skip these levels?
- Is it worth playing an **extra level** to reach that pipe?
- Do I clear this **fort now** while I'm here, or skip it and risk backtracking?
- Which of these **forts** actually opens the way forward?

Those tradeoffs *are* the game. Archetypes, locks, level steering, shortcuts,
missions — all of it is **scaffolding that exists to generate those decisions.**
The archetypes (chain / fork / single-gate) and the per-world *mission* are ways
to *codify and produce* progression decisions; they are a means, never the end.

The consequences of taking this seriously:

- **A shape can be perfectly realized and still fail.** "Beat the one fort and
  the whole world opens" is a valid SingleGate that asks the player *nothing* — a
  100% on the realization scoreboard, a 0 on the real goal. When a build produces
  no decision, it has failed, whatever the scoreboard says.
- **Archetypes and missions may be bent or broken** whenever doing so buys a
  better decision. They are the tool, not the contract.
- **The metric that matters is decision density and the realness of the
  tradeoffs** — is the shortcut genuinely tempting *and* genuinely costly? — not
  "did we realize the sampled shape." Realization % is a means-check; it must not
  be mistaken for the goal-check.

Read the rest of this document through this lens: any rule that produces
structure without producing a decision is serving the scaffolding, not the goal.

## The four values (these win ties)

These are operational values — *how* we build. They serve the north star above;
when one of them conflicts with producing a real decision, the decision wins.

1. **Drop-in.** Same input, same output, same seams. The writer, the ROM format,
   and everything after are untouched. We never change the boundary.

2. **Grug-brain.** The simplest code that does the job. Concrete over generic,
   explicit over clever, readable and maintainable by one person with no AI. We
   add complexity ONLY when a metric proves we need it — never on spec.

3. **Proven with metrics.** Every step ships with a scoreboard number, the way
   fort realization went from 34–49% to 100%. We measure; we don't guess. A step
   isn't "done" until a metric says it is — and the metric that counts is whether
   the map produces decisions, not just whether a shape embedded.

4. **Clear knobs.** Tuning is done on **what to build**, in plain units you can
   read and predict — counts, on/off, tile distances. NOT opaque scoring weights,
   softmax temperatures, or percentages tuned to hit some other percentage. If you
   can't look at a knob and say what the map will do, it's the wrong knob.

   *This is the payoff of building to a plan:* a decision is discrete ("a 3-fort
   chain, 1 optional"), so you tune the decision, not the physics.

## What "complete" means — everything a build must produce

The current builder produces all of this; the replacement must too. For each,
"driven" = placed to create a decision, "filler" = mechanical placement into
whatever slots are left.

| Piece | Role |
|---|---|
| Pipes | driven: a **fixed count per world** (a given), each pipe assigned a role — connectivity / shortcut / alternative / scout / dead. See the Pipes section. |
| Fortresses + locks | driven: create the world's routing decisions. See the Fortresses & locks section. |
| Levels | driven: **the pathing/pacing/agency currency** (steer routes, price shortcuts). See the Levels section. |
| Hammer Bros | filler |
| Toad Houses | filler, with a light purpose: they can give players needed items if they are struggling, try to keep them even across worlds, fill space so the map doesn't look empty |
| Spades / bonus games | filler, same light purpose |

Nothing gets forgotten: the charter lists them so each has an owner.

## Even distribution — the unifying placement structure

Once the decision-bearing pieces are placed (the forts, locks, and pipes the
progression needs), everything else — levels and filler — is spread as **evenly
as possible** across the map. This is a first-class positive rule, not a deferred
nicety: a good map has content spread across it, not clustered. The exact
mechanism doesn't matter much once the shape is fixed; "as even as we can" is the
goal. Filler (toad houses, spades) exists partly to fill gaps this leaves, so no
corner of the map reads as empty.

## The world plan — the decision structure of a world

A world's plan is bigger than "a chain of forts." It's the world's whole **agency
structure**: the mandatory level spine, where the shortcuts are and what kind,
where the level-vs-level choices are, and which forts are detours off the path.
Levels, pipes, rocks, and locks are all placed **together** to build that
structure — the point of the whole thing is to hand the player *decisions*.

This is the ambitious part, and it will be **built in iterations** — a simple
version first (mandatory spine + one shortcut type), proven with metrics, then
richer. We don't build the whole agency engine in one go.

Note: `Mission` (the fort-role list we already have) is one *ingredient* of the
world plan, not the whole thing. It is the mechanism for the fort/lock decisions
specifically; the plan is everything that produces a choice.

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

### Lock quality — the decision the lock forces

A lock is a **routing-decision tool**. Gating progress is the mechanic; forcing
the player to make a *good decision about their route* is the point. So lock
quality is not measured by how much map it walls off — it's measured by the
**quality of the decision it creates.** (An earlier scoring pass optimized for
the smallest gated region; that was the wrong axis and is retired.)

**Low-quality locks — no decision:**

- **Fort on the mandatory walking path → the lock is redundant.** The player
  passes the key on the way through, so the lock adds nothing. The cardinal sin
  (and the sharper form of the placement rule above).
- **Obvious + sole + mandatory.** One clear fort, it's the only accessible thing
  to do, and opening it is the only way forward: "fort → lock → progress," no
  thought. Busywork. *Moving the fort off the walking path but leaving it the
  sole accessible option is still this* — off-path is necessary, not sufficient.
- **A goal gate that consistently hugs the goal (<~3 tiles).** Reads as
  artificial — tacked-on extra work, not part of the routing. Fine as an
  occasional variant, never the default.

**High-quality locks — a real decision:**

- **Gate a shortcut, not the sole path.** The lock sits on a *lighter* route
  (fewer levels — not necessarily the goal), so opening it is a weighed choice:
  go find the fort and open the shortcut, or just take the longer way. The
  player can guess wrong, and it feels good when they're right.
- **Ambiguity — the fork.** Several plausible forts; the player must work out
  *which* one opens the gate. This is how a **mandatory** goal gate earns agency:
  you have to open it, but *which fort is the key?* A single goal gate in a
  multi-fort world is exactly this and is a strong option.
- **Distance is a visibility knob, not a length target.** A fort far from its
  lock means the player can't see both at once, so they must *guess* what the
  fort does — mystery in the routing. Goal-gate forts further from the goal feel
  better for the same reason. "Far" is about hiding information, not maximizing
  span.
- **Backtrack tension.** A lock placed between a big shortcut (e.g. a pipe that
  skips a lot of the world to near the goal) and the goal: the player takes the
  shortcut, hits the lock, and may have to go *back* to find the fort. The
  shortcut isn't free — a good, self-inflicted decision.

**Goal gates specifically.** A goal gate is mandatory by definition — there's no
"open it or don't" agency — so its quality comes from **ambiguity** (which fort;
prefer the fork) and **mystery** (fort far from the goal, relationship hidden). A
plain, obvious goal gate isn't *bad*, but it's "extra work without having to
think" — acceptable in small doses, not the default the builder reaches for.

*These are legible and measurable (an alternate route exists past the lock;
fort and lock on different map screens; several candidate forts; the fort is off
the mandatory spine) — so this section becomes the embed's candidate-ordering
contract, replacing raw strand size.*

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

Realization is a *means-check*, not the goal. A shape embedding at 100% still has
to pass the real bar — did it create a decision? — from the north star above.

**Hard rules (settled 2026-07-25):** every budgeted fort is placed and every
fort gets a real lock — neither ever degrades. Only the *roles* degrade, down
a fixed ladder: sampled shape → single gate → chain → **loose single gate**
(the goal gate may strand Safe decoys — for cul-de-sac geometry where any
goal-gating lock strands most of the map, e.g. swapped-start W7) → all-Safe.
A map that can't host even all-Safe is a geometry-pipeline bug and fails
loudly, never silently.

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

### Topology rules (settled 2026-07-25, built in iteration 3)

Connectivity pipes construct the world's island topology; the builder owns its
own pipe pass. Three rules, all hard *preferences* with explicit fallbacks
(completability always outranks topology quality):

- **Choked entrances.** A pipe is untraversable if you can't walk to its
  entrance, so an island whose source-side entrance sits behind a lockable
  tile is gateable with one lock. Chain missions get chained, individually
  gateable islands; a fork's terminal group gets a one-lock island home.
- **Gate preservation.** New islands attach OUTSIDE the goal's best gate
  (the smallest strand containing the goal), so their fort slots don't get
  swallowed by the gate's strand and starve `GoalGate`'s feasibility.
- **No start→goal express.** The pipe that connects the goal's component
  avoids sourcing from the start island when the map allows depth.

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

## The pipeline (ordered)

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

Each step lands with a diagnostic, run on demand over many seeds. The scoreboard
numbers below are *means-checks* — necessary, not sufficient. The one that
actually gates a step is the last: does the map produce decisions?

- **Realization** — % of worlds where the sampled shape actually embeds
  (target ~100%). Already have it. A means-check.
- **Connectivity** — % of worlds where the shape stays embeddable *after* pipe
  placement (proves step 3 doesn't over-connect).
- **Shortcut correctness** — a shortcut pipe bypasses exactly its optional fort and
  nothing else.
- **Completability** — every produced world is beatable start-to-goal.
- **Decision density (the goal-check)** — how many *real* route decisions the map
  offers: locks that gate a genuine choice, shortcuts that are tempting and costly,
  forts with no tell. A map that scores 100% on realization but offers no decision
  is a *failure*, and this is the metric that says so.
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
  above, and used to keep the map from looking empty). We do not invent decision
  roles for them.
- We do not chase aesthetic *perfection*. "Spread things evenly" is the whole
  aesthetic rule; we don't add fussier placement scoring unless a metric proves
  a real problem.

## Settled by the interviews (2026-07-25)

- The builder's north star is **player decisions**; "mission" is a mechanism that
  serves it, not the goal.
- Levels are **a pathing/pacing/agency tool**, not filler.
- Forts are **walking-optional, lock-mandatory** detours (never say never).
- The real "world plan" is the world's whole **agency structure**, built in
  iterations.
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
7. **Measuring decision density.** The goal-check metric is still fuzzy. What's a
   concrete, seed-averaged score for "how many real decisions does this map offer"?
   Until we can measure it, the north star is a principle, not yet a scoreboard.

## Implementation status (2026-07-27)

The first constructive builder is live on `feature/choice-first`. Answering
open question 7 first turned out to unlock everything else: the metric is
`route_choice::analyze_route_choice` — a weighted set-cost Dijkstra (pipe 1 /
level 3 / fort 5 / rock 8, each clearable charged once) that enumerates every
distinct near-optimal route (identity = level-set), drops dominated superset
detours, and calls a world *choiceful* when ≥2 routes sit within 3 points.

**Pipeline** (per world): connectivity pipes → levels placed as the terrain
(`place_levels` — first half greedy, second half measured) → measured fort
shaping (`shape.rs`) → fort sections renumbered by BFS rank → locks → spare
pipes. The `WorldPlan` archetype /
`LockRole` layer is deleted — the measured route structure decides directly.
No rerolls: worlds whose terrain can't fork stay honestly linear.

**Mechanisms, in the order the censuses forced them into existence:**

1. *Fort re-pricing* (`shape.rs` phase A): a fort on the cheap route's
   exclusive stretch pulls a +4..8-gap parallel route into the band.
2. *Dominated detours are the raw material.* On these tree-ish maps almost
   every alternative is a superset detour, which the domination filter hides.
   The scorer now reports them (`RouteChoice::detours`) as rescue targets.
3. *Golden locks* (the workhorse): a nested detour's differentiator is usually
   a single path TILE, not a node — the shortcut edge. A lock there, gated by
   any fort, prices the shortcut at +5 vs the 3-6-point level loop: "beat the
   fort or take the long way around." These locks gate ~0 nodes, so they are
   derived from the route structure (exclusive walk-edge mids) and measured
   explicitly; lock selection is max (in-band route delta, then score).
4. *Gate-first, feasibility-anchored*: the goal-gate section is drawn
   uniformly from the sections that CAN gate (a severing tile keeping its own
   and all earlier forts reachable) and its lock is placed before any other —
   committed safe locks otherwise combine with the sever and kill every gate
   candidate. The secret-exit-safe retry carries the gate through instead of
   flattening it (this bug was producing goal-open worlds).
5. *Choice-aware spare pipes*: candidate pairs are measured for in-band route
   deltas — creators get a dominating bonus (and may ignore the skip cap),
   destroyers a soft veto that never blocks the vanilla pipe budget.
6. *Choice-aware second-half level placement*: the aesthetic `path_bonus`
   glues levels to the trunk, leaving every cycle's short side empty — a
   nested dominated detour. So only the FIRST half of each world's levels is
   placed greedy on the aesthetic score; each second-half level measures its
   candidates (top aesthetic scorers ∪ blanks on the cheap route's exclusive
   stretch vs each level-rescuable detour) and takes the max (in-band route
   count, then aesthetic score). ONE level on a node-bearing short side makes
   the level-sets disjoint and the detour a real fork — the level counterpart
   of golden locks, which keep the zero-node short sides. An A/B against a
   random first half measured 46% vs 50% linear for random, but its
   forced-level streak fell to 1.70 (off the ≈1.79 calibration — required
   routes thinned), so greedy stays the default; random survives as the
   `random_first_half` knob.
7. *C1 floor* (`C1_FLOOR = 14`): the choice metric bounds the GAP (C2−C1)
   but nothing bounded C1 itself — even gated, ~13% of worlds let the
   player finish for under 14 points (the goal gate proved choice-neutral
   AND floor-less in the `test_c1_floor_probe` A/B). `enforce_c1_floor`
   runs after locks: while C1 < 14, an off-route level moves onto the cheap
   route, measure-verified on the key (C1 capped at the floor, in-band
   count) — the cap stops floor-chasing from overshooting at choice's
   expense, and the tie-break means floor repair often CREATES ties. The
   spare-pipe pass gained a matching soft veto (a shortcut may not price
   the world below the floor; the vanilla pipe budget still outranks it).
   Sub-14 worlds: 13.4% → 1.2%. A Level↔HammerBro swap never changes
   walkability, so locks and the goal-open guarantee are untouched.

**Numbers** (1000 seeds, slack 3): 46% linear overall, mean 1.85 routes/world
— vs 50% before the C1 floor, 58% before the choice-aware level pass, 68%
for the pre-reroll builder, and ~20% for best-of-8 rerolling. W6 richest
(94% choiceful), W8 poorest (25% — the corridor); W1 lifted from 12% to 33%
choiceful by the level pass (its lone cycle now gets a level even when no
fort can fork it). Worlds under C1 14: 1.2% (was 13.4%), min 9, mean C1
19.0. Goal-open: 0/1000 SAS-off. Forced-level streak 1.91/1.81 (reference
≈1.79). ~0.32 s per full seed. Same-seed byte-identical.

**Honest gap vs the reroll era:** rerolls exploited raw geometry variance —
eight fresh level layouts per world — where the constructive builder gets one
deterministic layout given pipes (the measured second half claws back part of
that, and the `random_first_half` knob would buy more variance at a measured
cost: 46% vs 50% linear but streak 1.70, off calibration). The identified
next lever is *cycle dressing*: enumerate the walk graph's physical cycles
directly and dress each (levels on one side, golden lock on the other, loads
tuned to tie) instead of only rescuing the also-rans the route scorer happens
to surface. Level rebalancing (phase A0) exists but rarely fires — parallel
non-nested routes are rare without it.
