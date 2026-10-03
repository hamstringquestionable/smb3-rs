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
pub(crate) mod build;
pub(crate) mod node_catalog;
pub(crate) mod pickup;
pub(crate) mod writer;

// --- Passes that edit the build model before the writer ---
pub(crate) mod hands_levels;
pub(crate) mod start_airship_swap;
pub(crate) mod troll_pipes;

// --- Shuffles and tables written outside the builder ---
/// Airship shuffle: the one cross-world level shuffle that is still
/// independent of the overworld builder.
pub(crate) mod airship_shuffle;
// Every lock a fortress opens, keyed by where the player is standing rather
// than by a slot index. Replaces vanilla's fortress-FX tables outright, and
// absorbs what used to be a second, differently-keyed cross-world mechanism.
pub(crate) mod lock_keys;
pub(crate) mod world_order;

// --- Shared machinery ---
pub(crate) mod helpers;
mod level_helpers;
pub(crate) mod map_walker;
pub(crate) mod pipe_helpers;
