//! The two definitions every v2 pass hangs off: the world state a phase
//! mutates, and the phase unit itself.

use super::*;

/// Everything true about one world mid-build. A phase receives this, changes
/// it, and the next phase sees the result — there is no other channel.
pub(crate) struct WorldState {
    pub world_idx: usize,
    /// Map tiles. Locks are NEVER stamped here — they live in `locks` as an
    /// overlay, matching the route scorer's conditional-edge model (a lock
    /// tile on the grid would read as a plain wall).
    pub grid: Grid,
    /// What occupies each placeable node: level / fortress / pipe / filler.
    pub slots: Vec<SlotAssignment>,
    /// Lock overlay. `fort_section` pairs each lock to the fortress that
    /// opens it.
    pub locks: Vec<LockAssignment>,
    /// Teleport pipe endpoint pairs.
    pub pipe_pairs: Vec<TeleportEdge>,
    /// The START tile, if the grid has one.
    pub start: Option<Pos>,
    /// The world goal (airship dock / Bowser's castle), if present.
    pub target: Option<Pos>,
    /// Positions no phase may claim (floating map sprites, pinned toad
    /// houses, airship/Bowser tiles) — input from the shared pickup/catalog
    /// phases, constant across the build.
    pub fixed: HashSet<Pos>,
    /// What each phase did, in run order — the build's own story, read by
    /// the metrics harness and by per-feature breakdowns.
    pub log: Vec<PhaseReport>,
}

impl WorldState {
    /// Number of fortress slots — also the number of lock sections.
    pub(crate) fn fort_count(&self) -> usize {
        self.slots
            .iter()
            .filter(|s| s.kind == SlotKind::Fortress)
            .count()
    }

    /// View this state as a `BuiltWorld` so the shipping builder's route
    /// scorer (and every census metric built on it) can measure it. This is
    /// the "same measuring stick" bridge — v2 never re-implements scoring.
    pub(crate) fn to_built(&self) -> BuiltWorld {
        BuiltWorld {
            world_idx: self.world_idx,
            grid: self.grid.clone(),
            slots: self.slots.clone(),
            locks: self.locks.clone(),
            section_count: self.fort_count(),
            pipe_pairs: self.pipe_pairs.clone(),
            hb_sprites: Vec::new(),
        }
    }
}

/// What one phase did to the world, in plain words. Starts as free-form
/// action lines; structured fields get added when a real consumer needs them.
pub(crate) struct PhaseReport {
    pub phase: &'static str,
    pub actions: Vec<String>,
}

/// One placement pass. Implementations should be small: read the state,
/// make one kind of decision, report it.
pub(crate) trait Phase {
    /// Short name shown in logs and metrics tables.
    fn name(&self) -> &'static str;
    /// Do the work. The report is also pushed onto `state.log` by
    /// [`run_schedule`], so a phase only returns it.
    fn run(&self, state: &mut WorldState, rng: &mut dyn RngCore) -> PhaseReport;
}

/// Run phases in the order given. The schedule is a plain slice: reordering,
/// omitting, or repeating a phase is entirely the caller's choice.
pub(crate) fn run_schedule(
    state: &mut WorldState,
    schedule: &[&dyn Phase],
    rng: &mut dyn RngCore,
) {
    for phase in schedule {
        let report = phase.run(state, rng);
        state.log.push(report);
    }
}
