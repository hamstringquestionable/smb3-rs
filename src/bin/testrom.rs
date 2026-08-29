//! `testrom` — build a ROM for playtesting.
//!
//! Thin CLI over `smb3_rs::testrom`. All the ROM knowledge lives in the
//! library; this file only parses flags and does file I/O.

use std::fs;
use std::path::PathBuf;
use std::process;

use clap::Parser;

use smb3_rs::Options;
use smb3_rs::testrom::{self, Base, EnemyOverride, Placement, TestRomSpec};

const DEFAULT_ROM: &str = "roms/Super Mario Bros. 3 (USA) (Rev 1).nes";
const DEFAULT_PRACTICE_IPS: &str = "patches/smb3practice_SE.ips";

#[derive(Parser)]
#[command(
    name = "testrom",
    about = "Build a Super Mario Bros. 3 ROM for playtesting",
    long_about = "Build a playtest ROM by composing independent knobs: what levels to \
park on the map, whether the base map is vanilla or randomized, which world to start \
in, and how much of the map is walkable.\n\n\
By default the map is fully open — locks removed, water gaps bridged, and open \
movement patched in so Mario walks over level and fortress tiles without entering \
them. Pass --keep-locks when the locks are what you're testing.\n\n\
EXAMPLES:\n  \
  testrom --place 6F1                    park 6-F1 on tile 1 of World 1\n  \
  testrom --place 6F1 5F1 8B             park three levels on tiles 1, 2, 3\n  \
  testrom --place 3:8B                   park Bowser's Castle on tile 3\n  \
  testrom --place 7A --world 7           test World 7's airship on its home map\n  \
  testrom --randomize --seed 12345       playtest a randomized overworld\n  \
  testrom --randomize --keep-locks       test lock placement on a real seed\n  \
  testrom --place-all 7F1                every numbered level in W1 becomes 7-F1\n  \
  testrom --list                         show every placeable level name"
)]
struct Cli {
    /// Vanilla ROM to build from.
    #[arg(short, long, default_value = DEFAULT_ROM)]
    rom: PathBuf,

    /// Output ROM path.
    #[arg(short, long, default_value = "test_level.nes")]
    output: PathBuf,

    /// Level(s) to park on numbered tiles, in order: "6F1", or "3:8B" to pick
    /// the tile. Names are case-insensitive and dashes are optional.
    #[arg(short, long, num_args = 1..)]
    place: Vec<String>,

    /// Replace every numbered level in the starting world with this one.
    #[arg(long, value_name = "LEVEL")]
    place_all: Option<String>,

    /// List every placeable level name and exit.
    #[arg(long)]
    list: bool,

    /// Include the 9 unreferenced beta stages as placeable names.
    #[arg(long)]
    beta: bool,

    /// Build on a randomized map instead of vanilla. Use when testing the
    /// overworld itself rather than a level's contents.
    #[arg(long)]
    randomize: bool,

    /// Seed for --randomize (default: random).
    #[arg(long)]
    seed: Option<u64>,

    /// Flag key for --randomize, e.g. SMB3R-01FFFD14.
    #[arg(long)]
    flags: Option<String>,

    /// Search seeds until the map satisfies this, e.g. "lock@w8:s2",
    /// "fort@w3>=2", "tile:0x54@w8". Implies --randomize.
    /// Classes: lock, gap, fortress, level, pipe, toadhouse, airship, bowser.
    #[arg(long, value_name = "PREDICATE")]
    require: Option<String>,

    /// How many seeds --require may try before giving up.
    #[arg(long, default_value_t = 500)]
    search: usize,

    /// First seed --require tries. Ascending and reproducible; bump it to get
    /// a different map that still satisfies the predicate.
    #[arg(long, default_value_t = 1)]
    seed_from: u64,

    /// Starting world, 1-8 (default: the ROM's own).
    #[arg(short, long, value_parser = clap::value_parser!(u8).range(1..=8))]
    world: Option<u8>,

    /// Leave lock tiles in place (default: removed).
    #[arg(long)]
    keep_locks: bool,

    /// Leave water gaps in place (default: bridged).
    #[arg(long)]
    keep_gaps: bool,

    /// Apply the always-on gameplay patches (currently stomp fairness) to a
    /// vanilla base. Use when playtesting one of them — a plain vanilla base
    /// does not contain them, so the test silently exercises vanilla instead.
    /// Ignored on a --randomize base, which already has them.
    #[arg(long)]
    patches: bool,

    /// Skip the open-movement patch, so tiles must be entered and cleared.
    #[arg(long)]
    no_walk: bool,

    /// Apply the open-movement records that don't clash with randomizer
    /// patches, instead of refusing. Open movement is then PARTIAL — verify
    /// in-game that you can actually walk where you need to.
    #[arg(long)]
    walk_skip_conflicts: bool,

    /// Start with these inventory items, up to 3, e.g. "hammer,leaf,fire".
    /// Run --list-items for the full set.
    #[arg(long, value_delimiter = ',')]
    starting_items: Vec<String>,

    /// Starting lives (only applied alongside --starting-items).
    #[arg(long, default_value_t = 5)]
    starting_lives: u8,

    /// Let the Hammer item break fortress lock tiles on the map.
    #[arg(long)]
    hammer_locks: bool,

    /// Let the Hammer item break water-gap (bridge) tiles on the map.
    #[arg(long)]
    hammer_bridges: bool,

    /// Overwrite one enemy slot outright, as "PTR:SLOT:ID" in hex, e.g.
    /// "DA0F:1:66" to put a downward water current in the Coin Ship fight's
    /// second slot. Repeatable. Use to see what an object actually does in a
    /// room the randomizer's pools would never put it in.
    #[arg(long, value_name = "PTR:SLOT:ID")]
    set_enemy: Vec<String>,

    /// Open "Unused Level 5" -- the unreferenced test level holding eight Big
    /// [?] Block rooms -- from every Big [?] pipe in the game, landing 5-2's
    /// pipe in the room on this screen (0-7). Screen 3 reproduces 5-2's
    /// vanilla arrival column. Pair with --place 5-2.
    #[arg(long, value_name = "SCREEN", value_parser = clap::value_parser!(u8).range(0..=7))]
    bigq_unused5: Option<u8>,

    /// BG palette (0-7) for those rooms. The level's own 6 is the placeholder
    /// palette -- black and white. Real fortresses use 0, 1, 3 or 4.
    #[arg(long, value_name = "INDEX", default_value_t = 0,
          value_parser = clap::value_parser!(u8).range(0..=7))]
    bigq_palette: u8,

    /// Override where --bigq-unused5 puts the player, as "COL,YIDX". Y index
    /// is into LevelJct_YLHStarts: 0 = row 0, 4 = row 15, 5 = row 20,
    /// 6 = row 23. Use when a room's table entry lands badly.
    #[arg(long, value_name = "COL,YIDX")]
    bigq_aim: Option<String>,

    /// List valid --starting-items names and exit.
    #[arg(long)]
    list_items: bool,

    /// Apply a whole IPS patch to the base before test edits. Repeatable.
    /// Use for the full practice ROM (level select + warp whistles) or
    /// anything in patches/ — unlike the default movement patch, this applies
    /// every record.
    #[arg(long, value_name = "PATH")]
    apply_ips: Vec<PathBuf>,

    /// Stop --apply-ips patches from rewriting the overworld maps. The full
    /// practice patch removes lock tiles as a side effect; this keeps the
    /// map you started with. (Enemy data is not protected.)
    #[arg(long)]
    protect_map: bool,

    /// Practice patch supplying open map movement.
    #[arg(long, default_value = DEFAULT_PRACTICE_IPS)]
    practice_ips: PathBuf,
}

fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("error: {msg}");
    process::exit(1);
}

/// Parse a `--place` item: `"6F1"` or `"3:6F1"` (tile number : level name).
fn parse_placement(spec: &str) -> Result<Placement, String> {
    match spec.split_once(':') {
        Some((slot, level)) => {
            let slot: u8 = slot
                .trim()
                .parse()
                .map_err(|_| format!("bad tile number in {spec:?} (expected e.g. 3:8B)"))?;
            Ok(Placement { level: level.trim().to_string(), slot: Some(slot) })
        }
        None => Ok(Placement { level: spec.trim().to_string(), slot: None }),
    }
}

/// Parse a `--set-enemy` item: `"DA0F:1:66"` — enemy pointer, slot, object ID.
/// Pointer and ID are hex (a leading `0x` is accepted); the slot is decimal.
fn parse_set_enemy(spec: &str) -> Result<EnemyOverride, String> {
    fn hex(s: &str) -> &str {
        s.trim().trim_start_matches("0x").trim_start_matches("0X")
    }
    let bad = || format!("bad --set-enemy {spec:?} (expected PTR:SLOT:ID, e.g. DA0F:1:66)");
    let mut parts = spec.split(':');
    let (p, s, i) = match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(p), Some(s), Some(i), None) => (p, s, i),
        _ => return Err(bad()),
    };
    let enemy_ptr = u16::from_str_radix(hex(p), 16).map_err(|_| bad())?;
    if !(0xC000..=0xDFFF).contains(&enemy_ptr) {
        return Err(format!("enemy pointer ${enemy_ptr:04X} is outside $C000-$DFFF"));
    }
    Ok(EnemyOverride {
        enemy_ptr,
        slot: s.trim().parse().map_err(|_| bad())?,
        id: u8::from_str_radix(hex(i), 16).map_err(|_| bad())?,
    })
}

fn main() {
    let cli = Cli::parse();

    if cli.list_items {
        for &(name, id, display) in smb3_rs::ITEMS {
            println!("{name:<20} 0x{id:02X}  {display}");
        }
        return;
    }

    let vanilla =
        fs::read(&cli.rom).unwrap_or_else(|e| die(format!("reading {}: {e}", cli.rom.display())));

    if cli.list {
        match testrom::list_levels(&vanilla, cli.beta) {
            Ok(names) => {
                for name in names {
                    println!("{name}");
                }
                return;
            }
            Err(e) => die(e),
        }
    }

    // Randomizer options only matter with --randomize; reject the combination
    // that silently does nothing rather than pretending it worked.
    let randomize = cli.randomize || cli.require.is_some();
    if !randomize && (cli.seed.is_some() || cli.flags.is_some()) {
        die("--seed/--flags require --randomize (vanilla base ignores them)");
    }
    if cli.require.is_some() && cli.seed.is_some() {
        die("--require searches for a seed; drop --seed (or use --seed-from)");
    }

    let base = if randomize {
        let options = match &cli.flags {
            Some(key) => Options::from_flag_key(key)
                .unwrap_or_else(|e| die(format!("invalid --flags value: {e}"))),
            None => Options::default(),
        };

        let seed = match &cli.require {
            Some(spec) => {
                let req = testrom::Requirement::parse(spec).unwrap_or_else(|e| die(e));
                eprintln!("searching for {} ...", req.describe());
                match testrom::search_seed(&vanilla, &options, &req, cli.seed_from, cli.search)
                    .unwrap_or_else(|e| die(e))
                {
                    Some(hit) => {
                        for line in &hit.report {
                            eprintln!("  {line}");
                        }
                        eprintln!("  (tried {} seed(s))\n", hit.tried);
                        hit.seed
                    }
                    None => die(format!(
                        "no seed satisfying {} in {} tried from {}\n       \
                         raise --search, change --seed-from, or relax the predicate",
                        req.describe(),
                        cli.search,
                        cli.seed_from
                    )),
                }
            }
            None => cli.seed.unwrap_or_else(rand::random),
        };
        Base::Randomized { seed, options: Box::new(options) }
    } else {
        Base::Vanilla
    };

    let movement_patch = if cli.no_walk {
        None
    } else {
        match fs::read(&cli.practice_ips) {
            Ok(bytes) => Some(bytes),
            Err(e) => die(format!(
                "reading {}: {e}\n       pass --no-walk to build without open movement",
                cli.practice_ips.display()
            )),
        }
    };

    // "COL,YIDX" -> (col, Y-start index). This overrides where --bigq-unused5
    // drops the player, which is how a badly-landing room gets re-aimed.
    let big_q_aim = cli.bigq_aim.as_deref().map(|a| {
        let (c, y) = a
            .split_once(',')
            .unwrap_or_else(|| die(format!("--bigq-aim wants \"COL,YIDX\", got \"{a}\"")));
        let col: u8 = c.trim().parse().unwrap_or_else(|_| die(format!("bad column \"{c}\"")));
        let y_idx: u8 = y.trim().parse().unwrap_or_else(|_| die(format!("bad Y index \"{y}\"")));
        if col > 15 || y_idx > 7 {
            die("--bigq-aim: column is 0-15, Y index 0-7".to_string());
        }
        (col, y_idx)
    });

    let placements: Vec<Placement> =
        cli.place.iter().map(|s| parse_placement(s).unwrap_or_else(|e| die(e))).collect();

    let set_enemies: Vec<EnemyOverride> =
        cli.set_enemy.iter().map(|s| parse_set_enemy(s).unwrap_or_else(|e| die(e))).collect();

    let starting_items: Vec<u8> = cli
        .starting_items
        .iter()
        .map(|name| {
            smb3_rs::item_id(name).unwrap_or_else(|| {
                let valid: Vec<&str> = smb3_rs::ITEMS.iter().map(|&(n, _, _)| n).collect();
                die(format!("unknown item {name:?}\n       valid: {}", valid.join(", ")))
            })
        })
        .collect();

    if starting_items.len() > 3 {
        die(format!(
            "at most 3 starting items (got {}) — the inventory has 3 slots",
            starting_items.len()
        ));
    }

    // Labelled by filename: the label is what the write log and any collision
    // report name, so a second patch clashing with the first is legible.
    let extra_patches: Vec<(String, Vec<u8>)> = cli
        .apply_ips
        .iter()
        .map(|p| {
            let bytes =
                fs::read(p).unwrap_or_else(|e| die(format!("reading {}: {e}", p.display())));
            let label = p
                .file_name()
                .map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned());
            (label, bytes)
        })
        .collect();

    let spec = TestRomSpec {
        base,
        placements,
        place_all: cli.place_all.clone(),
        world: cli.world,
        extra_patches,
        protect_map: cli.protect_map,
        movement_patch,
        always_on_patches: cli.patches,
        walk_skip_conflicts: cli.walk_skip_conflicts,
        remove_locks: !cli.keep_locks,
        remove_gaps: !cli.keep_gaps,
        starting_items,
        starting_lives: cli.starting_lives,
        hammer_breaks_locks: cli.hammer_locks,
        hammer_breaks_bridges: cli.hammer_bridges,
        include_beta: cli.beta,
        big_q_unused5: cli.bigq_unused5,
        big_q_palette: Some(cli.bigq_palette),
        big_q_aim,
        set_enemies,
    };

    let built = testrom::build(&vanilla, &spec).unwrap_or_else(|e| die(e));

    fs::write(&cli.output, &built.bytes)
        .unwrap_or_else(|e| die(format!("writing {}: {e}", cli.output.display())));

    for line in &built.report {
        eprintln!("  {line}");
    }
    eprintln!("\nWrote {}", cli.output.display());
}
