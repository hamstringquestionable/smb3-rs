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

/// Stamp assigned slots onto a grid so `walk_map` sees them as nodes.
/// Pipes are already stamped on the build grid and HammerBro slots stay
/// blank path tiles, so neither needs a stamp. Locks are NOT stamped —
/// callers model them separately (test grids / conditional edges).
pub(crate) fn stamp_slots(grid: &mut Grid, slots: &[SlotAssignment]) {
    for slot in slots {
        match slot.kind {
            SlotKind::Fortress => grid.set(slot.pos.0, slot.pos.1, TILE_FORTRESS),
            SlotKind::Level
                if BACKGROUND_TILES.contains(&grid.get(slot.pos.0, slot.pos.1)) =>
            {
                grid.set(slot.pos.0, slot.pos.1, TILE_NODE);
            }
            SlotKind::BonusGame => grid.set(slot.pos.0, slot.pos.1, TILE_BONUS_GAME),
            SlotKind::ToadHouse => grid.set(slot.pos.0, slot.pos.1, TILE_TOAD_HOUSE),
            // Pipe: already stamped; HammerBro: stays a blank path tile;
            // Level on a non-background tile: keep the existing tile.
            _ => {}
        }
    }
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
    /// The C1 floor this world was built to (see `deal_c1_floors`). Carried
    /// out of the build so the census can score each world against its OWN
    /// floor — a global comparison would read a dealt 11 as a failure and a
    /// dealt 17 as a pass it never had to earn.
    #[allow(dead_code)] // read in tests
    pub c1_floor: u32,
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
// Reason: production consumes only `capacities` (via allot_budgets); the
// grids and fixed sets are read by the distribution-tuning tests, which must
// compute capacity exactly as production does.
#[allow(dead_code)]
pub(crate) struct CapacityPrep {
    pub(super) patched_grids: Vec<Grid>,
    pub(super) fixed_positions: Vec<HashSet<(usize, usize)>>,
    pub(crate) capacities: [usize; 8],
}
