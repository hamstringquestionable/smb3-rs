use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process;

use smb3r::Options;

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

    let options = Options {
        powerups: !cli.no_powerups,
        palettes: !cli.no_palettes,
        enemies: cli.enemies,
        world_order: cli.world_order,
        big_q_blocks: cli.big_q_blocks,
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
