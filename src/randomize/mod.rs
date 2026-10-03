pub mod anchor_visuals;
pub mod credits;
pub mod enemies;
pub mod fire_flower;
/// The two level-geometry spots where an unwanted Frog Suit strands the
/// player. Applied only with Random Fire Flower on, since that is the only way
/// to arrive there in a frog. See [`fire_flower`].
pub mod frog_softlocks;
pub mod items;
pub mod king_quotes;
/// World maze: the generator (eight `WorldState`s, the cross-world edge set,
/// the winnability fixpoint, the shaping passes) and everything the mode
/// installs in the ROM. See `docs/world_maze_design.md`.
pub mod levels;
pub mod maze;
pub mod overworld;
pub mod palette_variants;
pub mod palettes;
pub mod poison_mushroom;
pub mod powerups;
pub mod qol;
pub mod rom_data;
pub mod title_screen;
/// Retires the 2-player Vs Challenge, whose trigger is unsound once the two
/// players can be in different worlds and which bypassed the map's
/// tile-enterability rules. Unconditional, and it frees 339 bytes of PRG030.
pub mod two_player_vs;
