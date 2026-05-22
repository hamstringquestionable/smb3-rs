//! W8 Hand treasure-room variety:
//! clone the shared Hand sub-area enemy stream so 8-Hnd1/2/3 have
//! independent `OBJ_TREASURESET` Y-bytes (randomized per-Hand by
//! `items::randomize`).
//!
//! Earlier revisions also rolled a 3/4 chance to redirect one Hand to
//! the 3-7 coin heaven, but that re-pointed at the same shared sub-area
//! as 3-7 itself — same enemy stream, same chest item byte — so the
//! Hand-redirect outcome was indistinguishable from beating 3-7 except
//! for losing the autoscroll-as-risk gating on the chest. Removed in
//! favour of making the coin heaven reachable only from 3-7.

use crate::rom::Rom;
use super::rom_data::FS_HAND_ROOMS;
use super::segment_writer::{self, SegmentSpec, SortMode};

/// File offset of the original Hand sub-area enemy stream (11 bytes:
/// 1 page byte + 3 enemy entries × 3 bytes + 1 terminator). CPU $D0CF.
const HAND_ROOM_SRC: usize = 0x0D0DF;
const HAND_ROOM_LEN: usize = 11;

/// File offsets of each Hand level's 9-byte main-level layout header in
/// PRG019. Bytes 2-3 hold the `alt_objects` CPU pointer that selects the
/// sub-area enemy stream.
const HND2_HDR: usize = 0x27CEE;
const HND3_HDR: usize = 0x27D9F;

/// CPU addresses of the two cloned enemy streams inside PRG006.
const CLONE_A_CPU: u16 = 0xDA64; // file 0x0DA74
const CLONE_B_CPU: u16 = 0xDA6F; // file 0x0DA7F

/// Y-byte file offsets of the OBJ_TREASURESET (0xD6) entry in each clone.
/// Layout within an 11-byte stream: [page][D6 X Y][52 X Y][BA X Y][FF],
/// so the first entry's Y-byte sits at offset +3.
pub(super) const HAND_ROOM_CLONE_A_ITEM: usize = 0x0DA74 + 3; // 0x0DA77
pub(super) const HAND_ROOM_CLONE_B_ITEM: usize = 0x0DA7F + 3; // 0x0DA82

/// Clone the shared Hand sub-area enemy stream so 8-Hnd2 and 8-Hnd3 each
/// get an independent `OBJ_TREASURESET` Y-byte. 8-Hnd1 keeps the original.
pub fn patch_clone_hand_rooms(rom: &mut Rom) {
    rom.push_tag("hand_rooms");

    // Clone the 3-entry source segment to both destinations. The segment
    // layout is [page_byte][entry × 3][terminator]; segment_writer handles
    // the entry bytes, the page byte + terminator are written directly so
    // the destinations end up byte-identical to the source.
    let page_byte = rom.read_byte(HAND_ROOM_SRC);
    let src_entries = segment_writer::read_segment(rom, HAND_ROOM_SRC, 3);
    for (dst, label) in [
        (FS_HAND_ROOMS,                      "8-Hnd2 cloned treasure room"),
        (FS_HAND_ROOMS + HAND_ROOM_LEN,      "8-Hnd3 cloned treasure room"),
    ] {
        rom.write_byte(dst, page_byte);
        segment_writer::write_segment(rom, &SegmentSpec {
            file_offset: dst,
            original_count: 3,
            entries: &src_entries,
            label: Some(label),
            sort_mode: SortMode::SortByX,
        }).expect("hand_rooms: clone write failed");
        rom.write_byte(dst + HAND_ROOM_LEN - 1, 0xFF);
    }
    rom.write_range(HND2_HDR + 2, &CLONE_A_CPU.to_le_bytes());
    rom.write_range(HND3_HDR + 2, &CLONE_B_CPU.to_le_bytes());

    rom.pop_tag();
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

        let src: [u8; HAND_ROOM_LEN] = [
            0x01,
            0xD6, 0x0C, 0x03,
            0x52, 0x0D, 0x15,
            0xBA, 0x0E, 0x15,
            0xFF,
        ];
        data[HAND_ROOM_SRC..HAND_ROOM_SRC + HAND_ROOM_LEN].copy_from_slice(&src);

        // Vanilla Hand headers: all three share alt_layout=$BE17,
        // alt_objects=$D0CF, alt_tileset=11.
        for hdr in [HND2_HDR, HND3_HDR] {
            data[hdr]     = 0x17;
            data[hdr + 1] = 0xBE;
            data[hdr + 2] = 0xCF;
            data[hdr + 3] = 0xD0;
            data[hdr + 6] = 0x0B;
        }

        Rom::from_bytes_lax(&data, true).unwrap()
    }

    fn hdr_obj_ptr(rom: &Rom, off: usize) -> u16 {
        let b = rom.read_range(off, 4);
        u16::from_le_bytes([b[2], b[3]])
    }

    #[test]
    fn test_clones_match_source() {
        let mut rom = make_test_rom();
        patch_clone_hand_rooms(&mut rom);

        let src = rom.read_range(HAND_ROOM_SRC, HAND_ROOM_LEN).to_vec();
        assert_eq!(rom.read_range(FS_HAND_ROOMS, HAND_ROOM_LEN).to_vec(), src);
        assert_eq!(rom.read_range(FS_HAND_ROOMS + HAND_ROOM_LEN, HAND_ROOM_LEN).to_vec(), src);
    }

    #[test]
    fn test_clone_item_byte_offsets_align_with_treasureset() {
        let mut rom = make_test_rom();
        patch_clone_hand_rooms(&mut rom);

        assert_eq!(rom.read_byte(HAND_ROOM_CLONE_A_ITEM - 2), 0xD6);
        assert_eq!(rom.read_byte(HAND_ROOM_CLONE_B_ITEM - 2), 0xD6);
        assert_eq!(rom.read_byte(HAND_ROOM_CLONE_A_ITEM), 0x03);
        assert_eq!(rom.read_byte(HAND_ROOM_CLONE_B_ITEM), 0x03);
    }

    #[test]
    fn test_hnd2_hnd3_repointed_to_clones() {
        // 8-Hnd2 should point at clone A, 8-Hnd3 at clone B. 8-Hnd1 keeps
        // the vanilla pointer ($D0CF) so the source room still has a reader.
        let mut rom = make_test_rom();
        patch_clone_hand_rooms(&mut rom);

        assert_eq!(hdr_obj_ptr(&rom, HND2_HDR), CLONE_A_CPU);
        assert_eq!(hdr_obj_ptr(&rom, HND3_HDR), CLONE_B_CPU);
    }
}
