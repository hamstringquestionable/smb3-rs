# Big [?] Bonus Room Shuffle — design notes

**Status: implemented 2026-08-27.** Always on, no option. The `testrom
--bigq-unused5` knob remains for looking at a single room in isolation.

Mechanism reference lives in `docs/smb3_rom_reference.md` under "Big [?] Block
Bonus Areas" — tables, junction decode, pipe-tile types, palette indices, the
per-tileset command sizes. This file is only the plan and the decisions.

## What is built

`testrom --bigq-unused5 <screen> [--bigq-palette N]` points all eight
`LevelJctBQ_*` entries at "Unused Level 5" (TCRF), aims 5-2's bonus pipe at one
of its eight rooms, writes that room a return junction, and recolours the area.

Playtested in an emulator, all eight rooms: the area loads, the fortress-tileset
bank swap works, the block is collectable, and the pipe returns to 5-2. Palette
4 was chosen. This is the proof that the rooms are usable.

## What is built in the randomizer

`randomize/big_q_rooms.rs` — every host draws from the 19-room pool, always on,
no option and no flag-key bit. `qol/big_q.rs` grew the slot-seeding halves and
the four payload tables the pass fills in.

## The pairing model

A host level and a bonus room are bound by two junction commands, one at each
end. Re-pairing host H to room R is two 2-byte writes:

- `H.junction ← R.arrival` — where the player lands in the room. In the host's
  own layout, slot = the host pipe's screen.
- `R.junction ← H.return` — where the player lands back in the host. In the
  bonus **area**'s layout, slot = the room's screen.

Every vanilla area already carries a junction per room, so re-pairing costs no
space there. Unused Level 5 carries none and needs 3 bytes per room.

## Decisions taken

- **7-F1 is not pinned.** Flight is required to beat 7-F1 because of level
  geometry, so its block must always be a flight suit — but its *room* still
  shuffles. Resolve the pairing first, then force the block in whatever room
  7-F1 lands in and exclude that offset from the contents roll.
  `enemies::tables::W7F1_TANOOKI_OFFSET` becomes derived rather than constant.
  Guard it with an invariant test over N seeds, the way the enemy protections
  are guarded — the failure mode is silent if the passes are ever reordered.
- **No room is deleted from Unused Level 5.** The 24 bytes for eight junctions
  come from eight of its fifteen 3-byte `Coin` commands. Ten of those fifteen
  sit in the sealed chamber on screen 4 (rows 3-9, walled on all four sides and
  at both screen seams) which the player cannot reach.
- **BigQ8 s1 is dropped from the pool.** It is the only spare vanilla room that
  would need new bytes, its area has no spare decoration, and it is a duplicate
  of BigQ8 s4 and BigQ2 s1 — so excluding it costs no novelty.
- **No option at all — always on** (2026-08-26). Which room a pipe leads to is
  not a gameplay difference worth a control: every room hands out one block and
  returns you where you came from, so there is nothing to turn off. It joins
  `qol::fix_big_q_block_rooms` as unconditional behavior. No `Options` field, no
  flag-key bit, no key version bump, no web control — and `big_q_blocks` keeps
  its current meaning (what is *in* the block) and its existing key bit,
  untouched. Room selection and per-area palette ride together in the one pass.

  Consequence to keep in mind: the pass consumes RNG unconditionally, so it
  shifts the stream for everything downstream of it and rebaselines any pinned
  output. Place it deliberately in `randomizer/mod.rs`, not wherever is handy.

## Why it is worth doing

Vanilla's 15 Big [?] rooms are only **10 distinct layouts** — BigQ2 s1 = BigQ8
s1 = BigQ8 s4, BigQ3 s4 = s5, BigQ4 s2 = BigQ6 s5, and BigQ5 s7 = BigQ7 s6.
Unused Level 5's eight are all unique, taking the pool to 18 designs. Note the
last pair: 7-F1's room, the most-entered bonus room in the game, is already a
room you have seen in World 5.

## Plan

Pool as shipped: 11 vanilla rooms + 6 from Unused Level 5 = 17 rooms for 11
host levels. (This section's original 22 counted the spare vanilla rooms, whose
arrivals were never authored, and all eight Unused Level 5 rooms.)

Data to assemble:

- **Per host (11 rows)** — junction offset and return bytes, all harvestable
  from the room vanilla pairs it with.
- **Per room (22 rows)** — area index, screen, arrival bytes. 11 come free from
  the vanilla hosts; the 8 for Unused Level 5 already exist as
  `UNUSED5_ARRIVALS` in `testrom.rs`; 3 spare vanilla rooms need authoring.

Constraints the pairing must enforce:

- **Distinct room screens per world.** `BigQBlock_GotIt` is an 8-bit mask
  indexed by the room's screen and cleared on world entry, so two hosts in the
  same world paired to rooms with the same screen number share one
  "already opened" bit. "Same world" means after the overworld builder has
  placed levels, so this pass runs downstream of it.
- **7-F1's room holds a flight suit** (see above).
- **Pairing runs before `randomize_big_q_blocks`**, or the protection cannot be
  expressed.

Palette: per *area*, not per room — it is header byte 5 bits 0-2 of the area's
layout. Fortress areas roll from {0, 3, 4} (1 wants object palette 10). The
underground areas need the same survey run before trusting a set; vanilla only
samples 3, 4 and 7 there.

## Harvest, part 1: the room side (done, verified 2026-08-26)

Every vanilla room, its block, and the return junction its area supplies.
`arrival` is the byte2 a host junction needs to land on that room's screen —
`(col << 4) | screen`, though only the *screen* nibble is forced (see below).
`return` is the group-7 command in the bonus area's own layout, slot = room
screen, that sends the player back; bytes 1-2 are the position in the host.

| Area | bgpal | Room | Block | arrival | Return cmd | Return offset | Lands in host at |
|---|---|---|---|---|---|---|---|
| BigQ1 | 7 | — | *(no blocks)* | — | — | — | — |
| BigQ2 | 3 | s1 | 3-Up | `$81` | `E1 12 95` | 0x1B294 | scr 5 col 9, Yidx 1 dir 2 |
| BigQ2 | 3 | s4 | *(none)* | — | `E4 71 25` | 0x1B297 | scr 5 col 2, Yidx 7 dir 1 |
| BigQ3 | 3 | s1 | Tanooki | `$61` | `E1 00 00` | 0x1B378 | **null — dir 0** |
| BigQ3 | 3 | s4 | 3-Up | `$84` | `E4 71 46` | 0x1B37B | scr 6 col 4, Yidx 7 dir 1 |
| BigQ3 | 3 | s5 | Frog | `$85` | `E5 61 76` | 0x1B37E | scr 6 col 7, Yidx 6 dir 1 |
| BigQ4 | 3 | s2 | 3-Up | `$72` | `E2 51 86` | 0x1B3E1 | scr 6 col 8, Yidx 5 dir 1 |
| BigQ4 | 3 | s3 | Frog | `$73` | `E3 61 03` | 0x1B3E4 | scr 3 col 0, Yidx 6 dir 1 |
| BigQ5 | 3 | s3 | 3-Up | `$73` | `E3 61 65` | 0x1B479 | scr 5 col 6, Yidx 6 dir 1 |
| BigQ5 | 3 | s7 | Tanooki | `$77` | `E7 42 E5` | 0x1B47C | scr 5 col 14, Yidx 4 dir 2 |
| BigQ6 | 4 | s3 | 3-Up | `$53` | `E3 61 64` | 0x1B5E8 | scr 4 col 6, Yidx 6 dir 1 |
| BigQ6 | 4 | s5 | Tanooki | `$75` | `E5 11 A3` | 0x1B5EB | scr 3 col 10, Yidx 1 dir 1 |
| BigQ6 | 4 | s6 | Hammer | `$56` | `E6 61 E3` | 0x1B5EE | scr 3 col 14, Yidx 6 dir 1 |
| BigQ7 | 3 | s4 | Hammer | `$44` | `E4 61 29` | 0x1B664 | scr 9 col 2, Yidx 6 dir 1 |
| BigQ7 | 3 | s6 | Tanooki | `$76` | `E6 61 C6` | 0x1B667 | scr 6 col 12, Yidx 6 dir 1 |
| BigQ8 | 7 | s1 | Tanooki | `$81` | *(none)* | — | — |
| BigQ8 | 7 | s4 | 3-Up | `$84` | `E4 51 F5` | 0x1B725 | scr 5 col 15, Yidx 5 dir 1 |

Area pointers (`LevelJctBQ_*` index = vanilla `World_Num`), all tileset 14:

| Area | Layout | Objects |
|---|---|---|
| BigQ1 | `$B1AF` 0x1B1BF | `$C976` 0x0C986 |
| BigQ2 | `$B1BC` 0x1B1CC | `$C978` 0x0C988 |
| BigQ3 | `$B28B` 0x1B29B | `$C97D` 0x0C98D |
| BigQ4 | `$B372` 0x1B382 | `$C988` 0x0C998 |
| BigQ5 | `$B3D8` 0x1B3E8 | `$C990` 0x0C9A0 |
| BigQ6 | `$B470` 0x1B480 | `$C998` 0x0C9A8 |
| BigQ7 | `$B5E2` 0x1B5F2 | `$C9A3` 0x0C9B3 |
| BigQ8 | `$B65B` 0x1B66B | `$C9AB` 0x0C9BB |

Three things this settled:

- **The junction slots match the ROM reference exactly**, including BigQ5's two
  and BigQ8's single one — so the "BigQ8 s1 needs new bytes" decision holds.
- **BigQ3 s1's return is `E1 00 00`** — a null command, dir 0, outside the valid
  1-4 range. That is one of the three spare rooms, and its return was never
  authored. Any room the pass hands a host needs a real return written, spare
  rooms included; `sanitize_exit_dir`'s rule (remap to 3) is the precedent.
- **Object streams open with a 1-byte prefix** before the 3-byte entries. Parsing
  from the pointer directly steps over the `$FF` terminator into the next
  stream and silently over-counts — it produced blocks for BigQ1 (which has
  none) and shifted every area's block list by one. Vanilla is 0/1/3/2/2/3/2/2
  = 15, which is what the reference already said.

## Harvest, part 2: the host side (candidates, 2026-08-26)

Found by taking each host's entry `obj_ptr` straight from `qol/big_q.rs`'s own
lookup table, resolving it through the world pointer tables, walking its alt
chain, and keeping every group-7 command whose byte2 screen nibble is a block
screen in that host's area.

`arrival` is the host command's bytes 1-2 — reusable as *any* host's arrival for
that room. `return` is the room's command bytes 1-2, from the part-1 table.

| Host | Room | Junction | Bytes | arrival | return | Confidence |
|---|---|---|---|---|---|---|
| 3-5 | BigQ3 s4 3-Up | 0x25227 | `E4 02 14` | `02 14` | `71 46` | single candidate |
| 3-9 | BigQ3 s5 Frog | 0x1F31B | `E2 02 15` | `02 15` | `61 76` | single candidate |
| 4-F2 | BigQ4 s2 3-Up | 0x2B6B2 | `EA 52 22` | `52 22` | `51 86` | single candidate |
| 5-2 | BigQ5 s3 3-Up | 0x1A807 | `E4 02 73` | `02 73` | `61 65` | **confirmed** |
| 5-5 | BigQ5 s7 Tanooki | 0x23045 | `E2 02 17` | `02 17` | `42 E5` | single candidate |
| 6-3 | BigQ6 s5 Tanooki | 0x22B2F | `E3 52 25` | `52 25` | `11 A3` | single candidate |
| 6-9 | BigQ6 s3 3-Up | 0x20B2F **or** 0x20C40 | `E3 02 83` / `E6 61 43` | — | `61 64` | room certain, command ambiguous |
| 6-10 | BigQ6 s6 Hammer | 0x23A93 | `E4 12 D6` | `12 D6` | `61 E3` | single candidate |
| 7-F1 | BigQ7 s6 Tanooki | 0x2B47E | `E6 02 16` | `02 16` | `61 C6` | **room confirmed** |
| 7-8 | BigQ7 s4 Hammer | 0x1F074 | `E6 52 14` | `52 14` | `61 29` | by elimination |
| 8-1 | BigQ8 s4 3-Up | 0x1F8C8 | `E2 02 D4` | `02 D4` | `51 F5` | single candidate |

5-2 is ground truth — `testrom --bigq-unused5` already drives that command, and
it agrees. 7-F1's *room* is pinned independently by
`enemies::tables::W7F1_TANOOKI_OFFSET` (0x0C9B7), which is the second entry in
BigQ7's object stream, i.e. the s6 Tanooki — the block flight is required for.
That resolves 7-F1 to s6 and, since a room serves one host, 7-8 to s4.

Eight of the eleven arrivals are `02 xx` — Y index 0, dir 2 — the "placed at row
0 inside the ceiling pipe, falls out of the mouth" shape that `UNUSED5_ARRIVALS`
was built to imitate. 4-F2, 6-3 and 6-10 use `52`, `52` and `12` instead.

### The return is not tied to the pipe (resolved 2026-08-26)

An earlier note here called it an unresolved inconsistency that a host's
junction slot does not match the screen in its room's return bytes — 5-2's pipe
is on screen 4 and its return lands on screen 5. That was a bad assumption, not
a data problem. **A return position is just a hand-authored safe spot in the
host; nothing ties it to the pipe.**

Both halves of the slot model are confirmed in the disassembly:

```asm
LoadLevel_StoreJctStart:            ; prg014 - fills the slots during the parse
    LDA <Temp_Var15                 ; the command's byte0
    AND #$0f
    TAY                             ; slot = byte0 & $0F
    ...  STA Level_JctYLHStart,Y    ; byte1
    ...  STA Level_JctXLHStart,Y    ; byte2

LevelJct_BigQuestionBlock:          ; prg026 - reads one back
    LDX <Player_XHi
    LDA Level_JctXLHStart,X         ; indexed by the player's current screen
```

So the pass is unaffected. Writing an arrival touches only bytes 1-2 of the
host's existing junction — the slot stays put because the pipe has not moved.
Writing a return copies the host's own vanilla return bytes, a spot already
proven safe in that host, into the drawn room's slot. Whichever room a host
draws, its players come back exactly where they always did.

## Design: seed the slots instead of finding the junctions (2026-08-27)

Everything hard about this feature came from one decision — that the arrival and
return positions are read out of `Level_JctXLHStart` / `Level_JctYLHStart`, whose
contents are written by parsing a layout command stream. That forced us to
locate a group-7 command inside a variable-size stream, per tileset, with no
byte-level signature distinguishing a Big [?] junction from any other. Two
sessions of searching produced a candidate table with one verified row.

**We do not have to read from there.** The slot arrays are plain RAM. If we
write the bytes we want into the slot the engine is about to read, the engine
does the rest — no layout command is located, parsed or edited.

### Why level identity is a sufficient key

Inside `LevelJct_BigQuestionBlock` we already know the junction is a Big [?] one,
because that is the routine we are in. A level has at most one Big [?] pipe. So
the level alone identifies the pipe — the screen it stands on is not needed, and
neither is its junction command. `qol::big_q` already resolves level identity
here; this design widens what that lookup returns.

### The two hooks

Both sit on code only a Big [?] junction reaches, so no shared path changes.

**Entry** — inside the existing lookup routine, on a table match, before it
returns the area index in `Y`. At that moment `X` is the matched row:

```asm
    LDA BQ_ARRIVE_Y,X     ; byte1 of the arrival (Y index | pipe dir)
    STA $7EB6             ; scratch (no zero page is free - see memory)
    LDA BQ_ARRIVE_X,X     ; byte2 (col << 4 | screen)
    LDY <Player_XHi       ; the slot the BQ entry path will read
    STA Level_JctXLHStart,Y
    LDA $7EB6
    STA Level_JctYLHStart,Y
    ; ... existing: LDA BQ_ROOM,X / TAY / SEC / RTS
```

The BQ entry path then reads those two slots exactly as it always did, and every
line of arithmetic after it is untouched.

**Exit** — at `PRG026_AA5A`, after the pointer restores and before the
`JMP PRG026_AA8A`. `Level_ObjPtrOrig_AddrL/H` has just been restored to the
host's pointer, so it is the same key pass 1 already scans:

```asm
    ; copy Level_ObjPtrOrig -> $7EB2/$7EB3, JSR bq_scan
    ; on match, X = row:
    LDA BQ_RETURN_Y,X
    STA $7EB6
    LDA BQ_RETURN_X,X
    LDY <Player_XHi       ; the room screen - bonus areas are horizontal
    STA Level_JctXLHStart,Y
    LDA $7EB6
    STA Level_JctYLHStart,Y
```

`bq_scan` is reused as-is; it already takes its key from `$7EB2/$7EB3`.

### What this removes

- **Locating the 11 host junction commands.** Not needed at all. The one open
  blocker disappears.
- **Spare `Coin` commands in Unused Level 5.** No return junctions are written
  into it, so its byte-tight layout is never touched.
- **The one-host-per-room cap.** The return is keyed by host, not by the room's
  single slot, so two hosts may share a room screen.
- **The frozen-screen problem.** The arrival is ours, so the room is a free
  choice per host rather than whatever vanilla's byte2 said.

Net effect: 19 rooms, 11 hosts, unconstrained draw. Only the per-world
`BigQBlock_GotIt` screen rule survives, and that one was never binding.

### Cost

Two 13-byte tables per leg (arrival byte1/byte2, return byte1/byte2) = 52 bytes,
plus roughly 20 bytes of seeding at each hook and the `bq_scan` key copy at the
exit — **an estimated ~100 bytes**, to be measured, not trusted. It lands in
PRG026 beside the existing routine (2603 free, largest gap 2537), which is also
the bank the routine already runs in. `BIG_Q_ROUTINE_LEN` grows from 106 and the
`FS_BIG_Q_LOOKUP` allocation must grow with it; the routine derives its internal
absolute operands from its own base, so it is not origin-locked and can move.

### Risks and open checks

- **Vertical hosts.** `PRG026_AA9A` indexes vertical levels through
  `LevelJct_GetVScreenH` rather than `Player_XHi`, while the Big [?] *entry* path
  uses `Player_XHi` unconditionally. Entry is therefore safe, but the exit seed
  writes to `Player_XHi` and the read may use a different index if a bonus area
  is ever vertical. All eight vanilla areas and Unused Level 5 are horizontal,
  so this holds today — it needs a guard or a comment, not a fix.
- **`Level_7Vertical` on the return.** The arrival byte1's bit 7 sets vertical
  mode. Seeding a return for a vertical host has to reproduce whatever the
  vanilla return command carried, which the part-1 table already records.
- **Two hosts, one room.** Now allowed, but both then share one
  `BigQBlock_GotIt` bit if they are in the same world. The per-world screen rule
  still applies.
- The `asm::check` in the same commit, per CLAUDE.md, with `.allocation` and
  `.origin`.

### Why this generalises

Lobby shuffle rewrites junction commands in place, which is why it needed a
hand-maintained table of offsets and why 5-2's slot-4 command had to be excluded
by hand. This design overrides the *read* instead of editing the *data*. For the
further junction shuffling planned later, the override pattern never inherits
the layout-parsing problem — the cost is one hook per junction handler, and the
handlers are already separate routines (`LevelJct_General`,
`LevelJct_GenericExit`, `LevelJct_SpecialToadHouse`,
`LevelJct_BigQuestionBlock`).

The catch to keep in view: `LevelJct_General` is shared by every ordinary
junction, so a general shuffle cannot seed slots from there without a key that
distinguishes one pipe from another — and at that point `Player_XHi` is all the
engine knows. Big [?] is easy precisely because its handler is exclusive.

## What shipped, against the design

Built as designed: two hooks, both on Big [?]-only paths, seeding
`Level_JctXLHStart` / `Level_JctYLHStart` from tables in the lookup routine. No
layout command is located or edited, so neither open blocker — host junction
offsets, spare `Coin` commands in Unused Level 5 — had to be resolved.

Measured, not estimated: the routine is **207 bytes in a 224-byte allocation**,
against the ~100 the design guessed. The guess forgot that the seeding is
duplicated for entry and exit and that four 13-byte payload tables cost 52 on
their own. PRG026's largest gap starts exactly where `FS_BIG_Q_LOOKUP` ends, so
it grew in place; the bank is down to 2485 free / 2419 largest.

Two things resolved along the way that the earlier sections got wrong:

- **The host-to-room pairing is settled**, and the pairing table in "Harvest,
  part 2" is now load-bearing for the *return* bytes (a wrong pairing would send
  a host's players back into a different level). The discriminator that finished
  it: 7-F1's only in-area candidate carries **pipe-exit dir 8**, which is a front
  door, not a bonus pipe. Excluding dir-8 leaves 7-8 on s4, so 7-F1 takes s6 —
  agreeing with `W7F1_TANOOKI_OFFSET` and with 6-9 taking BigQ6 s3 by
  elimination. All 11 rooms come out distinct.
- **Vertical hosts index differently.** `PRG026_AA9A` sends vertical levels
  through `LevelJct_GetVScreenH` instead of `Player_XHi`, while the Big [?]
  *entry* path uses `Player_XHi` unconditionally. Entry is therefore safe. The
  exit seed writes at `Player_XHi` and is correct as long as bonus areas stay
  horizontal, which all eight vanilla ones and Unused Level 5 are.

## Unused Level 5 screens 6 and 7 are held back

Playtested 2026-08-27 and **both dropped the player through the floor** — he
appears in the wall and falls to his death. They are the only two rooms with no
ceiling pipe, so their arrivals were aimed "beside the floor pipe" at Y index 6,
row 23. That is the pipe body's row, not ground:

```
--- screen 6 ---            --- screen 7 ---
0F 61 E4 04  row 15 col 1   0F 71 E1 07  row 15 col 1 len 7
37 61 B1     row 23 col 1   17 76 E1 09  row 23 col 6 len 9
             ^ the pipe     37 72 B1     row 23 col 2  ^ the pipe
```

The arrivals put him at col 3 (s6) and col 5 (s7), both at row 23. Screen 6 has
nothing at row 23 except the pipe itself; screen 7's row-23 run starts at col 6.
They are also the only two rooms using Y index 6 — every room that works uses
0, 4 or 5 — which was the signal to treat them as suspect before shipping.

**Re-aiming them needs the room's tile grid, not its command list.** The layout
commands say where a generator starts, not which tiles end up solid, and
`level_sim.py` only knows tileset 1. Until something can answer "is (col, row)
solid in a fortress-tileset room", these two stay out and the pool is 17.

## Playtest ROMs

Five seeds through the real pipeline, each parking 5-2 on W1 tile 1 and 7-F1 on
the next tile with a P-Wing, at
`~/Copyparty/MiSTer/games/NES/_rando/bigq_seed<N>.nes`. Between them they cover
**all six** Unused Level 5 rooms still in the pool, plus two 7-F1 forces:

| Seed | 5-2 opens | 7-F1 opens | 7-F1's block |
|---|---|---|---|
| 10 | Unused Level 5 s2 | Unused Level 5 s0 | already Tanooki |
| 4 | Unused Level 5 s3 | Unused Level 5 s4 | already Tanooki |
| 25 | Unused Level 5 s1 | BigQ6 s6 | Hammer → forced Tanooki |
| 26 | Unused Level 5 s5 | Unused Level 5 s3 | already Tanooki |
| 24 | Unused Level 5 s2 | BigQ6 s3 | 3-Up → forced Tanooki |

Rebuild with:

```sh
testrom --randomize --seed <N> --place 5-2 7F1 --no-walk --starting-items p-wing
```

`--no-walk` is required on a randomized base: the open-movement patch and the
overworld writer want the same free space. Delete the target on the mount first,
it never overwrites.

**These seed numbers are specific to `testrom`'s option set.** Its `Options`
differ from the CLI's defaults, which moves the shared RNG, so the same seed
picks different rooms under `smb3-rs`. Read the room out of the ROM — the four
payload tables start at `FS_BIG_Q_LOOKUP + 0x8E` — rather than assuming.

Parking both levels on World 1 puts both rooms under **one** world's
`BigQBlock_GotIt` mask, which the real seed would not necessarily do, so these
five were picked to have distinct room screens. A colliding pair would show the
second block already collected.

To look at one Unused Level 5 room in isolation, without the randomizer:
`testrom --place 5-2 --bigq-unused5 <screen> --bigq-palette 4`. That knob still
reaches screens 6 and 7, which the randomizer no longer draws.
