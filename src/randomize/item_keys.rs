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
//! `LATP_Flower` and `LATP_Leaf` are each referenced from exactly one place —
//! their own word in `LATP_JumpTable`. Repointing those two words frees both
//! bodies outright, and they are adjacent: 28 contiguous bytes at CPU `$B7EC`,
//! in the dispatcher's own bank, so there is no mapping question and none of
//! the nearly-full always-mapped banks are spent. PRG008 has **zero** bytes of
//! `$FF` filler, so reclaiming is not merely the cheapest option here, it is
//! the only one.
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
/// Offset of the products table inside the routine.
const PRODUCTS_OFFSET: usize = 24;
const PRODUCTS_CPU: u16 = GATE_CPU + PRODUCTS_OFFSET as u16;

/// A key. The value is the `Bouncer_PUp` index the block returns when the item
/// is found — `prg001.asm:1015`: `$00, $00, FIREFLOWER, SUPERLEAF, STARMAN,
/// MUSHROOM, GROWINGVINE, 1UP`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Mushroom,
    Flower,
    Leaf,
    Star,
}

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

    pub fn parse(name: &str) -> Option<Key> {
        match name.trim().to_ascii_lowercase().as_str() {
            "mushroom" | "big" => Some(Key::Mushroom),
            "flower" | "fire" => Some(Key::Flower),
            "leaf" => Some(Key::Leaf),
            "star" | "starman" => Some(Key::Star),
            _ => None,
        }
    }
}

/// The routine, with its products table left blank for [`gate_bytes`].
#[rustfmt::skip]
const GATE: [u8; 27] = [
    0xA9, 0x00,                                              //  0: LDA #$00
    0x8D, 0x86, 0x05,                                        //  2: STA PUp_StarManFlash
    0x98,                                                    //  5: TYA        ; type * 2
    0x4A,                                                    //  6: LSR A      ; -> row
    0xA4, 0xED,                                              //  7: LDY Player_Suit
    0xD0, 0x02,                                              //  9: BNE big
    0xA9, 0x00,                                              // 11: LDA #$00   ; small -> row 0
    0xA8,                                                    // 13: big: TAY
    0xB9, PRODUCTS_CPU as u8, (PRODUCTS_CPU >> 8) as u8,     // 14: LDA PRODUCTS,Y
    0xF0, 0x02,                                              // 17: BEQ locked
    0xA8,                                                    // 19: TAY
    0x60,                                                    // 20: RTS
    0x4C, LATP_COIN_CPU as u8, (LATP_COIN_CPU >> 8) as u8,   // 21: locked: JMP LATP_Coin
    0x00, 0x00, 0x00,                                        // 24: PRODUCTS: mushroom, flower, leaf
];

/// The routine with its products table filled in for `found`. A row of zero is
/// a locked item, and zero is also what the routine tests, which is the whole
/// trick — see the module docs.
fn gate_bytes(found: &[Key]) -> [u8; 27] {
    let mut out = GATE;
    for (row, key) in [Key::Mushroom, Key::Flower, Key::Leaf].into_iter().enumerate() {
        out[PRODUCTS_OFFSET + row] = if found.contains(&key) { key.product() } else { 0 };
    }
    out
}

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

/// Install the dispenser gate. Only the items in `found` are dispensed; every
/// other power-up block pays a coin.
pub fn apply_poc(rom: &mut Rom, found: &[Key]) {
    // Refuses to run after `qol::apply_modern_powerups`, which rewrites the two
    // `LDY #$05` operands inside these very handlers (0x11802, 0x11810) so a
    // small player is powered up directly. That deletes the mushroom rung this
    // gate turns into a key, and whichever pass ran second would silently win.
    //
    // **The combination is wanted, not forbidden** — it is the mode's easier
    // arm, where the mushroom stops being a gate (see the allocation section
    // of `docs/item_keys_design.md`). It needs a 21-byte variant of this
    // routine with the `Player_Suit` test dropped, because under that patch
    // both paths return the same product. Until that exists, refusing is the
    // honest answer; silently producing a gate the player cannot open is not.
    assert_eq!(
        rom.read_byte(0x11802),
        0x05,
        "LATP_Flower's small product is not vanilla's mushroom — \
         Modern Power-ups is already installed, and the two cannot both be on"
    );
    rom.push_tag("item_keys/gate");
    rom.write_range(FS_ITEM_GATE, &gate_bytes(found));
    repoint(rom, 1, LATP_FLOWER_CPU, GATE_CPU);
    repoint(rom, 2, LATP_LEAF_CPU, GATE_CPU);
    if !found.contains(&Key::Star) {
        repoint(rom, 3, LATP_STAR_CPU, LATP_COIN_CPU);
    }
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

    /// The table is where this module says it is. Without this every write
    /// here lands in the middle of something else.
    #[test]
    fn jump_table_is_where_we_think() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        assert_eq!(word(&rom, 0), 0xB7E9, "entry 0 = LATP_None");
        assert_eq!(word(&rom, 1), LATP_FLOWER_CPU);
        assert_eq!(word(&rom, 2), LATP_LEAF_CPU);
        assert_eq!(word(&rom, 3), LATP_STAR_CPU);
        assert_eq!(word(&rom, 4), LATP_COIN_CPU);
        assert_eq!(FS_ITEM_GATE, 0x117FC, "the routine goes where LATP_Flower was");
    }

    /// A found item returns its product; an unfound one returns zero, which is
    /// what the routine branches on.
    #[test]
    fn products_table_encodes_the_found_set() {
        let none = gate_bytes(&[]);
        assert_eq!(&none[PRODUCTS_OFFSET..], &[0, 0, 0]);

        let both = gate_bytes(&[Key::Mushroom, Key::Leaf]);
        assert_eq!(&both[PRODUCTS_OFFSET..], &[5, 0, 3]);

        // The star has no row: it is all-or-nothing at the jump table.
        let star = gate_bytes(&[Key::Star]);
        assert_eq!(&star[PRODUCTS_OFFSET..], &[0, 0, 0]);
    }

    /// The star entry is repointed only when the star is locked, and the
    /// `LATP_Star` body survives either way — the found case still runs it.
    #[test]
    fn star_is_all_or_nothing() {
        let Some(mut locked) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let mut found = locked.clone();
        let body = locked.read_range(0x11818, 8).to_vec();

        apply_poc(&mut locked, &[]);
        assert_eq!(word(&locked, 3), LATP_COIN_CPU);
        assert_eq!(locked.read_range(0x11818, 8), &body[..]);

        apply_poc(&mut found, &[Key::Star]);
        assert_eq!(word(&found, 3), LATP_STAR_CPU);
    }

    /// The two power-up entries point at the routine, and no other vector
    /// moves.
    #[test]
    fn only_the_gated_entries_are_repointed() {
        let Some(mut rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let before: Vec<u16> = (0..12).map(|n| word(&rom, n)).collect();
        apply_poc(&mut rom, &[Key::Star]);
        for (n, &was) in before.iter().enumerate() {
            let expected = match n {
                1 | 2 => GATE_CPU,
                _ => was,
            };
            assert_eq!(word(&rom, n), expected, "entry {n}");
        }
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
        asm::check(&gate_bytes(&[Key::Mushroom, Key::Flower, Key::Leaf]))
            .allocation(FS_ITEM_GATE)
            .origin(GATE_CPU)
            .data_from(PRODUCTS_OFFSET)
            .assert_ok();
    }
}
