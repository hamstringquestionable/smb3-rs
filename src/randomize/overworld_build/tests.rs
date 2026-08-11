use super::*;
use super::capacity::{
    W8_HB_CAP, distribute_levels, prepare_capacities,
    redistribute_fortresses,
};



use super::types::stamp_slots;
use crate::rom::Rom;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

/// Shannon entropy (bits) of a count distribution summing to `total`.
fn shannon_entropy<'a>(counts: impl IntoIterator<Item = &'a u32>, total: f64) -> f64 {
    counts
        .into_iter()
        .map(|&c| {
            let p = c as f64 / total;
            -p * p.log2()
        })
        .sum()
}

/// Load the test ROM with the always-on pre-builder QoL patches applied —
/// the same map state production hands the builder. Diagnostics measured on
/// the raw ROM lie about connectivity: e.g. W2's secret-path rock (row 0)
/// walls off a pocket that production opens, turning W2's only pipe pair
/// into an island bridge in tests while production gets a spare pipe.
fn load_rom() -> Option<Rom> {
    let data = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
    let rom = Rom::from_bytes(&data).ok()?;
    Some(apply_qol_for_overworld(&rom))
}

/// Apply the QoL patches that the real pipeline runs before the overworld
/// builder (`randomizer.rs` pre-build block). These mutate the world-map
/// grid — rocks blocking pipe shortcuts, W3 drawbridge tiles, big-Q rooms,
/// the always-on W8 screen-3 water/bridge page — so the catalog must see
/// the post-patch state, not vanilla. (The W8 canoe edits are gated behind
/// `8s are Wild` and are NOT applied here.) Idempotent: `load_rom` already
/// applies it; explicit callers just re-write the same bytes.
fn apply_qol_for_overworld(rom: &Rom) -> Rom {
    apply_qol_variant(rom, false, false)
}

/// Arm-parameterized map QOL: the always-on patches plus optional
/// `more_hammer_rocks` / `8s are Wild` edits, in production order
/// (`randomize_inner` applies these before the builder reads the map).
fn apply_qol_variant(rom: &Rom, hammer_rocks: bool, eights_wild: bool) -> Rom {
    let mut out = rom.clone();
    super::super::qol::fix_w3_drawbridges(&mut out);
    super::super::qol::remove_rocks(&mut out);
    if hammer_rocks {
        super::super::qol::make_hammer_rocks(&mut out);
    }
    super::super::qol::apply_w1_shortcut(&mut out, hammer_rocks);
    super::super::qol::apply_w8_bridges(&mut out);
    if eights_wild {
        super::super::qol::apply_w8_canoe_and_paths(&mut out);
    }
    super::super::qol::fix_big_q_block_rooms(&mut out);
    out
}

/// Build `(catalog, pickup)` for one seed. When the `SAS` env var is set,
/// applies per-seed start↔airship swap before pickup runs, matching the
/// real pipeline in `randomizer.rs` when `swap_start_airship` is on.
fn build_catalog_pickup(rom: &Rom, seed: u64) -> (NodeCatalog, PickupResult) {
    let mut catalog = NodeCatalog::build(rom, false);
    if std::env::var("SAS").is_ok() {
        let mut swap_rng = ChaCha8Rng::seed_from_u64(seed);
        super::super::start_airship_swap::pick_swaps(&mut catalog, &mut swap_rng);
    }
    let pickup = super::super::overworld_pickup::pick_up(
        rom,
        &catalog,
        super::super::overworld_pickup::PickupFlags {
            shuffle_spade_games: true,
            shuffle_toad_houses: true,
            ..Default::default()
        },
    );
    (catalog, pickup)
}

/// Run `per_seed` for seeds 0..`seeds` across worker threads and return the
/// results in seed order. The census diagnostics are embarrassingly parallel
/// — each seed builds from a fresh RNG and catalog/pickup and only reads the
/// ROM — so the multi-minute serial 1000-seed loops drop to one core-share
/// of wall clock. Aggregation stays serial in each caller.
fn par_seeds<T: Send>(seeds: u64, per_seed: impl Fn(u64) -> T + Sync) -> Vec<T> {
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(seeds.max(1) as usize);
    let next = std::sync::atomic::AtomicU64::new(0);
    let mut results: Vec<Option<T>> =
        std::iter::repeat_with(|| None).take(seeds as usize).collect();
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut out: Vec<(u64, T)> = Vec::new();
                    loop {
                        let seed = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if seed >= seeds {
                            break;
                        }
                        out.push((seed, per_seed(seed)));
                    }
                    out
                })
            })
            .collect();
        for h in handles {
            for (seed, val) in h.join().unwrap() {
                results[seed as usize] = Some(val);
            }
        }
    });
    results.into_iter().map(|o| o.unwrap()).collect()
}

/// Census flag arms. Every census seed runs with start↔airship swap ON
/// (per-world 50/50, exactly as the real flag rolls it — so unswapped
/// worlds stay covered inside every arm), and the map-QOL arms split
/// 50% base / 25% `more hammer rocks` / 25% `8s are Wild` by seed.
#[derive(Clone, Copy, PartialEq)]
enum CensusArm {
    Base,
    HammerRocks,
    EightsWild,
}

fn census_arm(seed: u64) -> CensusArm {
    match seed % 4 {
        2 => CensusArm::HammerRocks,
        3 => CensusArm::EightsWild,
        _ => CensusArm::Base,
    }
}

/// One standard census build: fresh per-seed RNG + catalog/pickup over the
/// seed's flag arm (see [`CensusArm`]). Takes the RAW rom — QOL is applied
/// here, per arm. The shared parallel body of the censuses.
fn census_build(rom: &Rom, seed: u64) -> BuildResult {
    let arm = census_arm(seed);
    let rom = apply_qol_variant(
        rom,
        arm == CensusArm::HammerRocks,
        arm == CensusArm::EightsWild,
    );
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut catalog = NodeCatalog::build(&rom, false);
    let mut swap_rng = ChaCha8Rng::seed_from_u64(seed);
    super::super::start_airship_swap::pick_swaps(&mut catalog, &mut swap_rng);
    let pickup = super::super::overworld_pickup::pick_up(
        &rom,
        &catalog,
        super::super::overworld_pickup::PickupFlags {
            shuffle_spade_games: true,
            shuffle_toad_houses: true,
            ..Default::default()
        },
    );
    build(
        &rom,
        &OverworldData { pickup: &pickup, catalog: &catalog },
        &mut rng,
        BuildFlags {
            shuffle_toad_houses: true,
            eights_are_wild: arm == CensusArm::EightsWild,
            ..Default::default()
        },
    )
}

#[test]
fn test_fortress_redistribution() {
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    for _ in 0..100 {
        let counts = redistribute_fortresses(&mut rng);
        let total: usize = counts.iter().sum();
        assert_eq!(total, 17, "total fortresses must be 17");
        assert_eq!(counts[7], 4, "W8 must keep 4");
        for (w, &count) in counts[..7].iter().enumerate() {
            assert!((1..=3).contains(&count),
                "W{} got {count} forts, expected 1-3", w + 1);
        }
    }
}

/// Budget invariants of a finished build: no content may go missing. Run over
/// `CENSUS_SEEDS` seeds (default 25) through [`census_build`], so every seed
/// exercises a different flag arm rather than one hand-made `BuildFlags`.
///
/// The level-shortfall assert replaces `test_measure_shortfalls`, a 1000-seed
/// `#[ignore]`d printout that reported these same three counts and had to be
/// eyeballed (it read 0/1000 on all three at the time it was folded in here,
/// 2026-08-07). A number nobody runs is not a guarantee.
#[test]
fn test_build_all_worlds() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };
    let seeds: u64 = std::env::var("CENSUS_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(25);

    par_seeds(seeds, |seed| {
        let result = census_build(&rom, seed);
        assert_eq!(result.worlds.len(), 8, "seed {seed}");

        for built in &result.worlds {
            let wi = built.world_idx;
            let forts = built.slots.iter().filter(|s| s.kind == SlotKind::Fortress).count();
            let pipes = built.pipe_pairs.len();
            let locks = built.locks.len();

            // Forts: the allotment is a target, and the Locks phase may remove
            // an unlockable fort (rare). Every surviving fort keeps one lock.
            assert!(forts <= result.fort_counts[wi],
                "seed {seed} W{}: fort slots {forts} > allotted {}", wi + 1, result.fort_counts[wi]);
            // The full vanilla pipe budget is always spent (an unplaced pair is
            // deleted content — its transit levels vanish from the game).
            assert_eq!(pipes, VANILLA_PIPE_PAIRS[wi],
                "seed {seed} W{}: pipe pairs {pipes} != budget {}", wi + 1, VANILLA_PIPE_PAIRS[wi]);
            assert_eq!(locks, forts,
                "seed {seed} W{}: every fort must have a lock ({locks} locks, {forts} forts)",
                wi + 1);
        }

        let total_levels: usize = result.worlds.iter()
            .map(|b| b.slots.iter().filter(|s| s.kind == SlotKind::Level).count())
            .sum();
        let total_forts: usize = result.worlds.iter()
            .map(|b| b.slots.iter().filter(|s| s.kind == SlotKind::Fortress).count())
            .sum();
        // Every vanilla level must land somewhere: a level the builder fails to
        // place is content deleted from the game, invisible in a finished ROM.
        assert_eq!(total_levels, VANILLA_LEVEL_COUNT,
            "seed {seed}: total levels {total_levels} != {VANILLA_LEVEL_COUNT}");
        // 17 allotted; the Locks phase may remove an unlockable fort (rare).
        assert!(total_forts <= 17, "seed {seed}: total forts {total_forts} > 17");
        assert!(total_forts >= 15, "seed {seed}: total forts {total_forts} suspiciously low");
    });
}

/// Regression: the overworld builder must never strand a world's target
/// (airship/Bowser) — that would be an unbeatable world. Covers SAS on/off,
/// hammer-bro shuffle on/off, and both the raw ROM and the QoL-patched ROM.
/// The raw-ROM arm (path rocks still present) locks in that the pipe
/// island-connect logic recovers connectivity even when a rock blocks a
/// path, so this can't regress if the rock-removal QoL ever changes.
///
/// `CENSUS_SEEDS` seeds per arm (default 20 = 160 builds, 1280 world-checks).
/// The **arms** are what this test buys, not seed depth: eight genuinely
/// different code paths, and every build already samples all 8 worlds. Seed
/// depth was cut from 40 to 20 on evidence rather than taste — a one-off run
/// at 500/arm (4000 builds, 32000 world-checks, 2026-08-07) found zero
/// stranded targets, matching the 10000-build result from the express-routing
/// work. The failure this guards is a logic regression that strands a sizeable
/// fraction of seeds, which shows up in the first handful; a rare
/// topology-conditional one was never reliably caught at 40 either.
///
/// **Run it deep after touching overworld logic, and before a release** — a
/// stranding bug that needs a rare pipe layout will pass at 20 seeds and fail
/// a player. It is one env var (see CLAUDE.md's pre-commit section):
///   CENSUS_SEEDS=500 cargo test --release --lib \
///     all_world_targets_reachable        # ~4 min
#[test]
fn all_world_targets_reachable() {
    let raw = match load_rom() {
        Some(r) => r,
        None => return,
    };
    let qol = apply_qol_for_overworld(&raw);
    let names = ["W1", "W2", "W3", "W4", "W5", "W6", "W7", "W8"];
    let seeds: u64 = std::env::var("CENSUS_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);

    for (rom_label, rom) in [("raw", &raw), ("qol", &qol)] {
        for hb in [false, true] {
            for sas in [false, true] {
                for seed in 0..seeds {
                    let mut catalog = NodeCatalog::build(rom, false);
                    let mut rng = ChaCha8Rng::seed_from_u64(seed);
                    if sas {
                        super::super::start_airship_swap::pick_swaps(&mut catalog, &mut rng);
                    }
                    let pickup = super::super::overworld_pickup::pick_up(
                        rom,
                        &catalog,
                        super::super::overworld_pickup::PickupFlags {
                            shuffle_spade_games: true,
                            shuffle_toad_houses: true,
                            shuffle_hammer_bros: hb,
                        },
                    );
                    let result = build(
                        rom,
                        &OverworldData { pickup: &pickup, catalog: &catalog },
                        &mut rng,
                        BuildFlags {
                            shuffle_toad_houses: true,
                            shuffle_hammer_bros: hb,
                            ..Default::default()
                        },
                    );
                    for built in &result.worlds {
                        let wi = built.world_idx;
                        let start = rom_data::find_start(&built.grid);
                        if let Some(t) = find_target(&built.grid, wi) {
                            assert!(
                                walk_map(&built.grid, &built.pipe_pairs, start, wi)
                                    .nodes
                                    .contains(&t),
                                "{rom_label} hb={hb} sas={sas} seed={seed}: \
                                 {} target unreachable from start",
                                names[wi],
                            );
                        }
                    }
                }
            }
        }
    }
}

/// Tuning diagnostic: sweep the level-spread exponent and show the resulting
/// per-world mean assigned-level count, plus how often we hit "overflow" —
/// a world's fair share exceeding its hard capacity (clamp), or the total
/// not fully placeable (underfill). Uses the same capacity + distribution
/// code as production. Run with:
///   cargo test --lib report_distribution_by_exponent -- --nocapture
///
/// `#[ignore]`d (2026-08-07): a baseline instrument, not a gate — it
/// carries no assertions, so running it in CI only spent time. The
/// censuses that DO assert stay in CI.
#[test]
#[ignore]
fn report_distribution_by_exponent() {
    let raw = match load_rom() {
        Some(r) => r,
        None => return,
    };
    let rom = apply_qol_for_overworld(&raw);
    let (catalog, pickup) = build_catalog_pickup(&rom, 0);
    let mut vanilla = [0usize; 8];
    for e in &catalog.entries {
        if matches!(e.kind, NodeKind::Level) {
            vanilla[e.world_idx] += 1;
        }
    }

    const SEEDS: u64 = 300;
    let names = ["W1", "W2", "W3", "W4", "W5", "W6", "W7", "W8"];
    let header: String = names.iter().map(|n| format!("{n:>6}")).collect();
    eprintln!("\nLevel distribution by exponent ({SEEDS} seeds, mean assigned per world):");
    eprintln!("  exp  {header}");
    let van: String = vanilla.iter().map(|v| format!("{v:>6}")).collect();
    eprintln!("  van  {van}");

    for &exp in &[1.0_f64, 0.7, 0.6, 0.5, 0.4] {
        let mut sums = [0usize; 8];
        let mut clamp_events = 0usize; // (seed,world) share floored > capacity
        let mut underfill_seeds = 0usize; // seeds where total placed < 62
        for seed in 0..SEEDS {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let fort_counts = redistribute_fortresses(&mut rng);
            let caps = prepare_capacities(&rom, &catalog, &pickup, &fort_counts, false, true, false)
                .capacities;

            // Detect clamp events (a world's fair share exceeds its capacity).
            let weights: [f64; 8] = std::array::from_fn(|wi| {
                if caps[wi] == 0 { 0.0 } else { (caps[wi] as f64).powf(exp) }
            });
            let tw: f64 = weights.iter().sum();
            for wi in 0..8 {
                let share = weights[wi] / tw * VANILLA_LEVEL_COUNT as f64;
                if share.floor() as usize > caps[wi] {
                    clamp_events += 1;
                }
            }

            let counts = distribute_levels(&caps, VANILLA_LEVEL_COUNT, exp, &mut rng);
            if counts.iter().sum::<usize>() < VANILLA_LEVEL_COUNT {
                underfill_seeds += 1;
            }
            for wi in 0..8 {
                sums[wi] += counts[wi];
            }
        }
        let row: String = (0..8)
            .map(|wi| format!("{:>6.1}", sums[wi] as f64 / SEEDS as f64))
            .collect();
        eprintln!("  {exp:<3}  {row}    clamp_events={clamp_events} underfill_seeds={underfill_seeds}");
    }
}

#[test]
fn hammer_bro_redistribution_invariants() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };
    for seed in 0..32u64 {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let catalog = NodeCatalog::build(&rom, false);
        let pickup = super::super::overworld_pickup::pick_up(
            &rom,
            &catalog,
            super::super::overworld_pickup::PickupFlags {
                shuffle_spade_games: true,
                shuffle_toad_houses: true,
                shuffle_hammer_bros: true,
            },
        );
        let result = build(
            &rom,
            &OverworldData { pickup: &pickup, catalog: &catalog },
            &mut rng,
            BuildFlags { shuffle_toad_houses: true, shuffle_hammer_bros: true, ..Default::default() },
        );

        // The vanilla 15 encounters are always placed (W1-W6 alone have the
        // capacity), spread across the worlds.
        let total: usize = result.worlds.iter().map(|w| w.hb_sprites.len()).sum();
        assert_eq!(total, 15, "seed {seed}: total HB sprites {total} != 15");

        for w in &result.worlds {
            let n = w.hb_sprites.len();
            // Best-effort 1-3 per world (W8 capped lower); a feature-dense
            // world (e.g. W7's 8 pipe pairs) can be left with no spare
            // HammerBro tile and get 0, with its share spilling elsewhere.
            let max = if w.world_idx == 7 { W8_HB_CAP } else { 3 };
            assert!(
                n <= max,
                "seed {seed} W{}: {n} HB sprites (max {max})", w.world_idx + 1
            );
            // At least RESERVED_DYNAMIC_SLOTS eligible map-object slots stay
            // free for a runtime white-house spawn.
            let eligible = rom_data::eligible_hb_map_slots(&rom, w.world_idx).len();
            assert!(
                eligible.saturating_sub(n) >= RESERVED_DYNAMIC_SLOTS,
                "seed {seed} W{}: only {} eligible slots free after {n} HBs",
                w.world_idx + 1, eligible.saturating_sub(n)
            );
            // Every sprite sits on one of this world's HammerBro slot tiles.
            let hb_tiles: HashSet<(usize, usize)> = w
                .slots
                .iter()
                .filter(|s| s.kind == SlotKind::HammerBro)
                .map(|s| s.pos)
                .collect();
            let mut seen: HashSet<(usize, usize)> = HashSet::new();
            for sprite in &w.hb_sprites {
                assert!(
                    hb_tiles.contains(&sprite.grid_pos),
                    "seed {seed} W{}: HB sprite at {:?} is not a HammerBro slot",
                    w.world_idx + 1, sprite.grid_pos
                );
                assert!(
                    seen.insert(sprite.grid_pos),
                    "seed {seed} W{}: duplicate HB sprite position {:?}",
                    w.world_idx + 1, sprite.grid_pos
                );
            }
        }
    }
}



/// Per-world topology snapshot for the goal-open probes. Distilled from the
/// `progression_metrics` logic: same lock-state model (grid holds locks
/// open; close by stamping gap tiles), same walk_map traversal. (The parked
/// plan-sweep harness carried richer fields — mandatory/forts_at_start/
/// fort_skip — restore them from the backup branch when a census consumer
/// returns.)
struct WorldTopology {
    fort_count: usize,
    /// Rounds of "beat every reachable fort" until the goal opens (with
    /// pipes). 0 = goal open at start; 99 = infeasible (never observed).
    depth: usize,
}

fn world_topology(built: &BuiltWorld) -> Option<WorldTopology> {
    let wi = built.world_idx;
    let mut base = built.grid.clone();
    stamp_slots(&mut base, &built.slots);
    let start = rom_data::find_start(&base)?;
    let target = find_target(&base, wi)?;
    let forts: Vec<(usize, Pos)> = built
        .slots
        .iter()
        .filter(|s| s.kind == SlotKind::Fortress)
        .map(|s| (s.section, s.pos))
        .collect();
    if forts.is_empty() {
        return None;
    }
    let all_secs: HashSet<usize> = built.locks.iter().map(|l| l.fort_section).collect();
    let grid_with = |opened: &HashSet<usize>| -> Grid {
        let mut g = base.clone();
        for l in &built.locks {
            if opened.contains(&l.fort_section) {
                g.set(l.pos.0, l.pos.1, l.replace_tile);
            } else {
                g.set(l.pos.0, l.pos.1, l.gap_tile);
            }
        }
        g
    };

    // Chain depth: rounds of beat-all-reachable until the goal opens.
    let mut opened: HashSet<usize> = HashSet::new();
    let mut depth = 0usize;
    loop {
        let g = grid_with(&opened);
        let walk = walk_map(&g, &built.pipe_pairs, Some(start), wi);
        if walk.nodes.contains(&target) {
            break;
        }
        let newly: Vec<usize> = forts
            .iter()
            .filter(|(sec, pos)| {
                walk.nodes.contains(pos) && all_secs.contains(sec) && !opened.contains(sec)
            })
            .map(|(sec, _)| *sec)
            .collect();
        if newly.is_empty() {
            depth = 99;
            break;
        }
        opened.extend(newly);
        depth += 1;
    }

    Some(WorldTopology { fort_count: forts.len(), depth })
}

/// Baseline metrics for the lock/fort progression-topology work. Emits, over
/// many seeds:
///   Problem 2 (chain topology):
///     - chain depth: rounds of "beat every reachable fort" until the airship
///       is reachable (0 = goal open at start, 2 = the deep-chain pattern)
///     - a topology census bucketing each world by (depth, mandatory-fort
///       count, forts-accessible-at-start)
///   Problem 1 (fort/lock cramping):
///     - Manhattan distance from each fort to the lock it opens (% <=2 tiles)
///     - the fort-side component size when only that fort's own lock is closed
///       (small = fort stranded on a tiny island with its own gate)
/// Run with:
///   cargo test --lib progression_metrics -- --ignored --nocapture
#[test]
#[ignore]
fn progression_metrics() {
    // RAW rom: `census_build` applies the map QOL itself, per flag arm.
    let rom_bytes = match std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") {
        Ok(b) => b,
        Err(_) => {
            eprintln!("ROM not found, skipping");
            return;
        }
    };
    let rom = match Rom::from_bytes(&rom_bytes) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("ROM parse failed: {e}");
            return;
        }
    };

    let seeds: u64 = std::env::var("CENSUS_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);

    // --- Problem 2 accumulators ---
    let mut depth_hist = [0u64; 8]; // index = chain depth (capped at 7)
    let mut depth_by_world = [(0u64, 0u64); 8]; // (sum, count) per world
    // Topology buckets.
    let mut b_goal_open = 0u64;
    let mut b_single_gate = 0u64; // depth 1, 1 mandatory, 1 fort at start
    let mut b_single_plus_optional = 0u64; // depth 1, 1 mandatory, >=2 at start
    let mut b_either_key = 0u64; // depth 1, 0 mandatory, >=2 at start
    let mut b_parallel_and = 0u64; // depth 1, >=2 mandatory
    let mut b_chain = 0u64; // depth >= 2
    let mut b_other = 0u64;
    let mut census_total = 0u64; // 2+-fort world-instances only
    let mut single_fort_worlds = 0u64; // 1-fort worlds excluded from census
    let mut chain_by_world = [(0u64, 0u64); 8]; // (chain, total) among 2+-fort worlds

    // --- Problem 1 accumulators ---
    let mut man_hist = [0u64; 12]; // Manhattan dist fort->own lock (capped at 11)
    let mut man_total = 0u64;
    let mut man_le2 = 0u64;
    let mut comp_hist = [0u64; 12]; // fort-side component size (capped at 11)
    let mut comp_total = 0u64;

    for seed in 0..seeds {
        let result = census_build(&rom, seed);

        for built in &result.worlds {
            let wi = built.world_idx;
            // Base grid = pipes + slots stamped, all lock tiles left OPEN
            // (built.grid holds each lock position as its original path tile).
            let mut base = built.grid.clone();
            stamp_slots(&mut base, &built.slots);

            let forts: Vec<Pos> = built
                .slots
                .iter()
                .filter(|s| s.kind == SlotKind::Fortress)
                .map(|s| s.pos)
                .collect();
            if forts.is_empty() {
                continue;
            }
            let start = rom_data::find_start(&base);
            let target = find_target(&base, wi);

            // Build a grid with `opened` fort-sections' locks restored (open)
            // and every other lock closed (gapped).
            let grid_with = |opened: &HashSet<usize>| -> Grid {
                let mut g = base.clone();
                for l in &built.locks {
                    if opened.contains(&l.fort_section) {
                        g.set(l.pos.0, l.pos.1, l.replace_tile);
                    } else {
                        g.set(l.pos.0, l.pos.1, l.gap_tile);
                    }
                }
                g
            };
            let all_lock_sections: HashSet<usize> =
                built.locks.iter().map(|l| l.fort_section).collect();

            // ---- Problem 2: chain depth (round-count) ----
            if let Some(tgt) = target {
                let mut opened: HashSet<usize> = HashSet::new();
                let mut depth = 0usize;
                let mut infeasible = false;
                loop {
                    let g = grid_with(&opened);
                    let walk = walk_map(&g, &built.pipe_pairs, start, wi);
                    if walk.nodes.contains(&tgt) {
                        break;
                    }
                    // Beat every reachable, not-yet-opened fort this round.
                    let newly: Vec<usize> = built
                        .slots
                        .iter()
                        .filter(|s| s.kind == SlotKind::Fortress && walk.nodes.contains(&s.pos))
                        .map(|s| s.section)
                        .filter(|sec| all_lock_sections.contains(sec) && !opened.contains(sec))
                        .collect();
                    if newly.is_empty() {
                        infeasible = true;
                        break;
                    }
                    for sec in newly {
                        opened.insert(sec);
                    }
                    depth += 1;
                }

                if !infeasible {
                    depth_hist[depth.min(7)] += 1;
                    depth_by_world[wi].0 += depth as u64;
                    depth_by_world[wi].1 += 1;

                    // Mandatory forts: goal unreachable if this one stays closed
                    // while all other locks are open.
                    let mut mandatory = 0usize;
                    for &sec in &all_lock_sections {
                        let mut opened_but_one = all_lock_sections.clone();
                        opened_but_one.remove(&sec);
                        let g = grid_with(&opened_but_one);
                        let walk = walk_map(&g, &built.pipe_pairs, start, wi);
                        if !walk.nodes.contains(&tgt) {
                            mandatory += 1;
                        }
                    }

                    // Forts accessible at start (all locks closed).
                    let g0 = grid_with(&HashSet::new());
                    let walk0 = walk_map(&g0, &built.pipe_pairs, start, wi);
                    let at_start =
                        forts.iter().filter(|p| walk0.nodes.contains(*p)).count();

                    // Census only worlds that CAN chain (2+ forts); single-fort
                    // worlds are trivially single-gate and would dilute the rate.
                    if forts.len() < 2 {
                        single_fort_worlds += 1;
                    } else {
                        census_total += 1;
                        let is_chain = depth >= 2;
                        chain_by_world[wi].0 += is_chain as u64;
                        chain_by_world[wi].1 += 1;
                        if depth == 0 {
                            b_goal_open += 1;
                        } else if is_chain {
                            b_chain += 1;
                        } else if mandatory >= 2 {
                            b_parallel_and += 1;
                        } else if mandatory == 1 {
                            if at_start >= 2 {
                                b_single_plus_optional += 1;
                            } else {
                                b_single_gate += 1;
                            }
                        } else if mandatory == 0 && at_start >= 2 {
                            b_either_key += 1;
                        } else {
                            b_other += 1;
                        }
                    }
                }
            }

            // ---- Problem 1: fort <-> own-lock proximity & cramping ----
            for l in &built.locks {
                // The fort this lock opens.
                let fort_pos = built.slots.iter().find(|s| {
                    s.kind == SlotKind::Fortress && s.section == l.fort_section
                });
                let Some(fp) = fort_pos.map(|s| s.pos) else { continue };

                let man = fp.0.abs_diff(l.pos.0) + fp.1.abs_diff(l.pos.1);
                man_hist[man.min(11)] += 1;
                man_total += 1;
                if man <= 2 {
                    man_le2 += 1;
                }

                // Fort-side component: only this lock closed, all others open;
                // walk from the fort. Small => fort stuck with its own gate.
                let mut opened = all_lock_sections.clone();
                opened.remove(&l.fort_section);
                let g = grid_with(&opened);
                let walk = walk_map(&g, &built.pipe_pairs, Some(fp), wi);
                comp_hist[walk.nodes.len().min(11)] += 1;
                comp_total += 1;
            }
        }
    }

    let names = ["W1", "W2", "W3", "W4", "W5", "W6", "W7", "W8"];

    eprintln!("\n=== Problem 2: chain depth (rounds of beat-all-reachable to open airship), {seeds} seeds ===");
    let dtot: u64 = depth_hist.iter().sum();
    for (d, &c) in depth_hist.iter().enumerate() {
        if c == 0 {
            continue;
        }
        let label = if d == 7 { "7+".to_string() } else { d.to_string() };
        eprintln!("  depth {label}: {c:6} ({:5.1}%)", 100.0 * c as f64 / dtot as f64);
    }
    eprint!("  mean depth per world: ");
    for wi in 0..8 {
        let (s, c) = depth_by_world[wi];
        if c > 0 {
            eprint!("{}={:.2}  ", names[wi], s as f64 / c as f64);
        }
    }
    eprintln!();

    eprintln!(
        "\n=== Problem 2: topology census ({census_total} worlds w/ 2+ forts; \
         {single_fort_worlds} single-fort worlds excluded) ===");
    let pc = |c: u64| 100.0 * c as f64 / census_total as f64;
    eprintln!("  goal open at start        : {b_goal_open:6} ({:5.1}%)", pc(b_goal_open));
    eprintln!("  single gate (1 req, 1 open): {b_single_gate:6} ({:5.1}%)", pc(b_single_gate));
    eprintln!("  single gate + optional fort: {b_single_plus_optional:6} ({:5.1}%)", pc(b_single_plus_optional));
    eprintln!("  either-key choice (any opens): {b_either_key:6} ({:5.1}%)", pc(b_either_key));
    eprintln!("  parallel-AND (all req, reachable together): {b_parallel_and:6} ({:5.1}%)", pc(b_parallel_and));
    eprintln!("  CHAIN (depth >= 2)        : {b_chain:6} ({:5.1}%)", pc(b_chain));
    eprintln!("  other                     : {b_other:6} ({:5.1}%)", pc(b_other));
    eprint!("  chain rate per world (2+ forts only): ");
    for wi in 0..8 {
        let (ch, tot) = chain_by_world[wi];
        if tot > 0 {
            eprint!("{}={:.0}%  ", names[wi], 100.0 * ch as f64 / tot as f64);
        }
    }
    eprintln!();

    eprintln!("\n=== Problem 1: Manhattan distance fort -> its own lock ({man_total} locks) ===");
    for (d, &c) in man_hist.iter().enumerate() {
        if c == 0 {
            continue;
        }
        let label = if d == 11 { "11+".to_string() } else { d.to_string() };
        eprintln!("  dist {label:>3}: {c:6} ({:5.1}%)", 100.0 * c as f64 / man_total as f64);
    }
    eprintln!("  <=2 tiles (immediately behind): {man_le2} ({:5.1}%)", 100.0 * man_le2 as f64 / man_total as f64);

    eprintln!("\n=== Problem 1: fort-side component size when its own lock is closed ({comp_total} forts) ===");
    for (sz, &c) in comp_hist.iter().enumerate() {
        if c == 0 {
            continue;
        }
        let label = if sz == 11 { "11+".to_string() } else { sz.to_string() };
        eprintln!("  {label:>3} nodes: {c:6} ({:5.1}%)", 100.0 * c as f64 / comp_total as f64);
    }
}

/// Distribution analyzer for fortress placement.
///
/// Runs the builder for N seeds and reports, per world:
///   - unique fortress positions and Shannon entropy (bits)
///   - top-5 most-picked positions
///   - per-section breakdown (each section places exactly one fortress)
///
/// Use the entropy number to compare scoring tweaks: higher = more variety.
/// Run with: cargo test --release test_fortress_distribution -- --ignored --nocapture
/// Override seed count with CENSUS_SEEDS=N.
#[test]
#[ignore]
fn test_fortress_distribution() {
    let rom = match load_rom() {
        Some(r) => r,
        None => {
            eprintln!("ROM not found, skipping");
            return;
        }
    };

    let seeds: u64 = std::env::var("CENSUS_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);

    // Per-world tallies
    let mut world_counts: [HashMap<(usize, usize), u32>; 8] = Default::default();
    let mut world_total = [0u32; 8];
    // Per-section tallies: [world][section] -> position frequency
    let mut section_counts: [Vec<HashMap<(usize, usize), u32>>; 8] = Default::default();
    let mut section_total: [Vec<u32>; 8] = Default::default();

    for result in &par_seeds(seeds, |seed| census_build(&rom, seed)) {
        for built in &result.worlds {
            let wi = built.world_idx;

            // Grow per-section storage to match observed section_count.
            if section_counts[wi].len() < built.section_count {
                section_counts[wi].resize(built.section_count, HashMap::new());
                section_total[wi].resize(built.section_count, 0);
            }

            for slot in &built.slots {
                if slot.kind != SlotKind::Fortress {
                    continue;
                }
                *world_counts[wi].entry(slot.pos).or_insert(0) += 1;
                world_total[wi] += 1;

                if slot.section < section_counts[wi].len() {
                    *section_counts[wi][slot.section].entry(slot.pos).or_insert(0) += 1;
                    section_total[wi][slot.section] += 1;
                }
            }
        }
    }

    eprintln!("\n=== Fortress Distribution over {seeds} seeds ===");

    for wi in 0..8 {
        let counts = &world_counts[wi];
        let total = world_total[wi];
        if total == 0 {
            continue;
        }
        let total_f = total as f64;

        let entropy = shannon_entropy(counts.values(), total_f);
        let max_entropy = (counts.len() as f64).log2();
        let forts_per_seed = total as f64 / seeds as f64;

        eprintln!(
            "\n--- W{} ({:.0} fort{}/seed) ---",
            wi + 1,
            forts_per_seed,
            if forts_per_seed == 1.0 { "" } else { "s" },
        );
        eprintln!(
            "  Positions: {} unique  |  entropy {:.2} / {:.2} bits ({:.0}%)",
            counts.len(),
            entropy,
            max_entropy,
            if max_entropy > 0.0 { entropy / max_entropy * 100.0 } else { 0.0 },
        );

        let mut sorted: Vec<_> = counts.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1));
        eprintln!("  Top positions:");
        for (pos, count) in sorted.iter().take(5) {
            let count = **count;
            let pct = count as f64 / total_f * 100.0;
            let bar = "#".repeat((pct as usize).min(40));
            eprintln!(
                "    ({:2},{:2})  {:>5} ({:5.1}%)  {bar}",
                pos.0, pos.1, count, pct,
            );
        }

        // Per-section breakdown
        for (si, sec_counts) in section_counts[wi].iter().enumerate() {
            if sec_counts.is_empty() {
                continue;
            }
            let sec_total = section_total[wi][si] as f64;
            let sec_entropy = shannon_entropy(sec_counts.values(), sec_total);
            let sec_max = (sec_counts.len() as f64).log2();
            let mut sec_sorted: Vec<_> = sec_counts.iter().collect();
            sec_sorted.sort_by(|a, b| b.1.cmp(a.1));
            let top: Vec<String> = sec_sorted
                .iter()
                .take(3)
                .map(|(p, c)| {
                    let pct = **c as f64 / sec_total * 100.0;
                    format!("({},{})={:.0}%", p.0, p.1, pct)
                })
                .collect();
            eprintln!(
                "    Section {si}: {} unique, entropy {:.2}/{:.2} bits, top: {}",
                sec_counts.len(),
                sec_entropy,
                sec_max,
                top.join("  "),
            );
        }
    }

    eprintln!("\n=== Fortress entropy summary (bits) ===");
    let summary: Vec<String> = (0..8)
        .filter(|&wi| world_total[wi] > 0)
        .map(|wi| {
            let entropy = shannon_entropy(world_counts[wi].values(), world_total[wi] as f64);
            format!("W{}={entropy:.2}", wi + 1)
        })
        .collect();
    eprintln!("  {}", summary.join("  "));

    // Sanity: 17 fortresses per seed total
    let grand_total: u32 = world_total.iter().sum();
    let expected = 17 * seeds as u32;
    eprintln!("\nGrand total: {grand_total} fortresses across {seeds} seeds (expected {expected})");
    assert_eq!(grand_total, expected, "fortress count invariant broken");
}

/// Regression: under start↔airship swap, the W3 fixed pipe used to be
/// paired with a random opposite-side blank, which could land on a dead
/// canoe island and strand the start — leaving the airship unreachable
/// (~1.6% of SAS seeds). `place_pipes` now biases the fixed pipe's partner
/// toward a blank that actually reconnects start to target. These seeds all
/// failed before that fix; they must stay reachable.
#[test]
fn test_sas_w3_fixed_pipe_keeps_target_reachable() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return, // ROM not present in this environment; skip.
    };
    let rom = apply_qol_for_overworld(&rom);
    // Previously-unreachable SAS W3 seeds (from the SAS=1 progression sweep).
    for seed in [123u64, 385, 515, 559, 629] {
        let mut catalog = NodeCatalog::build(&rom, false);
        let mut swap_rng = ChaCha8Rng::seed_from_u64(seed);
        super::super::start_airship_swap::pick_swaps(&mut catalog, &mut swap_rng);
        let pickup = super::super::overworld_pickup::pick_up(
            &rom,
            &catalog,
            super::super::overworld_pickup::PickupFlags {
                shuffle_spade_games: true,
                shuffle_toad_houses: true,
                ..Default::default()
            },
        );
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let result = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, BuildFlags { shuffle_toad_houses: true, ..Default::default() });
        let w3 = result.worlds.iter().find(|b| b.world_idx == 2).unwrap();
        assert!(
            analyze_required_progression(w3, false).reachable,
            "SAS seed {seed}: W3 airship must be reachable",
        );
    }
}

/// Per-world linearity accumulators for one measurement pass.
#[derive(Clone, Default)]
struct ProgLinWorld {
    reachable: u32,
    unreachable: u32,
    sum_clears: u64,
    sum_streak: u64,
    max_streak: usize,
    streak_ge2: u32,
    streak_ge3: u32,
    sum_goal: u64,
    goal_ge2: u32,
    sum_adj: u64,
    // Pipe classification (summed over seeds): counts per class + total
    // forced clears skipped by shortcut pipes.
    sum_conn: u64,
    sum_shortcut: u64,
    sum_access: u64,
    sum_scenic: u64,
    sum_redundant: u64,
    sum_pipe_skip: u64,
    // Start→goal express pipe: how often a single pipe bridges start-island
    // straight to goal-island. express_total = seeds where it's applicable
    // (start and goal on different islands).
    sum_express: u64,
    sum_express_total: u64,
    sum_islands: u64,
    sum_rock_skip: u64,
}

/// Run `seeds` full-pipeline builds under `options` and accumulate per-world
/// linearity stats. Uses `randomize_rom_with_overworld_capture` so the maps
/// are exactly what the CLI/web ships (not a direct `build()`, whose RNG
/// state differs from a real run).
fn prog_measure_pass(rom_bytes: &[u8], options: &crate::Options, seeds: u64) -> [ProgLinWorld; 8] {
    let mut worlds: [ProgLinWorld; 8] = std::array::from_fn(|_| ProgLinWorld::default());
    // Full-pipeline builds fan out; the (much cheaper) progression analysis
    // below stays serial. The dropped ROM is rebuilt per seed in-thread.
    let results = par_seeds(seeds, |seed| {
        match crate::randomize_rom_with_overworld_capture(rom_bytes, seed, options, None) {
            Ok((_rom, result)) => Some(result),
            Err(e) => {
                eprintln!("seed {seed}: randomize failed: {e}");
                None
            }
        }
    });
    for result in results.iter().flatten() {
        for built in &result.worlds {
            let w = &mut worlds[built.world_idx];
            let nh = analyze_required_progression(built, false);
            if !nh.reachable {
                w.unreachable += 1;
                continue;
            }
            w.reachable += 1;
            w.sum_clears += (nh.forts_required + nh.levels_required) as u64;
            w.sum_streak += nh.level_streak as u64;
            w.max_streak = w.max_streak.max(nh.level_streak);
            if nh.level_streak >= 2 {
                w.streak_ge2 += 1;
            }
            if nh.level_streak >= 3 {
                w.streak_ge3 += 1;
            }
            w.sum_goal += nh.goal_stack as u64;
            if nh.goal_stack >= 2 {
                w.goal_ge2 += 1;
            }
            w.sum_adj += level_adjacency_pairs(built) as u64;
            w.sum_rock_skip += hammer_skip(built) as u64;
            for class in classify_pipes(built) {
                match class {
                    PipeClass::Connectivity => w.sum_conn += 1,
                    PipeClass::Shortcut(n) => {
                        w.sum_shortcut += 1;
                        w.sum_pipe_skip += n as u64;
                    }
                    PipeClass::ContentAccess => w.sum_access += 1,
                    PipeClass::Scenic => w.sum_scenic += 1,
                    PipeClass::Redundant => w.sum_redundant += 1,
                }
            }
            w.sum_islands += island_count(built) as u64;
            if let Some(direct) = start_goal_express_pipe(built) {
                w.sum_express_total += 1;
                if direct {
                    w.sum_express += 1;
                }
            }
        }
    }
    worlds
}

/// Print one pass's per-world linearity + pipe-role tables.
fn prog_print_pass(label: &str, worlds: &[ProgLinWorld; 8]) {
    eprintln!("\n  [{label}]");
    eprintln!(
        "    {:<4} {:>7} {:>7} {:>5} {:>5} {:>8} {:>6} {:>10}",
        "", "clears", "streak", "≥2", "≥3", "goalstk", "adj", "rock-skip",
    );
    let (mut t_reach, mut t_streak, mut t_ge2, mut t_ge3, mut t_goal2, mut t_rock) =
        (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
    for (wi, w) in worlds.iter().enumerate() {
        if w.reachable == 0 {
            eprintln!("    W{}: (no reachable seeds)", wi + 1);
            continue;
        }
        let r = w.reachable as f64;
        eprintln!(
            "    W{}  {:>7.2} {:>7.2} {:>4.0}% {:>4.0}% {:>8.2} {:>6.2} {:>10.2}",
            wi + 1,
            w.sum_clears as f64 / r,
            w.sum_streak as f64 / r,
            w.streak_ge2 as f64 / r * 100.0,
            w.streak_ge3 as f64 / r * 100.0,
            w.sum_goal as f64 / r,
            w.sum_adj as f64 / r,
            w.sum_rock_skip as f64 / r,
        );
        t_reach += w.reachable as u64;
        t_streak += w.sum_streak;
        t_ge2 += w.streak_ge2 as u64;
        t_ge3 += w.streak_ge3 as u64;
        t_goal2 += w.goal_ge2 as u64;
        t_rock += w.sum_rock_skip;
    }
    if t_reach > 0 {
        let tr = t_reach as f64;
        eprintln!(
            "    overall: streak {:.2} | ≥2 {:.0}% ≥3 {:.0}% | goal-stack≥2 {:.0}% | rock-skip {:.2}",
            t_streak as f64 / tr,
            t_ge2 as f64 / tr * 100.0,
            t_ge3 as f64 / tr * 100.0,
            t_goal2 as f64 / tr * 100.0,
            t_rock as f64 / tr,
        );
    }

    // Pipe-role breakdown (mean pipes/world), by what removing the pipe would
    // cost the player: conn = strands the target; shortcut = raises min-clears
    // (lvls-skipped = how many); access = strands other level/fort content;
    // redundant = nothing changes (pure waste).
    eprintln!(
        "    pipes/world:   {:>5} {:>9} {:>8} {:>7} {:>7} {:>10} {:>9}",
        "conn", "shortcut", "skipped", "access", "scenic", "redundant", "express%",
    );
    let (mut g_conn, mut g_short, mut g_skip, mut g_access, mut g_scenic, mut g_redundant) =
        (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
    // Same aggregate but excluding W7, whose 8-pair pipe mesh dominates the
    // redundancy total and hides the picture for the rest of the map.
    let (mut x_conn, mut x_short, mut x_access, mut x_scenic, mut x_redundant, mut x_reach) =
        (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
    for (wi, w) in worlds.iter().enumerate() {
        if w.reachable == 0 {
            continue;
        }
        let r = w.reachable as f64;
        let express = if w.sum_express_total > 0 {
            format!("{:.1}%", w.sum_express as f64 / w.sum_express_total as f64 * 100.0)
        } else {
            "n/a".to_string()
        };
        eprintln!(
            "      W{}         {:>5.2} {:>9.2} {:>8.2} {:>7.2} {:>7.2} {:>10.2} {:>9}",
            wi + 1,
            w.sum_conn as f64 / r,
            w.sum_shortcut as f64 / r,
            w.sum_pipe_skip as f64 / r,
            w.sum_access as f64 / r,
            w.sum_scenic as f64 / r,
            w.sum_redundant as f64 / r,
            express,
        );
        g_conn += w.sum_conn;
        g_short += w.sum_shortcut;
        g_skip += w.sum_pipe_skip;
        g_access += w.sum_access;
        g_scenic += w.sum_scenic;
        g_redundant += w.sum_redundant;
        if wi != 6 {
            x_conn += w.sum_conn;
            x_short += w.sum_shortcut;
            x_access += w.sum_access;
            x_scenic += w.sum_scenic;
            x_redundant += w.sum_redundant;
            x_reach += w.reachable as u64;
        }
    }
    if t_reach > 0 {
        let tr = t_reach as f64;
        let all_pipes = g_conn + g_short + g_access + g_scenic + g_redundant;
        eprintln!(
            "      overall:   {:>5.2} {:>9.2} {:>8.2} {:>7.2} {:>7.2} {:>10.2}  (redundant {:.0}% of all pipes)",
            g_conn as f64 / tr,
            g_short as f64 / tr,
            g_skip as f64 / tr,
            g_access as f64 / tr,
            g_scenic as f64 / tr,
            g_redundant as f64 / tr,
            if all_pipes > 0 {
                g_redundant as f64 / all_pipes as f64 * 100.0
            } else {
                0.0
            },
        );
    }
    if x_reach > 0 {
        let xr = x_reach as f64;
        let x_pipes = x_conn + x_short + x_access + x_scenic + x_redundant;
        eprintln!(
            "      ex-W7:     {:>5.2} {:>9.2} {:>8} {:>7.2} {:>7.2} {:>10.2}  (redundant {:.0}% of all pipes)",
            x_conn as f64 / xr,
            x_short as f64 / xr,
            "",
            x_access as f64 / xr,
            x_scenic as f64 / xr,
            x_redundant as f64 / xr,
            if x_pipes > 0 {
                x_redundant as f64 / x_pipes as f64 * 100.0
            } else {
                0.0
            },
        );
    }
    let unreach: u32 = worlds.iter().map(|w| w.unreachable).sum();
    if unreach > 0 {
        eprintln!("    WARNING: {unreach} unreachable-target case(s) — possible builder bug");
    }
}

/// Required-progression / linearity analyzer.
///
/// Per world, computes the minimum fortress + level entries the player must
/// clear to reach the airship/Bowser (locks gate path tiles until their
/// fortress is cleared; pipes/canoes taken when they shorten the route),
/// then derives linearity metrics: back-to-back forced-level streaks, levels
/// stacked on the goal approach, physical level-adjacency, and how many
/// forced clears pipes / a lock-breaking hammer let the player skip.
///
/// Reports BOTH start↔airship modes (SAS off and on) in one run, since SAS
/// redistributes which worlds stack levels.
///
/// Run with: cargo test --release test_required_progression -- --ignored --nocapture
/// Override seed count with CENSUS_SEEDS=N (runs that many PER mode).
/// FLAGS=SMB3R-… measures a specific flag set (e.g. hammer-breaks-locks on).
#[test]
#[ignore]
fn test_required_progression() {
    let rom_bytes = match std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") {
        Ok(b) => b,
        Err(_) => {
            eprintln!("ROM not found, skipping");
            return;
        }
    };

    let seeds: u64 = std::env::var("CENSUS_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);

    // Base options = the shipped map. Options::default() is hand-tuned to the
    // CLI/web defaults; the full pipeline (randomize_rom_with_overworld_capture)
    // consumes RNG exactly as a real run, so the built maps match real ROMs.
    // FLAGS=SMB3R-… overrides the flag set (e.g. hammer-breaks-locks).
    // Palettes off only for run-to-run determinism (cosmetic; no topology).
    let mut base = match std::env::var("FLAGS") {
        Ok(key) => match crate::Options::from_flag_key(&key) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("Invalid FLAGS key: {e}");
                return;
            }
        },
        Err(_) => crate::Options::default(),
    };
    base.palettes = false;
    base.palette_themed = false;

    // Combined report: measure BOTH start↔airship modes, since SAS
    // redistributes which worlds stack levels.
    let mut sas_off = base.clone();
    sas_off.swap_start_airship = false;
    let mut sas_on = base.clone();
    sas_on.swap_start_airship = true;

    let off = prog_measure_pass(&rom_bytes, &sas_off, seeds);
    prog_print_pass("SAS off", &off);
    let on = prog_measure_pass(&rom_bytes, &sas_on, seeds);
    prog_print_pass("SAS on", &on);

    // Combined one-line comparison of the two modes.
    let overall = |w: &[ProgLinWorld; 8]| -> (f64, f64) {
        let reach: u64 = w.iter().map(|x| x.reachable as u64).sum();
        if reach == 0 {
            return (0.0, 0.0);
        }
        let s: u64 = w.iter().map(|x| x.sum_streak).sum();
        let g2: u64 = w.iter().map(|x| x.goal_ge2 as u64).sum();
        (s as f64 / reach as f64, g2 as f64 / reach as f64 * 100.0)
    };
    let (so, go) = overall(&off);
    let (sn, gn) = overall(&on);
    eprintln!(
        "\n  Combined: SAS-off streak {so:.2} / goal-stack≥2 {go:.0}%   vs   SAS-on streak {sn:.2} / goal-stack≥2 {gn:.0}%",
    );
}

/// Single-seed dump of the required-progression analysis. Intended for
/// verification by eye — prints the fortress/lock inventory and the
/// step-by-step path Dijkstra picked, both without and with hammer.
///
/// Run with:
///   DUMP_SEED=0 DUMP_WORLD=4 cargo test --release \
///     test_dump_required_progression -- --ignored --nocapture
/// Omit DUMP_WORLD to print all 8 worlds.
#[test]
#[ignore]
fn test_dump_required_progression() {
    use crate::Options;

    let rom_bytes = match std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") {
        Ok(b) => b,
        Err(_) => {
            eprintln!("ROM not found, skipping");
            return;
        }
    };

    let seed: u64 = std::env::var("DUMP_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let world_filter: Option<usize> = std::env::var("DUMP_WORLD")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .map(|w| w.saturating_sub(1));

    // PROBE=1: print the vanilla world grid (post-QoL) for the chosen
    // DUMP_WORLD, then exit. Used to inspect map topology without any
    // build randomization.
    if std::env::var("PROBE").is_ok() {
        let rom = Rom::from_bytes(&rom_bytes).unwrap();
        let rom = apply_qol_for_overworld(&rom);
        let wi = world_filter.unwrap_or(2); // default W3
        let grid = rom_data::read_tile_grid(&rom, wi);
        eprintln!("=== Vanilla W{} grid (post-QoL) ===", wi + 1);
        for r in 0..grid.rows() {
            eprint!("  r{r:1}:");
            for c in 0..grid.cols {
                eprint!(" {:02X}", grid.get(r, c));
            }
            eprintln!();
        }
        return;
    }

    // STANDALONE=1 bypasses the full pipeline and runs the builder directly
    // off a fresh `seed_from_u64(seed)` RNG.
    //
    // It no longer matches `test_required_progression`: that census was moved
    // onto `randomize_rom_with_overworld_capture` (the shipped pipeline, whose
    // RNG stream differs), and the DEFAULT path below tracks it. So to
    // reproduce something the census reported for a seed, use the default
    // path, NOT this one. STANDALONE survives as the raw-builder arm — the way
    // to tell a builder behaviour apart from a pipeline effect.
    if std::env::var("STANDALONE").is_ok() {
        let rom = match Rom::from_bytes(&rom_bytes) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("ROM parse failed: {e}");
                return;
            }
        };
        let rom = apply_qol_for_overworld(&rom);
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let (catalog, pickup) = build_catalog_pickup(&rom, seed);
        let result = build(
            &rom,
            &OverworldData { pickup: &pickup, catalog: &catalog },
            &mut rng,
            BuildFlags { shuffle_toad_houses: true, ..Default::default() },
        );
        let sas_label = if std::env::var("SAS").is_ok() {
            " [SAS=1]"
        } else {
            ""
        };
        eprintln!("=== Required Progression dump (seed={seed}{sas_label}, STANDALONE) ===");
        for built in &result.worlds {
            if let Some(w) = world_filter
                && built.world_idx != w
            {
                continue;
            }
            dump_required_progression(built);
            // GRID=1: also print the post-build grid for visual inspection.
            if std::env::var("GRID").is_ok() {
                eprintln!("\n  Post-build grid:");
                for r in 0..built.grid.rows() {
                    eprint!("    r{r:1}:");
                    for c in 0..built.grid.cols {
                        eprint!(" {:02X}", built.grid.get(r, c));
                    }
                    eprintln!();
                }
                if let (Some(start), Some(target)) = (
                    rom_data::find_start(&built.grid),
                    find_target(&built.grid, built.world_idx),
                ) {
                    let probe = |grid: &Grid, label: &str, pos: (usize, usize)| {
                        let r = pos.0 as i32 - 1;
                        let c = pos.1 as i32 - 1;
                        let dirs = [
                            ("N", r, pos.1 as i32),
                            ("S", pos.0 as i32 + 1, pos.1 as i32),
                            ("W", pos.0 as i32, c),
                            ("E", pos.0 as i32, pos.1 as i32 + 1),
                        ];
                        eprintln!("  {label}={pos:?} tile=0x{:02X}", grid.get(pos.0, pos.1));
                        for (d, rr, cc) in dirs {
                            if rr < 0 || cc < 0 || rr as usize >= grid.rows() || cc as usize >= grid.cols {
                                eprintln!("    {d} ({rr},{cc}): off-grid");
                            } else {
                                eprintln!("    {d} ({rr},{cc}): 0x{:02X}", grid.get(rr as usize, cc as usize));
                            }
                        }
                    };
                    probe(&built.grid, "start", start);
                    probe(&built.grid, "target", target);

                    // What does walk_map see as reachable from start?
                    let walk = walk_map(&built.grid, &built.pipe_pairs, Some(start), built.world_idx);
                    let mut reachable: Vec<(usize, usize)> = walk.nodes.iter().copied().collect();
                    reachable.sort();
                    eprintln!("\n  walk_map reachable from start ({} nodes):", reachable.len());
                    for pos in &reachable {
                        eprintln!("    {pos:?} tile=0x{:02X}", built.grid.get(pos.0, pos.1));
                    }
                }
            }
        }
        return;
    }

    // Build Options from either a FLAGS=SMB3R-... key (preferred — covers
    // every randomizer toggle) or fall back to `Options::default()` plus
    // an `SAS=1` override. This matches what the user would pass to the
    // CLI/web, so the RNG sequence reaching the overworld builder is the
    // one a real playthrough sees.
    let mut options = match std::env::var("FLAGS") {
        Ok(key) => match crate::Options::from_flag_key(&key) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("Invalid FLAGS key: {e}");
                return;
            }
        },
        Err(_) => Options::default(),
    };
    if std::env::var("SAS").is_ok() {
        options.swap_start_airship = true;
    }
    // Palettes (both character-only and themed) use a fresh OS RNG, so
    // they introduce noise that breaks reproducibility without affecting
    // the topology this analyzer cares about. Force both off so identical
    // (seed, flags) inputs produce identical ROM bytes.
    options.palettes = false;
    options.palette_themed = false;

    let (rom, result) = match crate::randomize_rom_with_overworld_capture(
        &rom_bytes, seed, &options, None,
    ) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("randomize_rom_with_overworld_capture failed: {e}");
            return;
        }
    };

    let sas_label = if options.swap_start_airship { " [SAS=1]" } else { "" };
    let flag_key = options.to_flag_key();
    eprintln!("=== Required Progression dump (seed={seed}{sas_label}) ===");
    eprintln!("Flags: {flag_key}");

    for built in &result.worlds {
        if let Some(w) = world_filter
            && built.world_idx != w
        {
            continue;
        }
        dump_required_progression(built);
    }

    // Save the fully-randomized ROM (matches the real playthrough state).
    let sas_tag = if options.swap_start_airship { "_sas" } else { "" };
    let filename = format!("progression_seed{seed}{sas_tag}.nes");
    std::fs::write(&filename, rom.output_bytes()).unwrap();
    eprintln!("\nWrote {filename}");
}


// ---------------------------------------------------------------------------
// Census column arithmetic. Every per-world census prints some mix of these
// over one world's sample set; keeping the `sum / len as f64` in one place
// stopped three copies of the same table drifting apart (the C1 column was
// computed identically in two tests before they were merged below).
// ---------------------------------------------------------------------------

fn mean_of(v: &[u32]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.iter().sum::<u32>() as f64 / v.len() as f64
}

/// Share of samples satisfying `pred`, in percent.
fn pct_of(v: &[u32], pred: impl Fn(u32) -> bool) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.iter().filter(|&&x| pred(x)).count() as f64 / v.len() as f64 * 100.0
}

/// `p`-th percentile of an ALREADY-SORTED sample set (p in 0..100).
fn pctile_of(sorted: &[u32], p: usize) -> u32 {
    if sorted.is_empty() {
        return 0;
    }
    sorted[sorted.len() * p / 100]
}

/// What one world contributes for one seed. `None` route data = unreachable.
struct RouteSample {
    routes: usize,
    c1: u32,
    /// The floor this world was DEALT (`deal_c1_floors`) — every floor
    /// column scores against this, not against the band's center.
    floor: u32,
    reachable: bool,
    goal_open: bool,
    has_rock: bool,
    c1_breaks_rock: bool,
    alt_breaks_rock: bool,
}

/// The route-choice census — ONE build pass over the seeds, three reports.
///
/// Merged 2026-08-07 from `test_route_choice` + `test_c1_floor_probe` +
/// `test_rock_route_census`, which each rebuilt every seed to measure the same
/// `RouteChoice`. Their C1 and linear% columns were computed twice, identically;
/// this way the numbers cannot disagree, and one invocation gives the whole
/// picture instead of three.
///
///   1. Route choice — distinct non-dominated routes per world (weighted
///      set-cost: pipe 1 / level 3 / fort 5 / rock 8, each clearable charged
///      once). LINEAR = one best route, CHOICE = 2+ within `slack`.
///   2. C1 floor — distribution of the cheapest route's cost, the quantity
///      `C1_FLOOR` bounds, plus the goal-open rate.
///   3. Rock paths — does the cheap route break a rock, or is the rock the
///      price of an ALTERNATIVE? Also relative to worlds that have one.
///
/// Run with:
///   CENSUS_SEEDS=200 CENSUS_SLACK=3 cargo test --release --lib \
///     test_route_census -- --ignored --nocapture
/// DUMP_SEED=<n> also prints per-world one-liners for that seed (RENDER=1 for
/// the ASCII maps).
#[test]
#[ignore]
fn test_route_census() {
    let rom_bytes = match std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") {
        Ok(b) => b,
        Err(_) => {
            eprintln!("ROM not found, skipping");
            return;
        }
    };
    // RAW rom on purpose: `census_build` applies the map QOL itself, per arm.
    let rom = match Rom::from_bytes(&rom_bytes) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("ROM parse failed: {e}");
            return;
        }
    };

    let seeds: u64 = std::env::var("CENSUS_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);
    let slack: u32 = std::env::var("CENSUS_SLACK")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(route_choice::DEFAULT_SLACK);
    let dump_seed: Option<u64> = std::env::var("DUMP_SEED")
        .ok()
        .and_then(|s| s.parse().ok());

    // Build AND analyze in the parallel closure — the route Dijkstras are a
    // real share of the census cost. Dump output (one seed) prints in-thread.
    let per_seed = par_seeds(seeds, |seed| {
        let result = census_build(&rom, seed);
        if Some(seed) == dump_seed {
            eprintln!("\n=== Route choice, seed {seed} (slack {slack}) ===");
        }
        let mut out: [Option<RouteSample>; 8] = [const { None }; 8];
        for built in &result.worlds {
            let rc = analyze_route_choice(built, slack);
            let has_rock = (0..built.grid.rows()).any(|r| {
                (0..built.grid.cols).any(|c| matches!(built.grid.get(r, c), 0x51 | 0x52))
            });
            out[built.world_idx] = Some(RouteSample {
                routes: if rc.reachable { rc.routes.len() } else { 0 },
                c1: rc.best_cost,
                floor: built.c1_floor,
                reachable: rc.reachable,
                goal_open: world_topology(built)
                    .is_some_and(|t| t.fort_count >= 2 && t.depth == 0),
                has_rock,
                c1_breaks_rock: rc.routes.first().is_some_and(|r| !r.rocks.is_empty()),
                alt_breaks_rock: rc.routes.iter().skip(1).any(|r| !r.rocks.is_empty()),
            });
            if Some(seed) == dump_seed {
                if std::env::var("RENDER").is_ok() {
                    eprint!("{}", route_choice::render_route_choice(built, slack));
                } else {
                    dump_route_choice(built, slack);
                }
            }
        }
        out
    });

    // Per-world sample sets. C1/goal-open only count REACHABLE worlds (an
    // unreachable world has no cheapest route to price); route counts keep the
    // 0 so `linear%` still sees it as choice-free.
    let mut routes: Vec<Vec<u32>> = vec![Vec::new(); 8];
    let mut c1s: Vec<Vec<u32>> = vec![Vec::new(); 8];
    let mut goal_open = [0usize; 8];
    let mut rocks = [[0u64; 4]; 8]; // seen, rock-world, C1 rock, alt rock
    // Floor accounting: per world, (samples, missed own floor, floor sum);
    // and the same sliced BY the dealt floor, which is the cut that shows
    // what a floor actually buys (one row per distinct value in the deal).
    let mut floor_stat = [[0u64; 3]; 8];
    let mut by_floor: Vec<(u32, Vec<u32>, Vec<u32>, u64)> = {
        let mut vals = C1_FLOOR_BAND.to_vec();
        vals.sort_unstable();
        vals.dedup();
        vals.into_iter().map(|f| (f, Vec::new(), Vec::new(), 0)).collect()
    };
    for worlds in &per_seed {
        for (wi, s) in worlds.iter().enumerate() {
            let Some(s) = s else { continue };
            routes[wi].push(s.routes as u32);
            if s.reachable {
                c1s[wi].push(s.c1);
                goal_open[wi] += s.goal_open as usize;
                let stat = &mut floor_stat[wi];
                stat[0] += 1;
                stat[1] += (s.c1 < s.floor) as u64;
                stat[2] += s.floor as u64;
                if let Some(row) =
                    by_floor.iter_mut().find(|(f, ..)| *f == s.floor)
                {
                    row.1.push(s.c1);
                    row.2.push(s.routes as u32);
                    row.3 += (s.c1 < s.floor) as u64;
                }
            }
            let row = &mut rocks[wi];
            row[0] += 1;
            row[1] += s.has_rock as u64;
            row[2] += s.c1_breaks_rock as u64;
            row[3] += s.alt_breaks_rock as u64;
        }
    }

    // -- 1. Route choice ----------------------------------------------------
    eprintln!("\n=== Route choice over {seeds} seeds (slack {slack}) ===");
    eprintln!(
        "  {:<4} {:>6} {:>8} {:>8} {:>5} {:>7} {:>9}",
        "", "mean", "linear%", "choice%", "max", "C1", "<floor%"
    );
    let mut all_routes: Vec<u32> = Vec::new();
    let mut all_c1: Vec<u32> = Vec::new();
    for wi in 0..8 {
        let (r, c) = (&routes[wi], &c1s[wi]);
        if r.is_empty() {
            continue;
        }
        all_routes.extend(r.iter().copied());
        all_c1.extend(c.iter().copied());
        let stat = floor_stat[wi];
        eprintln!(
            "  W{:<3} {:>6.2} {:>7.0}% {:>7.0}% {:>5} {:>7.1} {:>8.1}%",
            wi + 1,
            mean_of(r),
            pct_of(r, |n| n <= 1),
            pct_of(r, |n| n >= 2),
            r.iter().copied().max().unwrap_or(0),
            mean_of(c),
            stat[1] as f64 / stat[0].max(1) as f64 * 100.0,
        );
    }
    if !all_routes.is_empty() {
        let (n, missed): (u64, u64) =
            floor_stat.iter().fold((0, 0), |(n, m), s| (n + s[0], m + s[1]));
        eprintln!(
            "  overall: mean {:.3} routes/world; {:.2}% linear; C1 {:.1}, {:.2}% below \
             its own dealt floor (n={})",
            mean_of(&all_routes),
            pct_of(&all_routes, |n| n <= 1),
            mean_of(&all_c1),
            missed as f64 / n.max(1) as f64 * 100.0,
            all_routes.len(),
        );
    }

    // -- 2. C1 floor --------------------------------------------------------
    eprintln!("\n=== C1 floor probe over {seeds} seeds ===");
    eprintln!(
        "  {:<4} {:>5} {:>5} {:>6} {:>6} {:>6} {:>6} {:>6} {:>9} {:>8}",
        "", "min", "p10", "mean", "floor", "<8", "<14", "<own", "goal-open", "linear%",
    );
    for wi in 0..8 {
        let c = &mut c1s[wi];
        if c.is_empty() {
            continue;
        }
        c.sort_unstable();
        let stat = floor_stat[wi];
        eprintln!(
            "  W{:<3} {:>5} {:>5} {:>6.1} {:>6.1} {:>5.0}% {:>5.0}% {:>5.1}% {:>8.1}% {:>7.0}%",
            wi + 1,
            c[0],
            pctile_of(c, 10),
            mean_of(c),
            stat[2] as f64 / stat[0].max(1) as f64,
            pct_of(c, |x| x < 8),
            pct_of(c, |x| x < 14),
            stat[1] as f64 / stat[0].max(1) as f64 * 100.0,
            goal_open[wi] as f64 / c.len() as f64 * 100.0,
            pct_of(&routes[wi], |n| n <= 1),
        );
    }
    // Per-seed total C1 — the "is the budget really conserved" number. The
    // dealt floor conserves the GUARANTEE (8 x C1_FLOOR) but only bounds
    // from below, so this is what actually moves. Seeds with an unreachable
    // world are skipped rather than counted short.
    let seed_totals: Vec<u32> = per_seed
        .iter()
        .filter(|w| w.iter().all(|s| s.as_ref().is_some_and(|s| s.reachable)))
        .map(|w| w.iter().flatten().map(|s| s.c1).sum())
        .collect();
    if !seed_totals.is_empty() {
        eprintln!(
            "  per-seed total C1: mean {:.1} (floor total {}, n={})",
            mean_of(&seed_totals),
            C1_FLOOR * 8,
            seed_totals.len(),
        );
    }
    if !all_c1.is_empty() {
        all_c1.sort_unstable();
        let go: usize = goal_open.iter().sum();
        eprintln!(
            "  overall: min {} p10 {} mean {:.1}; <14 {:.1}%; goal-open {:.1}%; linear {:.0}%",
            all_c1[0],
            pctile_of(&all_c1, 10),
            mean_of(&all_c1),
            pct_of(&all_c1, |x| x < 14),
            go as f64 / all_c1.len() as f64 * 100.0,
            pct_of(&all_routes, |n| n <= 1),
        );
    }

    // What the dealt floor actually buys, sliced by the floor itself — the
    // per-world table above averages the whole band away (every world sees
    // every floor across seeds, so its mean floor is ~14 by construction).
    // `lift` is C1 minus the floor: how far ABOVE its guarantee the world
    // finished, which is what says whether a floor is binding or inert.
    eprintln!("\n=== By dealt C1 floor ===");
    eprintln!(
        "  {:<6} {:>7} {:>7} {:>7} {:>7} {:>8} {:>7}",
        "floor", "n", "C1", "lift", "routes", "linear%", "missed%",
    );
    for (f, c1, rts, missed) in &by_floor {
        if c1.is_empty() {
            continue;
        }
        eprintln!(
            "  {:<6} {:>7} {:>7.1} {:>7.1} {:>7.2} {:>7.0}% {:>6.1}%",
            f,
            c1.len(),
            mean_of(c1),
            mean_of(c1) - *f as f64,
            mean_of(rts),
            pct_of(rts, |n| n <= 1),
            *missed as f64 / c1.len() as f64 * 100.0,
        );
    }

    // -- 3. Rock paths ------------------------------------------------------
    eprintln!("\n=== Rock-path census over {seeds} seeds ===");
    eprintln!(
        "  {:<4} {:>10} {:>10} {:>12} {:>14} {:>14}",
        "", "rock-world", "C1 rock", "alt(C2+) rock", "C1|rock-world", "alt|rock-world"
    );
    for (wi, t) in rocks.iter().enumerate() {
        let [n, rockw, c1, alt] = *t;
        if n == 0 {
            continue;
        }
        let pc = |x: u64, base: u64| {
            if base == 0 { 0.0 } else { x as f64 / base as f64 * 100.0 }
        };
        eprintln!(
            "  W{:<3} {:>9.0}% {:>9.1}% {:>11.1}% {:>13.1}% {:>13.1}%",
            wi + 1,
            pc(rockw, n),
            pc(c1, n),
            pc(alt, n),
            pc(c1, rockw),
            pc(alt, rockw),
        );
    }
}
/// Per-seed build wall time — the number the WASM app's generate latency
/// tracks. Serial on purpose (per-seed timing, no thread contention).
///   CENSUS_SEEDS=40 cargo test --release --lib test_build_time -- --ignored --nocapture
#[test]
#[ignore]
fn test_build_time() {
    let rom_bytes = match std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") {
        Ok(b) => b,
        Err(_) => {
            eprintln!("ROM not found, skipping");
            return;
        }
    };
    let rom = Rom::from_bytes(&rom_bytes).unwrap();
    let seeds: u64 = std::env::var("CENSUS_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);

    let mut per_seed_ms: Vec<f64> = Vec::new();
    for seed in 0..seeds {
        let t = std::time::Instant::now();
        let _ = census_build(&rom, seed);
        per_seed_ms.push(t.elapsed().as_secs_f64() * 1e3);
    }
    let mean = per_seed_ms.iter().sum::<f64>() / per_seed_ms.len() as f64;
    let max = per_seed_ms.iter().cloned().fold(0.0f64, f64::max);
    let min = per_seed_ms.iter().cloned().fold(f64::INFINITY, f64::min);
    eprintln!("\nbuild time over {seeds} seeds: mean {mean:.1} ms  min {min:.1}  max {max:.1}");
}

/// Reuse-correctness for the hoisted walk-graph base (issue #120). Compiling a
/// `WalkGraph` once and `measure`-ing candidates against it must yield EXACTLY
/// what a from-scratch `analyze_route_choice` on the mutated world would — for
/// every candidate whose flip leaves the walk graph unchanged. This validates
/// three things on real census data:
///   1. the compile/measure split is behavior-preserving (base-measure ==
///      one-shot on the unmutated world),
///   2. kind flips (HB↔Fortress / HB↔Level) really ARE walk-invariant for the
///      slots the passes flip (the premise the issue said to verify, not
///      assume) — and where a flip is NOT invariant (a slot on a background
///      tile), reuse is correctly skipped,
///   3. adding a lock (an overlay `walk_map` never sees) is always invariant.
///
/// CENSUS_SEEDS=100 cargo test --release --lib test_walkgraph_reuse -- --ignored --nocapture
#[test]
#[ignore]
fn test_walkgraph_reuse() {
    let rom_bytes = match std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") {
        Ok(b) => b,
        Err(_) => {
            eprintln!("ROM not found, skipping");
            return;
        }
    };
    let rom = Rom::from_bytes(&rom_bytes).unwrap();
    let seeds: u64 = std::env::var("CENSUS_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);
    let slack = route_choice::DEFAULT_SLACK;

    let (mut invariant_flips, mut variant_flips, mut lock_trials) = (0usize, 0usize, 0usize);

    for seed in 0..seeds {
        for built in &census_build(&rom, seed).worlds {
            let Some(base) = route_choice::WalkGraph::compile(built) else {
                continue;
            };
            let world = built.world_idx + 1;

            // (1) Split is behavior-preserving: reuse on the SAME world equals
            // the one-shot entry point, byte for byte (paths included).
            assert_eq!(
                base.measure(built, slack, true),
                analyze_route_choice(built, slack),
                "W{world} seed {seed}: base-measure differs from one-shot",
            );

            // (2) Kind-flip candidates: flip each slot to each other content
            // kind. When the flip leaves the walk graph unchanged (checked
            // directly), reuse MUST match a fresh measure; when it doesn't
            // (background-tile slot), reuse is unsound and correctly skipped.
            for i in 0..built.slots.len() {
                for new_kind in [SlotKind::Fortress, SlotKind::Level, SlotKind::HammerBro] {
                    if built.slots[i].kind == new_kind {
                        continue;
                    }
                    let mut cand = built.clone();
                    cand.slots[i].kind = new_kind.clone();
                    if base.walk_invariant(&cand) {
                        invariant_flips += 1;
                        assert_eq!(
                            base.measure(&cand, slack, true),
                            analyze_route_choice(&cand, slack),
                            "W{world} seed {seed}: reuse != fresh for a walk-invariant \
                             {:?}→{new_kind:?} flip at slot {i}",
                            built.slots[i].kind,
                        );
                    } else {
                        variant_flips += 1;
                    }
                }
            }

            // (3) Lock candidates: add a lock on an arbitrary existing lock's
            // tile (or, if the world has none, on a real path tile), opening a
            // random section. Locks never touch the grid walk_map sees, so the
            // base stays valid and reuse must match fresh.
            if built.section_count > 0 {
                let mut cand = built.clone();
                let lock_pos = built
                    .locks
                    .first()
                    .map(|l| l.pos)
                    .or_else(|| walk_edge_path_tile(built));
                if let Some(pos) = lock_pos {
                    cand.locks.push(super::types::LockAssignment {
                        pos,
                        gap_tile: 0x54,
                        replace_tile: 0x00,
                        fort_section: seed as usize % built.section_count,
                        secret_exit_safe: false,
                    });
                    assert!(
                        base.walk_invariant(&cand),
                        "W{world} seed {seed}: adding a lock changed the walk graph",
                    );
                    lock_trials += 1;
                    assert_eq!(
                        base.measure(&cand, slack, true),
                        analyze_route_choice(&cand, slack),
                        "W{world} seed {seed}: reuse != fresh after adding a lock",
                    );
                }
            }
        }
    }

    // The test must actually exercise the reuse path, not pass vacuously.
    assert!(invariant_flips > 0, "no walk-invariant kind flips were exercised");
    assert!(lock_trials > 0, "no lock trials were exercised");
    eprintln!(
        "\nWalkGraph reuse over {seeds} seeds: {invariant_flips} walk-invariant kind flips, \
         {variant_flips} walk-changing flips (correctly skipped), {lock_trials} lock trials — \
         all reuse results matched a fresh measure.",
    );
}

/// Any path tile crossed by a walk edge in `built` (a valid lock site) — for
/// the reuse test's lock candidate when the world has no lock yet.
fn walk_edge_path_tile(built: &BuiltWorld) -> Option<(usize, usize)> {
    let mut grid = built.grid.clone();
    stamp_slots(&mut grid, &built.slots);
    let start = rom_data::find_start(&grid);
    walk_map(&grid, &built.pipe_pairs, start, built.world_idx)
        .edges
        .values()
        .flatten()
        .find_map(|e| e.path_pos)
}

/// Production-parity route-choice render for ONE real seed: runs the full
/// randomizer pipeline (so the RNG stream is exactly what that CLI/web seed
/// produces), then prints the route-choice verdict and an ASCII map per
/// route for every world.
///
///   DUMP_SEED=<n> [DUMP_WORLD=w] [CENSUS_SLACK=3] [FLAGS=SMB3R-...] [SAS=1] \
///     cargo test --release --lib test_render_route_choice -- --ignored --nocapture
#[test]
#[ignore]
fn test_render_route_choice() {
    use crate::Options;

    let rom_bytes = match std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") {
        Ok(b) => b,
        Err(_) => {
            eprintln!("ROM not found, skipping");
            return;
        }
    };
    let seed: u64 = std::env::var("DUMP_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let world_filter: Option<usize> = std::env::var("DUMP_WORLD")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .map(|w| w.saturating_sub(1));
    let slack: u32 = std::env::var("CENSUS_SLACK")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(route_choice::DEFAULT_SLACK);

    // Same options handling as test_dump_required_progression: FLAGS key
    // preferred, palettes forced off for reproducibility.
    let mut options = match std::env::var("FLAGS") {
        Ok(key) => match Options::from_flag_key(&key) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("Invalid FLAGS key: {e}");
                return;
            }
        },
        Err(_) => Options::default(),
    };
    if std::env::var("SAS").is_ok() {
        options.swap_start_airship = true;
    }
    options.palettes = false;
    options.palette_themed = false;

    let (_rom, result) =
        match crate::randomize_rom_with_overworld_capture(&rom_bytes, seed, &options, None) {
            Ok(pair) => pair,
            Err(e) => {
                eprintln!("randomize_rom_with_overworld_capture failed: {e}");
                return;
            }
        };

    eprintln!("=== Route-choice render (seed={seed}, slack={slack}) ===");
    eprintln!("Flags: {}", options.to_flag_key());
    for built in &result.worlds {
        if let Some(w) = world_filter
            && built.world_idx != w
        {
            continue;
        }
        dump_route_choice(built, slack);
        eprintln!("{}", route_choice::render_route_choice(built, slack));
    }
}
