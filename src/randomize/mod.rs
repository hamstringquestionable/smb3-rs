pub mod anchor_visuals;
pub mod antechambers;
pub mod autoscroll;
pub mod beta_tornado;
pub mod big_q_rooms;
pub mod bowser_castle;
/// World-maze phase 1: the packed per-world completion-bit storage the
/// two-world swap in [`world_persist`] has to become. Reached on both targets:
/// `randomize_inner` applies it whenever `world_maze` is set, and the web app
/// offers that option.
pub mod completion_bits;
pub mod credits;
pub mod enemies;
pub mod enemy_protections;
pub mod fire_flower;
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
/// World-maze: map objects a world has already lost stay lost. `Map_Init`
/// rebuilds all nine of a world's object slots from ROM on every entry, so
/// without this a beaten Hammer Bro is standing there again when you come back.
pub mod map_objects;
pub mod map_walker;
/// World maze: the generator. Eight `WorldState`s, the cross-world edge set,
/// the fixpoint that decides whether the result is winnable, and the passes
/// that shape it. See `docs/world_maze_design.md`.
pub mod maze;
/// The world maze's state map, in the cartridge WRAM SMB3 already carries —
/// the one place its SRAM addresses are decided. Not battery-backed: nothing
/// sets the iNES battery bit, so this survives a reset, not a power-off.
pub mod maze_state;
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
/// The world maze's goal gate: a wall on World 8's bridge that stands until
/// the player holds K of the seven wands, plus the counter that the wands are
/// counted in. See `docs/world_maze_design.md`, "The wand gate".
pub mod wand_gate;
pub mod world_order;
/// World-maze persistence: a world you leave is the world you come back to.
/// Applied by `randomize_inner` on both targets whenever `world_maze` is set —
/// it was `testrom`-only while the mode was still a POC.
pub mod world_persist;
/// World-maze fast travel: the warp whistle hops between worlds the player has
/// already stood on the start tile of. Its SRAM map is [`maze_state`]'s.
pub mod world_travel;
