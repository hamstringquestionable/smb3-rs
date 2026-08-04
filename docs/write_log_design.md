# ROM Write Log — current state and planned enhancements

Status: Enhancement 1 (free-space auditor) is implemented — see "Enhancement 1"
below for what it does and what it found. Enhancements 2 and 3 are still design
only.

## What exists today

`Rom` records every mutation that goes through its accessors:

```rust
pub struct WriteRecord {
    pub offset: usize,
    pub len: usize,
    pub old_bytes: Vec<u8>,
    pub new_bytes: Vec<u8>,
    pub tag: String,
}
```

- `write_byte` and `write_range` both push a record, **unconditionally** — not
  gated on `debug_assertions`. They skip the record when the write is a no-op
  (new bytes equal old), so the log holds only real changes.
- The tag comes from a stack: `set_tag` replaces it (used by the orchestrator
  before each pass), `push_tag`/`pop_tag` nest a sub-tag within a pass. There
  are 83 `set_tag` call sites and 8 `push_tag`.
- Consumers today: `main.rs`'s write-log dump and `find_collisions()`, and
  `testrom`'s patch-collision guard (`testrom::collisions`), which checks each
  incoming IPS record against `rom.writes_in_range`.

## The principle to hold

**The log is about bytes, provenance, and ownership. It does not carry
meaning.**

It should never grow fields like "placed a vertical lock at W8 (6,40)". A
semantic description of a seed belongs in the seed report / spoiler log (see
`docs/seed_report_design.md`), derived from the finished ROM. Encoding the same
fact in both places makes the log a second producer for something the ROM
already answers — the exact drift that `rom_data::tiles` was created to remove.

The two answer different questions:

| | Question | Shape |
|---|---|---|
| Write log | what did we change, and which pass did it | history |
| Seed report | what does the player get | final state |

## Known gaps

### 1. `apply_ips_patch` bypasses the log — **fixed for the `Rom` path**

`Rom::apply_ips_patch` used to write with a direct slice copy
(`self.data[HEADER_SIZE..].copy_from_slice(&patched)`), recording nothing. It now
decodes with `ips::parse_ips_records` and applies each record through
`write_range` under a caller-supplied tag, so patched bytes are logged, audited
and collision-checked like any other write.

Still outstanding: **the CLI does not use that method.** `--toad` and
`--sprite-patch` apply the free function to the raw bytes before a `Rom` exists
(`main.rs`), so those two remain invisible to the log — and they are exactly the
pair most likely to collide, since both replace player graphics. Closing it means
routing them through `randomize_rom`, whose `visual_patch` parameter takes one
patch where the CLI layers two, while keeping `pristine_input` intact so the
emitted IPS still contains the visual bytes.

### 2. Tags leak between passes

`set_tag` replaces the stack and persists until the next call, so a pass that
sets a tag but writes nothing leaves its name attached to whatever writes next.

Real instance, fixed in `cd5e459`: `randomize::troll_pipes::mark_troll_pipes`
mutates the `BuildResult`, not the ROM. Its `set_tag("troll_pipes")` therefore
labelled nothing of its own and leaked onto every byte the overworld writer
subsequently emitted. The collision guard reported six overlaps against
`troll_pipes` when they were really `fx_screen_check`'s, which sent an
investigation after an optional feature instead of the routine under test.

The fix applied was local (drop the stray tag, tag the writer, sub-tag the FX
patch). The structural problem remains: correctness depends on every one of 83
call sites being disciplined.

### 3. No dynamic free-space verification — **fixed, see Enhancement 1**

The tests used to be **static** only — they checked the registry against itself:

- `test_free_space_no_overlap` — allocations don't overlap each other
- `test_free_space_constants_match_registry` — `FS_*` constants match entries

Nothing checked what was *actually written*, so the `// N reserved, M used`
comments could go stale (`FS_FX_SCREEN_CHECK` read `112 reserved, 97 used` until
it was corrected to `82` by hand in `1fe86fb`), and nothing would have caught a
patch overrunning its allocation or a module writing into space it does not own.

---

## Enhancement 1 — free-space auditor (implemented)

Cross-references `FREE_SPACE_ALLOCATIONS` against the write log after a real
randomization run.

**Where it lives**

- `rom_data::free_space` — `FreeSpaceAlloc` (the registry row, now carrying an
  `owners` list of write-log tags), `audit_free_space`, `free_space_map`,
  `format_free_space_report`.
- `randomizer::tests::free_space_audit_matches_registry` — the CI check.
- `--write-log` appends the report, so the file that says what changed also says
  what it cost and what is left.

**What it asserts**

1. Every write inside a registered region carries a tag one of that region's
   `owners` covers. Owners match whole `/`-separated tag components, so
   `fx_screen_check` covers `overworld_writer/fx_screen_check` and
   `big_q_blocks` covers both `qol/` and `enemies/` variants.
2. No write crosses a region boundary, in either direction.
3. Every allocation is exercised by the audit run. Without this the check is
   vacuous for anything flag-gated: a patch that never ran writes nothing, and
   an audit over nothing passes. `audit_options()` is `all_on_options()` with
   `world_count: 7` and `swap_start_airship: true`; a new gated allocation has
   to be added there or the test says so by name.

It also reports bytes used per allocation, which is where `// N reserved, M
used` comments should now come from.

**What it found on the first run**

- `march_veto`'s 107 bytes were tagged `overworld_writer`. Not a lie — the
  writer really does emit them — but too coarse to attribute, and the comment
  above `fx_screen_check` claimed to be "the one writer patch that claims free
  space", which had stopped being true. Both are now sub-tagged.
- The `hand_rooms` and `piranha_rooms` clone blocks are genuinely co-owned:
  the cloning module builds the enemy stream, then `items` rewrites the
  `OBJ_TREASURESET` byte inside it via the offsets those modules export. That is
  why `owners` is a list — a second entry documents intended sharing.

**Limits, by construction**

- `used` is *bytes covered by a logged write* — what the patch occupies, not
  the narrower "bytes differing from vanilla". `write_range` logs the whole
  range whenever any byte in it differs, so a routine emitted as one range is
  measured exactly even where its bytes match the filler underneath. Only a
  patch built byte-at-a-time with `write_byte` under-counts, since no-op bytes
  are skipped there. Overrun detection is unaffected either way.
- A patch may legitimately write fewer bytes than reserved; only *over* is an
  error. Reserved headroom is encouraged (see CLAUDE.md).

**Companion: per-bank budget.** `free_space_map` scans the *vanilla* PRG for
filler runs (≥ 8 bytes) no allocation has claimed, per bank, and reports free
`$FF`, the largest single gap, and `$00` runs separately — zeroed data looks
exactly like zero padding, so that column is a candidate list, not space.

Unlike the per-allocation audit this depends on nothing but the vanilla bytes
and the registry: same answer for every seed and option set, no randomization
run needed. `smb3-rs <rom> --free-space` prints it on its own.

**Which number to decide on.** Three numbers come out of this work and only one
of them is an input to a decision:

| Number | Caveats | Use |
|---|---|---|
| bytes used per allocation | flag-dependent; under-counts byte-at-a-time patches | audit output — catches drift, never sites a patch |
| free bytes per bank | ≥ 8 threshold, `$00` ambiguity, unclaimed ≠ unreferenced | "is this bank roomy", nothing finer |
| **gap of N contiguous bytes** | **none that bear on the answer** | **where a patch goes** |

A threshold can only hide gaps too small to use, and `$00` ambiguity never
arises if you claim `$FF` runs — so the question `--free-space --fit N` answers
is clean, while the aggregate it is derived from is not. The remaining check
(is this gap *referenced*?) is once per gap, and recording it as a registry row
retires it permanently. That is why the caveats are not a recurring tax: what
recurs is arithmetic, and `free_space_doc_table_is_current` does that for you.

CLAUDE.md's per-bank table is that output, and
`free_space_doc_table_is_current` fails when a row drifts. Reconciling the two
found: the doc's older figures came from a ≥ 16-byte scan (which is the whole
of the PRG031 68→81 and PRG010 880→896 difference), and its PRG000–007 row had
simply missed PRG004 and PRG006, whose bank tails hold 426 and 1392 bytes.
Both tails were checked against the disassembly before being counted — see the
note in CLAUDE.md. "Unclaimed filler" is not by itself "unreferenced"; the scan
finds candidates, the disassembly settles them.

## Enhancement 2 — route `apply_ips_patch` through `write_range` (implemented for the `Rom` path)

Both decisions went the expected way: one write record per IPS record, and a
caller-supplied tag. `testrom` passes the filename, so a collision report names
the patch; `randomize_rom` passes `visual_patch`.

Volume is not a problem — the bundled patches run 265–672 records each, and all
of them end at 0x05FD50, inside the 0x60000 payload.

Two things that came out of doing it:

- **The tag stack had to become `Vec<String>`.** It was `Vec<&'static str>`,
  which cannot hold a filename. `set_tag`/`push_tag` now take `&str`; all 91
  existing call sites pass literals and were untouched.
- **An out-of-range record is now an error, not a panic.** The old path grew a
  scratch buffer to fit the record and then panicked on the length mismatch when
  copying it back. Records are validated up front, so a patch that does not fit
  leaves the ROM untouched instead of half-applied.

**Measured effect on collision reports.** Under default options, visual-patch
bytes newly collide with `palettes` — 7 bytes for Peach, 1 for Dr. Mario, 0 for
Toad; with `palette_themed` on, 36 for Peach. The count varies run to run
because palettes draw on OS entropy rather than the seed. This is real signal:
the palette randomizer is overwriting colors the visual patch chose. Whether
that is wanted is a design question (recoloring the swapped sprite may well be
intended), not a defect in the logging.

## The randomizer-vs-randomizer collisions

A default run used to report ~178 of these. Investigated 2026-08-04: **142 were
an artifact of the report, and all 36 real ones are intended.**

The artifact: `write_range` logs the *whole* range whenever any byte in it
differs, so a pass that rewrites a block is credited with every byte in it,
including the ones it left exactly as it found them. `autoscroll -> powerups`
(131 bytes, the largest group by far) was entirely this — `powerups` "overwrote"
those bytes with the identical value autoscroll had put there. `find_collisions`
now reads `WriteRecord::changes()` instead of the covered range, which drops a
default seed from 178 to 36–39.

**Not fixed at the source, deliberately.** Narrowing `write_range` to log only
changed sub-runs would discard the covered/changed distinction, and two
consumers need *covered*: the free-space audit (a routine byte equal to the
`$FF` filler under it is still part of the routine, and must stay attributed to
its owner) and testrom's guard (an incoming patch overwriting such a byte would
genuinely break that routine). Both facts are recoverable from a record, so the
fix was to give the distinction a name — `changes()` / `changed_len()` — and
teach the one consumer that meant "overwrote" to use it.

The per-tag header in `--write-log` now shows both when they differ, which makes
the shape visible: `[powerups] 52968 bytes (156 changed), 9 writes`. Powerups
rewrites whole level-data regions to move 156 bytes, which is where essentially
all of the noise came from. Correct, just coarse — not worth a byte-identity
risk to narrow.

The 36 that remain, all explained:

| Bytes | Pair | Why |
|---|---|---|
| 19 | `autoscroll -> levels/airships` | autoscroll redirects each world's airship ObjSets/LevelLayouts pointers; the airship shuffle then permutes those *post-autoscroll* values between slots. The ordering CLAUDE.md requires, working |
| 13 | `overworld_writer -> items` | the Hammer Bro reward table at 0x16190: the writer places bros with default rewards, `items` randomizes the reward. Layered by design |
| 3 | `qol -> overworld_writer` | `gap_tile_for(0xB3) = 0x9D` (the documented bridge→water-gap), the FX slot-16 `replace_tile` handoff after pickup has consumed qol's value, and a level placed on the W1 shortcut stub — which `apply_w1_shortcut` explicitly leaves as a valid blank node |
| 1 | `hand_rooms -> items` | the co-ownership already recorded in `FREE_SPACE_ALLOCATIONS` |

No defects. Worth re-running after any pass is reordered, since three of the
four groups are ordering-dependent by construction.

**Known limit of `find_collisions`:** it compares only the *top-level* tag, so
passes within one family (`koopalings/y_clamp` vs `koopalings/random_hits`) are
mutually invisible to it. Fine for finding cross-module clobbering, useless for
intra-module — query full tags via `writes_in_range` for that.

## Enhancement 3 — make tags un-leakable

Replace `set_tag` + convention with an RAII scope guard:

```rust
let _t = rom.tag("qol/starting_items");   // restores the previous tag on drop
```

This makes leaking structurally impossible rather than a matter of discipline.

**Cost:** 83 `set_tag` call sites plus 8 `push_tag`. Mechanical but wide, and
touching every randomization pass. Worth doing *after* Enhancement 1, so the
audit can confirm whether any other tag is currently wrong — that determines
whether this is urgent or merely tidy.

**What the audit says about urgency:** the audit run found no *leaked* tag —
the one attribution problem (`march_veto` under `overworld_writer`) was a
correct tag that was merely too coarse, not a stale one bleeding across a pass.
That is weak evidence, though: the audit only sees the 36 free-space regions,
which is a small share of what a run writes. It lowers the urgency; it does not
clear the structural problem.

**Verification:** `tests/overworld_baseline.rs` covers it. Tagging is metadata
and must not move a single ROM byte, so all 20 seeds must stay byte-identical.
Note that harness excludes palettes (not seed-stable) and only exercises default
options, so pair it with the existing pinned-table tests for flag-gated passes.

---

## Explicitly out of scope

- Semantic content in the log (see "The principle to hold").
- Deriving the spoiler log from the write log. Evaluated and rejected: the log
  is a change stream requiring the same interpretation layer as reading the
  finished ROM, plus replay; it does not record decisions that produced no bytes
  (e.g. which level donated an antechamber); and it would make a player-facing
  feature depend on tag accuracy.
