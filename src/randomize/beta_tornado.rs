//! Turn one of β9's Fire Chomps into a Tornado.
//!
//! The β9 beta stage (tileset 13, obj `$CEBD`) has three Fire Chomps in its
//! enemy stream. When beta stages are included, this picks one of the three at
//! random and rewrites it into a Tornado, borrowing the vertical position from
//! the World 2 quicksand level's Tornado so it spawns at a sensible height.
//!
//! The other two Fire Chomps are left alone (they remain subject to the normal
//! enemy randomizer). Because a Fire Chomp is itself a randomizable id, this
//! must run *after* `enemies::randomize` so the forced Tornado is final.

use rand::Rng;

use crate::rom::Rom;

/// Tornado object id (as used by the W2 quicksand level at file `$0C866`).
const TORNADO_ID: u8 = 0x5D;

/// β9's three Fire Chomps, by file offset of the id byte. These sit in β9's own
/// enemy stream (`$0CECD`..=`$0CEEC`), a gap between the W4-32 and W5-33 vanilla
/// segments that no live pointer-table entry references — so overwriting them
/// touches only β9.
const BETA9_FIRE_CHOMP_OFFSETS: [usize; 3] = [0x0CEE3, 0x0CEE6, 0x0CEE9];

/// The Tornado's y position byte from the W2 quicksand level (`$0C868`): row 2.
/// Only the height is borrowed; the screen/column byte of the replaced Fire
/// Chomp is kept as-is.
const TORNADO_Y_BYTE: u8 = 0x12;

/// Replace one random β9 Fire Chomp with a Tornado.
///
/// Gated on beta stages being included — otherwise β9's data is never loaded and
/// there is no reason to touch it. Only the id and y (height) bytes change; the
/// screen/column byte is preserved from the replaced Fire Chomp.
pub fn randomize_beta9_tornado<R: Rng>(rom: &mut Rom, rng: &mut R) {
    let id_offset = BETA9_FIRE_CHOMP_OFFSETS[rng.random_range(..BETA9_FIRE_CHOMP_OFFSETS.len())];
    rom.write_byte(id_offset, TORNADO_ID);
    rom.write_byte(id_offset + 2, TORNADO_Y_BYTE);
}
