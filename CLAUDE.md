# SMB3R — Super Mario Bros. 3 Randomizer

## Project Overview

A Rust utility that randomizes Super Mario Bros. 3 (USA Rev 1) and outputs an IPS patch or patched ROM. Compiles to both a native CLI binary and a WebAssembly module for a browser-based web app. The application never stores or bundles the ROM — users must provide their own.

## Build Commands

All builds require `nix-shell` (or the equivalent packages: gcc, rustup, wasm-pack, pkg-config, openssl):

```sh
nix-shell                                    # enter dev shell
cargo build                                  # native CLI binary -> target/debug/smb3-rs
cargo test                                   # run all tests (19 tests)
cargo build --release                        # optimized binary -> target/release/smb3-rs
wasm-pack build --target web --out-dir pkg   # WASM module -> pkg/
```

## Architecture: Separate Randomization from ROM Writes

Randomization modules follow a **decide then write** pattern. Each feature area has two layers:

1. **Randomization modules** (`pipes.rs`, `overworld.rs`, `levels.rs`, etc.) — contain the algorithms that decide *what* to change (BFS placement, shuffle logic, constraint solving). They consume RNG and produce descriptions of changes (new positions, new assignments, etc.).

2. **Helper modules** (`pipe_helpers.rs`, and future `overworld_helpers.rs`) — contain the mechanical ROM write operations that execute those decisions. These are pure functions that take explicit inputs (positions, indices, tile values) and write to the ROM. They have no randomization logic or decision-making.

**Why this matters:** Multiple randomization modules may need to perform the same ROM operations (e.g., swapping pointer table entries, updating pipe destination tables, re-sorting the pointer table). Centralizing these writes in helper modules avoids duplication and ensures consistent behavior. When adding new randomization features, check the helper modules first — the write operation you need may already exist.

**Current helpers:**
- `pipe_helpers.rs` — entry position swaps, pipe destination table writes, pointer table re-sorting

**Planned helpers (not yet extracted):**
- Fortress/lock/FX table helpers (currently inline in `overworld.rs`)

## Project Structure

```
src/
  lib.rs             # Public API: generate_patch(), generate_patched_rom()
  main.rs            # CLI (clap): file I/O, arg parsing
  rom.rs             # iNES header parsing, ROM validation, Rom struct
  ips.rs             # IPS patch builder (build_ips_patch) and applier (apply_ips_patch)
  randomizer.rs      # Orchestration: Options struct, calls randomize modules
  wasm.rs            # wasm-bindgen glue (only compiled for wasm32)
  randomize/
    mod.rs
    rom_data.rs      # Shared ROM constants, data structures, read helpers
    pipe_helpers.rs  # ROM write helpers for pipe movement operations
    pipes.rs         # Pipe shuffle randomization (BFS placement algorithm)
    overworld.rs     # Fortress redistribution, lock shuffle
    levels.rs        # Level shuffle (intra/cross-world), fortress/airship shuffle
    powerups.rs      # ? block item randomization (0x02611–0x0262A)
    palettes.rs      # Character/lava/Bowser color randomization
    enemies.rs       # Enemy type swapping within class (0x0BFD8–0x0E00D)
    world_order.rs   # Shuffle world progression order (patches INC World_Num at 0x3D0A1)
    map_walker.rs    # BFS map walker for overworld connectivity analysis
    items.rs         # Chest/reward item randomization
    qol.rs           # Quality-of-life patches (lives, drawbridges, W2 rock)
    autoscroll.rs    # Autoscroll removal
web/
  index.html         # Browser frontend
  style.css
  app.js             # Loads WASM, handles file input, triggers download
tools/
  rom_map.py         # Generates tools/rom_map.json from the ROM
  rom_map.json       # Pre-built ROM map (gitignored, regenerate with rom_map.py)
  level_sim.py       # Level tile simulator for debugging individual levels
docs/
  smb3_rom_reference.md   # ROM hacking reference (offsets, data structures, RAM map)
```

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

The map is gitignored since it's derived from the ROM file.

## ROM Reference

`docs/smb3_rom_reference.md` contains comprehensive documentation of SMB3 ROM offsets, data structures, RAM addresses, and bank layout. **When researching new ROM hacking information (offsets, data formats, pointer tables, RAM addresses, etc.), always update this document with the findings.** This avoids redundant research across sessions.

## Working Style

- When encountering unexpected results during investigation, **stop and ask the user** rather than continuing to dig deeper. Present what you found and what doesn't match, then let the user guide the next step.
- **Don't chase rabbits.** When a task leads to a secondary problem, stop and summarize what you've found so far instead of diving deeper. Present the situation and let the user decide whether to pursue it. This applies to debugging chains, research tangents, and refactoring urges alike.

## Key Technical Notes

- ROM is SMB3 USA Rev 1: 393,232 bytes (16 header + 256KB PRG + 128KB CHR), Mapper 4 (MMC3)
- Seedable RNG via ChaCha8Rng — same seed produces identical output on native and WASM
- IPS generation is diff-based: modify ROM bytes in memory, then diff against original
- Conditional compilation: `clap` for native only, `wasm-bindgen` for WASM only
- `getrandom` 0.3+ on wasm32 requires `--cfg getrandom_backend="wasm_js"` (set in `.cargo/config.toml`)
- `rand` 0.9: use `IndexedRandom` for `.choose()`, `SliceRandom` for `.shuffle()`, `rng.random_range(..N)` instead of `gen_range`
