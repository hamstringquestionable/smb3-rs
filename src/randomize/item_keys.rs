//! Item keys — a power-up block only dispenses what the player has found.
//!
//! The ROM half of `docs/mimaze_layer_design.md`, which turns on one chain: a
//! level tile is a wall until it is beaten; a level that requires a power-up
//! carries its own renewable source of it (`vision.md`'s fourth charter
//! point); so **gating that source turns the level into a wall.** This is the
//! gating.
//!
//! Reached only from `testrom` so far — no flag key, no web control, nothing
//! in a shipped seed.
//!
//! # The split, and why it is load-bearing
//!
//! `LATP_Flower` and `LATP_Leaf` each have **two** products, chosen by
//! vanilla's own `Player_Suit` test: a mushroom for a small player, the suit
//! for a big one. They have to be gated *independently*. Degrading the whole
//! handler would take the mushroom default with it, so a player who found a
//! leaf but no mushroom could not get big, could not break a brick, and would
//! be stuck in a level nothing authored as a gate.
//!
//! So the gate splits where vanilla already splits, and the mushroom becomes a
//! first-class key: locked, the player is permanently small.
//!
//! # Two halves
//!
//! **The gate** replaces all three power-up handlers. Repointing their
//! jump-table words frees `LATP_Flower`, `LATP_Leaf` and `LATP_Star` outright
//! — 36 adjacent bytes at `$B7EC`, in the dispatcher's own bank, so there is
//! no mapping question and none of the nearly-full always-mapped banks are
//! spent. PRG008 has **zero** `$FF` filler, so reclaiming is not the cheapest
//! option here, it is the only one.
//!
//! **The found table** is four bytes of SRAM
//! ([`maze_state::FOUND_PRODUCTS`](crate::randomize::maze_state::FOUND_PRODUCTS)),
//! one row per key, written by whoever grants an item. A row holds the
//! `Bouncer_PUp` index to dispense and **zero means locked**, so the gate's
//! "is it unlocked" test and its "what does it give" lookup are a single
//! `LDA` and no mask ever appears in the code. That is also what let the table
//! move from ROM to SRAM without the routine changing.
//!
//! # Three things that keep the gate small
//!
//! **`Y` already holds the block type.** The dispatcher does
//! `LDA Temp_Var1 / ASL A / TAY` and never touches `Y` again before
//! `JMP [Temp_Var1]`, so the row index is `TYA / LSR A` rather than a pair of
//! entry stubs.
//!
//! **The star rides along.** It has no `Player_Suit` split, so one `CMP #$03`
//! sends it past that test, and a flash-value table row keeps its `$80` where
//! the others want `$00` — no second routine and no branch.
//!
//! **`X` is never touched.** It carries the tile-check index into these
//! handlers — `LATP_Brick` reads it with `CPX #$04`, `LATP_GetCoinAboveBlock`
//! backs it up around a call — so the row index goes in `Y`, which the handler
//! owns. Indexing with `X` was the same byte count and a live-register bug.
//!
//! # What is not covered
//!
//! The Big [?] block (7-F1's tanooki) is an object in PRG005, a different bank
//! and a different mechanism. One bank at a time.

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

    install_found_recorder(rom);
}

// --- Finding an item -------------------------------------------------
//
// **There is no single "an item entered the inventory" routine.** Vanilla
// open-codes the free-slot scan in three places, and each writes
// `Inventory_Items` itself:
//
// | site | bank | grants |
// |---|---|---|
// | `Player_GetItem` `$FD6C` | PRG031 | in-level chests, **Hammer Bro rewards**, N-Spade card matches |
// | `ToadHouse_GiveItem` | PRG000 | Toad House chests |
// | `Letter_GiveIncludedItem` `$A1D9` | PRG027 | Princess letters |
//
// So there are three hooks — but only **one recorder**, and it lives in
// PRG031 because that bank is always mapped and can therefore be `JSR`ed
// from all three regardless of what is banked at the time. That is what
// always-mapped space is for, and it is the whole reason this costs 25 bytes
// rather than three copies of them.
//
// It also puts the item-id-to-row mapping in exactly one place. A second
// copy is how a leaf ends up recorded in the mushroom's row.

/// The shared recorder: `A` is a Global Item ID on entry, and the found table
/// row for it (if any) is set to the product that row dispenses.
///
/// The mapping is regular enough to compute rather than look up: ids 1, 2, 3
/// are rows 0, 1, 2, and id 9 (the starman) is row 3. Id 0 wraps to `$FF`
/// under the subtract and falls out of the same bound check that rejects 4
/// and up, so "not a key" costs no test of its own.
///
/// Clobbers `A` and `X`; **preserves `Y`**, which every caller needs — two of
/// them are holding an inventory offset or a world number across the call.
#[rustfmt::skip]
const RECORD: [u8; 25] = [
    0xA2, 0x03,                                    //  0: LDX #$03      ; the starman's row
    0xC9, 0x09,                                    //  2: CMP #$09
    0xF0, 0x08,                                    //  4: BEQ have
    0x38,                                          //  6: SEC
    0xE9, 0x01,                                    //  7: SBC #$01      ; row = id - 1
    0xC9, 0x03,                                    //  9: CMP #$03
    0xB0, 0x07,                                    // 11: BCS done      ; id 0 -> $FF, ids 4+ -> out
    0xAA,                                          // 13: TAX
    0xBD, PROD_CPU as u8, (PROD_CPU >> 8) as u8,   // 14: have: LDA PROD,X
    0x9D, PRODUCTS as u8, (PRODUCTS >> 8) as u8,   // 17: STA FOUND_PRODUCTS,X
    0x60,                                          // 20: done: RTS
    0x05, 0x02, 0x03, 0x04,                        // 21: PROD by row
];

const FS_RECORD: usize = 0x3E972;
const RECORD_CPU: u16 = 0xE962;
const PROD_CPU: u16 = RECORD_CPU + 21;

/// `Player_GetItem`'s tail — `PLA / STA Inventory_Items,Y / RTS`. Hooked at
/// the `STA` rather than the entry because `A` is the item there and nothing
/// has to be saved around the call.
const GETITEM_TAIL: usize = 0x3FD90;
const GETITEM_TAIL_VANILLA: [u8; 4] = [0x99, 0x80, 0x7D, 0x60];
const FS_GETITEM_TAIL: usize = 0x3E2C6;
const GETITEM_TAIL_CPU: u16 = 0xE2B6;

#[rustfmt::skip]
const GETITEM_TAIL_HOOK: [u8; 7] = [
    0x99, 0x80, 0x7D,                                      // 0: STA Inventory_Items,Y (displaced)
    0x20, RECORD_CPU as u8, (RECORD_CPU >> 8) as u8,       // 3: JSR RECORD
    0x60,                                                  // 6: RTS
];

/// `Letter_GiveIncludedItem`: `LDA LetterItem_ByWorld,Y / STA CineKing_Var`,
/// then a `BEQ` on "no item this world". The hook re-reads `CineKing_Var`
/// before returning so that branch still sees the right `Z`.
const LETTER_SITE: usize = 0x361EC;
const LETTER_SITE_VANILLA: [u8; 5] = [0xB9, 0xCE, 0xA0, 0x85, 0x9A];
const FS_LETTER_HOOK: usize = 0x37D57;
const LETTER_HOOK_CPU: u16 = 0xBD47;

#[rustfmt::skip]
const LETTER_HOOK: [u8; 11] = [
    0xB9, 0xCE, 0xA0,                                      // 0: LDA LetterItem_ByWorld,Y
    0x85, 0x9A,                                            // 3: STA CineKing_Var
    0x20, RECORD_CPU as u8, (RECORD_CPU >> 8) as u8,       // 5: JSR RECORD
    0xA5, 0x9A,                                            // 8: LDA CineKing_Var  ; restore A and Z
    0x60,                                                  // 10: RTS
];

/// `ToadHouse_ChestPressB`'s tail. `A` is the item and the caller's next
/// instruction is `TAX`, so `X` is free to clobber but `A` is not — hence the
/// `PHA`/`PLA` around the call rather than a reload, which would need an `X`
/// the recorder has already spent.
const TOAD_GRANT: usize = 0x3B1C1;
const TOAD_GRANT_VANILLA: [u8; 3] = [0xBD, 0x3B, 0xD1];
const FS_TOAD_HOOK: usize = 0x3B81A;
const TOAD_HOOK_CPU: u16 = 0xD80A;

#[rustfmt::skip]
const TOAD_HOOK: [u8; 9] = [
    0xBD, 0x3B, 0xD1,                                      // 0: LDA Item2Inventory,X (displaced)
    0x48,                                                  // 3: PHA
    0x20, RECORD_CPU as u8, (RECORD_CPU >> 8) as u8,       // 4: JSR RECORD
    0x68,                                                  // 7: PLA
    0x60,                                                  // 8: RTS
];

fn splice(rom: &mut Rom, site: usize, vanilla: &[u8], patch: &[u8]) {
    assert_eq!(rom.read_range(site, vanilla.len()), vanilla, "site {site:#07X} is not vanilla");
    rom.write_range(site, patch);
}

/// Install the recorder and every hook that feeds it.
fn install_found_recorder(rom: &mut Rom) {
    rom.push_tag("item_keys/found");
    rom.write_range(FS_RECORD, &RECORD);

    rom.write_range(FS_GETITEM_TAIL, &GETITEM_TAIL_HOOK);
    splice(
        rom,
        GETITEM_TAIL,
        &GETITEM_TAIL_VANILLA,
        &[0x4C, GETITEM_TAIL_CPU as u8, (GETITEM_TAIL_CPU >> 8) as u8, 0xEA],
    );

    rom.write_range(FS_LETTER_HOOK, &LETTER_HOOK);
    splice(
        rom,
        LETTER_SITE,
        &LETTER_SITE_VANILLA,
        &[0x20, LETTER_HOOK_CPU as u8, (LETTER_HOOK_CPU >> 8) as u8, 0xEA, 0xEA],
    );

    rom.write_range(FS_TOAD_HOOK, &TOAD_HOOK);
    splice(
        rom,
        TOAD_GRANT,
        &TOAD_GRANT_VANILLA,
        &[0x20, TOAD_HOOK_CPU as u8, (TOAD_HOOK_CPU >> 8) as u8],
    );
    rom.pop_tag();
}

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

    /// Every grant site calls the recorder and keeps its own behaviour.
    #[test]
    fn all_three_sources_are_hooked() {
        let Some(mut rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        apply(&mut rom, false);
        let jsr = |cpu: u16| [0x20, cpu as u8, (cpu >> 8) as u8];

        // Toad House: JSR, and the caller's TAX / INX / RTS still follows.
        assert_eq!(rom.read_range(TOAD_GRANT, 3), &jsr(TOAD_HOOK_CPU)[..]);
        assert_eq!(rom.read_range(TOAD_GRANT + 3, 3), &[0xAA, 0xE8, 0x60]);
        // Player_GetItem's tail: JMP, since the hook ends the routine.
        assert_eq!(
            rom.read_range(GETITEM_TAIL, 4),
            &[0x4C, GETITEM_TAIL_CPU as u8, (GETITEM_TAIL_CPU >> 8) as u8, 0xEA]
        );
        // The letter: JSR plus the two NOPs the five-byte site leaves over.
        assert_eq!(
            rom.read_range(LETTER_SITE, 5),
            &[0x20, LETTER_HOOK_CPU as u8, (LETTER_HOOK_CPU >> 8) as u8, 0xEA, 0xEA]
        );
        assert_eq!(rom.read_range(FS_RECORD, RECORD.len()), &RECORD[..]);
    }

    /// Every hook reaches the *same* recorder. Three copies of the item-id
    /// mapping is how a leaf gets recorded into the mushroom's row.
    #[test]
    fn one_recorder_serves_every_hook() {
        let call = [0x20, RECORD_CPU as u8, (RECORD_CPU >> 8) as u8];
        for (name, hook) in [
            ("toad", &TOAD_HOOK[..]),
            ("get_item", &GETITEM_TAIL_HOOK[..]),
            ("letter", &LETTER_HOOK[..]),
        ] {
            assert!(
                hook.windows(3).any(|w| w == call),
                "{name} hook does not call the shared recorder"
            );
        }
    }

    /// The recorder's product table agrees with the gate's rows, for every
    /// key and only the keys.
    #[test]
    fn recorder_products_match_the_gate_rows() {
        const PROD: usize = 21;
        for key in [Key::Mushroom, Key::Flower, Key::Leaf, Key::Star] {
            assert_eq!(RECORD[PROD + key.row()], key.product(), "product for {key:?}");
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

    /// The shared recorder: decodes, ends in `RTS`, both branches land on
    /// instruction boundaries, and `LDA PROD,X` resolves into its own tail.
    #[test]
    fn recorder_is_well_formed() {
        asm::check(&RECORD).origin(RECORD_CPU).data_from(21).assert_ok();
    }

    /// Each hook displaces whole instructions and names its own origin.
    #[test]
    fn hooks_are_well_formed() {
        asm::check(&TOAD_HOOK)
            .origin(TOAD_HOOK_CPU)
            .hook(&TOAD_GRANT_VANILLA, 0, &[0x20, TOAD_HOOK_CPU as u8, (TOAD_HOOK_CPU >> 8) as u8])
            .assert_ok();
        asm::check(&GETITEM_TAIL_HOOK)
            .origin(GETITEM_TAIL_CPU)
            .hook(
                &GETITEM_TAIL_VANILLA,
                0,
                &[0x4C, GETITEM_TAIL_CPU as u8, (GETITEM_TAIL_CPU >> 8) as u8, 0xEA],
            )
            .assert_ok();
        asm::check(&LETTER_HOOK)
            .origin(LETTER_HOOK_CPU)
            .hook(
                &LETTER_SITE_VANILLA,
                0,
                &[0x20, LETTER_HOOK_CPU as u8, (LETTER_HOOK_CPU >> 8) as u8, 0xEA, 0xEA],
            )
            .assert_ok();
    }
}
