//! Levels phase — place the world's action levels on the connected map.
//!
//! Placement is uniform-random over legal blanks with ONE earned rule —
//! avoid-adjacency-when-avoidable: if the uniform pick would sit
//! orthogonally next to an already-placed level and non-adjacent blanks
//! remain, re-pick uniformly among the non-adjacent ones. Pure uniform
//! measured terrible (playtest + census 2026-08-01: 14-21% of worlds put
//! 4+ levels in one connected chain, worst 7 — the whole budget in a
//! snake and the rest of the map empty; the old builder's spread scorer
//! had prevented this and was deleted with it). No weights: adjacency is
//! avoided exactly when the map allows, so small forced corridors (W7)
//! still chain like vanilla does.
//!
//! What binds anyway (facts, not preferences):
//! - levels go on blank tiles only, never on `fixed` positions;
//! - `state.level_budget` levels are placed, no more (a chosen per-world
//!   count, seeded from vanilla — see the field doc);
//! - the row-7/8 completion-bit rule: the ROM keeps ONE completion bit per
//!   column for rows 7 and 8 together, so completable content at (7,c) and
//!   (8,c) would mark each other beaten — placing into that pattern is a
//!   real gameplay bug, not an aesthetic choice. Placing on one row bars
//!   the partner row's cell for the rest of the build.
//!
//! Running out of legal blanks before the budget is spent is reported and
//! counted, not panicked over.

use super::*;

use rand::seq::IndexedRandom;

pub(crate) struct Levels;

impl Phase for Levels {
    fn name(&self) -> &'static str {
        "levels"
    }

    fn run(&self, state: &mut WorldState, rng: &mut dyn RngCore) -> PhaseReport {
        let mut actions = Vec::new();

        let mut candidates = state.legal_blanks();
        let mut placed = 0usize;

        while placed < state.level_budget {
            let clear: Vec<Pos> =
                candidates.iter().copied().filter(|&c| !next_to_level(state, c)).collect();
            let pool = if clear.is_empty() { &candidates } else { &clear };
            let Some(&pos) = pool.choose(rng) else {
                actions.push(format!(
                    "stuck: {placed}/{} levels placed, no legal blanks left",
                    state.level_budget,
                ));
                break;
            };
            state.slots.push(SlotAssignment {
                pos,
                kind: SlotKind::Level,
                section: 0,
                is_hand_trap: false,
                is_troll_pipe: false,
            });
            placed += 1;
            // The chosen cell is taken; its row-7/8 partner (if any) is now
            // barred by the completion-bit rule.
            candidates.retain(|&c| c != pos && Some(c) != row78_partner(pos));
        }

        actions.push(format!(
            "done: {placed}/{} levels placed (pool was {} blanks)",
            state.level_budget,
            state.legal_blanks().len() + placed,
        ));
        PhaseReport { phase: self.name(), actions }
    }
}

/// Orthogonally adjacent (2-tile walk neighbor) to an already-placed level.
pub(super) fn next_to_level(state: &WorldState, pos: Pos) -> bool {
    state.slots.iter().any(|s| {
        s.kind == SlotKind::Level && {
            let (dr, dc) = (s.pos.0.abs_diff(pos.0), s.pos.1.abs_diff(pos.1));
            (dr == 2 && dc == 0) || (dr == 0 && dc == 2)
        }
    })
}
