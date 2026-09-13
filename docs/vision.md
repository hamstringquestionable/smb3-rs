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
- **Not multi-version.** Targets SMB3 USA Rev 1 (PRG1); every patch, offset
  table and free-space allocation assumes those bytes. A Rev 0 (PRG0) dump is
  accepted, but by *converting* it to Rev 1 at load time with a bundled patch —
  not by being revision-aware. Other revisions are out of scope.

## Released, and what that changed

SMB3-RS **released on 2026-07-27** and is on 2.0.1 as of 2026-09-12. The two
bars 1.0 was defined against — feature parity with **SMB3R** and enough
community confidence that seeds are beatable and free of game-breaking bugs —
were the right bars, and clearing them changed the rules in one specific way:

**Flag keys are now compatibility surface.** Before release, bit assignments
were free to move. They are not any more: a key someone posted in a race or a
Discord thread has to keep meaning the same thing. Adding an option means
claiming reserved bits and bumping the key version deliberately — never
renumbering what is already out there. Everything else in this document was
written to outlast the release and still holds.

Beta work lands on `beta/next`, which deploys its own build; `main` deploys the
public site.
