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
    /// Hard limit on pipe pairs in this world — currently the vanilla
    /// per-world count, a chosen design budget (not an inherent ROM limit;
    /// it may be raised later). All pipe-placing phases share it: whatever
    /// connectivity doesn't spend, routing may. Every world's map needs
    /// fewer pipes than its budget to fully connect, so the bound also stops
    /// a bug from spraying pipes (e.g. 8 into W2) instead of looping.
    pub pipe_budget: usize,
    /// How many action levels the Levels phase places — a chosen per-world
    /// count, seeded from the vanilla catalog's Level tally. Loader sources
    /// (`from_built`/`from_vanilla`) set it to what's already on the map.
    pub level_budget: usize,
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

    /// Blanks a phase may claim for new content: blank tile, not `fixed`,
    /// not already a slot, and not the row-7/8 completion-bit partner of
    /// existing content (rows 7 and 8 share one completion bit per column —
    /// a ROM mechanic; stacked completable content marks each other beaten).
    pub(crate) fn legal_blanks(&self) -> Vec<Pos> {
        let taken: HashSet<Pos> = self.slots.iter().map(|s| s.pos).collect();
        let barred: HashSet<Pos> = self
            .slots
            .iter()
            .filter_map(|s| row78_partner(s.pos))
            .collect();
        let mut out = Vec::new();
        for r in 0..self.grid.rows() {
            for c in 0..self.grid.cols {
                let pos = (r, c);
                if rom_data::VALID_BLANK_TILES.contains(&self.grid.get(r, c))
                    && !self.fixed.contains(&pos)
                    && !taken.contains(&pos)
                    && !barred.contains(&pos)
                {
                    out.push(pos);
                }
            }
        }
        out
    }

    /// Stamp a teleport pipe pair: both tiles become pipes, both positions
    /// become Pipe slots, and the edge joins the pair list. One writer so
    /// grid, slots, and pairs can never drift apart.
    pub(crate) fn add_pipe_pair(&mut self, a: Pos, b: Pos) {
        for pos in [a, b] {
            self.grid.set(pos.0, pos.1, TILE_PIPE);
            self.slots.push(SlotAssignment {
                pos,
                kind: SlotKind::Pipe,
                section: 0,
                is_hand_trap: false,
                is_troll_pipe: false,
            });
        }
        self.pipe_pairs.push((a, b));
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

/// The cell sharing `pos`'s completion bit: (8,c) for (7,c) and vice versa.
/// `None` for every other row.
pub(crate) fn row78_partner(pos: Pos) -> Option<Pos> {
    match pos.0 {
        7 => Some((8, pos.1)),
        8 => Some((7, pos.1)),
        _ => None,
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
