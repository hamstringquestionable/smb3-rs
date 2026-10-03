//! The World 2 Hammer Bro arena, rebuilt so nothing in it is thick enough to
//! lose an enemy inside.

use crate::randomize::rom_data;
use crate::rom::Rom;

/// Layout pointer and tileset of the desert bro arena — the one room every
/// World 2 bro encounter loads.
const DESERT_ARENA_LAYOUT: (u16, u8) = (0xB1F6, 9);

/// Offset into that layout of the four commands replaced: two cactus runs and
/// the two stacked rows of sand bricks. The 9-byte header and the clouds,
/// trees and first cactus before them are left alone.
const DESERT_ARENA_PATCH_AT: usize = 30;

#[cfg(test)]
const DESERT_ARENA_VANILLA: [u8; 12] = [
    0x79, 0x02, 0x23, // four cactus, columns 2-5
    0x79, 0x07, 0x20, // one cactus, column 7
    0x18, 0x09, 0x62, // sand bricks, upper row
    0x19, 0x09, 0x62, // sand bricks, lower row
];

/// Same byte count as what it replaces, so the layout keeps its length and
/// every walker that steps through the desert region stays aligned.
#[rustfmt::skip]
const DESERT_ARENA_REBUILT: [u8; 12] = [
    0x36, 0x08, 0x14, // five bricks, row $16, columns 8-12
    0x39, 0x04, 0x40, // wood block, row $19, column 4
    0x38, 0x04, 0x40, // wood block, row $18
    0x57, 0x04, 0x05, // wood block holding a leaf (item-shuffled), row $17
];

/// Enemy streams that use the arena. Slot 0 of each is the bro that stood on
/// the sand bricks.
const DESERT_ARENA_ENEMY_PTRS: [u16; 2] = [0xD14D, 0xD142];

/// `(column, row)` that puts slot 0 on the new brick row. Vanilla's `(0B, 16)`
/// stood it on bricks whose top was row `$18`; the new row is two higher.
const DESERT_ARENA_BRO_POS: [u8; 2] = [0x0A, 0x14];

/// Replace the desert bro arena's sand-brick block with a one-tile-thick brick
/// platform and a wood-block column.
///
/// The vanilla block is six tiles wide and two thick, and enemies that end up
/// inside it cannot be reached. Nothing in the rebuilt room is thicker than
/// one tile in the direction an enemy falls. The column is three single-block
/// commands because the desert tileset has no vertical block run, which is
/// what the two cactus runs pay for.
pub(crate) fn rebuild_desert_bro_arena(rom: &mut Rom) {
    let (lay, tileset) = DESERT_ARENA_LAYOUT;
    let Some(base) = rom_data::layout_file_offset(lay, tileset) else {
        return;
    };
    rom.write_range(base + DESERT_ARENA_PATCH_AT, &DESERT_ARENA_REBUILT);

    for ptr in DESERT_ARENA_ENEMY_PTRS {
        // Page byte, then (id, column, row) entries: slot 0's position.
        let pos = rom_data::enemy_ptr_to_file_offset(ptr) + 2;
        rom.write_range(pos, &DESERT_ARENA_BRO_POS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VANILLA: &str = "roms/Super Mario Bros. 3 (USA) (Rev 1).nes";

    #[test]
    fn rebuilds_the_arena_in_place() {
        let Ok(bytes) = std::fs::read(VANILLA) else {
            return;
        };
        let mut rom = Rom::from_bytes_lax(&bytes, true).unwrap();
        let (lay, tileset) = DESERT_ARENA_LAYOUT;
        let base = rom_data::layout_file_offset(lay, tileset).unwrap();
        let at = base + DESERT_ARENA_PATCH_AT;

        // The patch is aimed by offset, so pin what it is aimed at.
        assert_eq!(rom.read_range(at, 12), DESERT_ARENA_VANILLA);
        assert_eq!(rom.read_byte(at + 12), 0xFF, "the replaced commands end the layout");
        for ptr in DESERT_ARENA_ENEMY_PTRS {
            let slot0 = rom_data::enemy_ptr_to_file_offset(ptr) + 1;
            assert_eq!(rom.read_range(slot0, 3), [rom_data::HAMMER_BRO_ID, 0x0B, 0x16]);
        }

        let header_and_scenery = rom.read_range(base, DESERT_ARENA_PATCH_AT).to_vec();
        rebuild_desert_bro_arena(&mut rom);

        assert_eq!(rom.read_range(base, DESERT_ARENA_PATCH_AT), header_and_scenery);
        assert_eq!(rom.read_range(at, 12), DESERT_ARENA_REBUILT);
        assert_eq!(rom.read_byte(at + 12), 0xFF);
        for ptr in DESERT_ARENA_ENEMY_PTRS {
            let slot0 = rom_data::enemy_ptr_to_file_offset(ptr) + 1;
            assert_eq!(rom.read_range(slot0, 3), [rom_data::HAMMER_BRO_ID, 0x0A, 0x14]);
        }
    }

    /// The column's item block is an ordinary group-2 wood block, so the
    /// powerup shuffle has to find it — and only it — among the new commands.
    #[test]
    fn item_block_is_shuffled_and_nothing_else_moves() {
        use rand::SeedableRng;
        use rand_chacha::ChaCha8Rng;

        let Ok(bytes) = std::fs::read(VANILLA) else {
            return;
        };
        let (lay, tileset) = DESERT_ARENA_LAYOUT;
        let at = rom_data::layout_file_offset(lay, tileset).unwrap() + DESERT_ARENA_PATCH_AT;
        let mut items = std::collections::BTreeSet::new();
        for seed in 0..40 {
            let mut rom = Rom::from_bytes_lax(&bytes, true).unwrap();
            rebuild_desert_bro_arena(&mut rom);
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            crate::randomize::items::powerups::randomize(&mut rom, &mut rng, false);
            let got = rom.read_range(at, 12);
            assert_eq!(got[..11], DESERT_ARENA_REBUILT[..11]);
            items.insert(got[11]);
        }
        assert_eq!(items.into_iter().collect::<Vec<_>>(), [0x04, 0x05, 0x06]);
    }
}
