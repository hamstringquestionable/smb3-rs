use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

use crate::randomize;
use crate::rom::Rom;

/// Returns default starting lives (4).
fn default_starting_lives() -> u8 { 4 }

/// Level shuffle mode.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LevelShuffle {
    Off,
    IntraWorld,
    CrossWorld,
}

impl Default for LevelShuffle {
    fn default() -> Self {
        LevelShuffle::Off
    }
}

// Re-export FortressRedistribute from rom_data module
pub use crate::randomize::rom_data::FortressRedistribute;

/// Options controlling which randomizations to apply.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Options {
    pub powerups: bool,
    pub palettes: bool,
    pub enemies: bool,
    pub world_order: bool,
    #[serde(default = "default_false")]
    pub big_q_blocks: bool,
    #[serde(default)]
    pub level_shuffle: LevelShuffle,
    #[serde(default = "default_true")]
    pub disable_autoscroll: bool,
    /// Set starting lives for both Mario and Luigi (1–99).
    #[serde(default = "default_starting_lives")]
    pub starting_lives: u8,
    /// Enable always-on airship lock (anchor effect, disables airship movement on death)
    #[serde(default = "default_true")]
    pub airship_lock: bool,
    /// Randomize chest and reward items (Hammer Bros, Toad House, Princess letter, treasure chests).
    #[serde(default = "default_true")]
    pub chest_items: bool,
    /// Remove warp whistles and replace with random items.
    #[serde(default = "default_true")]
    pub remove_whistles: bool,
    /// Shuffle fortresses and airships across worlds.
    #[serde(default = "default_false")]
    pub shuffle_fortresses: bool,
    /// Fortress redistribute mode: off, intra-world (lock shuffle), or cross-world (redistribute).
    #[serde(default)]
    pub fortress_redistribute: FortressRedistribute,
    /// Shuffle pipe endpoint positions on overworld maps.
    #[serde(default = "default_false")]
    pub shuffle_pipes: bool,
    /// Fix W3 drawbridges so all paths are always passable.
    #[serde(default = "default_true")]
    pub fix_drawbridges: bool,
    /// Remove rocks blocking paths (W2 secret path, W3 boat dock).
    #[serde(default = "default_true", alias = "remove_w2_rock")]
    pub remove_rocks: bool,
    /// Clear cards instantly (no cutscene, no lives) when collecting one of each type.
    #[serde(default = "default_true")]
    pub card_speed_clear: bool,
    /// Remove N-card (N-Spade) panels from the overworld map.
    #[serde(default = "default_true")]
    pub remove_n_cards: bool,
    /// Skip the wand falling cutscene after defeating a Koopaling.
    #[serde(default = "default_true")]
    pub skip_wand_cutscene: bool,
}

fn default_false() -> bool {
    false
}

fn default_true() -> bool {
    true
}

const FLAG_KEY_VERSION: u8 = 2;
const FLAG_KEY_PREFIX: &str = "SMB3R-";
/// Free space in PRG012 after the Big ? Block trampoline (0x19DD0 region).
/// The trampoline uses 0x19DD0–0x19DE1; we place the 16-byte stamp at 0x19DF0.
const STAMP_OFFSET: usize = 0x19DF0;

impl Options {
    /// Encode options into 5 raw bytes.
    pub fn to_flag_bytes(&self) -> [u8; 5] {
        let level_shuffle_val = match self.level_shuffle {
            LevelShuffle::Off => 0u8,
            LevelShuffle::IntraWorld => 1,
            LevelShuffle::CrossWorld => 2,
        };
        let fortress_val = match self.fortress_redistribute {
            FortressRedistribute::Off => 0u8,
            FortressRedistribute::IntraWorld => 1,
            FortressRedistribute::CrossWorld => 2,
        };

        let b0 = FLAG_KEY_VERSION;

        let b1 = (self.powerups as u8) << 7
            | (self.palettes as u8) << 6
            | (self.enemies as u8) << 5
            | (self.world_order as u8) << 4
            | (self.big_q_blocks as u8) << 3
            | (self.disable_autoscroll as u8) << 2
            | (self.airship_lock as u8) << 1
            | (self.chest_items as u8);

        let b2 = (self.remove_whistles as u8) << 7
            | (self.shuffle_fortresses as u8) << 6
            | (self.shuffle_pipes as u8) << 5
            | (self.fix_drawbridges as u8) << 4
            | (self.remove_rocks as u8) << 3
            | (level_shuffle_val & 0x03) << 1
            | ((fortress_val >> 1) & 0x01);

        let b3 = ((fortress_val & 0x01) << 7)
            | (self.starting_lives.min(99).max(1) & 0x7F);

        let b4 = (self.card_speed_clear as u8) << 7
            | (self.remove_n_cards as u8) << 6
            | (self.skip_wand_cutscene as u8) << 5;

        [b0, b1, b2, b3, b4]
    }

    /// Encode options into a compact hex flag key (e.g. "SMB3R-02E3880480").
    pub fn to_flag_key(&self) -> String {
        let [b0, b1, b2, b3, b4] = self.to_flag_bytes();
        format!("{FLAG_KEY_PREFIX}{b0:02X}{b1:02X}{b2:02X}{b3:02X}{b4:02X}")
    }

    /// Decode a flag key string (e.g. "SMB3R-02E3880480") into Options.
    /// Also accepts v1 keys (8 hex digits) with defaults for new fields.
    pub fn from_flag_key(key: &str) -> Result<Options, String> {
        let hex = key.strip_prefix(FLAG_KEY_PREFIX)
            .or_else(|| key.strip_prefix("smb3r-"))
            .unwrap_or(key);

        if hex.len() != 8 && hex.len() != 10 {
            return Err(format!("Flag key must be 8 or 10 hex digits (got {})", hex.len()));
        }

        let num_bytes = hex.len() / 2;
        let bytes: Vec<u8> = (0..num_bytes)
            .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Invalid hex in flag key: {e}"))?;

        let version = bytes[0];
        if version != 1 && version != FLAG_KEY_VERSION {
            return Err(format!("Unsupported flag key version {version} (expected {FLAG_KEY_VERSION})"));
        }

        let b1 = bytes[1];
        let b2 = bytes[2];
        let b3 = bytes[3];
        let b4 = if bytes.len() > 4 { bytes[4] } else { 0x80 }; // v1 default: card_speed_clear on

        let level_shuffle_val = (b2 >> 1) & 0x03;
        let fortress_val = ((b2 & 0x01) << 1) | ((b3 >> 7) & 0x01);

        let level_shuffle = match level_shuffle_val {
            0 => LevelShuffle::Off,
            1 => LevelShuffle::IntraWorld,
            2 => LevelShuffle::CrossWorld,
            _ => return Err(format!("Invalid level_shuffle value {level_shuffle_val}")),
        };

        let fortress_redistribute = match fortress_val {
            0 => FortressRedistribute::Off,
            1 => FortressRedistribute::IntraWorld,
            2 => FortressRedistribute::CrossWorld,
            _ => return Err(format!("Invalid fortress_redistribute value {fortress_val}")),
        };

        let starting_lives = b3 & 0x7F;
        let starting_lives = if starting_lives == 0 { 1 } else { starting_lives };

        Ok(Options {
            powerups: (b1 >> 7) & 1 != 0,
            palettes: (b1 >> 6) & 1 != 0,
            enemies: (b1 >> 5) & 1 != 0,
            world_order: (b1 >> 4) & 1 != 0,
            big_q_blocks: (b1 >> 3) & 1 != 0,
            disable_autoscroll: (b1 >> 2) & 1 != 0,
            airship_lock: (b1 >> 1) & 1 != 0,
            chest_items: b1 & 1 != 0,
            remove_whistles: (b2 >> 7) & 1 != 0,
            shuffle_fortresses: (b2 >> 6) & 1 != 0,
            shuffle_pipes: (b2 >> 5) & 1 != 0,
            fix_drawbridges: (b2 >> 4) & 1 != 0,
            remove_rocks: (b2 >> 3) & 1 != 0,
            level_shuffle,
            fortress_redistribute,
            starting_lives,
            card_speed_clear: (b4 >> 7) & 1 != 0,
            remove_n_cards: (b4 >> 6) & 1 != 0,
            skip_wand_cutscene: (b4 >> 5) & 1 != 0,
        })
    }
}

impl Default for Options {
    fn default() -> Self {
        Options {
            powerups: true,
            palettes: true,
            enemies: false,
            world_order: false,
            big_q_blocks: false,
            level_shuffle: LevelShuffle::Off,
            disable_autoscroll: true,
            airship_lock: true,
            chest_items: true,
            remove_whistles: true,
            shuffle_fortresses: false,
            fortress_redistribute: FortressRedistribute::Off,
            shuffle_pipes: false,
            fix_drawbridges: true,
            remove_rocks: true,
            card_speed_clear: true,
            remove_n_cards: true,
            skip_wand_cutscene: true,
            starting_lives: default_starting_lives(),
        }
    }
}

/// Apply all enabled randomizations to a ROM using the given seed.
pub fn randomize(rom: &mut Rom, seed: u64, options: &Options) {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);

    // QoL map patches run first so all subsequent overworld operations
    // (fortress redistribution, pipe shuffle, lock shuffle) see the final
    // map connectivity and store correct replacement tiles.
    if options.fix_drawbridges {
        rom.set_tag("qol/drawbridges");
        randomize::qol::fix_w3_drawbridges(rom);
    }
    if options.remove_rocks {
        rom.set_tag("qol/w2_rock");
        randomize::qol::remove_w2_rock(rom);
        rom.set_tag("qol/w3_boat_rock");
        randomize::qol::remove_w3_boat_rock(rom);
    }

    // Fix Big ? Block bonus rooms so they follow the level, not the world slot.
    // Always applied — needed whenever world_order or cross-world shuffle is active,
    // and harmless (identity mapping) when worlds aren't shuffled.
    rom.set_tag("qol/big_q_blocks");
    randomize::qol::fix_big_q_block_rooms(rom);

    // Autoscroll must run BEFORE powerups and the overworld builder:
    // it writes pre-baked replacement level data for airship levels, and
    // powerups/enemies need to randomize on top of that patched data.
    // It also writes airship pointer table redirects to hardcoded vanilla
    // offsets — the overworld builder's resort_pointer_table() rearranges
    // entries later, so autoscroll must go first.
    if options.disable_autoscroll {
        rom.set_tag("autoscroll");
        randomize::autoscroll::disable_autoscroll(rom);
    }
    if options.powerups {
        rom.set_tag("powerups");
        randomize::powerups::randomize(rom, &mut rng);
    }
    if options.palettes {
        rom.set_tag("palettes");
        randomize::palettes::randomize(rom, &mut rng);
    }
    if options.enemies {
        rom.set_tag("enemies");
        randomize::enemies::randomize(rom, &mut rng);
    }
    if options.world_order {
        rom.set_tag("world_order");
        randomize::world_order::randomize(rom, &mut rng);
    }
    if options.big_q_blocks {
        rom.set_tag("enemies/big_q_blocks");
        randomize::enemies::randomize_big_q_blocks(rom, &mut rng);
    }
    // Airship shuffle runs after autoscroll (which patches airship pointer
    // entries at vanilla indices) and before the overworld builder (whose
    // resort_pointer_table re-sorts everything). shuffle_entries only moves
    // tileset + ObjSets + LevelLayouts, preserving row/col position, so
    // patched data travels correctly to its new world.
    if options.shuffle_fortresses {
        rom.set_tag("levels/airships");
        randomize::levels::randomize_airships(rom, &mut rng);
    }
    // Overworld builder: unified lock shuffle, pipe shuffle, level/fortress
    // redistribution, and overworld map rewriting. When active, it handles
    // all level, fortress, and pipe shuffling — bypassing levels.rs.
    let shuffle_locks = options.fortress_redistribute != FortressRedistribute::Off;
    let shuffle_pipes = options.shuffle_pipes;
    let shuffle_levels_cross = options.level_shuffle == LevelShuffle::CrossWorld;
    let shuffle_fortresses_cross = options.shuffle_fortresses;
    let overworld_active = shuffle_locks || shuffle_pipes
        || shuffle_levels_cross || shuffle_fortresses_cross;

    if overworld_active {
        rom.set_tag("overworld/builder");
        let cross_world = shuffle_levels_cross || shuffle_fortresses_cross;
        let catalog = randomize::node_catalog::NodeCatalog::build(rom);
        let pickup = randomize::overworld_pickup::pick_up(rom, &catalog);
        let build = randomize::overworld_build::build(rom, &pickup, &catalog, &mut rng);
        randomize::overworld_writer::write_overworld(
            rom, &build, &pickup, &catalog, &mut rng, cross_world,
        );
    } else if options.level_shuffle == LevelShuffle::IntraWorld {
        // Intra-world level shuffle (no overworld rebuild needed)
        rom.set_tag("levels");
        randomize::levels::randomize_intra(rom, &mut rng);
    }
    if options.chest_items {
        rom.set_tag("items");
        randomize::items::randomize(rom, &mut rng, options.remove_whistles);
    } else if options.remove_whistles {
        rom.set_tag("items/whistles");
        randomize::items::remove_whistles_only(rom, &mut rng);
    }

    // Set starting lives (default 4; user/configurable)
    rom.set_tag("qol/starting_lives");
    randomize::qol::set_starting_lives(rom, options.starting_lives);

    // Airship lock (anchor effect always on): patch at 0x1FABC ("KXUUXZVG" / Game Genie)
    if options.airship_lock {
        rom.set_tag("airship_lock");
        // A9 01 EA = LDA #$01; NOP (forces anchor flag always set)
        rom.write_range(0x1FABC, &[0xA9, 0x01, 0xEA]);
        // Anchors are now useless — replace all anchor items in item tables
        // with a single randomly chosen powerup for this seed.
        rom.set_tag("items/anchors");
        randomize::items::replace_anchors(rom, &mut rng);
    }

    // Patch double-digit level tiles (11–19) to show a "1" tens digit
    rom.set_tag("metatile/double_digit");
    randomize::overworld_writer::patch_double_digit_metatiles(rom);

    // Randomize king quotes (always on — cosmetic flavor text)
    rom.set_tag("king_quotes");
    randomize::king_quotes::randomize(rom, &mut rng);

    // Skip the wand falling cutscene after defeating a Koopaling.
    if options.skip_wand_cutscene {
        rom.set_tag("qol/skip_wand_cutscene");
        randomize::qol::skip_wand_cutscene(rom);
    }

    // Remove N-card (N-Spade) panels from the overworld map.
    if options.remove_n_cards {
        rom.set_tag("qol/remove_n_cards");
        randomize::qol::remove_n_cards(rom);
    }

    // Card speed clear: one-of-each clears cards with +1 life but no cutscene.
    if options.card_speed_clear {
        rom.set_tag("qol/card_speed_clear");
        randomize::qol::card_speed_clear(rom);
    }

    // Stamp flag key + seed into free space at STAMP_OFFSET (PRG012). 17 bytes:
    //   [0..4]  "S3R\x02" magic + version
    //   [4..9]  flag key bytes (encoding of Options)
    //   [9..17] seed (little-endian u64)
    rom.set_tag("stamp");
    let flag_bytes = options.to_flag_bytes();
    let mut stamp = [0u8; 17];
    stamp[0..3].copy_from_slice(b"S3R");
    stamp[3] = FLAG_KEY_VERSION;
    stamp[4..9].copy_from_slice(&flag_bytes);
    stamp[9..17].copy_from_slice(&seed.to_le_bytes());
    rom.write_range(STAMP_OFFSET, &stamp);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rom::Rom;

    const ANCHOR_PATCH_OFFSET: usize = 0x1FABC;
    const PATCHED_BYTES: [u8; 3] = [0xA9, 0x01, 0xEA];
    const ANCHOR: u8 = 0x0A;

    // Item table offsets (must match items.rs)
    const HAMMER_BROS_ITEMS_OFFSET: usize = 0x16190;
    const TOAD_HOUSE_ITEMS_OFFSET: usize = 0x3B14B;

    fn make_test_rom() -> Rom {
        let mut data = vec![0u8; 393232];
        // iNES header
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;
        Rom::from_bytes(&data).unwrap()
    }

    #[test]
    fn randomized_rom_has_anchor_lock_patch_by_default() {
        let mut rom = make_test_rom();
        let original_bytes = rom.read_range(ANCHOR_PATCH_OFFSET, 3).to_vec();
        let options = Options::default();
        randomize(&mut rom, 0x12345678, &options);

        assert_eq!(
            rom.read_range(ANCHOR_PATCH_OFFSET, 3),
            &PATCHED_BYTES,
            "Anchor lock patch should be present by default"
        );
        // Sanity: the patch actually changed something
        assert_ne!(
            original_bytes, PATCHED_BYTES,
            "Test ROM should not already contain the patch bytes"
        );
    }

    #[test]
    fn anchor_lock_patch_can_be_disabled() {
        let mut rom = make_test_rom();
        let original_bytes = rom.read_range(ANCHOR_PATCH_OFFSET, 3).to_vec();
        let mut options = Options::default();
        options.airship_lock = false;
        randomize(&mut rom, 0x12345678, &options);

        assert_eq!(
            rom.read_range(ANCHOR_PATCH_OFFSET, 3),
            &original_bytes[..],
            "Anchor lock patch must NOT be present when airship_lock = false"
        );
    }

    #[test]
    fn anchors_replaced_when_airship_lock_on() {
        let mut rom = make_test_rom();
        // Place anchors in item tables
        rom.write_byte(HAMMER_BROS_ITEMS_OFFSET + 2, ANCHOR);
        rom.write_byte(TOAD_HOUSE_ITEMS_OFFSET + 1, ANCHOR);

        let mut options = Options::default();
        options.airship_lock = true;
        // Disable chest_items so our manually placed anchors survive to the replacement step
        options.chest_items = false;
        options.remove_whistles = false;
        randomize(&mut rom, 0x12345678, &options);

        let r1 = rom.read_byte(HAMMER_BROS_ITEMS_OFFSET + 2);
        let r2 = rom.read_byte(TOAD_HOUSE_ITEMS_OFFSET + 1);
        assert_ne!(r1, ANCHOR, "Anchor in Hammer Bros table was not replaced");
        assert_ne!(r2, ANCHOR, "Anchor in Toad House table was not replaced");
        assert_eq!(r1, r2, "All anchors should become the same powerup for a given seed");
    }

    #[test]
    fn anchors_kept_when_airship_lock_off() {
        let mut rom = make_test_rom();
        rom.write_byte(HAMMER_BROS_ITEMS_OFFSET + 2, ANCHOR);

        let mut options = Options::default();
        options.airship_lock = false;
        options.chest_items = false;
        options.remove_whistles = false;
        randomize(&mut rom, 0x12345678, &options);

        assert_eq!(
            rom.read_byte(HAMMER_BROS_ITEMS_OFFSET + 2),
            ANCHOR,
            "Anchor should be preserved when airship_lock is off"
        );
    }

    #[test]
    fn write_log_populated_after_randomize() {
        let mut rom = make_test_rom();
        let options = Options::default();
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
    fn flag_key_round_trip_defaults() {
        let opts = Options::default();
        let key = opts.to_flag_key();
        assert!(key.starts_with("SMB3R-"));
        assert_eq!(key.len(), 16); // "SMB3R-" + 10 hex
        let decoded = Options::from_flag_key(&key).unwrap();
        assert_eq!(opts.powerups, decoded.powerups);
        assert_eq!(opts.palettes, decoded.palettes);
        assert_eq!(opts.enemies, decoded.enemies);
        assert_eq!(opts.world_order, decoded.world_order);
        assert_eq!(opts.big_q_blocks, decoded.big_q_blocks);
        assert_eq!(opts.disable_autoscroll, decoded.disable_autoscroll);
        assert_eq!(opts.airship_lock, decoded.airship_lock);
        assert_eq!(opts.chest_items, decoded.chest_items);
        assert_eq!(opts.remove_whistles, decoded.remove_whistles);
        assert_eq!(opts.shuffle_fortresses, decoded.shuffle_fortresses);
        assert_eq!(opts.shuffle_pipes, decoded.shuffle_pipes);
        assert_eq!(opts.fix_drawbridges, decoded.fix_drawbridges);
        assert_eq!(opts.remove_rocks, decoded.remove_rocks);
        assert_eq!(opts.level_shuffle, decoded.level_shuffle);
        assert_eq!(opts.fortress_redistribute, decoded.fortress_redistribute);
        assert_eq!(opts.starting_lives, decoded.starting_lives);
        assert_eq!(opts.card_speed_clear, decoded.card_speed_clear);
        assert_eq!(opts.remove_n_cards, decoded.remove_n_cards);
        assert_eq!(opts.skip_wand_cutscene, decoded.skip_wand_cutscene);
    }

    #[test]
    fn flag_key_round_trip_all_on() {
        let opts = Options {
            powerups: true,
            palettes: true,
            enemies: true,
            world_order: true,
            big_q_blocks: true,
            level_shuffle: LevelShuffle::CrossWorld,
            disable_autoscroll: true,
            airship_lock: true,
            chest_items: true,
            remove_whistles: true,
            shuffle_fortresses: true,
            shuffle_pipes: true,
            fix_drawbridges: true,
            remove_rocks: true,
            fortress_redistribute: FortressRedistribute::CrossWorld,
            starting_lives: 99,
            card_speed_clear: true,
            remove_n_cards: true,
            skip_wand_cutscene: true,
        };
        let key = opts.to_flag_key();
        let decoded = Options::from_flag_key(&key).unwrap();
        assert_eq!(opts.enemies, decoded.enemies);
        assert_eq!(opts.world_order, decoded.world_order);
        assert_eq!(opts.level_shuffle, decoded.level_shuffle);
        assert_eq!(opts.fortress_redistribute, decoded.fortress_redistribute);
        assert_eq!(opts.starting_lives, decoded.starting_lives);
        assert_eq!(opts.shuffle_pipes, decoded.shuffle_pipes);
        assert_eq!(opts.shuffle_fortresses, decoded.shuffle_fortresses);
        assert_eq!(opts.remove_n_cards, decoded.remove_n_cards);
        assert_eq!(opts.skip_wand_cutscene, decoded.skip_wand_cutscene);
    }

    #[test]
    fn flag_key_round_trip_all_off() {
        let opts = Options {
            powerups: false,
            palettes: false,
            enemies: false,
            world_order: false,
            big_q_blocks: false,
            level_shuffle: LevelShuffle::Off,
            disable_autoscroll: false,
            airship_lock: false,
            chest_items: false,
            remove_whistles: false,
            shuffle_fortresses: false,
            shuffle_pipes: false,
            fix_drawbridges: false,
            remove_rocks: false,
            fortress_redistribute: FortressRedistribute::Off,
            starting_lives: 1,
            card_speed_clear: false,
            remove_n_cards: false,
            skip_wand_cutscene: false,
        };
        let key = opts.to_flag_key();
        let decoded = Options::from_flag_key(&key).unwrap();
        assert!(!decoded.powerups);
        assert!(!decoded.palettes);
        assert!(!decoded.enemies);
        assert!(!decoded.disable_autoscroll);
        assert_eq!(decoded.starting_lives, 1);
        assert_eq!(decoded.level_shuffle, LevelShuffle::Off);
        assert_eq!(decoded.fortress_redistribute, FortressRedistribute::Off);
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
        let hex = key.strip_prefix("SMB3R-").unwrap();
        let decoded = Options::from_flag_key(hex).unwrap();
        assert_eq!(opts.powerups, decoded.powerups);
    }

    #[test]
    fn flag_key_invalid_version() {
        let result = Options::from_flag_key("FF000000");
        assert!(result.is_err());
    }

    #[test]
    fn flag_key_invalid_hex() {
        let result = Options::from_flag_key("ZZZZZZZZ");
        assert!(result.is_err());
    }

    #[test]
    fn write_log_tags_match_enabled_modules() {
        let mut rom = make_test_rom();
        let mut options = Options::default();
        // Disable optional modules we can check for absence
        options.enemies = false;
        options.world_order = false;
        // Keep these on — they write to known offsets even on a zeroed ROM
        options.disable_autoscroll = true;
        options.airship_lock = true;
        randomize(&mut rom, 42, &options);

        let tags: Vec<&str> = rom.write_log().iter().map(|r| r.tag.as_str()).collect();
        // These modules write to fixed offsets that differ from zero
        assert!(tags.iter().any(|t| t.starts_with("autoscroll")));
        assert!(tags.iter().any(|t| t.starts_with("airship_lock")));
        // Disabled modules should not appear
        assert!(!tags.iter().any(|t| t.starts_with("enemies")));
        assert!(!tags.iter().any(|t| t.starts_with("world_order")));
    }
}
