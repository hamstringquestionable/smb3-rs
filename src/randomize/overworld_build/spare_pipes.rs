//! Spare-pipes phase — spend whatever pipe budget connectivity and shaping
//! left unspent. Every world ships its FULL vanilla pipe count: an unplaced
//! pair is deleted content (its transit levels drop out of the pointer
//! table and the map loses its pipe density), so placement is MANDATORY —
//! measurement only steers WHERE, never WHETHER.
//!
//! Guarded toss, route-seeking: for each remaining pair, up to
//! [`SPARE_TRIES`] uniform candidate pairs are measured on the finished
//! world and preferred in tiers — a pair that CREATES an in-band route
//! beats a merely harmless pair (floor + routes kept), which beats the
//! best (floor-clamped C1, routes) fallback — which is placed anyway:
//! placement is mandatory. No wide-window tier here, deliberately: spares
//! run after the shaping loop, so a wide route would never receive the
//! parity trims — measured near-inert (~1pt W7 noalt) for a large share
//! of the phase's measurement cost (the shortcut rung keeps its wide
//! accept; refinement follows it there). The final floor screen is the
//! web-redeal wrapper: a world whose spares can only crash the floor
//! redeals its whole web.
//!
//! Runs LAST in the shaped pipeline (after shaping): spares must not eat
//! the budget headroom the gated-shortcut rung draws on, and the guard
//! needs the finished structure to measure against.
//!
//! Pocket-doubling pairs (a second direct link between two pockets that
//! already have one) are DEMOTED, not banned: same pocket pair = usually
//! the same level set both ways, which the domination filter deletes —
//! measured dead weight (~0.5 pairs/seed on W7, census 2026-08-01). The
//! creator tier still judges every pair by measurement (a doubled link
//! with a different forced level on its approach CAN create a route —
//! vanilla W7's micro-theta), so only the harmless tier and the fallback
//! ranking prefer non-doubling pairs.

use super::*;

use super::connectivity::{linked_pocket_pairs, pocket_map};
use super::locks::recompute_safety_flags;
use rand::seq::IndexedRandom;

/// Candidate pairs measured per spare pipe before settling.
const SPARE_TRIES: usize = 8;

/// Fallback ranking key for a trial placement: (non-doubling, floor-clamped
/// C1, routes in band, raw C1) — avoid dead pocket-doubles first, reach the
/// floor, then keep routes, then cost.
type TrialKey = (bool, u32, usize, u32);

pub(crate) struct SparePipes;

impl Phase for SparePipes {
    fn name(&self) -> &'static str {
        "spare_pipes"
    }

    fn run(&self, state: &mut WorldState, rng: &mut dyn RngCore) -> PhaseReport {
        let mut actions = Vec::new();
        let mut placed_any = false;

        // Pipe-free pocket ids are stable across spare placement (a new
        // mouth was a blank in some pocket already), so compute once.
        let (pocket, _) = pocket_map(state);

        while state.pipe_pairs.len() < state.pipe_budget {
            // Anchor-adjacent blanks are barred for pipe endpoints (an
            // ungateable express) — but placement is mandatory, so fall
            // back to the full set rather than shorting the budget.
            let legal = state.legal_blanks();
            let clear: Vec<Pos> = legal
                .iter()
                .copied()
                .filter(|&p| !state.near_anchor(p))
                .collect();
            let candidates = if clear.len() >= 2 { clear } else { legal };
            if candidates.len() < 2 {
                actions.push(format!(
                    "stuck: {} blanks left, need 2 for a pipe pair",
                    candidates.len(),
                ));
                break;
            }

            let linked = linked_pocket_pairs(&pocket, &state.pipe_pairs);
            let before = measure_world(state);
            let mut creator: Option<(Pos, Pos)> = None;
            let mut harmless: Option<(Pos, Pos)> = None;
            // Fallback ranking: non-doubling, then floor, routes, C1.
            let mut best: Option<(TrialKey, (Pos, Pos))> = None;
            for _ in 0..SPARE_TRIES {
                let picked: Vec<Pos> = candidates.choose_multiple(rng, 2).copied().collect();
                let [a, b] = picked[..] else { break };
                if Some(b) == row78_partner(a) {
                    continue;
                }
                let doubling = match (pocket.get(&a), pocket.get(&b)) {
                    (Some(&pa), Some(&pb)) if pa != pb => {
                        linked.contains(&(pa.min(pb), pa.max(pb)))
                    }
                    _ => false,
                };
                state.add_pipe_pair(a, b);
                let m = measure_world(state);
                state.pop_pipe_pair();
                if m.c1 >= C1_FLOOR && m.routes_in_band >= before.routes_in_band {
                    if m.routes_in_band > before.routes_in_band {
                        creator = Some((a, b));
                        break;
                    }
                    if !doubling && harmless.is_none() {
                        harmless = Some((a, b));
                    }
                }
                let key = (!doubling, m.c1.min(C1_FLOOR), m.routes_in_band, m.c1);
                if best.as_ref().is_none_or(|(k, _)| key > *k) {
                    best = Some((key, (a, b)));
                }
            }
            let Some((a, b)) = creator.or(harmless).or(best.map(|(_, pair)| pair)) else {
                actions.push("stuck: no candidate pair could be formed".into());
                break;
            };
            state.add_pipe_pair(a, b);
            placed_any = true;
            let after = measure_world(state);
            actions.push(format!(
                "pipe {a:?} <-> {b:?}: routes {} -> {}, C1 {} -> {}",
                before.routes_in_band, after.routes_in_band, before.c1, after.c1,
            ));
        }

        // New topology can change which locks may stay sealed forever — the
        // cross-world secret-exit backstop reads these flags next.
        if placed_any {
            recompute_safety_flags(state);
        }

        actions.push(format!(
            "done: {}/{} pipe pairs on the map",
            state.pipe_pairs.len(),
            state.pipe_budget,
        ));
        PhaseReport { phase: self.name(), actions }
    }
}
