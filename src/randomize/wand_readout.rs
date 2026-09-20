//! World-maze: how many wands the player holds, on the map's status bar.
//!
//! # The question it answers
//!
//! The wand gate ([`super::wand_gate`]) is a wall on World 8's bridge that
//! stands until the player holds K of the seven wands. Nothing in the game says
//! how many they have. The failure that produces is specific and expensive:
//! the player pad-hops to World 8, finds the wall, and has to go back out
//! without knowing how much further there is to go.
//!
//! So the readout is the *count*, not "is the gate open" — `3/7` rather than a
//! recoloured icon. The boolean is only useful once; the count changes the next
//! decision.
//!
//! # Why the status bar, and not the item box
//!
//! The item box was the first candidate and it is the wrong surface. Its icons
//! are 2x2 background metatiles drawn from one shared palette — there is no
//! per-item attribute table anywhere in the inventory path, so a recoloured
//! whistle is a recoloured *status bar*. Nor is there room for new art: every
//! 2x2 in `$00`-`$7F` of the bar's pattern page that the item and card tables
//! do not claim already holds something, and that page is shared with the
//! in-level status bar, so anything overwritten shows up in every level.
//!
//! The status bar has none of those problems. It is on screen the whole time
//! the player is on the map, no menu required, and the glyphs for a readout are
//! already in the page: the digits at `$F0`-`$F9`, and — the find that makes
//! this cheap — a **slash at `$FA`**. `N/K` costs no CHR at all.
//!
//! # Own the buffer, not the template
//!
//! The bar is painted from two copies of the same template: PRG026's item-box
//! flip set, and a `StatusBar` macro in PRG030 that the map's entry path runs
//! through `Video_Do_Update`. Patching either is pointless. Immediately after
//! that draw, the map init banks in PRG026 and calls `StatusBar_UpdateValues`,
//! whose fills overwrite exactly the cells worth having:
//!
//! * `StatusBar_Fill_PowerMT` has no world-map branch at all and always writes
//!   six arrow tiles from `Player_Power`.
//! * `StatusBar_Fill_Time` takes one (`Level_Tileset == 0`, commented "no timer
//!   on map EVER") — but it skips only the *countdown*. `Timer_NoChange` still
//!   writes three digits into `StatusBar_Time` from `Level_TimerMSD`.
//!
//! So the map's clock digits are live output, not leftover template data. The
//! lever is the RAM buffer those fills write through: [`STATUS_BAR_TIME`], the
//! three tiles the bar commits to VRAM `$2B51`-`$2B53`. Write it and the map's
//! own status-bar update carries it to the screen.
//!
//! # The item box flip does not carry it, and that needs a second hook
//!
//! **This module shipped claiming the flip repainted the readout for free. It
//! does not, and the claim is corrected here rather than quietly dropped.**
//! `InvFlipFrame_DrawMLLivesScore` patches exactly two things into the copied
//! template — lives at buffer `+8`/`+9`, and the six score digits at `+11`
//! through `+16`. It never touches `+$14`-`+$16`, the timer cells. So closing
//! the item box repaints that row from `Flip_MidBStatCards`' static
//! `$ED $F0 $F0 $F0` and the readout reverts to a clock and "000".
//!
//! It comes back, but not promptly: the map loop's `StatusBar_UpdateValues` at
//! `$8729` is gated on `Map_Operation >= 2`, so it runs when the map is *doing*
//! something rather than every frame. The player sees a flash of "000" that
//! heals on their next move — which is exactly how it was reported, and why it
//! was hard to reproduce on purpose.
//!
//! [`FLIP_CARRY`] closes it by copying [`STATUS_BAR_TIME`] into the flip's own
//! buffer, twenty bytes hooked over the `JSR StatusBar_Fill_Score` inside that
//! routine.
//!
//! **That hook site is chosen for its gate, not its convenience.** Vanilla
//! skips the score fill while the box is *opening*
//! (`LDA InvFlip_Frame / AND #$08 / BNE rts`), and the call sits after that
//! test, so the patch inherits the gate. It has to: while the box is opening,
//! `$2B52`/`$2B53` are the bottom half of item slot 4, and writing the readout
//! there would shred an item icon. The two writes do not collide either —
//! vanilla's score lands at `+11`-`+16`, ours at `+20`-`+22`.
//!
//! The `400` that flashes on *map entry* is a different bug with the same
//! shape, and is deliberately left alone: it comes from PRG030's macro copy,
//! which is shared with the level status bar.
//!
//! # The hook
//!
//! `StatusBar_Fill_Time` has **exactly one caller** in the whole ROM, the last
//! of the five fills in `StatusBar_UpdateValues`. That makes a three-for-three
//! `JSR` swap total: there is no second path into the routine that could
//! bypass us. [`super::rom_data`]'s `map_status_bar_offsets_match_real_rom`
//! asserts the single-caller property rather than trusting it.
//!
//! The replacement branches on `Level_Tileset` itself and tail-jumps to the
//! vanilla routine in a level, so nothing about in-level timing changes.
//!
//! **Hooking the routine's own `BEQ` was the obvious alternative and does not
//! work.** `Timer_NoChange` is reached from three tests, not one — the map, a
//! tileset of 15 or more, and a timer-disabled level — so intercepting the
//! label would clobber the real timer in two in-level cases.
//!
//! # Ordering
//!
//! After [`super::wand_gate::apply`]. Both read the same K and the same
//! [`WANDS_TABLE`], and `wand_gate` is what installs the marker that fills that
//! table in the first place; a readout without it would count zero forever.
//!
//! K = 0 writes nothing, because there is no gate to report progress towards.

use crate::rom::Rom;

use super::maze_state::WANDS_TABLE;
use super::rom_data::{
    FLIP_FILL_SCORE_CALL, FS_WAND_READOUT, FS_WAND_READOUT_FLIP, GRAPHICS_BUFFER, LEVEL_TILESET,
    PRG026_FILE_BASE, STATUS_BAR_FILL_SCORE_CPU, STATUS_BAR_FILL_TIME_CALL,
    STATUS_BAR_FILL_TIME_CPU, STATUS_BAR_TIME, STATUS_GLYPH_DIGIT0, STATUS_GLYPH_SLASH, TEMP_VAR9,
};
use super::wand_gate::MAX_WANDS;

/// Where [`wand_readout_routine`]'s output is assembled to run. `$B520`.
const WAND_READOUT_CPU: u16 = (0xA000 + (FS_WAND_READOUT - PRG026_FILE_BASE)) as u16;

/// Where [`FLIP_CARRY`] is assembled to run. `$B6ED`.
const FLIP_CARRY_CPU: u16 = (0xA000 + (FS_WAND_READOUT_FLIP - PRG026_FILE_BASE)) as u16;

/// Buffer index of the first timer cell within the row the flip is drawing:
/// payload index 16 (`$2B50`) plus the three header bytes, plus one to reach
/// `$2B51`, the first cell the readout owns.
const FLIP_TIMER_BUFFER_INDEX: u16 = 0x14;

/// The routine that replaces the one `JSR StatusBar_Fill_Time`.
///
/// In a level it is a three-byte detour and nothing else. On the map it fills
/// [`STATUS_BAR_TIME`] with `N`, `/`, `K` and returns without running the
/// vanilla fill at all — which is the whole point, since what the vanilla fill
/// would put there is a stale level timer.
///
/// The sum cannot carry: [`WANDS_TABLE`] holds eight bytes that are 0 or 1 and
/// World 8 never sets its own, so the total is at most seven. That is what lets
/// `ORA #$F0` stand in for the usual add — a digit tile is `$F0 + n` and `n`
/// fits in the low nibble with room to spare.
fn wand_readout_routine(k: u8) -> [u8; 35] {
    let [tileset_lo, tileset_hi] = LEVEL_TILESET.to_le_bytes();
    let [wands_lo, wands_hi] = WANDS_TABLE.to_le_bytes();
    // Each store's address is derived from the full u16, not by bumping the low
    // byte: `StatusBar_Time` sits at $7F50 today, but a low-byte increment would
    // go quietly wrong if it ever moved across a page boundary.
    let [time0_lo, time0_hi] = STATUS_BAR_TIME.to_le_bytes();
    let [time1_lo, time1_hi] = (STATUS_BAR_TIME + 1).to_le_bytes();
    let [time2_lo, time2_hi] = (STATUS_BAR_TIME + 2).to_le_bytes();
    let [fill_lo, fill_hi] = STATUS_BAR_FILL_TIME_CPU.to_le_bytes();
    #[rustfmt::skip]
    let code = [
        0xAD, tileset_lo, tileset_hi,            //  0: LDA Level_Tileset
        0xD0, 0x1B,                              //  3: BNE vanilla   (-> 32)
        0xA0, 0x07,                              //  5: LDY #$07
        0xA9, 0x00,                              //  7: LDA #$00
        0x18,                                    //  9: CLC    (sum <= 7, stays clear)
        0x79, wands_lo, wands_hi,                // 10: ADC WANDS_TABLE,Y   <- loop
        0x88,                                    // 13: DEY
        0x10, 0xFA,                              // 14: BPL loop     (-> 10)
        0x09, STATUS_GLYPH_DIGIT0,               // 16: ORA #$F0     (n -> digit tile)
        0x8D, time0_lo, time0_hi,                // 18: STA StatusBar_Time+0  ($2B51)
        0xA9, STATUS_GLYPH_SLASH,                // 21: LDA #$FA
        0x8D, time1_lo, time1_hi,                // 23: STA StatusBar_Time+1  ($2B52)
        0xA9, STATUS_GLYPH_DIGIT0 + k,           // 26: LDA #digit K
        0x8D, time2_lo, time2_hi,                // 28: STA StatusBar_Time+2  ($2B53)
        0x60,                                    // 31: RTS
        0x4C, fill_lo, fill_hi,                  // 32: JMP StatusBar_Fill_Time  <- vanilla
    ];
    code
}

/// Carry [`STATUS_BAR_TIME`] through the item-box flip.
///
/// Replaces the flip's `JSR StatusBar_Fill_Score`, replays it, and then copies
/// the three readout tiles into the flip's own buffer — which vanilla leaves
/// holding the template's static clock and "000". See the module docs for why
/// this call site and no other: it is the one inside the opening-frame gate.
///
/// A loop rather than three unrolled stores: 20 bytes against 24, and the four
/// saved belong to whatever needs PRG026 next.
fn flip_carry_routine() -> [u8; 20] {
    let [score_lo, score_hi] = STATUS_BAR_FILL_SCORE_CPU.to_le_bytes();
    let [time_lo, time_hi] = STATUS_BAR_TIME.to_le_bytes();
    let [buf_lo, buf_hi] = (GRAPHICS_BUFFER + FLIP_TIMER_BUFFER_INDEX).to_le_bytes();
    #[rustfmt::skip]
    let code = [
        0x20, score_lo, score_hi,   //  0: JSR StatusBar_Fill_Score   (displaced)
        0xA6, TEMP_VAR9,            //  3: LDX <Temp_Var9   (this row's buffer base)
        0xA0, 0x00,                 //  5: LDY #$00
        0xB9, time_lo, time_hi,     //  7: LDA StatusBar_Time,Y   <- loop
        0x9D, buf_lo, buf_hi,       // 10: STA Graphics_Buffer+$14,X
        0xE8,                       // 13: INX
        0xC8,                       // 14: INY
        0xC0, 0x03,                 // 15: CPY #$03
        0xD0, 0xF4,                 // 17: BNE loop     (-> 7)
        0x60,                       // 19: RTS
    ];
    code
}

/// Install the readout. K = 0 writes nothing.
///
/// See the module docs for the ordering rule: this must run after
/// [`super::wand_gate::apply`], which installs the marker that fills
/// [`WANDS_TABLE`].
pub(crate) fn apply(rom: &mut Rom, wands_required: u8) {
    if wands_required == 0 {
        return;
    }
    let k = wands_required.min(MAX_WANDS);
    rom.write_range(FS_WAND_READOUT, &wand_readout_routine(k));
    rom.write_range(
        STATUS_BAR_FILL_TIME_CALL,
        &[0x20, WAND_READOUT_CPU as u8, (WAND_READOUT_CPU >> 8) as u8],
    );
    rom.write_range(FS_WAND_READOUT_FLIP, &flip_carry_routine());
    rom.write_range(
        FLIP_FILL_SCORE_CALL,
        &[0x20, FLIP_CARRY_CPU as u8, (FLIP_CARRY_CPU >> 8) as u8],
    );
}

#[cfg(test)]
mod asm_checks {
    use super::*;
    use crate::randomize::rom_data::asm;

    fn vanilla() -> Option<Rom> {
        let bytes = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&bytes).ok()
    }

    #[test]
    fn wand_readout_is_well_formed() {
        let Some(rom) = vanilla() else { return };
        for k in 1..=MAX_WANDS {
            let code = wand_readout_routine(k);
            asm::check(&code)
                .allocation(FS_WAND_READOUT)
                .origin(WAND_READOUT_CPU)
                .hook(
                    &rom.data,
                    STATUS_BAR_FILL_TIME_CALL,
                    &[0x20, WAND_READOUT_CPU as u8, (WAND_READOUT_CPU >> 8) as u8],
                )
                .assert_ok();
        }
    }

    /// The tail must be the call we displaced, aimed at the real routine —
    /// otherwise a level silently loses its timer.
    #[test]
    fn the_displaced_call_is_replayed() {
        let Some(rom) = vanilla() else { return };
        assert_eq!(
            rom.read_range(STATUS_BAR_FILL_TIME_CALL, 3),
            &[0x20, STATUS_BAR_FILL_TIME_CPU as u8, (STATUS_BAR_FILL_TIME_CPU >> 8) as u8],
            "the JSR StatusBar_Fill_Time hook site has moved"
        );
        let code = wand_readout_routine(3);
        assert_eq!(
            &code[32..35],
            &[0x4C, STATUS_BAR_FILL_TIME_CPU as u8, (STATUS_BAR_FILL_TIME_CPU >> 8) as u8],
            "the level path no longer reaches the vanilla fill"
        );
    }

    /// K = 0 is the pure maze: no gate, so no progress to report, and both
    /// hooks must be left alone entirely.
    #[test]
    fn k_zero_writes_nothing() {
        let Some(mut rom) = vanilla() else { return };
        let time_call = rom.read_range(STATUS_BAR_FILL_TIME_CALL, 3).to_vec();
        let score_call = rom.read_range(FLIP_FILL_SCORE_CALL, 3).to_vec();
        apply(&mut rom, 0);
        assert_eq!(rom.read_range(STATUS_BAR_FILL_TIME_CALL, 3), &time_call[..]);
        assert_eq!(rom.read_range(FLIP_FILL_SCORE_CALL, 3), &score_call[..]);
    }

    #[test]
    fn flip_carry_is_well_formed() {
        let Some(rom) = vanilla() else { return };
        asm::check(&flip_carry_routine())
            .allocation(FS_WAND_READOUT_FLIP)
            .origin(FLIP_CARRY_CPU)
            .hook(
                &rom.data,
                FLIP_FILL_SCORE_CALL,
                &[0x20, FLIP_CARRY_CPU as u8, (FLIP_CARRY_CPU >> 8) as u8],
            )
            .assert_ok();
    }

    /// The hook must land *inside* vanilla's opening-frame gate, because on the
    /// opening frames those buffer cells are item slot 4's lower half. The gate
    /// is `LDA InvFlip_Frame / AND #$08 / BNE rts` and the call we displace has
    /// to sit after it — so the three bytes before our site are the branch that
    /// skips us.
    #[test]
    fn the_flip_hook_sits_inside_the_opening_gate() {
        let Some(rom) = vanilla() else { return };
        assert_eq!(
            rom.read_range(FLIP_FILL_SCORE_CALL, 3),
            &[0x20, STATUS_BAR_FILL_SCORE_CPU as u8, (STATUS_BAR_FILL_SCORE_CPU >> 8) as u8],
            "the flip's JSR StatusBar_Fill_Score has moved"
        );
        // `AND #$08` then a `BNE` forward over the call: the opening-frame skip.
        assert_eq!(
            rom.read_range(FLIP_FILL_SCORE_CALL - 4, 4),
            &[0x29, 0x08, 0xD0, 0x13],
            "the opening-frame gate no longer guards this call"
        );
    }

    /// Ours and vanilla's writes share a row and must not overlap: vanilla
    /// patches score into buffer +11..+16, we write +20..+22.
    #[test]
    fn the_flip_writes_miss_vanillas_score_digits() {
        const SCORE_FIRST: u16 = 11;
        const SCORE_LAST: u16 = 16;
        let ours = FLIP_TIMER_BUFFER_INDEX;
        assert!(
            ours > SCORE_LAST,
            "timer cells at +{ours} collide with score digits +{SCORE_FIRST}..+{SCORE_LAST}"
        );
    }

    /// A K above the seven airships would print a total the player can never
    /// reach, so [`apply`] clamps it the way the gate does. Driven through
    /// `apply` and read back out of the ROM: clamping in the test and then
    /// asserting it would only be testing `u8::min`.
    #[test]
    fn k_clamps_to_the_airships_that_exist() {
        let Some(mut rom) = vanilla() else { return };
        apply(&mut rom, 200);
        assert_eq!(
            rom.read_byte(FS_WAND_READOUT + 27),
            STATUS_GLYPH_DIGIT0 + MAX_WANDS,
            "K digit must clamp to 7"
        );
    }
}
