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

This project is **lint-clean**: `cargo clippy --all-targets` must produce zero warnings. CI (`.github/workflows/ci.yml`) enforces this by running `cargo clippy --all-targets -- -D warnings`, which converts any warning into a build failure.

Before committing:

```sh
cargo clippy --all-targets   # must show no warnings
cargo test                   # must pass
```

When clippy flags new code:

1. **Idiom lints** (`needless_range_loop`, `manual_clamp`, `useless_vec`, etc.): apply the suggested fix. Clippy's lint pages link to docs explaining the *why*.
2. **Judgment-call lints** (`too_many_arguments`, `type_complexity`): consider whether the suggested refactor reveals a real concept. If yes, do the refactor. If no, add `#[allow(clippy::<lint_name>)]` immediately above the item, prefixed with a `// Reason: ...` comment explaining the decision.

Never silence a lint by deleting the warning text or globally disabling — the goal is "every warning was considered," not "no warnings emitted."

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

The total free-space figure is misleading. Free space is **per-bank**, and a
routine has to live in a bank that is mapped when it runs, so the only number
that matters is what's left in *the bank you need*. Measured against the current
allocations (see `FREE_SPACE_ALLOCATIONS` in `rom_data/free_space.rs`):

| Bank | Mapped at | Free left | Largest single gap |
|------|-----------|-----------|--------------------|
| PRG031 | `$E000–$FFFF`, always | **68** | **30** |
| PRG030 | `$8000–$9FFF`, always | **120** | **74** |
| PRG000–PRG007 | swapped, in-level | 34–58 each (bank 3: **0**) | ≤ 38 |
| PRG010 | `$C000–$DFFF`, map | 880 | 588 |
| PRG026 | `$A000–$BFFF`, map/inventory | 2603 | 2537 |

The always-mapped banks are effectively full. Any patch that must run regardless
of the current bank has under 30 contiguous bytes to work with, so a trampoline
into a swapped bank is usually the only option — and that costs bytes too.

Regenerate these numbers after adding allocations; the script pattern is to sum
`FREE_SPACE_ALLOCATIONS` and scan the PRG region for `$FF`/`$00` runs not
covered by one.

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

1. **Randomization modules** (`overworld_build/`, `levels.rs`, etc.) — contain the algorithms that decide *what* to change (BFS placement, shuffle logic, constraint solving). They consume RNG and produce descriptions of changes (new positions, new assignments, etc.).

2. **Helper modules** (`pipe_helpers.rs`, `overworld_helpers.rs`, `level_helpers.rs`) — contain the mechanical ROM write operations that execute those decisions. These are pure functions that take explicit inputs (positions, indices, tile values) and write to the ROM. They have no randomization logic or decision-making.

**Why this matters:** Multiple randomization modules may need to perform the same ROM operations (e.g., swapping pointer table entries, updating pipe destination tables, re-sorting the pointer table). Centralizing these writes in helper modules avoids duplication and ensures consistent behavior. When adding new randomization features, check the helper modules first — the write operation you need may already exist.

**Current helpers:**
- `pipe_helpers.rs` — entry position swaps, pipe destination table writes, pointer table re-sorting
- `overworld_helpers.rs` — lockable tiles, FX patterns, gap tiles, target finding
- `level_helpers.rs` — shared `shuffle_entries()` for level entry shuffling

## Project Structure

```
src/
  lib.rs               # Public API: generate_patch(), generate_patched_rom()
  main.rs              # CLI (clap): file I/O, arg parsing
  rom.rs               # iNES header parsing, ROM validation, Rom struct
  ips.rs               # IPS patch builder (build_ips_patch) and applier (apply_ips_patch)
  randomizer.rs        # Orchestration: Options struct, calls randomize modules
  testrom.rs           # Playtest ROM builder (native-only) — see below
  bin/testrom.rs       # `testrom` CLI: thin clap wrapper over testrom.rs
  wasm.rs              # wasm-bindgen glue (only compiled for wasm32)
  randomize/
    mod.rs
    rom_data.rs        # Shared ROM constants, data structures, read helpers
    # --- Overworld builder pipeline (catalog → pickup → build → write) ---
    node_catalog.rs    # Phase 1: classify all 340 pointer table entries
    overworld_pickup.rs # Phase 2: clear map, build level/HB pools
    overworld_build/   # Phase 3: choice-first builder (placement phases + shaping loop + censuses)
    overworld_writer.rs # Phase 4: write assignments to ROM (pointer tables, FX, map tiles)
    overworld_helpers.rs # Shared overworld write helpers (locks, FX, gap tiles)
    # --- Helper modules (ROM write operations, no RNG) ---
    pipe_helpers.rs    # Pipe destination tables, entry swaps, pointer table re-sorting
    level_helpers.rs   # Shared shuffle_entries() for level entry shuffling
    # --- Feature modules ---
    map_walker.rs      # BFS map walker for overworld connectivity analysis
    levels.rs          # Airship shuffle (the one cross-world level shuffle that's still independent of the overworld builder)
    powerups.rs        # ? block item randomization
    palettes.rs        # Player wardrobe colors + themed world palettes
    enemies.rs         # Enemy type swapping within class
    world_order.rs     # Shuffle world progression order
    items.rs           # Chest/reward item randomization
    qol.rs             # Quality-of-life patches (lives, drawbridges, W2 rock)
    autoscroll.rs      # Autoscroll removal
    title_screen.rs    # Title screen seed hash icons
    king_quotes.rs     # Randomized king rescue quotes
web/
  index.html           # Browser frontend
  style.css
  app.js               # Loads WASM, handles file input, triggers download
tools/
  rom_map.py           # ROM map generator + diagnostic modes (see below)
  rom_map.json         # Pre-built ROM map (gitignored, regenerate with rom_map.py)
  fx_check.py          # Cross-checks FX slots against actual map tiles
  level_sim.py         # Level tile simulator for debugging individual levels
  gen_test_roms.py     # Batch test ROM generation
  gen_visual_previews.py # Renders the player-sprite preview PNGs for visual IPS patches
  offset_dups.py       # Flags ROM offsets that bypass their rom_data.rs constant
docs/
  smb3_rom_reference.md # ROM hacking reference (offsets, data structures, RAM map)
```

## Overworld Builder Pipeline

The overworld builder is the core randomization system, implemented as a four-phase pipeline in `randomizer.rs`: **catalog → pickup → build → write**.

1. **Catalog** (`node_catalog.rs`) — classifies all 340 pointer table entries across 8 worlds (Level, Fortress, Pipe, HammerBro, ToadHouse, Airship, Bowser, etc.)
2. **Pickup** (`overworld_pickup.rs`) — clears the map to blank path tiles, builds a shuffleable pool of levels and hammer bro encounters, applies theme-aware blank tiles per screen
3. **Build** (`overworld_build/`) — the choice-first builder: per world, knob-free uniform placement phases (connectivity pipes bridge islands → levels → forts → locks) followed by a diagnosis-driven shaping loop (lock re-place, gated shortcut, fort+lock move, level move, pipe move) that guarantees a minimum cheapest-route cost (`C1_FLOOR`) and seeks ≥2 routes in the choice band; a world finishing below the floor redeals its pipe web. Cross-world passes handle secret-exit safety, hammer-bro fill, toad-house/spade promotion. Hard invariants: order-free completability fixpoint, row 7/8 completion-bit rule
4. **Write** (`overworld_writer.rs`) — single-pass ROM write: updates pointer tables, FX table, pipe destination tables, map tiles, and hammer bro sprite assignments

When the overworld builder is active, `levels.rs` intra-world shuffle and airship shuffle are bypassed since the builder handles them.

## ROM Map

**`tools/rom_map.json`** is a pre-built JSON map of the entire ROM. Before scanning the ROM manually for offsets, powerup locations, level data, enemy positions, or pointer tables, **always check `tools/rom_map.json` first**. It contains:

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
- Seedable RNG via ChaCha8Rng — same seed produces identical output on native and WASM
- IPS generation is diff-based: modify ROM bytes in memory, then diff against original
- Conditional compilation: `clap` for native only, `wasm-bindgen` for WASM only
- `getrandom` 0.3+ on wasm32 requires `--cfg getrandom_backend="wasm_js"` (set in `.cargo/config.toml`)
- `rand` 0.9: use `IndexedRandom` for `.choose()`, `SliceRandom` for `.shuffle()`, `rng.random_range(..N)` instead of `gen_range`
