//! Card (N-Spade) one-of-each speed clear.

use crate::rom::Rom;
use crate::randomize::rom_data::FS_CARD_CLEAR as CARD_TRAMPOLINE;

// Card speed clear: one-of-each detection via XOR check.
//
// Card code lives in a bank mapped at CPU $A000. When 3 cards are collected,
// $BCD8 checks the 3rd slot and sets up a ~255-frame animation + 1-UP.
// We hook at $BCD8 (5 bytes: LDA $7D9E,Y; BEQ $BCFF) to jump to a
// trampoline in PRG031 dead space ($FFE0, always mapped at $E000-$FFFF).
//
// The trampoline executes the displaced 3rd-card check, then XORs all 3
// card values: card[0] ^ card[1] ^ card[2]. For one-of-each (values 1,2,3
// in any order): 1^2^3 = 0. All other combos produce non-zero.
// If zero → JMP $BD5A (clear cards + RTS, no animation).
// Otherwise → execute displaced state setup and JMP $BCE1 (normal flow).
//
// Bank 9 map-screen tables are also patched as belt-and-suspenders:
// lives=0, cutscene flag=0, match indicator=0 for bitmask index 7.

// Hook point: $BCD8 in card bank at $A000 (file 0x05CE8)
// Original 5 bytes: LDA $7D9E,Y (B9 9E 7D); BEQ $BCFF (F0 22)
const CARD_HOOK: usize = 0x05CE8;

// Trampoline in PRG031 dead space — offset from rom_data::FS_CARD_CLEAR
// (imported as CARD_TRAMPOLINE at the top of the file).
// Overwrites 3 unused $FF bytes + "SUPER MARIO 3" string + dead padding.
// 26 bytes available ($FFE0-$FFF9), routine uses 26.

// Bank 9 map-screen tables (belt-and-suspenders)
const CARD_LIVES_AWARD: usize = 0x12017; // $A000[7]
const CARD_CUTSCENE_FLAG: usize = 0x1201F; // $A008[7]
const CARD_MATCH_INDICATOR: usize = 0x12027; // $A010[7]
const CARD_CLEAR_GUARD: usize = 0x12090; // BEQ at $A080

/// Patch one-of-each card collection to skip the animation entirely.
/// Cards are cleared instantly and the level ends as if < 3 cards — a speed bonus.
/// Other mixed combos and matching triples still play the normal animation.
pub fn card_speed_clear(rom: &mut Rom) {
    // Hook: replace 5 bytes at $BCD8 with JMP $FFE0; NOP; NOP
    rom.write_range(CARD_HOOK, &[
        0x4C, 0xE0, 0xFF, // JMP $FFE0
        0xEA, 0xEA,        // NOP; NOP (pad to 5 bytes)
    ]);

    // Trampoline at $FFE0 (PRG031, always mapped, 24 bytes):
    //
    // $FFE0: LDA $7D9E,Y      ; displaced: load 3rd card
    // $FFE3: BNE +3            ; if not empty → check cards
    // $FFE5: JMP $BCFF          ; displaced: 3rd card empty → card placement
    // $FFE8: EOR $7D9C,Y       ; A has card[2], XOR card[0]
    // $FFEB: EOR $7D9D,Y       ; XOR card[1]
    // $FFEE: BNE +3            ; non-zero = not one-of-each
    // $FFF0: JMP $BD5A          ; one-of-each: clear cards, RTS
    // $FFF3: LDA #$04           ; displaced: animation state = 4
    // $FFF5: STA $9A,X          ; displaced: store state
    // $FFF7: JMP $BCE1          ; return to normal animation flow
    rom.write_range(CARD_TRAMPOLINE, &[
        0xB9, 0x9E, 0x7D, // LDA $7D9E,Y (displaced)
        0xD0, 0x03,        // BNE +3 (card present)
        0x4C, 0xFF, 0xBC, // JMP $BCFF (displaced: empty → placement)
        0x59, 0x9C, 0x7D, // EOR $7D9C,Y
        0x59, 0x9D, 0x7D, // EOR $7D9D,Y
        0xD0, 0x03,        // BNE +3 (not one-of-each)
        0x4C, 0x5A, 0xBD, // JMP $BD5A (clear cards)
        0xA9, 0x04,        // LDA #$04 (displaced)
        0x95, 0x9A,        // STA $9A,X (displaced)
        0x4C, 0xE1, 0xBC, // JMP $BCE1 (continue normal)
    ]);

    // Bank 9 map-screen patches (belt-and-suspenders for map cutscene)
    rom.write_byte(CARD_LIVES_AWARD, 0x00);
    rom.write_byte(CARD_CUTSCENE_FLAG, 0x00);
    rom.write_byte(CARD_MATCH_INDICATOR, 0x00);
    rom.write_range(CARD_CLEAR_GUARD, &[0xEA, 0xEA]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::randomize::qol::test_support::make_test_rom;

    #[test]
    fn test_card_speed_clear() {
        let mut rom = make_test_rom();
        // Place vanilla values
        rom.write_range(CARD_HOOK, &[0xB9, 0x9E, 0x7D, 0xF0, 0x22]);
        rom.write_byte(CARD_LIVES_AWARD, 0x01);
        rom.write_byte(CARD_CUTSCENE_FLAG, 0x40);
        rom.write_byte(CARD_MATCH_INDICATOR, 0x01);
        rom.write_range(CARD_CLEAR_GUARD, &[0xF0, 0x0D]);

        card_speed_clear(&mut rom);

        // Hook: JMP $FFE0; NOP; NOP
        assert_eq!(rom.read_range(CARD_HOOK, 5), &[0x4C, 0xE0, 0xFF, 0xEA, 0xEA]);
        // Trampoline: 24 bytes at PRG031 dead space
        assert_eq!(rom.read_byte(CARD_TRAMPOLINE), 0xB9); // LDA $7D9E,Y
        assert_eq!(rom.read_byte(CARD_TRAMPOLINE + 2), 0x7D);
        // One-of-each path: JMP $BD5A
        assert_eq!(rom.read_range(CARD_TRAMPOLINE + 16, 3), &[0x4C, 0x5A, 0xBD]);
        // Normal path tail: JMP $BCE1
        assert_eq!(rom.read_range(CARD_TRAMPOLINE + 23, 3), &[0x4C, 0xE1, 0xBC]);
        // Bank 9 belt-and-suspenders
        assert_eq!(rom.read_byte(CARD_LIVES_AWARD), 0x00);
        assert_eq!(rom.read_byte(CARD_CUTSCENE_FLAG), 0x00);
        assert_eq!(rom.read_byte(CARD_MATCH_INDICATOR), 0x00);
        assert_eq!(rom.read_range(CARD_CLEAR_GUARD, 2), &[0xEA, 0xEA]);
    }
}
