//! **On-clear triggers**: "beat the level on this map cell, set this flag".
//!
//! Every return to the map alive passes map operation 8 with the player
//! standing on what they just cleared.
//! [`install_on_clear`](crate::randomize::overworld::lock_keys::install_on_clear)
//! sends op 8's no-orb exit to [`FS_ON_CLEAR`] with that cell already built
//! into `T1`/`T2` in [`cell_key`]'s encoding. Keying on the cell rather than
//! the level is what makes Deja Vu, chest endings and secret exits nobody's
//! problem: whatever was beaten, the player is standing on it.
//!
//! # The shape: a table, not a routine per feature
//!
//! A feature that wants a clear noticed hands [`apply`] a [`Trigger`] — a cell,
//! a flag address and a value — and the table gets one 4-byte row:
//!
//! | byte | |
//! |---|---|
//! | 0-1 | the cell's `key0`, `key1` |
//! | 2 | the flag's low byte; the high byte is fixed at `$7A`, the page all of [`maze_state`](crate::randomize::maze::maze_state) lives in |
//! | 3 | the value **`ORA`'d** into the flag |
//!
//! `ORA` rather than `STA` so rows can share a byte — a bit each — and so a
//! flag that starts at zero ends up holding exactly the value, which is what
//! lets Wandom store its digit tile through the same row shape. Every row is
//! scanned, not just the first match, so one cell may set several flags. Any
//! match queues the 1-up jingle.
//!
//! The *consumer* of a flag — a status-bar readout, a block dispenser — reads it
//! where it acts, and needs that code whether or not this table exists. So the
//! trigger side never needs code of its own. If a future trigger needs more
//! than a flag write, the row format grows a kind; until then it does not.
//!
//! # Where it runs
//!
//! The map has PRG010 at `$C000` and PRG011 at `$A000`, and neither has room to
//! grow. PRG026 does, and the map loop already swaps it into `$A000` every frame
//! (`prg030.asm`, `PRG030_86F9` and `PRG030_8715`: page 26 for the inventory
//! and the status bar, page 11 for `Map_DoMap`), so the map running with
//! PRG026 mapped is vanilla's own normal state, NMI included.
//!
//! [`FS_ON_CLEAR`] is therefore a fixed 14-byte trampoline: map in PRG026,
//! `JSR` the dispatcher, `JMP advance`. The dispatcher puts PRG011 back itself
//! by **tail-calling** [`PRGROM_CHANGE_A000`] — its `JMP` is fetched before the
//! swap, the swap runs from PRG031, and its `RTS` lands back in the trampoline
//! in PRG010. PRG011 must be back by then: `Map_DoMap`, which op 8 returns
//! through, lives in it.
//!
//! Registers: `A`, `X` and `Y` are all clobbered. Nothing at `advance` reads
//! them — `INC Map_Operation / JMP WorldMap_UpdateAndDraw`, which loads its own
//! `Y`, and seventeen vanilla map ops reach it with whatever they left in `X`.

use crate::randomize::maze::MazePos;
use crate::randomize::overworld::lock_keys::{ADVANCE_CPU, cell_key};
use crate::randomize::rom_data::{
    FS_ON_CLEAR, FS_ON_CLEAR_DISPATCH, PAGE_A000, PRGROM_CHANGE_A000, SND_LEVEL1UP, SOUND_QLEVEL1,
    prg_bank_file_to_cpu,
};
use crate::rom::Rom;

/// One "beat this cell, set this flag" row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Trigger {
    /// The map cell whose clear fires it, `(world, (row, col))`.
    pub(crate) cell: MazePos,
    /// The flag's address. Must be in the `$7A00` page — see [`FLAG_PAGE`].
    pub(crate) flag: u16,
    /// What gets `ORA`'d into the flag.
    pub(crate) value: u8,
}

/// The high byte every flag shares, so a row stores only the low one. All of
/// the maze's SRAM (`$7AC1-$7ADF`) is in this page.
const FLAG_PAGE: u8 = 0x7A;

/// The most rows the reservation holds. Also what keeps the scan's `BPL`
/// honest: the last row's offset, `(MAX_TRIGGERS - 1) * 4`, must stay below
/// `$80` or `X` starts out negative.
pub(crate) const MAX_TRIGGERS: usize = 32;
const _: () = assert!((MAX_TRIGGERS - 1) * ROW_LEN < 0x80);

const ROW_LEN: usize = 4;

/// Bytes of the dispatcher before its table.
const CODE_LEN: usize = 47;

/// Bytes [`FS_ON_CLEAR_DISPATCH`] reserves; the table must fit behind the code.
const DISPATCH_RESERVED: usize = 192;
const _: () = assert!(CODE_LEN + MAX_TRIGGERS * ROW_LEN <= DISPATCH_RESERVED);

/// Where the dispatcher runs once PRG026 is mapped: `$B735`.
const DISPATCH_CPU: u16 = prg_bank_file_to_cpu(26, FS_ON_CLEAR_DISPATCH);
/// The table, straight after the dispatcher's code.
const TABLE_CPU: u16 = DISPATCH_CPU + CODE_LEN as u16;

/// The page numbers the trampoline and the dispatcher latch.
const PAGE_DISPATCH: u8 = 26;
const PAGE_MAP: u8 = 11;

/// The trampoline at [`FS_ON_CLEAR`]. Fixed: adding a trigger never touches it.
#[rustfmt::skip]
fn trampoline() -> [u8; 14] {
    let [page_lo, page_hi] = PAGE_A000.to_le_bytes();
    let [chg_lo, chg_hi] = PRGROM_CHANGE_A000.to_le_bytes();
    let [dsp_lo, dsp_hi] = DISPATCH_CPU.to_le_bytes();
    let [adv_lo, adv_hi] = ADVANCE_CPU.to_le_bytes();
    [
        0xA9, PAGE_DISPATCH,           //  0: LDA #26
        0x8D, page_lo, page_hi,        //  2: STA PAGE_A000
        0x20, chg_lo, chg_hi,          //  5: JSR PRGROM_Change_A000   ; PRG026 now at $A000
        0x20, dsp_lo, dsp_hi,          //  8: JSR dispatch             ; returns with PRG011 back
        0x4C, adv_lo, adv_hi,          // 11: JMP advance
    ]
}

/// The dispatcher at [`FS_ON_CLEAR_DISPATCH`], for a table of `rows` rows.
///
/// Walks the table from the last row down to row 0 (`X` = row offset), so
/// the loop's exit test is the `BPL` that `DEX` already set up — no compare.
#[rustfmt::skip]
fn dispatcher(rows: usize) -> [u8; CODE_LEN] {
    assert!((1..=MAX_TRIGGERS).contains(&rows), "the dispatcher needs 1-{MAX_TRIGGERS} rows");
    let last = ((rows - 1) * ROW_LEN) as u8;
    let [k0_lo, k0_hi] = TABLE_CPU.to_le_bytes();
    let [k1_lo, k1_hi] = (TABLE_CPU + 1).to_le_bytes();
    let [fl_lo, fl_hi] = (TABLE_CPU + 2).to_le_bytes();
    let [va_lo, va_hi] = (TABLE_CPU + 3).to_le_bytes();
    let [snd_lo, snd_hi] = SOUND_QLEVEL1.to_le_bytes();
    let [page_lo, page_hi] = PAGE_A000.to_le_bytes();
    let [chg_lo, chg_hi] = PRGROM_CHANGE_A000.to_le_bytes();
    [
        0xA2, last,                    //  0: LDX #last row's offset
        0xA5, 0x00,                    //  2: LDA <T1        ; loop: key0 of the cell just cleared
        0xDD, k0_lo, k0_hi,            //  4: CMP key0,X
        0xD0, 0x18,                    //  7: BNE next       (-> 33)
        0xA5, 0x01,                    //  9: LDA <T2        ; key1
        0xDD, k1_lo, k1_hi,            // 11: CMP key1,X
        0xD0, 0x11,                    // 14: BNE next       (-> 33)
        0xBC, fl_lo, fl_hi,            // 16: LDY flag,X     ; the flag's low byte
        0xBD, va_lo, va_hi,            // 19: LDA value,X
        0x19, 0x00, FLAG_PAGE,         // 22: ORA $7A00,Y
        0x99, 0x00, FLAG_PAGE,         // 25: STA $7A00,Y
        0xA9, SND_LEVEL1UP,            // 28: LDA #SND_LEVEL1UP
        0x8D, snd_lo, snd_hi,          // 30: STA Sound_QLevel1
        0xCA,                          // 33: DEX            ; next: back one row
        0xCA,                          // 34: DEX
        0xCA,                          // 35: DEX
        0xCA,                          // 36: DEX
        0x10, 0xDB,                    // 37: BPL loop       (-> 2)
        0xA9, PAGE_MAP,                // 39: LDA #11
        0x8D, page_lo, page_hi,        // 41: STA PAGE_A000
        0x4C, chg_lo, chg_hi,          // 44: JMP PRGROM_Change_A000   ; tail call: its RTS
                                       //     returns to the trampoline
    ]
}

/// One table row's bytes.
fn row(t: &Trigger) -> [u8; ROW_LEN] {
    let [lo, hi] = t.flag.to_le_bytes();
    assert_eq!(hi, FLAG_PAGE, "on-clear flag ${:04X} is outside the $7A00 page", t.flag);
    let [key0, key1] = cell_key(t.cell.0, t.cell.1);
    [key0, key1, lo, t.value]
}

/// Write the trampoline, the dispatcher and the table. `install_on_clear` (in
/// the lock stage, after the effect it hangs off is written) is what makes op
/// 8 reach the trampoline, and it must only run when this has.
pub(crate) fn apply(rom: &mut Rom, triggers: &[Trigger]) {
    rom.write_range(FS_ON_CLEAR, &trampoline());
    rom.write_range(FS_ON_CLEAR_DISPATCH, &dispatcher(triggers.len()));
    let table: Vec<u8> = triggers.iter().flat_map(row).collect();
    rom.write_range(FS_ON_CLEAR_DISPATCH + CODE_LEN, &table);
}

/// The installed table read back, or `None` when the ROM has no dispatcher.
/// Every code byte must match, so the reclaimed vanilla code under
/// [`FS_ON_CLEAR`] cannot pass for one.
#[cfg(any(test, not(target_arch = "wasm32")))]
pub(crate) fn installed(rom: &Rom) -> Option<Vec<Trigger>> {
    if rom.read_range(FS_ON_CLEAR, 14) != trampoline() {
        return None;
    }
    let rows = usize::from(rom.read_byte(FS_ON_CLEAR_DISPATCH + 1)) / ROW_LEN + 1;
    if rows > MAX_TRIGGERS || rom.read_range(FS_ON_CLEAR_DISPATCH, CODE_LEN) != dispatcher(rows) {
        return None;
    }
    let table = rom.read_range(FS_ON_CLEAR_DISPATCH + CODE_LEN, rows * ROW_LEN);
    Some(
        table
            .chunks(ROW_LEN)
            .map(|r| Trigger {
                cell: cell_from_key(r[0], r[1]),
                flag: u16::from_le_bytes([r[2], FLAG_PAGE]),
                value: r[3],
            })
            .collect(),
    )
}

/// [`cell_key`] inverted.
#[cfg(any(test, not(target_arch = "wasm32")))]
fn cell_from_key(key0: u8, key1: u8) -> MazePos {
    let world = usize::from(key0 & 0x0F);
    let row = usize::from(key0 >> 4) - 2;
    let col = usize::from(key1 >> 4) + 16 * usize::from(key1 & 0x0F);
    (world, (row, col))
}

/// Move every row that sets `flag` onto `cell`, keeping its value: `testrom`'s
/// way to playtest a trigger on a level next to the start. `false`, and
/// nothing written, when there is no dispatcher or no row sets `flag`.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn retarget(rom: &mut Rom, flag: u16, cell: MazePos) -> bool {
    let Some(mut triggers) = installed(rom) else {
        return false;
    };
    let mut moved = false;
    for t in triggers.iter_mut().filter(|t| t.flag == flag) {
        t.cell = cell;
        moved = true;
    }
    if moved {
        apply(rom, &triggers);
    }
    moved
}

#[cfg(test)]
mod asm_checks {
    use super::*;
    use crate::randomize::overworld::lock_keys::ON_CLEAR_CPU;
    use crate::randomize::rom_data::{NMI_SAFE_MAX, asm};
    use mos6502::cpu::CPU;
    use mos6502::instruction::Ricoh2a03;
    use mos6502::memory::{Bus, Memory};

    #[test]
    fn trampoline_is_well_formed() {
        asm::check(&trampoline())
            .allocation(FS_ON_CLEAR)
            .origin(ON_CLEAR_CPU)
            .zero_page(NMI_SAFE_MAX, &[])
            .assert_ok();
    }

    #[test]
    fn dispatcher_is_well_formed() {
        for rows in [1, 2, 9, MAX_TRIGGERS] {
            let triggers: Vec<Trigger> = (0..rows)
                .map(|i| Trigger { cell: (i % 8, (2, i)), flag: 0x7AC1 + i as u16, value: 1 })
                .collect();
            let mut code = dispatcher(rows).to_vec();
            code.extend(triggers.iter().flat_map(row));
            asm::check(&code)
                .allocation(FS_ON_CLEAR_DISPATCH)
                .origin(DISPATCH_CPU)
                .data_from(CODE_LEN)
                .zero_page(NMI_SAFE_MAX, &[])
                .assert_ok();
        }
    }

    /// Run trampoline + dispatcher on an emulated 2A03 with the player on `at`
    /// and `sram` preloaded, and return the memory at `advance`. The bank swap
    /// is a stub `RTS` at `$FFC2` — flat memory has both routines mapped at
    /// once — so what is checked is the latch, [`PAGE_A000`], and the stack.
    fn run(triggers: &[Trigger], at: MazePos, sram: &[(u16, u8)]) -> Memory {
        let mut mem = Memory::new();
        mem.set_bytes(ON_CLEAR_CPU, &trampoline());
        mem.set_bytes(DISPATCH_CPU, &dispatcher(triggers.len()));
        let table: Vec<u8> = triggers.iter().flat_map(row).collect();
        mem.set_bytes(TABLE_CPU, &table);
        mem.set_byte(PRGROM_CHANGE_A000, 0x60);
        let [k0, k1] = cell_key(at.0, at.1);
        mem.set_byte(0x0000, k0);
        mem.set_byte(0x0001, k1);
        mem.set_byte(PAGE_A000, PAGE_MAP);
        for &(addr, v) in sram {
            mem.set_byte(addr, v);
        }
        let mut cpu = CPU::new(mem, Ricoh2a03);
        cpu.registers.program_counter = ON_CLEAR_CPU;
        let sp = cpu.registers.stack_pointer;
        for _ in 0..2000 {
            if cpu.registers.program_counter == ADVANCE_CPU {
                assert_eq!(cpu.registers.stack_pointer, sp, "the stack is unbalanced");
                assert_eq!(cpu.memory.get_byte(PAGE_A000), PAGE_MAP, "PRG011 not latched back");
                return cpu.memory;
            }
            cpu.single_step();
        }
        panic!("the on-clear trampoline never reached advance");
    }

    const A: MazePos = (2, (4, 20));
    const B: MazePos = (5, (6, 37));

    /// A match ORs its value in and queues the jingle; a near miss — same
    /// world, same row, same column, another world — changes nothing.
    #[test]
    fn only_the_keyed_cell_fires() {
        let t = [Trigger { cell: A, flag: 0x7ADE, value: 0xF4 }];
        let mut mem = run(&t, A, &[]);
        assert_eq!((mem.get_byte(0x7ADE), mem.get_byte(SOUND_QLEVEL1)), (0xF4, SND_LEVEL1UP));
        for other in [(2, (4, 21)), (2, (5, 20)), (3, (4, 20)), (2, (4, 4)), (2, (6, 36))] {
            let mut mem = run(&t, other, &[]);
            assert_eq!(
                (mem.get_byte(0x7ADE), mem.get_byte(SOUND_QLEVEL1)),
                (0, 0),
                "{other:?} fired"
            );
        }
    }

    /// Every row is scanned: rows sharing a byte set their own bits and leave
    /// the others alone, one cell can fire two rows, and the first and last
    /// rows of a full table are both reached.
    #[test]
    fn every_row_is_scanned_and_bits_accumulate() {
        let mut t: Vec<Trigger> = (0..MAX_TRIGGERS - 3)
            .map(|i| Trigger { cell: (7, (8, i)), flag: 0x7AC1, value: 0x80 })
            .collect();
        t.insert(0, Trigger { cell: A, flag: 0x7ADF, value: 0x01 });
        t.push(Trigger { cell: B, flag: 0x7ADF, value: 0x04 });
        t.push(Trigger { cell: B, flag: 0x7ADE, value: 0x10 });
        assert_eq!(t.len(), MAX_TRIGGERS);

        let mut mem = run(&t, A, &[(0x7ADF, 0x40)]);
        assert_eq!(mem.get_byte(0x7ADF), 0x41, "row 0 must OR, not store");
        let mut mem = run(&t, B, &[(0x7ADF, 0x01)]);
        assert_eq!((mem.get_byte(0x7ADF), mem.get_byte(0x7ADE)), (0x05, 0x10));
        assert_eq!(mem.get_byte(0x7AC1), 0, "a row for another cell fired");
    }

    fn blank_rom() -> Rom {
        // Minimal valid iNES image: header + 256 KiB PRG + 128 KiB CHR, zeroed.
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;
        Rom::from_bytes_lax(&data, true).unwrap()
    }

    #[test]
    fn installed_reads_back_what_apply_wrote() {
        let mut rom = blank_rom();
        assert_eq!(installed(&rom), None, "a blank ROM has no dispatcher");
        let t = vec![
            Trigger { cell: A, flag: 0x7ADE, value: 0xF3 },
            Trigger { cell: (7, (8, 47)), flag: 0x7ADF, value: 0x02 },
        ];
        apply(&mut rom, &t);
        assert_eq!(installed(&rom), Some(t.clone()));

        assert!(retarget(&mut rom, 0x7ADF, B));
        assert_eq!(installed(&rom).unwrap()[1], Trigger { cell: B, ..t[1] });
        assert_eq!(installed(&rom).unwrap()[0], t[0], "a row for another flag moved");
        assert!(!retarget(&mut rom, 0x7AC1, B), "no row sets that flag");
    }
}
