# Changelog

All notable changes to SMB3-RS are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
New work accumulates under **[Unreleased]** and is moved into a new versioned
section when a release is cut — a merge to `main`, which bumps the version and
deploys.

## [Unreleased]

### Added

- Beta site is now visually distinct from the main site: the `/beta/` deploy
  shows a hazard-striped "BETA BUILD" banner, a violet frame, and a BETA badge
  in the header so it can't be confused with the stable release page.

- Canoe "call the boat" rescue: stand on any dock and press A to summon the
  canoe to the water beside you, then board as usual. Prevents canoe softlocks
  where the boat was left out of reach, in both 1- and 2-player games.
- Two-player "warp to partner" escape hatch: on the overworld map, the active
  player can press Start+Select to jump to the other player's tile. This
  prevents softlocks where one player moves a shared map object (such as the
  `8s are Wild` canoe) out of the other's reach. No effect in 1-player games.

## [1.0.3] - 2026-07-21

### Fixed

- Archived version pages (`.../smb3-rs/v/<version>/`) were shipping without their
  WASM bundle, so they loaded a blank shell with no options. The snapshot step
  now force-includes `pkg/`, which `wasm-pack`'s generated `.gitignore` had been
  causing `git add` to skip.

## [1.0.2] - 2026-07-20

### Added

- Every version of the web app is now archived at a permanent URL
  (`.../smb3-rs/v/<version>/`). The site root keeps serving the latest build;
  each merged version is also frozen at its own path so it never changes. The
  "Share URL" button now points at the exact version that generated the seed, so
  a shared link keeps producing the same seed even after newer versions ship. A
  version picker in the footer lets players open any older build.

## [1.0.1] - 2026-07-20

### Changed

- The 7-Fortress 1 ? block that gates the Tanooki area now randomizes 50/50
  between a Fire Flower and a Super Leaf instead of always being a Fire Flower.
  It can never roll a star, so small Mario always gets a power-up that lets him
  break the bricks to reach the area.

## [1.0.0] - 2026-07-19

### Changed

- The title-screen seed-verification icons now depend on the randomizer version
  in addition to the seed and options. Two builds with different randomization
  logic no longer show identical icons for the same seed. (CI now requires a
  version bump on every merge to `main`, so each release is a distinct version.)

## [0.12.9] - 2026-07-18

### Fixed

- The title screen no longer rolls the attract-mode demo. Sitting on the 1P/2P
  menu now holds indefinitely instead of timing out into the recorded demo
  playback.

## [0.12.8] - 2026-07-18

### Fixed

- The final Big Green Troopa in 4-1 is now covered by the level's hazard
  protection, like the Big Red Troopas earlier in the stage. Each troopa sits on
  a small platform Mario must land on to progress, so a hazard enemy there could
  force an unavoidable hit.

## [0.12.7] - 2026-07-17

### Fixed

- Randomized enemies no longer place a Dry Bones in the Coin Ship reward fight.
  That room is enclosed and never scrolls, so a Dry Bones — which revives after
  every stomp and has nowhere to wander off — could never be cleared.

## [0.12.6] - 2026-07-17

### Changed

- Overworld shortcut pipes now vary how much they skip: each pipe rolls a random
  cap on how many forced levels it may bypass (usually 1–2, occasionally more)
  instead of always grabbing the largest possible skip. Big skips still happen,
  just less often — so a single pipe no longer routinely trivializes a short
  world like 2 or 6, while the overall maps stay less linear.

## [0.12.5] - 2026-07-17

### Changed

- The ending credits montage now presents the eight world scenes in the same
  order the player traversed the worlds when World Order randomization is on
  (Dark Land still closes the sequence). Each world's picture, sprites,
  palette, and graphics keep their original pairing — only the order changes.
- The credits mini-maps are redrawn from the randomized overworld: each world's
  little top-down map now shows a (randomly chosen) page of that world's actual
  randomized map — real terrain, paths, and level / fortress / pipe / toad-house
  markers — in the world's own palette. The picture frame, sprites, and colors
  are untouched; only the map inside each frame is regenerated. World 8 is framed
  on Bowser's castle (the finale), showing its randomized dark-world approach.
- Each credits scene's "WORLD n" caption is renumbered to match the new
  progression order, so the first world shown reads "WORLD 1", the second
  "WORLD 2", and so on (the world's name and theme are unchanged).
- Credits mini-maps now draw hand-trap slots with a ring node marker (the
  spade/bonus-game tile) instead of a stray straight path segment.

## [0.12.4] - 2026-07-17

### Added

- New shipped visual patch **Baldman Bros** by Dr. Trash Panda
  (<https://www.twitch.tv/doctor_tp>), selectable in the web app's Visual
  Patch picker.

## [0.12.3] - 2026-07-16

### Added

- **Remove Flashing** (MaCobra52): a Visual option that suppresses the
  full-screen palette flash/fade animation for photosensitive-safe play. On by
  default; not encoded in the flag key and consumes no RNG. Turn it off with
  `--keep-flashing` on the CLI.

### Fixed

- **Fire enemies stay dead** (MaCobra52's "Tail Enemies don't respawn", always
  on): Fire Chomp and Fire Snake no longer respawn after you defeat them and
  scroll them off-screen and back. ("Tail" in the patch name refers to these
  fire-trail enemies — nothing to do with the Raccoon/Tanooki tail.)

## [0.12.2] - 2026-07-16

### Changed

- Wild injections reworked to be level-centric (driven by the node catalog
  instead of raw enemy pointers). Chasers are now placed into real action
  levels only: **fortresses, airships and Bowser are excluded by type**, so a
  chaser can no longer turn up in a boss room. A level is never given a chaser it
  already has (fixes a second Angry Sun stacking onto 2-Quicksand and breaking
  it), shared enemy sets inject at most once, and injections now write to the
  correct enemy-data location (the old path was offset by 0x10, which could
  corrupt a level). Suns still spawn on screen 0. **Boss Bass is dropped from the
  injection pool** — it's a water-class enemy, so the enemy shuffle reshuffled an
  injected one away; injections are now Lakitu + Angry Sun, weighted ~2:1 toward
  the sun since Lakitu is the harder chaser. An injected Lakitu's height is
  randomized between the replaced enemy's spot and a raised height, so it isn't
  always at the harder low position.
- Wild injections roll more often (~15% → ~40% per level) so a seed lands
  noticeably more Lakitu / Angry Sun chasers.

## [0.12.1] - 2026-07-15

### Changed

- Wild injections (Lakitu / Angry Sun / Boss Bass) are no longer placed in any
  level segment that contains a Boom-Boom, so a level-wide chaser can't turn up
  in a fortress boss room.

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
  from the flag key (flag-key version bumped to 25). Also fixed the web UI so
  applying a flag key with the toggle off actually clears it — the option was
  marked as not-in-flag-key, so `applyOptions` skipped it and left a
  previously-enabled checkbox on.

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
