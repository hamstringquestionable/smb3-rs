# Big [?] Bonus Room Shuffle — design notes

**Status: parked 2026-08-26**, branch `experiment/unused5-bigq-testrom`.
Playtested and working as a `testrom` knob; not started as a randomizer option.

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

## What is not built

Anything in the randomizer proper. No option, no flag-key bit, no web control.

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
- **One option, not three.** Room selection and per-area palette ride together;
  `big_q_blocks` keeps its current meaning (what is *in* the block) and its
  existing key bit.

## Why it is worth doing

Vanilla's 15 Big [?] rooms are only **10 distinct layouts** — BigQ2 s1 = BigQ8
s1 = BigQ8 s4, BigQ3 s4 = s5, BigQ4 s2 = BigQ6 s5, and BigQ5 s7 = BigQ7 s6.
Unused Level 5's eight are all unique, taking the pool to 18 designs. Note the
last pair: 7-F1's room, the most-entered bonus room in the game, is already a
room you have seen in World 5.

## Plan

Pool: 14 vanilla rooms + 8 from Unused Level 5 = 22 rooms for 11 host levels.

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

## Next step when this resumes

Harvest the 11 vanilla pairings — dump each host's junction bytes and each
room's return bytes and check they line up with the model. That table either
falls out cleanly or tells us early that something does not fit.

## Playtest ROMs

`~/Copyparty/MiSTer/games/NES/_rando/unused5_bigq_s0..s7.nes` (one per room,
palette 0) and `unused5_pal1/3/4.nes` (room 5 in the other fortress palettes).
Rebuild any of them with `testrom --place 5-2 --bigq-unused5 <screen>
--bigq-palette <index>`; delete the target on the mount first, it never
overwrites.
