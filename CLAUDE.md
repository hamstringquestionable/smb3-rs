# SMB3-RS — Super Mario Bros. 3 Randomizer

## Project Overview

A Rust utility that randomizes Super Mario Bros. 3 (USA Rev 1) and outputs an IPS patch or patched ROM. Compiles to both a native CLI binary and a WebAssembly module for a browser-based web app. The application never stores or bundles the ROM — users must provide their own.

## Build Commands

All builds require `nix-shell` (or the equivalent packages: gcc, rustup, wasm-pack, pkg-config, openssl). On NixOS, all commands must run inside `nix-shell` — bare `cargo`/`python3` are not on PATH:

```sh
nix-shell                                    # enter dev shell
cargo build                                  # native CLI binary -> target/debug/smb3-rs
cargo test                                   # run all tests
cargo build --release                        # optimized binary -> target/release/smb3-rs
wasm-pack build --target web --out-dir pkg   # WASM module -> pkg/
```

## Lint Policy

This project is **lint-clean**: `cargo clippy --all-targets` must produce zero warnings. CI (`.github/workflows/ci.yml`) enforces this with four gates, in this order — `cargo fmt --check`, then clippy on the native target, then clippy on wasm32, then rustdoc. Each converts a warning into a build failure.

Before committing:

```sh
cargo fmt --check            # the repo IS rustfmt-formatted; CI checks this FIRST
cargo clippy --all-targets   # must show no warnings
cargo clippy --lib --target wasm32-unknown-unknown   # CI's second pass
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items  # doc links
cargo test                   # must pass
```

**The Rust version is pinned in `rust-toolchain.toml`**, and rustup uses it
for every cargo command in this checkout, locally and in CI. Without the pin,
CI floated on the newest stable while local stayed wherever it was last
updated, so a new clippy lint could pass locally and fail CI. Bumping the
version is a deliberate change: edit `channel`, run the gates below, and fix
what they report in the same PR.

The wasm pass is not redundant: native clippy never evaluates code behind
`cfg(target_arch = "wasm32")`, so a warning that only exists there is invisible
until CI fails. Note also that `cargo fmt --check` runs *before* both clippy
passes — a formatting slip fails the build before a single lint is reported.

**If you touched overworld logic, `cargo test` alone is not enough.** The
builder's guarantees are statistical, and the suite runs them shallow on
purpose so CI stays fast. Re-run the two that matter at depth:

```sh
# The target must never be stranded — an unbeatable world. 8 arms
# (raw/qol x hammer-bro x SAS); CI runs 20 seeds/arm, this runs 500.
CENSUS_SEEDS=500 cargo test --release --lib all_world_targets_reachable   # ~4 min

# Route choice + C1 floor + rock paths, one build pass.
CENSUS_SEEDS=1000 cargo test --release --lib test_route_census -- --ignored --nocapture
```

A stranding bug that only shows on a rare pipe layout will pass at 20 seeds and
fail a player. The measured baselines these should be compared against are in
`docs/choice_first_charter.md`; `CENSUS_SEEDS` / `CENSUS_SLACK` drive every
census in `overworld::build`.

When clippy flags new code:

1. **Idiom lints** (`needless_range_loop`, `manual_clamp`, `useless_vec`, etc.): apply the suggested fix. Clippy's lint pages link to docs explaining the *why*.
2. **Judgment-call lints** (`too_many_arguments`, `type_complexity`): consider whether the suggested refactor reveals a real concept. If yes, do the refactor. If no, add `#[allow(clippy::<lint_name>)]` immediately above the item, prefixed with a `// Reason: ...` comment explaining the decision.

Never silence a lint by deleting the warning text or globally disabling — the goal is "every warning was considered," not "no warnings emitted."

**Give every item the narrowest visibility its users need**: private if only
its own module uses it, `pub(crate)` if other modules do, `pub` only for what
`tests/`, `main.rs`, the bins or `lib.rs`'s re-exports reach from outside the
crate. Prefer `pub(crate)` to `pub(super)` for anything another folder
uses: `pub(super)` is relative, so moving a file silently changes its
meaning. `unreachable_pub` (on in `lib.rs`) catches a `pub` item nothing
outside can reach, but not a `pub` item on a public path that nobody outside
uses, and not a `pub(crate)` that could be private, so new code still needs
the judgment. A `pub` item is also exempt from the dead-code lint, which is
how the 2026-10 narrowing pass found dead code hidden behind it.

**The one crate-wide exception is `rustdoc::private_intra_doc_links`**, allowed
in `lib.rs` with its reason. It guards published API docs, where private pages
do not exist; these docs are read with `--document-private-items`, where a link
to a `pub(crate)` item works. Every other rustdoc lint is fatal. The usual
rustdoc fixes: point a moved item's link at its new path; make a link to a
`#[cfg(test)]` item a plain code span (doc builds never compile it); wrap
bracketed prose like `W3[41]` or `<site>` in backticks; and keep `///` off a
`pub mod x;` line whose file has `//!` docs — rustdoc merges the two and
resolves the file's own links from the parent's scope. Use `//` there.

## Seeds Are Stable Within A Version, Never Across

A change may move generated output. **Byte identity is an instrument, not a
policy** — it answers "I intended to change nothing, prove it"
(`tests/rom_identity.rs`) and is the wrong gate for anything else. What an
intentional-output change owes instead is **census equivalence**: run
`test_route_census` before and after and account for every figure that moved
beyond noise, reading the per-world columns and not just the overall (W1 and W7
are the sensitive ones; a global mean hides a world going flat).

The policy, the current baseline, the three instruments and the rules for
recapturing one are in `docs/seed_stability.md`. Read it before arguing that a
change cannot land because it moves seeds.

## ROM Free Space Is Scarce — Optimize Every Patch for Size

**Treat bytes of ROM free space as the project's scarcest resource.** Every new
6502 patch must be written as small as it can be made, not merely small enough
to fit its current allocation. "It fits, ship it" is not the standard — a patch
that wastes 5 bytes has spent 5 bytes that a future feature will need.

Space is **always** a concern. Never argue from the local gap ("there are 600
free bytes right after this patch, so size doesn't matter here") — that
reasoning is wrong even when the gap is real, because the gap belongs to the
next feature, not to this one. Optimize the code every time.

This applies to the *code*, not to the *allocation*. Reserving headroom in an
allocation is encouraged: a routine that later needs a few more bytes is far
safer to extend in place than to relocate, and several allocations here are
origin-locked by self-referential absolute addresses. Write the tightest bytes
you can, then reserve a sensible margin around them and note both numbers
(e.g. `// 112 reserved, 97 used`).

**Ask "does a run of N bytes exist", never "how much is free".** Free space is
per-bank *and* per-gap: a routine needs one contiguous run in a bank that is
mapped when it runs, so a bank total is worthless on its own — PRG031's 81 free
bytes are scraps that will not hold a 40-byte routine. Ask the question directly:

```sh
smb3-rs <rom> --free-space --fit 60   # every unclaimed gap that holds 60 bytes
smb3-rs <rom> --free-space            # per-bank summary
```

Two filters the tool cannot apply for you, in this order: the bank must be
mapped when your code runs, and the gap must be *unreferenced*, not merely
unclaimed — unclaimed filler can still be data something reads, so check the
disassembly (`tools/southbird-smb3/PRG/prgNNN.asm`) before taking it. That check
is once per gap: record it as a `FREE_SPACE_ALLOCATIONS` row and it is settled
for good.

The per-bank summary, measured against the current allocations (see
`FREE_SPACE_ALLOCATIONS` in `rom_data/free_space.rs`) — the largest-gap column
is the one to read:

| Bank | Mapped at | Free left | Largest single gap |
|------|-----------|-----------|--------------------|
| PRG031 | `$E000–$FFFF`, always | 27 | **16** |
| PRG030 | `$8000–$9FFF`, always | 18 | 18 |
| PRG001 | swapped, in-level (object AI) | 54 | 38 |
| PRG003 | swapped, in-level (object AI) | 5 | 5 |
| PRG004 | swapped, in-level (object AI, group 3) | 426 | 426 |
| PRG005 | swapped, in-level (object AI) | 58 | 58 |
| PRG006 | `$C000–$DFFF`, in-level (enemy data) | 1392 | 1392 |
| PRG007 | swapped, in-level (object AI) | 27 | 27 |
| PRG010 | `$C000–$DFFF`, map | 80 | 64 |
| PRG011 | `$A000–$BFFF`, map | 46 | 14 |
| PRG025 | `$C000–$DFFF`, title screen | 2707 | 2695 |
| PRG012 | `$A000–$BFFF`, map reload | 620 | 240 |
| PRG026 | `$A000–$BFFF`, map/inventory | 2061 | 2043 |
| PRG027 | `$A000–$BFFF`, letter cutscene | 657 | 657 |
| PRG029 | `$C000–$DFFF`, Toad House | 2548 | 1528 |

PRG000 and PRG002 have no `$FF` filler left at all.

The always-mapped banks are effectively full, and PRG031's scraps are now
spent: `anchor_dedup` took its 30- and 22-byte gaps in 2026-09, on the
principle that a run too small for a real allocation should go to the routine
that fits it. PRG030's run went the same way in 2026-09: `player_worlds`' hand-over router
took 24 of its 42 bytes, because the turn hand-over is entered with the level's
banks and could not be reasoned about from anywhere else. A patch that must run
regardless of the current bank now has **one 18-byte gap in PRG030 and nothing
over 16 bytes in PRG031**, so past that a trampoline into a swapped bank is the
only option — and that costs bytes too. Guard what is left of the PRG030 run
accordingly, and check first whether the hook really needs it: the router does,
its sibling marker gate in PRG010 does not.

PRG029's 1528-byte run is mid-bank and has **not** been through the
unreferenced check; its 20-byte tail (`prg029.asm` ends "Rest of ROM bank was
empty" after `PRG029_DFEB`) has been, and is what `FS_ANCHOR_HOUSE` claims.

**PRG010's 64-byte "largest gap" is not free — it is audio.** The run at file
0x15B50 (CPU `$DB40`) sits *inside* a DPCM sample, with sample bytes either
side of it, and `$FF` in a DMC stream is a valid run of set bits rather than
filler. The scan cannot tell the two apart, so this bank is the clearest case
of why the unreferenced check is per-gap and not per-bank. PRG010's usable
space is the tail from 0x15DD0 to the bank end, which `prg010.asm` confirms:
its last `.byte` line before `DMC08_End` matches the sixteen ROM bytes at
0x15DC0 exactly. `FS_PAD_BOX_DONE` claims the head of it.

**Check where your hook actually runs before paying that rent.** A hook on the
world map does not need an always-mapped bank at all: `$84A0` maps PRG010 into
`$C000` and PRG011 into `$A000` for the whole map, so map-side code has hundreds
of bytes available instead of PRG030's 42. The world-maze telepad hook is there
for exactly this reason; the pipe-portal version it replaced had to sit in
PRG030 because at *level exit* the banks belong to the level, and it consumed
this bank's only usable run while it existed.

**Do not hand-edit these numbers — regenerate them.** `smb3-rs <rom>
--free-space` prints the whole per-bank budget without randomizing (the same
table ends every `--write-log` dump, after the per-allocation audit).
`free_space_doc_table_is_current` fails when a row here drifts and prints the
replacement rows, so adding an allocation forces this table to be updated in the
same commit. That test needs the ROM, so it skips where the ROM is absent — it
guards the machine the patch is written on, not CI.

The scan counts `$FF` runs of ≥ 8 bytes (the older PRG031=68 / PRG010=880
figures came from a ≥ 16 scan) and lists `$00` runs in a separate column, which
is a candidate list rather than space — zeroed data and zero padding look
identical.

The two largest gaps here have been through the unreferenced check already
(2026-08-04): `prg004.asm` ends with "Rest of ROM bank was empty" at `$BE56`,
and PRG006's last assembled data is the dead object stream `_DA60` (file
0x0DA70..0x0DA74), referenced from nowhere, with the furthest referenced enemy
stream ending at 0x0D9F6. PRG025's tail was checked the same way (2026-08-10):
`prg025.asm` ends with "Rest of ROM bank was empty" after the palette sequence
at `$D505`, so the whole run from 0x33529 to the bank end is filler. That bank
is mapped at `$C000` for the entire title screen (PRG030's title entry loads
page 24 into `$A000` and page 25 into `$C000`), which makes it the right home
for title-only code instead of the nearly-full always-mapped banks.

**The scan cannot see reclaimed vanilla code, so the table understates PRG010.**
The fortress-FX rework (2026-09-06) retired vanilla's `MO_DoFortressFX` and its
seven slot tables by repointing one word of the map-operation jump table,
freeing `$C7BD..$C9D5` — 537 contiguous bytes, the largest run in the map bank.
Those bytes were never `$FF`, so `--free-space` counts none of them. The
registry rows are the record instead: `FS_FORTRESS_FX` claims the first 496
(484 used) and `FS_ON_CLEAR` the next 16 (14 used) — the fixed trampoline into
the on-clear trigger table in PRG026 (`maze::on_clear`), which a "beat this
level, do that" feature adds a row to rather than editing map-bank code. The
next 16 (`FS_WORLD_DIGIT_MAP`) hold the hidden-worlds banner digit, and the
**last 9 (`$C9CD..$C9D5`) are unclaimed**. Those slack bytes are **not visible to the
per-bank table above**. The same is true of every allocation sited on retired vanilla
code — check `FREE_SPACE_ALLOCATIONS` alongside the scan, not instead of it.

**Repointing a jump-table vector is the cheapest way to reclaim a large run**
in this ROM: one word, and a whole subsystem's code *and* data become free at
once. It is worth asking, before writing a trampoline, whether the vanilla
routine you are working around is reached from exactly one vector.

**PRG030's largest run is reclaimed vanilla code, not `$FF`: the retired
2-player Vs Challenge.** `two_player_vs.rs` disabled it on every seed in
2026-09, which freed **301 bytes** at `$934C..$9478` (file 0x3D35C..0x3D489) and
**38** at `$88F4..$8919`. Both are **unclaimed** — the first feature that needs
always-mapped space adds its own `FS_*` row over them, the way `FS_FORTRESS_FX`
works in PRG010. The `--free-space` scan cannot see either run, so the table
above understates PRG030 by 339 bytes; read
`docs/smb3_rom_reference.md` → "The 2-Player Vs Challenge" before concluding
that an always-mapped patch will not fit.

### Size techniques that have actually paid off here

- **Fold constant math into flags.** `ASL` leaves the shifted-out bit in carry;
  `AND` + `ADC #$00` can then combine two extracted fields in 5 bytes where the
  arithmetic-first version took 11 (see `fortress_fx.rs`, saved 6 bytes).
- **Pick the instruction that preserves the register you still need.** `LDX abs`
  to test a flag keeps `A` live for a following `STA`; `LDA` would force a
  reload.
- **Stash in a zero-page temp, not on the stack.** `PHA`/`PLA` is 1 byte per
  half, but every exit path then needs its own discard — two exits and the
  zero-page version is already smaller *and* removes the stack-balance hazard.
- **Derive an index from one you already hold.** `TYA`/`ASL`/`TAY` (3) beats
  re-reading the source and shifting (5).
- **Reach a shared exit with a conditional branch.** When a flag is known (e.g.
  `A` is non-zero), `BNE common` is 2 bytes where a second `JMP` is 3.
- **Reuse the engine's own state and routines** instead of storing your own.
  Querying a flag the engine already maintains costs a few bytes; a parallel
  per-slot table costs bytes *and* can drift out of sync.

## Every New 6502 Patch Gets an `asm::check` Test

A patch here is a `&[u8]` no assembler ever checked. The classic failure is a
miscounted relative branch: it still "assembles", lands mid-instruction, the CPU
runs an operand as an opcode — and the ROM boots. **Add the check in the same
commit as the patch**, in an `asm_checks` module at the end of the file:

```rust
#[cfg(test)]
mod asm_checks {
    use super::*;
    use crate::randomize::rom_data::asm;

    #[test]
    fn my_routine_is_well_formed() {
        asm::check(&MY_ROUTINE)
            .allocation(FS_MY_ROUTINE)   // fits its row, stays in its bank
            .origin(MY_ROUTINE_CPU)      // absolute self-references resolve
            .assert_ok();
    }
}
```

It decodes the bytes with the `mos6502` crate's Ricoh 2A03 table (a
dev-dependency; it reaches neither the binary nor the WASM bundle) and checks
that every byte decodes, the routine ends in `RTS`/`RTI`/`JMP` rather than
running into the `$FF` filler after it, every relative branch lands on an
instruction boundary, the code fits its `FREE_SPACE_ALLOCATIONS` row without
crossing its bank, and absolute references back into the routine still resolve.

Six builder methods, each for a real shape:

| Method | Use when |
|---|---|
| `.allocation(FS_*)` | the routine claims free space (nearly always) |
| `.origin(cpu)` | the routine has absolute references to itself or its tables |
| `.data_from(n)` | the tail is a lookup table, not code |
| `.fragment()` | it is spliced in place over vanilla, or continues into a sibling write |
| `.hook(&vanilla, off, &bytes)` | it is reached from a hook — checks whole instructions are displaced, and that a `JSR`/`JMP` hook names the origin |
| `.zero_page(max, &[vars])` | the routine runs long enough for the NMI to land inside its loops — see below |

`.zero_page` is the one that came from a real, invisible failure. The NMI pushes
and pulls exactly `Temp_Var1`-`Temp_Var3` (`$00`-`$02`) each frame and leaves the
rest of the page to whoever was using it, so a long-running routine that parks
anything in `$03` or above has it destroyed mid-loop *on hardware only* — no
emulated-CPU test can see it, and three playtests failed on it while every test
passed. The `engine_vars` list is the escape hatch: a map-side routine that
legitimately reads `World_Map_Y` (`$75`) names it, and every other byte outside
the protected three stays a failure. Reads count as well as writes.

`.fragment()` narrows the check rather than switching it off; branches landing
inside the array are still verified. Prefer fixing the array over reaching for
an opt-out.

**`.origin` is the one to remember**, because relocating is the dangerous
operation. A relative branch says "12 bytes forward" and survives being moved;
an absolute `JMP $DEB7` has the address baked in and does not. Several
allocations are origin-locked this way (`FS_CANOE_SUMMON` says so in its own
comment), which is why reserving headroom beats relocating.

The check cannot see *meaning* — a well-formed routine can still compute the
wrong answer. Where a routine is a self-contained calculation with no calls out
to the engine, execute it too: `stomp_fairness` runs its routine on an emulated
CPU over all 65,536 inputs. Most patches read mid-level engine state and cannot
be tested that way; for those the ROM is the instrument (see `testrom`).

## Changelog

`CHANGELOG.md` (repo root, [Keep a Changelog](https://keepachangelog.com/) format) tracks notable changes. When a change is user-visible or notable (a new flag/option, a behavior change, a fixed bug players would notice), add a one-line entry under the `[Unreleased]` section in the right group (`Added` / `Changed` / `Fixed` / `Removed`) as part of the same change. Skip purely internal refactors, test-only changes, and tooling tweaks. At version-bump time, move the accumulated `[Unreleased]` entries into a new versioned section.

## Playtest ROMs: Use `testrom`, Never Hand-Patch

Playtesting needs ROMs the randomizer would never produce — a specific level on
tile 1, a map with every lock removed, a fortress reachable without clearing
three levels first. **Build those with the `testrom` binary, never with an
ad-hoc Python one-liner against raw offsets.**

```sh
cargo build --bin testrom
./target/debug/testrom --place 6F1 5F1 8B   # three levels on W1 tiles 1-3
./target/debug/testrom --randomize --seed 12345 --world 3
./target/debug/testrom --randomize --keep-locks --hammer-locks \
    --starting-items hammer,leaf,fire       # locks intact + a way to break them
./target/debug/testrom --list               # every placeable level name
```

**Write every playtest ROM to the repo's `roms/` folder** (`-o roms/<name>.nes`)
— not `/tmp`, not the scratchpad, not `~/Copyparty`. `roms/` is where the user
looks, and `.gitignore` already excludes `*.nes`, so nothing there is committed.

### `--require`: search seeds for the feature under test

A random seed often just doesn't contain the thing you need to test. `--require`
searches for one that does:

```sh
./target/debug/testrom --require 'lock@w8:s2' --keep-locks --world 8
./target/debug/testrom --require 'fort@w3>=2'
./target/debug/testrom --require 'tile:0x54@w8'
```

Syntax is `<class>@w<N>[:s<M>][>=<K>]`; classes are `lock`, `gap`, `fortress`,
`level`, `pipe`, `toadhouse`, `airship`, `bowser`, or `tile:0xNN`. Seeds ascend
from `--seed-from` (default 1) so a given predicate always yields the same map;
bump it for a different one. `--search` caps the attempt count (default 500,
about 60ms per seed).

This is **rejection sampling on purpose**. Biasing the builder to *place* the
feature would produce a map the randomizer never generates — worthless for
verifying a fix. Searching keeps the real distribution and just skips seeds that
don't exercise the thing under test.

A predicate can pass while the ROM still fails to test what you meant — a lock
with no fortress beside it is broken by a hammer, which is a different code path
from a fortress clear. So the search **prints what it matched** plus a census of
the target screen. Check that output; don't just trust the exit code.

The knobs are deliberately orthogonal — base ROM (vanilla vs `--randomize`),
what to place, starting world, and how open the map is are four independent
axes, so level / overworld / airship / lock testing are combinations rather
than named modes. The map is fully open by default (locks removed, gaps
bridged, open movement patched in); `--keep-locks`, `--keep-gaps` and
`--no-walk` opt out individually.

Level names resolve through `NodeCatalog`, which already names every one of the
340 pointer table entries (`6F1`, `8B`, `7A`, `8-Tank`, `1-4`, plus the beta
stages under `--beta`). Matching is case-insensitive and dashes are optional.

**Why the rule:** the offsets a hand-written patch needs — map grids, pointer
tables, the starting-world byte — all already exist as constants in
`rom_data.rs`. Copying them into a throwaway script duplicates the single source
of truth (exactly what `tools/offset_dups.py` exists to catch) and re-derives
error-prone arithmetic each time. Map grids in particular are stored
**screen-major** (144 bytes per screen), so the obvious `base + row * columns +
col` is wrong; use `rom_data::map_tile_offset`. If `testrom` can't express what
a test needs, add the flag — don't work around it.

## Architecture: Separate Randomization from ROM Writes

Randomization modules follow a **decide then write** pattern. Each feature area has two layers:

1. **Randomization modules** (`overworld/build/`, `overworld/airship_shuffle.rs`, etc.) — contain the algorithms that decide *what* to change (BFS placement, shuffle logic, constraint solving). They consume RNG and produce descriptions of changes (new positions, new assignments, etc.).

2. **Helper modules** (`overworld/pipe_helpers.rs`, `overworld/helpers.rs`, `overworld/level_helpers.rs`) — contain the mechanical ROM write operations that execute those decisions. These are pure functions that take explicit inputs (positions, indices, tile values) and write to the ROM. They have no randomization logic or decision-making.

**Why this matters:** Multiple randomization modules may need to perform the same ROM operations (e.g., swapping pointer table entries, updating pipe destination tables, re-sorting the pointer table). Centralizing these writes in helper modules avoids duplication and ensures consistent behavior. When adding new randomization features, check the helper modules first — the write operation you need may already exist.

**Current helpers** (all in `randomize/overworld/`):
- `pipe_helpers.rs` — entry position swaps, pipe destination table writes, pointer table re-sorting
- `helpers.rs` — lockable tiles, FX patterns, gap tiles, target finding
- `level_helpers.rs` — shared `shuffle_entries()` for level entry shuffling

## Project Structure

```
src/
  lib.rs               # Public API: generate_patch(), generate_patched_rom()
  main.rs              # CLI (clap): file I/O, arg parsing, --free-space, --write-log
  rom.rs               # iNES header parsing, ROM validation, Rom struct
  ips.rs               # IPS patch builder (build_ips_patch) and applier (apply_ips_patch)
  pipeline/            # Orchestration: vanilla ROM in, randomized ROM out
    mod.rs             #   randomize_inner(): the table of contents — 19 stage
                       #   calls, in order. Start here to find where anything runs.
    stages.rs          #   each stage's body; calls the randomize modules
    options.rs         #   the Options struct — one field per player-facing choice
    flag_key.rs        #   Options <-> the shareable flag key
  testrom.rs           # Playtest ROM builder (native-only) — see below
  bin/testrom.rs       # `testrom` CLI: thin clap wrapper over testrom.rs
  rominfo.rs           # Read-only ROM lookups (native-only): the rom_map.py replacement
  bin/rominfo.rs       # `rominfo` CLI: thin clap wrapper over rominfo.rs
  bin/flagstats.rs     # `flagstats`: decode the flag keys GoatCounter recorded into
                       #   a per-option census (needs GOATCOUNTER_TOKEN)
  wasm.rs              # wasm-bindgen glue (only compiled for wasm32)
  randomize/           # Everything the randomizer can change, BY SUBJECT. The
                       #   pipeline decides the order; this tree is where code
                       #   lives. Each folder's mod.rs lists and groups its modules.
    mod.rs             # The index of the folders below
    rom_data/          # Shared ROM constants and read helpers, split by concern:
                       #   tables.rs (offset tables), free_space.rs (the allocation
                       #   registry), asm.rs (the 6502 patch checker), access.rs
                       #   (bank<->file mapping), grid.rs, tiles.rs, engine.rs,
                       #   fingerprint.rs (the overworld baseline hash),
                       #   level_index/ (every level area traced from its pointers:
                       #   headers, commands, powerup blocks, objects, names)
    overworld/         # The world maps
      map_source/      #   Where each world's map comes from: WorldMap (grid,
                       #     positions, sprites, gaps). vanilla/ reads the ROM and
                       #     applies the map edits (rocks, W1 shortcut, W8 bridges,
                       #     drawbridge tiles) in memory. See
                       #     docs/procedural_maps_design.md
      node_catalog/    #   Builder phase 1: classify all 340 pointer table entries
      pickup.rs        #   Phase 2: clear map, build level/HB pools
      build/           #   Phase 3: choice-first builder. Per world:
                       #     connectivity → levels → forts → locks → shaping → spare
                       #     pipes, plus route_choice.rs (the cost model) and the
                       #     censuses in builder_tests.rs
      writer/          #   Phase 4: write assignments to ROM (pointer tables, FX,
                       #     map tiles, sprites, march veto)
      lock_keys.rs     #   Every lock a fortress opens, keyed by position
      world_order.rs / airship_shuffle.rs / map_walker.rs / troll_pipes.rs / ...
      helpers.rs / pipe_helpers.rs / level_helpers.rs  # ROM write helpers, no RNG
    maze/              # The world maze (see docs/world_maze_design.md). mod.rs
                       #   groups it: the generator (graph, fill, roles, walk...),
                       #   the ROM side (completion_bits, world_persist,
                       #   world_travel, map_objects, maze_state, wand_gate...),
                       #   item gates (item_keys, key_sites, canoe_gate...), hints
    levels/            # Inside a level: antechambers, big_q_rooms, hand/piranha
                       #   rooms, composed sub-areas, autoscroll, segment_writer
    enemies/           # Enemy swaps within class, wild injection, protections.rs,
                       #   plus koopalings.rs, stomp_fairness.rs, water_stomp.rs
    items/             # mod.rs = chest/Toad House/Hammer Bro/letter tables;
                       #   powerups.rs (? blocks), fire_flower, poison_mushroom
    cosmetic/          # palettes + palette_variants, title_screen (seed hash),
                       #   credits, king_quotes, anchor_visuals
    qol/               # Quality-of-life patches, grouped by subject rather than
                       #   one file per patch: overworld_map.rs holds the map
                       #   patches that are not tiles (drawbridge toggle, W8
                       #   canoe sprite, N-cards); starting_state.rs holds lives AND
                       #   starting items. Also canoe.rs, cards.rs, macobra.rs,
                       #   two_player_vs.rs, ...
web/
  index.html           # Browser frontend
  style.css
  app.js               # Loads WASM, handles file input, triggers download
  options.js           # Option schema (labels, tips, defaults, icon specs)
  chr.js               # CHR decoder — every icon in the app is drawn from the
                       #   player's own ROM, so there are no bundled sprite
                       #   sheets. Pixel art only ever scales by a whole number.
  chr-picker.html      # Icon picker. Lays CHR out as 8x16 pairs, which is the
                       #   only way sprite art is legible (a flat tile grid
                       #   interleaves each sprite's halves). Emits icon specs
                       #   for options.js, or ICON_TILES rows in title-hash mode.
  chr-viewer.html      # Plain CHR browser for the player's ROM
  visual-patches/      # Bundled optional IPS patches (they claim free space too)
tools/
  README.md            # INDEX OF ALL 14 TOOLS — read this before writing a throwaway script
  rom_map.py           # ROM map generator + diagnostic modes (see below)
  rom_map.json         # Pre-built ROM map (gitignored, regenerate with rom_map.py)
  map_viz.py           # Renders any ROM's world maps as labelled ASCII (use this,
                       #   don't hand-decode tile grids)
  map_walker.py        # BFS map connectivity + fortress progression
  fx_check.py          # Cross-checks FX slots against actual map tiles
  level_sim.py         # Level tile simulator for debugging individual levels
  offset_dups.py       # Flags ROM offsets that bypass their rom_data constant
  ... 9 more           # See tools/README.md
docs/                  # See docs/README.md for the index and each doc's status
  smb3_rom_reference.md # ROM hacking reference (offsets, data structures, RAM map)
  vision.md            # What the project is for, and what it refuses to be
  choice_first_charter.md # The overworld builder's design authority
  world_maze_design.md # The world maze
  (the pipeline order lives in src/pipeline/mod.rs, not in a doc)
```

## Overworld Builder Pipeline

The overworld builder is the core randomization system, implemented as a four-phase pipeline in `pipeline/stages.rs` (stages 5-9): **catalog → pickup → build → write**, on the maps `overworld/map_source/` supplies.

1. **Catalog** (`overworld/node_catalog/`) — classifies all 340 pointer table entries across 8 worlds (Level, Fortress, Pipe, HammerBro, ToadHouse, Airship, Bowser, etc.)
2. **Pickup** (`overworld/pickup.rs`) — clears the map to blank path tiles, builds a shuffleable pool of levels and hammer bro encounters, applies theme-aware blank tiles per screen
3. **Build** (`overworld/build/`) — the choice-first builder: per world, knob-free uniform placement phases (connectivity pipes bridge islands → levels → forts → locks) followed by a diagnosis-driven shaping loop (lock re-place, gated shortcut, fort+lock move, level move, pipe move) that guarantees a minimum cheapest-route cost (`C1_FLOOR`) and seeks ≥2 routes in the choice band; a world finishing below the floor redeals its pipe web. Cross-world passes handle secret-exit safety, hammer-bro fill, toad-house/spade promotion. Hard invariants: order-free completability fixpoint, row 7/8 completion-bit rule
4. **Write** (`overworld/writer/`) — single-pass ROM write: updates pointer tables, FX table, pipe destination tables, map tiles, and hammer bro sprite assignments

The airship shuffle (`overworld/airship_shuffle.rs`) runs before the catalog, at stage 3, and is not bypassed: the builder carries each airship entry's shuffled data along when it re-sorts the pointer table.

## Tooling

**The Python tools are being retired. Rust is the only source of truth for
what is in the ROM.** A Python script and the Rust code describing the same
bytes is two answers that can drift, and they have: `rom_map.py`'s region scan
missed 5-7's sub-area for years, and its copies of Rust tables went stale. The
rules, until `tools/` holds only the palette and preview generators:

- **Answer new ROM questions in Rust**, as a `rominfo` lookup or a test, never
  as a new `tools/*.py` script or a new mode on an old one.
- **Port a Python mode by replacing it**: the PR that ships the Rust version
  deletes the Python one, after a one-time comparison of the two outputs whose
  disagreements are reported, not silently resolved.
- **Until a mode is ported, Rust still wins**: when a script disagrees with
  Rust, suspect the script.

`tools/README.md` tracks what is left, with the Rust replacement for each
script. `rom_map.py --level` was the first to go (2026-11): `rominfo --level`
and the `/level` skill replace it.

`rominfo` (`src/rominfo.rs`, `src/bin/rominfo.rs`) is the Rust home for
lookups. It reads through the same model the randomizer uses
(`rom_data::level_index`), so a lookup and a randomizer module cannot disagree
about where a level's bytes are:

```sh
cargo run --release --bin rominfo -- --level 6F1   # areas, headers, blocks, objects
cargo run --release --bin rominfo -- --list        # every name --level accepts
cargo run --release --bin rominfo -- --rom roms/out.nes --level 1-1   # any ROM
```

Two other answers live in the main CLI, because they read the allocation
registry: `--free-space [--fit N]` for where a patch can go, and `--write-log`
for what a run changed, which module owns each byte, and whether any patch
overran its allocation. Don't write a script to scan for free space.

Until they are ported, `tools/README.md` still indexes the scripts with a
verified status for each; read it before scanning the ROM by hand. In
particular `map_viz.py <rom.nes> --world N` renders any ROM's map as labelled
ASCII; don't hand-decode tile grids.

**Keep `tools/` from re-accumulating.** 15 one-off investigation scripts were
deleted on 2026-08-03 once their findings were captured in the Rust source and
`docs/smb3_rom_reference.md`. A script written to answer a single question
should end with its answer folded into the docs and the script deleted — not
left behind to rot into a trap for the next reader. Recover any of them from git
history if a line of investigation reopens.

## Level Contents: `rom_data::level_index`

**Find a level's bytes through `rom_data::level_index`, never by scanning a
level-data region.** The index reaches every header the way the engine does —
from a world pointer table entry, then each header's alt pointer (bytes 0-3,
tileset in byte 6) — and decodes each area's commands, powerup blocks and
object list with their file offsets. A scan from a region's start finds ~380
"headers" that are filler, zeroes or misaligned commands, and cannot tell when
it has slipped.

It is read-only and cheap (a few ms), and the ROM changes as the pipeline runs,
so build it from the ROM you are about to edit rather than reusing one. The
vanilla quirks it has to respect are pinned by its tests (`level_index/tests.rs`):
3HB18 names tileset 3 where its siblings name 1, so its layout pointer lands
inside 7-3's last command (a garbled arena, whose header overlaps the W1 arena's;
the pipeline corrects it to tileset 1 with `qol::fix_3hb18_tileset`), and 3-9 and
3HB18 end on alt pointers naming tileset 0.

Modules that read through it: `items::powerups` (`every_power_block`),
`cosmetic::level_music` (`every_level`), and the enemy pass, Big ? pass and
5-F2/Bowser composers (`every_level` / `read_objects`). The region walk
powerups used before rerolled every real block but also 15 header-shaped bytes
no level loads, and misread junctions after an Ice/Sky `E7` (fixed in the
shared `LevelDataRegion::command_size`, which still sizes every command).
`items::chest_item_offsets` stays a hand-kept list; a pipeline test checks it
against every `$D6` chest the index finds on finished ROMs. Enemy protections
name their level and area, and a test checks each against the index. A stream
no level loads is not randomized;
`every_loaded_object_stream_was_visible_to_the_enemy_pass` checks, across a
flag matrix, that no flag makes one live later.

## ROM Map

**`tools/rom_map.json`** is a pre-built JSON map of the entire ROM, generated by
`rom_map.py`. **For a level's contents use `rominfo --level` instead** (see
Tooling); the JSON is still the place for the tables below until they are ported.
It contains:

- All 493 powerup block offsets (byte2 values, tile IDs, randomize class, protection flags)
- All 9 level data regions with every level header, command count, and per-level powerup lists
- All 340 world pointer table entries (type, tileset, obj/lay pointers, shuffleability)
- All 2077 enemy/object entries (class, randomizability, protection flags)
- Level groups with sub-area tracing and boss detection (Boom-Boom, Koopaling, Bowser per group)
- Key ROM tables (LL_PowerBlocks, LATP_QBlocks, palettes, etc.)
- Protected offsets (7-7 Q-stars, 7-F1 Tanooki)

Regenerate after ROM structure changes: `nix-shell -p python3 --run "python3 tools/rom_map.py"`

`rom_map.py` also has diagnostic modes:
- `--numbered [--world N]` — BFS-ordered map with human-readable level names
- `--walk [--world N]` — BFS walk visualization
- `--progression [--world N]` — Fortress progression simulation
- `--check [--world N]` — Check for uncovered blank nodes

The map is gitignored since it's derived from the ROM file.

## ROM Reference

`docs/smb3_rom_reference.md` contains comprehensive documentation of SMB3 ROM offsets, data structures, RAM addresses, and bank layout. **When researching new ROM hacking information (offsets, data formats, pointer tables, RAM addresses, etc.), always update this document with the findings.** This avoids redundant research across sessions.

## Working Style

- When encountering unexpected results during investigation, **stop and ask the user** rather than continuing to dig deeper. Present what you found and what doesn't match, then let the user guide the next step.
- **Don't chase rabbits.** When a task leads to a secondary problem, stop and summarize what you've found so far instead of diving deeper. Present the situation and let the user decide whether to pursue it. This applies to debugging chains, research tangents, and refactoring urges alike.
- **Prefer simplicity.** Think like grug — avoid clever abstractions, premature generalization, and over-engineering. The simplest code that solves the problem is the right code.
- **Clarify before building.** When a request is ambiguous or could go multiple directions, ask a clarifying question rather than guessing. A 30-second question saves a 30-minute redo.
- **Check `rom_data.rs` before writing patches.** All ROM constants, free space maps, and offset tables live in `rom_data.rs`. Before adding new 6502 patches or claiming free ROM space, review it to avoid collisions with existing patches and to keep the single source of truth up to date.

## Key Technical Notes

- ROM is SMB3 USA Rev 1: 393,232 bytes (16 header + 256KB PRG + 128KB CHR), Mapper 4 (MMC3)
- A Rev 0 (PRG0) input is converted to Rev 1 inside `Rom::from_bytes_lax` with a
  bundled IPS (`assets/prg0_to_prg1.ips`, headered offsets), so **nothing else in
  the crate is revision-aware** — keep it that way. `rom.original` is the
  converted Rev 1 bytes; `rom.ips_baseline_bytes()` is what the user supplied and
  is what an emitted patch must diff against
- Seedable RNG via ChaCha8Rng — same seed produces identical output on native and WASM
- IPS generation is diff-based: modify ROM bytes in memory, then diff against original
- Conditional compilation: `clap` for native only, `wasm-bindgen` for WASM only
- `getrandom` 0.3+ on wasm32 requires `--cfg getrandom_backend="wasm_js"` (set in `.cargo/config.toml`)
- `rand` 0.9: use `IndexedRandom` for `.choose()`, `SliceRandom` for `.shuffle()`, `rng.random_range(..N)` instead of `gen_range`
