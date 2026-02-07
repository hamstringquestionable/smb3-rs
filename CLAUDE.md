# SMB3R — Super Mario Bros. 3 Randomizer

## Project Overview

A Rust utility that randomizes Super Mario Bros. 3 (USA Rev 1) and outputs an IPS patch or patched ROM. Compiles to both a native CLI binary and a WebAssembly module for a browser-based web app. The application never stores or bundles the ROM — users must provide their own.

## Build Commands

All builds require `nix-shell` (or the equivalent packages: gcc, rustup, wasm-pack, pkg-config, openssl):

```sh
nix-shell                                    # enter dev shell
cargo build                                  # native CLI binary -> target/debug/smb3r
cargo test                                   # run all tests (19 tests)
cargo build --release                        # optimized binary -> target/release/smb3r
wasm-pack build --target web --out-dir pkg   # WASM module -> pkg/
```

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
    powerups.rs      # ? block item randomization (0x02611–0x0262A)
    palettes.rs      # Character/lava/Bowser color randomization
    enemies.rs       # Enemy type swapping within class (0x0BFD8–0x0E00D)
web/
  index.html         # Browser frontend
  style.css
  app.js             # Loads WASM, handles file input, triggers download
docs/
  smb3_rom_reference.md   # ROM hacking reference (offsets, data structures, RAM map)
```

## ROM Reference

`docs/smb3_rom_reference.md` contains comprehensive documentation of SMB3 ROM offsets, data structures, RAM addresses, and bank layout. **When researching new ROM hacking information (offsets, data formats, pointer tables, RAM addresses, etc.), always update this document with the findings.** This avoids redundant research across sessions.

## Key Technical Notes

- ROM is SMB3 USA Rev 1: 393,232 bytes (16 header + 256KB PRG + 128KB CHR), Mapper 4 (MMC3)
- Seedable RNG via ChaCha8Rng — same seed produces identical output on native and WASM
- IPS generation is diff-based: modify ROM bytes in memory, then diff against original
- Conditional compilation: `clap` for native only, `wasm-bindgen` for WASM only
- `getrandom` 0.3+ on wasm32 requires `--cfg getrandom_backend="wasm_js"` (set in `.cargo/config.toml`)
- `rand` 0.9: use `IndexedRandom` for `.choose()`, `rng.random_range(..N)` instead of `gen_range`
