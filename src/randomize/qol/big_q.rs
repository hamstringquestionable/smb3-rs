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
#[rustfmt::skip]
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
pub(crate) const BQ_OBJ_HI: [u8; 13] =
    [0xCD, 0xC3, 0xD5, 0xC8, 0xCB, 0xCA, 0xCD, 0xCC, 0xD4, 0xC3, 0xC4, 0xCE, 0xC6];
pub(crate) const BQ_OBJ_LO: [u8; 13] =
    [0xEB, 0x8F, 0x08, 0xBE, 0x0A, 0x8E, 0x2D, 0xE8, 0xE4, 0x2D, 0x24, 0x4B, 0x0E];
pub(crate) const BQ_ROOM: [u8; 13] =
    [0x02, 0x02, 0x03, 0x04, 0x04, 0x05, 0x05, 0x05, 0x06, 0x06, 0x07, 0x04, 0x05];

/// Vanilla arrival spawn bytes per row: `(byte1, byte2)` where byte1 is
/// `(Y-start index << 4) | pipe-exit dir` and byte2 is `(col << 4) | screen`.
/// The screen nibble is what picks the room inside the area.
///
/// Harvested from each host's own Big ? junction command. Rows 11 and 12 mirror
/// rows 3 and 6 (the same two levels reached through a lobby).
pub(crate) const BQ_ARRIVE: [(u8, u8); 13] = [
    (0x02, 0x14), // 3-5   -> BigQ3 s4
    (0x02, 0x15), // 3-9   -> BigQ3 s5
    (0x52, 0x22), // 4-F2  -> BigQ4 s2
    (0x02, 0x73), // 5-2   -> BigQ5 s3
    (0x02, 0x17), // 5-5   -> BigQ5 s7
    (0x52, 0x25), // 6-3   -> BigQ6 s5
    (0x02, 0x83), // 6-9   -> BigQ6 s3
    (0x12, 0xD6), // 6-10  -> BigQ6 s6
    (0x02, 0x16), // 7-F1  -> BigQ7 s6
    (0x52, 0x14), // 7-8   -> BigQ7 s4
    (0x02, 0xD4), // 8-1   -> BigQ8 s4
    (0x02, 0x73), // 5-2 sub-area — mirrors row 3
    (0x02, 0x83), // 6-9 interior — mirrors row 6
];

/// Vanilla return spawn bytes per row, in the same `(byte1, byte2)` format but
/// naming a position in the HOST. Harvested from each room's own group-7
/// command in the bonus area (slot = the room's screen).
///
/// A return position is not tied to the pipe you came down — it is a
/// hand-authored safe spot. 5-2's pipe is on screen 4 and its return lands on
/// screen 5; that is vanilla, not a bug.
pub(crate) const BQ_RETURN: [(u8, u8); 13] = [
    (0x71, 0x46), // 3-5   <- BigQ3 slot 4
    (0x61, 0x76), // 3-9   <- BigQ3 slot 5
    (0x51, 0x86), // 4-F2  <- BigQ4 slot 2
    (0x61, 0x65), // 5-2   <- BigQ5 slot 3
    (0x42, 0xE5), // 5-5   <- BigQ5 slot 7
    (0x11, 0xA3), // 6-3   <- BigQ6 slot 5
    (0x61, 0x64), // 6-9   <- BigQ6 slot 3
    (0x61, 0xE3), // 6-10  <- BigQ6 slot 6
    (0x61, 0xC6), // 7-F1  <- BigQ7 slot 6
    (0x61, 0x29), // 7-8   <- BigQ7 slot 4
    (0x51, 0xF5), // 8-1   <- BigQ8 slot 4
    (0x61, 0x65), // 5-2 sub-area — mirrors row 3
    (0x61, 0x64), // 6-9 interior — mirrors row 6
];

/// Rows that must move together: reaching a level through a lobby has to open
/// the same room as reaching it from its own tile. `(primary, mirror)`.
#[cfg(test)]
pub(crate) const BQ_MIRROR_ROWS: [(usize, usize); 2] = [(3, 11), (6, 12)];

// Byte offsets of the pieces inside the assembled routine. `apply_room_rooms`
// rewrites the four payload tables in place; the seeding code and the scan
// never change.
const OFF_HIT: usize = 0x09;
const OFF_EXIT: usize = 0x1C;
const OFF_LOOKUP: usize = 0x32;
const OFF_SCAN: usize = 0x53;
const OFF_HI: usize = 0x6C;
const OFF_LO: usize = 0x79;
pub(crate) const OFF_ROOM: usize = 0x86;
pub(crate) const OFF_ARR_Y: usize = 0x93;
pub(crate) const OFF_ARR_X: usize = 0xA0;
pub(crate) const OFF_RET_Y: usize = 0xAD;
pub(crate) const OFF_RET_X: usize = 0xBA;
const BIG_Q_ROUTINE_LEN: usize = 0xC7; // 199

// Zero page / RAM the routine touches.
const PLAYER_XHI: u8 = 0x75;
const JCT_YLH_START: u16 = 0x7F54; // 16-byte slot array
const JCT_XLH_START: u16 = 0x7F64; // 16-byte slot array

/// `JMP PRG026_AA8A` at the tail of the Big ? exit path (`PRG026_AA5A`), which
/// runs only when leaving a bonus room. Swapping this 3-byte jump for a jump
/// into our own routine displaces whole instructions and needs no NOP padding.
pub(crate) const BIG_Q_EXIT_HOOK: usize = 0x34A84;
const BIG_Q_EXIT_RETURN: u16 = 0xAA8A;

/// Build the PRG026 Big ? Block lookup + slot-seeding routine.
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
/// **Slot seeding.** On a match the routine also stamps the row's spawn bytes
/// into `Level_JctYLHStart` / `Level_JctXLHStart` at the slot the engine is
/// about to read (`Player_XHi`). That is what lets a host open *any* room: the
/// arrival no longer has to come from the host's own group-7 layout command, so
/// no layout data is located or edited. The exit half does the same with the
/// return bytes, hooked at the Big ?-only exit path where the pointer restore
/// has just put the host's own obj_ptr back into `Level_ObjPtrOrig`.
///
/// **Both halves share `bq_lookup`, and must.** 7-F1's bonus pipe is not in the
/// area its map tile enters — it sits in the alternate area ($B3CB), whose
/// `Level_ObjPtrOrig` is the shared `Empty_ObjLayout` ($C006). Pass 1 misses
/// there and only pass 2 resolves it, so an exit half that scanned the current
/// area alone would seed nothing and hand the player back a stale slot. Sharing
/// the subroutine also makes the two halves resolve the same row by
/// construction: neither `$7EBB` nor `$7EB4` changes while the bonus room is
/// open, so exit sees exactly the inputs entry saw.
///
/// Internal absolute operands are derived from where the routine is written, so
/// it is not origin-locked to a hardcoded CPU address.
fn build_lookup_routine() -> Vec<u8> {
    build_routine_with(&BQ_ROOM, &BQ_ARRIVE, &BQ_RETURN)
}

fn build_routine_with(
    rooms: &[u8; 13],
    arrive: &[(u8, u8); 13],
    ret: &[(u8, u8); 13],
) -> Vec<u8> {
    let base = prg_bank_file_to_cpu(26, BIG_Q_ROUTINE_OFFSET);
    let at = |off: usize| (base + off as u16).to_le_bytes();
    let (lookup, scan, hi, lo) = (at(OFF_LOOKUP), at(OFF_SCAN), at(OFF_HI), at(OFF_LO));
    let (room_t, arr_y, arr_x) = (at(OFF_ROOM), at(OFF_ARR_Y), at(OFF_ARR_X));
    let (ret_y, ret_x) = (at(OFF_RET_Y), at(OFF_RET_X));
    let ylh = JCT_YLH_START.to_le_bytes();
    let xlh = JCT_XLH_START.to_le_bytes();
    let back = BIG_Q_EXIT_RETURN.to_le_bytes();

    #[rustfmt::skip]
    let mut r: Vec<u8> = vec![
        // --- entry ($00): reached from the replaced `LDY World_Num` ---
        0x20, lookup[0], lookup[1], // JSR bq_lookup
        0xB0, 0x04,             // BCS .hit
        // --- fallback: World_Num ---
        0xAC, 0x27, 0x07,       // LDY $0727
        0x60,                   // RTS
        // --- .hit ($09): X = row. Seed the arrival, return the area in Y ---
        0xA4, PLAYER_XHI,       // LDY Player_XHi   ; the slot the engine reads
        0xBD, arr_y[0], arr_y[1], // LDA BQ_ARR_Y,X
        0x99, ylh[0], ylh[1],   // STA Level_JctYLHStart,Y
        0xBD, arr_x[0], arr_x[1], // LDA BQ_ARR_X,X
        0x99, xlh[0], xlh[1],   // STA Level_JctXLHStart,Y
        0xBD, room_t[0], room_t[1], // LDA BQ_ROOM,X
        0xA8,                   // TAY
        0x60,                   // RTS
        // --- exit seed ($1C): reached from the Big ?-only exit path. The
        //     pointer restore just before it put the HOST's obj_ptr back. ---
        0x20, lookup[0], lookup[1], // JSR bq_lookup
        0x90, 0x0E,             // BCC .done   (not ours — leave vanilla alone)
        0xA4, PLAYER_XHI,       // LDY Player_XHi   ; the room's screen
        0xBD, ret_y[0], ret_y[1], // LDA BQ_RET_Y,X
        0x99, ylh[0], ylh[1],   // STA Level_JctYLHStart,Y
        0xBD, ret_x[0], ret_x[1], // LDA BQ_RET_X,X
        0x99, xlh[0], xlh[1],   // STA Level_JctXLHStart,Y
        0x4C, back[0], back[1], // .done: JMP PRG026_AA8A
        // --- bq_lookup ($32): two passes -> carry set + X = row on match ---
        // pass 1: current area (Level_ObjPtrOrig $7EBB/$7EBC)
        0xAD, 0xBB, 0x7E,       // LDA $7EBB     ; current-area obj_lo
        0x8D, 0xB2, 0x7E,       // STA $7EB2     ; scratch lo
        0xAD, 0xBC, 0x7E,       // LDA $7EBC     ; current-area obj_hi
        0x8D, 0xB3, 0x7E,       // STA $7EB3     ; scratch hi
        0x20, scan[0], scan[1], // JSR bq_scan
        0xB0, 0x0F,             // BCS .out
        // pass 2: frozen map-entry ptr ($7EB4/$7EB5, saved by Part A)
        0xAD, 0xB4, 0x7E,       // LDA $7EB4     ; frozen obj_lo
        0x8D, 0xB2, 0x7E,       // STA $7EB2
        0xAD, 0xB5, 0x7E,       // LDA $7EB5     ; frozen obj_hi
        0x8D, 0xB3, 0x7E,       // STA $7EB3
        0x4C, scan[0], scan[1], // JMP bq_scan   ; its RTS returns to our caller
        0x60,                   // .out: RTS
        // --- bq_scan ($53): $7EB2/$7EB3 -> carry set + X = row on match ---
        0xA2, 0x0C,             // LDX #12  (13 entries, index 0..12)
        0xAD, 0xB3, 0x7E,       // .loop: LDA $7EB3
        0xDD, hi[0], hi[1],     // CMP BQ_OBJ_HI,X
        0xD0, 0x0A,             // BNE .next
        0xAD, 0xB2, 0x7E,       // LDA $7EB2
        0xDD, lo[0], lo[1],     // CMP BQ_OBJ_LO,X
        0xD0, 0x02,             // BNE .next
        0x38,                   // SEC
        0x60,                   // RTS
        0xCA,                   // .next: DEX
        0x10, 0xEB,             // BPL .loop
        0x18,                   // CLC
        0x60,                   // RTS
    ];
    r.extend_from_slice(&BQ_OBJ_HI);
    r.extend_from_slice(&BQ_OBJ_LO);
    r.extend_from_slice(rooms);
    r.extend(arrive.iter().map(|&(y, _)| y));
    r.extend(arrive.iter().map(|&(_, x)| x));
    r.extend(ret.iter().map(|&(y, _)| y));
    r.extend(ret.iter().map(|&(_, x)| x));
    debug_assert_eq!(r.len(), BIG_Q_ROUTINE_LEN);
    // The three hand-written branch displacements, checked against the labels
    // they are meant to reach rather than against the byte that was typed.
    debug_assert_eq!(0x05 + r[0x04] as usize, OFF_HIT, "entry BCS .hit");
    debug_assert_eq!(0x21 + r[0x20] as usize, OFF_LOOKUP - 3, "exit BCC .done");
    debug_assert_eq!(0x43 + r[0x42] as usize, OFF_SCAN - 1, "pass 1 BCS .out");
    r
}

/// Patch Big ? Block bonus room selection to use level identity instead of
/// World_Num, and seed the spawn slots from our own tables.
pub fn fix_big_q_block_rooms(rom: &mut Rom) {
    // Part A: PRG030 save trampoline (saves $65/$66 before W8 overwrite)
    rom.write_range(BIG_Q_PRG030_HOOK, &BIG_Q_PRG030_JMP);
    rom.write_range(BIG_Q_PRG030_OFFSET, &BIG_Q_PRG030_ROUTINE);
    // Part B: PRG026 two-pass lookup routine + hook
    rom.write_range(BIG_Q_HOOK_OFFSET, &jsr_into_bank(26, BIG_Q_ROUTINE_OFFSET));
    rom.write_range(BIG_Q_ROUTINE_OFFSET, &build_lookup_routine());
    // Part C: the exit path's `JMP PRG026_AA8A` -> our return seeder
    let exit = prg_bank_file_to_cpu(26, BIG_Q_ROUTINE_OFFSET) + OFF_EXIT as u16;
    rom.write_range(BIG_Q_EXIT_HOOK, &[0x4C, exit as u8, (exit >> 8) as u8]);
}

/// Rewrite the four per-row payload tables in the already-written routine.
///
/// `fix_big_q_block_rooms` must have run first — this only stamps over the
/// tables, leaving the code and the obj_ptr key tables untouched.
pub(crate) fn write_room_tables(
    rom: &mut Rom,
    rooms: &[u8; 13],
    arrive: &[(u8, u8); 13],
    ret: &[(u8, u8); 13],
) {
    let base = BIG_Q_ROUTINE_OFFSET;
    rom.write_range(base + OFF_ROOM, rooms);
    for (i, &(y, x)) in arrive.iter().enumerate() {
        rom.write_byte(base + OFF_ARR_Y + i, y);
        rom.write_byte(base + OFF_ARR_X + i, x);
    }
    for (i, &(y, x)) in ret.iter().enumerate() {
        rom.write_byte(base + OFF_RET_Y + i, y);
        rom.write_byte(base + OFF_RET_X + i, x);
    }
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
        // Part C names `.exit`, not the origin — `asm::check` validates branches
        // *inside* the routine and cannot see a hook that lands mid-instruction.
        let exit = prg_bank_file_to_cpu(26, BIG_Q_ROUTINE_OFFSET) + OFF_EXIT as u16;
        assert_eq!(
            rom.read_range(BIG_Q_EXIT_HOOK, 3),
            &[0x4C, exit as u8, (exit >> 8) as u8]
        );
    }

    #[test]
    fn routine_scans_current_area_before_frozen_entry() {
        let r = build_lookup_routine();
        assert_eq!(r.len(), BIG_Q_ROUTINE_LEN);
        let l = OFF_LOOKUP;
        assert_eq!(&r[l..l + 3], &[0xAD, 0xBB, 0x7E], "pass 1 LDA $7EBB");
        assert_eq!(&r[l + 6..l + 9], &[0xAD, 0xBC, 0x7E], "pass 1 LDA $7EBC");
        assert_eq!(&r[l + 17..l + 20], &[0xAD, 0xB4, 0x7E], "pass 2 LDA $7EB4");
        assert_eq!(&r[l + 23..l + 26], &[0xAD, 0xB5, 0x7E], "pass 2 LDA $7EB5");
        assert_eq!(&r[5..OFF_HIT], &[0xAC, 0x27, 0x07, 0x60], "fallback LDY $0727; RTS");
        assert_eq!(&r[OFF_HI..OFF_LO], &BQ_OBJ_HI);
        assert_eq!(&r[OFF_LO..OFF_ROOM], &BQ_OBJ_LO);
        assert_eq!(&r[OFF_ROOM..OFF_ARR_Y], &BQ_ROOM);
    }

    /// The bug that shipped in the first cut of the room shuffle: the exit half
    /// scanned only the current area, so 7-F1 — whose bonus pipe lives in an
    /// alternate area with a shared `Empty_ObjLayout` pointer — never got its
    /// return bytes seeded and came back to a stale slot. Both halves must call
    /// the same two-pass subroutine.
    #[test]
    fn entry_and_exit_share_the_two_pass_lookup() {
        let r = build_lookup_routine();
        let lookup = prg_bank_file_to_cpu(26, BIG_Q_ROUTINE_OFFSET) + OFF_LOOKUP as u16;
        let jsr = [0x20, lookup as u8, (lookup >> 8) as u8];
        assert_eq!(&r[0..3], &jsr, "entry JSR bq_lookup");
        assert_eq!(&r[OFF_EXIT..OFF_EXIT + 3], &jsr, "exit JSR bq_lookup");
    }

    /// The hook must name the routine's own origin. The routine's *internal*
    /// self-references are checked by `lookup_routine_is_well_formed`, which
    /// resolves them against the decoded instruction boundaries rather than
    /// against a hand-written byte index.
    #[test]
    fn jsr_target_matches_routine_origin() {
        let base = prg_bank_file_to_cpu(26, BIG_Q_ROUTINE_OFFSET);
        let hook = jsr_into_bank(26, BIG_Q_ROUTINE_OFFSET);
        assert_eq!(u16::from_le_bytes([hook[1], hook[2]]), base);
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

    /// The PRG026 lookup routine, whose internal operands are derived from
    /// `base` rather than hardcoded — so `.origin` here proves the derivation
    /// lands on real instruction boundaries, not merely on the right numbers.
    /// The last 91 bytes are the seven 13-entry tables.
    #[test]
    fn lookup_routine_is_well_formed() {
        let routine = build_lookup_routine();
        asm::check(&routine)
            .allocation(BIG_Q_ROUTINE_OFFSET)
            .origin(prg_bank_file_to_cpu(26, BIG_Q_ROUTINE_OFFSET))
            .data_from(routine.len() - 91)
            .assert_ok();
    }

    fn vanilla() -> Option<Vec<u8>> {
        std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()
    }

    /// All three hook sites, against the vanilla bytes they overwrite. The
    /// structural checks above see only the routine; nothing there can tell
    /// that a hook chopped a vanilla instruction in half and left its operand
    /// for the CPU to run as an opcode. Split out because it needs the ROM.
    ///
    /// The exit hook is checked as a *fragment* starting at `OFF_EXIT`, because
    /// `.hook` requires the hook to name the checked array's origin and this
    /// one deliberately names a label partway into the routine.
    #[test]
    fn hooks_displace_whole_instructions() {
        let Some(v) = vanilla() else {
            eprintln!("SKIP: requires the ROM, which is not included in the repo");
            return;
        };
        let base = prg_bank_file_to_cpu(26, BIG_Q_ROUTINE_OFFSET);
        let routine = build_lookup_routine();

        // Part A: PRG030 save trampoline, reached by `JMP` over `CPY #$07`.
        asm::check(&BIG_Q_PRG030_ROUTINE)
            .allocation(BIG_Q_PRG030_OFFSET)
            .origin(crate::randomize::rom_data::prg030_file_to_cpu(BIG_Q_PRG030_OFFSET))
            .hook(&v, BIG_Q_PRG030_HOOK, &BIG_Q_PRG030_JMP)
            .assert_ok();

        // Part B: the entry hook, replacing `LDY World_Num`.
        asm::check(&routine)
            .origin(base)
            .data_from(routine.len() - 91)
            .hook(&v, BIG_Q_HOOK_OFFSET, &jsr_into_bank(26, BIG_Q_ROUTINE_OFFSET))
            .assert_ok();

        // Part C: the exit hook, replacing the tail `JMP PRG026_AA8A`.
        let exit_cpu = base + OFF_EXIT as u16;
        let exit_jmp = [0x4C, exit_cpu as u8, (exit_cpu >> 8) as u8];
        asm::check(&routine[OFF_EXIT..OFF_LOOKUP])
            .origin(exit_cpu)
            .hook(&v, BIG_Q_EXIT_HOOK, &exit_jmp)
            .assert_ok();
    }
}
