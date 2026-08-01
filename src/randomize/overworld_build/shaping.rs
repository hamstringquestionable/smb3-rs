//! Shaping phase — the diagnosis-driven improvement loop, run as the LAST
//! phase in the schedule on the completed dumb world.
//!
//! Every earlier phase stays knob-free; all judgment concentrates here. The
//! loop reads the world through [`measure_world`], and the measured symptom
//! selects the move — no move portfolio is evaluated side by side:
//!
//! - **Satisfied** (routes in band ≥ [`TARGET_ROUTES`] AND C1 ≥ the shipping
//!   builder's [`C1_FLOOR`], reused verbatim): no move at all. Satisficing,
//!   not optimizing — worlds the dumb roll already made interesting keep
//!   their shape, and the across-seed diversity the uniform baseline bought
//!   is only spent where the measure says the world is linear or nearly
//!   free (a C1-4 world is beat-Bowser-after-one-level; the floor is the
//!   same "not skippable" guarantee v1 ended up needing).
//! - **COST MODE** (C1 below the floor): the floor comes FIRST — the choice
//!   band is relative to C1, so choice shaped on a trivial trunk is trivial
//!   too. Every rung except the shortcut (pipes only cheapen) stays
//!   available regardless of route count, and every acceptance switches to
//!   "C1 rose" — routes may be SPENT: gating an express collapses its
//!   variants into one route, so demanding "routes not down" vetoes the one
//!   working move (the W4 sub-floor tail was exactly this refusal, census
//!   2026-07-31). Satisfaction still demands routes ≥ 2, so the choice
//!   ladder re-earns above the floor what cost mode spent below it. Only
//!   once the floor holds does the loop shape choice.
//! - **Linear with lock ammunition** (zero-gate locks, or detour structure a
//!   trunk gate could pull into the band): **lock re-place** — the cheapest
//!   rung. Re-runs lock placement from scratch via `place_locks_gating`
//!   (v1's #125 lesson: re-deciding the whole coupled set beats nudging one
//!   lock).
//! - **Linear with no raw material** (no dominated detours): only new
//!   topology can help — walk edges are local, so a pipe is the one move
//!   that creates a route that doesn't exist yet. **Gated shortcut**: add a
//!   pipe pair, re-place ALL locks on the new topology, judge the bundle.
//! - **Linear with a wide runner-up or detour** (a second route exists
//!   within [`SHAPING_SLACK`] but outside the band, or dominated): **arm
//!   balance** — move a level onto the cheap route's EXCLUSIVE stretch,
//!   closing the cost gap from the C1 side without touching the runner-up
//!   (and breaking a detour's superset relation, which un-dominates it).
//!   The generic level move can't do this: it targets the whole trunk, and
//!   a level on a SHARED stretch raises both routes alike, leaving the gap
//!   fixed. Vanilla W7 is the precedent — two pocket arms tied at cost 35.
//!   After the shortcut: creation first, then trimming what it created.
//! - **Both cheaper rungs spent** (exhausted or unavailable): **fort+lock
//!   move** — relocate one fort and re-place all locks as one bundle. The
//!   trunk-fort symptom: a fort ON the cheapest route makes every lock
//!   decorative and no lock or pipe move can fix that; only moving the fort
//!   changes the lock's search space.
//! - **Everything spent** (last resort — it spends layout diversity, the
//!   scarcest currency): **level move** — relocate one off-route level onto
//!   the cheapest route, making it mandatory. The cheap-trunk symptom: when
//!   C1 is tiny, no alternative can fit the choice band no matter where
//!   forts, locks, or pipes go; each forced level raises C1 by 3 and widens
//!   what the band admits, and every accept resets the other rungs'
//!   exhaustion so they retry into the wider band.
//! - **Cost mode's last resort — pipe move**: relocate one mouth of an
//!   existing pipe pair. When the express IS a pipe (no single lockable
//!   tile cuts the goal, levels out of trunk targets), the mouth itself is
//!   the only thing left to move. The vacated tile returns to an unclaimed
//!   blank; global connectivity (every slot + goal reachable) is a hard
//!   guard.
//!
//! Moves are judged on the complete world and rolled back on rejection; a
//! move type that keeps failing escalates past itself ([`ESCALATE_AFTER`]),
//! so most iterations cost one proposal, not five. Acceptance is minimal —
//! routes up (or, for lock re-place, zero-gate down at equal routes) — PLUS
//! the floor guard: every choice-mode acceptance requires `after.c1 >=
//! C1_FLOOR`. The original design had no C1 guard, arguing that for the
//! route count to rise the old structure must stay within the band of the
//! new cheapest, so trivializing shortcuts reject themselves. Measured
//! FALSE (census 2026-07-31): one new pipe can create two brand-new cheap
//! routes at once — "routes rose" held while C1 crashed 18 -> 5, on ~8% of
//! all worlds (59% of accepted W7 shortcuts) — and the wreck was silently
//! repaired by cost mode spending level moves, or by a web redeal blaming
//! healthy connectivity. With the guard the two modes ratchet: cost mode
//! may spend routes to buy the floor, choice mode may spend anything BUT
//! the floor.
//!
//! The loop's knobs (the ONLY knobs in the builder): [`MOVE_BUDGET`],
//! [`TARGET_ROUTES`], [`SHORTCUT_TRIES`], [`ARM_BALANCE_TRIES`],
//! [`FORTLOCK_TRIES`], [`LEVELMOVE_TRIES`], [`PIPEMOVE_TRIES`],
//! [`ESCALATE_AFTER`].

use super::*;

use rand::seq::{IndexedRandom, SliceRandom};

use super::locks::{place_locks_gating, recompute_safety_flags};
use super::metrics::WorldMeasure;

/// Max moves (accepted or rejected) per world.
const MOVE_BUDGET: usize = 24;
/// Satisficing target: stop once this many routes are in the choice band.
const TARGET_ROUTES: usize = 2;
/// Candidate pipe pairs tried inside ONE gated-shortcut move.
const SHORTCUT_TRIES: usize = 4;
/// Level relocations tried inside ONE arm-balance move.
const ARM_BALANCE_TRIES: usize = 4;
/// Full fort-relocation evaluations paid inside ONE fort+lock move.
const FORTLOCK_TRIES: usize = 4;
/// Level relocations tried inside ONE level move (cheap: no lock re-place).
const LEVELMOVE_TRIES: usize = 4;
/// Full endpoint-relocation evaluations paid inside ONE pipe move.
const PIPEMOVE_TRIES: usize = 4;
/// Consecutive rejections of a move type before escalating past it.
const ESCALATE_AFTER: usize = 2;

pub(crate) struct Shaping;

impl Phase for Shaping {
    fn name(&self) -> &'static str {
        "shaping"
    }

    fn run(&self, state: &mut WorldState, rng: &mut dyn RngCore) -> PhaseReport {
        let mut actions = Vec::new();
        let mut moves = 0usize;
        let mut accepted = 0usize;
        // Per-rung consecutive-rejection counters, indexed by ladder order:
        // lock re-place, gated shortcut, arm balance, fort+lock, level move,
        // pipe move. The shortcut CREATES wide routes, arm balance prices
        // them into the band — creation before trimming (measured: balance
        // first displaced W8's shortcut accepts, linear 8% -> 11%).
        let mut rejects = [0usize; 6];

        let status = loop {
            let before = measure_world(state);
            let routes_needy = before.routes_in_band < TARGET_ROUTES;
            let cheap = before.c1 < C1_FLOOR;
            if !routes_needy && !cheap {
                break "satisfied";
            }
            if moves >= MOVE_BUDGET {
                break "budget spent";
            }

            let zero_gate = state.zero_gate_locks().len();
            // COST MODE below the floor: the choice band is relative to C1,
            // so choice shaped on a trivial trunk is trivial too — a
            // sub-floor world does nothing else until the floor holds. Every
            // rung except the shortcut (pipes only cheapen) is available and
            // accepts on C1 progress; the try_ fns branch their acceptance
            // on the same `before.c1 < C1_FLOOR` test. Above the floor, the
            // normal choice ladder.
            let available = if cheap {
                // The pipe move is the final cost rung: when no lock cuts
                // the goal and levels run out of trunk targets, the express
                // itself (a pipe mouth) is what must move. Arm balance is a
                // choice tool only — cost mode has its own level rung.
                [
                    !state.locks.is_empty(),
                    false,
                    false,
                    state.fort_count() > 0,
                    true,
                    !state.pipe_pairs.is_empty(),
                ]
            } else {
                [
                    routes_needy
                        && !state.locks.is_empty()
                        && (zero_gate > 0 || before.dominated_detours > 0),
                    routes_needy && state.pipe_pairs.len() < state.pipe_budget,
                    routes_needy, // arm balance finds its own wide runner-up
                    routes_needy && state.fort_count() > 0,
                    true, // level move checks its own sources/targets
                    false, // pipe move is a cost tool only
                ]
            };
            let Some(rung) = (0..6)
                .find(|&r| available[r] && rejects[r] < ESCALATE_AFTER)
            else {
                break "stuck: no move available";
            };

            moves += 1;
            let outcome = match rung {
                0 => try_lock_replace(state, rng, &before, zero_gate),
                1 => try_gated_shortcut(state, rng, &before),
                2 => try_arm_balance(state, rng, &before),
                3 => try_fort_lock(state, rng, &before),
                4 => try_level_move(state, rng, &before),
                _ => try_pipe_move(state, rng, &before),
            };
            match outcome {
                Ok(line) => {
                    // The world changed — every rung's ammunition is fresh.
                    rejects = [0; 6];
                    accepted += 1;
                    actions.push(line);
                }
                Err(line) => {
                    rejects[rung] += 1;
                    actions.push(line);
                }
            }
        };

        if accepted > 0 {
            recompute_safety_flags(state);
        }
        let m = measure_world(state);
        actions.push(format!(
            "done ({status}): {accepted}/{moves} moves accepted, routes {}, C1 {}, {} zero-gate locks",
            m.routes_in_band,
            m.c1,
            state.zero_gate_locks().len(),
        ));
        PhaseReport { phase: self.name(), actions }
    }
}

/// The cheap rung: re-place every lock, preferring gating candidates. Accept
/// iff routes rise, or (routes flat) decorative locks were converted into
/// gating ones.
fn try_lock_replace(
    state: &mut WorldState,
    rng: &mut dyn RngCore,
    before: &WorldMeasure,
    zero_gate_before: usize,
) -> Result<String, String> {
    let saved = state.locks.clone();
    // Below the floor, ask for a goal gate: the pipe-proof cut that raises
    // C1 when nothing else does.
    if !place_locks_gating(state, rng, before.c1 < C1_FLOOR) {
        state.locks = saved;
        return Err("lock_replace REJECT: could not lock every fort".into());
    }
    let after = measure_world(state);
    let zero_gate_after = state.zero_gate_locks().len();
    // Below the floor, cost first: accept iff C1 rose. Routes may drop — a
    // goal gate collapses the express variants it prices up, and the choice
    // ladder re-earns routes once the floor holds.
    let improved = if before.c1 < C1_FLOOR {
        after.c1 > before.c1
    } else {
        (after.routes_in_band > before.routes_in_band
            || (after.routes_in_band == before.routes_in_band
                && zero_gate_after < zero_gate_before))
            && after.c1 >= C1_FLOOR
    };
    let line = format!(
        "routes {} -> {}, C1 {} -> {}, zero-gate {zero_gate_before} -> {zero_gate_after}",
        before.routes_in_band, after.routes_in_band, before.c1, after.c1,
    );
    if improved {
        Ok(format!("lock_replace ACCEPT: {line}"))
    } else {
        state.locks = saved;
        Err(format!("lock_replace REJECT: {line}"))
    }
}

/// The parity rung: when a runner-up route exists in the WIDE window
/// ([`SHAPING_SLACK`]) but outside the choice band, close the cost gap
/// surgically — move a level that is NOT on the cheap route onto a blank on
/// the cheap route's EXCLUSIVE stretch (on the cheap path, off the
/// runner-up's). Each move raises C1 by 3 while leaving the runner-up
/// priced as it was (and cheapens it by 3 when the source level sat on it),
/// so the gap closes without the whole world re-pricing. The generic level
/// move can't do this: its targets are the whole trunk, and a level landing
/// on a stretch SHARED with the runner-up raises both costs alike — gap
/// unchanged, noalt stays noalt. Locks untouched, so an evaluation is
/// relocate + measure, same as the level move.
///
/// The runner-up can be a DOMINATED detour: on tree-ish maps the loop's
/// second arm usually plays a superset of the cheap route's levels (its
/// exclusive stretch holds none of its own), so it sits in `detours`, not
/// `routes` — invisible to the band. The same move fixes that too: a level
/// on the cheap route's exclusive stretch is a level the detour does NOT
/// play, so the superset relation breaks and the detour becomes a real
/// alternative. Accept iff routes rise, a wide route materialized (the
/// detour split off), or the wide gap shrank at flat routes (floor held) —
/// a 9-point gap takes multiple moves, and the early ones buy no route
/// yet. Vanilla W7 is the design precedent: two pocket arms tied at cost
/// 35 (the loop closer in connectivity builds the arms; this rung prices
/// them).
fn try_arm_balance(
    state: &mut WorldState,
    rng: &mut dyn RngCore,
    before: &WorldMeasure,
) -> Result<String, String> {
    let wide_before = analyze_route_choice(&state.to_built(), SHAPING_SLACK);
    let cheap: HashSet<Pos> = match wide_before.routes.first() {
        Some(r) => r.path.iter().copied().collect(),
        None => return Err("arm_balance REJECT: no route at all".into()),
    };
    let blanks = state.legal_blanks();

    // Runner-up material: the kept wide route, else the cheapest dominated
    // detour this move can actually work with — one whose cheap-exclusive
    // stretch holds a legal blank. Detours that differ by a path-tile-only
    // bypass (no node on the stretch) are lock material, not level
    // material; skip them.
    let alt_candidates = if wide_before.routes.len() >= 2 {
        std::slice::from_ref(&wide_before.routes[1])
    } else {
        &wide_before.detours[..]
    };
    let usable = alt_candidates.iter().find_map(|r| {
        let alt: HashSet<Pos> = r.path.iter().copied().collect();
        let targets: Vec<Pos> = blanks
            .iter()
            .copied()
            .filter(|p| cheap.contains(p) && !alt.contains(p))
            .collect();
        (!targets.is_empty()).then_some((r, targets))
    });
    let Some((alt_route, targets)) = usable else {
        return Err(format!(
            "arm_balance REJECT: no runner-up/detour with an exclusive blank ({} candidates)",
            alt_candidates.len(),
        ));
    };
    let gap_before = alt_route.cost - wide_before.routes[0].cost;

    let sources: Vec<usize> = state
        .slots
        .iter()
        .enumerate()
        .filter(|(_, s)| s.kind == SlotKind::Level && !cheap.contains(&s.pos))
        .map(|(i, _)| i)
        .collect();
    if sources.is_empty() {
        return Err(format!(
            "arm_balance REJECT: gap {gap_before}, no off-cheap levels to move",
        ));
    }

    let saved_slots = state.slots.clone();
    for _ in 0..ARM_BALANCE_TRIES {
        let Some(&si) = sources.choose(rng) else { break };
        let Some(&target) = targets.choose(rng) else { break };
        let old_pos = state.slots[si].pos;
        state.slots[si].pos = target;
        let after = measure_world(state);
        let wide_after = analyze_route_choice(&state.to_built(), SHAPING_SLACK);
        let gap_after = if wide_after.routes.len() >= 2 {
            wide_after.routes[1].cost - wide_after.routes[0].cost
        } else {
            u32::MAX
        };
        // The gap-shrink tier only applies when the alt was a KEPT
        // runner-up — after a detour-splitting move the before/after gaps
        // would describe different detours.
        let had_runner_up = wide_before.routes.len() >= 2;
        let improved = after.c1 >= C1_FLOOR
            && (after.routes_in_band > before.routes_in_band
                || (after.routes_in_band == before.routes_in_band
                    && (wide_after.routes.len() > wide_before.routes.len()
                        || (had_runner_up && gap_after < gap_before))));
        if improved {
            return Ok(format!(
                "arm_balance ACCEPT: level {old_pos:?} -> {target:?} (cheap-exclusive), routes {} -> {}, wide {} -> {}, gap {gap_before} -> {}, C1 {} -> {}",
                before.routes_in_band,
                after.routes_in_band,
                wide_before.routes.len(),
                wide_after.routes.len(),
                if gap_after == u32::MAX { "-".into() } else { gap_after.to_string() },
                before.c1,
                after.c1,
            ));
        }
        state.slots = saved_slots.clone();
    }
    Err(format!(
        "arm_balance REJECT: no exclusive-stretch move closes gap {gap_before} ({ARM_BALANCE_TRIES} tries)",
    ))
}

/// The topology rung: spend one pipe pair AND re-place all locks as a single
/// bundle, judged together on the finished world. Accept iff the pipe
/// CREATED a distinct route — measured with the WIDE stick
/// ([`SHAPING_SLACK`]): a new route anywhere within C1+9 counts, in band or
/// not — AND the floor holds ("express" pairs stay rejected: a pair can
/// manufacture two brand-new cheap routes, so "routes rose" alone passed
/// while C1 crashed sub-floor — the measured crash mode, 59% of accepted W7
/// shortcuts before the guard).
///
/// Parity is the LOOP's job, not the proposal's (2026-07-31 design): the
/// narrow band demanded a random pair land within 3 points of C1 in one
/// draw (~4% hit rate); a new off-price route is kept as structure, and the
/// existing rungs walk it into the band — each above-floor level-move trim
/// (+3 to C1) widens the absolute band until the route falls in.
///
/// Proposal seeding was tried and REVERTED (census-measured, 100 seeds):
/// a pipe-only measurement pre-screen ("skip pairs whose pipe changes
/// nothing against the current locks") filtered out true positives — a pipe
/// inert against the CURRENT locks can still create a route once locks are
/// re-placed, which is the entire point of the bundle (W8 accepts halved,
/// linear 73→84%). A walk-distance floor on the endpoints was neutral:
/// uniform pairs are rarely walk-close, so it filtered almost nothing.
/// Any future proposal filter must judge the pipe+locks bundle, not the
/// pipe alone.
fn try_gated_shortcut(
    state: &mut WorldState,
    rng: &mut dyn RngCore,
    before: &WorldMeasure,
) -> Result<String, String> {
    let saved_grid = state.grid.clone();
    let saved_slots = state.slots.clone();
    let saved_locks = state.locks.clone();
    let saved_pairs = state.pipe_pairs.clone();

    let wide_before = analyze_route_choice(&state.to_built(), SHAPING_SLACK).routes.len();

    for _ in 0..SHORTCUT_TRIES {
        // Anchor-adjacent blanks are barred for pipe endpoints — same rule
        // as connectivity and spare pipes (an ungateable express).
        let candidates: Vec<Pos> = state
            .legal_blanks()
            .into_iter()
            .filter(|&p| !state.near_anchor(p))
            .collect();
        let picked: Vec<Pos> = candidates.choose_multiple(rng, 2).copied().collect();
        let [a, b] = picked[..] else { break };
        if Some(b) == row78_partner(a) {
            continue;
        }
        state.add_pipe_pair(a, b);
        // Shortcut moves only run above the floor — no goal gate wanted.
        if place_locks_gating(state, rng, false) {
            let after = measure_world(state);
            if after.c1 >= C1_FLOOR {
                // In-band already: parity for free.
                if after.routes_in_band > before.routes_in_band {
                    return Ok(format!(
                        "gated_shortcut ACCEPT: pipe {a:?} <-> {b:?}, routes {} -> {}, C1 {} -> {}",
                        before.routes_in_band, after.routes_in_band, before.c1, after.c1,
                    ));
                }
                // New route within the wide band: keep the structure; the
                // loop's trims walk it to parity.
                let wide_after =
                    analyze_route_choice(&state.to_built(), SHAPING_SLACK).routes.len();
                if wide_after > wide_before {
                    return Ok(format!(
                        "gated_shortcut ACCEPT (wide): pipe {a:?} <-> {b:?}, wide routes {wide_before} -> {wide_after}, C1 {} -> {}",
                        before.c1, after.c1,
                    ));
                }
            }
        }
        state.grid = saved_grid.clone();
        state.slots = saved_slots.clone();
        state.locks = saved_locks.clone();
        state.pipe_pairs = saved_pairs.clone();
    }
    Err(format!(
        "gated_shortcut REJECT: no pair in {SHORTCUT_TRIES} tries creates a route (band or wide {wide_before})",
    ))
}

/// The escalation rung: relocate ONE fort AND re-place all locks as a single
/// bundle judged on the finished world. Jurisdiction is the trunk-fort
/// symptom: a fort sitting on the cheapest route makes every lock decorative
/// (the player passes it regardless), and neither lock re-place nor a
/// shortcut can manufacture an alternative — only moving the fort changes
/// what its lock can gate. Accept iff routes rise.
///
/// Forts on the cheapest route are proposed first (shuffled within each
/// group). That ordering comes from the full-world measure already in hand —
/// per the seeding lesson, the bundle is still the only thing judged.
fn try_fort_lock(
    state: &mut WorldState,
    rng: &mut dyn RngCore,
    before: &WorldMeasure,
) -> Result<String, String> {
    let saved_slots = state.slots.clone();
    let saved_locks = state.locks.clone();

    let trunk: HashSet<Pos> = before
        .rc
        .routes
        .first()
        .map(|r| r.path.iter().copied().collect())
        .unwrap_or_default();

    let mut fort_indices: Vec<usize> = state
        .slots
        .iter()
        .enumerate()
        .filter(|(_, s)| s.kind == SlotKind::Fortress)
        .map(|(i, _)| i)
        .collect();
    fort_indices.shuffle(rng);
    // Stable sort: trunk forts first, shuffled order kept within each group.
    fort_indices.sort_by_key(|&i| !trunk.contains(&state.slots[i].pos));

    let mut evals = 0usize;
    for &fi in &fort_indices {
        if evals >= FORTLOCK_TRIES {
            break;
        }
        let fort_id = state.slots[fi].section;
        let old_pos = state.slots[fi].pos;
        for _ in 0..2 {
            if evals >= FORTLOCK_TRIES {
                break;
            }
            let candidates = state.legal_blanks();
            let Some(&new_pos) = candidates.choose(rng) else { break };
            state.slots[fi].pos = new_pos;
            evals += 1;
            if place_locks_gating(state, rng, before.c1 < C1_FLOOR) {
                let after = measure_world(state);
                // Below the floor: any relocation that raises C1 is
                // progress (routes may be spent). Above: routes must rise.
                let improved = if before.c1 < C1_FLOOR {
                    after.c1 > before.c1
                } else {
                    after.routes_in_band > before.routes_in_band && after.c1 >= C1_FLOOR
                };
                if improved {
                    return Ok(format!(
                        "fort_lock ACCEPT: fort {fort_id} {old_pos:?} -> {new_pos:?}, routes {} -> {}, C1 {} -> {}",
                        before.routes_in_band, after.routes_in_band, before.c1, after.c1,
                    ));
                }
            }
            state.slots = saved_slots.clone();
            state.locks = saved_locks.clone();
        }
    }
    Err(format!(
        "fort_lock REJECT: no relocation beats routes {} ({evals} full evals)",
        before.routes_in_band,
    ))
}

/// The last-resort rung: relocate ONE off-route level onto the cheapest
/// route, making it mandatory. Deliberately NARROW — locks are untouched
/// (level positions affect neither lock validity nor the completability
/// fixpoint), so an evaluation is just relocate + measure.
///
/// Jurisdiction is the cheap-trunk symptom (a C1 so small that no
/// alternative can fit the choice band anywhere) plus the mandatory-level
/// deficit the progression census flagged. Accept iff routes rise, or C1
/// rises at flat routes — the level genuinely became forced, the absolute
/// band widened, and the accept resets the other rungs' exhaustion so
/// shortcut/fort+lock retry into the wider band. Last in the ladder because
/// it spends layout diversity, the census's scarcest currency.
fn try_level_move(
    state: &mut WorldState,
    rng: &mut dyn RngCore,
    before: &WorldMeasure,
) -> Result<String, String> {
    let trunk: HashSet<Pos> = before
        .rc
        .routes
        .first()
        .map(|r| r.path.iter().copied().collect())
        .unwrap_or_default();

    let sources: Vec<usize> = state
        .slots
        .iter()
        .enumerate()
        .filter(|(_, s)| s.kind == SlotKind::Level && !trunk.contains(&s.pos))
        .map(|(i, _)| i)
        .collect();
    let targets: Vec<Pos> = state
        .legal_blanks()
        .into_iter()
        .filter(|p| trunk.contains(p))
        .collect();
    if sources.is_empty() || targets.is_empty() {
        return Err(format!(
            "level_move REJECT: {} off-route levels, {} on-route blanks — nothing to work with",
            sources.len(),
            targets.len(),
        ));
    }

    let saved_slots = state.slots.clone();
    let mut evals = 0usize;
    for _ in 0..LEVELMOVE_TRIES {
        let Some(&si) = sources.choose(rng) else { break };
        let Some(&target) = targets.choose(rng) else { break };
        let old_pos = state.slots[si].pos;
        if old_pos == target {
            continue;
        }
        state.slots[si].pos = target;
        evals += 1;
        let after = measure_world(state);
        // Below the floor: C1 progress (routes may be spent). Above: routes
        // up, or the fine +3 trim at flat routes.
        let improved = if before.c1 < C1_FLOOR {
            after.c1 > before.c1
        } else {
            (after.routes_in_band > before.routes_in_band
                || (after.routes_in_band == before.routes_in_band && after.c1 > before.c1))
                && after.c1 >= C1_FLOOR
        };
        if improved {
            return Ok(format!(
                "level_move ACCEPT: level {old_pos:?} -> {target:?} (on trunk), routes {} -> {}, C1 {} -> {}",
                before.routes_in_band, after.routes_in_band, before.c1, after.c1,
            ));
        }
        state.slots = saved_slots.clone();
    }
    Err(format!(
        "level_move REJECT: no on-trunk relocation improves ({evals} evals)",
    ))
}

/// The pipe-web rung (cost mode only): relocate ONE endpoint of an existing
/// pipe pair to a fresh blank elsewhere. Owns the residual no other rung
/// can touch — a sub-floor world whose express IS a pipe mouth (wide goal
/// approach fed by a pipe, no single lockable tile cuts it): when no lock
/// placement can price the world up, the mouth itself must move.
///
/// The vacated tile returns to being an unclaimed blank (the future
/// hammer-bro pool), restored to its theme blank via `blank_tile_for`.
/// Connectivity is enforced globally: after the move, every slot and the
/// goal must still be walk-reachable — the "new spot must be walkable from
/// the old one" condition in its chain-safe form. Locks are re-placed on
/// the new topology (goal-first) and the bundle is judged whole; accept on
/// the cost rule (C1 up; routes may be spent). Pairs with a mouth on the
/// cheapest route are proposed first — trunk mouths are the express
/// carriers.
fn try_pipe_move(
    state: &mut WorldState,
    rng: &mut dyn RngCore,
    before: &WorldMeasure,
) -> Result<String, String> {
    let saved_grid = state.grid.clone();
    let saved_slots = state.slots.clone();
    let saved_locks = state.locks.clone();
    let saved_pairs = state.pipe_pairs.clone();

    let trunk: HashSet<Pos> = before
        .rc
        .routes
        .first()
        .map(|r| r.path.iter().copied().collect())
        .unwrap_or_default();

    let mut pair_indices: Vec<usize> = (0..state.pipe_pairs.len()).collect();
    pair_indices.shuffle(rng);
    pair_indices.sort_by_key(|&i| {
        let (a, b) = state.pipe_pairs[i];
        !(trunk.contains(&a) || trunk.contains(&b))
    });

    let mut evals = 0usize;
    for &pi in &pair_indices {
        if evals >= PIPEMOVE_TRIES {
            break;
        }
        let (a, b) = saved_pairs[pi];
        // Move the trunk-side mouth; coin-flip when both or neither qualify.
        let move_a = match (trunk.contains(&a), trunk.contains(&b)) {
            (true, false) => true,
            (false, true) => false,
            _ => rng.next_u32() & 1 == 0,
        };
        let (old, keep) = if move_a { (a, b) } else { (b, a) };

        for _ in 0..2 {
            if evals >= PIPEMOVE_TRIES {
                break;
            }
            let candidates: Vec<Pos> = state
                .legal_blanks()
                .into_iter()
                .filter(|&p| !state.near_anchor(p))
                .collect();
            let Some(&new_pos) = candidates.choose(rng) else { break };

            let blank = blank_tile_for(&state.grid, state.world_idx, old.0, old.1);
            state.grid.set(old.0, old.1, blank);
            state.grid.set(new_pos.0, new_pos.1, TILE_PIPE);
            if let Some(slot) = state
                .slots
                .iter_mut()
                .find(|s| s.kind == SlotKind::Pipe && s.pos == old)
            {
                slot.pos = new_pos;
            }
            state.pipe_pairs[pi] = if move_a { (new_pos, keep) } else { (keep, new_pos) };
            evals += 1;

            if all_content_reachable(state) && place_locks_gating(state, rng, true) {
                let after = measure_world(state);
                if after.c1 > before.c1 {
                    return Ok(format!(
                        "pipe_move ACCEPT: mouth {old:?} -> {new_pos:?} (pair with {keep:?}), routes {} -> {}, C1 {} -> {}",
                        before.routes_in_band, after.routes_in_band, before.c1, after.c1,
                    ));
                }
            }
            state.grid = saved_grid.clone();
            state.slots = saved_slots.clone();
            state.locks = saved_locks.clone();
            state.pipe_pairs = saved_pairs.clone();
        }
    }
    Err(format!(
        "pipe_move REJECT: no relocation lifts C1 {} ({evals} full evals)",
        before.c1,
    ))
}

/// Every slot and the goal walk-reachable from start on the all-open world
/// — the global connectivity guard for moves that rewire the pipe web.
fn all_content_reachable(state: &WorldState) -> bool {
    let mut g = state.grid.clone();
    stamp_slots(&mut g, &state.slots);
    let reach = walk_reachable(&g, &state.pipe_pairs, state.start, state.world_idx);
    state.slots.iter().all(|s| reach.contains(s.pos))
        && state.target.is_none_or(|t| reach.contains(t))
}
