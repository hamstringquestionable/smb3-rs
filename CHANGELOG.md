# Changelog

All notable changes to SMB3-RS are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
The project is pre-1.0; new work accumulates under **[Unreleased]** and is moved
into a versioned section when a release is cut.

## [Unreleased]

### Fixed

- Wild-injected Angry Suns no longer get stuck idling in the background (which
  could also stop a level's goal card from spawning, making the level
  uncompletable). Injection used to leave the sun at the replaced enemy's
  position — usually deep in the level — but with Early Sun on, the sun only
  attacks if it spawned on the first screen. Injected suns are now seeded at the
  vanilla screen-0 spawn so they engage as intended.
- The "Oops all Anchors" (`anchor_visuals`) toggle is now encoded in the
  shareable flag key, so turning it on/off actually changes the key and the
  option round-trips when a key is loaded. Previously it was silently dropped
  from the flag key (flag-key version bumped to 25).

## [0.12.0] - 2026-07-12

### Fixed

- Lobby Shuffle no longer crashes when a level whose interior is a vertical
  shaft (7-1, 7-6) or a door room (2-Pyramid) is entered through another
  level's front pipe. Those interiors carry an out-of-range pipe-exit
  direction that vanilla only ever reaches by falling in or through a door;
  the shuffle now normalizes the donated direction to a valid pipe exit so
  the player lands correctly instead of crashing.

### Changed

- Overworld level placement is less linear: the weight biasing levels onto the
  main start→airship route was halved (1.5 → 0.75), so fewer forced levels get
  glued back-to-back along the critical path. Average run of consecutive
  must-play levels drops from ~2.1 to ~1.8 (in line with the reference SMB3
  randomizer) while levels still favor the route over dead-end spurs.
- Overworld pipe routing in multi-island worlds now grows a chain outward from
  the start, bridging the nearest unreached island each step, instead of always
  piping the start island straight to the goal island. Worlds like 7 and 8
  (5-7 islands) now route the player through the intermediate islands as
  intended rather than collapsing the journey into one jump; connectivity is
  still guaranteed (a direct link to the goal is used only as a last-pipe
  fallback).
- Overworld "spare" pipes (those beyond what island connectivity requires) are
  now placed after levels are laid out, so each one is aimed to skip a run of
  forced levels instead of being scored on spatial spread alone. Fewer pointless
  pipe loops, more genuine shortcuts (pipes now skip ~60% more levels), and a
  shorter average forced-level run (~1.8 → ~1.4). Every world keeps its vanilla
  pipe count; connectivity pipes are unchanged.
- World 8's showcase bridges are gated out (as a fortress lock) more often: at
  least one bridge is out in ~99% of seeds (was ~80%) and two in ~30% (was ~6%),
  with a rare ~0.08% chance all four are out at once. Pure lock-placement bias;
  connectivity and beatability are unaffected.
- Lobby Shuffle pool grows to 11 with the 2-Pyramid bonus rejoining (its
  pipe-exit crash is fixed above).
- Garbled enemy sprites in levels with player-chasing enemies: Lakitu, the
  Angry Sun, and the Big Berthas (vanilla, wild-picked, or wild-injected)
  now pin their graphics page across the whole level instead of just their
  own screen, and wild injections check the entire enemy segment (including
  levels that share its data).
- Garbled enemy sprites in levels with cannons and spawner pipes: the cannon
  fire family now counts toward graphics-page compatibility — cannonball and
  bob-omb cannons force their page level-wide (matching how the game engine
  reloads it every frame), goomba pipes and Bill cannons account for the
  page their spawned enemies need, and cannon shuffle picks respect the
  pages already committed around them.

## [0.11.2] - 2026-07-10

### Added

- **Lobby Shuffle** (off/on/maybe, `--antechamber-shuffle`) — the ten
  levels that open with an entry area whose pipe leads into the level
  itself (4-3, 5-2, 5-3, 6-6, 6-9, 7-1, 7-4, 7-5, 7-6, 7-7) get their
  interiors randomly permuted, so one level's entrance can drop into
  another's interior. The level then plays out through that interior's
  vanilla ending; map completion still credits the tile you entered from.
- 34 new king rescue quotes: 26 suit-specific (9 frog, 8 raccoon, 9 hammer)
  plus eight standard quotes.

### Changed

- Wandering map bros now avoid stepping onto hand-trap tiles entirely
  (previously they stepped on and immediately marched off again).

### Fixed

- Wandering Hammer Bros can no longer land on beaten piranha-plant or W8
  army map nodes, which let the player replay the beaten level by touching
  the bro.

### Removed

- The no-op `--shuffle-pipes` and `--shuffle-airships` CLI flags — both
  features are on by default; use `--no-shuffle-pipes` /
  `--no-shuffle-airships` to disable them.

## [0.11.1] - 2026-07-09

### Added

- **Piranha Shuffle** (off/on/wild, `--piranha-shuffle`) — frees the two W7
  piranha plant levels (7-P1/7-P2) into the level shuffle pool. On: their
  plant sprites travel with them, guarding whichever slot they land on
  (auto-starts on step, poofs when beaten, vanilla style). Wild: the plants
  scatter instead — one lands on a random level slot in each world. The
  plant levels' treasure chests now carry their own item (randomized with
  chest items), so they reward correctly no matter how they're entered.

## [0.11.0] - 2026-07-08

### Added

- **Player color picker** — choose Mario's color from a NES palette grid in
  the web app (or `--player-color <hex>` in the CLI); Luigi and the power-up
  suits get matching colors derived from the pick, keeping the vanilla
  brother contrast and natural skin tones. Random (the default) now rolls a
  random color through the same matching-wardrobe scheme instead of the old
  fully-independent byte picks. Composes with the visual re-skin patches:
  the scheme anchors on the character's current colors, so picking works
  the same on Luigi-35th, Peach, and Dr. Mario re-skins.

### Changed

- **Palette options reorganized into "Player colors" and "World colors"** —
  the old Palettes / Themed per-tileset / Player color trio is now two
  independent toggles: Player colors (the wardrobe: off = vanilla outfits,
  random, or a picked color) and World colors (themed level/enemy/map
  recoloring). Themed world colors no longer require player colors to be
  on, and turning them on no longer re-rolls the wardrobe.
- **Themed palettes: context-aware color themes + wider coverage** — themed
  palette randomization now applies subtle, context-aware hue shifts on top
  of the variant swap: each context (plains, water, fortress, desert,
  lava, maps, ...) rolls its own small shift (at most 2 steps on the NES
  hue wheel) from a per-context allowed set, so water stays watery, lava
  stays warm, and skies never go magenta. Brightness is never changed, so
  visibility is preserved. Coverage extended to the W6/W7 overworld maps,
  the slot-table tail (lava/Bowser quartets), the 0x36E20 palette pool,
  and stragglers past slice 4 — 118 new curated positions plus 324
  rotate-only positions that previously stayed vanilla.

## [0.10.3] - 2026-07-07

### Fixed

- **Airship-lock patch corrupted 4-4's sub-area** — removed a dead always-on
  write (`A9 01 EA` at `0x1FABC`) that was intended to keep the airship from
  moving. The offset actually landed in the middle of level 4-4's sub-area
  layout data, so entering that sub-area black-screened. The write did nothing
  for airship behavior — the mobile airship is a live map-object the builder
  never spawns (the airship is placed as a static tile), so there is nothing to
  lock — and removing it fixes the crash with no behavior change.

## [0.10.2] - 2026-07-05

### Fixed

- **Hold-left airship entry** — holding Left while entering an airship no longer
  spawns Mario out over the pit and kills him (seen with autoscrollers disabled).
  Applies MaCobra52's "Hold left fix" as an always-on bugfix.

## [0.10.1] - 2026-07-05

### Fixed

- **Start↔Airship swap — death respawn** — in a swapped world, dying with lives
  remaining in a level on a different overworld page than the swapped start no
  longer strands Mario on a blank tile with the map drawn on the wrong screen.
  The engine's "skid back from afar" restores the camera from a secondary scroll
  backup the swap scaffolding never seeded, so it scrolled to page 0; it is now
  seeded (at both Map Init and the game-over finalize) so the skid scrolls to the
  real start page.

### Changed

- **Start↔Airship swap — start framing** — a swapped start on a non-zero screen
  is now centered half a screen back instead of pinned at the left edge of its
  page, so the surrounding map is visible and the camera no longer auto-pans on
  arrival. Page-0 / unswapped worlds are unaffected.

## [0.10.0] - 2026-07-01

### Added

- **Randomized Boom-Boom stomp counts** — each fortress's Boom-Boom now takes a
  random 1–5 stomps to defeat (per-fortress, distinct within each world) instead
  of the fixed 3. On by default; disable with `--keep-boomboom-stomps`. Fireball
  defeats are unaffected.
- **β9 Tornado** — when beta stages are included (`--include-beta-stages`), one of
  the β9 beta stage's three Fire Chomps is randomly turned into a Tornado (borrowing
  the World 2 quicksand Tornado's height).

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
