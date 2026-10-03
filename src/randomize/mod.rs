//! Everything the randomizer can change, grouped by what part of the game it
//! touches. `crate::pipeline` decides the order these run in; this tree is
//! only where they live.

// Shared ROM constants, the free-space registry, and the 6502 patch checker.
pub mod rom_data;

// Palettes, the title screen, credits, king quotes: how the game looks and reads.
pub mod cosmetic;
// Enemy swaps and protections, plus enemy and boss behaviour patches.
pub mod enemies;
// Item tables, ? block contents, and power-up behaviour.
pub mod items;
// What is inside a level: room shuffles, treasure rooms, composed sub-areas.
pub mod levels;
// World maze: the generator (eight `WorldState`s, the cross-world edge set,
// the winnability fixpoint, the shaping passes) and everything the mode
// installs in the ROM. See `docs/world_maze_design.md`.
pub mod maze;
// The world maps: the builder pipeline (catalog -> pickup -> build -> write),
// locks, world order, and the airship shuffle.
pub mod overworld;
// Quality-of-life and always-on engine fixes, grouped by subject.
pub mod qol;
