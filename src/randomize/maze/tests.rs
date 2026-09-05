//! The maze verifier's own tests, and the null-model census.
//!
//! The census is the point of build-order step 1: it produces the baselines
//! ("what happens if we connect eight worlds and change nothing else") that
//! every later lever has to be argued as a delta against. Nothing here has a
//! target number yet — that is the whole reason it exists.

use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

use super::pads::{PAD_BUDGET, place_pads_uniform};
use super::{GlobalState, IDENTITY_SPINE, MazeEdge};
use crate::randomize::map_walker::{MazeWorld, walk_maze, walk_reachable};
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

/// Invariant 3, measured rather than asserted: it has no placement phase yet,
/// so this pins how often the terrain satisfies it by accident. A rate of 100%
/// would make the start-region rule a cheap guard; anything less makes it a
/// real placement obligation.
#[test]
fn start_region_exit_rate() {
    let Some(raw) = load_rom() else { return };
    let seeds = census_seeds(8);
    let mut hits = 0usize;
    let mut total = 0usize;
    let mut failing = Vec::new();
    for seed in 0..seeds {
        let state = null_model(&raw, seed);
        for wi in 0..8 {
            total += 1;
            if state.start_region_has_exit(wi) {
                hits += 1;
            } else {
                failing.push(format!("seed {seed} W{}", wi + 1));
            }
        }
    }
    eprintln!(
        "invariant 3 (start region has an ungated exit): {hits}/{total} = {:.1}%",
        100.0 * hits as f64 / total as f64
    );
    if !failing.is_empty() {
        eprintln!("  failures: {}", failing.join(", "));
    }
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
        for w in &state.worlds {
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
