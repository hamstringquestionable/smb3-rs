//! Builder harness tests: the schedule contract, and the two measurement demos
//! (vanilla ground truth, current-builder baseline). Table output prints with
//! `cargo test overworld_build -- --nocapture`.

use super::*;

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;


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
    qol::apply_w1_shortcut(&mut out, hammer_rocks);
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
/// The builder has no toad-house / hammer-bro / spade placement phases yet — when a
/// shuffle rolls on, those slots are simply picked up and stay blank space.
/// Spade shuffle stays off until such a phase exists.
struct CensusCtx {
    rom: Rom,
    catalog: NodeCatalog,
    pickup: Pickup,
    flags: BuildFlags,
    /// Per-seed level/fort allotment, rolled once for all 8 worlds via the
    /// shipping builder's exact machinery (`allot_budgets`).
    level_counts: [usize; 8],
    fort_counts: [usize; 8],
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
    let flags = BuildFlags {
        shuffle_toad_houses,
        eights_are_wild: eights_wild,
        shuffle_hammer_bros,
    };
    let (level_counts, fort_counts) =
        allot_budgets(&rom, &catalog, &pickup, &flags, &mut roll_rng);
    CensusCtx {
        rom,
        catalog,
        pickup,
        flags,
        level_counts,
        fort_counts,
    }
}

impl CensusCtx {
    fn world(&self, world_idx: usize) -> WorldState {
        let mut state =
            from_pickup(&self.rom, &self.catalog, &self.pickup, world_idx, &self.flags);
        state.level_budget = self.level_counts[world_idx];
        state.fort_budget = self.fort_counts[world_idx];
        state
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
fn test_builder_schedule_runs_phases_in_order() {
    let mut state = WorldState {
        world_idx: 0,
        grid: Grid { tiles: vec![vec![0x00; 4]; 9], cols: 4, eights_are_wild: false },
        slots: Vec::new(),
        locks: Vec::new(),
        pipe_pairs: Vec::new(),
        start: None,
        target: None,
        fixed: HashSet::new(),
        hammer_gated: HashSet::new(),
        pipe_budget: 0,
        level_budget: 0,
        fort_budget: 0,
        ptr_slots: 0,
        hb_sprite_pins: Vec::new(),
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

/// The W1 shortcut must be WALKABLE, not just drawn.
///
/// `Map_CheckDoMove` validates only the adjacent tile and then moves a
/// hardcoded 32 units — two tiles — so a new link needs exactly one path tile
/// between two nodes an even distance apart. Off-by-one drawings (a node
/// adjacent to its anchor, two stacked path tiles below) look right in a tile
/// editor and are dead in game, and worse: the stray blank still enters the
/// placement pool, so the builder puts content on a node no one can reach.
///
/// This pins all three halves. With `More hammer rocks` OFF the rock is the
/// permanent `0x53` — same pixels as `0x52`, so the map gives away nothing
/// about how a `Maybe` roll came out — and W1 stays vanilla-shaped. With it ON
/// the rock is a wall to the plain walk, and once broken (4,8) and (6,8) each
/// step directly onto the other.
#[test]
fn test_w1_shortcut_is_walkable() {
    let Some(raw) = load_rom() else {
        eprintln!("ROM not found, skipping");
        return;
    };
    let off = rom_data::read_tile_grid(&base_qol(&raw), 0);
    assert_eq!(off.get(5, 8), 0x53, "flag off must still place the decoy rock");
    assert_eq!(off.get(6, 8), 0x4A, "the path stub is part of the disguise");
    let off_pipes = rom_data::read_pipe_pairs(&base_qol(&raw)).remove(&0).unwrap_or_default();
    let off_walk = walk_map(&off, &off_pipes, None, 0);
    assert!(
        !off_walk.edges[&(4, 8)].iter().any(|e| e.dest == (6, 8)),
        "0x53 is not breakable, so the shortcut must not exist at all",
    );

    let rom = qol_variant(&raw, true, false);
    // W1 has no pipes, so its key is simply absent from the map.
    let pipes = rom_data::read_pipe_pairs(&rom).remove(&0).unwrap_or_default();

    let mut grid = rom_data::read_tile_grid(&rom, 0);
    assert_eq!(grid.get(5, 8), 0x52, "the shortcut must be gated by a rock");
    let closed = walk_map(&grid, &pipes, None, 0);
    assert!(
        !closed.edges[&(4, 8)].iter().any(|e| e.dest == (6, 8)),
        "an unbroken rock must not be walkable — the link would be free",
    );

    grid.set(5, 8, 0x46); // what `BREAKABLE_ROCKS` opens 0x52 into
    let open = walk_map(&grid, &pipes, None, 0);
    for (from, to) in [((4, 8), (6, 8)), ((6, 8), (4, 8))] {
        assert!(open.nodes.contains(&from), "{from:?} unreachable from start");
        let joins = open.edges[&from].iter().any(|e| e.dest == to);
        assert!(joins, "{from:?} does not step onto {to:?}");
    }
}

/// Measure the eight VANILLA worlds — known ground truth for calibrating the
/// metrics themselves. Raw ROM, no QOL: this is the game as shipped.
#[test]
fn test_builder_vanilla_worlds() {
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

/// Measure SHIPPING-builder output through the builder harness — the baseline
/// table the builder is compared against, on the SAME flag mix as the censuses
/// (see [`census_ctx`]: QOL arms, SAS 50/50 per world, toad-house /
/// hammer-bro rolls) and the same metric columns as the shaping census.
/// Seeds via `CENSUS_SEEDS` (default 20).
#[test]
fn test_builder_current_builder_worlds() {
    let Some(raw) = load_rom() else {
        eprintln!("ROM not found, skipping");
        return;
    };
    let seeds: u64 = std::env::var("CENSUS_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);

    #[derive(Default, Clone, Copy)]
    struct Tally {
        c1: u64,
        c1_min: u32,
        cheap: usize,
        routes: usize,
        linear: usize,
        uniq_sum: f64,
        multi: usize,
        gap_sum: u64,
        gapped: usize,
        noalt: usize,
        goal_open: usize,
    }
    let mut tallies = [Tally { c1_min: u32::MAX, ..Default::default() }; 8];

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
            let state = from_built(built);
            let m = measure_world(&state);
            assert!(
                m.reachable,
                "seed {} W{} goal must be reachable",
                seed,
                built.world_idx + 1
            );
            let t = &mut tallies[built.world_idx];
            t.c1 += u64::from(m.c1);
            t.c1_min = t.c1_min.min(m.c1);
            if m.c1 < 12 {
                t.cheap += 1;
            }
            t.routes += m.routes_in_band;
            if m.routes_in_band < 2 {
                t.linear += 1;
            } else {
                t.multi += 1;
                t.uniq_sum += m.mean_exclusive_levels;
            }
            let wide = analyze_route_choice(&state.to_built(), SHAPING_SLACK);
            if wide.routes.len() >= 2 {
                t.gap_sum += u64::from(wide.routes[1].cost - wide.routes[0].cost);
                t.gapped += 1;
            } else {
                t.noalt += 1;
            }
            if m.goal_open {
                t.goal_open += 1;
            }
        }
    }

    println!("shipping builder through the builder harness ({seeds} seeds, flag-mix arms)");
    println!("world  C1(mean)  C1min  C1<12%  routes  linear%  uniq  C2-C1  noalt%  goal-open%");
    let n = seeds as f64;
    for (world_idx, t) in tallies.iter().enumerate() {
        println!(
            "  W{}   {:>7.1} {:>6} {:>6.1}% {:>7.2} {:>7.0}% {:>5.2} {:>6.2} {:>6.0}% {:>9.0}%",
            world_idx + 1,
            t.c1 as f64 / n,
            t.c1_min,
            100.0 * t.cheap as f64 / n,
            t.routes as f64 / n,
            100.0 * t.linear as f64 / n,
            t.uniq_sum / t.multi.max(1) as f64,
            t.gap_sum as f64 / t.gapped.max(1) as f64,
            100.0 * t.noalt as f64 / n,
            100.0 * t.goal_open as f64 / n,
        );
    }
}

/// Connectivity discovery census: run ONLY the connectivity phase on the
/// pickup-cleared worlds and measure what it does — pipes spent vs the
/// vanilla budget, blanks left stranded, goal reachability, and endpoint
/// variety across seeds. Runs the realistic flag mix (see [`census_ctx`]).
/// `CENSUS_SEEDS` sets the seed count (default 100).
///
/// No assertions on coverage yet: this is the rediscovery baseline the
/// controls will be justified against.
#[test]
fn test_builder_connectivity_census() {
    let Some(raw) = load_rom() else {
        eprintln!("ROM not found, skipping");
        return;
    };
    let seeds: u64 = std::env::var("CENSUS_SEEDS")
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

    println!("connectivity census ({seeds} seeds, knob-free uniform endpoints, flag-mix arms)");
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
/// [`census_ctx`]). `CENSUS_SEEDS` seeds (default 100).
#[test]
fn test_builder_levels_census() {
    let Some(raw) = load_rom() else {
        eprintln!("ROM not found, skipping");
        return;
    };
    let seeds: u64 = std::env::var("CENSUS_SEEDS")
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

    println!("levels census ({seeds} seeds, uniform placement after connectivity, flag-mix arms)");
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
/// flag mix (see [`census_ctx`]). `CENSUS_SEEDS` seeds (default 100).
#[test]
fn test_builder_forts_census() {
    let Some(raw) = load_rom() else {
        eprintln!("ROM not found, skipping");
        return;
    };
    let seeds: u64 = std::env::var("CENSUS_SEEDS")
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

    println!("forts census ({seeds} seeds, uniform placement, full dumb pipeline, flag-mix arms)");
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
/// `test_builder_current_builder_worlds`. Columns: removed% (forts removed by the
/// every-fort-locked invariant — expected ~0), safe% (locks that are
/// secret-exit-safe), goal-open% (world finishable with all locks closed),
/// zero-gate% (locks that wall off nothing — pure decoration). Runs the
/// realistic flag mix (see [`census_ctx`]) — the invariant asserts cover
/// every arm. `CENSUS_SEEDS` seeds (default 100).
#[test]
fn test_builder_locks_census() {
    let Some(raw) = load_rom() else {
        eprintln!("ROM not found, skipping");
        return;
    };
    let seeds: u64 = std::env::var("CENSUS_SEEDS")
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

    println!("full-skeleton census ({seeds} seeds, all five dumb phases, flag-mix arms)");
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
/// → spare pipes → forts → locks) against the production pipeline (arm
/// `shaped`, via `run_shaped_with_web_retries`: connectivity → levels →
/// forts → locks → shaping → guarded spare pipes). Shaping runs BEFORE the
/// spares so the gated-shortcut rung keeps first claim on the budget; the
/// spares then spend the remainder — every world ships its full vanilla
/// pipe count, so the pipes column should read ≈ budget.
///
/// Shaped-arm move columns: touched% = seeds where at least one move was
/// accepted (satisficing means most worlds should be untouched), lr/gs =
/// lock_replace / gated_shortcut accepts:rejects summed over all seeds. The
/// time line is the performance watchdog: mean wall-clock per seed (all 8
/// worlds) per arm. Runs the realistic flag mix (see [`census_ctx`]).
/// `CENSUS_SEEDS` seeds (default 100).
#[test]
fn test_builder_shaping_census() {
    let Some(raw) = load_rom() else {
        eprintln!("ROM not found, skipping");
        return;
    };
    let seeds: u64 = std::env::var("CENSUS_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);

    #[derive(Clone, Copy)]
    struct ArmTally {
        c1: u64,
        c1_min: u32,
        // Seeds whose C1 lands under the old v1 floor (12) — the
        // nearly-free-world tail the mean hides.
        cheap: usize,
        routes: usize,
        linear: usize,
        // Choice quality, over the seeds that HAVE a choice (>= 2 routes in
        // the DEFAULT_SLACK band): how many levels the alternatives play
        // that the cheapest route doesn't.
        uniq_sum: f64,
        multi: usize,
        // Distance to the runner-up, measured in the wide (SHAPING_SLACK)
        // band so linear worlds report it too: C2 - C1 when a second route
        // exists within +SHAPING_SLACK, else the seed counts as `noalt`.
        gap_sum: u64,
        gapped: usize,
        noalt: usize,
        zero_gate: usize,
        goal_gates: usize,
        locks: usize,
        goal_open: usize,
        pipes: usize,
        // Adjacent-level pairs (orthogonal 2-tile neighbors — visually
        // stacked levels) — the clustering the playtest flagged.
        adjacent_levels: usize,
        // Largest connected chain of adjacent levels: sum (for the mean),
        // worst seed, and how many seeds have a chain of 4+ levels.
        chain_sum: usize,
        chain_worst: usize,
        chain4: usize,
    }

    impl Default for ArmTally {
        fn default() -> Self {
            ArmTally {
                c1: 0,
                c1_min: u32::MAX,
                cheap: 0,
                routes: 0,
                linear: 0,
                uniq_sum: 0.0,
                multi: 0,
                gap_sum: 0,
                gapped: 0,
                noalt: 0,
                zero_gate: 0,
                goal_gates: 0,
                locks: 0,
                goal_open: 0,
                pipes: 0,
                adjacent_levels: 0,
                chain_sum: 0,
                chain_worst: 0,
                chain4: 0,
            }
        }
    }

    fn tally(t: &mut ArmTally, state: &WorldState) {
        let m = measure_world(state);
        t.c1 += u64::from(m.c1);
        t.c1_min = t.c1_min.min(m.c1);
        if m.c1 < 12 {
            t.cheap += 1;
        }
        t.routes += m.routes_in_band;
        if m.routes_in_band < 2 {
            t.linear += 1;
        } else {
            t.multi += 1;
            t.uniq_sum += m.mean_exclusive_levels;
        }
        let wide = analyze_route_choice(&state.to_built(), SHAPING_SLACK);
        if wide.routes.len() >= 2 {
            t.gap_sum += u64::from(wide.routes[1].cost - wide.routes[0].cost);
            t.gapped += 1;
        } else {
            t.noalt += 1;
        }
        if m.goal_open {
            t.goal_open += 1;
        }
        t.zero_gate += state.zero_gate_locks().len();
        t.goal_gates += state.goal_gate_locks();
        t.locks += state.locks.len();
        t.pipes += state.pipe_pairs.len();
        let levels: Vec<Pos> = state
            .slots
            .iter()
            .filter(|s| s.kind == SlotKind::Level)
            .map(|s| s.pos)
            .collect();
        let adjacent = |a: Pos, b: Pos| {
            let (dr, dc) = (a.0.abs_diff(b.0), a.1.abs_diff(b.1));
            (dr == 2 && dc == 0) || (dr == 0 && dc == 2)
        };
        for (i, &a) in levels.iter().enumerate() {
            for &b in &levels[i + 1..] {
                if adjacent(a, b) {
                    t.adjacent_levels += 1;
                }
            }
        }
        // Largest connected chain under the same adjacency.
        let mut comp: Vec<usize> = (0..levels.len()).collect();
        for i in 0..levels.len() {
            for j in i + 1..levels.len() {
                if adjacent(levels[i], levels[j]) {
                    let (mut ri, mut rj) = (i, j);
                    while comp[ri] != ri { ri = comp[ri]; }
                    while comp[rj] != rj { rj = comp[rj]; }
                    comp[ri] = rj;
                }
            }
        }
        let mut sizes: HashMap<usize, usize> = HashMap::new();
        for i in 0..levels.len() {
            let mut r = i;
            while comp[r] != r { r = comp[r]; }
            *sizes.entry(r).or_default() += 1;
        }
        let max_chain = sizes.values().copied().max().unwrap_or(0);
        t.chain_sum += max_chain;
        t.chain_worst = t.chain_worst.max(max_chain);
        if max_chain >= 4 {
            t.chain4 += 1;
        }
    }

    let mut dumb = [ArmTally::default(); 8];
    let mut shaped = [ArmTally::default(); 8];
    let mut touched = [0usize; 8];
    let mut lr_acc = [0usize; 8];
    let mut lr_rej = [0usize; 8];
    let mut ab_acc = [0usize; 8];
    let mut ab_rej = [0usize; 8];
    let mut gs_acc = [0usize; 8];
    let mut gs_rej = [0usize; 8];
    let mut fl_acc = [0usize; 8];
    let mut fl_rej = [0usize; 8];
    let mut lm_acc = [0usize; 8];
    let mut lm_rej = [0usize; 8];
    let mut pm_acc = [0usize; 8];
    let mut pm_rej = [0usize; 8];
    let mut redeals = [0usize; 8];
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
            run_shaped_with_web_retries(&mut state, &mut rng);
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

            // A web redeal re-runs the shaping phase, so counts aggregate
            // over every attempt's report; redeals = attempts - 1.
            let shaping_reports =
                || state.log.iter().filter(|r| r.phase == "shaping");
            assert!(shaping_reports().count() > 0, "shaping always reports");
            redeals[world_idx] += shaping_reports().count() - 1;
            let count = |needle: &str| {
                shaping_reports()
                    .flat_map(|r| r.actions.iter())
                    .filter(|a| a.contains(needle))
                    .count()
            };
            let la = count("lock_replace ACCEPT");
            let aa = count("arm_balance ACCEPT");
            let ga = count("gated_shortcut ACCEPT");
            let fa = count("fort_lock ACCEPT");
            let ma = count("level_move ACCEPT");
            let pa = count("pipe_move ACCEPT");
            lr_acc[world_idx] += la;
            lr_rej[world_idx] += count("lock_replace REJECT");
            ab_acc[world_idx] += aa;
            ab_rej[world_idx] += count("arm_balance REJECT");
            gs_acc[world_idx] += ga;
            gs_rej[world_idx] += count("gated_shortcut REJECT");
            fl_acc[world_idx] += fa;
            fl_rej[world_idx] += count("fort_lock REJECT");
            lm_acc[world_idx] += ma;
            lm_rej[world_idx] += count("level_move REJECT");
            pm_acc[world_idx] += pa;
            pm_rej[world_idx] += count("pipe_move REJECT");
            if la + aa + ga + fa + ma + pa > 0 {
                touched[world_idx] += 1;
            }
        }
    }

    println!("shaping A/B census ({seeds} seeds, dumb skeleton vs diagnosis-driven shaping, flag-mix arms)");
    println!("world  arm     C1(mean)  C1min  C1<12%  routes  linear%  uniq  C2-C1  noalt%  zero-gate%  gGate  goal-open%  pipes  touched%  lr(acc:rej)  ab(acc:rej)  gs(acc:rej)  fl(acc:rej)  lm(acc:rej)  pm(acc:rej)  redeals");
    let n = seeds as f64;
    for world_idx in 0..8 {
        for (arm, t) in [("dumb", &dumb[world_idx]), ("shaped", &shaped[world_idx])] {
            let base = format!(
                "  W{}   {arm:<7} {:>7.1} {:>6} {:>6.0}% {:>7.2} {:>7.0}% {:>5.2} {:>6.2} {:>6.0}% {:>9.0}% {:>5.2} {:>9.0}% {:>6.2} adjL={:.2}",
                world_idx + 1,
                t.c1 as f64 / n,
                t.c1_min,
                100.0 * t.cheap as f64 / n,
                t.routes as f64 / n,
                100.0 * t.linear as f64 / n,
                t.uniq_sum / t.multi.max(1) as f64,
                t.gap_sum as f64 / t.gapped.max(1) as f64,
                100.0 * t.noalt as f64 / n,
                100.0 * t.zero_gate as f64 / t.locks.max(1) as f64,
                t.goal_gates as f64 / n,
                100.0 * t.goal_open as f64 / n,
                t.pipes as f64 / n,
                t.adjacent_levels as f64 / n,
            ) + &format!(
                " chain={:.2}/max{}/4+{:.0}%",
                t.chain_sum as f64 / n,
                t.chain_worst,
                100.0 * t.chain4 as f64 / n,
            );
            if arm == "shaped" {
                println!(
                    "{base} {:>7.0}% {:>8}:{:<4} {:>6}:{:<4} {:>6}:{:<4} {:>6}:{:<4} {:>6}:{:<4} {:>6}:{:<4} {:>6}",
                    100.0 * touched[world_idx] as f64 / n,
                    lr_acc[world_idx],
                    lr_rej[world_idx],
                    ab_acc[world_idx],
                    ab_rej[world_idx],
                    gs_acc[world_idx],
                    gs_rej[world_idx],
                    fl_acc[world_idx],
                    fl_rej[world_idx],
                    lm_acc[world_idx],
                    lm_rej[world_idx],
                    pm_acc[world_idx],
                    pm_rej[world_idx],
                    redeals[world_idx],
                );
            } else {
                println!("{base}        -        -         -         -         -         -         -        -");
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
/// (knob-free skeleton), `shaped` (shaping loop), `build` (the full
/// build() entry, promotions included — shaped and build should agree) —
/// all fed the same flag-mix input (see
/// [`census_ctx`]). `CENSUS_SEEDS` seeds (default 100).
#[test]
fn test_builder_progression_census() {
    let Some(raw) = load_rom() else {
        eprintln!("ROM not found, skipping");
        return;
    };
    let seeds: u64 = std::env::var("CENSUS_SEEDS")
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
            run_shaped_with_web_retries(&mut state, &mut rng);
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
        "progression census ({seeds} seeds, no-hammer required-progression, flag-mix arms)"
    );
    println!("world  arm      forts-req  lvls-req  streak  streak>=2%  goalstk  goalstk>=2%");
    for world_idx in 0..8 {
        for (arm, t) in [
            ("vanilla", &vanilla[world_idx]),
            ("dumb", &dumb[world_idx]),
            ("shaped", &shaped[world_idx]),
            ("build", &shipping[world_idx]),
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

/// Cross-world secret-exit-safety invariant: after all 8 worlds run a full
/// schedule, at least one lock somewhere must be secret-exit-safe (the
/// writer parks the 1-F fortress level on it). Measures how often placement
/// satisfies this naturally vs how often the relocation backstop has to
/// act. BOTH arms are asserted — the dumb skeleton and the shaped pipeline
/// (whose moves rewrite lock sets and pipe webs; `recompute_safety_flags`
/// after accepted moves is what keeps the flags honest). Runs the realistic
/// flag mix (see [`census_ctx`]). `CENSUS_SEEDS` seeds (default 100 — each seed
/// builds all 8 worlds per arm).
#[test]
fn test_builder_secret_exit_safety() {
    let Some(raw) = load_rom() else {
        eprintln!("ROM not found, skipping");
        return;
    };
    let seeds: u64 = std::env::var("CENSUS_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);

    for arm in ["dumb", "shaped"] {
        let mut natural = 0usize;
        let mut relocated = 0usize;
        for seed in 0..seeds {
            let ctx = census_ctx(&raw, seed);
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let mut worlds: Vec<WorldState> = (0..8)
                .map(|world_idx| {
                    let mut state = ctx.world(world_idx);
                    if arm == "dumb" {
                        run_schedule(
                            &mut state,
                            &[&Connectivity, &Levels, &SparePipes, &Forts, &Locks],
                            &mut rng,
                        );
                    } else {
                        run_shaped_with_web_retries(&mut state, &mut rng);
                    }
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
                println!("[{arm}] seed {seed}: backstop acted — {:?}", report.actions);
            }
            assert!(
                worlds
                    .iter()
                    .any(|w| w.locks.iter().any(|l| l.secret_exit_safe)),
                "[{arm}] seed {seed}: no secret-exit-safe lock in any world after backstop"
            );
            // The flags the invariant relied on must be honest: safe means
            // the world survives that lock never opening.
            for w in &worlds {
                for li in 0..w.locks.len() {
                    assert_eq!(
                        w.locks[li].secret_exit_safe,
                        w.completable_sealed(Some(li)),
                        "[{arm}] seed {seed} W{}: stale secret_exit_safe flag on lock {li}",
                        w.world_idx + 1
                    );
                }
            }
        }
        println!(
            "[{arm}] secret-exit safety over {seeds} seeds: natural {natural}, backstop relocations {relocated}"
        );
    }
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
/// Arms: dumb skeleton (uniform placement), skeleton + shaping (the improvement
/// loop's diversity spend), and the shipping builder (scored placement) —
/// the collapse any judgment-bearing pass must stay far away from.
/// `CENSUS_SEEDS` seeds (default 20).
///
/// Deliberately runs the CONTROLLED configuration (always-on QOL only — no
/// SAS, no map arms, no shuffle rolls), unlike the report-card censuses:
/// this census isolates PLACEMENT-driven diversity, and mixing flag-driven
/// input variation into the pairwise distances would inflate every arm's
/// numbers with differences the placement code didn't produce.
#[test]
fn test_builder_diversity_census() {
    let Some(rom) = load_rom() else {
        eprintln!("ROM not found, skipping");
        return;
    };
    let rom = base_qol(&rom);
    let seeds: u64 = std::env::var("CENSUS_SEEDS")
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

    // Per-seed budget allotment for the builder arms — same variance rules the
    // shipping arm gets internally from build().
    let allotments: Vec<([usize; 8], [usize; 8])> = (0..seeds)
        .map(|seed| {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            allot_budgets(&rom, &catalog, &pickup, &BuildFlags::default(), &mut rng)
        })
        .collect();

    println!("across-seed diversity census ({seeds} seeds, uniform vs old-scored baseline)");
    println!("world  arm       layout-div  route-div  NN(mean±sd)  salt-r");
    for (world_idx, shipping_shapes) in shipping.iter().enumerate() {
        let budgeted_world = |seed: u64| {
            let mut state =
                from_pickup(&rom, &catalog, &pickup, world_idx, &BuildFlags::default());
            let (level_counts, fort_counts) = &allotments[seed as usize];
            state.level_budget = level_counts[world_idx];
            state.fort_budget = fort_counts[world_idx];
            state
        };

        let dumb_shapes: Vec<SeedShape> = (0..seeds)
            .map(|seed| {
                let mut state = budgeted_world(seed);
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
                let mut state = budgeted_world(seed);
                let mut rng = ChaCha8Rng::seed_from_u64(seed);
                run_shaped_with_web_retries(&mut state, &mut rng);
                SeedShape::of(&state)
            })
            .collect();

        for (arm, shapes) in [
            ("dumb", &dumb_shapes),
            ("shaped", &shaped_shapes),
            ("build", shipping_shapes),
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
/// Runs the realistic flag mix (see [`census_ctx`]). `CENSUS_SEEDS` seeds
/// (default 100).
#[test]
fn test_builder_spare_pipes_census() {
    let Some(raw) = load_rom() else {
        eprintln!("ROM not found, skipping");
        return;
    };
    let seeds: u64 = std::env::var("CENSUS_SEEDS")
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

    println!("spare-pipes census ({seeds} seeds, random toss, observe-only deltas, flag-mix arms)");
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
/// dominated routes both printed. `CENSUS_WORLD` picks the world (1-8, default
/// 2), `CENSUS_SLACK` the band (default 12). Born from the W2 rock question —
/// "where did the non-rock route go?" — and kept for the next such question.
#[test]
fn test_builder_probe_vanilla_world() {
    let Some(rom) = load_rom() else {
        eprintln!("ROM not found, skipping");
        return;
    };
    let world: usize = std::env::var("CENSUS_WORLD").ok().and_then(|s| s.parse().ok()).unwrap_or(2);
    let slack: u32 = std::env::var("CENSUS_SLACK").ok().and_then(|s| s.parse().ok()).unwrap_or(12);
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
    println!("{}", route_choice::render_route_choice(&state.to_built(), slack));
}

/// PRODUCTION-OUTPUT invariants, on the real `build()` (post promotions and
/// sprite redistribution), across the census flag mix:
///
/// - **Completability**: every world passes the order-free fixpoint — close
///   every lock, beat every reachable fort, open its lock, repeat; the goal
///   must be reached and every fort beaten.
/// - **One lock per fort**, and fort `section` values dense `0..n` in slot
///   order — the writer pairs locks to forts by index.
/// - **Secret-exit safety**: each seed has at least one lock, across all 8
///   worlds, that can stay closed forever (honesty of the flag re-checked
///   against `completable_sealed`).
///
/// `CENSUS_SEEDS` scales the sweep (default 25 — every seed builds 8 worlds).
#[test]
fn test_builder_output_completable() {
    let Some(raw) = load_rom() else {
        eprintln!("ROM not found, skipping");
        return;
    };
    let seeds: u64 = std::env::var("CENSUS_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(25);
    for seed in 0..seeds {
        let ctx = census_ctx(&raw, seed);
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let result = build(
            &ctx.rom,
            &OverworldData { pickup: &ctx.pickup, catalog: &ctx.catalog },
            &mut rng,
            ctx.flags,
        );
        let mut any_safe = false;
        for built in &result.worlds {
            let wi = built.world_idx;
            let state = from_built(built);
            assert!(
                state.completable(),
                "seed {seed} W{}: built world must be completable",
                wi + 1
            );
            let forts: Vec<usize> = built
                .slots
                .iter()
                .filter(|s| s.kind == SlotKind::Fortress)
                .map(|s| s.section)
                .collect();
            assert_eq!(
                built.locks.len(),
                forts.len(),
                "seed {seed} W{}: every fort must have exactly one lock",
                wi + 1
            );
            assert_eq!(
                forts,
                (0..forts.len()).collect::<Vec<_>>(),
                "seed {seed} W{}: fort sections must be dense 0..n in slot order",
                wi + 1
            );
            for (li, lock) in built.locks.iter().enumerate() {
                assert!(
                    lock.fort_section < forts.len(),
                    "seed {seed} W{}: lock {li} points at missing fort {}",
                    wi + 1,
                    lock.fort_section
                );
                assert_eq!(
                    lock.secret_exit_safe,
                    state.completable_sealed(Some(li)),
                    "seed {seed} W{}: stale secret_exit_safe flag on lock {li}",
                    wi + 1
                );
            }
            any_safe |= built.locks.iter().any(|l| l.secret_exit_safe);
        }
        assert!(
            any_safe,
            "seed {seed}: no secret-exit-safe lock in any world"
        );
    }
}

/// Fort-removal watchdog: the Locks phase removes a fortress when NO lock
/// placement keeps the world completable ("expected rate ~0"). A removed
/// fort is deleted content (its fortress level leaves the game), so the
/// rate must actually BE ~0 — this census counts, over the full shaped
/// pipeline, every world whose final fort count fell short of its
/// allotment, plus removal events in any redeal attempt (the log keeps
/// every attempt's story). `CENSUS_SEEDS` scales (default 200).
#[test]
fn test_builder_fort_removal_census() {
    let Some(raw) = load_rom() else {
        eprintln!("ROM not found, skipping");
        return;
    };
    let seeds: u64 = std::env::var("CENSUS_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(200);
    let mut short_worlds = 0usize;
    let mut removal_events = 0usize;
    let mut worlds = 0usize;
    for seed in 0..seeds {
        let ctx = census_ctx(&raw, seed);
        for world_idx in 0..8 {
            let mut state = ctx.world(world_idx);
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            run_shaped_with_web_retries(&mut state, &mut rng);
            worlds += 1;
            if state.fort_count() < state.fort_budget {
                short_worlds += 1;
                eprintln!(
                    "seed {seed} W{}: shipped {} of {} allotted forts",
                    world_idx + 1,
                    state.fort_count(),
                    state.fort_budget,
                );
            }
            removal_events += state
                .log
                .iter()
                .flat_map(|r| r.actions.iter())
                .filter(|a| a.contains("REMOVED"))
                .count();
        }
    }
    println!(
        "fort removal census: {short_worlds} short worlds / {worlds}; {removal_events} removal events across all attempts"
    );
    // Invariant since the redeal screen gained the full-roster condition:
    // a removal may occur inside a failed attempt, but never ships.
    assert_eq!(short_worlds, 0, "worlds shipped short of their fort allotment");
}

/// TEMP: how often do locks land on bridge-class tiles (water bridge 0xB3,
/// drawbridges 0xB1/0xB2) today, and how often does a world have at least
/// one bridge tile available?
#[test]
fn test_builder_bridge_lock_rate() {
    let Some(raw) = load_rom() else {
        eprintln!("ROM not found, skipping");
        return;
    };
    let seeds: u64 = std::env::var("CENSUS_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(300);
    const BRIDGE: [u8; 3] = [0xB3, 0xB1, 0xB2];
    let mut bridge_locks = [0usize; 8];
    let mut total_locks = [0usize; 8];
    let mut worlds_with_bridge_tile = [0usize; 8];
    for seed in 0..seeds {
        let ctx = census_ctx(&raw, seed);
        for world_idx in 0..8 {
            let mut state = ctx.world(world_idx);
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            run_shaped_with_web_retries(&mut state, &mut rng);
            total_locks[world_idx] += state.locks.len();
            bridge_locks[world_idx] += state
                .locks
                .iter()
                .filter(|l| BRIDGE.contains(&l.replace_tile))
                .count();
            let has_bridge = (0..state.grid.rows()).any(|r| {
                (0..state.grid.cols).any(|c| BRIDGE.contains(&state.grid.get(r, c)))
            });
            if has_bridge {
                worlds_with_bridge_tile[world_idx] += 1;
            }
        }
    }
    println!("bridge-lock rate over {seeds} seeds (bridge tiles 0xB3/0xB1/0xB2):");
    for w in 0..8 {
        println!(
            "  W{}: {}/{} locks on bridges ({:.1}%), bridge tiles present in {}/{} seeds",
            w + 1,
            bridge_locks[w],
            total_locks[w],
            100.0 * bridge_locks[w] as f64 / total_locks[w].max(1) as f64,
            worlds_with_bridge_tile[w],
            seeds,
        );
    }
}

/// Pin the island anatomy the role model is calibrated on (user design
/// review 2026-08-01): vanilla orientation, base QOL map. A map edit or
/// walker change that silently shifts the decomposition fails here first.
#[test]
fn test_builder_island_roles() {
    let Some(raw) = load_rom() else {
        eprintln!("ROM not found, skipping");
        return;
    };
    use super::islands::IslandRole::{Corridor, Entry, Final, Routing, Utility};
    let rom = base_qol(&raw);
    let catalog = NodeCatalog::build(&rom, false);
    // Toad-house / hammer-bro shuffle ON: those tiles get picked up as
    // placeable blanks, which is the anatomy the role model was calibrated
    // on (the flags-off variant shrinks three W7 islands by one tile each).
    let pickup = pick_up(
        &rom,
        &catalog,
        PickupFlags {
            shuffle_spade_games: false,
            shuffle_toad_houses: true,
            shuffle_hammer_bros: true,
        },
    );
    let flags = BuildFlags {
        shuffle_toad_houses: true,
        eights_are_wild: false,
        shuffle_hammer_bros: true,
    };
    let sizes_roles = |world_idx: usize| {
        let state = from_pickup(&rom, &catalog, &pickup, world_idx, &flags);
        let (pocket, count) = super::islands::pocket_map(&state);
        let islands = super::islands::classify(&state, &pocket, count);
        let mut v: Vec<(usize, super::islands::IslandRole)> =
            islands.iter().map(|i| (i.size, i.role)).collect();
        v.sort_unstable_by_key(|&(n, _)| n);
        v
    };

    // W7 — the full taxonomy: troll pipe (1), fort/utility slot (2),
    // corridor (4), start (6 = start tile + 5 placeable), two routing
    // hubs (8, 9), final island with space (13).
    assert_eq!(
        sizes_roles(6),
        vec![
            (1, Utility),
            (2, Utility),
            (4, Corridor),
            (6, Entry),
            (8, Routing),
            (9, Routing),
            (13, Final),
        ],
    );
    // W5 — the spiral-tower split: twin big islands, roles taken by
    // start/target rather than size.
    let w5 = sizes_roles(4);
    assert_eq!(w5.len(), 2);
    assert!(w5.iter().any(|&(_, r)| r == Entry) && w5.iter().any(|&(_, r)| r == Final));
    // Single-island worlds: the role machinery has nothing to do.
    assert_eq!(sizes_roles(0).len(), 1);
    assert_eq!(sizes_roles(1).len(), 1);
    // W8 — fragmented but with a single big routing hub.
    let w8 = sizes_roles(7);
    assert_eq!(w8.len(), 5);
    assert_eq!(w8.iter().filter(|&&(_, r)| r == Routing).count(), 2);
}

/// TEMP DIAGNOSTIC (W7 linearity investigation): shaped-arm results for one
/// world, split by start position (i.e. SAS orientation), with walk-graph
/// geometry per seed — cycle rank (independent cycles in the final walk
/// graph; 0 = tree = no parallel routes possible), node count, start→goal
/// hops, and leftover blanks. `CENSUS_WORLD` (default 7), `CENSUS_SEEDS`
/// (default 200).
#[test]
fn test_builder_w7_probe() {
    let Some(raw) = load_rom() else {
        eprintln!("ROM not found, skipping");
        return;
    };
    let seeds: u64 = std::env::var("CENSUS_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);
    let world: usize = std::env::var("CENSUS_WORLD")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(7);
    let world_idx = world - 1;

    #[derive(Default)]
    struct T {
        n: usize,
        linear: usize,
        noalt: usize,
        c1: u64,
        routes: usize,
        uniq: f64,
        multi: usize,
        beta_sum: usize,
        beta_linear_sum: usize,
        beta_multi_sum: usize,
        beta_zero: usize,
        nodes_sum: usize,
        dist_sum: usize,
        blanks_sum: usize,
        renders: usize,
        pockets_sum: usize,
        max_pocket_sum: usize,
        cross_sum: usize,
        intra_sum: usize,
        doubled_sum: usize,
        pocket_cycles_sum: usize,
        pocket_cycles_linear_sum: usize,
        pocket_cycles_multi_sum: usize,
        trunk_blanks_sum: usize,
        trunk_levels_sum: usize,
        noalt_probe: usize,
        noalt_with_detour: usize,
        noalt_exclusive: usize,
        noalt_goldable: usize,
        start_mouths_sum: usize,
        start_mouths_linear_sum: usize,
        start_mouths_multi_sum: usize,
    }

    let mut arms: std::collections::BTreeMap<Pos, T> = std::collections::BTreeMap::new();

    for seed in 0..seeds {
        let ctx = census_ctx(&raw, seed);
        let mut state = ctx.world(world_idx);
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        run_shaped_with_web_retries(&mut state, &mut rng);

        let m = measure_world(&state);
        let built = state.to_built();
        let wide = analyze_route_choice(&built, SHAPING_SLACK);

        // Replicate route_choice::stage_grid: open breakable rocks, stamp slots.
        let mut grid = built.grid.clone();
        for r in 0..grid.rows() {
            for c in 0..grid.cols {
                let t = grid.get(r, c);
                for (closed, open) in [(0x51u8, 0x45u8), (0x52, 0x46)] {
                    if t == closed {
                        grid.set(r, c, open);
                    }
                }
            }
        }
        types::stamp_slots(&mut grid, &built.slots);
        let start = rom_data::find_start(&grid).expect("start");
        let target = find_target(&grid, world_idx).expect("target");
        let walk = walk_map(&grid, &built.pipe_pairs, Some(start), world_idx);

        // Undirected edge count: each symmetric edge is recorded once per
        // endpoint; the target sink never expands, so its edges appear once.
        let directed: usize = walk.edges.values().map(Vec::len).sum();
        let v = walk.nodes.len();
        let e = directed.div_ceil(2);
        let beta = (e + 1).saturating_sub(v);
        let dist = walk.distances.get(&target).copied().unwrap_or(999);

        // Pocket structure: connected components over WALK edges only
        // (path_pos Some — teleport edges excluded). Then classify each pipe
        // pair: cross-pocket (a mandatory chain link) vs intra-pocket (adds a
        // cycle the domination filter usually eats).
        let node_list: Vec<Pos> = walk.nodes.iter().copied().collect();
        let index: std::collections::HashMap<Pos, usize> =
            node_list.iter().enumerate().map(|(i, &p)| (p, i)).collect();
        let mut comp: Vec<usize> = (0..node_list.len()).collect();
        fn find(comp: &mut Vec<usize>, i: usize) -> usize {
            if comp[i] != i {
                let root = find(comp, comp[i]);
                comp[i] = root;
            }
            comp[i]
        }
        for (from, edges) in &walk.edges {
            for edge in edges {
                if edge.path_pos.is_some()
                    && let (Some(&a), Some(&b)) = (index.get(from), index.get(&edge.dest))
                {
                    let (ra, rb) = (find(&mut comp, a), find(&mut comp, b));
                    comp[ra] = rb;
                }
            }
        }
        let mut sizes: std::collections::HashMap<usize, usize> =
            std::collections::HashMap::new();
        for i in 0..node_list.len() {
            *sizes.entry(find(&mut comp, i)).or_default() += 1;
        }
        let pockets = sizes.len();
        let max_pocket = sizes.values().copied().max().unwrap_or(0);
        let mut cross = 0;
        let mut intra = 0;
        let mut pair_edges: std::collections::HashSet<(usize, usize)> =
            std::collections::HashSet::new();
        let mut doubled = 0;
        for &(a, b) in &built.pipe_pairs {
            if let (Some(&ia), Some(&ib)) = (index.get(&a), index.get(&b)) {
                let (ra, rb) = (find(&mut comp, ia), find(&mut comp, ib));
                if ra == rb {
                    intra += 1;
                } else {
                    cross += 1;
                    if !pair_edges.insert((ra.min(rb), ra.max(rb))) {
                        doubled += 1;
                    }
                }
            }
        }
        // Cycle rank of the pocket graph counting only DISTINCT pocket-pair
        // links — cycles that thread 3+ pockets (the rebalanceable kind).
        let pocket_cycles = (pair_edges.len() + 1).saturating_sub(sizes.len());

        // Pipe mouths that landed on the START island — the user's
        // shortcut-concentration question.
        let start_root = state
            .start
            .and_then(|s| index.get(&s).copied())
            .map(|i| find(&mut comp, i));
        let start_mouths = built
            .pipe_pairs
            .iter()
            .flat_map(|&(a, b)| [a, b])
            .filter(|p| {
                index.get(p).copied().map(|i| find(&mut comp, i)) == start_root
                    && start_root.is_some()
            })
            .count();

        let t = arms.entry(state.start.unwrap_or((0, 0))).or_default();
        t.start_mouths_sum += start_mouths;
        if m.routes_in_band < 2 {
            t.start_mouths_linear_sum += start_mouths;
        } else {
            t.start_mouths_multi_sum += start_mouths;
        }
        t.pockets_sum += pockets;
        t.max_pocket_sum += max_pocket;
        t.cross_sum += cross;
        t.intra_sum += intra;
        t.doubled_sum += doubled;
        t.pocket_cycles_sum += pocket_cycles;
        if m.routes_in_band < 2 {
            t.pocket_cycles_linear_sum += pocket_cycles;
        } else {
            t.pocket_cycles_multi_sum += pocket_cycles;
        }
        t.n += 1;
        t.c1 += u64::from(m.c1);
        t.routes += m.routes_in_band;
        if m.routes_in_band < 2 {
            t.linear += 1;
            t.beta_linear_sum += beta;
        } else {
            t.multi += 1;
            t.uniq += m.mean_exclusive_levels;
            t.beta_multi_sum += beta;
        }
        if wide.routes.len() < 2 {
            t.noalt += 1;
        }
        if beta == 0 {
            t.beta_zero += 1;
        }
        t.beta_sum += beta;
        t.nodes_sum += v;
        t.dist_sum += dist;
        t.blanks_sum += state.legal_blanks().len();
        // Free blanks the cheap route walks THROUGH — the only tiles
        // arm_balance / level_move can use to discriminate routes.
        if let Some(r) = m.rc.routes.first() {
            let path: HashSet<Pos> = r.path.iter().copied().collect();
            t.trunk_blanks_sum += state
                .legal_blanks()
                .iter()
                .filter(|p| path.contains(*p))
                .count();
            t.trunk_levels_sum += state
                .slots
                .iter()
                .filter(|s| s.kind == SlotKind::Level && path.contains(&s.pos))
                .count();
        }

        // Golden-lock feasibility on NOALT seeds: does a dominated detour
        // exist within the wide window, and does the cheap route's
        // exclusive stretch vs that detour contain a LOCKABLE tile? (If
        // yes, a lock on that edge would split the pair; if the stretch is
        // empty or unlockable, only new topology can help.)
        if wide.routes.len() < 2 {
            t.noalt_probe += 1;
            if let Some(d) = wide.detours.first() {
                t.noalt_with_detour += 1;
                let dpath: HashSet<Pos> = d.path.iter().copied().collect();
                let exclusive: Vec<Pos> = wide
                    .routes
                    .first()
                    .map(|r| {
                        r.path.iter().copied().filter(|p| !dpath.contains(p)).collect()
                    })
                    .unwrap_or_default();
                if !exclusive.is_empty() {
                    t.noalt_exclusive += 1;
                }
                if exclusive
                    .iter()
                    .any(|&(r, c)| LOCKABLE_TILES.contains(&grid.get(r, c)))
                {
                    t.noalt_goldable += 1;
                }
            }
        }

        // Render the first couple of NOALT worlds per arm for eyeballing,
        // with their shaping logs.
        if wide.routes.len() < 2 && t.renders < 2 {
            t.renders += 1;
            println!(
                "--- seed {seed} W{world} start {:?} C1 {} beta {beta} (NOALT) ---",
                state.start.unwrap_or((0, 0)),
                m.c1
            );
            println!("{}", route_choice::render_route_choice(&built, SHAPING_SLACK));
            for report in &state.log {
                if report.phase == "shaping" {
                    for action in &report.actions {
                        println!("    [{}] {action}", report.phase);
                    }
                }
            }
        }
    }

    println!("W{world} probe ({seeds} seeds, shaped arm, split by start pos):");
    println!("start      n  linear%  noalt%  C1    routes  uniq  beta  beta(lin)  beta(multi)  beta0%  nodes  dist  blanks  pockets  maxpkt  pipes(cross:intra)");
    for (start, t) in &arms {
        let n = t.n as f64;
        println!(
            "{:>7?} {:>4} {:>7.0}% {:>6.0}% {:>5.1} {:>6.2} {:>5.2} {:>5.2} {:>9.2} {:>11.2} {:>6.0}% {:>6.1} {:>5.1} {:>6.1}",
            start,
            t.n,
            100.0 * t.linear as f64 / n,
            100.0 * t.noalt as f64 / n,
            t.c1 as f64 / n,
            t.routes as f64 / n,
            t.uniq / t.multi.max(1) as f64,
            t.beta_sum as f64 / n,
            t.beta_linear_sum as f64 / t.linear.max(1) as f64,
            t.beta_multi_sum as f64 / t.multi.max(1) as f64,
            100.0 * t.beta_zero as f64 / n,
            t.nodes_sum as f64 / n,
            t.dist_sum as f64 / n,
            t.blanks_sum as f64 / n,
        );
        println!(
            "        trunk-blanks {:.2}  trunk-levels {:.2}  noalt breakdown: {} total / {} with detour / {} any-exclusive-tile / {} lockable-exclusive-edge",
            t.trunk_blanks_sum as f64 / n,
            t.trunk_levels_sum as f64 / n,
            t.noalt_probe,
            t.noalt_with_detour,
            t.noalt_exclusive,
            t.noalt_goldable,
        );
        println!(
            "        pockets {:.2}  max-pocket {:.1}  pipes cross {:.2} intra {:.2}  doubled {:.2}  pocket-cycles {:.2} (linear {:.2} / multi {:.2})",
            t.pockets_sum as f64 / n,
            t.max_pocket_sum as f64 / n,
            t.cross_sum as f64 / n,
            t.intra_sum as f64 / n,
            t.doubled_sum as f64 / n,
            t.pocket_cycles_sum as f64 / n,
            t.pocket_cycles_linear_sum as f64 / t.linear.max(1) as f64,
            t.pocket_cycles_multi_sum as f64 / t.multi.max(1) as f64,
        );
        println!(
            "        start-island pipe mouths {:.2} (linear {:.2} / multi {:.2})",
            t.start_mouths_sum as f64 / n,
            t.start_mouths_linear_sum as f64 / t.linear.max(1) as f64,
            t.start_mouths_multi_sum as f64 / t.multi.max(1) as f64,
        );
    }

    // One-time island inventory of the fresh (pre-placement) map: size,
    // whether it holds the start / target, and its bridge-tile count.
    let ctx = census_ctx(&raw, 0);
    let fresh = ctx.world(world_idx);
    let (pocket, count) = super::islands::pocket_map(&fresh);
    println!("fresh W{world} island inventory ({count} islands):");
    for id in 0..count {
        let members: Vec<Pos> = pocket
            .iter()
            .filter(|&(_, &p)| p == id)
            .map(|(&pos, _)| pos)
            .collect();
        let bridges = members
            .iter()
            .filter(|&&(r, c)| [0xB3u8, 0xB1, 0xB2].contains(&fresh.grid.get(r, c)))
            .count();
        let mut sorted = members.clone();
        sorted.sort_unstable();
        println!(
            "  island {id}: {} tiles, start={} target={} bridges={} members={:?}",
            members.len(),
            fresh.start.is_some_and(|s| pocket.get(&s) == Some(&id)),
            fresh.target.is_some_and(|t| pocket.get(&t) == Some(&id)),
            bridges,
            sorted,
        );
    }
}
