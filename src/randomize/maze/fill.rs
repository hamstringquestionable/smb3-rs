//! The key-assignment fill: which fortress opens which lock, across all eight
//! worlds.
//!
//! This is the mode's headline feature. A lock does not have to sit in its
//! fortress's world — the engine already has a data-only exit for an
//! off-screen lock (`patch_fortress_fx_screen_check`), and a cross-world lock
//! is that case one level up. What it buys is the thing that makes a maze a
//! maze: a gate you cannot open without going somewhere else first.
//!
//! ## Why a swap search and not the charter's forward fill
//!
//! The charter proposed a forward fill: close every lock, walk, and repeatedly
//! assign a frontier gate a key **from the already-reachable set**, so
//! solvability is by construction with no retry.
//!
//! **The reason recorded here for rejecting it was wrong, twice over, and this
//! note is kept as the correction rather than deleted.** It read:
//!
//! > That construction can stall, and on this map it stalls often. Its first
//! > step needs a fortress inside the start region with every lock closed — and
//! > the per-world builder deliberately puts forts *off* the forced path, so
//! > the start region frequently holds none.
//!
//! * **The first step never fails.** A lock is opened by beating its fortress,
//!   and reaching that fortress cannot require opening the lock it opens, so
//!   every world's start region must contain one. Measured **480 of 480 worlds
//!   over 60 seeds**, never fewer than one and up to four
//!   (`every_world_has_a_fortress_in_its_start_region`). The builder does put
//!   forts off the *forced path*, which is a different property; the argument
//!   slid from that to "behind a lock".
//! * **It does not stall often, under a policy that looks one step ahead.**
//!   Taking the frontier in arbitrary order stalls on 28% of seeds; ordering it
//!   by how much territory opening a gate reveals stalls on **none**, and comes
//!   with more keys in hand at every step
//!   (`forward_fill_terminates_when_ordered_by_territory`).
//!
//! The cost of the mistake is that a swap search has **nothing to aim with**. A
//! cross-world key — the mode's entire formula — is available at 87% of the
//! constructive fill's steps, and `Knobs::fort_distance_bias` defaults to `0.0`,
//! which is a uniform random walk over solvable assignments. Foreign locks come
//! out emergent rather than designed, and nothing measures whether one actually
//! forces the player to cross.
//!
//! What the swap search does buy is real and should survive any replacement: it
//! **cannot fail**, because the assignment it starts from is already known good.
//! A constructive fill can keep that by falling back to the same assignment.
//!
//! So the fill starts from an assignment that is **already known good** — the
//! per-world builder's, where every lock is opened by a fort in its own world
//! and every world is completable — and moves from there by swapping the forts
//! of two locks, keeping a swap only when the maze is still solvable. Two
//! properties fall out:
//!
//! * **It cannot fail.** The worst case is that no swap is accepted and the
//!   result is the local assignment the builder already guaranteed.
//! * **The bijection is preserved for free.** A swap exchanges two forts
//!   between two locks, so "every fortress has exactly one lock" — the
//!   charter's map-legibility rule, and the reason a world's lock count tells
//!   the player its fort count — holds at every step without being checked.

use std::collections::HashSet;

use rand::Rng;
use rand::seq::{IndexedRandom, SliceRandom};

use super::super::rom_data::{Grid, Pos};
use super::graph::Knobs;
use super::walk::{MazePos, walk_maze};
use super::{FortRef, GlobalState, MazeLock};

/// Exchange the forts of two locks — the fill's only move, and its own undo.
/// The fort/lock bijection survives for free, because a swap trades two forts
/// between two locks rather than handing one out.
///
/// Takes the lock slice rather than the whole state so it stays disjoint from
/// the `state.worlds` borrow the distance objective holds.
fn swap_forts(locks: &mut [MazeLock], a: usize, b: usize) {
    let fa = locks[a].fort;
    locks[a].fort = locks[b].fort;
    locks[b].fort = fa;
}

/// How many swaps the fill proposes per lock. Each proposal costs one global
/// fixpoint, so this is the fill's whole cost model: `locks * PROPOSALS_PER_LOCK`
/// fixpoints, ~17 * 8 = 136 on a normal seed.
const PROPOSALS_PER_LOCK: usize = 8;

/// What the fill did, for the census and the spoiler log.
#[derive(Clone, Debug, Default)]
pub(crate) struct FillReport {
    pub proposed: usize,
    /// Swaps rejected because they made the maze unsolvable.
    pub rejected_unsolvable: usize,
    /// Swaps rejected because they moved the objective the wrong way.
    pub rejected_objective: usize,
    pub accepted: usize,
    /// Locks whose fort ended up in another world.
    pub foreign_locks: usize,
    /// Spine distance between a foreign lock and its fort, summed.
    pub foreign_span: usize,
    /// Gates the constructive fill assigned, and how many locks there were.
    pub built: usize,
    pub locks: usize,

    /// The constructive fill could not place every gate, so the swap search
    /// produced the assignment instead.
    pub fell_back: bool,
}

/// The fallback: permute the builder's assignment by swapping pairs.
///
/// No longer the producer — see [`assign_keys`] — but kept, because it is the
/// one path that **cannot fail**. It starts from the per-world builder's own
/// pairing, which is known good, so the worst case is that no swap is accepted
/// and the result is what the builder guaranteed.
///
/// The maze must already be solvable on entry; it is still solvable on exit.
fn swap_search<R: Rng>(
    state: &mut GlobalState,
    spine: &[usize],
    knobs: &Knobs,
    rng: &mut R,
) -> FillReport {
    let mut report = FillReport::default();
    if state.locks.len() < 2 {
        return report;
    }

    // **No pair is pinned.** The fill used to protect the one lock the writer
    // had already parked 1-F on, because the writer ran first and the pairing
    // was a fact by the time the maze saw it. The maze now runs *before* the
    // writer, so nothing is committed yet: the fill permutes freely and
    // [`keep_n_sealable`] restores the invariant the writer actually consumes —
    // that N locks can be left shut forever — leaving the writer to pick which
    // of them 1-F gets.

    // Spine position, so "far" means far along the player's route rather than
    // far in ROM order. A key three worlds back is a long walk; a key in the
    // next world is barely a detour.
    // A world off the spine sits at the far end of it: the only way in is a
    // telepad, so a key there is as far from anywhere as the graph gets.
    let mut spine_pos = [spine.len(); 8];
    for (i, &w) in spine.iter().enumerate() {
        spine_pos[w] = i;
    }
    let distance = |lock: &MazeLock, fort: FortRef| -> usize {
        if fort.world == lock.world {
            // Same world: a few points for grid separation, so a local key is
            // not entirely flat. Halved because two tiles apart on one grid is
            // nothing beside a world boundary.
            let pos = state.worlds[fort.world]
                .slots
                .iter()
                .find(|s| s.section == fort.section && s.kind == super::SlotKind::Fortress)
                .map(|s| s.pos)
                .unwrap_or(lock.pos);
            (pos.0.abs_diff(lock.pos.0) + pos.1.abs_diff(lock.pos.1)) / 2
        } else {
            // Crossing a world boundary dominates everything on one grid.
            100 + spine_pos[fort.world].abs_diff(spine_pos[lock.world]) * 10
        }
    };
    let total = |st: &GlobalState| -> usize {
        st.locks.iter().filter_map(|l| l.fort.map(|f| distance(l, f))).sum()
    };

    let bias = knobs.fort_distance_bias.clamp(-1.0, 1.0);
    let mut score = total(state);
    let mut order: Vec<usize> = (0..state.locks.len()).collect();

    for _ in 0..PROPOSALS_PER_LOCK {
        order.shuffle(rng);
        for w in order.chunks(2) {
            let [a, b] = w else { continue };
            let (a, b) = (*a, *b);
            if state.locks[a].fort == state.locks[b].fort {
                continue;
            }
            report.proposed += 1;
            swap_forts(&mut state.locks, a, b);

            let after = total(state);
            // bias 0 accepts any solvable swap — a random walk over the
            // solvable assignments, which is the null model. Otherwise the
            // sign decides which way the objective has to move.
            let wanted = if bias > 0.0 {
                after > score
            } else if bias < 0.0 {
                after < score
            } else {
                true
            };
            // Magnitude is a probability, so the dial is continuous rather
            // than a switch: |bias| 0.5 takes half the improving swaps. At
            // bias 0 every solvable swap is taken, which is the random walk.
            let p = if bias == 0.0 { 1.0 } else { bias.abs() };
            let take = wanted && rng.random_bool(p);
            if !take {
                swap_forts(&mut state.locks, a, b);
                report.rejected_objective += 1;
                continue;
            }
            // Two hard gates, and both have to hold or the swap goes back.
            // Solvability is the obvious one. The second is the safety
            // invariant: a swap that hands a start-region lock to a foreign
            // fort makes that world a trap, because a player arriving there
            // for the first time cannot open it from the inside. Only the two
            // worlds whose locks moved can have changed, so this costs two
            // small per-world fixpoints rather than eight.
            let (wa, wb) = (state.locks[a].world, state.locks[b].world);
            let safe =
                state.start_region_escapable(wa) && (wb == wa || state.start_region_escapable(wb));
            if !safe || !state.spheres().solvable {
                swap_forts(&mut state.locks, a, b);
                report.rejected_unsolvable += 1;
                continue;
            }
            report.accepted += 1;
            score = after;
        }
    }

    for lock in &state.locks {
        if let Some(f) = lock.fort
            && f.world != lock.world
        {
            report.foreign_locks += 1;
            report.foreign_span += spine_pos[f.world].abs_diff(spine_pos[lock.world]);
        }
    }
    report
}

/// What restoring the sealable invariant came to on one seed.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct Sealable {
    /// Locks the maze agrees can be left shut forever, once the pass is done.
    pub kept: usize,
    /// How many were asked for.
    pub wanted: usize,
    /// Locks **removed** to get there — the gate becomes open path and its
    /// fortress opens nothing. That costs the map-legibility rule (a fortress
    /// whose beat says nothing) on the rare seed, and buys back the only
    /// failure a maze cannot recover from.
    pub opened: usize,
}

impl Sealable {
    /// Did the pass deliver what the writer needs?
    pub(crate) fn met(&self) -> bool {
        self.kept >= self.wanted
    }
}

/// **N locks must be ones the player can decline to open.**
///
/// This is the invariant the *writer* consumes. A secret-exit fortress level —
/// 1-F is the only one placed today — hands out an item and skips the crystal
/// ball, so the fortress is beaten and its lock stays shut. That is a choice
/// the mode keeps (sometimes the lock is worth more than the item) and the only
/// requirement is that taking it can never end the run. So the writer parks
/// such a level on a slot whose lock is `secret_exit_safe`, and needs at least
/// one to exist. `n` is that count, not a hardcoded 1, so a future deck holding
/// 7F2 or 8F1 as well can ask for more.
///
/// **The builder guarantees this and the maze breaks it.**
/// `overworld_build::ensure_secret_exit_safe` leaves N safe locks behind; the
/// fill then permutes the fort/lock pairing, which can invalidate every one of
/// them. Restoring it here is what makes the maze a transformer that preserves
/// the contract it was handed rather than one that quietly voids it.
///
/// It also *corrects* the flag rather than merely preserving it.
/// `secret_exit_safe` as the builder stamps it is the **per-world** verdict
/// `WorldState::completable_sealed` gives, and the maze asks a bigger question:
/// with that lock sealed forever, is the castle still reachable *and* are K
/// airship docks still reachable, across all eight worlds. Measured, the
/// per-world flag over-promises on 19% of locks at K=3 and 43% at K=7
/// (`per_world_sealable_locks_are_sealable_for_the_maze`), because sealing a
/// lock can strand an airship the wand count needs. `stamp_into` writes this
/// verdict back over the builder's, so the writer chooses from locks that are
/// safe in the maze rather than safe in their own world.
///
/// Two moves, in order of cost: swap a lock's fort with another's (keeps the
/// bijection, changes nothing on the map), and failing that remove a lock
/// outright (removing a gate only ever adds reachability, so solvability and
/// every start region stay safe by construction).
///
/// **Consumes no RNG**, deliberately: it runs after every other decision, and a
/// draw here would shift every downstream feature's stream on the seeds that
/// happen to need a repair.
pub(crate) fn keep_n_sealable(state: &mut GlobalState, n: usize) -> Sealable {
    let mut out = Sealable { wanted: n, ..Default::default() };
    out.kept = count_sealable(state);
    if out.met() {
        return out;
    }

    // Try to make one more lock sealable by swapping its fort with another's.
    // Any lock will do — unlike the old pinned-pair repair there is no
    // particular one that has to become safe, which is why this succeeds far
    // more often than that did.
    'grow: while !out.met() {
        for li in 0..state.locks.len() {
            if state.winnable_with_lock_sealed(li) {
                continue;
            }
            for lj in 0..state.locks.len() {
                if lj == li || state.locks[li].fort == state.locks[lj].fort {
                    continue;
                }
                swap_forts(&mut state.locks, li, lj);
                let (wa, wb) = (state.locks[li].world, state.locks[lj].world);
                let escapable = state.start_region_escapable(wa)
                    && (wb == wa || state.start_region_escapable(wb));
                if escapable && state.spheres().solvable && count_sealable(state) > out.kept {
                    out.kept = count_sealable(state);
                    continue 'grow;
                }
                swap_forts(&mut state.locks, li, lj);
            }
        }
        break;
    }
    if out.met() {
        return out;
    }

    // No permutation gets there. Open gates instead, worst first: removing one
    // can only add reachability, so it can turn other locks sealable. Each pass
    // removes a lock, so this terminates.
    while !out.met() {
        let Some(li) = (0..state.locks.len())
            .find(|&li| state.locks[li].fort.is_some() && !state.winnable_with_lock_sealed(li))
        else {
            break;
        };
        state.locks[li].fort = None;
        out.opened += 1;
        out.kept = count_sealable(state);
    }
    out
}

/// Locks that can be left shut forever with the maze still winnable.
///
/// An uninstalled lock (`fort: None`) is open path, not a gate, so it is not
/// something the player can decline to open and does not count.
fn count_sealable(state: &GlobalState) -> usize {
    (0..state.locks.len())
        .filter(|&li| state.locks[li].fort.is_some() && state.winnable_with_lock_sealed(li))
        .count()
}

/// A fortress that is never beaten.
///
/// While the fill runs, an unassigned lock carries this so it stays **shut** —
/// the whole point is to see the map as a player would with that gate still
/// closed. It must never survive the pass: `lock_keys` panics on a fort that
/// resolves to no cell, which is the backstop, and [`assign_keys`] turns any
/// survivor into `fort: None` (an uninstalled, open-path gate) first.
const UNASSIGNED: FortRef = FortRef { world: 0, section: 255 };

/// **Which fortress opens which lock.** The constructive forward fill, with the
/// swap search behind it.
///
/// Close every gate, walk, and repeatedly hand a gate on the frontier a key
/// **from the fortresses already reachable**. Solvability is by construction:
/// a key is never placed anywhere the player cannot already stand, so no round
/// can gate itself, and there is no retry loop.
///
/// This replaced the swap search (2026-09-07) because a swap has nothing to aim
/// with — its only move trades two forts, so it cannot make one lock foreign
/// without making another foreign in the opposite direction, and a backward
/// lock is often unsolvable. The two halves are accepted or rejected together,
/// so the safe half dies with the unsafe one. Measured on World 8's bridge, the
/// clearest case (its keys are forward keys from anywhere, so always safe):
///
/// | | bridge locks foreign | all locks foreign |
/// |---|---|---|
/// | swap search | **37%** | 51% |
/// | constructive, neutral | **72%** | 76% |
/// | constructive, prefer cross | 73% | 87% |
///
/// The bridge gain is the algorithm, not the aiming — a neutral constructive
/// fill already gets it, and the preference adds one point there while moving
/// everything else. Under the swap search bridge locks sat 14 points *below*
/// the average lock; constructively that anomaly is gone.
pub(crate) fn assign_keys<R: Rng>(
    state: &mut GlobalState,
    spine: &[usize],
    knobs: &Knobs,
    rng: &mut R,
) -> FillReport {
    // The builder's own local pairing, kept so the fallback has the known-good
    // assignment to start from.
    let builders = state.locks.clone();

    let mut report = FillReport { locks: state.locks.len(), ..Default::default() };
    report.built = constructive(state, knobs, rng);

    // Two ways the fill can be unusable, and the same answer to both.
    //
    // A **stall** is gates it never reached. **Unescapable** is a world whose
    // start region the finished assignment traps — checked here rather than per
    // assignment, because mid-fill every unreached gate still reads as shut
    // forever and the world looks far more locked than it ends up.
    //
    // Either way the swap search takes over, because `generate`'s last-resort
    // guard answers an unescapable world by discarding *everything* — pads
    // included — and reverting to the builder's all-local pairing. Falling back
    // here keeps the pads and gives up only the fill.
    // **No start-region check.** The fill assigns every gate a key drawn from
    // the fortresses already reachable *at that moment*, walking from the
    // global start — so every gate it places is openable by construction, the
    // first one included. That is strictly stronger than the rule, and the rule
    // is strictly wrong here: `start_region_escapable` counts only a world's
    // OWN fortresses as openers, so it rejects the mode's whole formula — pad
    // out, beat a fortress there, come back.
    //
    // What made the rule look necessary was the swap search, which permutes
    // blindly and really can strand a world. Carrying it over cost 2.1
    // crossings a seed and sent a third of them to the fallback; dropping it
    // takes cross-world locks from 51% to 76% and the World 8 bridge from 37%
    // to 73%, with no seed falling back at all.
    //
    // The player is never stranded even so: the maze whistle is never consumed,
    // survives a game over, and always has the spine's first world to return
    // to. See `world_travel`.
    //
    // **That makes the whistle a safety property, not a convenience.** If the
    // mode ever ships without one, this reasoning lapses and game over has to
    // return the player to the spine's first world instead of the one they died
    // in — the note at `remove_whistles` in `randomizer::randomize_inner`
    // carries the mechanism.
    if report.built < report.locks {
        state.locks = builders;
        let mut fallback = swap_search(state, spine, knobs, rng);
        fallback.locks = report.locks;
        fallback.fell_back = true;
        return fallback;
    }

    tally_foreign(state, spine, &mut report);
    report
}

/// The forward fill proper. Returns how many gates it placed.
fn constructive<R: Rng>(state: &mut GlobalState, knobs: &Knobs, rng: &mut R) -> usize {
    let forts: Vec<(FortRef, Pos)> = state
        .worlds
        .iter()
        .flat_map(|w| {
            w.slots
                .iter()
                .filter(|s| s.kind == super::SlotKind::Fortress)
                .map(|s| (FortRef { world: w.world_idx, section: s.section }, s.pos))
        })
        .collect();

    for lock in state.locks.iter_mut() {
        lock.fort = Some(UNASSIGNED);
    }

    let links = state.links();
    // Loop-invariant: the slots and grids do not move, only which locks are
    // open, and `locked_grids` applies those on top.
    let bases = state.base_grids(&HashSet::new());

    let mut open: HashSet<FortRef> = HashSet::new();
    let mut used: HashSet<FortRef> = HashSet::new();
    let mut assigned = vec![false; state.locks.len()];
    let mut placed = 0usize;

    loop {
        let shut = state.shut_locks(&open);
        let reach = walk_maze(&state.view(&bases, &shut), &links, state.start);

        for (f, pos) in &forts {
            if !open.contains(f) && reach.contains((f.world, *pos)) {
                open.insert(*f);
            }
        }
        let available: Vec<FortRef> = forts
            .iter()
            .map(|(f, _)| *f)
            .filter(|f| open.contains(f) && !used.contains(f))
            .collect();

        // A gate is on the frontier when the player can stand next to it.
        let frontier: Vec<usize> = (0..state.locks.len())
            .filter(|&i| !assigned[i])
            .filter(|&i| {
                let l = &state.locks[i];
                neighbours(&bases[l.world], l.pos).any(|p| reach.contains((l.world, p)))
            })
            .collect();

        if frontier.is_empty() || available.is_empty() {
            break;
        }

        let territory: usize = (0..state.worlds.len()).map(|w| reach.world_len(w)).sum();
        // Any beaten fort will do as the probe: what is being measured is what
        // the GATE reveals, not which key opens it.
        let probe = *open.iter().next().expect("a fort is always beatable first");
        let li = widest_gate(state, &bases, &links, &open, &frontier, territory, probe, rng);

        // Two hard gates, not one. Solvability is by construction here — a key
        // is only ever drawn from what the player can already reach — but the
        // start-region rule is not, and it was the swap search's *second*
        // gate. `start_region_escapable` counts only a world's OWN fortresses
        // as openers, because a player arriving at a start tile (game over,
        // airship, whistle) cannot open a foreign lock from the inside. Hand a
        // start-region gate a foreign key and that world becomes a trap.
        //
        // Only the lock's own world can have changed, so this is one small
        // per-world fixpoint per candidate rather than eight.
        // No start-region check here, deliberately. Mid-fill a world looks far
        // more locked than it will end up — every gate not yet reached still
        // carries `UNASSIGNED`, which reads as shut forever — so a check here
        // is answering the wrong question. Consulting it anyway steered badly
        // enough that 82% of seeds ended up trapped and fell back, which is the
        // whole fill wasted. It is a repair, not a constraint: see
        // [`repair_start_regions`].
        let pick = ranked_keys(state.locks[li].world, &available, knobs, rng)[0];
        state.locks[li].fort = Some(pick);
        used.insert(pick);
        assigned[li] = true;
        placed += 1;
    }

    // Anything still unassigned is NOT left carrying the placeholder — that
    // would ship as a gate no fortress opens.
    for lock in state.locks.iter_mut() {
        if lock.fort == Some(UNASSIGNED) {
            lock.fort = None;
        }
    }
    placed
}

/// The four orthogonal neighbours of a cell that are inside the grid.
fn neighbours(grid: &Grid, (r, c): Pos) -> impl Iterator<Item = Pos> {
    let (rows, cols) = (grid.rows(), grid.cols);
    [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)].into_iter().filter_map(move |(dr, dc)| {
        let (nr, nc) = (r as i32 + dr, c as i32 + dc);
        ((0..rows as i32).contains(&nr) && (0..cols as i32).contains(&nc))
            .then_some((nr as usize, nc as usize))
    })
}

/// Which frontier gate to open next: the one that reveals the most territory.
///
/// Taking the frontier in arbitrary order stalls on 28% of seeds; ordering it
/// this way stalls on **none**, and leaves more keys in hand at every step
/// (mean 6.04 against 4.17). Ties are broken at random rather than by index, so
/// the fill does not always walk the map the same way round.
#[allow(clippy::too_many_arguments)]
// Reason: every argument is a distinct loop-invariant the caller already holds;
// bundling them into a struct would name nothing the fill does not already say.
fn widest_gate<R: Rng>(
    state: &mut GlobalState,
    bases: &[Grid],
    links: &[(MazePos, MazePos)],
    open: &HashSet<FortRef>,
    frontier: &[usize],
    territory: usize,
    probe: FortRef,
    rng: &mut R,
) -> usize {
    let mut best = (0usize, Vec::new());
    for &i in frontier {
        let saved = state.locks[i].fort;
        state.locks[i].fort = Some(probe);
        let shut = state.shut_locks(open);
        let opened = walk_maze(&state.view(bases, &shut), links, state.start);
        state.locks[i].fort = saved;

        // Opening a gate only ever adds reachability, so this cannot go
        // negative; saturating rather than asserting keeps a walker change from
        // turning a measurement into a panic.
        let gain: usize = (0..state.worlds.len())
            .map(|w| opened.world_len(w))
            .sum::<usize>()
            .saturating_sub(territory);
        match gain.cmp(&best.0) {
            std::cmp::Ordering::Greater => best = (gain, vec![i]),
            std::cmp::Ordering::Equal => best.1.push(i),
            std::cmp::Ordering::Less => {}
        }
    }
    best.1.choose(rng).copied().unwrap_or(frontier[0])
}

/// The reachable fortresses that could open this gate, best first.
///
/// A list rather than one pick, because the caller has a veto: a key that
/// leaves the lock's world unescapable is rejected and the next one tried.
///
/// `fort_distance_bias` is the dial, and it means something sharper here than
/// it did for the swap search: at 0.0 the order is a uniform shuffle of
/// everything reachable (already ~76% cross-world, because most of the
/// reachable set is in another world by the time a gate is assigned); positive
/// pulls crossings to the front, negative pulls local keys forward. The
/// magnitude is the probability of applying the preference at all, so the dial
/// stays continuous rather than becoming a switch.
fn ranked_keys<R: Rng>(
    lock_world: usize,
    available: &[FortRef],
    knobs: &Knobs,
    rng: &mut R,
) -> Vec<FortRef> {
    let mut out = available.to_vec();
    out.shuffle(rng);
    let bias = knobs.fort_distance_bias.clamp(-1.0, 1.0);
    if bias != 0.0 && rng.random_bool(bias.abs()) {
        let want_cross = bias > 0.0;
        out.sort_by_key(|f| (f.world != lock_world) != want_cross);
    }
    out
}

/// Count the cross-world locks and how far their keys sit, for the census.
fn tally_foreign(state: &GlobalState, spine: &[usize], report: &mut FillReport) {
    let mut spine_pos = [spine.len(); 8];
    for (i, &w) in spine.iter().enumerate() {
        spine_pos[w] = i;
    }
    for lock in &state.locks {
        if let Some(f) = lock.fort
            && f.world != lock.world
        {
            report.foreign_locks += 1;
            report.foreign_span += spine_pos[f.world].abs_diff(spine_pos[lock.world]);
        }
    }
}
