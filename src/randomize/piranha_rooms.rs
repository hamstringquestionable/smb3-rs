//! Piranha-shuffle chest rooms: make the two W7 piranha plant levels
//! (7-P1 / 7-P2) self-rewarding.
//!
//! In vanilla, the plant levels' treasure chest has no `OBJ_TREASURESET`
//! (0xD6) — the chest awards whatever `Level_TreasureItem` holds, which is
//! only set when the level is entered *via* the map-object sprite (the
//! engine copies the sprite slot's item byte on entry). Once piranha shuffle
//! releases these levels into the pool they can be entered like any numbered
//! level tile, and the chest would open on a stale/invalid item.
//!
//! Fix (same mechanism as the W8 Hand rooms, see `hand_rooms.rs`): clone each
//! plant level's tiny chest-room enemy stream into PRG006 free space with an
//! `OBJ_TREASURESET` entry prepended, and repoint the level header's
//! sub-area enemy pointer at the clone. The D6 row byte *is* the item; the
//! defaults reproduce the vanilla sprite rewards (P-Wing / Mushroom) and
//! `items::randomize` re-rolls them when chest randomization is on.

use crate::rom::Rom;
use super::rom_data::{self, FS_PIRANHA_ROOMS, MAP_OBJ_ENTRY_LINKS};
use super::segment_writer::{self, SegmentEntry, SegmentSpec, SortMode};

/// The map-object sprite id of a stationary piranha plant.
pub(super) const PLANT_SPRITE_ID: u8 = 0x07;

/// Clear the vanilla W7 plant sprites (map-object slots 2-3) and their reward
/// bytes. Must run before the overworld builder: it frees the two grid
/// positions for placement (`fixed_positions_for_world` reads sprite positions
/// from the ROM) and makes the slots eligible for redistributed Hammer Bros.
pub fn clear_vanilla_plants(rom: &mut Rom) {
    rom.push_tag("piranha_rooms/clear_vanilla");
    for &(world_idx, slot, _entry_idx) in MAP_OBJ_ENTRY_LINKS {
        rom_data::clear_map_sprite(rom, world_idx, slot);
    }
    rom.pop_tag();
}

/// File offsets of the vanilla chest-room enemy streams (8 bytes each:
/// 1 page byte + 2 entries × 3 bytes + terminator).
const P1_ROOM_SRC: usize = 0x0D0F2; // CPU $D0E2
const P2_ROOM_SRC: usize = 0x0D0EA; // CPU $D0DA

/// File offsets of each plant level's 9-byte layout header in PRG019.
/// Bytes 2-3 hold the `alt_objects` CPU pointer selecting the chest-room
/// enemy stream.
const P1_HDR: usize = 0x27C33;
const P2_HDR: usize = 0x27B30;

/// Each clone: [page][D6 X item][52 X Y][BA X Y][FF] = 11 bytes.
const CLONE_LEN: usize = 11;

/// CPU addresses of the two cloned streams inside PRG006.
const CLONE_P1_CPU: u16 = 0xDA7A; // file 0x0DA8A
const CLONE_P2_CPU: u16 = 0xDA85; // file 0x0DA95

/// Item (row) byte file offsets of the injected OBJ_TREASURESET entries.
/// The D6 entry is first in the stream, so its item byte sits at +3.
pub(super) const P1_ROOM_ITEM: usize = FS_PIRANHA_ROOMS + 3;
pub(super) const P2_ROOM_ITEM: usize = FS_PIRANHA_ROOMS + CLONE_LEN + 3;

/// Vanilla rewards carried by the W7 plant sprites (map-object item table):
/// plant 1 = P-Wing (0x08), plant 2 = Mushroom (0x01). Used as the injected
/// chest defaults so `chest_items: off` seeds keep the vanilla prizes.
const P1_DEFAULT_ITEM: u8 = 0x08;
const P2_DEFAULT_ITEM: u8 = 0x01;

/// Clone both chest-room streams into free space with an `OBJ_TREASURESET`
/// prepended, and repoint the plant levels' sub-area enemy pointers.
pub fn install_treasure_sets(rom: &mut Rom) {
    rom.push_tag("piranha_rooms");

    for (src, hdr, cpu, dst, item, label) in [
        (P1_ROOM_SRC, P1_HDR, CLONE_P1_CPU, FS_PIRANHA_ROOMS, P1_DEFAULT_ITEM,
         "7-P1 cloned treasure room"),
        (P2_ROOM_SRC, P2_HDR, CLONE_P2_CPU, FS_PIRANHA_ROOMS + CLONE_LEN, P2_DEFAULT_ITEM,
         "7-P2 cloned treasure room"),
    ] {
        let page_byte = rom.read_byte(src);
        let src_entries = segment_writer::read_segment(rom, src, 2);
        // D6 goes one column left of the treasure box so it spawns first
        // (mirrors the vanilla Hand-room layout).
        let d6 = SegmentEntry {
            obj_id: 0xD6,
            x: src_entries[0].x.saturating_sub(1),
            y: item,
        };
        let entries = [d6, src_entries[0], src_entries[1]];
        rom.write_byte(dst, page_byte);
        segment_writer::write_segment(rom, &SegmentSpec {
            file_offset: dst,
            original_count: 3,
            entries: &entries,
            label: Some(label),
            sort_mode: SortMode::SortByX,
        }).expect("piranha_rooms: clone write failed");
        rom.write_byte(dst + CLONE_LEN - 1, 0xFF);
        rom.write_range(hdr + 2, &cpu.to_le_bytes());
    }

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

        // Both vanilla chest rooms share the same 8-byte shape.
        let src: [u8; 8] = [0x01, 0x52, 0x0B, 0x13, 0xBA, 0x0C, 0x13, 0xFF];
        data[P1_ROOM_SRC..P1_ROOM_SRC + 8].copy_from_slice(&src);
        data[P2_ROOM_SRC..P2_ROOM_SRC + 8].copy_from_slice(&src);

        // Vanilla headers: alt_objects = $D0E2 (P1) / $D0DA (P2).
        for (hdr, ptr) in [(P1_HDR, 0xD0E2u16), (P2_HDR, 0xD0DAu16)] {
            data[hdr + 2..hdr + 4].copy_from_slice(&ptr.to_le_bytes());
        }

        Rom::from_bytes_lax(&data, true).unwrap()
    }

    fn hdr_obj_ptr(rom: &Rom, off: usize) -> u16 {
        let b = rom.read_range(off, 4);
        u16::from_le_bytes([b[2], b[3]])
    }

    #[test]
    fn test_clones_prepend_treasureset() {
        let mut rom = make_test_rom();
        install_treasure_sets(&mut rom);

        for (dst, item) in [
            (FS_PIRANHA_ROOMS, P1_DEFAULT_ITEM),
            (FS_PIRANHA_ROOMS + CLONE_LEN, P2_DEFAULT_ITEM),
        ] {
            assert_eq!(
                rom.read_range(dst, CLONE_LEN).to_vec(),
                vec![0x01, 0xD6, 0x0A, item, 0x52, 0x0B, 0x13, 0xBA, 0x0C, 0x13, 0xFF],
            );
        }
    }

    #[test]
    fn test_item_offsets_align_with_treasureset() {
        let mut rom = make_test_rom();
        install_treasure_sets(&mut rom);

        assert_eq!(rom.read_byte(P1_ROOM_ITEM - 2), 0xD6);
        assert_eq!(rom.read_byte(P2_ROOM_ITEM - 2), 0xD6);
        assert_eq!(rom.read_byte(P1_ROOM_ITEM), P1_DEFAULT_ITEM);
        assert_eq!(rom.read_byte(P2_ROOM_ITEM), P2_DEFAULT_ITEM);
    }

    #[test]
    fn test_headers_repointed_to_clones() {
        let mut rom = make_test_rom();
        install_treasure_sets(&mut rom);

        assert_eq!(hdr_obj_ptr(&rom, P1_HDR), CLONE_P1_CPU);
        assert_eq!(hdr_obj_ptr(&rom, P2_HDR), CLONE_P2_CPU);
    }
}
