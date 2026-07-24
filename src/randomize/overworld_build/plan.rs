//! The per-world plan: one sampled `WorldPlan` carries the progression
//! archetype AND every scoring knob the placement passes consume.
//!
//! This is the single place to look to understand what a world will try to
//! be. Placement passes never branch on the archetype — they only read their
//! own knob struct (`plan.fort`, `plan.level`, `plan.lock`, `plan.pipe`), so
//! an archetype's entire personality is expressed as data in
//! [`WorldPlan::sample`], and adding a new archetype never touches placement
//! code.
//!
//! Two kinds of "awareness" flow through the pipeline:
//! - awareness of the PAST (things already placed) — passes read real state;
//! - awareness of the FUTURE (things not placed yet) — passes read this plan.
//!
//! Feasibility: knobs are soft biases and can never fail. The enforcing
//! passes (locks realizing `roles`, spare pipes realizing `pipe.fort_skip`)
//! apply hard filters with explicit fallbacks — geography that can't host
//! the sampled shape degrades the shape, never completability.

use super::*;

/// The world's sampled progression archetype. Stored on the plan for
/// diagnostics and census; placement passes must NOT branch on it — its
/// effects live entirely in `roles` and the scoring knobs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Archetype {
    /// One fort (or one designated fort) gates the goal; other forts are
    /// safe decoys/inerts.
    SingleGate,
    /// Each fort gates the next; the last gates the goal. The classic
    /// beat-fort-to-advance grind — valued, but not meant to dominate.
    Chain,
    /// The first `fort_count - k` forts chain, then a `k`-way terminal fork:
    /// one GoalGate + `k - 1` accessible decoys ("pick the right fort").
    Fork {
        /// Terminal fork width, capped at 3 (a 4-way fork reads as a field
        /// of fakes).
        k: usize,
    },
}

/// The role a fortress's lock plays in the world's progression archetype.
/// Derived per-fort from the archetype in [`WorldPlan::sample`] and realized
/// by `place_locks` as a hard candidate filter (with feasibility fallback).
#[derive(Clone, Debug)]
pub(crate) enum LockRole {
    /// Lock must gate (make unreachable) every fort section in `targets` — a
    /// chain link (gate the next fort) or the prefix→fork entrance (gate the
    /// whole terminal group at once).
    ChainLink { targets: Vec<usize> },
    /// Lock must gate the airship/Bowser target while stranding no fortress.
    GoalGate,
    /// Lock must gate nothing important (no fort, no target) — the fortress
    /// stays reachable. Decoy and inert forts.
    Safe,
}

/// Whether the spare-pipe pass may realize a fort-skip (a pipe bridging
/// across one closed lock, making that fort optional).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FortSkipPolicy {
    /// No pipe may bypass any mandatory fort — pairs that would are rejected.
    // Reason: not yet sampled — every plan currently uses AnyOneFort. This
    // variant is the "plan says no skip" arm the sampler starts choosing when
    // fort-skips become plan-gated (next step); the consuming check in
    // place_spare_pipes already honors it.
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
/// [`LevelScoring`] so archetypes can diverge the two independently.
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

/// Knobs for fortress placement (`populate_sections` phase 1).
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

/// Knobs for level placement (`populate_sections` phase 2).
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
    /// Bonus when the closed lock makes a LATER fort unreachable — the
    /// strong chain-progression signal. Sized to dominate gated-node counts;
    /// sweeps showed 100→50 changes nothing and →0 only modestly reduces
    /// chains (greedy converges to chains regardless — that's what the
    /// archetype roles are for).
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
}

impl Default for LockScoring {
    fn default() -> Self {
        LockScoring {
            blocks_later_fort_bonus: 100,
            blocks_target_bonus: 10,
            spread_radius: 8,
            spread_penalty_per_tile: 2,
            bridge_bonus: 1,
            w8_bridge_bonus: 8,
            weak_lock_threshold: 5,
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

/// A complete per-world intent: sampled once before any placement. Every
/// placement pass reads ONLY its own section; the sampler is the ONLY place
/// where an archetype turns into behavior.
#[derive(Clone, Debug)]
pub(crate) struct WorldPlan {
    // Reason: placement passes must NOT branch on the archetype (its effects
    // are the roles + knobs), so nothing reads it yet. It's stored for the
    // diagnostics/census tests to compare sampled intent vs realized topology
    // (plan-realization rate) as those get built out.
    #[allow(dead_code)]
    pub archetype: Archetype,
    /// Per-fort lock roles (index = fort section), derived from the
    /// archetype. Realized as hard filters by `place_locks`.
    pub roles: Vec<LockRole>,
    pub fort: FortScoring,
    pub level: LevelScoring,
    pub lock: LockScoring,
    pub pipe: PipeScoring,
}

/// Top-level progression family: how much of the world's fort content is
/// compulsory. This is the dimension players feel (beat everything / choose /
/// free) and the one the census metrics measure, so sampling weights live at
/// this level — adding a shape to a family never silently shifts how often
/// "you must beat every fort" worlds appear.
// Reason: enum_variant_names — the shared "Forts" postfix is the point: the
// family axis IS "how many forts are compulsory" (all / partial / none), and
// bare All/Partial/None would read as generic quantifiers.
#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Family {
    /// Every fort is required. Shapes: Chain (ordered). (TwinGates — an
    /// unordered tail — is this family's parked second shape.)
    AllForts,
    /// One fort really matters; the rest are decoys. Shapes: SingleGate,
    /// Fork.
    PartialForts,
    /// No fort is required. RESERVED at weight 0 until an open/shortcut
    /// shape exists to populate it.
    NoForts,
}

/// Family weights [AllForts, PartialForts, NoForts] out of 100, W1–W7.
/// NoForts stays 0 until it has a shape; any weight past AllForts falls
/// through to PartialForts.
const FAMILY_WEIGHTS: [u32; 3] = [30, 70, 0];
/// W8 family weights — denser (4 forts), forks stay dominant. AllForts=Chain
/// keeps W8's pre-family 25% chain rate.
const W8_FAMILY_WEIGHTS: [u32; 3] = [25, 75, 0];
/// Within PartialForts: [SingleGate, Fork] out of 100.
const PARTIAL_SPLIT: [u32; 2] = [40, 60];

impl WorldPlan {
    /// Family-first per-world sample → the full plan.
    ///
    /// - 1 fort: SingleGate (the lone fort gates the goal).
    /// - Family rolled from `FAMILY_WEIGHTS` (`W8_FAMILY_WEIGHTS` for W8),
    ///   then the shape within it: AllForts → Chain; PartialForts →
    ///   SingleGate/Fork per `PARTIAL_SPLIT` (W8's Fork is always
    ///   chain-2 → 2-way). A Fork samples terminal width `k` (capped at 3,
    ///   surplus forts chain in front), so a 3-fort Fork is a 50/50 mix of
    ///   chain-1→2-fork and a pure 3-way fork.
    ///
    /// All scoring knobs are currently the uniform defaults — archetypes gain
    /// scoring opinions here (and only here) as sweeps justify them.
    pub(crate) fn sample<R: Rng>(fort_count: usize, world_idx: usize, rng: &mut R) -> Self {
        if fort_count <= 1 {
            return WorldPlan::from_archetype(Archetype::SingleGate, fort_count);
        }

        let weights = if world_idx == 7 { &W8_FAMILY_WEIGHTS } else { &FAMILY_WEIGHTS };
        let roll = rng.random_range(..100u32);
        let family = if roll < weights[0] {
            Family::AllForts
        } else if roll < weights[0] + weights[1] {
            Family::PartialForts
        } else {
            Family::NoForts
        };

        let archetype = match family {
            Family::AllForts => Archetype::Chain,
            // NoForts has no shapes yet (weight 0); fall through to the
            // partial shapes so a nonzero weight can't brick sampling.
            Family::PartialForts | Family::NoForts => {
                if world_idx == 7 {
                    Archetype::Fork { k: 2 }
                } else if rng.random_range(..100u32) < PARTIAL_SPLIT[0] {
                    Archetype::SingleGate
                } else {
                    // 2-fort worlds: a plain 2-way fork. 3-fort worlds mix
                    // 50/50 between chain-1 → 2-fork and a pure 3-way fork.
                    let k = if fort_count == 2 {
                        2
                    } else {
                        2 + rng.random_range(..2u32) as usize
                    };
                    Archetype::Fork { k }
                }
            }
        };

        WorldPlan::from_archetype(archetype, fort_count)
    }

    /// Derive the full plan (roles + knobs) from a decided archetype.
    pub(crate) fn from_archetype(archetype: Archetype, fort_count: usize) -> Self {
        let roles = match (fort_count, archetype) {
            (0, _) => Vec::new(),
            (1, _) => vec![LockRole::GoalGate],
            (n, Archetype::Chain) => (0..n)
                .map(|i| {
                    if i + 1 < n {
                        LockRole::ChainLink { targets: vec![i + 1] }
                    } else {
                        LockRole::GoalGate
                    }
                })
                .collect(),
            (n, Archetype::SingleGate) => (0..n)
                .map(|i| if i + 1 == n { LockRole::GoalGate } else { LockRole::Safe })
                .collect(),
            (n, Archetype::Fork { k }) => fork_roles(n, k),
        };

        WorldPlan {
            archetype,
            roles,
            fort: FortScoring::default(),
            level: LevelScoring::default(),
            lock: LockScoring::default(),
            pipe: PipeScoring::default(),
        }
    }

    /// The retry-path plan: every lock Safe (used with `force_safe` when no
    /// world produced a secret-exit-safe lock), default knobs.
    pub(crate) fn all_safe(fort_count: usize) -> Self {
        let mut plan = WorldPlan::from_archetype(Archetype::SingleGate, fort_count);
        plan.roles = vec![LockRole::Safe; fort_count];
        plan
    }
}

/// Seed-level invariant: at least one lock role somewhere in the seed must
/// be Safe. Without it, a seed can sample Chain everywhere (plus 1-fort
/// GoalGates) — zero Safe roles — and the only escape hatch is the
/// post-build force_safe retry, which flattens a whole world to all-Safe
/// (measured: ~1.4% of seeds). Downgrading up front is shape-preserving:
/// the LAST ChainLink of one random Chain world becomes Safe (its chain
/// shortens by one link; that fort becomes a decoy). Runs after all 8 plans
/// are sampled, before any placement.
pub(super) fn ensure_seed_safe_role<R: Rng>(plans: &mut [WorldPlan], rng: &mut R) {
    let has_safe = plans
        .iter()
        .any(|p| p.roles.iter().any(|r| matches!(r, LockRole::Safe)));
    if has_safe {
        return;
    }
    // No Safe anywhere ⇒ every multi-fort world rolled Chain (multi-fort
    // SingleGate and Fork always carry Safes), so ChainLink roles exist.
    let candidates: Vec<usize> = plans
        .iter()
        .enumerate()
        .filter(|(_, p)| p.roles.iter().any(|r| matches!(r, LockRole::ChainLink { .. })))
        .map(|(i, _)| i)
        .collect();
    let Some(&wi) = candidates.choose(rng) else {
        return;
    };
    let plan = &mut plans[wi];
    if let Some(idx) = plan
        .roles
        .iter()
        .rposition(|r| matches!(r, LockRole::ChainLink { .. }))
    {
        plan.roles[idx] = LockRole::Safe;
    }
}

/// Chain the first `n - k` forts, then a `k`-way fork (one GoalGate + `k-1`
/// Safe decoys). `k` is capped at 3 and at `n`.
fn fork_roles(n: usize, k: usize) -> Vec<LockRole> {
    let k = k.min(n).min(3);
    let prefix = n - k;
    let mut roles = Vec::with_capacity(n);
    for i in 0..prefix {
        let targets = if i + 1 < prefix {
            vec![i + 1]
        } else {
            // Last prefix link opens the whole terminal fork as a unit.
            (prefix..n).collect()
        };
        roles.push(LockRole::ChainLink { targets });
    }
    for i in prefix..n {
        roles.push(if i == n - 1 { LockRole::GoalGate } else { LockRole::Safe });
    }
    roles
}
