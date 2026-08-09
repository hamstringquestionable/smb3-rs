# Seed report / spoiler log — design note

Status: design note. Nothing here is implemented yet.

Two features share one data layer: a human-readable **spoiler log** (a wanted
user feature) and an **overworld map view** (which would retire
`tools/map_viz.py`). They need the same facts, so deriving them separately would
create two producers for things like "which fortress opens which lock".

## Shape

```
SeedReport                         serializable; serde is already a dependency
  ├── worlds: Vec<WorldReport>     grid tiles, nodes, locks, pipes
  ├── items                        where each inventory item comes from
  └── progression                  world order, SAS, donors
        ↓
  render_ascii()   render_text()   render_json()
```

The ASCII renderer replaces `map_viz.py`. Sharing one glyph table with the rest
of the report means the `F`-glyph discrepancy noted in `tools/README.md` cannot
recur.

**Location:** not `src/testrom.rs` — that is native-only and the web app will
want the spoiler. `src/randomize/report.rs`, compiled for both targets.

## The rule

**No fact has two producers.**

Derive from the **finished ROM** by default. It is the artifact the player
actually plays, so a writer bug shows up in the spoiler — which is correct,
because the game has it too. `BuildResult` may *enrich* the report with things
bytes cannot recover, but must never re-derive a field the ROM already answers.

Concretely: fortress→lock comes from decoding the FX tables, **not** from
`LockAssignment::fort_section`, even though the latter is easier to read.

## Recoverability of the wanted content

| Content | From ROM? | Notes |
|---|---|---|
| Overworld topology, locks, pipes | yes | tiles + pointer tables + FX tables |
| Fortress → lock it opens | yes | FX tables; layout is documented in `smb3_rom_reference.md` § "Fortress Lock & Bridge FX (PRG010: 0x147CD–0x148B7)" — **read it, do not re-derive from the disassembly.** Doing the latter cost a wrong answer once: `FortressFX_W1–W8` is 32 bytes of slots at `0x14888` *followed* by the 8-byte `FortressFXBase_ByWorld` at `0x148A8`, not the other way round. Validate any decoder against vanilla, where W8 must read `0D 0E 0F 10`. |
| World order (e.g. `3,5,6`) | yes | `FS_WORLD_ORDER` holds routine + display table |
| Inventory item sources (HB1 = Fire Flower, 1F secret exit = Hammer) | probably | written to tables; **not yet verified where each source lives** |
| Antechamber/lobby donor (e.g. "7-5 donated") | **no** | `antechambers::shuffle(rom, rng, beta)` returns nothing; the write destroys provenance |

The donor case is the one that needs a capture hook. The overworld already has
the pattern — `randomize_with_overworld_capture`, carrying a
`// WASM hook to follow` comment. Nothing else does. Add hooks **only** where
the write genuinely destroys provenance; world order and items must not get one,
because the ROM already answers.

## Staging

1. **`from_rom` + ASCII renderer, overworld only.** Self-contained, retires
   `map_viz.py`, needs none of the product decisions below settled.
2. **Item and progression sections.** Requires verifying where item assignments
   are stored.
3. **Antechamber capture hook** — its own change, with its own justification.
4. **WASM hook + web download.**

## Open product questions (not technical)

- Separate file that never ships alongside the ROM? (Accidental spoiling is the
  classic failure.)
- Placements only, or solve the progression path? "1F Secret Exit — Hammer"
  implies annotating *reachability*, which is substantially more than a
  placement dump.
- Reproducible from seed + flag key alone, without the ROM? Nearly free given
  determinism, and makes a spoiler shareable as a one-liner. Caveat: palette
  randomization is not seed-stable (a handful of NES palette bytes differ
  between runs of the same seed), though it does not touch topology.
