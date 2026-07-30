//! Shaping phase — the diagnosis-driven improvement loop, run as the LAST
//! phase in the schedule on the completed dumb world.
//!
//! Every earlier phase stays knob-free; all judgment concentrates here. The
//! loop reads the world through [`measure_world`], and the measured symptom
//! selects the move — no move portfolio is evaluated side by side:
//!
//! - **Satisfied** (routes in band ≥ [`TARGET_ROUTES`]): no move at all.
//!   Satisficing, not optimizing — worlds the dumb roll already made
//!   interesting keep their shape, and the across-seed diversity the uniform
//!   baseline bought is only spent where the measure says the world is
//!   linear.
//! - **Linear with lock ammunition** (zero-gate locks, or detour structure a
//!   trunk gate could pull into the band): **lock re-place** — the cheapest
//!   rung. Re-runs lock placement from scratch via `place_locks_gating`
//!   (v1's #125 lesson: re-deciding the whole coupled set beats nudging one
//!   lock).
//! - **Linear with no raw material** (no dominated detours): only new
//!   topology can help — walk edges are local, so a pipe is the one move
//!   that creates a route that doesn't exist yet. **Gated shortcut**: add a
//!   pipe pair, re-place ALL locks on the new topology, judge the bundle.
//! - **Both cheaper rungs spent** (exhausted or unavailable): **fort+lock
//!   move** — relocate one fort and re-place all locks as one bundle. The
//!   trunk-fort symptom: a fort ON the cheapest route makes every lock
//!   decorative and no lock or pipe move can fix that; only moving the fort
//!   changes the lock's search space.
//!
//! Moves are judged on the complete world and rolled back on rejection; a
//! move type that keeps failing escalates past itself ([`ESCALATE_AFTER`]),
//! so most iterations cost one proposal, not five. Acceptance is deliberately
//! minimal — routes up (or, for lock re-place, zero-gate down at equal
//! routes) — with no C1 guard: for the route count to rise, the old
//! structure must stay within the choice band of the new cheapest route, so
//! trivializing shortcuts reject themselves. The census C1 column is the
//! watchdog on that argument.
//!
//! The loop's knobs (the ONLY knobs in v2): [`MOVE_BUDGET`],
//! [`TARGET_ROUTES`], [`SHORTCUT_TRIES`], [`FORTLOCK_TRIES`],
//! [`ESCALATE_AFTER`].

use super::*;

use rand::seq::{IndexedRandom, SliceRandom};

use super::locks::{place_locks_gating, recompute_safety_flags};
use super::metrics::WorldMeasure;

/// Max moves (accepted or rejected) per world.
const MOVE_BUDGET: usize = 8;
/// Satisficing target: stop once this many routes are in the choice band.
const TARGET_ROUTES: usize = 2;
/// Candidate pipe pairs tried inside ONE gated-shortcut move.
const SHORTCUT_TRIES: usize = 4;
/// Full fort-relocation evaluations paid inside ONE fort+lock move.
const FORTLOCK_TRIES: usize = 4;
/// Consecutive rejections of a move type before escalating past it.
const ESCALATE_AFTER: usize = 2;

pub(crate) struct Shaping;

impl Phase for Shaping {
    fn name(&self) -> &'static str {
        "shaping"
    }

    fn run(&self, state: &mut WorldState, rng: &mut dyn RngCore) -> PhaseReport {
        let mut actions = Vec::new();
        let mut moves = 0usize;
        let mut accepted = 0usize;
        let mut lock_rejects = 0usize;
        let mut shortcut_rejects = 0usize;
        let mut fortlock_rejects = 0usize;

        let status = loop {
            let before = measure_world(state);
            if before.routes_in_band >= TARGET_ROUTES {
                break "satisfied";
            }
            if moves >= MOVE_BUDGET {
                break "budget spent";
            }

            let zero_gate = state.zero_gate_locks().len();
            let lock_available = !state.locks.is_empty()
                && lock_rejects < ESCALATE_AFTER
                && (zero_gate > 0 || before.dominated_detours > 0);
            let shortcut_available = state.pipe_pairs.len() < state.pipe_budget
                && shortcut_rejects < ESCALATE_AFTER;
            let fortlock_available =
                state.fort_count() > 0 && fortlock_rejects < ESCALATE_AFTER;

            moves += 1;
            // On any accept the world changed, so every rung's ammunition is
            // fresh — all three exhaustion counters reset together.
            let outcome = if lock_available {
                try_lock_replace(state, rng, &before, zero_gate)
            } else if shortcut_available {
                try_gated_shortcut(state, rng, &before)
            } else if fortlock_available {
                try_fort_lock(state, rng, &before)
            } else {
                moves -= 1;
                break "stuck: no move available";
            };
            match outcome {
                Ok(line) => {
                    lock_rejects = 0;
                    shortcut_rejects = 0;
                    fortlock_rejects = 0;
                    accepted += 1;
                    actions.push(line);
                }
                Err(line) => {
                    if lock_available {
                        lock_rejects += 1;
                    } else if shortcut_available {
                        shortcut_rejects += 1;
                    } else {
                        fortlock_rejects += 1;
                    }
                    actions.push(line);
                }
            }
        };

        if accepted > 0 {
            recompute_safety_flags(state);
        }
        let m = measure_world(state);
        actions.push(format!(
            "done ({status}): {accepted}/{moves} moves accepted, routes {}, C1 {}, {} zero-gate locks",
            m.routes_in_band,
            m.c1,
            state.zero_gate_locks().len(),
        ));
        PhaseReport { phase: self.name(), actions }
    }
}

/// The cheap rung: re-place every lock, preferring gating candidates. Accept
/// iff routes rise, or (routes flat) decorative locks were converted into
/// gating ones.
fn try_lock_replace(
    state: &mut WorldState,
    rng: &mut dyn RngCore,
    before: &WorldMeasure,
    zero_gate_before: usize,
) -> Result<String, String> {
    let saved = state.locks.clone();
    if !place_locks_gating(state, rng) {
        state.locks = saved;
        return Err("lock_replace REJECT: could not lock every fort".into());
    }
    let after = measure_world(state);
    let zero_gate_after = state.zero_gate_locks().len();
    let improved = after.routes_in_band > before.routes_in_band
        || (after.routes_in_band == before.routes_in_band && zero_gate_after < zero_gate_before);
    let line = format!(
        "routes {} -> {}, C1 {} -> {}, zero-gate {zero_gate_before} -> {zero_gate_after}",
        before.routes_in_band, after.routes_in_band, before.c1, after.c1,
    );
    if improved {
        Ok(format!("lock_replace ACCEPT: {line}"))
    } else {
        state.locks = saved;
        Err(format!("lock_replace REJECT: {line}"))
    }
}

/// The topology rung: spend one pipe pair AND re-place all locks as a single
/// bundle, judged together on the finished world. Accept iff routes rise —
/// the shortcut must become a gated alternative, not an ungated express.
///
/// Proposal seeding was tried and REVERTED (census-measured, 100 seeds):
/// a pipe-only measurement pre-screen ("skip pairs whose pipe changes
/// nothing against the current locks") filtered out true positives — a pipe
/// inert against the CURRENT locks can still create a route once locks are
/// re-placed, which is the entire point of the bundle (W8 accepts halved,
/// linear 73→84%). A walk-distance floor on the endpoints was neutral:
/// uniform pairs are rarely walk-close, so it filtered almost nothing.
/// Any future proposal filter must judge the pipe+locks bundle, not the
/// pipe alone.
fn try_gated_shortcut(
    state: &mut WorldState,
    rng: &mut dyn RngCore,
    before: &WorldMeasure,
) -> Result<String, String> {
    let saved_grid = state.grid.clone();
    let saved_slots = state.slots.clone();
    let saved_locks = state.locks.clone();
    let saved_pairs = state.pipe_pairs.clone();

    for _ in 0..SHORTCUT_TRIES {
        let candidates = state.legal_blanks();
        let picked: Vec<Pos> = candidates.choose_multiple(rng, 2).copied().collect();
        let [a, b] = picked[..] else { break };
        if Some(b) == row78_partner(a) {
            continue;
        }
        state.add_pipe_pair(a, b);
        if place_locks_gating(state, rng) {
            let after = measure_world(state);
            if after.routes_in_band > before.routes_in_band {
                return Ok(format!(
                    "gated_shortcut ACCEPT: pipe {a:?} <-> {b:?}, routes {} -> {}, C1 {} -> {}",
                    before.routes_in_band, after.routes_in_band, before.c1, after.c1,
                ));
            }
        }
        state.grid = saved_grid.clone();
        state.slots = saved_slots.clone();
        state.locks = saved_locks.clone();
        state.pipe_pairs = saved_pairs.clone();
    }
    Err(format!(
        "gated_shortcut REJECT: no pair in {SHORTCUT_TRIES} tries beats routes {}",
        before.routes_in_band,
    ))
}

/// The escalation rung: relocate ONE fort AND re-place all locks as a single
/// bundle judged on the finished world. Jurisdiction is the trunk-fort
/// symptom: a fort sitting on the cheapest route makes every lock decorative
/// (the player passes it regardless), and neither lock re-place nor a
/// shortcut can manufacture an alternative — only moving the fort changes
/// what its lock can gate. Accept iff routes rise.
///
/// Forts on the cheapest route are proposed first (shuffled within each
/// group). That ordering comes from the full-world measure already in hand —
/// per the seeding lesson, the bundle is still the only thing judged.
fn try_fort_lock(
    state: &mut WorldState,
    rng: &mut dyn RngCore,
    before: &WorldMeasure,
) -> Result<String, String> {
    let saved_slots = state.slots.clone();
    let saved_locks = state.locks.clone();

    let trunk: HashSet<Pos> = before
        .rc
        .routes
        .first()
        .map(|r| r.path.iter().copied().collect())
        .unwrap_or_default();

    let mut fort_indices: Vec<usize> = state
        .slots
        .iter()
        .enumerate()
        .filter(|(_, s)| s.kind == SlotKind::Fortress)
        .map(|(i, _)| i)
        .collect();
    fort_indices.shuffle(rng);
    // Stable sort: trunk forts first, shuffled order kept within each group.
    fort_indices.sort_by_key(|&i| !trunk.contains(&state.slots[i].pos));

    let mut evals = 0usize;
    for &fi in &fort_indices {
        if evals >= FORTLOCK_TRIES {
            break;
        }
        let fort_id = state.slots[fi].section;
        let old_pos = state.slots[fi].pos;
        for _ in 0..2 {
            if evals >= FORTLOCK_TRIES {
                break;
            }
            let candidates = state.legal_blanks();
            let Some(&new_pos) = candidates.choose(rng) else { break };
            state.slots[fi].pos = new_pos;
            evals += 1;
            if place_locks_gating(state, rng) {
                let after = measure_world(state);
                if after.routes_in_band > before.routes_in_band {
                    return Ok(format!(
                        "fort_lock ACCEPT: fort {fort_id} {old_pos:?} -> {new_pos:?}, routes {} -> {}, C1 {} -> {}",
                        before.routes_in_band, after.routes_in_band, before.c1, after.c1,
                    ));
                }
            }
            state.slots = saved_slots.clone();
            state.locks = saved_locks.clone();
        }
    }
    Err(format!(
        "fort_lock REJECT: no relocation beats routes {} ({evals} full evals)",
        before.routes_in_band,
    ))
}
