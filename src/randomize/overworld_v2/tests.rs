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
        level_budget: 0,
        fort_budget: 0,
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
        let state = from_vanilla(&rom, &catalog, world_idx);
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
            let mut state = from_pickup(&rom, &catalog, &pickup, world_idx);
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
    let mut state = from_pickup(&rom, &catalog, &pickup, 2);
    let mut rng = ChaCha8Rng::seed_from_u64(0);
    run_schedule(&mut state, &[&Connectivity], &mut rng);
    println!("example (W3, seed 0):");
    for action in &state.log[0].actions {
        println!("    {action}");
    }
}

/// Levels discovery census: connectivity + uniform-random level placement,
/// measured for the numbers any future placement preference must beat —
/// budget shortfalls, clustering (adjacent-level rate, nearest-neighbor
/// distance), screen crowding, and what the route scorer sees in a world
/// of pure levels (no forts/locks yet). `V2_SEEDS` seeds (default 50).
#[test]
fn test_v2_levels_census() {
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

    println!("v2 levels census ({seeds} seeds, uniform placement after connectivity)");
    println!("world  budget  placed  short%  adj%  nn-dist  maxscreen%  C1(mean)  routes  linear%");
    for world_idx in 0..8 {
        let mut placed_sum = 0usize;
        let mut short = 0usize;
        let mut adj_levels = 0usize;
        let mut level_count = 0usize;
        let mut nn_sum = 0f64;
        let mut maxscreen_sum = 0f64;
        let mut c1_sum = 0u64;
        let mut routes_sum = 0usize;
        let mut linear = 0usize;
        let mut budget = 0usize;

        for seed in 0..seeds {
            let mut state = from_pickup(&rom, &catalog, &pickup, world_idx);
            budget = state.level_budget;
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            run_schedule(&mut state, &[&Connectivity, &Levels], &mut rng);

            let levels: Vec<Pos> = state
                .slots
                .iter()
                .filter(|s| s.kind == SlotKind::Level)
                .map(|s| s.pos)
                .collect();
            placed_sum += levels.len();
            if levels.len() < state.level_budget {
                short += 1;
            }
            for &a in &levels {
                let nearest = levels
                    .iter()
                    .filter(|&&b| b != a)
                    .map(|&b| a.0.abs_diff(b.0) + a.1.abs_diff(b.1))
                    .min();
                if let Some(d) = nearest {
                    nn_sum += d as f64;
                    // Map nodes sit two tiles apart, so distance 2 on one
                    // axis is "the very next node".
                    if d == 2 {
                        adj_levels += 1;
                    }
                }
                level_count += 1;
            }
            let screens = state.grid.cols.div_ceil(16);
            let mut per_screen = vec![0usize; screens];
            for &(_, c) in &levels {
                per_screen[c / 16] += 1;
            }
            if !levels.is_empty() {
                maxscreen_sum +=
                    *per_screen.iter().max().unwrap() as f64 / levels.len() as f64;
            }

            let measure = measure_world(&state);
            c1_sum += u64::from(measure.c1);
            routes_sum += measure.routes_in_band;
            if measure.routes_in_band < 2 {
                linear += 1;
            }
        }

        let n = seeds as f64;
        println!(
            "  W{}   {:>5} {:>7.2} {:>6.0}% {:>4.0}% {:>8.2} {:>10.0}% {:>9.1} {:>7.2} {:>7.0}%",
            world_idx + 1,
            budget,
            placed_sum as f64 / n,
            100.0 * short as f64 / n,
            100.0 * adj_levels as f64 / level_count.max(1) as f64,
            nn_sum / level_count.max(1) as f64,
            100.0 * maxscreen_sum / n,
            c1_sum as f64 / n,
            routes_sum as f64 / n,
            100.0 * linear as f64 / n,
        );
    }

    // One narrated example for hand-checking: W5, seed 0.
    let mut state = from_pickup(&rom, &catalog, &pickup, 4);
    let mut rng = ChaCha8Rng::seed_from_u64(0);
    run_schedule(&mut state, &[&Connectivity, &Levels], &mut rng);
    println!("example (W5, seed 0):");
    for report in &state.log {
        for action in &report.actions {
            println!("    [{}] {action}", report.phase);
        }
    }
}

/// Forts discovery census: the full dumb pipeline so far (connectivity →
/// levels → spare pipes → forts). The headline number is on-route%: how
/// often a random fort lands on the cheapest route — where the player pays
/// its 5 points no matter what, and its future lock gates nothing (the
/// "on-path fort = decorative lock" thesis, baselined). `V2_SEEDS` seeds
/// (default 50).
#[test]
fn test_v2_forts_census() {
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

    println!("v2 forts census ({seeds} seeds, uniform placement, full dumb pipeline)");
    println!("world  budget  placed  on-route%  C1(mean)  routes  linear%");
    for world_idx in 0..8 {
        let mut placed_sum = 0usize;
        let mut on_route = 0usize;
        let mut fort_count = 0usize;
        let mut c1_sum = 0u64;
        let mut routes_sum = 0usize;
        let mut linear = 0usize;
        let mut budget = 0usize;

        for seed in 0..seeds {
            let mut state = from_pickup(&rom, &catalog, &pickup, world_idx);
            budget = state.fort_budget;
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            run_schedule(&mut state, &[&Connectivity, &Levels, &SparePipes, &Forts], &mut rng);

            let forts: Vec<Pos> = state
                .slots
                .iter()
                .filter(|s| s.kind == SlotKind::Fortress)
                .map(|s| s.pos)
                .collect();
            placed_sum += forts.len();

            let measure = measure_world(&state);
            if let Some(cheap) = measure.rc.routes.first() {
                let path: HashSet<Pos> = cheap.path.iter().copied().collect();
                on_route += forts.iter().filter(|p| path.contains(p)).count();
            }
            fort_count += forts.len();
            c1_sum += u64::from(measure.c1);
            routes_sum += measure.routes_in_band;
            if measure.routes_in_band < 2 {
                linear += 1;
            }
        }

        let n = seeds as f64;
        println!(
            "  W{}   {:>5} {:>7.2} {:>9.0}% {:>9.1} {:>7.2} {:>7.0}%",
            world_idx + 1,
            budget,
            placed_sum as f64 / n,
            100.0 * on_route as f64 / fort_count.max(1) as f64,
            c1_sum as f64 / n,
            routes_sum as f64 / n,
            100.0 * linear as f64 / n,
        );
    }
}

/// Full-skeleton census: all five dumb phases (connectivity → levels →
/// spare pipes → forts → locks). The skeleton's report card, measured with
/// the same stick as the shipping builder — compare against
/// `test_v2_current_builder_worlds`. New columns: lockless% (forts whose
/// every lock candidate broke completability), goal-open% (world finishable
/// with all locks closed), zero-gate% (locks that wall off nothing — pure
/// decoration). `V2_SEEDS` seeds (default 50).
#[test]
fn test_v2_locks_census() {
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

    println!("v2 full-skeleton census ({seeds} seeds, all five dumb phases)");
    println!("world  forts  locks  lockless%  goal-open%  zero-gate%  C1(mean)  routes  linear%");
    for world_idx in 0..8 {
        let mut fort_sum = 0usize;
        let mut lock_sum = 0usize;
        let mut zero_gate = 0usize;
        let mut goal_open_count = 0usize;
        let mut c1_sum = 0u64;
        let mut routes_sum = 0usize;
        let mut linear = 0usize;

        for seed in 0..seeds {
            let mut state = from_pickup(&rom, &catalog, &pickup, world_idx);
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            run_schedule(
                &mut state,
                &[&Connectivity, &Levels, &SparePipes, &Forts, &Locks],
                &mut rng,
            );
            assert!(
                state.completable(),
                "seed {} W{} must be completable after locks",
                seed,
                world_idx + 1
            );

            fort_sum += state
                .slots
                .iter()
                .filter(|s| s.kind == SlotKind::Fortress)
                .count();
            lock_sum += state.locks.len();

            // Zero-gate locks: with all OTHER locks open, closing this one
            // walls off no node at all — pure decoration.
            let mut open_grid = state.grid.clone();
            super::super::overworld_build::stamp_slots(&mut open_grid, &state.slots);
            for lock in &state.locks {
                open_grid.set(lock.pos.0, lock.pos.1, lock.replace_tile);
            }
            let open_reach =
                walk_reachable(&open_grid, &state.pipe_pairs, state.start, state.world_idx);
            for lock in &state.locks {
                let mut g = open_grid.clone();
                g.set(lock.pos.0, lock.pos.1, lock.gap_tile);
                let reach = walk_reachable(&g, &state.pipe_pairs, state.start, state.world_idx);
                if reach.len() == open_reach.len() {
                    zero_gate += 1;
                }
            }

            let measure = measure_world(&state);
            if measure.goal_open {
                goal_open_count += 1;
            }
            c1_sum += u64::from(measure.c1);
            routes_sum += measure.routes_in_band;
            if measure.routes_in_band < 2 {
                linear += 1;
            }
        }

        let n = seeds as f64;
        println!(
            "  W{}   {:>4.1} {:>6.1} {:>9.0}% {:>10.0}% {:>10.0}% {:>9.1} {:>7.2} {:>7.0}%",
            world_idx + 1,
            fort_sum as f64 / n,
            lock_sum as f64 / n,
            100.0 * (fort_sum - lock_sum) as f64 / fort_sum.max(1) as f64,
            100.0 * goal_open_count as f64 / n,
            100.0 * zero_gate as f64 / lock_sum.max(1) as f64,
            c1_sum as f64 / n,
            routes_sum as f64 / n,
            100.0 * linear as f64 / n,
        );
    }

    // One narrated example: W4, seed 0 — a two-fort world end to end.
    let mut state = from_pickup(&rom, &catalog, &pickup, 3);
    let mut rng = ChaCha8Rng::seed_from_u64(0);
    run_schedule(
        &mut state,
        &[&Connectivity, &Levels, &SparePipes, &Forts, &Locks],
        &mut rng,
    );
    println!("example (W4, seed 0):");
    for report in &state.log {
        for action in &report.actions {
            println!("    [{}] {action}", report.phase);
        }
    }
}

/// One logged spare-pipe delta, parsed back out of the report line
/// `pipe (r, c) <-> (r, c): routes A -> B, C1 M -> N`.
fn parse_pipe_delta(line: &str) -> Option<(usize, usize, u32, u32)> {
    let rest = line.split(": routes ").nth(1)?;
    let (routes, c1) = rest.split_once(", C1 ")?;
    let (ra, rb) = routes.split_once(" -> ")?;
    let (ca, cb) = c1.split_once(" -> ")?;
    Some((ra.parse().ok()?, rb.parse().ok()?, ca.parse().ok()?, cb.parse().ok()?))
}

/// Spare-pipes discovery census: what do RANDOM pipes do to route structure?
/// Buckets per placed pipe, from the phase's own delta instrumentation:
/// create (routes up), destroy (routes down), cheapen (routes flat, C1 down
/// — the silent dominating shortcut), inert (nothing measurable changed).
/// `V2_SEEDS` seeds (default 50).
#[test]
fn test_v2_spare_pipes_census() {
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

    println!("v2 spare-pipes census ({seeds} seeds, random toss, observe-only deltas)");
    println!("world  spares  create%  destroy%  cheapen%  inert%  C1(mean)  routes  linear%");
    for world_idx in 0..8 {
        let mut spares = 0usize;
        let (mut create, mut destroy, mut cheapen, mut inert) = (0usize, 0usize, 0usize, 0usize);
        let mut c1_sum = 0u64;
        let mut routes_sum = 0usize;
        let mut linear = 0usize;

        for seed in 0..seeds {
            let mut state = from_pickup(&rom, &catalog, &pickup, world_idx);
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            run_schedule(&mut state, &[&Connectivity, &Levels, &SparePipes], &mut rng);

            let spare_report = state
                .log
                .iter()
                .find(|r| r.phase == "spare_pipes")
                .expect("spare_pipes always reports");
            for line in &spare_report.actions {
                let Some((ra, rb, ca, cb)) = parse_pipe_delta(line) else { continue };
                spares += 1;
                if rb > ra {
                    create += 1;
                } else if rb < ra {
                    destroy += 1;
                } else if cb < ca {
                    cheapen += 1;
                } else {
                    inert += 1;
                }
            }

            let measure = measure_world(&state);
            c1_sum += u64::from(measure.c1);
            routes_sum += measure.routes_in_band;
            if measure.routes_in_band < 2 {
                linear += 1;
            }
        }

        let n = seeds as f64;
        let per = |x: usize| 100.0 * x as f64 / spares.max(1) as f64;
        println!(
            "  W{}   {:>6.2} {:>7.0}% {:>8.0}% {:>8.0}% {:>6.0}% {:>9.1} {:>7.2} {:>7.0}%",
            world_idx + 1,
            spares as f64 / n,
            per(create),
            per(destroy),
            per(cheapen),
            per(inert),
            c1_sum as f64 / n,
            routes_sum as f64 / n,
            100.0 * linear as f64 / n,
        );
    }

    // One narrated example: W2 (whose single spare IS its whole pipe story).
    let mut state = from_pickup(&rom, &catalog, &pickup, 1);
    let mut rng = ChaCha8Rng::seed_from_u64(0);
    run_schedule(&mut state, &[&Connectivity, &Levels, &SparePipes], &mut rng);
    println!("example (W2, seed 0):");
    for report in &state.log {
        for action in &report.actions {
            println!("    [{}] {action}", report.phase);
        }
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
    let state = from_vanilla(&rom, &catalog, world - 1);
    let rc = analyze_route_choice(&state.to_built(), slack);
    println!("vanilla W{world} wide band (slack {slack}): best={}", rc.best_cost);
    for r in &rc.routes {
        println!("  kept route: cost={:2} levels={} forts={} rocks={}", r.cost, r.levels.len(), r.forts, r.rocks.len());
    }
    for d in &rc.detours {
        println!("  dominated:  cost={:2} levels={} forts={} rocks={}", d.cost, d.levels.len(), d.forts, d.rocks.len());
    }
}
