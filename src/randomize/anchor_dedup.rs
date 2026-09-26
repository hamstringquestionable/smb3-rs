//! Don't hand the player an Anchor they already have.
//!
//! In World Maze the Anchor is the canoe key and is **never consumed** — see
//! [`canoe_gate`](super::canoe_gate). One is all anyone needs, so every later
//! Anchor is dead weight in the inventory. Four sources deal them (Hammer Bro
//! rewards, in-level chests, Toad Houses, Princess letters), and with the maze
//! pool adding houses a long run can collect a fistful.
//!
//! This makes a duplicate grant hand over **nothing instead**, by substituting
//! item `$00` — except at a Toad House, which hands over a random power-up
//! rolled once per seed. A house *draws* the item it gives, and `$00` is not
//! drawable; see [`house`].
//!
//! # `$00` is vanilla's own "no item"
//!
//! Nothing new is invented here. The letter path already has
//! `BEQ` → `RTS` for World 7, whose reward byte is `$00`; and
//! `Player_GetItem` with `A = 0` finds the first empty slot and stores `$00`
//! into it, which is a no-op. So the reward sequence still runs in full — the
//! box opens, the item is revealed, the fanfare plays — and the inventory
//! simply does not grow. That is deliberate: a reward that visibly does
//! nothing reads as "you already have this", where a reward that does not
//! appear at all reads as a bug.
//!
//! # Where the hooks go, and why not at the store
//!
//! Every path *stores* into `Inventory_Items` in PRG000 or PRG002, and both
//! banks have **zero** free bytes. But every path *selects* the item first,
//! and those sites sit in banks with room:
//!
//! | Source | Hook | Bank |
//! |---|---|---|
//! | Hammer Bro + in-level chest | `Player_GetItem` | PRG031 (always mapped) |
//! | Toad House | `ToadHouse_ChestPressB` tail | PRG029 |
//! | Princess letter | `Letter_GiveIncludedItem` load | PRG027 |
//!
//! Hammer Bro and in-level chest share `Player_GetItem`: the map copies
//! `Map_Objects_Itm` into `Level_TreasureItem` on entry, and the level's win
//! path does `LDA Level_TreasureItem / JSR Player_GetItem`. So two sources,
//! one hook.
//!
//! [`ANCHOR_HAS`] is the shared test, and it has to live in an always-mapped
//! bank because the three hooks run in three different bank configurations.
//! It sits in PRG031's 30-byte scrap rather than PRG030's 42-byte run: that
//! run is the only always-mapped gap big enough for a future feature needing a
//! real allocation, and 27 bytes of scrap was never going to hold one.
//!
//! # Maze only
//!
//! Outside the maze the Anchor is `mystery_anchor`'s per-seed surprise
//! power-up and **is** consumed on use, so duplicates are worth having. This
//! is installed only alongside the canoe gate, where the Anchor is permanent.

use super::rom_data::{FS_ANCHOR_GET_GLUE, FS_ANCHOR_HAS, FS_ANCHOR_HOUSE, FS_ANCHOR_LETTER};
use crate::rom::Rom;

/// Global Item ID of the Anchor.
const ANCHOR: u8 = 0x0A;

/// `Player_Current` — 0 for Mario, 1 for Luigi.
const PLAYER_CURRENT: u16 = 0x0726;
/// `Inventory_Items`, Mario's 28 slots.
const INVENTORY_ITEMS: u16 = 0x7D80;
/// `Inventory_Items2 - Inventory_Items`, the offset to Luigi's.
const LUIGI_OFFSET: u8 = 0x23;
/// Slots the vanilla walk checks (`Inventory_Cards - Inventory_Items - 1`).
const SLOT_COUNT: u8 = 0x1B;

/// CPU addresses of the four pieces. PRG031 is `$E000` at file `0x3E010`,
/// PRG029 is `$C000` at `0x3A010`, PRG027 is `$A000` at `0x36010`.
const ANCHOR_HAS_CPU: u16 = 0xE962;
const GET_GLUE_CPU: u16 = 0xFF2A;
const HOUSE_CPU: u16 = 0xDFEC;
const LETTER_CPU: u16 = 0xBD47;

/// `Player_GetItem` (PRG031), and the byte after the `LDY Player_Current` the
/// hook displaces — where [`GET_GLUE`] rejoins it.
const PLAYER_GET_ITEM: usize = 0x3FD7C;
const PLAYER_GET_ITEM_REJOIN: u16 = 0xFD70;

/// The tail of `ToadHouse_ChestPressB` (PRG029): `TAX / INX / RTS`, which the
/// hook replaces with a `JMP` and [`HOUSE`] performs itself.
const CHEST_PRESS_B_TAIL: usize = 0x3B1C4;

/// `Letter_GiveIncludedItem`'s `LDA LetterItem_ByWorld,Y` (PRG027).
const LETTER_LOAD: usize = 0x361EC;
/// `LetterItem_ByWorld` as PRG027 sees it (file `0x360DE`).
const LETTER_TABLE_CPU: u16 = 0xA0CE;

/// **Does this player already hold an Anchor?** Returns carry set if so.
///
/// Preserves `A`, which every caller needs: the item about to be granted is in
/// it, and the scan would otherwise clobber it. `PLA` leaves carry alone, so
/// the flag survives the restore.
///
/// Walks the same slots the vanilla grant walks. That is exact rather than
/// approximate: `Inv_UseItem_ShiftOver` keeps the inventory packed, so the
/// walk to the first empty slot passes every item the player owns.
#[rustfmt::skip]
const ANCHOR_HAS: [u8; 27] = [
    0x48,                                                   //  0: PHA
    0xAC, PLAYER_CURRENT as u8, (PLAYER_CURRENT >> 8) as u8, //  1: LDY Player_Current
    0xF0, 0x02,                                             //  4: BEQ +2 (Mario)
    0xA0, LUIGI_OFFSET,                                     //  6: LDY #$23
    0xA2, SLOT_COUNT,                                       //  8: LDX #$1B
    0xB9, INVENTORY_ITEMS as u8, (INVENTORY_ITEMS >> 8) as u8, // 10: LDA Inventory_Items,Y
    0xC9, ANCHOR,                                           // 13: CMP #$0A
    0xF0, 0x07,                                             // 15: BEQ found
    0xC8,                                                   // 17: INY
    0xCA,                                                   // 18: DEX
    0xD0, 0xF5,                                             // 19: BNE -11 (loop)
    0x18,                                                   // 21: CLC  (not held)
    0x68,                                                   // 22: PLA
    0x60,                                                   // 23: RTS
    0x38,                                                   // 24: found: SEC
    0x68,                                                   // 25: PLA
    0x60,                                                   // 26: RTS
];

/// The `Player_GetItem` hook body — Hammer Bro rewards and in-level chests.
///
/// Re-does the `PHA` and `LDY Player_Current` the 3-byte `JMP` displaced, then
/// rejoins vanilla at its `BEQ`. On a duplicate it discards the pushed item and
/// returns, which is cheaper than storing `$00` and avoids the one case where
/// storing `$00` would misbehave: with the inventory full, vanilla "goes at the
/// end" and a `$00` there would erase the last item.
#[rustfmt::skip]
const GET_GLUE: [u8; 18] = [
    0x48,                                                   //  0: PHA
    0xC9, ANCHOR,                                           //  1: CMP #$0A
    0xD0, 0x07,                                             //  3: BNE cont
    0x20, ANCHOR_HAS_CPU as u8, (ANCHOR_HAS_CPU >> 8) as u8,//  5: JSR anchor_has
    0x90, 0x02,                                             //  8: BCC cont
    0x68,                                                   // 10: PLA (discard)
    0x60,                                                   // 11: RTS
    0xAC, PLAYER_CURRENT as u8, (PLAYER_CURRENT >> 8) as u8,// 12: cont: LDY Player_Current
    0x4C, PLAYER_GET_ITEM_REJOIN as u8, (PLAYER_GET_ITEM_REJOIN >> 8) as u8, // 15: JMP
];

/// The Toad House hook body: swap the duplicate Anchor for `substitute`, then
/// do the `TAX / INX / RTS` the `JMP` displaced.
///
/// **This is the one path that cannot use `$00`.** A Toad House *draws* the
/// item it is handing over: `ObjNorm_ToadHouseItem` reads `Objects_Frame`
/// three times — for the palette, for the inventory store, and for the sprite
/// tiles via `ToadItem_PatternLeft-1,X`. One value drives all three, so
/// "show an Anchor but store nothing" is not expressible from here; splitting
/// them would mean hooking the store itself, in PRG002, which has no free
/// bytes and would put the routine in PRG030's last always-mapped run.
///
/// And `$00` garbles. Vanilla never puts a zero here (`0` means "box not
/// opened yet" — `PRG008` does `TXA / BEQ` on exactly that), so index 0 reads
/// the byte *before* `ToadItem_PatternLeft`, which is the `RTS` of the routine
/// above it, and draws `$60` as a sprite. Confirmed on hardware: the reveal
/// came out as garbage while correctly granting nothing.
///
/// So a real power-up it is — rolled once per seed by
/// [`items::toad_house_substitute`](super::items::toad_house_substitute), from
/// a pool that holds no Anchor. The player opens the box, sees a real item and
/// gets it; they just do not get a second Anchor.
#[rustfmt::skip]
fn house(substitute: u8) -> [u8; 14] {
    [
        0xC9, ANCHOR,                                           //  0: CMP #$0A
        0xD0, 0x07,                                             //  2: BNE out
        0x20, ANCHOR_HAS_CPU as u8, (ANCHOR_HAS_CPU >> 8) as u8,//  4: JSR anchor_has
        0x90, 0x02,                                             //  7: BCC out
        0xA9, substitute,                                       //  9: LDA #substitute
        0xAA,                                                   // 11: out: TAX
        0xE8,                                                   // 12: INX
        0x60,                                                   // 13: RTS
    ]
}

/// The Princess letter hook body. Does the `LDA LetterItem_ByWorld,Y` the
/// `JSR` displaced, then zeroes it on a duplicate — vanilla's next two
/// instructions are `STA CineKing_Var / BEQ rts`, so `$00` takes the no-item
/// exit World 7 already uses.
#[rustfmt::skip]
const LETTER: [u8; 15] = [
    0xB9, LETTER_TABLE_CPU as u8, (LETTER_TABLE_CPU >> 8) as u8, // 0: LDA table,Y
    0xC9, ANCHOR,                                           //  3: CMP #$0A
    0xD0, 0x07,                                             //  5: BNE out
    0x20, ANCHOR_HAS_CPU as u8, (ANCHOR_HAS_CPU >> 8) as u8,//  7: JSR anchor_has
    0x90, 0x02,                                             // 10: BCC out
    0xA9, 0x00,                                             // 12: LDA #$00
    0x60,                                                   // 14: out: RTS
];

/// Install all three hooks and the shared test.
///
/// `house_substitute` is what a Toad House hands over instead of a duplicate
/// Anchor — see [`house`] for why that one cannot simply give nothing.
pub fn apply(rom: &mut Rom, house_substitute: u8) {
    debug_assert_ne!(house_substitute, ANCHOR, "substituting an Anchor for an Anchor");
    rom.push_tag("anchor_dedup");

    rom.write_range(FS_ANCHOR_HAS, &ANCHOR_HAS);
    rom.write_range(FS_ANCHOR_GET_GLUE, &GET_GLUE);
    rom.write_range(FS_ANCHOR_HOUSE, &house(house_substitute));
    rom.write_range(FS_ANCHOR_LETTER, &LETTER);

    // Hammer Bro + in-level chest: jump out of `Player_GetItem`'s head, which
    // is `PHA` + `LDY Player_Current` = 4 bytes. The `JMP` is 3, so it is
    // padded with a `NOP` rather than leaving that `LDY`'s last operand byte
    // behind — `asm::check` rejects a dangling operand, and rightly: nothing
    // reaches it today, but a byte the CPU could decode as an opcode is not
    // something to leave lying in an always-mapped bank.
    rom.write_range(PLAYER_GET_ITEM, &jmp_padded(GET_GLUE_CPU));
    // Toad House: over `TAX / INX / RTS`.
    rom.write_range(CHEST_PRESS_B_TAIL, &jmp(HOUSE_CPU));
    // Princess letter: over the reward-table load.
    rom.write_range(LETTER_LOAD, &jsr(LETTER_CPU));

    rom.pop_tag();
}

const fn jmp(target: u16) -> [u8; 3] {
    [0x4C, target as u8, (target >> 8) as u8]
}

/// [`jmp`] plus a `NOP`, for a hook displacing four bytes of vanilla.
const fn jmp_padded(target: u16) -> [u8; 4] {
    [0x4C, target as u8, (target >> 8) as u8, 0xEA]
}

const fn jsr(target: u16) -> [u8; 3] {
    [0x20, target as u8, (target >> 8) as u8]
}

#[cfg(test)]
mod asm_checks {
    use super::*;
    use crate::randomize::rom_data::asm;

    /// Vanilla bytes the three hooks displace, read from the ROM 2026-09-24.
    pub(super) const VANILLA_GET_ITEM: [u8; 4] = [0x48, 0xAC, 0x26, 0x07];
    pub(super) const VANILLA_HOUSE_TAIL: [u8; 3] = [0xAA, 0xE8, 0x60];
    pub(super) const VANILLA_LETTER_LOAD: [u8; 3] = [0xB9, 0xCE, 0xA0];

    #[test]
    fn anchor_has_is_well_formed() {
        asm::check(&ANCHOR_HAS).allocation(FS_ANCHOR_HAS).origin(ANCHOR_HAS_CPU).assert_ok();
    }

    #[test]
    fn get_glue_is_well_formed() {
        asm::check(&GET_GLUE)
            .allocation(FS_ANCHOR_GET_GLUE)
            .origin(GET_GLUE_CPU)
            .hook(&VANILLA_GET_ITEM, 0, &jmp_padded(GET_GLUE_CPU))
            .assert_ok();
    }

    /// Leaf stands in for "any substitute" — the operand is data to the
    /// decoder, so every non-Anchor value assembles identically.
    pub(super) const TEST_SUBSTITUTE: u8 = 0x03;

    #[test]
    fn house_is_well_formed() {
        asm::check(&house(TEST_SUBSTITUTE))
            .allocation(FS_ANCHOR_HOUSE)
            .origin(HOUSE_CPU)
            .hook(&VANILLA_HOUSE_TAIL, 0, &jmp(HOUSE_CPU))
            .assert_ok();
    }

    #[test]
    fn letter_is_well_formed() {
        asm::check(&LETTER)
            .allocation(FS_ANCHOR_LETTER)
            .origin(LETTER_CPU)
            .hook(&VANILLA_LETTER_LOAD, 0, &jsr(LETTER_CPU))
            .assert_ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_rom() -> Option<Rom> {
        let data = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&data).ok()
    }

    /// **The Toad House substitute is never itself an Anchor.**
    ///
    /// Swapping a duplicate Anchor for an Anchor would leave the duplicate in
    /// place and make the whole hook a no-op. The pool it is drawn from holds
    /// no Anchor — this is the guard for that staying true.
    #[test]
    fn the_substitute_is_never_an_anchor() {
        use rand::SeedableRng;
        for seed in 0..200u64 {
            let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
            let item = crate::randomize::items::toad_house_substitute(&mut rng);
            assert_ne!(item, ANCHOR, "seed {seed} rolled an Anchor as the substitute");
            assert_ne!(item, 0x00, "seed {seed} rolled the undrawable $00");
        }
    }

    /// **The three hook sites hold what this patch thinks they hold.**
    ///
    /// Every hook displaces whole vanilla instructions chosen by hand from the
    /// disassembly. If any of those bytes is not what was read on 2026-09-24,
    /// the offset is wrong and the patch would splice a `JMP` into the middle
    /// of something — which still boots, and then misbehaves.
    #[test]
    fn the_hook_sites_are_where_we_think() {
        let Some(rom) = load_rom() else { return };
        assert_eq!(
            rom.read_range(PLAYER_GET_ITEM, 4),
            &asm_checks::VANILLA_GET_ITEM[..],
            "Player_GetItem head"
        );
        assert_eq!(
            rom.read_range(CHEST_PRESS_B_TAIL, 3),
            &asm_checks::VANILLA_HOUSE_TAIL[..],
            "ToadHouse_ChestPressB tail"
        );
        assert_eq!(
            rom.read_range(LETTER_LOAD, 3),
            &asm_checks::VANILLA_LETTER_LOAD[..],
            "Letter_GiveIncludedItem load"
        );
    }

    /// **The rejoin point is the instruction after the head we displaced.**
    ///
    /// The glue re-does `PHA` and `LDY Player_Current` itself and jumps back to
    /// vanilla's `BEQ`. If that address drifted, the jump would land mid-
    /// instruction.
    #[test]
    fn the_glue_rejoins_a_real_instruction() {
        let Some(rom) = load_rom() else { return };
        // CPU $FD70 -> file: PRG031 is $E000 at 0x3E010.
        let rejoin = 0x3E010 + (PLAYER_GET_ITEM_REJOIN as usize - 0xE000);
        assert_eq!(rejoin, PLAYER_GET_ITEM + 4, "rejoin should follow the displaced head");
        assert_eq!(rom.read_byte(rejoin), 0xF0, "expected vanilla's BEQ at the rejoin");
    }

    /// **Applying it leaves every hook and body exactly where the registry
    /// says**, and does not disturb the instruction after each hook.
    #[test]
    fn apply_writes_only_its_own_bytes() {
        let Some(base) = load_rom() else { return };
        let mut rom = base.clone();
        apply(&mut rom, asm_checks::TEST_SUBSTITUTE);

        assert_eq!(rom.read_range(FS_ANCHOR_HAS, ANCHOR_HAS.len()), &ANCHOR_HAS[..]);
        assert_eq!(rom.read_range(FS_ANCHOR_GET_GLUE, GET_GLUE.len()), &GET_GLUE[..]);
        let house_bytes = house(asm_checks::TEST_SUBSTITUTE);
        assert_eq!(rom.read_range(FS_ANCHOR_HOUSE, house_bytes.len()), &house_bytes[..]);
        assert_eq!(rom.read_range(FS_ANCHOR_LETTER, LETTER.len()), &LETTER[..]);

        assert_eq!(rom.read_range(PLAYER_GET_ITEM, 4), &jmp_padded(GET_GLUE_CPU)[..]);
        assert_eq!(rom.read_range(CHEST_PRESS_B_TAIL, 3), &jmp(HOUSE_CPU)[..]);
        assert_eq!(rom.read_range(LETTER_LOAD, 3), &jsr(LETTER_CPU)[..]);

        // The byte after each hook must still be vanilla.
        for off in [PLAYER_GET_ITEM + 4, CHEST_PRESS_B_TAIL + 3, LETTER_LOAD + 3] {
            assert_eq!(
                rom.read_byte(off),
                base.read_byte(off),
                "clobbered past a hook at {off:#07X}"
            );
        }
    }
}
