# SMB3R — Super Mario Bros. 3 Randomizer

A randomizer for Super Mario Bros. 3 (USA Rev 1). Runs entirely in your
browser — your ROM never leaves your machine.

## Use it

**→ https://hamstringquestionable.github.io/smb3-rs/**

Provide your own SMB3 (USA Rev 1) ROM. Choose options or paste a flag key
to reproduce someone else's settings, then generate an IPS patch or
patched ROM. All randomization runs locally via WebAssembly.

The deploy pipeline also publishes branch builds at `/beta/<branch>/` for
testing in-progress changes.

## Report a bug

File an issue at https://github.com/hamstringquestionable/smb3-rs/issues.
Include the seed and flag key from the web app so the run is reproducible.

## Contributing

### Build

Requires Rust (edition 2024), `wasm-pack`, and the usual native toolchain
(gcc, pkg-config, openssl). On NixOS, `nix-shell` provides everything.

```sh
cargo build                                  # native CLI
cargo test                                   # run the test suite
wasm-pack build --target web --out-dir pkg   # web app build
cargo clippy --all-targets -- -D warnings    # lint check (CI enforces)
```

After a wasm build, open `web/index.html` from a local server to test
the frontend.

### Project layout

- `src/` — Rust source. Library + CLI + WASM glue (`src/wasm.rs`).
- `src/randomize/` — per-feature randomization modules.
- `web/` — frontend (HTML/CSS/JS, talks to the wasm module).
- `tools/` — Python helpers for ROM analysis (`rom_map.py` is the big one).
- `docs/smb3_rom_reference.md` — SMB3 ROM offsets, data structures, RAM
  map. Update this when you discover new ROM details.
- `CLAUDE.md` — orientation for working with the codebase (build commands,
  lint policy, architecture conventions). Worth reading before submitting
  changes.

### CLI

There's also a native CLI (`smb3-rs`) used for batch testing and
debugging. Run with `--help` for the full flag list.

```sh
smb3-rs <rom> --seed 12345 --patched-rom -o out.nes
```

## Acknowledgments

- **Fred (fcoughlin)** ([Twitch](https://www.twitch.tv/fcoughlin)) — original SMB3R and inspiration
- **MaCobra52** ([GitHub](https://github.com/MaCobra52) | [Twitch](https://www.twitch.tv/macobra52)) — continuing the labour of love and the community work
- **Captain Southbird** ([SMB3 Disassembly](https://github.com/captainsouthbird/smb3)) — the comprehensive SMB3 disassembly that made all of this possible
