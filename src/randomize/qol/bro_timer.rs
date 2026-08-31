//! Bro encounters get a short clock instead of the level's own time setting.
//!
//! # Why this needs a patch at all
//!
//! A level's time comes from bits 6-7 of header byte 8, and those two bits are
//! an *index*, not a value — `GamePlay_TimeStart` (PRG030 $97A8) holds
//! `3, 4, 2, 0`, and the loader stores the entry straight into
//! `Level_TimerMSD`, the leading digit:
//!
//! ```asm
//!   LDA [Level_LayPtr_AddrL],Y
//!   AND #%11000000
//!   CLC
//!   ROL A
//!   ROL A
//!   ROL A
//!   TAX
//!   LDA GamePlay_TimeStart,X
//!   STA Level_TimerMSD        ; <- $98C0, the hook site
//!   BNE PRG030_98C8           ; non-zero -> done
//!   INC Level_TimerEn         ; index 3 stores 0 -> clock disabled entirely
//! ```
//!
//! So the header can express 300, 400, 200 and "unlimited" and nothing else.
//! Anything under 100 has to be written into the digits directly, which is what
//! this routine does. Every vanilla bro arena asks for index 2 — 200.
//!
//! # Which levels count as a bro encounter
//!
//! `Map_EnterViaID` ($1E) holds the map-object id the player walked into, and
//! `3`-`6` are exactly the four bro sprites (`MAPOBJ_HAMMERBRO`,
//! `MAPOBJ_BOOMERANGBRO`, `MAPOBJ_HEAVYBRO`, `MAPOBJ_FIREBRO`). Keying off it
//! rather than off the level is what makes the patch survive randomization: the
//! builder moves bro sprites around and `hb_encounters` rewrites what is inside
//! the arena, but a sprite the player walks into always sets one of those four.
//!
//! It is also the reason W8 is not caught by accident. W8's pointer-table entry
//! `0xC03D` is classified `HammerBro` and is a full 7-7 action level, but W8 has
//! no bro *sprites* at all — its map objects are Tank, Battleship and Airship —
//! so that entry is walked onto as an ordinary tile with `Map_EnterViaID` = 0
//! and keeps its own clock.
//!
//! The engine clears `$0000-$06FF` (`Clear_RAM_thru_ZeroPage`, `LDY #$06`) on
//! every path from a level back to the map, so `Map_EnterViaID` cannot leak a
//! stale bro id into the next ordinary level.
//!
//! # Why only two digits are written
//!
//! That same clear covers `Level_TimerMid`/`Level_TimerLSD` ($05EF/$05F0), and
//! nothing else in the ROM writes them outside the countdown itself
//! (`StatusBar_Fill_Time`) and the end-of-level drain (`DoTimeBonus`). Both are
//! therefore 0 when a level loads, so the routine sets the leading digit to 0
//! and the middle digit to the tens of the target time, and leaves the units
//! digit alone.
//!
//! # Ten means ten
//!
//! A clock unit is a real second, because [`super::level_clock`] always runs and
//! fixes vanilla's divider (41 frames, ~0.68 s) to 60. So the number below is
//! the number of seconds the fight lasts, with no conversion.
//!
//! That patch is unconditional, so the two cannot disagree. If it ever became
//! optional this constant would have to be scaled with it — a bro clock of 10
//! reads as ~6.8 real seconds on an unpatched divider.
//!
//! Both effects vanilla already had still apply, and neither is introduced
//! here: the countdown hangs off a render call the gameplay loop may skip, and
//! it is paced by the main loop rather than the NMI, so a lagging frame costs
//! the clock a tick. A bro arena is one screen with a handful of enemies, which
//! is about the least demanding thing the engine ever draws, so neither
//! meaningfully stretches the fight. See `docs/smb3_rom_reference.md`
//! ("The Level Clock").

use crate::randomize::rom_data::{BRO_TIMER_CPU, FS_BRO_TIMER};
use crate::rom::Rom;

/// Seconds a bro encounter starts with — and, since [`super::level_clock`] makes
/// a clock unit a real second, the number of seconds the fight actually lasts.
///
/// Whole tens only: the routine writes the leading digit as 0 and this value's
/// tens into `Level_TimerMid`, which is what keeps it to two instructions. To
/// retune after a playtest, change this number — nothing else moves.
const BRO_CLOCK: u8 = 10;

/// File offset of `STA Level_TimerMSD` at PRG030 $98C0, inside `LevelLoad`.
///
/// This is on the non-junction side of `LDA Level_JctCtl; BNE PRG030_98C8`, so
/// a pipe or Big [?] junction inside the arena leaves the running clock alone,
/// exactly as vanilla does.
const TIMER_HOOK: usize = 0x3D8D0;

/// `JSR BRO_TIMER_CPU`, over the three bytes of the displaced `STA`.
#[rustfmt::skip]
const TIMER_HOOK_PATCH: [u8; 3] = [
    0x20, BRO_TIMER_CPU as u8, (BRO_TIMER_CPU >> 8) as u8,
];

/// The routine. 25 bytes.
///
/// ```asm
/// +$00  STA Level_TimerMSD    ; displaced from $98C0
/// +$03  TAX                   ; keep the header's time value for the caller
/// +$04  LDA <Map_EnterViaID
/// +$06  SEC
/// +$07  SBC #$03              ; ids 3-6 (the four bros) fold to 0-3...
/// +$09  CMP #$04
/// +$0B  BCS out               ; ...anything else leaves the level alone
/// +$0D  LDX #$00
/// +$0F  STX Level_TimerMSD    ; leading digit 0 (units are already 0)
/// +$12  LDX #tens
/// +$14  STX Level_TimerMid
/// out:
/// +$17  TXA                   ; A = the header value, or a non-zero tens digit
/// +$18  RTS
/// ```
///
/// `TAX`/`TXA` is doing real work rather than shuffling registers. The caller's
/// next instruction is `BNE PRG030_98C8`, which skips `INC Level_TimerEn` — so
/// the routine has to return with Z set exactly when vanilla would have. On the
/// ordinary path `TXA` replays the header value's flags. On the bro path it
/// returns the tens digit, which is non-zero for any clock this option can be
/// set to, and that is what stops a bro arena whose header asked for unlimited
/// time from disabling the clock we just set.
#[rustfmt::skip]
const BRO_TIMER: [u8; 25] = [
    0x8D, 0xEE, 0x05,       // STA Level_TimerMSD
    0xAA,                   // TAX
    0xA5, 0x1E,             // LDA <Map_EnterViaID
    0x38,                   // SEC
    0xE9, 0x03,             // SBC #$03
    0xC9, 0x04,             // CMP #$04
    0xB0, 0x0A,             // BCS out
    0xA2, 0x00,             // LDX #$00
    0x8E, 0xEE, 0x05,       // STX Level_TimerMSD
    0xA2, BRO_CLOCK / 10,   // LDX #tens
    0x8E, 0xEF, 0x05,       // STX Level_TimerMid
    0x8A,                   // out: TXA
    0x60,                   // RTS
];

/// Give Hammer / Boomerang / Heavy / Fire Bro encounters a [`BRO_CLOCK`]-second
/// clock, whatever their level header asks for. Every other level is untouched.
pub fn apply_bro_battle_timer(rom: &mut Rom) {
    rom.write_range(FS_BRO_TIMER, &BRO_TIMER);
    rom.write_range(TIMER_HOOK, &TIMER_HOOK_PATCH);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::randomize::qol::test_support::make_test_rom;

    #[test]
    fn hook_replaces_the_header_store_with_a_call_to_the_routine() {
        let mut rom = make_test_rom();
        rom.write_range(TIMER_HOOK, &[0x8D, 0xEE, 0x05]); // vanilla STA Level_TimerMSD

        apply_bro_battle_timer(&mut rom);

        assert_eq!(rom.read_range(TIMER_HOOK, 3), &[0x20, 0x60, 0x9F]); // JSR $9F60
        // The routine opens with the store the hook displaced, so the ordinary
        // path still lands the header's own time in the leading digit.
        assert_eq!(rom.read_range(FS_BRO_TIMER, 3), &[0x8D, 0xEE, 0x05]);
    }

    /// The clock the option writes is the one [`BRO_CLOCK`] names — a retune
    /// that forgets the `/ 10` would otherwise sail through as a plain constant.
    #[test]
    fn the_middle_digit_is_the_tens_of_the_configured_clock() {
        assert_eq!(BRO_TIMER[19], BRO_CLOCK / 10, "LDX #tens operand");
        assert!(BRO_CLOCK >= 10 && BRO_CLOCK < 100 && BRO_CLOCK.is_multiple_of(10));
    }
}

#[cfg(test)]
mod asm_checks {
    use super::*;
    use crate::randomize::rom_data::asm;

    fn vanilla() -> Option<Vec<u8>> {
        std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()
    }

    #[test]
    fn bro_timer_routine_is_well_formed() {
        let mut check = asm::check(&BRO_TIMER).allocation(FS_BRO_TIMER).origin(BRO_TIMER_CPU);
        let v = vanilla();
        if let Some(v) = &v {
            check = check.hook(v, TIMER_HOOK, &TIMER_HOOK_PATCH);
        } else {
            eprintln!("SKIP hook check: requires the ROM, which is not included in the repo");
        }
        check.assert_ok();
    }

    /// The bytes the hook overwrites really are `STA Level_TimerMSD`, and the
    /// index it feeds really is `GamePlay_TimeStart` — both read out of the ROM,
    /// so a mis-derived offset cannot pass quietly.
    #[test]
    fn hook_site_matches_the_rom() {
        let Some(v) = vanilla() else {
            eprintln!("SKIP: requires the ROM, which is not included in the repo");
            return;
        };
        assert_eq!(&v[TIMER_HOOK..TIMER_HOOK + 3], &[0x8D, 0xEE, 0x05], "STA Level_TimerMSD");
        // Immediately before: LDA GamePlay_TimeStart,X ($97A8).
        assert_eq!(&v[TIMER_HOOK - 3..TIMER_HOOK], &[0xBD, 0xA8, 0x97]);
        // Immediately after: BNE +3; INC Level_TimerEn — the flags the routine
        // has to reproduce on return.
        assert_eq!(&v[TIMER_HOOK + 3..TIMER_HOOK + 8], &[0xD0, 0x03, 0xEE, 0xF3, 0x05]);
        // And the table itself is still 300 / 400 / 200 / unlimited.
        assert_eq!(&v[0x3D7B8..0x3D7BC], &[3, 4, 2, 0], "GamePlay_TimeStart");
    }
}

/// The routine reads one zero-page byte and writes two RAM bytes, with no call
/// out to the engine, so it can be *run* rather than merely decoded — which is
/// the only way to check the flag it returns, and that flag is what decides
/// whether the caller disables the clock we just set.
#[cfg(test)]
mod execution {
    use super::*;
    use mos6502::cpu::CPU;
    use mos6502::instruction::Ricoh2a03;
    use mos6502::memory::{Bus, Memory};
    use mos6502::registers::Status;

    const MAP_ENTER_VIA_ID: u16 = 0x001E;
    const TIMER_MSD: u16 = 0x05EE;
    const TIMER_MID: u16 = 0x05EF;
    const TIMER_LSD: u16 = 0x05F0;

    /// One run with `A` holding what `LDA GamePlay_TimeStart,X` produced.
    /// Returns the three clock digits and the Z flag the caller's `BNE` reads.
    fn run(enter_via: u8, header_time: u8) -> ([u8; 3], bool) {
        let mut mem = Memory::new();
        mem.set_bytes(BRO_TIMER_CPU, &BRO_TIMER);
        mem.set_byte(MAP_ENTER_VIA_ID, enter_via);
        // Poison every digit: the engine really does leave these at 0, but a
        // store the routine forgot has to be visible rather than coincidental.
        mem.set_byte(TIMER_MSD, 0xAA);
        mem.set_byte(TIMER_MID, 0xAA);
        mem.set_byte(TIMER_LSD, 0xAA);

        let mut cpu = CPU::new(mem, Ricoh2a03);
        cpu.registers.program_counter = BRO_TIMER_CPU;
        cpu.registers.accumulator = header_time;

        for _ in 0..BRO_TIMER.len() {
            if cpu.memory.get_byte(cpu.registers.program_counter) == 0x60 {
                let digits = [
                    cpu.memory.get_byte(TIMER_MSD),
                    cpu.memory.get_byte(TIMER_MID),
                    cpu.memory.get_byte(TIMER_LSD),
                ];
                return (digits, cpu.registers.status.contains(Status::PS_ZERO));
            }
            cpu.single_step();
        }
        panic!("routine ran past {} instructions without reaching its RTS", BRO_TIMER.len());
    }

    /// Over every map-object id the engine defines (0-16) and every time the
    /// header table can produce. A bro is 3-6 and nothing else — off-by-one at
    /// either end of `SBC #$03` / `CMP #$04` would grab the airship or the
    /// N-Spade, and both are levels a 10-second clock would ruin.
    #[test]
    fn only_the_four_bro_sprites_get_the_short_clock() {
        for enter_via in 0..=0x10u8 {
            for header_time in [0, 2, 3, 4] {
                let (digits, zero) = run(enter_via, header_time);
                let bro = (3..=6).contains(&enter_via);
                let want = if bro {
                    // 0, tens, and the units digit left as the engine's clear
                    // leaves it — poison here proves the routine never wrote it.
                    ([0, BRO_CLOCK / 10, 0xAA], false)
                } else {
                    ([header_time, 0xAA, 0xAA], header_time == 0)
                };
                assert_eq!((digits, zero), want, "enter_via {enter_via}, time {header_time}");
            }
        }
    }

    /// The flag is not decoration. Vanilla's `BNE PRG030_98C8` falls through to
    /// `INC Level_TimerEn` when the leading digit is 0, and a bro arena whose
    /// header asks for unlimited time stores 0 — so returning Z set there would
    /// disable the very clock the routine just wrote.
    #[test]
    fn an_unlimited_time_header_does_not_disable_the_bro_clock() {
        let (digits, zero) = run(3, 0);
        assert_eq!(digits[0], 0, "leading digit");
        assert!(!zero, "Z must be clear so the caller skips INC Level_TimerEn");
    }
}
