//! Pool-assignment data structures shared across the writer steps.

use super::*;

/// A concrete assignment of a pool entry to a grid position.
///
/// `pub(crate)` alongside [`WorldAssignments`]: the maze's reality census
/// reads the deal the writer made and compares it against the model the item
/// layer solved.
#[derive(Clone, Debug)]
pub(crate) struct Assignment {
    /// Index into `pickup.pool`.
    pub(crate) pool_idx: usize,
    /// Target grid position.
    pub(crate) pos: (usize, usize),
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
pub(crate) struct WorldAssignments {
    /// Fortress assignments, ordered by section (for FX ordinal computation).
    pub(crate) fortress: Vec<Assignment>,
    /// Level assignments.
    pub(crate) level: Vec<Assignment>,
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
    /// Slots the item layer marked with a `requires` the deck could not
    /// satisfy, as `(position, requirement index)`.
    ///
    /// **Not harmless, and the earlier note here saying so was wrong.** The
    /// gate on that cell does open for free, which is the decorative half —
    /// but the dispenser gate in the ROM keys off the *level*, not off the
    /// cell, so the named level is still unbeatable without its item wherever
    /// the deck put it instead. An unmet mark therefore moves a wall to a cell
    /// the model never saw and put no key in front of.
    ///
    /// Both halves of the deal now avoid it rather than tolerate it: level
    /// marks are dealt globally before any world draws, and fortress marks
    /// keep clear of the slot 1-F is pre-assigned to. This stays because
    /// `friendlier_levels`, `deja_vu` and the top-up still reshape the deck
    /// after the mark was made, and `maze::tests::item_layer_reality_census`
    /// is what watches the number.
    // Reason: written on the shipping path, read by that census.
    #[allow(dead_code)]
    pub(crate) unmet_requirements: Vec<((usize, usize), usize)>,
}
