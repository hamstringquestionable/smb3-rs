//! Locks phase — one lock (map gap) per fortress, marginal-cut-ranked
//! placement.
//!
//! One earned control (census-argued, 2026-07-31): candidates are ranked
//! by MARGINAL CUT — the nodes closing the site severs that already-placed
//! locks don't sever — with bridges as the aesthetic tiebreak and shuffled
//! order under that (see [`rank_candidates`]). The bridge experiment
//! proved lock-site cut power drives every quality metric (zero-gate
//! halved, linearity down); marginal rather than raw cut is what stops the
//! locks piling onto the goal approach — the second lock on a claimed
//! corridor scores only the slice between, and loses to fresh territory.
//! The first candidate that keeps the world completable wins.
//!
//! Hard invariants (true safety, per the charter):
//!
//! - **ORDER-FREE completability** (`WorldState::completable`): close every
//!   lock, beat every reachable fort, open those locks, repeat — the world
//!   is valid iff the goal is reached AND every fort is eventually beatable
//!   (a fort sealed behind its own lock is permanently unplayable content).
//!   This single fixpoint subsumes the shipping builder's per-section hard
//!   rules without its "sections beaten in order" linearization.
//! - **Every fort has a lock.** A fort's Boom-Boom ordinal indexes the world
//!   FX list, and a lockless fort is dead weight (its beat does nothing). If
//!   every candidate breaks completability, the fort is REMOVED from the
//!   world — loudly reported, counted by the census (expected rate ~0: a
//!   dead-end path tile gates nothing and always passes the fixpoint).
//! - **Secret-exit safety is computed, not stamped false.** A lock is safe
//!   iff the world stays completable with that lock sealed forever
//!   (`completable_sealed`) — the 1-F fortress's secret exit skips the
//!   lock-opening FX. The ROM-level requirement is at least one safe lock
//!   ACROSS ALL WORLDS (the writer parks 1-F on a safe slot);
//!   [`ensure_secret_exit_safe`] is that cross-world backstop.

use super::*;

use rand::seq::SliceRandom;

pub(crate) struct Locks;

impl Phase for Locks {
    fn name(&self) -> &'static str {
        "locks"
    }

    fn run(&self, state: &mut WorldState, rng: &mut dyn RngCore) -> PhaseReport {
        let mut actions = Vec::new();

        let fort_ids: Vec<usize> = state
            .slots
            .iter()
            .filter(|s| s.kind == SlotKind::Fortress)
            .map(|s| s.section)
            .collect();

        let mut open = state.grid.clone();
        stamp_slots(&mut open, &state.slots);
        let open_reach = walk_reachable(&open, &state.pipe_pairs, state.start, state.world_idx);
        let mut covered: HashSet<Pos> = HashSet::new();

        for fort_id in fort_ids {
            let candidates = rank_candidates(state, &open, &open_reach, &covered, rng);

            let mut placed = false;
            let tried = candidates.len();
            for (pos, tile, cut) in candidates {
                state.locks.push(LockAssignment {
                    pos,
                    gap_tile: gap_tile_for(tile),
                    replace_tile: tile,
                    fort_section: fort_id,
                    // Stamped by recompute_safety_flags once the set is
                    // final.
                    secret_exit_safe: false,
                });
                if state.completable() {
                    actions.push(format!("lock for fort {fort_id} at {pos:?}"));
                    covered.extend(cut);
                    placed = true;
                    break;
                }
                state.locks.pop();
            }
            if !placed {
                // Invariant: every fort has a lock. An unlockable fort is
                // removed rather than left as dead weight on the map.
                let idx = state
                    .slots
                    .iter()
                    .position(|s| s.kind == SlotKind::Fortress && s.section == fort_id)
                    .expect("fort id came from the slot list");
                let pos = state.slots[idx].pos;
                state.slots.remove(idx);
                actions.push(format!(
                    "fort {fort_id} REMOVED from {pos:?}: all {tried} lock candidates break completability",
                ));
            }
        }

        recompute_safety_flags(state);

        actions.push(format!(
            "done: {}/{} forts locked, {} secret-exit-safe",
            state.locks.len(),
            state.fort_count(),
            state.locks.iter().filter(|l| l.secret_exit_safe).count(),
        ));
        PhaseReport { phase: self.name(), actions }
    }
}

/// Stamp every lock's `secret_exit_safe` flag from the final lock set: safe
/// iff the world stays completable with that lock sealed forever. Computed
/// after all placements — sealing interacts with the OTHER locks, so the
/// flag is only meaningful on the finished set.
pub(crate) fn recompute_safety_flags(state: &mut WorldState) {
    for li in 0..state.locks.len() {
        state.locks[li].secret_exit_safe = state.completable_sealed(Some(li));
    }
}

/// Cross-world invariant: at least one lock across all worlds must be
/// secret-exit-safe — the write phase parks the 1-F fortress level (whose
/// secret exit skips the lock-opening FX) on a slot whose lock can stay
/// closed forever without softlocking.
///
/// Knob-free backstop, run after every world's schedule: if uniform
/// placement produced no safe lock anywhere, relocate ONE existing lock —
/// shuffled worlds, shuffled forts, shuffled candidates, first tile that
/// keeps the world completable both normally and with the new lock sealed.
pub(crate) fn ensure_secret_exit_safe(
    worlds: &mut [WorldState],
    rng: &mut dyn RngCore,
) -> PhaseReport {
    let mut actions = Vec::new();
    let phase = "secret_exit_safety";

    if worlds
        .iter()
        .any(|w| w.locks.iter().any(|l| l.secret_exit_safe))
    {
        actions.push("already satisfied: uniform placement produced a safe lock".into());
        return PhaseReport { phase, actions };
    }

    let mut world_order: Vec<usize> = (0..worlds.len()).collect();
    world_order.shuffle(rng);
    for wi in world_order {
        let state = &mut worlds[wi];
        let mut open = state.grid.clone();
        stamp_slots(&mut open, &state.slots);
        let open_reach = walk_reachable(&open, &state.pipe_pairs, state.start, state.world_idx);
        let mut lock_indices: Vec<usize> = (0..state.locks.len()).collect();
        lock_indices.shuffle(rng);
        for li in lock_indices {
            let original = state.locks[li].clone();
            // Marginal ranking against the OTHER locks' cuts — the
            // relocated lock should claim fresh territory too.
            let covered: HashSet<Pos> = state
                .locks
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != li)
                .flat_map(|(_, l)| {
                    cut_set(
                        &open, &open_reach, &state.pipe_pairs, state.start, state.world_idx,
                        l.pos, l.replace_tile,
                    )
                })
                .collect();
            let candidates = rank_candidates(state, &open, &open_reach, &covered, rng);
            for (pos, tile, _) in candidates {
                state.locks[li] = LockAssignment {
                    pos,
                    gap_tile: gap_tile_for(tile),
                    replace_tile: tile,
                    fort_section: original.fort_section,
                    secret_exit_safe: false,
                };
                if state.completable() && state.completable_sealed(Some(li)) {
                    // Relocation changed the world — every flag in it is
                    // stale, not just the moved lock's.
                    recompute_safety_flags(state);
                    actions.push(format!(
                        "W{} fort {} lock relocated {:?} -> {pos:?} (secret-exit-safe)",
                        state.world_idx + 1,
                        original.fort_section,
                        original.pos,
                    ));
                    return PhaseReport { phase, actions };
                }
            }
            state.locks[li] = original;
        }
    }

    actions.push("UNSATISFIED: no relocation in any world yields a safe lock".into());
    PhaseReport { phase, actions }
}

/// Which candidates a lock-placement pass admits, strictest first.
#[derive(Clone, Copy, PartialEq)]
enum LockPass {
    /// Closing the tile makes the GOAL unreachable in the all-open world —
    /// the pipe-proof cut (nothing routes around the goal approach), the
    /// same duty the shipping builder's C1-floor backstop assigns.
    GoalGate,
    /// Closing the tile walls off at least one node (the
    /// [`WorldState::zero_gate_locks`] definition).
    AnyGate,
    /// Any completable tile.
    Any,
}

/// Re-place every lock from scratch, preferring candidates that actually
/// gate something. The shaping loop's wide lock move: clears the lock set
/// and rebuilds it one fort at a time (shuffled order) — for each fort, the
/// passes in [`LockPass`] order over shuffled candidates, and every
/// placement keeps the completability fixpoint true.
///
/// `goal_first` (cost mode's request): the first fort that CAN gate the
/// goal does — a goal gate is the one cut a pipe web cannot bypass, so it
/// reliably raises C1 on the express-shaped worlds where "gate anything"
/// finds only worthless cuts. Only one goal gate is sought; the remaining
/// forts fall back to the normal passes (a goal crowded by every lock is
/// not a better world).
///
/// Returns false if some fort ends up unlockable — the caller owns the
/// snapshot and must restore it (unlike the dumb Locks phase, shaping never
/// removes forts; a failed proposal is just rolled back).
pub(crate) fn place_locks_gating(
    state: &mut WorldState,
    rng: &mut dyn RngCore,
    goal_first: bool,
) -> bool {
    state.locks.clear();

    let mut open = state.grid.clone();
    stamp_slots(&mut open, &state.slots);
    let open_reach = walk_reachable(&open, &state.pipe_pairs, state.start, state.world_idx);

    let mut fort_ids: Vec<usize> = state
        .slots
        .iter()
        .filter(|s| s.kind == SlotKind::Fortress)
        .map(|s| s.section)
        .collect();
    fort_ids.shuffle(rng);

    let mut covered: HashSet<Pos> = HashSet::new();
    let mut goal_gated = false;
    for fort_id in fort_ids {
        // Ranked candidates carry their cut sets, so pass admission reads
        // straight off them (no per-candidate walk here).
        let candidates = rank_candidates(state, &open, &open_reach, &covered, rng);

        let mut placed = false;
        'passes: for pass in [LockPass::GoalGate, LockPass::AnyGate, LockPass::Any] {
            if pass == LockPass::GoalGate && (!goal_first || goal_gated) {
                continue;
            }
            for (pos, tile, cut) in &candidates {
                let admits = match pass {
                    LockPass::GoalGate => state.target.is_some_and(|t| cut.contains(&t)),
                    LockPass::AnyGate => !cut.is_empty(),
                    LockPass::Any => true,
                };
                if !admits {
                    continue;
                }
                state.locks.push(LockAssignment {
                    pos: *pos,
                    gap_tile: gap_tile_for(*tile),
                    replace_tile: *tile,
                    fort_section: fort_id,
                    secret_exit_safe: false,
                });
                if state.completable() {
                    placed = true;
                    covered.extend(cut.iter().copied());
                    if pass == LockPass::GoalGate {
                        goal_gated = true;
                    }
                    break 'passes;
                }
                state.locks.pop();
            }
        }
        if !placed {
            return false;
        }
    }
    true
}

/// Bridge-class path tiles: the water bridge and the two drawbridges —
/// vanilla's natural lock sites (the drawbridge). Used as the TIEBREAK in
/// candidate ranking: among equal marginal cuts, the bridge wins the look.
const BRIDGE_TILES: [u8; 3] = [0xB3, 0xB1, 0xB2];

/// Positions severed when `pos` alone is closed on the all-open world:
/// reachable normally, unreachable with the gap. The measure of what a
/// lock site is actually worth.
fn cut_set(
    open: &Grid,
    open_reach: &Reach,
    pipe_pairs: &[TeleportEdge],
    start: Option<Pos>,
    world_idx: usize,
    pos: Pos,
    tile: u8,
) -> HashSet<Pos> {
    let mut g = open.clone();
    g.set(pos.0, pos.1, gap_tile_for(tile));
    let closed = walk_reachable(&g, pipe_pairs, start, world_idx);
    let mut cut = HashSet::new();
    for r in 0..open.rows() {
        for c in 0..open.cols {
            if open_reach.contains((r, c)) && !closed.contains((r, c)) {
                cut.insert((r, c));
            }
        }
    }
    cut
}

/// Rank one fort's candidates by MARGINAL cut — the nodes a site severs
/// that `covered` (the union of already-placed locks' cuts) doesn't
/// already claim — descending, bridges as the tiebreak, shuffled order
/// within remaining ties. Marginal (not raw) is what stops the pile-up:
/// the first goal-corridor lock scores the whole region, the second one
/// scores the thin slice between them and loses to fresh territory — locks
/// spread out to claim their own gates (the vanilla milestone shape), and
/// because cuts depend on the seed's layout the proposal order varies per
/// seed (unlike the static bridges-first sort this replaces).
fn rank_candidates(
    state: &WorldState,
    open: &Grid,
    open_reach: &Reach,
    covered: &HashSet<Pos>,
    rng: &mut dyn RngCore,
) -> Vec<(Pos, u8, HashSet<Pos>)> {
    let mut out: Vec<(Pos, u8, HashSet<Pos>)> = lock_candidates(state)
        .into_iter()
        .map(|(pos, tile)| {
            let cut = cut_set(
                open, open_reach, &state.pipe_pairs, state.start, state.world_idx, pos, tile,
            );
            (pos, tile, cut)
        })
        .collect();
    out.shuffle(rng);
    out.sort_by_key(|(_, tile, cut)| {
        let marginal = cut.iter().filter(|p| !covered.contains(*p)).count();
        (std::cmp::Reverse(marginal), !BRIDGE_TILES.contains(tile))
    });
    out
}

/// Tiles a lock may claim: lockable path-tile types (the kinds the FX engine
/// can gap out and restore), not already locked, and not the row-7/8 partner
/// of completable content — a lock consumes that shared completion bit too.
fn lock_candidates(state: &WorldState) -> Vec<(Pos, u8)> {
    let locked: HashSet<Pos> = state.locks.iter().map(|l| l.pos).collect();
    let content: HashSet<Pos> = state.slots.iter().map(|s| s.pos).collect();
    let mut out = Vec::new();
    for r in 0..state.grid.rows() {
        for c in 0..state.grid.cols {
            let pos = (r, c);
            let tile = state.grid.get(r, c);
            if !LOCKABLE_TILES.contains(&tile) || locked.contains(&pos) {
                continue;
            }
            if let Some(partner) = row78_partner(pos)
                && content.contains(&partner)
            {
                continue;
            }
            out.push((pos, tile));
        }
    }
    out
}
