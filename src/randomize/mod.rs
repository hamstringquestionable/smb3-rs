pub mod anchor_visuals;
pub mod antechambers;
pub mod autoscroll;
pub mod beta_tornado;
pub mod big_q_rooms;
pub mod bowser_castle;
/// World-maze phase 1: the packed per-world completion-bit storage the
/// two-world swap in [`world_persist`] has to become. Native-only for the same
/// reason as that module — nothing on wasm reaches it yet.
pub mod completion_bits;
pub mod credits;
pub mod enemies;
pub mod enemy_protections;
pub mod fire_flower;
/// World-maze cross-world locks: a fortress in one world that busts a lock in
/// another, by setting its completion bit in [`completion_bits`]' packed store.
pub mod foreign_locks;
pub mod hand_rooms;
pub mod hands_levels;
pub mod items;
pub mod king_quotes;
pub mod koopalings;
pub mod level_helpers;
pub mod levels;
pub mod map_walker;
/// World maze: the generator. Eight `WorldState`s, the cross-world edge set,
/// the fixpoint that decides whether the result is winnable, and the passes
/// that shape it. See `docs/world_maze_design.md`.
pub mod maze;
/// The world maze's battery-backed state map — the one place its SRAM
/// addresses are decided. Native-only for the same reason as [`world_persist`]:
/// nothing on wasm reaches it yet.
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
/// World-maze persistence POC. Native-only for the same reason as
/// [`crate::testrom`]: nothing but `testrom` applies it, so on wasm the whole
/// module is dead and CI's wasm clippy pass says so.
pub mod world_persist;
/// World-maze fast travel: the warp whistle hops between worlds the player has
/// already stood on the start tile of. Native-only because its SRAM map is
/// [`maze_state`]'s, which is native-only for the same reason.
pub mod world_travel;
