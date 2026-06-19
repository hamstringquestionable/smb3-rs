use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process;

use smb3_rs::{EnemyMode, FireFlowerMode, Options, Tri, STARTING_LIVES_VALUES};

/// Human-readable label for a tri-state flag in the run summary.
fn tri_str(t: Tri) -> &'static str {
    match t {
        Tri::Off => "off",
        Tri::On => "on",
        Tri::Maybe => "maybe (seed decides)",
    }
}

/// CLI validator for `--starting-lives` — only the four canonical values
/// that map cleanly to the 2-bit flag-key encoding are accepted.
fn parse_starting_lives(s: &str) -> Result<u8, String> {
    let n: u8 = s.parse().map_err(|e: std::num::ParseIntError| e.to_string())?;
    if STARTING_LIVES_VALUES.contains(&n) {
        Ok(n)
    } else {
        Err(format!(
            "must be one of {} (got {})",
            STARTING_LIVES_VALUES
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(", "),
            n
        ))
    }
}

#[derive(Parser)]
#[command(name = "smb3-rs", version, about = "Super Mario Bros. 3 Randomizer")]
struct Cli {
    /// Path to the SMB3 ROM file (user must provide their own)
    rom: PathBuf,

    /// Random seed (default: random)
    #[arg(long)]
    seed: Option<u64>,

    /// Output file path (default: smb3-rs_<seed>.ips or .nes)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Output a patched ROM instead of an IPS patch
    #[arg(long)]
    patched_rom: bool,

    /// Disable power-up randomization
    #[arg(long)]
    no_powerups: bool,

    /// Disable palette randomization
    #[arg(long)]
    no_palettes: bool,

    /// Use themed per-tileset palette randomization instead of character-only mode.
    /// Cosmetic; does not affect ROM content and is not encoded in the flag key.
    #[arg(long)]
    themed_palettes: bool,

    /// Enable world order randomization
    #[arg(long)]
    world_order: bool,

    /// Number of worlds before Dark Land (1-7, default 7; requires --world-order)
    #[arg(long, default_value_t = 7, value_parser = clap::value_parser!(u8).range(1..=7))]
    world_count: u8,

    /// Enable Big ? Block randomization
    #[arg(long)]
    big_q_blocks: bool,

    /// Keep autoscrollers enabled (they are disabled by default)
    #[arg(long)]
    keep_autoscroll: bool,

    /// Disable chest/reward item randomization
    #[arg(long)]
    no_chest_items: bool,

    /// Keep warp whistles (they are removed by default)
    #[arg(long)]
    keep_whistles: bool,

    /// Shuffle pipe endpoint positions during the overworld rebuild
    #[arg(long)]
    shuffle_pipes: bool,

    /// Disable pipe shuffle (on by default)
    #[arg(long)]
    no_shuffle_pipes: bool,

    /// Shuffle airship levels across worlds
    #[arg(long)]
    shuffle_airships: bool,

    /// Disable airship shuffle (on by default)
    #[arg(long)]
    no_shuffle_airships: bool,

    /// Keep path-blocking rocks (W2 secret path, W3 boat dock, W4 pipe shortcut)
    #[arg(long)]
    keep_rocks: bool,

    /// Convert the W1 (6,5) blocking decoration into a hammer-breakable rock:
    /// off, on, or maybe (the seed decides, hidden from the flag key). Default: off.
    #[arg(long, default_value = "off")]
    w1_hammer_rock: String,

    /// Disable card speed clear (one-of-each skips cutscene, on by default)
    #[arg(long)]
    no_card_speed_clear: bool,

    /// Keep N-card (N-Spade) panels on the overworld map (removed by default)
    #[arg(long)]
    keep_n_cards: bool,

    /// Keep the wand falling cutscene after defeating a Koopaling (skipped by default)
    #[arg(long)]
    keep_wand_cutscene: bool,

    /// Keep vanilla boss hitboxes (adjusted by default for easier hits on Bowser/Koopalings)
    #[arg(long)]
    keep_boss_hitboxes: bool,

    /// Keep vanilla Koopaling stomp counts (3 hits each; randomized 1–5 per Koopaling by default)
    #[arg(long)]
    keep_koopaling_stomps: bool,

    /// Make Koopalings vulnerable to thrown hammers (off by default)
    #[arg(long)]
    hammer_vulnerable_koopalings: bool,

    /// Randomize which Koopaling appears in each world (off by default)
    #[arg(long)]
    random_koopalings: bool,

    /// Hammer item also breaks fortress lock tiles on the overworld map:
    /// off, on, or maybe (the seed decides, hidden from the flag key). Default: off.
    #[arg(long, default_value = "off")]
    hammer_breaks_locks: String,

    /// Hammer item also breaks water gap (bridge) tiles on the overworld map:
    /// off, on, or maybe (the seed decides, hidden from the flag key). Default: off.
    #[arg(long, default_value = "off")]
    hammer_breaks_bridges: String,

    /// Angry Sun begins swooping immediately on spawn (MaCobra52's "Early Sun" patch)
    #[arg(long)]
    early_sun: bool,

    /// Gate wandering Hammer Bros' overworld movement so they roam less aggressively ("Limit Bro Movement" patch)
    #[arg(long)]
    limit_bro_movement: bool,

    /// Damage drops to Small Mario or kills outright instead of demoting tier-by-tier (MaCobra52's "Japanese damage system" patch)
    #[arg(long)]
    japanese_damage: bool,

    /// Toad / Mushroom Houses can be visited any number of times (MaCobra52's "Infinite use Mushroom Houses" patch)
    #[arg(long)]
    infinite_mushroom_houses: bool,

    /// Skip the entry-input-lock and shorten the exit transition for Toad / Mushroom Houses (MaCobra52)
    #[arg(long)]
    fast_mushroom_house: bool,

    /// Reduce tail-swipe slowdown; bundles a slight flight-time cut and 7-6 wall adjustment so level design holds (MaCobra52)
    #[arg(long)]
    faster_tail_speed: bool,

    /// Game Over no longer wipes reserve inventory, world map progress, or card state (MaCobra52)
    #[arg(long)]
    no_game_over_penalty: bool,

    /// Speed up Frog-Suit swimming and running ("Faster Frog", tail-attack-while-swimming compatible)
    #[arg(long)]
    faster_frog: bool,

    /// Random Fire Flower: in-level Fire Flowers grant a position-derived power
    /// state instead of always Fire — off, on, or wild (default: off).
    /// `on` = Fire/Frog/Tanooki/Hammer; `wild` also allows Small/Big.
    #[arg(long, default_value = "off")]
    fire_flower: String,

    /// Disable spade-game shuffle (on by default; off keeps vanilla spade positions)
    #[arg(long)]
    no_shuffle_spade_games: bool,

    /// Disable Toad House shuffle (on by default; off keeps vanilla Toad House positions)
    #[arg(long)]
    no_shuffle_toad_houses: bool,

    /// Disable hand-trap level slots (on by default; ~10% of levels become visible 0xE6 hand-traps that grab the player on arrival)
    #[arg(long)]
    no_hands_levels: bool,

    /// Troll-pipe level slots (one regular level per world W2-W8 disguised as a
    /// pipe tile): off, on, or maybe (the seed decides, hidden from the flag
    /// key). Default: on.
    #[arg(long, default_value = "on")]
    troll_pipes: String,

    /// Include ~9 unreferenced beta levels in the overworld shuffle pool (off by default)
    #[arg(long)]
    include_beta_stages: bool,

    /// For each W1-W7, independently coin-flip to swap Mario's start tile with
    /// the airship tile. W8 is never swapped. Off by default.
    #[arg(long)]
    swap_start_airship: bool,

    /// Cosmetic: every inventory item displays as the Anchor sprite while
    /// keeping its original behavior. Covers the world-map reserve grid,
    /// Toad House chests, in-level treasure boxes, and the Princess letter
    /// cutscene.
    #[arg(long)]
    anchor_visuals: bool,

    /// Ground enemies: off, shuffle, or wild (default: shuffle)
    #[arg(long, default_value = "shuffle")]
    ground: String,

    /// Shell enemies: off, shuffle, or wild (default: shuffle)
    #[arg(long, default_value = "shuffle")]
    shell: String,

    /// Flying enemies: off, shuffle, or wild (default: shuffle)
    #[arg(long, default_value = "shuffle")]
    flying: String,

    /// Piranha plant variants: off, shuffle, or wild (default: shuffle)
    #[arg(long, default_value = "shuffle")]
    piranhas: String,

    /// Ghost house enemies: off, shuffle, or wild (default: shuffle)
    #[arg(long, default_value = "shuffle")]
    ghosts: String,

    /// Thwomp variants: off, shuffle, or wild (default: off)
    #[arg(long, default_value = "off")]
    thwomps: String,

    /// Rotodisc variants: off, shuffle, or wild (default: off)
    #[arg(long, default_value = "off")]
    rotodiscs: String,

    /// Cannon fire variants: off, shuffle, or wild (default: off)
    #[arg(long, default_value = "off")]
    cannons: String,

    /// Water enemies: off, shuffle, or wild (default: shuffle)
    #[arg(long, default_value = "shuffle")]
    water: String,

    /// Hammer/Boomerang/Fire Bros: off, shuffle, or wild (default: shuffle)
    #[arg(long, default_value = "shuffle")]
    bros: String,

    /// HB encounter segments: off, shuffle, or wild (default: off)
    #[arg(long, default_value = "off")]
    hb_encounters: String,

    /// Inject Lakitu/Angry Sun/Boss Bass into ~15% of segments
    #[arg(long)]
    wild_injections: bool,

    /// Disable airship lock (anchor effect always on by default, use this flag to disable)
    #[arg(long)]
    no_airship_lock: bool,
    /// Set starting lives. Must be one of 1, 5, 20, 99 (default: 5).
    #[arg(long, default_value_t = 5, value_parser = parse_starting_lives)]
    starting_lives: u8,

    /// Start with up to 3 items in inventory (comma-separated names)
    /// Valid: mushroom, fire, leaf, frog, tanooki, hammer-suit, cloud, p-wing, star, anchor, hammer, whistle, music-box, random, random-no-whistle, random-suit-only
    #[arg(long, value_delimiter = ',')]
    starting_items: Vec<String>,

    /// Apply options from a flag key (e.g. SMB3R-3GFR0P...). Overrides all other flag options.
    #[arg(long)]
    flags: Option<String>,

    /// Apply a sprite IPS patch to the ROM before randomizing
    #[arg(long)]
    sprite_patch: Option<PathBuf>,

    /// Apply the bundled Super Toad (Blue) sprite swap by JosueCr4ft.
    /// Source: https://mfgg.net/index.php?act=resdb&param=02&c=7&id=38435
    #[arg(long)]
    toad: bool,

    /// Dump the write log to a file (shows every ROM byte changed, grouped by module)
    #[arg(long)]
    write_log: Option<PathBuf>,

    /// Skip the SMB3 (USA) header / page-count / size checks so modded or
    /// translated ROMs can be loaded. The title-screen seed hash is also
    /// skipped, since its hooks assume the vanilla ROM layout.
    #[arg(long)]
    skip_rom_validation: bool,
}

fn main() {
    let cli = Cli::parse();

    let rom_data = match fs::read(&cli.rom) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("Error reading ROM: {e}");
            process::exit(1);
        }
    };

    let seed = cli.seed.unwrap_or_else(rand::random);

    fn parse_enemy_mode(s: &str, name: &str) -> EnemyMode {
        match s {
            "off" => EnemyMode::Off,
            "shuffle" => EnemyMode::Shuffle,
            "wild" => EnemyMode::Wild,
            other => {
                eprintln!("Invalid --{name} value: {other}");
                eprintln!("Valid values: off, shuffle, wild");
                process::exit(1);
            }
        }
    }

    fn parse_fire_flower(s: &str) -> FireFlowerMode {
        match s {
            "off" => FireFlowerMode::Off,
            "on" => FireFlowerMode::On,
            "wild" => FireFlowerMode::Wild,
            other => {
                eprintln!("Invalid --fire-flower value: {other}");
                eprintln!("Valid values: off, on, wild");
                process::exit(1);
            }
        }
    }

    fn parse_tri(s: &str, name: &str) -> Tri {
        match s {
            "off" => Tri::Off,
            "on" => Tri::On,
            "maybe" => Tri::Maybe,
            other => {
                eprintln!("Invalid --{name} value: {other}");
                eprintln!("Valid values: off, on, maybe");
                process::exit(1);
            }
        }
    }

    let starting_items: Vec<u8> = cli.starting_items.iter().map(|name| {
        match name.to_lowercase().as_str() {
            "mushroom" => 0x01,
            "fire" | "fire-flower" | "fireflower" => 0x02,
            "leaf" => 0x03,
            "frog" | "frog-suit" => 0x04,
            "tanooki" | "tanooki-suit" => 0x05,
            "hammer-suit" | "hammersuit" => 0x06,
            "cloud" => 0x07,
            "p-wing" | "pwing" => 0x08,
            "star" | "starman" => 0x09,
            "anchor" => 0x0A,
            "hammer" => 0x0B,
            "whistle" => 0x0C,
            "music-box" | "musicbox" => 0x0D,
            "random" => 0x0E,
            "random-no-whistle" => 0x0F,
            "random-suit-only" | "random-suit" => 0x10,
            other => {
                eprintln!("Unknown item: {other}");
                eprintln!("Valid: mushroom, fire, leaf, frog, tanooki, hammer-suit, cloud, p-wing, star, anchor, hammer, whistle, music-box, random, random-no-whistle, random-suit-only");
                process::exit(1);
            }
        }
    }).collect();
    if starting_items.len() > 3 {
        eprintln!("At most 3 starting items allowed (got {})", starting_items.len());
        process::exit(1);
    }

    let options = if let Some(ref flag_key) = cli.flags {
        match Options::from_flag_key(flag_key) {
            Ok(mut opts) => {
                // palette_themed is cosmetic and deliberately not encoded in the
                // flag key, so --themed-palettes must overlay on top of a decoded key.
                if cli.themed_palettes {
                    opts.palette_themed = true;
                }
                // Same for skip_rom_validation — a property of the input ROM.
                if cli.skip_rom_validation {
                    opts.skip_rom_validation = true;
                }
                opts
            }
            Err(e) => {
                eprintln!("Invalid --flags value: {e}");
                process::exit(1);
            }
        }
    } else {
        Options {
            powerups: !cli.no_powerups,
            palettes: !cli.no_palettes,
            palette_themed: cli.themed_palettes,
            world_order: cli.world_order,
            world_count: cli.world_count,
            big_q_blocks: cli.big_q_blocks,
            shuffle_pipes: !cli.no_shuffle_pipes,
            shuffle_airships: !cli.no_shuffle_airships,
            disable_autoscroll: !cli.keep_autoscroll,
            chest_items: !cli.no_chest_items,
            remove_whistles: !cli.keep_whistles,
            remove_rocks: !cli.keep_rocks,
            w1_hammer_rock: parse_tri(&cli.w1_hammer_rock, "w1-hammer-rock"),
            card_speed_clear: !cli.no_card_speed_clear,
            remove_n_cards: !cli.keep_n_cards,
            skip_wand_cutscene: !cli.keep_wand_cutscene,
            adjust_boss_hitboxes: !cli.keep_boss_hitboxes,
            koopaling_hits: !cli.keep_koopaling_stomps,
            hammer_vulnerable_koopalings: cli.hammer_vulnerable_koopalings,
            random_koopalings: cli.random_koopalings,
            hammer_breaks_locks: parse_tri(&cli.hammer_breaks_locks, "hammer-breaks-locks"),
            hammer_breaks_bridges: parse_tri(&cli.hammer_breaks_bridges, "hammer-breaks-bridges"),
            early_sun: cli.early_sun,
            limit_bro_movement: cli.limit_bro_movement,
            japanese_damage: cli.japanese_damage,
            infinite_mushroom_houses: cli.infinite_mushroom_houses,
            fast_mushroom_house: cli.fast_mushroom_house,
            faster_tail_speed: cli.faster_tail_speed,
            no_game_over_penalty: cli.no_game_over_penalty,
            faster_frog: cli.faster_frog,
            fire_flower: parse_fire_flower(&cli.fire_flower),
            shuffle_spade_games: !cli.no_shuffle_spade_games,
            shuffle_toad_houses: !cli.no_shuffle_toad_houses,
            hands_levels: !cli.no_hands_levels,
            troll_pipes: parse_tri(&cli.troll_pipes, "troll-pipes"),
            include_beta_stages: cli.include_beta_stages,
            swap_start_airship: cli.swap_start_airship,
            anchor_visuals: cli.anchor_visuals,
            airship_lock: !cli.no_airship_lock,
            ground: parse_enemy_mode(&cli.ground, "ground"),
            shell: parse_enemy_mode(&cli.shell, "shell"),
            flying: parse_enemy_mode(&cli.flying, "flying"),
            piranhas: parse_enemy_mode(&cli.piranhas, "piranhas"),
            ghosts: parse_enemy_mode(&cli.ghosts, "ghosts"),
            thwomps: parse_enemy_mode(&cli.thwomps, "thwomps"),
            rotodiscs: parse_enemy_mode(&cli.rotodiscs, "rotodiscs"),
            cannons: parse_enemy_mode(&cli.cannons, "cannons"),
            water: parse_enemy_mode(&cli.water, "water"),
            bros: parse_enemy_mode(&cli.bros, "bros"),
            hb_encounters: parse_enemy_mode(&cli.hb_encounters, "hb-encounters"),
            wild_injections: cli.wild_injections,
            starting_lives: cli.starting_lives,
            starting_items: starting_items.clone(),
            skip_rom_validation: cli.skip_rom_validation,
        }
    };

    let ext = if cli.patched_rom { "nes" } else { "ips" };
    let output_path = cli
        .output
        .unwrap_or_else(|| PathBuf::from(format!("smb3-rs_{seed}.{ext}")));

    eprintln!("SMB3 Randomizer v{}", env!("CARGO_PKG_VERSION"));
    eprintln!("  Seed: {seed}");
    eprintln!("  Flags: {}", options.to_flag_key());
    eprintln!("  Powerups: {}", if options.powerups { "on" } else { "off" });
    eprintln!("  Palettes: {}", match (options.palettes, options.palette_themed) {
        (false, _)   => "off",
        (true, false) => "characters",
        (true, true)  => "themed",
    });
    eprintln!("  Enemies:  {}", if options.any_enemies_active() { "on" } else { "off" });
    eprintln!("  World order: {}", if options.world_order { "on" } else { "off" });
    if options.world_order && options.world_count < 7 {
        eprintln!("  World count: {}", options.world_count);
    }
    eprintln!("  Big ? Blocks: {}", if options.big_q_blocks { "on" } else { "off" });
    eprintln!("  Starting Lives: {}", options.starting_lives);
    eprintln!("  Pipe shuffle: {}", if options.shuffle_pipes { "on" } else { "off" });
    eprintln!("  Airship shuffle: {}", if options.shuffle_airships { "on" } else { "off" });
    eprintln!("  Autoscroll: {}", if options.disable_autoscroll { "disabled" } else { "enabled" });
    eprintln!("  Chest items: {}", if options.chest_items { "on" } else { "off" });
    eprintln!("  Warp whistles: {}", if options.remove_whistles { "removed" } else { "kept" });
    eprintln!("  Remove rocks: {}", if options.remove_rocks { "on" } else { "off" });
    eprintln!("  W1 hammer rock: {}", tri_str(options.w1_hammer_rock));
    eprintln!("  Random fire flower: {}", match options.fire_flower {
        FireFlowerMode::Off => "off",
        FireFlowerMode::On => "on",
        FireFlowerMode::Wild => "wild",
    });
    eprintln!("  Airship lock: {}", if options.airship_lock { "on" } else { "off" });
    if !options.starting_items.is_empty() {
        let item_names: Vec<&str> = options.starting_items.iter().map(|&id| match id {
            0x01 => "Mushroom", 0x02 => "Fire Flower", 0x03 => "Super Leaf",
            0x04 => "Frog Suit", 0x05 => "Tanooki Suit", 0x06 => "Hammer Suit",
            0x07 => "Cloud", 0x08 => "P-Wing", 0x09 => "Starman",
            0x0A => "Anchor", 0x0B => "Hammer", 0x0C => "Whistle", 0x0D => "Music Box",
            0x0E => "Random", 0x0F => "Random (No Whistle)", 0x10 => "Random (Suit Only)",
            _ => "?",
        }).collect();
        eprintln!("  Starting items: {}", item_names.join(", "));
    }
    eprintln!("  Output:   {}", output_path.display());

    // Apply sprite patch before randomization so randomizer writes take priority.
    // --toad applies a bundled IPS first; --sprite-patch layers on top of that.
    // Keep the pristine input bytes so the final IPS diff includes the visual
    // swap (otherwise the .ips file would only contain the randomization delta
    // relative to a visual-patched ROM).
    let pristine_input = rom_data.clone();
    const TOAD_IPS: &[u8] = include_bytes!("../web/visual-patches/super-toad-josuecr4ft.ips");
    let rom_data = if cli.toad {
        match smb3_rs::ips::apply_ips_patch(&rom_data, TOAD_IPS) {
            Ok(patched) => {
                eprintln!("  Sprite swap: Super Toad (Blue) by JosueCr4ft");
                eprintln!("               https://mfgg.net/index.php?act=resdb&param=02&c=7&id=38435");
                patched
            }
            Err(e) => {
                eprintln!("Error applying bundled Toad swap: {e}");
                process::exit(1);
            }
        }
    } else {
        rom_data
    };
    let rom_data = if let Some(ref patch_path) = cli.sprite_patch {
        let patch_data = match fs::read(patch_path) {
            Ok(data) => data,
            Err(e) => {
                eprintln!("Error reading sprite patch: {e}");
                process::exit(1);
            }
        };
        match smb3_rs::ips::apply_ips_patch(&rom_data, &patch_data) {
            Ok(patched) => {
                eprintln!("  Sprite patch: {}", patch_path.display());
                patched
            }
            Err(e) => {
                eprintln!("Error applying sprite patch: {e}");
                process::exit(1);
            }
        }
    } else {
        rom_data
    };

    let rom = match smb3_rs::randomize_rom(&rom_data, seed, &options, None) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error: {e}");
            process::exit(1);
        }
    };

    // Dump write log and collision report if requested
    if let Some(ref log_path) = cli.write_log {
        let mut log = rom.format_write_log();

        let collisions = rom.find_collisions();
        if !collisions.is_empty() {
            log.push_str(&format!("\n--- {} write collision(s) ---\n", collisions.len()));
            for (off, tag1, tag2) in &collisions {
                log.push_str(&format!("  0x{off:05X}: {tag1} vs {tag2}\n"));
            }
        }

        if let Err(e) = fs::write(log_path, &log) {
            eprintln!("Error writing log: {e}");
        } else {
            eprintln!("  Write log: {}", log_path.display());
        }
    }

    let output_data = if cli.patched_rom {
        rom.output_bytes().to_vec()
    } else {
        // Diff against the pristine input (pre-visual-patch) so the IPS
        // is self-contained. When the input is unheadered, output_bytes()
        // strips the synthetic header back off, matching pristine_input.
        smb3_rs::ips::build_ips_patch(&pristine_input, rom.output_bytes())
    };

    if let Err(e) = fs::write(&output_path, &output_data) {
        eprintln!("Error writing output: {e}");
        process::exit(1);
    }
    eprintln!("Done! Wrote {} bytes to {}", output_data.len(), output_path.display());
}
