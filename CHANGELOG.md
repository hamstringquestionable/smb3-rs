# Changelog

All notable changes to SMB3-RS are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
The project has not had a tagged release yet — it is pre-1.0 and the
`Cargo.toml` version is bumped internally as work lands (currently `0.8.x`).
Until the first release, everything below lives under **[Unreleased]**; cut a
versioned section here when a release is tagged.

## [Unreleased]

This is a baseline backfill of notable changes since the project began. It
summarizes feature areas rather than every commit — see `git log` for the
full history.

### Added

- **Random Fire Flower** (`--fire-flower off|on|wild`, issue #22) — an in-level
  Fire Flower still looks the same but grants a power state derived
  deterministically from a seed salt (the shuffled starting world), the current
  world, the level, and the flower's screen, instead of always Fire. `on`
  substitutes among Fire/Frog/Tanooki/Hammer; `wild` also allows the Small/Big
  downgrades. Same seed always gives the same suit for a given flower; the
  mapping rotates per seed when world-order shuffle is enabled.
- **Overworld builder pipeline** — the core randomization system. A
  four-phase pipeline (catalog → pickup → build → write) that re-lays each
  world: assigns levels to map slots via BFS-ordered placement, places
  fortresses with locks, distributes pipes, and tags hammer-bro slots while
  enforcing connectivity.
- **Start ↔ airship swap (SAS)** — per-world option that swaps the start tile
  with the airship, including engine scaffolding, death-respawn handling, and
  game-over finalize.
- **Troll-pipe level slots** — disguise level slots as pipe tiles (`0xBC`),
  one candidate per world W2–W8.
- **Hand-trap level slots** — visible grabbing-hand tiles (`0xE6`) with a 100%
  grab.
- **Cross-world shuffles** — Toad Houses and spade games shuffled across
  worlds; world progression order shuffle.
- **Segment composers** — `segment_writer` foundation plus the Bowser-castle
  and 5-F2 podoboo-gauntlet composers for safe, X-sorted enemy-segment edits.
- **Enemy randomization** — within-class enemy swapping, a Wild piranha pool
  (self-contained, with Rocky Wrench and directional fire jets), and an
  expanded hazard taxonomy.
- **Quality-of-life flags** — Faster Frog, Limit Bro Movement, MaCobra
  tail-attack patches, W1 hammer rock, and lives/drawbridge tweaks.
- **Tri-state (off/on/maybe) flags** — seed-hidden options that resolve via a
  dedicated RNG substream.
- **Cosmetic options** — palette randomization, "Oops all Anchors" anchor
  visuals, title-screen seed-hash icons with seeded menu music, randomized
  king rescue quotes, and bundled visual patches (Super Princess Peach,
  Super Toad, Dr. Mario Bros 3, and others).
- **Web app** — browser frontend with grouped options
  (Map/Enemies/Bosses/Items/Player/Cosmetic), Off/On pills, presets, and
  sprite-sheet icons.
- **Tooling** — `tools/rom_map.py` ROM map generator with diagnostic modes,
  plus `/level`, `/tile`, and other lookup helpers; ROM Rev 1 CRC
  fingerprint and upload-time validation.

### Changed

- Fortress FX visibility checks use Mario's position and the real per-world
  FX slot (derived from `FortressFX_W1_W8[...]`) rather than `$0745`
  directly.
- Bullet-bill class points at cannon IDs (`0xBC`/`0xBD`) with asymmetric Wild
  counts that never exceed vanilla.
- Pipes are forbidden adjacent to start/target tiles to eliminate
  trivial-bypass worlds.
- Option tooltips show contributor credits on their own line, linked to the
  contributor (MaCobra52 credited across the features he authored).

### Fixed

- Level-data walk no longer overruns the PRG bank into the desert metatile
  table.
- Canoe edges are scoped to their own world in the overworld walker, with a
  stateful required-progression analyzer.
- SAS game-over continue softlock when the start is on a non-zero overworld
  page; W3 fixed-pipe partner biased so the SAS start can reach the airship.
- Wild piranhas keep their hitbox/visibility correct when shuffled into other
  slots; piranhas are never replaced by upward-firing hazards.
- Numerous per-level enemy protections (4-F1 narrow hallway, 7-F2 boss room,
  7-5 walkways, 8-1 Boo, 8F Roto-Discs) so randomized hazards can't block
  required paths.

[Unreleased]: https://github.com/hamstringquestionable/smb3-rs/commits/main
