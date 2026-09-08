use super::*;
use crate::randomize::{
    map_walker, node_catalog, overworld_build, overworld_pickup, piranha_rooms, qol, troll_pipes,
};
use crate::rom::Rom;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

fn load_rom() -> Option<Rom> {
    let data = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
    Rom::from_bytes(&data).ok()
}

/// The ROM as the builder actually sees it in production.
///
/// `randomizer.rs` runs these QoL patches *before* the overworld builder, and
/// they move map tiles — rocks off the pipe shortcuts, the W3 drawbridges, the
/// always-on W8 screen-3 water page. A builder run against plain vanilla is
/// therefore building a map no player ever gets, and it shows: on the unprepped
/// ROM the builder places **14-17** fortress slots depending on the seed, and on
/// the prepped one it places **17 every time**, which is what
/// `redistribute_fortresses` deals.
///
/// `overworld_build::tests::load_rom` has always done this; this module's
/// `load_rom` above never has. The fortress tests below use this one, because a
/// census taken on a map the game does not produce is worth nothing. Fixing the
/// rest of the module is a separate job — several tests here pin values
/// measured against the unprepped map.
fn load_prepped_rom() -> Option<Rom> {
    let mut out = load_rom()?;
    qol::fix_w3_drawbridges(&mut out);
    qol::remove_rocks(&mut out);
    qol::apply_w1_shortcut(&mut out, false);
    qol::apply_w8_bridges(&mut out);
    qol::fix_big_q_block_rooms(&mut out);
    Some(out)
}

/// Standard test pickup: spade games + toad houses shuffled.
fn standard_pickup(
    rom: &Rom,
    catalog: &node_catalog::NodeCatalog,
) -> overworld_pickup::PickupResult {
    overworld_pickup::pick_up(
        rom,
        catalog,
        overworld_pickup::PickupFlags {
            shuffle_spade_games: true,
            shuffle_toad_houses: true,
            ..Default::default()
        },
    )
}

/// Standard test build flags: toad houses shuffled.
fn standard_build_flags() -> overworld_build::BuildFlags {
    overworld_build::BuildFlags { shuffle_toad_houses: true, ..Default::default() }
}

#[test]
fn test_pool_assignment_exhaustive() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };
    let catalog = node_catalog::NodeCatalog::build(&rom, false);
    let pickup = standard_pickup(&rom, &catalog);
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let build = overworld_build::build(
        &rom,
        &OverworldData { pickup: &pickup, catalog: &catalog },
        &mut rng,
        standard_build_flags(),
    );

    let mut rng2 = ChaCha8Rng::seed_from_u64(99);
    let assignments = assign_pool(
        &rom,
        &build,
        &OverworldData { pickup: &pickup, catalog: &catalog },
        &mut rng2,
        WriteFlags::default(),
    );

    // Collect all assigned pool indices.
    let mut used: Vec<usize> = Vec::new();
    for wa in &assignments {
        for a in &wa.fortress {
            used.push(a.pool_idx);
        }
        for a in &wa.level {
            used.push(a.pool_idx);
        }
        for pa in &wa.pipes {
            used.push(pa.pool_idx_a);
            used.push(pa.pool_idx_b);
        }
        if let Some(a) = &wa.airship {
            used.push(a.pool_idx);
        }
        if let Some(a) = &wa.bowser {
            used.push(a.pool_idx);
        }
        for a in &wa.bonus {
            used.push(a.pool_idx);
        }
        for a in &wa.toad {
            used.push(a.pool_idx);
        }
    }

    // No pool entry assigned more than once.
    let total_used = used.len();
    used.sort();
    used.dedup();
    assert_eq!(used.len(), total_used, "duplicate pool assignments detected",);

    // Per-world assignment count must not exceed available pointer table slots.
    for (wi, wa) in assignments.iter().enumerate() {
        let level_like = wa.fortress.len()
            + wa.level.len()
            + wa.pipes.len() * 2
            + wa.bonus.len()
            + wa.toad.len();
        let total = level_like + wa.hammer_bro.len();
        let available = pickup.worlds[wi].pool_indices.len();
        assert!(
            total <= available,
            "W{}: {} assignments exceed {} available pointer table slots",
            wi + 1,
            total,
            available,
        );
    }
}

#[test]
fn test_troll_pipes_never_assigned_hand_levels() {
    // Troll pipes don't clear when beaten — a hand level (8-Hnd1/2/3)
    // behind a troll pipe would be infinitely farmable for items. The
    // level-assignment pass must skip hand levels for troll-pipe slots.
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };
    let catalog = node_catalog::NodeCatalog::build(&rom, false);
    let pickup = standard_pickup(&rom, &catalog);

    for seed in 0u64..32 {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut build = overworld_build::build(
            &rom,
            &OverworldData { pickup: &pickup, catalog: &catalog },
            &mut rng,
            standard_build_flags(),
        );
        troll_pipes::mark_troll_pipes(&mut build, &mut rng);

        let troll_positions: HashSet<(usize, (usize, usize))> = build
            .worlds
            .iter()
            .flat_map(|w| {
                w.slots.iter().filter(|s| s.is_troll_pipe).map(move |s| (w.world_idx, s.pos))
            })
            .collect();

        let assignments = assign_pool(
            &rom,
            &build,
            &OverworldData { pickup: &pickup, catalog: &catalog },
            &mut rng,
            WriteFlags::default(),
        );

        for (wi, wa) in assignments.iter().enumerate() {
            for a in &wa.level {
                if !troll_positions.contains(&(wi, a.pos)) {
                    continue;
                }
                let ce = &catalog.entries[pickup.pool[a.pool_idx].catalog_idx];
                assert!(
                    !rom_data::is_hand_level(ce.world_idx, ce.entry_idx),
                    "seed {seed}: W{} troll pipe at {:?} got hand level (W{} entry {})",
                    wi + 1,
                    a.pos,
                    ce.world_idx + 1,
                    ce.entry_idx,
                );
            }
        }
    }
}

/// Friendlier Levels must (a) actually keep the blocked levels off the map and
/// (b) leave the deck long enough to deal. The draw is a bare
/// `pop_front().expect(...)` against a fixed `VANILLA_LEVEL_COUNT` of slots, so
/// a short deck panics rather than degrading — which makes the count assertion
/// as load-bearing as the blocklist one.
///
/// Both beta arms are checked because they refill differently: with beta stages
/// on the deck is already oversized and absorbs the removals, so no duplicate
/// is needed; with them off every removal becomes a duplicate.
#[test]
fn test_friendlier_levels_blocks_and_refills() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };

    for beta in [false, true] {
        let catalog = node_catalog::NodeCatalog::build(&rom, beta);
        let pickup = standard_pickup(&rom, &catalog);

        for seed in 0u64..16 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let build = overworld_build::build(
                &rom,
                &OverworldData { pickup: &pickup, catalog: &catalog },
                &mut rng,
                standard_build_flags(),
            );
            let assignments = assign_pool(
                &rom,
                &build,
                &OverworldData { pickup: &pickup, catalog: &catalog },
                &mut rng,
                WriteFlags { friendlier_levels: true, ..Default::default() },
            );

            let placed: Vec<usize> =
                assignments.iter().flat_map(|wa| wa.level.iter().map(|a| a.pool_idx)).collect();

            assert_eq!(
                placed.len(),
                overworld_build::VANILLA_LEVEL_COUNT,
                "beta={beta} seed {seed}: dealt {} levels, expected {}",
                placed.len(),
                overworld_build::VANILLA_LEVEL_COUNT,
            );

            let mut seen: HashMap<usize, usize> = HashMap::new();
            for pi in &placed {
                let ce = &catalog.entries[pickup.pool[*pi].catalog_idx];
                assert!(
                    !rom_data::is_friendlier_blocked(&ce.name),
                    "beta={beta} seed {seed}: blocked level {} was placed",
                    ce.name,
                );
                *seen.entry(*pi).or_insert(0) += 1;
            }

            // Nothing is dealt three times, and nothing holding a one-off item
            // is dealt twice.
            for (&pi, &n) in &seen {
                let ce = &catalog.entries[pickup.pool[pi].catalog_idx];
                assert!(n <= 2, "beta={beta} seed {seed}: {} dealt {n} times", ce.name);
                if n > 1 {
                    assert!(
                        !rom_data::is_chest_level(ce.world_idx, ce.entry_idx)
                            && !rom_data::is_hand_level(ce.world_idx, ce.entry_idx),
                        "beta={beta} seed {seed}: {} holds a one-off item and was dealt twice",
                        ce.name,
                    );
                }
            }

            // Beta stages absorb the removals; without them the shortfall is
            // made up with duplicates, one per blocked level.
            let dupes = placed.len() - seen.len();
            let expected = if beta { 0 } else { rom_data::FRIENDLIER_BLOCKED_LEVELS.len() };
            assert_eq!(dupes, expected, "beta={beta} seed {seed}: {dupes} duplicates");
        }
    }
}

/// Deja Vu deck surgery, both modes and both beta arms.
///
/// Three things the deck has to keep true no matter how it is redealt:
///
///  - The deal still fills every slot. The draw is a bare
///    `pop_front().expect(...)` against a fixed `VANILLA_LEVEL_COUNT`, so a
///    short deck panics rather than degrading.
///  - A level holding a one-off inventory item is dealt exactly once — never
///    twice (the item would be handed out twice) and never zero times (it
///    would be unreachable).
///  - The mode does what it says: `Double` caps a level at two tiles, `Wild`
///    does not.
///
/// Both beta arms are checked because they change the deck length before Deja
/// Vu ever sees it: with beta stages on it starts oversized.
#[test]
fn test_deja_vu_repeats_levels() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };

    for beta in [false, true] {
        let catalog = node_catalog::NodeCatalog::build(&rom, beta);
        let pickup = standard_pickup(&rom, &catalog);

        for mode in [DejaVuMode::Double, DejaVuMode::Wild] {
            // Wild only has to *allow* unbounded repeats, so the "it repeats at
            // all" check is over the whole seed range rather than per seed.
            let mut max_copies = 0usize;

            for seed in 0u64..16 {
                let mut rng = ChaCha8Rng::seed_from_u64(seed);
                let build = overworld_build::build(
                    &rom,
                    &OverworldData { pickup: &pickup, catalog: &catalog },
                    &mut rng,
                    standard_build_flags(),
                );
                let assignments = assign_pool(
                    &rom,
                    &build,
                    &OverworldData { pickup: &pickup, catalog: &catalog },
                    &mut rng,
                    WriteFlags { deja_vu: mode, ..Default::default() },
                );

                let placed: Vec<usize> =
                    assignments.iter().flat_map(|wa| wa.level.iter().map(|a| a.pool_idx)).collect();

                assert_eq!(
                    placed.len(),
                    overworld_build::VANILLA_LEVEL_COUNT,
                    "beta={beta} {mode:?} seed {seed}: dealt {} levels",
                    placed.len(),
                );

                let mut seen: HashMap<usize, usize> = HashMap::new();
                for &pi in &placed {
                    *seen.entry(pi).or_insert(0) += 1;
                }

                for (&pi, &n) in &seen {
                    let ce = &catalog.entries[pickup.pool[pi].catalog_idx];
                    let unique = rom_data::is_chest_level(ce.world_idx, ce.entry_idx)
                        || rom_data::is_hand_level(ce.world_idx, ce.entry_idx);
                    if unique {
                        assert_eq!(
                            n, 1,
                            "beta={beta} {mode:?} seed {seed}: {} holds a one-off item and was dealt {n} times",
                            ce.name,
                        );
                    }
                    if mode == DejaVuMode::Double {
                        assert!(
                            n <= 2,
                            "beta={beta} {mode:?} seed {seed}: {} dealt {n} times",
                            ce.name,
                        );
                    }
                    max_copies = max_copies.max(n);
                }

                // Every one-off item still reaches the map.
                for &pi in &level_pool_unique_items(&catalog, &pickup) {
                    assert!(
                        seen.contains_key(&pi),
                        "beta={beta} {mode:?} seed {seed}: {} holds a one-off item and was not dealt",
                        catalog.entries[pickup.pool[pi].catalog_idx].name,
                    );
                }
            }

            assert!(
                max_copies >= 2,
                "beta={beta} {mode:?}: no level was ever repeated across 16 seeds",
            );
        }
    }
}

/// The pool index of 1-F, the one fortress that holds a chest item.
fn fort_1f_pool_idx(
    catalog: &node_catalog::NodeCatalog,
    pickup: &overworld_pickup::PickupResult,
) -> usize {
    let found: Vec<usize> = pickup
        .pool
        .iter()
        .enumerate()
        .filter(|(_, pe)| {
            let ce = &catalog.entries[pe.catalog_idx];
            matches!(ce.kind, NodeKind::Fortress)
                && rom_data::is_chest_level(ce.world_idx, ce.entry_idx)
        })
        .map(|(pi, _)| pi)
        .collect();
    assert_eq!(found.len(), 1, "expected exactly one chest-holding fortress (1-F)");
    found[0]
}

/// Deja Vu's fortress half, both modes and both Friendlier arms.
///
/// This is a **redeal**, the same one the level deck gets: the deck is rebuilt
/// to exactly the number of fortress slots on the map, so a fortress the deal
/// misses sits the seed out. Four things have to hold no matter which arm runs:
///
///  - Every fortress slot still gets a fortress. The draw is a bare `expect`,
///    so a short deck is a panic rather than a gap.
///  - 1-F is dealt exactly once — seeded into the deck ahead of the redeal and
///    never a source. A second copy hands the warp whistle over twice, and its
///    secret exit skips Boom-Boom, so a copy could land on a lock it can never
///    open.
///  - With Friendlier Levels on, the blocked pair is gone from every arm.
///  - The mode does what it says: `Double` caps a fortress at two tiles, `Wild`
///    does not.
///
/// The pairing itself is checked elsewhere — `lock_keys::assert_one_key_per_lock`
/// is what says two tiles holding the same fortress still key two locks, and it
/// holds because a key is a map position, not a level.
#[test]
fn test_deja_vu_repeats_fortresses() {
    let rom = match load_prepped_rom() {
        Some(r) => r,
        None => return,
    };
    let catalog = node_catalog::NodeCatalog::build(&rom, false);
    let pickup = standard_pickup(&rom, &catalog);
    let fort_1f = fort_1f_pool_idx(&catalog, &pickup);

    for mode in [DejaVuMode::Double, DejaVuMode::Wild] {
        for friendlier in [false, true] {
            // Wild only has to *allow* unbounded repeats, and a fort only
            // sometimes sits out, so both are asserted over the seed range
            // rather than per seed.
            let mut max_copies = 0usize;
            let mut ever_sat_out = false;

            for seed in 0u64..16 {
                let mut rng = ChaCha8Rng::seed_from_u64(seed);
                let build = overworld_build::build(
                    &rom,
                    &OverworldData { pickup: &pickup, catalog: &catalog },
                    &mut rng,
                    standard_build_flags(),
                );
                let slots: usize = build
                    .worlds
                    .iter()
                    .map(|w| {
                        w.slots
                            .iter()
                            .filter(|s| s.kind == overworld_build::SlotKind::Fortress)
                            .count()
                    })
                    .sum();

                let assignments = assign_pool(
                    &rom,
                    &build,
                    &OverworldData { pickup: &pickup, catalog: &catalog },
                    &mut rng,
                    WriteFlags {
                        deja_vu: mode,
                        deja_vu_forts: true,
                        friendlier_levels: friendlier,
                        ..Default::default()
                    },
                );
                let placed: Vec<usize> = assignments
                    .iter()
                    .flat_map(|wa| wa.fortress.iter().map(|a| a.pool_idx))
                    .collect();

                assert_eq!(
                    placed.len(),
                    slots,
                    "{mode:?} friendlier={friendlier} seed {seed}: {} forts for {slots} slots",
                    placed.len(),
                );
                assert_eq!(
                    slots,
                    rom_data::FORTRESS_ENTRIES.len(),
                    "{mode:?} friendlier={friendlier} seed {seed}: builder placed {slots} \
                     fortress slots, not the full roster",
                );

                let mut seen: HashMap<usize, usize> = HashMap::new();
                for &pi in &placed {
                    *seen.entry(pi).or_insert(0) += 1;
                }

                assert_eq!(
                    seen.get(&fort_1f).copied().unwrap_or(0),
                    1,
                    "{mode:?} friendlier={friendlier} seed {seed}: 1-F dealt {:?} times, not once",
                    seen.get(&fort_1f),
                );

                for (&pi, &n) in &seen {
                    let name = &catalog.entries[pickup.pool[pi].catalog_idx].name;
                    if friendlier {
                        assert!(
                            !rom_data::is_friendlier_blocked_fort(name),
                            "{mode:?} seed {seed}: blocked fort {name} was placed",
                        );
                    }
                    if mode == DejaVuMode::Double {
                        assert!(
                            n <= 2,
                            "{mode:?} friendlier={friendlier} seed {seed}: {name} dealt {n} times",
                        );
                    }
                    max_copies = max_copies.max(n);
                }

                // A redeal deals `slots` cards from a deck of `slots` distinct
                // forts (minus any the removal took), so fewer distinct forts
                // than cards means somebody sat the seed out.
                ever_sat_out |= seen.len() < placed.len();
            }

            assert!(
                max_copies >= 2,
                "{mode:?} friendlier={friendlier}: no fortress repeated across 16 seeds",
            );
            assert!(
                ever_sat_out,
                "{mode:?} friendlier={friendlier}: no fortress ever sat a seed out, \
                 so this is not behaving like a redeal",
            );
        }
    }
}

/// The regular-level pool entries that hand out a one-off inventory item —/// The regular-level pool entries that hand out a one-off inventory item —
/// the chest levels and the W8 hand rooms. (1-F is a chest level too but is a
/// fortress, so it never sits in the level pool.)
fn level_pool_unique_items(
    catalog: &node_catalog::NodeCatalog,
    pickup: &overworld_pickup::PickupResult,
) -> Vec<usize> {
    pickup
        .pool
        .iter()
        .enumerate()
        .filter(|(_, pe)| {
            let ce = &catalog.entries[pe.catalog_idx];
            matches!(ce.kind, NodeKind::Level)
                && (rom_data::is_chest_level(ce.world_idx, ce.entry_idx)
                    || rom_data::is_hand_level(ce.world_idx, ce.entry_idx))
        })
        .map(|(pi, _)| pi)
        .collect()
}

/// Friendlier Levels' fortress half: 7F2 and 8F1 are **not on the map at all**,
/// and the tiles that would have been theirs take a second visit to a fortress
/// that stayed.
///
/// Three things, and the third is the one that used to be impossible:
///
///  - Neither blocked fort is dealt, on any seed.
///  - Every fortress slot still gets a fortress. The deal is a bare `expect`
///    against however many slots the builder placed, so a card removed without
///    a duplicate to replace it is a panic, not a gap.
///  - 1-F is dealt exactly once. It hands over the warp whistle and its secret
///    exit skips Boom-Boom, so it can be neither duplicated nor dropped.
///
/// Deja Vu is off here, so the duplicates come from the mandatory top-up rather
/// than from a redeal — `test_deja_vu_repeats_fortresses` covers that arm.
#[test]
fn test_friendlier_levels_blocks_forts() {
    let rom = match load_prepped_rom() {
        Some(r) => r,
        None => return,
    };
    let catalog = node_catalog::NodeCatalog::build(&rom, false);
    let pickup = standard_pickup(&rom, &catalog);
    let fort_1f = fort_1f_pool_idx(&catalog, &pickup);

    let mut dupe_hist = [0usize; 3];
    for seed in 0u64..120 {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let build = overworld_build::build(
            &rom,
            &OverworldData { pickup: &pickup, catalog: &catalog },
            &mut rng,
            standard_build_flags(),
        );
        let slots: usize = build
            .worlds
            .iter()
            .map(|w| {
                w.slots.iter().filter(|s| s.kind == overworld_build::SlotKind::Fortress).count()
            })
            .sum();

        let assignments = assign_pool(
            &rom,
            &build,
            &OverworldData { pickup: &pickup, catalog: &catalog },
            &mut rng,
            WriteFlags { friendlier_levels: true, ..Default::default() },
        );
        let placed: Vec<usize> =
            assignments.iter().flat_map(|wa| wa.fortress.iter().map(|a| a.pool_idx)).collect();

        // Two separate claims. Every fortress slot got a fortress — and the
        // builder placed the whole roster in the first place, which is what
        // `redistribute_fortresses` deals (13 + World 8's 4) and what the
        // prepped map reliably yields.
        assert_eq!(placed.len(), slots, "seed {seed}: {} forts for {slots} slots", placed.len());
        assert_eq!(
            slots,
            rom_data::FORTRESS_ENTRIES.len(),
            "seed {seed}: builder placed {slots} fortress slots, not the full roster",
        );

        let mut seen: HashMap<usize, usize> = HashMap::new();
        for &pi in &placed {
            let name = &catalog.entries[pickup.pool[pi].catalog_idx].name;
            assert!(
                !rom_data::is_friendlier_blocked_fort(name),
                "seed {seed}: blocked fort {name} was placed",
            );
            *seen.entry(pi).or_insert(0) += 1;
        }

        assert_eq!(
            seen.get(&fort_1f).copied().unwrap_or(0),
            1,
            "seed {seed}: 1-F was dealt {:?} times, not once",
            seen.get(&fort_1f),
        );

        // The top-up draws without replacement, so nothing reaches a third
        // tile, and it only makes up the shortfall the removal created.
        let dupes = placed.len() - seen.len();
        for (&pi, &n) in &seen {
            assert!(
                n <= 2,
                "seed {seed}: {} dealt {n} times",
                catalog.entries[pickup.pool[pi].catalog_idx].name,
            );
        }
        // Exactly the removal's worth, not merely at most: the roster is full
        // and two cards came out, so two tiles must take a second visit. An
        // inequality here would have hidden the builder dropping a fort.
        assert_eq!(
            dupes,
            rom_data::FRIENDLIER_BLOCKED_FORTS.len(),
            "seed {seed}: {dupes} duplicate forts, expected one per removed fort",
        );
        dupe_hist[dupes] += 1;
    }

    eprintln!(
        "friendlier fort removal over 120 seeds: two second visits {}, one {}, none {}",
        dupe_hist[2], dupe_hist[1], dupe_hist[0],
    );
}

#[test]
fn test_write_deterministic() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };
    let catalog = node_catalog::NodeCatalog::build(&rom, false);
    let pickup = standard_pickup(&rom, &catalog);

    let mut rom1 = rom.clone();
    let mut rom2 = rom.clone();

    for pass in 0..2 {
        let target = if pass == 0 { &mut rom1 } else { &mut rom2 };
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let build = overworld_build::build(
            &rom,
            &OverworldData { pickup: &pickup, catalog: &catalog },
            &mut rng,
            standard_build_flags(),
        );
        write_overworld(
            target,
            &build,
            &OverworldData { pickup: &pickup, catalog: &catalog },
            &mut rng,
            WriteFlags::default(),
        );
    }

    assert_eq!(rom1.data, rom2.data, "same seed must produce identical output");
}

#[test]
fn test_w8_sprites_moved() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };
    let catalog = node_catalog::NodeCatalog::build(&rom, false);
    let pickup = standard_pickup(&rom, &catalog);
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let build = overworld_build::build(
        &rom,
        &OverworldData { pickup: &pickup, catalog: &catalog },
        &mut rng,
        standard_build_flags(),
    );

    let mut test_rom = rom.clone();
    write_overworld(
        &mut test_rom,
        &build,
        &OverworldData { pickup: &pickup, catalog: &catalog },
        &mut rng,
        WriteFlags::default(),
    );

    // Read W8 sprite positions after write.
    let positions = rom_data::read_map_sprite_positions(&test_rom, 7);

    // The army sprites (slots 2-5) should be at slot positions, not vanilla.
    // We can't predict exact positions (random), but they should be valid
    // grid positions within the W8 map.
    for &(row, col) in &positions {
        assert!(row < 9, "W8 sprite row {row} out of range");
        assert!(col < 64, "W8 sprite col {col} out of range");
    }
}

/// Every lock the build placed gets exactly one entry, naming a fortress that
/// really stands where the entry says.
///
/// This replaces `test_fx_slots_valid`, which pinned the byte layout of
/// `FortressFX_W1..W8` — a table the fortress-FX rework deleted, along with the
/// four-locks-per-world ceiling it imposed. What is left to check is the
/// pairing itself, which is the only thing the builder knows and the console
/// cannot derive.
#[test]
fn every_lock_is_paired_with_its_fortress() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };
    let catalog = node_catalog::NodeCatalog::build(&rom, false);
    let pickup = standard_pickup(&rom, &catalog);
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let data = OverworldData { pickup: &pickup, catalog: &catalog };
    let build = overworld_build::build(&rom, &data, &mut rng, standard_build_flags());

    let mut test_rom = rom.clone();
    let fx = write_overworld(&mut test_rom, &build, &data, &mut rng, WriteFlags::default());
    let entries = fx.lock_entries(&build);

    let placed: usize = build.worlds.iter().map(|w| w.locks.len()).sum();
    assert_eq!(entries.len(), placed, "every placed lock needs a key");

    for e in &entries {
        assert_eq!(e.key_world, e.target_world, "a non-maze run has no cross-world locks");
        let world = &build.worlds[e.key_world];
        assert!(
            world.locks.iter().any(|l| l.pos == e.target_pos),
            "W{} entry targets ({},{}), which holds no lock",
            e.key_world + 1,
            e.target_pos.0,
            e.target_pos.1
        );
        assert!(
            world
                .slots
                .iter()
                .any(|s| s.pos == e.key_pos && s.kind == overworld_build::SlotKind::Fortress),
            "W{} entry is keyed on ({},{}), which holds no fortress",
            e.key_world + 1,
            e.key_pos.0,
            e.key_pos.1
        );
    }
}

#[test]
fn test_hammer_bro_redistribution_written() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };
    for seed in 0..16u64 {
        let catalog = node_catalog::NodeCatalog::build(&rom, false);
        let pickup = overworld_pickup::pick_up(
            &rom,
            &catalog,
            overworld_pickup::PickupFlags {
                shuffle_spade_games: true,
                shuffle_toad_houses: true,
                shuffle_hammer_bros: true,
            },
        );
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let data = OverworldData { pickup: &pickup, catalog: &catalog };
        let build = overworld_build::build(
            &rom,
            &data,
            &mut rng,
            overworld_build::BuildFlags {
                shuffle_toad_houses: true,
                shuffle_hammer_bros: true,
                ..Default::default()
            },
        );

        let mut test_rom = rom.clone();
        write_overworld(
            &mut test_rom,
            &build,
            &data,
            &mut rng,
            WriteFlags { shuffle_hammer_bros: true, ..Default::default() },
        );

        // Each world's written HB sprite count matches the build decision,
        // and every sprite landed on the position the builder chose.
        for wi in 0..8 {
            let written: std::collections::HashSet<(usize, usize)> =
                rom_data::read_hb_sprite_positions(&test_rom, wi).into_iter().collect();
            let decided: std::collections::HashSet<(usize, usize)> =
                build.worlds[wi].hb_sprites.iter().map(|s| s.grid_pos).collect();
            assert_eq!(
                written,
                decided,
                "seed {seed} W{}: written HB sprite positions != build decision",
                wi + 1
            );

            // After writing HBs, at least 2 eligible map-object slots remain
            // empty for a runtime white-house spawn. eligible_hb_map_slots
            // counts both placed HBs and still-empty slots, so the empties
            // are the eligible count minus what we wrote.
            let eligible = rom_data::eligible_hb_map_slots(&test_rom, wi).len();
            let empty = eligible - written.len();
            assert!(
                empty >= 2,
                "seed {seed} W{}: only {empty} empty map-object slots left",
                wi + 1
            );
        }

        // The 15 encounters' rewards are all present and non-zero (the
        // vanilla rewards are all real items, just redistributed).
        let rewards = rom_data::collect_hb_sprite_rewards(&test_rom);
        assert_eq!(rewards.len(), 15, "seed {seed}: {} HB rewards written != 15", rewards.len());
        assert!(rewards.iter().all(|&r| r != 0), "seed {seed}: a written HB reward is zero");
    }
}

/// Rows 7 and 8 share ONE completion bit per column, and
/// `Map_Reload_with_Completions` reads row 7 FIRST — it only drops to row 8
/// (`PRG012_A55C`) when row 7's tile matched nothing completable. So anything
/// the engine catches at (7,c) swallows the bit, and content at (8,c) is
/// never marked beaten, never crumbled and never removed on reload.
///
/// The builder barred the partner of every placed *slot* and lock, which
/// missed the third claimant: **map terrain**. W2's oasis (`TILE_POOL`, `$BF`)
/// sits at (7,6), is scenery rather than a pointer entry, and lands exactly on
/// the page-2 `Tile_Attributes_TS0` threshold — so seeds put a level or a
/// fortress at (8,6) that could never show beaten, and a lock there would have
/// grown back on every map reload.
///
/// Asserted on the written ROM rather than on `BuildResult`, because that is
/// where a lock is a tile and terrain, content and locks are finally the same
/// kind of thing — exactly the view the engine has.
#[test]
fn row78_completion_bit_is_never_double_claimed() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };

    for seed in 0..20u64 {
        let catalog = node_catalog::NodeCatalog::build(&rom, false);
        let pickup = overworld_pickup::pick_up(
            &rom,
            &catalog,
            overworld_pickup::PickupFlags {
                shuffle_spade_games: true,
                shuffle_toad_houses: true,
                shuffle_hammer_bros: true,
            },
        );
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let data = OverworldData { pickup: &pickup, catalog: &catalog };
        let build = overworld_build::build(
            &rom,
            &data,
            &mut rng,
            overworld_build::BuildFlags {
                shuffle_toad_houses: true,
                shuffle_hammer_bros: true,
                ..Default::default()
            },
        );

        let mut test_rom = rom.clone();
        write_overworld(
            &mut test_rom,
            &build,
            &data,
            &mut rng,
            WriteFlags { shuffle_hammer_bros: true, ..Default::default() },
        );

        for wi in 0..8 {
            let grid = rom_data::read_tile_grid(&test_rom, wi);
            let tables = &rom_data::WORLDS[wi];
            let entries: std::collections::HashSet<(usize, usize)> = (0..tables.entry_count)
                .map(|i| rom_data::entry_grid_position(&test_rom, tables, i))
                .collect();

            for c in 0..grid.cols {
                let row8 = grid.get(8, c);
                if !overworld_build::is_completion_unsafe(grid.get(7, c))
                    || !overworld_build::is_completion_unsafe(row8)
                {
                    // Either row 7 leaves the bit alone, or row 8 never wanted
                    // it (a Hammer Bro rides a plain path tile the completion
                    // pass does not touch). Two adjacent scenery tiles that
                    // both read as completable — W6's `$EA` band — are the
                    // same harmless case: no entry, no bit, nothing to lose.
                    continue;
                }
                // Row 7 owns the bit. Anything at (8,c) that needs it — a
                // pointer entry, or a lock the reload would redraw — is lost.
                assert!(
                    !entries.contains(&(8, c)) && !rom_data::LOCK_TILES.contains(&row8),
                    "seed {seed} W{}: (7,{c})={:02X} claims the completion bit that \
                     (8,{c})={row8:02X} needs",
                    wi + 1,
                    grid.get(7, c),
                );
            }
        }
    }
}

#[test]
fn test_pointer_table_sorted() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };
    let catalog = node_catalog::NodeCatalog::build(&rom, false);
    let pickup = standard_pickup(&rom, &catalog);
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let build = overworld_build::build(
        &rom,
        &OverworldData { pickup: &pickup, catalog: &catalog },
        &mut rng,
        standard_build_flags(),
    );

    let mut test_rom = rom.clone();
    write_overworld(
        &mut test_rom,
        &build,
        &OverworldData { pickup: &pickup, catalog: &catalog },
        &mut rng,
        WriteFlags::default(),
    );

    // Verify each world's pointer table is sorted by (screen, row, col).
    for (wi, world) in WORLDS.iter().enumerate() {
        let n = world.entry_count;
        let rt = world.rowtype_offset;
        let sc = rt + n;

        let mut prev = (0u8, 0u8, 0u8);
        for i in 0..n {
            let rowtype = test_rom.read_byte(rt + i);
            let scrcol = test_rom.read_byte(sc + i);
            let screen = (scrcol >> 4) & 0x0F;
            let row_nib = (rowtype >> 4) & 0x0F;
            let col = scrcol & 0x0F;
            let key = (screen, row_nib, col);

            assert!(
                key >= prev,
                "W{} entry {i} not sorted: ({},{},{}) < ({},{},{})",
                wi + 1,
                key.0,
                key.1,
                key.2,
                prev.0,
                prev.1,
                prev.2,
            );
            prev = key;
        }
    }
}

/// Every BFS-reachable blank tile must have a pointer table entry after
/// writing. Uncovered blanks crash the game when the player walks onto them.
#[test]
fn test_no_uncovered_blank_nodes() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };
    let catalog = node_catalog::NodeCatalog::build(&rom, false);
    let pickup = standard_pickup(&rom, &catalog);

    for seed in [42u64, 123, 999, 7777, 31337] {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let build = overworld_build::build(
            &rom,
            &OverworldData { pickup: &pickup, catalog: &catalog },
            &mut rng,
            standard_build_flags(),
        );

        let mut test_rom = rom.clone();
        qol::fix_w3_drawbridges(&mut test_rom);
        qol::remove_rocks(&mut test_rom);
        qol::fix_big_q_block_rooms(&mut test_rom);
        write_overworld(
            &mut test_rom,
            &build,
            &OverworldData { pickup: &pickup, catalog: &catalog },
            &mut rng,
            WriteFlags::default(),
        );

        let pipes_by_world = rom_data::read_pipe_pairs(&test_rom);

        for (wi, world) in WORLDS.iter().enumerate() {
            let grid = rom_data::read_tile_grid(&test_rom, wi);
            let pipe_pairs = pipes_by_world.get(&wi).cloned().unwrap_or_default();
            let walk = map_walker::walk_map(&grid, &pipe_pairs, None, wi);

            // Collect positions that have pointer table entries.
            let mut covered: HashSet<(usize, usize)> = HashSet::new();
            for i in 0..world.entry_count {
                let pos = rom_data::entry_grid_position(&test_rom, world, i);
                if pos.0 < grid.rows() {
                    covered.insert(pos);
                }
            }

            // Every reachable blank tile must be covered.
            for &node in &walk.nodes {
                let (r, c) = node;
                if r >= grid.rows() || c >= grid.cols {
                    continue;
                }
                let tile = grid.get(r, c);
                if !rom_data::VALID_BLANK_TILES.contains(&tile) {
                    continue;
                }
                assert!(
                    covered.contains(&node),
                    "seed {seed} W{}: uncovered blank tile ${tile:02X} at ({r},{c})",
                    wi + 1,
                );
            }
        }
    }
}

/// Generate a full ROM for manual/emulator testing.
#[test]
#[ignore]
fn test_generate_rom() {
    let rom = match load_rom() {
        Some(r) => r,
        None => {
            eprintln!("ROM not found, skipping");
            return;
        }
    };
    let catalog = node_catalog::NodeCatalog::build(&rom, false);
    let pickup = standard_pickup(&rom, &catalog);

    for seed in [42u64, 123, 999] {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let build = overworld_build::build(
            &rom,
            &OverworldData { pickup: &pickup, catalog: &catalog },
            &mut rng,
            standard_build_flags(),
        );

        let mut out = rom.clone();

        // Apply QoL patches that the builder expects.
        qol::fix_w3_drawbridges(&mut out);
        qol::remove_rocks(&mut out);
        qol::fix_big_q_block_rooms(&mut out);

        write_overworld(
            &mut out,
            &build,
            &OverworldData { pickup: &pickup, catalog: &catalog },
            &mut rng,
            WriteFlags::default(),
        );

        let filename = format!("writer_test_seed{seed}.nes");
        std::fs::write(&filename, &out.data).unwrap();
        eprintln!("Wrote {filename}");
    }
}

/// Piranha shuffle end-to-end: run the full randomizer in On and Wild
/// modes and check the ROM-side invariants — plants written with no
/// reward byte, sitting on path-node tiles over real pointer entries,
/// and every world keeping enough empty map-object slots for runtime
/// bonus spawns. Off mode must keep the vanilla W7 plants.
#[test]
fn test_piranha_shuffle_plants_written() {
    use crate::{Options, PiranhaMode};

    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };

    let plant_slots = |out: &Rom, wi: usize| -> Vec<(usize, (usize, usize))> {
        (0..9)
            .filter(|&slot| {
                out.read_byte(rom_data::map_obj_slot_offset(
                    out,
                    rom_data::MAP_OBJ_IDS_MASTER,
                    wi,
                    slot,
                )) == 0x07
            })
            .map(|slot| {
                let y = out.read_byte(rom_data::map_obj_slot_offset(
                    out,
                    rom_data::MAP_OBJ_YS_MASTER,
                    wi,
                    slot,
                )) as usize;
                let xhi = out.read_byte(rom_data::map_obj_slot_offset(
                    out,
                    rom_data::MAP_OBJ_XHIS_MASTER,
                    wi,
                    slot,
                )) as usize;
                let xlo = out.read_byte(rom_data::map_obj_slot_offset(
                    out,
                    rom_data::MAP_OBJ_XLOS_MASTER,
                    wi,
                    slot,
                )) as usize;
                (slot, (y / 16 - 2, xhi * 16 + xlo / 16))
            })
            .collect()
    };

    // Per-seed skips are legal (a released plant level can land on a
    // hand-trap slot, a covered tile, or a budget-0 world), so the count
    // thresholds are asserted over a small seed scan — a systematic no-op
    // still fails every seed, while a single unlucky topology doesn't
    // break the suite. The ROM invariants are checked on every plant of
    // every scanned seed.
    for mode in [PiranhaMode::On, PiranhaMode::Wild] {
        let mut met = false;
        for seed in 42..47u64 {
            let mut out = rom.clone();
            let options = Options { piranha_shuffle: mode, palettes: false, ..Default::default() };
            crate::randomizer::randomize(&mut out, seed, &options);

            let mut total_plants = 0;
            for wi in 0..8 {
                let empty = (0..9)
                    .filter(|&slot| {
                        out.read_byte(rom_data::map_obj_slot_offset(
                            &out,
                            rom_data::MAP_OBJ_IDS_MASTER,
                            wi,
                            slot,
                        )) == 0x00
                    })
                    .count();
                assert!(
                    empty >= overworld_build::RESERVED_DYNAMIC_SLOTS,
                    "{mode:?}: W{} has only {empty} empty map-object slots",
                    wi + 1,
                );

                for (slot, (row, col)) in plant_slots(&out, wi) {
                    total_plants += 1;
                    assert_eq!(
                        out.read_byte(rom_data::map_obj_reward_offset(wi, slot)),
                        0,
                        "{mode:?}: relocated plant carries a reward byte",
                    );
                    // Under-tile is a path node, not a numbered level tile.
                    let tile = out.read_byte(rom_data::map_tile_offset(wi, row, col));
                    assert!(
                        !(0x03..=0x15).contains(&tile),
                        "{mode:?}: W{} plant at ({row},{col}) sits on level tile {tile:#04x}",
                        wi + 1,
                    );
                    // A pointer entry (the level the plant fronts) exists there.
                    let world = &rom_data::WORLDS[wi];
                    let found = (0..world.entry_count)
                        .any(|i| rom_data::entry_grid_position(&out, world, i) == (row, col));
                    assert!(
                        found,
                        "{mode:?}: W{} plant at ({row},{col}) has no pointer entry",
                        wi + 1,
                    );
                }
            }
            met = match mode {
                PiranhaMode::On => (1..=2).contains(&total_plants),
                PiranhaMode::Wild => total_plants >= 6,
                PiranhaMode::Off => unreachable!(),
            };
            if met {
                break;
            }
        }
        assert!(
            met,
            "{mode:?}: no seed in the scan produced the expected plant count — feature no-opped",
        );
    }

    // Off: vanilla plants stay at their linked slots with a reward.
    let mut out = rom.clone();
    let options = Options { palettes: false, ..Default::default() };
    crate::randomizer::randomize(&mut out, 42, &options);
    for &(wi, slot, _) in rom_data::MAP_OBJ_ENTRY_LINKS {
        let id = out.read_byte(rom_data::map_obj_slot_offset(
            &out,
            rom_data::MAP_OBJ_IDS_MASTER,
            wi,
            slot,
        ));
        assert_eq!(id, 0x07, "Off: vanilla plant missing at W{} slot {slot}", wi + 1);
        assert_ne!(
            out.read_byte(rom_data::map_obj_reward_offset(wi, slot)),
            0,
            "Off: vanilla plant reward cleared",
        );
    }
}

/// With piranha shuffle active the two plant levels enter the regular
/// level pool — and they end in a treasure chest, so a troll pipe must
/// never disguise them (CHEST_LEVELS membership drives the exclusion,
/// same as 3-7 / 5-1 / 8-Tank).
#[test]
fn test_troll_pipes_never_assigned_piranha_levels() {
    use crate::PiranhaMode;

    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };
    // Piranha-active pipeline: sprites cleared, catalog entries released.
    let mut prepped = rom.clone();
    piranha_rooms::clear_vanilla_plants(&mut prepped);
    let mut catalog = node_catalog::NodeCatalog::build(&prepped, false);
    catalog.release_map_objects();
    let pickup = standard_pickup(&prepped, &catalog);

    // Both released plant levels must be in the pool at all.
    let pooled_piranhas = pickup
        .pool
        .iter()
        .filter(|pe| {
            rom_data::MAP_OBJ_ENTRY_LINKS
                .iter()
                .any(|&(w, _, e)| pe.world_idx == w && pe.entry_idx == e)
        })
        .count();
    assert_eq!(pooled_piranhas, 2, "released plant levels missing from pool");

    for seed in 0u64..32 {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut build = overworld_build::build(
            &prepped,
            &OverworldData { pickup: &pickup, catalog: &catalog },
            &mut rng,
            standard_build_flags(),
        );
        troll_pipes::mark_troll_pipes(&mut build, &mut rng);

        let troll_positions: HashSet<(usize, (usize, usize))> = build
            .worlds
            .iter()
            .flat_map(|w| {
                w.slots.iter().filter(|s| s.is_troll_pipe).map(move |s| (w.world_idx, s.pos))
            })
            .collect();

        let flags = WriteFlags { piranha: PiranhaMode::Wild, ..Default::default() };
        let assignments = assign_pool(
            &prepped,
            &build,
            &OverworldData { pickup: &pickup, catalog: &catalog },
            &mut rng,
            flags,
        );

        for (wi, wa) in assignments.iter().enumerate() {
            for a in &wa.level {
                if !troll_positions.contains(&(wi, a.pos))
                    || wa.demoted_troll_pipes.contains(&a.pos)
                {
                    continue;
                }
                let ce = &catalog.entries[pickup.pool[a.pool_idx].catalog_idx];
                assert!(
                    !rom_data::is_chest_level(ce.world_idx, ce.entry_idx),
                    "seed {seed}: W{} troll pipe at {:?} got chest level (W{} entry {})",
                    wi + 1,
                    a.pos,
                    ce.world_idx + 1,
                    ce.entry_idx,
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// March veto (keep wandering bros off plant/army nodes + hand traps)
// ---------------------------------------------------------------------------

/// Parse the written registry back out of the ROM: per-world address lists.
fn read_veto_registry(rom: &Rom) -> Vec<Vec<u16>> {
    let offs = rom
        .read_range(rom_data::FS_MARCH_VETO + march_veto::ROUTINE_LEN, march_veto::OFFSETS_LEN)
        .to_vec();
    let list = rom
        .read_range(
            rom_data::FS_MARCH_VETO + march_veto::ROUTINE_LEN + march_veto::OFFSETS_LEN,
            march_veto::LIST_LEN,
        )
        .to_vec();
    offs.iter()
        .map(|&o| {
            let mut v = Vec::new();
            let mut i = o as usize;
            while list[i] != 0 {
                v.push(u16::from_be_bytes([list[i], list[i + 1]]));
                i += 2;
            }
            v
        })
        .collect()
}

/// Semantic hook check: the bytes at the hook site must be a JSR whose
/// operand resolves (via the bank mapping, not recomputed arithmetic) to the
/// FS_MARCH_VETO block, and the block must start with the displaced vanilla
/// instruction.
fn assert_veto_hook_installed(rom: &Rom) {
    let hook = rom.read_range(march_veto::MARCH_VETO_HOOK, 3);
    assert_eq!(hook[0], 0x20, "hook must be a JSR");
    let cpu = u16::from_le_bytes([hook[1], hook[2]]);
    assert_eq!(
        rom_data::prg_bank_cpu_to_file(11, cpu),
        rom_data::FS_MARCH_VETO,
        "hook JSR must land on the veto trampoline"
    );
    assert_eq!(
        rom.read_range(rom_data::FS_MARCH_VETO, 3),
        march_veto::DISPLACED_JSR,
        "trampoline must start with the displaced PickTravel JSR"
    );
}

#[test]
fn test_march_veto_registry_roundtrip() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };
    let mut test_rom = rom.clone();
    // W8 armies (world 7 implied) + plants; (7, (4, 6)) duplicates an army.
    let w8 = vec![(2usize, (4usize, 6usize)), (3, (2, 20))];
    let plants = vec![(0usize, (2usize, 3usize)), (7, (4, 6))];
    march_veto::write_march_veto(&mut test_rom, &w8, &plants);

    assert_veto_hook_installed(&test_rom);

    let registry = read_veto_registry(&test_rom);
    assert_eq!(registry[0], vec![march_veto::veto_addr(2, 3)]);
    let w7 = &registry[7];
    assert!(w7.contains(&march_veto::veto_addr(4, 6)));
    assert!(w7.contains(&march_veto::veto_addr(2, 20)));
    assert_eq!(w7.len(), 2, "duplicate army/plant coordinate must dedup");
    for (wi, list) in registry.iter().enumerate().take(7).skip(1) {
        assert!(list.is_empty(), "world {} should have no entries", wi + 1);
    }
    for addr in registry.iter().flatten() {
        assert!(
            (0x6110..=0x66AF).contains(addr),
            "veto address {addr:#06X} outside the map tile SRAM window"
        );
    }
}

/// veto_addr must reproduce the engine's own address computation: the real
/// Tile_Mem_Addr word table (PRG030, file 0x3C010) + $F0 + Temp_Var3.
#[test]
fn test_march_veto_addr_matches_engine_tile_mem_table() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };
    for &(row, col) in &[(0usize, 0usize), (2, 3), (4, 19), (7, 35), (8, 63)] {
        let word_off = 0x3C010 + 2 * (col / 16);
        let screen_base =
            u16::from_le_bytes([rom.read_byte(word_off), rom.read_byte(word_off + 1)]);
        let expected = screen_base + 0xF0 + ((((row as u16) + 2) * 16) | (col as u16 % 16));
        assert_eq!(
            march_veto::veto_addr(row, col),
            expected,
            "veto_addr({row}, {col}) diverges from the engine formula"
        );
    }
}

/// Full pipeline: piranha Wild + HB shuffle -> the hook is installed and the
/// registry is well-formed with W8's army nodes present.
#[test]
fn test_march_veto_pipeline_writes_registry() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };
    let mut prepped = rom.clone();
    piranha_rooms::clear_vanilla_plants(&mut prepped);
    let mut catalog = node_catalog::NodeCatalog::build(&prepped, false);
    catalog.release_map_objects();
    let pickup = overworld_pickup::pick_up(
        &prepped,
        &catalog,
        overworld_pickup::PickupFlags {
            shuffle_spade_games: true,
            shuffle_toad_houses: true,
            shuffle_hammer_bros: true,
        },
    );
    let data = OverworldData { pickup: &pickup, catalog: &catalog };
    let mut rng = ChaCha8Rng::seed_from_u64(7);
    let build = overworld_build::build(
        &prepped,
        &data,
        &mut rng,
        overworld_build::BuildFlags {
            shuffle_toad_houses: true,
            shuffle_hammer_bros: true,
            ..Default::default()
        },
    );
    let mut out = prepped.clone();
    write_overworld(
        &mut out,
        &build,
        &data,
        &mut rng,
        WriteFlags {
            hints: false,
            piranha: PiranhaMode::Wild,
            shuffle_hammer_bros: true,
            friendlier_levels: false,
            deja_vu: DejaVuMode::Off,
            deja_vu_forts: false,
        },
    );

    assert_veto_hook_installed(&out);
    let registry = read_veto_registry(&out);
    let total: usize = registry.iter().map(Vec::len).sum();
    assert!(total <= 16, "registry overflow: {total} entries");
    assert!(
        !registry[7].is_empty(),
        "W8 army nodes must always be vetoed (tank sprite at minimum)"
    );
    for addr in registry.iter().flatten() {
        assert!(
            (0x6110..=0x66AF).contains(addr),
            "veto address {addr:#06X} outside the map tile SRAM window"
        );
    }
}

/// The veto must compose with MaCobra's opt-in limit_bro_movement rewrite in
/// either application order — the two touch disjoint byte ranges.
#[test]
fn test_march_veto_composes_with_limit_bro_movement() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };
    let w8 = vec![(2usize, (4usize, 6usize))];
    let plants = vec![(3usize, (2usize, 9usize))];

    let mut veto_first = rom.clone();
    march_veto::write_march_veto(&mut veto_first, &w8, &plants);
    qol::apply_limit_bro_movement(&mut veto_first);

    let mut limit_first = rom.clone();
    qol::apply_limit_bro_movement(&mut limit_first);
    march_veto::write_march_veto(&mut limit_first, &w8, &plants);

    // Identical output either way: no overlap between the two patches.
    for (range_start, range_len, what) in [
        (0x17398usize, 0x15usize, "limit-bro whitelist table + fill"),
        (0x17419, 8, "limit-bro rewritten scan code"),
        (march_veto::MARCH_VETO_HOOK, 3, "veto hook"),
        (rom_data::FS_MARCH_VETO, 107, "veto trampoline + registry"),
    ] {
        assert_eq!(
            veto_first.read_range(range_start, range_len),
            limit_first.read_range(range_start, range_len),
            "{what} differs between application orders"
        );
    }
    assert_veto_hook_installed(&veto_first);
    assert_veto_hook_installed(&limit_first);

    // Fold-in regression: the old bros_no_hands hook site ($B425) must stay
    // vanilla — hand-trap avoidance now lives in the veto trampoline.
    assert_eq!(veto_first.read_range(0x17435, 3), &[0xD9, 0x98, 0x7E]);
}

/// **The map the writer hands over is the map on the cartridge.**
///
/// `WrittenOverworld::grids` exists so `completion_bits`, `world_travel` and
/// `lock_keys` stop reading the finished map back out of the ROM. That trades
/// a round trip for a second copy, and the copy is only worth having while it
/// agrees with the first — a world's packed-store slice is sized from it, so a
/// single stale cell shifts every world after it and completion marks land in
/// the wrong one. Silent, and several worlds into a playthrough.
///
/// `grids()` debug-asserts this on every call; this runs it as a test over a
/// real build, and the mutation below proves the check can fail.
#[test]
fn grids_match_the_rom() {
    let Some(rom) = load_rom() else { return };
    let catalog = node_catalog::NodeCatalog::build(&rom, false);
    let pickup = standard_pickup(&rom, &catalog);

    for seed in 0..4u64 {
        let mut out = rom.clone();
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let data = OverworldData { pickup: &pickup, catalog: &catalog };
        let build = overworld_build::build(&rom, &data, &mut rng, standard_build_flags());
        let written = write_overworld(&mut out, &build, &data, &mut rng, WriteFlags::default());

        if let Err(why) = grids_agree_with_rom(&written.grids, &out) {
            panic!("seed {seed}: {why}");
        }

        // The check is only worth having if it can fail. Write one map cell
        // straight to the ROM — exactly the mistake it exists to catch — and
        // it must notice.
        let (row, col) = (4usize, 4usize);
        let off = rom_data::map_tile_offset(0, row, col);
        let was = out.read_byte(off);
        out.write_byte(off, was ^ 0xFF);
        assert!(
            grids_agree_with_rom(&written.grids, &out).is_err(),
            "seed {seed}: a map cell was changed behind the writer's back and the guard \
             did not notice — it would not catch the bug it exists for"
        );
        out.write_byte(off, was);
    }
}
