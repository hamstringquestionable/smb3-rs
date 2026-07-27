//! Scoring knobs for every placement pass, bundled in one [`Knobs`] struct.
//!
//! This is the single place to look for the numeric personality of the
//! builder. Placement passes each read ONLY their own section (`knobs.fort`,
//! `knobs.level`, `knobs.lock`, `knobs.pipe`).
//!
//! Progression shape is NOT decided here: the choice-first builder measures
//! the built route structure (`route_choice`) and shapes forts/locks/pipes
//! against it, so there is no sampled archetype. Knobs are soft biases and
//! can never fail — geometry that resists a bias degrades the aesthetics,
//! never completability.

/// Whether the spare-pipe pass may realize a fort-skip (a pipe bridging
/// across one closed lock, making that fort optional).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FortSkipPolicy {
    /// No pipe may bypass any mandatory fort — pairs that would are rejected.
    // Reason: not yet used — every world currently allows AnyOneFort. This
    // variant is the "no skip" arm for a future per-world roll; the consuming
    // check in place_spare_pipes already honors it.
    #[allow(dead_code)]
    Never,
    /// The first candidate pair that bypasses exactly ONE mandatory fort is
    /// taken (the prize), at most once per world. Pairs bypassing 2+ forts,
    /// or opening the goal with every lock closed, are always rejected
    /// regardless of policy.
    AnyOneFort,
}

/// Spread/density weights shared by the level and fortress scorers
/// (`score_with_weights`). Embedded separately in [`FortScoring`] and
/// [`LevelScoring`] so the two can diverge independently.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SpreadScoring {
    /// Weight on min Manhattan distance to already-placed content
    /// (visual/spatial spread).
    pub manhattan: f64,
    /// Weight on min BFS-distance difference (traversal spread — weighted
    /// above grid distance so spread follows the walk, not the screen).
    pub bfs: f64,
    /// Penalty per already-placed tile within `density_radius`.
    pub density_penalty: f64,
    /// Combined manhattan+BFS distance that counts as "nearby" for the
    /// density penalty.
    pub density_radius: usize,
    /// Cap on each separation metric's contribution, so one huge gap can't
    /// dominate the score.
    pub separation_cap: f64,
}

impl Default for SpreadScoring {
    fn default() -> Self {
        SpreadScoring {
            manhattan: 1.0,
            bfs: 1.5,
            density_penalty: 3.0,
            density_radius: 4,
            separation_cap: 8.0,
        }
    }
}

/// Knobs for fortress placement (the `shape_forts` pass — candidate ranking
/// in the measured phase, softmax sampling in the aesthetic phase).
#[derive(Clone, Copy, Debug)]
pub(crate) struct FortScoring {
    pub spread: SpreadScoring,
    /// Bonus for one-exit positions — fortresses naturally belong at path
    /// termini.
    pub dead_end_bonus: f64,
    /// Bonus for the designated island positions in
    /// `FORTRESS_BONUS_POSITIONS` (isolated spots that rarely win placement
    /// without a boost).
    pub island_bonus: f64,
    /// Softmax temperature. Score range ≈ [-12, +15] including the dead-end
    /// bonus; 4.0 keeps top candidates favored without determinism.
    pub softmax_t: f64,
}

impl Default for FortScoring {
    fn default() -> Self {
        FortScoring {
            spread: SpreadScoring::default(),
            dead_end_bonus: 5.0,
            island_bonus: 0.5,
            softmax_t: 4.0,
        }
    }
}

/// Knobs for level placement (`place_levels`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct LevelScoring {
    pub spread: SpreadScoring,
    /// Bonus for one-exit positions (small — levels only mildly prefer
    /// dead ends).
    pub dead_end_bonus: f64,
    /// Path-relevance weight: positions on the main start→target route (low
    /// detour) score higher. Max bonus = `path_detour_cap * path_bonus`.
    ///
    /// THE dominant linearity lever. Lowered 1.5 → 0.75 to cut back-to-back
    /// forced-level streaks (route bias was gluing levels onto the trunk).
    /// Sweep via test_required_progression: 1.5→streak 2.12, 1.0→1.89,
    /// 0.75→1.79 (≈ the reference randomizer), 0.5→1.70, 0.0→1.34
    /// (overshoots and thins the required route). History: 3.0 clumped hard;
    /// 0.5 was "decorative" at the old weightings.
    pub path_bonus: f64,
    /// Max detour (BFS hops off the shortest start→target route) that still
    /// earns any path bonus.
    pub path_detour_cap: f64,
}

impl Default for LevelScoring {
    fn default() -> Self {
        LevelScoring {
            spread: SpreadScoring::default(),
            dead_end_bonus: 0.5,
            path_bonus: 0.75,
            path_detour_cap: 6.0,
        }
    }
}

/// Knobs for lock placement (`place_locks`). Scores are integer points on a
/// base of "gated node count" (how much of the map the closed lock walls
/// off).
#[derive(Clone, Copy, Debug)]
pub(crate) struct LockScoring {
    /// Bonus when the closed lock makes a LATER fort unreachable. Held at 0:
    /// this bonus is the chain magnet (greedy converges to all-chain worlds
    /// with it high — measured in the archetype era), and the choice-guard
    /// re-measure in `place_locks` is its replacement. Kept as a knob for
    /// sweeps.
    pub blocks_later_fort_bonus: i32,
    /// Bonus for the FIRST lock in a world that gates the airship/Bowser.
    /// Later target-blockers get nothing (they'd pile up next to the
    /// airship).
    pub blocks_target_bonus: i32,
    /// Spread penalty radius: locks closer than this (Manhattan) to an
    /// already-placed lock are penalized...
    pub spread_radius: i32,
    /// ...by this many points per tile of shortfall.
    pub spread_penalty_per_tile: i32,
    /// Nudge toward bridge tiles (water gaps read better than path locks).
    pub bridge_bonus: i32,
    /// W8 only: extra bias toward the screen-3 showcase bridges. Tuned via a
    /// 200k-seed sweep: +8 puts ≥1 bridge out in ~99.6% of seeds, two in
    /// ~30%, all four in ~0.08% (a rare treat).
    pub w8_bridge_bonus: i32,
    /// If the best candidate scores below this, prefer a safe lock instead —
    /// no point spending an impactful-lock slot on a weak chokepoint.
    pub weak_lock_threshold: i32,
    /// Max candidate locks per section re-measured by the choice-guard
    /// before giving up and accepting the top-scored one.
    pub choice_guard_tries: usize,
}

impl Default for LockScoring {
    fn default() -> Self {
        LockScoring {
            blocks_later_fort_bonus: 0,
            blocks_target_bonus: 10,
            spread_radius: 8,
            spread_penalty_per_tile: 2,
            bridge_bonus: 1,
            w8_bridge_bonus: 8,
            weak_lock_threshold: 5,
            choice_guard_tries: 4,
        }
    }
}

/// Knobs for pipe placement — both the connectivity pass (`place_pipes`) and
/// the post-lock spare-pipe pass (`place_spare_pipes`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct PipeScoring {
    /// Softmax temperature for every pipe candidate pick. Score range is
    /// typically ~[-8, +12].
    pub softmax_t: f64,
    /// Connectivity build-outward: Manhattan distance at which an island
    /// blank's frontier-proximity score reaches zero (normalization cap).
    pub frontier_max_dist: f64,
    /// Connectivity build-outward: weight on frontier proximity (nearer
    /// islands are bridged first, growing a chain instead of jumping to the
    /// goal).
    pub frontier_weight: f64,
    /// Per-spare-pipe skip-cap weights: relative chance the pipe's max
    /// level-skip is 1, 2, 3, or 4. Each pipe rolls a cap, then greedily
    /// takes the biggest skip within it — big skips happen sometimes, not
    /// every pipe. Tuned via sweep (≈ halves W2/W6 skip size vs uncapped
    /// while keeping most of the anti-linearity win).
    pub spare_cap_weights: [f64; 4],
    /// Spare-pipe score: points per level the pipe lets the player skip.
    pub level_skip_weight: f64,
    /// Spare-pipe score: points per hop of route-distance jump (tie-breaker
    /// under `level_skip_weight`).
    pub jump_weight: f64,
    /// Whether a spare pipe may deliberately bridge across one closed lock.
    /// See [`FortSkipPolicy`]. Rejection of 2+-fort bypasses and goal-opening
    /// pipes applies under every policy.
    pub fort_skip: FortSkipPolicy,
}

impl Default for PipeScoring {
    fn default() -> Self {
        PipeScoring {
            softmax_t: 4.0,
            frontier_max_dist: 20.0,
            frontier_weight: 5.0,
            spare_cap_weights: [30.0, 25.0, 25.0, 20.0],
            level_skip_weight: 10.0,
            jump_weight: 1.0,
            fort_skip: FortSkipPolicy::AnyOneFort,
        }
    }
}

/// Every placement pass's knobs, bundled. One instance (currently the
/// defaults) is built per randomization and threaded through `build_world`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Knobs {
    pub fort: FortScoring,
    pub level: LevelScoring,
    pub lock: LockScoring,
    pub pipe: PipeScoring,
}
