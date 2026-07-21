//! Two-player map "warp to partner" escape hatch (always on).
//!
//! In 2-player mode both players share one loaded overworld map, and movable
//! map objects (the canoe, wandering Hammer Bros) are common to both — so one
//! player can strand the other. The classic case is the `8s are Wild` W8 canoe:
//! player A sails it to an island and ends their turn there, leaving player B on
//! the mainland with no boat to board. Nothing lets B cross.
//!
//! This adds a manual recovery: while resting on the map, the current player
//! presses **Start+Select** to warp onto the other player's tile, reusing the
//! game's own death/turn-return flow. That flow is a *top-level* transition:
//! it jumps to the map re-entry `PRG030_84D7` ($84D7, always-mapped bank),
//! which reloads the map screen (restoring each player from `Map_Entered_*`),
//! shows the lives card, then runs the skid (`MO_SkidToPrev`, Map_Operation
//! `$02`) that slides the player from its position to `Map_Previous_*` and
//! hands control back. The skid is *only* meant to run as part of that reload —
//! triggering it from inside the map loop animates the slide but never restarts
//! the player. So we mirror the turn-switch setup: snapshot the current
//! player's position/scroll into the `Map_Entered_*`/`Map_Prev_*` backups,
//! point `Map_Previous_*` at the partner, set Map_Operation `$02`, reset the
//! stack (a clean top-level transition, SP = $FF as `IntReset` establishes),
//! and jump to `$84D7`.
//!
//! Exploit-safe: the only reachable target is wherever the partner legitimately
//! stands, and fortress FX clears the lock/bridge in *both* players'
//! `Map_Completions` — so a lock is open for both or neither, and warping can
//! never land past a barrier the partner didn't already open for everyone.

use crate::rom::Rom;
use crate::randomize::rom_data::FS_MAP_WARP;

// Hook site: PRG010 `PRG010_CE78`, the "player not moving" input handler inside
// MO_NormalMoveEnter (Map_Operation $0D). X already holds Player_Current here.
// The vanilla code reads the A button for level entry / the 2P battle; we splice
// our Start+Select check in front of it. File offset of the `LDA
// Controller1Press / ORA Controller2Press / AND #$80` triplet (6 bytes).
const MAP_IDLE_INPUT_HOOK: usize = 0x14E88;

// CPU address of FS_MAP_WARP (PRG010 is mapped at $C000 during the map screen):
// $C000 + (0x15E13 - 0x14010) = $DE03. The hook JSRs here (same bank).
const MAP_WARP_CPU: u16 = (0xC000 + FS_MAP_WARP - 0x14010) as u16;

// CPU address of PRG010's WorldMap_UpdateAndDraw (the per-frame map exit), used
// by the same-screen path to run the skid in the live loop.
const WORLDMAP_UPDATE_DRAW_CPU: u16 = 0xCF29;

// CPU address of the map re-entry PRG030_84D7 (PRG030 is the fixed bank always
// mapped at $8000, so it's reachable from anywhere during the map screen). It
// disables rendering, clears/redraws the nametables, restores each player from
// Map_Entered_*, reloads the scroll from Map_Prev_*, and returns to the map
// loop — the same reload the game runs on every turn/level return.
const MAP_REENTRY_CPU: u16 = 0x84D7;

/// The warp routine (PRG010 free space). On entry X = Player_Current.
///
/// Guards: 2-player only, current player holding Start+Select, partner alive.
/// When it fires it branches on whether the partner is on the same map screen:
///
/// - **Same screen** (`World_Map_XHi` matches): point `Map_Previous_*` at the
///   partner, back up the current scroll, and run the skid (`MO_SkidToPrev`,
///   Map_Operation `$02`) in the live map loop — the player *slides* onto the
///   partner in place, no reload. The camera never changes page, so there's no
///   re-pan afterward.
///
/// - **Different screen**: teleport via the game's own map-reload (`$84D7`) — the
///   screen blanks and redraws at the partner's tile, no animation. `Map_Entered_*`
///   and `Map_Previous_*` both point at the partner (zero-distance skid), and the
///   scroll backups are **page-aligned to the partner's screen** (XHi <- partner's
///   `World_Map_XHi`, XOff <- 0) so the camera lands on the right page even if the
///   partner never moved. The stack is reset (SP = $FF, as `IntReset` establishes)
///   because entering the reload abandons the current map-loop call chain.
///
/// Both paths zero the 15-byte `Map_March_Count` array so the skid's finalize
/// doesn't stall on a stale march/airship counter, and set `Map_NoLoseTurn` so the
/// post-skid `$0C` state resumes with the *same* player instead of switching to
/// the partner — the warping player keeps control, like a single-player
/// death-respawn. (`MO_NormalMoveEnter` re-clears `Map_NoLoseTurn` next frame.)
///
/// When it does not fire it re-executes the displaced `LDA/ORA/AND #$80` and
/// returns with the Z flag intact, so the vanilla `BEQ` after the hook still
/// decides the A-button path.
#[rustfmt::skip]
const MAP_WARP_ROUTINE: [u8; 162] = [
    0xAD, 0x2B, 0x07,       // LDA Total_Players ($072B) — 1 for 1P, 2 for 2P
    0xC9, 0x02,             // CMP #$02
    0xD0, 0x13,             // BNE pass           (not a 2-player game -> skip)
    0xB5, 0xF7,             // LDA Controller1,X  (held input, current player)
    0x29, 0x30,             // AND #$30           (Start | Select)
    0xC9, 0x30,             // CMP #$30           (both held?)
    0xD0, 0x0B,             // BNE pass
    0x8A,                   // TXA
    0x49, 0x01,             // EOR #$01
    0xA8,                   // TAY                (Y = the other player)
    0xB9, 0x36, 0x07,       // LDA Player_Lives,Y ($0736)
    0xC9, 0xFF,             // CMP #$FF
    0xD0, 0x07,             // BNE fire           (partner alive -> fire)
    // pass: re-execute the displaced input read, return with Z from AND #$80.
    0xA5, 0xF5,             // LDA Controller1Press ($F5)
    0x05, 0xF6,             // ORA Controller2Press ($F6)
    0x29, 0x80,             // AND #$80
    0x60,                   // RTS
    // fire: same screen as the partner?
    0xB5, 0x77,             // LDA World_Map_XHi,X (current)
    0xD9, 0x77, 0x00,       // CMP World_Map_XHi,Y (partner)
    0xD0, 0x37,             // BNE cross
    // ===== SAME-SCREEN: slide via the skid in the live map loop =====
    0xB9, 0x75, 0x00,       // LDA World_Map_Y,Y    ($0075)
    0x9D, 0x7E, 0x79,       // STA Map_Previous_Y,X  ($797E)
    0xB9, 0x77, 0x00,       // LDA World_Map_XHi,Y   ($0077)
    0x9D, 0x80, 0x79,       // STA Map_Previous_XHi,X ($7980)
    0xB9, 0x79, 0x00,       // LDA World_Map_X,Y     ($0079)
    0x9D, 0x82, 0x79,       // STA Map_Previous_X,X  ($7982)
    0xA5, 0xFD,             // LDA Horz_Scroll ($FD)
    0x9D, 0x86, 0x79,       // STA Map_Prev_XOff2,X  ($7986)
    0xA5, 0x12,             // LDA Horz_Scroll_Hi ($12)
    0x9D, 0x88, 0x79,       // STA Map_Prev_XHi2,X   ($7988)
    0xA9, 0x00,             // LDA #$00
    0xA2, 0x0E,             // LDX #$0E
    0x9D, 0x3C, 0x05,       // clr1: STA Map_March_Count,X ($053C)
    0xCA,                   // DEX
    0x10, 0xFA,             // BPL clr1
    0x85, 0xC5,             // STA Map_SkidBack ($C5)  (A=0)
    0xA9, 0x01,             // LDA #$01
    0x8D, 0x6E, 0x79,       // STA Map_NoLoseTurn ($796E)
    0xA9, 0x02,             // LDA #$02
    0x8D, 0x29, 0x07,       // STA Map_Operation ($0729) = MO_SkidToPrev
    0x68,                   // PLA
    0x68,                   // PLA
    0x4C, (WORLDMAP_UPDATE_DRAW_CPU & 0xFF) as u8, (WORLDMAP_UPDATE_DRAW_CPU >> 8) as u8, // JMP WorldMap_UpdateAndDraw
    // ===== cross: teleport via the map reload =====
    0xB9, 0x75, 0x00,       // LDA World_Map_Y,Y    ($0075)
    0x9D, 0x76, 0x79,       // STA Map_Entered_Y,X   ($7976)
    0x9D, 0x7E, 0x79,       // STA Map_Previous_Y,X  ($797E)
    0xB9, 0x77, 0x00,       // LDA World_Map_XHi,Y   ($0077)
    0x9D, 0x78, 0x79,       // STA Map_Entered_XHi,X ($7978)
    0x9D, 0x80, 0x79,       // STA Map_Previous_XHi,X ($7980)
    0x9D, 0x24, 0x07,       // STA Map_Prev_XHi,X    ($0724) — camera page
    0x9D, 0x88, 0x79,       // STA Map_Prev_XHi2,X   ($7988) — secondary
    0xB9, 0x79, 0x00,       // LDA World_Map_X,Y     ($0079)
    0x9D, 0x7A, 0x79,       // STA Map_Entered_X,X   ($797A)
    0x9D, 0x82, 0x79,       // STA Map_Previous_X,X  ($7982)
    0xA9, 0x00,             // LDA #$00
    0x9D, 0x22, 0x07,       // STA Map_Prev_XOff,X   ($0722)
    0x9D, 0x86, 0x79,       // STA Map_Prev_XOff2,X  ($7986)
    0x85, 0xC5,             // STA Map_SkidBack ($C5)
    0xA2, 0x0E,             // LDX #$0E
    0x9D, 0x3C, 0x05,       // clr2: STA Map_March_Count,X ($053C)
    0xCA,                   // DEX
    0x10, 0xFA,             // BPL clr2
    0xA9, 0x01,             // LDA #$01
    0x8D, 0x6E, 0x79,       // STA Map_NoLoseTurn ($796E)
    0xA9, 0x02,             // LDA #$02
    0x8D, 0x29, 0x07,       // STA Map_Operation ($0729)
    0xA2, 0xFF,             // LDX #$FF
    0x9A,                   // TXS
    0x4C, (MAP_REENTRY_CPU & 0xFF) as u8, (MAP_REENTRY_CPU >> 8) as u8, // JMP PRG030_84D7
];

/// Install the 2-player Start+Select "warp to partner" escape hatch.
pub fn apply_map_warp(rom: &mut Rom) {
    rom.write_range(FS_MAP_WARP, &MAP_WARP_ROUTINE);

    // Replace `LDA Controller1Press / ORA Controller2Press / AND #$80` (6 bytes)
    // with `JSR map_warp` + NOP padding. The following vanilla `BEQ` reads the Z
    // flag the routine leaves set on its non-firing path.
    let [lo, hi] = MAP_WARP_CPU.to_le_bytes();
    rom.write_range(MAP_IDLE_INPUT_HOOK, &[0x20, lo, hi, 0xEA, 0xEA, 0xEA]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::randomize::qol::test_support::make_test_rom;

    #[test]
    fn hook_and_routine_written() {
        let mut rom = make_test_rom();
        apply_map_warp(&mut rom);

        // Hook: JSR to the routine's CPU address, then NOP padding.
        let [lo, hi] = MAP_WARP_CPU.to_le_bytes();
        assert_eq!(
            rom.read_range(MAP_IDLE_INPUT_HOOK, 6),
            &[0x20, lo, hi, 0xEA, 0xEA, 0xEA]
        );
        // Routine: starts with `LDA Total_Players`, ends with `JMP $84D7`.
        assert_eq!(rom.read_range(FS_MAP_WARP, 3), &[0xAD, 0x2B, 0x07]);
        assert_eq!(
            rom.read_range(FS_MAP_WARP + MAP_WARP_ROUTINE.len() - 3, 3),
            &[0x4C, 0xD7, 0x84]
        );
    }
}
