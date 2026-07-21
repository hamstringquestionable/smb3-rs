//! Random Lakitu egg type.
//!
//! Vanilla decides what a Lakitu throws from the level-wide `Level_SlopeEn`
//! flag (a tileset property): non-sloped levels get the real **Spiny Egg**
//! (`OBJ_SPINYEGG`, red, hatches into a chasing Spiny), sloped levels (Hills /
//! Underground tilesets) get the harmless **green "dud" egg** (`OBJ_SPINYEGGDUD`,
//! palette 2, never hatches). Because wild injections land in ordinary
//! (non-sloped) levels, an injected Lakitu always throws the real red egg.
//!
//! This patch substitutes that hardcoded, tileset-driven choice for a
//! position-hash — the same deterministic-without-RNG trick as
//! [`super::fire_flower`]. Each Lakitu's egg type becomes a pure function of
//! stable game state plus a seed-derived salt, so the same seed always plays the
//! same way with no live/console RNG, and different levels can differ.
//!
//! ## How it works
//!
//! The egg-select inside `Lakitu_TossEnemy` (PRG004) is vanilla:
//!
//! ```text
//! LDA #$00 / CMP Level_SlopeEn        ; carry set iff SlopeEn == 0
//! LDA #OBJ_SPINYEGG ($84)             ; assume real egg
//! BGE +7                              ; SlopeEn == 0 -> keep real egg
//! LDA #$02 / STA Objects_SprAttr,Y    ; else palette 2
//! LDA #OBJ_SPINYEGGDUD ($85)          ; ...and the green dud egg
//! ```
//!
//! We overwrite that 16-byte block with a `JSR` to an injected routine (padded
//! with NOPs) that computes, with `Y` still holding the egg's object slot:
//!
//! ```text
//! low_bit_of(salt + World_Num + Level_LayPtr_AddrL) == 0 -> real red egg
//!                                                    == 1 -> green dud egg
//! ```
//!
//! and returns `A` = the chosen object id (setting `Objects_SprAttr,Y` to
//! palette 2 for the dud). The block falls through into the vanilla
//! `STA Level_ObjectID,Y` exactly as before.
//!
//! ## Why these inputs (and only these)
//!
//! The hash must use values that are constant for the whole level, or a single
//! Lakitu would flip egg types as it moves — a Lakitu drifts across screens as
//! it chases, so its live position is NOT stable (the same determinism trap
//! documented in [`super::fire_flower`]). We therefore key only on:
//!
//! - **`salt`** — the shuffled starting world (`world_order`'s
//!   [`WORLD_INIT_OPERAND`]); 0 when world-order shuffle is off. Seed-derived, so
//!   the mapping rotates seed to seed.
//! - **`World_Num`** (`$0727`) — spreads worlds apart; constant within a level.
//! - **`Level_LayPtr_AddrL`** (`$61`) — the level layout pointer low byte, a
//!   per-area constant set at level load. Distinguishes levels/areas.
//!
//! SMB3 only supports one active Lakitu at a time (`Lakitu_Active` is a single
//! global), so per-(seed, world, area) resolution is effectively per-Lakitu.
//!
//! ## Scope
//!
//! Patches the shared toss routine, so **every** Lakitu in the ROM is affected
//! (native and wild-injected alike) — deliberate, see the enemy-injection
//! feature. The toss *cadence* (also keyed on `Level_SlopeEn`) is left untouched;
//! only the egg type changes.

use crate::rom::Rom;

use super::rom_data::{FS_LAKITU_EGG, LAKITU_EGG_SUB_CPU};
use super::world_order::WORLD_INIT_OPERAND;

/// File offset of the egg-select block inside `Lakitu_TossEnemy` (PRG004). The
/// 16 vanilla bytes here are `LDA #$00 / CMP $0563 / LDA #$84 / BGE +7 /
/// LDA #$02 / STA $7FE7,Y / LDA #$85` — everything up to (but not including) the
/// shared `STA $0671,Y` that stores the chosen object id.
const HOOK_SITE: usize = 0x08E58;

/// The 16 vanilla bytes we overwrite, asserted before patching so a shifted ROM
/// can't be silently mispatched.
#[rustfmt::skip]
const HOOK_VANILLA: [u8; 16] = [
    0xA9, 0x00,             // LDA #$00
    0xCD, 0x63, 0x05,       // CMP $0563  (Level_SlopeEn)
    0xA9, 0x84,             // LDA #$84   (OBJ_SPINYEGG)
    0xB0, 0x07,             // BGE +7
    0xA9, 0x02,             // LDA #$02
    0x99, 0xE7, 0x7F,       // STA $7FE7,Y (Objects_SprAttr,Y)
    0xA9, 0x85,             // LDA #$85   (OBJ_SPINYEGGDUD)
];

const OBJ_SPINYEGG: u8 = 0x84; // real egg (hatches into a Spiny), keeps SPR_PAL1
const OBJ_SPINYEGGDUD: u8 = 0x85; // green dud egg (never hatches), palette 2
const DUD_PALETTE: u8 = 0x02; // Objects_SprAttr value the vanilla dud path sets

/// Length of the injected routine.
const ROUTINE_LEN: u16 = 25;

/// Install the Random Lakitu Egg patch. `enabled == false` is a no-op.
///
/// Must run after [`super::world_order`] so the starting-world salt is read from
/// its final value (the orchestrator guarantees this ordering — it sits next to
/// the `fire_flower` call, which shares the same salt).
pub fn apply(rom: &mut Rom, enabled: bool) {
    if !enabled {
        return;
    }

    // Seed-derived salt: the shuffled starting world (0 when world-order shuffle
    // is off). Baked as an immediate, so the mapping is deterministic per seed.
    let salt = rom.read_byte(WORLD_INIT_OPERAND);

    // Injected routine. Y still holds the egg object's slot on entry and is left
    // untouched. Every summed input is constant for the whole level (see the
    // module doc) so the egg type is stable across a Lakitu's tosses.
    //   LDA #salt
    //   CLC
    //   ADC $0727        ; + World_Num
    //   ADC $61          ; + Level_LayPtr_AddrL
    //   AND #$01         ; coin flip: low bit of the sum
    //   BEQ real         ; 0 -> real egg
    //   LDA #$02
    //   STA $7FE7,Y      ; Objects_SprAttr,Y = palette 2 (green)
    //   LDA #OBJ_SPINYEGGDUD
    //   SEC              ; downstream SBC #12 (egg spawn Y) expects carry set
    //   RTS
    // real:
    //   LDA #OBJ_SPINYEGG ; SprAttr already SPR_PAL1 from before the hook
    //   SEC
    //   RTS
    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        0xA9, salt,             // LDA #salt
        0x18,                   // CLC
        0x6D, 0x27, 0x07,       // ADC $0727  (World_Num)
        0x65, 0x61,             // ADC $61    (Level_LayPtr_AddrL)
        0x29, 0x01,             // AND #$01
        0xF0, 0x09,             // BEQ +9 -> real
        0xA9, DUD_PALETTE,      // LDA #$02
        0x99, 0xE7, 0x7F,       // STA $7FE7,Y (Objects_SprAttr,Y)
        0xA9, OBJ_SPINYEGGDUD,  // LDA #$85
        0x38,                   // SEC
        0x60,                   // RTS
        0xA9, OBJ_SPINYEGG,     // LDA #$84   (real @ +9 from the BEQ)
        0x38,                   // SEC
        0x60,                   // RTS
    ];
    debug_assert_eq!(code.len() as u16, ROUTINE_LEN);
    rom.write_range(FS_LAKITU_EGG, &code);

    // Divert the egg-select to our routine. The JSR (3 bytes) is NOP-padded to
    // fill the 16-byte vanilla block; A (the chosen object id) and Y survive the
    // NOPs and flow into the vanilla STA $0671,Y that follows.
    assert_eq!(
        rom.read_range(HOOK_SITE, HOOK_VANILLA.len()),
        &HOOK_VANILLA[..],
        "Lakitu egg-select hook site does not match vanilla bytes",
    );
    let lo = (LAKITU_EGG_SUB_CPU & 0xFF) as u8;
    let hi = (LAKITU_EGG_SUB_CPU >> 8) as u8;
    let mut hook = vec![0x20, lo, hi]; // JSR routine
    hook.resize(HOOK_VANILLA.len(), 0xEA); // NOP pad to 16 bytes
    rom.write_range(HOOK_SITE, &hook);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rom::Rom;

    /// Mirror the injected routine's coin flip: low bit of the stable-input sum.
    /// `true` = green dud egg, `false` = real red egg.
    fn is_dud(salt: u8, world: u8, layptr: u8) -> bool {
        salt.wrapping_add(world).wrapping_add(layptr) & 1 == 1
    }

    fn blank_rom() -> Rom {
        // Minimal valid iNES image: header + 256 KiB PRG + 128 KiB CHR, zeroed.
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16; // PRG pages
        data[5] = 16; // CHR pages
        data[6] = 0x40; // mapper 4 lower nibble
        Rom::from_bytes_lax(&data, true).unwrap()
    }

    /// Plant the vanilla egg-select bytes so `apply` can assert against them.
    fn rom_with_hook() -> Rom {
        let mut rom = blank_rom();
        rom.write_range(HOOK_SITE, &HOOK_VANILLA);
        rom
    }

    #[test]
    fn disabled_is_noop() {
        let mut rom = rom_with_hook();
        let before = rom.read_range(0, 393232).to_vec();
        apply(&mut rom, false);
        assert_eq!(rom.read_range(0, 393232), &before[..], "disabled must not touch the ROM");
    }

    #[test]
    fn enabled_writes_hook_and_routine() {
        let mut rom = rom_with_hook();
        apply(&mut rom, true);

        // Hook: JSR to the routine, then NOP padding to the full 16 bytes.
        let lo = (LAKITU_EGG_SUB_CPU & 0xFF) as u8;
        let hi = (LAKITU_EGG_SUB_CPU >> 8) as u8;
        let hook = rom.read_range(HOOK_SITE, HOOK_VANILLA.len());
        assert_eq!(&hook[0..3], &[0x20, lo, hi], "JSR to routine");
        assert!(hook[3..].iter().all(|&b| b == 0xEA), "rest is NOP padding");

        // Routine length + terminating RTS.
        assert_eq!(
            rom.read_range(FS_LAKITU_EGG, ROUTINE_LEN as usize).len(),
            ROUTINE_LEN as usize,
        );
        assert_eq!(rom.read_byte(FS_LAKITU_EGG + ROUTINE_LEN as usize - 1), 0x60);
        // Salt immediate is the starting world (0 in a blank ROM).
        assert_eq!(rom.read_byte(FS_LAKITU_EGG), 0xA9);
        assert_eq!(rom.read_byte(FS_LAKITU_EGG + 1), 0x00);
    }

    #[test]
    fn wrong_salt_reflected_in_routine() {
        // The salt immediate must track WORLD_INIT_OPERAND, not a constant.
        let mut rom = rom_with_hook();
        rom.write_byte(WORLD_INIT_OPERAND, 0x05);
        apply(&mut rom, true);
        assert_eq!(rom.read_byte(FS_LAKITU_EGG + 1), 0x05);
    }

    #[test]
    fn coin_flip_is_deterministic_and_both_outcomes_occur() {
        // Same inputs -> same egg; across realistic inputs we see both eggs.
        let mut saw_real = false;
        let mut saw_dud = false;
        for salt in 0..9u8 {
            for world in 0..9u8 {
                for layptr in [0x00u8, 0x14, 0x2B, 0x40, 0x7F, 0xC3, 0xFE] {
                    let a = is_dud(salt, world, layptr);
                    let b = is_dud(salt, world, layptr);
                    assert_eq!(a, b, "must be deterministic");
                    saw_real |= !a;
                    saw_dud |= a;
                }
            }
        }
        assert!(saw_real && saw_dud, "both egg types must be reachable");
    }
}
