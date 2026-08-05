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
    use super::*;

    fn vanilla() -> Option<Vec<u8>> {
        std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()
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
        // PRG030 is only mapped at $8000-$9FFF; a routine crossing into the
        // next bank would execute whatever happens to be paged in.
        assert!(FS_STOMP_RISE + STOMP_RISE_CODE.len() <= 0x3E010);
    }

    /// The two `BPL`s are the only relative branches; a miscount lands
    /// mid-instruction and would still assemble, so pin both targets.
    #[test]
    fn routine_branches_land_on_instruction_boundaries() {
        // BPL at index 2 skips to `.store` (STY $01) at index 21.
        assert_eq!(STOMP_RISE_CODE[3] as usize + 4, 21);
        assert_eq!(STOMP_RISE_CODE[21], 0x84);
        // BPL at index 16 skips the clamp to `.keep` (TAY) at index 20.
        assert_eq!(STOMP_RISE_CODE[17] as usize + 18, 20);
        assert_eq!(STOMP_RISE_CODE[20], 0xA8);
    }
}
