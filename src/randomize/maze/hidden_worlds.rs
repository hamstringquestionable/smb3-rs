//! World-maze **hidden world numbers**: every "WORLD n" the game draws shows
//! `×` until the player beats one marked level, the *world-number obelisk*.
//!
//! # Why
//!
//! In the maze a player can work forward from wherever a telepad drops them,
//! and the world number is what tells them where they are in the run: whether
//! this world's airship is the one that leads into World 8, and which world a
//! Full-hints away-lock digit refers to. Worlds 1-7 are shuffled, so their maps
//! do not give the number away; World 8 is the exception, its map the same
//! every seed. Hiding the number until the obelisk is beaten makes that knowledge
//! something earned.
//!
//! # The three parts
//!
//! - **Which level** ([`pick_obelisk`]): Wandom's rule exactly, through
//!   [`pick_pyramid`] — an optional level, nearest depth 3 first — and never
//!   Wandom's own obelisk, which stays walled off while this one is tested so
//!   the two are optional *together*. Drawn as the same pyramid tile.
//! - **The trigger** ([`trigger`]): one [`on_clear`](crate::randomize::maze::on_clear) row that sets
//!   [`WORLDS_REVEAL`].
//! - **The display** ([`apply`]): the two places the game draws a world
//!   number, both already rewritten by `world_order` to read its display
//!   table, now `JSR` a routine that answers `×` until the flag is set.
//!
//! # Two draw sites cover three screens
//!
//! | Site | Bank | What the player sees |
//! |---|---|---|
//! | `Map_ConfigWorldIntro` | PRG010 | the "WORLD n" banner on entering a world |
//! | `StatusBar_Fill_World` | PRG026 | the map status bar, and the level's |
//!
//! Each gets its own copy of the 14-byte routine in a bank that is mapped when
//! it runs, rather than one copy in the always-mapped PRG030: neither caller
//! needs the always-mapped space, and that bank's last runs are for a hook
//! that does. `$FB` is `×` in both the map's CHR (page `$16`) and the status
//! bar's (page `$5E`), so one glyph serves both.

use rand::Rng;

use crate::randomize::maze::maze_state::WORLDS_REVEAL;
use crate::randomize::maze::on_clear::Trigger;
use crate::randomize::maze::wandom::pick_pyramid;
use crate::randomize::maze::{GlobalState, MazePos};
use crate::randomize::overworld::world_order::{
    DISPLAY_TABLE_CPU, MAP_DISPLAY_OFFSET, STATUS_DISPLAY_OFFSET,
};
use crate::randomize::rom_data::{
    FS_WORLD_DIGIT_BAR, FS_WORLD_DIGIT_MAP, STATUS_GLYPH_TIMES, WORLD_NUM, prg_bank_file_to_cpu,
    prg010_file_to_cpu,
};
use crate::rom::Rom;

/// Salt for the obelisk pick's substream: "WORLDOB1". Its own stream, so
/// turning this on moves nothing else in the seed — Wandom's pick included.
pub(crate) const HIDDEN_WORLDS_SALT: u64 = 0x574F_524C_444F_4231;

/// Pick the world-number obelisk's cell, avoiding (and walling off) `wandom`'s.
/// `None` when no level qualifies; the numbers are then shown from the start.
pub(crate) fn pick_obelisk<R: Rng>(
    state: &GlobalState,
    rng: &mut R,
    wandom: Option<MazePos>,
) -> Option<MazePos> {
    let taken: Vec<MazePos> = wandom.into_iter().collect();
    pick_pyramid(state, rng, &taken)
}

/// The obelisk's on-clear row: beating it sets [`WORLDS_REVEAL`].
pub(crate) fn trigger(obelisk: MazePos) -> Trigger {
    Trigger { cell: obelisk, flag: WORLDS_REVEAL, value: 0x01 }
}

const MAP_DIGIT_CPU: u16 = prg010_file_to_cpu(FS_WORLD_DIGIT_MAP);
const BAR_DIGIT_CPU: u16 = prg_bank_file_to_cpu(26, FS_WORLD_DIGIT_BAR);

/// The digit routine: `A` = the world's display tile, or `×` while hidden.
/// Clobbers `X`; leaves `Y` alone, which the status-bar site indexes its store
/// with.
///
/// Loading `×` *first* and testing the flag with `LDX` is what makes the
/// hidden path free: `LDX` sets Z from the flag without touching `A`, so a
/// zero flag branches straight to `RTS` with `×` already in hand.
#[rustfmt::skip]
fn digit_routine() -> [u8; 14] {
    let [rev_lo, rev_hi] = WORLDS_REVEAL.to_le_bytes();
    let [wn_lo, wn_hi] = WORLD_NUM.to_le_bytes();
    let [tbl_lo, tbl_hi] = DISPLAY_TABLE_CPU.to_le_bytes();
    [
        0xA9, STATUS_GLYPH_TIMES,      //  0: LDA #'×'
        0xAE, rev_lo, rev_hi,          //  2: LDX WORLDS_REVEAL  ; Z from the flag, A kept
        0xF0, 0x06,                    //  5: BEQ done           (-> 13)
        0xAE, wn_lo, wn_hi,            //  7: LDX World_Num
        0xBD, tbl_lo, tbl_hi,          // 10: LDA display,X      ; world_order's digit
        0x60,                          // 13: RTS                <- done
    ]
}

/// A draw site after `world_order` rewrote it: `LDX World_Num / LDA
/// display,X / <store> / NOP`. `store` is the site's own three-byte store.
fn world_order_site(store: [u8; 3]) -> [u8; 10] {
    let [wn_lo, wn_hi] = WORLD_NUM.to_le_bytes();
    let [tbl_lo, tbl_hi] = DISPLAY_TABLE_CPU.to_le_bytes();
    [0xAE, wn_lo, wn_hi, 0xBD, tbl_lo, tbl_hi, store[0], store[1], store[2], 0xEA]
}

/// The same site calling the digit routine: `JSR digit / <store> / NOP x4`.
fn hooked_site(digit: u16, store: [u8; 3]) -> [u8; 10] {
    let [lo, hi] = digit.to_le_bytes();
    [0x20, lo, hi, store[0], store[1], store[2], 0xEA, 0xEA, 0xEA, 0xEA]
}

/// `STA $0304` — the banner's store.
const MAP_STORE: [u8; 3] = [0x8D, 0x04, 0x03];
/// `STA $0304,Y` — the status bar's.
const BAR_STORE: [u8; 3] = [0x99, 0x04, 0x03];

/// Hide the world number until [`WORLDS_REVEAL`] is set: both digit routines,
/// and both draw sites pointed at them.
///
/// **After `world_order`**, whose rewrite of the two sites is what this hooks;
/// the asserts turn the wrong order into a panic rather than a ROM whose
/// numbers silently stay visible.
pub(crate) fn apply(rom: &mut Rom) {
    for (site, store, what) in [
        (MAP_DISPLAY_OFFSET, MAP_STORE, "the world-intro banner"),
        (STATUS_DISPLAY_OFFSET, BAR_STORE, "StatusBar_Fill_World"),
    ] {
        assert_eq!(
            rom.read_range(site, 10),
            world_order_site(store),
            "0x{site:05X} ({what}) does not hold world_order's display lookup: \
             hidden_worlds must run after world_order"
        );
    }
    rom.write_range(FS_WORLD_DIGIT_MAP, &digit_routine());
    rom.write_range(FS_WORLD_DIGIT_BAR, &digit_routine());
    rom.write_range(MAP_DISPLAY_OFFSET, &hooked_site(MAP_DIGIT_CPU, MAP_STORE));
    rom.write_range(STATUS_DISPLAY_OFFSET, &hooked_site(BAR_DIGIT_CPU, BAR_STORE));
}

/// Point the obelisk's row at another cell: `testrom --world-obelisk-at`.
/// `false`, and nothing written, when the ROM has no such row.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn retarget(rom: &mut Rom, cell: MazePos) -> bool {
    crate::randomize::maze::on_clear::retarget(rom, WORLDS_REVEAL, cell)
}

#[cfg(test)]
mod asm_checks {
    use super::*;
    use crate::randomize::rom_data::{NMI_SAFE_MAX, asm};
    use mos6502::cpu::CPU;
    use mos6502::instruction::Ricoh2a03;
    use mos6502::memory::{Bus, Memory};

    #[test]
    fn digit_routines_are_well_formed() {
        for (fs, cpu) in [(FS_WORLD_DIGIT_MAP, MAP_DIGIT_CPU), (FS_WORLD_DIGIT_BAR, BAR_DIGIT_CPU)]
        {
            asm::check(&digit_routine())
                .allocation(fs)
                .origin(cpu)
                .zero_page(NMI_SAFE_MAX, &[])
                .assert_ok();
        }
    }

    #[test]
    fn hooked_sites_are_well_formed() {
        for (digit, store) in [(MAP_DIGIT_CPU, MAP_STORE), (BAR_DIGIT_CPU, BAR_STORE)] {
            asm::check(&hooked_site(digit, store)).fragment().assert_ok();
        }
    }

    /// Run the routine with `World_Num` = `world` and the flag = `reveal`;
    /// return `(A, Y)` at the `RTS`.
    fn run(world: u8, reveal: u8) -> (u8, u8) {
        let mut mem = Memory::new();
        mem.set_bytes(BAR_DIGIT_CPU, &digit_routine());
        let table: Vec<u8> = (0..8).map(|w| 0xF1 + (w * 3) % 8).collect();
        mem.set_bytes(DISPLAY_TABLE_CPU, &table);
        mem.set_byte(WORLD_NUM, world);
        mem.set_byte(WORLDS_REVEAL, reveal);
        let mut cpu = CPU::new(mem, Ricoh2a03);
        cpu.registers.program_counter = BAR_DIGIT_CPU;
        cpu.registers.index_y = 0x5A;
        for _ in 0..20 {
            if cpu.memory.get_byte(cpu.registers.program_counter) == 0x60 {
                return (cpu.registers.accumulator, cpu.registers.index_y);
            }
            cpu.single_step();
        }
        panic!("the digit routine never reached RTS");
    }

    /// Hidden: `×` for every world. Revealed: the world's own display digit,
    /// whatever non-zero value the flag holds. `Y` survives both ways.
    #[test]
    fn hidden_is_times_and_revealed_is_the_display_digit() {
        for w in 0..8u8 {
            assert_eq!(run(w, 0), (STATUS_GLYPH_TIMES, 0x5A), "world {w} hidden");
            let want = 0xF1 + (w * 3) % 8;
            for reveal in [0x01, 0x80, 0xFF] {
                assert_eq!(run(w, reveal), (want, 0x5A), "world {w} revealed by {reveal:#04X}");
            }
        }
    }

    fn vanilla() -> Option<Rom> {
        let bytes = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&bytes).ok()
    }

    /// `apply` refuses a ROM `world_order` has not rewritten.
    #[test]
    #[should_panic(expected = "must run after world_order")]
    fn apply_before_world_order_panics() {
        // Minimal valid iNES image, zeroed: neither site holds the lookup.
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;
        let mut rom = Rom::from_bytes_lax(&data, true).unwrap();
        apply(&mut rom);
    }

    /// **The feature on finished ROMs.** Both sites call their digit routine;
    /// the obelisk row names a real pyramid tile, apart from Wandom's; and the
    /// obelisk is optional alongside Wandom's.
    #[test]
    fn hidden_worlds_on_finished_roms() {
        use crate::randomize::maze::LAST_MAZE;
        use crate::randomize::maze::maze_state::WANDOM_REVEAL;
        use crate::randomize::maze::on_clear::installed;
        use crate::randomize::maze::wandom::TILE_PYRAMID;
        use crate::randomize::rom_data::map_tile_offset;
        use std::collections::HashSet;
        let Some(base) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        for (wandom, seed) in [(false, 1u64), (false, 2), (true, 3), (true, 4)] {
            let options = crate::Options {
                world_maze: true,
                hidden_worlds: true,
                maze_wands: if wandom { crate::MAZE_WANDS_WANDOM } else { 3 },
                palettes: false,
                ..Default::default()
            };
            let mut rom = base.clone();
            crate::pipeline::randomize(&mut rom, seed, &options);
            let state = LAST_MAZE.with(|s| s.borrow().clone()).expect("the maze ran");

            assert_eq!(
                rom.read_range(MAP_DISPLAY_OFFSET, 10),
                hooked_site(MAP_DIGIT_CPU, MAP_STORE)
            );
            assert_eq!(
                rom.read_range(STATUS_DISPLAY_OFFSET, 10),
                hooked_site(BAR_DIGIT_CPU, BAR_STORE)
            );
            let rows = installed(&rom).expect("a hidden-worlds seed has a trigger table");
            let row = rows.iter().find(|t| t.flag == WORLDS_REVEAL).expect("an obelisk row");
            assert_eq!(row.value, 0x01);
            let (w, (r, c)) = row.cell;
            assert_eq!(rom.read_byte(map_tile_offset(w, r, c)), TILE_PYRAMID, "seed {seed}");

            let mut obelisks: HashSet<MazePos> = HashSet::from([row.cell]);
            let pyramid = rows.iter().find(|t| t.flag == WANDOM_REVEAL);
            assert_eq!(pyramid.is_some(), wandom, "seed {seed}: Wandom row presence");
            if let Some(p) = pyramid {
                assert_ne!(p.cell, row.cell, "seed {seed}: both obelisks on one cell");
                obelisks.insert(p.cell);
            }
            assert!(
                state.spheres_with_blocked(&obelisks).solvable,
                "seed {seed}: the obelisks are not optional together"
            );
        }
    }

    /// Off, both sites keep `world_order`'s lookup.
    #[test]
    fn off_leaves_the_sites_alone() {
        let Some(mut rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let options = crate::Options { world_maze: true, palettes: false, ..Default::default() };
        crate::pipeline::randomize(&mut rom, 1, &options);
        assert_eq!(rom.read_range(MAP_DISPLAY_OFFSET, 10), world_order_site(MAP_STORE));
        assert_eq!(rom.read_range(STATUS_DISPLAY_OFFSET, 10), world_order_site(BAR_STORE));
    }
}
