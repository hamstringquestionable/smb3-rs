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
                pin: None,
                lock_hint: LockHint::default(),
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
        let mut order: Vec<Pos> =
            bfs_ordered(&state.grid, &state.pipe_pairs, state.start, state.world_idx)
                .into_iter()
                .map(|(pos, _)| pos)
                .filter(|pos| legal.contains(pos))
                .collect();
        let wanted = take_fillers(&order, usize::MAX, state.row78_barred());
        if wanted.len() > budget {
            let rowless = rowless_area(state, &wanted, wanted.len() - budget);
            order.retain(|pos| !rowless.contains(pos));
        }
        let fillers = take_fillers(&order, budget, state.row78_barred());
        let placed = fillers.len();
        for pos in fillers {
            state.slots.push(SlotAssignment {
                pos,
                kind: SlotKind::HammerBro,
                section: 0,
                is_hand_trap: false,
                is_troll_pipe: false,
                pin: None,
                lock_hint: LockHint::default(),
            });
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

/// The first `budget` blanks of `order` that the row 7/8 rule leaves free,
/// barring each one's partner as it is taken.
fn take_fillers(order: &[Pos], budget: usize, mut barred: HashSet<Pos>) -> Vec<Pos> {
    let mut taken = Vec::new();
    for &pos in order {
        if taken.len() >= budget {
            break;
        }
        if barred.contains(&pos) {
            continue;
        }
        taken.push(pos);
        if let Some(partner) = row78_partner(pos) {
            barred.insert(partner);
        }
    }
    taken
}

/// The march area to leave without Hammer Bro slots when the budget cannot
/// cover all `overflow` of the blanks the fill `wanted`.
///
/// A blank without a pointer entry is harmless until a Hammer Bro lands on
/// it, and a bro never leaves the area its march can reach: W8's pages are
/// each one, joined only by pipes, which sprites do not take. Sprites spawn
/// only on `HammerBro` slots, so an area given none holds no bro, and every
/// other area is filled in full. The area chosen is the smallest that holds
/// the whole overflow (the rows it leaves unspent go to the writer's spare-row
/// fill), or the largest when none does. An area a pinned sprite can march
/// into is never chosen.
fn rowless_area(state: &WorldState, wanted: &[Pos], overflow: usize) -> HashSet<Pos> {
    let blocked: HashSet<Pos> = state.slots.iter().map(|s| s.pos).collect();
    let mut areas: Vec<HashSet<Pos>> = Vec::new();
    let mut pinned: Vec<bool> = Vec::new();
    for &pos in wanted {
        if areas.iter().any(|a| a.contains(&pos)) {
            continue;
        }
        let reach = super::march::march_distances(&state.grid, &blocked, pos);
        areas.push(wanted.iter().copied().filter(|p| reach.contains_key(p)).collect());
        pinned.push(state.hb_sprite_pins.iter().any(|p| reach.contains_key(p)));
    }
    let free = || areas.iter().zip(&pinned).filter(|(_, p)| !**p).map(|(a, _)| a);
    let fits = free().filter(|a| a.len() >= overflow).min_by_key(|a| a.len());
    fits.or_else(|| free().max_by_key(|a| a.len())).cloned().unwrap_or_default()
}
