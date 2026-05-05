# SMB3R - Super Mario Bros. 3 Randomizer

A randomizer for Super Mario Bros. 3 (USA Rev 1) that outputs an IPS patch or patched ROM. Runs as a native CLI binary or in the browser via WebAssembly. Users must provide their own ROM.

## Building

Requires: gcc, rustup, wasm-pack, pkg-config, openssl

On NixOS, use the included dev shell:

```sh
nix-shell
```

### Native CLI

```sh
cargo build                  # debug binary -> target/debug/smb3r
cargo build --release        # optimized binary -> target/release/smb3r
```

### WebAssembly

```sh
wasm-pack build --target web --out-dir pkg   # WASM module -> pkg/
```

### Tests

```sh
cargo test
```

## Usage

```sh
smb3r <rom> [options]
```

### Options

| Flag | Description |
|------|-------------|
| `--seed <N>` | Random seed (default: random) |
| `-o, --output <path>` | Output file path |
| `--patched-rom` | Output a patched ROM instead of an IPS patch |
| `--flag-key <key>` | Apply options from a flag key (e.g. `SMB3R-02E3880480`). Overrides other flags |
| `--no-powerups` | Disable power-up randomization |
| `--no-palettes` | Disable palette randomization |
| `--enemies` | Enable enemy randomization (experimental) |
| `--world-order` | Enable world order randomization |
| `--big-q-blocks` | Enable Big ? Block randomization |
| `--no-shuffle-pipes` | Disable pipe endpoint shuffle during overworld rebuild (on by default) |
| `--no-shuffle-airships` | Disable airship shuffle across worlds (on by default) |
| `--keep-autoscroll` | Keep autoscrollers enabled (disabled by default) |
| `--no-chest-items` | Disable chest/reward item randomization |
| `--keep-whistles` | Keep warp whistles (removed by default) |
| `--keep-drawbridges` | Keep W3 drawbridges toggling (fixed open by default) |
| `--keep-w2-rock` | Keep W2 secret path rock (removed by default) |
| `--no-card-speed-clear` | Disable card speed clear (one-of-each skips cutscene, on by default) |
| `--no-airship-lock` | Disable airship lock (anchor effect, on by default) |
| `--starting-lives <N>` | Set starting lives, 1-99 (default: 4) |

### Examples

```sh
# Generate an IPS patch with default settings
smb3r rom.nes

# Generate a patched ROM with a specific seed
smb3r rom.nes --seed 12345 --patched-rom

# Apply settings from a flag key
smb3r rom.nes --flag-key SMB3R-02E3880480

# Full randomization
smb3r rom.nes --world-order --big-q-blocks
```

## Web App

Open `web/index.html` in a browser after building the WASM module. The web app loads the WASM from `pkg/` and lets users select a ROM file, configure options, and download the patched output.

## Acknowledgments

This project stands on the shoulders of giants:

- **Fred (fcoughlin)** ([Twitch](https://www.twitch.tv/fcoughlin)) — for the original SMB3R and inspiration
- **MaCobra52** ([GitHub](https://github.com/MaCobra52) | [Twitch](https://www.twitch.tv/macobra52)) — for continuing the labour of love for SMB3R and for all the community efforts.
- **Captain Southbird** ([SMB3 Disassembly](https://github.com/captainsouthbird/smb3)) — the comprehensive SMB3 disassembly that made all of this possible
