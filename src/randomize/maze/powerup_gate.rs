//! The power-up gate: a block hands out a power-up only once the player has
//! unlocked it, and pays a coin until then.
//!
//! The ROM half of the power-up gates. An unlock is a bit in
//! [`POWERUPS_FOUND`], set by beating that power-up's **key level** through an
//! [`on_clear`](crate::randomize::maze::on_clear) row ([`trigger`]). The levels
//! that need a power-up to finish (6-5's leaf, 7-7's stars, 7-F1 and 8-F's
//! mushroom) become gates with no map patch at all: their own blocks refuse
//! until the key is found.
//!
//! Where the gates and their key levels go is
//! [`powerup_placement`](crate::randomize::maze::powerup_placement)'s job: it
//! puts every key where the player reaches it before the gates it opens.
//!
//! # Where it goes
//!
//! `LATP_JumpTable` (PRG008 `$B7D1`) turns a hit block's item code into a
//! handler that returns, in `Y`, the `Bouncer_PUp` index of what to spawn.
//! Codes 1-3 are `LATP_Flower`, `LATP_Leaf` and `LATP_Star`: 36 adjacent bytes
//! at `$B7EC`, each reached only from its own word. Repointing the three words
//! at one routine frees all 36 for it, in the dispatcher's own bank, and the
//! run ends exactly where `LATP_Coin` begins — so "locked" is a branch onto
//! the coin handler rather than a `JMP`.
//!
//! The routine is 34 bytes; the last two of the run are filler.
//!
//! # What each block gives, and which bit it needs
//!
//! | block | small player | big player | bit |
//! |---|---|---|---|
//! | flower (code 1) | mushroom (5) | flower (2) | `$10` / `$02` |
//! | leaf (code 2) | mushroom (5) | leaf (3) | `$10` / `$04` |
//! | star (code 3) | star (4) | star (4) | `$08` |
//!
//! The small player's mushroom is vanilla's own split, kept: a player without
//! the mushroom gets a coin from every flower and leaf block, so every Leaf
//! and Fire Flower gate needs the mushroom too. A star block ignores the
//! player's size, as vanilla's does.
//!
//! **The bit is derived from the index, not looked up.** The routine shifts
//! [`POWERUPS_FOUND`] right `index` times, which leaves bit `index - 1` in the
//! carry. So each power-up's bit is fixed by its `Bouncer_PUp` index
//! ([`PowerUp::bit`]), and there is no table mapping one to the other.
//!
//! # Modern Power-ups
//!
//! That option hands a *small* player the flower or leaf directly. Here that
//! is the same routine with `LDY Player_Suit` replaced by `LDY #$01`: `Y` is
//! never zero, so the routine always takes the big player's row and the
//! mushroom row is never read. Under it the mushroom gates nothing, and a
//! flower or leaf gate needs only its own bit.
//!
//! The option also writes two operands inside vanilla `LATP_Flower` and
//! `LATP_Leaf` (0x11802, 0x11810). The gate replaces those bytes with the rest
//! of the run, so it is installed **after** Modern Power-ups and accepts either
//! version of the run.
//!
//! # Suits
//!
//! Tanooki (`$20`), Frog (`$40`) and Hammer (`$80`) each have their own key,
//! and gate two things:
//!
//! - **Big `[?]` boxes** ([`BIGQ_GATE`], PRG005). A box is strictly its own
//!   suit: a locked one turns into vanilla's 3-Up box and pays three 1-ups.
//!   7-F1's Tanooki box is a gate this way.
//! - **Suit Blocks**, which are gated where the block's hash picks the prize
//!   ([`items::block_spawn`](crate::randomize::items::block_spawn)): the pick
//!   is made from the unlocked ones among star, Tanooki, Frog and Hammer, and
//!   a 1-up when none is. So item code B does not come through here.
//!
//! # Not covered
//!
//! 1-Ups, vines, P-switches and the coin-or-star blocks of 7-7's chains
//! (code 5), which give a star only to a player who is already invincible.

use crate::randomize::maze::MazePos;
pub(crate) use crate::randomize::maze::maze_state::POWERUPS_FOUND;
use crate::randomize::maze::on_clear::{DISPATCH_CPU, TABLE_CPU, Trigger};
use crate::randomize::rom_data::{
    FS_BIGQ_GATE, FS_POWERUP_GATE, FS_POWERUP_ICON, GRAPHICS_BUFFER, STATUS_BAR_FILL_WORLD,
    prg_bank_file_to_cpu,
};
use crate::rom::Rom;

/// `LATP_JumpTable`, PRG008 CPU `$B7D1`: twelve words, indexed by item code.
const LATP_JUMP_TABLE: usize = 0x117E1;

/// The vanilla handlers whose words are repointed, by item code.
const LATP_FLOWER_CPU: u16 = 0xB7EC;
const LATP_LEAF_CPU: u16 = 0xB7FA;
const LATP_STAR_CPU: u16 = 0xB808;
/// `LATP_Coin`, the first byte after the reclaimed run.
const LATP_COIN_CPU: u16 = 0xB810;

/// The routine's origin and only entry: where `LATP_Flower` was.
const GATE_CPU: u16 = LATP_FLOWER_CPU;
/// Bytes of the run the routine fills; the rest, up to `LATP_Coin`, is filler.
const GATE_LEN: usize = 34;
const RUN_LEN: usize = 36;

/// What vanilla has in the run: `LATP_Flower`, `LATP_Leaf`, `LATP_Star`.
#[rustfmt::skip]
const VANILLA_RUN: [u8; 36] = [
    0xA9, 0x00, 0x8D, 0x86, 0x05, 0xA0, 0x05, 0xA5, 0xED, 0xF0, 0x02, 0xA0, 0x02, 0x60,
    0xA9, 0x00, 0x8D, 0x86, 0x05, 0xA0, 0x05, 0xA5, 0xED, 0xF0, 0x02, 0xA0, 0x03, 0x60,
    0xA9, 0x80, 0x8D, 0x86, 0x05, 0xA0, 0x04, 0x60,
];

/// Where Modern Power-ups writes inside the run, and what: the two
/// `LDY #$05` (small player's mushroom) operands become the flower and leaf.
const MODERN_RUN_EDITS: [(usize, u8); 2] = [(6, 0x02), (20, 0x03)];

/// A power-up the gate can hold back.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub(crate) enum PowerUp {
    Mushroom,
    Flower,
    Leaf,
    Star,
    Tanooki,
    Frog,
    Hammer,
}

impl PowerUp {
    pub(crate) const ALL: [PowerUp; 7] = [
        PowerUp::Mushroom,
        PowerUp::Flower,
        PowerUp::Leaf,
        PowerUp::Star,
        PowerUp::Tanooki,
        PowerUp::Frog,
        PowerUp::Hammer,
    ];

    /// The number of right shifts that leave this power-up's bit in the carry:
    /// for a block, its `Bouncer_PUp` index (the gate's shift loop); for a
    /// suit, its Big `[?]` box object ID less `$92` ([`BIGQ_GATE`]'s).
    const fn shifts(self) -> u8 {
        match self {
            PowerUp::Flower => 2,
            PowerUp::Leaf => 3,
            PowerUp::Star => 4,
            PowerUp::Mushroom => 5,
            PowerUp::Tanooki => BIGQ_TANOOKI - BIGQ_SHIFT_BASE,
            PowerUp::Frog => BIGQ_TANOOKI + 1 - BIGQ_SHIFT_BASE,
            PowerUp::Hammer => BIGQ_TANOOKI + 2 - BIGQ_SHIFT_BASE,
        }
    }

    /// Its bit in [`POWERUPS_FOUND`]: bit `shifts - 1`, the one that many
    /// right shifts leave in the carry.
    pub(crate) const fn bit(self) -> u8 {
        1 << (self.shifts() - 1)
    }

    /// The Global Item ID whose inventory icon announces the unlock.
    const fn inventory_item(self) -> u8 {
        match self {
            PowerUp::Mushroom => 1,
            PowerUp::Flower => 2,
            PowerUp::Leaf => 3,
            PowerUp::Frog => 4,
            PowerUp::Tanooki => 5,
            PowerUp::Hammer => 6,
            PowerUp::Star => 9,
        }
    }

    /// Parse a name as `testrom` spells it.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn from_name(name: &str) -> Option<PowerUp> {
        match name.to_ascii_lowercase().as_str() {
            "mushroom" => Some(PowerUp::Mushroom),
            "flower" | "fire" | "fireflower" => Some(PowerUp::Flower),
            "leaf" => Some(PowerUp::Leaf),
            "star" => Some(PowerUp::Star),
            "tanooki" => Some(PowerUp::Tanooki),
            "frog" => Some(PowerUp::Frog),
            "hammer" => Some(PowerUp::Hammer),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Which levels are gates, and what each needs
// ---------------------------------------------------------------------------

/// One thing a gate level needs to be beaten.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Need {
    /// This power-up, whatever the arm.
    Key(PowerUp),
    /// **To be big**, from these flower-or-leaf blocks
    /// ([`FLOWER_OR_LEAF_OFFSETS`](crate::randomize::items::powerups::FLOWER_OR_LEAF_OFFSETS)).
    /// Without Modern Power-ups: the Mushroom, which is what a small player
    /// gets from either.
    /// Modern Power-ups: the key for what each block actually rolled, since a
    /// small player gets the block's own item there.
    Big(&'static [usize]),
}

/// A gate: a level or fortress that cannot be beaten without some power-ups.
///
/// **The table is the extension point.** The placement pass, the solver and the
/// writer all take gates as data, so a new gate level is one row here.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GateDef {
    /// The level, as `NodeCatalog` names it (`testrom --list`); the writer pins
    /// this entry onto the cell the placement pass chose.
    pub(crate) name: &'static str,
    /// A level gate or a fortress gate: which kind of slot it can go on.
    pub(crate) fortress: bool,
    pub(crate) needs: &'static [Need],
}

/// The gates: every level whose finish depends on a protected power-up
/// (`items::powerups`).
pub(crate) const GATES: &[GateDef] = &[
    // Flying is the way across the ice; a small player gets the mushroom row.
    GateDef { name: "6-5", fortress: false, needs: &[Need::Big(&[]), Need::Key(PowerUp::Leaf)] },
    // The star chain: the level's protected ? stars.
    GateDef { name: "7-7", fortress: false, needs: &[Need::Key(PowerUp::Star)] },
    // Bricks stand between the player and the Tanooki box.
    GateDef {
        name: "7F1",
        fortress: true,
        needs: &[Need::Big(&[0x2B39E]), Need::Key(PowerUp::Tanooki)],
    },
    // A block later in the level only breaks for a big player.
    GateDef { name: "8F1", fortress: true, needs: &[Need::Big(&[0x2B900, 0x2BB18])] },
];

impl GateDef {
    /// The keys this gate needs on this ROM: `modern` picks how [`Need::Big`]
    /// resolves, and `rom` (after the block reroll) says what its blocks hold.
    pub(crate) fn keys(&self, modern: bool, rom: &Rom) -> Vec<PowerUp> {
        let mut out = Vec::new();
        for need in self.needs {
            match *need {
                Need::Key(p) => out.push(p),
                Need::Big(_) if !modern => out.push(PowerUp::Mushroom),
                Need::Big(blocks) => {
                    for &at in blocks {
                        match flower_or_leaf(rom.read_byte(at)) {
                            Some(p) => out.push(p),
                            // Not a shape the reroll writes: say both. Logic
                            // that is stricter than the game strands nobody.
                            None => out.extend([PowerUp::Flower, PowerUp::Leaf]),
                        }
                    }
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }
}

/// What a flower-or-leaf block's shape byte holds: `?` block 0/1, brick 6/7.
fn flower_or_leaf(shape: u8) -> Option<PowerUp> {
    match shape {
        0x00 | 0x06 => Some(PowerUp::Flower),
        0x01 | 0x07 => Some(PowerUp::Leaf),
        _ => None,
    }
}

/// Every power-up that gets a key level: all of them, less the Mushroom under
/// Modern Power-ups, where no block hands one out.
pub(crate) fn keyed_powerups(modern: bool) -> Vec<PowerUp> {
    PowerUp::ALL.into_iter().filter(|&p| !(modern && p == PowerUp::Mushroom)).collect()
}

/// The map tile a key level wears: the dancing flower (`$BD`), a special
/// entry completable like a panel. Vanilla also draws it as off-path scenery
/// in Worlds 4 and 7, so a key level stands out by sitting on a path.
pub(crate) const TILE_KEY_LEVEL: u8 = 0xBD;

/// The on-clear row that makes `cell` the key level for `powerup`.
pub(crate) fn trigger(cell: MazePos, powerup: PowerUp) -> Trigger {
    Trigger { cell, flag: POWERUPS_FOUND, value: powerup.bit() }
}

/// The [`POWERUPS_FOUND`] bits of what a suit block can hand out — star,
/// Tanooki, Frog, Hammer — in
/// [`block_spawn`](crate::randomize::items::block_spawn)'s pool order.
pub(crate) const SUIT_POOL_BITS: [u8; 4] =
    [PowerUp::Star.bit(), PowerUp::Tanooki.bit(), PowerUp::Frog.bit(), PowerUp::Hammer.bit()];

/// Offset of the `LDY Player_Suit` operand pair, the whole difference between
/// the classic and Modern arms.
const SUIT_TEST_AT: usize = 4;

/// The gate. Entered with `Y` = item code × 2 (2, 4, 6).
///
/// ```text
/// gate:  TYA / LSR A           ; A = 1 flower, 2 leaf, 3 star
///        CMP #$03              ; C = star: size does not matter
///        LDY Player_Suit       ; (Modern: LDY #$01)
///        BNE big               ; big: the block's own product
///        BCS big               ; small, but a star
///        LDA #$04              ; small at a flower/leaf: the mushroom
/// big:   TAY / INY             ; Y = the Bouncer_PUp index, A + 1
///        LDA #$00 / ROR A      ; carry is still the CMP's: $80 for the star
///        STA PUp_StarManFlash
///        STY Temp_Var1
///        LDA POWERUPS_FOUND
/// shift: LSR A / DEY / BNE shift   ; index shifts: C = bit (index - 1)
///        LDY Temp_Var1         ; Y = the index again
///        BCC LATP_Coin         ; locked: the next byte is LATP_Coin
///        RTS
/// ```
///
/// Two things keep it inside the run. **The `CMP` serves twice**: its
/// carry picks the star out of the size test, and survives the
/// `LDY`, `TAY`, `INY` and `LDA #` that follow (none touches carry) to become
/// the star flash through `ROR`. And **the locked path costs one branch**,
/// because the coin handler is the next byte.
///
/// `Temp_Var1` is free: the dispatcher stored the jump address there for its
/// `JMP [Temp_Var1]`, its caller reads only `Y` on return, and the locked path
/// overwrites it anyway (`LATP_CoinCommon` stores the block's Y in it). It is
/// also one of the three bytes the NMI preserves.
#[rustfmt::skip]
const GATE: [u8; GATE_LEN] = [
    0x98,                           //  0: gate: TYA
    0x4A,                           //  1: LSR A
    0xC9, 0x03,                     //  2: CMP #$03
    0xA4, 0xED,                     //  4: LDY Player_Suit
    0xD0, 0x04,                     //  6: BNE big
    0xB0, 0x02,                     //  8: BCS big
    0xA9, 0x04,                     // 10: LDA #$04
    0xA8,                           // 12: big: TAY
    0xC8,                           // 13: INY
    0xA9, 0x00,                     // 14: LDA #$00
    0x6A,                           // 16: ROR A
    0x8D, 0x86, 0x05,               // 17: STA PUp_StarManFlash
    0x84, 0x00,                     // 20: STY Temp_Var1
    0xAD, POWERUPS_FOUND as u8, (POWERUPS_FOUND >> 8) as u8, // 22: LDA POWERUPS_FOUND
    0x4A,                           // 25: shift: LSR A
    0x88,                           // 26: DEY
    0xD0, 0xFC,                     // 27: BNE shift
    0xA4, 0x00,                     // 29: LDY Temp_Var1
    0x90, (LATP_COIN_CPU - (GATE_CPU + GATE_LEN as u16 - 1)) as u8, // 31: BCC LATP_Coin
    0x60,                           // 33: RTS
];

/// The gate for one arm or the other: `modern` swaps the size test for
/// `LDY #$01`, which is always non-zero.
fn gate_bytes(modern: bool) -> [u8; GATE_LEN] {
    let mut out = GATE;
    if modern {
        out[SUIT_TEST_AT] = 0xA0; // LDY #imm
        out[SUIT_TEST_AT + 1] = 0x01;
    }
    out
}

fn word(rom: &Rom, code: usize) -> u16 {
    let off = LATP_JUMP_TABLE + code * 2;
    u16::from_le_bytes([rom.read_byte(off), rom.read_byte(off + 1)])
}

fn repoint(rom: &mut Rom, code: usize, vanilla: u16, to: u16) {
    let found = word(rom, code);
    assert_eq!(found, vanilla, "LATP_JumpTable code {code} holds {found:#06X}, not {vanilla:#06X}");
    rom.write_range(LATP_JUMP_TABLE + code * 2, &to.to_le_bytes());
}

/// The run as it stands before the gate: vanilla, or vanilla with Modern
/// Power-ups' two operands.
fn expected_run(modern: bool) -> [u8; RUN_LEN] {
    let mut run = VANILLA_RUN;
    if modern {
        for (at, v) in MODERN_RUN_EDITS {
            run[at] = v;
        }
    }
    run
}

/// Install the block gate and the Big `[?]` box gate. Runs after Modern
/// Power-ups, which it is told about rather than guessing from the bytes:
/// `modern` picks the arm. Suit blocks are gated by
/// [`block_spawn`](crate::randomize::items::block_spawn)'s own flag, and the
/// unlock icon by [`install_icon`], which the on-clear table needs first.
pub(crate) fn apply(rom: &mut Rom, modern: bool) {
    assert_eq!(
        rom.read_range(FS_POWERUP_GATE, RUN_LEN),
        &expected_run(modern)[..],
        "LATP_Flower/Leaf/Star are not as {} leaves them",
        if modern { "Modern Power-ups" } else { "vanilla" }
    );
    rom.write_range(FS_POWERUP_GATE, &gate_bytes(modern));
    rom.write_range(FS_POWERUP_GATE + GATE_LEN, &[0xFF; RUN_LEN - GATE_LEN]);
    repoint(rom, 1, LATP_FLOWER_CPU, GATE_CPU);
    repoint(rom, 2, LATP_LEAF_CPU, GATE_CPU);
    repoint(rom, 3, LATP_STAR_CPU, GATE_CPU);

    assert_eq!(rom.read_range(BIGQ_OPEN_SITE, 5), BIGQ_OPEN_VANILLA, "BigQBlock_Open moved");
    rom.write_range(FS_BIGQ_GATE, &BIGQ_GATE);
    let [lo, hi] = BIGQ_GATE_CPU.to_le_bytes();
    rom.write_range(BIGQ_OPEN_SITE, &[0x20, lo, hi, 0xEA, 0xEA]);
}

/// Install the unlock icon. Before any power-up row goes into the on-clear
/// table, which calls it.
pub(crate) fn install_icon(rom: &mut Rom) {
    rom.write_range(FS_POWERUP_ICON, &icon_routine());
}

// ---------------------------------------------------------------------------
// Big [?] boxes
// ---------------------------------------------------------------------------

/// `OBJ_BIGQBLOCK_3UP` and `OBJ_BIGQBLOCK_TANOOKI`; Frog and Hammer follow it.
const BIGQ_3UP: u8 = 0x94;
const BIGQ_TANOOKI: u8 = 0x98;
/// Subtracted from a suit box's ID to get its shift count (6, 7, 8 → bits
/// 5, 6, 7).
const BIGQ_SHIFT_BASE: u8 = 0x92;

/// `BigQBlock_Open`'s `LDY #5 / JSR BigQBlock_EmergePowerup`, PRG005 CPU
/// `$B6E5`. Straight after it vanilla tests `Level_ObjectID,X` for the 3-Up
/// box, so a box the hook has turned into one pays out three 1-ups.
const BIGQ_OPEN_SITE: usize = 0x0B6F5;
const BIGQ_OPEN_VANILLA: [u8; 5] = [0xA0, 0x05, 0x20, 0x0E, 0xB7];
/// `BigQBlock_EmergePowerup`.
const BIGQ_EMERGE_CPU: u16 = 0xB70E;
/// `Level_ObjectID`.
const LEVEL_OBJECT_ID: u16 = 0x0671;

const BIGQ_GATE_CPU: u16 = prg_bank_file_to_cpu(5, FS_BIGQ_GATE);

/// Reached by `JSR` from [`BIGQ_OPEN_SITE`] (two `NOP`s pad the five bytes),
/// with `X` = the box. A suit box whose bit is clear becomes the 3-Up box
/// before its prize emerges.
///
/// ```text
///        LDA Level_ObjectID,X
///        CMP #$98 / BCC go        ; the 3-Up box (and unused $95-$97): ungated
///        SBC #$92 / TAY           ; carry set: 6 Tanooki, 7 Frog, 8 Hammer
///        LDA POWERUPS_FOUND
/// shift: LSR A / DEY / BNE shift  ; C = bit 5, 6 or 7
///        BCS go
///        LDA #$94 / STA Level_ObjectID,X
/// go:    LDY #5
///        JMP BigQBlock_EmergePowerup   ; its RTS returns past the NOPs
/// ```
#[rustfmt::skip]
const BIGQ_GATE: [u8; 29] = [
    0xBD, LEVEL_OBJECT_ID as u8, (LEVEL_OBJECT_ID >> 8) as u8, //  0: LDA Level_ObjectID,X
    0xC9, BIGQ_TANOOKI,             //  3: CMP #$98
    0x90, 0x11,                     //  5: BCC go (-> 24)
    0xE9, BIGQ_SHIFT_BASE,          //  7: SBC #$92
    0xA8,                           //  9: TAY
    0xAD, POWERUPS_FOUND as u8, (POWERUPS_FOUND >> 8) as u8, // 10: LDA POWERUPS_FOUND
    0x4A,                           // 13: shift: LSR A
    0x88,                           // 14: DEY
    0xD0, 0xFC,                     // 15: BNE shift (-> 13)
    0xB0, 0x05,                     // 17: BCS go (-> 24)
    0xA9, BIGQ_3UP,                 // 19: LDA #$94
    0x9D, LEVEL_OBJECT_ID as u8, (LEVEL_OBJECT_ID >> 8) as u8, // 21: STA Level_ObjectID,X
    0xA0, 0x05,                     // 24: go: LDY #5
    0x4C, BIGQ_EMERGE_CPU as u8, (BIGQ_EMERGE_CPU >> 8) as u8, // 26: JMP BigQBlock_EmergePowerup
];

// ---------------------------------------------------------------------------
// The unlock icon
// ---------------------------------------------------------------------------

/// Where the icon routine runs: PRG026, mapped while the on-clear dispatcher
/// runs. [`on_clear`](crate::randomize::maze::on_clear) calls it in place of
/// `StatusBar_Fill_World` when its table holds a power-up row.
pub(crate) const ICON_CPU: u16 = prg_bank_file_to_cpu(26, FS_POWERUP_ICON);

/// `InvItem_Tile_Layout`, PRG026 CPU `$A27C`: four tiles (UL, UR, LL, LR) per
/// Global Item ID, the art the item box draws.
const INV_ITEM_TILE_LAYOUT: u16 = 0xA27C;

/// `Graphics_BufCnt`, the byte in front of the buffer.
const GRAPHICS_BUF_CNT: u16 = GRAPHICS_BUFFER - 1;

/// The low bytes of the card boxes' top and bottom rows on the map's status
/// bar (`$2B36`, `$2B56`): `CardVStartU`/`CardVStartL`'s first card.
const CARD_ROW_TOP: u8 = 0x36;
const CARD_ROW_BOTTOM: u8 = 0x56;

/// `$FE` is the bar's blank, `$AA` the divider between two card boxes.
const BLANK: u8 = 0xFE;
const DIVIDER: u8 = 0xAA;

/// Each queued row: 3 header bytes and 8 tiles, box 1, divider, box 2,
/// divider, box 3. The icon goes in box 2's two cells.
const ROW_TILES: usize = 8;
const ROW_ENTRY: usize = 3 + ROW_TILES;
const ICON_COL: usize = 3;

/// The two rows, with box 2 left blank for the icon.
#[rustfmt::skip]
const BOXES: [u8; 2 * ROW_ENTRY] = [
    0x2B, CARD_ROW_TOP, ROW_TILES as u8,
    BLANK, BLANK, DIVIDER, BLANK, BLANK, DIVIDER, BLANK, BLANK,
    0x2B, CARD_ROW_BOTTOM, ROW_TILES as u8,
    BLANK, BLANK, DIVIDER, BLANK, BLANK, DIVIDER, BLANK, BLANK,
];

/// `InvItem_Tile_Layout` offsets indexed by bit number in [`POWERUPS_FOUND`].
/// Bits no power-up owns point at row 0, the empty slot.
fn icon_offsets() -> [u8; 8] {
    let mut out = [0; 8];
    for p in [
        PowerUp::Mushroom,
        PowerUp::Flower,
        PowerUp::Leaf,
        PowerUp::Star,
        PowerUp::Tanooki,
        PowerUp::Frog,
        PowerUp::Hammer,
    ] {
        out[p.bit().trailing_zeros() as usize] = p.inventory_item() * 4;
    }
    out
}

const ICON_CODE_LEN: usize = 96;
const ICON_LEN: usize = ICON_CODE_LEN + 8 + BOXES.len();

/// After a clear, find a power-up row for the cell just cleared and draw its
/// icon in the middle card box, with the outer two blank; then on to
/// `StatusBar_Fill_World` either way.
///
/// ```text
///        LDX dispatcher+1        ; the dispatcher's own last-row operand
/// loop:  LDA <T1 / CMP key0,X / BNE next
///        LDA <T2 / CMP key1,X / BNE next
///        LDA flag,X / CMP #<POWERUPS_FOUND / BEQ found
/// next:  DEX ×4 / BPL loop
///        JMP StatusBar_Fill_World
/// found: LDA value,X             ; one bit
///        LDY #$FF
/// bit:   INY / LSR A / BCC bit   ; Y = its number
///        LDA icons,Y / STA <T3   ; InvItem_Tile_Layout offset
///        LDX Graphics_BufCnt / LDY #0
/// copy:  LDA boxes,Y / STA Graphics_Buffer,X / INX / INY
///        CPY #22 / BNE copy
///        STX Graphics_BufCnt
///        LDY <T3                 ; the icon's four tiles into box 2
///        LDA layout,Y   / STA Graphics_Buffer-16,X
///        LDA layout+1,Y / STA Graphics_Buffer-15,X
///        LDA layout+2,Y / STA Graphics_Buffer-5,X
///        LDA layout+3,Y / STA Graphics_Buffer-4,X
///        JMP StatusBar_Fill_World ; appends its digit and the terminator
/// ```
///
/// **Rescanning the table** keeps the dispatcher generic: it ORs a value and
/// knows nothing of power-ups. `T1`/`T2` still hold the cleared cell, since
/// the dispatcher only reads them. Reading the row count from the
/// dispatcher's own `LDX #` operand means this routine never changes with the
/// table.
///
/// **Nothing remembers the icon.** The card boxes are next drawn by closing
/// the item box or entering a level, and both draw the real cards; until
/// then the icon stays.
#[rustfmt::skip]
pub(crate) fn icon_routine() -> [u8; ICON_LEN] {
    let lo = |a: u16| a.to_le_bytes()[0];
    let hi = |a: u16| a.to_le_bytes()[1];
    let last = DISPATCH_CPU + 1;
    let key0 = TABLE_CPU;
    let key1 = TABLE_CPU + 1;
    let flag = TABLE_CPU + 2;
    let value = TABLE_CPU + 3;
    let fill = STATUS_BAR_FILL_WORLD;
    let icons = ICON_CPU + ICON_CODE_LEN as u16;
    let boxes = icons + 8;
    let buf = GRAPHICS_BUFFER;
    let cnt = GRAPHICS_BUF_CNT;
    let lay = INV_ITEM_TILE_LAYOUT;
    // Buffer offsets of box 2's cells, back from X = base + 22.
    let ul = buf - (2 * ROW_ENTRY - (3 + ICON_COL)) as u16;
    let ll = buf - (ROW_ENTRY - (3 + ICON_COL)) as u16;
    let code: [u8; ICON_CODE_LEN] = [
        0xAE, lo(last), hi(last),          //  0: LDX dispatcher+1
        0xA5, 0x00,                        //  3: loop: LDA <T1
        0xDD, lo(key0), hi(key0),          //  5: CMP key0,X
        0xD0, 0x0E,                        //  8: BNE next (-> 24)
        0xA5, 0x01,                        // 10: LDA <T2
        0xDD, lo(key1), hi(key1),          // 12: CMP key1,X
        0xD0, 0x07,                        // 15: BNE next (-> 24)
        0xBD, lo(flag), hi(flag),          // 17: LDA flag,X
        0xC9, POWERUPS_FOUND as u8,        // 20: CMP #<POWERUPS_FOUND
        0xF0, 0x09,                        // 22: BEQ found (-> 33)
        0xCA, 0xCA, 0xCA, 0xCA,            // 24: next: DEX ×4
        0x10, 0xE5,                        // 28: BPL loop (-> 3)
        0x4C, lo(fill), hi(fill),          // 30: JMP StatusBar_Fill_World
        0xBD, lo(value), hi(value),        // 33: found: LDA value,X
        0xA0, 0xFF,                        // 36: LDY #$FF
        0xC8,                              // 38: bit: INY
        0x4A,                              // 39: LSR A
        0x90, 0xFC,                        // 40: BCC bit (-> 38)
        0xB9, lo(icons), hi(icons),        // 42: LDA icons,Y
        0x85, 0x02,                        // 45: STA <T3
        0xAE, lo(cnt), hi(cnt),            // 47: LDX Graphics_BufCnt
        0xA0, 0x00,                        // 50: LDY #0
        0xB9, lo(boxes), hi(boxes),        // 52: copy: LDA boxes,Y
        0x9D, lo(buf), hi(buf),            // 55: STA Graphics_Buffer,X
        0xE8,                              // 58: INX
        0xC8,                              // 59: INY
        0xC0, BOXES.len() as u8,           // 60: CPY #22
        0xD0, 0xF4,                        // 62: BNE copy (-> 52)
        0x8E, lo(cnt), hi(cnt),            // 64: STX Graphics_BufCnt
        0xA4, 0x02,                        // 67: LDY <T3
        0xB9, lo(lay), hi(lay),            // 69: LDA layout,Y      ; UL
        0x9D, lo(ul), hi(ul),              // 72: STA box 2 top-left
        0xB9, lo(lay + 1), hi(lay + 1),    // 75: LDA layout+1,Y    ; UR
        0x9D, lo(ul + 1), hi(ul + 1),      // 78: STA box 2 top-right
        0xB9, lo(lay + 2), hi(lay + 2),    // 81: LDA layout+2,Y    ; LL
        0x9D, lo(ll), hi(ll),              // 84: STA box 2 bottom-left
        0xB9, lo(lay + 3), hi(lay + 3),    // 87: LDA layout+3,Y    ; LR
        0x9D, lo(ll + 1), hi(ll + 1),      // 90: STA box 2 bottom-right
        0x4C, lo(fill), hi(fill),          // 93: JMP StatusBar_Fill_World
    ];
    let mut out = [0; ICON_LEN];
    out[..ICON_CODE_LEN].copy_from_slice(&code);
    out[ICON_CODE_LEN..ICON_CODE_LEN + 8].copy_from_slice(&icon_offsets());
    out[ICON_CODE_LEN + 8..].copy_from_slice(&BOXES);
    out
}

/// Whether [`install_icon`] has written the icon routine, which the on-clear
/// dispatcher must not call into otherwise.
pub(crate) fn icon_installed(rom: &Rom) -> bool {
    rom.read_range(FS_POWERUP_ICON, ICON_LEN) == icon_routine()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROM_PATH: &str = "roms/Super Mario Bros. 3 (USA) (Rev 1).nes";

    fn vanilla() -> Option<Rom> {
        Rom::from_bytes(&std::fs::read(ROM_PATH).ok()?).ok()
    }

    /// The run, the jump table and the coin handler are where this module
    /// says, so the writes land on what they mean to replace.
    #[test]
    fn the_rom_is_shaped_the_way_this_assumes() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        assert_eq!(rom.read_range(FS_POWERUP_GATE, 36), &VANILLA_RUN[..]);
        assert_eq!(word(&rom, 1), LATP_FLOWER_CPU);
        assert_eq!(word(&rom, 2), LATP_LEAF_CPU);
        assert_eq!(word(&rom, 3), LATP_STAR_CPU);
        assert_eq!(word(&rom, 4), LATP_COIN_CPU, "LATP_Coin follows the run");
        assert_eq!(usize::from(LATP_COIN_CPU - GATE_CPU), RUN_LEN, "the run ends at LATP_Coin");
        // Big [?] boxes: the hook site, the emerge routine and the 3-Up test
        // right after the site, and the suit IDs in Tanooki/Frog/Hammer order.
        assert_eq!(rom.read_range(BIGQ_OPEN_SITE, 5), BIGQ_OPEN_VANILLA);
        let emerge = 0x0A010 + usize::from(BIGQ_EMERGE_CPU - 0xA000);
        assert_eq!(rom.read_range(emerge, 3), &[0xA9, 0x01, 0x99], "LDA #INIT / STA State,Y");
        assert_eq!(
            rom.read_range(BIGQ_OPEN_SITE + 5, 5),
            &[0xBD, 0x71, 0x06, 0xC9, BIGQ_3UP],
            "LDA Level_ObjectID,X / CMP #OBJ_BIGQBLOCK_3UP"
        );
        // BigQBlock_StarManFlash: frames 1/2/3 for $98/$99/$9A.
        let flash = 0x0A010 + (0xB6D9 - 0xA000);
        assert_eq!(rom.read_range(flash + 4, 3), &[1, 2, 3], "Tanooki, Frog, Hammer");
    }

    /// What the icon routine draws from and over: the item box's tile table,
    /// the first card's VRAM row starts, and the bar's card boxes as blanks
    /// between `$AA` dividers.
    #[test]
    fn the_status_bar_is_shaped_the_way_the_icon_assumes() {
        let Some(rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let layout = 0x34010 + usize::from(INV_ITEM_TILE_LAYOUT - 0xA000);
        assert_eq!(rom.read_range(layout + 4, 4), &[0x20, 0x21, 0x30, 0x31], "Super Mushroom");
        assert_eq!(rom.read_range(layout + 5 * 4, 4), &[0x00, 0x01, 0x10, 0x11], "Tanooki Suit");
        // CardVStartU / CardVStartL, PRG031.
        assert_eq!(
            rom.read_range(0x3FCC6, 6),
            &[CARD_ROW_TOP, 0x39, 0x3C, CARD_ROW_BOTTOM, 0x59, 0x5C]
        );
        // The $2B00 StatusBar copy's middle rows end with the card boxes.
        let boxes = [BLANK, BLANK, DIVIDER, BLANK, BLANK, DIVIDER, BLANK, BLANK];
        let bar = rom.read_range(0x3C143 + 16, 0x80);
        assert!(bar.windows(8).filter(|w| *w == boxes).count() >= 2, "card boxes not found");
    }

    /// Each power-up's bit picks its own icon, and no two share one.
    #[test]
    fn every_power_up_has_its_icon() {
        let icons = icon_offsets();
        for p in PowerUp::ALL {
            assert_eq!(icons[p.bit().trailing_zeros() as usize], p.inventory_item() * 4);
        }
    }

    /// Modern Power-ups' edits are the bytes this module expects inside the
    /// run, read from the patch itself rather than restated.
    #[test]
    fn modern_powerups_edits_are_the_ones_the_gate_expects() {
        let Some(mut rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        crate::randomize::qol::apply_modern_powerups(&mut rom);
        assert_eq!(rom.read_range(FS_POWERUP_GATE, RUN_LEN), &expected_run(true)[..]);
    }

    /// Codes 1-3 reach the routine, and no other word moves.
    #[test]
    fn only_the_gated_codes_are_repointed() {
        let Some(mut rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let before: Vec<u16> = (0..12).map(|c| word(&rom, c)).collect();
        apply(&mut rom, false);
        install_icon(&mut rom);
        for (code, &was) in before.iter().enumerate() {
            let want = if (1..=3).contains(&code) { GATE_CPU } else { was };
            assert_eq!(word(&rom, code), want, "code {code}");
        }
    }

    /// Both arms install over the run they expect.
    #[test]
    fn modern_arm_installs_over_modern_powerups() {
        let Some(mut rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        crate::randomize::qol::apply_modern_powerups(&mut rom);
        apply(&mut rom, true);
        assert_eq!(rom.read_range(FS_POWERUP_GATE, GATE_LEN), &gate_bytes(true)[..]);
    }

    /// The arms differ in the size test's two bytes and nowhere else.
    #[test]
    fn the_modern_arm_differs_only_in_the_size_test() {
        let (classic, modern) = (gate_bytes(false), gate_bytes(true));
        assert_eq!(&classic[SUIT_TEST_AT..SUIT_TEST_AT + 2], &[0xA4, 0xED], "LDY Player_Suit");
        assert_eq!(&modern[SUIT_TEST_AT..SUIT_TEST_AT + 2], &[0xA0, 0x01], "LDY #$01");
        for i in (0..GATE_LEN).filter(|i| !(SUIT_TEST_AT..SUIT_TEST_AT + 2).contains(i)) {
            assert_eq!(classic[i], modern[i], "byte {i}");
        }
    }

    /// The suit bits are the ones a box's ID reaches: `$98`/`$99`/`$9A` less
    /// `$92` shifts, the bit that leaves in the carry.
    #[test]
    fn the_suit_bits_follow_the_box_ids() {
        assert_eq!(PowerUp::Tanooki.bit(), 1 << (BIGQ_TANOOKI - BIGQ_SHIFT_BASE - 1));
        assert_eq!(
            [PowerUp::Tanooki.bit(), PowerUp::Frog.bit(), PowerUp::Hammer.bit()],
            [0x20, 0x40, 0x80]
        );
    }

    /// Every `Big` block is one the reroll keeps to flower or leaf.
    #[test]
    fn every_big_block_is_a_flower_or_leaf_block() {
        use crate::randomize::items::powerups::FLOWER_OR_LEAF_OFFSETS;
        for gate in GATES {
            for need in gate.needs {
                if let Need::Big(blocks) = need {
                    for b in *blocks {
                        assert!(FLOWER_OR_LEAF_OFFSETS.contains(b), "{} block {b:#07X}", gate.name);
                    }
                }
            }
        }
    }

    /// Normal power-ups resolve "be big" to the Mushroom; Modern to each block's roll,
    /// and to both keys when a block's byte is not a roll at all.
    #[test]
    fn gate_keys_follow_the_arm_and_the_rolls() {
        let Some(mut rom) = vanilla() else {
            eprintln!("SKIP: requires the ROM");
            return;
        };
        let f8 = GATES.iter().find(|g| g.name == "8F1").unwrap();
        let f7 = GATES.iter().find(|g| g.name == "7F1").unwrap();
        let six5 = GATES.iter().find(|g| g.name == "6-5").unwrap();
        assert_eq!(f8.keys(false, &rom), [PowerUp::Mushroom]);
        assert_eq!(f7.keys(false, &rom), [PowerUp::Mushroom, PowerUp::Tanooki]);
        assert_eq!(six5.keys(false, &rom), [PowerUp::Mushroom, PowerUp::Leaf]);
        assert_eq!(six5.keys(true, &rom), [PowerUp::Leaf]);

        rom.write_byte(0x2B39E, 0x00); // ? block: flower
        assert_eq!(f7.keys(true, &rom), [PowerUp::Flower, PowerUp::Tanooki]);
        rom.write_byte(0x2B900, 0x07); // brick: leaf
        rom.write_byte(0x2BB18, 0x00); // ? block: flower
        assert_eq!(f8.keys(true, &rom), [PowerUp::Flower, PowerUp::Leaf]);
        rom.write_byte(0x2BB18, 0x01);
        assert_eq!(f8.keys(true, &rom), [PowerUp::Leaf]);
        rom.write_byte(0x2BB18, 0x42);
        assert_eq!(f8.keys(true, &rom), [PowerUp::Flower, PowerUp::Leaf], "unknown: both");
    }

    /// The bits are distinct, one each, and the trigger carries the bit.
    #[test]
    fn every_powerup_has_its_own_bit() {
        let mut seen = 0u8;
        for p in PowerUp::ALL {
            assert_eq!(p.bit().count_ones(), 1, "{p:?}");
            assert_eq!(seen & p.bit(), 0, "{p:?} shares a bit");
            seen |= p.bit();
            assert_eq!(trigger((0, (2, 4)), p).value, p.bit());
        }
    }
}

/// Running the gate on an emulated 2A03 over every input it can see.
///
/// The routine reads three things — the item code (as `Y`), the player's size
/// and the found byte — and decides two: what spawns, and the star flash. All
/// of it fits in a loop: 4 entries × 7 sizes × 256 found bytes × 2 arms.
#[cfg(test)]
mod execution {
    use super::*;
    use mos6502::cpu::CPU;
    use mos6502::instruction::Ricoh2a03;
    use mos6502::memory::{Bus, Memory};

    const SENTINEL: u16 = 0x0F00;
    const PLAYER_SUIT: u16 = 0x00ED;
    const STAR_FLASH: u16 = 0x0586;

    #[derive(Clone, Copy, Debug)]
    enum Block {
        Flower,
        Leaf,
        Star,
    }

    /// What the routine did: `None` is the coin handler, `Some((y, flash))`
    /// a power-up.
    fn run(modern: bool, block: Block, suit: u8, found: u8) -> Option<(u8, u8)> {
        let mut mem = Memory::new();
        mem.set_bytes(GATE_CPU, &gate_bytes(modern));
        mem.set_byte(PLAYER_SUIT, suit);
        mem.set_byte(POWERUPS_FOUND, found);
        mem.set_byte(STAR_FLASH, 0x55);
        let mut cpu = CPU::new(mem, Ricoh2a03);
        let ret = SENTINEL.wrapping_sub(1);
        cpu.memory.set_byte(0x01FF, (ret >> 8) as u8);
        cpu.memory.set_byte(0x01FE, ret as u8);
        cpu.registers.stack_pointer = mos6502::registers::StackPointer(0xFD);
        let y = match block {
            Block::Flower => 2,
            Block::Leaf => 4,
            Block::Star => 6,
        };
        cpu.registers.program_counter = GATE_CPU;
        cpu.registers.index_y = y;
        // X carries the tile-check index through the handlers; it must survive.
        cpu.registers.index_x = 0x3C;
        for _ in 0..200 {
            match cpu.registers.program_counter {
                SENTINEL => {
                    assert_eq!(cpu.registers.index_x, 0x3C, "X was clobbered");
                    let flash = cpu.memory.get_byte(STAR_FLASH);
                    return Some((cpu.registers.index_y, flash));
                }
                LATP_COIN_CPU => {
                    assert_eq!(cpu.registers.index_x, 0x3C, "X was clobbered");
                    return None;
                }
                _ => {
                    cpu.single_step();
                }
            }
        }
        panic!("the gate ran away: {block:?} suit {suit} found {found:#04X}");
    }

    /// The table in the module doc, as a function.
    fn model(modern: bool, block: Block, suit: u8, found: u8) -> Option<(u8, u8)> {
        let small = suit == 0 && !modern;
        let (p, flash) = match block {
            Block::Flower if small => (PowerUp::Mushroom, 0x00),
            Block::Leaf if small => (PowerUp::Mushroom, 0x00),
            Block::Flower => (PowerUp::Flower, 0x00),
            Block::Leaf => (PowerUp::Leaf, 0x00),
            Block::Star => (PowerUp::Star, 0x80),
        };
        (found & p.bit() != 0).then_some((p.shifts(), flash))
    }

    #[test]
    fn every_input_matches_the_table() {
        for modern in [false, true] {
            for block in [Block::Flower, Block::Leaf, Block::Star] {
                // 0 small, 1 big, then the suits.
                for suit in 0..7 {
                    for found in 0..=255u8 {
                        assert_eq!(
                            run(modern, block, suit, found),
                            model(modern, block, suit, found),
                            "modern {modern}, {block:?}, suit {suit}, found {found:#04X}"
                        );
                    }
                }
            }
        }
    }

    /// With everything unlocked the classic arm is vanilla's three handlers,
    /// index for index — the gate adds a lock and changes nothing else.
    #[test]
    fn fully_unlocked_is_vanilla() {
        for suit in 0..7 {
            let small = suit == 0;
            assert_eq!(run(false, Block::Flower, suit, 0xFF), Some((if small { 5 } else { 2 }, 0)));
            assert_eq!(run(false, Block::Leaf, suit, 0xFF), Some((if small { 5 } else { 3 }, 0)));
            assert_eq!(run(false, Block::Star, suit, 0xFF), Some((4, 0x80)));
        }
    }

    /// What a Big [?] box becomes: run the gate for every box and found byte
    /// and read back `Level_ObjectID,X`, `Y` and where it jumped.
    #[test]
    fn a_locked_suit_box_becomes_the_3up_box() {
        const X: u8 = 3;
        for id in BIGQ_3UP..=BIGQ_TANOOKI + 2 {
            for found in 0..=255u8 {
                let mut mem = Memory::new();
                mem.set_bytes(BIGQ_GATE_CPU, &BIGQ_GATE);
                mem.set_byte(LEVEL_OBJECT_ID + u16::from(X), id);
                mem.set_byte(POWERUPS_FOUND, found);
                let mut cpu = CPU::new(mem, Ricoh2a03);
                cpu.registers.program_counter = BIGQ_GATE_CPU;
                cpu.registers.index_x = X;
                for _ in 0..100 {
                    if cpu.registers.program_counter == BIGQ_EMERGE_CPU {
                        break;
                    }
                    cpu.single_step();
                }
                assert_eq!(cpu.registers.program_counter, BIGQ_EMERGE_CPU, "{id:#04X}");
                assert_eq!((cpu.registers.index_x, cpu.registers.index_y), (X, 5));
                let suit = [PowerUp::Tanooki, PowerUp::Frog, PowerUp::Hammer]
                    .get(usize::from(id.wrapping_sub(BIGQ_TANOOKI)));
                let want = match suit {
                    Some(p) if found & p.bit() == 0 => BIGQ_3UP,
                    _ => id,
                };
                let got = cpu.memory.get_byte(LEVEL_OBJECT_ID + u16::from(X));
                assert_eq!(got, want, "box {id:#04X}, found {found:#04X}");
            }
        }
    }
}

#[cfg(test)]
mod asm_checks {
    use super::*;
    use crate::randomize::rom_data::{NMI_SAFE_MAX, asm};

    /// Both arms decode, branch onto instruction boundaries, fit the 36-byte
    /// run, and keep to the NMI-safe temps plus `Player_Suit`.
    ///
    /// `.fragment()` because the locked path branches one byte past the
    /// array, onto `LATP_Coin` — the run's whole point. The two things that
    /// relaxes are pinned by hand below: the last instruction is still an
    /// `RTS`, and the one branch that leaves lands exactly on `LATP_Coin`.
    #[test]
    fn gate_is_well_formed() {
        for modern in [false, true] {
            let code = gate_bytes(modern);
            asm::check(&code)
                .allocation(FS_POWERUP_GATE)
                .origin(GATE_CPU)
                .zero_page(NMI_SAFE_MAX, &[0xED])
                .fragment()
                .assert_ok();
            assert_eq!(code[GATE_LEN - 1], 0x60, "the routine must end in RTS");
            assert_eq!(code[GATE_LEN - 3], 0x90, "the locked path's BCC");
            let target = GATE_CPU + GATE_LEN as u16 - 1 + u16::from(code[GATE_LEN - 2]);
            assert_eq!(target, LATP_COIN_CPU, "the locked path must land on LATP_Coin");
        }
    }

    /// The Big [?] gate decodes, fits its allocation, and ends in its `JMP`;
    /// the hook displaces exactly `LDY #5 / JSR`.
    #[test]
    fn bigq_gate_is_well_formed() {
        asm::check(&BIGQ_GATE)
            .allocation(FS_BIGQ_GATE)
            .origin(BIGQ_GATE_CPU)
            .zero_page(NMI_SAFE_MAX, &[])
            .assert_ok();
        let [lo, hi] = BIGQ_GATE_CPU.to_le_bytes();
        asm::check(&BIGQ_GATE)
            .origin(BIGQ_GATE_CPU)
            .hook(&BIGQ_OPEN_VANILLA, 0, &[0x20, lo, hi, 0xEA, 0xEA])
            .assert_ok();
    }

    /// The icon routine decodes, ends in its `JMP`, fits its allocation, and
    /// keeps to the NMI-safe temps; its two tables follow the code.
    #[test]
    fn icon_routine_is_well_formed() {
        asm::check(&icon_routine())
            .allocation(FS_POWERUP_ICON)
            .origin(ICON_CPU)
            .data_from(ICON_CODE_LEN)
            .zero_page(NMI_SAFE_MAX, &[])
            .assert_ok();
    }
}
