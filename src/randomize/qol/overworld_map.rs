//! Overworld map patches that are not map tiles: the W3 drawbridge toggle, the
//! W8 canoe sprite, and N-cards. The tile edits themselves live with the
//! vanilla map source, `overworld::map_source::vanilla`.

use crate::randomize::rom_data::write_map_sprite;
use crate::rom::Rom;

// Toggle code: LDA $07BB; EOR #$01; STA $07BB (8 bytes at 0x14A68)
const W3_TOGGLE_OFFSET: usize = 0x14A68;
const W3_TOGGLE_LEN: usize = 8;

/// Freeze W3's drawbridges down: NOP the code that flips `World3_Bridge` at
/// the end of every W3 turn. The drawbridge tiles themselves are made
/// permanent by the vanilla map source.
pub(crate) fn fix_w3_drawbridges(rom: &mut Rom) {
    // NOP out the toggle code (LDA $07BB; EOR #$01; STA $07BB)
    rom.write_range(W3_TOGGLE_OFFSET, &[0xEA; W3_TOGGLE_LEN]);
}

/// Place the W8 canoe (object ID 0x10) in map-object slot 6, floating at
/// (5,7) beside the mainland dock (5,6). Slot 6 is past the builder's army
/// sprites (slots 2-5), so the overworld writer leaves it intact. This is the
/// boat that makes the screen-0 island docks reachable in-game; its docks are
/// part of the vanilla map source's `8s are Wild` edits.
///
/// Must run before the map source reads sprite positions.
pub(crate) fn place_w8_canoe(rom: &mut Rom) {
    write_map_sprite(rom, 7, 6, 5, 7, 0x10);
}

/// Remove N-card (N-Spade) panels from the overworld map.
///
/// Patches the map-screen code so N-Spade tiles never appear.
/// Original IPS: 3 bytes at 0x016C90 → LDA #$07; NOP.
const N_CARD_OFFSET: usize = 0x016C90;

pub(crate) fn remove_n_cards(rom: &mut Rom) {
    rom.write_range(N_CARD_OFFSET, &[0xA9, 0x07, 0xEA]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::randomize::qol::test_support::make_test_rom;

    #[test]
    fn test_remove_n_cards() {
        let mut rom = make_test_rom();
        rom.write_range(N_CARD_OFFSET, &[0x00, 0x00, 0x00]);
        remove_n_cards(&mut rom);
        assert_eq!(rom.read_range(N_CARD_OFFSET, 3), &[0xA9, 0x07, 0xEA]);
    }

    #[test]
    fn test_fix_w3_drawbridges() {
        let mut rom = make_test_rom();
        rom.write_range(W3_TOGGLE_OFFSET, &[0xAD, 0xBB, 0x07, 0x49, 0x01, 0x8D, 0xBB, 0x07]);

        fix_w3_drawbridges(&mut rom);

        assert_eq!(rom.read_range(W3_TOGGLE_OFFSET, W3_TOGGLE_LEN), &[0xEA; 8]);
    }
}
