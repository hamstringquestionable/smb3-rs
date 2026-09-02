//! Hammer-bro fill phase — the LAST placement phase: every leftover
//! reachable blank becomes a `HammerBro` slot, the conversion stock the
//! cross-world post-passes draw on (toad-house / spade promotion, wandering
//! sprite redistribution) and the pool the pointer-table writer fills with
//! cycling hammer-bro levels.
//!
//! Two rules carried over from the shipping builder, both ROM facts:
//!
//! - **Sprite pins are mandatory.** When the hammer-bro shuffle is off, the
//!   vanilla wandering sprites keep their map positions, and a sprite's
//!   tile needs a pointer entry — the player can be caught there on turn
//!   one. Those positions are in `fixed` (nothing else may claim them), so
//!   the pin bypasses `legal_blanks` deliberately.
//! - **The pointer table is the cap.** Every slot consumes one of the
//!   world's pool entries; fill stops at `ptr_slots` minus the pipe
//!   endpoints already spent. Nearest-to-start (BFS order) wins the budget,
//!   matching the shipping builder's "drop the farthest fillers" rule.
//!
//! Runs after shaping — during shaping these blanks must stay free, they
//! are the moves' target space.

use super::*;

pub(crate) struct HammerBroFill;

impl Phase for HammerBroFill {
    fn name(&self) -> &'static str {
        "hammer_bro_fill"
    }

    fn run(&self, state: &mut WorldState, _rng: &mut dyn RngCore) -> PhaseReport {
        let mut actions = Vec::new();

        // Mandatory pins first — never dropped, never counted against the
        // budget ordering (they were excluded from every earlier phase via
        // `fixed`, so no slot can already sit there; guard anyway).
        let taken: HashSet<Pos> = state.slots.iter().map(|s| s.pos).collect();
        let pins: Vec<Pos> =
            state.hb_sprite_pins.iter().copied().filter(|p| !taken.contains(p)).collect();
        for pos in &pins {
            state.slots.push(SlotAssignment {
                pos: *pos,
                kind: SlotKind::HammerBro,
                section: 0,
                is_hand_trap: false,
                is_troll_pipe: false,
            });
        }
        if !pins.is_empty() {
            actions.push(format!("{} mandatory sprite-position slots", pins.len()));
        }

        // Budget: pointer entries left after every existing slot (pipes
        // included — each endpoint is an entry).
        let budget = state.ptr_slots.saturating_sub(state.slots.len());

        // Candidates: legal blanks, walk-reachable, nearest-to-start first.
        let legal: HashSet<Pos> = state.legal_blanks().into_iter().collect();
        let ordered = bfs_ordered(&state.grid, &state.pipe_pairs, state.start, state.world_idx);
        let mut barred: HashSet<Pos> = state.row78_barred();
        let mut placed = 0usize;
        for (pos, _) in ordered {
            if placed >= budget {
                break;
            }
            if !legal.contains(&pos) || barred.contains(&pos) {
                continue;
            }
            state.slots.push(SlotAssignment {
                pos,
                kind: SlotKind::HammerBro,
                section: 0,
                is_hand_trap: false,
                is_troll_pipe: false,
            });
            if let Some(partner) = row78_partner(pos) {
                barred.insert(partner);
            }
            placed += 1;
        }

        actions.push(format!(
            "done: {placed} filler slots (+{} pins), {} of {} pointer entries used",
            pins.len(),
            state.slots.len(),
            state.ptr_slots,
        ));
        PhaseReport { phase: self.name(), actions }
    }
}
