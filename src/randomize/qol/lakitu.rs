//! Defeated Lakitus stay defeated.
//!
//! Vanilla Lakitu never dies. Stomp it and it flips over and falls, but
//! `ObjNorm_Lakitu` catches it once it drops off the bottom of the screen and
//! puts it straight back (`prg004.asm`, CPU $AD2F):
//!
//! ```asm
//!   LDA Horz_Scroll
//!   SUB #$00
//!   STA Objects_X,X
//!   LDA Horz_Scroll_Hi
//!   SBC #$02          ; Two screens back
//!   STA Objects_XHi,X
//!   LDA #OBJSTATE_NORMAL
//!   STA Objects_State,X
//!   ...               ; restore its original Y from Targeting[XY]Val
//!   JSR Level_PrepareNewObject
//! ```
//!
//! There is no respawn *timer* — the delay is however long the fall takes plus
//! the walk back from two screens behind, so defeating one buys a few seconds
//! and nothing more.
//!
//! That matters beyond the nuisance, because Lakitu is exempt from off-screen
//! deletion for as long as it lives:
//!
//! ```asm
//!   LDA Lakitu_Active
//!   BNE PRG004_AD65            ; active -> skip the despawn entirely
//!   JSR Object_DeleteOffScreen ; Lakitu is leaving...
//! ```
//!
//! So a Lakitu holds one of the five general object slots for the whole level,
//! and every Spiny Egg it throws takes another from the same five
//! (`Lakitu_TossEnemy` runs the identical `LDY #$04 ... DEY/BPL` search). Those
//! five slots are also what the level's own enemy stream spawns into, and what
//! a pick-up-able ice block needs — `Level_IceBlock_GrabNew` silently refuses
//! when they are all taken, which is why a busy screen can make a block
//! unliftable.
//!
//! This patch replaces the respawn with a delete. Three bytes, in place:
//!
//! ```asm
//!   JMP Object_SetDeadEmpty
//! ```
//!
//! `Object_SetDeadEmpty` rather than `Object_SetDeadAndNotSpawned` is the whole
//! point — the latter clears the object's `Level_ObjectsSpawned` bit so it
//! comes back when it scrolls into view again, while the former leaves the bit
//! set and the Lakitu stays gone. It is the same "this one is finished" exit
//! the engine already uses for an enemy that has fallen out of the level, so
//! the slot is released through vanilla's own bookkeeping.
//!
//! PRG000 holds `Object_SetDeadEmpty` and is mapped at $C000-$DFFF while object
//! AI runs, so the `JMP` needs no bank switch — the vanilla code being replaced
//! already reaches into the same bank with `JSR Level_PrepareNewObject`.
//!
//! The 38 bytes of respawn code left behind are unreachable while the option is
//! on, but they are still live vanilla code when it is off, so they are not
//! free space and are not claimed as such.

use crate::rom::Rom;

/// `LDA Horz_Scroll` at CPU $AD2F, the first instruction of the respawn block
/// inside `ObjNorm_Lakitu` (PRG004, file base 0x08010).
///
/// Reached only from `CMP #$02 / BLS` on `Objects_YHi` — that is, once a
/// defeated Lakitu has fallen clear of the level — so nothing else routes
/// through these bytes.
const RESPAWN_BLOCK: usize = 0x08D3F;

/// `Object_SetDeadEmpty` (PRG000 CPU $D45E): `LDA #$00 / STA Objects_State,X`.
///
/// The entry *after* the `Level_ObjectsSpawned` clear, so the object is marked
/// dead without being marked respawnable.
const OBJECT_SET_DEAD_EMPTY: u16 = 0xD45E;

/// `JMP Object_SetDeadEmpty`, displacing `LDA Horz_Scroll` (2) + `SEC` (1).
#[rustfmt::skip]
const PERMA_DEATH: [u8; 3] = [
    0x4C,                                   // JMP
    OBJECT_SET_DEAD_EMPTY as u8,
    (OBJECT_SET_DEAD_EMPTY >> 8) as u8,
];

/// Make a defeated Lakitu stay defeated instead of returning two screens back.
pub fn apply_lakitu_stays_down(rom: &mut Rom) {
    rom.write_range(RESPAWN_BLOCK, &PERMA_DEATH);
}

#[cfg(test)]
mod asm_checks {
    use super::*;
    use crate::randomize::rom_data::asm;

    fn vanilla() -> Option<Vec<u8>> {
        std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()
    }

    /// The splice must displace *whole* vanilla instructions, or the tail of a
    /// half-overwritten one is left for the CPU to run as an opcode. Here that
    /// is `LDA Horz_Scroll` (2 bytes) plus `SEC` (1) — exactly 3.
    #[test]
    fn perma_death_jmp_displaces_whole_instructions() {
        let Some(v) = vanilla() else {
            eprintln!("SKIP: requires the ROM, which is not included in the repo");
            return;
        };
        asm::check(&PERMA_DEATH).fragment().hook(&v, RESPAWN_BLOCK, &PERMA_DEATH).assert_ok();
    }

    /// The bytes we replace really are the respawn block, and the address we
    /// jump to really is `Object_SetDeadEmpty` — both read straight out of the
    /// ROM, so a mis-derived offset cannot pass quietly.
    #[test]
    fn patch_targets_match_the_rom() {
        let Some(v) = vanilla() else {
            eprintln!("SKIP: requires the ROM, which is not included in the repo");
            return;
        };
        // ObjNorm_Lakitu: LDA Horz_Scroll ($FD) / SEC / SBC #$00
        assert_eq!(
            &v[RESPAWN_BLOCK..RESPAWN_BLOCK + 5],
            &[0xA5, 0xFD, 0x38, 0xE9, 0x00],
            "respawn block is not where we think it is"
        );
        // Object_SetDeadEmpty: LDA #$00 / STA Objects_State,X / RTS
        let dead = 0x10 + (OBJECT_SET_DEAD_EMPTY as usize - 0xC000);
        assert_eq!(
            &v[dead..dead + 6],
            &[0xA9, 0x00, 0x9D, 0x61, 0x06, 0x60],
            "Object_SetDeadEmpty is not at the address we jump to"
        );
    }
}
