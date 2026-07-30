//! v2 harness tests: the schedule contract, and the two measurement demos
//! (vanilla ground truth, current-builder baseline). Table output prints with
//! `cargo test overworld_v2 -- --nocapture`.

use super::*;

use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

use super::super::overworld_build::{BuildFlags, OverworldData, build};
use super::super::overworld_pickup::{PickupFlags, pick_up};
use super::super::qol;

fn load_rom() -> Option<Rom> {
    let bytes = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
    Rom::from_bytes(&bytes).ok()
}

/// The always-on map QOL edits the shipping builder assumes (the census's
/// "base" arm): W3 drawbridges fixed, vanilla rocks removed, W8 bridges in,
/// big ? block rooms fixed.
fn base_qol(rom: &Rom) -> Rom {
    let mut out = rom.clone();
    qol::fix_w3_drawbridges(&mut out);
    qol::remove_rocks(&mut out);
    qol::apply_w8_bridges(&mut out);
    qol::fix_big_q_block_rooms(&mut out);
    out
}

/// A phase that only records that it ran — the schedule contract probe.
struct Recorder(&'static str);

impl Phase for Recorder {
    fn name(&self) -> &'static str {
        self.0
    }
    fn run(&self, state: &mut WorldState, _rng: &mut dyn RngCore) -> PhaseReport {
        PhaseReport {
            phase: self.0,
            actions: vec![format!("ran on world {}", state.world_idx)],
        }
    }
}

/// The schedule is a plain list: phases run in the order given and their
/// reports land on the state's log in that same order.
#[test]
fn test_v2_schedule_runs_phases_in_order() {
    let mut state = WorldState {
        world_idx: 0,
        grid: Grid { tiles: vec![vec![0x00; 4]; 9], cols: 4, eights_are_wild: false },
        slots: Vec::new(),
        locks: Vec::new(),
        pipe_pairs: Vec::new(),
        start: None,
        target: None,
        fixed: HashSet::new(),
        pipe_budget: 0,
        log: Vec::new(),
    };
    let first = Recorder("first");
    let second = Recorder("second");
    assert_eq!(first.name(), "first");
    let mut rng = ChaCha8Rng::seed_from_u64(0);
    run_schedule(&mut state, &[&first, &second], &mut rng);
    let ran: Vec<&str> = state.log.iter().map(|r| r.phase).collect();
    assert_eq!(ran, ["first", "second"]);
    assert_eq!(state.log[0].actions, ["ran on world 0"]);
}

/// Measure the eight VANILLA worlds — known ground truth for calibrating the
/// metrics themselves. Raw ROM, no QOL: this is the game as shipped.
#[test]
fn test_v2_vanilla_worlds() {
    let Some(rom) = load_rom() else {
        eprintln!("ROM not found, skipping");
        return;
    };
    let catalog = NodeCatalog::build(&rom, false);

    println!("vanilla worlds (raw ROM, scorer points: level 3 / fort 5 / pipe 1 / rock 8)");
    println!("world  levels forts pipes locks |  C1  routes  uniq  detours  goal-open");
    for world_idx in 0..8 {
        let state = vanilla_world(&rom, &catalog, world_idx);
        let measure = measure_world(&state);
        let count = |kind: SlotKind| state.slots.iter().filter(|s| s.kind == kind).count();
        println!(
            "  W{}   {:>5} {:>5} {:>5} {:>5} | {:>3} {:>6} {:>5.1} {:>8}  {}",
            world_idx + 1,
            count(SlotKind::Level),
            count(SlotKind::Fortress),
            state.pipe_pairs.len(),
            state.locks.len(),
            measure.c1,
            measure.routes_in_band,
            measure.mean_exclusive_levels,
            measure.dominated_detours,
            if measure.goal_open { "OPEN" } else { "gated" },
        );
        for route in measure.rc.routes.iter().take(5) {
            println!(
                "         route: cost={:>2}  levels={:>2}  forts={}  rocks={}",
                route.cost,
                route.levels.len(),
                route.forts,
                route.rocks.len(),
            );
        }
        if measure.rc.routes.len() > 5 {
            println!("         ... {} more routes", measure.rc.routes.len() - 5);
        }
        assert!(
            measure.reachable,
            "vanilla W{} goal must be reachable",
            world_idx + 1
        );
    }
}

/// Measure current-builder output through the v2 harness — the baseline
/// table v2 phases will be compared against. Seeds via `V2_SEEDS` (default
/// 20). Base QOL arm, no start↔airship swap: the simplest production-like
/// configuration.
#[test]
fn test_v2_current_builder_worlds() {
    let Some(rom) = load_rom() else {
        eprintln!("ROM not found, skipping");
        return;
    };
    let rom = base_qol(&rom);
    let seeds: u64 = std::env::var("V2_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);

    let catalog = NodeCatalog::build(&rom, false);
    let pickup = pick_up(
        &rom,
        &catalog,
        PickupFlags { shuffle_spade_games: true, ..Default::default() },
    );

    let mut linear_per_world = [0usize; 8];
    let mut c1_min = u32::MAX;
    let mut c1_sum: u64 = 0;
    let mut route_sum: u64 = 0;
    let mut goal_open_worlds = 0usize;
    let mut world_count = 0usize;

    for seed in 0..seeds {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let result = build(
            &rom,
            &OverworldData { pickup: &pickup, catalog: &catalog },
            &mut rng,
            BuildFlags { shuffle_toad_houses: true, ..Default::default() },
        );
        for built in &result.worlds {
            let state = from_built(built);
            let measure = measure_world(&state);
            assert!(
                measure.reachable,
                "seed {} W{} goal must be reachable",
                seed,
                built.world_idx + 1
            );
            if measure.routes_in_band < 2 {
                linear_per_world[built.world_idx] += 1;
            }
            c1_min = c1_min.min(measure.c1);
            c1_sum += u64::from(measure.c1);
            route_sum += measure.routes_in_band as u64;
            if measure.goal_open {
                goal_open_worlds += 1;
            }
            world_count += 1;
        }
    }

    println!("current builder through the v2 harness ({seeds} seeds, base arm)");
    println!(
        "  linear per world: {}",
        (0..8)
            .map(|w| format!("W{} {}/{}", w + 1, linear_per_world[w], seeds))
            .collect::<Vec<_>>()
            .join("  ")
    );
    let linear_total: usize = linear_per_world.iter().sum();
    println!(
        "  overall: linear {}/{} ({:.1}%)  mean routes {:.2}  C1 min {}  C1 mean {:.1}  goal-open {}",
        linear_total,
        world_count,
        100.0 * linear_total as f64 / world_count as f64,
        route_sum as f64 / world_count as f64,
        c1_min,
        c1_sum as f64 / world_count as f64,
        goal_open_worlds,
    );
}

/// Connectivity discovery census: run ONLY the connectivity phase on the
/// pickup-cleared worlds and measure what it does — pipes spent vs the
/// vanilla budget, blanks left stranded, goal reachability, and endpoint
/// variety across seeds. `V2_SEEDS` sets the seed count (default 50).
///
/// No assertions on coverage yet: this is the rediscovery baseline the
/// controls will be justified against.
#[test]
fn test_v2_connectivity_census() {
    let Some(rom) = load_rom() else {
        eprintln!("ROM not found, skipping");
        return;
    };
    let rom = base_qol(&rom);
    let seeds: u64 = std::env::var("V2_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50);
    let catalog = NodeCatalog::build(&rom, false);
    let pickup = pick_up(&rom, &catalog, PickupFlags::default());

    println!("v2 connectivity census ({seeds} seeds, knob-free uniform endpoints)");
    println!("world  budget  pipes(mean)  stranded(mean)  goal-ok%  distinct-pairsets");
    for world_idx in 0..8 {
        let mut pipes_sum = 0usize;
        let mut stranded_sum = 0usize;
        let mut goal_ok = 0usize;
        let mut pair_sets: HashSet<Vec<TeleportEdge>> = HashSet::new();
        for seed in 0..seeds {
            let mut state = pickup_world(&rom, &catalog, &pickup, world_idx);
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            run_schedule(&mut state, &[&Connectivity], &mut rng);
            pipes_sum += state.pipe_pairs.len();
            let mut pairs = state.pipe_pairs.clone();
            pairs.sort();
            pair_sets.insert(pairs);
            let last = state.log.last().expect("connectivity always reports");
            let done = last.actions.last().expect("has a done line");
            let stranded: usize = done
                .split("done: ")
                .nth(1)
                .and_then(|s| s.split(' ').next())
                .and_then(|s| s.parse().ok())
                .expect("done line starts with the stranded count");
            stranded_sum += stranded;
            if done.ends_with("true") {
                goal_ok += 1;
            }
        }
        println!(
            "  W{}   {:>5} {:>10.2} {:>14.2} {:>8.0}% {:>13}",
            world_idx + 1,
            VANILLA_PIPE_PAIRS[world_idx],
            pipes_sum as f64 / seeds as f64,
            stranded_sum as f64 / seeds as f64,
            100.0 * goal_ok as f64 / seeds as f64,
            pair_sets.len(),
        );
    }

    // One narrated example for hand-checking: W3, seed 0.
    let mut state = pickup_world(&rom, &catalog, &pickup, 2);
    let mut rng = ChaCha8Rng::seed_from_u64(0);
    run_schedule(&mut state, &[&Connectivity], &mut rng);
    println!("example (W3, seed 0):");
    for action in &state.log[0].actions {
        println!("    {action}");
    }
}

/// Hand-check probe: one vanilla world at a wide measuring band, kept and
/// dominated routes both printed. `V2_WORLD` picks the world (1-8, default
/// 2), `V2_SLACK` the band (default 12). Born from the W2 rock question —
/// "where did the non-rock route go?" — and kept for the next such question.
#[test]
fn test_v2_probe_vanilla_world() {
    let Some(rom) = load_rom() else {
        eprintln!("ROM not found, skipping");
        return;
    };
    let world: usize = std::env::var("V2_WORLD").ok().and_then(|s| s.parse().ok()).unwrap_or(2);
    let slack: u32 = std::env::var("V2_SLACK").ok().and_then(|s| s.parse().ok()).unwrap_or(12);
    let catalog = NodeCatalog::build(&rom, false);
    let state = vanilla_world(&rom, &catalog, world - 1);
    let rc = analyze_route_choice(&state.to_built(), slack);
    println!("vanilla W{world} wide band (slack {slack}): best={}", rc.best_cost);
    for r in &rc.routes {
        println!("  kept route: cost={:2} levels={} forts={} rocks={}", r.cost, r.levels.len(), r.forts, r.rocks.len());
    }
    for d in &rc.detours {
        println!("  dominated:  cost={:2} levels={} forts={} rocks={}", d.cost, d.levels.len(), d.forts, d.rocks.len());
    }
}
