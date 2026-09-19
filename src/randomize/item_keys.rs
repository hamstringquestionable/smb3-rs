//! Item keys — the dispenser gate, as a proof of concept.
//!
//! **This is not the feature.** It is the smallest thing that answers the one
//! question MiMaze rests on: *is a level that needs its own power-up genuinely
//! unbeatable once that power-up stops being dispensed, and does a locked
//! block read as a locked door rather than a bug?* See
//! `docs/mimaze_layer_design.md`; if the answer is no, nothing downstream is
//! worth writing.
//!
//! What is real here is the **routine and its allocation** — the same 28 bytes
//! and the same shape the shipping feature needs. What is fake is where the
//! mask comes from: the found set is baked in at build time as a table of
//! products rather than read from SRAM. Turning this into the feature means
//! changing that one thing.
//!
//! # The split, and why it is load-bearing
//!
//! `LATP_Flower` and `LATP_Leaf` each have **two** products, chosen by
//! vanilla's own `Player_Suit` test: a mushroom for a small player, the suit
//! for a big one. They have to be gated *independently*. Degrading the whole
//! handler would take the mushroom default with it, so a player who has found
//! a leaf but no mushroom could not get big, could not break a brick, and
//! would be stuck in a level nothing authored as a gate.
//!
//! So the gate splits where vanilla already splits, and the mushroom becomes a
//! first-class key: locked, the player is permanently small.
//!
//! # The routine
//!
//! ```text
//!   LDA #$00 / STA PUp_StarManFlash   vanilla's own flash clear
//!   TYA / LSR A                       Y is the block type * 2 on entry, so
//!                                     this is the row: 1 flower, 2 leaf
//!   LDY Player_Suit / BNE big
//!   LDA #$00                          small -> row 0, the mushroom
//! big:
//!   TAY / LDA PRODUCTS,Y              0 means locked
//!   BEQ locked
//!   TAY / RTS                         Y = the Bouncer_PUp index
//! locked:
//!   JMP LATP_Coin
//! ```
//!
//! Three things make it fit in 28 bytes with one to spare.
//!
//! **`Y` already holds the block type.** The dispatcher does
//! `LDA Temp_Var1 / ASL A / TAY` and never touches `Y` again before
//! `JMP [Temp_Var1]`, so the handler is entered with `type * 2` in `Y` and the
//! row index costs two bytes rather than a pair of entry stubs.
//!
//! **One table, not two.** A row holds the `Bouncer_PUp` index to return, and
//! **zero means locked** — so the "is it unlocked" test and the "what does it
//! give" lookup are one `LDA`, and the mask never appears in the code at all.
//! That is what makes the found set a build-time table here and an SRAM read
//! later: only the table's *source* changes.
//!
//! **`X` is never touched.** It carries the tile-check index into these
//! handlers — `LATP_Brick` reads it with `CPX #$04`, and
//! `LATP_GetCoinAboveBlock` backs it up around a call — so the row index goes
//! in `Y`, which the handler owns. Indexing with `X` would have been the same
//! byte count and a live-register bug.
//!
//! # Where it lives
//!
//! `LATP_Flower`, `LATP_Leaf` and `LATP_Star` are each referenced from exactly
//! one place — their own word in `LATP_JumpTable`. Repointing those three
//! words frees all three bodies outright, and they are adjacent: **36
//! contiguous bytes** at CPU `$B7EC`, in the dispatcher's own bank, so there
//! is no mapping question and none of the nearly-full always-mapped banks are
//! spent. PRG008 has **zero** bytes of `$FF` filler, so reclaiming is not
//! merely the cheapest option here, it is the only one.
//!
//! # The star is all-or-nothing
//!
//! `LATP_Star` has no `Player_Suit` split — a star block gives a starman to
//! anyone — so it needs no routine. When the star is not found, entry 3 is
//! pointed at `LATP_Coin` directly; when it is found, the entry is left alone.
//! Its 8-byte body stays intact either way, because the found case still runs
//! it.
//!
//! The Big [?] block (7-F1's tanooki) is **not** covered: it is an object in
//! PRG005, a different bank and a different mechanism. One bank at a time.

use crate::randomize::rom_data::FS_ITEM_GATE;
use crate::rom::Rom;

/// `LATP_JumpTable`, PRG008 CPU `$B7D1`: twelve words, indexed by `type * 2`.
const LATP_JUMP_TABLE: usize = 0x117E1;

/// CPU addresses of the three vanilla handlers this pass may retire, and of
/// the coin handler both degrade paths reach.
const LATP_FLOWER_CPU: u16 = 0xB7EC;
const LATP_LEAF_CPU: u16 = 0xB7FA;
const LATP_STAR_CPU: u16 = 0xB808;
const LATP_COIN_CPU: u16 = 0xB810;

/// The gate routine replaces `LATP_Flower` in place, so its origin is that
/// handler's own address.
const GATE_CPU: u16 = LATP_FLOWER_CPU;

/// A key, as the ROM tables encode one.
///
/// Test-only: [`FOUND_RECORD`]'s tables and the gate's rows are the source of
/// truth, and this exists so a test can cross-check them against a statement
/// of the domain rather than against themselves.
///
/// The value is the `Bouncer_PUp` index the block returns when the item is
/// found — `prg001.asm:1015`: `$00, $00, FIREFLOWER, SUPERLEAF, STARMAN,
/// MUSHROOM, GROWINGVINE, 1UP`.
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Mushroom,
    Flower,
    Leaf,
    Star,
}

#[cfg(test)]
impl Key {
    /// The `Bouncer_PUp` index this key's block dispenses once found.
    const fn product(self) -> u8 {
        match self {
            Key::Mushroom => 5,
            Key::Flower => 2,
            Key::Leaf => 3,
            Key::Star => 4,
        }
    }

    /// The routine's row for this key. Rows 1-3 are the block type the
    /// dispatcher already has in `Y`; row 0 is where a small player is sent.
    const fn row(self) -> usize {
        match self {
            Key::Mushroom => 0,
            Key::Flower => 1,
            Key::Leaf => 2,
            Key::Star => 3,
        }
    }
}

/// Where the found table lives: [`maze_state::FOUND_PRODUCTS`], four bytes of
/// SRAM. Row 0 mushroom, 1 fire flower, 2 super leaf, 3 starman — rows 1-3 are
/// the block type the dispatcher already has in `Y`.
const PRODUCTS: u16 = crate::randomize::maze_state::FOUND_PRODUCTS;

/// `PUp_StarManFlash`. Vanilla's flower and leaf handlers clear it; the star
/// handler sets it. One row per product row, so the merged routine keeps both
/// behaviours for a three-byte table and no branch.
const FLASH_OFFSET: usize = 29;
const FLASH_CPU: u16 = GATE_CPU + FLASH_OFFSET as u16;

/// The gate, covering all three power-up entries.
///
/// ```text
///   TYA / LSR A        Y is type*2 on entry, so A is the row: 1 flower,
///                      2 leaf, 3 star
///   CMP #$03 / BEQ     a star ignores Player_Suit, as vanilla does
///   LDY Player_Suit
///   BNE have
///   LDA #$00           small -> row 0, the mushroom
/// have:
///   TAY
///   LDA FLASH,Y / STA PUp_StarManFlash
///   LDA PRODUCTS,Y     SRAM; zero means locked
///   BEQ locked
///   TAY / RTS          Y = the Bouncer_PUp index
/// locked:
///   JMP LATP_Coin
/// ```
///
/// 33 bytes of the 36 reclaimed by repointing all three jump-table words.
#[rustfmt::skip]
const GATE: [u8; 33] = [
    0x98,                                              //  0: TYA
    0x4A,                                              //  1: LSR A
    0xC9, 0x03,                                        //  2: CMP #$03      ; star?
    0xF0, 0x06,                                        //  4: BEQ have
    0xA4, 0xED,                                        //  6: LDY Player_Suit
    0xD0, 0x02,                                        //  8: BNE have
    0xA9, 0x00,                                        // 10: LDA #$00      ; small -> row 0
    0xA8,                                              // 12: have: TAY
    0xB9, FLASH_CPU as u8, (FLASH_CPU >> 8) as u8,     // 13: LDA FLASH,Y
    0x8D, 0x86, 0x05,                                  // 16: STA PUp_StarManFlash
    0xB9, PRODUCTS as u8, (PRODUCTS >> 8) as u8,       // 19: LDA PRODUCTS,Y
    0xF0, 0x02,                                        // 22: BEQ locked
    0xA8,                                              // 24: TAY
    0x60,                                              // 25: RTS
    0x4C, LATP_COIN_CPU as u8, (LATP_COIN_CPU >> 8) as u8, // 26: locked: JMP LATP_Coin
    0x00, 0x00, 0x00, 0x80,                            // 29: FLASH: -, -, -, starman
];

/// The **easier arm**: the same gate with the `Player_Suit` test removed.
///
/// `qol::apply_modern_powerups` rewrites the two `LDY #$05` operands inside
/// the vanilla handlers so a *small* player is handed the suit directly. Under
/// it both of a handler's paths return the same product, so the suit test is
/// dead weight — and, more to the point, the mushroom stops being a rung to
/// gate at all. Dropping the test is what expresses that: the row is always
/// the block type, row 0 is never read, and the player never has to find a
/// mushroom to start using what they find.
///
/// 23 bytes, in the same allocation. See the allocation section of
/// `docs/item_keys_design.md`.
#[rustfmt::skip]
const GATE_EASY: [u8; 23] = [
    0x98,                                              //  0: TYA
    0x4A,                                              //  1: LSR A         ; row = block type
    0xA8,                                              //  2: TAY
    0xB9, EASY_FLASH_CPU as u8, (EASY_FLASH_CPU >> 8) as u8, //  3: LDA FLASH,Y
    0x8D, 0x86, 0x05,                                  //  6: STA PUp_StarManFlash
    0xB9, PRODUCTS as u8, (PRODUCTS >> 8) as u8,       //  9: LDA PRODUCTS,Y
    0xF0, 0x02,                                        // 12: BEQ locked
    0xA8,                                              // 14: TAY
    0x60,                                              // 15: RTS
    0x4C, LATP_COIN_CPU as u8, (LATP_COIN_CPU >> 8) as u8, // 16: locked: JMP LATP_Coin
    0x00, 0x00, 0x00, 0x80,                            // 19: FLASH
];

const EASY_FLASH_OFFSET: usize = 19;
const EASY_FLASH_CPU: u16 = GATE_CPU + EASY_FLASH_OFFSET as u16;

/// File offset of jump-table entry `n`'s word.
const fn entry_offset(n: usize) -> usize {
    LATP_JUMP_TABLE + n * 2
}

fn repoint(rom: &mut Rom, entry: usize, vanilla: u16, to: u16) {
    let off = entry_offset(entry);
    let found = u16::from_le_bytes([rom.read_byte(off), rom.read_byte(off + 1)]);
    assert_eq!(
        found, vanilla,
        "LATP_JumpTable entry {entry} holds {found:#06X}, not vanilla's {vanilla:#06X}"
    );
    rom.write_range(off, &to.to_le_bytes());
}

/// Install the dispenser gate.
///
/// `easier` selects the arm: `false` keeps the mushroom as a key, `true` is
/// the Modern Power-ups arm where it is not a gate. The found set itself lives
/// in SRAM at [`PRODUCTS`] and is written by whoever grants an item, so
/// nothing here bakes it in.
pub fn apply(rom: &mut Rom, easier: bool) {
    // Modern Power-ups rewrites the two `LDY #$05` operands inside these very
    // handlers (0x11802, 0x11810), so a small player is powered up directly.
    // With `easier` that is the intent and the routine accounts for it; the
    // combination is only wrong when this arm does not know about it.
    if !easier {
        assert_eq!(
            rom.read_byte(0x11802),
            0x05,
            "LATP_Flower's small product is not vanilla's mushroom — Modern Power-ups \
             is installed, so pass `easier` and use the arm that expects it"
        );
    }
    rom.push_tag("item_keys/gate");
    if easier {
        rom.write_range(FS_ITEM_GATE, &GATE_EASY);
    } else {
        rom.write_range(FS_ITEM_GATE, &GATE);
    }
    repoint(rom, 1, LATP_FLOWER_CPU, GATE_CPU);
    repoint(rom, 2, LATP_LEAF_CPU, GATE_CPU);
    repoint(rom, 3, LATP_STAR_CPU, GATE_CPU);
    rom.pop_tag();

    // The one source wired so far. Without it the found table never changes
    // and the gate is the all-coin POC again.
    rom.push_tag("item_keys/found");
    assert_eq!(
        rom.read_range(TOAD_GRANT, 3),
        &TOAD_GRANT_VANILLA[..],
        "the Toad House grant site is not vanilla's LDA"
    );
    rom.write_range(FS_FOUND_RECORD, &FOUND_RECORD);
    rom.write_range(TOAD_GRANT, &[0x20, RECORD_CPU as u8, (RECORD_CPU >> 8) as u8]);
    rom.pop_tag();
}

// --- Finding an item -------------------------------------------------
//
// One source is wired so far: the Toad House chest. The Hammer Bro reward and
// the Princess letter are the other two the design names, and they are
// separate sites in other banks.

/// `PRG029_D1B1`, the tail of `ToadHouse_ChestPressB`: `LDA
/// ToadHouse_Item2Inventory,X / TAX / INX / RTS`. `A` holds the Global Item ID
/// the chest is about to hand over, which is exactly what the found table
/// needs to record.
const TOAD_GRANT: usize = 0x3B1C1;
/// What stands there in vanilla — the `LDA` this pass displaces.
const TOAD_GRANT_VANILLA: [u8; 3] = [0xBD, 0x3B, 0xD1];

/// Where the recorder lives: the tail of PRG029, which `prg029.asm` ends by
/// declaring "Rest of ROM bank was empty" — so the run from here to the bank
/// end is filler, not data something reads.
const FS_FOUND_RECORD: usize = 0x3B81A;
const RECORD_CPU: u16 = 0xD80A;
const ROW_CPU: u16 = RECORD_CPU + 21;
const PROD_CPU: u16 = RECORD_CPU + 31;

/// Record a granted item in the found table, then carry on as vanilla did.
///
/// Reached by replacing the `LDA` at [`TOAD_GRANT`] with a `JSR` here: the
/// routine performs that load itself, records what it saw, and returns with
/// `A` still holding the item id so the caller's `TAX / INX / RTS` is
/// untouched. `X` is free to clobber — the caller overwrites it with `TAX` on
/// the next instruction — but `A` is not, which is why the id is parked in
/// `Y` and restored.
///
/// Two tables rather than one packed byte: unpacking a nibble pair costs more
/// than the ten bytes it would save, and `$FF` in the row table is a clean
/// "not a key" that `BMI` tests for nothing.
#[rustfmt::skip]
const FOUND_RECORD: [u8; 41] = [
    0xBD, 0x3B, 0xD1,                              //  0: LDA ToadHouse_Item2Inventory,X
    0xA8,                                          //  3: TAY            ; keep the id
    0xC0, 0x0A,                                    //  4: CPY #$0A
    0xB0, 0x0B,                                    //  6: BCS done       ; ids $0A+ are not keys
    0xBE, ROW_CPU as u8, (ROW_CPU >> 8) as u8,     //  8: LDX ROW,Y
    0x30, 0x06,                                    // 11: BMI done       ; $FF = not a key
    0xB9, PROD_CPU as u8, (PROD_CPU >> 8) as u8,   // 13: LDA PROD,Y
    0x9D, PRODUCTS as u8, (PRODUCTS >> 8) as u8,   // 16: STA FOUND_PRODUCTS,X
    0x98,                                          // 19: done: TYA      ; A = id, as the caller expects
    0x60,                                          // 20: RTS
    // 21: ROW — the gate's row for each Global Item ID, $FF for "not a key".
    0xFF, 0x00, 0x01, 0x02, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x03,
    // 31: PROD — the Bouncer_PUp index that row then dispenses.
    0x00, 0x05, 0x02, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04,
];

#[cfg(test)]
mod tests {
    use super::*;

    const ROM_PATH: &str = "roms/Super Mario Bros. 3 (USA) (Rev 1).nes";

    fn vanilla() -> Option<Rom> {
        Rom::from_bytes(&std::fs::read(ROM_PATH).ok()?).ok()
    }

    fn word(rom: &Rom, entry: usize) -> u16 {
        let off = entry_offset(entry);
        u16::from_le_bytes([rom.read_byte(off), rom.read_byte(off + 1)])
    }

    /// Both sites are where this module says they are. Without this the
    /// writes land in the middle of something else.
    #[test]
    fn the_rom_is_shaped_the_way_this_assumes() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        assert_eq!(word(&rom, 0), 0xB7E9, "entry 0 = LATP_None");
        assert_eq!(word(&rom, 1), LATP_FLOWER_CPU);
        assert_eq!(word(&rom, 2), LATP_LEAF_CPU);
        assert_eq!(word(&rom, 3), LATP_STAR_CPU);
        assert_eq!(word(&rom, 4), LATP_COIN_CPU);
        assert_eq!(rom.read_range(TOAD_GRANT, 3), &TOAD_GRANT_VANILLA[..]);
        assert_eq!(FS_ITEM_GATE, 0x117FC, "the routine goes where LATP_Flower was");
    }

    /// All three power-up entries reach the routine, and no other vector
    /// moves. The star has to come through it now: with the found set in SRAM
    /// its locked state changes while the game runs, so it can no longer be
    /// decided by repointing at build time.
    #[test]
    fn all_three_entries_reach_the_gate() {
        let Some(mut rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let before: Vec<u16> = (0..12).map(|n| word(&rom, n)).collect();
        apply(&mut rom, false);
        for (n, &was) in before.iter().enumerate() {
            let expected = match n {
                1..=3 => GATE_CPU,
                _ => was,
            };
            assert_eq!(word(&rom, n), expected, "entry {n}");
        }
    }

    /// The grant site calls the recorder and keeps its own tail.
    #[test]
    fn the_toad_house_grant_is_hooked() {
        let Some(mut rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        apply(&mut rom, false);
        assert_eq!(
            rom.read_range(TOAD_GRANT, 3),
            &[0x20, RECORD_CPU as u8, (RECORD_CPU >> 8) as u8],
            "the LDA should have become a JSR"
        );
        assert_eq!(rom.read_range(TOAD_GRANT + 3, 3), &[0xAA, 0xE8, 0x60], "TAX / INX / RTS");
        assert_eq!(rom.read_range(FS_FOUND_RECORD, 41), &FOUND_RECORD[..]);
    }

    /// The recorder's two tables agree with the gate's rows, for every key.
    /// A drift here would record a leaf into the mushroom's row and unlock
    /// the wrong thing.
    #[test]
    fn recorder_rows_match_the_gate_rows() {
        const ROW: usize = 21;
        const PROD: usize = 31;
        for (id, key) in [(1usize, Key::Mushroom), (2, Key::Flower), (3, Key::Leaf), (9, Key::Star)]
        {
            assert_eq!(FOUND_RECORD[ROW + id] as usize, key.row(), "row for {key:?}");
            assert_eq!(FOUND_RECORD[PROD + id], key.product(), "product for {key:?}");
        }
        // Everything else is "not a key", and must be, or an unrelated item
        // would silently open a gate.
        for id in [0usize, 4, 5, 6, 7, 8] {
            assert_eq!(FOUND_RECORD[ROW + id], 0xFF, "item {id} must not be a key");
        }
    }

    /// The easier arm drops the suit test, so its row is always the block
    /// type and the mushroom row is never read.
    #[test]
    fn the_easier_arm_has_no_suit_test() {
        assert!(!GATE_EASY.contains(&0xED), "Player_Suit must not be read");
        assert!(GATE.contains(&0xED), "the default arm must read it");
        assert!(GATE_EASY.len() < GATE.len());
    }
}

#[cfg(test)]
mod asm_checks {
    use super::*;
    use crate::randomize::rom_data::asm;

    /// The routine decodes, ends in a `JMP`, its two branches land on
    /// instruction boundaries, it fits its allocation without leaving PRG008,
    /// and `LDA PRODUCTS,Y` resolves back into its own tail.
    ///
    /// `.origin` is the one that matters: the products table is addressed
    /// absolutely, so the routine cannot be relocated without recomputing it.
    #[test]
    fn gate_is_well_formed() {
        asm::check(&GATE)
            .allocation(FS_ITEM_GATE)
            .origin(GATE_CPU)
            .data_from(FLASH_OFFSET)
            .assert_ok();
    }

    /// The easier arm shares the allocation and the origin.
    #[test]
    fn easier_gate_is_well_formed() {
        asm::check(&GATE_EASY)
            .allocation(FS_ITEM_GATE)
            .origin(GATE_CPU)
            .data_from(EASY_FLASH_OFFSET)
            .assert_ok();
    }

    /// The recorder is reached by a  that displaces a whole instruction,
    /// and its two tables are data, not code.
    #[test]
    fn found_record_is_well_formed() {
        asm::check(&FOUND_RECORD)
            .origin(RECORD_CPU)
            .data_from(21)
            .hook(&TOAD_GRANT_VANILLA, 0, &[0x20, RECORD_CPU as u8, (RECORD_CPU >> 8) as u8])
            .assert_ok();
    }
}
