//! Power-up gates in the maze model: which cells the gate levels go on, and
//! which levels hand out the keys.
//!
//! Runs on a finished maze, after the Anchor's keys and the obelisks are
//! placed, and decides cells only — the writer pins the gate levels onto them
//! and the on-clear table turns the key levels into unlocks.
//!
//! # Gates on required cells, with room in front
//!
//! A gate only *gates* if the player cannot go around it, so each one goes on a
//! **required** cell of its kind — a level or fortress the castle cannot be
//! reached without (walled off, the maze is unwinnable). With too few of those
//! it falls back to any reachable cell of the kind.
//!
//! Candidates are tried **deepest first** — by the round the solver first
//! reaches them, shuffled within a round — and the first required one with
//! **room** wins: enough levels still reachable with it gated to hold its
//! keys. A required level one step from the start has none, so a gate there
//! could never be keyed. Stopping at the first fit keeps the pass to a handful
//! of solver runs per gate rather than one per cell.
//!
//! # Keys at the stall
//!
//! The same discipline as [`key_placement`](super::key_placement): run the
//! solver with every gate shut, and when it stalls, the reach it came to rest
//! at is where the missing key may go — anything outside is behind the gate it
//! opens. So **a key cannot land behind its own gate**, at any number of gates,
//! without anything checking for it. Inside the reach a key prefers an
//! **optional** level (the maze is winnable without it), tried a few at a
//! time, then any level.
//!
//! "Required" and "optional" are always asked of the maze **without** this
//! pass's gates and keys: with a gate shut and its key not yet placed, every
//! cell would look required.
//!
//! A gate whose keys find no site is dropped, not the mode: it goes back to
//! being an ordinary level. Power-ups no gate needs still get a key level —
//! an optional one anywhere — so their blocks are not coins for the whole game.
//!
//! # What is never used
//!
//! Hand traps, troll pipes and the pyramid have tiles of their own; `avoid`
//! (the obelisks) is walled off throughout, so no key ever depends on beating
//! one and each stays optional. One cell holds one gate or one key.

use std::collections::{HashMap, HashSet};

use rand::Rng;
use rand::seq::SliceRandom;

use super::item_keys::Key;
use super::powerup_gate::{GATES, PowerUp, TILE_KEY_LEVEL};
use super::walk::MazePos;
use super::{Gate, GateTarget, GlobalState, KeySource};
use crate::randomize::overworld::build::{BuildResult, SlotKind};
use crate::randomize::overworld::node_catalog::NodeCatalog;
use crate::randomize::rom_data;
use crate::rom::Rom;

/// One gate to place: its level, what kind of slot it goes on, and the keys
/// it needs.
#[derive(Clone, Debug)]
pub(crate) struct GateRequest {
    /// The pointer-table entry `(world_idx, entry_idx)` the writer pins.
    entry: (usize, usize),
    fortress: bool,
    needs: Vec<PowerUp>,
}

/// The gate table ([`GATES`]) as this run's requests: each gate's catalog
/// entry and the keys it needs on this ROM (`rom` after the power-up reroll).
///
/// **A gate Friendlier Levels takes off the map is not requested.** A pinned
/// entry is dealt from the withheld list, past Friendlier's removal, so
/// requesting it would put the level back.
pub(crate) fn gate_requests(
    rom: &Rom,
    catalog: &NodeCatalog,
    modern: bool,
    friendlier: bool,
) -> Vec<GateRequest> {
    GATES
        .iter()
        .filter(|g| {
            let blocked = if g.fortress {
                rom_data::is_friendlier_blocked_fort(g.name)
            } else {
                rom_data::is_friendlier_blocked(g.name)
            };
            !(friendlier && blocked)
        })
        .filter_map(|g| {
            let ce = catalog.entries.iter().find(|e| e.name == g.name)?;
            Some(GateRequest {
                entry: (ce.world_idx, ce.entry_idx),
                fortress: g.fortress,
                needs: g.keys(modern, rom),
            })
        })
        .collect()
}

/// A key level: the power-up it unlocks, and its cell.
pub(crate) type KeyLevel = (PowerUp, MazePos);

/// What [`place`] decided.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Plan {
    /// `(request index, cell)` for every gate that went in.
    gates: Vec<(usize, MazePos)>,
    /// Each key level, one per power-up.
    pub(crate) keys: Vec<KeyLevel>,
    /// Requests that got no cell, or whose keys got no site. **Not gates**:
    /// the writer must leave these levels unpinned.
    dropped: Vec<usize>,
    /// Power-ups no level could be found for. Their blocks stay coins.
    unkeyed: Vec<PowerUp>,
}

/// The placement's own substream, so turning power-up gates on moves nothing
/// else in the seed.
pub(crate) const POWERUP_GATES_SALT: u64 = 0x5055_505F_4741_5445; // "PUP_GATE"

/// Belt and braces: every pass places a key or drops a gate.
const MAX_PASSES: usize = 64;

/// Place `requests` as gates and every power-up in `keyed` as a key level.
///
/// Leaves `state` with the gates installed and the keys as sources, so the
/// caller's solver reads the maze as it will ship.
pub(crate) fn place<R: Rng>(
    state: &mut GlobalState,
    requests: &[GateRequest],
    keyed: &[PowerUp],
    avoid: &[MazePos],
    rng: &mut R,
) -> Plan {
    let walled: HashSet<MazePos> = avoid.iter().copied().collect();
    let mut plan = Plan::default();
    let (base, base_reach) = state.spheres_inner(&walled, None);
    if !base.solvable {
        // Not this pass's to fix; place nothing rather than guess.
        plan.dropped = (0..requests.len()).collect();
        plan.unkeyed = keyed.to_vec();
        return plan;
    }

    let levels = eligible(state, SlotKind::Level, &walled, |p| base_reach.contains(p));
    let forts = eligible(state, SlotKind::Fortress, &walled, |p| base_reach.contains(p));
    // The round the base fixpoint first reaches each cell: gates are tried
    // deepest first, which is where the room for their keys is.
    let mut depth: HashMap<MazePos, usize> = HashMap::new();
    for (round, sphere) in base.spheres.iter().enumerate() {
        for &cell in &sphere.reached {
            depth.entry(cell).or_insert(round);
        }
    }
    let mut required = Required::new(walled.clone());

    // --- Gates ---
    // Deepest first, shuffled within a round; the first required cell with
    // room for its keys wins. Most gates settle in a handful of solver runs
    // instead of one per cell. Without a required cell, the first with room.
    let mut taken: HashSet<MazePos> = HashSet::new();
    for (ri, request) in requests.iter().enumerate() {
        let pool = if request.fortress { &forts } else { &levels };
        let mut order: Vec<MazePos> = pool.iter().copied().filter(|c| !taken.contains(c)).collect();
        order.shuffle(rng);
        order.sort_by_key(|c| std::cmp::Reverse(depth.get(c).copied().unwrap_or(0)));
        let fits = |state: &mut GlobalState, c: MazePos| {
            room(state, &walled, c, &levels, &taken) >= request.needs.len()
        };
        let mut chosen = None;
        for &c in &order {
            if required.is(state, c) && fits(state, c) {
                chosen = Some(c);
                break;
            }
        }
        if chosen.is_none() {
            chosen = order.iter().copied().find(|&c| fits(state, c));
        }
        let Some(cell) = chosen else {
            plan.dropped.push(ri);
            continue;
        };
        taken.insert(cell);
        plan.gates.push((ri, cell));
        state.gates.push(Gate {
            target: GateTarget::Cell(cell),
            needs: request.needs.iter().map(|&p| Key::PowerUp(p)).collect(),
        });
    }

    // --- Keys at the stall ---
    for _ in 0..MAX_PASSES {
        let (spheres, reach) = state.spheres_inner(&walled, None);
        if spheres.solvable {
            break;
        }
        // One of our gates, shut for want of a key. Anything else (a canoe)
        // is not this pass's to key.
        let stuck = plan.gates.iter().enumerate().find_map(|(gi, &(ri, _))| {
            let missing = requests[ri]
                .needs
                .iter()
                .copied()
                .find(|&p| !spheres.found.contains(&Key::PowerUp(p)))?;
            Some((gi, missing))
        });
        let Some((gi, key)) = stuck else { break };

        let in_reach: Vec<MazePos> =
            levels.iter().copied().filter(|&c| !taken.contains(&c) && reach.contains(c)).collect();
        match pick_site(state, &mut required, &in_reach, rng) {
            Some(cell) => {
                taken.insert(cell);
                state.sources.push(KeySource { pos: cell, key: Key::PowerUp(key) });
                plan.keys.push((key, cell));
            }
            None => {
                let (ri, cell) = plan.gates.remove(gi);
                state.gates.retain(|g| g.target != GateTarget::Cell(cell));
                taken.remove(&cell);
                plan.dropped.push(ri);
            }
        }
    }

    // --- Every power-up keyed somewhere ---
    // Gates the run never had to pass (a fallback cell off the castle route)
    // still need their keys, and so does every power-up no gate needs.
    let mut wanted: Vec<PowerUp> = keyed.to_vec();
    for &(ri, _) in &plan.gates {
        wanted.extend(requests[ri].needs.iter().copied());
    }
    wanted.sort();
    wanted.dedup();
    wanted.shuffle(rng);
    for p in wanted {
        if plan.keys.iter().any(|&(k, _)| k == p) {
            continue;
        }
        let (_, reach) = state.spheres_inner(&walled, None);
        let in_reach: Vec<MazePos> =
            levels.iter().copied().filter(|&c| !taken.contains(&c) && reach.contains(c)).collect();
        match pick_site(state, &mut required, &in_reach, rng) {
            Some(cell) => {
                taken.insert(cell);
                state.sources.push(KeySource { pos: cell, key: Key::PowerUp(p) });
                plan.keys.push((p, cell));
            }
            None => plan.unkeyed.push(p),
        }
    }
    // A gate whose key went unplaced is a wall nothing opens: drop it.
    let unkeyed = plan.unkeyed.clone();
    plan.gates.retain(|&(ri, cell)| {
        let keep = !requests[ri].needs.iter().any(|p| unkeyed.contains(p));
        if !keep {
            state.gates.retain(|g| g.target != GateTarget::Cell(cell));
            plan.dropped.push(ri);
        }
        keep
    });

    debug_assert!(
        state.spheres_with_blocked(&walled).solvable,
        "power-up placement left the maze unwinnable"
    );
    plan
}

/// Hand the plan to the writer: each gate's level pinned onto its cell, each
/// key level marked with [`TILE_KEY_LEVEL`], and **every** requested gate
/// level withheld from the decks — so a gate appears only where the model put
/// it, and a dropped one appears nowhere.
pub(crate) fn mark(build: &mut BuildResult, requests: &[GateRequest], plan: &Plan) {
    for &(ri, cell) in &plan.gates {
        let kind = if requests[ri].fortress { SlotKind::Fortress } else { SlotKind::Level };
        slot(build, cell, kind).pin = Some(requests[ri].entry);
    }
    for &(_, cell) in &plan.keys {
        slot(build, cell, SlotKind::Level).marker_tile = Some(TILE_KEY_LEVEL);
    }
    build.withheld.extend(requests.iter().map(|r| r.entry));
}

/// The build's slot of `kind` at `cell`.
fn slot(
    build: &mut BuildResult,
    (world, pos): MazePos,
    kind: SlotKind,
) -> &mut crate::randomize::overworld::build::SlotAssignment {
    build.worlds[world]
        .slots
        .iter_mut()
        .find(|s| s.pos == pos && s.kind == kind)
        .expect("the plan was made from a slot the build still has")
}

/// World 8, by internal index: its fortress slots belong to the army.
const WORLD_8: usize = 7;

/// Level or fortress cells of `kind` a gate or key may use: plain slots the
/// player can reach, not walled off.
///
/// **No World 8 fortress slot.** Three of World 8's army sprites (navy, air
/// force, super tank) each take one, and the sprite's battle replaces what is
/// under it — a gate there would be erased, and a gate the writer kept the
/// army off would leave a sprite, and the level it carries, off the map.
fn eligible(
    state: &GlobalState,
    kind: SlotKind,
    walled: &HashSet<MazePos>,
    reached: impl Fn(MazePos) -> bool,
) -> Vec<MazePos> {
    let mut out: Vec<MazePos> = state
        .worlds
        .iter()
        .filter(|w| state.in_maze[w.world_idx])
        .flat_map(|w| {
            w.slots
                .iter()
                .filter(|s| {
                    s.kind == kind
                        && !s.is_hand_trap
                        && !s.is_troll_pipe
                        && s.marker_tile.is_none()
                        && s.pin.is_none()
                })
                .map(move |s| (w.world_idx, s.pos))
        })
        .filter(|c| !walled.contains(c) && reached(*c))
        .filter(|c| !(kind == SlotKind::Fortress && c.0 == WORLD_8))
        .collect();
    out.sort();
    out
}

/// How many levels could still hold a key with `cell` gated: the levels the
/// player reaches with it shut, less itself and anything already taken.
fn room(
    state: &mut GlobalState,
    walled: &HashSet<MazePos>,
    cell: MazePos,
    levels: &[MazePos],
    taken: &HashSet<MazePos>,
) -> usize {
    // On the base maze, so gates placed earlier do not shrink it: their keys
    // come before them anyway.
    let mut shut = walled.clone();
    shut.insert(cell);
    let (_, reach) = on_base(state, |state| state.spheres_inner(&shut, None));
    levels.iter().filter(|&&c| c != cell && !taken.contains(&c) && reach.contains(c)).count()
}

/// How many shuffled candidates a key tries for an optional level before
/// taking whichever came first. Most reachable levels are optional, so the
/// first try usually lands.
const OPTIONAL_TRIES: usize = 6;

/// An optional level from `in_reach` if one turns up in a few tries, else any.
/// Shuffle-then-first: a uniform pick that usually costs one solver run.
fn pick_site<R: Rng>(
    state: &mut GlobalState,
    required: &mut Required,
    in_reach: &[MazePos],
    rng: &mut R,
) -> Option<MazePos> {
    let mut order = in_reach.to_vec();
    order.shuffle(rng);
    let optional = order.iter().take(OPTIONAL_TRIES).copied().find(|&c| !required.is(state, c));
    optional.or_else(|| order.first().copied())
}

/// "Is this cell required?", asked of the base maze and remembered.
struct Required {
    walled: HashSet<MazePos>,
    seen: HashMap<MazePos, bool>,
}

impl Required {
    fn new(walled: HashSet<MazePos>) -> Self {
        Required { walled, seen: HashMap::new() }
    }

    /// Walled off, the castle is out of reach — on the maze without our gates
    /// and keys. Asked with them in, a maze still waiting on a key reads as
    /// unwinnable whatever is walled, and every cell would look required.
    fn is(&mut self, state: &mut GlobalState, cell: MazePos) -> bool {
        if let Some(&r) = self.seen.get(&cell) {
            return r;
        }
        let mut blocked = self.walled.clone();
        blocked.insert(cell);
        let r = on_base(state, |state| !state.spheres_with_blocked(&blocked).solvable);
        self.seen.insert(cell, r);
        r
    }
}

/// Run `f` on the maze without this pass's gates and keys, then put them back.
/// Every other gate (the canoes) and key stays.
fn on_base<T>(state: &mut GlobalState, f: impl FnOnce(&mut GlobalState) -> T) -> T {
    let gates = state.gates.clone();
    let sources = state.sources.clone();
    state.gates.retain(|g| !matches!(g.target, GateTarget::Cell(_)));
    state.sources.retain(|s| !matches!(s.key, Key::PowerUp(_)));
    let out = f(state);
    state.gates = gates;
    state.sources = sources;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::randomize::maze::key_sites::test_support::{load_rom, one_maze};
    use crate::randomize::maze::powerup_gate::GATES;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    /// The classic gate table, resolved the way the vanilla ROM resolves it.
    fn requests() -> Vec<GateRequest> {
        let rom = load_rom().expect("caller checked");
        let catalog = crate::randomize::overworld::node_catalog::NodeCatalog::build(&rom, false);
        let reqs = gate_requests(&rom, &catalog, false, false);
        assert_eq!(reqs.len(), GATES.len(), "every gate name is in the catalog");
        reqs
    }

    /// The plan reaches the build as pins, flower tiles and the withheld list.
    #[test]
    fn mark_pins_gates_flowers_keys_and_withholds_every_gate() {
        if load_rom().is_none() {
            eprintln!("SKIP: requires the ROM");
            return;
        }
        let raw = load_rom().unwrap();
        let reqs = requests();
        let (_, mut build, mut state) = one_maze(&raw, 3);
        let plan = place(&mut state, &reqs, &PowerUp::ALL, &[], &mut ChaCha8Rng::seed_from_u64(3));
        mark(&mut build, &reqs, &plan);
        let at = |(w, pos): MazePos| build.worlds[w].slots.iter().find(|s| s.pos == pos).unwrap();
        for &(ri, cell) in &plan.gates {
            assert_eq!(at(cell).pin, Some(reqs[ri].entry));
        }
        for &(_, cell) in &plan.keys {
            assert_eq!(at(cell).marker_tile, Some(TILE_KEY_LEVEL));
        }
        let all: Vec<(usize, usize)> = reqs.iter().map(|r| r.entry).collect();
        assert_eq!(build.withheld, all);
    }

    /// Over many mazes: still winnable with every gate honoured, which is
    /// the guarantee that each key comes before its gate; one cell, one job;
    /// every power-up keyed; and gates mostly on required cells.
    #[test]
    fn every_maze_stays_winnable_with_keys_before_gates() {
        if load_rom().is_none() {
            eprintln!("SKIP: requires the ROM");
            return;
        }
        let raw = load_rom().unwrap();
        let reqs = requests();
        let seeds = 1..=24u64;
        let (mut placed, mut on_required, mut dropped) = (0, 0, 0);
        for seed in seeds.clone() {
            let (_, _, mut state) = one_maze(&raw, seed);
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let plan = place(&mut state, &reqs, &PowerUp::ALL, &[], &mut rng);

            assert!(state.spheres().solvable, "seed {seed}: unwinnable after placement");
            assert!(plan.unkeyed.is_empty(), "seed {seed}: unkeyed {:?}", plan.unkeyed);
            let mut cells: Vec<MazePos> = plan.gates.iter().map(|g| g.1).collect();
            cells.extend(plan.keys.iter().map(|k| k.1));
            let n = cells.len();
            cells.sort();
            cells.dedup();
            assert_eq!(cells.len(), n, "seed {seed}: a cell used twice");
            assert_eq!(plan.keys.len(), PowerUp::ALL.len(), "seed {seed}: one key each");

            // A gate placed is a gate in the model, and the solver needed
            // every one of its keys before it.
            for &(_, cell) in &plan.gates {
                assert!(state.gates.iter().any(|g| g.target == GateTarget::Cell(cell)));
                let mut blocked = HashSet::new();
                blocked.insert(cell);
                if !state.spheres_with_blocked(&blocked).solvable {
                    on_required += 1;
                }
            }
            placed += plan.gates.len();
            dropped += plan.dropped.len();
        }
        eprintln!("gates placed {placed}, on required cells {on_required}, dropped {dropped}");
        assert!(
            dropped * 10 <= placed,
            "too many gates dropped: {dropped} of {}",
            placed + dropped
        );
        assert!(on_required * 2 >= placed, "most gates should be required: {on_required}/{placed}");
    }

    /// Walling a key's cell off makes its gate unbeatable — the key really is
    /// the one the run depends on — unless the gate fell back off the route.
    #[test]
    fn a_gate_on_a_required_cell_depends_on_its_keys() {
        if load_rom().is_none() {
            eprintln!("SKIP: requires the ROM");
            return;
        }
        let raw = load_rom().unwrap();
        let reqs = requests();
        for seed in 1..=8u64 {
            let (_, _, mut state) = one_maze(&raw, seed);
            let plan =
                place(&mut state, &reqs, &PowerUp::ALL, &[], &mut ChaCha8Rng::seed_from_u64(seed));
            for &(ri, gate) in &plan.gates {
                let mut blocked = HashSet::new();
                blocked.insert(gate);
                if state.spheres_with_blocked(&blocked).solvable {
                    continue; // a fallback cell: nothing depends on it
                }
                for &p in &reqs[ri].needs {
                    let &(_, key_cell) = plan.keys.iter().find(|k| k.0 == p).unwrap();
                    let mut blocked = HashSet::new();
                    blocked.insert(key_cell);
                    assert!(
                        !state.spheres_with_blocked(&blocked).solvable,
                        "seed {seed}: the {p:?} key at {key_cell:?} is not needed for its gate"
                    );
                }
            }
        }
    }

    /// **What the pass costs**, per seed, native release. Instrument: run with
    /// `cargo test --release --lib placement_cost -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn placement_cost() {
        let Some(raw) = load_rom() else { return };
        let reqs = requests();
        let mut total = std::time::Duration::ZERO;
        let seeds = 40u64;
        for seed in 1..=seeds {
            let (_, _, mut state) = one_maze(&raw, seed);
            let t = std::time::Instant::now();
            place(&mut state, &reqs, &PowerUp::ALL, &[], &mut ChaCha8Rng::seed_from_u64(seed));
            total += t.elapsed();
        }
        eprintln!("placement: {:.1} ms per seed", total.as_secs_f64() * 1000.0 / seeds as f64);
    }

    /// Obelisk cells are never used, and no key depends on one.
    #[test]
    fn avoided_cells_are_walled_off() {
        if load_rom().is_none() {
            eprintln!("SKIP: requires the ROM");
            return;
        }
        let raw = load_rom().unwrap();
        let reqs = requests();
        let (_, _, state) = one_maze(&raw, 5);
        let mut rng = ChaCha8Rng::seed_from_u64(5);
        let obelisk = crate::randomize::maze::wandom::pick_pyramid(&state, &mut rng, &[])
            .expect("seed 5 has an optional level");
        let mut state = state;
        let plan = place(&mut state, &reqs, &PowerUp::ALL, &[obelisk], &mut rng);
        assert!(plan.gates.iter().all(|g| g.1 != obelisk));
        assert!(plan.keys.iter().all(|k| k.1 != obelisk));
        let walled: HashSet<MazePos> = [obelisk].into();
        assert!(state.spheres_with_blocked(&walled).solvable, "a key needs the obelisk");
    }
}
