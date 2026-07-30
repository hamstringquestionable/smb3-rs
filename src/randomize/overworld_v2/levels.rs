//! Levels phase — place the world's action levels on the connected map.
//!
//! Deliberately KNOB-FREE: every level lands on a uniform-random legal
//! blank. None of the shipping builder's placement scoring (spread, path
//! relevance, dead-end preference) exists here — the census measures what
//! uniform placement produces (clustering, screen balance, route effects),
//! and any future preference has to beat those numbers to justify itself.
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

        let mut candidates = legal_blanks(state);
        let mut placed = 0usize;

        while placed < state.level_budget {
            let Some(&pos) = candidates.choose(rng) else {
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

        actions.push(format!("done: {placed}/{} levels placed", state.level_budget));
        PhaseReport { phase: self.name(), actions }
    }
}

/// Blanks a level may claim: blank tile, not `fixed`, not already a slot,
/// and not the row-7/8 partner of existing completable content.
fn legal_blanks(state: &WorldState) -> Vec<Pos> {
    let taken: HashSet<Pos> = state.slots.iter().map(|s| s.pos).collect();
    let barred: HashSet<Pos> = state
        .slots
        .iter()
        .filter_map(|s| row78_partner(s.pos))
        .collect();
    let mut out = Vec::new();
    for r in 0..state.grid.rows() {
        for c in 0..state.grid.cols {
            let pos = (r, c);
            if rom_data::VALID_BLANK_TILES.contains(&state.grid.get(r, c))
                && !state.fixed.contains(&pos)
                && !taken.contains(&pos)
                && !barred.contains(&pos)
            {
                out.push(pos);
            }
        }
    }
    out
}

/// The cell sharing `pos`'s completion bit: (8,c) for (7,c) and vice versa.
/// `None` for every other row.
fn row78_partner(pos: Pos) -> Option<Pos> {
    match pos.0 {
        7 => Some((8, pos.1)),
        8 => Some((7, pos.1)),
        _ => None,
    }
}
