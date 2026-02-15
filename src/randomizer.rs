use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

use crate::randomize;
use crate::rom::Rom;

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
    /// Enable debug mode: press Select to cycle through powerup forms in-game.
    #[serde(default = "default_false")]
    pub debug_mode: bool,
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
            debug_mode: false,
        }
    }
}

/// Apply all enabled randomizations to a ROM using the given seed.
pub fn randomize(rom: &mut Rom, seed: u64, options: &Options) {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);

    if options.powerups {
        randomize::powerups::randomize(rom, &mut rng);
    }
    if options.palettes {
        randomize::palettes::randomize(rom, &mut rng);
    }
    if options.enemies {
        randomize::enemies::randomize(rom, &mut rng);
    }
    if options.world_order {
        randomize::world_order::randomize(rom, &mut rng);
    }
    if options.big_q_blocks {
        randomize::enemies::randomize_big_q_blocks(rom, &mut rng);
    }
    match options.level_shuffle {
        LevelShuffle::Off => {}
        LevelShuffle::IntraWorld => randomize::levels::randomize_intra(rom, &mut rng),
        LevelShuffle::CrossWorld => randomize::levels::randomize_cross(rom, &mut rng),
    }
    if options.disable_autoscroll {
        randomize::autoscroll::disable_autoscroll(rom);
    }
    // Always apply: 99 starting lives
    randomize::qol::set_starting_lives(rom, 99);
    if options.debug_mode {
        randomize::qol::enable_debug_mode(rom);
    }
}
