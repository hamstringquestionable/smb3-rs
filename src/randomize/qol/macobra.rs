//! MaCobra52 patch bundle: always-on bugfixes plus opt-in feature patches.

use crate::rom::Rom;
use crate::randomize::rom_data::{FS_BROS_NO_HANDS, FS_FASTER_FROG, jsr_into_bank};

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

// Bros don't stop on hands (by MaCobra52) — fixes issue #14. Roaming
// overworld bros decide where they may rest via the object-movement level
// gate at PRG011 $B425 (`CMP $7E98,Y`), the sprite-side twin of the player
// level gate. It reuses the shared per-palette-page threshold table
// ($7E98,Y); a tile at-or-above its page threshold is a level slot the bro
// won't rest on, below-threshold tiles are plain path it can settle on.
// HANDTRAP tile 0xE6 sits just *below* the page-3 threshold (0xE9), so a
// wandering bro treats it as plain path and can come to rest on a hand-trap
// slot — colliding the two encounters (one clears the other).
//
// The fix replaces the inline `CMP $7E98,Y` at the gate with a JSR to an
// 8-byte helper in PRG011 free space (FS_BROS_NO_HANDS, CPU $BD32) that
// forces 0xE6 to read as gated and otherwise performs the identical compare:
//
//   CMP #$E6     ; hand-trap tile?
//   BEQ +3       ; yes -> return with carry SET (gated), skipping the compare
//   CMP $7E98,Y  ; no  -> vanilla threshold compare (flags identical)
//   RTS          ; RTS preserves flags; A/X/Y untouched
//
// So 0xE6 now behaves like a normal uncompleted level tile to roaming bros:
// they may walk *over* it but never rest on it. Completed hand-traps are
// rewritten to a checkmark tile (already above-threshold), so they act as a
// barrier with no extra work — matching issue #14's desired behavior.
//
// MaCobra's standalone IPS placed the helper at CPU $BC80 (file 0x17C90),
// which collides with our FS_SAS_GAMEOVER_FINALIZE allocation (0x17C87); it
// is relocated here to FS_BROS_NO_HANDS. The JSR operand is derived from the
// helper's file offset via `jsr_into_bank` (never hand-written) so it can't
// drift from where the helper bytes actually land — see issue #14.
const BROS_NO_HANDS_HOOK: usize = 0x17435; // CPU $B425, vanilla `CMP $7E98,Y`
const BROS_NO_HANDS_SUB: [u8; 8] = [0xC9, 0xE6, 0xF0, 0x03, 0xD9, 0x98, 0x7E, 0x60];

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

    // Roaming bros don't rest on hand-trap tiles (0xE6). Fixes issue #14.
    // FS_BROS_NO_HANDS lives in PRG011 (bank 11); derive the JSR operand from
    // its file offset so the hook can never point past the helper.
    rom.write_range(BROS_NO_HANDS_HOOK, &jsr_into_bank(11, FS_BROS_NO_HANDS));
    rom.write_range(FS_BROS_NO_HANDS, &BROS_NO_HANDS_SUB);
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
    fn test_macobra_bros_no_hands_writes() {
        let mut rom = make_test_rom();
        apply_macobra_patches(&mut rom);

        // Helper landed in PRG011 free space.
        assert_eq!(
            rom.read_range(FS_BROS_NO_HANDS, BROS_NO_HANDS_SUB.len()),
            &BROS_NO_HANDS_SUB
        );
        // Follow the JSR operand actually written into the ROM and prove it
        // resolves to the helper bytes. This is deliberately *semantic*: it
        // does not recompute the expected operand (the previous version did,
        // with the same off-by-header arithmetic the production const used, so
        // it confirmed the bug instead of catching it — see issue #14). A
        // mis-aimed JSR lands somewhere other than FS_BROS_NO_HANDS, and the
        // bytes there won't match, regardless of *how* the operand was wrong.
        let hook = rom.read_range(BROS_NO_HANDS_HOOK, 3);
        assert_eq!(hook[0], 0x20, "hook must be a JSR");
        let target_cpu = u16::from_le_bytes([hook[1], hook[2]]) as usize;
        assert!(
            (0xA000..0xC000).contains(&target_cpu),
            "JSR target must stay in the PRG011 $A000-$BFFF window, got {target_cpu:#06X}"
        );
        // PRG011 is bank 11: file = 11 * 0x2000 + 0x10 (iNES header) + (cpu - $A000).
        let target_file = 11 * 0x2000 + 0x10 + (target_cpu - 0xA000);
        assert_eq!(
            target_file, FS_BROS_NO_HANDS,
            "JSR must point at the registered helper allocation, not arbitrary free space"
        );
        assert_eq!(
            rom.read_range(target_file, BROS_NO_HANDS_SUB.len()),
            &BROS_NO_HANDS_SUB,
            "JSR operand must land on the helper bytes, not past them"
        );
        // Helper preserves vanilla behavior for non-hand tiles: its tail is the
        // original `CMP $7E98,Y` (D9 98 7E) the hook replaced.
        assert_eq!(&BROS_NO_HANDS_SUB[4..7], &[0xD9, 0x98, 0x7E]);
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
