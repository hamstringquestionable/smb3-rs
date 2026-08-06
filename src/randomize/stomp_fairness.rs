//! Stop a *rising* enemy from turning a stomp into damage.
//!
//! Landing on an enemy that is jumping up at you can register as a side hit
//! instead of a stomp, purely because the enemy closed the gap faster than one
//! frame of collision testing can resolve. Two independent adjudicators decide
//! "stomp or hurt", and which one an enemy uses depends on its
//! `ObjectGroup_CollideJumpTable` entry, so the fix has to land in both:
//!
//! * Enemies with their own collide handler (Koopaling → `ObjHit_Koopaling`)
//!   read `Temp_Var12` bit 0, which `Object_HitTestRespond` may *revoke* at
//!   CPU $D8E1 when the two boxes overlap too deeply. See [`OVERLAP_THRESHOLD`].
//! * Enemies with `ObjHit_DoNothing` (most of the roster, reached through
//!   `Player_HitEnemy`) are resolved by the generic path at CPU $D218, which
//!   picks a "stompable height" into `Temp_Var2` and requires the Player's top
//!   to clear the enemy's top by at least that many pixels. See
//!   [`STOMP_HEIGHT_HOOK`].
//!
//! The two paths do not overlap: `Player_HitEnemy` zeroes
//! `Objects_PlayerHitStat` immediately after the hit test, discarding whatever
//! the $D8E1 flip decided, so that byte only reaches the first group.
//!
//! Derived from MaCobra52's "SMB3 - Koopaling & Cheep Cheep hitbox fix 3.ips",
//! which supplies the $D8E1 constant verbatim. Its other half — a trampoline
//! giving the hopping Cheep Cheep a fixed 2-pixel allowance — is replaced here
//! by a rise-aware height that covers every jumping enemy at its actual speed;
//! see [`STOMP_RISE_CODE`] for why a constant cannot do that job.

use crate::rom::Rom;

use super::rom_data::{FS_STOMP_RISE, STOMP_RISE_CPU};

/// Operand of `CMP #$08` at CPU $D8E1, inside `Object_HitTestRespond`.
///
/// After `ObjectObject_Intersect` reports "Player is above" in `Temp_Var12`
/// bit 0, vanilla second-guesses it: `Temp_Var1` holds the height of whichever
/// box is on top and `Temp_Var11` the vertical separation of the two box tops,
/// so `Temp_Var1 - Temp_Var11` is the *overlap depth*. Eight or more pixels of
/// overlap flips bit 0 back off and the stomp becomes damage.
///
/// The threshold is unreachable unless the pair closes faster than 8 px/frame,
/// and a stationary enemy contributes nothing to the closing speed, so raising
/// it cannot affect an ordinary stomp onto an ordinary enemy. Velocities are
/// 4.4 fixed point (see `Object_ApplyXVel`), so a Koopaling's fastest jump
/// (`Koopaling_JumpYVels` = -$70) is 7 px/frame; with the Player's
/// `FALLRATE_MAX` = $40 (4 px/frame) the worst case closes at 11 px/frame.
/// MaCobra's 11 covers all but the single deepest overlap that can produce.
const OVERLAP_THRESHOLD: usize = 0x0018F2;
/// Replacement tolerance: an overlap of 10 or less still counts as a stomp.
const OVERLAP_TOLERANT: u8 = 0x0B;

/// `STY Temp_Var2; LDA Objects_Y,X` at CPU $D22E — where the selected stomp
/// height is committed, common to every object on the generic path.
const STOMP_HEIGHT_HOOK: usize = 0x0123E;

/// Subtract the object's upward speed (in whole pixels) from the stomp height.
///
/// The generic path's test is positional:
///
/// ```text
/// stomp  iff  Player_Y  <=  Objects_Y - Temp_Var2
/// ```
///
/// A rising enemy drags that line upward at its own speed, so a Player whose
/// top is within `rise` pixels of it has a stomp turned into damage purely by
/// the enemy's motion. Cancelling `rise` out leaves the line where it would
/// have been had the enemy held still, which restores the margin vanilla
/// already sizes to absorb the Player's own 4 px/frame fall.
///
/// Expressed as a budget — the window is `30 - T - h` pixels wide for a box
/// with top offset `T`, and a stomp lands when the closing speed fits inside
/// it — subtracting the rise from `h` adds it back to the budget, so the
/// enemy's speed cancels from both sides and only the Player's fall is left to
/// pay for. A per-enemy constant cannot do that: the hopping Cheep Cheep
/// launches at -$30 (3 px/frame) but -$60 (6 px/frame) when the Player is
/// close, and the budget also varies with each enemy's bounding box, which was
/// chosen for drawing rather than for collision feel.
///
/// ```asm
///   LDA Objects_YVel,X   ; 4.4 fixed point
///   BPL .store           ; not rising -> vanilla height
///   EOR #$FF             ; |vel| - 1  (the velocities here are multiples of 16)
///   LSR A ×4             ; -> whole pixels, one short
///   STA Temp_Var2        ; scratch; the STY below overwrites it
///   TYA
///   CLC                  ; borrow makes up the one-short above, exactly
///   SBC Temp_Var2
///   BPL .keep
///   LDA #$00             ; a rise deeper than the height would wrap
/// .keep:
///   TAY
/// .store:
///   STY Temp_Var2        ; displaced
///   LDA Objects_Y,X      ; displaced
///   RTS
/// ```
///
/// The caller re-establishes carry with `SEC` immediately after, so the flags
/// this returns do not matter.
#[rustfmt::skip]
const STOMP_RISE_CODE: [u8; 26] = [
    0xB5, 0xD0,          // LDA $D0,X      Objects_YVel
    0x10, 0x11,          // BPL .store
    0x49, 0xFF,          // EOR #$FF
    0x4A,                // LSR A
    0x4A,                // LSR A
    0x4A,                // LSR A
    0x4A,                // LSR A
    0x85, 0x01,          // STA $01        Temp_Var2 (scratch)
    0x98,                // TYA
    0x18,                // CLC
    0xE5, 0x01,          // SBC $01
    0x10, 0x02,          // BPL .keep
    0xA9, 0x00,          // LDA #$00
    0xA8,                // TAY            .keep
    0x84, 0x01,          // STY $01        .store  (displaced)
    0xB5, 0xA3,          // LDA $A3,X      Objects_Y (displaced)
    0x60,                // RTS
];

/// Apply both halves of the fix.
///
/// Always on: it removes a source of unearned damage and has no effect on any
/// collision that vanilla already resolved correctly, so there is nothing for
/// a player to opt out of and no flag-key bit is spent on it.
pub fn apply(rom: &mut Rom) {
    rom.write_byte(OVERLAP_THRESHOLD, OVERLAP_TOLERANT);
    rom.write_range(FS_STOMP_RISE, &STOMP_RISE_CODE);
    rom.write_range(STOMP_HEIGHT_HOOK, &[
        0x20,
        STOMP_RISE_CPU as u8,
        (STOMP_RISE_CPU >> 8) as u8,
        0xEA, // NOP, filling the 4th displaced byte
    ]);
}

#[cfg(test)]
mod tests {
    use mos6502::cpu::CPU;
    use mos6502::instruction::{Instruction, Ricoh2a03};
    use mos6502::memory::{Bus, Memory};
    use mos6502::Variant;

    use super::*;
    use crate::randomize::rom_data::asm;

    fn vanilla() -> Option<Vec<u8>> {
        std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()
    }

    /// Object slot the routine is exercised on; `LDA Objects_YVel,X` reads
    /// `$D0 + X`, and any slot exercises the same code.
    const SLOT: u8 = 3;
    const OBJECTS_YVEL: u16 = 0x00D0;
    const TEMP_VAR2: u16 = 0x0001;

    /// A CPU with the routine loaded at its real origin. Reused across cases:
    /// the routine does not modify itself, so only the inputs need resetting.
    fn cpu_with_routine() -> CPU<Memory, Ricoh2a03> {
        let mut mem = Memory::new();
        mem.set_bytes(STOMP_RISE_CPU, &STOMP_RISE_CODE);
        CPU::new(mem, Ricoh2a03)
    }

    /// Run the routine for one `(velocity, height)` pair and return the stomp
    /// height it commits to `Temp_Var2`.
    ///
    /// Entry state mirrors the hook site: `Y` holds the height `$D218` selected
    /// and `X` the object slot, which is exactly what the displaced
    /// `STY Temp_Var2` was about to act on.
    fn stomp_height(cpu: &mut CPU<Memory, Ricoh2a03>, vel: u8, height: u8) -> u8 {
        cpu.memory.set_byte(OBJECTS_YVEL + u16::from(SLOT), vel);
        cpu.memory.set_byte(TEMP_VAR2, 0xAA); // poison: a missing store stays visible
        cpu.registers.program_counter = STOMP_RISE_CPU;
        cpu.registers.index_x = SLOT;
        cpu.registers.index_y = height;

        for _ in 0..STOMP_RISE_CODE.len() {
            let op = cpu.memory.get_byte(cpu.registers.program_counter);
            if matches!(Ricoh2a03::decode(op), Some((Instruction::RTS, _))) {
                return cpu.memory.get_byte(TEMP_VAR2);
            }
            cpu.single_step();
        }
        panic!("routine ran past {} instructions without reaching RTS", STOMP_RISE_CODE.len());
    }

    /// The whole point of the routine, over every input it can ever see.
    ///
    /// A non-negative velocity must leave the vanilla height alone; a rising
    /// one must lose its whole-pixel rise, floored at zero. 65,536 cases, which
    /// is the entire state space — the routine reads nothing else.
    #[test]
    fn rise_is_subtracted_from_the_stomp_height() {
        let mut cpu = cpu_with_routine();
        for height in 0..=u8::MAX {
            for vel in 0..=u8::MAX {
                let want = if vel < 0x80 {
                    height // falling or stationary: untouched
                } else {
                    let rise = (vel ^ 0xFF) >> 4; // |vel| - 1, in whole pixels
                    let t = height.wrapping_sub(rise).wrapping_sub(1); // CLC borrows the 1 back
                    if t & 0x80 == 0 { t } else { 0 } // clamp rather than wrap
                };
                assert_eq!(
                    stomp_height(&mut cpu, vel, height),
                    want,
                    "vel=0x{vel:02X} height={height}"
                );
            }
        }
    }

    /// The claim [`STOMP_RISE_CODE`]'s comment makes: for the whole-pixel
    /// velocities the engine actually produces, the height loses *exactly* the
    /// rise — the `EOR`'s off-by-one and the `CLC`'s borrow cancel. This is the
    /// property the size trick is only worth having if it preserves.
    #[test]
    fn whole_pixel_rises_are_cancelled_exactly() {
        let mut cpu = cpu_with_routine();
        for px in 1..=8u8 {
            let vel = 0u8.wrapping_sub(px * 16); // -$10 ..= -$80
            for height in px..=64 {
                // heights where the clamp cannot bite
                assert_eq!(
                    stomp_height(&mut cpu, vel, height),
                    height - px,
                    "{px} px/frame at height {height}"
                );
            }
        }
    }

    /// The velocities and heights that actually occur in the game, spelled out.
    /// Heights are the three `$D218` picks (`prg000.asm:3800`); velocities come
    /// from `ObjNorm_CheepCheepHopper` and `Koopaling_JumpYVels`.
    #[test]
    fn vanilla_cases() {
        let mut cpu = cpu_with_routine();
        // Cheep Cheep Hopper, height 17: launches at -$30, or -$60 close in.
        assert_eq!(stomp_height(&mut cpu, 0xD0, 17), 14); // 3 px/frame
        assert_eq!(stomp_height(&mut cpu, 0xA0, 17), 11); // 6 px/frame
        // Koopaling's fastest jump against the generic height.
        assert_eq!(stomp_height(&mut cpu, 0x90, 19), 12); // 7 px/frame
        // Falling or stationary leaves vanilla behaviour untouched — including
        // the Player's own FALLRATE_MAX, which is downward and so not a rise.
        assert_eq!(stomp_height(&mut cpu, 0x00, 19), 19);
        assert_eq!(stomp_height(&mut cpu, 0x40, 19), 19);
        // A giant (height 8) at the fastest representable rise clamps to zero
        // instead of wrapping to 255, which would invert the comparison.
        assert_eq!(stomp_height(&mut cpu, 0x80, 8), 0);
    }

    /// Decode the assembled bytes and check the structural properties no
    /// assembler was around to check. Supersedes the hand-derived branch-index
    /// assertions this test used to carry.
    ///
    /// Reads only the byte array, so unlike the ROM-dependent tests above it
    /// runs everywhere — including CI, where the ROM is absent.
    #[test]
    fn routine_is_well_formed() {
        asm::check(&STOMP_RISE_CODE).allocation(FS_STOMP_RISE).assert_ok();
    }

    /// The hook must displace *whole* vanilla instructions — `STY Temp_Var2`
    /// (2 bytes) plus `LDA Objects_Y,X` (2) — or the tail of the second would
    /// be left for the CPU to run as an opcode. Split from the check above
    /// because comparing against vanilla needs the ROM, and gating the whole
    /// structural check on that would keep it out of CI.
    #[test]
    fn hook_displaces_whole_instructions() {
        let Some(v) = vanilla() else { return };
        asm::check(&STOMP_RISE_CODE).hook(&v, STOMP_HEIGHT_HOOK, 4).assert_ok();
    }

    /// Both patch sites, against the bytes the disassembly says are there. A
    /// drifted offset would otherwise sail past the byte-level asserts below.
    #[test]
    fn patch_sites_match_vanilla() {
        let Some(v) = vanilla() else { return };
        // Operand of CMP #$08 at $D8E1.
        assert_eq!(v[OVERLAP_THRESHOLD], 0x08);
        // STY Temp_Var2; LDA Objects_Y,X at $D22E.
        assert_eq!(
            &v[STOMP_HEIGHT_HOOK..STOMP_HEIGHT_HOOK + 4],
            &[0x84, 0x01, 0xB5, 0xA3],
        );
        // The PRG030 gap the routine lives in is untouched filler.
        assert!(v[FS_STOMP_RISE..FS_STOMP_RISE + STOMP_RISE_CODE.len()].iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn apply_writes_both_halves() {
        let Some(v) = vanilla() else { return };
        let mut rom = Rom::from_bytes_lax(&v, true).expect("valid ROM");
        apply(&mut rom);
        assert_eq!(rom.read_byte(OVERLAP_THRESHOLD), OVERLAP_TOLERANT);
        assert_eq!(rom.read_range(FS_STOMP_RISE, STOMP_RISE_CODE.len()), &STOMP_RISE_CODE);
        assert_eq!(rom.read_range(STOMP_HEIGHT_HOOK, 3), &[
            0x20,
            STOMP_RISE_CPU as u8,
            (STOMP_RISE_CPU >> 8) as u8,
        ]);
        // Bank containment and allocation fit are checked generically by
        // `routine_is_well_formed`.
    }
}
