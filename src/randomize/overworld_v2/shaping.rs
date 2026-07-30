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
//! [`TARGET_ROUTES`], [`SHORTCUT_TRIES`], [`ESCALATE_AFTER`].

use super::*;

use rand::seq::IndexedRandom;

use super::locks::{place_locks_gating, recompute_safety_flags};
use super::metrics::WorldMeasure;

/// Max moves (accepted or rejected) per world.
const MOVE_BUDGET: usize = 8;
/// Satisficing target: stop once this many routes are in the choice band.
const TARGET_ROUTES: usize = 2;
/// Candidate pipe pairs tried inside ONE gated-shortcut move.
const SHORTCUT_TRIES: usize = 4;
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

            moves += 1;
            if lock_available {
                match try_lock_replace(state, rng, &before, zero_gate) {
                    Ok(line) => {
                        lock_rejects = 0;
                        accepted += 1;
                        actions.push(line);
                    }
                    Err(line) => {
                        lock_rejects += 1;
                        actions.push(line);
                    }
                }
            } else if shortcut_available {
                match try_gated_shortcut(state, rng, &before) {
                    Ok(line) => {
                        // New topology is fresh lock ammunition — un-exhaust
                        // the cheaper rung too.
                        shortcut_rejects = 0;
                        lock_rejects = 0;
                        accepted += 1;
                        actions.push(line);
                    }
                    Err(line) => {
                        shortcut_rejects += 1;
                        actions.push(line);
                    }
                }
            } else {
                moves -= 1;
                break "stuck: no move available";
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
