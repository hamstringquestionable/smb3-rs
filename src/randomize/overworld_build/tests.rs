use super::*;
use super::capacity::{
    W8_HB_CAP, distribute_levels, fixed_positions_for_world, prepare_capacities,
    redistribute_fortresses,
};
use super::locks::debug_stamp_rom;
use super::pipes::VANILLA_PIPE_PAIRS;
use super::scoring::{VANILLA_LEVEL_COUNT, is_dead_end};
use super::sections::find_blank_slots;
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

fn load_rom() -> Option<Rom> {
    let data = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
    Rom::from_bytes(&data).ok()
}

/// Apply the QoL patches that the real pipeline runs before the overworld
/// builder (`randomizer.rs` ~L657-670). These mutate the world-map grid —
/// rocks blocking pipe shortcuts, W3 drawbridge tiles, big-Q rooms — so
/// the catalog must see the post-patch state, not vanilla.
fn apply_qol_for_overworld(rom: &Rom) -> Rom {
    let mut out = rom.clone();
    super::super::qol::fix_w3_drawbridges(&mut out);
    super::super::qol::remove_rocks(&mut out);
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

#[test]
fn test_build_all_worlds() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };
    let (catalog, pickup) = build_catalog_pickup(&rom, 42);
    let mut rng = ChaCha8Rng::seed_from_u64(42);

    let result = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, BuildFlags { shuffle_toad_houses: true, ..Default::default() });

    assert_eq!(result.worlds.len(), 8);

    for built in &result.worlds {
        let wi = built.world_idx;
        let forts = built.slots.iter().filter(|s| s.kind == SlotKind::Fortress).count();
        let pipes = built.pipe_pairs.len();
        let locks = built.locks.len();

        assert_eq!(forts, result.fort_counts[wi],
            "W{}: fort slots {} != expected {}", wi + 1, forts, result.fort_counts[wi]);
        assert_eq!(pipes, VANILLA_PIPE_PAIRS[wi],
            "W{}: pipe pairs {} != expected {}", wi + 1, pipes, VANILLA_PIPE_PAIRS[wi]);
        assert!(locks <= result.fort_counts[wi],
            "W{}: locks {} > fort count {}", wi + 1, locks, result.fort_counts[wi]);
    }

    let total_levels: usize = result.worlds.iter()
        .map(|b| b.slots.iter().filter(|s| s.kind == SlotKind::Level).count())
        .sum();
    let total_forts: usize = result.worlds.iter()
        .map(|b| b.slots.iter().filter(|s| s.kind == SlotKind::Fortress).count())
        .sum();
    assert_eq!(total_levels, VANILLA_LEVEL_COUNT,
        "total levels {} != {}", total_levels, VANILLA_LEVEL_COUNT);
    assert_eq!(total_forts, 17, "total forts {} != 17", total_forts);
}

/// Regression: the overworld builder must never strand a world's target
/// (airship/Bowser) — that would be an unbeatable world. Covers SAS on/off,
/// hammer-bro shuffle on/off, and both the raw ROM and the QoL-patched ROM.
/// The raw-ROM arm (path rocks still present) locks in that the pipe
/// island-connect logic recovers connectivity even when a rock blocks a
/// path, so this can't regress if the rock-removal QoL ever changes.
#[test]
fn all_world_targets_reachable() {
    let raw = match load_rom() {
        Some(r) => r,
        None => return,
    };
    let qol = apply_qol_for_overworld(&raw);
    let names = ["W1", "W2", "W3", "W4", "W5", "W6", "W7", "W8"];

    for (rom_label, rom) in [("raw", &raw), ("qol", &qol)] {
        for hb in [false, true] {
            for sas in [false, true] {
                for seed in 0..40u64 {
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
#[test]
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

/// Diagnostic (not an assertion): tabulate how many levels the builder
/// places in each world across many seeds, next to the vanilla count, plus
/// the number of leftover open path tiles (placeable blank nodes left with
/// nothing on them). Run with:
///   cargo test report_levels_per_world -- --nocapture
#[test]
fn report_levels_per_world() {
    let raw = match load_rom() {
        Some(r) => r,
        None => return,
    };
    // QoL map edits (incl. remove_rocks) run before the builder in the real
    // pipeline; build from the patched ROM so capacities/connectivity match
    // what players actually get.
    let rom = apply_qol_for_overworld(&raw);

    const SEEDS: u64 = 200;

    // Vanilla per-world Level counts, straight from the catalog (the same
    // source VANILLA_LEVEL_COUNT is derived from).
    let vanilla_catalog = NodeCatalog::build(&rom, false);
    let mut vanilla_levels = [0usize; 8];
    for e in &vanilla_catalog.entries {
        if matches!(e.kind, NodeKind::Level) {
            vanilla_levels[e.world_idx] += 1;
        }
    }

    // Per world, collect the placed-level count and open-tile count per seed.
    let mut levels: [Vec<usize>; 8] = Default::default();
    let mut opens: [Vec<usize>; 8] = Default::default();

    for seed in 0..SEEDS {
        let (catalog, pickup) = build_catalog_pickup(&rom, seed);
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let result = build(
            &rom,
            &OverworldData { pickup: &pickup, catalog: &catalog },
            &mut rng,
            BuildFlags { shuffle_toad_houses: true, ..Default::default() },
        );

        for built in &result.worlds {
            let wi = built.world_idx;
            let lv = built.slots.iter().filter(|s| s.kind == SlotKind::Level).count();

            // Open path = currently-blank placeable node tiles not occupied
            // by a non-pipe slot. The build phase stamps pipe tiles onto the
            // grid but leaves level/fort/HB/bonus/toad slots blank, so those
            // slot positions still read as blank here and must be subtracted.
            let fixed = fixed_positions_for_world(&rom, &catalog, wi, true, false);
            let blank = find_blank_slots(&built.grid, &fixed).len();
            let non_pipe_slots =
                built.slots.iter().filter(|s| s.kind != SlotKind::Pipe).count();
            let open = blank.saturating_sub(non_pipe_slots);

            levels[wi].push(lv);
            opens[wi].push(open);
        }
    }

    let stats = |v: &[usize]| -> (usize, f64, usize) {
        let min = *v.iter().min().unwrap();
        let max = *v.iter().max().unwrap();
        let mean = v.iter().sum::<usize>() as f64 / v.len() as f64;
        (min, mean, max)
    };

    let names = ["Grass", "Desert", "Water", "Giant", "Sky", "Ice", "Pipe", "Dark"];
    eprintln!("\nLevels placed per world over {SEEDS} seeds (shuffle_toad_houses on):\n");
    eprintln!(
        "  {:<14} {:>7} | {:>18} | {:>18}",
        "World", "Vanilla", "Levels min/mean/max", "Open min/mean/max"
    );
    eprintln!("  {}", "-".repeat(66));
    let mut van_total = 0usize;
    let mut lvl_mean_total = 0.0f64;
    let mut open_mean_total = 0.0f64;
    for wi in 0..8 {
        let (lmin, lmean, lmax) = stats(&levels[wi]);
        let (omin, omean, omax) = stats(&opens[wi]);
        van_total += vanilla_levels[wi];
        lvl_mean_total += lmean;
        open_mean_total += omean;
        eprintln!(
            "  W{} {:<11} {:>7} | {:>5} {:>6.1} {:>4} | {:>5} {:>6.1} {:>4}",
            wi + 1, names[wi], vanilla_levels[wi],
            lmin, lmean, lmax, omin, omean, omax,
        );
    }
    eprintln!("  {}", "-".repeat(66));
    eprintln!(
        "  {:<14} {:>7} | {:>5} {:>6.1} {:>4} | {:>5} {:>6.1} {:>4}",
        "Total", van_total, "", lvl_mean_total, "", "", open_mean_total, "",
    );
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

#[test]
fn test_locks_dont_block_own_fort() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };
    let (catalog, pickup) = build_catalog_pickup(&rom, 0);

    for seed in 0..10 {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let result = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, BuildFlags { shuffle_toad_houses: true, ..Default::default() });

        for built in &result.worlds {
            let start_pos = rom_data::find_start(&built.grid);

            // Build grid with all assignments stamped (same as production)
            let mut test_grid = built.grid.clone();
            stamp_slots(&mut test_grid, &built.slots);

            // For each lock, verify its fort is still reachable
            for lock in &built.locks {
                // Place ALL locks
                let mut locked_grid = test_grid.clone();
                for l in &built.locks {
                    locked_grid.set(l.pos.0, l.pos.1, l.gap_tile);
                }
                // But open THIS lock (as if its fort was beaten)
                locked_grid.set(lock.pos.0, lock.pos.1, lock.replace_tile);

                // Open all locks from earlier sections too
                for earlier in &built.locks {
                    if earlier.fort_section < lock.fort_section {
                        locked_grid.set(earlier.pos.0, earlier.pos.1, earlier.replace_tile);
                    }
                }

                let fort_pos = built.slots.iter()
                    .find(|s| s.section == lock.fort_section && s.kind == SlotKind::Fortress)
                    .map(|s| s.pos);

                if let Some(fp) = fort_pos {
                    let walk = walk_map(&locked_grid, &built.pipe_pairs, start_pos, built.world_idx);
                    assert!(walk.nodes.contains(&fp),
                        "Seed {seed} W{}: lock at {:?} blocks its own fort at {:?}",
                        built.world_idx + 1, lock.pos, fp);
                }
            }
        }
    }
}

#[test]
#[ignore]
fn test_dump_debug_rom() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };
    let (catalog, pickup) = build_catalog_pickup(&rom, 0);

    for seed in [42, 123, 999] {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let result = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, BuildFlags { shuffle_toad_houses: true, ..Default::default() });

        let mut rom_copy = Rom::from_bytes(&rom.data).unwrap();
        debug_stamp_rom(&mut rom_copy, &result);

        let filename = format!("debug_build_seed{seed}.nes");
        std::fs::write(&filename, &rom_copy.data).unwrap();

        eprintln!("\n=== Seed {seed} ===");
        for built in &result.worlds {
            let forts = built.slots.iter().filter(|s| s.kind == SlotKind::Fortress).count();
            let levels = built.slots.iter().filter(|s| s.kind == SlotKind::Level).count();
            let pipes = built.pipe_pairs.len();
            let locks = built.locks.len();
            eprintln!(
                "  W{}: {} forts, {} levels, {} pipe pairs, {} locks",
                built.world_idx + 1, forts, levels, pipes, locks,
            );
        }
        eprintln!("  Wrote {filename}");
    }
}

#[test]
#[ignore]
fn test_print_build() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };
    let (catalog, pickup) = build_catalog_pickup(&rom, 42);
    let mut rng = ChaCha8Rng::seed_from_u64(42);

    let result = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, BuildFlags { shuffle_toad_houses: true, ..Default::default() });

    for built in &result.worlds {
        eprintln!("\n=== World {} ({} sections) ===",
            built.world_idx + 1, built.section_count);

        for (si, section_slots) in (0..built.section_count).map(|si| {
            (si, built.slots.iter().filter(|s| s.section == si).collect::<Vec<_>>())
        }) {
            let fort = section_slots.iter().find(|s| s.kind == SlotKind::Fortress);
            let levels = section_slots.iter().filter(|s| s.kind == SlotKind::Level).count();
            let lock = built.locks.iter().find(|l| l.fort_section == si);

            eprintln!("  Section {si}: {} slots ({} levels, fort at {:?})",
                section_slots.len(), levels,
                fort.map(|f| f.pos));
            if let Some(l) = lock {
                eprintln!("    Lock at {:?} (gap=${:02X}, restore=${:02X})",
                    l.pos, l.gap_tile, l.replace_tile);
            }
        }

        eprintln!("  Pipes: {} pairs", built.pipe_pairs.len());
        for (i, &(a, b)) in built.pipe_pairs.iter().enumerate() {
            eprintln!("    Pair {i}: ({},{}) ↔ ({},{})", a.0, a.1, b.0, b.1);
        }
    }
}

#[test]
#[ignore]
fn test_measure_shortfalls() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };
    let (catalog, pickup) = build_catalog_pickup(&rom, 0);

    let mut level_shortfalls = 0u32;
    let mut lock_shortfalls = 0u32;
    let seeds = 1000;

    for seed in 0..seeds {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let result = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, BuildFlags { shuffle_toad_houses: true, ..Default::default() });

        let total_levels: usize = result.worlds.iter()
            .map(|b| b.slots.iter().filter(|s| s.kind == SlotKind::Level).count())
            .sum();
        if total_levels < VANILLA_LEVEL_COUNT {
            level_shortfalls += 1;
            let deficit = VANILLA_LEVEL_COUNT - total_levels;
            // Show per-world breakdown
            let mut detail = String::new();
            for built in &result.worlds {
                let levels = built.slots.iter().filter(|s| s.kind == SlotKind::Level).count();
                let section_sizes: Vec<usize> = (0..built.section_count)
                    .map(|si| built.slots.iter().filter(|s| s.section == si).count())
                    .collect();
                if levels < 3 {
                    detail.push_str(&format!(" W{}={levels}(sections={section_sizes:?})", built.world_idx + 1));
                }
            }
            eprintln!("Seed {seed}: {total_levels}/{VANILLA_LEVEL_COUNT} (-{deficit}){detail}");
        }

        for built in &result.worlds {
            let expected_locks = result.fort_counts[built.world_idx];
            if built.locks.len() < expected_locks {
                lock_shortfalls += 1;
                // Find which section(s) are missing locks
                let placed: HashSet<usize> = built.locks.iter().map(|l| l.fort_section).collect();
                for si in 0..built.section_count {
                    if !placed.contains(&si) {
                        let section_size = built.slots.iter().filter(|s| s.section == si).count();
                        let fort = built.slots.iter().find(|s| s.section == si && s.kind == SlotKind::Fortress);
                        eprintln!("Seed {seed} W{} section {si}: NO LOCK, section_size={section_size}, fort={:?}, total_slots={}",
                            built.world_idx + 1, fort.map(|f| f.pos),
                            built.slots.len());
                    }
                }
            }
        }
    }

    // Count seeds with at least one secret_exit_safe lock and
    // track which worlds have safe locks in failing seeds
    let mut safe_count = 0u32;
    let mut no_safe_details: Vec<(u64, [usize; 8])> = Vec::new();
    for seed in 0..seeds {
        let mut rng2 = ChaCha8Rng::seed_from_u64(seed);
        let result2 = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng2, BuildFlags { shuffle_toad_houses: true, ..Default::default() });
        let has_safe = result2.worlds.iter().any(|b| {
            b.locks.iter().any(|l| l.secret_exit_safe)
        });
        if has_safe {
            safe_count += 1;
        } else {
            // For failing seeds, count locks per world to see which have room
            let mut lock_counts = [0usize; 8];
            for b in &result2.worlds {
                lock_counts[b.world_idx] = b.locks.len();
            }
            no_safe_details.push((seed, lock_counts));
        }
    }

    eprintln!("\n=== {seeds} seeds ===");
    eprintln!("Level shortfalls: {level_shortfalls}/{seeds}");
    eprintln!("Lock shortfalls:  {lock_shortfalls}/{seeds} (world-level)");
    eprintln!("Seeds with >=1 secret_exit_safe lock: {safe_count}/{seeds}");
    if !no_safe_details.is_empty() {
        eprintln!("No-safe seeds (first 10):");
        for (seed, counts) in no_safe_details.iter().take(10) {
            eprintln!("  Seed {seed}: locks per world = {counts:?}");
        }
    }
}

#[test]
#[ignore]
fn test_w6_slot_distribution() {
    let rom = match load_rom() {
        Some(r) => r,
        None => {
            eprintln!("ROM not found, skipping");
            return;
        }
    };
    let (catalog, pickup) = build_catalog_pickup(&rom, 0);

    for seed in 0..6u64 {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let result = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, BuildFlags { shuffle_toad_houses: true, ..Default::default() });
        let built = &result.worlds[5]; // W6 (0-indexed)

        eprintln!("\n===== Seed {seed} — W6 =====");
        eprintln!("level_count received: {} (from distribute_levels)",
            built.slots.iter().filter(|s| s.kind == SlotKind::Level).count());
        eprintln!("fort_count: {}", result.fort_counts[5]);
        eprintln!("total slots: {}", built.slots.len());
        eprintln!("section_count: {}", built.section_count);
        eprintln!("pipe_pairs: {}", built.pipe_pairs.len());

        // Group by kind
        let mut fortresses = Vec::new();
        let mut levels = Vec::new();
        let mut hammer_bros = Vec::new();
        let mut pipes = Vec::new();
        let mut bonus_games = Vec::new();
        let mut toad_houses = Vec::new();
        for slot in &built.slots {
            match slot.kind {
                SlotKind::Fortress => fortresses.push(slot),
                SlotKind::Level => levels.push(slot),
                SlotKind::HammerBro => hammer_bros.push(slot),
                SlotKind::Pipe => pipes.push(slot),
                SlotKind::BonusGame => bonus_games.push(slot),
                SlotKind::ToadHouse => toad_houses.push(slot),
            }
        }

        eprintln!("\nFortresses ({}):", fortresses.len());
        for s in &fortresses {
            eprintln!("  ({:2}, {:2})  section={}", s.pos.0, s.pos.1, s.section);
        }

        eprintln!("\nLevels ({}):", levels.len());
        for s in &levels {
            // Compute min Manhattan distance to nearest other Level slot
            let min_dist = levels.iter()
                .filter(|o| o.pos != s.pos)
                .map(|o| {
                    let dr = (s.pos.0 as isize - o.pos.0 as isize).unsigned_abs();
                    let dc = (s.pos.1 as isize - o.pos.1 as isize).unsigned_abs();
                    dr + dc
                })
                .min()
                .unwrap_or(0);
            eprintln!("  ({:2}, {:2})  section={}  min_dist_to_level={}", s.pos.0, s.pos.1, s.section, min_dist);
        }

        eprintln!("\nHammerBros ({}):", hammer_bros.len());
        for s in &hammer_bros {
            eprintln!("  ({:2}, {:2})  section={}", s.pos.0, s.pos.1, s.section);
        }

        eprintln!("\nPipes ({}):", pipes.len());
        for s in &pipes {
            eprintln!("  ({:2}, {:2})  section={}", s.pos.0, s.pos.1, s.section);
        }

        eprintln!("\nBonus Games ({}):", bonus_games.len());
        for s in &bonus_games {
            eprintln!("  ({:2}, {:2})  section={}", s.pos.0, s.pos.1, s.section);
        }

        eprintln!("\nToad Houses ({}):", toad_houses.len());
        for s in &toad_houses {
            eprintln!("  ({:2}, {:2})  section={}", s.pos.0, s.pos.1, s.section);
        }

        eprintln!("\nLocks ({}):", built.locks.len());
        for l in &built.locks {
            eprintln!("  ({:2}, {:2})  gap=0x{:02X}  replace=0x{:02X}  fort_section={}  safe={}",
                l.pos.0, l.pos.1, l.gap_tile, l.replace_tile, l.fort_section, l.secret_exit_safe);
        }
    }
}

#[test]
#[ignore]
fn test_dump_w7_blank_vs_bfs() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };
    let (catalog, pickup) = build_catalog_pickup(&rom, 0);
    let wi = 6; // W7

    let cw = &pickup.worlds[wi];
    eprintln!("\n=== W7 Pickup: {} pool entries ===", cw.pool_indices.len());

    let fixed = fixed_positions_for_world(&rom, &catalog, wi, true, false);
    eprintln!("Fixed positions: {} {:?}", fixed.len(), fixed);

    let blank_positions = find_blank_slots(&cw.grid, &fixed);
    eprintln!("Blank tiles on grid: {}", blank_positions.len());

    // Run the actual build for several seeds and check coverage
    for seed in 0..5u64 {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let result = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, BuildFlags { shuffle_toad_houses: true, ..Default::default() });
        let built = &result.worlds[wi];

        // All positions that got a slot assignment
        let slot_positions: HashSet<(usize, usize)> = built.slots.iter().map(|s| s.pos).collect();
        // Add pipe positions
        let pipe_positions: HashSet<(usize, usize)> = built.pipe_pairs.iter()
            .flat_map(|&(a, b)| vec![a, b]).collect();
        let all_assigned: HashSet<(usize, usize)> = slot_positions.union(&pipe_positions).copied().collect();

        // Blank tiles with no assignment
        let uncovered: Vec<(usize, usize)> = blank_positions.iter()
            .filter(|p| !all_assigned.contains(p))
            .copied()
            .collect();

        let total_slots = built.slots.len() + pipe_positions.len();
        eprintln!("\n--- Seed {seed} ---");
        eprintln!("  Slots: {} (L={}, F={}, P={}, HB={})",
            total_slots,
            built.slots.iter().filter(|s| s.kind == SlotKind::Level).count(),
            built.slots.iter().filter(|s| s.kind == SlotKind::Fortress).count(),
            pipe_positions.len(),
            built.slots.iter().filter(|s| s.kind == SlotKind::HammerBro).count(),
        );
        eprintln!("  Pool entries (ptr slots): {}", cw.pool_indices.len());
        eprintln!("  max_non_pipe_slots: {}", cw.pool_indices.len() - VANILLA_PIPE_PAIRS[wi] * 2);
        eprintln!("  Blanks on grid: {}", blank_positions.len());
        eprintln!("  Assigned positions: {}", all_assigned.len());
        eprintln!("  Uncovered blanks: {}", uncovered.len());

        if !uncovered.is_empty() {
            for (r, c) in &uncovered {
                eprintln!("    UNCOVERED: ({},{}) tile=${:02X}", r, c, cw.grid.get(*r, *c));
                // Check if BFS can reach it with the placed pipes
                let bfs_all = bfs_ordered(&built.grid, &built.pipe_pairs, rom_data::find_start(&built.grid), built.world_idx);
                let bfs_set: HashSet<(usize, usize)> = bfs_all.iter().map(|&(p, _)| p).collect();
                eprintln!("      BFS reachable: {}", bfs_set.contains(&(*r, *c)));
            }
        }

        // Check for assignments NOT on blank tiles (double-covering or wrong pos)
        let non_blank_assignments: Vec<_> = all_assigned.iter()
            .filter(|p| !blank_positions.contains(p) && !pipe_positions.contains(p))
            .collect();
        if !non_blank_assignments.is_empty() {
            eprintln!("  Assignments on non-blank tiles:");
            for &&(r, c) in &non_blank_assignments {
                eprintln!("    ({},{}) tile=${:02X}", r, c, cw.grid.get(r, c));
            }
        }
    }
}

#[test]
#[ignore]
fn test_lock_scoring_detail() {
    let rom = match load_rom() {
        Some(r) => r,
        None => {
            eprintln!("ROM not found, skipping");
            return;
        }
    };
    let (catalog, pickup) = build_catalog_pickup(&rom, 0);

    for seed in [42u64, 123, 999] {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let result = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, BuildFlags { shuffle_toad_houses: true, ..Default::default() });

        eprintln!("\n{}", "=".repeat(60));
        eprintln!("=== Seed {seed} ===");

        for built in &result.worlds {
            let wi = built.world_idx;
            let target_pos = find_target(&built.grid, wi);
            let start_pos = rom_data::find_start(&built.grid);

            let forts: Vec<_> = built.slots.iter()
                .filter(|s| s.kind == SlotKind::Fortress)
                .collect();
            let levels: Vec<_> = built.slots.iter()
                .filter(|s| s.kind == SlotKind::Level)
                .collect();

            eprintln!("\n  W{}: {} forts, {} levels, {} pipes, {} locks, target={:?}",
                wi + 1, forts.len(), levels.len(), built.pipe_pairs.len(),
                built.locks.len(), target_pos);

            // Build stamped grid (no locks), same as production
            let mut base_grid = built.grid.clone();
            stamp_slots(&mut base_grid, &built.slots);

            for lock in &built.locks {
                // Open grid: no locks
                let walk_open = walk_map(&base_grid, &built.pipe_pairs, start_pos, built.world_idx);

                // Locked grid: this lock closed
                let mut locked_grid = base_grid.clone();
                locked_grid.set(lock.pos.0, lock.pos.1, lock.gap_tile);
                let walk_locked = walk_map(&locked_grid, &built.pipe_pairs, start_pos, built.world_idx);

                let gated_count = walk_open.nodes.len() as i32 - walk_locked.nodes.len() as i32;

                // What specifically gets gated?
                let gated_forts: Vec<_> = forts.iter()
                    .filter(|f| walk_open.nodes.contains(&f.pos) && !walk_locked.nodes.contains(&f.pos))
                    .collect();
                let gated_levels: Vec<_> = levels.iter()
                    .filter(|l| walk_open.nodes.contains(&l.pos) && !walk_locked.nodes.contains(&l.pos))
                    .collect();
                let gates_target = target_pos
                    .map(|tp| walk_open.nodes.contains(&tp) && !walk_locked.nodes.contains(&tp))
                    .unwrap_or(false);

                // BFS distance from lock to target (via adjacent nodes)
                let target_dist = if let Some(tp) = target_pos {
                    let walk_from_target = walk_map(&base_grid, &built.pipe_pairs, Some(tp), built.world_idx);
                    let (lr, lc) = lock.pos;
                    [(-1i16, 0i16), (1, 0), (0, -1), (0, 1)].iter()
                        .filter_map(|&(dr, dc)| {
                            let nr = lr as i16 + dr;
                            let nc = lc as i16 + dc;
                            if nr < 0 || nc < 0 { return None; }
                            walk_from_target.distances.get(&(nr as usize, nc as usize)).copied()
                        })
                        .min()
                } else {
                    None
                };

                eprintln!("    Lock ({:2},{:2}) sect={} safe={:<5} gated={:<3} dist_to_target={:<4} gates: {} forts, {} levels{}",
                    lock.pos.0, lock.pos.1,
                    lock.fort_section,
                    lock.secret_exit_safe,
                    gated_count,
                    target_dist.map(|d| d.to_string()).unwrap_or("-".into()),
                    gated_forts.len(),
                    gated_levels.len(),
                    if gates_target { ", TARGET" } else { "" },
                );
            }
        }
    }
}

/// Dump all lock candidates and their scores for a specific world.
/// Usage: change seed/target_wi below, then run with --nocapture.
#[test]
#[ignore]
fn test_lock_candidates_dump() {
    let rom = match load_rom() {
        Some(r) => r,
        None => { eprintln!("ROM not found"); return; }
    };
    let seed = 42u64;
    let target_wi = 6; // 0-indexed: W7 = 6

    let (catalog, pickup) = build_catalog_pickup(&rom, seed);
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let result = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, BuildFlags { shuffle_toad_houses: true, ..Default::default() });
    let built = &result.worlds[target_wi];

    let start_pos = rom_data::find_start(&built.grid);
    let target_pos = find_target(&built.grid, target_wi);

    // Build base grid with slots stamped (no locks), same as production
    let mut base_grid = built.grid.clone();
    stamp_slots(&mut base_grid, &built.slots);

    eprintln!("\n=== Seed {seed}, W{} — Lock Candidate Dump ===", target_wi + 1);
    eprintln!("Target: {:?}, Start: {:?}", target_pos, start_pos);
    eprintln!("Forts: {}, Sections: {}", result.fort_counts[target_wi], built.section_count);

    // For each section, enumerate all lockable tiles and score them
    for section_idx in 0..built.section_count {
        let fort_pos = match built.slots.iter()
            .find(|s| s.section == section_idx && s.kind == SlotKind::Fortress)
        {
            Some(s) => s.pos,
            None => continue,
        };

        eprintln!("\n  Section {section_idx} (fort at {:?}):", fort_pos);

        // Open grid for this section: earlier locks open, no current lock
        let walk_open = walk_map(&base_grid, &built.pipe_pairs, start_pos, built.world_idx);
        let open_node_count = walk_open.nodes.len();

        // Find all lockable path tiles
        // (pos, gated, safe, score, blocks_later_fort, blocks_target)
        type LockDebugCandidate = (Pos, i32, bool, i32, bool, bool);
        let mut candidates: Vec<LockDebugCandidate> = Vec::new();

        for r in 0..base_grid.rows() {
            for c in 0..base_grid.cols {
                let tile = base_grid.get(r, c);
                if !LOCKABLE_TILES.contains(&tile) { continue; }

                let gap = gap_tile_for(tile);
                let mut test_grid = base_grid.clone();
                test_grid.set(r, c, gap);
                let walk = walk_map(&test_grid, &built.pipe_pairs, start_pos, built.world_idx);

                // Hard rule: fort must be reachable
                if !walk.nodes.contains(&fort_pos) { continue; }

                let gated = open_node_count as i32 - walk.nodes.len() as i32;
                let target_reachable = target_pos
                    .map(|tp| walk.nodes.contains(&tp))
                    .unwrap_or(true);
                let safe = target_reachable && built.slots.iter().all(|s| {
                    s.kind != SlotKind::Fortress || walk.nodes.contains(&s.pos)
                });
                let blocks_later_fort = built.slots.iter().any(|s| {
                    s.kind == SlotKind::Fortress
                        && s.section > section_idx
                        && !walk.nodes.contains(&s.pos)
                });
                let mut score = gated;
                if blocks_later_fort { score += 100; }

                candidates.push(((r, c), gated, safe, score, blocks_later_fort, !target_reachable));
            }
        }

        // Sort by score descending
        candidates.sort_by_key(|c| std::cmp::Reverse(c.3));

        let chosen = built.locks.iter().find(|l| l.fort_section == section_idx);
        eprintln!("    {} candidates pass hard rules, chosen={:?}",
            candidates.len(),
            chosen.map(|l| l.pos));

        for (pos, gated, safe, score, blf, bt) in &candidates {
            let marker = if chosen.map(|l| l.pos == *pos).unwrap_or(false) { " <-- CHOSEN" } else { "" };
            eprintln!("    ({:2},{:2}) gated={:<3} score={:<4} safe={:<5} blk_fort={:<5} blk_target={}{marker}",
                pos.0, pos.1, gated, score, safe, blf, bt);
        }
    }
}

#[test]
#[ignore]
fn test_lock_airship_distance() {
    let rom = match load_rom() {
        Some(r) => r,
        None => {
            eprintln!("ROM not found, skipping");
            return;
        }
    };
    let rom = apply_qol_for_overworld(&rom);

    let seeds: u64 = std::env::var("LOCK_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);
    // BFS distance histogram: index = distance, value = count
    let mut histogram = [0u32; 30];
    let mut total_locks = 0u32;
    let mut no_target_locks = 0u32;
    // Per-world stats: (sum_of_distances, count)
    let mut per_world: [(u64, u32); 8] = [(0, 0); 8];
    // Track locks at distance <= 2 per seed for flagging
    let mut close_lock_seeds = 0u32;
    // Inter-lock Manhattan distance (only for worlds with 2+ locks)
    let mut inter_hist = [0u32; 40];
    let mut total_pairs = 0u32;
    let mut per_world_pairs: [(u64, u32); 8] = [(0, 0); 8];
    let mut close_pair_seeds = 0u32;

    for seed in 0..seeds {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let (catalog, pickup) = build_catalog_pickup(&rom, seed);
        let result = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, BuildFlags { shuffle_toad_houses: true, ..Default::default() });
        let mut seed_has_close = false;
        let mut seed_has_close_pair = false;

        for built in &result.worlds {
            let wi = built.world_idx;
            let target_pos = find_target(&built.grid, wi);

            // Inter-lock Manhattan distance (works regardless of target).
            if built.locks.len() >= 2 {
                for i in 0..built.locks.len() {
                    for j in (i + 1)..built.locks.len() {
                        let (ar, ac) = built.locks[i].pos;
                        let (br, bc) = built.locks[j].pos;
                        let d = ar.abs_diff(br) + ac.abs_diff(bc);
                        let idx = d.min(inter_hist.len() - 1);
                        inter_hist[idx] += 1;
                        total_pairs += 1;
                        per_world_pairs[wi].0 += d as u64;
                        per_world_pairs[wi].1 += 1;
                        if d <= 3 {
                            seed_has_close_pair = true;
                        }
                    }
                }
            }

            if target_pos.is_none() {
                no_target_locks += built.locks.len() as u32;
                continue;
            }
            let tp = target_pos.unwrap();

            // Build a fully-stamped grid with all locks open so BFS
            // reflects the walkable map. walk_map uses node-to-node
            // hops (nodes are 2 tiles apart), so lock path tiles
            // won't appear in distances. Instead, BFS from the target
            // and measure to the node(s) adjacent to each lock.
            let mut stamped = built.grid.clone();
            stamp_slots(&mut stamped, &built.slots);

            // BFS from target — distances to every reachable node
            let walk_from_target = walk_map(&stamped, &built.pipe_pairs, Some(tp), built.world_idx);

            for lock in &built.locks {
                total_locks += 1;

                // Lock is on a path tile between two nodes. Find the
                // closest adjacent node (in BFS hops from target).
                let (lr, lc) = lock.pos;
                let adjacent_nodes: Vec<(usize, usize)> = [(-1i16, 0i16), (1, 0), (0, -1), (0, 1)]
                    .iter()
                    .filter_map(|&(dr, dc)| {
                        let nr = lr as i16 + dr;
                        let nc = lc as i16 + dc;
                        if nr < 0 || nr >= stamped.rows() as i16 || nc < 0 || nc >= stamped.cols as i16 {
                            return None;
                        }
                        let pos = (nr as usize, nc as usize);
                        // Only count positions that are actual BFS nodes
                        if walk_from_target.distances.contains_key(&pos) {
                            Some(pos)
                        } else {
                            None
                        }
                    })
                    .collect();

                // Use the minimum distance among adjacent nodes
                // (the side closer to the target).
                let min_dist = adjacent_nodes.iter()
                    .filter_map(|pos| walk_from_target.distances.get(pos))
                    .min()
                    .copied();

                if let Some(dist) = min_dist {
                    let idx = dist.min(histogram.len() - 1);
                    histogram[idx] += 1;
                    per_world[wi].0 += dist as u64;
                    per_world[wi].1 += 1;

                    if dist <= 2 {
                        seed_has_close = true;
                    }
                } else {
                    no_target_locks += 1;
                }
            }
        }
        if seed_has_close {
            close_lock_seeds += 1;
        }
        if seed_has_close_pair {
            close_pair_seeds += 1;
        }
    }

    eprintln!("\n=== Lock-to-Airship BFS Distance ({seeds} seeds, {total_locks} locks) ===\n");

    // Histogram
    eprintln!("Distance | Count | Bar");
    eprintln!("---------+-------+----");
    let max_dist_with_data = histogram.iter().rposition(|&c| c > 0).unwrap_or(0);
    for (d, &count) in histogram[..=max_dist_with_data].iter().enumerate() {
        let bar = "#".repeat((count as usize).min(60));
        eprintln!("{d:>5}    | {count:<5} | {bar}");
    }

    // Summary stats
    let total_dist: u64 = histogram.iter().enumerate().map(|(d, &c)| d as u64 * c as u64).sum();
    let mean = total_dist as f64 / total_locks.max(1) as f64;
    let close = histogram[0] + histogram[1] + histogram[2];
    let close_pct = close as f64 / total_locks.max(1) as f64 * 100.0;

    eprintln!("\nMean distance:         {mean:.1}");
    eprintln!("Locks at dist <= 2:    {close}/{total_locks} ({close_pct:.1}%)");
    eprintln!("Seeds with any <= 2:   {close_lock_seeds}/{seeds}");
    if no_target_locks > 0 {
        eprintln!("Locks without target:  {no_target_locks}");
    }

    eprintln!("\nPer-world averages:");
    for (wi, &(sum, count)) in per_world.iter().enumerate() {
        if count > 0 {
            let avg = sum as f64 / count as f64;
            eprintln!("  W{}: {avg:.1} avg ({count} locks)", wi + 1);
        }
    }

    eprintln!("\n=== Inter-Lock Manhattan Distance ({total_pairs} pairs) ===\n");
    eprintln!("Distance | Count | Bar");
    eprintln!("---------+-------+----");
    let max_inter = inter_hist.iter().rposition(|&c| c > 0).unwrap_or(0);
    for (d, &count) in inter_hist[..=max_inter].iter().enumerate() {
        let bar = "#".repeat((count as usize).min(60));
        eprintln!("{d:>5}    | {count:<5} | {bar}");
    }
    let inter_total: u64 = inter_hist.iter().enumerate().map(|(d, &c)| d as u64 * c as u64).sum();
    let inter_mean = inter_total as f64 / total_pairs.max(1) as f64;
    let close_pairs: u32 = inter_hist[..=3].iter().sum();
    let close_pair_pct = close_pairs as f64 / total_pairs.max(1) as f64 * 100.0;
    eprintln!("\nMean pair distance:    {inter_mean:.1}");
    eprintln!("Pairs at dist <= 3:    {close_pairs}/{total_pairs} ({close_pair_pct:.1}%)");
    eprintln!("Seeds with any pair <=3: {close_pair_seeds}/{seeds}");
    eprintln!("\nPer-world pair averages:");
    for (wi, &(sum, count)) in per_world_pairs.iter().enumerate() {
        if count > 0 {
            let avg = sum as f64 / count as f64;
            eprintln!("  W{}: {avg:.1} avg ({count} pairs)", wi + 1);
        }
    }
}

/// Distribution analyzer for pipe placement.
///
/// Runs the builder for N seeds and reports, per world:
///   - endpoint frequency (how often each position appears as a pipe end)
///   - unordered-pair frequency
///   - Shannon entropy of the endpoint distribution (bits)
///   - top-5 most-picked endpoints and pairs
///
/// Use the entropy number to compare scoring tweaks: higher = more variety.
/// Run with: cargo test --release test_pipe_distribution -- --ignored --nocapture
#[test]
#[ignore]
fn test_pipe_distribution() {
    let rom = match load_rom() {
        Some(r) => r,
        None => {
            eprintln!("ROM not found, skipping");
            return;
        }
    };
    let rom = apply_qol_for_overworld(&rom);

    let seeds: u64 = std::env::var("PIPE_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);

    // Per-world tallies
    let mut endpoint_counts: [HashMap<(usize, usize), u32>; 8] = Default::default();
    let mut pair_counts: [HashMap<TeleportEdge, u32>; 8] = Default::default();
    let mut total_endpoints = [0u32; 8];
    let mut total_pairs = [0u32; 8];

    for seed in 0..seeds {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let (catalog, pickup) = build_catalog_pickup(&rom, seed);
        let result = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, BuildFlags { shuffle_toad_houses: true, ..Default::default() });

        for built in &result.worlds {
            let wi = built.world_idx;
            for &(a, b) in &built.pipe_pairs {
                *endpoint_counts[wi].entry(a).or_insert(0) += 1;
                *endpoint_counts[wi].entry(b).or_insert(0) += 1;
                total_endpoints[wi] += 2;

                // Normalize unordered pair (smaller first)
                let pair = if a <= b { (a, b) } else { (b, a) };
                *pair_counts[wi].entry(pair).or_insert(0) += 1;
                total_pairs[wi] += 1;
            }
        }
    }

    eprintln!("\n=== Pipe Distribution over {seeds} seeds ===");

    for wi in 0..8 {
        let expected_pairs = VANILLA_PIPE_PAIRS[wi];
        if expected_pairs == 0 {
            continue;
        }

        let endpoints = &endpoint_counts[wi];
        let pairs = &pair_counts[wi];
        let total_ep = total_endpoints[wi] as f64;
        let total_pr = total_pairs[wi] as f64;

        // Shannon entropy (bits) over endpoint distribution
        let entropy = shannon_entropy(endpoints.values(), total_ep);
        // Max entropy if uniform over all observed endpoints
        let max_entropy = (endpoints.len() as f64).log2();

        // Same for pairs
        let pair_entropy = shannon_entropy(pairs.values(), total_pr);
        let pair_max_entropy = (pairs.len() as f64).log2();

        eprintln!(
            "\n--- W{} ({} pair{}/seed) ---",
            wi + 1,
            expected_pairs,
            if expected_pairs == 1 { "" } else { "s" },
        );
        eprintln!(
            "  Endpoints: {} unique  |  entropy {:.2} / {:.2} bits ({:.0}%)",
            endpoints.len(),
            entropy,
            max_entropy,
            if max_entropy > 0.0 { entropy / max_entropy * 100.0 } else { 0.0 },
        );
        eprintln!(
            "  Pairs:     {} unique  |  entropy {:.2} / {:.2} bits ({:.0}%)",
            pairs.len(),
            pair_entropy,
            pair_max_entropy,
            if pair_max_entropy > 0.0 { pair_entropy / pair_max_entropy * 100.0 } else { 0.0 },
        );

        let mut ep_sorted: Vec<_> = endpoints.iter().collect();
        ep_sorted.sort_by(|a, b| b.1.cmp(a.1));
        eprintln!("  Top endpoints:");
        for (pos, count) in ep_sorted.iter().take(5) {
            let count = **count;
            let pct = count as f64 / total_ep * 100.0;
            let bar = "#".repeat((pct as usize).min(40));
            eprintln!(
                "    ({:2},{:2})  {:>5} ({:5.1}%)  {bar}",
                pos.0, pos.1, count, pct,
            );
        }

        let mut pr_sorted: Vec<_> = pairs.iter().collect();
        pr_sorted.sort_by(|a, b| b.1.cmp(a.1));
        eprintln!("  Top pairs:");
        for (pair, count) in pr_sorted.iter().take(5) {
            let count = **count;
            let pct = count as f64 / total_pr * 100.0;
            let bar = "#".repeat((pct as usize).min(40));
            eprintln!(
                "    ({:2},{:2}) <-> ({:2},{:2})  {:>5} ({:5.1}%)  {bar}",
                pair.0.0, pair.0.1, pair.1.0, pair.1.1, count, pct,
            );
        }
    }

    // One-line summary line for easy before/after diffing
    eprintln!("\n=== Endpoint entropy summary (bits) ===");
    let summary: Vec<String> = (0..8)
        .filter(|&wi| VANILLA_PIPE_PAIRS[wi] > 0)
        .map(|wi| {
            let entropy = shannon_entropy(endpoint_counts[wi].values(), total_endpoints[wi] as f64);
            format!("W{}={entropy:.2}", wi + 1)
        })
        .collect();
    eprintln!("  {}", summary.join("  "));
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
/// Override seed count with FORT_SEEDS=N.
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
    let rom = apply_qol_for_overworld(&rom);

    let seeds: u64 = std::env::var("FORT_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);

    // Per-world tallies
    let mut world_counts: [HashMap<(usize, usize), u32>; 8] = Default::default();
    let mut world_total = [0u32; 8];
    // Per-section tallies: [world][section] -> position frequency
    let mut section_counts: [Vec<HashMap<(usize, usize), u32>>; 8] = Default::default();
    let mut section_total: [Vec<u32>; 8] = Default::default();

    for seed in 0..seeds {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let (catalog, pickup) = build_catalog_pickup(&rom, seed);
        let result = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, BuildFlags { shuffle_toad_houses: true, ..Default::default() });

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

/// Quality analyzer for level placement.
///
/// Level placement is deterministic given pipes+forts, so a position-entropy
/// test would mostly just measure upstream randomness. Instead, this measures
/// whether the scoring achieves its stated goals:
///
///   - Spread: avg pairwise distance between placed levels, density-rule
///     violations (pairs within combined radius 4). Anti-clumping is the
///     primary anti-degeneracy signal.
///   - Path bonus: avg detour from start→target shortest path for placed
///     levels vs all candidates. Negative bias = levels biased toward main
///     route, the intended design goal.
///   - Dead-end bonus: % of dead-end candidates that became levels vs the
///     random baseline. Treated as a tiebreaker — it should win where it
///     doesn't conflict with path bias, but not override it.
///
/// Run with: cargo test --release test_level_placement_quality -- --ignored --nocapture
/// Override seed count with LEVEL_SEEDS=N.
#[test]
#[ignore]
fn test_level_placement_quality() {
    let rom = match load_rom() {
        Some(r) => r,
        None => {
            eprintln!("ROM not found, skipping");
            return;
        }
    };
    let rom = apply_qol_for_overworld(&rom);

    let seeds: u64 = std::env::var("LEVEL_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);

    // Per-world aggregates
    let mut total_pairwise_dist = [0u64; 8];
    let mut total_pairs = [0u64; 8];
    let mut density_violations = [0u64; 8];
    let mut dead_ends_picked = [0u64; 8];
    let mut total_dead_end_candidates = [0u64; 8];
    let mut levels_picked = [0u64; 8];
    let mut total_candidates = [0u64; 8];
    let mut total_level_detour = [0u64; 8];
    let mut total_levels_for_detour = [0u64; 8];
    let mut total_candidate_detour = [0u64; 8];
    let mut total_candidates_for_detour = [0u64; 8];

    for seed in 0..seeds {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let (catalog, pickup) = build_catalog_pickup(&rom, seed);
        let result = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, BuildFlags { shuffle_toad_houses: true, ..Default::default() });

        for built in &result.worlds {
            let wi = built.world_idx;
            let start_pos = rom_data::find_start(&built.grid);
            let target_pos = find_target(&built.grid, wi);

            let levels: Vec<(usize, usize)> = built.slots.iter()
                .filter(|s| s.kind == SlotKind::Level)
                .map(|s| s.pos)
                .collect();

            // Candidate pool seen by level placement: all non-fort, non-pipe
            // section positions. In the final state, these became Level or
            // HammerBro slots.
            let candidates: Vec<(usize, usize)> = built.slots.iter()
                .filter(|s| matches!(s.kind, SlotKind::Level | SlotKind::HammerBro))
                .map(|s| s.pos)
                .collect();

            // BFS from start (matches what scoring used).
            let walk = walk_map(&built.grid, &built.pipe_pairs, start_pos, built.world_idx);
            let bfs_distances = &walk.distances;

            // === Spread: avg pairwise distance between placed levels ===
            for i in 0..levels.len() {
                for j in (i + 1)..levels.len() {
                    let manhattan = levels[i].0.abs_diff(levels[j].0)
                        + levels[i].1.abs_diff(levels[j].1);
                    total_pairwise_dist[wi] += manhattan as u64;
                    total_pairs[wi] += 1;

                    // Density rule: max(manhattan, |bfs_diff|) <= 4
                    let bfs_diff = match (bfs_distances.get(&levels[i]), bfs_distances.get(&levels[j])) {
                        (Some(&a), Some(&b)) => a.abs_diff(b),
                        _ => manhattan,
                    };
                    if manhattan.max(bfs_diff) <= 4 {
                        density_violations[wi] += 1;
                    }
                }
            }

            // === Dead-end utilization ===
            for &pos in &candidates {
                if is_dead_end(&built.grid, pos) {
                    total_dead_end_candidates[wi] += 1;
                    if levels.contains(&pos) {
                        dead_ends_picked[wi] += 1;
                    }
                }
            }
            levels_picked[wi] += levels.len() as u64;
            total_candidates[wi] += candidates.len() as u64;

            // === Path detour: levels vs candidate baseline ===
            // Only positions reachable in BOTH directions count — unreachable
            // positions can't have a meaningful detour relative to a route
            // they're not on.
            if let Some(tp) = target_pos {
                let reverse_walk = walk_map(&built.grid, &built.pipe_pairs, Some(tp), built.world_idx);
                if let Some(&td) = bfs_distances.get(&tp) {
                    for &pos in &levels {
                        if let (Some(&fwd), Some(&rev)) = (
                            bfs_distances.get(&pos),
                            reverse_walk.distances.get(&pos),
                        ) {
                            let detour = (fwd + rev).saturating_sub(td);
                            total_level_detour[wi] += detour as u64;
                            total_levels_for_detour[wi] += 1;
                        }
                    }
                    for &pos in &candidates {
                        if let (Some(&fwd), Some(&rev)) = (
                            bfs_distances.get(&pos),
                            reverse_walk.distances.get(&pos),
                        ) {
                            let detour = (fwd + rev).saturating_sub(td);
                            total_candidate_detour[wi] += detour as u64;
                            total_candidates_for_detour[wi] += 1;
                        }
                    }
                }
            }
        }
    }

    eprintln!("\n=== Level Placement Quality over {seeds} seeds ===");

    for wi in 0..8 {
        if total_candidates[wi] == 0 {
            continue;
        }

        eprintln!("\n--- W{} ---", wi + 1);

        // Spread
        if total_pairs[wi] > 0 {
            let avg_pair = total_pairwise_dist[wi] as f64 / total_pairs[wi] as f64;
            let dens_pct = density_violations[wi] as f64 / total_pairs[wi] as f64 * 100.0;
            eprintln!("  Spread:");
            eprintln!("    Avg pairwise level distance: {avg_pair:.1} tiles");
            eprintln!(
                "    Density violations (combined radius <=4): {} / {} pairs ({:.1}%)",
                density_violations[wi], total_pairs[wi], dens_pct,
            );
        }

        // Dead-end bonus
        let dead_end_util = if total_dead_end_candidates[wi] > 0 {
            dead_ends_picked[wi] as f64 / total_dead_end_candidates[wi] as f64 * 100.0
        } else { 0.0 };
        let random_baseline = levels_picked[wi] as f64 / total_candidates[wi] as f64 * 100.0;
        let lift = dead_end_util - random_baseline;
        eprintln!("  Dead-end bonus (+0.5):");
        eprintln!(
            "    Dead-end candidates: {} ({:.1}% of all candidates)",
            total_dead_end_candidates[wi],
            total_dead_end_candidates[wi] as f64 / total_candidates[wi] as f64 * 100.0,
        );
        eprintln!("    Picked as level:     {dead_end_util:.1}%");
        eprintln!("    Random baseline:     {random_baseline:.1}%");
        eprintln!(
            "    Lift: {lift:+.1} pp  ({})",
            if lift.abs() < 2.0 { "negligible" }
            else if lift > 0.0 { "bias toward dead-ends" }
            else { "bias against dead-ends" },
        );

        // Path bonus
        if total_levels_for_detour[wi] > 0 && total_candidates_for_detour[wi] > 0 {
            let avg_lvl = total_level_detour[wi] as f64 / total_levels_for_detour[wi] as f64;
            let avg_cand = total_candidate_detour[wi] as f64 / total_candidates_for_detour[wi] as f64;
            let bias = avg_lvl - avg_cand;
            eprintln!("  Path bonus (max = PATH_DETOUR_CAP * W_PATH):");
            eprintln!("    Avg detour for placed levels: {avg_lvl:.2} hops");
            eprintln!("    Avg detour for all candidates: {avg_cand:.2} hops");
            eprintln!(
                "    Bias: {bias:+.2} hops  ({})",
                if bias.abs() < 0.3 { "negligible" }
                else if bias < 0.0 { "toward main route" }
                else { "off main route" },
            );
        }
    }

    // One-line summary for diffing
    eprintln!("\n=== Summary (avg pairwise distance / dead-end lift / path bias) ===");
    for wi in 0..8 {
        if total_candidates[wi] == 0 { continue; }
        let avg_pair = if total_pairs[wi] > 0 {
            total_pairwise_dist[wi] as f64 / total_pairs[wi] as f64
        } else { 0.0 };
        let dead_end_util = if total_dead_end_candidates[wi] > 0 {
            dead_ends_picked[wi] as f64 / total_dead_end_candidates[wi] as f64 * 100.0
        } else { 0.0 };
        let random_baseline = levels_picked[wi] as f64 / total_candidates[wi] as f64 * 100.0;
        let lift = dead_end_util - random_baseline;
        let path_bias = if total_levels_for_detour[wi] > 0 && total_candidates_for_detour[wi] > 0 {
            let avg_lvl = total_level_detour[wi] as f64 / total_levels_for_detour[wi] as f64;
            let avg_cand = total_candidate_detour[wi] as f64 / total_candidates_for_detour[wi] as f64;
            avg_lvl - avg_cand
        } else { 0.0 };
        eprintln!(
            "  W{}: dist={avg_pair:5.1}  dead-end-lift={lift:+5.1}pp  path-bias={path_bias:+5.2}",
            wi + 1,
        );
    }
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

/// Required-progression analyzer.
///
/// Per world, computes the minimum number of fortress + level entries
/// the player must clear to reach the airship/Bowser. Locks block path
/// tiles until the fortress whose section opens them is cleared; pipes
/// are taken whenever they shorten the route. Also reports a "hammer
/// mode" where all locks start open, isolating fortresses that were
/// only required because of lock gating.
///
/// Run with: cargo test --release test_required_progression -- --ignored --nocapture
/// Override seed count with PROG_SEEDS=N.
/// Toggle start↔airship swap with SAS=1.
#[test]
#[ignore]
fn test_required_progression() {
    let rom = match load_rom() {
        Some(r) => r,
        None => {
            eprintln!("ROM not found, skipping");
            return;
        }
    };
    let rom = apply_qol_for_overworld(&rom);

    let seeds: u64 = std::env::var("PROG_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);

    // Per-world tallies: sums for mean, plus min/max.
    let mut sum_forts = [0u64; 8];
    let mut sum_levels = [0u64; 8];
    let mut sum_h_forts = [0u64; 8];
    let mut sum_h_levels = [0u64; 8];
    let mut min_forts = [usize::MAX; 8];
    let mut max_forts = [0usize; 8];
    let mut min_levels = [usize::MAX; 8];
    let mut max_levels = [0usize; 8];
    let mut min_h_forts = [usize::MAX; 8];
    let mut max_h_forts = [0usize; 8];
    let mut unreachable = [0u32; 8];
    let mut unreachable_seeds: [Vec<u64>; 8] = Default::default();
    // "Trivial bypass" = hammerless playthrough requires 0 forts AND 0
    // levels (player walks/pipes straight to the airship). Tracked per
    // world plus classified by whether the path uses a pipe right after
    // start (pipe_start), right before target (pipe_target), both, or
    // neither — diagnostic that pinpoints the failure mode.
    let mut zero_zero = [0u32; 8];
    let mut zero_zero_seeds: [Vec<u64>; 8] = Default::default();
    let mut bypass_both = [0u32; 8];
    let mut bypass_start = [0u32; 8];
    let mut bypass_target = [0u32; 8];
    let mut bypass_other = [0u32; 8];

    for seed in 0..seeds {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let (catalog, pickup) = build_catalog_pickup(&rom, seed);
        let result = build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, BuildFlags { shuffle_toad_houses: true, ..Default::default() });

        for built in &result.worlds {
            let wi = built.world_idx;
            let no_hammer = analyze_required_progression(built, false);
            let with_hammer = analyze_required_progression(built, true);

            if !no_hammer.reachable {
                unreachable[wi] += 1;
                if unreachable_seeds[wi].len() < 5 {
                    unreachable_seeds[wi].push(seed);
                }
                continue;
            }
            if no_hammer.forts_required == 0 && no_hammer.levels_required == 0 {
                zero_zero[wi] += 1;
                if zero_zero_seeds[wi].len() < 5 {
                    zero_zero_seeds[wi].push(seed);
                }
                let path = &no_hammer.path;
                let pipe_after_start = path.get(1).is_some_and(|(_, k)| matches!(k, PathNodeKind::Pipe));
                let pipe_before_target = path.len() >= 2
                    && matches!(path[path.len() - 2].1, PathNodeKind::Pipe);
                match (pipe_after_start, pipe_before_target) {
                    (true, true)  => bypass_both[wi]  += 1,
                    (true, false) => bypass_start[wi] += 1,
                    (false, true) => bypass_target[wi] += 1,
                    (false, false)=> bypass_other[wi] += 1,
                }
            }
            sum_forts[wi] += no_hammer.forts_required as u64;
            sum_levels[wi] += no_hammer.levels_required as u64;
            sum_h_forts[wi] += with_hammer.forts_required as u64;
            sum_h_levels[wi] += with_hammer.levels_required as u64;

            min_forts[wi] = min_forts[wi].min(no_hammer.forts_required);
            max_forts[wi] = max_forts[wi].max(no_hammer.forts_required);
            min_levels[wi] = min_levels[wi].min(no_hammer.levels_required);
            max_levels[wi] = max_levels[wi].max(no_hammer.levels_required);
            min_h_forts[wi] = min_h_forts[wi].min(with_hammer.forts_required);
            max_h_forts[wi] = max_h_forts[wi].max(with_hammer.forts_required);
        }
    }

    let sas_label = if std::env::var("SAS").is_ok() { " [SAS=1]" } else { "" };
    eprintln!("\n=== Required Progression to Airship ({seeds} seeds{sas_label}) ===");
    eprintln!();
    eprintln!(
        "{:<4} {:>8} {:>8}  {:>8} {:>8}   {:>8} {:>8}  {:>8}",
        "", "forts", "(range)", "levels", "(range)", "h-forts", "(range)", "saves",
    );

    let mut grand_forts = 0u64;
    let mut grand_levels = 0u64;
    let mut grand_h_forts = 0u64;
    let mut grand_h_levels = 0u64;

    for wi in 0..8 {
        let seeds_ok = (seeds as u32 - unreachable[wi]) as f64;
        if seeds_ok == 0.0 {
            eprintln!("  W{}: (no reachable seeds)", wi + 1);
            continue;
        }
        let avg_f = sum_forts[wi] as f64 / seeds_ok;
        let avg_l = sum_levels[wi] as f64 / seeds_ok;
        let avg_hf = sum_h_forts[wi] as f64 / seeds_ok;
        let saves = avg_f - avg_hf;

        grand_forts += sum_forts[wi];
        grand_levels += sum_levels[wi];
        grand_h_forts += sum_h_forts[wi];
        grand_h_levels += sum_h_levels[wi];

        eprintln!(
            "  W{}  {:>6.2}   {}-{:<3}  {:>6.2}   {}-{:<3}    {:>6.2}   {}-{:<3}   {:>5.2}",
            wi + 1,
            avg_f, min_forts[wi], max_forts[wi],
            avg_l, min_levels[wi], max_levels[wi],
            avg_hf, min_h_forts[wi], max_h_forts[wi],
            saves,
        );
    }

    let avg_total_forts = grand_forts as f64 / seeds as f64;
    let avg_total_levels = grand_levels as f64 / seeds as f64;
    let avg_total_h_forts = grand_h_forts as f64 / seeds as f64;
    let avg_total_h_levels = grand_h_levels as f64 / seeds as f64;
    eprintln!();
    eprintln!("  Per-seed totals (excludes the 8 objectives):");
    eprintln!(
        "    Without hammer: {:.2} forts + {:.2} levels  =  {:.2} clears",
        avg_total_forts, avg_total_levels, avg_total_forts + avg_total_levels,
    );
    eprintln!(
        "    With hammer:    {:.2} forts + {:.2} levels  =  {:.2} clears  (saves {:.2})",
        avg_total_h_forts, avg_total_h_levels,
        avg_total_h_forts + avg_total_h_levels,
        (avg_total_forts + avg_total_levels) - (avg_total_h_forts + avg_total_h_levels),
    );

    let total_unreach: u32 = unreachable.iter().sum();
    if total_unreach > 0 {
        eprintln!("\n  WARNING: {total_unreach} unreachable-target case(s) — builder bug?");
        for (wi, &count) in unreachable.iter().enumerate() {
            if count > 0 {
                let pct = count as f64 / seeds as f64 * 100.0;
                let seed_examples: Vec<String> = unreachable_seeds[wi]
                    .iter()
                    .map(u64::to_string)
                    .collect();
                eprintln!(
                    "    W{}: {count}/{seeds} ({pct:.1}%)  example seeds: {}",
                    wi + 1,
                    seed_examples.join(", "),
                );
            }
        }
    }

    let total_zero_zero: u32 = zero_zero.iter().sum();
    let total_world_seeds = seeds as u32 * 8;
    let overall_pct = total_zero_zero as f64 / total_world_seeds as f64 * 100.0;
    eprintln!(
        "\n  Trivial-bypass (0 forts + 0 levels) — overall {total_zero_zero}/{total_world_seeds} ({overall_pct:.2}%):"
    );
    for (wi, &count) in zero_zero.iter().enumerate() {
        let pct = count as f64 / seeds as f64 * 100.0;
        let examples = if zero_zero_seeds[wi].is_empty() {
            String::new()
        } else {
            let s: Vec<String> = zero_zero_seeds[wi].iter().map(u64::to_string).collect();
            format!("  example seeds: {}", s.join(", "))
        };
        eprintln!("    W{}: {count}/{seeds} ({pct:.1}%){examples}", wi + 1);
    }
    let tb = bypass_both.iter().sum::<u32>();
    let ts = bypass_start.iter().sum::<u32>();
    let tt = bypass_target.iter().sum::<u32>();
    let to_ = bypass_other.iter().sum::<u32>();
    if total_zero_zero > 0 {
        eprintln!(
            "  Bypass classification — pipe-both: {tb}, pipe-start-only: {ts}, pipe-target-only: {tt}, neither: {to_}",
        );
    }
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

    // STANDALONE=1 bypasses the full pipeline and runs the builder
    // directly off a fresh `seed_from_u64(seed)` RNG, matching what the
    // distribution analyzer (test_required_progression) sees. Use this
    // to reproduce unreachable-target findings reported by that test.
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
