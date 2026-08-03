# `tools/` — index

Diagnostic and generator scripts. **Check here before scanning the ROM by hand
or writing a throwaway script** — most questions about ROM layout, map topology,
or palettes already have a tool.

Keep this directory honest: 15 one-off investigation scripts were deleted on
2026-08-03 once their findings were captured in the Rust source and
`docs/smb3_rom_reference.md`. If you write a script to answer a single question,
either fold the answer into the docs and delete it, or add it here with a note
on why it's worth keeping.

All commands need `nix-shell`:

```sh
nix-shell -p python3 --run "python3 tools/<script>.py [args]"
```

Two scripts need Pillow — use `nix-shell -p python3 python3Packages.pillow`
(marked **[PIL]**).

> **Trust note.** The Rust implementation is the source of truth, not these
> scripts. When a tool disagrees with Rust, suspect the tool.
> `validate_shuffleable.py` currently reports 4 discrepancies against
> `rom_map.json` — unproven until re-checked; do not "fix" Rust on its say-so.

**Verified 2026-08-03** by executing every script and reading its git history.
Commit counts below exclude `ecfc1b3` (a bulk path-fixup chore that touched 20
files without changing behaviour), so "1 commit" means *created once, never
revisited*.

---

## Tier 1 — Live

Maintained, or part of a pipeline that regenerates checked-in artifacts. Keep
these working.

| Tool | Status | Notes |
|------|--------|-------|
| `rom_map.py` | OK | **30 commits** — the workhorse. Generates `rom_map.json`; also `--level <name>`, `--tile <byte>`, `--numbered`, `--walk`, `--progression`, `--check`. |
| `gen_palette_variants.py` | **rewrites source** | Regenerates `src/randomize/palette_variants.rs` from the Recolored IPS + vanilla ROM. Check `git diff` after running. |
| `extract_palette_variants.py` | OK | Feeds the above — extracts quartet-level `VariantGroup` entries. |
| `add_variant_family.py` | **rewrites source** | Appends a hue-family-tinted variant to every `VariantGroup`. |
| `gen_visual_previews.py` | OK **[PIL]** | Regenerates `web/assets/visual-previews/*.png`, shipped in the web app. |
| `required_progression.py` | needs args | Required-progression / linearity metric for randomized ROMs. |
| `offset_dups.py` | OK | Finds ROM offset literals duplicating a `rom_data.rs` constant. Run before committing a patch — CLAUDE.md leans on this. |

## Tier 2 — Working general diagnostics

Single commit each, but they answer *recurring* questions and still run clean.
Written once because they were written well.

| Tool | Status | Notes |
|------|--------|-------|
| `map_viz.py` | OK | **Renders any ROM's world maps as labelled ASCII**, with screen boundaries. Use this instead of hand-decoding tile grids. `map_viz.py <rom.nes> --world 8`. |
| `map_walker.py` | OK | BFS map connectivity + fortress progression. |
| `level_sim.py` | OK | Level tile simulator, for debugging one level's layout. |
| `fx_check.py` | needs args | `fx_check.py <rom.nes>` — cross-checks FX slots against actual map tiles. |
| `parse_ips.py` | OK | Dump every record in an IPS patch. |
| `credits_render.py` | needs args **[PIL]** | `<rom.nes> <out_dir> [scale]` — renders the 8 ending-screen vignettes. |
| `preview_palette_pools.py` | OK | Writes `palette_pools_preview.html` to the repo root (untracked — delete after). |
| `preview_palette_variants.py` | OK | Writes `palette_variants_preview.html` to the repo root. |
| `validate_shuffleable.py` | **see trust note** | Validates `collect_shuffleable` against `rom_map.json`. Reports 4 discrepancies. |

## Side effects

`preview_palette_pools.py` and `preview_palette_variants.py` write HTML into the
repo root (untracked — delete after use). `gen_palette_variants.py` and
`add_variant_family.py` rewrite `src/randomize/palette_variants.rs`; check
`git diff` after running either.

## Playtest ROMs

Use the `testrom` binary, not a script here — see CLAUDE.md. It replaced the
deleted `gen_test_roms.py` (`testrom --place-all 7F1`) and `apply_ips_subset.py`
(the practice-patch subset is now native).
