//! The level clock counts real seconds.
//!
//! # What vanilla does
//!
//! `StatusBar_Fill_Time` (PRG026 `$AF9D`) drives the countdown with a frame
//! divider:
//!
//! ```asm
//!   DEC Level_TimerTick
//!   BPL Timer_NoChange     ; still inside this unit
//!   LDA #$28
//!   STA Level_TimerTick    ; reload and step a digit
//! ```
//!
//! # This is a design choice, not a bug
//!
//! `$28` is **40** — and 40 frames is exactly 2/3 of a second at 60.0988 Hz.
//! That is a round decimal number deliberately chosen, not a hardware constant:
//! the European (PAL, 50.007 Hz) build stores the same `$28`, so it cannot have
//! been derived from a refresh rate, and 41/50.007 = 0.82 s is no closer to a
//! second than NTSC's 0.68. A clock running at 1.5x real time is a plausible
//! way to get a big round "300" onto the status bar for a level that lasts
//! about three minutes — though the round 40 and the matching PAL value are the
//! evidence here, not any statement from Nintendo.
//!
//! What *is* unintentional is the extra frame. `DEC`/`BPL` tests after
//! decrementing, so the divider spends one more frame at `$FF` before
//! reloading: **41** frames per unit, not 40. A 2.4% slip on top of the
//! deliberate 50% one.
//!
//! # What this patch changes, and why it is unconditional
//!
//! It replaces the design choice, not just the slip. Writing `$3B` (59) to both
//! reload immediates makes the span 60 frames — 0.99835 s, within 0.2% of a
//! second — so the displayed time means what it says. Two bytes, no free space,
//! no new routine.
//!
//! The point is not the extra time. It is that **a clock unit becomes a unit
//! anything can state.** A randomizer feature that wants to give the player ten
//! seconds can now write `10` and be done; before this it had to write `15` and
//! carry a comment explaining the 0.68 conversion, and any later retune had to
//! redo that arithmetic. [`super::bro_timer`] is the first consumer and the
//! reason this landed, but the property is general — timed events, countdowns,
//! anything measured in seconds gets to mean seconds.
//!
//! That is also why it is not an option. A conditional clock would make every
//! such constant mean two different things depending on a flag, which is the
//! bug this is meant to remove.
//!
//! The side effect is that every level is more generous in real terms, since
//! the header times (200/300/400) were picked against a 1.5x clock — vanilla's
//! 300 was ~3:25 of real time and is now 5:00. In practice this is close to
//! inert: running the clock out is already a rare way to lose a randomizer
//! seed, so the extra headroom seldom decides anything. The header table
//! (`GamePlay_TimeStart`, PRG030 `$97A8`, file `0x3D7B8`) is three data bytes
//! if that ever needs revisiting, though it can only express multiples of 100 —
//! each entry is stored straight into the hundreds digit.
//!
//! Measured on FCEUX (vanilla PRG1, 1-1, frame-counted between digit steps):
//!
//! | | vanilla | patched |
//! |---|---|---|
//! | screen scrolling | 41.00 frames (min 41, max 41) | 60.00 (min 60, max 60) |
//! | standing still | 46.80 (min 41, max 70) | 69.67 (min 60, max 89) |
//!
//! # Why the digit still isn't a perfect second
//!
//! The countdown hangs off a *rendering* call, and the gameplay loop is allowed
//! to skip that call when the video pipeline is busy (`Scroll_ToVRAMHi`/`HA`,
//! `Graphics_Queue`, `Level_SkipStatusBarUpd`, `InvFlip_Counter` — PRG030
//! `$8F5A`). A skipped call is a frame the clock never sees.
//!
//! The same coupling means **lag stretches a second**: the loop is paced by
//! `GraphicsBuf_Prep_And_WaitVSync`, so a frame that overruns VBlank costs the
//! clock a tick. Nothing writes `Level_TimerTick` from the NMI — the only writes
//! in the ROM are this reload, the PRG030 `$8506` init, and the `DEC` above — so
//! the clock counts *game-loop iterations*, not elapsed frames.
//!
//! Both effects existed in vanilla and are unchanged here; this patch corrects
//! the divider, not the coupling. Fixing the coupling would mean moving the
//! countdown onto `Counter_1` (a true per-frame counter) and accumulating
//! elapsed frames, which needs free space and a spare RAM byte. See
//! `docs/smb3_rom_reference.md` ("The Level Clock") for the measurements.

use crate::rom::Rom;

/// Frames a clock unit should span. `DEC`/`BPL` from `N` spans `N + 1` frames,
/// so the stored reload is one less.
const FRAMES_PER_SECOND: u8 = 60;

/// The value written to both reload sites.
const TICK_RELOAD: u8 = FRAMES_PER_SECOND - 1;

/// Vanilla's reload: 40, giving 41 frames per unit. Only the tests need it — the patch
/// overwrites the site unconditionally rather than checking first, so that a
/// re-run or a differently-patched base still lands on the right value.
#[cfg(test)]
const VANILLA_RELOAD: u8 = 0x28;

/// Operand of `LDA #$28` in `StatusBar_Fill_Time`, PRG026 `$AFB5`. This is the
/// reload that runs every time a digit steps.
const RELOAD_IN_COUNTDOWN: usize = 0x34FC6;

/// Operand of `LDA #$28` in the world/level init, PRG030 `$8506`. Primes the
/// divider so the first unit of a level is the same length as the rest.
const RELOAD_AT_INIT: usize = 0x3C517;

/// Both reload sites, for the test that checks them against the ROM.
const RELOAD_SITES: [usize; 2] = [RELOAD_IN_COUNTDOWN, RELOAD_AT_INIT];

/// Make one unit on the level clock last one real second instead of 0.68.
///
/// Always applied, and deliberately a change of intent rather than a bugfix:
/// vanilla's 40-frame divider is a chosen 1.5x clock, not a mistake. Running it
/// unconditionally is what lets the rest of the randomizer state times in plain
/// seconds — `bro_timer`'s constant means what it says only because this runs,
/// and any future timed feature inherits the same guarantee. Every level is
/// more generous than Nintendo shipped as a result (a 300 level goes from ~3:25
/// to 5:00), which matters little in practice: timing out is already a rare way
/// to lose a seed.
pub fn apply_real_time_clock(rom: &mut Rom) {
    for site in RELOAD_SITES {
        rom.write_byte(site, TICK_RELOAD);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::randomize::qol::test_support::make_test_rom;

    #[test]
    fn both_reload_sites_are_rewritten() {
        let mut rom = make_test_rom();
        for site in RELOAD_SITES {
            rom.write_byte(site, VANILLA_RELOAD);
        }

        apply_real_time_clock(&mut rom);

        for site in RELOAD_SITES {
            assert_eq!(rom.read_byte(site), TICK_RELOAD, "site {site:#07X}");
        }
    }

    /// `DEC`/`BPL` spans one more frame than the reload holds, so the stored
    /// byte is 59 for a 60-frame second. Off by one here is a 1.7% drift.
    #[test]
    fn the_reload_is_one_less_than_the_frame_count() {
        assert_eq!(TICK_RELOAD, 59);
        assert_eq!(u16::from(TICK_RELOAD) + 1, u16::from(FRAMES_PER_SECOND));
    }
}

#[cfg(test)]
mod asm_checks {
    use super::*;

    fn vanilla() -> Option<Vec<u8>> {
        std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()
    }

    /// Both offsets must be the *operand* of an `LDA #$28`, not some other byte
    /// that happens to be `$28`. Read straight out of the ROM, so a mis-derived
    /// offset cannot pass quietly — and a one-byte slip here would rewrite an
    /// opcode instead of a constant.
    #[test]
    fn reload_sites_are_lda_immediate_in_the_rom() {
        let Some(v) = vanilla() else {
            eprintln!("SKIP: requires the ROM, which is not included in the repo");
            return;
        };
        for site in RELOAD_SITES {
            assert_eq!(v[site - 1], 0xA9, "opcode at {site:#07X} is not LDA #imm");
            assert_eq!(v[site], VANILLA_RELOAD, "operand at {site:#07X} is not $28");
            // Followed by STA Level_TimerTick ($05F1) — proves it is the clock
            // divider and not an unrelated `LDA #$28`.
            assert_eq!(&v[site + 1..site + 4], &[0x8D, 0xF1, 0x05], "not STA $05F1");
        }
    }

    /// Nothing else in the ROM reloads the divider, so these two sites are the
    /// whole change. A third site appearing would mean a unit could still be 41
    /// frames on some path.
    #[test]
    fn there_are_no_other_writes_to_the_divider() {
        let Some(v) = vanilla() else {
            eprintln!("SKIP: requires the ROM, which is not included in the repo");
            return;
        };
        let prg = &v[16..16 + 0x40000];
        let stores: Vec<usize> = prg
            .windows(3)
            .enumerate()
            // STY / STA / STX absolute, in opcode order
            .filter(|(_, w)| matches!(w[0], 0x8C..=0x8E) && w[1] == 0xF1 && w[2] == 0x05)
            .map(|(i, _)| i + 16)
            .collect();
        assert_eq!(stores, vec![RELOAD_IN_COUNTDOWN + 1, RELOAD_AT_INIT + 1]);
    }
}
