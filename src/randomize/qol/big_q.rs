//! Big ? Block bonus-room selection by level identity (not World_Num).

use crate::rom::Rom;
use crate::randomize::rom_data::{
    FS_BIG_Q_LOOKUP as BIG_Q_ROUTINE_OFFSET,
    FS_BIG_Q_SAVE as BIG_Q_PRG030_OFFSET,
    jsr_into_bank, prg_bank_file_to_cpu,
};

// Big ? Block bonus room patch: decouple room selection from World_Num.
//
// Entering the bonus-room pipe runs `LevelJct_BigQuestionBlock`, which vanilla
// selects with `LDY World_Num`. Move a level to another world (world-order /
// level shuffle) and that opens the wrong world's room — often a void.
//
// Part A — PRG030 (fixed bank): save the entry obj_ptr from $65/$66 to scratch
//   RAM ($7EB4/$7EB5) at level init, before the W8 code clobbers it to $C033.
// Part B — PRG026: replace `LDY World_Num` with a JSR to a two-pass lookup that
//   maps the level to its room. See build_lookup_routine.

// Part A: PRG030 (fixed bank) trampoline for level init.
const BIG_Q_PRG030_HOOK: usize = 0x3C958;  // file offset of CPY #$07
const BIG_Q_PRG030_JMP: [u8; 4] = [0x4C, 0x2C, 0x9F, 0xEA];
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
const BIG_Q_HOOK_OFFSET: usize = 0x349F9;

// obj_ptr -> vanilla bonus-room index (= World_Num 0-7) for every AREA that can
// be the "current area" (`Level_ObjPtrOrig`) when a Big ? Block bonus pipe is
// entered. The 11 base rooms belong to levels 3-5, 3-9, 4-F2, 5-2, 5-5, 6-3,
// 6-9, 6-10, 7-F1, 7-8, 8-1 (keyed on their ENTRY obj_ptr, which pass 2 catches
// via the frozen save for levels entered from their own tile).
//
// Two extra rows exist for the antechamber (lobby-shuffle) levels, whose content
// is reachable through a FOREIGN lobby — so their bonus pipe must resolve by the
// room you're standing in, not the entered tile:
//   * 5-2 sub-area $CE4B -> room 4 (its block is in the entry $C8BE, already
//     above; the sub row is a belt-and-suspenders for the sub-area path).
//   * 6-9 donated interior $C60E -> room 5 (its block lives HERE, not in the
//     entry $CD2D — so without this row, reaching 6-9 via a lobby falls through
//     to World_Num and picks the wrong room).
const BQ_OBJ_HI: [u8; 13] =
    [0xCD, 0xC3, 0xD5, 0xC8, 0xCB, 0xCA, 0xCD, 0xCC, 0xD4, 0xC3, 0xC4, 0xCE, 0xC6];
const BQ_OBJ_LO: [u8; 13] =
    [0xEB, 0x8F, 0x08, 0xBE, 0x0A, 0x8E, 0x2D, 0xE8, 0xE4, 0x2D, 0x24, 0x4B, 0x0E];
const BQ_ROOM: [u8; 13] =
    [0x02, 0x02, 0x03, 0x04, 0x04, 0x05, 0x05, 0x05, 0x06, 0x06, 0x07, 0x04, 0x05];

const BIG_Q_ROUTINE_LEN: usize = 106;

/// Build the PRG026 Big ? Block bonus-room lookup routine.
///
/// **Two-pass lookup.** The room a bonus pipe opens is a property of the *level
/// whose area you're standing in*. The routine resolves that by scanning the
/// obj_ptr table against two sources, in order:
///
/// 1. **`Level_ObjPtrOrig` ($7EBB/$7EBC)** — the obj_ptr of the area the player
///    is *currently in*. `Level_JctInit` writes each area's `Level_AltObjects`
///    here on every junction, so under lobby (antechamber) shuffle — where you
///    enter 5-2's content through a *foreign lobby* and its interior loops back
///    into its entry — this pointer is still 5-2's when you hit the bonus pipe,
///    not the lobby's. That makes the lookup lobby-shuffle-aware **without any
///    per-seed table changes**, and it resolves each level (5-2→4, 6-9→5, …)
///    correctly on its own.
/// 2. **Frozen map-entry ptr ($7EB4/$7EB5)** — saved by Part A before the W8
///    code clobbers `Level_ObjPtrOrig` to $C033. Covers 8-1 and any pipe hit in
///    an area not itself in the table.
/// 3. **`LDY $0727` (World_Num) fallback** — vanilla default, last resort.
///
/// Internal absolute operands are derived from where the routine is written, so
/// it is not origin-locked to a hardcoded CPU address.
fn build_lookup_routine() -> Vec<u8> {
    let base = prg_bank_file_to_cpu(26, BIG_Q_ROUTINE_OFFSET); // routine start (CPU)
    let scan = (base + 0x26).to_le_bytes(); // bq_scan subroutine
    let hi = (base + 0x43).to_le_bytes(); // BQ_OBJ_HI table
    let lo = (base + 0x50).to_le_bytes(); // BQ_OBJ_LO table (13 bytes after hi)
    let room = (base + 0x5D).to_le_bytes(); // BQ_ROOM table (13 bytes after lo)
    let mut r: Vec<u8> = vec![
        // --- pass 1: current area (Level_ObjPtrOrig $7EBB/$7EBC) ---
        0xAD, 0xBB, 0x7E,       // LDA $7EBB     ; current-area obj_lo
        0x8D, 0xB2, 0x7E,       // STA $7EB2     ; scratch lo
        0xAD, 0xBC, 0x7E,       // LDA $7EBC     ; current-area obj_hi
        0x8D, 0xB3, 0x7E,       // STA $7EB3     ; scratch hi
        0x20, scan[0], scan[1], // JSR bq_scan
        0xB0, 0x14,             // BCS .ret      ; matched -> Y = room
        // --- pass 2: frozen map-entry ptr ($7EB4/$7EB5, saved by Part A) ---
        0xAD, 0xB4, 0x7E,       // LDA $7EB4     ; frozen obj_lo
        0x8D, 0xB2, 0x7E,       // STA $7EB2
        0xAD, 0xB5, 0x7E,       // LDA $7EB5     ; frozen obj_hi
        0x8D, 0xB3, 0x7E,       // STA $7EB3
        0x20, scan[0], scan[1], // JSR bq_scan
        0xB0, 0x03,             // BCS .ret
        // --- fallback: World_Num ---
        0xAC, 0x27, 0x07,       // LDY $0727
        0x60,                   // .ret: RTS
        // --- bq_scan: scratch $7EB2=lo/$7EB3=hi -> carry set + Y=room on match ---
        0xA2, 0x0C,             // LDX #12  (13 entries, index 0..12)
        0xAD, 0xB3, 0x7E,       // .loop: LDA $7EB3
        0xDD, hi[0], hi[1],     // CMP BQ_OBJ_HI,X
        0xD0, 0x0E,             // BNE .next
        0xAD, 0xB2, 0x7E,       // LDA $7EB2
        0xDD, lo[0], lo[1],     // CMP BQ_OBJ_LO,X
        0xD0, 0x06,             // BNE .next
        0xBD, room[0], room[1], // LDA BQ_ROOM,X
        0xA8,                   // TAY
        0x38,                   // SEC
        0x60,                   // RTS
        0xCA,                   // .next: DEX
        0x10, 0xE7,             // BPL .loop
        0x18,                   // CLC
        0x60,                   // RTS
    ];
    r.extend_from_slice(&BQ_OBJ_HI);
    r.extend_from_slice(&BQ_OBJ_LO);
    r.extend_from_slice(&BQ_ROOM);
    debug_assert_eq!(r.len(), BIG_Q_ROUTINE_LEN);
    r
}

/// Patch Big ? Block bonus room selection to use level identity instead of World_Num.
pub fn fix_big_q_block_rooms(rom: &mut Rom) {
    // Part A: PRG030 save trampoline (saves $65/$66 before W8 overwrite)
    rom.write_range(BIG_Q_PRG030_HOOK, &BIG_Q_PRG030_JMP);
    rom.write_range(BIG_Q_PRG030_OFFSET, &BIG_Q_PRG030_ROUTINE);
    // Part B: PRG026 two-pass lookup routine + hook
    rom.write_range(BIG_Q_HOOK_OFFSET, &jsr_into_bank(26, BIG_Q_ROUTINE_OFFSET));
    rom.write_range(BIG_Q_ROUTINE_OFFSET, &build_lookup_routine());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::randomize::qol::test_support::make_test_rom;

    #[test]
    fn test_fix_big_q_block_rooms() {
        let mut rom = make_test_rom();
        rom.write_range(BIG_Q_HOOK_OFFSET, &[0xAC, 0x27, 0x07]);
        rom.write_range(BIG_Q_PRG030_HOOK, &[0xC0, 0x07, 0xD0, 0x18]);

        fix_big_q_block_rooms(&mut rom);

        assert_eq!(rom.read_range(BIG_Q_PRG030_HOOK, 4), &BIG_Q_PRG030_JMP);
        assert_eq!(
            rom.read_range(BIG_Q_PRG030_OFFSET, BIG_Q_PRG030_ROUTINE.len()),
            &BIG_Q_PRG030_ROUTINE
        );
        assert_eq!(
            rom.read_range(BIG_Q_HOOK_OFFSET, 3),
            &jsr_into_bank(26, BIG_Q_ROUTINE_OFFSET)
        );
        let expected = build_lookup_routine();
        assert_eq!(
            rom.read_range(BIG_Q_ROUTINE_OFFSET, expected.len()),
            expected.as_slice()
        );
    }

    #[test]
    fn routine_scans_current_area_before_frozen_entry() {
        let r = build_lookup_routine();
        assert_eq!(r.len(), BIG_Q_ROUTINE_LEN);
        assert_eq!(&r[0..3], &[0xAD, 0xBB, 0x7E], "pass 1 LDA $7EBB");
        assert_eq!(&r[6..9], &[0xAD, 0xBC, 0x7E], "pass 1 LDA $7EBC");
        assert_eq!(&r[17..20], &[0xAD, 0xB4, 0x7E], "pass 2 LDA $7EB4");
        assert_eq!(&r[23..26], &[0xAD, 0xB5, 0x7E], "pass 2 LDA $7EB5");
        assert_eq!(&r[34..38], &[0xAC, 0x27, 0x07, 0x60], "fallback LDY $0727; RTS");
        assert_eq!(&r[0x43..0x50], &BQ_OBJ_HI);
        assert_eq!(&r[0x50..0x5D], &BQ_OBJ_LO);
        assert_eq!(&r[0x5D..0x6A], &BQ_ROOM);
    }

    #[test]
    fn jsr_target_matches_routine_origin() {
        let base = prg_bank_file_to_cpu(26, BIG_Q_ROUTINE_OFFSET);
        let hook = jsr_into_bank(26, BIG_Q_ROUTINE_OFFSET);
        assert_eq!(u16::from_le_bytes([hook[1], hook[2]]), base);
        let r = build_lookup_routine();
        assert_eq!(u16::from_le_bytes([r[13], r[14]]), base + 0x26);
    }
}

#[cfg(test)]
mod asm_checks {
    //! Decode each assembled routine and check the structural properties no
    //! assembler was around to enforce. See [`crate::randomize::rom_data::asm`].
    use super::*;
    use crate::randomize::rom_data::asm;

    #[test]
    fn big_q_prg030_is_well_formed() {
        asm::check(&BIG_Q_PRG030_ROUTINE).allocation(BIG_Q_PRG030_OFFSET).assert_ok();
    }
}
