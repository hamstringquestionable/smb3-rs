use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process;

use smb3r::{FortressRedistribute, LevelShuffle, Options};

#[derive(Parser)]
#[command(name = "smb3r", about = "Super Mario Bros. 3 Randomizer")]
struct Cli {
    /// Path to the SMB3 ROM file (user must provide their own)
    rom: PathBuf,

    /// Random seed (default: random)
    #[arg(long)]
    seed: Option<u64>,

    /// Output file path (default: smb3r_<seed>.ips or .nes)
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

    /// Shuffle fortresses and airships across worlds
    #[arg(long)]
    shuffle_fortresses: bool,

    /// Fortress redistribute: off, intra-world (lock shuffle), or cross-world (redistribute)
    #[arg(long, default_value = "off")]
    fortress_redistribute: String,

    /// Shuffle pipe endpoint positions on overworld maps
    #[arg(long)]
    shuffle_pipes: bool,

    /// Keep W3 drawbridges toggling (they are fixed open by default)
    #[arg(long)]
    keep_drawbridges: bool,

    /// Keep W2 secret path rock (it is removed by default)
    #[arg(long)]
    keep_w2_rock: bool,

    /// Disable airship lock (anchor effect always on by default, use this flag to disable)
    #[arg(long)]
    no_airship_lock: bool,
    /// Set starting lives (1–99, default: 4)
    #[arg(long, default_value_t = 4)]
    starting_lives: u8,

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

    let fortress_redistribute = match cli.fortress_redistribute.as_str() {
        "off" => FortressRedistribute::Off,
        "intra" | "intra-world" | "intra_world" => FortressRedistribute::IntraWorld,
        "cross" | "cross-world" | "cross_world" => FortressRedistribute::CrossWorld,
        other => {
            eprintln!("Invalid --fortress-redistribute value: {other}");
            eprintln!("Valid values: off, intra-world, cross-world");
            process::exit(1);
        }
    };

    let options = Options {
        powerups: !cli.no_powerups,
        palettes: !cli.no_palettes,
        enemies: cli.enemies,
        world_order: cli.world_order,
        big_q_blocks: cli.big_q_blocks,
        level_shuffle,
        disable_autoscroll: !cli.keep_autoscroll,
        chest_items: !cli.no_chest_items,
        remove_whistles: !cli.keep_whistles,
        shuffle_fortresses: cli.shuffle_fortresses,
        fortress_redistribute,
        shuffle_pipes: cli.shuffle_pipes,
        fix_drawbridges: !cli.keep_drawbridges,
        remove_w2_rock: !cli.keep_w2_rock,
        airship_lock: !cli.no_airship_lock,
        starting_lives: cli.starting_lives,
    };

    let ext = if cli.patched_rom { "nes" } else { "ips" };
    let output_path = cli
        .output
        .unwrap_or_else(|| PathBuf::from(format!("smb3r_{seed}.{ext}")));

    eprintln!("SMB3 Randomizer");
    eprintln!("  Seed: {seed}");
    eprintln!("  Powerups: {}", if options.powerups { "on" } else { "off" });
    eprintln!("  Palettes: {}", if options.palettes { "on" } else { "off" });
    eprintln!("  Enemies:  {}", if options.enemies { "on" } else { "off" });
    eprintln!("  World order: {}", if options.world_order { "on" } else { "off" });
    eprintln!("  Big ? Blocks: {}", if options.big_q_blocks { "on" } else { "off" });
    eprintln!("  Starting Lives: {}", options.starting_lives);
    eprintln!("  Level shuffle: {}", match &options.level_shuffle {
        LevelShuffle::Off => "off",
        LevelShuffle::IntraWorld => "intra-world",
        LevelShuffle::CrossWorld => "cross-world",
    });
    eprintln!("  Fortress/airship shuffle: {}", if options.shuffle_fortresses { "on" } else { "off" });
    eprintln!("  Fortress redistribute: {}", match &options.fortress_redistribute {
        FortressRedistribute::Off => "off",
        FortressRedistribute::IntraWorld => "intra-world",
        FortressRedistribute::CrossWorld => "cross-world",
    });
    eprintln!("  Pipe shuffle: {}", if options.shuffle_pipes { "on" } else { "off" });
    eprintln!("  Autoscroll: {}", if options.disable_autoscroll { "disabled" } else { "enabled" });
    eprintln!("  Chest items: {}", if options.chest_items { "on" } else { "off" });
    eprintln!("  Warp whistles: {}", if options.remove_whistles { "removed" } else { "kept" });
    eprintln!("  W3 drawbridges: {}", if options.fix_drawbridges { "fixed open" } else { "toggling" });
    eprintln!("  W2 secret rock: {}", if options.remove_w2_rock { "removed" } else { "kept" });
    eprintln!("  Airship lock: {}", if options.airship_lock { "on" } else { "off" });
    eprintln!("  Output:   {}", output_path.display());

    let result = if cli.patched_rom {
        smb3r::generate_patched_rom(&rom_data, seed, &options)
    } else {
        smb3r::generate_patch(&rom_data, seed, &options)
    };

    match result {
        Ok(output_data) => {
            if let Err(e) = fs::write(&output_path, &output_data) {
                eprintln!("Error writing output: {e}");
                process::exit(1);
            }
            eprintln!("Done! Wrote {} bytes to {}", output_data.len(), output_path.display());
        }
        Err(e) => {
            eprintln!("Error: {e}");
            process::exit(1);
        }
    }
}
