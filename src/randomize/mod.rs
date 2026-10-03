pub mod anchor_visuals;
pub mod antechambers;
pub mod autoscroll;
pub mod beta_tornado;
pub mod big_q_rooms;
pub mod bowser_castle;
pub mod credits;
pub mod enemies;
pub mod enemy_protections;
pub mod fire_flower;
/// The two level-geometry spots where an unwanted Frog Suit strands the
/// player. Applied only with Random Fire Flower on, since that is the only way
/// to arrive there in a frog. See [`fire_flower`].
pub mod frog_softlocks;
pub mod hand_rooms;
pub mod hands_levels;
pub mod items;
pub mod king_quotes;
pub mod koopalings;
pub mod level_helpers;
pub mod levels;
/// Every lock a fortress opens, keyed by where the player is standing rather
/// than by a slot index. Replaces vanilla's fortress-FX tables outright, and
/// absorbs what used to be a second, differently-keyed cross-world mechanism.
pub mod lock_keys;
pub mod map_walker;
/// World maze: the generator (eight `WorldState`s, the cross-world edge set,
/// the winnability fixpoint, the shaping passes) and everything the mode
/// installs in the ROM. See `docs/world_maze_design.md`.
pub mod maze;
pub mod node_catalog;
pub mod overworld_build;
pub mod overworld_helpers;
pub mod overworld_pickup;
pub mod overworld_writer;
pub mod palette_variants;
pub mod palettes;
pub mod pipe_helpers;
pub mod piranha_rooms;
pub mod podoboo_gauntlet;
pub mod poison_mushroom;
pub mod powerups;
pub mod qol;
pub mod rom_data;
pub mod segment_writer;
pub mod start_airship_swap;
pub mod stomp_fairness;
pub mod title_screen;
pub mod troll_pipes;
/// Retires the 2-player Vs Challenge, whose trigger is unsound once the two
/// players can be in different worlds and which bypassed the map's
/// tile-enterability rules. Unconditional, and it frees 339 bytes of PRG030.
pub mod two_player_vs;
pub mod water_stomp;
pub mod world_order;
