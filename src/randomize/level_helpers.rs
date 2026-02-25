/// Level helpers: shared write operations for level entry manipulation.
///
/// Parallels `pipe_helpers.rs` and `overworld_helpers.rs`. Contains
/// mechanical ROM write operations used by multiple randomization modules.

use rand::Rng;
use rand::seq::SliceRandom;

use crate::rom::Rom;

use super::rom_data::{self, LevelEntry, WORLDS};

/// Shuffle level entries among the given (world_idx, entry_idx) slots.
///
/// Reads all entries, shuffles them, and writes back. The ByRowType byte
/// (which contains the tileset in its lower nibble) travels with the level
/// data so the game's lookup key stays consistent.
pub(super) fn shuffle_entries<R: Rng>(rom: &mut Rom, rng: &mut R, indices: &[(usize, usize)]) {
    if indices.len() <= 1 {
        return;
    }

    let mut entries: Vec<LevelEntry> = indices
        .iter()
        .map(|&(w, i)| rom_data::read_entry(rom, &WORLDS[w], i))
        .collect();

    entries.as_mut_slice().shuffle(rng);

    for (slot, &(w, idx)) in indices.iter().enumerate() {
        rom_data::write_entry(rom, &WORLDS[w], idx, &entries[slot]);
    }
}
