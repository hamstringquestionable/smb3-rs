//! Overworld map patches that are not map tiles: the W3 drawbridge toggle, the
//! W8 canoe sprite, N-cards, and the canoe passing under bridges. The tile
//! edits themselves live with the vanilla map source,
//! `overworld::map_source::vanilla`.

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

/// `Map_Object_Valid_Down` and `Map_Object_Valid_Up` (PRG010, CPU `$D25A`
/// and `$D263`): the tiles Mario may cross moving down or up. Each opens with
/// `TILE_VERTPATH` three times, padding to the left/right lists' length; the
/// first of each is the byte [`canoe_under_bridges`] takes.
const VALID_DOWN_PAD: usize = 0x1526A;
const VALID_UP_PAD: usize = 0x15273;

/// Let the canoe sail under a bridge (Generated Overworld only; user,
/// 2026-10-23). `Map_CheckDoMove` tries the crossing lists before the canoe's
/// water range (`$82-$A9`), so a bridge (`TILE_BRIDGE`, `$B3`) in the up and
/// down lists makes the canoe take a normal two-tile move straight over it,
/// onto the sea beyond. A move is 32 units, one a frame in the canoe, so it
/// never stops on the bridge.
///
/// Safe only where every bridge has the canoe's water on both sides or
/// neither, and none sits on the top or bottom row (the hop would leave the
/// map: vanilla W3's `(0,17)`), and none has a path or node above or below it,
/// which keeps walking Mario and marching Hammer Bros from crossing one
/// upright. Generated maps hold all three (`generated_is_completable`).
pub(crate) fn canoe_under_bridges(rom: &mut Rom) {
    rom.write_byte(VALID_DOWN_PAD, 0xB3);
    rom.write_byte(VALID_UP_PAD, 0xB3);
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

    /// The bridge takes one of the three `TILE_VERTPATH` pads in each list,
    /// and the path stays in both.
    #[test]
    fn canoe_under_bridges_takes_a_pad() {
        let Ok(data) = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes") else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let mut rom = Rom::from_bytes(&data).unwrap();
        for list in [VALID_DOWN_PAD, VALID_UP_PAD] {
            assert_eq!(rom.read_range(list, 3), &[0x46; 3], "vanilla opens with three pads");
        }
        canoe_under_bridges(&mut rom);
        for list in [VALID_DOWN_PAD, VALID_UP_PAD] {
            assert_eq!(rom.read_range(list, 3), &[0xB3, 0x46, 0x46]);
        }
    }

    #[test]
    fn test_fix_w3_drawbridges() {
        let mut rom = make_test_rom();
        rom.write_range(W3_TOGGLE_OFFSET, &[0xAD, 0xBB, 0x07, 0x49, 0x01, 0x8D, 0xBB, 0x07]);

        fix_w3_drawbridges(&mut rom);

        assert_eq!(rom.read_range(W3_TOGGLE_OFFSET, W3_TOGGLE_LEN), &[0xEA; 8]);
    }
}
