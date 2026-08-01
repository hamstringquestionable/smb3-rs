//! Connectivity phase — the first placement phase.
//!
//! Job: make the world traversable. Every placeable blank and the world goal
//! should be walk-reachable from start; terrain the walker can't cross
//! (water, gaps between screens) gets bridged by teleport pipe pairs — one
//! endpoint on the already-reachable mainland, one on the cut-off island.
//!
//! Deliberately KNOB-FREE: both endpoints are uniform-random picks from the
//! legal candidates. Where an endpoint *should* land (junction vs dead end,
//! near vs far) is a routing question, and the whole point of the
//! rediscovery is to let measurement justify every control before it exists.
//! The phase spends as few pipes as possible (one pair per island) and
//! leaves the rest of the world's pipe budget for the routing phase.
//!
//! One measured-and-earned placement bar: no endpoint within
//! `PIPE_ANCHOR_RADIUS` of the start or goal (census-diagnosed — an
//! anchor-adjacent pipe mouth is an ungateable express no later phase can
//! price back up), with a full-set fallback when an island has no clear
//! candidate, because bridging outranks the bar.
//!
//! One structural addition after bridging: the LOOP CLOSER. Spanning
//! bridges alone leave the pocket graph a tree — every route threads the
//! same pocket sequence, so no second route can exist no matter where
//! later phases put content (W7 census 2026-08-01: ~45% linear, and
//! choice tracked the cycles spare pipes closed by accident; vanilla W7
//! wires the same 8 pipes into a loop with two arms tied at cost 35).
//! When the world had 2+ pockets and budget remains, ONE extra pair goes
//! between two pockets that aren't already directly linked, turning the
//! tree into a loop the shaping rungs can price. Endpoints stay
//! uniform-random over the legal candidates.
//!
//! Not hard rules, by design: a world can end this phase with unreachable
//! blanks (an island whose every blank is `fixed` has no legal endpoint, or
//! the pipe budget runs out first). The phase reports what it couldn't do;
//! the census measures how often.
//!
//! The one hard bound is `state.pipe_budget` — the world's chosen pipe
//! limit (see its doc). Real terrain always connects within it, so hitting
//! the bound means a bug, contained to this world's few pipes instead of
//! looping; it also guarantees the island loop terminates.

use super::*;

use super::islands::{blank_positions, classify, linked_pocket_pairs, pocket_map, spread_mouth};
use rand::seq::IndexedRandom;

pub(crate) struct Connectivity;

impl Phase for Connectivity {
    fn name(&self) -> &'static str {
        "connectivity"
    }

    fn run(&self, state: &mut WorldState, rng: &mut dyn RngCore) -> PhaseReport {
        let mut actions = Vec::new();

        // Islands are pipe-free components — stable across everything this
        // phase does, so decompose and classify once.
        let (pocket, pocket_count) = pocket_map(state);
        let islands = classify(state, &pocket, pocket_count);

        while state.pipe_pairs.len() < state.pipe_budget {
            let reach = walk_reachable(&state.grid, &state.pipe_pairs, state.start, state.world_idx);
            let blanks = blank_positions(state);

            let Some(island_seed) = next_island_seed(state, &reach, &blanks) else {
                break; // everything that can be reached is reached
            };

            // The island = whatever the walker can cover starting from the
            // seed. Endpoint candidacy uses the shared `legal_blanks` rules
            // (not fixed, not taken, row-7/8 partner respected — a pipe is
            // completable content too), split by which side of the cut each
            // blank sits on. Anchor-adjacent blanks are barred (an endpoint
            // next to start/goal is an ungateable express) — but bridging is
            // mandatory, so a side whose every candidate is near an anchor
            // falls back to the full set rather than stranding the island.
            let island_reach =
                walk_reachable(&state.grid, &state.pipe_pairs, Some(island_seed), state.world_idx);
            let legal = state.legal_blanks();
            let pick_side = |on_side: &dyn Fn(&Pos) -> bool| -> Vec<Pos> {
                let side: Vec<Pos> = legal.iter().copied().filter(|p| on_side(p)).collect();
                let clear: Vec<Pos> =
                    side.iter().copied().filter(|&p| !state.near_anchor(p)).collect();
                if clear.is_empty() { side } else { clear }
            };
            let island_candidates = pick_side(&|p| island_reach.contains(*p));
            let mainland_candidates = pick_side(&|p| reach.contains(*p));

            let (Some(&near), Some(&far)) =
                (mainland_candidates.choose(rng), island_candidates.choose(rng))
            else {
                actions.push(format!(
                    "stuck: island at {island_seed:?} has no legal endpoint ({} island / {} mainland candidates)",
                    island_candidates.len(),
                    mainland_candidates.len(),
                ));
                break;
            };

            // Spread-mouths refinement (uniform pick keeps the cross-island
            // distribution; the refinement fixes WHERE on the island): on a
            // routing/corridor/final island that already has a mouth, land
            // the new mouth as far from the existing ones as the island's
            // walk network allows, so traversal crosses the interior
            // instead of hugging one corner.
            let rn = spread_mouth(state, &pocket, &islands, &mainland_candidates, near);
            let rf = spread_mouth(state, &pocket, &islands, &island_candidates, far);
            // Keep the originals if refinement manufactured a row-7/8
            // partner pair.
            let (near, far) = if Some(rf) == row78_partner(rn) { (near, far) } else { (rn, rf) };

            state.add_pipe_pair(near, far);
            actions.push(format!("pipe {near:?} <-> {far:?} (island of {} blanks)", island_candidates.len()));
        }

        // Loop closer: one pair between two islands with no direct link yet
        // (see module doc). Purely optional — no fallback past the anchor
        // bar, and single-pocket worlds skip it (their pipes are all free
        // routing ammo already). A stricter "leave shortcut headroom" rule
        // was tried and reverted: it only ever throttled W7 (the world the
        // loop is FOR — census 2026-08-01, linear 40% -> 44%), while the
        // world it meant to protect (W8) never fires the closer at all
        // (its extra pockets have no legal endpoints).
        if state.pipe_pairs.len() < state.pipe_budget && pocket_count >= 2 {
            let linked = linked_pocket_pairs(&pocket, &state.pipe_pairs);
            let legal: Vec<Pos> = state
                .legal_blanks()
                .into_iter()
                .filter(|&p| !state.near_anchor(p))
                .collect();
            let mut cands: Vec<(Pos, Pos)> = Vec::new();
            for (i, &a) in legal.iter().enumerate() {
                let Some(&pa) = pocket.get(&a) else { continue };
                for &b in &legal[i + 1..] {
                    let Some(&pb) = pocket.get(&b) else { continue };
                    if pa != pb
                        && loop_eligible(&islands, pa, pb)
                        && !linked.contains(&(pa.min(pb), pa.max(pb)))
                        && Some(b) != row78_partner(a)
                    {
                        cands.push((a, b));
                    }
                }
            }
            if let Some(&(a, b)) = cands.choose(rng) {
                let ra = spread_mouth(state, &pocket, &islands, &legal, a);
                let rb = spread_mouth(state, &pocket, &islands, &legal, b);
                // Refinement moves picks within their islands; keep the
                // originals if it manufactured a row-7/8 partner pair.
                let (a, b) = if Some(rb) == row78_partner(ra) { (a, b) } else { (ra, rb) };
                state.add_pipe_pair(a, b);
                actions.push(format!(
                    "loop pipe {a:?} <-> {b:?} (pockets {} <-> {} of {pocket_count})",
                    pocket[&a], pocket[&b],
                ));
            }
        }

        // Count what remains cut off — measured, not enforced. Hammer-gated
        // pockets are deliberately left alone, so they are reported apart
        // from genuine stranding.
        let reach = walk_reachable(&state.grid, &state.pipe_pairs, state.start, state.world_idx);
        let stranded = blank_positions(state)
            .iter()
            .filter(|p| !reach.contains(**p) && !state.hammer_gated.contains(p))
            .count();
        let goal_ok = state.target.is_none_or(|t| reach.contains(t));
        let gated_note = if state.hammer_gated.is_empty() {
            String::new()
        } else {
            format!(", {} hammer-gated left alone", state.hammer_gated.len())
        };
        actions.push(format!(
            "done: {stranded} blanks stranded{gated_note}, goal reachable: {goal_ok}"
        ));

        PhaseReport { phase: self.name(), actions }
    }
}

/// Which island pairs the loop closer may join: any two distinct islands.
/// A routing/final-only restriction was tried and REJECTED (census
/// 2026-08-01: W7 linear 25% -> 32%): the pair only picks where the new
/// EDGE lands — the cycle it closes runs through the existing tree path
/// between the two islands, which crosses the big islands regardless, and
/// under spread mouths even a small island's arc can carry a level. The
/// restriction just shrank candidate diversity.
fn loop_eligible(_islands: &[super::islands::Island], _pa: usize, _pb: usize) -> bool {
    true
}

/// Where to grow next: the goal's component first (a world you can't finish
/// is the worst kind of cut off), then the first stranded blank in scan
/// order — skipping hammer-gated pockets, which are not stranded (and, being
/// excluded from `legal_blanks`, could never take an endpoint). `None` when
/// start is unknown or nothing is stranded.
fn next_island_seed(state: &WorldState, reach: &Reach, blanks: &[Pos]) -> Option<Pos> {
    state.start?;
    if let Some(target) = state.target
        && !reach.contains(target)
    {
        return Some(target);
    }
    blanks
        .iter()
        .copied()
        .find(|p| !reach.contains(*p) && !state.hammer_gated.contains(p))
}

