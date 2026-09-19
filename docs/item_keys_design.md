# Item Keys — gating power-ups on what the player has found

> **Status: design note. Nothing is implemented.** No branch, no flag, no code.
> This records a design conversation (2026-09-19) to the point where it could be
> built, and marks what is still undecided. Line numbers and offsets cited from
> Rust will drift; the disassembly citations will not.
>
> It supersedes the *approach* of the parked item-economy research
> ([artifact, 2026-09-12](https://claude.ai/code/artifact/d4bd2f24-1cfa-4e34-969e-04f365339c42),
> memory `item_economy_spheres`) while keeping its findings. That design emptied
> the levels — every power-up block became a coin, permanently, and the inventory
> became the only supply. This one leaves the levels alone and *schedules* them.

## The mode in one paragraph

A power-up block only dispenses an item the player has already **found** from an
overworld source (Toad House, treasure chest, the Peach letter). Finding one is
permanent and world-wide: find a leaf and every leaf block in the game starts
working. Until then that block pays a coin. Because "has found X" is a durable,
monotone fact, it can gate content — a level that genuinely needs flight, a canoe
that needs the anchor, a wall that needs fire. The run starts restricted and
converges back to vanilla as the player finds things.

The point is that **World Maze currently has exactly one key** — a fortress clear
— and `locks.rs` keeps fort↔lock a bijection, so progression can only ever be a
matching, never a hierarchy. Item keys break the bijection in both directions and
introduce conjunction, which the current model cannot express at all.

## Why this shape and not the parked one

The parked design funded an economy by removing power-ups from levels, which made
**coins gate access**. A currency that gates access is where stalls live, and for
the generator a spendable key breaks the fixpoint's monotonicity — the property
the parked design's own permanence argument existed to protect.

Here the in-level block *is* the supply, and it is free. Coins (if the economy
layer is ever built on top) buy carry-in convenience only, which means **the
generator never has to model the economy at all.** The two mechanisms also cover
each other: no unlock and broke → locked blocks pay coins toward it; unlocked and
broke → blocks work, play normally. No cell of that table is a dead end.

## The model: three layers

| Layer | Holds | Example |
|---|---|---|
| **Found-mask** | *items* the player has acquired | leaf, tanooki, mushroom, star |
| **Ability** | what a suit lets you *do* | flight, big, fire, invincibility |
| **Requirement** | what a gate demands | 6-5 requires leaf |

The ROM already ships part of the item→ability map. `prg000.asm:558`:

```
; Bit 0 (1) = Able to fly and flutter (Raccoon tail wagging)
; Bit 1 (2) = NOT able to slide on slopes
;     Small, Big, Fire, Leaf, Frog, Tanooki, Hammer
PowerUp_Ability:
	.byte $00,   $00, $00,  $01,  $02,  $01,     $02
```

Flight is satisfied by leaf **or** tanooki. So abilities are disjunctive.

**But a level requirement names an item, not an ability**, and this is the
subtle part. Logic assumes the player enters small and uses *the level's own
dispenser*, which is item-specific: 6-5's block is a leaf, so 6-5 requires
**leaf**. Having tanooki unlocked does not help you inside 6-5.

The split, therefore:

- **Level gates name items.** Conservative, source-specific.
- **Map gates may name abilities.** The player brings their own suit, so a flight
  wall can accept leaf or tanooki.

### Carrying a suit in to break logic is a feature

A player who walks into 6-5 already wearing a tanooki beats it without the leaf
unlock. Logic is the conservative floor — beatable entering small, using only
what is inside — and beating it is a sequence break the player earned.

**The generator must never try to prevent this.** Recorded here so nobody
"fixes" it later.

## The dispenser hook

### Vanilla already makes this decision

`LATP_JumpTable` (`prg008.asm:5137`) dispatches on a block's content byte. Every
handler's job is to return `Y`, an index into `Bouncer_PUp` (`prg001.asm:1015`:
`$00, $00, FIREFLOWER, SUPERLEAF, STARMAN, MUSHROOM, GROWINGVINE, 1UP`). And the
power-up handlers *already branch on player state* — `prg008.asm:5170`,
unmodified vanilla:

```
LATP_Leaf:
	LDA #$00
	STA PUp_StarManFlash
	LDY #$05	 ; Y = 5 (spawn a mushroom)
	LDA <Player_Suit
	BEQ PRG008_B807	 ; If Player is small, jump (keep mushroom)
	LDY #$03	 ; Y = 3 (spawn a leaf)
PRG008_B807:
	RTS
```

So a ? block's contents are already chosen at spawn time by a runtime test, which
is exactly the shape this feature needs. `LATP_Flower` (`:5155`) is identical with
`LDY #$02`.

And `LATP_CoinStar` (`:5216`) is the precedent for the degraded case — a
power-up block that becomes a **genuine** coin block based on runtime state,
for three bytes:

```
LATP_CoinStar:
	LDY #$04	 	; starman
	LDA Player_StarInv
	BNE PRG008_B83F	 ; already invincible → keep the star
	; Otherwise, sorry, just a coin :(
	JMP LATP_Coin
```

`LATP_Coin` runs `LATP_CoinCommon` and the 10-coin-block bookkeeping, so the
block really is a coin block for that bump.

### The gate: split on the branch that already exists

The handler has **two products**, and they must be gated **independently**:

```
LATP_Leaf:
	LDY #$05	      ; mushroom
	LDA <Player_Suit
	BEQ small
	;  --- big path ---
	leaf found?       → no: JMP LATP_Coin
	LDY #$03          ; leaf
	RTS
small:
	;  --- small path ---
	mushroom found?   → no: JMP LATP_Coin
	RTS               ; Y = 5, mushroom
```

**Why the split is load-bearing.** Degrading the whole handler to a coin also
removes its mushroom default, which silently gates *big* through whatever item
the randomizer happened to roll into that block. Concretely: leaf found, fire
flower not, the block is a fire flower, player is small → coin → cannot get big →
cannot break a brick → stuck, in a level nothing authored as a gate. Splitting on
the vanilla `BEQ` removes that entire class.

Splitting also makes the mushroom a **first-class key**. Locked, the player is
permanently small until they find one: one-hit death, no brick breaking. That is
the strongest gate in the vocabulary and the single biggest pacing lever in the
mode — and it is what makes 8-F a real gate and 7-F1 a genuine double gate.

**Why locking the mushroom cannot brick a level:** `vision.md:39` — every level
is beatable entering as small Mario. The recorded exceptions to that charter are
`powerups.rs`'s `PROTECTED_OFFSETS`, and those exceptions *are* the gates. The
exception list and the gate list are the same list.

Cost: roughly 12 bytes per handler for two tests, and vanilla's own `BEQ` does
half the work.

### Where it does not go

**Not `Player_QueueSuit`.** Every suit change funnels through one consumer
(`prg008.asm:820`, `AND #$0f / TAY / DEY / STY Player_Suit`), which makes it a
tempting single chokepoint. It is the wrong site: by the time it runs the item is
already on screen and in the player's hands, so the only achievable behaviour is
"you collected it and nothing happened", which reads as a bug.

**Not the level data.** A block's content byte is read-only at runtime while the
unlock is runtime state, so the substitution cannot be baked in at generation.

### Two dispensers with no mushroom rung

**Big [?] blocks** (`qol/big_q.rs`) and **treasure chests** are separate paths and
have no ladder rung to preserve. Coin is the right degrade for both. 7-F1's
tanooki comes from a Big [?], so this path is required, not optional.

## Gate carriers

| Carrier | Shape | Where it can go | Legible? |
|---|---|---|---|
| **Wall** | map tile, `wand_gate.rs` pattern | any path cell — authorable | yes |
| **Canoe / anchor** | traversal ability | W3, W8 (docks only) | yes, if the boat is visible |
| **Gated level** | the level's own dispenser | wherever the level lands | only with a marker tile |
| **Gated fort** | same, on a fortress | wherever the fort lands | **no — open problem** |
| ~~Hammer rock~~ | parked, see below | | |

### Walls

`wand_gate.rs` is this pattern, already shipped and playtested. Its own doc
comment carries the argument:

> The gate is a **wall**, not a lock. A lock that no fortress opens teaches the
> player the wrong rule about every other lock on the map. … `Map_Reload_with_Completions`
> rebuilds the whole map into `Tile_Mem` from the ROM grid on every map load. The
> opener therefore does not have to *record* anything: it re-derives the gate
> every time the map is drawn. … Game over, Continue and re-entering World 8 are
> all automatically correct because there is nothing to get out of sync.

An item wall is that module with one predicate swapped: `WANDS_TABLE` sum ≥ K
becomes a found-mask test. Same tile trick, same position-keyed stamp, same
no-persistence argument.

Walls are the backbone of the vocabulary because they are **authorable** — an
item wall's opener is global, so it needs a path cell, not a graph cut. That
largely dissolves the parked research's open fork (may the planner *manufacture*
chokepoints, or only *discover* them): feasibility stops being terrain's
decision. Discovery still governs quality — a wall that severs nothing is
decoration.

### Canoe and anchor

The only **traversal** gate in the vocabulary, which is what a metroidvania runs
on. Park the boat out of reach and gate `canoe_summon.rs` on the anchor:

- **No anchor** — boat unreachable, summon dead, water is a wall.
- **Anchor** — summon live, canoe available at *any* dock, stranding impossible.

Note the objection this inverts. `canoe_summon.rs` is always-on *because* a
parked canoe strands the player ("the classic canoe softlock"). Gating boarding
alone would reintroduce that; gating **the summon** makes the post-gate state
strictly safer than vanilla.

**The unanswered objection.** `world_maze_design.md`'s "Still open" list already
parked this idea, for a reason this document does not dispose of: *canoe edges
gate on dock walk-reachability in the walker and that is load-bearing, so a
portable boat changes walker semantics rather than adding an item.* Gating the
summon instead of boarding does not escape it — a conditional summon still makes
canoe edges conditional. **So the canoe is not the cheap gate it looks like**,
and it should be costed as a walker change, not as a bit test. The wall carrier
has no such cost, which is another reason to build walls first.

Two mechanics notes:

- `FS_CANOE_SUMMON` is **origin-locked** to `$DEA5` (self-referential `JMP` plus
  table reads). Inserting the test at the front shifts every internal absolute
  reference. Put it in a tail, or recompute and let
  `asm::check(…).origin(CANOE_SUMMON_CPU)` catch the error — that check exists
  for exactly this.
- Make the boat **visible but unreachable**, not absent. Seeing the thing you
  cannot reach is what sends a player looking. A dock where A does nothing
  teaches nothing.

### Marker tiles for gated levels

`$68` (2-Pyramid) and `$69` (2-Quicksand) are real pointer-table entries (W2
entries 32 and 42) that **the randomizer never places** — verified, no
references anywhere in the overworld code. Both sit above the page-1 enterable
threshold `$67`, so vanilla enters them as levels: the hard requirement for a
marker tile, met for free.

**Verify before committing:** both are absent from `Map_Removable_Tiles` *and*
`Map_Completable_Tiles`, and `smb3_rom_reference.md:5651` states they are
"non-completing, so the classification never fires." A gated level that can never
render as cleared is a problem on its own; whether the clear still registers in
`completion_bits` (and the row 7/8 shared bit) is the question that could sink
the tile choice.

**The bigger opportunity is the free tail.** Page 1's undefined indices are
`$6B`–`$7F` — 21 slots, same palette page, all above the enterable threshold. So
a marker *family* is possible, one per item, rather than two generic markers.
`hint_locks` set the precedent for composing hint tiles per seed; a new tile
family has to be taught to `hint_orientation`.

The trade: the vanilla pair is free but carries **desert** semantics, and a
pyramid in World 6's ice reads as a bug. A minted marker reads the same
everywhere but costs CHR.

> **Doc drift found on the way.** `smb3_rom_reference.md:5650` calls `$6A`
> unused; `rom_data/tables.rs:254` claims it as `TILE_FORTRESS_W8`. Fix when
> touching that section.

### Gated forts — the open problem

7-F1 **keeps its fortress tile no matter what**, because the maze hint system
needs fortresses to be fortresses (`LockHint`, `lock_keys.rs`). So a walled fort
cannot wear a marker, while a walled level can.

That is not merely a legibility asymmetry. **A fort holds a lock key**, so an
ability-walled fort makes that item a prerequisite for the lock it opens. If the
winnability fixpoint does not know, it will believe a lock is openable when it is
not and ship an unwinnable seed. This belongs in the same invariant as
`locks.rs:27` ("a fort sealed behind its own lock is permanently unplayable
content"), generalized: **never place a gate whose cut contains a source of the
key that opens it.** Across eight worlds that is acyclicity of the key graph.

## The requirement table

Authored, one row per known gate. Nothing can derive these from level data — each
is the output of a chain a human read off the level.

| Level | Logic item | Why |
|---|---|---|
| 6-5 | **Leaf** | flight required; the level's dispenser is a Q-leaf (`0x22D74`) |
| 7-7 | **Star** | stars required to cross the muncher fields (4 Q-stars) |
| 7-F1 | **Mushroom + Tanooki** | big to break bricks → Big [?] → tanooki → flight |
| 8-F | **Mushroom** | must be big to break a block in sub-area 2 (`0x2B900`) |

7-F1 is the **conjunction** — the shape the parked research identified as the
direct source of sphere depth and the thing the current model cannot represent
at all. It arrives for free, authored by vanilla.

**These rows are stable only because the offsets are pinned.** `PROTECTED_OFFSETS`
and `FLOWER_OR_LEAF_QBLOCK_OFFSETS` in `powerups.rs` exist to *prevent*
softlocks; under this feature they become the gate *definitions*. Unpin them and
the requirement becomes a per-seed roll — `requirement(level) = whatever byte2
landed there` — which is the cheapest path to more gates later. The data is
already per-seed.

## Generator consequences

1. **The walker gates edges, not nodes.** A level you cannot finish still reads
   as "reached". Per the parked research (not re-verified this session):
   `expand` at `walk.rs:193` tests the connecting path tile; the working
   mechanism already exists as a census instrument — `required_levels` blanks a
   cell to `BACKGROUND_TILES[0]` in `base_grids`, and background is in neither
   `VALID_HORZ` nor `VALID_VERT`. The defect is scheduling: `base_grids` is
   computed once *outside* the fixpoint loop (`mod.rs:433`) while `shut` is
   rebuilt every round. Re-stamp gated cells per round.
2. **Acyclicity of the key graph** — see "Gated forts" above. The highest-stakes
   case is a mushroom source behind a big-gate.
3. **Monotonicity survives** because the found-mask is sticky. It would not for a
   consumable, which is why coins and hammers are not keys.
4. **Mushroom placement is the pacing lever**, not a knob. How long the player
   stays small is decided by where the first mushroom source sits — the sphere
   planner's job.

## Backtracking is a hard dependency

The loop is: hit wall → go find item → **come back**. Issue
[#266](https://github.com/hamstringquestionable/SMB3R/issues/266) records that
the maze forces No Game Over Penalty on and `completion_bits` keeps clears across
world hops, so *"in the maze, a cleared level stays shut for the rest of the
run."* For bingo that is a lost square. Here it breaks the core loop, because the
item may be inside a level already cleared, or the route back may run through
one.

**#266 is a prerequisite, not a nicety.** Its "farming: probably fine, decide on
purpose" bullet also stops being incidental — re-entering for the item *is* the
mechanic.

## Build order

0. **Prerequisites.** Confirm the found-mask byte survives Game Over → Continue.
   Confirm `$68`/`$69` completion behaviour. Resolve #266.
1. **The found-mask.** One byte of persistent RAM; per the parked research
   `$7ABD` and `$7ADA–$7ADF` are proven-free at runtime (not re-verified here).
   Set the bit wherever an item is granted from a Toad House, chest or letter —
   vanilla behaviour otherwise unchanged, so the acquisition moment still reads
   as a normal reward.
2. **The dispenser gate.** The split hook in `LATP_Flower` and `LATP_Leaf`, then
   Big [?] and chests. Allocation is the open mechanical item: `LATP_JumpTable`
   lives in PRG008, which has no row in the free-space table, so the routines
   likely live elsewhere and are reached by repointing the jump-table words —
   the project's own cheapest move. **Verify the target bank is mapped when the
   bump handler runs** before counting on it (PRG006 has 1392 contiguous bytes
   at `$C000` in-level and is the first candidate).
3. **Walls.** The `wand_gate.rs` pattern with a found-mask predicate.
4. **The requirement table + walker fix**, so gated levels become real nodes.
5. **Canoe / anchor.**
6. **Marker tiles.**
7. **Sphere planning**, once the vocabulary is real enough to plan with.

Standard mode must come out byte-identical (`rom_identity`) and maze
census-equivalent (`test_route_census`, read per-world, not on the global mean).

## Open decisions

1. **How does a walled fort announce itself?** It cannot wear a marker tile, and
   it is the one place illegibility becomes a soundness risk rather than a UX
   one. The highest-value unresolved question in this document.
2. **What does a marker encode** — "this level is walled" (one tile, cheap, the
   player still guesses the item) or "this level needs *X*" (a family from
   `$6B`–`$7F`, legible, costs CHR)? Leaning toward the family: a wall you cannot
   read sends the player nowhere in particular, and the genre runs on knowing
   what you are looking for.
3. **Vanilla tiles or minted ones**, given the desert-semantics problem.
4. **Maze-only, or a game-wide flag.**

## Parked, with reasons

- **Hammer rocks as gates.** The hammer is consumable, so it cannot be a key —
  the same reason coins cannot. **But the stated blocker may not be real:** gate
  the rock on *"has ever found a hammer"* rather than on holding one, and
  wasting hammers on scenery becomes harmless, vanilla rocks can all stay, and
  no rock-removal pass is needed. Worth knowing before un-parking.
- **Buying an item you have not found.** Good feature, no good mechanism yet.
  Note that saying yes makes coins an alternate key source and forces the
  generator to model the economy — the thing this design otherwise avoids
  entirely.
- **The coin / permanent-item economy layer** from the parked research. Once
  finding a leaf turns on every leaf block, the permanent-item layer only buys
  carry-in, while costing the panel traps (slot-0 dead panel, the A-press
  pricing hook, consumption suppression). Judge it separately, after the unlock
  mechanism stands on its own.
- **Ice blocks.** Tileset-12 only. Flavour, never a key.

## Related documents

- [world_maze_design.md](world_maze_design.md) — the mode this feature is for.
  Its "Still open" list already parks the water-gap idea; see the canoe section
  above for the objection that is still unanswered.
- [vision.md](vision.md) — `:39` ("every level beatable entering as small
  Mario") is what makes gating the mushroom safe, and the charter a game-wide
  flag would have to be argued against.
- [choice_first_charter.md](choice_first_charter.md) — the overworld builder's
  authority. This feature adds gates to worlds it builds; it does not change it.
- [seed_stability.md](seed_stability.md) — the bar any of this has to clear:
  standard byte-identical, maze census-equivalent per world.
- The parked item-economy research —
  [artifact, 2026-09-12](https://claude.ai/code/artifact/d4bd2f24-1cfa-4e34-969e-04f365339c42).
  Approach superseded here; findings, ROM offsets and the corrections it records
  still stand. **The artifact itself carries no forward pointer to this
  document.**

## Provenance

Read from the local Southbird disassembly and this repo on 2026-09-19:
`LATP_JumpTable` and its handlers (`prg008.asm:5137-5230`), `Bouncer_PUp`
(`prg001.asm:1015`), `PowerUp_Ability` (`prg000.asm:558`), the `Player_QueueSuit`
consumer (`prg008.asm:795-830`), `qol/canoe_summon.rs`, `wand_gate.rs`,
`powerups.rs`, `rom_data/tables.rs`, `smb3_rom_reference.md:5620-5690`, and
issue #266. The `$68`/`$69` "never placed" claim is from a reference search of
`src/randomize/`.

Inherited from the parked artifact and **not re-verified here**: the free
persistent-RAM bytes, `walk.rs:193`, `locks.rs:27` and `:60-65`, `mod.rs:433`
and `:499`, and the PRG026 free-space figures.

The census figures quoted in the parked artifact (4.92 spheres, 60 seeds/arm,
2026-09-12) are a dated snapshot. Re-measure before settling a decision on one.
