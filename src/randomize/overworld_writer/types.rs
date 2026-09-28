//! Pool-assignment data structures shared across the writer steps.

use super::*;

/// A pin the deck could not satisfy: the slot grid position that carried it,
/// and the `(world_idx, entry_idx)` it named.
///
/// Two same-shaped pairs meaning different things is exactly why this is named
/// rather than spelled out at each use.
pub(super) type UnmetPin = ((usize, usize), (usize, usize));

/// A concrete assignment of a pool entry to a grid position.
#[derive(Clone, Debug)]
pub(super) struct Assignment {
    /// Index into `pickup.pool`.
    pub(super) pool_idx: usize,
    /// Target grid position.
    pub(super) pos: (usize, usize),
}

/// Pipe pair assignment: two pool entries, a dest_idx, and two positions.
#[derive(Clone, Debug)]
pub(super) struct PipeAssignment {
    pub(super) pool_idx_a: usize,
    pub(super) pool_idx_b: usize,
    pub(super) dest_idx: usize,
    pub(super) pos_a: (usize, usize),
    pub(super) pos_b: (usize, usize),
}

/// Hammer bro assignment: carries its own LevelEntry from the cycling pool.
#[derive(Clone, Debug)]
pub(super) struct HammerBroAssignment {
    /// Target grid position.
    pub(super) pos: (usize, usize),
    /// Level data from the cycling hammer bro level pool.
    pub(super) level_entry: rom_data::LevelEntry,
}

/// All assignments for one world.
pub(super) struct WorldAssignments {
    /// Fortress assignments, ordered by section (for FX ordinal computation).
    pub(super) fortress: Vec<Assignment>,
    /// Level assignments.
    pub(super) level: Vec<Assignment>,
    /// Pipe pair assignments.
    pub(super) pipes: Vec<PipeAssignment>,
    /// Airship assignment (W1-W7 only).
    pub(super) airship: Option<Assignment>,
    /// Bowser assignment (W8 only).
    pub(super) bowser: Option<Assignment>,
    /// Bonus game (spade) assignments.
    pub(super) bonus: Vec<Assignment>,
    /// Toad House assignments (each preserves its vanilla obj_ptr / reward variant).
    pub(super) toad: Vec<Assignment>,
    /// Hammer bro assignments (remaining blank slots).
    pub(super) hammer_bro: Vec<HammerBroAssignment>,
    /// Positions of slots that were marked as troll pipes in `build` but could
    /// not be filled with a non-hand-level entry from the pool. They are
    /// demoted to regular level tiles at tile-stamping time so the player
    /// sees a normal level icon rather than a pipe leading to a hand-trap.
    pub(super) demoted_troll_pipes: HashSet<(usize, usize)>,
    /// Slots whose [`pin`](crate::randomize::overworld_build::SlotAssignment)
    /// the deck could not satisfy, as `(position, (world_idx, entry_idx))`.
    ///
    /// Empty on every seed that pins nothing, which today is all of them. It
    /// exists because an unmet pin is **not** the same as no pin: the named
    /// entry is still dealt, just somewhere the model did not choose, and a
    /// pass that assumed otherwise would be reasoning about a map that does
    /// not exist.
    // Reason: written on the shipping path; the readers are the tests, and the
    // model passes that will consult it once something pins.
    #[allow(dead_code)]
    pub(super) unmet_pins: Vec<UnmetPin>,
}
