//! Level data: rewrites that change what is inside a level, as opposed to
//! where the level sits on the map (`crate::randomize::overworld`).

// --- Room and entry shuffles ---
pub mod antechambers;
pub mod big_q_rooms;

// --- Treasure rooms that need their own enemy streams ---
pub mod hand_rooms;
pub mod piranha_rooms;

// --- Composed or rewritten sub-areas ---
/// Autoscroll removal: pre-baked replacement level data for the airships.
pub mod autoscroll;
pub mod beta_tornado;
pub mod bowser_castle;
pub mod podoboo_gauntlet;

// --- Shared machinery ---
pub mod segment_writer;
