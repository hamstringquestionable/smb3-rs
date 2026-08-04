//! Shared ROM constants, data structures, and helpers for SMB3 randomization.
//!
//! This module holds all the shared knowledge about the ROM layout — constants,
//! lookup tables, data structures, and low-level read/write helpers — used by
//! multiple randomization modules. The BFS map walker lives in `map_walker.rs`.
//!
//! Split into submodules: `free_space` (allocation registry), `tables` (static
//! data), `grid` (the overworld grid), `access` (typed read/write helpers),
//! `tiles` (overworld tile classification — the single producer for "which
//! bytes mean what"). All items are re-exported flat so callers keep using
//! `rom_data::ITEM`.

use crate::rom::Rom;

mod access;
mod free_space;
mod grid;
mod tables;
mod tiles;

pub(crate) use access::*;
pub(crate) use free_space::*;
pub(crate) use grid::*;
pub(crate) use tables::*;
pub(crate) use tiles::*;

// Part of the lib's public API (consumed by the chr_stats integration test).
pub use access::{ENEMY_DATA_END, ENEMY_DATA_START};
