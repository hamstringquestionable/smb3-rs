use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

use crate::randomize;
use crate::rom::Rom;

/// Options controlling which randomizations to apply.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Options {
    pub powerups: bool,
    pub palettes: bool,
    pub enemies: bool,
    pub world_order: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            powerups: true,
            palettes: true,
            enemies: false,
            world_order: false,
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
}
