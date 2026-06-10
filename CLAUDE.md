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

## Architecture: Separate Randomization from ROM Writes

Randomization modules follow a **decide then write** pattern. Each feature area has two layers:

1. **Randomization modules** (`overworld_build.rs`, `levels.rs`, etc.) — contain the algorithms that decide *what* to change (BFS placement, shuffle logic, constraint solving). They consume RNG and produce descriptions of changes (new positions, new assignments, etc.).

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
  wasm.rs              # wasm-bindgen glue (only compiled for wasm32)
  randomize/
    mod.rs
    rom_data.rs        # Shared ROM constants, data structures, read helpers
    # --- Overworld builder pipeline (catalog → pickup → build → write) ---
    node_catalog.rs    # Phase 1: classify all 340 pointer table entries
    overworld_pickup.rs # Phase 2: clear map, build level/HB pools
    overworld_build.rs # Phase 3: assign levels to slots, place locks/pipes/HBs
    overworld_writer.rs # Phase 4: write assignments to ROM (pointer tables, FX, map tiles)
    overworld_helpers.rs # Shared overworld write helpers (locks, FX, gap tiles)
    # --- Helper modules (ROM write operations, no RNG) ---
    pipe_helpers.rs    # Pipe destination tables, entry swaps, pointer table re-sorting
    level_helpers.rs   # Shared shuffle_entries() for level entry shuffling
    # --- Feature modules ---
    map_walker.rs      # BFS map walker for overworld connectivity analysis
    levels.rs          # Airship shuffle (the one cross-world level shuffle that's still independent of the overworld builder)
    powerups.rs        # ? block item randomization
    palettes.rs        # Character/lava/Bowser color randomization
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
  offset_dups.py       # Flags ROM offsets that bypass their rom_data.rs constant
docs/
  smb3_rom_reference.md # ROM hacking reference (offsets, data structures, RAM map)
```

## Overworld Builder Pipeline

The overworld builder is the core randomization system, implemented as a four-phase pipeline in `randomizer.rs`: **catalog → pickup → build → write**.

1. **Catalog** (`node_catalog.rs`) — classifies all 340 pointer table entries across 8 worlds (Level, Fortress, Pipe, HammerBro, ToadHouse, Airship, Bowser, etc.)
2. **Pickup** (`overworld_pickup.rs`) — clears the map to blank path tiles, builds a shuffleable pool of levels and hammer bro encounters, applies theme-aware blank tiles per screen
3. **Build** (`overworld_build.rs`) — assigns levels to map slots via BFS-ordered placement, places fortresses with locks, distributes pipes, tags remaining blanks as hammer bro slots. Enforces connectivity (secret exit safety, row 7/8 completion bit conflicts, cross-screen FX)
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
