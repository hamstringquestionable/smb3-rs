use rand::Rng;

use crate::rom::Rom;

use super::level_helpers;
use super::rom_data::AIRSHIP_ENTRIES;

/// Shuffle airships across worlds 1-7. Each world's airship map tile
/// can load any of the 7 airship levels.
///
/// Note: when autoscroll is disabled, the autoscroll patch overwrites
/// airship pointer entries with world-specific redesigned data after
/// this shuffle runs, so airship shuffle only has a visible effect
/// when autoscroll is kept enabled.
pub fn randomize_airships<R: Rng>(rom: &mut Rom, rng: &mut R) {
    level_helpers::shuffle_entries(rom, rng, AIRSHIP_ENTRIES);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::randomize::rom_data::{self, WORLDS, read_word};
    use rand_chacha::ChaCha8Rng;
    use rand::SeedableRng;

    fn make_test_rom_with_airships() -> Rom {
        let mut data = vec![0u8; 393232];
        // iNES header
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;

        // Give each airship entry a unique lay pointer so we can detect a shuffle.
        for &(w_idx, entry_idx) in AIRSHIP_ENTRIES.iter() {
            let w = &WORLDS[w_idx];
            let (_, _, layouts) = rom_data::table_offsets(w);
            let lay_off = layouts + entry_idx * 2;
            let lay_val: u16 = 0xA800 + (w_idx as u16) * 0x10;
            data[lay_off] = (lay_val & 0xFF) as u8;
            data[lay_off + 1] = ((lay_val >> 8) & 0xFF) as u8;
        }

        Rom::from_bytes_lax(&data, true).unwrap()
    }

    #[test]
    fn test_airship_shuffle() {
        let mut rom = make_test_rom_with_airships();
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let original_lays: Vec<u16> = AIRSHIP_ENTRIES.iter().map(|&(w, i)| {
            let world = &WORLDS[w];
            let (_, _, layouts) = rom_data::table_offsets(world);
            read_word(&rom, layouts + i * 2)
        }).collect();

        randomize_airships(&mut rom, &mut rng);

        let shuffled_lays: Vec<u16> = AIRSHIP_ENTRIES.iter().map(|&(w, i)| {
            let world = &WORLDS[w];
            let (_, _, layouts) = rom_data::table_offsets(world);
            read_word(&rom, layouts + i * 2)
        }).collect();

        let mut orig_sorted = original_lays.clone();
        let mut shuf_sorted = shuffled_lays.clone();
        orig_sorted.sort();
        shuf_sorted.sort();
        assert_eq!(orig_sorted, shuf_sorted, "Airship lay pointers should be a permutation");
    }
}
