# ROM Write Log — current state and planned enhancements

Status: design note. Nothing here is implemented yet.

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

### 1. `apply_ips_patch` bypasses the log

`Rom::apply_ips_patch` writes with a direct slice copy
(`self.data[HEADER_SIZE..].copy_from_slice(&patched)`), so none of its bytes are
recorded.

Consequences:

- Visual-patch bytes (Luigi, Peach, Dr. Mario, …) never appear in the log.
- `testrom --apply-ips` inherits the blind spot: its collision guard compares
  incoming records against the log, so a *second* applied patch cannot be seen
  colliding with the first. Patch-vs-randomizer collisions are caught;
  patch-vs-patch are not.

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

### 3. No dynamic free-space verification

The existing tests are **static** — they check the registry against itself:

- `test_free_space_no_overlap` — allocations don't overlap each other
- `test_free_space_constants_match_registry` — `FS_*` constants match entries

Nothing checks what is *actually written*. Consequences:

- The `// N reserved, M used` comments on 41 allocations are hand-maintained and
  can silently go stale. `FS_FX_SCREEN_CHECK` read `112 reserved, 97 used` until
  it was corrected to `82` by hand in `1fe86fb`; others have not been audited.
- Nothing detects a patch overrunning its allocation into a neighbour's, or a
  module writing into free space it does not own.

---

## Enhancement 1 — free-space auditor (do this first)

Cross-reference `FREE_SPACE_ALLOCATIONS` against the write log after a real
randomization run.

**Assertions**

1. Every write whose offset falls inside a registered free-space region carries
   the tag of the module that owns that allocation.
2. For each allocation, the highest byte actually written is within
   `offset + size`. (Overrun = failing test.)
3. Report actual bytes used per allocation, so the `reserved / used` comments
   can be regenerated rather than remembered.

**Why first**

- It needs no call-site churn — a test plus a small helper.
- It mechanizes `/review-patch` sections 1–3, currently re-derived by hand for
  every 6502 patch.
- It would have caught the stale `97 used` automatically.
- It tells us whether tags are still lying anywhere else, which is the
  information needed to decide how urgent Enhancement 3 is.

**Prerequisite, already met:** accurate tags. Before `cd5e459` the overworld
writer's bytes were attributed to `troll_pipes`, which would have made
assertion 1 fail for reasons unrelated to free space.

**Gotchas**

- Flag-dependent code paths do not run under `Options::default()`. Anything
  gated off by default (e.g. `hammer_breaks_locks`, `troll_pipes` when off) will
  simply be absent from the log — the audit must not read "no writes" as
  "allocation unused". Run the audit over a few option sets, or assert only over
  allocations whose feature was enabled for that run. This is the same trap that
  made the overworld baseline sweep vacuous for `hammer_breaks`
  (see `tests/overworld_baseline.rs`).
- A patch may legitimately write fewer bytes than reserved; only *over* is an
  error. Reserved headroom is encouraged (see CLAUDE.md).

## Enhancement 2 — route `apply_ips_patch` through `write_range`

Small and self-contained. Volume is not a concern: the largest bundled patch is
the practice ROM at 193 records.

Two decisions to make when implementing:

- One record per IPS record (preferred — keeps offsets meaningful) rather than
  one giant record for the whole ROM.
- What tag to use. Probably a caller-supplied one, so `--apply-ips` can label
  the patch by filename and patch-vs-patch collisions become legible.

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
