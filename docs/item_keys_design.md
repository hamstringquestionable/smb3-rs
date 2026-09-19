# Item Keys — gating power-ups on what the player has found

> **Status: design note. Nothing is implemented.** No flag, no code; the branch
> `docs/item-keys-design` carries this document and nothing else.
> This records a design conversation (2026-09-19) to the point where it could be
> built, and marks what is still undecided. Line numbers and offsets cited from
> Rust will drift; the disassembly citations will not.
>
> **Prerequisite pass, 2026-09-19 (second session).** Two of step 0's three
> prerequisites are discharged from source and one is not — see "Build order".
> That pass also settled the last allocation unknown, corrected the second
> dispenser, and found one claim in `smb3_rom_reference.md` wrong (now fixed
> there).
>
> It supersedes the *approach* of the parked item-economy research
> ([artifact, 2026-09-12](https://claude.ai/code/artifact/d4bd2f24-1cfa-4e34-969e-04f365339c42),
> memory `item_economy_spheres`) while keeping its findings. That design emptied
> the levels — every power-up block became a coin, permanently, and the inventory
> became the only supply. This one leaves the levels alone and *schedules* them.

## The mode in one paragraph

A power-up block only dispenses an item the player has already **found** from an
overworld source (a Toad House chest, the Peach letter). Finding one is
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

### The second dispenser is the Big [?] block — and the chest is not one

**Verified 2026-09-19.** An earlier draft named "Big [?] blocks and treasure
chests" as two dispensers with no mushroom rung. Half of that is right.

**Big [?] blocks are a dispenser, and a well-shaped one.** They are *objects*
`$94`–`$9A` (`OBJ_BIGQBLOCK_3UP` … `_HAMMER`) with their AI in **PRG005** — not
a `LATP_` handler at all, so `qol/big_q.rs` (which only fixes *which room* the
bonus pipe opens) is not where this goes. One site decides the product:
`BigQBlock_EmergePowerup` (`prg005.asm:4662`) reads
`BigQBlock_Item-OBJ_BIGQBLOCK_3UP,X` into `Level_ObjectID,Y` and
`BigQBlock_StarManFlash-…,X` into `PUp_StarManFlash` — the emerging object plus
the frame that turns a starman into the tanooki, frog or hammer suit. So the
gate is again a substitution at a single site, indexed by the block's own object
id, and the tanooki **7-F1 depends on** goes through it. That block is not *in*
7-F1: it lives in whichever bonus room 7-F1's pipe draws, and
`randomizer/mod.rs:519` forces that room's block to `OBJ_BIGQBLOCK_TANOOKI`
(`$98`) after the contents roll — which is why the roll itself
(`enemies/mod.rs:56`) exempts nothing and needs no offset pin. The "no
mushroom rung" reading holds: the table is one item per block type, with no
`Player_Suit` branch anywhere in it.

**The coin degrade is not free here, though.** `LATP_Coin` is in PRG008, and
PRG008 is not resident while object AI runs — PRG002, PRG005 and PRG008 all map
to `$A000`. Nor is there a level *object* for a coin; the popped-out coin is a
special object (`SOBJ_POPPEDOUTCOIN`, spawned by writing `SpecialObj_ID` /
`_XLo` / `_YLo`, the pattern at `prg002.asm:6190`). Three candidate degrades,
undecided:

1. **Emerge nothing, and leave the block shut.** `BigQBlock_Open`
   (`prg005.asm:4627`) is the single hit site, and it calls
   `BigQBlock_EmergePowerup` *before* setting the opened frame and the
   `BigQBlock_GotIt` bit — so an early return costs a handful of bytes and the
   block is still there to hit after the unlock. It is the only degrade that
   loses the player nothing. Its problem is feedback: a Big [?] that does
   nothing reads as a bug, which is the objection that ruled out
   `Player_QueueSuit`.
2. **Spawn the popped-out coin.** Consistent with the LATP degrade, and a
   straight run of stores — no cross-bank call. Costs a free special-object slot
   and more bytes.
3. **Emerge a 1-Up.** Cheapest of all (one table byte), but it pays out a
   currency the mode does not otherwise use.

**Treasure chests are a source, not a dispenser.** There are two kinds, and an
earlier draft of this paragraph claimed there was only one — corrected
2026-09-19. The Toad House box (`LoadLevel_ToadChest`, `prg018.asm:1675`) is
opened by `ToadHouse_ChestPressB` (`prg029.asm:891`), which resolves
`THouse_Treasure` through `ToadHouse_ItemOff` / `ToadHouse_RandomItem` /
`ToadHouse_Item2Inventory`. And seven **in-level** chests exist as well — the
`D6` `OBJ_TREASURESET` object, whose row sets `Level_TreasureItem`: the Music
Box, Cloud and Star chests, 1-F's whistle chest, and the three 8-Hnd chests
(`TREASURE_CHEST_OFFSETS` in `items.rs`).

Both kinds hand the player an item **into the inventory**; neither dispenses
one into a level. So the conclusion stands and is if anything stronger: chests
are where step 1 *sets* the found-mask, and gating one on the mask would be
circular. PRG029 maps at `$C000` and has 2568 free bytes (largest gap 1528), so
the Toad House setter has room.

One consequence for the generator, recorded in `mimaze_layer_design.md`: the
in-level chests cannot be placed by geometry, because which *level* sits on a
given map slot is decided by `overworld_writer::assign_pool`, downstream of
where a planner would run.

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

**Verified 2026-09-19 — they complete, and the reference was wrong.** Both are
absent from `Map_Removable_Tiles` *and* `Map_Completable_Tiles`, which is what
sent the first draft to `smb3_rom_reference.md:5651` and its "non-completing, so
the classification never fires". That sentence is a fact about where vanilla
*puts* those bytes, not about the bytes themselves, and it has been corrected
there. The M/L test is not even a threshold any more: `lock_keys::ML_RANGE` made
it a window, and page 1's is `[$67, $6A)` (`ML_RANGE_UPPER = [0x16, 0x6A, 0xC0,
0xEC]`). So:

- `$68` and `$69` are **inside** the window, so a completed cell wearing one
  flips to an M/L marker on reload. A gated level renders as cleared.
- `is_completion_unsafe($68)` is therefore true (`overworld_build/capacity.rs`),
  so the cell **claims a completion bit** in the packed stencil and the clear
  survives world hops.
- Entry reads the *other* row — `+4`, through the `$7E94` RAM copy — with the
  same thresholds, and `$68 >= $67`, so the tile is enterable and a clear FX
  plays.

What comes out of this is a placement constraint rather than a blocker: because
a marker claims a completion bit, it is subject to the **row 7/8 shared bit**
like every other completable cell, so a marker at row 7 collides with a level at
row 8. Whatever places markers has to go through `completable_positions`, the
same as everything else.

**The bigger opportunity is the free tail** — but it costs more than CHR.
Page 1's undefined indices are `$6B`–`$7F`: 21 slots, same palette page, all
above the enterable threshold. So a marker *family* is possible, one per item,
rather than two generic markers. `hint_locks` set the precedent for composing
hint tiles per seed; a new tile family has to be taught to `hint_orientation`.

**The catch found on 2026-09-19: that tail is exactly what `ML_RANGE` cut out
of the M/L window.** `ML_RANGE_UPPER[1]` is `$6A` *on purpose* — bounding the
top of each page released the undefined tail to the **obstacle** role, so a tile
there falls through to the removable scan instead of becoming an M/L panel. A
marker minted at `$6B`–`$7F` therefore does **not** flip to M/L and takes **no**
completion bit: precisely the failure this section feared and did not find in
`$68`/`$69`. Re-admitting one costs a choice:

- **Widen page 1's window.** One byte in the bound table. It re-admits the whole
  span it covers as panels, so every index it reaches stops being available as
  an obstacle — the 21 slots are one pool, shared with `hint_locks`.
- **Give it a row in `Map_Completable_Tiles`.** That list is checked *first* and
  unconditionally, so it re-admits a tile whatever the window says. But it is 5
  bytes at `$A447`, contiguous with the removable pair on one side and
  `Map_CompleteByML_Tiles` on the other, so it needs the same relocation
  `FS_MAP_REMOVABLE` did for its neighbours — **and a matching row in
  `Map_ForcePoofTiles`** (`$A9D5`, PRG011), which is a second, separate copy of
  the same five bytes answering "which clear FX plays". Two tables, two banks,
  neither aware of the other.

So the trade is not "free but desert-flavoured" versus "costs CHR". It is: the
vanilla pair completes for nothing and carries **desert** semantics — a pyramid
in World 6's ice reads as a bug — while a minted family reads the same
everywhere and costs CHR *plus* a table relocation in two banks, or a slice of
the same 21 slots the obstacle role wants.

> **Doc drift found on the way — fixed 2026-09-19.** One sentence of
> `smb3_rom_reference.md` called `$6A` unused while `rom_data/tables.rs:254`
> claims it as `TILE_FORTRESS_W8`; the same sentence carried the wrong
> `$68`/`$69` claim above. Both are corrected, and the passage now points at
> that section's own "`$6A` `TILE_LARGEFORT` has no completion path", which had
> been right all along.

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

0. **Prerequisites.** Two of the three are discharged (2026-09-19); one is not.

   * ~~Confirm the found-mask byte survives Game Over → Continue.~~ **It does**,
     by shipped precedent. The mask belongs in the `$7A73–$7ADF` run, and
     `maze_state.rs`'s header states the rule the code keeps: the only thing
     that clears that run is the new-game signal,
     `completion_bits::NEW_GAME_INIT`, hooked onto the title menu's
     `STA Debug_Flag` — which a Continue never reaches. `WANDS_TABLE` and
     `MAP_OBJ_DEAD` already rest on exactly this, and have shipped and been
     playtested. Six bytes are free: `MAZE_STATE_NEXT` is `$7ADA`,
     `MAZE_STATE_END` is `$7ADF`. A mask declared in `maze_state.rs` is zeroed
     for free, because `NEW_GAME_INIT` clears the whole declared run rather than
     a list.

     **But that clear is maze-gated.** `world_persist::apply` runs only inside
     `if let Some((state, wands)) = maze` (`randomizer/mod.rs:438`). A maze-only
     flag inherits the new-game clear; a **game-wide** one has to install its
     own, or a second game in one session opens with the first game's unlocks
     still lit. That turns open decision 4 from a scoping question into a costed
     one.

   * ~~Confirm `$68`/`$69` completion behaviour.~~ **They complete** — see
     "Marker tiles for gated levels". The feared blocker is not there; what came
     out of it is the row 7/8 placement constraint.

   * **Resolve #266.** Still open, still labelled `needs refinement`, and its own
     "Settle before building" list — scope, which tiles, what a second clear
     does, farming — is undecided. This is the one real blocker, and it is a
     design decision rather than code.
1. **The found-mask.** One byte of persistent RAM; per the parked research
   `$7ABD` and `$7ADA–$7ADF` are proven-free at runtime (not re-verified here).
   Set the bit wherever an item is granted from a Toad House, chest or letter.
   The Toad House and the chest are the same site — `ToadHouse_ChestPressB`
   (`prg029.asm:891`), which returns an inventory index — and PRG029 has room to
   spare. Vanilla behaviour otherwise unchanged, so the acquisition moment still
   reads as a normal reward.
2. **The dispenser gate.** The split hook in `LATP_Flower` and `LATP_Leaf`, then
   the Big [?] block in PRG005 — the chest belongs to step 1, because it is a
   source. Reached by **repointing the `LATP_JumpTable` words** for
   entries 1 and 2 — the project's cheapest move, and the only one available
   here, because PRG008 cannot absorb even a `JSR` (see below).
3. **Walls.** The `wand_gate.rs` pattern with a found-mask predicate.
4. **The requirement table + walker fix**, so gated levels become real nodes.
5. **Canoe / anchor.**
6. **Marker tiles.**
7. **Sphere planning**, once the vocabulary is real enough to plan with.

Standard mode must come out byte-identical (`rom_identity`) and maze
census-equivalent (`test_route_census`, read per-world, not on the global mean).

## Allocation — verified 2026-09-19

`smb3-rs <rom> --free-space`, the `FREE_SPACE_ALLOCATIONS` registry, and label
ranges plus raw bytes from the ROM.

### The banks that are resident

During a block bump the four slots hold:

| Slot | Bank | `$FF` free | Largest gap |
|---|---|---|---|
| `$8000` | PRG030 (fixed) | 42 | 42 @ `0x3DFE6` |
| `$A000` | **PRG008** — all the LATP code (`$A0C8`–`$BFF9`) | **0** | — |
| `$C000` | **PRG000** (`$C3E7`–`$DEBB`) | **0** | — |
| `$E000` | PRG031 (fixed) | 81 | 30 @ `0x3E972` |

PRG008 and PRG000 are co-resident — proven by PRG008 reading `PowerUp_Ability`
at `$C3E0` in PRG000 — and neither has a byte of `$FF` filler. So the handlers
**cannot be patched in place**: there is not one spare byte in PRG008 for a
`JSR`. Repointing the jump-table words is not merely the cheapest option, it is
the only one, and it costs nothing because the word is overwritten.

> **Correction.** An earlier draft named PRG006 (1392 bytes at `$C000`) as the
> first candidate. That is wrong — PRG006 is a *data* bank swapped in to read
> enemy streams; during gameplay `$C000` is PRG000. Do not budget from the
> `$C000` row of the CLAUDE.md table without checking which bank is actually
> resident at the moment your code runs.

### Reclaimed space the scan cannot see

Neither existing PRG000/PRG001 allocation helps, and it is worth recording why,
because both look like they should:

- `FS_POISON_MUSHROOM` (`0x02713`, CPU `$A703`) and `FS_POISON_HOOK`
  (`0x02724`) are in **PRG001**, which sits at `$A000` — the same slot as
  PRG008. Mutually exclusive; unusable here.
- PRG000's only reclaimed row is `fs(0x00928, 7, ["macobra"])`, dead code at CPU
  `$C918` skipped by a vanilla `JMP $C927`. Seven bytes, already spent.

### The answer: repointing frees the handlers themselves

`LATP_Flower` and `LATP_Leaf` are each referenced from **exactly one place** —
their own word in `LATP_JumpTable`. Verified across the whole disassembly.
Repointing those two words therefore frees both bodies outright, and they are
adjacent. From the ROM at `0x117FC` (CPU `$B7EC`):

```
a9 00  8d 86 05  a0 05  a5 ed  f0 02  a0 02  60   ; LATP_Flower, 14 bytes
a9 00  8d 86 05  a0 05  a5 ed  f0 02  a0 03  60   ; LATP_Leaf,   14 bytes
a9 80  8d 86 ...                                   ; LATP_Star begins
```

**28 contiguous bytes in PRG008 itself** — the dispatcher's own bank, so there
is no mapping question at all, and **the always-mapped gaps do not have to be
spent.** This is the project's own recorded lesson in miniature: repointing a
jump-table vector reclaims the code it pointed at.

**Budget — settled by building it, 2026-09-19.** The sketch here guessed about
32 bytes and offered two ways to find the missing four. Neither was needed: the
routine is **27**, and `randomize::item_keys` is it. Three things paid for the
difference, and the first two were not in the sketch at all:

1. **`Y` already holds the block type on entry.** The dispatcher does
   `LDA Temp_Var1 / ASL A / TAY` and never touches `Y` again before
   `JMP [Temp_Var1]`, so the row index is `TYA / LSR A` — no entry stubs, and
   the `$2C` skip trick is not needed.
2. **One table, not two.** A row holds the `Bouncer_PUp` index to return, and
   **zero means locked**. So "is it unlocked" and "what does it give" are a
   single `LDA`, and the found-mask never appears in the code — which is also
   what lets the found set be a build-time table in the POC and an SRAM read in
   the feature, with no other change.
3. `X` is never touched, for correctness rather than size: it carries the
   tile-check index into these handlers (`LATP_Brick` reads it with
   `CPX #$04`, `LATP_GetCoinAboveBlock` backs it up around a call), so the row
   index lives in `Y`. Indexing with `X` would have been the same byte count
   and a live-register bug.

Entry 0 (`LATP_None`, `LDY #1 / RTS`) sits just above and is 3 more bytes if it
is ever worth repointing too — but it is live, reached from `LATP_QBlocks`.

**Correction, 2026-09-19: two of these 28 bytes are already spoken for.** The
free-space audit caught it on the first build. `qol::apply_modern_powerups`
(MaCobra52's Easy Power-up System, `Options::modern_powerups`, default off)
writes `0x11802` and `0x11810` — the `LDY #$05` operands *inside* these two
handlers — changing the product a **small** player gets from a mushroom to the
suit itself. So 26 of the 28 are free; two belong to that option whenever it is
on.

That is not merely an allocation clash. Modern Power-ups **deletes the mushroom
rung** — the thing "The gate: split on the branch that already exists" turns
into the strongest key in the vocabulary. The two features are making opposite
claims about the same two bytes.

**Resolved as a difficulty dial rather than an error** (decided 2026-09-19).
Modern Power-ups on is the *easier* mode: small Mario is powered up directly,
so the mushroom is no longer a gate and the permanently-small opening does not
happen. Off — the default — keeps the mushroom as a key. The routine has a
variant for it and both fit the same allocation:

| arm | routine | mushroom |
|---|---|---|
| default | 27 bytes, splits on `Player_Suit` | a key |
| Modern Power-ups | 21 bytes, no suit test — the row is the block type | not a gate |

The easier variant is simply the default one with `LDY Player_Suit / BNE big /
LDA #$00` removed, because under that patch both paths return the same product
anyway. **Not built yet** — `apply_poc` currently asserts vanilla's byte and
refuses the combination, which is the honest state until the variant exists.

**Nothing else competes for the run.** The Big [?]
path lives in PRG005 and the chest is not a dispenser at all — see "The second
dispenser is the Big [?] block" above. PRG005 has one 58-byte gap at `0x0BFD6`
(CPU `$BFC6`, the bank's tail), so the Big [?] gate pays its own rent in its own
bank. It has no choice: PRG002, PRG005 and PRG008 all map at `$A000`, so the
LATP gate and the Big [?] gate can never be one routine — no sharing, and no
call between them. What they *can* share is the found-mask itself, which is SRAM
and belongs to no bank.

## Open decisions

1. **How does a walled fort announce itself?** It cannot wear a marker tile, and
   it is the one place illegibility becomes a soundness risk rather than a UX
   one. The highest-value unresolved question in this document.
2. **What does a marker encode** — "this level is walled" (one tile, cheap, the
   player still guesses the item) or "this level needs *X*" (a family from
   `$6B`–`$7F`, legible, costs CHR)? Leaning toward the family: a wall you cannot
   read sends the player nowhere in particular, and the genre runs on knowing
   what you are looking for.
3. **Vanilla tiles or minted ones.** Now a costed question, not just an
   aesthetic one: `$68`/`$69` complete for free but read as desert, while the
   `$6B`–`$7F` family sits outside the M/L window `ML_RANGE` deliberately drew
   and has to be re-admitted — see "Marker tiles for gated levels".
4. **Maze-only, or a game-wide flag.** No longer free either way: the found-mask
   is cleared by `completion_bits::NEW_GAME_INIT`, which is installed only when
   the maze is on (`randomizer/mod.rs:438`). Game-wide means installing that
   clear outside the maze too, or a second game in one session starts with the
   first game's unlocks. See the prerequisites in "Build order".

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
issue #266. Bank residency and free space verified with `--free-space` and
disassembly label ranges (see "Allocation"). The `$68`/`$69` "never placed"
claim is from a reference search of
`src/randomize/`.

**Second pass, 2026-09-19** (the prerequisite sweep). Read: `maze_state.rs`
(the `$7AC1–$7ADF` map and its const assertions), `completion_bits.rs`
(`NEW_GAME_INIT` and its hook, the stencil, and
`is_completable_matches_rust_for_every_tile`),
`overworld_build/capacity.rs::is_completion_unsafe`, `lock_keys.rs`'s `ML_RANGE`
and `ML_RANGE_UPPER`, `randomizer/mod.rs:430-470`, `prg012.asm:125-400`
(`Map_Reload_with_Completions` and the three tile tables), `prg005.asm:4440-4710`
(the whole Big [?] object), `prg029.asm:860-965` (`ToadHouse_ChestPressB`),
`prg018.asm:1645-1710` (`LoadLevel_ToadChest`), `prg002.asm:6175-6200`
(`SOBJ_POPPEDOUTCOIN`), and `--free-space` against the Rev 1 ROM for the PRG005
and PRG029 figures. Bank CPU ranges were read off the disassembly's own label
prefixes (`PRG002_A…`/`PRG005_A…`/`PRG008_A…` at `$A000`, `PRG029_C…` at
`$C000`).

Inherited from the parked artifact and **not re-verified here**: `walk.rs:193`,
`locks.rs:27` and `:60-65`, `mod.rs:433` and `:499`, and the PRG026 free-space
figures. The free persistent-RAM bytes *were* re-verified this time — they are
`maze_state.rs`'s own six remaining bytes, `$7ADA–$7ADF`.

The census figures quoted in the parked artifact (4.92 spheres, 60 seeds/arm,
2026-09-12) are a dated snapshot. Re-measure before settling a decision on one.
