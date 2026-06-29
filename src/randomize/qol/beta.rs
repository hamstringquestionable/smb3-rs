//! Deterministic layout fixes for the 9 beta stages.

use crate::rom::Rom;
use crate::randomize::rom_data::BETA_PATCHES;

/// Apply deterministic layout fixes for the 9 beta stages.
///
/// The vanilla ROM has broken sub-area pointers, wrong start positions, and
/// misaligned tile commands in the beta level data. These 44 byte patches
/// repair the layouts so the stages are playable.
pub fn fix_beta_stages(rom: &mut Rom) {
    for &(offset, value) in BETA_PATCHES {
        rom.write_byte(offset, value);
    }
}
