    use super::*;
    use crate::rom::Rom;

    const ANCHOR: u8 = 0x0A;

    // Item table offsets (must match items.rs)
    const HAMMER_BROS_ITEMS_OFFSET: usize = 0x16190;
    const TOAD_HOUSE_ITEMS_OFFSET: usize = 0x3B14B;

    /// Options safe for zeroed test ROMs.
    /// Palettes disabled because they use OS entropy (cosmetic, decoupled from seed).
    fn test_options() -> Options {
        Options {
            shuffle_airships: false,
            palettes: false,
            ..Default::default()
        }
    }

    /// Load the real SMB3 ROM. Tests that drive the full `randomize()`
    /// pipeline need it — the overworld builder reads real pointer
    /// tables and panics on synthetic data. Returns `None` (caller
    /// silently skips) when the ROM isn't in the project root, mirroring
    /// `map_walker::tests::test_render_randomized_seed`.
    fn make_test_rom() -> Option<Rom> {
        let bytes = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&bytes).ok()
    }

    #[test]
    fn mystery_anchor_trampoline_written() {
        let Some(mut rom) = make_test_rom() else { return };
        // Place anchors in item tables — they should stay as 0x0A
        rom.write_byte(HAMMER_BROS_ITEMS_OFFSET + 2, ANCHOR);
        rom.write_byte(TOAD_HOUSE_ITEMS_OFFSET + 1, ANCHOR);

        let mut options = test_options();
        options.chest_items = false;
        options.remove_whistles = false;
        // Pin Hammer Bro redistribution off so the planted anchor in the HB
        // reward table isn't relocated — this test is about the mystery-anchor
        // trampoline, not sprite shuffling.
        options.shuffle_hammer_bros = false;
        randomize(&mut rom, 0x12345678, &options);

        // Anchor items should remain in data tables (mystery behavior)
        assert_eq!(rom.read_byte(HAMMER_BROS_ITEMS_OFFSET + 2), ANCHOR,
            "Anchor should stay in item table (mystery item)");
        assert_eq!(rom.read_byte(TOAD_HOUSE_ITEMS_OFFSET + 1), ANCHOR,
            "Anchor should stay in item table (mystery item)");

        // Trampoline should be written at PRG026 free space
        use crate::randomize::rom_data::FS_MYSTERY_ANCHOR as FS;
        // Trampoline starts with LDX $7D80,Y (0xBE)
        assert_eq!(rom.read_byte(FS), 0xBE, "Trampoline LDX abs,Y opcode");
        // Target powerup is at offset +8 (LDX #imm operand)
        let target = rom.read_byte(FS + 8);
        assert!((0x01..=0x08).contains(&target),
            "Trampoline target 0x{target:02X} should be a valid mystery pool item (1-8)");

        // DynJump table entry at 0x34564: $A5B6 (Inv_UseItem_Powerup)
        assert_eq!(rom.read_range(0x34564, 2), &[0xB6, 0xA5]);
        // Hook at 0x345D8: JSR $B562
        assert_eq!(rom.read_range(0x345D8, 3), &[0x20, 0x62, 0xB5]);
    }

    #[test]
    fn write_log_populated_after_randomize() {
        let Some(mut rom) = make_test_rom() else { return };
        let options = test_options();
        randomize(&mut rom, 0x12345678, &options);

        let log = rom.write_log();
        assert!(!log.is_empty(), "Write log should be non-empty after randomize");

        // Every write should have a proper tag (not "untagged")
        for record in log {
            assert_ne!(
                record.tag, "untagged",
                "Write at offset 0x{:05X} has no tag",
                record.offset
            );
        }
    }

    #[test]
    fn default_matches_serde_empty_object() {
        // Guard against drift between the manual Default impl and the
        // #[serde(default = ...)] attributes. Adding a field to Options
        // requires both to agree, or this test fails. Critical because
        // the WASM `default_options_json()` export ships these defaults
        // to the JS layer for parity-checking the schema.
        let from_default = Options::default();
        let from_empty: Options = serde_json::from_str("{}").unwrap();
        assert_eq!(from_default, from_empty);
    }

    #[test]
    fn flag_key_round_trip_defaults() {
        let opts = Options::default();
        let key = opts.to_flag_key();
        assert!(key.starts_with("SMB3R-"));
        assert_eq!(key.len(), 26); // "SMB3R-" + 20 base32
        let decoded = Options::from_flag_key(&key).unwrap();
        assert_eq!(opts.powerups, decoded.powerups);
        assert_eq!(opts.palettes, decoded.palettes);
        assert_eq!(opts.world_order, decoded.world_order);
        assert_eq!(opts.world_count, decoded.world_count);
        assert_eq!(opts.big_q_blocks, decoded.big_q_blocks);
        assert_eq!(opts.disable_autoscroll, decoded.disable_autoscroll);
        assert_eq!(opts.chest_items, decoded.chest_items);
        assert_eq!(opts.remove_whistles, decoded.remove_whistles);
        assert_eq!(opts.shuffle_pipes, decoded.shuffle_pipes);
        assert_eq!(opts.shuffle_airships, decoded.shuffle_airships);
        assert_eq!(opts.shuffle_hammer_bros, decoded.shuffle_hammer_bros);
        assert_eq!(opts.more_hammer_rocks, decoded.more_hammer_rocks);
        assert_eq!(opts.starting_lives, decoded.starting_lives);
        assert_eq!(opts.card_speed_clear, decoded.card_speed_clear);
        assert_eq!(opts.remove_n_cards, decoded.remove_n_cards);
        assert_eq!(opts.skip_wand_cutscene, decoded.skip_wand_cutscene);
        assert_eq!(opts.adjust_boss_hitboxes, decoded.adjust_boss_hitboxes);
        assert_eq!(opts.ground, decoded.ground);
        assert_eq!(opts.shell, decoded.shell);
        assert_eq!(opts.flying, decoded.flying);
        assert_eq!(opts.piranhas, decoded.piranhas);
        assert_eq!(opts.ghosts, decoded.ghosts);
        assert_eq!(opts.thwomps, decoded.thwomps);
        assert_eq!(opts.rotodiscs, decoded.rotodiscs);
        assert_eq!(opts.cannons, decoded.cannons);
        assert_eq!(opts.water, decoded.water);
        assert_eq!(opts.bros, decoded.bros);
        assert_eq!(opts.hb_encounters, decoded.hb_encounters);
        assert_eq!(opts.wild_injections, decoded.wild_injections);
        assert_eq!(opts.starting_items, decoded.starting_items);
        assert_eq!(opts.hammer_breaks_locks, decoded.hammer_breaks_locks);
        assert_eq!(opts.hammer_breaks_bridges, decoded.hammer_breaks_bridges);
    }

    #[test]
    fn flag_key_round_trip_all_wild() {
        let opts = Options {
            fire_flower: FireFlowerMode::Wild,
            powerups: true,
            palettes: true,
            palette_themed: false,
            player_color: None,
            world_order: true,
            world_count: 7,
            big_q_blocks: true,
            shuffle_pipes: true,
            shuffle_airships: true,
            shuffle_hammer_bros: true,
            disable_autoscroll: true,
            chest_items: true,
            remove_whistles: true,
            more_hammer_rocks: Tri::On,
            eights_are_wild: Tri::On,
            starting_lives: 99,
            card_speed_clear: true,
            remove_n_cards: true,
            skip_wand_cutscene: true,
            adjust_boss_hitboxes: true,
            koopaling_hits: true,
            boomboom_hits: true,
            hammer_vulnerable_koopalings: true,
            random_koopalings: true,
            include_beta_stages: true,
            hammer_breaks_locks: Tri::On,
            hammer_breaks_bridges: Tri::On,
            early_sun: true,
            limit_bro_movement: true,
            japanese_damage: true,
            infinite_mushroom_houses: true,
            fast_mushroom_house: true,
            faster_tail_speed: true,
            no_game_over_penalty: true,
            faster_frog: true,
            shuffle_spade_games: true,
            shuffle_toad_houses: true,
            hands_levels: true,
            troll_pipes: Tri::On,
            swap_start_airship: false,
            ground: EnemyMode::Wild,
            shell: EnemyMode::Wild,
            flying: EnemyMode::Wild,
            piranhas: EnemyMode::Wild,
            ghosts: EnemyMode::Wild,
            thwomps: EnemyMode::Wild,
            rotodiscs: EnemyMode::Wild,
            cannons: EnemyMode::Wild,
            water: EnemyMode::Wild,
            bros: EnemyMode::Wild,
            hb_encounters: EnemyMode::Wild,
            wild_injections: true,
            starting_items: vec![0x05, 0x09, 0x03],
            skip_rom_validation: false,
            anchor_visuals: false,
        };
        let key = opts.to_flag_key();
        let decoded = Options::from_flag_key(&key).unwrap();
        assert_eq!(opts.random_koopalings, decoded.random_koopalings);
        assert_eq!(opts.include_beta_stages, decoded.include_beta_stages);
        assert_eq!(opts.starting_items, decoded.starting_items);
        assert_eq!(opts.hammer_breaks_locks, decoded.hammer_breaks_locks);
        assert_eq!(opts.hammer_breaks_bridges, decoded.hammer_breaks_bridges);
        assert_eq!(opts.world_order, decoded.world_order);
        assert_eq!(opts.world_count, decoded.world_count);
        assert_eq!(opts.starting_lives, decoded.starting_lives);
        assert_eq!(opts.ground, decoded.ground);
        assert_eq!(opts.shell, decoded.shell);
        assert_eq!(opts.thwomps, decoded.thwomps);
        assert_eq!(opts.rotodiscs, decoded.rotodiscs);
        assert_eq!(opts.cannons, decoded.cannons);
        assert_eq!(opts.hb_encounters, decoded.hb_encounters);
        assert_eq!(opts.wild_injections, decoded.wild_injections);
    }

    #[test]
    fn flag_key_round_trip_all_off() {
        let opts = Options {
            fire_flower: FireFlowerMode::Off,
            powerups: false,
            palettes: false,
            palette_themed: false,
            player_color: None,
            world_order: false,
            world_count: 7,
            big_q_blocks: false,
            shuffle_pipes: false,
            shuffle_airships: false,
            shuffle_hammer_bros: false,
            disable_autoscroll: false,
            chest_items: false,
            remove_whistles: false,
            more_hammer_rocks: Tri::Off,
            eights_are_wild: Tri::Off,
            starting_lives: 1,
            card_speed_clear: false,
            remove_n_cards: false,
            skip_wand_cutscene: false,
            adjust_boss_hitboxes: false,
            koopaling_hits: false,
            boomboom_hits: false,
            hammer_vulnerable_koopalings: false,
            random_koopalings: false,
            include_beta_stages: false,
            hammer_breaks_locks: Tri::Off,
            hammer_breaks_bridges: Tri::Off,
            early_sun: false,
            limit_bro_movement: false,
            japanese_damage: false,
            infinite_mushroom_houses: false,
            fast_mushroom_house: false,
            faster_tail_speed: false,
            no_game_over_penalty: false,
            faster_frog: false,
            shuffle_spade_games: false,
            shuffle_toad_houses: false,
            hands_levels: false,
            troll_pipes: Tri::Off,
            swap_start_airship: false,
            ground: EnemyMode::Off,
            shell: EnemyMode::Off,
            flying: EnemyMode::Off,
            piranhas: EnemyMode::Off,
            ghosts: EnemyMode::Off,
            thwomps: EnemyMode::Off,
            rotodiscs: EnemyMode::Off,
            cannons: EnemyMode::Off,
            water: EnemyMode::Off,
            bros: EnemyMode::Off,
            hb_encounters: EnemyMode::Off,
            wild_injections: false,
            starting_items: vec![],
            skip_rom_validation: false,
            anchor_visuals: false,
        };
        let key = opts.to_flag_key();
        let decoded = Options::from_flag_key(&key).unwrap();
        assert!(decoded.starting_items.is_empty());
        assert!(!decoded.powerups);
        assert_eq!(decoded.hammer_breaks_locks, Tri::Off);
        assert_eq!(decoded.hammer_breaks_bridges, Tri::Off);
        assert!(decoded.palettes); // palettes always true from flag key (cosmetic, not encoded)
        assert!(!decoded.disable_autoscroll);
        assert!(!decoded.shuffle_airships);
        assert!(!decoded.shuffle_spade_games);
        assert_eq!(decoded.ground, EnemyMode::Off);
        assert_eq!(decoded.thwomps, EnemyMode::Off);
        assert_eq!(decoded.hb_encounters, EnemyMode::Off);
        assert!(!decoded.wild_injections);
        assert_eq!(decoded.starting_lives, 1);
    }

    #[test]
    fn flag_key_case_insensitive_prefix() {
        let opts = Options::default();
        let key = opts.to_flag_key();
        let lower = key.to_lowercase();
        let decoded = Options::from_flag_key(&lower).unwrap();
        assert_eq!(opts.powerups, decoded.powerups);
    }

    #[test]
    fn flag_key_without_prefix() {
        let opts = Options::default();
        let key = opts.to_flag_key();
        let b32 = key.strip_prefix("SMB3R-").unwrap();
        let decoded = Options::from_flag_key(b32).unwrap();
        assert_eq!(opts.powerups, decoded.powerups);
    }

    #[test]
    fn flag_key_invalid_version() {
        // Encode version 0xFF into base32 (first byte = 0xFF, rest zeros)
        let mut bad_bytes = [0u8; 12];
        bad_bytes[0] = 0xFF;
        let key = format!("SMB3R-{}", base32_encode(&bad_bytes));
        let result = Options::from_flag_key(&key);
        assert!(result.is_err());
    }

    #[test]
    fn flag_key_invalid_chars() {
        let result = Options::from_flag_key("SMB3R-!!!!!!!!!!!!!!!!!!!");
        assert!(result.is_err());
    }

    /// Holistic flag-key check: every encoded option must (a) change the flag
    /// key when toggled away from defaults, and (b) round-trip exactly through
    /// encode→decode. Catches bit-collision bugs where two fields share a bit.
    ///
    /// `palettes` and `palette_themed` are cosmetic — they intentionally do not
    /// change the flag key, so they're tested in the `cosmetic` table.
    #[test]
    fn flag_key_per_option_round_trip() {
        // Helper: clone defaults, apply mutator, encode/decode, return both.
        fn check_round_trip(
            label: &str,
            mutate: impl Fn(&mut Options),
            change_key: bool,
        ) {
            let default_opts = Options::default();
            let default_key = default_opts.to_flag_key();

            let mut mutated = default_opts.clone();
            mutate(&mut mutated);

            let mutated_key = mutated.to_flag_key();
            if change_key {
                assert_ne!(
                    default_key, mutated_key,
                    "{label}: mutating did not change the flag key (bit collision?)",
                );
            } else {
                assert_eq!(
                    default_key, mutated_key,
                    "{label}: cosmetic field unexpectedly changed the flag key",
                );
            }

            // Decode round-trip. Cosmetic fields are not encoded, so the
            // decoder always returns palettes=true, palette_themed=false;
            // normalize the expected value to match before comparing.
            let mut expected = mutated.clone();
            expected.palettes = true;
            expected.palette_themed = false;

            let recovered = Options::from_flag_key(&mutated_key)
                .unwrap_or_else(|e| panic!("{label}: failed to decode key '{mutated_key}': {e}"));
            assert_eq!(
                recovered, expected,
                "{label}: round-trip mismatch\n  encoded: {mutated:?}\n  decoded: {recovered:?}",
            );
        }

        /// A label + a closure that flips one Options field.
        type OptionTweak = (&'static str, Box<dyn Fn(&mut Options)>);

        // Cosmetic: must NOT change the flag key.
        let cosmetic: Vec<OptionTweak> = vec![
            ("palettes",       Box::new(|o| o.palettes = !o.palettes)),
            ("palette_themed", Box::new(|o| o.palette_themed = !o.palette_themed)),
        ];
        for (label, mutate) in cosmetic {
            check_round_trip(label, mutate, false);
        }

        // Encoded booleans: toggling must change the flag key.
        let bools: Vec<OptionTweak> = vec![
            ("powerups",                     Box::new(|o| o.powerups = !o.powerups)),
            ("world_order",                  Box::new(|o| o.world_order = !o.world_order)),
            ("big_q_blocks",                 Box::new(|o| o.big_q_blocks = !o.big_q_blocks)),
            ("shuffle_pipes",                Box::new(|o| o.shuffle_pipes = !o.shuffle_pipes)),
            ("shuffle_airships",             Box::new(|o| o.shuffle_airships = !o.shuffle_airships)),
            ("shuffle_hammer_bros",          Box::new(|o| o.shuffle_hammer_bros = !o.shuffle_hammer_bros)),
            ("disable_autoscroll",           Box::new(|o| o.disable_autoscroll = !o.disable_autoscroll)),
            ("chest_items",                  Box::new(|o| o.chest_items = !o.chest_items)),
            ("remove_whistles",              Box::new(|o| o.remove_whistles = !o.remove_whistles)),
            ("card_speed_clear",             Box::new(|o| o.card_speed_clear = !o.card_speed_clear)),
            ("remove_n_cards",               Box::new(|o| o.remove_n_cards = !o.remove_n_cards)),
            ("skip_wand_cutscene",           Box::new(|o| o.skip_wand_cutscene = !o.skip_wand_cutscene)),
            ("adjust_boss_hitboxes",         Box::new(|o| o.adjust_boss_hitboxes = !o.adjust_boss_hitboxes)),
            ("koopaling_hits",               Box::new(|o| o.koopaling_hits = !o.koopaling_hits)),
            ("boomboom_hits",                Box::new(|o| o.boomboom_hits = !o.boomboom_hits)),
            ("hammer_vulnerable_koopalings", Box::new(|o| o.hammer_vulnerable_koopalings = !o.hammer_vulnerable_koopalings)),
            ("random_koopalings",            Box::new(|o| o.random_koopalings = !o.random_koopalings)),
            ("include_beta_stages",          Box::new(|o| o.include_beta_stages = !o.include_beta_stages)),
            ("shuffle_spade_games",           Box::new(|o| o.shuffle_spade_games = !o.shuffle_spade_games)),
            ("shuffle_toad_houses",          Box::new(|o| o.shuffle_toad_houses = !o.shuffle_toad_houses)),
            ("wild_injections",              Box::new(|o| o.wild_injections = !o.wild_injections)),
        ];
        for (label, mutate) in bools {
            check_round_trip(label, mutate, true);
        }

        // Tri-state enemy modes: cycle through every value so each non-default
        // mode is exercised. Defaults differ per class, so test all three modes.
        type TriSetter = Box<dyn Fn(&mut Options, EnemyMode)>;
        let tristates: Vec<(&str, TriSetter)> = vec![
            ("ground",        Box::new(|o, m| o.ground = m)),
            ("shell",         Box::new(|o, m| o.shell = m)),
            ("flying",        Box::new(|o, m| o.flying = m)),
            ("piranhas",      Box::new(|o, m| o.piranhas = m)),
            ("ghosts",        Box::new(|o, m| o.ghosts = m)),
            ("thwomps",       Box::new(|o, m| o.thwomps = m)),
            ("rotodiscs",     Box::new(|o, m| o.rotodiscs = m)),
            ("cannons",       Box::new(|o, m| o.cannons = m)),
            ("water",         Box::new(|o, m| o.water = m)),
            ("bros",          Box::new(|o, m| o.bros = m)),
            ("hb_encounters", Box::new(|o, m| o.hb_encounters = m)),
        ];
        for (label, set) in tristates {
            for &mode in &[EnemyMode::Off, EnemyMode::Shuffle, EnemyMode::Wild] {
                let default_opts = Options::default();
                let mut mutated = default_opts.clone();
                set(&mut mutated, mode);
                let mut expected = mutated.clone();
                expected.palettes = true;
                expected.palette_themed = false;
                let recovered = Options::from_flag_key(&mutated.to_flag_key()).unwrap();
                assert_eq!(
                    recovered, expected,
                    "{label}={mode:?}: round-trip mismatch",
                );
            }
        }

        // Player-hidden tri flags (Off/On/Maybe): every state must round-trip,
        // and every non-default state must change the flag key.
        type TriFlagSetter = Box<dyn Fn(&mut Options, Tri)>;
        let tri_flags: Vec<(&str, TriFlagSetter)> = vec![
            ("hammer_breaks_locks",   Box::new(|o, t| o.hammer_breaks_locks = t)),
            ("hammer_breaks_bridges", Box::new(|o, t| o.hammer_breaks_bridges = t)),
            ("troll_pipes",           Box::new(|o, t| o.troll_pipes = t)),
            ("more_hammer_rocks",        Box::new(|o, t| o.more_hammer_rocks = t)),
            ("eights_are_wild",       Box::new(|o, t| o.eights_are_wild = t)),
        ];
        for (label, set) in tri_flags {
            let default_opts = Options::default();
            let default_key = default_opts.to_flag_key();
            for &state in &[Tri::Off, Tri::On, Tri::Maybe] {
                let mut mutated = default_opts.clone();
                set(&mut mutated, state);
                let mutated_key = mutated.to_flag_key();
                let mut expected = mutated.clone();
                expected.palettes = true;
                expected.palette_themed = false;
                let recovered = Options::from_flag_key(&mutated_key).unwrap();
                assert_eq!(recovered, expected, "{label}={state:?}: round-trip mismatch");
                // Default state shares its key with default; non-default must differ.
                let is_default_state = recovered == {
                    let mut d = default_opts.clone();
                    d.palettes = true;
                    d.palette_themed = false;
                    d
                };
                if !is_default_state {
                    assert_ne!(default_key, mutated_key, "{label}={state:?}: key must change");
                }
            }
        }

        // starting_lives is 2 bits indexing {1, 5, 20, 99} — only the four
        // canonical values round-trip exactly.
        for lives in STARTING_LIVES_VALUES {
            let opts = Options { starting_lives: lives, ..Default::default() };
            let expected = Options { palettes: true, palette_themed: false, ..opts.clone() };
            let recovered = Options::from_flag_key(&opts.to_flag_key()).unwrap();
            assert_eq!(recovered.starting_lives, lives, "starting_lives={lives}: round-trip mismatch");
            assert_eq!(recovered, expected, "starting_lives={lives}: full struct mismatch");
        }
        for wc in 1u8..=7 {
            let opts = Options { world_count: wc, ..Default::default() };
            let expected = Options { palettes: true, palette_themed: false, ..opts.clone() };
            let recovered = Options::from_flag_key(&opts.to_flag_key()).unwrap();
            assert_eq!(recovered.world_count, wc, "world_count={wc}: round-trip mismatch");
            assert_eq!(recovered, expected, "world_count={wc}: full struct mismatch");
        }

        // starting_items: empty, singles, multi, sentinels (random modes).
        for items in [
            vec![],
            vec![3u8],
            vec![3, 6, 9],
            vec![ITEM_RANDOM, ITEM_RANDOM_NO_WHISTLE, ITEM_RANDOM_SUIT_ONLY],
        ] {
            let opts = Options { starting_items: items.clone(), ..Default::default() };
            let expected = Options { palettes: true, palette_themed: false, ..opts.clone() };
            let recovered = Options::from_flag_key(&opts.to_flag_key()).unwrap();
            assert_eq!(recovered.starting_items, items, "starting_items={items:?}: round-trip mismatch");
            assert_eq!(recovered, expected, "starting_items={items:?}: full struct mismatch");
        }

        // Combination: every encoded boolean flipped from default, all
        // tri-states set to Wild, level shuffle on, beta stages, items.
        // Catches bit-collision bugs that only manifest when many fields
        // share their non-default values.
        let mut everything = Options::default();
        everything.powerups = !everything.powerups;
        everything.world_order = !everything.world_order;
        everything.big_q_blocks = !everything.big_q_blocks;
        everything.shuffle_pipes = !everything.shuffle_pipes;
        everything.shuffle_airships = !everything.shuffle_airships;
        everything.shuffle_hammer_bros = !everything.shuffle_hammer_bros;
        everything.disable_autoscroll = !everything.disable_autoscroll;
        everything.chest_items = !everything.chest_items;
        everything.remove_whistles = !everything.remove_whistles;
        everything.more_hammer_rocks = Tri::Maybe;
        everything.eights_are_wild = Tri::Maybe;
        everything.card_speed_clear = !everything.card_speed_clear;
        everything.remove_n_cards = !everything.remove_n_cards;
        everything.skip_wand_cutscene = !everything.skip_wand_cutscene;
        everything.adjust_boss_hitboxes = !everything.adjust_boss_hitboxes;
        everything.koopaling_hits = !everything.koopaling_hits;
        everything.boomboom_hits = !everything.boomboom_hits;
        everything.hammer_vulnerable_koopalings = true;
        everything.random_koopalings = true;
        everything.include_beta_stages = true;
        everything.hammer_breaks_locks = Tri::Maybe;
        everything.hammer_breaks_bridges = Tri::On;
        everything.troll_pipes = Tri::Maybe;
        everything.shuffle_spade_games = !everything.shuffle_spade_games;
        everything.shuffle_toad_houses = !everything.shuffle_toad_houses;
        everything.wild_injections = true;
        everything.ground = EnemyMode::Wild;
        everything.shell = EnemyMode::Wild;
        everything.flying = EnemyMode::Wild;
        everything.piranhas = EnemyMode::Wild;
        everything.ghosts = EnemyMode::Wild;
        everything.thwomps = EnemyMode::Wild;
        everything.rotodiscs = EnemyMode::Wild;
        everything.cannons = EnemyMode::Wild;
        everything.water = EnemyMode::Wild;
        everything.bros = EnemyMode::Wild;
        everything.hb_encounters = EnemyMode::Wild;
        everything.starting_lives = 99;
        everything.world_count = 1;
        everything.starting_items = vec![ITEM_RANDOM, 5, ITEM_RANDOM_SUIT_ONLY];
        let mut expected = everything.clone();
        expected.palettes = true;
        expected.palette_themed = false;
        let recovered = Options::from_flag_key(&everything.to_flag_key()).unwrap();
        assert_eq!(recovered, expected, "all-fields-flipped: round-trip mismatch");
    }

    #[test]
    fn flag_key_hammer_vuln_koopalings_distinct_from_hb_encounters() {
        // Regression: hammer_vulnerable_koopalings used to share bit 2 of b4
        // with the high bit of hb_encounters (a tri-state at bits 2-1).
        // When hb_encounters=Wild (em=2), bit 2 was already set, so toggling
        // hammer_vulnerable_koopalings produced no change in the flag key.
        let a = Options {
            hb_encounters: EnemyMode::Wild,
            hammer_vulnerable_koopalings: false,
            ..Default::default()
        };

        let b = Options { hammer_vulnerable_koopalings: true, ..a.clone() };

        assert_ne!(a.to_flag_key(), b.to_flag_key(),
            "toggling hammer_vulnerable_koopalings must change the flag key");

        let dec_a = Options::from_flag_key(&a.to_flag_key()).unwrap();
        let dec_b = Options::from_flag_key(&b.to_flag_key()).unwrap();
        assert!(!dec_a.hammer_vulnerable_koopalings);
        assert!(dec_b.hammer_vulnerable_koopalings);
        assert_eq!(dec_a.hb_encounters, EnemyMode::Wild);
        assert_eq!(dec_b.hb_encounters, EnemyMode::Wild);
    }

    #[test]
    fn base32_round_trip() {
        // Test with various byte patterns
        for data in [
            vec![0u8; 11],
            vec![0xFF; 11],
            vec![0x0E, 0xFF, 0xFE, 0x63, 0xFC, 0xAA, 0xAA, 0xAA, 0x59, 0x37, 0xC0],
            (0..11).collect::<Vec<u8>>(),
        ] {
            let encoded = base32_encode(&data);
            let decoded = base32_decode(&encoded, data.len()).unwrap();
            assert_eq!(data, decoded, "round-trip failed for {data:?} (encoded: {encoded})");
        }
    }

    /// Inline FNV-1a hash — no external dependency needed.
    fn fnv1a(data: &[u8]) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for &b in data {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }

    /// Build an Options with everything disabled (exercises "skip everything" branches).
    fn all_off_options() -> Options {
        Options {
            fire_flower: FireFlowerMode::Off,
            powerups: false,
            palettes: false,
            palette_themed: false,
            player_color: None,
            world_order: false,
            world_count: 7,
            big_q_blocks: false,
            shuffle_pipes: false,
            shuffle_airships: false,
            shuffle_hammer_bros: false,
            disable_autoscroll: false,
            chest_items: false,
            remove_whistles: false,
            more_hammer_rocks: Tri::Off,
            eights_are_wild: Tri::Off,
            starting_lives: 1,
            card_speed_clear: false,
            remove_n_cards: false,
            skip_wand_cutscene: false,
            adjust_boss_hitboxes: false,
            koopaling_hits: false,
            boomboom_hits: false,
            hammer_vulnerable_koopalings: false,
            random_koopalings: false,
            include_beta_stages: false,
            hammer_breaks_locks: Tri::Off,
            hammer_breaks_bridges: Tri::Off,
            early_sun: false,
            limit_bro_movement: false,
            japanese_damage: false,
            infinite_mushroom_houses: false,
            fast_mushroom_house: false,
            faster_tail_speed: false,
            no_game_over_penalty: false,
            faster_frog: false,
            shuffle_spade_games: false,
            shuffle_toad_houses: false,
            hands_levels: false,
            troll_pipes: Tri::Off,
            swap_start_airship: false,
            ground: EnemyMode::Off,
            shell: EnemyMode::Off,
            flying: EnemyMode::Off,
            piranhas: EnemyMode::Off,
            ghosts: EnemyMode::Off,
            thwomps: EnemyMode::Off,
            rotodiscs: EnemyMode::Off,
            cannons: EnemyMode::Off,
            water: EnemyMode::Off,
            bros: EnemyMode::Off,
            hb_encounters: EnemyMode::Off,
            wild_injections: false,
            starting_items: vec![],
            skip_rom_validation: false,
            anchor_visuals: false,
        }
    }

    /// Build an Options with all features cranked to max.
    /// Palettes disabled because they use OS entropy (cosmetic, decoupled from seed).
    fn all_on_options() -> Options {
        Options {
            fire_flower: FireFlowerMode::On,
            powerups: true,
            palettes: false,
            palette_themed: false,
            player_color: None,
            world_order: true,
            world_count: 3,
            big_q_blocks: true,
            shuffle_pipes: false,
            shuffle_airships: true,
            shuffle_hammer_bros: true,
            disable_autoscroll: true,
            chest_items: true,
            remove_whistles: true,
            more_hammer_rocks: Tri::On,
            eights_are_wild: Tri::On,
            starting_lives: 99,
            card_speed_clear: true,
            remove_n_cards: true,
            skip_wand_cutscene: true,
            adjust_boss_hitboxes: true,
            koopaling_hits: true,
            boomboom_hits: true,
            hammer_vulnerable_koopalings: true,
            random_koopalings: true,
            include_beta_stages: false,
            hammer_breaks_locks: Tri::On,
            hammer_breaks_bridges: Tri::On,
            early_sun: true,
            limit_bro_movement: true,
            japanese_damage: true,
            infinite_mushroom_houses: true,
            fast_mushroom_house: true,
            faster_tail_speed: true,
            no_game_over_penalty: true,
            faster_frog: true,
            shuffle_spade_games: true,
            shuffle_toad_houses: true,
            hands_levels: true,
            troll_pipes: Tri::On,
            swap_start_airship: false,
            ground: EnemyMode::Wild,
            shell: EnemyMode::Wild,
            flying: EnemyMode::Wild,
            piranhas: EnemyMode::Wild,
            ghosts: EnemyMode::Wild,
            thwomps: EnemyMode::Wild,
            rotodiscs: EnemyMode::Wild,
            cannons: EnemyMode::Wild,
            water: EnemyMode::Wild,
            bros: EnemyMode::Wild,
            hb_encounters: EnemyMode::Wild,
            wild_injections: true,
            starting_items: vec![0x05, 0x09, 0x03],
            skip_rom_validation: false,
            anchor_visuals: true,
        }
    }

    /// Build an Options testing world_order in isolation (no enemy RNG consumption).
    fn world_order_only_options() -> Options {
        let mut opts = all_off_options();
        opts.world_order = true;
        opts.world_count = 5;
        opts
    }

    #[test]
    fn test_full_determinism() {
        let configs: Vec<(&str, Options)> = vec![
            ("defaults", test_options()),
            ("all_on", all_on_options()),
            ("all_off", all_off_options()),
            ("world_order_only", world_order_only_options()),
        ];

        let seed = 42u64;
        for (name, options) in &configs {
            // Run 1
            let Some(mut rom1) = make_test_rom() else { return };
            randomize(&mut rom1, seed, options);

            // Run 2 (same seed, same options)
            let Some(mut rom2) = make_test_rom() else { return };
            randomize(&mut rom2, seed, options);

            // Same-run determinism — find first differing byte for diagnostics
            let b1 = rom1.output_bytes();
            let b2 = rom2.output_bytes();
            if b1 != b2 {
                for i in 0..b1.len() {
                    if b1[i] != b2[i] {
                        panic!(
                            "{name}: non-determinism at offset 0x{i:05X}: \
                             run1=0x{:02X} run2=0x{:02X}",
                            b1[i], b2[i]
                        );
                    }
                }
            }

            // Verify hashes match (determinism, not pinned to a specific value)
            let hash1 = fnv1a(b1);
            let hash2 = fnv1a(b2);
            assert_eq!(
                hash1, hash2,
                "{name}: hash mismatch between runs (0x{hash1:016X} vs 0x{hash2:016X})"
            );
        }
    }

    #[test]
    fn maybe_flags_are_deterministic_and_hidden() {
        // A `Maybe` flag must (1) round-trip through the flag key as `Maybe`,
        // (2) produce a flag key indistinguishable from the seed-resolved
        // concrete states (the value bit is forced to 0, like Off), and
        // (3) generate byte-identical ROMs across runs with the same seed.
        let mut opts = test_options();
        opts.troll_pipes = Tri::Maybe;
        opts.hammer_breaks_locks = Tri::Maybe;

        // (1) round-trip
        let decoded = Options::from_flag_key(&opts.to_flag_key()).unwrap();
        assert_eq!(decoded.troll_pipes, Tri::Maybe);
        assert_eq!(decoded.hammer_breaks_locks, Tri::Maybe);

        // (2) hidden: a Maybe key differs from both On and Off keys, so the
        // player can't read the resolved state off it.
        let on = Options { troll_pipes: Tri::On, hammer_breaks_locks: Tri::On, ..test_options() };
        let off = Options { troll_pipes: Tri::Off, hammer_breaks_locks: Tri::Off, ..test_options() };
        assert_ne!(opts.to_flag_key(), on.to_flag_key());
        assert_ne!(opts.to_flag_key(), off.to_flag_key());

        // (3) determinism across runs (needs the real ROM).
        let seed = 0xC0FFEEu64;
        let Some(mut rom1) = make_test_rom() else { return };
        let Some(mut rom2) = make_test_rom() else { return };
        randomize(&mut rom1, seed, &opts);
        randomize(&mut rom2, seed, &opts);
        assert_eq!(
            fnv1a(rom1.output_bytes()),
            fnv1a(rom2.output_bytes()),
            "Maybe flags must resolve identically for the same seed",
        );
    }

    #[test]
    fn maybe_resolves_both_ways_across_seeds() {
        // The more_hammer_rocks=Maybe coin flip must actually flip: across many
        // seeds it should land On for some and Off for others. We isolate the
        // *gameplay* effect (the make_hammer_rocks tile write) by comparing
        // each Maybe run's tile bytes to the explicit-On run's bytes, so the
        // flag-key stamp / title hash (which always differ for Maybe) don't
        // confound the comparison.
        let Some(_) = make_test_rom() else { return };
        let on = Options { more_hammer_rocks: Tri::On, ..test_options() };
        let maybe = Options { more_hammer_rocks: Tri::Maybe, ..test_options() };

        // Capture the byte ranges make_hammer_rocks touches from a known-On run.
        let on_touched: Vec<(usize, Vec<u8>)> = {
            let mut rom = make_test_rom().unwrap();
            randomize(&mut rom, 0, &on);
            rom.write_log().iter()
                .filter(|r| r.tag == "qol/more_hammer_rocks")
                .map(|r| (r.offset, rom.read_range(r.offset, r.len).to_vec()))
                .collect()
        };
        assert!(!on_touched.is_empty(), "expected more_hammer_rocks to write bytes when On");

        let mut saw_on = false;
        let mut saw_off = false;
        for seed in 0u64..24 {
            let mut rom = make_test_rom().unwrap();
            randomize(&mut rom, seed, &maybe);
            let matches_on = on_touched.iter()
                .all(|(off, bytes)| rom.read_range(*off, bytes.len()) == bytes.as_slice());
            if matches_on { saw_on = true; } else { saw_off = true; }
        }
        assert!(saw_on && saw_off,
            "more_hammer_rocks=Maybe never exercised both outcomes across 24 seeds \
             (saw_on={saw_on}, saw_off={saw_off})");
    }

    #[test]
    fn write_log_tags_match_enabled_modules() {
        let Some(mut rom) = make_test_rom() else { return };
        let mut options = test_options();
        // Disable optional modules we can check for absence
        options.ground = EnemyMode::Off;
        options.shell = EnemyMode::Off;
        options.flying = EnemyMode::Off;
        options.piranhas = EnemyMode::Off;
        options.ghosts = EnemyMode::Off;
        options.water = EnemyMode::Off;
        options.bros = EnemyMode::Off;
        options.world_order = false;
        // Keep this on — it writes to known offsets even on a zeroed ROM.
        options.disable_autoscroll = true;
        randomize(&mut rom, 42, &options);

        let tags: Vec<&str> = rom.write_log().iter().map(|r| r.tag.as_str()).collect();
        // These modules write to fixed offsets that differ from zero
        assert!(tags.iter().any(|t| t.starts_with("autoscroll")));
        // Disabled modules should not appear
        assert!(!tags.iter().any(|t| t.starts_with("enemies")));
        assert!(!tags.iter().any(|t| t.starts_with("world_order")));
    }

    #[test]
    fn flag_key_round_trip_all_random_items() {
        let opts = Options {
            starting_items: vec![ITEM_RANDOM, ITEM_RANDOM_NO_WHISTLE, ITEM_RANDOM_SUIT_ONLY],
            ..Default::default()
        };
        let key = opts.to_flag_key();
        let decoded = Options::from_flag_key(&key).unwrap();
        assert_eq!(decoded.starting_items, vec![ITEM_RANDOM, ITEM_RANDOM_NO_WHISTLE, ITEM_RANDOM_SUIT_ONLY]);
    }

    #[test]
    fn flag_key_round_trip_mixed_random_and_concrete() {
        let opts = Options {
            starting_items: vec![ITEM_RANDOM, 3],
            ..Default::default()
        };
        let key = opts.to_flag_key();
        let decoded = Options::from_flag_key(&key).unwrap();
        assert_eq!(decoded.starting_items, vec![ITEM_RANDOM, 3]);
    }

    #[test]
    fn resolve_starting_item_deterministic() {
        let mut rng1 = ChaCha8Rng::seed_from_u64(42);
        let mut rng2 = ChaCha8Rng::seed_from_u64(42);
        let a = resolve_starting_item(ITEM_RANDOM, &mut rng1);
        let b = resolve_starting_item(ITEM_RANDOM, &mut rng2);
        assert_eq!(a, b, "same seed must produce same item");
    }

    #[test]
    fn resolve_suit_only_in_range() {
        for seed in 0..100u64 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let item = resolve_starting_item(ITEM_RANDOM_SUIT_ONLY, &mut rng);
            assert!((1..=6).contains(&item), "suit-only produced {item}, expected 1-6");
        }
    }

    #[test]
    fn resolve_no_whistle_never_whistle() {
        for seed in 0..100u64 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let item = resolve_starting_item(ITEM_RANDOM_NO_WHISTLE, &mut rng);
            assert_ne!(item, 0x0C, "no-whistle produced a whistle on seed {seed}");
            assert!((1..=13).contains(&item), "no-whistle produced {item}, expected 1-13 (not 12)");
        }
    }

    #[test]
    fn resolve_concrete_passthrough() {
        let mut rng = ChaCha8Rng::seed_from_u64(0);
        assert_eq!(resolve_starting_item(0, &mut rng), 0);
        assert_eq!(resolve_starting_item(5, &mut rng), 5);
        assert_eq!(resolve_starting_item(13, &mut rng), 13);
    }
