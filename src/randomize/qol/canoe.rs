//! W3 canoe softlock fixes (respawn + map-data backup/restore).

use crate::rom::Rom;
use crate::randomize::rom_data::{FS_CANOE_BACKUP, FS_CANOE_RESPAWN, jsr_into_bank};

// Canoe softlock fix — based on "SMB3 - Canoe Softlock Fixes (Open World
// compatible).ips". Two hooks, one JSR retarget, and two free-space
// subroutines.

// Byte offset of Part B (map-data restore) inside CANOE_BACKUP_ROUTINE.
// Part A (backup) is the first 28 bytes; Part B follows at CPU $BD0C.
const CANOE_RESTORE_OFFSET: usize = 28;

// Hook at PRG010 CPU $C6EA (5 bytes): replaces the vanilla `LDA #$00 /
// STA $0500` with `JSR` to Part B of FS_CANOE_BACKUP, which restores the
// backed-up map data and then re-executes the displaced LDA/STA itself.
const CANOE_RESTORE_HOOK: usize = 0x146FA;

// Operand bytes of the vanilla `JSR $D1FE` at PRG010 CPU $CF12 (2 bytes),
// retargeted to FS_CANOE_RESPAWN — which runs the original $D1FE routine
// first, then saves the death-respawn position for canoe entry.
const CANOE_RESPAWN_RETARGET: usize = 0x14F23;

// FS_CANOE_RESPAWN's CPU address: PRG010 is mapped at $C000-$DFFF on the
// world map, so CPU = $C000 + (file - 0x14010) = $DDE0.
const CANOE_RESPAWN_CPU: u16 = (0xC000 + FS_CANOE_RESPAWN - 0x14010) as u16;

// Hook at PRG011 CPU $A22F → JSR FS_CANOE_BACKUP Part A (5 bytes incl. NOP NOP).
const CANOE_BACKUP_HOOK: usize = 0x1623F;

// Record 3: subroutine in PRG010 free space (FS_CANOE_RESPAWN).
// Saves player map position as death respawn point when entering via canoe ($4B).
#[rustfmt::skip]
const CANOE_RESPAWN_ROUTINE: [u8; 35] = [
    0x20, 0xFE, 0xD1, // JSR $D1FE  (the displaced original JSR target)
    0xC9, 0x4B,       // CMP #$4B   (canoe dock tile)
    0xD0, 0x1B,       // BNE +27    (skip if not canoe)
    0xB5, 0x75,       // LDA $75,X  (map obj Y)
    0x9D, 0x7E, 0x79, // STA $797E,X (death respawn Y)
    0xB5, 0x77,       // LDA $77,X  (map obj X hi)
    0x9D, 0x80, 0x79, // STA $7980,X (death respawn X hi)
    0xB5, 0x79,       // LDA $79,X  (map obj X lo)
    0x9D, 0x82, 0x79, // STA $7982,X (death respawn X lo)
    0xA5, 0xFD,       // LDA $FD    (Map_Scroll_X)
    0x9D, 0x86, 0x79, // STA $7986,X (death respawn scroll X)
    0xA5, 0x12,       // LDA $12    (Map_Scroll_XHi)
    0x9D, 0x88, 0x79, // STA $7988,X (death respawn scroll XHi)
    0xA5, 0xE5,       // LDA $E5    (reload game state)
    0x60,             // RTS
];

// Record 5: backup/restore subroutines in PRG011 free space (FS_CANOE_BACKUP).
// Part A ($BCF0): backs up 3 map data values before canoe overwrites them.
// Part B ($BD0C): restores backed-up values when canoe interaction ends.
#[rustfmt::skip]
const CANOE_BACKUP_ROUTINE: [u8; 66] = [
    // Part A: backup on canoe load
    0xC9, 0x10,       // CMP #$10   (canoe obj ID)
    0xD0, 0x12,       // BNE +18    (skip if not canoe)
    0xB9, 0xEB, 0x7E, // LDA $7EEB,Y
    0x8D, 0xF3, 0x7A, // STA $7AF3
    0xB9, 0x07, 0x7F, // LDA $7F07,Y
    0x8D, 0xF1, 0x7A, // STA $7AF1
    0xB9, 0xF9, 0x7E, // LDA $7EF9,Y
    0x8D, 0xF2, 0x7A, // STA $7AF2
    0xB1, 0x06,       // LDA ($06),Y (original instruction)
    0x99, 0x56, 0x79, // STA $7956,Y (original instruction)
    0x60,             // RTS
    // Part B: restore on canoe cleanup
    0xA0, 0x0D,       // LDY #$0D   (iterate backwards)
    0xB9, 0x15, 0x7F, // LDA $7F15,Y (map obj ID)
    0xC9, 0x10,       // CMP #$10   (canoe?)
    0xD0, 0x14,       // BNE +20    (skip if not canoe)
    0xAD, 0xF3, 0x7A, // LDA $7AF3
    0x99, 0xEB, 0x7E, // STA $7EEB,Y (restore)
    0xAD, 0xF1, 0x7A, // LDA $7AF1
    0x99, 0x07, 0x7F, // STA $7F07,Y (restore)
    0xAD, 0xF2, 0x7A, // LDA $7AF2
    0x99, 0xF9, 0x7E, // STA $7EF9,Y (restore)
    0xA0, 0x01,       // LDY #$01   (break loop)
    0x88,             // DEY
    0xD0, 0xE2,       // BNE -30    (loop)
    0xA9, 0x00,       // LDA #$00
    0x8D, 0x00, 0x05, // STA $0500  (clear game state flag)
    0x60,             // RTS
];

/// Fix W3 canoe softlocks: save death respawn position when entering via canoe,
/// and backup/restore the map tile data the canoe overwrites.
///
/// Without this, levels placed on W3 island tiles (freed by spade game removal)
/// can softlock if the player dies — the respawn position is invalid and the map
/// data under the canoe is permanently corrupted.
///
/// Based on "SMB3 - Canoe Softlock Fixes (Open World compatible).ips".
pub fn fix_canoe_softlock(rom: &mut Rom) {
    // Record 1: hook at PRG010 CPU $C6EA → JSR $BD0C (FS_CANOE_BACKUP Part B,
    // the map-data restore), NOP-padded over the displaced 5 bytes.
    let [jsr, lo, hi] = jsr_into_bank(11, FS_CANOE_BACKUP + CANOE_RESTORE_OFFSET);
    rom.write_range(CANOE_RESTORE_HOOK, &[jsr, lo, hi, 0xEA, 0xEA]);

    // Record 2: retarget the vanilla `JSR $D1FE` at CPU $CF12 to
    // FS_CANOE_RESPAWN ($DDE0) — operand bytes only, the JSR opcode stays.
    rom.write_range(CANOE_RESPAWN_RETARGET, &CANOE_RESPAWN_CPU.to_le_bytes());

    // Record 3: respawn-save subroutine
    rom.write_range(FS_CANOE_RESPAWN, &CANOE_RESPAWN_ROUTINE);

    // Record 4: hook at PRG011 CPU $A22F → JSR $BCF0 (FS_CANOE_BACKUP Part A,
    // the backup).
    let [jsr, lo, hi] = jsr_into_bank(11, FS_CANOE_BACKUP);
    rom.write_range(CANOE_BACKUP_HOOK, &[jsr, lo, hi, 0xEA, 0xEA]);

    // Record 5: backup/restore subroutines
    rom.write_range(FS_CANOE_BACKUP, &CANOE_BACKUP_ROUTINE);
}
