Review a new or modified 6502 ROM patch for correctness and safety.

## Usage
`/review-patch [description of the patch or file being modified]`

## Instructions

Perform the following checks and report findings:

### 1. Free Space Audit

Most of this is mechanised — run the tools before reading anything by hand.

```sh
cargo test --lib free_space          # overlap, FS_* constants, audit, doc table
smb3-rs <rom> --free-space --fit N   # where a patch of N bytes could go
smb3-rs <rom> --free-space           # per-bank budget
```

`cargo test --lib free_space` covers, without you re-deriving any of it:

- no two allocations overlap (`test_free_space_no_overlap`)
- every `FS_*` constant is a registry row (`test_free_space_constants_match_registry`)
- **what the randomizer actually wrote** stayed inside its allocation and carried
  its owner's write-log tag, over a run with every feature on
  (`free_space_audit_matches_registry`)
- CLAUDE.md's per-bank table still matches the ROM (`free_space_doc_table_is_current`)

So for the patch under review, check only what the tests cannot:

- The module uses the `FS_*` constant, not a local hardcoded offset
  (`tools/offset_dups.py` catches the general case)
- The registry row's `owners` names the write-log tag this patch writes under.
  If the patch is emitted by a larger pass it needs its own `push_tag` — see
  `march_veto` / `fx_screen_check` in `overworld_writer/mod.rs` — or its bytes
  are attributed to the whole pass and the audit cannot tell them apart
- The reserved size leaves sensible headroom, and any `// N reserved, M used`
  comment matches. `--write-log` prints measured usage per allocation. It counts
  bytes *covered by a logged write*, which is exact for a routine emitted as one
  `write_range`; it under-counts only where a patch writes byte-at-a-time and
  some bytes already match what was there

The registry lives in `src/randomize/rom_data/free_space.rs`.

### 2. Claiming New Free Space

If the patch claims a region not already registered:

- `--free-space --fit N` lists the candidate gaps. **Candidates, not
  confirmations** — filler nothing has *claimed* may still be data something
  *reads*. Verify against the disassembly (`tools/southbird-smb3/PRG/prgNNN.asm`)
  before taking it, and record what you found in the registry comment, the way
  the PRG004 and PRG006 tails are
- The bank must be mapped when the code runs. That is the first filter, not the
  last: a 2537-byte gap in PRG026 is useless to a routine that runs in-level

### 3. Patch Site Review

- For hook sites (where existing code is overwritten with JMP/JSR):
  - Verify the original instruction size matches the replacement (e.g. a 3-byte
    `JSR` replaced with a 3-byte `JMP`)
  - Check that the hook doesn't clobber adjacent instructions
  - Verify the bank is correct (CPU address ↔ file offset mapping)
- For subroutines in free space:
  - **Derive operands with `jsr_into_bank(bank, file_offset)` /
    `prg_bank_cpu_to_file` rather than transcribing address bytes.** Those
    helpers are pinned by `test_prg_bank_mapping_known_pairs` against
    known-correct triples from shipped patches, so getting them right protects
    every hook — dropping the 0x10 iNES header was the issue #14 root cause
  - `prg_bank_file_to_cpu` assumes the `$A000` window and does **not** apply to
    banks mapped elsewhere. There is no parity rule: PRG004 (even) maps at
    `$A000` while PRG000 (even) maps at `$C000`; PRG011 (odd) at `$A000` while
    PRG029 (odd) at `$C000`. The window is a per-bank fact recorded in the
    comment above each allocation in `free_space.rs` — read it there
  - Fixed banks: PRG030 at `$8000–$9FFF`, PRG031 at `$E000–$FFFF`. Hence
    `WORLD_ORDER_CPU` is `$9F10` for file 0x3DF20

### 4. Known Danger Zones

Flag if the patch writes to any of these ranges:

- **0x19100–0x19DCF** (PRG012): active tile lookup + map screen code. Writing
  here crashes on level entry.
- **Bank 24 (PRG024)**: avoid JMP-to-free-space — shares code paths with 2P
  mode, causes switching bugs. Prefer inline patches.

### 5. Ordering Concerns

- If the patch touches pointer tables or airship entries, check ordering
  relative to autoscroll and overworld builder in `randomizer.rs`
- Autoscroll MUST run before overworld builder (writes to hardcoded vanilla
  offsets that get displaced by `resort_pointer_table`)

### 6. Size

Free space is the project's scarcest resource — see "ROM Free Space Is Scarce"
in CLAUDE.md. Review the *code* for size even when the allocation has room:
folding constant math into flags, picking the instruction that preserves the
register you still need, zero-page temps over stack juggling.

### 7. Summary

Report: pass/fail for each check, any warnings, and suggested fixes.
