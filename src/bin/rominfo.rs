//! `rominfo` — look things up in a ROM without changing it.
//!
//! Thin CLI over `smb3_rs::rominfo`. All the ROM knowledge lives in the
//! library; this file only parses flags and does file I/O.

use std::fs;
use std::path::PathBuf;
use std::process;

use clap::Parser;

use smb3_rs::rominfo;

const DEFAULT_ROM: &str = "roms/Super Mario Bros. 3 (USA) (Rev 1).nes";

#[derive(Parser)]
#[command(
    name = "rominfo",
    about = "Look up what is in a Super Mario Bros. 3 ROM",
    long_about = "Read-only lookups over a ROM, vanilla or randomized. The Rust \
replacement for tools/rom_map.py.\n\n\
EXAMPLES:\n  \
  rominfo --level 1-1          every area, powerup block and object in 1-1\n  \
  rominfo --level 6F1          fortresses: 6F1 or 6-F1\n  \
  rominfo --list               every name --level accepts"
)]
struct Cli {
    /// ROM to read.
    #[arg(short, long, default_value = DEFAULT_ROM)]
    rom: PathBuf,

    /// Report one level by name (case-insensitive, dashes optional).
    #[arg(long)]
    level: Option<String>,

    /// List every level name.
    #[arg(long)]
    list: bool,
}

fn die(msg: String) -> ! {
    eprintln!("rominfo: {msg}");
    process::exit(1);
}

fn main() {
    let cli = Cli::parse();
    let bytes =
        fs::read(&cli.rom).unwrap_or_else(|e| die(format!("reading {}: {e}", cli.rom.display())));

    if cli.list {
        for line in rominfo::list_levels(&bytes).unwrap_or_else(|e| die(e)) {
            println!("{line}");
        }
    } else if let Some(name) = &cli.level {
        print!("{}", rominfo::level_report(&bytes, name).unwrap_or_else(|e| die(e)));
    } else {
        die("nothing to do: pass --level <name> or --list".into());
    }
}
