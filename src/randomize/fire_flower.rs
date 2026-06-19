//! Random Fire Flower (issue #22).
//!
//! When the player collects an **in-level Fire Flower** the sprite still looks
//! like a Fire Flower, but the power state granted is substituted for one drawn
//! from a small pool. The substitution is a pure deterministic function of game
//! state — `World_Num` plus the flower's absolute level position — so it takes
//! no RNG and bakes no per-seed table: the same flower in the same spot always
//! yields the same suit, on every ROM and for every player. Two flowers in one
//! level can still differ because their positions differ.
//!
//! ## How it works
//!
//! The vanilla in-level Fire Flower grant lives in `ObjHit_FireFlower`
//! (PRG001). For a non-small Mario it runs `LDA #$03 / STA Player_QueueSuit`
//! (`$0578`), i.e. it queues suit 2 (Fire); `Player_QueueSuit` holds
//! `Player_Suit + 1`. We replace that hardcoded store with a `JSR` to an
//! injected routine that computes:
//!
//! ```text
//! index = (salt + World_Num + Objects_XHi,X + Objects_X,X + Objects_Y,X) mod pool_len
//! Player_QueueSuit = POOL[index]
//! ```
//!
//! ## Where the variation comes from
//!
//! - **`salt`** is a single byte baked in at patch-generation time: the
//!   *starting world* of the shuffled progression (the first world in
//!   `world_order`). It is seed-derived, so the whole suit mapping rotates from
//!   seed to seed — which is the only way a level that **never moves worlds**
//!   (e.g. Bowser's castle) can give a different suit across seeds, since every
//!   pure-game-state input is identical for it on every seed. When world order
//!   shuffling is off, the starting world is always 0, so the salt is constant
//!   and fixed levels resolve to the same suit across seeds.
//! - **`World_Num`** (`$0727`) spreads different worlds apart within a single
//!   playthrough.
//! - The three object arrays (all indexed by the colliding object's slot in
//!   `X`) are the flower's *absolute* level coordinates — stable regardless of
//!   camera scroll — so each flower tile resolves to one fixed suit, and two
//!   flowers in the same level differ.
//!
//! Because the salt is baked into the ROM, the result is fully deterministic
//! for a given seed (every player gets the same suit from a given flower) with
//! no live/in-game RNG.
//!
//! The hook mirrors the community "Random Fire Flower" patch's site but diverts
//! to our own routine and uses the starting-world salt + `World_Num` (the
//! original used `Level_LayPtr_AddrL`, which ties the suit to level *content*).
//!
//! ## Scope / caveats
//!
//! - **In-level Fire Flower object only** (this also covers a Fire Flower popped
//!   from a `?` block, since it becomes the same object). Inventory item use is
//!   untouched.
//! - **Small Mario is unchanged:** vanilla sends a small Mario down the mushroom
//!   path (grants Super, not a suit), and we don't touch that branch. So the
//!   substitution applies only once the player is at least Super. For `Wild`
//!   this means the downgrade outcomes (Small/Big) can only ever *reduce* a
//!   big-or-better Mario — a small Mario can't get "more small".

use crate::randomizer::FireFlowerMode;
use crate::rom::Rom;

use super::rom_data::{FIRE_FLOWER_SUB_CPU, FS_FIRE_FLOWER};
use super::world_order::WORLD_INIT_OPERAND;

/// File offset of the suit-store inside `ObjHit_FireFlower` (PRG001). The 12
/// bytes here are vanilla `BEQ +0x0A / LDA #$1F / STA $0555 / LDA #$03 /
/// STA $0578` — the `BEQ` "already Fire, skip" early-out followed by the
/// transition-sparkle write and the hardcoded Fire store.
const HOOK_SITE: usize = 0x02A17;

// `Player_QueueSuit` values (= `Player_Suit` + 1) for each power state.
const Q_SMALL: u8 = 0x01; // Small Mario
const Q_BIG: u8 = 0x02; // Super (big) Mario
const Q_FIRE: u8 = 0x03; // Fire Mario
const Q_FROG: u8 = 0x05; // Frog Suit
const Q_TANOOKI: u8 = 0x06; // Tanooki Suit
const Q_HAMMER: u8 = 0x07; // Hammer Suit

/// `On` pool: the four "safe" big-form suits. Raccoon is deliberately excluded.
const POOL_ON: &[u8] = &[Q_FIRE, Q_FROG, Q_TANOOKI, Q_HAMMER];
/// `Wild` pool: adds the two downgrade outcomes (Small, Big) to the `On` pool.
const POOL_WILD: &[u8] = &[Q_SMALL, Q_BIG, Q_FIRE, Q_FROG, Q_TANOOKI, Q_HAMMER];

/// Length of the injected routine code (excluding the trailing pool table).
const ROUTINE_LEN: u16 = 28;

/// Install the Random Fire Flower patch. `Off` is a no-op.
///
/// Must run after [`super::world_order`] so the starting-world salt is read from
/// its final value (the orchestrator guarantees this ordering).
pub fn apply(rom: &mut Rom, mode: FireFlowerMode) {
    let pool: &[u8] = match mode {
        FireFlowerMode::Off => return,
        FireFlowerMode::On => POOL_ON,
        FireFlowerMode::Wild => POOL_WILD,
    };
    let n = pool.len() as u8;

    // Seed-derived salt: the starting world of the shuffled progression (0 when
    // world order shuffling is off). Baked in as an immediate so the result is
    // fully deterministic per seed but rotates seed-to-seed.
    let salt = rom.read_byte(WORLD_INIT_OPERAND);

    // The pool table sits immediately after the routine code.
    let table_cpu = FIRE_FLOWER_SUB_CPU + ROUTINE_LEN;
    let tlo = (table_cpu & 0xFF) as u8;
    let thi = (table_cpu >> 8) as u8;

    // Injected routine (X = colliding object's slot index):
    //   LDA #salt        ; seed-derived starting-world salt
    //   CLC
    //   ADC $0727        ; + World_Num  (current world)
    //   ADC $76,X        ; + Objects_XHi,X  (screen number, absolute)
    //   ADC $91,X        ; + Objects_X,X    (low X within screen, absolute)
    //   ADC $A3,X        ; + Objects_Y,X    (absolute Y)
    // modloop:
    //   CMP #n           ; reduce the (carry-folded) sum mod pool_len
    //   BCC moddone
    //   SBC #n
    //   BCS modloop
    // moddone:
    //   TAY
    //   LDA POOL,Y       ; Player_QueueSuit value for this flower
    //   STA $0578        ; Player_QueueSuit
    //   RTS
    #[rustfmt::skip]
    let mut code: Vec<u8> = vec![
        0xA9, salt,         // LDA #salt
        0x18,               // CLC
        0x6D, 0x27, 0x07,   // ADC $0727  (World_Num)
        0x75, 0x76,         // ADC $76,X  (Objects_XHi,X)
        0x75, 0x91,         // ADC $91,X  (Objects_X,X)
        0x75, 0xA3,         // ADC $A3,X  (Objects_Y,X)
        0xC9, n,            // CMP #n            (modloop @ +12)
        0x90, 0x04,         // BCC +4 -> moddone (+20)
        0xE9, n,            // SBC #n
        0xB0, 0xF8,         // BCS -8 -> modloop (+12)
        0xA8,               // TAY               (moddone @ +20)
        0xB9, tlo, thi,     // LDA POOL,Y
        0x8D, 0x78, 0x05,   // STA $0578  (Player_QueueSuit)
        0x60,               // RTS
    ];
    debug_assert_eq!(code.len() as u16, ROUTINE_LEN);
    code.extend_from_slice(pool);
    rom.write_range(FS_FIRE_FLOWER, &code);

    // Hook ObjHit_FireFlower: NOP the "already Fire" early-out (so the suit is
    // always recomputed) and the hardcoded `LDA #$03`, keep the transition
    // sparkle (`LDA #$1F / STA $0555`), and divert the suit store to our
    // routine. Execution falls through afterward into the vanilla
    // collect/sound tail at $AA13.
    let lo = (FIRE_FLOWER_SUB_CPU & 0xFF) as u8;
    let hi = (FIRE_FLOWER_SUB_CPU >> 8) as u8;
    #[rustfmt::skip]
    rom.write_range(HOOK_SITE, &[
        0xEA, 0xEA,                     // NOP NOP   (was BEQ "already Fire")
        0xA9, 0x1F, 0x8D, 0x55, 0x05,   // LDA #$1F / STA $0555  (transition sparkle)
        0xEA, 0xEA,                     // NOP NOP   (was LDA #$03)
        0x20, lo, hi,                   // JSR fire_flower routine
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rom::Rom;

    /// Mirror the injected 6502 routine in Rust to confirm the table index math.
    fn sim(salt: u8, world: u8, xhi: u8, x: u8, y: u8, pool: &[u8]) -> u8 {
        let sum = salt
            .wrapping_add(world)
            .wrapping_add(xhi)
            .wrapping_add(x)
            .wrapping_add(y);
        pool[(sum % pool.len() as u8) as usize]
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

    #[test]
    fn off_is_noop() {
        let mut rom = blank_rom();
        let before = rom.read_range(0, 393232).to_vec();
        apply(&mut rom, FireFlowerMode::Off);
        assert_eq!(rom.read_range(0, 393232), &before[..], "Off must not touch the ROM");
    }

    #[test]
    fn on_writes_hook_and_routine() {
        let mut rom = blank_rom();
        apply(&mut rom, FireFlowerMode::On);

        // Hook: BEQ + LDA#$03 NOP'd, JSR to the routine.
        let lo = (FIRE_FLOWER_SUB_CPU & 0xFF) as u8;
        let hi = (FIRE_FLOWER_SUB_CPU >> 8) as u8;
        assert_eq!(
            rom.read_range(HOOK_SITE, 12),
            &[0xEA, 0xEA, 0xA9, 0x1F, 0x8D, 0x55, 0x05, 0xEA, 0xEA, 0x20, lo, hi],
        );
        // CMP immediate is the pool length; table follows the routine.
        assert_eq!(rom.read_byte(FS_FIRE_FLOWER + 12), 0xC9);
        assert_eq!(rom.read_byte(FS_FIRE_FLOWER + 13), POOL_ON.len() as u8);
        assert_eq!(
            rom.read_range(FS_FIRE_FLOWER + ROUTINE_LEN as usize, POOL_ON.len()),
            POOL_ON,
        );
    }

    #[test]
    fn wild_uses_six_entry_pool() {
        let mut rom = blank_rom();
        apply(&mut rom, FireFlowerMode::Wild);
        assert_eq!(rom.read_byte(FS_FIRE_FLOWER + 13), POOL_WILD.len() as u8);
        assert_eq!(
            rom.read_range(FS_FIRE_FLOWER + ROUTINE_LEN as usize, POOL_WILD.len()),
            POOL_WILD,
        );
    }

    #[test]
    fn routine_stays_within_allocation() {
        // 28-byte routine + the larger (Wild) 6-byte table must fit the 36-byte
        // reservation and not spill past the PRG001 bank end (0x4010).
        const _: () = assert!(ROUTINE_LEN as usize + POOL_WILD.len() <= 36);
        const _: () = assert!(FS_FIRE_FLOWER + 36 <= 0x4010);
    }

    #[test]
    fn index_is_deterministic_and_in_pool() {
        // Same inputs -> same suit; result always a pool member.
        for &pool in &[POOL_ON, POOL_WILD] {
            for salt in 0..8u8 {
                for world in 0..8u8 {
                    for &(xhi, x, y) in &[(0u8, 0u8, 0u8), (1, 0x40, 0x80), (15, 0xFF, 0xFF)] {
                        let a = sim(salt, world, xhi, x, y, pool);
                        let b = sim(salt, world, xhi, x, y, pool);
                        assert_eq!(a, b);
                        assert!(pool.contains(&a));
                    }
                }
            }
        }
    }
}
