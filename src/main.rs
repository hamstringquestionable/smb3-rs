use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process;

use smb3_rs::{EnemyMode, LevelShuffle, Options};

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

    /// Enable world order randomization
    #[arg(long)]
    world_order: bool,

    /// Number of worlds before Dark Land (1-7, default 7; requires --world-order)
    #[arg(long, default_value_t = 7, value_parser = clap::value_parser!(u8).range(1..=7))]
    world_count: u8,

    /// Enable Big ? Block randomization
    #[arg(long)]
    big_q_blocks: bool,

    /// Shuffle levels: off, intra-world, or cross-world
    #[arg(long, default_value = "off")]
    level_shuffle: String,

    /// Keep autoscrollers enabled (they are disabled by default)
    #[arg(long)]
    keep_autoscroll: bool,

    /// Disable chest/reward item randomization
    #[arg(long)]
    no_chest_items: bool,

    /// Keep warp whistles (they are removed by default)
    #[arg(long)]
    keep_whistles: bool,

    /// Enable overworld map shuffle (rebuilds tile layout, overrides --level-shuffle)
    #[arg(long)]
    map_shuffle: bool,

    /// Disable overworld map shuffle (on by default when no --flags provided... see defaults)
    #[arg(long)]
    no_map_shuffle: bool,

    /// Shuffle pipe endpoint positions (only with --map-shuffle)
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

    /// Keep W3 drawbridges toggling (they are fixed open by default)
    #[arg(long)]
    keep_drawbridges: bool,

    /// Keep path-blocking rocks (W2 secret path, W3 boat dock, W4 pipe shortcut)
    #[arg(long)]
    keep_rocks: bool,

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

    /// Hammer item also breaks fortress lock tiles on the overworld map (off by default)
    #[arg(long)]
    hammer_breaks_locks: bool,

    /// Hammer item also breaks water gap (bridge) tiles on the overworld map (off by default)
    #[arg(long)]
    hammer_breaks_bridges: bool,

    /// Keep spade (card matching) games on the overworld (removed by default to free map slots)
    #[arg(long)]
    keep_spade_games: bool,

    /// Include ~9 unreferenced beta levels in the overworld shuffle pool (off by default)
    #[arg(long)]
    include_beta_stages: bool,

    /// Ground enemies: off, shuffle, or wild (default: shuffle)
    #[arg(long, default_value = "shuffle")]
    ground: String,

    /// Shell enemies: off, shuffle, or wild (default: shuffle)
    #[arg(long, default_value = "shuffle")]
    shell: String,

    /// Flying enemies: off, shuffle, or wild (default: shuffle)
    #[arg(long, default_value = "shuffle")]
    flying: String,

    /// Cheep cheep variants: off, shuffle, or wild (default: shuffle)
    #[arg(long, default_value = "shuffle")]
    cheeps: String,

    /// Bullet Bill variants: off, shuffle, or wild (default: shuffle)
    #[arg(long, default_value = "shuffle")]
    bullet_bills: String,

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
    /// Set starting lives (1–99, default: 4)
    #[arg(long, default_value_t = 4)]
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

    /// Dump the write log to a file (shows every ROM byte changed, grouped by module)
    #[arg(long)]
    write_log: Option<PathBuf>,
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

    let seed = cli.seed.unwrap_or_else(|| rand::random());

    let level_shuffle = match cli.level_shuffle.as_str() {
        "off" => LevelShuffle::Off,
        "intra" | "intra-world" | "intra_world" => LevelShuffle::IntraWorld,
        "cross" | "cross-world" | "cross_world" => LevelShuffle::CrossWorld,
        other => {
            eprintln!("Invalid --level-shuffle value: {other}");
            eprintln!("Valid values: off, intra-world, cross-world");
            process::exit(1);
        }
    };

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
            Ok(opts) => opts,
            Err(e) => {
                eprintln!("Invalid --flags value: {e}");
                process::exit(1);
            }
        }
    } else {
        Options {
            powerups: !cli.no_powerups,
            palettes: !cli.no_palettes,
            world_order: cli.world_order,
            world_count: cli.world_count,
            big_q_blocks: cli.big_q_blocks,
            level_shuffle,
            map_shuffle: if cli.no_map_shuffle { false } else { true },
            shuffle_pipes: if cli.no_shuffle_pipes { false } else { true },
            shuffle_airships: if cli.no_shuffle_airships { false } else { true },
            disable_autoscroll: !cli.keep_autoscroll,
            chest_items: !cli.no_chest_items,
            remove_whistles: !cli.keep_whistles,
            fix_drawbridges: !cli.keep_drawbridges,
            remove_rocks: !cli.keep_rocks,
            card_speed_clear: !cli.no_card_speed_clear,
            remove_n_cards: !cli.keep_n_cards,
            skip_wand_cutscene: !cli.keep_wand_cutscene,
            adjust_boss_hitboxes: !cli.keep_boss_hitboxes,
            koopaling_hits: !cli.keep_koopaling_stomps,
            hammer_vulnerable_koopalings: cli.hammer_vulnerable_koopalings,
            random_koopalings: cli.random_koopalings,
            hammer_breaks_locks: cli.hammer_breaks_locks,
            hammer_breaks_bridges: cli.hammer_breaks_bridges,
            remove_spade_games: !cli.keep_spade_games,
            include_beta_stages: cli.include_beta_stages,
            airship_lock: !cli.no_airship_lock,
            ground: parse_enemy_mode(&cli.ground, "ground"),
            shell: parse_enemy_mode(&cli.shell, "shell"),
            flying: parse_enemy_mode(&cli.flying, "flying"),
            cheeps: parse_enemy_mode(&cli.cheeps, "cheeps"),
            bullet_bills: parse_enemy_mode(&cli.bullet_bills, "bullet-bills"),
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
    eprintln!("  Palettes: {}", if options.palettes { "on" } else { "off" });
    eprintln!("  Enemies:  {}", if options.any_enemies_active() { "on" } else { "off" });
    eprintln!("  World order: {}", if options.world_order { "on" } else { "off" });
    if options.world_order && options.world_count < 7 {
        eprintln!("  World count: {}", options.world_count);
    }
    eprintln!("  Big ? Blocks: {}", if options.big_q_blocks { "on" } else { "off" });
    eprintln!("  Starting Lives: {}", options.starting_lives);
    eprintln!("  Map shuffle: {}", if options.map_shuffle { "on" } else { "off" });
    if !options.map_shuffle {
        eprintln!("  Level shuffle: {}", match &options.level_shuffle {
            LevelShuffle::Off => "off",
            LevelShuffle::IntraWorld => "intra-world",
            LevelShuffle::CrossWorld => "cross-world",
        });
    }
    eprintln!("  Pipe shuffle: {}", if options.shuffle_pipes { "on" } else { "off" });
    eprintln!("  Airship shuffle: {}", if options.shuffle_airships { "on" } else { "off" });
    eprintln!("  Autoscroll: {}", if options.disable_autoscroll { "disabled" } else { "enabled" });
    eprintln!("  Chest items: {}", if options.chest_items { "on" } else { "off" });
    eprintln!("  Warp whistles: {}", if options.remove_whistles { "removed" } else { "kept" });
    eprintln!("  W3 drawbridges: {}", if options.fix_drawbridges { "fixed open" } else { "toggling" });
    eprintln!("  Remove rocks: {}", if options.remove_rocks { "on" } else { "off" });
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

    // Apply sprite patch before randomization so randomizer writes take priority
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

    let rom = match smb3_rs::randomize_rom(&rom_data, seed, &options) {
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
        smb3_rs::ips::build_ips_patch(rom.original_bytes(), rom.output_bytes())
    };

    if let Err(e) = fs::write(&output_path, &output_data) {
        eprintln!("Error writing output: {e}");
        process::exit(1);
    }
    eprintln!("Done! Wrote {} bytes to {}", output_data.len(), output_path.display());
}
