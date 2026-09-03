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

use super::rom_data::{
    self, FS_CROSS_WORLD_FX, FS_CROSS_WORLD_TABLE, FS_WORLD_PERSIST_JUMP, FS_WORLD_PERSIST_SWAP,
};

// CPU addresses of the two routines. PRG010 is mapped at $C000 whenever
// either hook runs — `$84A0` maps it itself, and `MO_NormalMoveEnter` lives
// in it — so CPU = $C000 + (file - 0x14010), the same arithmetic as the
// other PRG010 patches (`map_warp.rs`, `canoe_summon.rs`).
const SWAP_COMPLETIONS_CPU: u16 = (0xC000 + FS_WORLD_PERSIST_SWAP - 0x14010) as u16;
const WORLD_JUMP_CHECK_CPU: u16 = (0xC000 + FS_WORLD_PERSIST_JUMP - 0x14010) as u16;
const CROSS_WORLD_FX_CPU: u16 = (0xC000 + FS_CROSS_WORLD_FX - 0x14010) as u16;
const SLOT_WORLD_CPU: u16 = (0xC000 + FS_CROSS_WORLD_TABLE - 0x14010) as u16;

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

/// Where the non-live world's completion arrays park — one bank per half.
///
/// The disassembly declares 384 bytes of SRAM as bare anonymous `.ds` runs, and
/// these are its two largest: `$7997-$79FF` (105) and `$7A73-$7ADF` (109). In
/// each case that declaration is the *only* mention of any address in the range
/// anywhere in the disassembly — nothing reads or writes them. Unlike the
/// context-reused zero-page blocks, which the disassembly marks with explicit
/// `.org`s, this is plain untouched SRAM. Using it here is deliberate: the
/// eight-world version depends on that budget, so the POC may as well prove it.
///
/// 64 bytes taken from each; 41 and 45 left.
const PARKED_MARIO: u16 = 0x7997;
const PARKED_LUIGI: u16 = 0x7A73;

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

/// Exchange the live world's completion arrays with the parked world's.
///
/// The whole two-world POC. `$7D00`/`$7D40` are the live world's Mario and
/// mirror halves; [`PARKED_MARIO`]/[`PARKED_LUIGI`] hold the other one's.
///
/// **Both halves, not just Mario's.** The mirror is not scratch state: the map
/// reload applies it as a second pass, and the game-over merge ANDs the two.
/// Banking one and leaving the other is what made a World 1 fortress light up a
/// tile in World 2.
///
/// Called in place of the wipe, so A/X/Y are all free: the wipe itself
/// clobbered A and Y, and the next thing `$84A0` does is `JSR
/// Sprite_RAM_Clear`.
///
/// `LDX abs,Y` (`$BE`) is what keeps each half's exchange to 13 bytes rather
/// than a two-pass copy through a scratch buffer.
#[rustfmt::skip]
const SWAP_COMPLETIONS: [u8; 32] = [
    0xA0, 0x3F,                                             //  0: LDY #$3F  ; 64 columns, down
    // Mario's half
    0xB9, 0x00, 0x7D,                                       //  2: LDA $7D00,Y       ; loop
    0xBE, PARKED_MARIO as u8, (PARKED_MARIO >> 8) as u8,    //  5: LDX PARKED_MARIO,Y
    0x99, PARKED_MARIO as u8, (PARKED_MARIO >> 8) as u8,    //  8: STA PARKED_MARIO,Y
    0x8A,                                                   // 11: TXA
    0x99, 0x00, 0x7D,                                       // 12: STA $7D00,Y
    // The mirror half — permanent alterations live here too, and the reload
    // applies it as a second pass over the same four screens.
    0xB9, 0x40, 0x7D,                                       // 15: LDA $7D40,Y
    0xBE, PARKED_LUIGI as u8, (PARKED_LUIGI >> 8) as u8,    // 18: LDX PARKED_LUIGI,Y
    0x99, PARKED_LUIGI as u8, (PARKED_LUIGI >> 8) as u8,    // 21: STA PARKED_LUIGI,Y
    0x8A,                                                   // 24: TXA
    0x99, 0x40, 0x7D,                                       // 25: STA $7D40,Y
    0x88,                                                   // 28: DEY
    0x10, 0xE3,                                             // 29: BPL -29 → loop
    0x60,                                                   // 31: RTS
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

/// Which FX slot belongs to which world, one byte per slot.
///
/// Derived from `FORTRESS_ENTRIES` rather than written out: the FX slots are in
/// that table's order — W1 takes slot 0, W2 slot 1, W3 slots 2-3 — which is the
/// same fact `mega_map::van_fx` leans on, and the same one a hand-written list
/// got wrong there by dropping a row.
fn slot_world_table() -> Vec<u8> {
    rom_data::FORTRESS_ENTRIES.iter().map(|&(world, _)| world as u8).collect()
}

/// Install the POC: bank completions across world transitions, and add the
/// SELECT+START jump.
///
/// `cross_world_locks` additionally swaps World 1's and World 2's fortress FX
/// row entries, so each world's fortress opens the *other* world's lock, and
/// installs the routine that routes the completion into the right world's bank.
pub(crate) fn apply(rom: &mut Rom, cross_world_locks: bool) {
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

    if cross_world_locks {
        apply_cross_world_locks(rom);
    }

    rom.pop_tag();
}

// --- Cross-world locks (POC round 2) ---------------------------------------

/// `FortressFX_MapCompIdx` at CPU `$C7DF` — `(column, row bit)` per FX slot.
const FX_MAP_COMP_IDX_CPU: u16 = 0xC7DF;

/// `MO_DoFortressFX`'s "nothing to do" exit (CPU `$C9C9`), vanilla's own
/// already-busted branch target. It zeroes `Map_DoFortressFX` and
/// `Map_ClearLevelFXCnt`, bumps `Map_Operation`, and returns to the map update —
/// exactly the bookkeeping a lock on another map needs, with no animation.
const FX_DONE_CPU: u16 = 0xC9C9;

/// Where `MO_DoFortressFX` resumes after the hook (CPU `$C8EA`).
const FX_RESUME_CPU: u16 = 0xC8EA;

/// Hook site inside `MO_DoFortressFX`, CPU `$C8E6` = file 0x148F6.
///
/// `$C8E3` has just resolved the FX slot into `Map_DoFortressFX` (`$0745`) via
/// `FortressFXBase_ByWorld` + the Boom-Boom ordinal, and nothing has happened
/// yet — no poof, no graphics buffer, no completion write. The four bytes here
/// are two whole instructions:
///
/// ```text
/// A9 01       LDA #$01
/// 85 20       STA Map_ClearLevelFXCnt
/// ```
///
/// This is the same site the shipped `fx_screen_check` patch takes, for the
/// same reason. They are not applied together: that one comes from the
/// randomizer, and this POC builds on a vanilla base.
const FX_HOOK_OFFSET: usize = 0x148F6;
const FX_HOOK_LEN: usize = 4;

/// Vanilla bytes at [`FX_HOOK_OFFSET`].
#[cfg(test)]
#[rustfmt::skip]
const FX_HOOK_VANILLA: [u8; FX_HOOK_LEN] = [
    0xA9, 0x01,             // LDA #$01
    0x85, 0x20,             // STA Map_ClearLevelFXCnt
];

/// Send a fortress's lock-break to a lock in *another* world.
///
/// The design claim this tests: a lock does not have to sit in its fortress's
/// own world. Vanilla already breaks locks it cannot show — a fortress on page
/// 0 opening a lock on page 2 gets no animation, just a map-data and
/// `Map_Completions` update — so a lock on another *map* is that case one level
/// further out.
///
/// It turns out to need no tile writing at all. `Map_Removable_Tiles` (PRG012)
/// lists `TILE_LOCKVERT`, `TILE_LOCKHORZ` and `TILE_ALTLOCK` alongside the
/// rocks, and `Map_Reload_with_Completions` swaps each one for its
/// `Map_RemoveTo_Tiles` counterpart wherever the completion bit is set. That is
/// how vanilla keeps a busted lock busted across a map reload — so **setting
/// the bit in the destination world's parked bank is the whole mechanic**. The
/// lock is simply gone when that world is next loaded.
///
/// Both parked halves get the bit, mirroring what vanilla does for every other
/// permanent alteration (see the module docs).
///
/// `Map_DoFortressFX` (`$0745`) holds the resolved slot by the time this runs,
/// and `$0B` is free scratch — `$C9C9` reads neither it nor `$0A`.
#[rustfmt::skip]
const CROSS_WORLD_FX: [u8; 47] = [
    0xAC, 0x45, 0x07,                                       //  0: LDY $0745    ; FX slot
    0xB9, 0, 0,                                             //  3: LDA SLOT_WORLD,Y   (patched)
    0xCD, WORLD_NUM as u8, (WORLD_NUM >> 8) as u8,          //  6: CMP World_Num
    0xF0, 0x1D,                                             //  9: BEQ +29 → same world

    // ----- the lock lives on the other map: stamp its parked banks -----
    0x98,                                                   // 11: TYA
    0x0A,                                                   // 12: ASL A
    0xA8,                                                   // 13: TAY          ; Y = slot * 2
    0xBE, FX_MAP_COMP_IDX_CPU as u8,
          (FX_MAP_COMP_IDX_CPU >> 8) as u8,                 // 14: LDX $C7DF,Y  ; column
    0xC8,                                                   // 17: INY
    0xB9, FX_MAP_COMP_IDX_CPU as u8,
          (FX_MAP_COMP_IDX_CPU >> 8) as u8,                 // 18: LDA $C7DF,Y  ; row bit
    0x85, 0x0B,                                             // 21: STA $0B      ; keep the bit
    0x1D, PARKED_MARIO as u8, (PARKED_MARIO >> 8) as u8,    // 23: ORA PARKED_MARIO,X
    0x9D, PARKED_MARIO as u8, (PARKED_MARIO >> 8) as u8,    // 26: STA PARKED_MARIO,X
    0xA5, 0x0B,                                             // 29: LDA $0B
    0x1D, PARKED_LUIGI as u8, (PARKED_LUIGI >> 8) as u8,    // 31: ORA PARKED_LUIGI,X
    0x9D, PARKED_LUIGI as u8, (PARKED_LUIGI >> 8) as u8,    // 34: STA PARKED_LUIGI,X
    0x4C, FX_DONE_CPU as u8, (FX_DONE_CPU >> 8) as u8,      // 37: JMP $C9C9    ; no animation here

    // ----- same world: the two displaced instructions, then carry on -----
    0xA9, 0x01,                                             // 40: LDA #$01
    0x85, 0x20,                                             // 42: STA Map_ClearLevelFXCnt
    0x4C, FX_RESUME_CPU as u8, (FX_RESUME_CPU >> 8) as u8,  // 44: JMP $C8EA
];

/// Offset of the `SLOT_WORLD` table operand inside [`CROSS_WORLD_FX`].
const SLOT_WORLD_OPERAND: usize = 4;

/// Point each of World 1's and World 2's fortresses at the other's lock.
///
/// Two byte writes do the aiming. `FortressFXBase_ByWorld` puts W1's row at
/// byte 0 and W2's at byte 4, each world has exactly one fortress, and the
/// Boom-Boom ordinal is 1 — so `FortressFX_W1[0]` and `[4]` are the two entries
/// those fortresses read, holding FX slots 0 and 1 (W1's lock and W2's lock).
/// Swapping them is the whole redirection; [`CROSS_WORLD_FX`] then notices the
/// slot belongs to another world and banks the completion instead of animating.
fn apply_cross_world_locks(rom: &mut Rom) {
    let table = slot_world_table();
    let mut routine = CROSS_WORLD_FX;
    routine[SLOT_WORLD_OPERAND] = SLOT_WORLD_CPU as u8;
    routine[SLOT_WORLD_OPERAND + 1] = (SLOT_WORLD_CPU >> 8) as u8;
    rom.write_range(FS_CROSS_WORLD_FX, &routine);
    rom.write_range(FS_CROSS_WORLD_TABLE, &table);

    let mut hook = [0xEA_u8; FX_HOOK_LEN];
    hook[0] = 0x4C; // JMP — the hook replaces both displaced instructions, and
    hook[1] = CROSS_WORLD_FX_CPU as u8; // the routine replays them on the
    hook[2] = (CROSS_WORLD_FX_CPU >> 8) as u8; // same-world path.
    rom.write_range(FX_HOOK_OFFSET, &hook);

    // Swap the two row entries. Read-then-write rather than hardcoding 0 and 1,
    // so this stays correct if the base ROM ever arrives with them elsewhere.
    let w1_row = rom.read_byte(rom_data::FX_WORLD_TABLE);
    let w2_row = rom.read_byte(rom_data::FX_WORLD_TABLE + 4);
    rom.write_byte(rom_data::FX_WORLD_TABLE, w2_row);
    rom.write_byte(rom_data::FX_WORLD_TABLE + 4, w1_row);
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

    #[test]
    fn cross_world_fx_is_well_formed() {
        let mut routine = CROSS_WORLD_FX;
        routine[SLOT_WORLD_OPERAND] = SLOT_WORLD_CPU as u8;
        routine[SLOT_WORLD_OPERAND + 1] = (SLOT_WORLD_CPU >> 8) as u8;
        asm::check(&routine).allocation(FS_CROSS_WORLD_FX).origin(CROSS_WORLD_FX_CPU).assert_ok();
    }

    /// The FX hook displaces two whole instructions, and the routine replays
    /// them on the same-world path. Losing them would leave
    /// `Map_ClearLevelFXCnt` clear, and the poof would never start.
    #[test]
    fn fx_hook_displaces_whole_instructions_and_replays_them() {
        let Some(rom) = load_vanilla() else { return };
        let vanilla = rom.read_range(FX_HOOK_OFFSET, FX_HOOK_LEN);
        assert_eq!(vanilla, FX_HOOK_VANILLA, "MO_DoFortressFX's hook site has moved");
        assert_eq!(
            CROSS_WORLD_FX[40..44],
            FX_HOOK_VANILLA,
            "the same-world path must replay the instructions the hook overwrote"
        );

        let mut routine = CROSS_WORLD_FX;
        routine[SLOT_WORLD_OPERAND] = SLOT_WORLD_CPU as u8;
        routine[SLOT_WORLD_OPERAND + 1] = (SLOT_WORLD_CPU >> 8) as u8;
        let mut jsr = [0xEA_u8; FX_HOOK_LEN];
        jsr[0] = 0x4C;
        jsr[1] = CROSS_WORLD_FX_CPU as u8;
        jsr[2] = (CROSS_WORLD_FX_CPU >> 8) as u8;
        asm::check(&routine)
            .allocation(FS_CROSS_WORLD_FX)
            .origin(CROSS_WORLD_FX_CPU)
            .hook(&FX_HOOK_VANILLA, 0, &jsr)
            .assert_ok();
    }

    /// One byte per FX slot, and each names the world that slot's lock is on.
    #[test]
    fn slot_world_table_covers_every_fx_slot() {
        let table = slot_world_table();
        assert_eq!(table.len(), 17, "17 FX slots, 17 entries");
        assert!(table.iter().all(|&w| w < 8), "every entry names a real world");
        assert_eq!(table[0], 0, "slot 0 is W1's lock");
        assert_eq!(table[1], 1, "slot 1 is W2's lock");
        assert!(table.len() <= 24, "table must fit its allocation");
    }

    /// The swap really crosses the two worlds over, and it is an involution —
    /// applying it twice would put them back, which is the sort of thing a
    /// read-then-write can get wrong by reading its own output.
    #[test]
    fn fx_rows_are_actually_swapped() {
        let Some(rom) = load_vanilla() else { return };
        let before =
            (rom.read_byte(rom_data::FX_WORLD_TABLE), rom.read_byte(rom_data::FX_WORLD_TABLE + 4));
        assert_ne!(before.0, before.1, "vanilla must differ, or the test proves nothing");

        let mut patched = rom.clone();
        apply(&mut patched, true);
        assert_eq!(patched.read_byte(rom_data::FX_WORLD_TABLE), before.1, "W1 fort -> W2 lock");
        assert_eq!(patched.read_byte(rom_data::FX_WORLD_TABLE + 4), before.0, "W2 fort -> W1 lock");
    }

    /// Without the flag, `MO_DoFortressFX` and the FX rows are untouched.
    #[test]
    fn cross_world_is_opt_in() {
        let Some(rom) = load_vanilla() else { return };
        let mut patched = rom.clone();
        apply(&mut patched, false);
        assert_eq!(
            patched.read_range(FX_HOOK_OFFSET, FX_HOOK_LEN),
            FX_HOOK_VANILLA,
            "the FX hook must not be installed unless asked for"
        );
        assert_eq!(
            patched.read_byte(rom_data::FX_WORLD_TABLE),
            rom.read_byte(rom_data::FX_WORLD_TABLE),
            "the FX rows must not move unless asked for"
        );
    }

    fn load_vanilla() -> Option<Rom> {
        let data = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&data).ok()
    }
}
