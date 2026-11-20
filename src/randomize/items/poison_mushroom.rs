//! Poison Mushroom (`--poison-mushrooms`) — 1-Up blocks may hand out a poison
//! trap instead of a real 1-Up.
//!
//! This replaces MaCobra52's "All 1UPs are Poison Mushrooms" recolor under the
//! same flag. Instead of turning *every* 1-Up into poison, each 1-Up **block**
//! independently gives either the real green 1-Up ($0B) or a purple upside-down
//! poison mushroom ($0A), chosen by a deterministic position hash — so a seed
//! keeps both real 1-Ups and traps, reproducibly.
//!
//! ## Two parts
//!
//! **1. The poison object ($0A)** — installed on the unused group-0 ID $0A (see
//! "Unused Group-0 Objects" in `docs/smb3_rom_reference.md`). It reuses the
//! 1-Up's own handler wholesale: its Norm slot points straight at
//! `ObjNorm_PUp1UpMush` ($A77E), and it adds only a 9-byte Init stub (call the
//! shared 1-Up/Mushroom init, then stamp `SPR_VFLIP` so it draws upside-down)
//! and an 8-byte Hit stub (consume + `JMP Player_GetHurt`). So it rises out of
//! a block, walks, and hit-tests exactly like a 1-Up powerup — but upside-down
//! and hurting on contact. Its palette is the 1-Up's (green); see
//! `POISON_ATTR1` to retint. Verified the V-flip is stable: nothing in the
//! 1-Up Norm path writes `Objects_FlipBits`, and `Object_ShakeAndDrawMirrored`
//! rewrites only bit 6.
//!
//! **2. Which 1-Ups become poison** is decided where a hit block hands out its
//! object, by the spawn hook shared with Suit Blocks — see [`super::block_spawn`]. Each 1-Up block gets a seed-salted hash
//! of its position and the map tile the level was entered from, and bit 0
//! keeps the real 1-Up or swaps in $0A: about half of them, fixed per seed.
//!
//! No block, tile, byte2, or `LL_PowerBlocks`/`LATP` table is touched — the
//! block still "contains a 1-Up"; only the object it hands out changes. So
//! there is no collateral on coin blocks and no `powerups.rs` change.
//!
//! ## Scope
//!
//! Only 1-Ups spawned *from blocks* are affected — white Mushroom House
//! rewards, Coin Ships, and direct enemy-data 1-Ups are untouched. The patch is
//! applied only when the flag is on, so flag-off ROMs are byte-identical.

use crate::rom::Rom;

use crate::randomize::rom_data::FS_POISON_MUSHROOM;

/// New object ID: unused group-0 slot $0A.
const POISON_ID: usize = 0x0A;

/// PRG001 file offset of CPU $A000 (bank 1 is mapped at $A000-$BFFF).
const PRG001_BASE: usize = 0x02010;

/// Per-ID dispatch/attribute table bases (file offsets), from the fixed
/// `.org` addresses in PRG001. See the ROM reference doc.
pub(crate) const INIT_TABLE: usize = PRG001_BASE; // $A000, word per ID
pub(crate) const NORM_TABLE: usize = 0x02058; // $A048, word per ID
pub(crate) const HIT_TABLE: usize = 0x020A0; // $A090, word per ID
pub(crate) const ATTR1_TABLE: usize = 0x020E8; // $A0D8, byte per ID
pub(crate) const ATTR2_TABLE: usize = 0x0210C; // $A0FC, byte per ID
pub(crate) const ATTR3_TABLE: usize = 0x02130; // $A120, byte per ID
/// `ObjectGroup00_PatTableSel`: the sprite page an object asks the level to
/// load (`OPTS_NOCHANGE` = 0 for none).
pub(crate) const PATTABLE_SEL_TABLE: usize = 0x02154; // $A144, byte per ID
pub(crate) const KILL_TABLE: usize = 0x02178; // $A168, byte per ID
pub(crate) const PATSTART_TABLE: usize = 0x0219C; // $A18C, byte per ID

/// 1-Up attribute values copied onto ID $0A. `POISON_ATTR1`'s low two bits are
/// the sprite palette index (2 = the 1-Up's green); change them to retint the
/// mushroom (the level's four sprite sub-palettes decide the actual colors).
const POISON_ATTR1: u8 = 0x12; // palette 2 (green), 16x16
const POISON_ATTR2: u8 = 0x10;
const POISON_ATTR3: u8 = 0x85;
const POISON_KILL: u8 = 0x02;
const POISON_PATSTART: u8 = 0x33;

/// Object-handler entry points (CPU). Init/Hit are our stubs; Norm reuses the
/// 1-Up's `ObjNorm_PUp1UpMush`.
const INIT_CPU: u16 = 0xA703;
const NORM_CPU: u16 = 0xA77E; // ObjNorm_PUp1UpMush — reused verbatim
const HIT_CPU: u16 = 0xA70C;

#[rustfmt::skip]
const HANDLER: [u8; 17] = [
    // PoisonInit ($A703)
    0x20, 0x3A, 0xA8,       // JSR ObjInit_PUpMush ($A83A)
    0xA9, 0x80,             // LDA #SPR_VFLIP
    0x9D, 0x79, 0x06,       // STA Objects_FlipBits,X
    0x60,                   // RTS
    // PoisonHit ($A70C)
    0xA9, 0x00,             // LDA #OBJSTATE_DEADEMPTY
    0x9D, 0x61, 0x06,       // STA Objects_State,X
    0x4C, 0xD3, 0xD9,       // JMP Player_GetHurt ($D9D3)
];

/// Install the poison object on group-0 ID $0A (stubs + per-ID table slots).
/// Only invoked when `--poison-mushrooms` is on; which 1-Ups hand it out is
/// [`super::block_spawn`]'s job.
pub(crate) fn apply(rom: &mut Rom) {
    rom.write_range(FS_POISON_MUSHROOM, &HANDLER);

    rom.write_range(INIT_TABLE + POISON_ID * 2, &INIT_CPU.to_le_bytes());
    rom.write_range(NORM_TABLE + POISON_ID * 2, &NORM_CPU.to_le_bytes());
    rom.write_range(HIT_TABLE + POISON_ID * 2, &HIT_CPU.to_le_bytes());

    rom.write_byte(ATTR1_TABLE + POISON_ID, POISON_ATTR1);
    rom.write_byte(ATTR2_TABLE + POISON_ID, POISON_ATTR2);
    rom.write_byte(ATTR3_TABLE + POISON_ID, POISON_ATTR3);
    rom.write_byte(KILL_TABLE + POISON_ID, POISON_KILL);
    rom.write_byte(PATSTART_TABLE + POISON_ID, POISON_PATSTART);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_rom() -> Rom {
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;
        Rom::from_bytes_lax(&data, true).unwrap()
    }

    #[test]
    fn handler_layout_is_consistent() {
        assert_eq!(FS_POISON_MUSHROOM, PRG001_BASE + (INIT_CPU as usize - 0xA000));
        assert_eq!(HIT_CPU, INIT_CPU + 9);
        assert_eq!(HANDLER.len(), 17);
    }

    #[test]
    fn apply_installs_the_object() {
        let mut rom = make_test_rom();
        apply(&mut rom);

        // Object stubs + table slots.
        assert_eq!(rom.read_range(FS_POISON_MUSHROOM, HANDLER.len()), &HANDLER);
        assert_eq!(rom.read_range(INIT_TABLE + 0x14, 2), &[0x03, 0xA7]); // $A703
        assert_eq!(rom.read_range(NORM_TABLE + 0x14, 2), &[0x7E, 0xA7]); // $A77E (1-Up)
        assert_eq!(rom.read_range(HIT_TABLE + 0x14, 2), &[0x0C, 0xA7]); // $A70C
        assert_eq!(rom.read_byte(ATTR1_TABLE + 0x0A), POISON_ATTR1);
        assert_eq!(rom.read_byte(PATSTART_TABLE + 0x0A), POISON_PATSTART);
    }
}

#[cfg(test)]
mod asm_checks {
    //! Decode each assembled routine and check the structural properties no
    //! assembler was around to enforce. See [`crate::randomize::rom_data::asm`].
    use super::*;
    use crate::randomize::rom_data::asm;

    #[test]
    fn handler_is_well_formed() {
        asm::check(&HANDLER).origin(INIT_CPU).allocation(FS_POISON_MUSHROOM).assert_ok();
    }
}
