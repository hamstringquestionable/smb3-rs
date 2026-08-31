use super::*;
use crate::rom::Rom;

const ANCHOR: u8 = 0x0A;

// Item table offsets (must match items.rs)
const HAMMER_BROS_ITEMS_OFFSET: usize = 0x16190;
const TOAD_HOUSE_ITEMS_OFFSET: usize = 0x3B14B;

/// Options safe for zeroed test ROMs.
/// Palettes disabled because they use OS entropy (cosmetic, decoupled from seed).
fn test_options() -> Options {
    Options { shuffle_airships: false, palettes: false, ..Default::default() }
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

/// What the flag-key decoder should return for `o`: cosmetic fields are not
/// encoded, so decoding always yields palettes=true and palette_themed=false.
fn normalized(mut o: Options) -> Options {
    o.palettes = true;
    o.palette_themed = false;
    o.remove_flashing = true;
    o
}

/// 7-F1 cannot be beaten without flight, and which bonus room its Big [?] pipe
/// opens is now drawn per seed — so the block in *whatever room it drew* has to
/// be a flight suit, and the contents roll has to leave that byte alone.
///
/// The failure mode is silent: the force has to be the LAST write to that byte,
/// and if `big_q_rooms` ever moved back above `randomize_big_q_blocks` the roll
/// would overwrite it while everything still appeared to run. So this walks the
/// drawn room's own object stream rather than trusting any fixed offset.
///
/// Run with the room shuffle both **on and off**. The contents roll exempts
/// nothing either way, so with the shuffle off the force still has to fire —
/// against 7-F1's vanilla room. That arm is the one a `shuffle_big_q_rooms`
/// gate could silently drop, since skipping it looks like "the feature is off".
#[test]
fn w7f1_block_is_always_a_flight_suit() {
    let Some(base) = make_test_rom() else { return };

    for shuffle_rooms in [false, true] {
        let mut opts = test_options();
        opts.big_q_blocks = true;
        opts.shuffle_big_q_rooms = shuffle_rooms;

        for seed in 1..=40u64 {
            let mut rom = base.clone();
            randomize(&mut rom, seed, &opts);

            // Row 8 of the lookup table is 7-F1; its area and the arrival's screen
            // nibble name the room it drew.
            let base_off = randomize::rom_data::FS_BIG_Q_LOOKUP;
            let area = rom.read_byte(base_off + randomize::qol::big_q::OFF_ROOM + 8);
            let screen = rom.read_byte(base_off + randomize::qol::big_q::OFF_ARR_X + 8) & 0x0F;

            let block =
                randomize::big_q_rooms::block_offset(&rom, area, screen).unwrap_or_else(|| {
                    panic!(
                        "seed {seed}: 7-F1 drew area {area} screen {screen}, \
                                       which holds no Big [?] block"
                    )
                });
            assert_eq!(
                rom.read_byte(block),
                randomize::big_q_rooms::BIGQBLOCK_TANOOKI,
                "seed {seed}: 7-F1's room (area {area} screen {screen}) does not hand out \
             flight (shuffle_big_q_rooms = {shuffle_rooms})",
            );
            // With the shuffle off, nothing may move 7-F1 off its vanilla room.
            if !shuffle_rooms {
                assert_eq!((area, screen), (6, 6), "seed {seed}: room moved with the shuffle off");
            }
        }
    }
}

#[test]
fn mystery_anchor_trampoline_written() {
    let Some(mut rom) = make_test_rom() else {
        eprintln!("SKIP: requires the ROM, which is not included in the repo");
        return;
    };
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
    assert_eq!(
        rom.read_byte(HAMMER_BROS_ITEMS_OFFSET + 2),
        ANCHOR,
        "Anchor should stay in item table (mystery item)"
    );
    assert_eq!(
        rom.read_byte(TOAD_HOUSE_ITEMS_OFFSET + 1),
        ANCHOR,
        "Anchor should stay in item table (mystery item)"
    );

    // Trampoline should be written at PRG026 free space
    use crate::randomize::rom_data::FS_MYSTERY_ANCHOR as FS;
    // Trampoline starts with LDX $7D80,Y (0xBE)
    assert_eq!(rom.read_byte(FS), 0xBE, "Trampoline LDX abs,Y opcode");
    // Target powerup is at offset +8 (LDX #imm operand)
    let target = rom.read_byte(FS + 8);
    assert!(
        (0x01..=0x08).contains(&target),
        "Trampoline target 0x{target:02X} should be a valid mystery pool item (1-8)"
    );

    // DynJump table entry at 0x34564: $A5B6 (Inv_UseItem_Powerup)
    assert_eq!(rom.read_range(0x34564, 2), &[0xB6, 0xA5]);
    // Hook at 0x345D8: JSR $B562
    assert_eq!(rom.read_range(0x345D8, 3), &[0x20, 0x62, 0xB5]);
}

/// Everything that owns ROM free space, turned on. `all_on_options` is close
/// but pins `world_count` to 3 and leaves `swap_start_airship` off, and both
/// gate allocations we want exercised.
fn audit_options() -> Options {
    Options { world_count: 7, swap_start_airship: true, ..all_on_options() }
}

/// Cross-check `FREE_SPACE_ALLOCATIONS` against a real run: every byte written
/// inside a registered region must come from the module that owns it, and no
/// write may cross a region boundary.
///
/// This is the only check on free space that looks at what the randomizer
/// *does* rather than at what the registry says about itself. Run with
/// `--nocapture` to see the usage table — the `used` column is where the
/// `// N reserved, M used` comments come from.
#[test]
fn free_space_audit_matches_registry() {
    use crate::randomize::rom_data::{audit_free_space, format_free_space_report};

    let Some(mut rom) = make_test_rom() else {
        eprintln!("SKIP: requires the ROM, which is not included in the repo");
        return;
    };
    randomize(&mut rom, 0xA11C0DE, &audit_options());

    let usage = audit_free_space(&rom);
    println!("{}", format_free_space_report(&rom));

    let problems: Vec<String> = usage
        .iter()
        .filter(|u| u.is_problem())
        .map(|u| {
            let foreign: Vec<String> =
                u.foreign.iter().map(|(tag, n)| format!("{n} byte(s) tagged '{tag}'")).collect();
            let over: Vec<String> = u
                .overruns
                .iter()
                .map(|(off, len, tag)| {
                    format!("0x{off:05X}+{len} tagged '{tag}' crosses the boundary")
                })
                .collect();
            format!(
                "0x{:05X} ({}, owner '{}'): {}",
                u.alloc.offset,
                u.alloc.label,
                u.alloc.owners.join(" + "),
                foreign.into_iter().chain(over).collect::<Vec<_>>().join("; "),
            )
        })
        .collect();

    assert!(
        problems.is_empty(),
        "free-space allocations written by a non-owner or overrun:\n  {}",
        problems.join("\n  "),
    );

    // A flag-gated patch that never ran writes nothing, and an audit over
    // nothing passes — the trap that made the overworld baseline vacuous for
    // hammer_breaks. Every allocation must be exercised for the check above to
    // mean anything, so assert that rather than trusting it.
    let untouched: Vec<String> = usage
        .iter()
        .filter(|u| u.used == 0)
        .map(|u| {
            format!(
                "0x{:05X} {} (owner '{}')",
                u.alloc.offset,
                u.alloc.label,
                u.alloc.owners.join(" + ")
            )
        })
        .collect();
    assert!(
        untouched.is_empty(),
        "these allocations saw no writes, so the audit above proved nothing about them.\n\
         Turn the owning feature on in audit_options(), or (if the write is seed-dependent)\n\
         pick a seed that exercises it:\n  {}",
        untouched.join("\n  "),
    );
}

#[test]
fn write_log_populated_after_randomize() {
    let Some(mut rom) = make_test_rom() else {
        eprintln!("SKIP: requires the ROM, which is not included in the repo");
        return;
    };
    let options = test_options();
    randomize(&mut rom, 0x12345678, &options);

    let log = rom.write_log();
    assert!(!log.is_empty(), "Write log should be non-empty after randomize");

    // Every write should have a proper tag (not "untagged")
    for record in log {
        assert_ne!(record.tag, "untagged", "Write at offset 0x{:05X} has no tag", record.offset);
    }
}

/// The web layer posts `wild_injections` as an array of these strings (the lit
/// pills in `web/options.js`). Deserialization is the only contract between
/// them, so pin the spelling here — a rename on either side breaks generation
/// at runtime with no compile error.
#[test]
fn wild_chasers_parse_web_values() {
    for (json, want) in [
        ("[]", vec![]),
        (r#"["sun"]"#, vec![WildChaser::Sun]),
        (r#"["lakitu"]"#, vec![WildChaser::Lakitu]),
        (r#"["bass"]"#, vec![WildChaser::Bass]),
        (r#"["sun","bass"]"#, vec![WildChaser::Sun, WildChaser::Bass]),
        (r#"["sun","lakitu","bass"]"#, WildChaser::ALL.to_vec()),
    ] {
        let opts: Options =
            serde_json::from_str(&format!(r#"{{"wild_injections":{json}}}"#)).unwrap();
        assert_eq!(opts.wild_injections, want, "web value {json} did not parse");
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
    assert_eq!(decoded, normalized(opts));
}

#[test]
fn flag_key_round_trip_all_wild() {
    // Everything flipped away from the all-off baseline.
    let opts = Options {
        fire_flower: FireFlowerMode::Wild,
        piranha_shuffle: PiranhaMode::Wild,
        powerups: true,
        palettes: true,
        world_order: true,
        big_q_blocks: true,
        shuffle_airships: true,
        shuffle_hammer_bros: true,
        disable_autoscroll: true,
        chest_items: true,
        remove_whistles: true,
        more_hammer_rocks: Tri::On,
        eights_are_wild: Tri::On,
        antechamber_shuffle: Tri::On,
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
        bro_battle_timer: true,
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
        wild_injections: WildChaser::ALL.to_vec(),
        starting_items: vec![0x05, 0x09, 0x03],
        ..all_off_options()
    };
    let key = opts.to_flag_key();
    let decoded = Options::from_flag_key(&key).unwrap();
    assert_eq!(opts.random_koopalings, decoded.random_koopalings);
    assert_eq!(opts.include_beta_stages, decoded.include_beta_stages);
    assert_eq!(opts.starting_items, decoded.starting_items);
    assert_eq!(opts.hammer_breaks_locks, decoded.hammer_breaks_locks);
    assert_eq!(opts.hammer_breaks_bridges, decoded.hammer_breaks_bridges);
    assert_eq!(opts.antechamber_shuffle, decoded.antechamber_shuffle);
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
    let opts = all_off_options();
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
    assert!(decoded.wild_injections.is_empty());
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
fn flag_key_mixed_case_prefix() {
    let opts = Options::default();
    let key = opts.to_flag_key();
    let mixed = format!("Smb3r-{}", key.strip_prefix("SMB3R-").unwrap());
    let decoded = Options::from_flag_key(&mixed).unwrap();
    assert_eq!(decoded, normalized(opts));
}

#[test]
fn flag_key_without_prefix() {
    let opts = Options::default();
    let key = opts.to_flag_key();
    let b32 = key.strip_prefix("SMB3R-").unwrap();
    let decoded = Options::from_flag_key(b32).unwrap();
    assert_eq!(opts.powerups, decoded.powerups);
}

/// A well-formed key from a format version this build doesn't speak is
/// rejected — but only after passing the envelope check, so the app can say
/// "older/newer version" rather than "mistyped".
#[test]
fn flag_key_invalid_version() {
    for version in [0x00, FLAG_KEY_VERSION - 1, FLAG_KEY_VERSION + 1, 0xFF] {
        let key = forge_key(version, &Options::default().to_flag_bytes()[2..]);
        assert_eq!(flag_key_version_of(&key), Ok(version), "version probe on v{version}");
        assert!(Options::from_flag_key(&key).is_err(), "v{version} must not decode");
    }
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
    fn check_round_trip(label: &str, mutate: impl Fn(&mut Options), change_key: bool) {
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

        // Decode round-trip, normalizing cosmetic fields on the expected side.
        let expected = normalized(mutated.clone());

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
        ("palettes", Box::new(|o| o.palettes = !o.palettes)),
        ("palette_themed", Box::new(|o| o.palette_themed = !o.palette_themed)),
        ("remove_flashing", Box::new(|o| o.remove_flashing = !o.remove_flashing)),
    ];
    for (label, mutate) in cosmetic {
        check_round_trip(label, mutate, false);
    }

    // Encoded booleans: toggling must change the flag key.
    let bools: Vec<OptionTweak> = vec![
        ("powerups", Box::new(|o| o.powerups = !o.powerups)),
        ("world_order", Box::new(|o| o.world_order = !o.world_order)),
        ("big_q_blocks", Box::new(|o| o.big_q_blocks = !o.big_q_blocks)),
        ("shuffle_airships", Box::new(|o| o.shuffle_airships = !o.shuffle_airships)),
        ("shuffle_hammer_bros", Box::new(|o| o.shuffle_hammer_bros = !o.shuffle_hammer_bros)),
        ("disable_autoscroll", Box::new(|o| o.disable_autoscroll = !o.disable_autoscroll)),
        ("chest_items", Box::new(|o| o.chest_items = !o.chest_items)),
        ("remove_whistles", Box::new(|o| o.remove_whistles = !o.remove_whistles)),
        ("card_speed_clear", Box::new(|o| o.card_speed_clear = !o.card_speed_clear)),
        ("remove_n_cards", Box::new(|o| o.remove_n_cards = !o.remove_n_cards)),
        ("skip_wand_cutscene", Box::new(|o| o.skip_wand_cutscene = !o.skip_wand_cutscene)),
        ("adjust_boss_hitboxes", Box::new(|o| o.adjust_boss_hitboxes = !o.adjust_boss_hitboxes)),
        ("koopaling_hits", Box::new(|o| o.koopaling_hits = !o.koopaling_hits)),
        ("boomboom_hits", Box::new(|o| o.boomboom_hits = !o.boomboom_hits)),
        (
            "hammer_vulnerable_koopalings",
            Box::new(|o| o.hammer_vulnerable_koopalings = !o.hammer_vulnerable_koopalings),
        ),
        ("random_koopalings", Box::new(|o| o.random_koopalings = !o.random_koopalings)),
        ("include_beta_stages", Box::new(|o| o.include_beta_stages = !o.include_beta_stages)),
        ("shuffle_spade_games", Box::new(|o| o.shuffle_spade_games = !o.shuffle_spade_games)),
        ("shuffle_toad_houses", Box::new(|o| o.shuffle_toad_houses = !o.shuffle_toad_houses)),
        ("anchor_visuals", Box::new(|o| o.anchor_visuals = !o.anchor_visuals)),
    ];
    for (label, mutate) in bools {
        check_round_trip(label, mutate, true);
    }

    // Tri-state enemy modes: cycle through every value so each non-default
    // mode is exercised. Defaults differ per class, so test all three modes.
    type TriSetter = Box<dyn Fn(&mut Options, EnemyMode)>;
    let tristates: Vec<(&str, TriSetter)> = vec![
        ("ground", Box::new(|o, m| o.ground = m)),
        ("shell", Box::new(|o, m| o.shell = m)),
        ("flying", Box::new(|o, m| o.flying = m)),
        ("piranhas", Box::new(|o, m| o.piranhas = m)),
        ("ghosts", Box::new(|o, m| o.ghosts = m)),
        ("thwomps", Box::new(|o, m| o.thwomps = m)),
        ("rotodiscs", Box::new(|o, m| o.rotodiscs = m)),
        ("cannons", Box::new(|o, m| o.cannons = m)),
        ("water", Box::new(|o, m| o.water = m)),
        ("bros", Box::new(|o, m| o.bros = m)),
        ("hb_encounters", Box::new(|o, m| o.hb_encounters = m)),
    ];
    for (label, set) in tristates {
        for &mode in &[EnemyMode::Off, EnemyMode::Shuffle, EnemyMode::Wild] {
            let default_opts = Options::default();
            let mut mutated = default_opts.clone();
            set(&mut mutated, mode);
            let expected = normalized(mutated.clone());
            let recovered = Options::from_flag_key(&mutated.to_flag_key()).unwrap();
            assert_eq!(recovered, expected, "{label}={mode:?}: round-trip mismatch",);
        }
    }

    // Player-hidden tri flags (Off/On/Maybe): every state must round-trip,
    // and every non-default state must change the flag key.
    type TriFlagSetter = Box<dyn Fn(&mut Options, Tri)>;
    let tri_flags: Vec<(&str, TriFlagSetter)> = vec![
        ("hammer_breaks_locks", Box::new(|o, t| o.hammer_breaks_locks = t)),
        ("hammer_breaks_bridges", Box::new(|o, t| o.hammer_breaks_bridges = t)),
        ("troll_pipes", Box::new(|o, t| o.troll_pipes = t)),
        ("more_hammer_rocks", Box::new(|o, t| o.more_hammer_rocks = t)),
        ("eights_are_wild", Box::new(|o, t| o.eights_are_wild = t)),
    ];
    for (label, set) in tri_flags {
        let default_opts = Options::default();
        let default_key = default_opts.to_flag_key();
        for &state in &[Tri::Off, Tri::On, Tri::Maybe] {
            let mut mutated = default_opts.clone();
            set(&mut mutated, state);
            let mutated_key = mutated.to_flag_key();
            let expected = normalized(mutated.clone());
            let recovered = Options::from_flag_key(&mutated_key).unwrap();
            assert_eq!(recovered, expected, "{label}={state:?}: round-trip mismatch");
            // Default state shares its key with default; non-default must differ.
            let is_default_state = recovered == normalized(default_opts.clone());
            if !is_default_state {
                assert_ne!(default_key, mutated_key, "{label}={state:?}: key must change");
            }
        }
    }

    // Piranha shuffle (Off/On/Wild): every state must round-trip, and the
    // non-default states must change the flag key.
    {
        let default_key = Options::default().to_flag_key();
        for &mode in &[PiranhaMode::Off, PiranhaMode::On, PiranhaMode::Wild] {
            let mutated = Options { piranha_shuffle: mode, ..Default::default() };
            let mutated_key = mutated.to_flag_key();
            let expected = normalized(mutated.clone());
            let recovered = Options::from_flag_key(&mutated_key).unwrap();
            assert_eq!(recovered, expected, "piranha_shuffle={mode:?}: round-trip mismatch");
            if mode != PiranhaMode::Off {
                assert_ne!(default_key, mutated_key, "piranha_shuffle={mode:?}: key must change");
            }
        }
    }

    // Wild injections: one bit per chaser, scattered across b4 bit 0, b12 bit 7
    // and b2 bit 5 (no byte had three free). Every one of the 8 subsets must
    // round-trip, and all 8 must produce distinct keys — a bit landing on top
    // of a neighbour would otherwise show up as two subsets sharing a key.
    {
        let default_key = Options::default().to_flag_key();
        let mut keys = Vec::new();
        for bits in 0u8..8 {
            let set: Vec<WildChaser> = WildChaser::ALL
                .iter()
                .enumerate()
                .filter(|(i, _)| bits & (1 << i) != 0)
                .map(|(_, &c)| c)
                .collect();
            let mutated = Options { wild_injections: set.clone(), ..Default::default() };
            let mutated_key = mutated.to_flag_key();
            let expected = normalized(mutated.clone());
            let recovered = Options::from_flag_key(&mutated_key).unwrap();
            assert_eq!(recovered, expected, "wild_injections={set:?}: round-trip mismatch");
            if !set.is_empty() {
                assert_ne!(default_key, mutated_key, "wild_injections={set:?}: key must change");
            }
            keys.push(mutated_key);
        }
        keys.sort();
        let distinct = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), distinct, "wild_injections: two chaser sets share a key");
    }

    // starting_lives is 2 bits indexing {1, 5, 20, 99} — only the four
    // canonical values round-trip exactly.
    for lives in STARTING_LIVES_VALUES {
        let opts = Options { starting_lives: lives, ..Default::default() };
        let expected = normalized(opts.clone());
        let recovered = Options::from_flag_key(&opts.to_flag_key()).unwrap();
        assert_eq!(recovered.starting_lives, lives, "starting_lives={lives}: round-trip mismatch");
        assert_eq!(recovered, expected, "starting_lives={lives}: full struct mismatch");
    }
    for wc in 1u8..=7 {
        let opts = Options { world_count: wc, ..Default::default() };
        let expected = normalized(opts.clone());
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
        let expected = normalized(opts.clone());
        let recovered = Options::from_flag_key(&opts.to_flag_key()).unwrap();
        assert_eq!(
            recovered.starting_items, items,
            "starting_items={items:?}: round-trip mismatch"
        );
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
    everything.wild_injections = WildChaser::ALL.to_vec();
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
    let expected = normalized(everything.clone());
    let recovered = Options::from_flag_key(&everything.to_flag_key()).unwrap();
    assert_eq!(recovered, expected, "all-fields-flipped: round-trip mismatch");
}

/// Exhaustive guard: every boolean field on `Options` must either be encoded
/// in the flag key (flipping it changes the key) or be a deliberate cosmetic /
/// operational exclusion listed below. Driven by serde so a NEW bool field
/// added to `Options` is automatically covered — if it's dropped from the flag
/// key (the exact bug that hid `anchor_visuals`), this test fails until the
/// author either encodes it or consciously adds it to `NOT_ENCODED`.
#[test]
fn flag_key_encodes_every_bool_option() {
    // The exclusion list lives with the encoder, not here — the destructure in
    // `to_flag_bits` names every field, so the two can't disagree about which
    // ones are deliberately left out.
    use flag_key::NOT_ENCODED;

    let default_key = Options::default().to_flag_key();
    let default_json = serde_json::to_value(Options::default()).unwrap();
    let obj = default_json.as_object().unwrap();

    let mut checked_encoded = 0;
    for (field, value) in obj {
        let Some(b) = value.as_bool() else { continue };

        // Flip this one bool in the serialized form and rebuild Options.
        let mut mutated_json = default_json.clone();
        mutated_json[field] = serde_json::Value::Bool(!b);
        let mutated: Options = serde_json::from_value(mutated_json).unwrap();
        let mutated_key = mutated.to_flag_key();

        if NOT_ENCODED.contains(&field.as_str()) {
            assert_eq!(
                default_key, mutated_key,
                "'{field}' is on the NOT_ENCODED list but flipping it changed the flag key",
            );
        } else {
            assert_ne!(
                default_key, mutated_key,
                "bool option '{field}' does not affect the flag key — encode it in \
                 to_flag_bytes/from_flag_key, or add it to NOT_ENCODED with a reason",
            );
            checked_encoded += 1;
        }
    }

    assert!(checked_encoded > 0, "no encoded bool fields found — serde reflection broke?");
}

// Removed at v29: `flag_key_hammer_vuln_koopalings_distinct_from_hb_encounters`
// was a regression test for a real shipped collision — hammer_vulnerable_koopalings
// shared a bit with the high bit of hb_encounters, and only misbehaved when
// hb_encounters was Wild. Bit positions are now assigned by declaration order in
// `FlagBits`, so no two options can share one; the test would be exercising the
// bitfield crate rather than this repo.

/// Build a key with a valid envelope for an arbitrary version and payload —
/// the shape a build from a different release would produce. Lets the version
/// branches be tested without a time machine.
fn forge_key(version: u8, payload: &[u8]) -> String {
    let mut bytes = vec![version, checksum(version, payload)];
    bytes.extend_from_slice(payload);
    format!("SMB3R-{}", base32_encode(&bytes))
}

/// The version probe backs the web app's three-way rejection message (older /
/// newer / not a flag key). Each case below is one of those branches.
#[test]
fn flag_key_version_probe() {
    let key = Options::default().to_flag_key();
    let payload = &Options::default().to_flag_bytes()[2..].to_vec();
    let current = current_flag_key_version();

    // A key this build made reports this build's version, with or without the
    // prefix, in any case, and with the whitespace a paste tends to carry.
    assert_eq!(flag_key_version_of(&key), Ok(current));
    assert_eq!(flag_key_version_of(key.trim_start_matches("SMB3R-")), Ok(current));
    assert_eq!(flag_key_version_of(&key.to_lowercase()), Ok(current));
    assert_eq!(flag_key_version_of(&format!("  {key}\n")), Ok(current));

    // An older key reads as "older" even though the full decode rejects it.
    let older = forge_key(current - 1, payload);
    assert_eq!(flag_key_version_of(&older), Ok(current - 1));
    assert!(Options::from_flag_key(&older).is_err());

    // A key from a future format that grew past this build's payload capacity.
    // The probe must stay lenient about length or this would read as garbage
    // instead of "newer", and the app would tell the user to check for typos in
    // a perfectly good key. This is the direction that bites today: beta runs
    // ahead of the released app.
    let mut long = payload.clone();
    long.extend_from_slice(&[0x7F; 40]);
    assert_eq!(flag_key_version_of(&forge_key(current + 1, &long)), Ok(current + 1));

    // Not a flag key at all — the app's third branch.
    assert!(flag_key_version_of("").is_err());
    assert!(flag_key_version_of("!!!!").is_err());
    assert!(flag_key_version_of("SMB3R-").is_err());

    // A truncated paste is caught by the checksum, so it lands in "invalid"
    // rather than being announced as a version mismatch and sending the user
    // looking for a build that was never released. Before the checksum existed
    // this needed a special rule about length; now it falls out of the format.
    assert!(flag_key_version_of("SMB3R-ZZZZ").is_err());
    assert!(flag_key_version_of(&key[..key.len() - 4]).is_err());
}

/// Keys made before v29 must be classified as *older*, not as mistyped.
///
/// They carry no checksum, so the envelope check rejects every one of them —
/// and on the day v29 ships, that is every key in circulation. Telling those
/// users to check for a typo would send them looking for something that isn't
/// there, and it is the exact three-way message (#159) this format is supposed
/// to keep honest.
#[test]
fn flag_key_pre_v29_keys_read_as_older() {
    // A real v28 key: the default set, quoted in issue #158's typo measurement.
    let v28 = "SMB3R-3JKWY87RAGA0A00700000";
    assert_eq!(flag_key_version_of(v28), Ok(28));
    assert!(Options::from_flag_key(v28).unwrap_err().contains("version 28"));

    // Synthetic keys across the whole legacy range, in the shape those versions
    // used: 13 bytes, version in byte 0, no checksum.
    for version in 1..=28u8 {
        let mut bytes = [0xA5u8; 13];
        bytes[0] = version;
        let key = format!("SMB3R-{}", base32_encode(&bytes));
        assert_eq!(flag_key_version_of(&key), Ok(version), "v{version} must read as older");
        assert!(Options::from_flag_key(&key).is_err(), "v{version} must not decode");
    }

    // The fallback must not swallow real damage. A v29 key with a mangled body
    // is still 13-plus bytes, but its version byte is 29, so it stays invalid
    // rather than being announced as some older version.
    let mut broken = Options::default().to_flag_bytes();
    let last = broken.len() - 1;
    broken[last] ^= 0xFF;
    assert!(flag_key_version_of(&base32_encode(&broken)).is_err());

    // And a legacy-length run of garbage whose first byte is out of range is
    // still invalid — the probe is a range check, not "trust byte 0".
    let mut garbage = [0x00u8; 13];
    garbage[0] = 200;
    assert!(flag_key_version_of(&base32_encode(&garbage)).is_err());
}

/// Golden fixtures pinning the v29 bit and byte order.
///
/// The layout is generated from declaration order in `FlagBits`, so nothing in
/// the source spells out which bit an option lives in. That makes reordering or
/// resizing a field an easy, silent way to invalidate every key in circulation.
/// These literals are the tripwire: if they change, the wire format changed and
/// `FLAG_KEY_VERSION` has to move with it.
///
/// Regenerate deliberately, never by pasting whatever the test printed.
#[test]
fn flag_key_v29_golden() {
    assert_eq!(current_flag_key_version(), 29, "these fixtures pin v29");

    // Defaults.
    let key = "SMB3R-3PHFKHRF000525AG00X0";
    assert_eq!(Options::default().to_flag_key(), key);
    assert_eq!(
        Options::default().to_flag_bytes(),
        vec![29, 162, 249, 199, 15, 0, 0, 81, 21, 80, 0, 58],
    );
    assert_eq!(Options::from_flag_key(key).unwrap(), normalized(Options::default()));

    // Everything off — the payload is nearly all zeros, so trailing-zero
    // stripping shows up here as a key that is no shorter than the default one
    // (the last non-zero byte is what sets the length, not the option count).
    let all_off = "SMB3R-3Q2000000000000000W0";
    assert_eq!(all_off_options().to_flag_key(), all_off);
    assert_eq!(Options::from_flag_key(all_off).unwrap(), normalized(all_off_options()));

    // A spread of non-default values across every field width: 5-bit item
    // slots, the 3-bit world count, 2-bit enums and the scattered chaser bits.
    let mixed = Options {
        starting_items: vec![ITEM_RANDOM, 5, ITEM_RANDOM_SUIT_ONLY],
        starting_lives: 99,
        world_count: 3,
        wild_injections: WildChaser::ALL.to_vec(),
        eights_are_wild: Tri::Maybe,
        fire_flower: FireFlowerMode::Wild,
        piranha_shuffle: PiranhaMode::Wild,
        hb_encounters: EnemyMode::Wild,
        ..Default::default()
    };
    let mixed_key = "SMB3R-3PTFKHRF020525AGXAFJP40";
    assert_eq!(mixed.to_flag_key(), mixed_key);
    assert_eq!(Options::from_flag_key(mixed_key).unwrap(), normalized(mixed));
}

/// Reserve bits are the whole durability story: adding an option must not
/// invalidate keys already in circulation. A key that stops short of a byte
/// decodes as if that byte were zero, which is exactly what an older key looks
/// like once a new option is appended.
#[test]
fn flag_key_short_key_zero_fills() {
    let full = Options::default().to_flag_bytes();

    // Same key with its all-zero reserve bytes spelled out explicitly: adding
    // trailing zeros must not change what it decodes to, or the encoder's
    // truncation would be lossy.
    let mut padded = full[2..].to_vec();
    padded.extend_from_slice(&[0u8; 8]);
    assert_eq!(
        Options::from_flag_key(&forge_key(FLAG_KEY_VERSION, &padded)).unwrap(),
        Options::from_flag_key(&Options::default().to_flag_key()).unwrap(),
    );

    // And a key that predates the last two bytes of payload: the options living
    // there come back off, everything before them survives.
    let short = &full[2..full.len() - 2];
    let decoded = Options::from_flag_key(&forge_key(FLAG_KEY_VERSION, short)).unwrap();
    assert!(decoded.powerups, "an early option must survive a short key");
    assert_eq!(decoded.ground, EnemyMode::Shuffle);
    // starting_lives/world_count/items live in the truncated tail.
    assert_eq!(decoded.world_count, default_world_count());
}

/// The checksum's reason for existing, measured.
///
/// Before it, a single mistyped character produced a valid key for *different*
/// settings 89.6% of the time (651 mutations of the default key; the only ones
/// caught were those that happened to corrupt the version byte). That is the
/// same failure as issue #158 — a silently different ruleset — but far more
/// likely, since a fumbled paste beats a version skew for frequency.
///
/// Measured at v29 over the same sweep: 605 rejected, 15 harmless (they land on
/// padding bits), **0 silently different**. Run with `--nocapture` to re-read
/// the split.
#[test]
fn flag_key_typos_are_rejected() {
    let key = Options::default().to_flag_key();
    let body = key.strip_prefix("SMB3R-").unwrap();
    let expected = Options::from_flag_key(&key).unwrap();

    let (mut rejected, mut same, mut different) = (0, 0, 0);
    for i in 0..body.len() {
        for &c in CROCKFORD.iter() {
            if body.as_bytes()[i] == c {
                continue;
            }
            let mut mutated: Vec<u8> = body.as_bytes().to_vec();
            mutated[i] = c;
            let candidate = format!("SMB3R-{}", String::from_utf8(mutated).unwrap());
            match Options::from_flag_key(&candidate) {
                Err(_) => rejected += 1,
                Ok(o) if o == expected => same += 1,
                Ok(_) => different += 1,
            }
        }
    }

    let total = rejected + same + different;
    println!(
        "single-character typos: {rejected} rejected, {same} harmless, {different} silently different (of {total})"
    );
    assert_eq!(total, body.len() * 31, "every single-character mutation is tried");
    // A CRC-8 lets through about 1 in 256 by chance. Assert well inside that
    // rather than on the nose so the test pins the property, not the arithmetic.
    assert!(
        different * 100 < total,
        "{different}/{total} single-character typos decoded to a DIFFERENT ruleset \
         ({rejected} rejected, {same} harmless) — the checksum is not doing its job",
    );
}

/// `NOT_ENCODED` is the one hand-maintained list left in the mapping, and the
/// web app's `inFlagKey` markings are checked against what it produces. A stale
/// name in it would quietly drop a real option from that check.
#[test]
fn flag_key_not_encoded_names_are_real_fields() {
    let json = serde_json::to_value(Options::default()).unwrap();
    let fields = json.as_object().unwrap();
    for name in flag_key::NOT_ENCODED {
        assert!(
            fields.contains_key(*name),
            "NOT_ENCODED lists '{name}', which is not an Options field (renamed or removed?)",
        );
    }
    let encoded = flag_key_fields();
    assert_eq!(encoded.len(), fields.len() - flag_key::NOT_ENCODED.len());
    for name in flag_key::NOT_ENCODED {
        assert!(!encoded.contains(&name.to_string()), "'{name}' must not be advertised as encoded");
    }
}

/// Cross-check the hand-written Crockford codec against an independent
/// implementation (`base32`, a dev-dependency — it reaches neither the binary
/// nor the WASM bundle).
///
/// The alphabet layer is the one part of the format that is a published spec
/// rather than our invention, so it can be verified against someone else's
/// reading of that spec instead of only against itself. A golden test proves we
/// still agree with *yesterday's us*; this proves we agree with Crockford.
///
/// Worth knowing before reaching for a crate here: "Crockford base32" crates
/// are **not** interchangeable. `c32` 0.6.1 encodes this same key as
/// `7D2Z73GY000A4AN001T` — 19 characters against our 20 — and swapping to it
/// would silently invalidate every key in circulation. `crockford` 1.2.1 only
/// handles `u64`, which cannot hold a 30-byte payload.
///
/// Two behaviours are deliberately *not* delegated, which is why the codec is
/// still ours: `base32` accepts non-canonical lengths (it decoded a 19-character
/// truncation of the key below to 11 bytes, where we reject it — that check
/// catches a tail character the checksum can miss), and it returns `Option`, so
/// it cannot say *which* character was bad in a message the app shows the user.
#[test]
fn base32_matches_an_independent_crockford_implementation() {
    use base32::Alphabet::Crockford;

    // Every payload width the format can produce, plus the envelope.
    for n in 0..=(2 + 30usize) {
        let data: Vec<u8> = (0..n).map(|i| (i as u8).wrapping_mul(37).wrapping_add(11)).collect();
        let ours = base32_encode(&data);
        assert_eq!(ours, base32::encode(Crockford, &data), "encode differs at {n} bytes");
        assert_eq!(
            base32_decode(&ours).unwrap(),
            base32::decode(Crockford, &ours).unwrap(),
            "decode differs at {n} bytes",
        );
    }

    // Real keys, not just synthetic patterns.
    for opts in [Options::default(), all_off_options(), all_on_options()] {
        let body = opts.to_flag_key();
        let body = body.strip_prefix("SMB3R-").unwrap();
        assert_eq!(
            base32::decode(Crockford, body).unwrap(),
            opts.to_flag_bytes(),
            "an independent decoder disagrees about a real key",
        );
    }

    // Crockford's exclusions. Asserted against the expectation as well as
    // against the crate, so both being wrong the same way still fails.
    for (probe, want_ok) in [
        ("3PHFKHRF000525AG00X0", true),  // canonical
        ("3phfkhrf000525ag00x0", true),  // case-insensitive
        ("3PHFKHRF000525AGU0X0", false), // U is excluded from the alphabet
        ("3PHFKHRF000525AG!0X0", false), // not an alphabet character at all
    ] {
        assert_eq!(base32_decode(probe).is_ok(), want_ok, "'{probe}' decodable?");
        assert_eq!(
            base32_decode(probe).is_ok(),
            base32::decode(Crockford, probe).is_some(),
            "disagreement on whether '{probe}' is decodable",
        );
    }

    // Crockford's ambiguity normalizations: I and L read as 1, O reads as 0.
    // Compared by decoded *bytes*, not just by "both accepted it" — dropping
    // the I/L arm entirely still leaves a decodable string, so an acceptance
    // check alone sails past it (confirmed by mutating the decoder).
    for (probe, canonical) in
        [("IILL", "1111"), ("iIlL", "1111"), ("OOOO", "0000"), ("oO00", "0000"), ("H1JK", "HIJK")]
    {
        let expected = base32_decode(canonical).unwrap();
        assert_eq!(base32_decode(probe).unwrap(), expected, "'{probe}' must read as '{canonical}'");
        assert_eq!(
            base32::decode(Crockford, probe).unwrap(),
            expected,
            "independent decoder disagrees that '{probe}' reads as '{canonical}'",
        );
    }
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
        let decoded = base32_decode(&encoded).unwrap();
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
///
/// Deliberately an exhaustive field-by-field literal (no `..Default::default()`):
/// adding a field to Options must fail to compile here, forcing the flag-key
/// tests to be updated alongside the struct.
fn all_off_options() -> Options {
    Options {
        fire_flower: FireFlowerMode::Off,
        friendlier_levels: false,
        limit_hazards: HazardLimit::Off,
        piranha_shuffle: PiranhaMode::Off,
        powerups: false,
        palettes: false,
        palette_themed: false,
        player_color: None,
        remove_flashing: false,
        world_order: false,
        world_count: 7,
        big_q_blocks: false,
        shuffle_airships: false,
        shuffle_hammer_bros: false,
        disable_autoscroll: false,
        chest_items: false,
        remove_whistles: false,
        more_hammer_rocks: Tri::Off,
        eights_are_wild: Tri::Off,
        antechamber_shuffle: Tri::Off,
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
        bro_battle_timer: false,
        japanese_damage: false,
        infinite_mushroom_houses: false,
        fast_mushroom_house: false,
        faster_tail_speed: false,
        no_game_over_penalty: false,
        faster_frog: false,
        lakitu_stays_down: false,
        shuffle_big_q_rooms: false,
        poison_mushrooms: false,
        modern_powerups: false,
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
        wild_injections: Vec::new(),
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
        limit_hazards: HazardLimit::All,
        friendlier_levels: true,
        piranha_shuffle: PiranhaMode::On,
        powerups: true,
        palettes: false,
        palette_themed: false,
        player_color: None,
        remove_flashing: true,
        world_order: true,
        world_count: 3,
        big_q_blocks: true,
        shuffle_airships: true,
        shuffle_hammer_bros: true,
        disable_autoscroll: true,
        chest_items: true,
        remove_whistles: true,
        more_hammer_rocks: Tri::On,
        eights_are_wild: Tri::On,
        antechamber_shuffle: Tri::On,
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
        bro_battle_timer: true,
        japanese_damage: true,
        infinite_mushroom_houses: true,
        fast_mushroom_house: true,
        faster_tail_speed: true,
        no_game_over_penalty: true,
        faster_frog: true,
        lakitu_stays_down: true,
        shuffle_big_q_rooms: true,
        poison_mushrooms: true,
        modern_powerups: true,
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
        wild_injections: WildChaser::ALL.to_vec(),
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
        let Some(mut rom1) = make_test_rom() else {
            eprintln!("SKIP: requires the ROM, which is not included in the repo");
            return;
        };
        randomize(&mut rom1, seed, options);

        // Run 2 (same seed, same options)
        let Some(mut rom2) = make_test_rom() else {
            eprintln!("SKIP: requires the ROM, which is not included in the repo");
            return;
        };
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
    let Some(mut rom1) = make_test_rom() else {
        eprintln!("SKIP: requires the ROM, which is not included in the repo");
        return;
    };
    let Some(mut rom2) = make_test_rom() else {
        eprintln!("SKIP: requires the ROM, which is not included in the repo");
        return;
    };
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
    let Some(_) = make_test_rom() else {
        eprintln!("SKIP: requires the ROM, which is not included in the repo");
        return;
    };
    let on = Options { more_hammer_rocks: Tri::On, ..test_options() };
    let maybe = Options { more_hammer_rocks: Tri::Maybe, ..test_options() };

    // Capture the byte ranges make_hammer_rocks touches from a known-On run.
    let on_touched: Vec<(usize, Vec<u8>)> = {
        let mut rom = make_test_rom().unwrap();
        randomize(&mut rom, 0, &on);
        rom.write_log()
            .iter()
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
        let matches_on = on_touched
            .iter()
            .all(|(off, bytes)| rom.read_range(*off, bytes.len()) == bytes.as_slice());
        if matches_on {
            saw_on = true;
        } else {
            saw_off = true;
        }
    }
    assert!(
        saw_on && saw_off,
        "more_hammer_rocks=Maybe never exercised both outcomes across 24 seeds \
         (saw_on={saw_on}, saw_off={saw_off})"
    );
}

#[test]
fn write_log_tags_match_enabled_modules() {
    let Some(mut rom) = make_test_rom() else {
        eprintln!("SKIP: requires the ROM, which is not included in the repo");
        return;
    };
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
    assert_eq!(
        decoded.starting_items,
        vec![ITEM_RANDOM, ITEM_RANDOM_NO_WHISTLE, ITEM_RANDOM_SUIT_ONLY]
    );
}

#[test]
fn flag_key_round_trip_mixed_random_and_concrete() {
    let opts = Options { starting_items: vec![ITEM_RANDOM, 3], ..Default::default() };
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
