//! **POC** — per-world map progress that survives leaving and re-entering a
//! world.
//!
//! The world-maze design needs one thing vanilla has no concept of: a world you
//! can come back to. Vanilla never returns you to a world, so it keeps exactly
//! one world's completion state and wipes it on every transition —
//! `Map_Completions` is 128 bytes at `$7D00`, split Mario / Luigi, "allows a
//! MAX of 4 map screens" per the disassembly, and `PRG030_84A0` ("initialize
//! the world map") opens by zeroing all 128 bytes:
//!
//! ```text
//! $84CD   A0 7F       LDY #$7F
//!         A9 00       LDA #$00
//!         99 00 7D    STA Map_Completions,Y
//!         88          DEY
//!         10 FA       BPL -6
//! ```
//!
//! Ten bytes, three whole instructions, nothing branches into the middle of it.
//! Replacing them is the entire hook — and by the time they run, `$84A0` has
//! already mapped PRG010 into `$C000` (its first act is `LDA #10 / STA
//! PAGE_C000`), so the replacement can live in PRG010's free space with no
//! trampoline and no always-mapped-bank scarcity.
//!
//! **What this POC does and does not prove.** It answers one question: with the
//! wipe replaced by a save/restore, does the map render correctly on return?
//! `$84A0` calls `Map_Reload_with_Completions` well after the wipe, so if the
//! array holds the right bits by then the tiles should come back beaten with no
//! further work. That is the hypothesis under test.
//!
//! **Luigi's half of `Map_Completions` is not spare storage.** The first cut of
//! this POC parked the other world at `$7D40` on the theory that one-player mode
//! never touches it. It does. Three sites mirror a completion into the *other*
//! player's array unconditionally, because a permanent map alteration has to
//! survive a game over:
//!
//! - `PRG011_BA7C`, level clear — "Fortress only... mark complete on both
//!   Players (so it remains after Game Over)"
//! - `MO_DoFortressFX` (PRG010) — "Mark lock busted / bridge built (Luigi)",
//!   `Map_Completions+$40,Y`
//! - `Map_SetCompletion_By_Poof` (PRG026) — "Rock removal sets completion bit
//!   for BOTH Players!"
//!
//! So clearing a fortress, busting a lock or smashing a rock in the live world
//! stamped the parked world's bank, and the next swap carried that stamp onto a
//! map it had nothing to do with — a completed tile appearing in World 2 at the
//! column World 1's was on. The parked bank now lives in genuinely unused SRAM
//! instead, which has the side benefit of proving that region is really free.
//!
//! **And the mirror is not private to Luigi either — the map reload applies
//! both halves.** `Map_Reload_with_Completions` loops to `CMP #$80`, all 128
//! bytes, folding the column index with `AND #$30`, which aliases Luigi's bytes
//! onto the same four screens as Mario's. Its own comment says so:
//!
//! ```text
//! ; Note: Loop goes through both Players sets of completion bits, but
//! ; this AND will basically cause 2 passes across the map...
//! ```
//!
//! So banking only Mario's half still leaked: a fortress cleared in World 1 left
//! its mirrored bit at `$7D40 + col`, and World 2's reload applied it to
//! whatever sat at that column — a pyramid, in the case this was found from.
//! Both halves are banked now, which also leaves the live world's game-over
//! semantics intact: `PRG030_9314` ANDs the two, and that is exactly how vanilla
//! keeps forts and locks broken while wiping plain level clears.
//!
//! It deliberately does **not** solve storage for eight worlds: 8 x 128 = 1024
//! bytes against 384 of declared-unused SRAM, so the real version has to store
//! bits per completable cell against a randomizer-emitted table rather than raw
//! columns — two bits, since the mirror is a real distinction the engine makes
//! and not a copy. At ~40 cells per world that is comfortable. Arithmetic, and
//! not what this POC is for.
//!
//! Also knowingly unhandled here: the player respawns at the world's start tile
//! rather than where they left, the per-world flags `$84A0` resets
//! (`Map_Anchored`, `Map_WhiteHouse`, `Map_CoinShip`, `Map_Got13Warp`) are not
//! banked, and with the wipe gone a game-over into a new game inherits the old
//! run's completions.

use crate::rom::Rom;

use super::completion_bits::{self, TRANSITION_FLAG};
use super::rom_data::{
    FS_PORTAL_EXIT, FS_RESTORE_ARRIVAL, FS_STASH_ARRIVAL, FS_WORLD_PERSIST_JUMP,
};

// CPU addresses of the two routines. PRG010 is mapped at $C000 whenever
// either hook runs — `$84A0` maps it itself, and `MO_NormalMoveEnter` lives
// in it — so CPU = $C000 + (file - 0x14010), the same arithmetic as the
// other PRG010 patches (`map_warp.rs`, `canoe_summon.rs`).
const WORLD_JUMP_CHECK_CPU: u16 = (0xC000 + FS_WORLD_PERSIST_JUMP - 0x14010) as u16;
// PRG011 is mapped at $A000 during the map, and `PRG011_ABBE` is its own code,
// so CPU = $A000 + (file - 0x16010).
const RESTORE_ARRIVAL_CPU: u16 = (0xC000 + FS_RESTORE_ARRIVAL - 0x14010) as u16;
const STASH_ARRIVAL_CPU: u16 = (0xC000 + FS_STASH_ARRIVAL - 0x14010) as u16;
// PRG030 is fixed at $8000-$9FFF, always mapped — file 0x3C010 is its $8000.
const PORTAL_EXIT_CPU: u16 = (0x8000 + FS_PORTAL_EXIT - 0x3C010) as u16;

// --- Engine symbols -----------------------------------------------------
//
// Verified against the ROM rather than read off the disassembly's labels:
// `$18` is the operand of the `LDA <Pad_Input / AND #$0F` direction test in
// `MO_NormalMoveEnter`, `$17` the `Pad_Holding` test sixteen bytes later.

/// `Pad_Holding` — buttons held, continuous.
const PAD_HOLDING: u8 = 0x17;
/// `Pad_Input` — buttons newly pressed this frame, one-shot.
const PAD_INPUT: u8 = 0x18;

const PAD_START: u8 = 0x10;
const PAD_SELECT: u8 = 0x20;

/// `World_Num`, 0-based.
const WORLD_NUM: u16 = 0x0727;

/// Where the pipeway's arrival coordinates wait out `Map_Init`.
///
/// Six bytes in the `$7A73` run, past the stencil scratch and its counters.
/// They used to sit at `$79D7`, inside what is now
/// [`completion_bits`]' packed region.
const ARRIVAL_Y: u16 = 0x7AB6;
const ARRIVAL_XHI: u16 = 0x7AB7;
const ARRIVAL_X: u16 = 0x7AB8;
const ARRIVAL_SCRL: u16 = 0x7AB9;
const ARRIVAL_SCRH: u16 = 0x7ABA;
const ARRIVAL_FLAG: u16 = 0x7ABB;

/// The player's map position, as `Map_Init` writes it (PRG011). Every one of
/// these is set from `Map_Y_Starts` and friends, so every one has to be
/// rewritten to land somewhere else:
///
/// ```text
/// LDA Map_Y_Starts,Y
/// STA Map_Entered_Y,X / STA Map_Previous_Y,X
/// ... STA Map_Entered_X,X / STA Map_Previous_X,X
/// ... STA Map_Entered_XHi,X / STA Map_Previous_XHi,X
/// ... STA Map_Prev_XOff2,X / STA Map_Prev_XHi2,X
/// ... STA Map_Prev_XOff,X  / STA Map_Prev_XHi,X
/// ```
const MAP_ENTERED_Y: u16 = 0x7976;
const MAP_ENTERED_XHI: u16 = 0x7978;
const MAP_ENTERED_X: u16 = 0x797A;
const MAP_PREVIOUS_Y: u16 = 0x797E;
const MAP_PREVIOUS_XHI: u16 = 0x7980;
const MAP_PREVIOUS_X: u16 = 0x7982;
const MAP_PREV_XOFF2: u16 = 0x7986;
const MAP_PREV_XHI2: u16 = 0x7988;
const MAP_PREV_XOFF: u16 = 0x0722;
const MAP_PREV_XHI: u16 = 0x0724;

/// CPU address of `PRG030_84A0`, "initialize the world map". Reached from
/// exactly two places — the airship-cleared path (`INC World_Num`) and the warp
/// zone (`World_Num = Map_Warp_PrevWorld`) — and it never returns: it falls
/// through into `WorldMap_Loop`.
const WORLD_MAP_INIT_CPU: u16 = 0x84A0;

/// The 10-byte `Map_Completions` wipe inside `PRG030_84A0` (CPU `$84CD`).
const WIPE_OFFSET: usize = 0x3C4DD;
/// Length of that run. `completion_bits` writes its own call into the first
/// three bytes and pads the rest; the arrival restore goes into the padding, so
/// only the tests here need the length.
#[cfg(test)]
const WIPE_LEN: usize = 10;

/// `MO_NormalMoveEnter` (CPU `$CDCA`) — map operation `$D`, the normal
/// standing-on-the-map state, run every frame from `Map_DoOperation`. Its first
/// eight bytes are three whole instructions:
///
/// ```text
/// A9 00       LDA #$00
/// 8D 6E 79    STA Map_NoLoseTurn
/// 8D 73 79    STA Map_WasInPipeway
/// ```
const NORMAL_MOVE_OFFSET: usize = 0x14DDA;
const NORMAL_MOVE_LEN: usize = 8;

/// Vanilla bytes at [`NORMAL_MOVE_OFFSET`], for the hook check.
#[cfg(test)]
#[rustfmt::skip]
const NORMAL_MOVE_VANILLA: [u8; NORMAL_MOVE_LEN] = [
    0xA9, 0x00,             // LDA #$00
    0x8D, 0x6E, 0x79,       // STA Map_NoLoseTurn
    0x8D, 0x73, 0x79,       // STA Map_WasInPipeway
];

// --- Routines -----------------------------------------------------------

/// Put the player where the portal aimed them, after `Map_Init` has had its say.
///
/// `Map_Init` writes the destination world's start into ten variables, so
/// landing anywhere else means rewriting all ten — five values, each in two
/// places. The values themselves are whatever `ObjNorm_PipewayCtlr` computed
/// on the way out of the transit room, copied byte for byte: those bytes are
/// what vanilla uses to place you on a same-world pipe return, so their
/// encoding is right by construction and needs no arithmetic here.
///
/// A no-op unless the pipe portal set the flag, which is why this can sit
/// unconditionally at the end of the completion swap.
#[rustfmt::skip]
const RESTORE_ARRIVAL: [u8; 56] = [
    0xAD, ARRIVAL_FLAG as u8, (ARRIVAL_FLAG >> 8) as u8,        //  0: LDA ARRIVAL_FLAG
    0xF0, 0x32,                                                 //  3: BEQ +50 → done
    0xA9, 0x00,                                                 //  5: LDA #$00
    0x8D, ARRIVAL_FLAG as u8, (ARRIVAL_FLAG >> 8) as u8,        //  7: STA ARRIVAL_FLAG

    0xAD, ARRIVAL_Y as u8, (ARRIVAL_Y >> 8) as u8,              // 10: LDA ARRIVAL_Y
    0x8D, MAP_ENTERED_Y as u8, (MAP_ENTERED_Y >> 8) as u8,      // 13: STA Map_Entered_Y
    0x8D, MAP_PREVIOUS_Y as u8, (MAP_PREVIOUS_Y >> 8) as u8,    // 16: STA Map_Previous_Y

    0xAD, ARRIVAL_X as u8, (ARRIVAL_X >> 8) as u8,              // 19: LDA ARRIVAL_X
    0x8D, MAP_ENTERED_X as u8, (MAP_ENTERED_X >> 8) as u8,      // 22: STA Map_Entered_X
    0x8D, MAP_PREVIOUS_X as u8, (MAP_PREVIOUS_X >> 8) as u8,    // 25: STA Map_Previous_X

    0xAD, ARRIVAL_XHI as u8, (ARRIVAL_XHI >> 8) as u8,          // 28: LDA ARRIVAL_XHI
    0x8D, MAP_ENTERED_XHI as u8, (MAP_ENTERED_XHI >> 8) as u8,  // 31: STA Map_Entered_XHi
    0x8D, MAP_PREVIOUS_XHI as u8,
          (MAP_PREVIOUS_XHI >> 8) as u8,                        // 34: STA Map_Previous_XHi

    0xAD, ARRIVAL_SCRL as u8, (ARRIVAL_SCRL >> 8) as u8,        // 37: LDA ARRIVAL_SCRL
    0x8D, MAP_PREV_XOFF as u8, (MAP_PREV_XOFF >> 8) as u8,      // 40: STA Map_Prev_XOff
    0x8D, MAP_PREV_XOFF2 as u8, (MAP_PREV_XOFF2 >> 8) as u8,    // 43: STA Map_Prev_XOff2

    0xAD, ARRIVAL_SCRH as u8, (ARRIVAL_SCRH >> 8) as u8,        // 46: LDA ARRIVAL_SCRH
    0x8D, MAP_PREV_XHI as u8, (MAP_PREV_XHI >> 8) as u8,        // 49: STA Map_Prev_XHi
    0x8D, MAP_PREV_XHI2 as u8, (MAP_PREV_XHI2 >> 8) as u8,      // 52: STA Map_Prev_XHi2

    0x60,                                                       // 55: RTS   ; done
];

/// Trigger: hold SELECT, press START on the map to jump to the other world.
///
/// Opens with the three instructions displaced from `MO_NormalMoveEnter`, so
/// the hook is a plain `JSR` and the vanilla path is unchanged when the combo
/// is not held.
///
/// `Pad_Holding` for SELECT and `Pad_Input` for START, rather than both
/// one-shot: two buttons landing on the same frame is not a thing a person can
/// do reliably. START also opens the inventory on this frame, which does not
/// matter — `$84A0` clears `Inventory_Open` on the way through.
///
/// **`LDX #$FF / TXS` before the jump is load-bearing.** `$84A0` never returns;
/// it falls into `WorldMap_Loop`. Vanilla's two call sites reach it by `JMP`
/// from a shallow, consistent stack depth and simply abandon the frame. This
/// trigger fires from inside `Map_DoOperation`, several frames deeper, so
/// without resetting the stack every jump would leak a few bytes and a long
/// ping-pong session would eventually wrap it. Resetting is correct precisely
/// because nothing above the map loop is ever coming back — vanilla does the
/// same `DEX / TXS` at reset for the same reason.
#[rustfmt::skip]
const WORLD_JUMP_CHECK: [u8; 48] = [
    // ----- displaced from MO_NormalMoveEnter -----
    0xA9, 0x00,             //  0: LDA #$00
    0x8D, 0x6E, 0x79,       //  2: STA Map_NoLoseTurn
    0x8D, 0x73, 0x79,       //  5: STA Map_WasInPipeway

    // Mid-scroll the map is between states and a jump from here misbehaves.
    // `PRG010_CDDC` tests the same counter one instruction later.
    0xAD, MAP_PAN_COUNT as u8, (MAP_PAN_COUNT >> 8) as u8,  //  8: LDA Map_Pan_Count
    0xD0, 0x22,             // 11: BNE +34 -> done

    // ----- SELECT held + START pressed? -----
    0xA5, PAD_HOLDING,      // 13: LDA Pad_Holding
    0x29, PAD_SELECT,       // 15: AND #PAD_SELECT
    0xF0, 0x1C,             // 17: BEQ +28 -> done
    0xA5, PAD_INPUT,        // 19: LDA Pad_Input
    0x29, PAD_START,        // 21: AND #PAD_START
    0xF0, 0x16,             // 23: BEQ +22 -> done

    // ----- jump: next world, wrapping at 8, and restart the map -----
    0xA9, 0x01,             // 25: LDA #$01          ; this is a transition,
    0x8D, TRANSITION_FLAG as u8,
          (TRANSITION_FLAG >> 8) as u8,              // 27: STA TRANSITION_FLAG
                                                    //     not a new game
    0xAD, WORLD_NUM as u8, (WORLD_NUM >> 8) as u8,  // 30: LDA World_Num
    0x18,                   // 33: CLC
    0x69, 0x01,             // 34: ADC #$01
    0x29, 0x07,             // 36: AND #$07          ; all eight, not a ping-pong
    0x8D, WORLD_NUM as u8, (WORLD_NUM >> 8) as u8,  // 38: STA World_Num
    0xA2, 0xFF,             // 41: LDX #$FF
    0x9A,                   // 43: TXS               ; see doc comment
    0x4C, WORLD_MAP_INIT_CPU as u8,
          (WORLD_MAP_INIT_CPU >> 8) as u8,           // 44: JMP $84A0 (never returns)

    0x60,                   // 47: RTS               ; done
];

// --- Writer -------------------------------------------------------------

/// Install the POC: packed per-world completions, plus the SELECT+START jump.
///
/// The storage itself is [`completion_bits`]; this module contributes the
/// debug jump that cycles worlds and, with `pipe_portal`, the portal that lands
/// you on another world's map. The arrival restore is written into the padding
/// `completion_bits` leaves at the wipe site, so it runs on the same pass and
/// after `Map_Init` has had its say.
pub(crate) fn apply(rom: &mut Rom, pipe_portal: Option<u8>) {
    // Must come first: it owns the wipe site, and this module writes into the
    // bytes it leaves behind.
    completion_bits::apply(rom);

    rom.push_tag("world_persist");

    rom.write_range(FS_RESTORE_ARRIVAL, &RESTORE_ARRIVAL);
    rom.write_range(FS_WORLD_PERSIST_JUMP, &WORLD_JUMP_CHECK);

    // Second half of the displaced wipe: `completion_bits` put its own call in
    // the first three bytes and NOPs in the rest. Seven wasted bytes is not the
    // standard this project holds patches to; a shipped version would
    // restructure the surrounding init instead of padding it.
    rom.write_range(
        WIPE_OFFSET + 3,
        &[0x20, RESTORE_ARRIVAL_CPU as u8, (RESTORE_ARRIVAL_CPU >> 8) as u8],
    );

    // Hook MO_NormalMoveEnter for the trigger.
    let mut hook = [0xEA_u8; NORMAL_MOVE_LEN];
    hook[0] = 0x20; // JSR
    hook[1] = WORLD_JUMP_CHECK_CPU as u8;
    hook[2] = (WORLD_JUMP_CHECK_CPU >> 8) as u8;
    rom.write_range(NORMAL_MOVE_OFFSET, &hook);

    if let Some(dest_world) = pipe_portal {
        apply_pipe_portal(rom, dest_world);
    }

    rom.pop_tag();
}

// --- Pipe portal (POC round 3) ---------------------------------------------

/// `Map_WasInPipeway` — set by `ObjInit_PipewayCtlr` when you enter a pipe
/// transit room, and still set when the level exits.
const MAP_WAS_IN_PIPEWAY: u16 = 0x7973;

/// `Map_Pan_Count` — non-zero while the map is scrolling.
const MAP_PAN_COUNT: u16 = 0x0710;

/// `PRG030_9097`, "Exiting to map somehow" — the common return-from-level path
/// (CPU `$9097` = file 0x3D0A7). Its first five bytes are two whole
/// instructions:
///
/// ```text
/// A9 C0       LDA #$C0
/// 8D 00 01    STA Update_Select
/// ```
///
/// **This is why the trigger moved here from `PRG011_ABBE`.** That hook fired
/// after the origin world's map had already been reloaded, faded in and drawn,
/// so jumping from it flashed World 1 for a frame before World 2's intro. Here,
/// the origin map is never built at all.
const EXIT_HOOK_OFFSET: usize = 0x3D0A7;
const EXIT_HOOK_LEN: usize = 5;

/// Vanilla bytes at [`EXIT_HOOK_OFFSET`].
#[cfg(test)]
#[rustfmt::skip]
const EXIT_HOOK_VANILLA: [u8; EXIT_HOOK_LEN] = [
    0xA9, 0xC0,             // LDA #$C0
    0x8D, 0x00, 0x01,       // STA Update_Select
];

/// `JSR Map_Init` inside `PRG030_84A0` (CPU `$84AD` = file 0x3C4BD).
///
/// The trampoline site for the stash. `Map_Init` is what overwrites the
/// pipeway's arrival coordinates, and by this point `$84A0` has already mapped
/// PRG010 into `$C000` and PRG011 into `$A000` — so the replacement can live in
/// PRG010 and still reach `Map_Init` in PRG011.
const MAP_INIT_CALL_OFFSET: usize = 0x3C4BD;
const MAP_INIT_CALL_LEN: usize = 3;

/// `Map_Init` itself (PRG011, CPU `$A1D8`).
const MAP_INIT_CPU: u16 = 0xA1D8;

/// Leave for another world on the way out of a transit room.
///
/// Lives in PRG030, which is always mapped — it has to, because the banks at
/// level exit belong to the level, not the map.
///
/// Sets only the flag and the destination; the coordinates are captured later,
/// by [`STASH_ARRIVAL`], because `Map_Entered_*` survive untouched until
/// `Map_Init` runs.
///
/// The destination world is patched into the operand at
/// [`PORTAL_DEST_OPERAND`], so `--pipe-portal-world` can aim it anywhere.
#[rustfmt::skip]
const PORTAL_EXIT: [u8; 37] = [
    0xAD, MAP_WAS_IN_PIPEWAY as u8,
          (MAP_WAS_IN_PIPEWAY >> 8) as u8,                  //  0: LDA Map_WasInPipeway
    0xF0, 0x1A,                                             //  3: BEQ +26 -> not a pipe
    0xAD, WORLD_NUM as u8, (WORLD_NUM >> 8) as u8,          //  5: LDA World_Num
    0xD0, 0x15,                                             //  8: BNE +21 -> not World 1

    0xA9, 0x01,                                             // 10: LDA #$01
    0x8D, ARRIVAL_FLAG as u8, (ARRIVAL_FLAG >> 8) as u8,    // 12: STA ARRIVAL_FLAG
    0xA9, 0x01,                                             // 15: LDA #dest   (patched)
    0x8D, WORLD_NUM as u8, (WORLD_NUM >> 8) as u8,          // 17: STA World_Num
    0xA9, 0x01,                                             // 20: LDA #$01    ; a transition,
    0x8D, TRANSITION_FLAG as u8,
          (TRANSITION_FLAG >> 8) as u8,                     // 22: STA TRANSITION_FLAG
                                                            //     not a new game
    0xA2, 0xFF,                                             // 25: LDX #$FF
    0x9A,                                                   // 27: TXS
    0x4C, WORLD_MAP_INIT_CPU as u8,
          (WORLD_MAP_INIT_CPU >> 8) as u8,                  // 28: JMP $84A0 (never returns)

    // ----- ordinary exit: the displaced instructions -----
    0xA9, 0xC0,                                             // 31: LDA #$C0
    0x8D, 0x00, 0x01,                                       // 33: STA Update_Select
    0x60,                                                   // 36: RTS
];

/// Offset of the destination-world operand inside [`PORTAL_EXIT`].
pub(crate) const PORTAL_DEST_OPERAND: usize = 16;

/// Capture the pipeway's arrival coordinates just before `Map_Init` erases them.
///
/// Replaces `$84A0`'s own `JSR Map_Init` and calls it afterwards, so the stash
/// lands in the one window where the controller's answer is still live and
/// `Map_Init` has not yet run.
#[rustfmt::skip]
const STASH_ARRIVAL: [u8; 38] = [
    0xAD, ARRIVAL_FLAG as u8, (ARRIVAL_FLAG >> 8) as u8,        //  0: LDA ARRIVAL_FLAG
    0xF0, 0x1E,                                                 //  3: BEQ +30 → no portal
    0xAD, MAP_ENTERED_Y as u8, (MAP_ENTERED_Y >> 8) as u8,      //  5: LDA Map_Entered_Y
    0x8D, ARRIVAL_Y as u8, (ARRIVAL_Y >> 8) as u8,              //  8: STA ARRIVAL_Y
    0xAD, MAP_ENTERED_XHI as u8,
          (MAP_ENTERED_XHI >> 8) as u8,                         // 11: LDA Map_Entered_XHi
    0x8D, ARRIVAL_XHI as u8, (ARRIVAL_XHI >> 8) as u8,          // 14: STA ARRIVAL_XHI
    0xAD, MAP_ENTERED_X as u8, (MAP_ENTERED_X >> 8) as u8,      // 17: LDA Map_Entered_X
    0x8D, ARRIVAL_X as u8, (ARRIVAL_X >> 8) as u8,              // 20: STA ARRIVAL_X
    0xAD, MAP_PREV_XOFF as u8, (MAP_PREV_XOFF >> 8) as u8,      // 23: LDA Map_Prev_XOff
    0x8D, ARRIVAL_SCRL as u8, (ARRIVAL_SCRL >> 8) as u8,        // 26: STA ARRIVAL_SCRL
    0xAD, MAP_PREV_XHI as u8, (MAP_PREV_XHI >> 8) as u8,        // 29: LDA Map_Prev_XHi
    0x8D, ARRIVAL_SCRH as u8, (ARRIVAL_SCRH >> 8) as u8,        // 32: STA ARRIVAL_SCRH
    0x4C, MAP_INIT_CPU as u8, (MAP_INIT_CPU >> 8) as u8,        // 35: JMP Map_Init
];

/// Install the pipe portal: a pipe taken in World 1 comes out in `dest_world`.
fn apply_pipe_portal(rom: &mut Rom, dest_world: u8) {
    let mut exit = PORTAL_EXIT;
    exit[PORTAL_DEST_OPERAND] = dest_world;
    rom.write_range(FS_PORTAL_EXIT, &exit);
    rom.write_range(FS_STASH_ARRIVAL, &STASH_ARRIVAL);

    let mut hook = [0xEA_u8; EXIT_HOOK_LEN];
    hook[0] = 0x20; // JSR
    hook[1] = PORTAL_EXIT_CPU as u8;
    hook[2] = (PORTAL_EXIT_CPU >> 8) as u8;
    rom.write_range(EXIT_HOOK_OFFSET, &hook);

    // `JSR Map_Init` → `JSR StashArrival`, which calls Map_Init itself.
    let mut trampoline = [0u8; MAP_INIT_CALL_LEN];
    trampoline[0] = 0x20; // JSR
    trampoline[1] = STASH_ARRIVAL_CPU as u8;
    trampoline[2] = (STASH_ARRIVAL_CPU >> 8) as u8;
    rom.write_range(MAP_INIT_CALL_OFFSET, &trampoline);
}

#[cfg(test)]
mod asm_checks {
    use super::*;
    use crate::randomize::rom_data::asm;

    #[test]
    fn world_jump_check_is_well_formed() {
        asm::check(&WORLD_JUMP_CHECK)
            .allocation(FS_WORLD_PERSIST_JUMP)
            .origin(WORLD_JUMP_CHECK_CPU)
            .assert_ok();
    }

    /// Both hooks displace whole instructions, and the vanilla bytes they
    /// displace are what this module claims they are.
    #[test]
    fn hooks_displace_whole_instructions() {
        let Some(rom) = load_vanilla() else { return };

        // `completion_bits` owns the wipe site now; this module only writes
        // the arrival restore into the padding it leaves.
        let mut patched = rom.clone();
        completion_bits::apply(&mut patched);
        let wipe = patched.read_range(WIPE_OFFSET, WIPE_LEN).to_vec();

        let mv = rom.read_range(NORMAL_MOVE_OFFSET, NORMAL_MOVE_LEN);
        assert_eq!(mv, NORMAL_MOVE_VANILLA, "MO_NormalMoveEnter has moved");

        assert_eq!(
            &wipe[3..6],
            &[0xEA, 0xEA, 0xEA],
            "completion_bits must leave NOPs at the wipe site for the arrival restore"
        );

        let mut jsr_check = [0xEA_u8; NORMAL_MOVE_LEN];
        jsr_check[0] = 0x20;
        jsr_check[1] = WORLD_JUMP_CHECK_CPU as u8;
        jsr_check[2] = (WORLD_JUMP_CHECK_CPU >> 8) as u8;
        asm::check(&WORLD_JUMP_CHECK)
            .allocation(FS_WORLD_PERSIST_JUMP)
            .origin(WORLD_JUMP_CHECK_CPU)
            .hook(&NORMAL_MOVE_VANILLA, 0, &jsr_check)
            .assert_ok();
    }

    /// The displaced instructions are re-executed by the trampoline, in order.
    /// Getting this wrong leaves `Map_NoLoseTurn` stale, which quietly stops
    /// the player losing their turn after a level.
    #[test]
    fn trampoline_replays_what_it_displaced() {
        assert_eq!(
            WORLD_JUMP_CHECK[..NORMAL_MOVE_LEN],
            NORMAL_MOVE_VANILLA,
            "the trampoline must open with the instructions the hook overwrote"
        );
    }

    /// The jump targets the real world-map init.
    #[test]
    fn jump_targets_world_map_init() {
        // Decoding is what proves it: `asm::check` walks instruction
        // boundaries, so a `JMP` found at 31 really is the final instruction
        // and not an operand read as an opcode.
        assert_eq!(WORLD_JUMP_CHECK[44], 0x4C, "trigger must end in a JMP");
        assert_eq!(
            u16::from_le_bytes([WORLD_JUMP_CHECK[45], WORLD_JUMP_CHECK[46]]),
            0x84A0,
            "trigger must JMP PRG030_84A0, the world-map init"
        );
    }

    #[test]
    fn restore_arrival_is_well_formed() {
        asm::check(&RESTORE_ARRIVAL)
            .allocation(FS_RESTORE_ARRIVAL)
            .origin(RESTORE_ARRIVAL_CPU)
            .assert_ok();
    }

    /// The arrival restore must actually be reached, or a portal's arrival is
    /// silently dropped and you land on the destination world's start tile —
    /// the version of this that is worse than no transit room at all.
    ///
    /// It rides in the padding `completion_bits` leaves at the wipe site, which
    /// is a shared 10-byte run and exactly the kind of arrangement that breaks
    /// quietly when either side moves.
    #[test]
    fn the_wipe_site_calls_the_arrival_restore() {
        let Some(rom) = load_vanilla() else { return };
        let mut patched = rom.clone();
        apply(&mut patched, None);
        let wipe = patched.read_range(WIPE_OFFSET, WIPE_LEN);
        assert_eq!(wipe[3], 0x20, "the arrival restore must be called, not fallen into");
        assert_eq!(
            u16::from_le_bytes([wipe[4], wipe[5]]),
            RESTORE_ARRIVAL_CPU,
            "and the call must name the restore"
        );
    }

    /// Every variable `Map_Init` sets from `Map_Y_Starts` has to be rewritten.
    /// Missing one leaves the player half-moved — drawn at the portal's tile
    /// but resuming at the world's start after a death, or vice versa.
    #[test]
    fn restore_covers_every_position_variable_map_init_writes() {
        let written: Vec<u16> = RESTORE_ARRIVAL
            .windows(3)
            .filter(|w| w[0] == 0x8D)
            .map(|w| u16::from_le_bytes([w[1], w[2]]))
            .collect();
        for (addr, name) in [
            (MAP_ENTERED_Y, "Map_Entered_Y"),
            (MAP_ENTERED_X, "Map_Entered_X"),
            (MAP_ENTERED_XHI, "Map_Entered_XHi"),
            (MAP_PREVIOUS_Y, "Map_Previous_Y"),
            (MAP_PREVIOUS_X, "Map_Previous_X"),
            (MAP_PREVIOUS_XHI, "Map_Previous_XHi"),
            (MAP_PREV_XOFF, "Map_Prev_XOff"),
            (MAP_PREV_XHI, "Map_Prev_XHi"),
            (MAP_PREV_XOFF2, "Map_Prev_XOff2"),
            (MAP_PREV_XHI2, "Map_Prev_XHi2"),
        ] {
            assert!(written.contains(&addr), "restore never writes {name} (${addr:04X})");
        }
    }

    #[test]
    fn portal_exit_is_well_formed() {
        let mut exit = PORTAL_EXIT;
        exit[PORTAL_DEST_OPERAND] = 1;
        asm::check(&exit).allocation(FS_PORTAL_EXIT).origin(PORTAL_EXIT_CPU).assert_ok();
    }

    #[test]
    fn stash_arrival_is_well_formed() {
        asm::check(&STASH_ARRIVAL)
            .allocation(FS_STASH_ARRIVAL)
            .origin(STASH_ARRIVAL_CPU)
            .assert_ok();
    }

    /// Both hooks displace whole instructions, and each replacement carries on
    /// where the original left off: the exit hook replays its two, and the
    /// stash calls the `Map_Init` it displaced.
    #[test]
    fn portal_hooks_displace_whole_instructions() {
        let Some(rom) = load_vanilla() else { return };
        assert_eq!(
            rom.read_range(EXIT_HOOK_OFFSET, EXIT_HOOK_LEN),
            EXIT_HOOK_VANILLA,
            "PRG030_9097 has moved"
        );
        assert_eq!(
            PORTAL_EXIT[31..36],
            EXIT_HOOK_VANILLA,
            "the ordinary-exit path must replay what the hook overwrote"
        );

        let call = rom.read_range(MAP_INIT_CALL_OFFSET, MAP_INIT_CALL_LEN);
        assert_eq!(
            call,
            [0x20, MAP_INIT_CPU as u8, (MAP_INIT_CPU >> 8) as u8],
            "$84AD is not the JSR Map_Init this trampoline replaces"
        );
        assert_eq!(
            STASH_ARRIVAL[35..38],
            [0x4C, MAP_INIT_CPU as u8, (MAP_INIT_CPU >> 8) as u8],
            "the stash must still call the Map_Init it displaced"
        );

        let mut exit = PORTAL_EXIT;
        exit[PORTAL_DEST_OPERAND] = 1;
        let mut jsr = [0xEA_u8; EXIT_HOOK_LEN];
        jsr[0] = 0x20;
        jsr[1] = PORTAL_EXIT_CPU as u8;
        jsr[2] = (PORTAL_EXIT_CPU >> 8) as u8;
        asm::check(&exit)
            .allocation(FS_PORTAL_EXIT)
            .origin(PORTAL_EXIT_CPU)
            .hook(&EXIT_HOOK_VANILLA, 0, &jsr)
            .assert_ok();
    }

    /// The destination world really is the operand `--pipe-portal-world` sets.
    #[test]
    fn portal_destination_is_patched() {
        let Some(rom) = load_vanilla() else { return };
        for world in 1u8..8 {
            let mut patched = rom.clone();
            apply(&mut patched, Some(world));
            assert_eq!(
                patched.read_byte(FS_PORTAL_EXIT + PORTAL_DEST_OPERAND),
                world,
                "destination operand"
            );
        }
    }

    /// Without the flag, neither the level-exit path nor `$84A0`'s call to
    /// `Map_Init` is touched.
    #[test]
    fn pipe_portal_is_opt_in() {
        let Some(rom) = load_vanilla() else { return };
        let mut patched = rom.clone();
        apply(&mut patched, None);
        assert_eq!(
            patched.read_range(EXIT_HOOK_OFFSET, EXIT_HOOK_LEN),
            EXIT_HOOK_VANILLA,
            "the level-exit hook must not be installed unless asked for"
        );
        assert_eq!(
            patched.read_range(MAP_INIT_CALL_OFFSET, MAP_INIT_CALL_LEN),
            [0x20, MAP_INIT_CPU as u8, (MAP_INIT_CPU >> 8) as u8],
            "Map_Init must still be called directly unless asked for"
        );
    }

    /// World 1 owns no pipe destinations, which is what lets the portal be
    /// identified by `World_Num` alone with no table. If that ever changes,
    /// the POC's shortcut is wrong and this says so.
    #[test]
    fn world_one_has_no_vanilla_pipe_destinations() {
        let w1: Vec<u8> = crate::randomize::rom_data::DEST_TO_WORLD
            .iter()
            .filter(|&&(_, w)| w == 0)
            .map(|&(d, _)| d)
            .collect();
        assert!(w1.is_empty(), "W1 gained pipe destinations {w1:?} — the portal needs a table now");
    }

    fn load_vanilla() -> Option<Rom> {
        let data = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&data).ok()
    }
}
