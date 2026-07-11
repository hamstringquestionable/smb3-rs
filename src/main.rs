use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process;

use smb3_rs::{EnemyMode, FireFlowerMode, Options, PiranhaMode, Tri, STARTING_LIVES_VALUES};

/// Human-readable label for a tri-state flag in the run summary.
fn tri_str(t: Tri) -> &'static str {
    match t {
        Tri::Off => "off",
        Tri::On => "on",
        Tri::Maybe => "maybe (seed decides)",
    }
}

/// Parse a NES color byte from hex ("16", "0x16", "$16") and require it to be
/// chromatic (hue nibble 1-C, value <= 0x3C) so the palette scheme has a hue
/// to anchor on.
fn parse_nes_color(s: &str) -> Result<u8, String> {
    let hex = s.trim_start_matches("0x").trim_start_matches("0X").trim_start_matches('$');
    let n = u8::from_str_radix(hex, 16).map_err(|e| e.to_string())?;
    let hue = n & 0x0F;
    if n <= 0x3C && (1..=0x0C).contains(&hue) {
        Ok(n)
    } else {
        Err(format!(
            "0x{n:02X} is not a chromatic NES color (need value <= 0x3C with low nibble 1-C; grays/blacks can't anchor a color scheme)"
        ))
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

/// clap value parser for the per-class enemy flags (off/shuffle/wild).
fn parse_enemy_mode(s: &str) -> Result<EnemyMode, String> {
    match s {
        "off" => Ok(EnemyMode::Off),
        "shuffle" => Ok(EnemyMode::Shuffle),
        "wild" => Ok(EnemyMode::Wild),
        _ => Err("valid values: off, shuffle, wild".to_string()),
    }
}

/// clap value parser for `--fire-flower` (off/on/wild).
fn parse_fire_flower(s: &str) -> Result<FireFlowerMode, String> {
    match s {
        "off" => Ok(FireFlowerMode::Off),
        "on" => Ok(FireFlowerMode::On),
        "wild" => Ok(FireFlowerMode::Wild),
        _ => Err("valid values: off, on, wild".to_string()),
    }
}

/// clap value parser for `--piranha-shuffle` (off/on/wild).
fn parse_piranha(s: &str) -> Result<PiranhaMode, String> {
    match s {
        "off" => Ok(PiranhaMode::Off),
        "on" => Ok(PiranhaMode::On),
        "wild" => Ok(PiranhaMode::Wild),
        _ => Err("valid values: off, on, wild".to_string()),
    }
}

/// clap value parser for the tri-state flags (off/on/maybe).
fn parse_tri(s: &str) -> Result<Tri, String> {
    match s {
        "off" => Ok(Tri::Off),
        "on" => Ok(Tri::On),
        "maybe" => Ok(Tri::Maybe),
        _ => Err("valid values: off, on, maybe".to_string()),
    }
}

/// Inventory items: (CLI name, item ID, display name). Single source for the
/// `--starting-items` parser and the run-summary printer; extra spellings are
/// handled as aliases in `item_id`.
const ITEMS: &[(&str, u8, &str)] = &[
    ("mushroom", 0x01, "Mushroom"),
    ("fire", 0x02, "Fire Flower"),
    ("leaf", 0x03, "Super Leaf"),
    ("frog", 0x04, "Frog Suit"),
    ("tanooki", 0x05, "Tanooki Suit"),
    ("hammer-suit", 0x06, "Hammer Suit"),
    ("cloud", 0x07, "Cloud"),
    ("p-wing", 0x08, "P-Wing"),
    ("star", 0x09, "Starman"),
    ("anchor", 0x0A, "Anchor"),
    ("hammer", 0x0B, "Hammer"),
    ("whistle", 0x0C, "Whistle"),
    ("music-box", 0x0D, "Music Box"),
    ("random", 0x0E, "Random"),
    ("random-no-whistle", 0x0F, "Random (No Whistle)"),
    ("random-suit-only", 0x10, "Random (Suit Only)"),
];

/// Look up a starting-item ID by CLI name (case-insensitive, with aliases).
fn item_id(name: &str) -> Option<u8> {
    let lower = name.to_lowercase();
    let canonical = match lower.as_str() {
        "fire-flower" | "fireflower" => "fire",
        "frog-suit" => "frog",
        "tanooki-suit" => "tanooki",
        "hammersuit" => "hammer-suit",
        "pwing" => "p-wing",
        "starman" => "star",
        "musicbox" => "music-box",
        "random-suit" => "random-suit-only",
        other => other,
    };
    ITEMS.iter().find(|&&(n, _, _)| n == canonical).map(|&(_, id, _)| id)
}

/// Display name for a starting-item ID in the run summary.
fn item_display_name(id: u8) -> &'static str {
    ITEMS.iter().find(|&&(_, i, _)| i == id).map_or("?", |&(_, _, n)| n)
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

    /// Disable player color randomization (keep the brothers' vanilla outfits)
    #[arg(long)]
    no_palettes: bool,

    /// Recolor levels, enemies, and world maps with a random theme (world
    /// colors). Independent of player colors.
    /// Cosmetic; not encoded in the flag key.
    #[arg(long)]
    themed_palettes: bool,

    /// Anchor the character palettes on a chosen NES color (hex, e.g. "16" or
    /// "0x12"): Mario wears it, Luigi and the suits are derived from it.
    /// Must be a chromatic NES color (hue nibble 1-C, value <= 0x3C).
    /// Cosmetic; requires palettes enabled. Default: random colors.
    #[arg(long, value_parser = parse_nes_color)]
    player_color: Option<u8>,

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

    /// Disable pipe shuffle (on by default)
    #[arg(long)]
    no_shuffle_pipes: bool,

    /// Disable airship shuffle (on by default)
    #[arg(long)]
    no_shuffle_airships: bool,

    /// Disable Hammer Bro location shuffle (on by default)
    #[arg(long)]
    no_shuffle_hammer_bros: bool,

    /// Add extra hammer-breakable rocks (W1 6,5 and W8 3,37 decorations):
    /// off, on, or maybe (the seed decides, hidden from the flag key). Default: off.
    #[arg(long, default_value = "off", value_parser = parse_tri)]
    more_hammer_rocks: Tri,

    /// 8s are Wild: enable the W8 Dark World canoe (screen 0) and extra paths
    /// (screen 2): off, on, or maybe (the seed decides, hidden from the flag
    /// key). Default: off.
    #[arg(long, default_value = "off", value_parser = parse_tri)]
    eights_are_wild: Tri,

    /// Antechamber shuffle: the twelve levels that open with an entry area
    /// piping into the level's interior (2-Pyr, 4-3, 5-2, 5-3, 6-5, 6-6, 6-9,
    /// 7-1, 7-4, 7-5, 7-6, 7-7) get their interiors randomly permuted, so one
    /// level's entry pipe can drop into another's interior: off, on, or maybe
    /// (the seed decides, hidden from the flag key). Default: off.
    #[arg(long, default_value = "off", value_parser = parse_tri)]
    antechamber_shuffle: Tri,

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

    /// Keep vanilla Boom-Boom stomp counts (3 hits each; randomized 1–5 per fortress by default)
    #[arg(long)]
    keep_boomboom_stomps: bool,

    /// Make Koopalings vulnerable to thrown hammers (off by default)
    #[arg(long)]
    hammer_vulnerable_koopalings: bool,

    /// Randomize which Koopaling appears in each world (off by default)
    #[arg(long)]
    random_koopalings: bool,

    /// Hammer item also breaks fortress lock tiles on the overworld map:
    /// off, on, or maybe (the seed decides, hidden from the flag key). Default: off.
    #[arg(long, default_value = "off", value_parser = parse_tri)]
    hammer_breaks_locks: Tri,

    /// Hammer item also breaks water gap (bridge) tiles on the overworld map:
    /// off, on, or maybe (the seed decides, hidden from the flag key). Default: off.
    #[arg(long, default_value = "off", value_parser = parse_tri)]
    hammer_breaks_bridges: Tri,

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
    #[arg(long, default_value = "off", value_parser = parse_fire_flower)]
    fire_flower: FireFlowerMode,

    /// Piranha shuffle: release the two W7 piranha plant levels into the level
    /// pool — off, on, or wild (default: off). `on` = their plant sprites follow
    /// them; `wild` = plants scatter onto ~1 random level slot per world instead.
    #[arg(long, default_value = "off", value_parser = parse_piranha)]
    piranha_shuffle: PiranhaMode,

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
    #[arg(long, default_value = "on", value_parser = parse_tri)]
    troll_pipes: Tri,

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
    #[arg(long, default_value = "shuffle", value_parser = parse_enemy_mode)]
    ground: EnemyMode,

    /// Shell enemies: off, shuffle, or wild (default: shuffle)
    #[arg(long, default_value = "shuffle", value_parser = parse_enemy_mode)]
    shell: EnemyMode,

    /// Flying enemies: off, shuffle, or wild (default: shuffle)
    #[arg(long, default_value = "shuffle", value_parser = parse_enemy_mode)]
    flying: EnemyMode,

    /// Piranha plant variants: off, shuffle, or wild (default: shuffle)
    #[arg(long, default_value = "shuffle", value_parser = parse_enemy_mode)]
    piranhas: EnemyMode,

    /// Ghost house enemies: off, shuffle, or wild (default: shuffle)
    #[arg(long, default_value = "shuffle", value_parser = parse_enemy_mode)]
    ghosts: EnemyMode,

    /// Thwomp variants: off, shuffle, or wild (default: off)
    #[arg(long, default_value = "off", value_parser = parse_enemy_mode)]
    thwomps: EnemyMode,

    /// Rotodisc variants: off, shuffle, or wild (default: off)
    #[arg(long, default_value = "off", value_parser = parse_enemy_mode)]
    rotodiscs: EnemyMode,

    /// Cannon fire variants: off, shuffle, or wild (default: off)
    #[arg(long, default_value = "off", value_parser = parse_enemy_mode)]
    cannons: EnemyMode,

    /// Water enemies: off, shuffle, or wild (default: shuffle)
    #[arg(long, default_value = "shuffle", value_parser = parse_enemy_mode)]
    water: EnemyMode,

    /// Hammer/Boomerang/Fire Bros: off, shuffle, or wild (default: shuffle)
    #[arg(long, default_value = "shuffle", value_parser = parse_enemy_mode)]
    bros: EnemyMode,

    /// HB encounter segments: off, shuffle, or wild (default: off)
    #[arg(long, default_value = "off", value_parser = parse_enemy_mode)]
    hb_encounters: EnemyMode,

    /// Inject Lakitu/Angry Sun/Boss Bass into ~15% of segments
    #[arg(long)]
    wild_injections: bool,

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

/// Assemble the randomizer `Options` from parsed CLI arguments (or from a
/// `--flags` key, which overrides everything except the cosmetic overlays).
/// Exits with an error message on invalid starting items or flag key.
fn build_options(cli: &Cli) -> Options {
    let starting_items: Vec<u8> = cli.starting_items.iter().map(|name| {
        item_id(name).unwrap_or_else(|| {
            eprintln!("Unknown item: {name}");
            let valid: Vec<&str> = ITEMS.iter().map(|&(n, _, _)| n).collect();
            eprintln!("Valid: {}", valid.join(", "));
            process::exit(1);
        })
    }).collect();
    if starting_items.len() > 3 {
        eprintln!("At most 3 starting items allowed (got {})", starting_items.len());
        process::exit(1);
    }

    if let Some(ref flag_key) = cli.flags {
        match Options::from_flag_key(flag_key) {
            Ok(mut opts) => {
                // palette_themed is cosmetic and deliberately not encoded in the
                // flag key, so --themed-palettes must overlay on top of a decoded key.
                if cli.themed_palettes {
                    opts.palette_themed = true;
                }
                // player_color is cosmetic too — overlay like palette_themed.
                if cli.player_color.is_some() {
                    opts.player_color = cli.player_color;
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
            player_color: cli.player_color,
            world_order: cli.world_order,
            world_count: cli.world_count,
            big_q_blocks: cli.big_q_blocks,
            shuffle_pipes: !cli.no_shuffle_pipes,
            shuffle_airships: !cli.no_shuffle_airships,
            shuffle_hammer_bros: !cli.no_shuffle_hammer_bros,
            disable_autoscroll: !cli.keep_autoscroll,
            chest_items: !cli.no_chest_items,
            remove_whistles: !cli.keep_whistles,
            more_hammer_rocks: cli.more_hammer_rocks,
            eights_are_wild: cli.eights_are_wild,
            antechamber_shuffle: cli.antechamber_shuffle,
            card_speed_clear: !cli.no_card_speed_clear,
            remove_n_cards: !cli.keep_n_cards,
            skip_wand_cutscene: !cli.keep_wand_cutscene,
            adjust_boss_hitboxes: !cli.keep_boss_hitboxes,
            koopaling_hits: !cli.keep_koopaling_stomps,
            boomboom_hits: !cli.keep_boomboom_stomps,
            hammer_vulnerable_koopalings: cli.hammer_vulnerable_koopalings,
            random_koopalings: cli.random_koopalings,
            hammer_breaks_locks: cli.hammer_breaks_locks,
            hammer_breaks_bridges: cli.hammer_breaks_bridges,
            early_sun: cli.early_sun,
            limit_bro_movement: cli.limit_bro_movement,
            japanese_damage: cli.japanese_damage,
            infinite_mushroom_houses: cli.infinite_mushroom_houses,
            fast_mushroom_house: cli.fast_mushroom_house,
            faster_tail_speed: cli.faster_tail_speed,
            no_game_over_penalty: cli.no_game_over_penalty,
            faster_frog: cli.faster_frog,
            fire_flower: cli.fire_flower,
            piranha_shuffle: cli.piranha_shuffle,
            shuffle_spade_games: !cli.no_shuffle_spade_games,
            shuffle_toad_houses: !cli.no_shuffle_toad_houses,
            hands_levels: !cli.no_hands_levels,
            troll_pipes: cli.troll_pipes,
            include_beta_stages: cli.include_beta_stages,
            swap_start_airship: cli.swap_start_airship,
            anchor_visuals: cli.anchor_visuals,
            ground: cli.ground,
            shell: cli.shell,
            flying: cli.flying,
            piranhas: cli.piranhas,
            ghosts: cli.ghosts,
            thwomps: cli.thwomps,
            rotodiscs: cli.rotodiscs,
            cannons: cli.cannons,
            water: cli.water,
            bros: cli.bros,
            hb_encounters: cli.hb_encounters,
            wild_injections: cli.wild_injections,
            starting_lives: cli.starting_lives,
            starting_items,
            skip_rom_validation: cli.skip_rom_validation,
        }
    }
}

/// Print the run summary (seed, flag key, active options, output path) to stderr.
fn print_summary(options: &Options, seed: u64, output_path: &std::path::Path) {
    eprintln!("SMB3 Randomizer v{}", env!("CARGO_PKG_VERSION"));
    eprintln!("  Seed: {seed}");
    eprintln!("  Flags: {}", options.to_flag_key());
    eprintln!("  Powerups: {}", if options.powerups { "on" } else { "off" });
    eprintln!("  Player colors: {}", match (options.palettes, options.player_color) {
        (false, _)      => "vanilla".to_string(),
        (true, None)    => "random".to_string(),
        (true, Some(c)) => format!("${c:02X}"),
    });
    eprintln!("  World colors: {}", if options.palette_themed { "themed" } else { "vanilla" });
    eprintln!("  Enemies:  {}", if options.any_enemies_active() { "on" } else { "off" });
    eprintln!("  World order: {}", if options.world_order { "on" } else { "off" });
    if options.world_order && options.world_count < 7 {
        eprintln!("  World count: {}", options.world_count);
    }
    eprintln!("  Big ? Blocks: {}", if options.big_q_blocks { "on" } else { "off" });
    eprintln!("  Starting Lives: {}", options.starting_lives);
    eprintln!("  Pipe shuffle: {}", if options.shuffle_pipes { "on" } else { "off" });
    eprintln!("  Airship shuffle: {}", if options.shuffle_airships { "on" } else { "off" });
    eprintln!("  Hammer Bro shuffle: {}", if options.shuffle_hammer_bros { "on" } else { "off" });
    eprintln!("  Autoscroll: {}", if options.disable_autoscroll { "disabled" } else { "enabled" });
    eprintln!("  Chest items: {}", if options.chest_items { "on" } else { "off" });
    eprintln!("  Warp whistles: {}", if options.remove_whistles { "removed" } else { "kept" });
    eprintln!("  More hammer rocks: {}", tri_str(options.more_hammer_rocks));
    eprintln!("  8s are Wild: {}", tri_str(options.eights_are_wild));
    eprintln!("  Antechamber shuffle: {}", tri_str(options.antechamber_shuffle));
    eprintln!("  Random fire flower: {}", match options.fire_flower {
        FireFlowerMode::Off => "off",
        FireFlowerMode::On => "on",
        FireFlowerMode::Wild => "wild",
    });
    eprintln!("  Piranha shuffle: {}", match options.piranha_shuffle {
        PiranhaMode::Off => "off",
        PiranhaMode::On => "on",
        PiranhaMode::Wild => "wild",
    });
    if !options.starting_items.is_empty() {
        let item_names: Vec<&str> =
            options.starting_items.iter().map(|&id| item_display_name(id)).collect();
        eprintln!("  Starting items: {}", item_names.join(", "));
    }
    eprintln!("  Output:   {}", output_path.display());
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

    let options = build_options(&cli);

    let ext = if cli.patched_rom { "nes" } else { "ips" };
    let output_path = cli
        .output
        .clone()
        .unwrap_or_else(|| PathBuf::from(format!("smb3-rs_{seed}.{ext}")));

    print_summary(&options, seed, &output_path);

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
