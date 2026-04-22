//! Clone the W8 Hand sub-area enemy stream so each of the three Hand levels
//! (8-Hnd1/2/3) has its own treasure room. In vanilla, all three Hand levels'
//! main-level layout headers point at the same `alt_objects=$D0CF` enemy
//! stream, so the OBJ_TREASURESET reward is shared across all three. Cloning
//! the 11-byte enemy stream into PRG006 free space and re-pointing two of the
//! three headers gives `items::randomize` three independent Y-bytes to roll.
//!
//! The layout (`alt_layout=$BE17`) stays shared — only the enemy data needs
//! per-Hand differentiation, since the OBJ_TREASURESET is in the enemy stream.

use crate::rom::Rom;
use super::rom_data::FS_HAND_ROOMS;

/// File offset of the original Hand sub-area enemy stream (11 bytes:
/// 1 page byte + 3 enemy entries × 3 bytes + 1 terminator). CPU $D0CF.
const HAND_ROOM_SRC: usize = 0x0D0DF;
const HAND_ROOM_LEN: usize = 11;

/// File offsets of bytes 2-3 (`alt_objects`) of each Hand level's main-level
/// layout header in PRG019.
const HND2_ALT_OBJ_LO: usize = 0x27CF0; // 8-Hnd2 header at 0x27CEE
const HND3_ALT_OBJ_LO: usize = 0x27DA1; // 8-Hnd3 header at 0x27D9F

/// CPU addresses of the two clones inside PRG006 ($C000 base).
const CLONE_A_CPU: u16 = 0xDA64; // file 0x0DA74
const CLONE_B_CPU: u16 = 0xDA6F; // file 0x0DA7F

/// Y-byte file offsets of the OBJ_TREASURESET (0xD6) entry in each clone.
/// Layout within an 11-byte stream: [page][D6 X Y][52 X Y][BA X Y][FF],
/// so the first entry's Y-byte sits at offset +3.
pub(super) const HAND_ROOM_CLONE_A_ITEM: usize = 0x0DA74 + 3; // 0x0DA77
pub(super) const HAND_ROOM_CLONE_B_ITEM: usize = 0x0DA7F + 3; // 0x0DA82

/// Duplicate the shared Hand sub-area into two clones and re-point 8-Hnd2 and
/// 8-Hnd3 to the new copies. Unconditional structural patch — runs before
/// `items::randomize` so each clone has a Y-byte that the chest randomizer
/// can roll independently.
pub fn patch_clone_hand_rooms(rom: &mut Rom) {
    let src = rom.read_range(HAND_ROOM_SRC, HAND_ROOM_LEN).to_vec();

    rom.write_range(FS_HAND_ROOMS, &src);
    rom.write_range(FS_HAND_ROOMS + HAND_ROOM_LEN, &src);

    rom.write_range(HND2_ALT_OBJ_LO, &CLONE_A_CPU.to_le_bytes());
    rom.write_range(HND3_ALT_OBJ_LO, &CLONE_B_CPU.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_rom() -> Rom {
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;

        // Vanilla source bytes for the shared Hand sub-area enemy stream.
        let src: [u8; HAND_ROOM_LEN] = [
            0x01,             // page
            0xD6, 0x0C, 0x03, // OBJ_TREASURESET (item Y-byte = 0x03 Leaf)
            0x52, 0x0D, 0x15, // OBJ_TREASUREBOX
            0xBA, 0x0E, 0x15, // OBJ_TREASUREBOXAPPEAR
            0xFF,             // terminator
        ];
        data[HAND_ROOM_SRC..HAND_ROOM_SRC + HAND_ROOM_LEN].copy_from_slice(&src);

        // Vanilla alt_objects bytes in each Hand header point at $D0CF.
        for off in [HND2_ALT_OBJ_LO, HND3_ALT_OBJ_LO, 0x27D52] {
            data[off]     = 0xCF;
            data[off + 1] = 0xD0;
        }

        Rom::from_bytes(&data).unwrap()
    }

    #[test]
    fn test_clones_match_source() {
        let mut rom = make_test_rom();
        patch_clone_hand_rooms(&mut rom);

        let src = rom.read_range(HAND_ROOM_SRC, HAND_ROOM_LEN).to_vec();
        let clone_a = rom.read_range(FS_HAND_ROOMS, HAND_ROOM_LEN).to_vec();
        let clone_b = rom.read_range(FS_HAND_ROOMS + HAND_ROOM_LEN, HAND_ROOM_LEN).to_vec();

        assert_eq!(clone_a, src, "clone A should match source bytes");
        assert_eq!(clone_b, src, "clone B should match source bytes");
    }

    #[test]
    fn test_headers_repointed() {
        let mut rom = make_test_rom();
        patch_clone_hand_rooms(&mut rom);

        // 8-Hnd1 header (untouched) should still point at $D0CF.
        assert_eq!(rom.read_range(0x27D52, 2), &[0xCF, 0xD0]);
        // 8-Hnd2 → clone A ($DA64)
        assert_eq!(rom.read_range(HND2_ALT_OBJ_LO, 2), &[0x64, 0xDA]);
        // 8-Hnd3 → clone B ($DA6F)
        assert_eq!(rom.read_range(HND3_ALT_OBJ_LO, 2), &[0x6F, 0xDA]);
    }

    #[test]
    fn test_clone_item_byte_offsets_align_with_treasureset() {
        let mut rom = make_test_rom();
        patch_clone_hand_rooms(&mut rom);

        // The byte two positions before each item-Y-byte must be the
        // OBJ_TREASURESET ID 0xD6 — otherwise items::randomize would be
        // overwriting the wrong byte.
        assert_eq!(rom.read_byte(HAND_ROOM_CLONE_A_ITEM - 2), 0xD6);
        assert_eq!(rom.read_byte(HAND_ROOM_CLONE_B_ITEM - 2), 0xD6);
        // And the Y-byte itself starts at the vanilla Leaf value.
        assert_eq!(rom.read_byte(HAND_ROOM_CLONE_A_ITEM), 0x03);
        assert_eq!(rom.read_byte(HAND_ROOM_CLONE_B_ITEM), 0x03);
    }
}
