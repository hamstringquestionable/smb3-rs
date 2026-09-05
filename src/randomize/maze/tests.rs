//! The maze verifier's own tests, and the null-model census.
//!
//! The census is the point of build-order step 1: it produces the baselines
//! ("what happens if we connect eight worlds and change nothing else") that
//! every later lever has to be argued as a delta against. Nothing here has a
//! target number yet — that is the whole reason it exists.

use rand::SeedableRng;
use rand::seq::SliceRandom;
use rand_chacha::ChaCha8Rng;

use super::graph::PAD_BUDGET;
use super::pads::place_pads_uniform;
use super::{GlobalState, IDENTITY_SPINE, MazeEdge};
use crate::randomize::map_walker::walk_reachable;
use crate::randomize::maze::walk::{MazeWorld, walk_maze};
use crate::randomize::node_catalog::NodeCatalog;
use crate::randomize::overworld_build::{
    BuildFlags, BuildResult, OverworldData, SlotKind, build, stamp_slots,
};
use crate::randomize::overworld_pickup::{PickupFlags, pick_up};
use crate::randomize::rom_data::Grid;
use crate::randomize::{qol, start_airship_swap};
use crate::rom::Rom;

fn load_rom() -> Option<Rom> {
    let bytes = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
    Rom::from_bytes(&bytes).ok()
}

fn census_seeds(default: u64) -> u64 {
    std::env::var("CENSUS_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

/// Map QOL in production order, per census arm — a copy of the overworld
/// census's `apply_qol_variant`, which is private to that test module.
fn qol_variant(rom: &Rom, hammer_rocks: bool, eights_wild: bool) -> Rom {
    let mut out = rom.clone();
    qol::fix_w3_drawbridges(&mut out);
    qol::remove_rocks(&mut out);
    if hammer_rocks {
        qol::make_hammer_rocks(&mut out);
    }
    qol::apply_w1_shortcut(&mut out, hammer_rocks);
    qol::apply_w8_bridges(&mut out);
    if eights_wild {
        qol::apply_w8_canoe_and_paths(&mut out);
    }
    qol::fix_big_q_block_rooms(&mut out);
    out
}

/// One seed's eight-world build, over the same flag arms the overworld census
/// uses: 50% base / 25% more-hammer-rocks / 25% 8s-are-wild, with start↔airship
/// swap rolled per world. The maze must hold on every map the builder can
/// produce, not on one arm of it.
fn census_build(raw: &Rom, seed: u64) -> (Rom, BuildResult) {
    let (hammer_rocks, eights_wild) = match seed % 4 {
        2 => (true, false),
        3 => (false, true),
        _ => (false, false),
    };
    let rom = qol_variant(raw, hammer_rocks, eights_wild);
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut catalog = NodeCatalog::build(&rom, false);
    let mut swap_rng = ChaCha8Rng::seed_from_u64(seed);
    start_airship_swap::pick_swaps(&mut catalog, &mut swap_rng);
    let pickup = pick_up(
        &rom,
        &catalog,
        PickupFlags { shuffle_spade_games: true, shuffle_toad_houses: true, ..Default::default() },
    );
    let result = build(
        &rom,
        &OverworldData { pickup: &pickup, catalog: &catalog },
        &mut rng,
        BuildFlags {
            shuffle_toad_houses: true,
            eights_are_wild: eights_wild,
            ..Default::default()
        },
    );
    (rom, result)
}

/// The null model: an unmodified eight-world build, the identity spine, and
/// uniform-random pads. K = 0 (a pure maze) because the wand gate does not
/// exist yet.
fn null_model(raw: &Rom, seed: u64) -> GlobalState {
    let (_, result) = census_build(raw, seed);
    let mut state = GlobalState::from_build(&result, &IDENTITY_SPINE, 0);
    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0x9E37_79B9);
    let pads = place_pads_uniform(&state.worlds, &mut rng);
    state.add_pads(pads);
    state
}

// ---------------------------------------------------------------------------
// The walker
// ---------------------------------------------------------------------------

/// The maze walker is a COPY of `reach_from`'s 2-tile move expansion (see the
/// note above it), and this is what stops the copy drifting: given the eight
/// worlds and no links, walking from one world's start must reproduce
/// `walk_reachable` for that world cell for cell — and reach nothing at all in
/// the other seven, since without a link there is no way across.
///
/// The oracle is the shipping walker rather than a hand-written expectation,
/// which is the point: a fixture that shares the code's assumption cannot test
/// that assumption.
#[test]
fn maze_walk_matches_the_per_world_walker() {
    let Some(raw) = load_rom() else { return };
    for seed in 0..census_seeds(4) {
        let (_, result) = census_build(&raw, seed);
        let state = GlobalState::from_build(&result, &IDENTITY_SPINE, 0);

        // Locks closed, slots stamped — the fixpoint's round-zero grids.
        let grids: Vec<Grid> = state
            .worlds
            .iter()
            .map(|w| {
                let mut g = w.grid.clone();
                stamp_slots(&mut g, &w.slots);
                for lock in &w.locks {
                    g.set(lock.pos.0, lock.pos.1, lock.gap_tile);
                }
                g
            })
            .collect();
        let view: Vec<MazeWorld> = grids
            .iter()
            .zip(&state.worlds)
            .map(|(grid, w)| MazeWorld { grid, pipe_pairs: &w.pipe_pairs })
            .collect();

        for (wi, grid) in grids.iter().enumerate() {
            let w = &state.worlds[wi];
            let want = walk_reachable(grid, &w.pipe_pairs, w.start, wi);
            let got = walk_maze(&view, &[], (wi, w.start.expect("every world has a START")));
            for r in 0..grid.rows() {
                for c in 0..grid.cols {
                    assert_eq!(
                        want.contains((r, c)),
                        got.contains((wi, (r, c))),
                        "seed {seed} W{} ({r},{c}): maze walker disagrees with walk_reachable",
                        wi + 1
                    );
                }
            }
            for other in (0..8).filter(|&o| o != wi) {
                assert_eq!(
                    got.world_len(other),
                    0,
                    "seed {seed}: walking W{} with no links reached W{}",
                    wi + 1,
                    other + 1
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Acceptance
// ---------------------------------------------------------------------------

/// `pad_ids_fit`: the pad count never exceeds the 16 arrival rows, and every
/// pad owns a distinct one — the key tables are indexed by arrival id, so two
/// pads on one tile would be two rows claiming the same key.
#[test]
fn pad_ids_fit() {
    let Some(raw) = load_rom() else { return };
    for seed in 0..census_seeds(8) {
        let state = null_model(&raw, seed);
        let pads = state.pad_edges();
        assert!(pads.len() <= PAD_BUDGET, "seed {seed}: {} pads > {PAD_BUDGET}", pads.len());
        let mut tiles: Vec<_> = pads.iter().map(|&(from, _)| from).collect();
        tiles.sort_unstable();
        let before = tiles.len();
        tiles.dedup();
        assert_eq!(before, tiles.len(), "seed {seed}: two pads share one tile");
    }
}

/// The spine alone must reach every world and finish the game. This is the
/// instrument's own calibration as much as a property: the per-world builder
/// already guarantees each world is completable from its own start, and the
/// spine deposits the player on exactly that start — so a failure here means
/// the maze walker or the global fixpoint is wrong, not the map.
///
/// Run with no pads, because a pad can only ADD edges: if the spine-only maze
/// is solvable, every padded one built on it is too.
#[test]
fn the_spine_alone_completes_the_maze() {
    let Some(raw) = load_rom() else { return };
    for seed in 0..census_seeds(8) {
        let (_, result) = census_build(&raw, seed);
        let state = GlobalState::from_build(&result, &IDENTITY_SPINE, 0);
        let s = state.spheres();
        assert!(s.solvable, "seed {seed}: spine-only maze unsolvable\n{}", s.spoiler());
        assert_eq!(
            s.wands_at_goal,
            7,
            "seed {seed}: the spine passes all 7 airships before the castle\n{}",
            s.spoiler()
        );
        // `wands_are_collectable`, at the hardest setting the dial reaches.
        // The spine visits all seven airships on the way, so K = 7 holds; the
        // number only becomes interesting once pads let the player skip ahead.
        let hardest = GlobalState::from_build(&result, &IDENTITY_SPINE, 7);
        assert!(
            hardest.wands_are_collectable(&hardest.spheres()),
            "seed {seed}: K=7 unsatisfiable on a spine-only maze"
        );
    }
}

/// Invariant 3, both readings, measured side by side — and the gap between them
/// is the point.
///
/// The charter asked for an exit reachable with **zero keys**. That is stricter
/// than safety needs: a fortress inside the start region is a key the player
/// can go and get, and what actually soft-locks is a start region with no
/// ungated exit AND no fortress that opens one. If the strict form is rare and
/// the correct one is near-universal, the start-region rule is a cheap guard
/// rather than a placement phase — which decides how much of the 16-pad budget
/// safety has to reserve.
#[test]
fn start_region_exit_rate() {
    let Some(raw) = load_rom() else { return };
    let seeds = census_seeds(8);
    let mut strict = 0usize;
    let mut escapable = 0usize;
    let mut total = 0usize;
    let mut trapped = Vec::new();
    for seed in 0..seeds {
        let state = null_model(&raw, seed);
        for wi in 0..8 {
            total += 1;
            strict += usize::from(state.start_region_has_exit(wi));
            if state.start_region_escapable(wi) {
                escapable += 1;
            } else {
                trapped.push(format!("seed {seed} W{}", wi + 1));
            }
        }
    }
    let pct = |n: usize| 100.0 * n as f64 / total as f64;
    eprintln!(
        "  strict  (ungated exit, the charter's wording): {strict}/{total} = {:.1}%",
        pct(strict)
    );
    eprintln!(
        "  correct (ungated exit OR a fort that opens one): {escapable}/{total} = {:.1}%",
        pct(escapable)
    );
    if !trapped.is_empty() {
        eprintln!("  worlds that could strand a player: {}", trapped.join(", "));
    }
    // The correct form is a safety property, not a preference: a world that
    // fails it can strand a player who arrives with nothing. The generator
    // spends a hub pad on any that do, so this is measuring how often that
    // rescue is even needed — and the answer decides whether the start-region
    // rule is a placement phase or a cheap guard.
    assert!(
        escapable >= strict,
        "the weaker form must hold wherever the stricter one does: {escapable} < {strict}"
    );
}

// ---------------------------------------------------------------------------
// The null-model census
// ---------------------------------------------------------------------------

/// The baseline the charter asks for. No target numbers: this measures what
/// eight connected worlds do with nothing else changed, and every later lever
/// is argued as a delta against it.
///
/// ```sh
/// CENSUS_SEEDS=200 cargo test --release --lib maze_null_model_census \
///     -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn maze_null_model_census() {
    let Some(raw) = load_rom() else { return };
    let seeds = census_seeds(50);

    let mut solvable = 0usize;
    let mut sphere_counts: Vec<usize> = Vec::new();
    let mut width_hist = [0usize; 8];
    let mut wide_widths = 0usize;
    let mut sphere0_content = Vec::new();
    let mut pad_counts: Vec<usize> = Vec::new();
    let mut foreign_locks = 0usize;
    let mut wands_at_goal: Vec<usize> = Vec::new();
    let mut goal_spheres: Vec<usize> = Vec::new();
    let mut invariant3 = 0usize;

    for seed in 0..seeds {
        let state = null_model(&raw, seed);
        let s = state.spheres();
        if s.solvable {
            solvable += 1;
        }
        sphere_counts.push(s.spheres.len());
        for sphere in &s.spheres {
            match width_hist.get_mut(sphere.width) {
                Some(slot) => *slot += 1,
                None => wide_widths += 1,
            }
        }
        let all_content: usize = s.spheres.iter().map(|x| x.reached.len()).sum();
        if all_content > 0 {
            sphere0_content.push(100.0 * s.spheres[0].reached.len() as f64 / all_content as f64);
        }
        pad_counts.push(state.pad_edges().len());
        foreign_locks += state.locks.iter().filter(|l| l.is_foreign()).count();
        wands_at_goal.push(s.wands_at_goal);
        goal_spheres.extend(s.goal_sphere);
        invariant3 += (0..8).filter(|&wi| state.start_region_has_exit(wi)).count();

        if seed == 0 {
            eprintln!("seed 0 spoiler:\n{}", s.spoiler());
        }
    }

    let mean = |v: &[usize]| v.iter().sum::<usize>() as f64 / v.len().max(1) as f64;
    eprintln!("\n=== world-maze null model, {seeds} seeds ===");
    eprintln!("  spine: identity (W1->..->W8), pads uniform, locks all local, K=0");
    eprintln!(
        "  solvable            {solvable}/{seeds} = {:.1}%",
        100.0 * solvable as f64 / seeds as f64
    );
    eprintln!(
        "  spheres             mean {:.2}  min {}  max {}",
        mean(&sphere_counts),
        sphere_counts.iter().min().copied().unwrap_or(0),
        sphere_counts.iter().max().copied().unwrap_or(0),
    );
    let total_spheres: usize = width_hist.iter().sum::<usize>() + wide_widths;
    eprint!("  sphere width        ");
    for (w, &n) in width_hist.iter().enumerate() {
        eprint!("{w}:{:.0}% ", 100.0 * n as f64 / total_spheres.max(1) as f64);
    }
    eprintln!("8+:{:.0}%", 100.0 * wide_widths as f64 / total_spheres.max(1) as f64);
    eprintln!(
        "  sphere-0 openness   {:.1}% of all content reachable with zero keys",
        sphere0_content.iter().sum::<f64>() / sphere0_content.len().max(1) as f64
    );
    eprintln!(
        "  goal sphere         mean {:.2}  max {}",
        mean(&goal_spheres),
        goal_spheres.iter().max().copied().unwrap_or(0),
    );
    eprintln!("  wands before goal   mean {:.2}", mean(&wands_at_goal));
    eprintln!(
        "  pads placed         mean {:.2}  max {}  (budget {PAD_BUDGET})",
        mean(&pad_counts),
        pad_counts.iter().max().copied().unwrap_or(0),
    );
    eprintln!("  foreign locks       {foreign_locks} (the fill does not exist yet, so 0 is right)");
    eprintln!(
        "  invariant 3         {invariant3}/{} = {:.1}%",
        seeds * 8,
        100.0 * invariant3 as f64 / (seeds * 8) as f64
    );
}

/// How much a pad web actually changes the null model — the first question the
/// census cannot answer on its own, because the spine already solves the game.
/// Prints the spine-only sphere structure beside the padded one.
#[test]
#[ignore]
fn maze_pads_vs_spine_only() {
    let Some(raw) = load_rom() else { return };
    let seeds = census_seeds(50);
    let mut bare = Vec::new();
    let mut padded = Vec::new();
    let mut pad_shortcuts = 0usize;

    for seed in 0..seeds {
        let (_, result) = census_build(&raw, seed);
        let plain = GlobalState::from_build(&result, &IDENTITY_SPINE, 0);
        let bare_s = plain.spheres();

        let mut with_pads = GlobalState::from_build(&result, &IDENTITY_SPINE, 0);
        let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0x9E37_79B9);
        let pads = place_pads_uniform(&with_pads.worlds, &mut rng);
        with_pads.add_pads(pads);
        let pad_s = with_pads.spheres();

        // A pad that crosses worlds is a shortcut only if it lands somewhere
        // the spine would not have reached by then; the sphere count falling
        // is the cheap proxy the null model can afford.
        if pad_s.spheres.len() < bare_s.spheres.len() {
            pad_shortcuts += 1;
        }
        bare.push(bare_s.spheres.len());
        padded.push(pad_s.spheres.len());
    }

    let mean = |v: &[usize]| v.iter().sum::<usize>() as f64 / v.len().max(1) as f64;
    eprintln!("\n=== pads vs spine-only, {seeds} seeds ===");
    eprintln!("  spheres, spine only  mean {:.2}", mean(&bare));
    eprintln!("  spheres, with pads   mean {:.2}", mean(&padded));
    eprintln!(
        "  pads shortened it    {pad_shortcuts}/{seeds} = {:.1}%",
        100.0 * pad_shortcuts as f64 / seeds as f64
    );
}

/// Content the maze holds, as the verifier sees it — a sanity read on the
/// wrapper rather than on the maze: every level, fort and pad the build placed
/// has to appear in the spoiler log's reached set of a solvable seed, or the
/// log is lying about coverage.
#[test]
fn every_placed_slot_is_reached() {
    let Some(raw) = load_rom() else { return };
    for seed in 0..census_seeds(4) {
        let state = null_model(&raw, seed);
        let s = state.spheres();
        assert!(s.solvable, "seed {seed}: unsolvable\n{}", s.spoiler());

        let reached: std::collections::HashSet<_> =
            s.spheres.iter().flat_map(|x| x.reached.iter().copied()).collect();
        for w in state.worlds.iter().filter(|w| state.in_maze[w.world_idx]) {
            for slot in w.slots.iter().filter(|s| s.kind != SlotKind::HammerBro) {
                assert!(
                    reached.contains(&(w.world_idx, slot.pos)),
                    "seed {seed} W{} {:?} at {:?} never became reachable\n{}",
                    w.world_idx + 1,
                    slot.kind,
                    slot.pos,
                    s.spoiler()
                );
            }
        }
    }
}

/// The pad placer's own shape: counts land in 0..=3, and the budget binds
/// globally rather than per world.
#[test]
fn pad_placer_respects_its_budget() {
    let Some(raw) = load_rom() else { return };
    let (_, result) = census_build(&raw, 1);
    let state = GlobalState::from_build(&result, &IDENTITY_SPINE, 0);
    for seed in 0..64u64 {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let pads = place_pads_uniform(&state.worlds, &mut rng);
        assert!(pads.len() <= PAD_BUDGET);
        let mut per_world = [0usize; 8];
        for pad in &pads {
            let MazeEdge::Pad { from, .. } = *pad else { panic!("placer emitted a non-pad edge") };
            per_world[from.0] += 1;
        }
        for (wi, &n) in per_world.iter().enumerate() {
            assert!(n <= 3, "seed {seed} W{}: {n} pads, max is 3", wi + 1);
        }
    }
}

/// The weighted roll is reachable at every count and biased to 1.
#[test]
fn pad_count_roll_is_biased_to_one() {
    let mut rng = ChaCha8Rng::seed_from_u64(9);
    let mut hist = [0usize; 4];
    for _ in 0..40_000 {
        hist[super::pads::roll_count(&mut rng)] += 1;
    }
    assert!(hist.iter().all(|&n| n > 0), "every count 0..=3 must be reachable: {hist:?}");
    let most = hist.iter().enumerate().max_by_key(|&(_, n)| n).map(|(i, _)| i);
    assert_eq!(most, Some(1), "1 pad must be the modal count: {hist:?}");
}

/// What terrain the roles actually have to work with. Placed before any role
/// placer exists, because a role whose pool is empty is a design that cannot
/// be built — and `island_sites` / `island_landings` are the ones at risk:
/// `Connectivity` bridges islands with pipes and `HammerBroFill` claims every
/// reachable blank, so a finished world may have nothing left that no walk
/// reaches.
#[test]
#[ignore]
fn maze_terrain_pools_census() {
    let Some(raw) = load_rom() else { return };
    let seeds = census_seeds(50);
    let mut hub = 0usize;
    let mut gated = 0usize;
    let mut island = 0usize;
    let mut island_land = 0usize;
    let mut ungated_target = 0usize;
    let mut worlds_with_island = 0usize;
    let mut worlds_without_hub = 0usize;
    let mut per_world_island = [0usize; 8];

    for seed in 0..seeds {
        let (_, result) = census_build(&raw, seed);
        let state = GlobalState::from_build(&result, &IDENTITY_SPINE, 0);
        for w in &state.worlds {
            let t = super::roles::classify(w, &state.locks, &state.reserved_in(w.world_idx));
            hub += t.hub_sites.len();
            gated += t.gated_sites.len();
            island += t.island_sites.len();
            island_land += t.island_landings.len();
            ungated_target += t.target_ungated as usize;
            if !t.island_landings.is_empty() {
                worlds_with_island += 1;
                per_world_island[w.world_idx] += 1;
            }
            if t.hub_sites.is_empty() {
                worlds_without_hub += 1;
            }
        }
    }

    let n = (seeds * 8) as f64;
    eprintln!("\n=== maze terrain pools, {seeds} seeds ({} world-seeds) ===", seeds * 8);
    eprintln!("  hub sites      mean {:.1} per world", hub as f64 / n);
    eprintln!("  gated sites    mean {:.1}", gated as f64 / n);
    eprintln!("  island sites   mean {:.2}", island as f64 / n);
    eprintln!("  island landings mean {:.2}", island_land as f64 / n);
    eprintln!(
        "  worlds with an island at all   {worlds_with_island}/{} = {:.1}%",
        seeds * 8,
        100.0 * worlds_with_island as f64 / n
    );
    eprintln!("    by world: {per_world_island:?}");
    eprintln!(
        "  worlds with NO hub site        {worlds_without_hub}/{} = {:.1}%",
        seeds * 8,
        100.0 * worlds_without_hub as f64 / n
    );
    eprintln!(
        "  target ungated (invariant 3 by terrain alone) {ungated_target}/{} = {:.1}%",
        seeds * 8,
        100.0 * ungated_target as f64 / n
    );
}

// ---------------------------------------------------------------------------
// The generator
// ---------------------------------------------------------------------------

/// What a pad's landing cell reads as once the map is drawn.
///
/// The distinction that matters is `Filler` against everything else.
/// `stamp_slots` leaves a `HammerBro` slot as a blank path tile — it is the
/// builder's leftover pool, not content — so a pad landing on one deposits the
/// player on a cell that looks like nothing. Every other variant is a tile the
/// map actually draws.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Landing {
    /// Another pad's tile: a spade panel, and stepping on it goes onward.
    Pad,
    Level,
    Fortress,
    ToadHouse,
    BonusGame,
    Pipe,
    /// The world's start tile — often plain path, but the one cell a player can
    /// always name, and touching it is what marks the world whistle-able.
    Start,
    /// A blank path tile. **The defect**, and what
    /// `every_pad_lands_somewhere_legible` bars.
    Filler,
}

const LANDING_LABELS: [&str; 8] =
    ["pad", "level", "fort", "house", "spade", "pipe", "start", "FILLER"];

fn landing_idx(l: Landing) -> usize {
    match l {
        Landing::Pad => 0,
        Landing::Level => 1,
        Landing::Fortress => 2,
        Landing::ToadHouse => 3,
        Landing::BonusGame => 4,
        Landing::Pipe => 5,
        Landing::Start => 6,
        Landing::Filler => 7,
    }
}

/// Classify one landing cell. Pad tiles are tested first because a pad is
/// stamped OVER whatever slot it took, so a pad on a Hammer Bro slot is a spade
/// panel on the finished map and not filler at all.
fn landing_kind(state: &GlobalState, to: (usize, (usize, usize))) -> Landing {
    if state.pad_edges().iter().any(|&(from, _)| from == to) {
        return Landing::Pad;
    }
    let w = &state.worlds[to.0];
    if let Some(slot) = w.slots.iter().find(|s| s.pos == to.1) {
        return match slot.kind {
            SlotKind::Level => Landing::Level,
            SlotKind::Fortress => Landing::Fortress,
            SlotKind::ToadHouse => Landing::ToadHouse,
            SlotKind::BonusGame => Landing::BonusGame,
            SlotKind::Pipe => Landing::Pipe,
            SlotKind::HammerBro => Landing::Filler,
        };
    }
    if w.start == Some(to.1) {
        return Landing::Start;
    }
    Landing::Filler
}

/// One generated maze at the given knobs.
fn generated(raw: &Rom, seed: u64, knobs: &super::graph::Knobs) -> (GlobalState, super::GenReport) {
    generated_k(raw, seed, knobs, 0)
}

/// The same, at a chosen wand requirement.
fn generated_k(
    raw: &Rom,
    seed: u64,
    knobs: &super::graph::Knobs,
    k: u8,
) -> (GlobalState, super::GenReport) {
    let (_, result) = census_build(raw, seed);
    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0x5EED_1234);
    super::generate(&result, &IDENTITY_SPINE, k, knobs, &mut rng)
}

/// The generator's two hard guarantees, at every knob setting: the maze is
/// finishable and no world can strand a player who arrives in it.
///
/// This is not a measurement — a failure is a bug, not a number. It runs the
/// knobs at both extremes and the null because the guarantee has to survive the
/// whole dial, not just the default.
#[test]
fn the_generator_never_ships_an_unwinnable_maze() {
    let Some(raw) = load_rom() else { return };
    let arms: [(&str, super::graph::Knobs); 3] = [
        ("null", super::graph::Knobs::default()),
        ("local keys", super::graph::Knobs { fort_distance_bias: -1.0, ..Default::default() }),
        (
            "far keys, all pads cross",
            super::graph::Knobs { fort_distance_bias: 1.0, foreign_landing_bias: 1.0 },
        ),
    ];
    for seed in 0..census_seeds(4) {
        for (name, knobs) in &arms {
            // K at both ends: the wand gate is the only thing that can make a
            // reachable castle unenterable, so the guarantee has to hold with
            // it wide open and with it demanding every wand in the game.
            for k in [0u8, super::DEFAULT_WANDS_REQUIRED, 7] {
                let (state, report) = generated_k(&raw, seed, knobs, k);
                let name = &format!("{name}, K={k}");
                assert!(
                    report.spheres.solvable,
                    "seed {seed} [{name}]: unwinnable\n{}",
                    report.spheres.spoiler()
                );
                assert!(
                    report.unsafe_worlds.is_empty(),
                    "seed {seed} [{name}]: worlds that can strand a player: {:?}",
                    report.unsafe_worlds
                );
                // Every fortress keeps exactly one lock — the charter's
                // map-legibility rule, and what lets a world's lock count tell the
                // player its fort count before the forts are found.
                let forts: usize = state
                    .worlds
                    .iter()
                    .map(|w| w.slots.iter().filter(|s| s.kind == SlotKind::Fortress).count())
                    .sum();
                let keyed: std::collections::HashSet<_> =
                    state.locks.iter().filter_map(|l| l.fort).collect();
                assert_eq!(
                    keyed.len(),
                    forts,
                    "seed {seed} [{name}]: {} forts but {} of them key a lock",
                    forts,
                    keyed.len()
                );
                assert!(
                    state.pad_edges().len() <= super::graph::PAD_BUDGET,
                    "seed {seed} [{name}]: pad budget blown"
                );
                // The run has to be playable, not merely reachable: the lazy
                // player must actually be able to collect K wands and finish.
                let cost = super::metrics::completion_cost(&state);
                assert!(cost.reached, "seed {seed} [{name}]: no completion path");
                assert_eq!(
                    cost.wand_detours, k as usize,
                    "seed {seed} [{name}]: collected {} wands, gate wants {k}",
                    cost.wand_detours
                );
            }
        }
    }
}

/// **Every pad lands on a cell the map draws something on, and no same-world
/// hop is a stroll.**
///
/// This is the playtest report, pinned. "The first spade in W1 ported to W1 but
/// onto a blank tile so that's not working" was two defects at once, and both
/// are asserted here:
///
/// * The landing was a Hammer Bro **filler** slot. `stamp_slots` leaves those
///   as blank path tiles — they are the leftover pool, not content — so the
///   player was deposited on a cell that looks like nothing, with no sign they
///   had arrived anywhere. Measured before the rule: **45% of every landing**.
/// * The hop was **same-world and short**. The census min span was 0: a pad
///   that teleported to its own tile. Anything under
///   [`SAME_WORLD_MIN_SPAN`](super::graph::SAME_WORLD_MIN_SPAN) spends one of
///   sixteen arrival rows to move the player a few tiles they can see.
///
/// A landing is legible when it is another pad's tile, a slot the map draws
/// (level, fortress, pipe, house, spade), or the world's start tile — the one
/// cell a player can always name, and the one whose touch marks the world
/// whistle-able.
///
/// **Landing on a pad tile cannot loop.** The enter hook replaces
/// `PRG010_CEA7`, and all three branches that reach it sit downstream of an
/// A-button EDGE test (`Controller1Press`/`Controller2Press` for the 2P path,
/// `Pad_Input AND #PAD_A` for the special-tile and attribute-table paths). It
/// fires when the player COMMITS to the tile they stand on, never on arriving
/// at one.
#[test]
fn every_pad_lands_somewhere_legible() {
    // Spelled out rather than read off `graph::SAME_WORLD_MIN_SPAN`, and the
    // difference is not cosmetic: the first cut of this test imported the
    // constant, and setting that constant to 0 then left the test PASSING. A
    // check that reads the number it is pinning pins nothing. This is the
    // number the rule IS, so lowering the generator's constant has to fail
    // here rather than quietly redefine the rule.
    const MIN_SPAN: usize = 8;

    let Some(raw) = load_rom() else { return };
    let arms: [(&str, super::graph::Knobs); 3] = [
        ("null", super::graph::Knobs::default()),
        (
            "all pads stay home",
            super::graph::Knobs { foreign_landing_bias: 0.0, fort_distance_bias: 0.0 },
        ),
        (
            "all pads cross",
            super::graph::Knobs { foreign_landing_bias: 1.0, fort_distance_bias: 1.0 },
        ),
    ];
    for seed in 0..census_seeds(4) {
        for (name, knobs) in &arms {
            let (state, _) = generated(&raw, seed, knobs);
            for (from, to) in state.pad_edges() {
                let kind = landing_kind(&state, to);
                assert_ne!(
                    kind,
                    Landing::Filler,
                    "seed {seed} [{name}]: the pad at W{} {:?} lands on W{} {:?}, which the map \
                     draws nothing on — the player cannot tell they arrived anywhere",
                    from.0 + 1,
                    from.1,
                    to.0 + 1,
                    to.1
                );
                // A pad landing on its own tile is the span-0 case, so the
                // span rule below covers it; there is no separate assert.
                if from.0 == to.0 {
                    let d = from.1.0.abs_diff(to.1.0) + from.1.1.abs_diff(to.1.1);
                    assert!(
                        d >= MIN_SPAN,
                        "seed {seed} [{name}]: same-world hop W{} {:?} -> {:?} spans {d}, under \
                         the {MIN_SPAN} that makes a pad worth its arrival id",
                        from.0 + 1,
                        from.1,
                        to.1,
                    );
                }
            }
        }
    }
}

/// The generator census, and the one that answers "how long is this game".
///
/// ```sh
/// CENSUS_SEEDS=100 cargo test --release --lib maze_generator_census \
///     -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn maze_generator_census() {
    let Some(raw) = load_rom() else { return };
    let seeds = census_seeds(50);
    let arms: [(&str, super::graph::Knobs); 4] = [
        ("null (bias 0)", super::graph::Knobs::default()),
        ("local keys (-1)", super::graph::Knobs { fort_distance_bias: -1.0, ..Default::default() }),
        ("far keys (+1)", super::graph::Knobs { fort_distance_bias: 1.0, ..Default::default() }),
        (
            "far keys + all pads cross",
            super::graph::Knobs { fort_distance_bias: 1.0, foreign_landing_bias: 1.0 },
        ),
    ];

    eprintln!("\n=== world-maze generator, {seeds} seeds per arm ===");
    eprintln!(
        "{:<28} {:>7} {:>7} {:>7} {:>8} {:>8} {:>7} {:>7}",
        "arm", "spheres", "width", "foreign", "levels", "forts", "pads", "cross"
    );
    for (name, knobs) in &arms {
        let mut spheres = Vec::new();
        let mut widths = Vec::new();
        let mut foreign = Vec::new();
        let mut content = Vec::new();
        let mut forts = Vec::new();
        let mut pads = Vec::new();
        let mut cross = Vec::new();
        let mut unreached = 0usize;
        // The charter asks for the granted-vs-requested role distribution as a
        // census output, because a role is a request and not a contract: some
        // terrain cannot host what was asked for.
        let mut asked = [0usize; 3];
        let mut got = [0usize; 3];
        let mut denied = 0usize;
        // The landing mix: what kind of tile a pad actually deposits the
        // player on. `FILLER` is the one that must stay 0 — see
        // `every_pad_lands_somewhere_legible`.
        let mut mix = [0usize; 8];
        let mut hops: Vec<usize> = Vec::new();
        let idx = |r: super::roles::PadRole| match r {
            super::roles::PadRole::Hub => 0,
            super::roles::PadRole::Shortcut => 1,
            super::roles::PadRole::Free => 2,
        };

        for seed in 0..seeds {
            let (state, report) = generated(&raw, seed, knobs);
            for pad in &report.pads {
                asked[idx(pad.requested)] += 1;
                got[idx(pad.granted)] += 1;
                if pad.requested != pad.granted && pad.requested != super::roles::PadRole::Free {
                    denied += 1;
                }
                let MazeEdge::Pad { from, to } = pad.edge else { unreachable!() };
                assert_eq!(
                    pad.foreign,
                    from.0 != to.0,
                    "PlacedPad::foreign disagrees with its own edge"
                );
            }
            spheres.push(report.spheres.spheres.len());
            widths.extend(report.spheres.spheres.iter().map(|s| s.width));
            foreign.push(report.fill.foreign_locks);
            let cost = super::metrics::completion_cost(&state);
            if !cost.reached {
                unreached += 1;
            }
            content.push(cost.content);
            forts.push(cost.forts);
            let p = state.pad_edges();
            for &(from, to) in &p {
                mix[landing_idx(landing_kind(&state, to))] += 1;
                if from.0 == to.0 {
                    hops.push(from.1.0.abs_diff(to.1.0) + from.1.1.abs_diff(to.1.1));
                }
            }
            pads.push(p.len());
            cross.push(p.iter().filter(|((a, _), (b, _))| a != b).count());
        }
        let mean = |v: &[usize]| v.iter().sum::<usize>() as f64 / v.len().max(1) as f64;
        eprintln!(
            "{name:<28} {:>7.2} {:>7.2} {:>7.2} {:>8.1} {:>8.1} {:>7.2} {:>7.2}",
            mean(&spheres),
            mean(&widths),
            mean(&foreign),
            mean(&content),
            mean(&forts),
            mean(&pads),
            mean(&cross),
        );
        eprintln!("    roles asked hub/shortcut/free {asked:?}  granted {got:?}  denied {denied}");
        let total: usize = mix.iter().sum();
        let mix_str: Vec<String> = LANDING_LABELS
            .iter()
            .zip(mix)
            .map(|(l, n)| format!("{l} {n} ({:.0}%)", 100.0 * n as f64 / total.max(1) as f64))
            .collect();
        eprintln!("    landings: {}", mix_str.join("  "));
        hops.sort_unstable();
        eprintln!(
            "    same-world hops: {} of {total}, span min {:?} median {:?} mean {:.1}",
            hops.len(),
            hops.first(),
            hops.get(hops.len() / 2),
            hops.iter().sum::<usize>() as f64 / hops.len().max(1) as f64,
        );
        assert_eq!(unreached, 0, "[{name}]: {unreached} seeds never reached the castle");
    }
    eprintln!("  levels = levels+forts a play-through beats; forts = how many of those were keys");
}

/// How many levels the game actually asks of a player, and how many it forces.
/// The gap between the two is the size of the choice the maze offers.
///
/// ```sh
/// CENSUS_SEEDS=25 cargo test --release --lib maze_game_length_census \
///     -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn maze_game_length_census() {
    let Some(raw) = load_rom() else { return };
    let seeds = census_seeds(20);
    let knobs = super::graph::Knobs::default();
    let mut played = Vec::new();
    let mut required = Vec::new();
    let mut total_levels = Vec::new();
    let mut detours = Vec::new();

    for seed in 0..seeds {
        // At the SHIPPING wand requirement, not K=0. K=0 is a legal setting but
        // it is the one where a pad chain can finish a seed in a single level,
        // so measuring the game's length there answers a question nobody asked.
        let (state, _) = generated_k(&raw, seed, &knobs, super::DEFAULT_WANDS_REQUIRED);
        let cost = super::metrics::completion_cost(&state);
        played.push(cost.content);
        detours.push(cost.detours);
        required.push(super::metrics::required_levels(&state));
        total_levels.push(
            state
                .worlds
                .iter()
                .map(|w| w.slots.iter().filter(|s| s.kind == SlotKind::Level).count())
                .sum::<usize>(),
        );
    }

    let mean = |v: &[usize]| v.iter().sum::<usize>() as f64 / v.len().max(1) as f64;
    let stat = |v: &[usize]| {
        let mut s = v.to_vec();
        s.sort_unstable();
        (s[0], s[s.len() / 2], s[s.len() - 1])
    };
    let (pl, pm, ph) = stat(&played);
    let (rl, rm, rh) = stat(&required);
    eprintln!(
        "\n=== how long is a maze game, {seeds} seeds, K={} ===",
        super::DEFAULT_WANDS_REQUIRED
    );
    eprintln!("  levels in the game        {:.0}", mean(&total_levels));
    eprintln!(
        "  BEATEN on a completion    mean {:.1}  min {pl}  median {pm}  max {ph}",
        mean(&played)
    );
    eprintln!(
        "  strictly REQUIRED         mean {:.1}  min {rl}  median {rm}  max {rh}",
        mean(&required)
    );
    eprintln!("  fort detours              mean {:.1}", mean(&detours));
    eprintln!(
        "  so a run plays {:.0}% of the game, and {:.0}% of it is unavoidable",
        100.0 * mean(&played) / mean(&total_levels),
        100.0 * mean(&required) / mean(&total_levels),
    );
}

/// **The K sweep** — what the wand gate is worth, and what it costs.
///
/// K is the mode's difficulty dial: the castle will not open until the player
/// holds K of the 7 wands, and a wand is an airship clear. Without it a pad
/// chain can drop the player next to the castle almost immediately — measured,
/// some seeds finish in ONE level. This is the table that says what each K buys.
///
/// ```sh
/// CENSUS_SEEDS=40 cargo test --release --lib maze_wand_gate_sweep \
///     -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn maze_wand_gate_sweep() {
    let Some(raw) = load_rom() else { return };
    let seeds = census_seeds(30);
    let knobs = super::graph::Knobs::default();

    eprintln!("\n=== the wand gate, {seeds} seeds per K ===");
    eprintln!(
        "{:>2}  {:>7} {:>7} {:>7} {:>7}  {:>7} {:>7} {:>9}",
        "K", "levels", "min", "median", "max", "unwin", "nopath", "wand trips"
    );
    for k in 0..=7u8 {
        let mut played = Vec::new();
        let mut wand_trips = Vec::new();
        let mut unwinnable = 0usize;
        let mut no_path = 0usize;
        for seed in 0..seeds {
            let (state, report) = generated_k(&raw, seed, &knobs, k);
            if !report.spheres.solvable {
                unwinnable += 1;
                continue;
            }
            let cost = super::metrics::completion_cost(&state);
            if !cost.reached {
                no_path += 1;
                continue;
            }
            played.push(cost.content);
            wand_trips.push(cost.wand_detours);
        }
        played.sort_unstable();
        let mean = |v: &[usize]| v.iter().sum::<usize>() as f64 / v.len().max(1) as f64;
        eprintln!(
            "{k:>2}  {:>7.1} {:>7} {:>7} {:>7}  {:>7} {:>7} {:>9.1}",
            mean(&played),
            played.first().copied().unwrap_or(0),
            played.get(played.len() / 2).copied().unwrap_or(0),
            played.last().copied().unwrap_or(0),
            unwinnable,
            no_path,
            mean(&wand_trips),
        );
    }
    eprintln!("  levels = levels+forts a play-through beats, of 62 levels + 17 forts");
}

/// `the_maze_leaves_every_pipe_alone`, for a **generated** maze rather than a
/// hand-specified one.
///
/// The property the whole telepad choice rests on: a pad is a tile, so it
/// spends no transit room and no destination-table slot, and the four pipe
/// destination tables plus every world pointer table come out byte-identical
/// to what the overworld writer left. The existing test in `world_persist`
/// asserts this for a POC ROM; this asserts it for what the generator actually
/// emits, which is where a stray write would come from.
#[test]
fn a_generated_maze_writes_only_pad_tiles_and_free_space() {
    use crate::randomize::rom_data;

    let Some(raw) = load_rom() else { return };
    for seed in 0..census_seeds(3) {
        let (rom, result) = census_build(&raw, seed);
        let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0x5EED_1234);
        let (state, _) = super::generate(
            &result,
            &IDENTITY_SPINE,
            super::DEFAULT_WANDS_REQUIRED,
            &super::graph::Knobs::default(),
            &mut rng,
        );

        let mut after = rom.clone();
        super::writer::stamp_pad_tiles(&mut after, &state);
        crate::randomize::world_persist::apply(&mut after, &super::writer::telepad_specs(&state));

        // Every byte the maze changed inside the map grids must be a pad tile
        // it meant to stamp. Anything else is a bug that a playtest would find
        // as a hole in the map.
        let pads: std::collections::HashSet<_> =
            state.pad_edges().into_iter().map(|(f, _)| f).collect();
        for wi in 0..8 {
            let grid = rom_data::read_tile_grid(&rom, wi);
            for r in 0..grid.rows() {
                for c in 0..grid.cols {
                    let off = rom_data::map_tile_offset(wi, r, c);
                    if rom.read_byte(off) == after.read_byte(off) {
                        continue;
                    }
                    assert!(
                        pads.contains(&(wi, (r, c))),
                        "seed {seed}: the maze changed W{} ({r},{c}) and no pad stands there",
                        wi + 1
                    );
                    assert_eq!(
                        after.read_byte(off),
                        rom_data::TILE_BONUS_GAME,
                        "seed {seed}: pad at W{} ({r},{c}) is not a spade panel",
                        wi + 1
                    );
                }
            }
        }
    }
}

/// A short spine — `world_count < 7` — still makes a winnable maze, and the
/// worlds it leaves out are reachable only by pad.
///
/// This is `world_count` keeping its meaning under the mode rather than being
/// forced to 7. The off-spine worlds have no airship edge into them, so the
/// only way in is a telepad; the guarantee that has to survive is that nothing
/// required is stranded behind one.
#[test]
fn a_short_spine_still_finishes() {
    let Some(raw) = load_rom() else { return };
    for seed in 0..census_seeds(4) {
        let (_, result) = census_build(&raw, seed);
        // World order's own shape: a shuffled prefix, Dark Land always last.
        for count in [3usize, 5] {
            let mut pool: Vec<usize> = (0..7).collect();
            let mut rng = ChaCha8Rng::seed_from_u64(seed ^ count as u64);
            pool.shuffle(&mut rng);
            let mut spine: Vec<usize> = pool[..count].to_vec();
            spine.push(7);

            // K cannot exceed the airships the spine actually offers.
            let k = (super::DEFAULT_WANDS_REQUIRED as usize).min(count) as u8;
            let (state, report) =
                super::generate(&result, &spine, k, &super::graph::Knobs::default(), &mut rng);
            assert!(
                report.spheres.solvable,
                "seed {seed} spine {spine:?}: unwinnable\n{}",
                report.spheres.spoiler()
            );
            assert!(
                report.unsafe_worlds.is_empty(),
                "seed {seed} spine {spine:?}: worlds that can strand a player: {:?}",
                report.unsafe_worlds
            );
            let cost = super::metrics::completion_cost(&state);
            assert!(cost.reached, "seed {seed} spine {spine:?}: no completion path");
        }
    }
}

/// The foreign-lock rows the ROM side is handed describe locks the maze
/// actually made foreign, and nothing else.
///
/// The failure this guards is silent and expensive: a row for a SAME-world lock
/// would set its completion bit twice — once through the fortress FX path the
/// engine already runs, once through the foreign-lock hook — and a row naming
/// the wrong fortress would bust a lock the player never earned.
#[test]
fn foreign_lock_rows_match_the_assignment() {
    let Some(raw) = load_rom() else { return };
    for seed in 0..census_seeds(4) {
        let (_, result) = census_build(&raw, seed);
        let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0x5EED_1234);
        let (state, _) = super::generate(
            &result,
            &IDENTITY_SPINE,
            super::DEFAULT_WANDS_REQUIRED,
            &super::graph::Knobs::default(),
            &mut rng,
        );
        let rows = super::writer::foreign_locks(&state);

        assert_eq!(
            rows.len(),
            state.locks.iter().filter(|l| l.is_foreign()).count(),
            "seed {seed}: a foreign lock was dropped on the way to the ROM"
        );
        for row in &rows {
            assert_ne!(
                row.fort_world, row.lock_world,
                "seed {seed}: a same-world lock reached the foreign-lock table"
            );
            // The named cell really holds that fortress, and the lock really
            // sits where the row says.
            let fort = state.worlds[row.fort_world]
                .slots
                .iter()
                .find(|s| s.pos == row.fort_pos)
                .expect("fort row names a cell with no slot");
            assert_eq!(fort.kind, SlotKind::Fortress, "seed {seed}: fort row names a non-fortress");
            let lock = state
                .locks
                .iter()
                .find(|l| l.world == row.lock_world && l.pos == row.lock_pos)
                .expect("lock row names a cell with no lock");
            assert_eq!(
                lock.fort.map(|f| (f.world, f.section)),
                Some((row.fort_world, fort.section)),
                "seed {seed}: the row pairs a lock with a fortress that does not open it"
            );
        }
    }
}

/// **The pads must not overflow the packed completion store.**
///
/// A pad tile is a spade panel (`TILE_BONUS_GAME`), and that tile is in the
/// engine's `Map_Completable_Tiles` — so every pad the generator stamps adds a
/// cell to the stencil and a bit to both planes. `PLANE_RESERVE` is 48 bytes
/// against a measured worst case of 42, so there are six bytes of margin and 16
/// pads could claim two of them.
///
/// The failure mode if this ever stops holding is not a crash: the planes would
/// silently run past their reserve into the arrival variables that live
/// immediately after them, and the symptom would be a corrupted teleport
/// several worlds later. That is why this is an assert and not a census.
#[test]
fn the_pads_still_fit_the_packed_store() {
    use crate::randomize::completion_bits::{CompletionMap, PLANE_RESERVE};

    let Some(raw) = load_rom() else { return };
    let mut worst = 0usize;
    let mut worst_seed = 0;
    for seed in 0..census_seeds(12) {
        let (rom, result) = census_build(&raw, seed);
        let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0x5EED_1234);
        // The knobs that place the most pads: every id spent, all crossing.
        let (state, _) = super::generate(
            &result,
            &IDENTITY_SPINE,
            super::DEFAULT_WANDS_REQUIRED,
            &super::graph::Knobs { foreign_landing_bias: 1.0, fort_distance_bias: 1.0 },
            &mut rng,
        );
        let mut after = rom.clone();
        super::writer::stamp_pad_tiles(&mut after, &state);
        let used = CompletionMap::from_rom(&after).mirror_offset();
        if used > worst {
            worst = used;
            worst_seed = seed;
        }
        assert!(
            used <= PLANE_RESERVE,
            "seed {seed}: {} pads pushed the packed plane to {used} bytes, past PLANE_RESERVE {PLANE_RESERVE}",
            state.pad_edges().len()
        );
    }
    eprintln!("  worst packed plane with pads: {worst} of {PLANE_RESERVE} (seed {worst_seed})");
}
