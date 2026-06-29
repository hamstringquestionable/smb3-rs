//! Build-phase data structures shared across the pipeline steps.

use super::*;

/// Read-only Phase 1 + 2 outputs that build and writer phases consume together.
/// Both fields are produced by earlier phases and never mutated downstream —
/// bundling them avoids threading two parallel references through every helper.
pub(crate) struct OverworldData<'a> {
    pub pickup: &'a PickupResult,
    pub catalog: &'a NodeCatalog,
}

/// Feature flags consumed by the build phase. Construct exhaustively in
/// production so a new flag forces a conscious wire-up; in tests use
/// `BuildFlags { ..Default::default() }` so adding a flag leaves them untouched.
#[derive(Copy, Clone, Default)]
pub(crate) struct BuildFlags {
    pub shuffle_toad_houses: bool,
    pub eights_are_wild: bool,
    pub shuffle_hammer_bros: bool,
}

/// What kind of node occupies a grid slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlotKind {
    Level,
    Fortress,
    Pipe,
    HammerBro,
    BonusGame,
    ToadHouse,
}

/// A single slot assignment on the grid.
#[derive(Clone, Debug)]
pub struct SlotAssignment {
    pub pos: (usize, usize),
    pub kind: SlotKind,
    /// Which section (0-based) this slot belongs to.
    pub section: usize,
    /// When true, the writer stamps a HANDTRAP tile (0xE6) at this slot
    /// instead of a level-number tile. Only set on `SlotKind::Level` slots.
    pub is_hand_trap: bool,
    /// When true, the writer stamps a PIPE tile (0xBC) at this slot instead
    /// of a level-number tile. Only set on `SlotKind::Level` slots. The
    /// slot's level pointer entry is unchanged; pressing A on the pipe-look
    /// tile drops the player into the underlying level (uniform Map_Op = $10
    /// dispatch — no pipe-transit state).
    pub is_troll_pipe: bool,
}

/// A lock/bridge placed on a path tile.
#[derive(Clone, Debug)]
pub(crate) struct LockAssignment {
    /// Path tile position where the lock goes.
    pub pos: (usize, usize),
    /// The blocking tile to write (0x54 vert lock, 0x56 horiz lock, 0xE4 sky lock, 0x9D water gap).
    pub gap_tile: u8,
    /// The original path tile (for FX restore).
    pub replace_tile: u8,
    /// Which fortress (section index) opens this lock.
    pub fort_section: usize,
    /// True if the world's target (airship/Bowser) is still reachable with
    /// this lock closed. These locks are safe for 1-F (secret exit doesn't
    /// trigger FX replacement).
    pub secret_exit_safe: bool,
    /// True if this lock makes the target unreachable when closed. Used to
    /// suppress redundant target-blocking bonuses for subsequent locks in
    /// the same world (avoids piling multiple locks against the airship).
    pub blocks_target: bool,
}

/// A redistributed wandering Hammer Bro sprite decided in the build phase.
/// The grid position is one of this world's `HammerBro` slot tiles; the writer
/// stamps it into a free map-object slot in the ROM tables.
#[derive(Clone, Debug)]
pub(crate) struct HbSprite {
    /// Grid position where the roaming sprite spawns.
    pub grid_pos: (usize, usize),
    /// Reward item granted for clearing the encounter (Global Item ID).
    pub reward: u8,
}

/// Complete build result for one world.
#[derive(Clone, Debug)]
pub(crate) struct BuiltWorld {
    #[allow(dead_code)] // read in tests
    pub world_idx: usize,
    /// The grid with pipes placed (but no forts/levels/locks stamped yet).
    pub grid: Grid,
    /// Slot assignments for placeable nodes.
    pub slots: Vec<SlotAssignment>,
    /// Lock/bridge assignments.
    pub locks: Vec<LockAssignment>,
    /// Number of sections (= number of fortresses in this world).
    pub section_count: usize,
    /// Pipe pair positions placed in this world: Vec of (endpoint_a, endpoint_b).
    pub pipe_pairs: Vec<TeleportEdge>,
    /// Redistributed wandering Hammer Bro sprites for this world. Empty when
    /// `shuffle_hammer_bros` is off (the writer keeps the vanilla sprites).
    pub hb_sprites: Vec<HbSprite>,
}

/// Complete Phase 3 output.
#[derive(Clone)]
pub(crate) struct BuildResult {
    pub worlds: Vec<BuiltWorld>,
    /// Fortress counts per world (decided in Step 0).
    #[allow(dead_code)] // read in tests
    pub fort_counts: [usize; 8],
}

/// Output of [`prepare_capacities`]: the per-world grids the builder walks, the
/// fixed-position sets, and the derived level capacity per world.
pub(super) struct CapacityPrep {
    pub(super) patched_grids: Vec<Grid>,
    pub(super) fixed_positions: Vec<HashSet<(usize, usize)>>,
    pub(super) capacities: [usize; 8],
}

/// Per-world numeric budgets passed into `build_world`. All five fields are
/// computed in `build()` from pickup capacity, vanilla pipe counts, and the
/// redistributed fortress counts.
pub(super) struct WorldSlotCounts {
    pub(super) fort_count: usize,
    pub(super) level_count: usize,
    pub(super) pipe_pair_count: usize,
    pub(super) max_non_pipe_slots: usize,
    pub(super) force_safe: bool,
}
