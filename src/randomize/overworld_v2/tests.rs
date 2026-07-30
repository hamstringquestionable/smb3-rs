//! v2 harness tests: the schedule contract, and the two measurement demos
//! (vanilla ground truth, current-builder baseline). Table output prints with
//! `cargo test overworld_v2 -- --nocapture`.

use super::*;

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use super::super::overworld_build::{OverworldData, analyze_required_progression, build};
use super::super::overworld_pickup::{PickupFlags, PickupResult as Pickup, pick_up};
use super::super::qol;
use super::super::start_airship_swap;

fn load_rom() -> Option<Rom> {
    let bytes = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
    Rom::from_bytes(&bytes).ok()
}

/// The always-on map QOL edits the shipping builder assumes (the census's
/// "base" arm): W3 drawbridges fixed, vanilla rocks removed, W8 bridges in,
/// big ? block rooms fixed.
fn base_qol(rom: &Rom) -> Rom {
    qol_variant(rom, false, false)
}

/// Arm-parameterized map QOL: the always-on patches plus the optional
/// `more hammer rocks` / `8s are Wild` map edits, in production order
/// (`randomize_inner` applies these before the builder reads the map).
fn qol_variant(rom: &Rom, hammer_rocks: bool, eights_wild: bool) -> Rom {
    let mut out = rom.clone();
    qol::fix_w3_drawbridges(&mut out);
    qol::remove_rocks(&mut out);
    if hammer_rocks {
        qol::make_hammer_rocks(&mut out);
    }
    qol::apply_w8_bridges(&mut out);
    if eights_wild {
        qol::apply_w8_canoe_and_paths(&mut out);
    }
    qol::fix_big_q_block_rooms(&mut out);
    out
}

/// One seed's realistic build input, mirroring the shipping census harness
/// (`overworld_build::tests::census_build`): always-on QOL plus the seed's
/// map arm (50% base / 25% more-hammer-rocks / 25% 8s-are-wild by seed % 4),
/// start↔airship swap rolled per world exactly as the real flag does (50/50
/// inside `pick_swaps`, so swapped and unswapped worlds are covered in every
/// arm), and toad-house / hammer-bro shuffle each rolled 50/50 per seed.
///
/// v2 has no toad-house / hammer-bro / spade placement phases yet — when a
/// shuffle rolls on, those slots are simply picked up and stay blank space.
/// Spade shuffle stays off until such a phase exists.
struct CensusCtx {
    rom: Rom,
    catalog: NodeCatalog,
    pickup: Pickup,
    flags: BuildFlags,
}

fn census_ctx(raw: &Rom, seed: u64) -> CensusCtx {
    let (hammer_rocks, eights_wild) = match seed % 4 {
        2 => (true, false),
        3 => (false, true),
        _ => (false, false),
    };
    let rom = qol_variant(raw, hammer_rocks, eights_wild);
    let mut catalog = NodeCatalog::build(&rom, false);
    let mut roll_rng = ChaCha8Rng::seed_from_u64(seed);
    start_airship_swap::pick_swaps(&mut catalog, &mut roll_rng);
    let shuffle_toad_houses = roll_rng.random_bool(0.5);
    let shuffle_hammer_bros = roll_rng.random_bool(0.5);
    let pickup = pick_up(
        &rom,
        &catalog,
        PickupFlags {
            shuffle_spade_games: false,
            shuffle_toad_houses,
            shuffle_hammer_bros,
        },
    );
    CensusCtx {
        rom,
        catalog,
        pickup,
        flags: BuildFlags {
            shuffle_toad_houses,
            eights_are_wild: eights_wild,
            shuffle_hammer_bros,
        },
    }
}

impl CensusCtx {
    fn world(&self, world_idx: usize) -> WorldState {
        from_pickup(&self.rom, &self.catalog, &self.pickup, world_idx, &self.flags)
    }
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
/// variety across seeds. Runs the realistic flag mix (see [`census_ctx`]).
/// `V2_SEEDS` sets the seed count (default 100).
///
/// No assertions on coverage yet: this is the rediscovery baseline the
/// controls will be justified against.
#[test]
fn test_v2_connectivity_census() {
    let Some(raw) = load_rom() else {
        eprintln!("ROM not found, skipping");
        return;
    };
    let seeds: u64 = std::env::var("V2_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);

    let mut pipes_sum = [0usize; 8];
    let mut stranded_sum = [0usize; 8];
    let mut goal_ok = [0usize; 8];
    let mut pair_sets: Vec<HashSet<Vec<TeleportEdge>>> = (0..8).map(|_| HashSet::new()).collect();
    for seed in 0..seeds {
        let ctx = census_ctx(&raw, seed);
        for world_idx in 0..8 {
            let mut state = ctx.world(world_idx);
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            run_schedule(&mut state, &[&Connectivity], &mut rng);
            pipes_sum[world_idx] += state.pipe_pairs.len();
            let mut pairs = state.pipe_pairs.clone();
            pairs.sort();
            pair_sets[world_idx].insert(pairs);
            let last = state.log.last().expect("connectivity always reports");
            let done = last.actions.last().expect("has a done line");
            let stranded: usize = done
                .split("done: ")
                .nth(1)
                .and_then(|s| s.split(' ').next())
                .and_then(|s| s.parse().ok())
                .expect("done line starts with the stranded count");
            stranded_sum[world_idx] += stranded;
            if done.ends_with("true") {
                goal_ok[world_idx] += 1;
            }
        }
    }

    println!("v2 connectivity census ({seeds} seeds, knob-free uniform endpoints, flag-mix arms)");
    println!("world  budget  pipes(mean)  stranded(mean)  goal-ok%  distinct-pairsets");
    for (world_idx, &budget) in VANILLA_PIPE_PAIRS.iter().enumerate() {
        println!(
            "  W{}   {:>5} {:>10.2} {:>14.2} {:>8.0}% {:>13}",
            world_idx + 1,
            budget,
            pipes_sum[world_idx] as f64 / seeds as f64,
            stranded_sum[world_idx] as f64 / seeds as f64,
            100.0 * goal_ok[world_idx] as f64 / seeds as f64,
            pair_sets[world_idx].len(),
        );
    }

    // One narrated example for hand-checking: W3, seed 0.
    let mut state = census_ctx(&raw, 0).world(2);
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
/// of pure levels (no forts/locks yet). Runs the realistic flag mix (see
/// [`census_ctx`]). `V2_SEEDS` seeds (default 100).
#[test]
fn test_v2_levels_census() {
    let Some(raw) = load_rom() else {
        eprintln!("ROM not found, skipping");
        return;
    };
    let seeds: u64 = std::env::var("V2_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);

    let mut placed_sum = [0usize; 8];
    let mut short = [0usize; 8];
    let mut adj_levels = [0usize; 8];
    let mut level_count = [0usize; 8];
    let mut nn_sum = [0f64; 8];
    let mut maxscreen_sum = [0f64; 8];
    let mut c1_sum = [0u64; 8];
    let mut routes_sum = [0usize; 8];
    let mut linear = [0usize; 8];
    let mut budget = [0usize; 8];

    for seed in 0..seeds {
        let ctx = census_ctx(&raw, seed);
        for world_idx in 0..8 {
            let mut state = ctx.world(world_idx);
            budget[world_idx] = state.level_budget;
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            run_schedule(&mut state, &[&Connectivity, &Levels], &mut rng);

            let levels: Vec<Pos> = state
                .slots
                .iter()
                .filter(|s| s.kind == SlotKind::Level)
                .map(|s| s.pos)
                .collect();
            placed_sum[world_idx] += levels.len();
            if levels.len() < state.level_budget {
                short[world_idx] += 1;
            }
            for &a in &levels {
                let nearest = levels
                    .iter()
                    .filter(|&&b| b != a)
                    .map(|&b| a.0.abs_diff(b.0) + a.1.abs_diff(b.1))
                    .min();
                if let Some(d) = nearest {
                    nn_sum[world_idx] += d as f64;
                    // Map nodes sit two tiles apart, so distance 2 on one
                    // axis is "the very next node".
                    if d == 2 {
                        adj_levels[world_idx] += 1;
                    }
                }
                level_count[world_idx] += 1;
            }
            let screens = state.grid.cols.div_ceil(16);
            let mut per_screen = vec![0usize; screens];
            for &(_, c) in &levels {
                per_screen[c / 16] += 1;
            }
            if !levels.is_empty() {
                maxscreen_sum[world_idx] +=
                    *per_screen.iter().max().unwrap() as f64 / levels.len() as f64;
            }

            let measure = measure_world(&state);
            c1_sum[world_idx] += u64::from(measure.c1);
            routes_sum[world_idx] += measure.routes_in_band;
            if measure.routes_in_band < 2 {
                linear[world_idx] += 1;
            }
        }
    }

    println!("v2 levels census ({seeds} seeds, uniform placement after connectivity, flag-mix arms)");
    println!("world  budget  placed  short%  adj%  nn-dist  maxscreen%  C1(mean)  routes  linear%");
    let n = seeds as f64;
    for world_idx in 0..8 {
        println!(
            "  W{}   {:>5} {:>7.2} {:>6.0}% {:>4.0}% {:>8.2} {:>10.0}% {:>9.1} {:>7.2} {:>7.0}%",
            world_idx + 1,
            budget[world_idx],
            placed_sum[world_idx] as f64 / n,
            100.0 * short[world_idx] as f64 / n,
            100.0 * adj_levels[world_idx] as f64 / level_count[world_idx].max(1) as f64,
            nn_sum[world_idx] / level_count[world_idx].max(1) as f64,
            100.0 * maxscreen_sum[world_idx] / n,
            c1_sum[world_idx] as f64 / n,
            routes_sum[world_idx] as f64 / n,
            100.0 * linear[world_idx] as f64 / n,
        );
    }

    // One narrated example for hand-checking: W5, seed 0.
    let mut state = census_ctx(&raw, 0).world(4);
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
/// "on-path fort = decorative lock" thesis, baselined). Runs the realistic
/// flag mix (see [`census_ctx`]). `V2_SEEDS` seeds (default 100).
#[test]
fn test_v2_forts_census() {
    let Some(raw) = load_rom() else {
        eprintln!("ROM not found, skipping");
        return;
    };
    let seeds: u64 = std::env::var("V2_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);

    let mut placed_sum = [0usize; 8];
    let mut on_route = [0usize; 8];
    let mut fort_count = [0usize; 8];
    let mut c1_sum = [0u64; 8];
    let mut routes_sum = [0usize; 8];
    let mut linear = [0usize; 8];
    let mut budget = [0usize; 8];

    for seed in 0..seeds {
        let ctx = census_ctx(&raw, seed);
        for world_idx in 0..8 {
            let mut state = ctx.world(world_idx);
            budget[world_idx] = state.fort_budget;
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            run_schedule(&mut state, &[&Connectivity, &Levels, &SparePipes, &Forts], &mut rng);

            let forts: Vec<Pos> = state
                .slots
                .iter()
                .filter(|s| s.kind == SlotKind::Fortress)
                .map(|s| s.pos)
                .collect();
            placed_sum[world_idx] += forts.len();

            let measure = measure_world(&state);
            if let Some(cheap) = measure.rc.routes.first() {
                let path: HashSet<Pos> = cheap.path.iter().copied().collect();
                on_route[world_idx] += forts.iter().filter(|p| path.contains(p)).count();
            }
            fort_count[world_idx] += forts.len();
            c1_sum[world_idx] += u64::from(measure.c1);
            routes_sum[world_idx] += measure.routes_in_band;
            if measure.routes_in_band < 2 {
                linear[world_idx] += 1;
            }
        }
    }

    println!("v2 forts census ({seeds} seeds, uniform placement, full dumb pipeline, flag-mix arms)");
    println!("world  budget  placed  on-route%  C1(mean)  routes  linear%");
    let n = seeds as f64;
    for world_idx in 0..8 {
        println!(
            "  W{}   {:>5} {:>7.2} {:>9.0}% {:>9.1} {:>7.2} {:>7.0}%",
            world_idx + 1,
            budget[world_idx],
            placed_sum[world_idx] as f64 / n,
            100.0 * on_route[world_idx] as f64 / fort_count[world_idx].max(1) as f64,
            c1_sum[world_idx] as f64 / n,
            routes_sum[world_idx] as f64 / n,
            100.0 * linear[world_idx] as f64 / n,
        );
    }
}

/// Full-skeleton census: all five dumb phases (connectivity → levels →
/// spare pipes → forts → locks). The skeleton's report card, measured with
/// the same stick as the shipping builder — compare against
/// `test_v2_current_builder_worlds`. Columns: removed% (forts removed by the
/// every-fort-locked invariant — expected ~0), safe% (locks that are
/// secret-exit-safe), goal-open% (world finishable with all locks closed),
/// zero-gate% (locks that wall off nothing — pure decoration). Runs the
/// realistic flag mix (see [`census_ctx`]) — the invariant asserts cover
/// every arm. `V2_SEEDS` seeds (default 100).
#[test]
fn test_v2_locks_census() {
    let Some(raw) = load_rom() else {
        eprintln!("ROM not found, skipping");
        return;
    };
    let seeds: u64 = std::env::var("V2_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);

    let mut fort_sum = [0usize; 8];
    let mut lock_sum = [0usize; 8];
    let mut safe_sum = [0usize; 8];
    let mut removed_sum = [0usize; 8];
    let mut zero_gate = [0usize; 8];
    let mut goal_open_count = [0usize; 8];
    let mut c1_sum = [0u64; 8];
    let mut routes_sum = [0usize; 8];
    let mut linear = [0usize; 8];

    for seed in 0..seeds {
        let ctx = census_ctx(&raw, seed);
        for world_idx in 0..8 {
            let mut state = ctx.world(world_idx);
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
            // Every-fort-locked invariant: unlockable forts were removed, so
            // the emitted world always pairs forts and locks 1:1.
            assert_eq!(
                state.locks.len(),
                state.fort_count(),
                "seed {} W{}: every fort must have a lock",
                seed,
                world_idx + 1
            );

            fort_sum[world_idx] += state.fort_count();
            lock_sum[world_idx] += state.locks.len();
            safe_sum[world_idx] += state.locks.iter().filter(|l| l.secret_exit_safe).count();
            removed_sum[world_idx] += state
                .log
                .iter()
                .flat_map(|r| &r.actions)
                .filter(|a| a.contains("REMOVED"))
                .count();

            zero_gate[world_idx] += state.zero_gate_locks().len();

            let measure = measure_world(&state);
            if measure.goal_open {
                goal_open_count[world_idx] += 1;
            }
            c1_sum[world_idx] += u64::from(measure.c1);
            routes_sum[world_idx] += measure.routes_in_band;
            if measure.routes_in_band < 2 {
                linear[world_idx] += 1;
            }
        }
    }

    println!("v2 full-skeleton census ({seeds} seeds, all five dumb phases, flag-mix arms)");
    println!("world  forts  locks  removed%  safe%  goal-open%  zero-gate%  C1(mean)  routes  linear%");
    let n = seeds as f64;
    for world_idx in 0..8 {
        println!(
            "  W{}   {:>4.1} {:>6.1} {:>8.0}% {:>5.0}% {:>10.0}% {:>10.0}% {:>9.1} {:>7.2} {:>7.0}%",
            world_idx + 1,
            fort_sum[world_idx] as f64 / n,
            lock_sum[world_idx] as f64 / n,
            100.0 * removed_sum[world_idx] as f64
                / (fort_sum[world_idx] + removed_sum[world_idx]).max(1) as f64,
            100.0 * safe_sum[world_idx] as f64 / lock_sum[world_idx].max(1) as f64,
            100.0 * goal_open_count[world_idx] as f64 / n,
            100.0 * zero_gate[world_idx] as f64 / lock_sum[world_idx].max(1) as f64,
            c1_sum[world_idx] as f64 / n,
            routes_sum[world_idx] as f64 / n,
            100.0 * linear[world_idx] as f64 / n,
        );
    }

    // One narrated example: W4, seed 0 — a two-fort world end to end.
    let mut state = census_ctx(&raw, 0).world(3);
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

/// Shaping A/B census: the dumb skeleton (arm `dumb`: connectivity → levels
/// → spare pipes → forts → locks) against the shaping loop (arm `shaped`:
/// connectivity → levels → forts → locks → shaping). SparePipes is
/// deliberately OMITTED from the shaped arm: shaping has first claim on the
/// pipe budget (pipes are a routing tool, not filler), and what it doesn't
/// spend shows up as a lower pipes column, not as random toss on top of
/// shaped structure.
///
/// Shaped-arm move columns: touched% = seeds where at least one move was
/// accepted (satisficing means most worlds should be untouched), lr/gs =
/// lock_replace / gated_shortcut accepts:rejects summed over all seeds. The
/// time line is the performance watchdog: mean wall-clock per seed (all 8
/// worlds) per arm. Runs the realistic flag mix (see [`census_ctx`]).
/// `V2_SEEDS` seeds (default 100).
#[test]
fn test_v2_shaping_census() {
    let Some(raw) = load_rom() else {
        eprintln!("ROM not found, skipping");
        return;
    };
    let seeds: u64 = std::env::var("V2_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);

    #[derive(Default, Clone, Copy)]
    struct ArmTally {
        c1: u64,
        routes: usize,
        linear: usize,
        zero_gate: usize,
        locks: usize,
        goal_open: usize,
        pipes: usize,
    }

    fn tally(t: &mut ArmTally, state: &WorldState) {
        let m = measure_world(state);
        t.c1 += u64::from(m.c1);
        t.routes += m.routes_in_band;
        if m.routes_in_band < 2 {
            t.linear += 1;
        }
        if m.goal_open {
            t.goal_open += 1;
        }
        t.zero_gate += state.zero_gate_locks().len();
        t.locks += state.locks.len();
        t.pipes += state.pipe_pairs.len();
    }

    let mut dumb = [ArmTally::default(); 8];
    let mut shaped = [ArmTally::default(); 8];
    let mut touched = [0usize; 8];
    let mut lr_acc = [0usize; 8];
    let mut lr_rej = [0usize; 8];
    let mut gs_acc = [0usize; 8];
    let mut gs_rej = [0usize; 8];
    let mut fl_acc = [0usize; 8];
    let mut fl_rej = [0usize; 8];
    let mut lm_acc = [0usize; 8];
    let mut lm_rej = [0usize; 8];
    let mut time_dumb = std::time::Duration::ZERO;
    let mut time_shaped = std::time::Duration::ZERO;

    for seed in 0..seeds {
        let ctx = census_ctx(&raw, seed);
        for world_idx in 0..8 {
            let mut state = ctx.world(world_idx);
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let t0 = std::time::Instant::now();
            run_schedule(
                &mut state,
                &[&Connectivity, &Levels, &SparePipes, &Forts, &Locks],
                &mut rng,
            );
            time_dumb += t0.elapsed();
            tally(&mut dumb[world_idx], &state);

            let mut state = ctx.world(world_idx);
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let t0 = std::time::Instant::now();
            run_schedule(
                &mut state,
                &[&Connectivity, &Levels, &Forts, &Locks, &Shaping],
                &mut rng,
            );
            time_shaped += t0.elapsed();
            assert!(
                state.completable(),
                "seed {seed} W{}: shaped world must stay completable",
                world_idx + 1
            );
            assert_eq!(
                state.locks.len(),
                state.fort_count(),
                "seed {seed} W{}: every fort must keep a lock through shaping",
                world_idx + 1
            );
            tally(&mut shaped[world_idx], &state);

            let report = state
                .log
                .iter()
                .find(|r| r.phase == "shaping")
                .expect("shaping always reports");
            let count =
                |needle: &str| report.actions.iter().filter(|a| a.contains(needle)).count();
            let la = count("lock_replace ACCEPT");
            let ga = count("gated_shortcut ACCEPT");
            let fa = count("fort_lock ACCEPT");
            let ma = count("level_move ACCEPT");
            lr_acc[world_idx] += la;
            lr_rej[world_idx] += count("lock_replace REJECT");
            gs_acc[world_idx] += ga;
            gs_rej[world_idx] += count("gated_shortcut REJECT");
            fl_acc[world_idx] += fa;
            fl_rej[world_idx] += count("fort_lock REJECT");
            lm_acc[world_idx] += ma;
            lm_rej[world_idx] += count("level_move REJECT");
            if la + ga + fa + ma > 0 {
                touched[world_idx] += 1;
            }
        }
    }

    println!("v2 shaping A/B census ({seeds} seeds, dumb skeleton vs diagnosis-driven shaping, flag-mix arms)");
    println!("world  arm     C1(mean)  routes  linear%  zero-gate%  goal-open%  pipes  touched%  lr(acc:rej)  gs(acc:rej)  fl(acc:rej)  lm(acc:rej)");
    let n = seeds as f64;
    for world_idx in 0..8 {
        for (arm, t) in [("dumb", &dumb[world_idx]), ("shaped", &shaped[world_idx])] {
            let base = format!(
                "  W{}   {arm:<7} {:>7.1} {:>7.2} {:>7.0}% {:>9.0}% {:>9.0}% {:>6.2}",
                world_idx + 1,
                t.c1 as f64 / n,
                t.routes as f64 / n,
                100.0 * t.linear as f64 / n,
                100.0 * t.zero_gate as f64 / t.locks.max(1) as f64,
                100.0 * t.goal_open as f64 / n,
                t.pipes as f64 / n,
            );
            if arm == "shaped" {
                println!(
                    "{base} {:>7.0}% {:>8}:{:<4} {:>6}:{:<4} {:>6}:{:<4} {:>6}:{:<4}",
                    100.0 * touched[world_idx] as f64 / n,
                    lr_acc[world_idx],
                    lr_rej[world_idx],
                    gs_acc[world_idx],
                    gs_rej[world_idx],
                    fl_acc[world_idx],
                    fl_rej[world_idx],
                    lm_acc[world_idx],
                    lm_rej[world_idx],
                );
            } else {
                println!("{base}        -        -         -         -         -");
            }
        }
    }
    println!(
        "time: dumb {:.1} ms/seed, shaped {:.1} ms/seed (8 worlds each)",
        time_dumb.as_secs_f64() * 1000.0 / n,
        time_shaped.as_secs_f64() * 1000.0 / n,
    );

    // One narrated example: W8, seed 0 — the most linear world under the
    // dumb pipeline, so shaping should have work to do.
    let mut state = census_ctx(&raw, 0).world(7);
    let mut rng = ChaCha8Rng::seed_from_u64(0);
    run_schedule(
        &mut state,
        &[&Connectivity, &Levels, &Forts, &Locks, &Shaping],
        &mut rng,
    );
    println!("example (W8, seed 0, shaped arm):");
    for report in &state.log {
        for action in &report.actions {
            println!("    [{}] {action}", report.phase);
        }
    }
}

/// Progression census — the OLDER linearity metrics (the required-progression
/// Dijkstra, a different model from the route-choice band): mandatory fort
/// and level counts (distinct clears the player is FORCED into), the longest
/// forced level streak, and the goal stack (forced levels sitting directly on
/// the airship/Bowser approach with nothing between them — the "clear path,
/// just 2+ levels on the goal" complaint; ≥2 is the historical badness
/// threshold). No-hammer analysis, matching the shipping progression census.
///
/// Arms per world: `vanilla` (ground truth, raw ROM, one row), `dumb`
/// (knob-free skeleton), `shaped` (shaping loop), `shipping` (current
/// builder) — the latter three all fed the same flag-mix input (see
/// [`census_ctx`]). `V2_SEEDS` seeds (default 100).
#[test]
fn test_v2_progression_census() {
    let Some(raw) = load_rom() else {
        eprintln!("ROM not found, skipping");
        return;
    };
    let seeds: u64 = std::env::var("V2_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);

    #[derive(Default, Clone, Copy)]
    struct ProgTally {
        forts: u64,
        levels: u64,
        streak: u64,
        streak_ge2: usize,
        goal: u64,
        goal_ge2: usize,
        n: usize,
    }

    fn tally(t: &mut ProgTally, built: &BuiltWorld) {
        let p = analyze_required_progression(built, false);
        assert!(p.reachable, "progression census world must be reachable");
        t.n += 1;
        t.forts += p.forts_required as u64;
        t.levels += p.levels_required as u64;
        t.streak += p.level_streak as u64;
        if p.level_streak >= 2 {
            t.streak_ge2 += 1;
        }
        t.goal += p.goal_stack as u64;
        if p.goal_stack >= 2 {
            t.goal_ge2 += 1;
        }
    }

    let mut dumb = [ProgTally::default(); 8];
    let mut shaped = [ProgTally::default(); 8];
    let mut shipping = [ProgTally::default(); 8];

    for seed in 0..seeds {
        let ctx = census_ctx(&raw, seed);

        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let result = build(
            &ctx.rom,
            &OverworldData { pickup: &ctx.pickup, catalog: &ctx.catalog },
            &mut rng,
            ctx.flags,
        );
        for built in &result.worlds {
            tally(&mut shipping[built.world_idx], built);
        }

        for world_idx in 0..8 {
            let mut state = ctx.world(world_idx);
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            run_schedule(
                &mut state,
                &[&Connectivity, &Levels, &SparePipes, &Forts, &Locks],
                &mut rng,
            );
            tally(&mut dumb[world_idx], &state.to_built());

            let mut state = ctx.world(world_idx);
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            run_schedule(
                &mut state,
                &[&Connectivity, &Levels, &Forts, &Locks, &Shaping],
                &mut rng,
            );
            tally(&mut shaped[world_idx], &state.to_built());
        }
    }

    // Vanilla ground truth: constant per world, measured once.
    let catalog = NodeCatalog::build(&raw, false);
    let mut vanilla = [ProgTally::default(); 8];
    for (world_idx, t) in vanilla.iter_mut().enumerate() {
        tally(t, &from_vanilla(&raw, &catalog, world_idx).to_built());
    }

    println!(
        "v2 progression census ({seeds} seeds, no-hammer required-progression, flag-mix arms)"
    );
    println!("world  arm      forts-req  lvls-req  streak  streak>=2%  goalstk  goalstk>=2%");
    for world_idx in 0..8 {
        for (arm, t) in [
            ("vanilla", &vanilla[world_idx]),
            ("dumb", &dumb[world_idx]),
            ("shaped", &shaped[world_idx]),
            ("shipping", &shipping[world_idx]),
        ] {
            let n = t.n.max(1) as f64;
            println!(
                "  W{}   {arm:<8} {:>8.2} {:>9.2} {:>7.2} {:>9.0}% {:>8.2} {:>10.0}%",
                world_idx + 1,
                t.forts as f64 / n,
                t.levels as f64 / n,
                t.streak as f64 / n,
                100.0 * t.streak_ge2 as f64 / n,
                t.goal as f64 / n,
                100.0 * t.goal_ge2 as f64 / n,
            );
        }
    }
}

/// Cross-world secret-exit-safety invariant: after all 8 worlds run the full
/// dumb schedule, at least one lock somewhere must be secret-exit-safe (the
/// writer parks the 1-F fortress level on it). Measures how often uniform
/// placement satisfies this naturally vs how often the relocation backstop
/// has to act. Runs the realistic flag mix (see [`census_ctx`]). `V2_SEEDS`
/// seeds (default 100 — each seed builds all 8 worlds).
#[test]
fn test_v2_secret_exit_safety() {
    let Some(raw) = load_rom() else {
        eprintln!("ROM not found, skipping");
        return;
    };
    let seeds: u64 = std::env::var("V2_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);

    let mut natural = 0usize;
    let mut relocated = 0usize;
    for seed in 0..seeds {
        let ctx = census_ctx(&raw, seed);
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut worlds: Vec<WorldState> = (0..8)
            .map(|world_idx| {
                let mut state = ctx.world(world_idx);
                run_schedule(
                    &mut state,
                    &[&Connectivity, &Levels, &SparePipes, &Forts, &Locks],
                    &mut rng,
                );
                state
            })
            .collect();

        let safe_before = worlds
            .iter()
            .any(|w| w.locks.iter().any(|l| l.secret_exit_safe));
        let report = ensure_secret_exit_safe(&mut worlds, &mut rng);

        if safe_before {
            natural += 1;
        } else {
            relocated += 1;
            println!("seed {seed}: backstop acted — {:?}", report.actions);
        }
        assert!(
            worlds
                .iter()
                .any(|w| w.locks.iter().any(|l| l.secret_exit_safe)),
            "seed {seed}: no secret-exit-safe lock in any world after backstop"
        );
        // The flags the invariant relied on must be honest: safe means the
        // world survives that lock never opening.
        for w in &worlds {
            for li in 0..w.locks.len() {
                assert_eq!(
                    w.locks[li].secret_exit_safe,
                    w.completable_sealed(Some(li)),
                    "seed {seed} W{}: stale secret_exit_safe flag on lock {li}",
                    w.world_idx + 1
                );
            }
        }
    }
    println!(
        "secret-exit safety over {seeds} seeds: natural {natural}, backstop relocations {relocated}"
    );
}

/// One seed's shape, for across-seed comparison: where its levels sit, which
/// of them the cheapest route plays, and how clumped the layout is.
struct SeedShape {
    levels: std::collections::BTreeSet<Pos>,
    cheap_route_levels: std::collections::BTreeSet<Pos>,
    /// Mean Manhattan distance from each level to its nearest other level —
    /// low = clustered, high = spread.
    nearest_neighbor: f64,
}

impl SeedShape {
    fn of(state: &WorldState) -> SeedShape {
        let positions: Vec<Pos> = state
            .slots
            .iter()
            .filter(|s| s.kind == SlotKind::Level)
            .map(|s| s.pos)
            .collect();
        let measure = measure_world(state);
        SeedShape {
            levels: positions.iter().copied().collect(),
            cheap_route_levels: measure
                .rc
                .routes
                .first()
                .map(|r| r.levels.clone())
                .unwrap_or_default(),
            nearest_neighbor: mean_nearest_neighbor(&positions),
        }
    }
}

/// Mean over points of the Manhattan distance to the nearest other point.
fn mean_nearest_neighbor(positions: &[Pos]) -> f64 {
    if positions.len() < 2 {
        return 0.0;
    }
    let total: usize = positions
        .iter()
        .map(|a| {
            positions
                .iter()
                .filter(|b| *b != a)
                .map(|b| a.0.abs_diff(b.0) + a.1.abs_diff(b.1))
                .min()
                .unwrap()
        })
        .sum();
    total as f64 / positions.len() as f64
}

/// Jaccard distance between two position sets: 0 = identical, 1 = disjoint.
fn jaccard_distance(a: &std::collections::BTreeSet<Pos>, b: &std::collections::BTreeSet<Pos>) -> f64 {
    let union = a.union(b).count();
    if union == 0 {
        return 0.0;
    }
    1.0 - a.intersection(b).count() as f64 / union as f64
}

fn pearson(xs: &[f64], ys: &[f64]) -> f64 {
    let n = xs.len() as f64;
    let mx = xs.iter().sum::<f64>() / n;
    let my = ys.iter().sum::<f64>() / n;
    let cov: f64 = xs.iter().zip(ys).map(|(x, y)| (x - mx) * (y - my)).sum();
    let vx: f64 = xs.iter().map(|x| (x - mx) * (x - mx)).sum();
    let vy: f64 = ys.iter().map(|y| (y - my) * (y - my)).sum();
    if vx == 0.0 || vy == 0.0 {
        return 0.0;
    }
    cov / (vx * vy).sqrt()
}

/// The across-seed numbers for one world+arm: mean pairwise layout/route
/// distance, nearest-neighbor mean and across-seed spread, and the
/// layout↔route distance correlation ("salt r").
fn diversity_row(shapes: &[SeedShape]) -> (f64, f64, f64, f64, f64) {
    let mut layout_d = Vec::new();
    let mut route_d = Vec::new();
    for i in 0..shapes.len() {
        for j in (i + 1)..shapes.len() {
            layout_d.push(jaccard_distance(&shapes[i].levels, &shapes[j].levels));
            route_d.push(jaccard_distance(
                &shapes[i].cheap_route_levels,
                &shapes[j].cheap_route_levels,
            ));
        }
    }
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
    let nn: Vec<f64> = shapes.iter().map(|s| s.nearest_neighbor).collect();
    let nn_mean = mean(&nn);
    let nn_sd =
        (nn.iter().map(|x| (x - nn_mean) * (x - nn_mean)).sum::<f64>() / nn.len() as f64).sqrt();
    (mean(&layout_d), mean(&route_d), nn_mean, nn_sd, pearson(&layout_d, &route_d))
}

/// Across-seed diversity census: how different are two SEEDS of the same
/// world? The within-seed metrics (routes, uniq) can look good while every
/// seed produces nearly the same world — this census measures the spread.
///
/// Columns, per world and arm:
/// - layout-div: mean pairwise Jaccard distance between level-position sets
/// - route-div: same distance over the cheapest route's level set
/// - NN mean±sd: per-seed clumpiness (mean nearest-neighbor Manhattan
///   distance); the sd ACROSS seeds is the shape-variety number — uniform
///   placement is expected to produce similar NN every seed (low sd)
/// - salt-r: correlation between layout distance and route distance over
///   seed pairs — the "levels are the rng salt" hypothesis test
///
/// Arms: v2 skeleton (uniform placement), v2 + shaping (the improvement
/// loop's diversity spend), and the shipping builder (scored placement) —
/// the collapse any judgment-bearing pass must stay far away from.
/// `V2_SEEDS` seeds (default 20).
///
/// Deliberately runs the CONTROLLED configuration (always-on QOL only — no
/// SAS, no map arms, no shuffle rolls), unlike the report-card censuses:
/// this census isolates PLACEMENT-driven diversity, and mixing flag-driven
/// input variation into the pairwise distances would inflate every arm's
/// numbers with differences the placement code didn't produce.
#[test]
fn test_v2_diversity_census() {
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
    let pickup = pick_up(&rom, &catalog, PickupFlags::default());

    // Shipping arm: one build() per seed yields all 8 worlds.
    let mut shipping: Vec<Vec<SeedShape>> = (0..8).map(|_| Vec::new()).collect();
    for seed in 0..seeds {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let result = build(
            &rom,
            &OverworldData { pickup: &pickup, catalog: &catalog },
            &mut rng,
            BuildFlags::default(),
        );
        for built in &result.worlds {
            shipping[built.world_idx].push(SeedShape::of(&from_built(built)));
        }
    }

    println!("v2 across-seed diversity census ({seeds} seeds, v2 uniform vs shipping scored)");
    println!("world  arm       layout-div  route-div  NN(mean±sd)  salt-r");
    for (world_idx, shipping_shapes) in shipping.iter().enumerate() {
        let v2_shapes: Vec<SeedShape> = (0..seeds)
            .map(|seed| {
                let mut state =
                    from_pickup(&rom, &catalog, &pickup, world_idx, &BuildFlags::default());
                let mut rng = ChaCha8Rng::seed_from_u64(seed);
                run_schedule(
                    &mut state,
                    &[&Connectivity, &Levels, &SparePipes, &Forts, &Locks],
                    &mut rng,
                );
                SeedShape::of(&state)
            })
            .collect();

        // The shaped arm measures what the improvement loop's accepted moves
        // cost in across-seed spread — the diversity spend of shaping.
        let shaped_shapes: Vec<SeedShape> = (0..seeds)
            .map(|seed| {
                let mut state =
                    from_pickup(&rom, &catalog, &pickup, world_idx, &BuildFlags::default());
                let mut rng = ChaCha8Rng::seed_from_u64(seed);
                run_schedule(
                    &mut state,
                    &[&Connectivity, &Levels, &Forts, &Locks, &Shaping],
                    &mut rng,
                );
                SeedShape::of(&state)
            })
            .collect();

        for (arm, shapes) in [
            ("v2", &v2_shapes),
            ("v2+shape", &shaped_shapes),
            ("shipping", shipping_shapes),
        ] {
            let (layout, route, nn_mean, nn_sd, salt_r) = diversity_row(shapes);
            println!(
                "  W{}   {arm:<9} {layout:>9.2} {route:>10.2}  {nn_mean:>5.2} ±{nn_sd:4.2} {salt_r:>7.2}",
                world_idx + 1,
            );
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
/// Runs the realistic flag mix (see [`census_ctx`]). `V2_SEEDS` seeds
/// (default 100).
#[test]
fn test_v2_spare_pipes_census() {
    let Some(raw) = load_rom() else {
        eprintln!("ROM not found, skipping");
        return;
    };
    let seeds: u64 = std::env::var("V2_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);

    let mut spares = [0usize; 8];
    let mut create = [0usize; 8];
    let mut destroy = [0usize; 8];
    let mut cheapen = [0usize; 8];
    let mut inert = [0usize; 8];
    let mut c1_sum = [0u64; 8];
    let mut routes_sum = [0usize; 8];
    let mut linear = [0usize; 8];

    for seed in 0..seeds {
        let ctx = census_ctx(&raw, seed);
        for world_idx in 0..8 {
            let mut state = ctx.world(world_idx);
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            run_schedule(&mut state, &[&Connectivity, &Levels, &SparePipes], &mut rng);

            let spare_report = state
                .log
                .iter()
                .find(|r| r.phase == "spare_pipes")
                .expect("spare_pipes always reports");
            for line in &spare_report.actions {
                let Some((ra, rb, ca, cb)) = parse_pipe_delta(line) else { continue };
                spares[world_idx] += 1;
                if rb > ra {
                    create[world_idx] += 1;
                } else if rb < ra {
                    destroy[world_idx] += 1;
                } else if cb < ca {
                    cheapen[world_idx] += 1;
                } else {
                    inert[world_idx] += 1;
                }
            }

            let measure = measure_world(&state);
            c1_sum[world_idx] += u64::from(measure.c1);
            routes_sum[world_idx] += measure.routes_in_band;
            if measure.routes_in_band < 2 {
                linear[world_idx] += 1;
            }
        }
    }

    println!("v2 spare-pipes census ({seeds} seeds, random toss, observe-only deltas, flag-mix arms)");
    println!("world  spares  create%  destroy%  cheapen%  inert%  C1(mean)  routes  linear%");
    let n = seeds as f64;
    for world_idx in 0..8 {
        let per = |x: usize| 100.0 * x as f64 / spares[world_idx].max(1) as f64;
        println!(
            "  W{}   {:>6.2} {:>7.0}% {:>8.0}% {:>8.0}% {:>6.0}% {:>9.1} {:>7.2} {:>7.0}%",
            world_idx + 1,
            spares[world_idx] as f64 / n,
            per(create[world_idx]),
            per(destroy[world_idx]),
            per(cheapen[world_idx]),
            per(inert[world_idx]),
            c1_sum[world_idx] as f64 / n,
            routes_sum[world_idx] as f64 / n,
            100.0 * linear[world_idx] as f64 / n,
        );
    }

    // One narrated example: W2 (whose single spare IS its whole pipe story).
    let mut state = census_ctx(&raw, 0).world(1);
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
