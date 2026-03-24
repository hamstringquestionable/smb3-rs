use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process;

use smb3_rs::{LevelShuffle, Options};

#[derive(Parser)]
#[command(name = "smb3-rs", about = "Super Mario Bros. 3 Randomizer")]
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

    /// Enable enemy randomization (experimental)
    #[arg(long)]
    enemies: bool,

    /// Enable world order randomization
    #[arg(long)]
    world_order: bool,

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

    /// Keep path-blocking rocks (W2 secret path, W3 boat dock)
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

    /// Disable airship lock (anchor effect always on by default, use this flag to disable)
    #[arg(long)]
    no_airship_lock: bool,
    /// Set starting lives (1–99, default: 4)
    #[arg(long, default_value_t = 4)]
    starting_lives: u8,

    /// Apply options from a flag key (e.g. SMB3R-01C79804). Overrides all other flag options.
    #[arg(long)]
    flags: Option<String>,

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
            enemies: cli.enemies,
            world_order: cli.world_order,
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
            airship_lock: !cli.no_airship_lock,
            starting_lives: cli.starting_lives,
        }
    };

    let ext = if cli.patched_rom { "nes" } else { "ips" };
    let output_path = cli
        .output
        .unwrap_or_else(|| PathBuf::from(format!("smb3-rs_{seed}.{ext}")));

    eprintln!("SMB3 Randomizer");
    eprintln!("  Seed: {seed}");
    eprintln!("  Flags: {}", options.to_flag_key());
    eprintln!("  Powerups: {}", if options.powerups { "on" } else { "off" });
    eprintln!("  Palettes: {}", if options.palettes { "on" } else { "off" });
    eprintln!("  Enemies:  {}", if options.enemies { "on" } else { "off" });
    eprintln!("  World order: {}", if options.world_order { "on" } else { "off" });
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
    eprintln!("  Output:   {}", output_path.display());

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
