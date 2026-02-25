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

// Re-export FortressShuffle from overworld module
pub use crate::randomize::overworld::FortressShuffle;

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
    /// Fortress shuffle mode: off, intra-world (lock shuffle), or cross-world (redistribute).
    #[serde(default)]
    pub fortress_shuffle: FortressShuffle,
    /// Shuffle pipe endpoint positions on overworld maps.
    #[serde(default = "default_false")]
    pub shuffle_pipes: bool,
    /// Fix W3 drawbridges so all paths are always passable.
    #[serde(default = "default_true")]
    pub fix_drawbridges: bool,
    /// Remove the W2 rock blocking the secret path.
    #[serde(default = "default_true")]
    pub remove_w2_rock: bool,
}

fn default_false() -> bool {
    false
}

fn default_true() -> bool {
    true
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
            fortress_shuffle: FortressShuffle::Off,
            shuffle_pipes: false,
            fix_drawbridges: true,
            remove_w2_rock: true,
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
    if options.remove_w2_rock {
        rom.set_tag("qol/w2_rock");
        randomize::qol::remove_w2_rock(rom);
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
    match options.level_shuffle {
        LevelShuffle::Off => {}
        LevelShuffle::IntraWorld => {
            rom.set_tag("levels");
            randomize::levels::randomize_intra(rom, &mut rng);
        }
        LevelShuffle::CrossWorld => {
            rom.set_tag("levels");
            randomize::levels::randomize_cross(rom, &mut rng);
        }
    }
    if options.shuffle_fortresses {
        rom.set_tag("levels/fortresses");
        randomize::levels::randomize_fortresses(rom, &mut rng);
        rom.set_tag("levels/airships");
        randomize::levels::randomize_airships(rom, &mut rng);
    }
    if options.fortress_shuffle != FortressShuffle::Off {
        rom.set_tag("overworld/fortress");
        randomize::overworld::randomize_fortresses(rom, &mut rng, &options.fortress_shuffle);
    }
    if options.shuffle_pipes {
        rom.set_tag("pipes");
        randomize::pipes::randomize(rom, &mut rng);
    }
    if options.chest_items {
        rom.set_tag("items");
        randomize::items::randomize(rom, &mut rng, options.remove_whistles);
    } else if options.remove_whistles {
        rom.set_tag("items/whistles");
        randomize::items::remove_whistles_only(rom, &mut rng);
    }
    if options.disable_autoscroll {
        rom.set_tag("autoscroll");
        randomize::autoscroll::disable_autoscroll(rom);
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
