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
//! **2. The spawn hook** — the random-fire-flower technique, applied to block
//! *spawns* instead of collects. When a hit block resolves its contents it runs
//! `LDA Bouncer_PUp,Y` → `STA Level_ObjectID,Y` (`99 71 06`) at one of two
//! PRG001 sites; every block-spawned 1-Up (brick-1up *and* invis-1up both
//! resolve to `Bouncer_PUp[7]=$0B`) passes through one of them, with the
//! block's world position sitting in `Objects_X/XHi/Y,X`. We replace that
//! 3-byte store with `JSR` to a routine that replicates the store, and if the
//! object is a 1-Up, position-hashes to keep $0B or swap to $0A.
//!
//! No block, tile, byte2, or `LL_PowerBlocks`/`LATP` table is touched — the
//! block still "contains a 1-Up"; we just redirect the object it hands out. So
//! there is no collateral on coin blocks and no `powerups.rs` change.
//!
//! ## Determinism
//!
//! The hash is `salt + World_Num + Level_LayPtr_AddrL + Objects_XHi + Objects_X
//! + Objects_Y`, all stable at spawn (the block's fixed tile position). `salt`
//! is the seed-derived shuffled starting world (`WORLD_INIT_OPERAND`, same
//! source as random fire flower), baked in as an immediate — so the poison/real
//! layout is fixed for a seed and rotates seed-to-seed. `AND #$01` gives a
//! ~50/50 split per block; see `POISON_RATE_MASK` to retune.
//!
//! ## Scope
//!
//! Only 1-Ups spawned *from blocks* are affected — white Mushroom House
//! rewards, Coin Ships, and direct enemy-data 1-Ups are untouched. The patch is
//! applied only when the flag is on, so flag-off ROMs are byte-identical.

use crate::rom::Rom;

use super::rom_data::{FS_POISON_HOOK, FS_POISON_MUSHROOM};
use super::world_order::WORLD_INIT_OPERAND;

/// New object ID: unused group-0 slot $0A.
const POISON_ID: usize = 0x0A;

/// PRG001 file offset of CPU $A000 (bank 1 is mapped at $A000-$BFFF).
const PRG001_BASE: usize = 0x02010;

/// Per-ID dispatch/attribute table bases (file offsets), from the fixed
/// `.org` addresses in PRG001. See the ROM reference doc.
const INIT_TABLE: usize = PRG001_BASE; // $A000, word per ID
const NORM_TABLE: usize = 0x02058; // $A048, word per ID
const HIT_TABLE: usize = 0x020A0; // $A090, word per ID
const ATTR1_TABLE: usize = 0x020E8; // $A0D8, byte per ID
const ATTR2_TABLE: usize = 0x0210C; // $A0FC, byte per ID
const ATTR3_TABLE: usize = 0x02130; // $A120, byte per ID
const KILL_TABLE: usize = 0x02178; // $A168, byte per ID
const PATSTART_TABLE: usize = 0x0219C; // $A18C, byte per ID

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

/// The vanilla `STA Level_ObjectID,Y` at both block-spawn sites.
const SPAWN_STORE: [u8; 3] = [0x99, 0x71, 0x06];
/// File offsets of the two `STA Level_ObjectID,Y` block-spawn sites (PRG001
/// $A587 / $AADD). Both flow through `Bouncer_PUp`, covering brick-1up and
/// invis-1up alike.
const SPAWN_SITES: [usize; 2] = [0x02597, 0x02AED];

/// CPU address of the spawn-hook routine (= FS_POISON_HOOK mapped into bank 1).
const HOOK_CPU: u16 = 0xA714;

/// 1-Up object ID (`Bouncer_PUp[7]`), the value the hook watches for.
const ONE_UP_ID: u8 = 0x0B;

/// Mask applied to the position hash: `AND #$01` → ~50% of 1-Up blocks become
/// poison. Use `0x03` for ~25% only-when-zero style tuning (would also flip the
/// branch); kept at 1 bit for a clean even split.
const POISON_RATE_MASK: u8 = 0x01;

/// Length of the injected hook routine.
const HOOK_LEN: usize = 31;

/// Apply the Poison Mushroom feature: install the $0A object, install the
/// 1-Up-spawn hook, and patch both block-spawn sites to call it. Only invoked
/// when `--poison-mushrooms` is on.
///
/// Must run after [`super::world_order`] so the seed-derived salt is read from
/// its final value (the orchestrator guarantees this ordering).
pub fn apply(rom: &mut Rom) {
    install_object(rom);
    install_hook(rom);
}

/// Install the poison object on group-0 ID $0A (stubs + per-ID table slots).
fn install_object(rom: &mut Rom) {
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

/// Install the spawn-hook routine and patch both spawn sites to call it.
fn install_hook(rom: &mut Rom) {
    // Seed-derived salt: the shuffled starting world (0 with world-order off),
    // baked in so the poison layout is deterministic per seed but rotates
    // seed-to-seed. Same source as random fire flower.
    let salt = rom.read_byte(WORLD_INIT_OPERAND);

    // Injected routine. Entry: A = object ID (just loaded from Bouncer_PUp),
    // Y = 5 (spawn slot), X = the hit block's object slot (its world position
    // lives in Objects_*,X). X and Y are preserved; A is free (the caller
    // reloads it on the next instruction).
    //
    //   STA $0671,Y      ; replicate the original store (Level_ObjectID[5] = A)
    //   CMP #$0B         ; a 1-Up?
    //   BNE done         ; no -> leave whatever it was
    //   LDA #salt
    //   CLC
    //   ADC $0727        ; + World_Num
    //   ADC $61          ; + Level_LayPtr_AddrL  (per-area constant)
    //   ADC $76,X        ; + Objects_XHi,X  (block screen)
    //   ADC $91,X        ; + Objects_X,X    (block fine X)
    //   ADC $A3,X        ; + Objects_Y,X    (block fine Y)
    //   AND #POISON_RATE_MASK
    //   BEQ done         ; hash even -> keep the 1-Up
    //   LDA #$0A         ; else swap in the poison object
    //   STA $0671,Y
    // done:
    //   RTS
    #[rustfmt::skip]
    let code: [u8; HOOK_LEN] = [
        0x99, 0x71, 0x06,       // STA Level_ObjectID,Y
        0xC9, ONE_UP_ID,        // CMP #$0B
        0xD0, 0x17,             // BNE done (+0x17 -> RTS)
        0xA9, salt,             // LDA #salt
        0x18,                   // CLC
        0x6D, 0x27, 0x07,       // ADC $0727  (World_Num)
        0x65, 0x61,             // ADC $61    (Level_LayPtr_AddrL)
        0x75, 0x76,             // ADC $76,X  (Objects_XHi,X)
        0x75, 0x91,             // ADC $91,X  (Objects_X,X)
        0x75, 0xA3,             // ADC $A3,X  (Objects_Y,X)
        0x29, POISON_RATE_MASK, // AND #mask
        0xF0, 0x05,             // BEQ done (+0x05 -> RTS)
        0xA9, POISON_ID as u8,  // LDA #$0A
        0x99, 0x71, 0x06,       // STA Level_ObjectID,Y
        0x60,                   // RTS  (done)
    ];
    rom.write_range(FS_POISON_HOOK, &code);

    // Patch both `STA Level_ObjectID,Y` sites to `JSR HOOK`.
    let jsr = [0x20, (HOOK_CPU & 0xFF) as u8, (HOOK_CPU >> 8) as u8];
    for &site in &SPAWN_SITES {
        debug_assert_eq!(rom.read_range(site, 3), SPAWN_STORE);
        rom.write_range(site, &jsr);
    }
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
        // Seed both spawn sites with the vanilla store so the site patch and
        // its debug-assert have real bytes to replace.
        for &site in &SPAWN_SITES {
            data[site..site + 3].copy_from_slice(&SPAWN_STORE);
        }
        Rom::from_bytes_lax(&data, true).unwrap()
    }

    #[test]
    fn handler_layout_is_consistent() {
        assert_eq!(FS_POISON_MUSHROOM, PRG001_BASE + (INIT_CPU as usize - 0xA000));
        assert_eq!(FS_POISON_HOOK, PRG001_BASE + (HOOK_CPU as usize - 0xA000));
        assert_eq!(HIT_CPU, INIT_CPU + 9);
        assert_eq!(HANDLER.len(), 17);
        // Hook block must sit after the stub block, not overlap it.
        assert!(FS_POISON_HOOK >= FS_POISON_MUSHROOM + HANDLER.len());
    }

    #[test]
    fn branch_targets_land_on_the_rts() {
        // Both forward branches must target the final RTS at offset HOOK_LEN-1.
        let rts = (HOOK_LEN - 1) as u8;
        // BNE opcode at offset 5 -> next instruction at 7; 7 + 0x17 == 30
        assert_eq!(7 + 0x17, rts);
        // BEQ opcode at offset 23 -> next instruction at 25; 25 + 0x05 == 30
        assert_eq!(25 + 0x05, rts);
    }

    #[test]
    fn apply_installs_object_and_hook() {
        let mut rom = make_test_rom();
        apply(&mut rom);

        // Object stubs + table slots.
        assert_eq!(rom.read_range(FS_POISON_MUSHROOM, HANDLER.len()), &HANDLER);
        assert_eq!(rom.read_range(INIT_TABLE + 0x14, 2), &[0x03, 0xA7]); // $A703
        assert_eq!(rom.read_range(NORM_TABLE + 0x14, 2), &[0x7E, 0xA7]); // $A77E (1-Up)
        assert_eq!(rom.read_range(HIT_TABLE + 0x14, 2), &[0x0C, 0xA7]); // $A70C
        assert_eq!(rom.read_byte(ATTR1_TABLE + 0x0A), POISON_ATTR1);
        assert_eq!(rom.read_byte(PATSTART_TABLE + 0x0A), POISON_PATSTART);

        // Hook routine: right prologue and it watches for the 1-Up ID.
        let hook = rom.read_range(FS_POISON_HOOK, HOOK_LEN);
        assert_eq!(&hook[0..5], &[0x99, 0x71, 0x06, 0xC9, ONE_UP_ID]);
        assert_eq!(hook[HOOK_LEN - 1], 0x60); // trailing RTS

        // Both spawn sites now JSR the hook.
        let jsr = [0x20, (HOOK_CPU & 0xFF) as u8, (HOOK_CPU >> 8) as u8];
        for &site in &SPAWN_SITES {
            assert_eq!(rom.read_range(site, 3), &jsr);
        }
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
