//! The world maps: which level sits on which tile, how the tiles connect, and
//! the locks that gate them.
//!
//! The builder is a four-phase pipeline, run by `pipeline::stages` in this
//! order: [`node_catalog`] classifies all 340 pointer-table entries,
//! [`pickup`] clears the maps and pools what was on them, [`build`] decides the
//! new layout as a model, and [`writer`] commits it to the ROM in one pass.
//! The world maze, when on, runs between `build` and `writer` — see
//! `crate::randomize::maze`.

// --- The builder pipeline: catalog -> pickup -> build -> write ---
pub mod build;
pub mod node_catalog;
pub mod pickup;
pub mod writer;

// --- Passes that edit the build model before the writer ---
pub mod hands_levels;
pub mod start_airship_swap;
pub mod troll_pipes;

// --- Shuffles and tables written outside the builder ---
/// Airship shuffle: the one cross-world level shuffle that is still
/// independent of the overworld builder.
pub mod airship_shuffle;
// Every lock a fortress opens, keyed by where the player is standing rather
// than by a slot index. Replaces vanilla's fortress-FX tables outright, and
// absorbs what used to be a second, differently-keyed cross-world mechanism.
pub mod lock_keys;
pub mod world_order;

// --- Shared machinery ---
pub mod helpers;
pub mod level_helpers;
pub mod map_walker;
pub mod pipe_helpers;
