# SMB3-RS — Vision & Scope

SMB3-RS (the Rust/WebAssembly randomizer, also called "web rando") is distinct
from **SMB3R**, the established randomizer referenced below as the parity target.

This document states *why* SMB3-RS exists, what it promises, and what it
deliberately is not. It is the north star for feature decisions. When a
proposed change conflicts with something here, the change is wrong — or this
document needs to change first, on purpose.

For *what it is* and *how to build it*, see `README.md`. For *how the code is
organized*, see `CLAUDE.md`.

## Goal

A fresh Super Mario Bros. 3 every time you play. The fun of SMB3 — reading a
level, timing a power-up, earning mastery — goes stale once you have the game
memorized. SMB3-RS keeps the fun and removes the memorization: each seed is a new
game to read on its feet.

## Audience

- **The racing community is the primary driver.** Most design decisions are
  made to serve racers: runs that are fair, reproducible, and varied enough that
  flags meaningfully change how you have to play.
- **Casual play must always remain an option.** Players who just want a fresh
  SMB3 experience are a first-class audience, never sacrificed to serve racing.

## Promises (hard guarantees)

These are non-negotiable. Every emitted seed must satisfy all of them.

1. **Every seed is completable — without exception.** Not "usually." A path to
   the end of the game always exists.
2. **No softlocks.** No state the player can enter and be unable to progress or
   escape from.
3. **No required impossible jumps.** A jump may be impossible only when another
   path exists. The intended route is always traversable.
4. **Every level is beatable entering as small Mario.** The player may always
   arrive at a level with no power-up and still complete it. This does not mean
   the level must be cleared *while* small — if a level requires a power-up, the
   level itself provides a renewable (effectively unlimited) source of it, so a
   player who enters small can always obtain what the level demands.
5. **Determinism.** The same seed + the same flags + the same version of the
   randomizer produces the same ROM, everywhere (native and WASM). The only
   permitted differences are purely visual.
6. **The ROM never leaves the user's machine.** SMB3-RS never hosts, bundles, or
   transmits the ROM. The user supplies their own; it stays on their system.
   This is a legal and privacy commitment, not just an implementation detail.

## Design values

- **Flags produce meaningful gameplay variance.** Almost every flag exists so
  players can tune how the randomization *feels*, and so racers have to *play
  differently* depending on the flags in effect. A flag that doesn't change how
  the game plays isn't pulling its weight.
- **The player chooses the intensity.** Off / on / "maybe" tri-flags and presets
  like Max Chaos exist so the player tunes their own experience rather than
  having one imposed. SMB3-RS offers the range; the player picks the point on it.
- **Beatability is a property of the generator, not the player's luck.** The
  guarantees above are enforced by the randomizer, never left to chance.

## Non-goals

What SMB3-RS deliberately is not, and does not intend to become:

- **Not online or multiplayer.** Single-player, original-hardware-compatible.
  It is a Nintendo game and stays playable on real hardware.
- **Not a level or graphics editor.** SMB3-RS randomizes an existing game; it is
  not a content-authoring tool.
- **Not a different game.** It does not alter core mechanics (physics, controls)
  beyond quality-of-life. It is SMB3, made fresh — not a new platformer.
- **Not multi-version.** Targets SMB3 USA Rev 1 (PRG1). Other revisions are out
  of scope. (PRG0 is text-only today; see `docs`/memory for the current limit.)

## What 1.0 means

SMB3-RS is currently **unreleased**. It reaches 1.0 when both are true:

1. **Feature parity.** The feature set is roughly on par with **SMB3R**, the
   established randomizer, with no marquee feature missing that would make a
   player switch back to it.
2. **Community confidence.** Enough of the community trusts that all seeds are
   beatable and that there are no game-breaking bugs.

Until then, nothing is frozen — including, notably, flag-key bit assignments,
which may change before 1.0.
