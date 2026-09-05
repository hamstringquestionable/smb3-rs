//! Forts phase — place the world's fortresses.
//!
//! Uniform-random over legal blanks, same shared rules as levels and pipe
//! endpoints, with ONE preference: a fort may not take a blank the player
//! cannot walk around. Everything else the shipping builder used to do here
//! (dead-end aesthetics, rescue duty) still does not exist.
//!
//! **Why this one preference.** A fort the player is forced through is a
//! wasted lock — they play it because it is in the way and its lock opens
//! with no decision made (the charter's cardinal sin, issue #163). The
//! forbidden blanks are exactly the cut vertices between start and goal, so
//! [`WorldState::forced_positions`] answers it directly; it is asked once
//! per phase because the answer depends on terrain and the pipe web alone,
//! never on what has been placed.
//!
//! Argued from the census as this module promised, not assumed
//! (`docs/choice_first_charter.md`, "Forts on the forced path"): uniform
//! placement left 24% of forts forced and the shaping loop could not repair
//! them, while the free pool covered every world's fort budget in 8000 out
//! of 8000 world-builds. With no seed ever short of safe blanks the
//! preference needs no rate and no fallback rule — the guard below fires
//! only if a future map ever runs the pool dry.
//!
//! Forts carry a plain **fort id** in `SlotAssignment.section`, assigned in
//! placement order. It exists only to pair each fort with its lock (a ROM
//! mechanic: the fort's Boom-Boom ordinal indexes the world FX list — the
//! write phase makes ids and ordinals agree). There is NO ordering
//! semantics: the lock invariants are order-free (beat-what-you-reach
//! fixpoint), unlike the shipping builder's "sections beaten in order"
//! linearization, which is deliberately not carried over.

use super::*;

use rand::seq::IndexedRandom;

pub(crate) struct Forts;

impl Phase for Forts {
    fn name(&self) -> &'static str {
        "forts"
    }

    fn run(&self, state: &mut WorldState, rng: &mut dyn RngCore) -> PhaseReport {
        let mut actions = Vec::new();

        // Asked once: placing content never changes who is a cut vertex.
        let mut candidates = state.legal_blanks();
        let forced: HashSet<Pos> = state.forced_positions(&candidates).into_iter().collect();
        if candidates.iter().any(|p| !forced.contains(p)) {
            candidates.retain(|p| !forced.contains(p));
        } else if !forced.is_empty() {
            actions.push(format!(
                "no unforced blank among {} candidates — placing on the path",
                candidates.len(),
            ));
        }
        let mut placed = 0usize;

        while placed < state.fort_budget {
            let Some(&pos) = candidates.choose(rng) else {
                actions.push(format!(
                    "stuck: {placed}/{} forts placed, no legal blanks left",
                    state.fort_budget,
                ));
                break;
            };
            state.slots.push(SlotAssignment {
                pos,
                kind: SlotKind::Fortress,
                section: placed, // fort id — pairing only, no order semantics
                is_hand_trap: false,
                is_troll_pipe: false,
            });
            placed += 1;
            candidates.retain(|&c| c != pos && Some(c) != row78_partner(pos));
        }

        actions.push(format!(
            "done: {placed}/{} forts placed, {} forced blanks excluded",
            state.fort_budget,
            forced.len(),
        ));
        PhaseReport { phase: self.name(), actions }
    }
}
