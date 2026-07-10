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

/// Standard test pickup: spade games + toad houses shuffled.
fn standard_pickup(
    rom: &Rom,
    catalog: &node_catalog::NodeCatalog,
) -> overworld_pickup::PickupResult {
    overworld_pickup::pick_up(rom, catalog, overworld_pickup::PickupFlags {
        shuffle_spade_games: true,
        shuffle_toad_houses: true,
        ..Default::default()
    })
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
    let build = overworld_build::build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, standard_build_flags());

    let mut rng2 = ChaCha8Rng::seed_from_u64(99);
    let assignments = assign_pool(&rom, &build, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng2, WriteFlags::default());

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
    assert_eq!(
        used.len(),
        total_used,
        "duplicate pool assignments detected",
    );

    // Per-world assignment count must not exceed available pointer table slots.
    for (wi, wa) in assignments.iter().enumerate() {
        let level_like = wa.fortress.len() + wa.level.len() + wa.pipes.len() * 2 + wa.bonus.len() + wa.toad.len();
        let total = level_like + wa.hammer_bro.len();
        let available = pickup.worlds[wi].pool_indices.len();
        assert!(
            total <= available,
            "W{}: {} assignments exceed {} available pointer table slots",
            wi + 1, total, available,
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
        let mut build = overworld_build::build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, standard_build_flags());
        troll_pipes::mark_troll_pipes(&mut build, &mut rng);

        let troll_positions: HashSet<(usize, (usize, usize))> = build.worlds.iter()
            .flat_map(|w| w.slots.iter()
                .filter(|s| s.is_troll_pipe)
                .map(move |s| (w.world_idx, s.pos)))
            .collect();

        let assignments = assign_pool(&rom, &build, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, WriteFlags::default());

        for (wi, wa) in assignments.iter().enumerate() {
            for a in &wa.level {
                if !troll_positions.contains(&(wi, a.pos)) { continue; }
                let ce = &catalog.entries[pickup.pool[a.pool_idx].catalog_idx];
                assert!(
                    !rom_data::is_hand_level(ce.world_idx, ce.entry_idx),
                    "seed {seed}: W{} troll pipe at {:?} got hand level (W{} entry {})",
                    wi + 1, a.pos, ce.world_idx + 1, ce.entry_idx,
                );
            }
        }
    }
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
        let build = overworld_build::build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, standard_build_flags());
        write_overworld(target, &build, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, WriteFlags::default());
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
    let build = overworld_build::build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, standard_build_flags());

    let mut test_rom = rom.clone();
    write_overworld(&mut test_rom, &build, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, WriteFlags::default());

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

#[test]
fn test_fx_slots_valid() {
    let rom = match load_rom() {
        Some(r) => r,
        None => return,
    };
    let catalog = node_catalog::NodeCatalog::build(&rom, false);
    let pickup = standard_pickup(&rom, &catalog);
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let build = overworld_build::build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, standard_build_flags());

    let mut test_rom = rom.clone();
    write_overworld(&mut test_rom, &build, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, WriteFlags::default());

    // write_fortress_fx hands out FX slots as one running index across
    // worlds in write order: world wi's table holds slots
    // [start, start + lock_count) followed by zeroes, where start is the
    // total lock count of all earlier worlds. Assert the exact bytes.
    let mut expected_slot = 0usize;
    for wi in 0..8 {
        let fx_base = rom_data::FX_WORLD_TABLE + wi * 4;
        let lock_count = build.worlds[wi].locks.len();
        assert!(lock_count <= 4, "W{}: {lock_count} locks exceed 4 FX entries", wi + 1);
        for i in 0..4 {
            let want = if i < lock_count { (expected_slot + i) as u8 } else { 0 };
            assert_eq!(
                test_rom.read_byte(fx_base + i),
                want,
                "W{} FX table entry {i}", wi + 1,
            );
        }
        expected_slot += lock_count;
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
        let build = overworld_build::build(&rom, &data, &mut rng, overworld_build::BuildFlags { shuffle_toad_houses: true, shuffle_hammer_bros: true, ..Default::default() });

        let mut test_rom = rom.clone();
        write_overworld(&mut test_rom, &build, &data, &mut rng, WriteFlags { shuffle_hammer_bros: true, ..Default::default() });

        // Each world's written HB sprite count matches the build decision,
        // and every sprite landed on the position the builder chose.
        for wi in 0..8 {
            let written: std::collections::HashSet<(usize, usize)> =
                rom_data::read_hb_sprite_positions(&test_rom, wi).into_iter().collect();
            let decided: std::collections::HashSet<(usize, usize)> =
                build.worlds[wi].hb_sprites.iter().map(|s| s.grid_pos).collect();
            assert_eq!(
                written, decided,
                "seed {seed} W{}: written HB sprite positions != build decision", wi + 1
            );

            // After writing HBs, at least 2 eligible map-object slots remain
            // empty for a runtime white-house spawn. eligible_hb_map_slots
            // counts both placed HBs and still-empty slots, so the empties
            // are the eligible count minus what we wrote.
            let eligible = rom_data::eligible_hb_map_slots(&test_rom, wi).len();
            let empty = eligible - written.len();
            assert!(
                empty >= 2,
                "seed {seed} W{}: only {empty} empty map-object slots left", wi + 1
            );
        }

        // The 15 encounters' rewards are all present and non-zero (the
        // vanilla rewards are all real items, just redistributed).
        let rewards = rom_data::collect_hb_sprite_rewards(&test_rom);
        assert_eq!(rewards.len(), 15, "seed {seed}: {} HB rewards written != 15", rewards.len());
        assert!(rewards.iter().all(|&r| r != 0), "seed {seed}: a written HB reward is zero");
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
    let build = overworld_build::build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, standard_build_flags());

    let mut test_rom = rom.clone();
    write_overworld(&mut test_rom, &build, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, WriteFlags::default());

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
                wi + 1, key.0, key.1, key.2, prev.0, prev.1, prev.2,
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
        let build = overworld_build::build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, standard_build_flags());

        let mut test_rom = rom.clone();
        qol::fix_w3_drawbridges(&mut test_rom);
        qol::remove_rocks(&mut test_rom);
        qol::fix_big_q_block_rooms(&mut test_rom);
        write_overworld(&mut test_rom, &build, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, WriteFlags::default());

        let pipes_by_world = rom_data::read_pipe_pairs(&test_rom);

        for (wi, world) in WORLDS.iter().enumerate() {
            let grid = rom_data::read_tile_grid(&test_rom, wi);
            let pipe_pairs = pipes_by_world.get(&wi)
                .cloned()
                .unwrap_or_default();
            let walk = map_walker::walk_map(&grid, &pipe_pairs, None, wi);

            // Collect positions that have pointer table entries.
            let mut covered: HashSet<(usize, usize)> = HashSet::new();
            for i in 0..world.entry_count {
                let pos = rom_data::entry_grid_position(&test_rom, world, i);
                if pos.0 < grid.rows {
                    covered.insert(pos);
                }
            }

            // Every reachable blank tile must be covered.
            for &node in &walk.nodes {
                let (r, c) = node;
                if r >= grid.rows || c >= grid.cols {
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
        let build = overworld_build::build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, standard_build_flags());

        let mut out = rom.clone();

        // Apply QoL patches that the builder expects.
        qol::fix_w3_drawbridges(&mut out);
        qol::remove_rocks(&mut out);
        qol::fix_big_q_block_rooms(&mut out);

        write_overworld(&mut out, &build, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, WriteFlags::default());

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
                    out, rom_data::MAP_OBJ_IDS_MASTER, wi, slot,
                )) == 0x07
            })
            .map(|slot| {
                let y = out.read_byte(rom_data::map_obj_slot_offset(
                    out, rom_data::MAP_OBJ_YS_MASTER, wi, slot,
                )) as usize;
                let xhi = out.read_byte(rom_data::map_obj_slot_offset(
                    out, rom_data::MAP_OBJ_XHIS_MASTER, wi, slot,
                )) as usize;
                let xlo = out.read_byte(rom_data::map_obj_slot_offset(
                    out, rom_data::MAP_OBJ_XLOS_MASTER, wi, slot,
                )) as usize;
                (slot, (y / 16 - 2, xhi * 16 + xlo / 16))
            })
            .collect()
    };

    for mode in [PiranhaMode::On, PiranhaMode::Wild] {
        let mut out = rom.clone();
        let options = Options {
            piranha_shuffle: mode,
            palettes: false,
            ..Default::default()
        };
        crate::randomizer::randomize(&mut out, 42, &options);

        let mut total_plants = 0;
        for wi in 0..8 {
            let empty = (0..9)
                .filter(|&slot| {
                    out.read_byte(rom_data::map_obj_slot_offset(
                        &out, rom_data::MAP_OBJ_IDS_MASTER, wi, slot,
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
                let found = (0..world.entry_count).any(|i| {
                    rom_data::entry_grid_position(&out, world, i) == (row, col)
                });
                assert!(
                    found,
                    "{mode:?}: W{} plant at ({row},{col}) has no pointer entry",
                    wi + 1,
                );
            }
        }
        match mode {
            // Both released levels got sprites at seed 42 (skips are
            // possible in principle — hand-trap slot, full world — but
            // zero plants would mean the feature silently no-opped).
            PiranhaMode::On => assert!(
                (1..=2).contains(&total_plants),
                "On: expected 1-2 plants, found {total_plants}",
            ),
            PiranhaMode::Wild => assert!(
                total_plants >= 6,
                "Wild: expected ~1 plant per world, found {total_plants}",
            ),
            PiranhaMode::Off => unreachable!(),
        }
    }

    // Off: vanilla plants stay at their linked slots with a reward.
    let mut out = rom.clone();
    let options = Options { palettes: false, ..Default::default() };
    crate::randomizer::randomize(&mut out, 42, &options);
    for &(wi, slot, _) in rom_data::MAP_OBJ_ENTRY_LINKS {
        let id = out.read_byte(rom_data::map_obj_slot_offset(
            &out, rom_data::MAP_OBJ_IDS_MASTER, wi, slot,
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
    let pooled_piranhas = pickup.pool.iter()
        .filter(|pe| rom_data::MAP_OBJ_ENTRY_LINKS.iter()
            .any(|&(w, _, e)| pe.world_idx == w && pe.entry_idx == e))
        .count();
    assert_eq!(pooled_piranhas, 2, "released plant levels missing from pool");

    for seed in 0u64..32 {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut build = overworld_build::build(&prepped, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, standard_build_flags());
        troll_pipes::mark_troll_pipes(&mut build, &mut rng);

        let troll_positions: HashSet<(usize, (usize, usize))> = build.worlds.iter()
            .flat_map(|w| w.slots.iter()
                .filter(|s| s.is_troll_pipe)
                .map(move |s| (w.world_idx, s.pos)))
            .collect();

        let flags = WriteFlags { piranha: PiranhaMode::Wild, ..Default::default() };
        let assignments = assign_pool(&prepped, &build, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, flags);

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
                    wi + 1, a.pos, ce.world_idx + 1, ce.entry_idx,
                );
            }
        }
    }
}
