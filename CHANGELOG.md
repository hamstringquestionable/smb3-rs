# Changelog

All notable changes to SMB3-RS are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
The project is pre-1.0; new work accumulates under **[Unreleased]** and is moved
into a versioned section when a release is cut.

## [Unreleased]

## [0.9.5] - 2026-07-01

### Fixed

- **Randomized Koopalings — ring graphics** — the moved ring attack now loads
  its own sprite CHR page on whichever body carries it, so the ring no longer
  renders as garbled tiles. Also fixes the reverse case where the (no-longer-
  ring) Wendy identity drew a garbled wand blast.

## [0.9.4] - 2026-06-30

### Changed

- **Randomized Koopalings — ring attack** — with random Koopalings on, Wendy's
  ring attack (ring projectile + firing cadence + straight aim) now rides a
  random Koopaling identity's body instead of always Wendy. There's still
  exactly one ring boss; only which body carries it is randomized.

## [0.9.3] - 2026-06-30

### Changed

- **Randomized Koopalings — heavy physics** — with random Koopalings on, the
  heavy-physics effect (enhanced gravity, floor-shake, player paralysis) is now
  reassigned to two random Koopaling identities instead of always Roy and
  Ludwig, so a differently-shaped boss can carry the crushing feel.

## [0.9.2] - 2026-06-28

### Fixed

- **4-1 hazard placement** — the three Big Red Troopas each sit on a small
  platform Mario must land on to move forward; in Wild enemy mode they could be
  swapped to a hazard (Thwomp/Ptooie/nipper/lotus/hotfoot), forcing an
  unavoidable hit. Those spots are now hazard-protected.

## [0.9.1] - 2026-06-27

### Changed

- **Level spread across worlds** — levels are distributed by compressed capacity
  (`capacity^0.5`) instead of straight proportional, so the densest worlds (Ice,
  Desert) no longer hoard levels and the emptier ones (Giant, Pipe, Dark) fill
  out, without forcing every world to the same count. The leftover from rounding
  is now placed in random worlds for a little per-seed variety. The old
  World 6-specific level cap is gone — the level-spread scoring's density penalty
  handles clumping, and measured clumping is actually lower at the new spread.

### Fixed

- **Overworld connectivity** — pipe placement could occasionally strand a
  world's airship/Bowser behind an unreachable region (most often Giant Land),
  producing an unbeatable world. The island-connect step now refuses to spend a
  pipe on a dead-end that doesn't lead toward the target, and will lift the
  start-adjacent no-pipe restriction when that's the only way to keep the world
  completable. This also subsumes the old World 3 start↔airship-swap pipe
  special-case, which has been removed.

### Removed

- **Remove Rocks** is no longer an option — path-blocking rocks (W2 secret path,
  W3 boat dock, W4 pipe shortcut) are always cleared, since the overworld builder
  depends on those tiles being open for connectivity. (Adding extra
  hammer-breakable shortcut rocks remains a separate option.)

## [0.9.0] - 2026-06-25

The first cut: a baseline of notable changes since the project began. It
summarizes feature areas rather than every commit — see `git log` for the
full history.

### Added

- **Shuffle HammerBro Locations** (`--no-shuffle-hammer-bros` to disable; on by
  default, issue #20) — the wandering Hammer Bro encounters are spread across all
  worlds (random 1-3 per world, 15 total, with light anti-clustering) instead of
  their fixed vanilla spots, and each carries its reward item. The Dark World
  keeps at most one, and a couple of map-object slots stay free in every world so
  level-triggered white mushroom houses can still appear. A feature-dense world
  with no spare path tile may get fewer, with its share spilling elsewhere.
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
  tail-attack patches, and lives/drawbridge tweaks.
- **More hammer rocks** (`--more-hammer-rocks off|on|maybe`) — adds
  hammer-breakable rock shortcuts by the W1 toad house and in W8. (Replaces the
  earlier W1-only "W1 hammer rock" flag.)
- **8s are Wild** (`--eights-are-wild off|on|maybe`) — opens up World 8 (Dark
  World) with a canoe on screen 0 and extra paths on screen 2. The W8 screen-3
  water/bridge approach is now always present, independent of this flag.
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

- Airship lock is now always on (the **Remove Anchor** / `--no-airship-lock`
  option is removed): anchors always become random power-ups and airships always
  stay put after a loss instead of moving. Flag-key version bumped to 20.
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
