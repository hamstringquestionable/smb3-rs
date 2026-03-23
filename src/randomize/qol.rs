use crate::rom::Rom;

/// Starting lives value byte (LDA #imm operand).
/// Both Mario and Luigi are initialized from this single byte.
const STARTING_LIVES_OFFSET: usize = 0x308E1;

// W3 drawbridge map tile offsets (2× $B2 horizontal, 2× $B1 vertical)
const W3_BRIDGE_H1: usize = 0x18777;
const W3_BRIDGE_H2: usize = 0x18779;
const W3_BRIDGE_V1: usize = 0x1880C;
const W3_BRIDGE_V2: usize = 0x188F3;

// Toggle code: LDA $07BB; EOR #$01; STA $07BB (8 bytes at 0x14A68)
const W3_TOGGLE_OFFSET: usize = 0x14A68;
const W3_TOGGLE_LEN: usize = 8;

// W2 rock blocking secret path (screen 1, row 0, col 5) — $51 → $45
const W2_SECRET_ROCK: usize = 0x186E0;

// W3 rock blocking boat path (screen 0, row 6, col 15) — $51 → $45
const W3_BOAT_ROCK: usize = 0x187DB;

// Big ? Block bonus room patch: decouple room selection from World_Num.
//
// Two-part patch:
// Part A — PRG012: At the end of Map_PrepareLevel, save the entry-point obj_ptr
//   to scratch RAM ($7EB4/$7EB5) before any junctions can overwrite ObjPtrOrig.
// Part B — PRG026: Replace `LDY World_Num` in LevelJct_BigQuestionBlock with a
//   JSR to a lookup routine that reads the saved obj_ptr from scratch RAM and
//   maps it to the correct per-world bonus room index.

// Part A: PRG012 trampoline to save entry-point obj_ptr.
// Replaces `LDA #$03; STA World_EnterState; RTS` (6 bytes) with `JMP $BDC0` + NOPs.
const BIG_Q_SAVE_HOOK: usize = 0x1920B;
const BIG_Q_SAVE_JMP: [u8; 6] = [0x4C, 0xC0, 0xBD, 0xEA, 0xEA, 0xEA];
// Trampoline in PRG012 free space (CPU $BDC0 = file 0x19DD0).
const BIG_Q_SAVE_OFFSET: usize = 0x19DD0;
const BIG_Q_SAVE_ROUTINE: [u8; 18] = [
    0xAD, 0xBB, 0x7E, // LDA Level_ObjPtrOrig_AddrL
    0x8D, 0xB4, 0x7E, // STA $7EB4  (scratch: entry obj_lo)
    0xAD, 0xBC, 0x7E, // LDA Level_ObjPtrOrig_AddrH
    0x8D, 0xB5, 0x7E, // STA $7EB5  (scratch: entry obj_hi)
    0xA9, 0x03,        // LDA #$03   (original displaced code)
    0x8D, 0x28, 0x07,  // STA World_EnterState ($0728)
    0x60,              // RTS
];

// Part B: PRG026 lookup routine.
// Hook point: replace `LDY $0727` with `JSR $B520` in LevelJct_BigQuestionBlock.
const BIG_Q_HOOK_OFFSET: usize = 0x349F9;
const BIG_Q_JSR: [u8; 3] = [0x20, 0x20, 0xB5];
// Lookup routine in PRG026 free space (CPU $B520 = file 0x35530).
const BIG_Q_ROUTINE_OFFSET: usize = 0x35530;
// Reads saved entry-point obj_ptr from $7EB4/$7EB5 (not ObjPtrOrig which
// gets overwritten by sub-area junctions). Falls back to World_Num for
// levels not in the table (W1/W2 levels don't use Big ? Blocks).
const BIG_Q_ROUTINE: [u8; 66] = [
    // LDA $7EB5 (saved entry obj_hi)
    0xAD, 0xB5, 0x7E,
    // LDX #10
    0xA2, 0x0A,
    // .loop: CMP $B541,X (obj_hi table)
    0xDD, 0x41, 0xB5,
    // BNE .next (+16)
    0xD0, 0x10,
    // PHA
    0x48,
    // LDA $7EB4 (saved entry obj_lo)
    0xAD, 0xB4, 0x7E,
    // CMP $B54C,X (obj_lo table)
    0xDD, 0x4C, 0xB5,
    // BNE .no_match (+6)
    0xD0, 0x06,
    // PLA
    0x68,
    // LDA $B557,X (room index table)
    0xBD, 0x57, 0xB5,
    // TAY
    0xA8,
    // RTS
    0x60,
    // .no_match: PLA
    0x68,
    // .next: DEX
    0xCA,
    // BPL .loop (-24)
    0x10, 0xE8,
    // fallback: LDY $0727
    0xAC, 0x27, 0x07,
    // RTS
    0x60,
    // obj_hi table (11 entries): 3-5,3-9,4-F2,5-2,5-5,6-3,6-9,6-10,7-F1,7-8,8-1
    0xCD, 0xC3, 0xD5, 0xC8, 0xCB, 0xCA, 0xCD, 0xCC, 0xD4, 0xC3, 0xC4,
    // obj_lo table (11 entries)
    0xEB, 0x8F, 0x08, 0xBE, 0x0A, 0x8E, 0x2D, 0xE8, 0xE4, 0x2D, 0x24,
    // room index table (11 entries): vanilla world indices (0-indexed)
    0x02, 0x02, 0x03, 0x04, 0x04, 0x05, 0x05, 0x05, 0x06, 0x06, 0x07,
];

/// Set starting lives for both Mario and Luigi (1–99).
pub fn set_starting_lives(rom: &mut Rom, lives: u8) {
    let clamped = lives.min(99).max(1);
    rom.write_byte(STARTING_LIVES_OFFSET, clamped);
}

/// Remove the W2 rock blocking the secret path, replacing it with horizontal path.
pub fn remove_w2_rock(rom: &mut Rom) {
    rom.write_byte(W2_SECRET_ROCK, 0x45);
}

/// Remove the W3 rock blocking the boat path, replacing it with horizontal path.
pub fn remove_w3_boat_rock(rom: &mut Rom) {
    rom.write_byte(W3_BOAT_ROCK, 0x45);
}

/// Patch Big ? Block bonus room selection to use level identity instead of World_Num.
///
/// Part A: Saves the entry-point obj_ptr to scratch RAM ($7EB4/$7EB5) at the end of
/// Map_PrepareLevel, before any sub-area junctions can overwrite Level_ObjPtrOrig.
///
/// Part B: Installs a lookup routine in PRG026 free space that reads the saved obj_ptr
/// and maps it to the correct per-world bonus room index. Falls back to World_Num for
/// levels not in the table (W1/W2 levels don't use Big ? Blocks).
pub fn fix_big_q_block_rooms(rom: &mut Rom) {
    // Part A: PRG012 save trampoline
    rom.write_range(BIG_Q_SAVE_HOOK, &BIG_Q_SAVE_JMP);
    rom.write_range(BIG_Q_SAVE_OFFSET, &BIG_Q_SAVE_ROUTINE);
    // Part B: PRG026 lookup routine
    rom.write_range(BIG_Q_HOOK_OFFSET, &BIG_Q_JSR);
    rom.write_range(BIG_Q_ROUTINE_OFFSET, &BIG_Q_ROUTINE);
}

// Card matching tables in PRG009 (mapped at CPU $A000).
// Three 8-entry tables indexed by OR'd card type bitmask:
//   bitmask: 1=mushroom, 2=flower, 4=star → one-of-each = 7
// $A000: lives to award      $A008: cutscene flag (0x40)      $A010: match indicator
const CARD_CUTSCENE_FLAG: usize = 0x1201F; // $A008[7] — cutscene trigger for one-of-each
const CARD_MATCH_INDICATOR: usize = 0x12027; // $A010[7] — match display flag for one-of-each

/// Patch one-of-each card collection to skip the cutscene.
/// Awards +1 life and clears the cards, but the level ends immediately
/// as if the player had fewer than 3 cards — a speed bonus.
pub fn card_speed_clear(rom: &mut Rom) {
    rom.write_byte(CARD_CUTSCENE_FLAG, 0x00); // don't set cutscene flag
    rom.write_byte(CARD_MATCH_INDICATOR, 0x00); // no match indicator
}

/// Replace W3 drawbridge tiles with normal path tiles and NOP the toggle code.
pub fn fix_w3_drawbridges(rom: &mut Rom) {
    // Replace horizontal drawbridge tiles with bridge ($B3, horizontal path)
    rom.write_byte(W3_BRIDGE_H1, 0xB3);
    rom.write_byte(W3_BRIDGE_H2, 0xB3);
    // Replace vertical drawbridge tiles with open path ($BA, vertical-compatible)
    rom.write_byte(W3_BRIDGE_V1, 0xBA);
    rom.write_byte(W3_BRIDGE_V2, 0xBA);
    // NOP out the toggle code (LDA $07BB; EOR #$01; STA $07BB)
    rom.write_range(W3_TOGGLE_OFFSET, &[0xEA; W3_TOGGLE_LEN]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rom::Rom;

    fn make_test_rom() -> Rom {
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;
        data[STARTING_LIVES_OFFSET] = 0x04;
        Rom::from_bytes(&data).unwrap()
    }

    #[test]
    fn test_starting_lives() {
        let mut rom = make_test_rom();
        assert_eq!(rom.read_byte(STARTING_LIVES_OFFSET), 0x04);
        set_starting_lives(&mut rom, 99);
        assert_eq!(rom.read_byte(STARTING_LIVES_OFFSET), 99);
    }

    #[test]
    fn test_starting_lives_clamped() {
        let mut rom = make_test_rom();
        set_starting_lives(&mut rom, 255);
        assert_eq!(rom.read_byte(STARTING_LIVES_OFFSET), 99);
        set_starting_lives(&mut rom, 0);
        assert_eq!(rom.read_byte(STARTING_LIVES_OFFSET), 1);
    }

    #[test]
    fn test_fix_big_q_block_rooms() {
        let mut rom = make_test_rom();
        // Place original bytes at hook points
        rom.write_range(BIG_Q_HOOK_OFFSET, &[0xAC, 0x27, 0x07]);
        rom.write_range(BIG_Q_SAVE_HOOK, &[0xA9, 0x03, 0x8D, 0x28, 0x07, 0x60]);

        fix_big_q_block_rooms(&mut rom);

        // Part A: PRG012 save trampoline
        assert_eq!(rom.read_range(BIG_Q_SAVE_HOOK, 6), &BIG_Q_SAVE_JMP);
        assert_eq!(
            rom.read_range(BIG_Q_SAVE_OFFSET, BIG_Q_SAVE_ROUTINE.len()),
            &BIG_Q_SAVE_ROUTINE
        );

        // Part B: PRG026 lookup routine
        assert_eq!(rom.read_range(BIG_Q_HOOK_OFFSET, 3), &BIG_Q_JSR);
        assert_eq!(
            rom.read_range(BIG_Q_ROUTINE_OFFSET, BIG_Q_ROUTINE.len()),
            &BIG_Q_ROUTINE
        );
        // Spot-check: routine reads $7EB5 (not $7EBC)
        assert_eq!(rom.read_byte(BIG_Q_ROUTINE_OFFSET + 1), 0xB5);
        assert_eq!(rom.read_byte(BIG_Q_ROUTINE_OFFSET + 2), 0x7E);
        // Spot-check: first obj_hi entry is $CD (3-5), last room index is $07 (8-1)
        assert_eq!(rom.read_byte(BIG_Q_ROUTINE_OFFSET + 33), 0xCD);
        assert_eq!(rom.read_byte(BIG_Q_ROUTINE_OFFSET + 65), 0x07);
    }

    #[test]
    fn test_remove_w2_rock() {
        let mut rom = make_test_rom();
        rom.write_byte(W2_SECRET_ROCK, 0x51);
        remove_w2_rock(&mut rom);
        assert_eq!(rom.read_byte(W2_SECRET_ROCK), 0x45);
    }

    #[test]
    fn test_remove_w3_boat_rock() {
        let mut rom = make_test_rom();
        rom.write_byte(W3_BOAT_ROCK, 0x51);
        remove_w3_boat_rock(&mut rom);
        assert_eq!(rom.read_byte(W3_BOAT_ROCK), 0x45);
    }

    #[test]
    fn test_fix_w3_drawbridges() {
        let mut rom = make_test_rom();
        // Place original drawbridge tiles
        rom.write_byte(W3_BRIDGE_H1, 0xB2);
        rom.write_byte(W3_BRIDGE_H2, 0xB2);
        rom.write_byte(W3_BRIDGE_V1, 0xB1);
        rom.write_byte(W3_BRIDGE_V2, 0xB1);
        // Place original toggle code
        rom.write_range(W3_TOGGLE_OFFSET, &[0xAD, 0xBB, 0x07, 0x49, 0x01, 0x8D, 0xBB, 0x07]);

        fix_w3_drawbridges(&mut rom);

        assert_eq!(rom.read_byte(W3_BRIDGE_H1), 0xB3);
        assert_eq!(rom.read_byte(W3_BRIDGE_H2), 0xB3);
        assert_eq!(rom.read_byte(W3_BRIDGE_V1), 0xBA);
        assert_eq!(rom.read_byte(W3_BRIDGE_V2), 0xBA);
        assert_eq!(rom.read_range(W3_TOGGLE_OFFSET, W3_TOGGLE_LEN), &[0xEA; 8]);
    }

    #[test]
    fn test_card_speed_clear() {
        let mut rom = make_test_rom();
        // Place vanilla values
        rom.write_byte(CARD_CUTSCENE_FLAG, 0x40);
        rom.write_byte(CARD_MATCH_INDICATOR, 0x01);

        card_speed_clear(&mut rom);

        assert_eq!(rom.read_byte(CARD_CUTSCENE_FLAG), 0x00);
        assert_eq!(rom.read_byte(CARD_MATCH_INDICATOR), 0x00);
    }
}
