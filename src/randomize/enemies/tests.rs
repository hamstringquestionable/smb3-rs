    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    /// Options with all default enemy classes enabled (Shuffle mode).
    fn enemy_opts() -> Options {
        Options::default()
    }

    /// A blank 393,232-byte ROM image with a valid iNES header
    /// (16 PRG pages, 16 CHR pages, mapper 4).
    fn blank_rom_image() -> Vec<u8> {
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;
        data
    }

    /// A blank ROM with `seg` copied to ENEMY_DATA_START — the standard
    /// fixture for walker tests that need one synthetic enemy segment.
    fn rom_with_segment(seg: &[u8]) -> Rom {
        let mut data = blank_rom_image();
        data[ENEMY_DATA_START..ENEMY_DATA_START + seg.len()].copy_from_slice(seg);
        Rom::from_bytes_lax(&data, true).unwrap()
    }

    fn make_test_rom() -> Rom {
        // A realistic enemy data segment: FF terminator, then a segment with
        // page flag + entries + FF. Entries MUST be sorted by ascending X
        // (real SMB3 format requirement, enforced by segment_writer).
        rom_with_segment(&[
            0xFF, // leading terminator
            0x01, // page flag
            0x72, 0x0E, 0x19, // Goomba at (0x0E, 0x19)
            0x6C, 0x24, 0x16, // Green Troopa at (0x24, 0x16)
            0xA6, 0x40, 0x17, // Venus Fire Trap at (0x40, 0x17)
            0x41, 0xA8, 0x15, // End Level Card at (0xA8, 0x15) — must not change
            0xD3, 0xC0, 0x50, // Autoscroll at (0xC0, 0x50) — must not change
            0xFF, // terminator
        ])
    }

    #[test]
    fn test_enemies_stay_in_class() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize(&mut rom, &mut rng, &enemy_opts());

        // Read back the segment (skip FF + page flag = offset 2)
        let base = ENEMY_DATA_START + 2;
        let result = rom.read_range(base, 15);

        // Goomba should be replaced with a ground enemy
        assert!(
            GROUND_ENEMIES.contains(&result[0]),
            "Goomba replaced with non-ground: 0x{:02X}",
            result[0]
        );
        // X must be unchanged; Y may be decremented by 1 for tall enemies
        assert_eq!(result[1], 0x0E);
        let expected_y = if TALL_ENEMIES.contains(&result[0]) { 0x18 } else { 0x19 };
        assert_eq!(result[2], expected_y,
            "Goomba slot Y: got 0x{:02X}, expected 0x{:02X} (replacement 0x{:02X})",
            result[2], expected_y, result[0]);

        // Green Troopa should be replaced with a shell enemy
        assert!(
            SHELL_ENEMIES.contains(&result[3]),
            "Green Troopa replaced with non-shell enemy: 0x{:02X}",
            result[3]
        );
        assert_eq!(result[4], 0x24);
        let expected_y = if TALL_ENEMIES.contains(&result[3]) { 0x15 } else { 0x16 };
        assert_eq!(result[5], expected_y,
            "Troopa slot Y: got 0x{:02X}, expected 0x{:02X} (replacement 0x{:02X})",
            result[5], expected_y, result[3]);

        // Venus Fire Trap should be replaced with a piranha
        assert!(
            PIRANHAS.contains(&result[6]),
            "Venus replaced with non-piranha: 0x{:02X}",
            result[6]
        );

        // End Level Card must NOT be changed
        assert_eq!(result[9], 0x41, "End Level Card was modified!");
        assert_eq!(result[10], 0xA8);
        assert_eq!(result[11], 0x15);

        // Autoscroll must NOT be changed
        assert_eq!(result[12], 0xD3, "Autoscroll was modified!");
    }

    #[test]
    fn test_deterministic() {
        let mut rom1 = make_test_rom();
        let mut rom2 = make_test_rom();
        let mut rng1 = ChaCha8Rng::seed_from_u64(77);
        let mut rng2 = ChaCha8Rng::seed_from_u64(77);

        randomize(&mut rom1, &mut rng1, &enemy_opts());
        randomize(&mut rom2, &mut rng2, &enemy_opts());

        let len = ENEMY_DATA_END - ENEMY_DATA_START;
        assert_eq!(
            rom1.read_range(ENEMY_DATA_START, len),
            rom2.read_range(ENEMY_DATA_START, len),
        );
    }

    fn make_bigq_test_rom() -> Rom {
        let mut data = blank_rom_image();

        // Pre-fill the enemy data region with 0xFF so gaps between fixture
        // segments don't look like one giant collision-prone segment to
        // segment_writer's walker.
        data[ENEMY_DATA_START..ENEMY_DATA_END].fill(0xFF);

        // Segment with a regular Big ? Block (should be randomized)
        let seg1_start = ENEMY_DATA_START;
        let seg1 = &[
            0xFF,
            0x01, // page flag
            0x94, 0x18, 0x05, // BIGQBLOCK_3UP
            0x98, 0x16, 0x14, // BIGQBLOCK_TANOOKI
            0x41, 0xA8, 0x15, // ENDLEVELCARD (must not change)
            0xFF,
        ];
        data[seg1_start..seg1_start + seg1.len()].copy_from_slice(seg1);

        // Place the protected W7 Big Q Tanooki at its exact file offset
        // W7F1_TANOOKI_OFFSET = 0x0C9B7, which is the ID byte of the entry.
        // We need: [FF] [page] [0x98, x, y] [0x41, x, y] [FF]
        // So page byte at 0x0C9B6, entry at 0x0C9B7
        let w7f1_seg_start = W7F1_TANOOKI_OFFSET - 2; // FF + page byte before the entry
        data[w7f1_seg_start] = 0xFF;
        data[w7f1_seg_start + 1] = 0x01; // page flag
        data[W7F1_TANOOKI_OFFSET] = 0x98; // BIGQBLOCK_TANOOKI
        data[W7F1_TANOOKI_OFFSET + 1] = 0x0A;
        data[W7F1_TANOOKI_OFFSET + 2] = 0x13;
        data[W7F1_TANOOKI_OFFSET + 3] = 0x41; // ENDLEVELCARD
        data[W7F1_TANOOKI_OFFSET + 4] = 0x48;
        data[W7F1_TANOOKI_OFFSET + 5] = 0x15;
        data[W7F1_TANOOKI_OFFSET + 6] = 0xFF;

        Rom::from_bytes_lax(&data, true).unwrap()
    }

    #[test]
    fn test_big_q_blocks_randomized() {
        let mut rom = make_bigq_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize_big_q_blocks(&mut rom, &mut rng);

        // Regular Big ? Blocks should be randomized to some Big ? Block ID
        let base = ENEMY_DATA_START + 2; // skip FF + page
        let result = rom.read_range(base, 9);
        assert!(
            BIG_Q_BLOCKS.contains(&result[0]),
            "Big Q block not replaced with Big Q: 0x{:02X}",
            result[0]
        );
        assert!(
            BIG_Q_BLOCKS.contains(&result[3]),
            "Big Q block not replaced with Big Q: 0x{:02X}",
            result[3]
        );
        // End level card must not change
        assert_eq!(result[6], 0x41);
    }

    #[test]
    fn test_chr_compatibility_enforced() {
        // Place a Goomba ($4F/+5) and Dry Bones ($13/+5) in the same segment.
        // After randomization, both must use compatible CHR pages on slot +5.
        let seg = &[
            0xFF,
            0x01, // page flag
            0x72, 0x10, 0x19, // Goomba (slot +5, page $4F)
            0x3F, 0x20, 0x19, // Dry Bones (slot +5, page $13)
            0x29, 0x30, 0x19, // Spike (slot +4, page $0A)
            0x71, 0x40, 0x19, // Spiny (slot +4, page $0B)
            0xFF,
        ];
        let rom = rom_with_segment(seg);

        // Run many times to exercise different random paths
        for seed in 0..200u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng, &enemy_opts());

            let base = ENEMY_DATA_START + 2;
            let result = rom_copy.read_range(base, 12);
            let enemy1 = result[0]; // was Goomba
            let enemy2 = result[3]; // was Dry Bones
            let enemy3 = result[6]; // was Spike
            let enemy4 = result[9]; // was Spiny

            // Each must stay in its class
            assert!(GROUND_ENEMIES.contains(&enemy1), "seed {seed}: enemy1 0x{enemy1:02X}");
            assert!(GHOST_ENEMIES.contains(&enemy2), "seed {seed}: enemy2 0x{enemy2:02X}");
            assert!(GROUND_ENEMIES.contains(&enemy3), "seed {seed}: enemy3 0x{enemy3:02X}");
            assert!(GROUND_ENEMIES.contains(&enemy4), "seed {seed}: enemy4 0x{enemy4:02X}");

            // Check CHR compatibility: no two enemies in the same segment
            // should request different CHR pages for the same bank slot.
            let enemies = [enemy1, enemy2, enemy3, enemy4];
            let mut seen_slot4: Option<u8> = None;
            let mut seen_slot5: Option<u8> = None;
            for &e in &enemies {
                if let Some(sb) = sprite_bank(e) {
                    match sb.slot {
                        4 => {
                            if let Some(prev) = seen_slot4 {
                                assert_eq!(
                                    prev, sb.chr_page,
                                    "seed {seed}: slot +4 conflict: 0x{prev:02X} vs 0x{:02X} (enemy 0x{e:02X})",
                                    sb.chr_page
                                );
                            }
                            seen_slot4 = Some(sb.chr_page);
                        }
                        5 => {
                            if let Some(prev) = seen_slot5 {
                                assert_eq!(
                                    prev, sb.chr_page,
                                    "seed {seed}: slot +5 conflict: 0x{prev:02X} vs 0x{:02X} (enemy 0x{e:02X})",
                                    sb.chr_page
                                );
                            }
                            seen_slot5 = Some(sb.chr_page);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    #[test]
    fn test_chr_resets_across_segments() {
        // Two segments: first has a Spike ($0A/+4), second has a Spiny ($0B/+4).
        // They should be able to choose independently since they're in different segments.
        let seg = &[
            0xFF,
            0x01,             // page flag
            0x29, 0x10, 0x19, // Spike (slot +4, page $0A)
            0xFF,             // segment boundary
            0x01,             // page flag
            0x71, 0x20, 0x19, // Spiny (slot +4, page $0B)
            0xFF,
        ];
        let rom = rom_with_segment(seg);

        // Run many times — Spiny in second segment should freely choose
        // any ground enemy, not be constrained by first segment's Spike.
        let mut saw_slot4_0a_in_seg2 = false;
        let mut saw_slot4_0b_in_seg2 = false;
        for seed in 0..200u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng, &enemy_opts());

            // Second segment's enemy is at offset: FF(1) + page(1) + entry(3) + FF(1) + page(1) = 7
            let enemy2 = rom_copy.read_byte(ENEMY_DATA_START + 7);
            assert!(GROUND_ENEMIES.contains(&enemy2), "seed {seed}: 0x{enemy2:02X}");

            if let Some(sb) = sprite_bank(enemy2) {
                if sb.slot == 4 && sb.chr_page == 0x0A {
                    saw_slot4_0a_in_seg2 = true;
                }
                if sb.slot == 4 && sb.chr_page == 0x0B {
                    saw_slot4_0b_in_seg2 = true;
                }
            }
        }
        // Over 200 seeds, we should see both CHR page variants in segment 2
        assert!(
            saw_slot4_0a_in_seg2 && saw_slot4_0b_in_seg2,
            "Segment 2 should not be constrained by segment 1's CHR choice"
        );
    }

    #[test]
    fn test_7f1_tanooki_protected() {
        let mut rom = make_bigq_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(99);
        randomize_big_q_blocks(&mut rom, &mut rng);

        // The 7-F1 Tanooki must remain 0x98
        let protected = rom.read_byte(W7F1_TANOOKI_OFFSET);
        assert_eq!(
            protected, 0x98,
            "7-F1 Tanooki was changed to 0x{:02X}!",
            protected
        );
    }

    #[test]
    fn test_ghost_enemies_stay_in_class() {
        let seg = &[
            0xFF,
            0x01,
            0x2F, 0x10, 0x08, // Boo
            0x45, 0x20, 0x18, // Hot Foot
            0xFF,
        ];
        let rom = rom_with_segment(seg);

        for seed in 0..100u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng, &enemy_opts());

            let base = ENEMY_DATA_START + 2;
            let result = rom_copy.read_range(base, 6);
            assert!(GHOST_ENEMIES.contains(&result[0]), "seed {seed}: ghost1 0x{:02X}", result[0]);
            assert!(GHOST_ENEMIES.contains(&result[3]), "seed {seed}: ghost2 0x{:02X}", result[3]);
        }
    }

    #[test]
    fn test_big_enemies_in_regular_classes() {
        // Big enemies are merged into their regular-sized counterparts' classes:
        // BigGreenTroopa → SHELL_ENEMIES, BigGreenPiranha/BigRedPiranha → PIRANHAS
        let seg = &[
            0xFF,
            0x01,
            0x7A, 0x10, 0x10, // BigGreenTroopa
            0x7D, 0x20, 0x10, // BigGreenPiranha
            0x7F, 0x30, 0x10, // BigRedPiranha
            0xFF,
        ];
        let rom = rom_with_segment(seg);

        for seed in 0..100u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng, &enemy_opts());

            let base = ENEMY_DATA_START + 2;
            let result = rom_copy.read_range(base, 9);
            assert!(SHELL_ENEMIES.contains(&result[0]), "seed {seed}: big troopa 0x{:02X}", result[0]);
            assert!(PIRANHAS.contains(&result[3]), "seed {seed}: big piranha1 0x{:02X}", result[3]);
            assert!(PIRANHAS.contains(&result[6]), "seed {seed}: big piranha2 0x{:02X}", result[6]);
        }
    }

    /// Build the synthetic header + segment the giant-red test reuses, with two
    /// far-apart piranha slots (separate CHR groups): a regular green piranha
    /// (0xA0) and a giant red (0x7F).
    fn giant_red_test_rom() -> Rom {
        rom_with_segment(&[
            0xFF,
            0x01,
            0xA0, 0x10, 0x17, // GreenPiranha — regular slot, X=0x10
            0x7F, 0x60, 0x10, // BigRedPiranha — X=0x60 (separate CHR group)
            0xFF,
        ])
    }

    #[test]
    fn rocky_wrench_joins_piranhas_only_in_wild() {
        // Shuffle / Off: Rocky Wrench (0xAD) belongs to no class, so it's left
        // untouched. Wild: it joins the standard piranha pool both directions.
        let mut shuffle = ClassModes::from_options(&Options::default());
        shuffle.piranhas = EnemyMode::Shuffle;
        assert!(find_class_pool(ROCKY_WRENCH, &shuffle).is_none());

        shuffle.piranhas = EnemyMode::Off;
        assert!(find_class_pool(ROCKY_WRENCH, &shuffle).is_none());

        let mut wild = ClassModes::from_options(&Options::default());
        wild.piranhas = EnemyMode::Wild;
        // Rocky Wrench can become a standard piranha…
        assert_eq!(find_class_pool(ROCKY_WRENCH, &wild), Some(ClassPool::PiranhaStd));
        // …and a standard piranha can become Rocky Wrench.
        assert_eq!(find_class_pool(0xA0, &wild), Some(ClassPool::PiranhaStd));
        assert!(PIRANHAS_WILD.contains(&ROCKY_WRENCH));
        // Ceiling piranhas stay self-contained (no Rocky Wrench, no upward jet).
        assert_eq!(find_class_pool(0xA1, &wild), Some(ClassPool::PiranhaCeil));
        assert!(!PIRANHASC_WILD.contains(&ROCKY_WRENCH));
        assert!(!PIRANHASC_WILD.contains(&FIREJET_UP));
    }

    #[test]
    fn firejets_join_piranha_pools_only_in_wild() {
        // Shuffle / Off: the fire jets belong to no class → untouched.
        let mut shuffle = ClassModes::from_options(&Options::default());
        shuffle.piranhas = EnemyMode::Shuffle;
        assert!(find_class_pool(FIREJET_UP, &shuffle).is_none());
        assert!(find_class_pool(FIREJET_DOWN, &shuffle).is_none());
        shuffle.piranhas = EnemyMode::Off;
        assert!(find_class_pool(FIREJET_UP, &shuffle).is_none());
        assert!(find_class_pool(FIREJET_DOWN, &shuffle).is_none());

        let mut wild = ClassModes::from_options(&Options::default());
        wild.piranhas = EnemyMode::Wild;
        // Upward jet ↔ standard pool; downward jet ↔ ceiling pool.
        assert_eq!(find_class_pool(FIREJET_UP, &wild), Some(ClassPool::PiranhaStd));
        assert_eq!(find_class_pool(FIREJET_DOWN, &wild), Some(ClassPool::PiranhaCeil));
        // Standard piranha can become the upward jet; ceiling the downward jet.
        assert_eq!(find_class_pool(0xA0, &wild), Some(ClassPool::PiranhaStd));
        assert_eq!(find_class_pool(0xA1, &wild), Some(ClassPool::PiranhaCeil));
        assert!(PIRANHAS_WILD.contains(&FIREJET_UP));
        assert!(PIRANHASC_WILD.contains(&FIREJET_DOWN));
        // No crossover: up jet never in ceiling pool, down jet never in standard.
        assert!(!PIRANHAS_WILD.contains(&FIREJET_DOWN));
        assert!(!PIRANHASC_WILD.contains(&FIREJET_UP));
    }

    #[test]
    fn firejet_y_offsets_are_symmetric() {
        let rise = FIREJET_UP_Y_RISE;
        let drop = FIREJET_DOWN_Y_DROP;
        // helper: run swap_enemy on a single 3-byte entry, return new Y
        let swap = |old: u8, new: u8| {
            let mut d = [old, 0x20, 0x40];
            swap_enemy(&mut d, 0, new);
            assert_eq!(d[0], new);
            d[2]
        };
        // Forward: jet replacing a piranha/wrench rises (up) / drops (down).
        assert_eq!(swap(0xA0, FIREJET_UP), 0x40u8.wrapping_sub(rise));
        assert_eq!(swap(ROCKY_WRENCH, FIREJET_UP), 0x40u8.wrapping_sub(rise));
        assert_eq!(swap(0xA1, FIREJET_DOWN), 0x40u8.wrapping_add(drop));
        // Reverse: piranha/wrench replacing a jet gets the exact opposite shift.
        assert_eq!(swap(FIREJET_UP, 0xA0), 0x40u8.wrapping_add(rise));
        assert_eq!(swap(FIREJET_UP, ROCKY_WRENCH), 0x40u8.wrapping_add(rise));
        assert_eq!(swap(FIREJET_DOWN, 0xA1), 0x40u8.wrapping_sub(drop));
        // No shift: jet→same-jet, piranha↔piranha, piranha↔wrench.
        assert_eq!(swap(FIREJET_UP, FIREJET_UP), 0x40);
        assert_eq!(swap(0xA0, 0xA2), 0x40);
        assert_eq!(swap(0xA0, ROCKY_WRENCH), 0x40);
    }

    #[test]
    fn bucket_first_weights_categories_and_respects_chr() {
        use rand::SeedableRng;
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        let buckets: &[&[u8]] = &[PIRANHAS_NO_RED, BUCKET_UP_JET, BUCKET_WRENCH];

        // All slots free: each category should land ~1/3 (1000 of 3000).
        let (mut npir, mut njet, mut nwr) = (0, 0, 0);
        for _ in 0..3000 {
            match pick_bucket_first(buckets, ChrSlot::Free, ChrSlot::Free, &mut rng).unwrap() {
                FIREJET_UP => njet += 1,
                ROCKY_WRENCH => nwr += 1,
                _ => npir += 1,
            }
        }
        assert!((800..1200).contains(&njet), "jet category share off: {njet}/3000");
        assert!((800..1200).contains(&nwr), "wrench category share off: {nwr}/3000");
        assert!((800..1200).contains(&npir), "piranha category share off: {npir}/3000");

        // slot 5 committed to the small-piranha page (0x4F) makes the upward jet
        // (page 0x37, slot 5) incompatible, so its bucket is skipped entirely.
        for _ in 0..500 {
            let pick = pick_bucket_first(buckets, ChrSlot::Free, ChrSlot::Page(0x4F), &mut rng).unwrap();
            assert_ne!(pick, FIREJET_UP, "up-jet placed despite slot 5 = 0x4F");
        }
    }

    #[test]
    fn wild_pools_are_unions_of_their_parts() {
        // The hand-restated union constants must stay in sync with the lists
        // they combine — nothing else enforces this.
        let mut piranhas_wild: Vec<u8> = PIRANHAS.to_vec();
        piranhas_wild.extend([ROCKY_WRENCH, FIREJET_UP]);
        assert_eq!(PIRANHAS_WILD, piranhas_wild.as_slice());

        let mut piranhasc_wild: Vec<u8> = PIRANHASC.to_vec();
        piranhasc_wild.push(FIREJET_DOWN);
        assert_eq!(PIRANHASC_WILD, piranhasc_wild.as_slice());

        let mut all_cannons: Vec<u8> = Vec::new();
        all_cannons.extend_from_slice(CFIRE_LEFT);
        all_cannons.extend_from_slice(CFIRE_RIGHT);
        all_cannons.extend_from_slice(CFIRE_BILLS);
        assert_eq!(ALL_CANNONS, all_cannons.as_slice());
    }

    #[test]
    fn piranhas_excluded_from_global_wild_pool() {
        // With every class Wild, piranhas (and Rocky Wrench) must NOT appear in
        // the shared wild pool — they're self-contained, so no other class can
        // ever turn into a piranha.
        let mut opts = Options::default();
        for m in [
            &mut opts.ground, &mut opts.shell, &mut opts.flying, &mut opts.piranhas,
            &mut opts.ghosts, &mut opts.water, &mut opts.bros,
        ] {
            *m = EnemyMode::Wild;
        }
        let pool = wild_pool_for(&opts);
        for id in PIRANHAS.iter().chain(PIRANHASC).chain(std::iter::once(&ROCKY_WRENCH)) {
            assert!(!pool.contains(id), "piranha-kind 0x{id:02X} leaked into global wild pool");
        }
    }

    #[test]
    fn giant_red_never_replaces_non_giant_red() {
        // A regular piranha slot must never become 0x7F (giant red), in both
        // Shuffle and Wild. The 0x7F slot may stay 0x7F or change.
        for piranha_mode in [EnemyMode::Shuffle, EnemyMode::Wild] {
            let opts = Options { piranhas: piranha_mode, ..Default::default() };
            for seed in 0..300u64 {
                let mut rom = giant_red_test_rom();
                let mut rng = ChaCha8Rng::seed_from_u64(seed);
                randomize(&mut rom, &mut rng, &opts);
                let result = rom.read_range(ENEMY_DATA_START + 2, 6);
                assert_ne!(
                    result[0], GIANT_RED_PIRANHA,
                    "{piranha_mode:?} seed {seed}: regular piranha slot became giant red",
                );
            }
        }
    }

    #[test]
    fn test_two_pass_precommit() {
        // Regression test for the CHR ordering bug:
        // A swappable ground enemy (Spike, $0A/+4) appears BEFORE a Boo ($12/+4,
        // uniform ghost class — pre-committed in pass 1). Without two-pass, the
        // Spike could be swapped to something that commits a conflicting slot+4 page.
        let seg = &[
            0xFF,
            0x01,
            // Swappable ground enemy BEFORE uniform-class ghost
            0x29, 0x10, 0x19, // Spike ($0A/+4) — swappable, mixed-CHR class
            0x2F, 0x20, 0x08, // Boo ($12/+4) — swappable, uniform-CHR class (pre-committed)
            0xFF,
        ];
        let rom = rom_with_segment(seg);

        for seed in 0..500u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng, &enemy_opts());

            let base = ENEMY_DATA_START + 2;
            let result = rom_copy.read_range(base, 6);
            let enemy = result[0];
            let ghost = result[3];

            // Ghost must stay in ghost class
            assert!(GHOST_ENEMIES.contains(&ghost), "seed {seed}: ghost changed to 0x{ghost:02X}");

            // The swapped ground enemy must be CHR-compatible with Boo's $12/+4.
            assert!(GROUND_ENEMIES.contains(&enemy), "seed {seed}: enemy 0x{enemy:02X}");
            if let Some(sb) = sprite_bank(enemy) && sb.slot == 4 {
                assert_eq!(
                    sb.chr_page, 0x12,
                    "seed {seed}: enemy 0x{enemy:02X} has slot+4 page 0x{:02X}, \
                     conflicts with Boo's $12",
                    sb.chr_page
                );
            }
        }
    }

    #[test]
    fn test_uniform_class_precommit() {
        // Boo ($12/+4, uniform ghost class) + ground enemy in same segment.
        // The ground enemy must never commit a conflicting slot+4 page because
        // uniform classes are pre-committed in pass 1.
        let seg = &[
            0xFF,
            0x01,
            0x72, 0x10, 0x19, // Goomba ($4F/+5) — ground, mixed-CHR
            0x2F, 0x20, 0x08, // Boo ($12/+4) — ghost, uniform-CHR
            0xFF,
        ];
        let rom = rom_with_segment(seg);

        for seed in 0..500u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng, &enemy_opts());

            let base = ENEMY_DATA_START + 2;
            let result = rom_copy.read_range(base, 6);
            let ground = result[0];
            let ghost = result[3];

            assert!(GROUND_ENEMIES.contains(&ground), "seed {seed}: ground 0x{ground:02X}");
            assert!(GHOST_ENEMIES.contains(&ghost), "seed {seed}: ghost 0x{ghost:02X}");

            // No slot+4 conflict: ground enemy's slot+4 must match Boo's $12 or not use slot+4
            if let Some(sb) = sprite_bank(ground) && sb.slot == 4 {
                assert_eq!(
                    sb.chr_page, 0x12,
                    "seed {seed}: ground 0x{ground:02X} slot+4=0x{:02X} conflicts with Boo's $12",
                    sb.chr_page
                );
            }
        }
    }

    #[test]
    fn test_conflicted_slot_blocks_all() {
        // Two non-swappable objects with different +4 pages in the same segment.
        // Slot+4 becomes Conflicted, so any swappable enemy needing slot+4 gets
        // no compatible candidates and must keep its original ID.
        let seg = &[
            0xFF,
            0x01,
            0x51, 0x10, 0x08, // Rotodisc CW ($12/+4) — non-swappable
            0x4A, 0x20, 0x18, // Boom-Boom std ($13/+4) — non-swappable
            0x29, 0x30, 0x19, // Spike ($0A/+4) — swappable ground enemy
            0xFF,
        ];
        let rom = rom_with_segment(seg);

        for seed in 0..100u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng, &enemy_opts());

            let base = ENEMY_DATA_START + 2;
            let result = rom_copy.read_range(base, 9);

            // Non-swappable objects must not change
            assert_eq!(result[0], 0x51, "seed {seed}: rotodisc changed");
            assert_eq!(result[3], 0x4A, "seed {seed}: boom-boom changed");

            // Spike: slot+4 is conflicted ($12 vs $13), so only ground enemies
            // that don't use slot+4 (use slot+5 or NOCHANGE) can be chosen.
            // If all ground enemies need slot+4, Spike keeps original.
            let enemy = result[6];
            assert!(GROUND_ENEMIES.contains(&enemy), "seed {seed}: enemy 0x{enemy:02X}");
            if let Some(sb) = sprite_bank(enemy) {
                // Must NOT use slot+4 (it's conflicted)
                assert_ne!(sb.slot, 4,
                    "seed {seed}: enemy 0x{enemy:02X} uses conflicted slot+4 page 0x{:02X}",
                    sb.chr_page
                );
            }
        }
    }

    #[test]
    fn test_kuribo_shoe_in_ground_class() {
        assert!(GROUND_ENEMIES.contains(&0x2B), "Kuribo's Shoe Goomba missing from ground class");
        let modes = ClassModes::from_options(&enemy_opts());
        assert_eq!(find_class_pool(0x2B, &modes), Some(ClassPool::Class(GROUND_ENEMIES)));
    }

    #[test]
    fn test_chain_chomp_fire_chomp_in_ground() {
        assert!(GROUND_ENEMIES.contains(&0x4F), "Chain Chomp (freed) missing from ground class");
        assert!(!GROUND_ENEMIES.contains(&0x2C), "0x2C (cloud platform) must NOT be in ground class");
        assert!(GROUND_ENEMIES.contains(&0x58), "Fire Chomp missing from ground class");
        let modes = ClassModes::from_options(&enemy_opts());
        assert_eq!(find_class_pool(0x4F, &modes), Some(ClassPool::Class(GROUND_ENEMIES)));
        assert_eq!(find_class_pool(0x58, &modes), Some(ClassPool::Class(GROUND_ENEMIES)));
    }

    #[test]
    fn test_wild_pool_merges_classes() {
        // With ground=Wild, shell=Wild, others=Shuffle: ground↔shell swaps happen
        let flags = Options {
            ground: EnemyMode::Wild,
            shell: EnemyMode::Wild,
            flying: EnemyMode::Shuffle,
            ..Options::default()
        };
        let modes = ClassModes::from_options(&flags);
        let wild_pool = modes.build_wild_pool();
        // Ground and shell IDs should be in the wild pool
        assert!(wild_pool.contains(&0x72)); // Goomba
        assert!(wild_pool.contains(&0x6C)); // GreenTroopa
        // Flying should NOT be in wild pool (it's Shuffle, not Wild)
        assert!(!wild_pool.contains(&0x6E)); // Paratroopa
        // Ground enemy → resolves to the wild pool
        assert_eq!(find_class_pool(0x72, &modes), Some(ClassPool::Wild));
        // Flying → returns own class only
        assert_eq!(find_class_pool(0x6E, &modes), Some(ClassPool::Class(FLYING_ENEMIES)));

        // Run many seeds and confirm cross-class swaps happen
        let seg = &[
            0xFF, 0x01,
            0x72, 0x10, 0x19, // Goomba (ground)
            0xFF,
        ];
        let rom = rom_with_segment(seg);

        let mut saw_shell = false;
        for seed in 0..500u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng, &flags);
            let result_id = rom_copy.read_byte(ENEMY_DATA_START + 2);
            assert!(
                wild_pool.contains(&result_id),
                "seed {seed}: 0x{result_id:02X} not in wild pool"
            );
            if SHELL_ENEMIES.contains(&result_id) {
                saw_shell = true;
            }
        }
        assert!(saw_shell, "500 seeds and never saw a ground→shell swap");
    }

    #[test]
    fn test_off_mode_leaves_untouched() {
        // With ground=Off, ground enemies should stay vanilla
        let flags = Options {
            ground: EnemyMode::Off,
            shell: EnemyMode::Shuffle,
            ..Options::default()
        };
        let seg = &[
            0xFF, 0x01,
            0x72, 0x10, 0x19, // Goomba (ground - Off)
            0x6C, 0x20, 0x16, // GreenTroopa (shell - Shuffle)
            0xFF,
        ];
        let rom = rom_with_segment(seg);

        for seed in 0..100u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng, &flags);
            // Ground enemy stays vanilla (Off mode)
            assert_eq!(rom_copy.read_byte(ENEMY_DATA_START + 2), 0x72,
                "seed {seed}: ground enemy should stay vanilla in Off mode");
            // Shell enemy can change
            let shell = rom_copy.read_byte(ENEMY_DATA_START + 5);
            assert!(SHELL_ENEMIES.contains(&shell), "seed {seed}: shell 0x{shell:02X}");
        }
    }

    #[test]
    fn test_wild_fortress_tier_merges() {
        // With ghosts=Wild, thwomps=Wild, rotodiscs=Wild: they all share one pool
        let flags = Options {
            ghosts: EnemyMode::Wild,
            thwomps: EnemyMode::Wild,
            rotodiscs: EnemyMode::Wild,
            ..Options::default()
        };
        let modes = ClassModes::from_options(&flags);
        let wild_pool = modes.build_wild_pool();
        // Ghost, thwomp, rotodisc IDs should all be in the wild pool
        assert!(wild_pool.contains(&0x2F)); // Boo
        assert!(wild_pool.contains(&0x8A)); // Thwomp
        assert!(wild_pool.contains(&0x51)); // Rotodisc
        // All resolve to the shared wild pool
        assert_eq!(find_class_pool(0x2F, &modes), Some(ClassPool::Wild));
        assert_eq!(find_class_pool(0x8A, &modes), Some(ClassPool::Wild));
        assert_eq!(find_class_pool(0x51, &modes), Some(ClassPool::Wild));
    }

    /// A vanilla Lakitu (out-of-pool chaser) must pin its CHR page for the
    /// WHOLE level, not just its own proximity group — it follows the player
    /// across every group (see CHASER_IDS).
    #[test]
    fn test_chaser_pins_distant_groups() {
        let flags = Options {
            ground: EnemyMode::Wild,
            shell: EnemyMode::Wild,
            flying: EnemyMode::Wild,
            water: EnemyMode::Wild,
            bros: EnemyMode::Wild,
            ..Options::default()
        };
        let rom = rom_with_segment(&[
            0xFF, 0x01,
            0x83, 0x05, 0x10, // Lakitu ($0B/+4, chaser) — screen 0
            0x72, 0x80, 0x19, // Goomba — 7 screens away, own CHR group
            0x6C, 0x84, 0x19, // Green Troopa — same distant group
            0xFF,
        ]);
        for seed in 0..200u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng, &flags);
            assert_eq!(
                rom_copy.read_byte(ENEMY_DATA_START + 2), 0x83,
                "seed {seed}: Lakitu must not be swapped"
            );
            for off in [5usize, 8] {
                let id = rom_copy.read_byte(ENEMY_DATA_START + off);
                if let Some(bank) = sprite_bank(id) {
                    assert!(
                        bank.slot != 4 || bank.chr_page == 0x0B,
                        "seed {seed}: pick 0x{id:02X} (page ${:02X}/+4) in a distant \
                         group conflicts with the level-wide Lakitu ($0B/+4)",
                        bank.chr_page
                    );
                }
            }
        }
    }

    /// A chaser pick (Big Bertha) must be CHR-compatible with pages pinned
    /// anywhere in the segment, not just its own group — it will follow the
    /// player to them. Non-chaser picks in the distant group stay free.
    #[test]
    fn test_chaser_pick_respects_distant_pins() {
        let flags = Options {
            water: EnemyMode::Wild,
            ..Options::default()
        };
        let rom = rom_with_segment(&[
            0xFF, 0x01,
            0x8A, 0x05, 0x10, // Thwomp ($12/+4), class off by default → pinned
            0x62, 0x80, 0x19, // Blooper — 7 screens away, own CHR group
            0xFF,
        ]);
        for seed in 0..300u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng, &flags);
            assert_eq!(rom_copy.read_byte(ENEMY_DATA_START + 2), 0x8A);
            let id = rom_copy.read_byte(ENEMY_DATA_START + 5);
            assert!(
                !BERTHA_IDS.contains(&id),
                "seed {seed}: Bertha 0x{id:02X} ($1A/+4) picked despite the Thwomp \
                 ($12/+4) pinned elsewhere in the level"
            );
        }
    }

    /// Boom-Booms are deliberately NOT CHR-pinned (see should_precommit):
    /// shell enemies (koopas, $4F/+5) must stay pickable next to a Boom-Boom
    /// ($33/+5) because the shell-vs-boss interaction is wanted gameplay.
    #[test]
    fn test_boomboom_does_not_block_shell_picks() {
        let flags = Options {
            ground: EnemyMode::Wild, // goomba slot draws from the wild pool…
            shell: EnemyMode::Wild,  // …which includes the koopas
            ..Options::default()
        };
        let rom = rom_with_segment(&[
            0xFF, 0x01,
            0x72, 0x0E, 0x19, // Goomba — same screen as the Boom-Boom
            0x4B, 0x10, 0x10, // Boom-Boom (jump, $33/+5)
            0xFF,
        ]);
        let mut saw_koopa = false;
        for seed in 0..200u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng, &flags);
            let boom = rom_copy.read_byte(ENEMY_DATA_START + 5);
            assert!(
                BOOMBOOM_SWAP.contains(&boom),
                "seed {seed}: Boom-Boom became 0x{boom:02X}"
            );
            // Green/Red Troopa are $4F/+5 — CHR-conflicting with the
            // Boom-Boom, but allowed on purpose.
            if matches!(rom_copy.read_byte(ENEMY_DATA_START + 2), 0x6C | 0x6D) {
                saw_koopa = true;
                break;
            }
        }
        assert!(
            saw_koopa,
            "no koopa ever picked next to a Boom-Boom in 200 seeds — \
             Boom-Boom's CHR page is being pinned, but it must stay unpinned \
             so shells remain available in boss rooms"
        );
    }

    #[test]
    fn test_chr_groups_split_distant_enemies() {
        // Two enemies far apart (screen 0 vs screen 5) should get independent
        // CHR groups. A Boo ($12/+4) on screen 0 should NOT block a ground enemy
        // on screen 5 from picking a non-$12 slot+4 page.
        let seg = &[
            0xFF,
            0x01,
            0x2F, 0x04, 0x08, // Boo ($12/+4) at x=4 (screen 0)
            0x29, 0x50, 0x19, // Spike ($0A/+4) at x=80 (screen 5)
            0xFF,
        ];
        let rom = rom_with_segment(seg);

        // Under segment-wide tracking, Spike would be locked to $12/+4 enemies only.
        // Under distance-based grouping, Spike should freely pick any ground enemy.
        let mut saw_non_12_slot4 = false;
        for seed in 0..500u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng, &enemy_opts());

            let ghost = rom_copy.read_byte(ENEMY_DATA_START + 2);
            let ground = rom_copy.read_byte(ENEMY_DATA_START + 5);
            assert!(GHOST_ENEMIES.contains(&ghost), "seed {seed}: ghost 0x{ghost:02X}");
            assert!(GROUND_ENEMIES.contains(&ground), "seed {seed}: ground 0x{ground:02X}");

            if let Some(sb) = sprite_bank(ground) && sb.slot == 4 && sb.chr_page != 0x12 {
                saw_non_12_slot4 = true;
            }
        }
        assert!(saw_non_12_slot4,
            "500 seeds: distant ground enemy never picked a non-$12 slot+4 page — grouping not working");
    }

    #[test]
    fn test_chr_groups_keep_close_together() {
        // Two enemies close together (10 tiles apart) should still share
        // CHR constraints — same behavior as before grouping.
        // Goomba ($4F/+5) won't conflict with Boo ($12/+4) on slot+4,
        // so we can verify that any slot+4 ground enemy picked must be $12.
        let seg = &[
            0xFF,
            0x01,
            0x2F, 0x08, 0x08, // Boo ($12/+4) at x=8
            0x72, 0x12, 0x19, // Goomba ($4F/+5) at x=18 (10 tiles away, same group)
            0xFF,
        ];
        let rom = rom_with_segment(seg);

        // Boo pre-commits $12/+4 as uniform ghost class, so the ground enemy
        // must be compatible — any slot+4 pick must be $12 (or use slot+5 only).
        for seed in 0..500u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng, &enemy_opts());

            let ground = rom_copy.read_byte(ENEMY_DATA_START + 5);
            assert!(GROUND_ENEMIES.contains(&ground), "seed {seed}: ground 0x{ground:02X}");
            if let Some(sb) = sprite_bank(ground) && sb.slot == 4 {
                assert_eq!(sb.chr_page, 0x12,
                    "seed {seed}: close enemy 0x{ground:02X} has slot+4 page 0x{:02X}, \
                     conflicts with Boo's $12", sb.chr_page);
            }
        }
    }

    #[test]
    fn test_chr_groups_basic() {
        // Verify the grouping function itself
        let entries = vec![
            SegmentEntry { data_index: 0, obj_id: 0x72, x_pos: 5 },
            SegmentEntry { data_index: 3, obj_id: 0x72, x_pos: 10 },
            SegmentEntry { data_index: 6, obj_id: 0x72, x_pos: 80 },
            SegmentEntry { data_index: 9, obj_id: 0x72, x_pos: 85 },
        ];
        let groups = chr_groups(&entries);
        assert_eq!(groups.len(), 2, "should split into 2 groups");
        assert_eq!(groups[0].len(), 2, "first group: x=5, x=10");
        assert_eq!(groups[1].len(), 2, "second group: x=80, x=85");
    }

    #[test]
    fn test_chr_groups_single() {
        // All entries close together — one group
        let entries = vec![
            SegmentEntry { data_index: 0, obj_id: 0x72, x_pos: 5 },
            SegmentEntry { data_index: 3, obj_id: 0x72, x_pos: 10 },
            SegmentEntry { data_index: 6, obj_id: 0x72, x_pos: 20 },
        ];
        let groups = chr_groups(&entries);
        assert_eq!(groups.len(), 1, "all within gap — one group");
        assert_eq!(groups[0].len(), 3);
    }

    /// Regression: at the exact ROM offsets where `disable_autoscroll`
    /// inserts a mid-segment $FF (here `$0CFE3`), a block-wide walker
    /// would treat the clobbered bytes as a "ghost" segment that
    /// swallows the page byte + first entry of the next real segment.
    /// The writeback's stable sort would then scramble those bytes —
    /// in the wild, that corrupted the W5 spiral castle's
    /// PIPEWAYCONTROLLER and broke its exit teleport on seed
    /// 1642218906354586 (beta.3).
    ///
    /// With `autoscroll::SPOILED_SEGMENT_RANGES` honored by the
    /// walker, the clobbered region is skipped and the trailing real
    /// segment survives byte-for-byte.
    #[test]
    fn ghost_segment_does_not_corrupt_trailing_segment() {
        let mut data = blank_rom_image();
        data[ENEMY_DATA_START..ENEMY_DATA_END].fill(0xFF);

        // Place the bug pattern at the real $0CFE2 (covered by a
        // SPOILED_SEGMENT_RANGES entry). The "autoscroll" segment is
        // clobbered exactly as disable_autoscroll leaves it; the
        // trailing real segment carries a PIPEWAYCONTROLLER.
        const PWC_SEG_START: usize = 0x0CFE7;
        data[0x0CFE2..0x0CFE7].copy_from_slice(&[0x01, 0xFF, 0x00, 0x10, 0xFF]);
        data[PWC_SEG_START..PWC_SEG_START + 5]
            .copy_from_slice(&[0x01, 0x25, 0x00, 0x80, 0xFF]);
        let pwc_before = data[PWC_SEG_START..PWC_SEG_START + 5].to_vec();

        let mut rom = Rom::from_bytes_lax(&data, true).unwrap();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        randomize(&mut rom, &mut rng, &enemy_opts());

        let pwc_after = rom.read_range(PWC_SEG_START, 5).to_vec();
        assert_eq!(
            pwc_after, pwc_before,
            "PIPEWAYCONTROLLER segment was corrupted by ghost-segment sort.\n  before: {:02X?}\n  after:  {:02X?}",
            pwc_before, pwc_after,
        );
    }

    // ===================================================================
    // Enemy-placement invariant harness
    //
    // Runs the real ROM through the enemy pass over many seeds on the
    // "Recommended" and "Max Chaos" presets and asserts every swap obeys
    // the protection/validity rules. Doubles as a regression oracle for
    // the predicate-pipeline refactor: the new pipeline must keep this
    // green (output diverges per-seed because RNG draw counts change, but
    // every swap must stay valid). Also prints a per-id placement
    // histogram so old vs new behavioral *shape* can be compared.
    //
    // Skips silently if the reference ROM isn't present (e.g. in CI).
    // Run with: cargo test enemy_invariant_baseline -- --nocapture
    // ===================================================================

    const REFERENCE_ROM_PATH: &str = "roms/Super Mario Bros. 3 (USA) (Rev 1).nes";

    fn load_reference_rom() -> Option<Rom> {
        let data = std::fs::read(REFERENCE_ROM_PATH).ok()?;
        Rom::from_bytes(&data).ok()
    }

    /// "Recommended" preset — only the fields the enemy pass actually reads.
    /// (thwomps -> Off, bros -> Shuffle come from Options::default(), matching
    /// the web preset which leaves them unset.)
    fn preset_recommended() -> Options {
        Options {
            ground: EnemyMode::Wild, shell: EnemyMode::Wild, flying: EnemyMode::Wild,
            piranhas: EnemyMode::Wild, ghosts: EnemyMode::Wild, water: EnemyMode::Wild,
            cannons: EnemyMode::Wild, hb_encounters: EnemyMode::Wild,
            rotodiscs: EnemyMode::Shuffle,
            wild_injections: true, early_sun: true,
            ..Options::default()
        }
    }

    /// "Max Chaos" preset — every enemy class wild.
    fn preset_max_chaos() -> Options {
        Options {
            ground: EnemyMode::Wild, shell: EnemyMode::Wild, flying: EnemyMode::Wild,
            piranhas: EnemyMode::Wild, ghosts: EnemyMode::Wild, thwomps: EnemyMode::Wild,
            rotodiscs: EnemyMode::Wild, cannons: EnemyMode::Wild, water: EnemyMode::Wild,
            bros: EnemyMode::Wild, hb_encounters: EnemyMode::Wild,
            wild_injections: true, early_sun: true,
            ..Options::default()
        }
    }

    #[derive(Default)]
    struct PlacementStats {
        seeds: u64,
        entries: u64,
        swapped: u64,
        injected: u64,
        berthas_placed: u64,
        giant_reds_placed: u64,
        /// Segment-instances (seed × segment) where the bertha cap was exceeded.
        /// Hard invariant: the predicate pipeline applies MAX_BERTHA_PER_SEGMENT
        /// to every pick (including the Force*/ExcludeHazards paths that used to
        /// bypass it), so this must stay 0 — any nonzero count is a violation.
        bertha_cap_exceeded: u64,
        max_berthas_in_seg: u8,
        /// new_id -> times placed (only counts actual swaps)
        histogram: std::collections::BTreeMap<u8, u64>,
    }

    /// The deterministic set of enemy-data offsets (relative to ENEMY_DATA_START)
    /// that the wild-injection pass may overwrite: the first real entry of each
    /// `NodeKind::Level` whose first enemy is swappable and unprotected. Mirrors
    /// `inject_wild_chasers`' candidate selection. RNG decides *whether* a slot
    /// is injected, but the candidate set is fixed.
    fn injectable_offsets(
        base: &Rom,
        vanilla: &[u8],
        modes: &ClassModes,
    ) -> std::collections::HashSet<usize> {
        use crate::randomize::node_catalog::{NodeCatalog, NodeKind};
        use crate::randomize::rom_data::enemy_ptr_to_file_offset;
        let mut set = std::collections::HashSet::new();
        let catalog = NodeCatalog::build(base, false);
        for e in &catalog.entries {
            if !matches!(e.kind, NodeKind::Level) {
                continue;
            }
            let Some(le) = &e.level_entry else { continue };
            let obj_ptr = ((le.obj_hi as u16) << 8) | le.obj_lo as u16;
            if obj_ptr < 0xC000 {
                continue;
            }
            let file_off = enemy_ptr_to_file_offset(obj_ptr);
            if !(ENEMY_DATA_START..ENEMY_DATA_END).contains(&file_off) {
                continue;
            }
            let page_idx = file_off - ENEMY_DATA_START;
            if page_idx >= vanilla.len() {
                continue;
            }
            let first = if matches!(vanilla[page_idx], 0x00 | 0x01) {
                page_idx + 1
            } else {
                page_idx
            };
            if first >= vanilla.len() || vanilla[first] == 0xFF {
                continue;
            }
            if entry_protection_at(ENEMY_DATA_START + first).is_some() {
                continue;
            }
            if find_class_pool(vanilla[first], modes).is_none() {
                continue;
            }
            set.insert(first);
        }
        set
    }

    /// Check one randomized ROM against vanilla; push any violations into `out`
    /// and fold counts into `stats`. Mirrors the randomizer's per-segment
    /// dispatch: protected segments are skipped, HammerBro segments draw from
    /// the HB wild pool (batch path), everything else uses the normal class
    /// pools — plus the Boom-Boom self-swap and wild injection side channels.
    fn check_invariants(
        vanilla: &[u8],
        randomized: &[u8],
        opts: &Options,
        injectable: &std::collections::HashSet<usize>,
        seed: u64,
        stats: &mut PlacementStats,
        out: &mut Vec<String>,
    ) {
        let normal_modes = ClassModes::from_options(opts);
        let normal_wild = normal_modes.build_wild_pool();
        let hb_modes = hb_class_modes(opts.hb_encounters);
        let hb_wild = hb_modes.build_wild_pool();
        // Same spoiled-range skips the randomizer uses, so segment boundaries
        // (and per-segment bertha counts) line up exactly.
        let skip_ranges: Vec<core::ops::Range<usize>> =
            crate::randomize::autoscroll::SPOILED_SEGMENT_RANGES
                .iter()
                .map(|r| (r.start - ENEMY_DATA_START)..(r.end - ENEMY_DATA_START))
                .collect();
        let bounds = segment_writer::walk_segments(randomized, 0, randomized.len(), &skip_ranges);

        for b in bounds {
            let seg_rule = walker_segment_rule_at(ENEMY_DATA_START + b.file_offset);
            let is_hb = seg_rule == WalkerSegmentRule::HammerBro;
            // HB segments with hb==Wild are batch-assigned from the HB wild pool
            // (randomize_hb_wild_segment), bypassing per-entry class pools and
            // the Force*/giant-red/piranha rules.
            let hb_batch = is_hb && opts.hb_encounters == EnemyMode::Wild;
            let (modes, wild_pool) = if is_hb {
                (&hb_modes, hb_wild.as_slice())
            } else {
                (&normal_modes, normal_wild.as_slice())
            };

            let mut bertha_in_seg = 0u8;
            for i in 0..b.entry_count {
                let off = b.file_offset + 1 + i * 3; // +1 skips the page flag
                let orig = vanilla[off];
                let new = randomized[off];
                let fo = ENEMY_DATA_START + off;

                stats.entries += 1;
                if BERTHA_IDS.contains(&new) {
                    bertha_in_seg += 1;
                }

                if new == orig {
                    continue; // no-op is always valid
                }

                stats.swapped += 1;
                *stats.histogram.entry(new).or_insert(0) += 1;
                if WILD_INJECTION_IDS.contains(&new) {
                    stats.injected += 1;
                }
                if new == GIANT_RED_PIRANHA {
                    stats.giant_reds_placed += 1;
                }
                if BERTHA_IDS.contains(&new) {
                    stats.berthas_placed += 1;
                }

                let mut bad = |msg: String| {
                    out.push(format!(
                        "seed {seed} @ {fo:#07X}: 0x{orig:02X}->0x{new:02X}: {msg}"
                    ));
                };

                // Protected segments must never change.
                if seg_rule == WalkerSegmentRule::Skip {
                    bad("entry in a Skip segment was changed".into());
                    continue;
                }

                // Injectable slots: the wild-injection pass may overwrite this
                // entry (0x83/0xAF/0x2D) before the walker re-swaps it, so the
                // final value can be any wild-pool member or surviving injection
                // id. The class/giant-red/piranha guards key off the *injected*
                // id, not vanilla, so accept a broad set here.
                if injectable.contains(&off) {
                    let ok = WILD_INJECTION_IDS.contains(&new)
                        || normal_wild.contains(&new)
                        || find_class_pool(orig, &normal_modes)
                            .is_some_and(|p| p.slice(&normal_wild).contains(&new));
                    if !ok {
                        bad("injectable slot: not a wild-pool member or injection id".into());
                    }
                    continue;
                }

                // HB wild batch: only constraint is membership in the HB pool.
                if hb_batch {
                    if !hb_wild.contains(&new) {
                        bad("HB-wild swap not in the HB wild pool".into());
                    }
                    continue;
                }

                let prot = entry_protection_at(fo);

                // --- Protection rules ---
                match prot {
                    Some(EntryProtection::SkipSwap) => {
                        bad("SkipSwap offset was changed".into());
                    }
                    Some(EntryProtection::ForceShell)
                        if opts.shell != EnemyMode::Off && !SHELL_ENEMIES.contains(&new) =>
                    {
                        bad("ForceShell but result not a shell enemy".into());
                    }
                    Some(EntryProtection::ForceStompable)
                        if !STOMPABLE_ENEMIES.contains(&new) =>
                    {
                        bad("ForceStompable but result not stompable".into());
                    }
                    Some(EntryProtection::ForceTankBro)
                        if opts.bros != EnemyMode::Off && !TANK_BRO_POOL.contains(&new) =>
                    {
                        bad("ForceTankBro but result not a tank bro".into());
                    }
                    Some(EntryProtection::ExcludeHazards)
                        if hazard_excluded(new, orig) =>
                    {
                        bad("ExcludeHazards but introduced a new hazard category".into());
                    }
                    _ => {}
                }

                // --- Class validity: a swap must land in the original's class
                // pool, be a wild injection, be a Boom-Boom self-swap, or be
                // covered by a pool-replacing Force* protection above. ---
                let forced_pool = matches!(
                    prot,
                    Some(EntryProtection::ForceShell | EntryProtection::ForceTankBro)
                );
                if !forced_pool {
                    let in_class = find_class_pool(orig, modes)
                        .is_some_and(|p| p.slice(wild_pool).contains(&new));
                    let boomboom = BOOMBOOM_SWAP.contains(&orig) && BOOMBOOM_SWAP.contains(&new);
                    if !in_class && !boomboom {
                        bad("swap not in original's class pool (and not boom-boom)".into());
                    }
                }

                // --- Giant red piranha may only land where one already was ---
                if new == GIANT_RED_PIRANHA && orig != GIANT_RED_PIRANHA {
                    bad("giant-red piranha placed where original wasn't giant-red".into());
                }

                // --- A piranha slot must never become a hazard (the runtime
                // guard was removed; this verifies the piranha pools' self-
                // containment achieves it). ---
                if (PIRANHAS.contains(&orig) || PIRANHASC.contains(&orig))
                    && hazard_category(new).is_some()
                {
                    bad("piranha slot replaced by a hazard".into());
                }
            }

            // Bertha cap: hard invariant now that the predicate pipeline applies
            // it to every pick (it used to be bypassed by the Force*/ExcludeHazards
            // branches — see PlacementStats::bertha_cap_exceeded).
            stats.max_berthas_in_seg = stats.max_berthas_in_seg.max(bertha_in_seg);
            if bertha_in_seg > MAX_BERTHA_PER_SEGMENT {
                stats.bertha_cap_exceeded += 1;
                out.push(format!(
                    "seed {seed} @ seg {:#07X}: {bertha_in_seg} berthas, cap is {} (rule={seg_rule:?}, hb_batch={hb_batch})",
                    ENEMY_DATA_START + b.file_offset,
                    MAX_BERTHA_PER_SEGMENT,
                ));
            }
        }
        stats.seeds += 1;
    }

    fn run_preset(label: &str, opts: &Options, seeds: u64, base: &Rom, vanilla: &[u8]) -> Vec<String> {
        let mut stats = PlacementStats::default();
        let mut violations = Vec::new();
        let modes = ClassModes::from_options(opts);
        let injectable = if opts.wild_injections {
            injectable_offsets(base, vanilla, &modes)
        } else {
            std::collections::HashSet::new()
        };
        for seed in 0..seeds {
            let mut rom = base.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom, &mut rng, opts);
            let randomized = rom.read_range(ENEMY_DATA_START, vanilla.len()).to_vec();
            check_invariants(vanilla, &randomized, opts, &injectable, seed, &mut stats, &mut violations);
        }

        eprintln!("\n=== {label}: {} seeds ===", stats.seeds);
        eprintln!(
            "entries/seed={} swapped/seed={} injected/seed={} berthas/seed={} giant-red/seed={}",
            stats.entries / stats.seeds.max(1),
            stats.swapped / stats.seeds.max(1),
            stats.injected / stats.seeds.max(1),
            stats.berthas_placed / stats.seeds.max(1),
            stats.giant_reds_placed / stats.seeds.max(1),
        );
        eprintln!(
            "bertha-cap exceeded in {} segment-instances (max {} berthas/seg)",
            stats.bertha_cap_exceeded, stats.max_berthas_in_seg,
        );
        eprintln!("placement histogram (id: total placements over all seeds):");
        for (id, n) in &stats.histogram {
            eprintln!("  0x{id:02X}: {n}");
        }
        violations
    }

    #[test]
    fn enemy_invariant_baseline() {
        let Some(base) = load_reference_rom() else {
            eprintln!("reference ROM not present — skipping enemy_invariant_baseline");
            return;
        };
        let vanilla = base.read_range(ENEMY_DATA_START, ENEMY_DATA_END - ENEMY_DATA_START).to_vec();

        let mut violations = Vec::new();
        violations.extend(run_preset("Recommended", &preset_recommended(), 250, &base, &vanilla));
        violations.extend(run_preset("Max Chaos", &preset_max_chaos(), 250, &base, &vanilla));

        assert!(
            violations.is_empty(),
            "{} invariant violation(s):\n{}",
            violations.len(),
            violations.iter().take(40).cloned().collect::<Vec<_>>().join("\n"),
        );
    }

    /// Every cannon-fire family member carries the CHR bank its engine
    /// behavior demands (see the cfire arm in `sprite_bank`): bills and
    /// goomba pipes spawn $4F/+5 children; the cannonball family (plus the
    /// 4-way and Rocky Wrench cfires) needs $36/+4. The laser needs nothing.
    /// The cannonball family is chaser-class (its handler rewrites slot 4
    /// every frame it stays loaded); bills/pipes are proximity-scoped.
    #[test]
    fn test_cfire_sprite_banks() {
        for &id in &[0xBCu8, 0xBD, 0xC0, 0xC1] {
            let b = sprite_bank(id).expect("bill/pipe cfire must have a bank");
            assert_eq!((b.chr_page, b.slot), (0x4F, 5), "id 0x{id:02X}");
            assert!(!CHASER_IDS.contains(&id), "0x{id:02X} is proximity-scoped");
        }
        for id in [0xBEu8, 0xBF].into_iter().chain(0xC2..=0xCF) {
            let b = sprite_bank(id).expect("cannonball-family cfire must have a bank");
            assert_eq!((b.chr_page, b.slot), (0x36, 4), "id 0x{id:02X}");
        }
        for id in 0xC2u8..=0xCF {
            assert!(CHASER_IDS.contains(&id), "0x{id:02X} must be chaser-class");
        }
        assert!(CHASER_IDS.contains(&0xBF), "4-way pins slot 4 every frame");
        assert!(sprite_bank(0xD0).is_none(), "laser has no CHR need");
    }

    /// A cannonball cfire pins slot 4 = $36 for the whole level — its
    /// handler re-writes the bank every frame it stays loaded, and a spawned
    /// cfire slot survives until pushed out of the 8-slot FIFO. Picks in
    /// distant proximity groups must therefore stay $36-compatible on
    /// slot 4, exactly like the true chasers.
    #[test]
    fn test_cannon_pins_distant_groups() {
        let flags = Options {
            ground: EnemyMode::Wild,
            shell: EnemyMode::Wild,
            flying: EnemyMode::Wild,
            water: EnemyMode::Wild,
            bros: EnemyMode::Wild,
            ..Options::default() // cannons Off → the cfire itself is pinned
        };
        let rom = rom_with_segment(&[
            0xFF, 0x01,
            0xC8, 0x05, 0x10, // HLCANNON2 ($36/+4 frame-pin) — screen 0
            0x72, 0x80, 0x19, // Goomba — 7 screens away, own CHR group
            0x6C, 0x84, 0x19, // Green Troopa — same distant group
            0xFF,
        ]);
        for seed in 0..200u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng, &flags);
            assert_eq!(
                rom_copy.read_byte(ENEMY_DATA_START + 2), 0xC8,
                "seed {seed}: pinned cannon must not be swapped"
            );
            for off in [5usize, 8] {
                let id = rom_copy.read_byte(ENEMY_DATA_START + off);
                if let Some(bank) = sprite_bank(id) {
                    assert!(
                        bank.slot != 4 || bank.chr_page == 0x36,
                        "seed {seed}: pick 0x{id:02X} (page ${:02X}/+4) in a distant \
                         group conflicts with the level-wide cannon pin ($36/+4)",
                        bank.chr_page
                    );
                }
            }
        }
    }

    /// Cannons-wild picks are CHR-gated like any other class: next to a
    /// pinned fire jet ($37/+5), a Bill cannon can only become a
    /// cannonball-family member — bills and goomba pipes are filtered out
    /// because their spawned children need $4F on the jet's slot.
    #[test]
    fn test_cannons_wild_respects_slot5_pin() {
        let flags = Options {
            cannons: EnemyMode::Wild,
            ..Options::default()
        };
        let rom = rom_with_segment(&[
            0xFF, 0x01,
            0xBC, 0x05, 0x0C, // Bullet Bill cannon ($4F/+5 via spawned bills)
            0xAC, 0x07, 0x11, // upward fire jet ($37/+5), no class → pinned
            0xFF,
        ]);
        for seed in 0..300u64 {
            let mut rom_copy = rom.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom_copy, &mut rng, &flags);
            let id = rom_copy.read_byte(ENEMY_DATA_START + 2);
            assert!(
                (0xC2..=0xCF).contains(&id),
                "seed {seed}: bill slot became 0x{id:02X}; only the $36/+4 \
                 cannonball family fits beside a $37/+5 fire jet"
            );
        }
    }

    /// End-to-end guarantees of the level-centric wild-injection rework against
    /// the real ROM: injections happen; every injected sun spawns on screen 0;
    /// no boss level (fortress / airship / Bowser) receives a chaser; and a
    /// level is never given a chaser it already has (the 2-Quicksand double).
    #[test]
    fn wild_injection_rework_guarantees() {
        use crate::randomize::node_catalog::{NodeCatalog, NodeKind};
        use crate::randomize::rom_data::enemy_ptr_to_file_offset;
        const INJ: [u8; 2] = [0x83, 0xAF]; // Lakitu + Angry Sun (Boss Bass dropped)

        let Some(base) = load_reference_rom() else {
            eprintln!("reference ROM not present — skipping wild_injection_rework_guarantees");
            return;
        };
        let len = ENEMY_DATA_END - ENEMY_DATA_START;
        let vanilla = base.read_range(ENEMY_DATA_START, len).to_vec();
        let catalog = NodeCatalog::build(&base, false);

        // First-enemy data index for a level's obj_ptr (after any page byte).
        let first_idx = |obj_ptr: u16, data: &[u8]| -> Option<usize> {
            if obj_ptr < 0xC000 {
                return None;
            }
            let fo = enemy_ptr_to_file_offset(obj_ptr);
            if !(ENEMY_DATA_START..ENEMY_DATA_END).contains(&fo) {
                return None;
            }
            let p = fo - ENEMY_DATA_START;
            if p >= data.len() {
                return None;
            }
            let first = if matches!(data[p], 0x00 | 0x01) { p + 1 } else { p };
            if first >= data.len() || data[first] == 0xFF {
                return None;
            }
            Some(first)
        };
        // Count occurrences of `id` across a level's first $FF run.
        let run_count = |obj_ptr: u16, data: &[u8], id: u8| -> usize {
            let Some(mut i) = first_idx(obj_ptr, data) else {
                return 0;
            };
            let mut n = 0;
            while i + 2 < data.len() && data[i] != 0xFF {
                if data[i] == id {
                    n += 1;
                }
                i += 3;
            }
            n
        };

        let opts = Options { wild_injections: true, ..preset_recommended() };
        let mut saw_injection = false;
        for seed in 0..30u64 {
            let mut rom = base.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom, &mut rng, &opts);
            let patched = rom.read_range(ENEMY_DATA_START, len).to_vec();

            for e in &catalog.entries {
                let Some(le) = &e.level_entry else { continue };
                let obj_ptr = ((le.obj_hi as u16) << 8) | le.obj_lo as u16;
                let Some(fi) = first_idx(obj_ptr, &patched) else { continue };
                let pid = patched[fi];
                let van_first = first_idx(obj_ptr, &vanilla).map(|v| vanilla[v]);

                // Boss levels are excluded by type — a chaser at their first
                // enemy can only be a vanilla-native one, never injected.
                if matches!(
                    e.kind,
                    NodeKind::Fortress { .. } | NodeKind::Airship | NodeKind::Bowser
                ) {
                    if INJ.contains(&pid) {
                        assert_eq!(
                            Some(pid), van_first,
                            "seed {seed}: boss level {} got chaser 0x{pid:02X}", e.name
                        );
                    }
                    continue;
                }
                if !matches!(e.kind, NodeKind::Level) {
                    continue;
                }

                // An injected chaser (first enemy changed to a chaser).
                let injected_here = INJ.contains(&pid) && van_first != Some(pid);
                if injected_here {
                    saw_injection = true;
                    // Suns must spawn on screen 0.
                    if pid == 0xAF {
                        assert_eq!(
                            patched[fi + 1], SUN_SPAWN_X,
                            "seed {seed}: injected sun in {} not at screen 0", e.name
                        );
                        assert_eq!(
                            patched[fi + 2], SUN_SPAWN_Y,
                            "seed {seed}: injected sun in {} wrong Y", e.name
                        );
                    }
                }

                // No-double: a chaser the level already had is never added again
                // (walker never creates chasers, so any increase is injection).
                for &c in &INJ {
                    let van = run_count(obj_ptr, &vanilla, c);
                    if van > 0 {
                        assert!(
                            run_count(obj_ptr, &patched, c) <= van,
                            "seed {seed}: {} doubled chaser 0x{c:02X}", e.name
                        );
                    }
                }
            }
        }
        assert!(saw_injection, "30 seeds and never saw a chaser injected into a level");
    }

    /// An injected Lakitu's height coin-flips between the replaced enemy's Y and
    /// LAKITU_ALT_Y (0x12): across many seeds we must see both outcomes, so it's
    /// not always stuck at the (harder) low inherited height.
    #[test]
    fn wild_injected_lakitu_height_varies() {
        use crate::randomize::node_catalog::NodeCatalog;
        use crate::randomize::rom_data::enemy_ptr_to_file_offset;
        const LAKITU: u8 = 0x83;

        let Some(base) = load_reference_rom() else {
            eprintln!("reference ROM not present — skipping wild_injected_lakitu_height_varies");
            return;
        };
        let len = ENEMY_DATA_END - ENEMY_DATA_START;
        let vanilla = base.read_range(ENEMY_DATA_START, len).to_vec();
        let catalog = NodeCatalog::build(&base, false);

        let first_idx = |obj_ptr: u16, data: &[u8]| -> Option<usize> {
            if obj_ptr < 0xC000 {
                return None;
            }
            let fo = enemy_ptr_to_file_offset(obj_ptr);
            if !(ENEMY_DATA_START..ENEMY_DATA_END).contains(&fo) {
                return None;
            }
            let p = fo - ENEMY_DATA_START;
            if p >= data.len() {
                return None;
            }
            let first = if matches!(data[p], 0x00 | 0x01) { p + 1 } else { p };
            if first >= data.len() || data[first] == 0xFF {
                return None;
            }
            Some(first)
        };

        let opts = Options { wild_injections: true, ..preset_recommended() };
        let mut saw_alt = false; // lifted to LAKITU_ALT_Y (0x12)
        let mut saw_kept = false; // kept a non-0x12 inherited height
        'seeds: for seed in 0..60u64 {
            let mut rom = base.clone();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            randomize(&mut rom, &mut rng, &opts);
            let patched = rom.read_range(ENEMY_DATA_START, len).to_vec();
            for e in &catalog.entries {
                let Some(le) = &e.level_entry else { continue };
                let obj_ptr = ((le.obj_hi as u16) << 8) | le.obj_lo as u16;
                let Some(fi) = first_idx(obj_ptr, &patched) else { continue };
                if patched[fi] != LAKITU {
                    continue;
                }
                let van_first = first_idx(obj_ptr, &vanilla).map(|v| vanilla[v]);
                if van_first == Some(LAKITU) {
                    continue; // vanilla-native Lakitu, not injected
                }
                if patched[fi + 2] == 0x12 {
                    saw_alt = true;
                } else {
                    saw_kept = true;
                }
                if saw_alt && saw_kept {
                    break 'seeds;
                }
            }
        }
        assert!(saw_alt, "no injected Lakitu lifted to 0x12 in 60 seeds");
        assert!(saw_kept, "no injected Lakitu kept its inherited height in 60 seeds");
    }
