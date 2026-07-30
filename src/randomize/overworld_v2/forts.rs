//! Forts phase — place the world's fortresses.
//!
//! KNOB-FREE: each fort lands on a uniform-random legal blank, same shared
//! rules as levels and pipe endpoints. None of the shipping builder's fort
//! thinking (off-path preference, dead-end aesthetics, rescue duty) exists
//! yet — the census measures where random forts land (notably: how often on
//! the cheapest route, where their future lock would gate nothing) and the
//! off-path preference gets argued from that number.
//!
//! Forts carry a plain **fort id** in `SlotAssignment.section`, assigned in
//! placement order. It exists only to pair each fort with its lock (a ROM
//! mechanic: the fort's Boom-Boom ordinal indexes the world FX list — the
//! write phase makes ids and ordinals agree). There is NO ordering
//! semantics: v2's lock invariants are order-free (beat-what-you-reach
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

        let mut candidates = state.legal_blanks();
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

        actions.push(format!("done: {placed}/{} forts placed", state.fort_budget));
        PhaseReport { phase: self.name(), actions }
    }
}
