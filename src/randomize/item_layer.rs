//! Choosing which levels are gated, and where their keys go.
//!
//! The generator half of `docs/mimaze_layer_design.md`. It runs on a finished
//! maze — after `maze::generate`, before `overworld_writer` — and decides two
//! things: which slots get a level that cannot be beaten without an item, and
//! which Hammer Bro hands that item over.
//!
//! # Generate and test
//!
//! The same shape the overworld builder uses for shaping, and for the same
//! reason: the property wanted is emergent, and a deal is cheap to retry.
//! Deal a gate, deal its keys, run the item-aware fixpoint, keep it if the
//! maze is still winnable and the gate actually opens. The fixpoint is the
//! whole acceptance test — put a key behind the gate it opens and the rounds
//! stop growing, so acyclicity needs no separate check.
//!
//! # Why Hammer Bros carry the keys
//!
//! They are the only source this pass can *choose*. A Hammer Bro's reward is
//! `BuiltWorld::hb_sprites[..].reward`, a model field the writer stamps — so
//! the pass can write a leaf into one and know the player will find a leaf
//! there. The other two sources cannot be aimed: a Toad House's item comes
//! from its pointer entry, and which entry lands on which slot is decided by
//! `overworld_writer::assign_pool`, downstream of here; a Princess letter is
//! per-world rather than per-cell.
//!
//! **So this pass requires `shuffle_hammer_bros`.** With it off the builder
//! populates no `hb_sprites` at all, the pass has nothing it can aim, and it
//! deals nothing — which is how its first census read zero gates on every
//! seed before anyone noticed the harness had that flag off. A real run
//! defaults it on; a caller that turns it off gets no gates rather than a
//! broken seed, which is the right failure but worth knowing about.
//!
//! That is also why the anchor is not placed yet. It exists in exactly one
//! place in the ROM — a Toad House type reached by an out-of-bounds read —
//! and this pass cannot aim a Toad House. Writing `$0A` into a Hammer Bro
//! reward would work (the recorder takes it), but whether the canoe *should*
//! be a dealt gate rather than the one the map already has is a design
//! question the censuses have not answered.

use rand::Rng;
use rand::seq::SliceRandom;

use super::item_keys::{Key, LEVEL_REQUIREMENTS};
use super::maze::walk::MazePos;
use super::maze::{GlobalState, ItemGate, ItemSource};
use super::overworld_build::{BuildResult, SlotKind};

/// How many candidate cuts to try per requirement before giving up on it.
/// Each try costs a fixpoint, and the censuses put ~19 gateable positions on
/// a seed, so this is "most of them" rather than a real cap.
const TRIES_PER_REQUIREMENT: usize = 24;

/// How many Hammer Bros to offer the anchor before declaring the seed
/// unable to carry a gated canoe. Each try is one fixpoint.
const ANCHOR_TRIES: usize = 12;

/// What a deal achieved, for the write log and the census.
#[derive(Default, Debug, Clone)]
pub(crate) struct Placement {
    /// **Whether the mode may be installed on this seed at all.**
    ///
    /// False means the pass could not make the maze winnable under its own
    /// rules and wrote nothing — the caller must then skip `item_keys` too,
    /// because the ROM patches gate the dispenser and the canoe whatever the
    /// model decided. Gating without a model that agrees is how a seed
    /// strands a player.
    pub installed: bool,
    /// Requirements that found a home.
    pub placed: Vec<PlacedGate>,
    /// Requirement rows no candidate could carry.
    pub unplaced: usize,
    /// Candidate cuts the pass considered.
    pub candidates: usize,
}

// Reason: the shipping path reads only `Placement::installed` — the veto.
// Every field here is detail for `item_layer_census`, which is what measures
// whether the pass is dealing gates worth having.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct PlacedGate {
    pub pos: MazePos,
    pub requirement: usize,
    /// How much content the gate seals while shut.
    pub cut: usize,
}

/// A slot whose gating puts content out of reach, with the size of the cut.
struct Candidate {
    pos: MazePos,
    is_fortress: bool,
    cut: usize,
}

/// The fortress slots this pass leaves alone for 1-F, as `(world, section)`.
///
/// 1-F's secret exit skips the Boom-Boom whose defeat opens a lock, so it may
/// only sit on a fortress whose lock can stay shut without stranding the
/// world's target — `secret_exit_safe`. The writer pre-assigns it there
/// **first**, before any requirement mark, so a mark on the slot it draws
/// simply loses: all 26 fortress marks in a 200-seed census, before either
/// side knew about the other.
///
/// Both sides now do. The writer draws 1-F from the *unmarked* safe slots,
/// and this reserves [`SECRET_EXIT_SLOTS_NEEDED`] of them so that list is
/// never empty — the same contract `maze::fill::keep_n_sealable` and
/// `overworld_build::ensure_secret_exit_safe` already hold up from their
/// ends.
///
/// Reserving the *whole* safe set instead was measured and is far too dear: a
/// fortress row then finds no home on a third of seeds, and a row with no home
/// vetoes the mode outright (200/200 seeds install with N reserved, 135/200
/// with all of them).
///
/// Deterministic — sorted, then the first N. The writer's own draw is uniform
/// over what is left, so there is no bias to spread here.
fn reserved_secret_exit_slots(build: &BuildResult) -> std::collections::HashSet<(usize, usize)> {
    let mut safe: Vec<(usize, usize)> = build
        .worlds
        .iter()
        .flat_map(|w| w.locks.iter())
        .filter(|l| l.secret_exit_safe)
        .map(|l| (l.fort.world, l.fort.section))
        .collect();
    safe.sort_unstable();
    safe.dedup();
    safe.truncate(crate::randomize::overworld_build::SECRET_EXIT_SLOTS_NEEDED);
    safe.into_iter().collect()
}

/// Every slot worth gating. Fortress slots held back for 1-F are left out —
/// see [`reserved_secret_exit_slots`].
fn candidates(
    state: &GlobalState,
    reserved_forts: &std::collections::HashSet<(usize, usize)>,
) -> Vec<Candidate> {
    let full = state.spheres();
    let reached: std::collections::HashSet<MazePos> =
        full.spheres.iter().flat_map(|s| s.reached.iter().copied()).collect();

    let mut out = Vec::new();
    for w in state.worlds.iter().filter(|w| state.in_maze[w.world_idx]) {
        for slot in &w.slots {
            let is_fortress = match slot.kind {
                SlotKind::Level => false,
                SlotKind::Fortress => true,
                _ => continue,
            };
            if is_fortress && reserved_forts.contains(&(w.world_idx, slot.section)) {
                continue;
            }
            let pos: MazePos = (w.world_idx, slot.pos);
            let blocked: std::collections::HashSet<MazePos> = std::iter::once(pos).collect();
            let sp = state.spheres_with_blocked(&blocked);
            let still: std::collections::HashSet<MazePos> =
                sp.spheres.iter().flat_map(|s| s.reached.iter().copied()).collect();
            // The gated cell stops being reachable content itself; what
            // matters is what it takes with it.
            let cut = reached.difference(&still).count().saturating_sub(1);
            out.push(Candidate { pos, is_fortress, cut });
        }
    }
    out
}

/// Every Hammer Bro that could carry a key, as `(world, cell, index)` into
/// its world's sprite list.
fn carriers(build: &BuildResult, state: &GlobalState) -> Vec<(usize, MazePos, usize)> {
    build
        .worlds
        .iter()
        .filter(|b| state.in_maze[b.world_idx])
        .flat_map(|b| {
            b.hb_sprites
                .iter()
                .enumerate()
                .map(move |(i, s)| (b.world_idx, (b.world_idx, s.grid_pos), i))
        })
        .collect()
}

/// The Global Item ID a key is handed over as.
fn item_id(key: Key) -> u8 {
    match key {
        Key::Mushroom => 0x01,
        Key::Flower => 0x02,
        Key::Leaf => 0x03,
        Key::Star => 0x09,
        Key::Anchor => 0x0A,
    }
}

/// Deal gates and keys onto a finished maze.
///
/// Mutates `state` (the gates and sources the fixpoint reads) and `build`
/// (the `requires` marks the writer honours). Both are left untouched if the
/// maze was not winnable to begin with — this pass makes a seed harder, never
/// playable.
pub(crate) fn place<R: Rng>(
    state: &mut GlobalState,
    build: &mut BuildResult,
    rng: &mut R,
) -> Placement {
    let mut report = Placement::default();
    if !state.spheres().solvable {
        return report;
    }

    let mut cands = candidates(state, &reserved_secret_exit_slots(build));
    report.candidates = cands.iter().filter(|c| c.cut > 0).count();
    if cands.is_empty() {
        return report;
    }
    cands.shuffle(rng);
    // Cells that seal content first, cells that seal nothing after — a stable
    // sort, so each tier keeps the shuffle's order.
    //
    // **Every requirement level is a gate wherever the deal puts it.** The
    // dispenser patch keys off the level, not off the cell this pass marked,
    // so a row this pass declines to place does not become "one gate fewer" —
    // the writer deals that level onto some ordinary cell and it is a wall the
    // model never saw and put no key in front of. Measured at 86 such walls in
    // 200 seeds, both fortress rows, nine of them unwinnable.
    //
    // So a cut of zero is not a reason to skip a cell. It makes a duller gate
    // — a level you cannot beat yet, taking nothing else with it — but a dull
    // modelled gate beats an unmodelled one, and the fallback is what lets
    // every row find a home.
    cands.sort_by_key(|c| c.cut == 0);

    let mut free_carriers = carriers(build, state);
    free_carriers.shuffle(rng);
    let mut rejected: Vec<(usize, MazePos, usize)> = Vec::new();

    // --- The canoe, before anything else --------------------------------
    //
    // `item_keys::apply` gates the summon and parks the boats out of reach,
    // and it does that whatever the model says — so the model has to agree or
    // it will route a player across water the game will not let them cross.
    // That means `anchor_gated` is not optional here, and an anchor has to be
    // findable before the water is needed.
    //
    // The ROM already supplies one (a Toad House type reached by an
    // out-of-bounds read), but this pass cannot aim a Toad House, so it puts
    // one on a Hammer Bro as well — an extra source only ever makes the model
    // stricter than the game, which is the safe direction.
    //
    // Which Hammer Bro carries it matters: the anchor has to be reachable
    // *before* the water it unlocks, so a carrier on the far side of a
    // crossing is no use. Rather than reason about that, try carriers until
    // the fixpoint accepts one — the same generate-and-test the gates use.
    state.anchor_gated = true;
    let mut anchored = None;
    for _ in 0..ANCHOR_TRIES {
        let Some(c) = free_carriers.pop() else { break };
        state.sources.push(ItemSource { pos: c.1, item: Key::Anchor });
        if state.spheres().solvable {
            anchored = Some(c);
            break;
        }
        state.sources.pop();
        rejected.push(c);
    }
    free_carriers.append(&mut rejected);
    match anchored {
        Some(c) => build.worlds[c.0].hb_sprites[c.2].reward = item_id(Key::Anchor),
        None => {
            // No carrier makes this seed survive a gated canoe. Write nothing
            // and tell the caller not to install the ROM side either.
            state.anchor_gated = false;
            state.sources.clear();
            return report;
        }
    }
    report.installed = true;

    let mut rows: Vec<usize> = (0..LEVEL_REQUIREMENTS.len()).collect();
    rows.shuffle(rng);

    for row in rows {
        let req = &LEVEL_REQUIREMENTS[row];
        let mut taken_cand = None;

        // Filter to the right kind *before* the cap, not inside it. A fortress
        // row has a handful of eligible cells among dozens of level cells, so
        // capping the raw list first spent nearly every try on candidates it
        // was always going to skip.
        let eligible = cands
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_fortress == req.is_fortress)
            .take(TRIES_PER_REQUIREMENT);
        for (ci, cand) in eligible {
            // Tentative: the gate, plus one carrier per key it demands.
            let gates_before = state.gates.len();
            let sources_before = state.sources.len();
            let carriers_before = free_carriers.len();

            state.gates.push(ItemGate { pos: cand.pos, items: req.items.to_vec() });
            let mut used = Vec::new();
            for &key in req.items {
                let Some(c) = free_carriers.pop() else { break };
                state.sources.push(ItemSource { pos: c.1, item: key });
                used.push((c, key));
            }

            let ok = used.len() == req.items.len() && {
                let sp = state.spheres();
                sp.solvable && sp.unopened.is_empty()
            };

            if ok {
                for ((world, _, idx), key) in &used {
                    build.worlds[*world].hb_sprites[*idx].reward = item_id(*key);
                }
                report.placed.push(PlacedGate { pos: cand.pos, requirement: row, cut: cand.cut });
                taken_cand = Some(ci);
                break;
            }

            // Undo and try the next cut.
            state.gates.truncate(gates_before);
            state.sources.truncate(sources_before);
            for (c, _) in used.into_iter().rev() {
                free_carriers.push(c);
            }
            debug_assert_eq!(free_carriers.len(), carriers_before);
        }

        match taken_cand {
            Some(ci) => {
                let pos = cands.remove(ci).pos;
                // Mark the slot so the writer deals the level this row names.
                for b in build.worlds.iter_mut().filter(|b| b.world_idx == pos.0) {
                    for slot in b.slots.iter_mut().filter(|s| s.pos == pos.1) {
                        slot.requires = Some(row);
                    }
                }
            }
            None => report.unplaced += 1,
        }
    }

    // **A row with no home is a veto, not a shortfall.** See the note on the
    // fallback tier: the level it names is still dealt, and the ROM still
    // gates it, so shipping the mode here would put a wall on a cell nothing
    // keyed. Hand the seed back as a plain maze instead.
    if report.unplaced > 0 {
        state.anchor_gated = false;
        state.gates.clear();
        state.sources.clear();
        report.installed = false;
    }
    report
}
