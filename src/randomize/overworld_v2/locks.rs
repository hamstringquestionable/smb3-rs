//! Locks phase — one lock (map gap) per fortress, uniform-random placement.
//!
//! KNOB-FREE: for each fort id, candidate tiles are shuffled and the first
//! one that keeps the world completable wins. No chokepoint scoring, no
//! golden-site hunt, no gate duty — the census measures what random locks
//! gate (including how often the answer is "nothing"), and the shaping
//! controls get argued from those numbers.
//!
//! The one invariant is ORDER-FREE completability (`WorldState::completable`):
//! close every lock, beat every reachable fort, open those locks, repeat —
//! the world is valid iff the goal is reached AND every fort is eventually
//! beatable (a fort sealed behind its own lock is permanently unplayable
//! content). This single fixpoint subsumes the shipping builder's per-section
//! hard rules without its "sections beaten in order" linearization.
//!
//! A fort may end up lockless when every candidate fails the invariant —
//! soft, reported, counted by the census.

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

        for fort_id in fort_ids {
            let mut candidates = lock_candidates(state);
            candidates.shuffle(rng);

            let mut placed = false;
            let tried = candidates.len();
            for (pos, tile) in candidates {
                state.locks.push(LockAssignment {
                    pos,
                    gap_tile: gap_tile_for(tile),
                    replace_tile: tile,
                    fort_section: fort_id,
                    // Write-phase concerns (1-F secret exit safety, gate
                    // bookkeeping) — not computed by the dumb skeleton.
                    secret_exit_safe: false,
                    blocks_target: false,
                });
                if state.completable() {
                    actions.push(format!("lock for fort {fort_id} at {pos:?}"));
                    placed = true;
                    break;
                }
                state.locks.pop();
            }
            if !placed {
                actions.push(format!(
                    "fort {fort_id} lockless: all {tried} candidates break completability",
                ));
            }
        }

        actions.push(format!(
            "done: {}/{} forts locked",
            state.locks.len(),
            state.slots.iter().filter(|s| s.kind == SlotKind::Fortress).count(),
        ));
        PhaseReport { phase: self.name(), actions }
    }
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
