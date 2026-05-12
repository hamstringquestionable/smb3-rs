//! W8 Hand treasure-room variety:
//! 1. Clone the shared Hand sub-area enemy stream so 8-Hnd1/2/3 have
//!    independent OBJ_TREASURESET Y-bytes (randomized by `items::randomize`).
//! 2. With a random chance, redirect one Hand to the 3-7 coin heaven
//!    (`$AB4F` / `$CE89` / tileset 11) instead of a treasure room. Only
//!    one Hand can point at the coin heaven per seed, and sometimes none do.
//!
//! The coin heaven's vanilla autoscroll is disabled by `autoscroll.rs` when
//! autoscroll is off — free-scroll makes the redirect more enjoyable.

use rand::Rng;

use crate::rom::Rom;
use super::rom_data::FS_HAND_ROOMS;

/// File offset of the original Hand sub-area enemy stream (11 bytes:
/// 1 page byte + 3 enemy entries × 3 bytes + 1 terminator). CPU $D0CF.
const HAND_ROOM_SRC: usize = 0x0D0DF;
const HAND_ROOM_LEN: usize = 11;

/// File offsets of each Hand level's 9-byte main-level layout header in
/// PRG019. We rewrite bytes 0-3 (`alt_layout` + `alt_objects`) and byte 6
/// (`alt_tileset` nibble) to re-point its sub-area.
const HND1_HDR: usize = 0x27D50;
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

/// 3-7 coin heaven sub-area target. Values taken from the vanilla header of
/// the $BD88 intermediate room that β2 (and 3-7) both reach by pipe.
const COIN_HEAVEN_ALT_LAYOUT:  u16 = 0xAB4F;
const COIN_HEAVEN_ALT_OBJECTS: u16 = 0xCE89;
const COIN_HEAVEN_ALT_TILESET: u8  = 11;

/// Clone the shared Hand sub-area enemy stream and, with probability 3/4,
/// redirect one random Hand level to the 3-7 coin heaven. The coin heaven
/// appears at most once per seed.
pub fn patch_clone_hand_rooms<R: Rng>(rom: &mut Rom, rng: &mut R) {
    // Unconditional: clone enemy data and re-point 8-Hnd2 / 8-Hnd3.
    let src = rom.read_range(HAND_ROOM_SRC, HAND_ROOM_LEN).to_vec();
    rom.write_range(FS_HAND_ROOMS, &src);
    rom.write_range(FS_HAND_ROOMS + HAND_ROOM_LEN, &src);
    rom.write_range(HND2_HDR + 2, &CLONE_A_CPU.to_le_bytes());
    rom.write_range(HND3_HDR + 2, &CLONE_B_CPU.to_le_bytes());

    // Roll: 0 = no redirect, 1..=3 = redirect that Hand to the coin heaven.
    match rng.random_range(..4u8) {
        0 => {}
        1 => redirect_to_coin_heaven(rom, HND1_HDR),
        2 => redirect_to_coin_heaven(rom, HND2_HDR),
        3 => redirect_to_coin_heaven(rom, HND3_HDR),
        _ => unreachable!(),
    }
}

fn redirect_to_coin_heaven(rom: &mut Rom, header_offset: usize) {
    let lay = COIN_HEAVEN_ALT_LAYOUT.to_le_bytes();
    let obj = COIN_HEAVEN_ALT_OBJECTS.to_le_bytes();
    rom.write_range(header_offset,     &lay);
    rom.write_range(header_offset + 2, &obj);
    let byte6 = rom.read_byte(header_offset + 6);
    rom.write_byte(header_offset + 6, (byte6 & 0xF0) | COIN_HEAVEN_ALT_TILESET);
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

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
        for hdr in [HND1_HDR, HND2_HDR, HND3_HDR] {
            data[hdr]     = 0x17;
            data[hdr + 1] = 0xBE;
            data[hdr + 2] = 0xCF;
            data[hdr + 3] = 0xD0;
            data[hdr + 6] = 0x0B;
        }

        Rom::from_bytes_lax(&data, true).unwrap()
    }

    fn hdr_bytes(rom: &Rom, off: usize) -> (u16, u16, u8) {
        let b = rom.read_range(off, 9);
        let lay = u16::from_le_bytes([b[0], b[1]]);
        let obj = u16::from_le_bytes([b[2], b[3]]);
        let ts  = b[6] & 0x0F;
        (lay, obj, ts)
    }

    fn is_coin_heaven(rom: &Rom, hdr: usize) -> bool {
        hdr_bytes(rom, hdr) == (COIN_HEAVEN_ALT_LAYOUT, COIN_HEAVEN_ALT_OBJECTS, COIN_HEAVEN_ALT_TILESET)
    }

    #[test]
    fn test_clones_match_source() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(0);
        patch_clone_hand_rooms(&mut rom, &mut rng);

        let src = rom.read_range(HAND_ROOM_SRC, HAND_ROOM_LEN).to_vec();
        assert_eq!(rom.read_range(FS_HAND_ROOMS, HAND_ROOM_LEN).to_vec(), src);
        assert_eq!(rom.read_range(FS_HAND_ROOMS + HAND_ROOM_LEN, HAND_ROOM_LEN).to_vec(), src);
    }

    #[test]
    fn test_clone_item_byte_offsets_align_with_treasureset() {
        let mut rom = make_test_rom();
        let mut rng = ChaCha8Rng::seed_from_u64(0);
        patch_clone_hand_rooms(&mut rom, &mut rng);

        assert_eq!(rom.read_byte(HAND_ROOM_CLONE_A_ITEM - 2), 0xD6);
        assert_eq!(rom.read_byte(HAND_ROOM_CLONE_B_ITEM - 2), 0xD6);
        assert_eq!(rom.read_byte(HAND_ROOM_CLONE_A_ITEM), 0x03);
        assert_eq!(rom.read_byte(HAND_ROOM_CLONE_B_ITEM), 0x03);
    }

    #[test]
    fn test_coin_heaven_appears_at_most_once_across_all_seeds() {
        // Exhaustively confirm the invariant: either zero or exactly one
        // Hand points at the coin heaven.
        for seed in 0..2000u64 {
            let mut rom = make_test_rom();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            patch_clone_hand_rooms(&mut rom, &mut rng);

            let ch_count = [HND1_HDR, HND2_HDR, HND3_HDR]
                .iter()
                .filter(|&&h| is_coin_heaven(&rom, h))
                .count();
            assert!(ch_count <= 1,
                "seed {seed}: coin heaven appears {ch_count} times, expected 0 or 1");
        }
    }

    #[test]
    fn test_coin_heaven_outcomes_are_reachable() {
        // Each of the 4 outcomes (none / H1 / H2 / H3) must occur across
        // many seeds. Catches bugs where the roll is biased or wrong-sized.
        let mut saw_none = false;
        let mut saw_h1 = false;
        let mut saw_h2 = false;
        let mut saw_h3 = false;
        for seed in 0..2000u64 {
            let mut rom = make_test_rom();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            patch_clone_hand_rooms(&mut rom, &mut rng);

            let h1 = is_coin_heaven(&rom, HND1_HDR);
            let h2 = is_coin_heaven(&rom, HND2_HDR);
            let h3 = is_coin_heaven(&rom, HND3_HDR);
            if !h1 && !h2 && !h3 { saw_none = true; }
            if h1 { saw_h1 = true; }
            if h2 { saw_h2 = true; }
            if h3 { saw_h3 = true; }
        }
        assert!(saw_none && saw_h1 && saw_h2 && saw_h3,
            "outcomes seen: none={saw_none} h1={saw_h1} h2={saw_h2} h3={saw_h3}");
    }

    #[test]
    fn test_non_redirected_hands_keep_treasure_room_pointers() {
        // Regardless of which hand (if any) is the coin heaven, the other
        // two must still point at vanilla $D0CF / clone A / clone B.
        for seed in 0..200u64 {
            let mut rom = make_test_rom();
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            patch_clone_hand_rooms(&mut rom, &mut rng);

            for (hdr, want_obj) in [
                (HND1_HDR, 0xD0CFu16),
                (HND2_HDR, CLONE_A_CPU),
                (HND3_HDR, CLONE_B_CPU),
            ] {
                if is_coin_heaven(&rom, hdr) { continue; }
                let (_, obj, _) = hdr_bytes(&rom, hdr);
                assert_eq!(obj, want_obj,
                    "seed {seed}: non-redirected Hand header 0x{hdr:05X} has obj=${obj:04X}, expected ${want_obj:04X}");
            }
        }
    }
}
