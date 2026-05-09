use rand::Rng;

use crate::rom::Rom;
use super::overworld_build::{BuildResult, SlotKind};

/// Per-slot probability that a regular-level slot is converted into a hand trap.
const HAND_TRAP_RATE: f64 = 0.10;

/// File offset of the post-arrival 50/50 grab roll at PRG010 $CF1F.
///
/// Vanilla bytes: `29 01 D0 06` = `AND #$01 / BNE +$06` — when the random
/// per-player flag bit is set, branch past the grab dispatch (skip grab).
///
/// Patched bytes: `EA EA EA EA` (4 NOPs) — the AND/BNE is removed, so every
/// 0xE6 arrival falls through to the grab path unconditionally.
const GRAB_ROLL_OFFSET: usize = 0x14F2F;
const GRAB_ROLL_NOPS: [u8; 4] = [0xEA, 0xEA, 0xEA, 0xEA];

/// Force every 0xE6 (HANDTRAP) tile arrival to grab the player.
///
/// Vanilla post-arrival flow at $CF15:
///   CMP #$E6        ; tile is HANDTRAP?
///   BNE skip        ; no → continue normally
///   LDX $0726       ; current player (0 or 1)
///   LDA $0782,X     ; per-player random bit
///   AND #$01        ; ← patched out
///   BNE skip        ; ← patched out (these 4 bytes become NOPs)
///   INC $0729       ; grab counter
///   JMP $CEAC       ; dispatch grab
///
/// After the patch, the AND/BNE is removed, so the grab dispatch always runs.
/// The grab logic loads the level pointed at by the slot's pointer-table
/// entry — for a regular-level slot dressed as a hand-trap, that's the
/// regular level. After the player beats it, vanilla level-completion code
/// rewrites the tile to a checkmark, so subsequent visits don't re-grab
/// (the CMP #$E6 fails on the new byte).
pub(crate) fn install_full_grab(rom: &mut Rom) {
    rom.write_range(GRAB_ROLL_OFFSET, &GRAB_ROLL_NOPS);
}

/// Mark each regular-level slot as a hand-trap with probability `HAND_TRAP_RATE`.
/// The writer reads `slot.is_hand_trap` and stamps 0xE6 instead of a level number.
pub(crate) fn mark_hand_traps<R: Rng>(build: &mut BuildResult, rng: &mut R) {
    for built in &mut build.worlds {
        for slot in &mut built.slots {
            if slot.kind == SlotKind::Level && rng.random_bool(HAND_TRAP_RATE) {
                slot.is_hand_trap = true;
            }
        }
    }
}
