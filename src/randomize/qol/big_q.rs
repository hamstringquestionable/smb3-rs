//! Big ? Block bonus-room selection by level identity (not World_Num).

use crate::rom::Rom;
use crate::randomize::rom_data::{
    FS_BIG_Q_LOOKUP as BIG_Q_ROUTINE_OFFSET,
    FS_BIG_Q_SAVE as BIG_Q_PRG030_OFFSET,
};

// Big ? Block bonus room patch: decouple room selection from World_Num.
//
// Two-part patch:
// Part A — PRG030 (fixed bank): During level init, save the entry-point obj_ptr
//   from $65/$66 to scratch RAM ($7EB4/$7EB5) before the W8-specific code at
//   $894C can overwrite it with a hardcoded $C033. This hook is in the fixed
//   bank so it fires for ALL entry paths (normal tile, army sprite, etc.).
//   The old PRG012 hook was insufficient — it only covered Map_PrepareLevel
//   (enter state #$03) but W8 army sprites use a different path (state #$08).
// Part B — PRG026: Replace `LDY World_Num` in LevelJct_BigQuestionBlock with a
//   JSR to a lookup routine that reads the saved obj_ptr from scratch RAM and
//   maps it to the correct per-world bonus room index.

// Part A: PRG030 (fixed bank) trampoline for level init.
// Saves the entry-point obj_ptr from $65/$66 to scratch RAM $7EB4/$7EB5.
// Hooked in PRG030 (always loaded) so it fires for ALL entry paths — normal
// tile entry, army sprite encounters, and any other mechanism.
// Replaces `CPY #$07; BNE +$18` (4 bytes) with `JMP $9F2C` + NOP.
const BIG_Q_PRG030_HOOK: usize = 0x3C958;  // file offset of CPY #$07
const BIG_Q_PRG030_JMP: [u8; 4] = [0x4C, 0x2C, 0x9F, 0xEA];
// Trampoline in PRG030 free space — offset from rom_data::FS_BIG_Q_SAVE
// (imported as BIG_Q_PRG030_OFFSET at the top of the file).
const BIG_Q_PRG030_ROUTINE: [u8; 20] = [
    0xA5, 0x65,        // LDA $65        (real obj_lo, before W8 overwrite)
    0x8D, 0xB4, 0x7E,  // STA $7EB4
    0xA5, 0x66,        // LDA $66        (real obj_hi)
    0x8D, 0xB5, 0x7E,  // STA $7EB5
    0xC0, 0x07,        // CPY #$07       (displaced: W8 check)
    0xD0, 0x03,        // BNE +3         (skip JMP for non-W8)
    0x4C, 0x4C, 0x89,  // JMP $894C      (W8 path: save + overwrite)
    0x4C, 0x64, 0x89,  // JMP $8964      (non-W8 path: skip overwrite)
];

// Part B: PRG026 lookup routine.
// Hook point: replace `LDY $0727` with `JSR $B520` in LevelJct_BigQuestionBlock.
const BIG_Q_HOOK_OFFSET: usize = 0x349F9;
const BIG_Q_JSR: [u8; 3] = [0x20, 0x20, 0xB5];
// Lookup routine in PRG026 free space — offset from rom_data::FS_BIG_Q_LOOKUP
// (imported as BIG_Q_ROUTINE_OFFSET at the top of the file).
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

/// Patch Big ? Block bonus room selection to use level identity instead of World_Num.
///
/// Part A: Saves the entry-point obj_ptr to scratch RAM ($7EB4/$7EB5) at the end of
/// Map_PrepareLevel, before any sub-area junctions can overwrite Level_ObjPtrOrig.
///
/// Part B: Installs a lookup routine in PRG026 free space that reads the saved obj_ptr
/// and maps it to the correct per-world bonus room index. Falls back to World_Num for
/// levels not in the table (W1/W2 levels don't use Big ? Blocks).
pub fn fix_big_q_block_rooms(rom: &mut Rom) {
    // Part A: PRG030 save trampoline (saves $65/$66 before W8 overwrite)
    rom.write_range(BIG_Q_PRG030_HOOK, &BIG_Q_PRG030_JMP);
    rom.write_range(BIG_Q_PRG030_OFFSET, &BIG_Q_PRG030_ROUTINE);
    // Part B: PRG026 lookup routine
    rom.write_range(BIG_Q_HOOK_OFFSET, &BIG_Q_JSR);
    rom.write_range(BIG_Q_ROUTINE_OFFSET, &BIG_Q_ROUTINE);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::randomize::qol::test_support::make_test_rom;

    #[test]
    fn test_fix_big_q_block_rooms() {
        let mut rom = make_test_rom();
        // Place original bytes at hook points
        rom.write_range(BIG_Q_HOOK_OFFSET, &[0xAC, 0x27, 0x07]);
        rom.write_range(BIG_Q_PRG030_HOOK, &[0xC0, 0x07, 0xD0, 0x18]);

        fix_big_q_block_rooms(&mut rom);

        // Part A: PRG030 save trampoline
        assert_eq!(rom.read_range(BIG_Q_PRG030_HOOK, 4), &BIG_Q_PRG030_JMP);
        assert_eq!(
            rom.read_range(BIG_Q_PRG030_OFFSET, BIG_Q_PRG030_ROUTINE.len()),
            &BIG_Q_PRG030_ROUTINE
        );
        // Spot-check: trampoline reads $65 (zp obj_lo)
        assert_eq!(rom.read_byte(BIG_Q_PRG030_OFFSET), 0xA5);
        assert_eq!(rom.read_byte(BIG_Q_PRG030_OFFSET + 1), 0x65);

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
}
