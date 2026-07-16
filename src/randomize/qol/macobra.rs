//! MaCobra52 patch bundle: always-on bugfixes plus opt-in feature patches.

use crate::rom::Rom;
use crate::randomize::rom_data::{FS_FASTER_FROG, FS_HOLD_LEFT_HELPER, FS_TAIL_STAY_DEAD};

// ---------------------------------------------------------------------------
// MaCobra patches — always-on bundle
// Consts in this section feed apply_macobra_patches() at the bottom of the
// file; that bundle ships with every randomized ROM. Opt-in MaCobra patches
// (gated by individual options) live in their own section further down.
// ---------------------------------------------------------------------------

// Forced hammer bro walk-over: NOPs `STA $053C,Y; RTS` in the map sprite
// collision check (PRG011, CPU $AEF6). Prevents hammer bros from walking
// onto the player to force a fight — player-initiated encounters still work.
const FORCED_BRO_FIGHT: usize = 0x16F06;

// Bowser upward kill glitch: changes a BNE ($D0) to BCC ($90) in PRG001
// (CPU $BEC1) to fix a glitch where Bowser can be killed from below.
const BOWSER_UPWARD_KILL: usize = 0x3ED1;

// Fire bro bump detection: adjusts collision parameters in PRG004 to add
// proper bump detection and make fire bros slightly more fair.
const FIRE_BRO_BUMP_A: usize = 0x8911;
const FIRE_BRO_BUMP_B: usize = 0x88C1;

// Hammer suit slope slide: allows hammer suit to slide on slopes (PRG000).
const HAMMER_SUIT_SLIDE: usize = 0x3F6;

// Vertical pipe clip fix: prevents an inter-level softlock caused by
// clipping through vertical pipes between areas (PRG029).
const PIPE_CLIP_FIX: usize = 0x3B5B1;

// Move after orb grab: NOPs `STY/STA $7CF4` in PRG003 (CPU $A8ED, $A903) so
// the player-input-lock flag isn't set when grabbing the fortress magic ball.
// Two 3-byte absolute stores → NOPs.
const MOVE_AFTER_ORB_STY: usize = 0x068FD; // STY $7CF4 at CPU $A8ED
const MOVE_AFTER_ORB_STA: usize = 0x06913; // STA $7CF4 at CPU $A903

// Tail attack while swimming (PRG008) — extends the swim subroutine so
// Raccoon/Tanooki Mario can tail-swipe enemies underwater. Two 5-byte hooks
// inside the vanilla swim routine plus a 285-byte replacement block that
// covers $A9D4–$AAF0.
const TAIL_SWIM_HOOK_A: usize = 0x01097B;
const TAIL_SWIM_HOOK_A_BYTES: [u8; 5] = [0xD5, 0xAA, 0xDE, 0xA9, 0xD2];
const TAIL_SWIM_HOOK_B: usize = 0x010989;
const TAIL_SWIM_HOOK_B_BYTES: [u8; 5] = [0xE5, 0xAA, 0x30, 0xAA, 0xE2];
const TAIL_SWIM_ROUTINE_OFFSET: usize = 0x0109E4;
#[rustfmt::skip]
const TAIL_SWIM_ROUTINE: [u8; 285] = [
    0x28, 0xAC, 0x20, 0x36, 0xB0, 0x20, 0xC6, 0xB0, 0x60, 0x60, 0x20, 0x44,
    0xAB, 0x20, 0x28, 0xAC, 0xAD, 0xA4, 0x06, 0xD0, 0x37, 0xA5, 0xD8, 0xF0,
    0x10, 0xAD, 0x8A, 0x05, 0x4A, 0xB0, 0x0A, 0xA9, 0x00, 0x8D, 0x13, 0x05,
    0xA0, 0x01, 0x4C, 0x1B, 0xAA, 0xAD, 0x13, 0x05, 0xD0, 0x15, 0x85, 0xBD,
    0xA5, 0x17, 0x29, 0x03, 0xF0, 0x0D, 0xAD, 0xF1, 0x04, 0x09, 0x80, 0x8D,
    0xF1, 0x04, 0xA9, 0x1F, 0x8D, 0x13, 0x05, 0x4A, 0x4A, 0x4A, 0xA8, 0xB9,
    0x69, 0xA0, 0x85, 0xEE, 0x60, 0x03, 0x07, 0x24, 0x12, 0x21, 0x02, 0x02,
    0x02, 0x01, 0x00, 0x01, 0x02, 0x02, 0x10, 0xF0, 0xA2, 0xFF, 0xA5, 0x17,
    0x29, 0x0C, 0xF0, 0x26, 0x85, 0xD8, 0x4A, 0x4A, 0x4A, 0xAA, 0xBD, 0x2E,
    0xAA, 0x10, 0x07, 0xAC, 0x44, 0x05, 0x10, 0x02, 0xA9, 0x00, 0xA4, 0x17,
    0x10, 0x01, 0x0A, 0xC9, 0xE1, 0x90, 0x06, 0xA4, 0xD8, 0xD0, 0x02, 0xA9,
    0xE0, 0x85, 0xCF, 0x4C, 0x6B, 0xAA, 0xA4, 0xCF, 0xF0, 0x09, 0xC8, 0xA5,
    0xCF, 0x30, 0x02, 0x88, 0x88, 0x84, 0xCF, 0xA5, 0x17, 0x29, 0x03, 0xF0,
    0x10, 0x4A, 0xA8, 0xB9, 0x2E, 0xAA, 0xA4, 0x17, 0x10, 0x01, 0x0A, 0x85,
    0xBD, 0xA2, 0x02, 0xD0, 0x18, 0xA4, 0xBD, 0xF0, 0x0C, 0xC8, 0xA5, 0xBD,
    0x30, 0x02, 0x88, 0x88, 0x84, 0xBD, 0x4C, 0x99, 0xAA, 0xA5, 0xD8, 0xD0,
    0x04, 0xA9, 0x15, 0xD0, 0x36, 0x8A, 0x30, 0x29, 0xA5, 0x15, 0x4A, 0x4A,
    0xA0, 0x00, 0x24, 0x17, 0x30, 0x02, 0x4A, 0xC8, 0x29, 0x07, 0xA8, 0xD0,
    0x0F, 0xA5, 0x15, 0x39, 0x21, 0xAA, 0xD0, 0x08, 0xAD, 0xF1, 0x04, 0x09,
    0x04, 0x8D, 0xF1, 0x04, 0xBD, 0x23, 0xAA, 0x18, 0x79, 0x26, 0xAA, 0xD0,
    0x0A, 0xA0, 0x1F, 0xA5, 0x15, 0x29, 0x08, 0xF0, 0x01, 0xC8, 0x98, 0x85,
    0xEE, 0x60, 0x20, 0xAE, 0xAF, 0x20, 0x44, 0xAB, 0x20, 0x28, 0xAC, 0x20,
    0x36, 0xB0, 0x20, 0xC6, 0xB0, 0x60, 0x20, 0xAE, 0xAF, 0x20, 0x02, 0xAC,
    0x20, 0x2F, 0xAD, 0x20, 0x7F, 0xAD, 0x20, 0xC6, 0xB0,
];

// Hot Foot and Chain Chomp tail vulnerability — three byte flips in
// enemy-collision tables (PRG002 / PRG004) that let the tail/spin defeat
// these enemies. Authored by MaCobra52.
const HOTFOOT_TAIL_A: usize = 0x0413C;
const HOTFOOT_TAIL_B: usize = 0x04151;
const HOTFOOT_TAIL_C: usize = 0x0814D;

// Hold-left airship-entry fix (by MaCobra52) — "SMB3 - Hold left fix.ips".
// Bug: holding Left while entering an airship spawns Mario out over the pit and
// kills him; it surfaces when autoscrollers are disabled. The in-level
// horizontal-scroll routine (PRG008, entry ~$B11F) has several exit paths that
// all fall through to a common tail at $B1CE. When the player holds Left the
// scroll anchor $AB pins to the left edge ($AB == 0) and that tail mispositions
// the airship-entry camera/spawn.
//
// The fix is nine ROM writes, reproduced verbatim from MaCobra's IPS:
//   * A 7-byte scroll-commit helper is dropped in PRG000 dead code at CPU $C918
//     (FS_HOLD_LEFT_HELPER) — `STA $FD; STA $0780; RTS` (+ a trailing NOP). That
//     is exactly the `STA $FD; STA $0780` the tail used to run inline; folding
//     it into a subroutine frees the 2 bytes the new guard needs.
//   * The tail is rewritten (HOLD_LEFT_TAIL) so a new guard sits at $B1CC, two
//     bytes ahead of the old $B1CE tail:
//         $B1CC: LDA $AB
//                BEQ $B208     ; scroll pinned at the left edge -> skip the
//                              ; clamp, jump straight to the finalize path
//                ...           ; vanilla clamp continues unchanged
//     The `STA $FD; STA $0780` there becomes `JSR $C918` (byte-for-byte the
//     helper above).
//   * Every branch/jump that used to land on $B1CE is retargeted to $B1CC
//     (HOLD_LEFT_RETARGETS + the tail's own BPL) so all exits run the guard.
//
// Confirmed against vanilla USA Rev1: every record's original bytes match, and
// no vanilla code references $C918. The "spawn over the pit" behavior itself is
// MaCobra's description of the bug, not independently re-derived here.
const HOLD_LEFT_HELPER_BYTES: [u8; 7] = [0x85, 0xFD, 0x8D, 0x80, 0x07, 0x60, 0xEA];

// Retarget the scroll-routine exits from the old $B1CE tail to the new $B1CC
// guard. Each entry is (file_offset, replacement bytes); only branch/jump
// operands change, so the surrounding instructions stay intact.
const HOLD_LEFT_RETARGETS: &[(usize, &[u8])] = &[
    (0x1113D, &[0xCC]),             // JMP $B1CC (was $B1CE)
    (0x1117B, &[0x60]),             // BMI $B1CC
    (0x11182, &[0x59, 0x30, 0x57]), // BEQ $B1CC / BMI $B1CC
    (0x1119A, &[0x41]),             // BMI $B1CC
    (0x111A3, &[0xCC]),             // JMP $B1CC
    (0x111B9, &[0x22]),             // BMI $B1CC
    (0x111C0, &[0x1B]),             // BPL $B1CC
];

// New scroll tail: BPL $B1CC / `LDA #$00; STA $12` / JSR $C918 / LDA $AB /
// BEQ $B208. The `20 18 C9` in the middle is `JSR $C918` = the helper above.
const HOLD_LEFT_TAIL_OFFSET: usize = 0x111D4;
const HOLD_LEFT_TAIL_BYTES: [u8; 12] =
    [0x07, 0xA9, 0x00, 0x85, 0x12, 0x20, 0x18, 0xC9, 0xA5, 0xAB, 0xF0, 0x38];

// Tail Enemies don't respawn (by MaCobra52) — "SMB3 - Tail Enemies don't
// respawn.ips". "Tail" here means the fire-TRAIL enemies Fire Chomp (object
// $58) and Fire Snake ($59) — NOT the Raccoon/Tanooki tail. Vanilla lets these
// respawn when scrolled off-screen and back; the fix marks a per-object flag so
// they stay dead once defeated. Two writes verified byte-for-byte against the
// IPS on USA Rev1:
//
//   1. An 8-byte routine dropped into a dead gap in PRG003 at CPU $A5F9
//      (FS_TAIL_STAY_DEAD, file 0x06609 — between the `RTS` at 0x06608 and the
//      routine at 0x06611):
//          LDA #$FF          ; a9 ff
//          STA $0659,X       ; 9d 59 06   per-object "stay dead" flag ($0659,X)
//          JMP $BA0B         ; 4c 0b ba   continue the vanilla despawn path
//   2. In the Fire Chomp / Fire Snake AI (PRG003), the `JMP $BA0B` at file
//      0x07E49 (CPU $BE39 — right after a `CMP #$59` Fire Snake ID check) is
//      retargeted to `JMP $A5F9` by rewriting its operand (file 0x07E4A:
//      0b ba -> f9 a5), so the new routine runs first and then falls through to
//      the original code.
const TAIL_STAY_DEAD_ROUTINE: [u8; 8] = [0xA9, 0xFF, 0x9D, 0x59, 0x06, 0x4C, 0x0B, 0xBA];
const TAIL_STAY_DEAD_HOOK_OFFSET: usize = 0x07E4A;
const TAIL_STAY_DEAD_HOOK_BYTES: [u8; 2] = [0xF9, 0xA5]; // JMP $A5F9 operand

// ---------------------------------------------------------------------------
// MaCobra patches — opt-in features
// Each apply_* below is gated by an individual option in randomizer.rs;
// none of these ship unless the corresponding flag is enabled.
// ---------------------------------------------------------------------------

// Early Sun (by MaCobra52) — drops the Angry Sun's pre-attack threshold
// from 5 to 0 so it begins swooping immediately on spawn instead of after
// the vanilla delay. Single byte: PRG005 CPU $AD71 = file 0xAD81, operand
// of `CMP #$05` becomes `CMP #$00`. Source:
// https://github.com/macobra52/smb3-hacks/blob/main/SMB3%20IPS/SMB3%20-%20Early%20Sun.ips
const EARLY_SUN_OFFSET: usize = 0x0AD81;

/// Apply MaCobra52's "Early Sun" patch — the Angry Sun starts attacking
/// without its vanilla pre-attack delay.
pub fn apply_early_sun(rom: &mut Rom) {
    rom.write_byte(EARLY_SUN_OFFSET, 0x00);
}

// Limit Bro Movement ("SMB3 - Limit Bro Movement.ips") — restricts where
// wandering map objects (Hammer Bros) may step on the overworld. The march
// validator `Map_MarchValidateTravel` (PRG011 CPU $B3A3) scans a tile table
// at $B388 (`Map_Object_Forbid_LandingTiles`); in vanilla that table is a
// BLACKLIST and a match rejects the move. This patch does two things:
//
//   1. Inverts the branch at $B409 (`F0 06 .. 4C A3 B3`) so a match now
//      ACCEPTS the move and an exhausted no-match scan rejects it — turning
//      the table from a blacklist into a WHITELIST.
//   2. Replaces the table with the 12 walkable path-tile IDs (the rest
//      filled with 0xFF sentinels that never match a real tile).
//
// Net effect: wandering Hammer Bros may only land on path tiles instead of
// "anything not forbidden", so they stay on the paths. All three writes are
// in-bank (World Map code/data, file 0x16010–0x1800F); the patched routine's
// `JMP $B3A3` stays in-bank. Offsets are header-inclusive (same as the IPS).
const LIMIT_BRO_TABLE_OFFSET: usize = 0x17398;
const LIMIT_BRO_TABLE: [u8; 12] =
    [0x44, 0x47, 0x48, 0x4A, 0xAE, 0xAF, 0xB5, 0xB6, 0xD9, 0xDC, 0xDD, 0xDE];
const LIMIT_BRO_FILL_OFFSET: usize = 0x173A4;
const LIMIT_BRO_FILL: [u8; 9] = [0xFF; 9];
const LIMIT_BRO_CODE_OFFSET: usize = 0x17419;
const LIMIT_BRO_CODE: [u8; 8] = [0xF0, 0x06, 0x88, 0xD0, 0xF8, 0x4C, 0xA3, 0xB3];

/// Apply the "Limit Bro Movement" patch — converts the wandering-object
/// landing-tile blacklist into a whitelist of path tiles, so wandering
/// Hammer Bros may only step onto overworld path tiles.
pub fn apply_limit_bro_movement(rom: &mut Rom) {
    rom.write_range(LIMIT_BRO_TABLE_OFFSET, &LIMIT_BRO_TABLE);
    rom.write_range(LIMIT_BRO_FILL_OFFSET, &LIMIT_BRO_FILL);
    rom.write_range(LIMIT_BRO_CODE_OFFSET, &LIMIT_BRO_CODE);
}

// Japanese damage system (by MaCobra52) — NOPs the `JMP $DA15` at file
// 0x019F9 so the vanilla "demote power-up by one tier" subroutine is
// skipped. Control falls through into the path that drops the player
// straight to Small Mario from any power-up state, matching the Famicom
// SMB3 damage model. Source:
// https://github.com/macobra52/smb3-hacks/blob/main/SMB3%20IPS/SMB3%20-%20Japanese%20damage%20system%20(fixed).ips
const JP_DAMAGE_OFFSET: usize = 0x019F9;
const JP_DAMAGE_BYTES: [u8; 3] = [0xEA, 0xEA, 0xEA];

/// Apply MaCobra52's "Japanese damage system (fixed)" patch — taking damage
/// from any power-up tier (Super, Fire, Raccoon, Frog, Tanooki, Hammer)
/// drops the player straight to Small Mario instead of demoting one tier
/// at a time.
pub fn apply_japanese_damage(rom: &mut Rom) {
    rom.write_range(JP_DAMAGE_OFFSET, &JP_DAMAGE_BYTES);
}

// Infinite use Mushroom Houses (by MaCobra52) — 5-byte rewrite at file
// 0x0169E5 (PRG011, CPU $A9D5) that drops the TOADHOUSE tile ($50) out of
// the "remove after use" tile list. The remaining list entries shift one
// position earlier and two NOPs are appended so the reader stops there.
// Effect: toad houses no longer disappear after entering them, so the
// reward can be collected repeatedly. Source:
// https://github.com/macobra52/smb3-hacks/blob/main/SMB3%20IPS/SMB3%20-%20Infinite%20use%20Mushroom%20Houses.ips
const INF_MUSHROOM_HOUSES_OFFSET: usize = 0x0169E5;
const INF_MUSHROOM_HOUSES_BYTES: [u8; 5] = [0xE8, 0xE6, 0xBD, 0xEA, 0xEA];

/// Apply MaCobra52's "Infinite use Mushroom Houses" patch — toad houses
/// stay on the map after entering and can be visited any number of times.
pub fn apply_infinite_mushroom_houses(rom: &mut Rom) {
    rom.write_range(INF_MUSHROOM_HOUSES_OFFSET, &INF_MUSHROOM_HOUSES_BYTES);
}

// Fast Mushroom House (by MaCobra52) — combination of two single-byte
// timer tweaks:
//   * "Move Sooner in Mushroom House (Instant)" — file 0x005234, the
//     post-entry input-lock timer (0xFF → 0x00), so the player can move
//     immediately on the chest-select screen instead of waiting for the
//     vanilla intro animation.
//   * "Exit Mushroom House Faster" — file 0x001E3F, the exit-transition
//     timer (0xFF → 0x5F), so closing the house and returning to the map
//     is roughly 60% shorter.
// Sources:
// https://github.com/macobra52/smb3-hacks/blob/main/SMB3%20IPS/SMB3%20-%20Move%20Sooner%20in%20Mushroom%20House%20(Instant).ips
// https://github.com/macobra52/smb3-hacks/blob/main/SMB3%20IPS/SMB3%20-%20Exit%20Mushroom%20House%20Faster.ips
const FAST_MUSH_MOVE_OFFSET: usize = 0x005234;
const FAST_MUSH_EXIT_OFFSET: usize = 0x001E3F;

/// Apply MaCobra52's "Fast Mushroom House" — combines the "Move Sooner"
/// and "Exit Faster" timer tweaks: skip the entry-input-lock and shorten
/// the exit transition.
pub fn apply_fast_mushroom_house(rom: &mut Rom) {
    rom.write_byte(FAST_MUSH_MOVE_OFFSET, 0x00);
    rom.write_byte(FAST_MUSH_EXIT_OFFSET, 0x5F);
}

// Faster Tail Speed (by MaCobra52) — bundles three writes:
//
//   1. Reduced tail slowdown. File 0x110A6 ← 0x29. Shortens the
//      post-swipe slowdown frames so the tail attack is less punishing
//      to use mid-run.
//   2. Slightly reduced raccoon/Tanooki flight time. File 0x10CAA
//      ← 0x78. The faster tail makes building meter cheaper, which
//      otherwise opens a known cheese skip in 8-1 by flying over a
//      large section of the level; trimming flight duration cancels
//      the cheese without removing flight outright.
//   3. Lower the 7-6 fly-strat wall. File 0x1F36A ← {0x42, 0x14, 0xBD}
//      (3-byte tile payload). The shortened flight time from (2) would
//      otherwise leave the intended 7-6 fly route unreachable; this
//      retunes the wall height so the strat still clears at the new
//      flight duration.
//
// Source: MaCobra52 (no public IPS link).
const FASTER_TAIL_SLOWDOWN_OFFSET: usize = 0x110A6;
const FASTER_TAIL_FLIGHT_OFFSET: usize = 0x10CAA;
const FASTER_TAIL_W76_WALL_OFFSET: usize = 0x1F36A;
const FASTER_TAIL_W76_WALL_BYTES: [u8; 3] = [0x42, 0x14, 0xBD];

/// Apply MaCobra52's "Faster Tail Speed" — reduces tail-swipe slowdown,
/// trims raccoon/Tanooki flight time to neutralize the 8-1 cheese the
/// faster tail enables, and lowers the 7-6 wall so the intended fly
/// strat still clears at the new flight duration.
pub fn apply_faster_tail_speed(rom: &mut Rom) {
    rom.write_byte(FASTER_TAIL_SLOWDOWN_OFFSET, 0x29);
    rom.write_byte(FASTER_TAIL_FLIGHT_OFFSET, 0x78);
    rom.write_range(FASTER_TAIL_W76_WALL_OFFSET, &FASTER_TAIL_W76_WALL_BYTES);
}

// Faster Frog ("SMB3 - Faster Frog (tail attack while swimming compatible)")
// — speeds up Frog-Suit swimming and running. Four writes, in two groups:
//
//   Group A — two edits INSIDE the Tail-Attack-While-Swimming replacement
//   routine (TAIL_SWIM_ROUTINE, written unconditionally by
//   apply_macobra_patches). These bytes only exist once tail-swim is
//   applied (vanilla holds 05 D0 / 01 02 here, not the tail-swim values),
//   which is exactly why the upstream patch is named "...compatible": it
//   patches the tail-swim version of the swim routine, not vanilla. Since
//   tail-swim is always-on in our builds, the base is always present.
//     1. 0x010A12 (TAIL_SWIM_ROUTINE +46) ← EA EA: NOP out a `STA $BD`.
//     2. 0x010A3E (TAIL_SWIM_ROUTINE +90) ← 14 EC: retune two swim-speed
//        table entries (vanilla-routine bytes 10 F0).
//
//   Group B — the standalone speed boost, independent of tail-swim:
//     3. FS_FASTER_FROG (0x3A600, PRG029, CPU $C5F0) ← 24-byte routine.
//        Checks the Frog-Suit power-up state ($ED == 4), remaps the swim
//        index into a faster speed-table slot, then runs the displaced
//        `LDA $CE37,X` and RTS (trampoline tail).
//     4. 0x03AEB1 ← 20 F0 C5 (JSR $C5F0): bank-local hook into the swim
//        physics that diverts through the new routine. Replaces the
//        vanilla `LDA $CE37,X` (BD 37 CE) that the routine re-runs.
//
// Source: "SMB3 - Faster Frog (tail attack while swimming compatible).ips"
// in the project root; bytes verified record-for-record against it.
const FASTER_FROG_EDIT_A_OFFSET: usize = 0x010A12;
const FASTER_FROG_EDIT_A_BYTES: [u8; 2] = [0xEA, 0xEA];
const FASTER_FROG_EDIT_B_OFFSET: usize = 0x010A3E;
const FASTER_FROG_EDIT_B_BYTES: [u8; 2] = [0x14, 0xEC];
const FASTER_FROG_ROUTINE: [u8; 24] = [
    0xA5, 0xED, 0xC9, 0x04, 0xD0, 0x0E, 0x8A, 0x38, 0xE9, 0x39, 0x30, 0x08, 0xC9, 0x03, 0x10, 0x04,
    0x18, 0x69, 0x29, 0xAA, 0xBD, 0x37, 0xCE, 0x60,
];
const FASTER_FROG_HOOK_OFFSET: usize = 0x03AEB1;
const FASTER_FROG_HOOK_BYTES: [u8; 3] = [0x20, 0xF0, 0xC5]; // JSR $C5F0

/// Apply "Faster Frog" — speeds up Frog-Suit swimming and running. Depends on the
/// always-on Tail-Attack-While-Swimming routine (two of its writes patch
/// inside that routine), plus a standalone speed-boost routine + hook in
/// PRG029. Must run AFTER apply_macobra_patches so the tail-swim base it
/// edits is already in place.
pub fn apply_faster_frog(rom: &mut Rom) {
    rom.write_range(FASTER_FROG_EDIT_A_OFFSET, &FASTER_FROG_EDIT_A_BYTES);
    rom.write_range(FASTER_FROG_EDIT_B_OFFSET, &FASTER_FROG_EDIT_B_BYTES);
    rom.write_range(FS_FASTER_FROG, &FASTER_FROG_ROUTINE);
    rom.write_range(FASTER_FROG_HOOK_OFFSET, &FASTER_FROG_HOOK_BYTES);
}

// No Game Over Penalty (by MaCobra52) — four writes verified byte-for-byte
// against the upstream IPS at `SMB3 - No Game Over Penalty.ips` in this
// repo. After a Game Over the player keeps their reserve inventory,
// world map state, and card progress instead of having them wiped.
//
//   1. File 0x016A0F: 2 bytes — JSR operand redirected to the new
//      subroutine at CPU $BD40.
//   2. File 0x017A82: 8 bytes — `JSR $BD46` + 5 NOPs replacing the
//      vanilla "reset on Game Over" call sequence.
//   3. File 0x017D50: 26 bytes — new subroutine in PRG011 free space
//      at $BD40-$BD59 that decides which state is allowed to reset
//      (returns 0 to skip the wipe, 1 to allow it) based on the
//      current map tile being checked against $50 / $E0 / $E8.
//   4. File 0x03D314: 3 NOPs killing the vanilla decrement/clear
//      instruction that ran unconditionally on Game Over.
const NGO_HOOK_A_OFFSET: usize = 0x016A0F;
const NGO_HOOK_A_BYTES: [u8; 2] = [0x40, 0xBD];
const NGO_HOOK_B_OFFSET: usize = 0x017A82;
const NGO_HOOK_B_BYTES: [u8; 8] = [0x20, 0x46, 0xBD, 0xEA, 0xEA, 0xEA, 0xEA, 0xEA];
const NGO_ROUTINE_OFFSET: usize = 0x017D50;
#[rustfmt::skip]
const NGO_ROUTINE: [u8; 26] = [
    0x20, 0xFE, 0xD1, 0x85, 0xE6, 0x60, 0xA5, 0xE6, 0xC9, 0x50, 0xF0, 0x0B,
    0xC9, 0xE0, 0xF0, 0x07, 0xC9, 0xE8, 0xF0, 0x03, 0xA9, 0x00, 0x60, 0xA9,
    0x01, 0x60,
];
const NGO_NOP_OFFSET: usize = 0x03D314;
const NGO_NOP_BYTES: [u8; 3] = [0xEA, 0xEA, 0xEA];

/// Apply MaCobra52's "No Game Over Penalty" patch — Game Overs no longer
/// wipe the player's reserve inventory, world map progress, or card
/// state.
pub fn apply_no_game_over_penalty(rom: &mut Rom) {
    rom.write_range(NGO_HOOK_A_OFFSET, &NGO_HOOK_A_BYTES);
    rom.write_range(NGO_HOOK_B_OFFSET, &NGO_HOOK_B_BYTES);
    rom.write_range(NGO_ROUTINE_OFFSET, &NGO_ROUTINE);
    rom.write_range(NGO_NOP_OFFSET, &NGO_NOP_BYTES);
}

// Remove Flashing (by MaCobra52) — "SMB3 - Remove Flashing.ips". Suppresses
// the full-screen palette-flash/fade animation (the rapid color strobing used
// on some transitions and effects) to make the game safer for photosensitive
// players. Eight writes, reproduced byte-for-byte from the IPS and verified
// against USA Rev1. The core mechanism:
//
//   * The fade routine's per-color palette writes (`STA $0301,X` … `STA $030C,X`)
//     are all redirected to a scratch address (`STA $0379,X`), so the computed
//     flash colors are never committed to the active palette RAM.
//   * The fade-index load `LDX $0300` is pinned to `LDX #$01` (+ NOP) so the
//     animation no longer cycles.
//   * Three single-byte tweaks (0x149F0, 0x1634E: 0x16->0x1F; 0x361B9:
//     0x16->0x0F) in the related banks complete the effect.
//
// Cosmetic / accessibility only — not encoded in the flag key and consumes no
// RNG. The IPS records are non-contiguous because it only overwrites the bytes
// that differ from vanilla, so each record is reproduced as its own write.
const REMOVE_FLASHING_WRITES: &[(usize, &[u8])] = &[
    (0x0E19F, &[0xA2, 0x01, 0xEA]),
    (
        0x0E1BB,
        &[
            0x79, 0x03, 0xA9, 0x04, 0x9D, 0x79, 0x03, 0xA9, 0x08, 0x9D, 0x79, 0x03, 0xB9, 0x8B,
            0xA1, 0x9D, 0x79, 0x03, 0x9D, 0x79, 0x03, 0x9D, 0x79, 0x03, 0x9D, 0x79, 0x03, 0xAD,
            0xC5, 0x07, 0x9D, 0x79, 0x03, 0xAD, 0xC9, 0x07, 0x9D, 0x79, 0x03, 0xAD, 0xCB, 0x07,
            0x9D, 0x79, 0x03, 0xAD, 0xCC, 0x07, 0x9D, 0x79, 0x03, 0xA9, 0x00, 0x9D, 0x79,
        ],
    ),
    (0x0E1F8, &[0x79]),
    (
        0x0E209,
        &[
            0x79, 0x03, 0xA9, 0x10, 0x9D, 0x79, 0x03, 0xAD, 0xD2, 0x07, 0x9D, 0x79, 0x03, 0xAD,
            0xD3, 0x07, 0x9D, 0x79, 0x03, 0xAD, 0xD4, 0x07, 0x9D, 0x79, 0x03, 0xA9, 0x3F, 0x9D,
            0x79, 0x03, 0xA9, 0x04, 0x9D, 0x79, 0x03, 0xA9, 0x00, 0x9D, 0x79,
        ],
    ),
    (0x0E236, &[0x79]),
    (0x149F0, &[0x1F]),
    (0x1634E, &[0x1F]),
    (0x361B9, &[0x0F]),
];

/// Apply MaCobra52's "Remove Flashing" patch — suppresses the full-screen
/// palette-flash/fade animation for photosensitive-safe play. Cosmetic /
/// accessibility option; not in the flag key and uses no RNG.
pub fn apply_remove_flashing(rom: &mut Rom) {
    for &(offset, bytes) in REMOVE_FLASHING_WRITES {
        rom.write_range(offset, bytes);
    }
}

/// Apply MaCobra's always-on bugfixes and fairness patches.
pub fn apply_macobra_patches(rom: &mut Rom) {
    // Prevent forced hammer bro fights (4 NOPs)
    rom.write_range(FORCED_BRO_FIGHT, &[0xEA; 4]);

    // Fix Bowser upward kill glitch
    rom.write_byte(BOWSER_UPWARD_KILL, 0x90);

    // Add proper fire bro bump detection and make them more fair
    rom.write_range(FIRE_BRO_BUMP_A, &[0x13, 0xAB]);
    rom.write_byte(FIRE_BRO_BUMP_B, 0x40);

    // Enable hammer suit to slide on slopes
    rom.write_byte(HAMMER_SUIT_SLIDE, 0x00);

    // Fix inter-level vertical pipe clip softlock
    rom.write_byte(PIPE_CLIP_FIX, 0x00);

    // Allow Mario to keep moving after grabbing the fortress orb / magic ball.
    rom.write_range(MOVE_AFTER_ORB_STY, &[0xEA; 3]);
    rom.write_range(MOVE_AFTER_ORB_STA, &[0xEA; 3]);

    // Tail attack while swimming (Raccoon/Tanooki tail-swipes underwater).
    rom.write_range(TAIL_SWIM_HOOK_A, &TAIL_SWIM_HOOK_A_BYTES);
    rom.write_range(TAIL_SWIM_HOOK_B, &TAIL_SWIM_HOOK_B_BYTES);
    rom.write_range(TAIL_SWIM_ROUTINE_OFFSET, &TAIL_SWIM_ROUTINE);

    // Make Hot Foot and Chain Chomp tail-vulnerable.
    rom.write_byte(HOTFOOT_TAIL_A, 0x00);
    rom.write_byte(HOTFOOT_TAIL_B, 0x00);
    rom.write_byte(HOTFOOT_TAIL_C, 0x25);

    // Tail Enemies don't respawn: mark Fire Chomp / Fire Snake with a per-object
    // "stay dead" flag so they don't respawn when scrolled off-screen and back.
    rom.write_range(FS_TAIL_STAY_DEAD, &TAIL_STAY_DEAD_ROUTINE);
    rom.write_range(TAIL_STAY_DEAD_HOOK_OFFSET, &TAIL_STAY_DEAD_HOOK_BYTES);

    // NOTE: MaCobra's "Bros don't stop on hands" (issue #14) used to live
    // here; it is subsumed by the overworld writer's march-veto trampoline
    // (overworld_writer/march_veto.rs), which rejects hand-trap landings
    // outright at Map_MarchValidateTravel's landing-zone check.

    // Hold-left airship-entry pit-death fix (MaCobra52). See notes above the
    // HOLD_LEFT_* constants. Nine verbatim writes: helper + tail + exit retargets.
    rom.write_range(FS_HOLD_LEFT_HELPER, &HOLD_LEFT_HELPER_BYTES);
    rom.write_range(HOLD_LEFT_TAIL_OFFSET, &HOLD_LEFT_TAIL_BYTES);
    for &(offset, bytes) in HOLD_LEFT_RETARGETS {
        rom.write_range(offset, bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::randomize::qol::test_support::make_test_rom;

    #[test]
    fn test_macobra_tail_swim_writes() {
        let mut rom = make_test_rom();
        apply_macobra_patches(&mut rom);

        assert_eq!(rom.read_range(TAIL_SWIM_HOOK_A, 5), &TAIL_SWIM_HOOK_A_BYTES);
        assert_eq!(rom.read_range(TAIL_SWIM_HOOK_B, 5), &TAIL_SWIM_HOOK_B_BYTES);
        assert_eq!(
            rom.read_range(TAIL_SWIM_ROUTINE_OFFSET, TAIL_SWIM_ROUTINE.len()),
            &TAIL_SWIM_ROUTINE
        );
    }

    #[test]
    fn test_faster_frog_writes() {
        let mut rom = make_test_rom();
        // Mirror randomizer order: tail-swim (always-on) first, then faster_frog
        // layers on top — its Group A edits patch inside the tail-swim routine.
        apply_macobra_patches(&mut rom);
        apply_faster_frog(&mut rom);

        // Group A: edits land inside the tail-swim routine span (0x0109E4..0x010B01),
        // so they must be written after apply_macobra_patches to survive.
        assert_eq!(
            rom.read_range(FASTER_FROG_EDIT_A_OFFSET, FASTER_FROG_EDIT_A_BYTES.len()),
            &FASTER_FROG_EDIT_A_BYTES
        );
        assert_eq!(
            rom.read_range(FASTER_FROG_EDIT_B_OFFSET, FASTER_FROG_EDIT_B_BYTES.len()),
            &FASTER_FROG_EDIT_B_BYTES
        );
        // Group B: standalone routine + hook.
        assert_eq!(rom.read_range(FS_FASTER_FROG, FASTER_FROG_ROUTINE.len()), &FASTER_FROG_ROUTINE);
        assert_eq!(
            rom.read_range(FASTER_FROG_HOOK_OFFSET, FASTER_FROG_HOOK_BYTES.len()),
            &FASTER_FROG_HOOK_BYTES
        );
    }

    #[test]
    fn test_macobra_hotfoot_chainchomp_tail() {
        let mut rom = make_test_rom();
        apply_macobra_patches(&mut rom);

        assert_eq!(rom.read_byte(HOTFOOT_TAIL_A), 0x00);
        assert_eq!(rom.read_byte(HOTFOOT_TAIL_B), 0x00);
        assert_eq!(rom.read_byte(HOTFOOT_TAIL_C), 0x25);
    }

    #[test]
    fn test_macobra_hold_left_fix_writes() {
        let mut rom = make_test_rom();
        apply_macobra_patches(&mut rom);

        // Helper + new tail land where expected.
        assert_eq!(
            rom.read_range(FS_HOLD_LEFT_HELPER, HOLD_LEFT_HELPER_BYTES.len()),
            &HOLD_LEFT_HELPER_BYTES
        );
        assert_eq!(
            rom.read_range(HOLD_LEFT_TAIL_OFFSET, HOLD_LEFT_TAIL_BYTES.len()),
            &HOLD_LEFT_TAIL_BYTES
        );
        for &(offset, bytes) in HOLD_LEFT_RETARGETS {
            assert_eq!(rom.read_range(offset, bytes.len()), bytes);
        }
    }

    #[test]
    fn test_hold_left_jsr_targets_helper() {
        // The new tail's `JSR $C918` (bytes 5..8 = 20 18 C9) must point at the
        // helper. PRG000's dead code is reached at $C000 + (file - iNES header).
        let jsr = &HOLD_LEFT_TAIL_BYTES[5..8];
        assert_eq!(jsr[0], 0x20, "expected a JSR opcode");
        let target_cpu = u16::from_le_bytes([jsr[1], jsr[2]]);
        let helper_cpu = (0xC000 + (FS_HOLD_LEFT_HELPER - 0x10)) as u16;
        assert_eq!(target_cpu, helper_cpu, "JSR operand must match the helper's $C918 address");
        assert_eq!(target_cpu, 0xC918);
    }

    // The former bros-don't-stop-on-hands hook ($B425) is gone: hand-trap
    // avoidance moved into the march-veto trampoline. Prove macobra no longer
    // touches the site (the writer's own tests cover the veto behavior).
    #[test]
    fn test_macobra_leaves_bro_gate_vanilla() {
        let mut rom = make_test_rom();
        let before = rom.read_range(0x17435, 3).to_vec();
        apply_macobra_patches(&mut rom);
        assert_eq!(rom.read_range(0x17435, 3), &before[..]);
    }

    #[test]
    fn test_macobra_tail_stay_dead_writes() {
        let mut rom = make_test_rom();
        apply_macobra_patches(&mut rom);

        // Routine lands in the PRG003 gap, and the defeat-handler JMP operand
        // is retargeted to it ($A5F9).
        assert_eq!(
            rom.read_range(FS_TAIL_STAY_DEAD, TAIL_STAY_DEAD_ROUTINE.len()),
            &TAIL_STAY_DEAD_ROUTINE
        );
        assert_eq!(
            rom.read_range(TAIL_STAY_DEAD_HOOK_OFFSET, TAIL_STAY_DEAD_HOOK_BYTES.len()),
            &TAIL_STAY_DEAD_HOOK_BYTES
        );
        // The retarget must name the routine's CPU address: PRG003 maps to
        // $A000, so CPU = $A000 + (FS_TAIL_STAY_DEAD - 0x06010).
        let routine_cpu = (0xA000 + (FS_TAIL_STAY_DEAD - 0x06010)) as u16;
        assert_eq!(routine_cpu, 0xA5F9);
        assert_eq!(u16::from_le_bytes(TAIL_STAY_DEAD_HOOK_BYTES), routine_cpu);
    }

    #[test]
    fn test_remove_flashing_writes() {
        let mut rom = make_test_rom();
        apply_remove_flashing(&mut rom);
        for &(offset, bytes) in REMOVE_FLASHING_WRITES {
            assert_eq!(rom.read_range(offset, bytes.len()), bytes);
        }
    }

    #[test]
    fn test_macobra_no_game_over_penalty_writes() {
        let mut rom = make_test_rom();
        apply_no_game_over_penalty(&mut rom);

        assert_eq!(rom.read_range(NGO_HOOK_A_OFFSET, NGO_HOOK_A_BYTES.len()), &NGO_HOOK_A_BYTES);
        assert_eq!(rom.read_range(NGO_HOOK_B_OFFSET, NGO_HOOK_B_BYTES.len()), &NGO_HOOK_B_BYTES);
        assert_eq!(rom.read_range(NGO_ROUTINE_OFFSET, NGO_ROUTINE.len()), &NGO_ROUTINE);
        assert_eq!(rom.read_range(NGO_NOP_OFFSET, NGO_NOP_BYTES.len()), &NGO_NOP_BYTES);
    }
}
