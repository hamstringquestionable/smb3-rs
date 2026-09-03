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
//! It deliberately does **not** solve storage for eight worlds. Two worlds need
//! no new RAM at all, because one-player mode never touches Luigi's half: the
//! live world sits at `$7D00` and the other world parks at `$7D40`, so a
//! transition is just a 64-byte **swap**. Eight worlds need 512 bytes against a
//! largest free SRAM run of 105 (`$7997–$79FF`, which nothing in the
//! disassembly references), so the real version has to store one bit per
//! completable cell against a randomizer-emitted table rather than raw columns.
//! That is arithmetic, and it is not what this POC is for.
//!
//! Also knowingly unhandled here: the player respawns at the world's start tile
//! rather than where they left, the per-world flags `$84A0` resets
//! (`Map_Anchored`, `Map_WhiteHouse`, `Map_CoinShip`, `Map_Got13Warp`) are not
//! banked, and with the wipe gone a game-over into a new game inherits the old
//! run's completions.

use crate::rom::Rom;

use super::rom_data::{FS_WORLD_PERSIST_JUMP, FS_WORLD_PERSIST_SWAP};

// CPU addresses of the two routines. PRG010 is mapped at $C000 whenever
// either hook runs — `$84A0` maps it itself, and `MO_NormalMoveEnter` lives
// in it — so CPU = $C000 + (file - 0x14010), the same arithmetic as the
// other PRG010 patches (`map_warp.rs`, `canoe_summon.rs`).
const SWAP_COMPLETIONS_CPU: u16 = (0xC000 + FS_WORLD_PERSIST_SWAP - 0x14010) as u16;
const WORLD_JUMP_CHECK_CPU: u16 = (0xC000 + FS_WORLD_PERSIST_JUMP - 0x14010) as u16;

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

/// CPU address of `PRG030_84A0`, "initialize the world map". Reached from
/// exactly two places — the airship-cleared path (`INC World_Num`) and the warp
/// zone (`World_Num = Map_Warp_PrevWorld`) — and it never returns: it falls
/// through into `WorldMap_Loop`.
const WORLD_MAP_INIT_CPU: u16 = 0x84A0;

/// The 10-byte `Map_Completions` wipe inside `PRG030_84A0` (CPU `$84CD`).
const WIPE_OFFSET: usize = 0x3C4DD;
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

/// Swap Mario's completion array with Luigi's.
///
/// The whole two-world POC. `$7D00` is the live world; `$7D40` parks the other
/// one. One-player mode never reads Luigi's half — the mega-map fold spent it
/// the same way to reach eight map pages — so this costs no RAM at all.
///
/// Called in place of the wipe, so A/X/Y are all free: the wipe itself
/// clobbered A and Y, and the next thing `$84A0` does is `JSR
/// Sprite_RAM_Clear`.
///
/// `LDX abs,Y` (`$BE`) is what makes the swap 19 bytes rather than a
/// two-pass copy through a scratch buffer.
#[rustfmt::skip]
const SWAP_COMPLETIONS: [u8; 19] = [
    0xA0, 0x3F,             //  0: LDY #$3F        ; 64 columns, counting down
    0xB9, 0x00, 0x7D,       //  2: LDA $7D00,Y     ; loop: Mario's byte
    0xBE, 0x40, 0x7D,       //  5: LDX $7D40,Y     ; the parked world's byte
    0x99, 0x40, 0x7D,       //  8: STA $7D40,Y
    0x8A,                   // 11: TXA
    0x99, 0x00, 0x7D,       // 12: STA $7D00,Y
    0x88,                   // 15: DEY
    0x10, 0xF0,             // 16: BPL -16 → loop
    0x60,                   // 18: RTS
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
const WORLD_JUMP_CHECK: [u8; 35] = [
    // ----- displaced from MO_NormalMoveEnter -----
    0xA9, 0x00,             //  0: LDA #$00
    0x8D, 0x6E, 0x79,       //  2: STA Map_NoLoseTurn
    0x8D, 0x73, 0x79,       //  5: STA Map_WasInPipeway

    // ----- SELECT held + START pressed? -----
    0xA5, PAD_HOLDING,      //  8: LDA Pad_Holding
    0x29, PAD_SELECT,       // 10: AND #PAD_SELECT
    0xF0, 0x14,             // 12: BEQ +20 → done
    0xA5, PAD_INPUT,        // 14: LDA Pad_Input
    0x29, PAD_START,        // 16: AND #PAD_START
    0xF0, 0x0E,             // 18: BEQ +14 → done

    // ----- jump: ping-pong World_Num bit 0, restart the map -----
    0xAD, WORLD_NUM as u8, (WORLD_NUM >> 8) as u8,          // 20: LDA World_Num
    0x49, 0x01,                                             // 23: EOR #$01
    0x8D, WORLD_NUM as u8, (WORLD_NUM >> 8) as u8,          // 25: STA World_Num
    0xA2, 0xFF,             // 28: LDX #$FF
    0x9A,                   // 30: TXS              ; see doc comment
    0x4C, WORLD_MAP_INIT_CPU as u8, (WORLD_MAP_INIT_CPU >> 8) as u8,  // 31: JMP $84A0 (never returns)

    0x60,                   // 34: RTS              ; done
];

// --- Writer -------------------------------------------------------------

/// Install the POC: bank completions across world transitions, and add the
/// SELECT+START jump.
pub(crate) fn apply(rom: &mut Rom) {
    rom.push_tag("world_persist");

    rom.write_range(FS_WORLD_PERSIST_SWAP, &SWAP_COMPLETIONS);
    rom.write_range(FS_WORLD_PERSIST_JUMP, &WORLD_JUMP_CHECK);

    // Replace the wipe with `JSR SwapCompletions`, padding the rest of the
    // displaced run with NOPs. Three bytes for seven wasted is not the standard
    // this project holds patches to; a shipped version would restructure the
    // surrounding init instead. It stays here because a POC that rearranges
    // `$84A0` is a POC testing two things at once.
    let mut wipe = [0xEA_u8; WIPE_LEN];
    wipe[0] = 0x20; // JSR
    wipe[1] = SWAP_COMPLETIONS_CPU as u8;
    wipe[2] = (SWAP_COMPLETIONS_CPU >> 8) as u8;
    rom.write_range(WIPE_OFFSET, &wipe);

    // Hook MO_NormalMoveEnter for the trigger.
    let mut hook = [0xEA_u8; NORMAL_MOVE_LEN];
    hook[0] = 0x20; // JSR
    hook[1] = WORLD_JUMP_CHECK_CPU as u8;
    hook[2] = (WORLD_JUMP_CHECK_CPU >> 8) as u8;
    rom.write_range(NORMAL_MOVE_OFFSET, &hook);

    rom.pop_tag();
}

#[cfg(test)]
mod asm_checks {
    use super::*;
    use crate::randomize::rom_data::asm;

    #[test]
    fn swap_completions_is_well_formed() {
        asm::check(&SWAP_COMPLETIONS)
            .allocation(FS_WORLD_PERSIST_SWAP)
            .origin(SWAP_COMPLETIONS_CPU)
            .assert_ok();
    }

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

        let wipe = rom.read_range(WIPE_OFFSET, WIPE_LEN).to_vec();
        assert_eq!(
            wipe,
            vec![0xA0, 0x7F, 0xA9, 0x00, 0x99, 0x00, 0x7D, 0x88, 0x10, 0xFA],
            "the Map_Completions wipe is not where this module thinks it is"
        );

        let mv = rom.read_range(NORMAL_MOVE_OFFSET, NORMAL_MOVE_LEN);
        assert_eq!(mv, NORMAL_MOVE_VANILLA, "MO_NormalMoveEnter has moved");

        let mut jsr_swap = [0xEA_u8; WIPE_LEN];
        jsr_swap[0] = 0x20;
        jsr_swap[1] = SWAP_COMPLETIONS_CPU as u8;
        jsr_swap[2] = (SWAP_COMPLETIONS_CPU >> 8) as u8;
        asm::check(&SWAP_COMPLETIONS)
            .allocation(FS_WORLD_PERSIST_SWAP)
            .origin(SWAP_COMPLETIONS_CPU)
            .hook(&wipe, 0, &jsr_swap)
            .assert_ok();

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
        assert_eq!(WORLD_JUMP_CHECK[31], 0x4C, "trigger must end in a JMP");
        assert_eq!(
            u16::from_le_bytes([WORLD_JUMP_CHECK[32], WORLD_JUMP_CHECK[33]]),
            0x84A0,
            "trigger must JMP PRG030_84A0, the world-map init"
        );
    }

    fn load_vanilla() -> Option<Rom> {
        let data = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&data).ok()
    }
}
