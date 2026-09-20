//! Item keys — a power-up block only dispenses what the player has found.
//!
//! The ROM half of `docs/mimaze_layer_design.md`, which turns on one chain: a
//! level tile is a wall until it is beaten; a level that requires a power-up
//! carries its own renewable source of it (`vision.md`'s fourth charter
//! point); so **gating that source turns the level into a wall.** This is the
//! gating.
//!
//! **Reached only from `testrom`, and it must stay that way until the MiMaze
//! layer exists — a seed built with this today can be unwinnable.** The found
//! table starts empty, so the player is permanently small until a mushroom is
//! found, and `powerups.rs`' `PROTECTED_OFFSETS` records that 8-F requires
//! being big to break a block in sub-area 2. Nothing here guarantees a
//! mushroom source is reachable before a fortress that needs one. Producing
//! that guarantee is exactly the job of the item-aware fixpoint in
//! `docs/mimaze_layer_design.md`, and the reason this has no option, no flag
//! key bit and no web control yet.
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

/// A key: an item whose blocks stay shut until the player has found one.
///
/// Four, because those are the products the gated handlers dispense. The
/// enum is the domain statement the ROM tables are checked against — the
/// recorder's `PROD` table and the gate's rows are the things that actually
/// run, and a test cross-checks them against this rather than against each
/// other.
///
/// The model side uses it too: `maze::ItemGate` and `maze::ItemSource` are
/// keyed on it, so "what a block dispenses" and "what a gate demands" are the
/// same vocabulary and cannot drift into two spellings of the same item.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Key {
    Mushroom,
    Flower,
    Leaf,
    Star,
    /// The canoe key. **Not a block key** — no `?` block dispenses an anchor,
    /// so it has no row in the products table and no `Bouncer_PUp` index. It
    /// lives in its own SRAM byte and is read by the summon gate, not by the
    /// dispenser. See [`FOUND_ANCHOR`](crate::randomize::maze_state::FOUND_ANCHOR).
    Anchor,
}

impl Key {
    /// The `Bouncer_PUp` index this key's block dispenses once found, or
    /// `None` for a key no block dispenses.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) const fn product(self) -> Option<u8> {
        match self {
            Key::Mushroom => Some(5),
            Key::Flower => Some(2),
            Key::Leaf => Some(3),
            Key::Star => Some(4),
            // Not a `Bouncer_PUp` index — nothing dispenses an anchor. Just a
            // non-zero flag in its row, which the canoe stub reads and the
            // gate never indexes.
            Key::Anchor => Some(1),
        }
    }

    /// The routine's row for this key. Rows 1-3 are the block type the
    /// dispatcher already has in `Y`; row 0 is where a small player is sent.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) const fn row(self) -> Option<usize> {
        match self {
            Key::Mushroom => Some(0),
            Key::Flower => Some(1),
            Key::Leaf => Some(2),
            Key::Star => Some(3),
            Key::Anchor => Some(4),
        }
    }
}

/// Where the found table lives: [`maze_state::FOUND_PRODUCTS`], four bytes of
/// SRAM. Row 0 mushroom, 1 fire flower, 2 super leaf, 3 starman — rows 1-3 are
/// the block type the dispatcher already has in `Y`.
const PRODUCTS: u16 = crate::randomize::maze_state::FOUND_PRODUCTS;

/// The gate, covering all three power-up entries.
///
/// ```text
///   TYA / LSR A          Y is type*2 on entry, so A is the row: 1 flower,
///                        2 leaf, 3 star
///   LDY Player_Suit      and Y == 0 *is* the mushroom row
///   BNE have             big: the row is the block type
///   CMP #$03 / BNE small a star ignores the suit, as vanilla does
/// have:  TAY
/// small: LDA #$00 / CPY #$03 / ROR A / STA PUp_StarManFlash
///   LDA FOUND_PRODUCTS,Y SRAM; zero means locked
///   BEQ locked
///   TAY / RTS            Y = the Bouncer_PUp index
/// locked: JMP LATP_Coin
/// ```
///
/// 29 bytes of the 36 reclaimed by repointing all three jump-table words.
/// Two tricks pay for the small-player case and the starman flash:
///
/// **The suit load doubles as the mushroom row.** `Player_Suit` is zero
/// exactly when the answer is row 0, so loading it into `Y` rather than `A`
/// means the small path needs no `LDA #$00` at all — it just declines to
/// overwrite `Y`. The `BNE small` skips precisely the one-byte `TAY`.
///
/// **The flash value is derived, not tabled.** `CPY #$03` sets carry iff the
/// row is the star's, and `ROR` rotates carry into bit 7 — so `$80` for a
/// star and `$00` for everything else costs three bytes instead of a
/// four-byte table. `PUp_StarManFlash` is only ever tested for bit 7 or for
/// zero (`prg001` tests it eight ways, all `BPL`/`BEQ`/`BNE`/`AND #$03`), so
/// those two values are its whole contract.
///
/// Losing the table also loses the routine's only absolute self-reference,
/// which is why this no longer needs `.origin()` and could be relocated.
#[rustfmt::skip]
const GATE: [u8; 29] = [
    0x98,                                              //  0: TYA
    0x4A,                                              //  1: LSR A
    0xA4, 0xED,                                        //  2: LDY Player_Suit
    0xD0, 0x04,                                        //  4: BNE have
    0xC9, 0x03,                                        //  6: CMP #$03      ; star?
    0xD0, 0x01,                                        //  8: BNE small
    0xA8,                                              // 10: have: TAY
    0xA9, 0x00,                                        // 11: small: LDA #$00
    0xC0, 0x03,                                        // 13: CPY #$03      ; carry iff star
    0x6A,                                              // 15: ROR A         ; -> $80 or $00
    0x8D, 0x86, 0x05,                                  // 16: STA PUp_StarManFlash
    0xB9, PRODUCTS as u8, (PRODUCTS >> 8) as u8,       // 19: LDA FOUND_PRODUCTS,Y
    0xF0, 0x02,                                        // 22: BEQ locked
    0xA8,                                              // 24: TAY
    0x60,                                              // 25: RTS
    0x4C, LATP_COIN_CPU as u8, (LATP_COIN_CPU >> 8) as u8, // 26: locked: JMP LATP_Coin
];

/// Offset of the `LDY Player_Suit` operand pair, which is the whole of the
/// difference between the two arms — see [`gate_bytes`].
const SUIT_TEST_AT: usize = 2;

/// The gate for one arm or the other.
///
/// The **easier arm** — `qol::apply_modern_powerups`, which hands a *small*
/// player the suit directly — is the same routine with `LDY Player_Suit`
/// replaced by `LDY #$01`. `Y` is then always non-zero, `BNE have` always
/// fires, the row is always the block type, and row 0 is never read. Which
/// is exactly right: under that patch the mushroom is not a rung to gate, so
/// there is nothing for the small path to do.
///
/// Two bytes, rather than a second 23-byte routine kept in step with this
/// one by hand.
fn gate_bytes(easier: bool) -> [u8; 29] {
    let mut out = GATE;
    if easier {
        out[SUIT_TEST_AT] = 0xA0; // LDY #imm
        out[SUIT_TEST_AT + 1] = 0x01;
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
    rom.write_range(FS_ITEM_GATE, &gate_bytes(easier));
    repoint(rom, 1, LATP_FLOWER_CPU, GATE_CPU);
    repoint(rom, 2, LATP_LEAF_CPU, GATE_CPU);
    repoint(rom, 3, LATP_STAR_CPU, GATE_CPU);
    rom.pop_tag();

    install_found_recorder(rom);
    gate_the_summon(rom);
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
/// Both id ranges are linear — `1,2,3 -> id-1` and `9,$0A -> id-6` — so the
/// whole mapping is two subtracts and two bound checks, with no table of ids
/// at all.
///
/// Two things make it this short. `SBC #$00` is a two-byte *conditional*
/// decrement: reached with carry clear (the `BCC` was taken) it subtracts
/// one, reached with carry set it leaves `A` alone. And an out-of-range high
/// id survives its own check only to fall **into** the low path with carry
/// set, where that `SBC` is a no-op and the low bound rejects it — so one
/// check does double duty and "not a key" costs no test of its own. Id 0
/// wraps to `$FF` and is rejected the same way.
///
/// Clobbers `A` and `X`; **preserves `Y`**, which every caller needs — two of
/// them are holding an inventory offset or a world number across the call.
#[rustfmt::skip]
const RECORD: [u8; 24] = [
    0xC9, 0x09,                                    //  0: CMP #$09
    0x90, 0x06,                                    //  2: BCC low        ; ids 1-3 ... carry CLEAR
    0xE9, 0x06,                                    //  4: SBC #$06       ; carry set: 9->3, $0A->4
    0xC9, 0x05,                                    //  6: CMP #$05
    0x90, 0x06,                                    //  8: BCC have
    0xE9, 0x00,                                    // 10: low: SBC #$00  ; C=0 decrements, C=1 does not
    0xC9, 0x03,                                    // 12: CMP #$03
    0xB0, 0x07,                                    // 14: BCS done
    0xAA,                                          // 16: have: TAX
    0xBD, PROD_CPU as u8, (PROD_CPU >> 8) as u8,   // 17: LDA PROD,X
    0x9D, PRODUCTS as u8, (PRODUCTS >> 8) as u8,   // 20: STA FOUND_PRODUCTS,X
    0x60,                                          // 23: done: RTS
];

/// The value written per row. Rows 0-3 are `Bouncer_PUp` indices the gate
/// reads back; **row 4 is the anchor and is only ever a flag** — the gate
/// indexes rows 0-3 only, and `FOUND_PRODUCTS + 4` is the byte the canoe
/// stub reads, so one `STA` serves both kinds.
///
/// In its own gap because the routine and this table together are 34 bytes
/// and the run at [`FS_RECORD`] holds 30.
#[rustfmt::skip]
const RECORD_PROD: [u8; 5] = [0x05, 0x02, 0x03, 0x04, 0x01];

const FS_RECORD: usize = 0x3E972;
const RECORD_CPU: u16 = 0xE962;
/// Sits in the same gap, immediately after the routine — 24 + 5 fits the
/// 30 bytes, which is why this needs no allocation of its own.
const PROD_CPU: u16 = RECORD_CPU + 24;

/// `Player_GetItem`'s tail — `PLA / STA Inventory_Items,Y / RTS`. Hooked at
/// the `STA` rather than the entry because `A` is the item there and nothing
/// has to be saved around the call.
const GETITEM_TAIL: usize = 0x3FD90;
const GETITEM_TAIL_VANILLA: [u8; 4] = [0x99, 0x80, 0x7D, 0x60];
const FS_GETITEM_TAIL: usize = 0x3E2C6;
const GETITEM_TAIL_CPU: u16 = 0xE2B6;

#[rustfmt::skip]
const GETITEM_TAIL_HOOK: [u8; 6] = [
    0x99, 0x80, 0x7D,                                      // 0: STA Inventory_Items,Y (displaced)
    0x4C, RECORD_CPU as u8, (RECORD_CPU >> 8) as u8,       // 3: JMP RECORD  ; its RTS is ours
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

/// Where "the player has held an anchor" lives.
const ANCHOR: u16 = crate::randomize::maze_state::FOUND_ANCHOR;

/// The Toad House hook.
///
/// It used to record the anchor itself, on the grounds that a Toad House was
/// the only place one could come from — `ItemOff[9]` is 17, which indexes
/// past the 15-byte `ToadHouse_Item2Inventory` into `ToadHouse_ItemOff`,
/// whose bytes 2-4 are `0A 0A 0A`. True of vanilla's tables, and then
/// `item_layer` began writing anchors into Hammer Bro rewards, which arrive
/// through `Player_GetItem` instead. A playtest found the hole: the anchor
/// was collected and nothing was recorded.
///
/// So the anchor is [`RECORD`]'s row 4 now, and every grant site gets it for
/// free. This hook is back to what it always should have been — the
/// displaced load, the call, and `A` restored for the caller's
/// `TAX / INX / RTS`.
#[rustfmt::skip]
const TOAD_HOOK: [u8; 9] = [
    0xBD, 0x3B, 0xD1,                                      // 0: LDA Item2Inventory,X (displaced)
    0x48,                                                  // 3: PHA
    0x20, RECORD_CPU as u8, (RECORD_CPU >> 8) as u8,       // 4: JSR RECORD
    0x68,                                                  // 7: PLA        ; A = the item id again
    0x60,                                                  // 8: RTS
];

// --- What a level demands ---------------------------------------------

/// A level that cannot be beaten without an item, and which items.
///
/// Keyed by vanilla `(world_idx, entry_idx)` the way
/// [`rom_data::CHEST_LEVELS`](crate::randomize::rom_data) is, because that is
/// the identity `overworld_writer::assign_pool` already reasons about when it
/// deals a level onto a slot.
// Reason: `items` and `is_fortress` are the placement pass's half of this
// table — it reads them to decide what a gate demands and which pool the
// level comes from. The writer only needs the identity, so until that pass
// lands the fields are written and checked but not read outside tests.
#[allow(dead_code)]
pub(crate) struct LevelRequirement {
    pub world_idx: usize,
    pub entry_idx: usize,
    /// Every item needed, **all of them**. A suit is always a conjunction
    /// with the mushroom: the gate routine sends a *small* player to the
    /// mushroom row whichever block they bump, so without one the suit behind
    /// it is unreachable even when its own key has been found.
    pub items: &'static [Key],
    /// A fortress rather than a level. It is dealt from a different pool, and
    /// it holds a lock key — gating one is a larger claim than gating a level.
    pub is_fortress: bool,
}

/// Every level known to require an item to finish.
///
/// **Four exist; three are here.** These are the levels `powerups.rs` already
/// protects from the power-up roll, because breaking them was the failure
/// that list exists to prevent — the protection list and the gate list are
/// the same list, which is what makes these authored rather than guessed.
///
/// **Every row here must be listed, even a half-enforced one.** The dispenser
/// gate is global: with item keys on, *every* power-up block in the game is
/// gated, so these levels are unbeatable without their item **wherever they
/// land** — not only where the layer chooses to mark them. A requirement left
/// out of this table is a gate the model cannot see, which is how a seed
/// strands a player.
///
/// That is why 7-F1 is here with **only the mushroom**. Its real requirement
/// is mushroom + tanooki, but the tanooki comes from a Big [?] — an object in
/// PRG005 this module does not gate — so the ROM hands one over regardless of
/// what has been found. The mushroom half *is* enforced, and modelling
/// exactly the enforced half is what keeps the model honest: claim the
/// tanooki too and the model is stricter than the game (harmless), omit the
/// row and the model is looser (not harmless). The tanooki joins it with the
/// Big [?] work.
pub(crate) const LEVEL_REQUIREMENTS: &[LevelRequirement] = &[
    // 6-5, the ice level: flight, from its own single Q-leaf (`0x22D74`).
    LevelRequirement {
        world_idx: 5,
        entry_idx: 39,
        items: &[Key::Mushroom, Key::Leaf],
        is_fortress: false,
    },
    // 7-7, the muncher fields: four Q-stars. The one row that is not a
    // conjunction, because `LATP_Star` has no `Player_Suit` split — a starman
    // goes to a small player.
    LevelRequirement { world_idx: 6, entry_idx: 25, items: &[Key::Star], is_fortress: false },
    // 7-F1: big enough to break the bricks that reach its Big [?]. The Big [?]
    // itself is ungated, so the tanooki is not part of what the ROM enforces
    // today — see the note above.
    LevelRequirement { world_idx: 6, entry_idx: 5, items: &[Key::Mushroom], is_fortress: true },
    // 8-F: big enough to break a block in sub-area 2 (`0x2B900`). The catalog
    // calls it `8F1`, which is also in `FRIENDLIER_BLOCKED_FORTS` — so with
    // **Friendlier Levels on this row cannot be satisfied**, the deck never
    // holds it, and the writer reports the mark unmet. That is the intended
    // failure (a gate that opens for free rather than a broken seed), but it
    // means the only fortress requirement in the table disappears under a
    // popular option.
    LevelRequirement { world_idx: 7, entry_idx: 26, items: &[Key::Mushroom], is_fortress: true },
];

/// The requirement for a vanilla `(world_idx, entry_idx)`, if it has one.
// Reason: the placement pass's lookup, and the model's — a gate resolves its
// items through here. Unused until that pass lands; tests cover it meanwhile.
#[allow(dead_code)]
pub(crate) fn requirement_of(
    world_idx: usize,
    entry_idx: usize,
) -> Option<&'static LevelRequirement> {
    LEVEL_REQUIREMENTS.iter().find(|r| r.world_idx == world_idx && r.entry_idx == entry_idx)
}

// --- The canoe ------------------------------------------------------

/// `MAPOBJ_CANOE`, the map-object id the summon routine scans for.
const MAPOBJ_CANOE: u8 = 0x10;

/// Push every canoe one tile further from its dock.
///
/// Boarding is walking into the boat from the dock, so a canoe parked on the
/// water cell *adjacent* to a dock is boardable on foot and the anchor gates
/// nothing. One tile further out it is visible and unreachable — which is the
/// point: seeing the thing you cannot reach is what sends a player looking
/// for the anchor, where an empty dock teaches nothing.
///
/// Both destinations are water and were checked: W3's boat sits at (6,21)
/// beside the dock at (6,20) with (6,22) also water (`$8D`), and under `8s
/// are Wild` W8's sits at (5,7) beside (5,6) with (5,8) water (`$8D`, written
/// by that option's own tile edits).
///
/// **This alone changes nothing a player can feel**, because
/// `qol::canoe_summon` is always on: wherever the boat is parked, A on a dock
/// calls it. It only becomes a gate once [`gate_the_summon`] runs.
///
/// Scans for the id rather than naming slots, so it covers W3's vanilla slot
/// and the one `8s are Wild` adds without depending on which ran first.
fn move_canoes_offshore(rom: &mut Rom) {
    use crate::randomize::rom_data::{
        MAP_OBJ_IDS_MASTER, MAP_OBJ_XHIS_MASTER, MAP_OBJ_XLOS_MASTER, map_obj_slot_offset,
    };
    for world in 0..8 {
        for slot in 0..9 {
            let id = rom.read_byte(map_obj_slot_offset(rom, MAP_OBJ_IDS_MASTER, world, slot));
            if id != MAPOBJ_CANOE {
                continue;
            }
            // One grid column is 16 units of XLo; past the last column of a
            // screen it carries into XHi. Done on the bytes rather than by
            // decoding to (row, col) and back, so the row is untouched.
            let xlo_off = map_obj_slot_offset(rom, MAP_OBJ_XLOS_MASTER, world, slot);
            let xhi_off = map_obj_slot_offset(rom, MAP_OBJ_XHIS_MASTER, world, slot);
            let xlo = rom.read_byte(xlo_off);
            if xlo >= 0xF0 {
                rom.write_byte(xlo_off, 0x00);
                rom.write_byte(xhi_off, rom.read_byte(xhi_off) + 1);
            } else {
                rom.write_byte(xlo_off, xlo + 0x10);
            }
        }
    }
}

/// `qol::canoe_summon`'s hook site — the `JSR` that replaced
/// `LDA World_Map_Tile / LDY #$1A` at CPU `$CEC5`, reached only when A was
/// just pressed.
const SUMMON_HOOK: usize = 0x14ED5;
/// What stands there before `canoe_summon` is installed.
const SUMMON_HOOK_VANILLA: [u8; 4] = [0xA5, 0xE5, 0xA0, 0x1A];
/// The summon routine's own address (`FS_CANOE_SUMMON`, origin-locked).
const SUMMON_CPU: u16 = 0xDEA5;

/// Where the gate stub goes: PRG010, CPU `$DDC0`, the `$FF` run that ends
/// where `fix_canoe_softlock`'s respawn routine begins.
const FS_SUMMON_GATE: usize = 0x15DD0;
const SUMMON_GATE_CPU: u16 = 0xDDC0;

/// Gate "call the boat" on the anchor.
///
/// **A stub, not an edit to the summon.** `FS_CANOE_SUMMON` is 151 bytes and
/// origin-locked — self-referential `JMP` plus table reads — so inserting a
/// test at its head would shift every internal absolute reference. Repointing
/// the hook at 13 bytes of our own costs less and moves nothing.
///
/// The stub reproduces the summon's own passthrough on the locked path:
/// every exit from that routine restores `A = World_Map_Tile` and `Y = $1A`
/// so the displaced special-enter-tile scan continues as in vanilla, and so
/// does this.
/// The summon's own passthrough exit: `LDA World_Map_Tile / LDY #$1A / RTS`
/// at `FS_CANOE_SUMMON + 18`, which every one of its exits falls to. Jumping
/// there beats copying those five bytes into the stub.
const SUMMON_PASSTHROUGH_CPU: u16 = SUMMON_CPU + 18;

#[rustfmt::skip]
const SUMMON_GATE: [u8; 11] = [
    0xAD, ANCHOR as u8, (ANCHOR >> 8) as u8,           // 0: LDA FOUND_ANCHOR
    0xD0, 0x03,                                        // 3: BNE summon
    0x4C, SUMMON_PASSTHROUGH_CPU as u8, (SUMMON_PASSTHROUGH_CPU >> 8) as u8, // 5: JMP csexit
    0x4C, SUMMON_CPU as u8, (SUMMON_CPU >> 8) as u8,   // 8: summon: JMP the real routine
];

fn gate_the_summon(rom: &mut Rom) {
    let installed = [0x20, SUMMON_CPU as u8, (SUMMON_CPU >> 8) as u8, 0xEA];
    // On a vanilla base the summon is not in yet; a randomized base already
    // has it, since `randomize_inner` applies it unconditionally.
    if rom.read_range(SUMMON_HOOK, 4) == SUMMON_HOOK_VANILLA {
        crate::randomize::qol::apply_canoe_summon(rom);
    }
    assert_eq!(
        rom.read_range(SUMMON_HOOK, 4),
        &installed[..],
        "the canoe summon is not installed, so there is nothing to gate"
    );
    rom.push_tag("item_keys/canoe");
    rom.write_range(FS_SUMMON_GATE, &SUMMON_GATE);
    rom.write_range(
        SUMMON_HOOK,
        &[0x20, SUMMON_GATE_CPU as u8, (SUMMON_GATE_CPU >> 8) as u8, 0xEA],
    );
    move_canoes_offshore(rom);
    rom.pop_tag();
}

fn splice(rom: &mut Rom, site: usize, vanilla: &[u8], patch: &[u8]) {
    assert_eq!(rom.read_range(site, vanilla.len()), vanilla, "site {site:#07X} is not vanilla");
    rom.write_range(site, patch);
}

/// Install the recorder and every hook that feeds it.
fn install_found_recorder(rom: &mut Rom) {
    rom.push_tag("item_keys/found");
    rom.write_range(FS_RECORD, &RECORD);
    rom.write_range(FS_RECORD + RECORD.len(), &RECORD_PROD);

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
    use rand::SeedableRng;

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
        // `JSR` or `JMP` — the get-item hook tail-calls, since the recorder's
        // own `RTS` can return to its caller. What matters is that all three
        // reach the same address.
        let lo = RECORD_CPU as u8;
        let hi = (RECORD_CPU >> 8) as u8;
        for (name, hook) in [
            ("toad", &TOAD_HOOK[..]),
            ("get_item", &GETITEM_TAIL_HOOK[..]),
            ("letter", &LETTER_HOOK[..]),
        ] {
            assert!(
                hook.windows(3).any(|w| (w[0] == 0x20 || w[0] == 0x4C) && w[1] == lo && w[2] == hi),
                "{name} hook does not reach the shared recorder"
            );
        }
    }

    /// **The registry names real levels, of the kind it claims.**
    ///
    /// Entry indices are positions in a 340-entry pointer table; nothing stops
    /// one drifting, and a drifted row would gate a different level entirely —
    /// silently, because the writer would happily deal whatever is there. So
    /// check each row against the catalog: the entry exists, it is a Level or
    /// a Fortress as the row says, and its name is the one the comment claims.
    #[test]
    fn every_requirement_names_the_level_it_says() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let catalog = crate::randomize::node_catalog::NodeCatalog::build(&rom, false);
        let expected = ["6-5", "7-7", "7F1", "8F1"];
        assert_eq!(LEVEL_REQUIREMENTS.len(), expected.len(), "a row was added without a name");

        for (req, want) in LEVEL_REQUIREMENTS.iter().zip(expected) {
            let entry = catalog
                .entries
                .iter()
                .find(|e| e.world_idx == req.world_idx && e.entry_idx == req.entry_idx)
                .unwrap_or_else(|| {
                    panic!("no catalog entry at world {} entry {}", req.world_idx, req.entry_idx)
                });
            assert_eq!(entry.name, want, "row points at the wrong level");
            let is_fort = matches!(entry.kind, crate::randomize::node_catalog::NodeKind::Fortress);
            assert_eq!(is_fort, req.is_fortress, "{want}: wrong kind, so the wrong pool");
            assert!(!req.items.is_empty(), "{want}: a requirement with no items gates nothing");
            assert_eq!(
                requirement_of(req.world_idx, req.entry_idx).map(|r| r.items),
                Some(req.items),
                "{want}: lookup disagrees with the table"
            );
        }
    }

    /// A suit requirement always includes the mushroom — the rule a playtest
    /// found, and the reason a gate carries a set. Only the star is exempt,
    /// because `LATP_Star` has no `Player_Suit` split.
    #[test]
    fn suit_requirements_include_the_mushroom() {
        for req in LEVEL_REQUIREMENTS {
            let suits = req.items.iter().any(|k| matches!(k, Key::Flower | Key::Leaf));
            if suits {
                assert!(
                    req.items.contains(&Key::Mushroom),
                    "a suit requirement without the mushroom is unreachable while small"
                );
            }
        }
    }

    /// **The anchor supply is an out-of-bounds read, and it is load-bearing.**
    ///
    /// `ToadHouse_Item2Inventory` is 15 bytes, but `ToadHouse_ItemOff[9]` is
    /// 17 — so that Toad House type indexes past the table into `ItemOff`
    /// itself, whose bytes 2-4 are `0A 0A 0A`. That is the only place in the
    /// ROM an anchor can come from: no reward table holds $0A, and neither
    /// pool `items::randomize` deals from contains one.
    ///
    /// Accidental, but the canoe key now depends on it, and the 2.0.1 bug
    /// (writing 21 bytes into the 15-byte table) already clobbered exactly
    /// these bytes once. So pin them — before and after randomization, since
    /// `items::randomize` rewrites the declared 15 and must not reach past.
    #[test]
    fn the_anchor_supply_survives() {
        let Some(mut rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        const I2I: usize = 0x3B14B;
        let anchors = |r: &Rom| r.read_range(I2I + 17, 3).to_vec();
        assert_eq!(
            anchors(&rom),
            vec![0x0A; 3],
            "vanilla should reach three anchors past the table"
        );

        let mut rng = rand_chacha::ChaCha8Rng::from_seed([7u8; 32]);
        crate::randomize::items::randomize(&mut rom, &mut rng, true, false);
        assert_eq!(
            anchors(&rom),
            vec![0x0A; 3],
            "randomizing the item tables must not reach past the declared 15 bytes"
        );
    }

    /// The canoe moves one tile further out, in W3 and under 8s-are-Wild in
    /// W8, and both destinations are water.
    #[test]
    fn the_canoe_moves_offshore() {
        let Some(mut rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        use crate::randomize::rom_data::{MAP_OBJ_XLOS_MASTER, map_obj_slot_offset};
        // W3's canoe is slot 4, XLo  -> column 21.
        let xlo = map_obj_slot_offset(&rom, MAP_OBJ_XLOS_MASTER, 2, 4);
        assert_eq!(rom.read_byte(xlo), 0x50, "W3 canoe should start at column 21");
        apply(&mut rom, false);
        assert_eq!(rom.read_byte(xlo), 0x60, "and end at column 22");
    }

    /// The summon is gated through a stub, and the origin-locked routine it
    /// guards is left exactly where it was.
    #[test]
    fn the_summon_is_gated_without_moving_it() {
        let Some(mut rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        apply(&mut rom, false);
        assert_eq!(
            rom.read_range(SUMMON_HOOK, 4),
            &[0x20, SUMMON_GATE_CPU as u8, (SUMMON_GATE_CPU >> 8) as u8, 0xEA],
            "the hook should call the gate, not the summon"
        );
        assert_eq!(rom.read_range(FS_SUMMON_GATE, SUMMON_GATE.len()), &SUMMON_GATE[..]);
        // The summon itself still begins with its own LDA / CMP #B.
        assert_eq!(
            rom.read_range(crate::randomize::rom_data::FS_CANOE_SUMMON, 4),
            &[0xA5, 0xE5, 0xC9, 0x4B],
            "the origin-locked routine must not have moved"
        );
    }

    /// The recorder's product table agrees with the gate's rows, for every
    /// key and only the keys.
    #[test]
    fn recorder_products_match_the_gate_rows() {
        for key in [Key::Mushroom, Key::Flower, Key::Leaf, Key::Star, Key::Anchor] {
            let (row, product) = (key.row().unwrap(), key.product().unwrap());
            assert_eq!(RECORD_PROD[row], product, "product for {key:?}");
        }
    }

    /// The easier arm drops the suit test, so its row is always the block
    /// type and the mushroom row is never read.
    #[test]
    fn the_easier_arm_has_no_suit_test() {
        let (normal, easy) = (gate_bytes(false), gate_bytes(true));
        assert_eq!(&normal[SUIT_TEST_AT..SUIT_TEST_AT + 2], &[0xA4, 0xED], "LDY Player_Suit");
        assert_eq!(&easy[SUIT_TEST_AT..SUIT_TEST_AT + 2], &[0xA0, 0x01], "LDY #$01");
        // and nothing else moves
        for i in (0..normal.len()).filter(|i| !(SUIT_TEST_AT..SUIT_TEST_AT + 2).contains(i)) {
            assert_eq!(normal[i], easy[i], "byte {i} differs");
        }
    }
}

/// Running the recorder, rather than reading its tables.
///
/// `RECORD` is a self-contained calculation — no engine state, no calls out —
/// which CLAUDE.md says is the shape to execute rather than merely decode. So
/// this hands it every Global Item ID in turn on an emulated 2A03 and reads
/// the found table back, which is the only way to be sure the arithmetic
/// (`SBC #$01`, the `CMP #$03` bound, the `$09` special case) maps what it is
/// meant to and *nothing else*.
///
/// It is also the guard for the failure that has now bitten twice: a key no
/// source records. The star was gated with nothing able to grant it, and the
/// anchor nearly went the same way.
#[cfg(test)]
mod execution {
    use super::*;
    use mos6502::cpu::CPU;
    use mos6502::instruction::Ricoh2a03;
    use mos6502::memory::{Bus, Memory};

    const SENTINEL: u16 = 0x0F00;

    /// Run `RECORD` with `A = item`, and return the four product bytes.
    fn record(item: u8) -> [u8; 5] {
        let mut mem = Memory::new();
        mem.set_bytes(RECORD_CPU, &RECORD);
        mem.set_bytes(PROD_CPU, &RECORD_PROD);
        let mut cpu = CPU::new(mem, Ricoh2a03);
        let ret = SENTINEL.wrapping_sub(1);
        cpu.memory.set_byte(0x01FF, (ret >> 8) as u8);
        cpu.memory.set_byte(0x01FE, ret as u8);
        cpu.registers.stack_pointer = mos6502::registers::StackPointer(0xFD);
        cpu.registers.accumulator = item;
        cpu.registers.program_counter = RECORD_CPU;
        for _ in 0..1_000 {
            if cpu.registers.program_counter == SENTINEL {
                break;
            }
            cpu.single_step();
        }
        assert_eq!(cpu.registers.program_counter, SENTINEL, "RECORD ran away on item {item:#04x}");
        [0, 1, 2, 3, 4].map(|r| cpu.memory.get_byte(PRODUCTS + r))
    }

    /// Every key is recorded into its own row, with its own product.
    #[test]
    fn every_key_records_itself() {
        for key in [Key::Mushroom, Key::Flower, Key::Leaf, Key::Star, Key::Anchor] {
            let id = match key {
                Key::Mushroom => 1,
                Key::Flower => 2,
                Key::Leaf => 3,
                Key::Star => 9,
                Key::Anchor => 0x0A,
            };
            let (row, product) = (key.row().unwrap(), key.product().unwrap());
            let table = record(id);
            assert_eq!(
                table[row], product,
                "{key:?} (item {id:#04x}) should land in row {row} as {product:#04x}"
            );
            // and disturbs no other row
            for (r, &v) in table.iter().enumerate() {
                if r != row {
                    assert_eq!(v, 0, "{key:?} also wrote row {r}");
                }
            }
        }
    }

    /// **Nothing else writes anything.** The bound check has to reject item 0
    /// (which wraps to `$FF` under the subtract) and everything from 4 up
    /// except the star — a stray index here would write past the four-byte
    /// table into the rest of the maze's SRAM run.
    #[test]
    fn no_other_item_touches_the_table() {
        for id in 0..=0x20u8 {
            if matches!(id, 1 | 2 | 3 | 9 | 0x0A) {
                continue;
            }
            assert_eq!(record(id), [0; 5], "item {id:#04x} should record nothing");
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
        asm::check(&gate_bytes(false)).allocation(FS_ITEM_GATE).assert_ok();
    }

    /// The easier arm is the same routine with two bytes changed, so it has
    /// to decode too.
    #[test]
    fn easier_gate_is_well_formed() {
        asm::check(&gate_bytes(true)).allocation(FS_ITEM_GATE).assert_ok();
    }

    /// The shared recorder: decodes, ends in `RTS`, both branches land on
    /// instruction boundaries, and `LDA PROD,X` resolves into its own tail.
    #[test]
    fn recorder_is_well_formed() {
        asm::check(&RECORD).origin(RECORD_CPU).assert_ok();
    }

    /// The summon gate decodes, ends in a , and its branch lands on an
    /// instruction boundary.
    #[test]
    fn summon_gate_is_well_formed() {
        asm::check(&SUMMON_GATE)
            .origin(SUMMON_GATE_CPU)
            .hook(
                &[0x20, SUMMON_CPU as u8, (SUMMON_CPU >> 8) as u8, 0xEA],
                0,
                &[0x20, SUMMON_GATE_CPU as u8, (SUMMON_GATE_CPU >> 8) as u8, 0xEA],
            )
            .assert_ok();
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
