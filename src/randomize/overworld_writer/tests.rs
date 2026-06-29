    use super::*;
    use crate::rom::Rom;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    fn load_rom() -> Option<Rom> {
        let data = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&data).ok()
    }

    #[test]
    fn test_pool_assignment_exhaustive() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        let catalog = super::super::node_catalog::NodeCatalog::build(&rom, false);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, super::super::overworld_pickup::PickupFlags { shuffle_spade_games: true, shuffle_toad_houses: true, ..Default::default() });
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let build = super::super::overworld_build::build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, super::super::overworld_build::BuildFlags { shuffle_toad_houses: true, ..Default::default() });

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
        let catalog = super::super::node_catalog::NodeCatalog::build(&rom, false);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, super::super::overworld_pickup::PickupFlags { shuffle_spade_games: true, shuffle_toad_houses: true, ..Default::default() });

        for seed in 0u64..32 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let mut build = super::super::overworld_build::build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, super::super::overworld_build::BuildFlags { shuffle_toad_houses: true, ..Default::default() });
            super::super::troll_pipes::mark_troll_pipes(&mut build, &mut rng);

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
                        !(ce.world_idx == 7 && matches!(ce.entry_idx, 14..=16)),
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
        let catalog = super::super::node_catalog::NodeCatalog::build(&rom, false);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, super::super::overworld_pickup::PickupFlags { shuffle_spade_games: true, shuffle_toad_houses: true, ..Default::default() });

        let mut rom1 = rom.clone();
        let mut rom2 = rom.clone();

        for pass in 0..2 {
            let target = if pass == 0 { &mut rom1 } else { &mut rom2 };
            let mut rng = ChaCha8Rng::seed_from_u64(42);
            let build = super::super::overworld_build::build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, super::super::overworld_build::BuildFlags { shuffle_toad_houses: true, ..Default::default() });
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
        let catalog = super::super::node_catalog::NodeCatalog::build(&rom, false);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, super::super::overworld_pickup::PickupFlags { shuffle_spade_games: true, shuffle_toad_houses: true, ..Default::default() });
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let build = super::super::overworld_build::build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, super::super::overworld_build::BuildFlags { shuffle_toad_houses: true, ..Default::default() });

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
        let catalog = super::super::node_catalog::NodeCatalog::build(&rom, false);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, super::super::overworld_pickup::PickupFlags { shuffle_spade_games: true, shuffle_toad_houses: true, ..Default::default() });
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let build = super::super::overworld_build::build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, super::super::overworld_build::BuildFlags { shuffle_toad_houses: true, ..Default::default() });

        let mut test_rom = rom.clone();
        write_overworld(&mut test_rom, &build, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, WriteFlags::default());

        // Count total locked fortresses across all worlds.
        let total_locks: usize = build.worlds.iter().map(|b| b.locks.len()).sum();

        // Read FX world tables — count non-zero entries.
        let mut fx_count = 0;
        for wi in 0..8 {
            let fx_base = rom_data::FX_WORLD_TABLE + wi * 4;
            for i in 0..4 {
                let slot_idx = test_rom.read_byte(fx_base + i);
                if slot_idx != 0 || (i == 0 && !build.worlds[wi].locks.is_empty()) {
                    // Slot 0 is valid (could be index 0), so check lock count.
                    if i < build.worlds[wi].locks.len() {
                        fx_count += 1;
                    }
                }
            }
        }

        assert_eq!(
            fx_count, total_locks,
            "FX slot count {fx_count} != total locks {total_locks}",
        );
    }

    #[test]
    fn test_hammer_bro_redistribution_written() {
        let rom = match load_rom() {
            Some(r) => r,
            None => return,
        };
        for seed in 0..16u64 {
            let catalog = super::super::node_catalog::NodeCatalog::build(&rom, false);
            let pickup = super::super::overworld_pickup::pick_up(
                &rom,
                &catalog,
                super::super::overworld_pickup::PickupFlags {
                    shuffle_spade_games: true,
                    shuffle_toad_houses: true,
                    shuffle_hammer_bros: true,
                },
            );
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let data = OverworldData { pickup: &pickup, catalog: &catalog };
            let build = super::super::overworld_build::build(&rom, &data, &mut rng, super::super::overworld_build::BuildFlags { shuffle_toad_houses: true, shuffle_hammer_bros: true, ..Default::default() });

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
        let catalog = super::super::node_catalog::NodeCatalog::build(&rom, false);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, super::super::overworld_pickup::PickupFlags { shuffle_spade_games: true, shuffle_toad_houses: true, ..Default::default() });
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let build = super::super::overworld_build::build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, super::super::overworld_build::BuildFlags { shuffle_toad_houses: true, ..Default::default() });

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
        let catalog = super::super::node_catalog::NodeCatalog::build(&rom, false);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, super::super::overworld_pickup::PickupFlags { shuffle_spade_games: true, shuffle_toad_houses: true, ..Default::default() });

        for seed in [42u64, 123, 999, 7777, 31337] {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let build = super::super::overworld_build::build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, super::super::overworld_build::BuildFlags { shuffle_toad_houses: true, ..Default::default() });

            let mut test_rom = rom.clone();
            super::super::qol::fix_w3_drawbridges(&mut test_rom);
            super::super::qol::remove_rocks(&mut test_rom);
            super::super::qol::fix_big_q_block_rooms(&mut test_rom);
            write_overworld(&mut test_rom, &build, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, WriteFlags::default());

            let pipes_by_world = rom_data::read_pipe_pairs(&test_rom);

            for (wi, world) in WORLDS.iter().enumerate() {
                let grid = rom_data::read_tile_grid(&test_rom, wi);
                let pipe_pairs = pipes_by_world.get(&wi)
                    .cloned()
                    .unwrap_or_default();
                let walk = super::super::map_walker::walk_map(&grid, &pipe_pairs, None, wi);

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
        let catalog = super::super::node_catalog::NodeCatalog::build(&rom, false);
        let pickup = super::super::overworld_pickup::pick_up(&rom, &catalog, super::super::overworld_pickup::PickupFlags { shuffle_spade_games: true, shuffle_toad_houses: true, ..Default::default() });

        for seed in [42u64, 123, 999] {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let build = super::super::overworld_build::build(&rom, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, super::super::overworld_build::BuildFlags { shuffle_toad_houses: true, ..Default::default() });

            let mut out = rom.clone();

            // Apply QoL patches that the builder expects.
            super::super::qol::fix_w3_drawbridges(&mut out);
            super::super::qol::remove_rocks(&mut out);
            super::super::qol::fix_big_q_block_rooms(&mut out);

            write_overworld(&mut out, &build, &OverworldData { pickup: &pickup, catalog: &catalog }, &mut rng, WriteFlags::default());

            let filename = format!("writer_test_seed{seed}.nes");
            std::fs::write(&filename, &out.data).unwrap();
            eprintln!("Wrote {filename}");
        }
    }
