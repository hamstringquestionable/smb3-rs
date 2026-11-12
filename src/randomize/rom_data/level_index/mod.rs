//! What is inside each level, read from the pointers the engine follows.
//!
//! Every header here was reached the way the engine reaches it: from a world
//! pointer table entry, or from another header's alt pointer. Nothing is found
//! by scanning a level-data region from its start — a region holds filler and
//! orphaned data too, and a scan that steps through commands has no way to
//! notice it has slipped out of alignment.
//!
//! Read-only: it records file offsets so a randomizer module can write through
//! them, but it never writes. The ROM changes as the pipeline runs (headers are
//! repointed and cloned), so build it from the ROM you are about to edit.

mod areas;
mod commands;
mod header;
mod levels;
// Object names are read only by `rominfo` and the tests, both native-only.
#[cfg(not(target_arch = "wasm32"))]
mod object_names;
mod objects;

pub(crate) use areas::{Area, trace};
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use areas::{ChainEnd, DeadAlt};
pub(crate) use commands::{PowerBlock, power_blocks, read_commands};
pub(crate) use header::{LEVEL_HEADER_SIZE, LevelHeader};
pub(crate) use levels::{every_level, every_power_block, shared_headers};
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use object_names::object_name;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use objects::Object;
pub(crate) use objects::read_objects;

#[cfg(test)]
mod tests;
