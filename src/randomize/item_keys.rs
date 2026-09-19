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
}

impl Key {
    /// The `Bouncer_PUp` index this key's block dispenses once found.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) const fn product(self) -> u8 {
        match self {
            Key::Mushroom => 5,
            Key::Flower => 2,
            Key::Leaf => 3,
            Key::Star => 4,
        }
    }

    /// The routine's row for this key. Rows 1-3 are the block type the
    /// dispatcher already has in `Y`; row 0 is where a small player is sent.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) const fn row(self) -> usize {
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

/// Where "the player has held an anchor" lives.
const ANCHOR: u16 = crate::randomize::maze_state::FOUND_ANCHOR;

/// The Toad House hook, which also records the **anchor** — because that is
/// the only place one can come from.
///
/// Not a fifth row in [`RECORD`], and not because of taste: `ItemOff[9]` is
/// `17`, which indexes *past* the 15-byte `ToadHouse_Item2Inventory` into
/// `ToadHouse_ItemOff` itself, whose bytes 2-4 happen to be `0A 0A 0A`. So
/// that Toad House type hands out an anchor on all three of its random
/// outcomes, and no other table in the ROM contains `$0A` — not the Hammer
/// Bro rewards, not the Princess letters, not the in-level chests, and not
/// the pools `items::randomize` deals from. One source, so one place to
/// record it, and `RECORD` stays at 25 bytes inside PRG031's 30-byte gap.
///
/// `A` survives: `STA` does not touch it, so the caller's `TAX / INX / RTS`
/// still sees the item id. Storing `$0A` itself is the flag — any non-zero
/// value means found.
#[rustfmt::skip]
const TOAD_HOOK: [u8; 16] = [
    0xBD, 0x3B, 0xD1,                                      //  0: LDA Item2Inventory,X (displaced)
    0x48,                                                  //  3: PHA
    0x20, RECORD_CPU as u8, (RECORD_CPU >> 8) as u8,       //  4: JSR RECORD
    0x68,                                                  //  7: PLA        ; A = the item id again
    0xC9, 0x0A,                                            //  8: CMP #$0A   ; the anchor
    0xD0, 0x03,                                            // 10: BNE out
    0x8D, ANCHOR as u8, (ANCHOR >> 8) as u8,               // 12: STA FOUND_ANCHOR
    0x60,                                                  // 15: out: RTS
];

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
#[rustfmt::skip]
const SUMMON_GATE: [u8; 13] = [
    0xAD, ANCHOR as u8, (ANCHOR >> 8) as u8,           //  0: LDA FOUND_ANCHOR
    0xD0, 0x05,                                        //  3: BNE summon
    0xA5, 0xE5,                                        //  5: LDA World_Map_Tile
    0xA0, 0x1A,                                        //  7: LDY #$1A
    0x60,                                              //  9: RTS
    0x4C, SUMMON_CPU as u8, (SUMMON_CPU >> 8) as u8,   // 10: summon: JMP the real routine
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
