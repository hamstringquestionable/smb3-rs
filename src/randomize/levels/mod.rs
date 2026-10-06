//! Level data: rewrites that change what is inside a level, as opposed to
//! where the level sits on the map (`crate::randomize::overworld`).

// --- Room and entry shuffles ---
pub(crate) mod antechambers;
pub(crate) mod big_q_rooms;

// --- Treasure rooms that need their own enemy streams ---
pub(crate) mod hand_rooms;
pub(crate) mod piranha_rooms;

// --- Composed or rewritten sub-areas ---
/// Autoscroll removal: pre-baked replacement level data for the airships.
pub mod autoscroll;
pub(crate) mod beta_tornado;
pub(crate) mod bowser_castle;
pub(crate) mod podoboo_gauntlet;

// --- Shared machinery ---
pub(crate) mod segment_writer;
