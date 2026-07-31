//! Spare-pipes phase — spend whatever pipe budget connectivity didn't use.
//!
//! KNOB-FREE, and deliberately dumber than it will end up: both endpoints of
//! every spare pair are uniform-random legal blanks. No choice scoring, no
//! veto — this is the observe-only baseline for the census question "what do
//! random pipes actually do to a world's routes?".
//!
//! What the phase DOES carry is instrumentation: each placement measures the
//! world before and after (route count in the choice band, cheapest-route
//! cost) and logs the delta. The measurement never influences the placement
//! — it exists so the census can tally created/destroyed/cheapened choice
//! and put a number on the natural rate of ungated dominating shortcuts.
//! When a choice-aware control is added later, this same measurement moves
//! from "logged after" to "consulted before".

use super::*;

use rand::seq::IndexedRandom;

pub(crate) struct SparePipes;

impl Phase for SparePipes {
    fn name(&self) -> &'static str {
        "spare_pipes"
    }

    fn run(&self, state: &mut WorldState, rng: &mut dyn RngCore) -> PhaseReport {
        let mut actions = Vec::new();

        while state.pipe_pairs.len() < state.pipe_budget {
            // Anchor-adjacent blanks are barred for pipe endpoints (an
            // ungateable express); spares are optional, so no fallback.
            let candidates: Vec<Pos> = state
                .legal_blanks()
                .into_iter()
                .filter(|&p| !state.near_anchor(p))
                .collect();
            let picked: Vec<Pos> = candidates.choose_multiple(rng, 2).copied().collect();
            let [a, b] = picked[..] else {
                actions.push(format!(
                    "stuck: {} blanks left, need 2 for a pipe pair",
                    candidates.len(),
                ));
                break;
            };

            let before = measure_world(state);
            state.add_pipe_pair(a, b);
            let after = measure_world(state);
            actions.push(format!(
                "pipe {a:?} <-> {b:?}: routes {} -> {}, C1 {} -> {}",
                before.routes_in_band, after.routes_in_band, before.c1, after.c1,
            ));
        }

        actions.push(format!(
            "done: {}/{} pipe pairs on the map",
            state.pipe_pairs.len(),
            state.pipe_budget,
        ));
        PhaseReport { phase: self.name(), actions }
    }
}
