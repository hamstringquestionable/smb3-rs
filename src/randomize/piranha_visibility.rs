//! Always-visible piranha plants when wild-shuffle mixes them with other classes.
//!
//! Vanilla piranha plants spawn in state `Objects_Var4 = 0` (HideInPipe), which
//! skips the draw call entirely until a timer + "Mario not too close" check
//! advances them to state 1 (Emerge). That's correct behavior for pipe-mounted
//! piranhas — the hide phase lets Mario stand next to the pipe safely.
//!
//! In wild-shuffle mode, however, piranha IDs (`0xA0–0xA7`, `0x7D`, `0x7F`) can
//! land in slots that previously held a ground enemy. The piranha keeps its
//! invisible HideInPipe initial state, so the player sees nothing — and then
//! the plant pops into view once they walk past, which feels unfair.
//!
//! This patch primes `Var4 = 1` (Emerge) at the end of the init routine so
//! every piranha is visible from spawn. The proximity gate is bypassed; vanilla
//! pipe placements still rise normally from the pipe-mouth Y, because Init
//! places them at pipe-mouth Y regardless.
//!
//! The patch is applied only when the seed actually permits a piranha→non-piranha
//! cross (or vice versa): `opts.piranhas == Wild` **and** at least one other
//! enemy class is also Wild. In all other configurations the wild pool either
//! does not contain piranhas, or contains only piranhas, so the visibility bug
//! cannot occur and vanilla behavior is preserved.
//!
//! ## Patch layout
//!
//! Two bank-local thunks, one per piranha bank:
//!
//! - **PRG005** (`0xA0–0xA7`) — shared init tail at file `0x0A662..0x0A664`
//!   ends with `STA <$91,X / RTS`. Replace those 3 bytes with `JMP $BFC6`.
//!   The thunk at file `0x0BFD6` (cpu `$BFC6`) re-does the displaced
//!   `STA <$91,X`, writes `Var4 = 1`, and returns.
//!
//! - **PRG004** (`0x7D`, `0x7F`) — shared init tail at file `0x09783..0x09786`
//!   ends with `INC $7FF7,X / RTS` (the `IsGiant` flag bump). Replace those
//!   4 bytes with `JMP $BE56 / RTS` (RTS is dead-code filler after the JMP).
//!   The thunk at file `0x09E66` (cpu `$BE56`) re-does the displaced
//!   `INC $7FF7,X`, writes `Var4 = 1`, and returns.
//!
//! `Objects_Var4` is zero-page `$7F` — verified from the dispatch instruction
//! `LDA <$7F,X / AND #$03` at the top of `ObjNorm_Piranha` in both banks.

use crate::rom::Rom;
use crate::randomizer::{EnemyMode, Options};
use super::rom_data::{
    FS_PIRANHA_VIS_BIG, FS_PIRANHA_VIS_SMALL,
    PIRANHA_VIS_BIG_CPU, PIRANHA_VIS_SMALL_CPU,
};

/// File offset of the small-piranha init tail's last 3 bytes (`STA <$91,X / RTS`).
const SMALL_PIRANHA_INIT_TAIL: usize = 0x0A662;

/// File offset of the big-piranha init tail's last 4 bytes (`INC $7FF7,X / RTS`).
const BIG_PIRANHA_INIT_TAIL: usize = 0x09783;

/// Apply the visibility patch when wild-shuffle can mix piranhas with other
/// classes. No-op otherwise.
pub fn apply(rom: &mut Rom, opts: &Options) {
    if !should_apply(opts) {
        return;
    }
    rom.push_tag("piranha_visibility");
    patch_small_piranha(rom);
    patch_big_piranha(rom);
    rom.pop_tag();
}

/// True when piranhas are Wild and at least one other class is also Wild,
/// meaning the shared wild pool can move piranha IDs into non-piranha slots
/// or other IDs into piranha slots. Cannons are excluded because their wild
/// pool is self-contained (see `build_wild_pool` in `enemies.rs`).
fn should_apply(opts: &Options) -> bool {
    if opts.piranhas != EnemyMode::Wild {
        return false;
    }
    [
        opts.ground, opts.shell, opts.flying, opts.ghosts,
        opts.thwomps, opts.rotodiscs, opts.water, opts.bros,
    ]
    .contains(&EnemyMode::Wild)
}

fn patch_small_piranha(rom: &mut Rom) {
    // Sanity check: confirm the 3 bytes we're displacing are still the vanilla
    // `STA <$91,X / RTS`. Catches ROM-version mismatches or upstream relocations.
    debug_assert_eq!(
        rom.read_range(SMALL_PIRANHA_INIT_TAIL, 3),
        &[0x95, 0x91, 0x60],
        "small piranha init tail does not match expected bytes — ROM mismatch?",
    );

    // Thunk: re-do displaced `STA <$91,X`, then `Var4 = 1`, then RTS.
    let thunk: [u8; 7] = [
        0x95, 0x91,       // STA <$91,X (displaced)
        0xA9, 0x01,       // LDA #$01
        0x95, 0x7F,       // STA <Objects_Var4,X
        0x60,             // RTS
    ];
    rom.write_range(FS_PIRANHA_VIS_SMALL, &thunk);

    // Patch site: replace `STA <$91,X / RTS` with `JMP thunk`.
    let lo = (PIRANHA_VIS_SMALL_CPU & 0xFF) as u8;
    let hi = (PIRANHA_VIS_SMALL_CPU >> 8) as u8;
    rom.write_range(SMALL_PIRANHA_INIT_TAIL, &[0x4C, lo, hi]);
}

fn patch_big_piranha(rom: &mut Rom) {
    debug_assert_eq!(
        rom.read_range(BIG_PIRANHA_INIT_TAIL, 4),
        &[0xFE, 0xF7, 0x7F, 0x60],
        "big piranha init tail does not match expected bytes — ROM mismatch?",
    );

    // Thunk: re-do displaced `INC $7FF7,X`, then `Var4 = 1`, then RTS.
    let thunk: [u8; 8] = [
        0xFE, 0xF7, 0x7F, // INC $7FF7,X (displaced — Objects_IsGiant)
        0xA9, 0x01,       // LDA #$01
        0x95, 0x7F,       // STA <Objects_Var4,X
        0x60,             // RTS
    ];
    rom.write_range(FS_PIRANHA_VIS_BIG, &thunk);

    // Patch site: replace `INC $7FF7,X / RTS` (4 bytes) with `JMP thunk` (3 bytes)
    // + dead-code RTS filler so the timer table at $B777 stays put.
    let lo = (PIRANHA_VIS_BIG_CPU & 0xFF) as u8;
    let hi = (PIRANHA_VIS_BIG_CPU >> 8) as u8;
    rom.write_range(BIG_PIRANHA_INIT_TAIL, &[0x4C, lo, hi, 0x60]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::randomizer::Options;

    fn opts_off() -> Options {
        Options {
            ground: EnemyMode::Off, shell: EnemyMode::Off, flying: EnemyMode::Off,
            piranhas: EnemyMode::Off, ghosts: EnemyMode::Off,
            thwomps: EnemyMode::Off, rotodiscs: EnemyMode::Off,
            cannons: EnemyMode::Off, water: EnemyMode::Off, bros: EnemyMode::Off,
            ..Options::default()
        }
    }

    #[test]
    fn should_apply_only_when_piranhas_wild_with_other_wild_class() {
        let mut o = opts_off();
        assert!(!should_apply(&o), "all-off");

        o.piranhas = EnemyMode::Wild;
        assert!(!should_apply(&o), "piranhas alone wild");

        o.ground = EnemyMode::Shuffle;
        assert!(!should_apply(&o), "piranhas wild + ground shuffle");

        o.ground = EnemyMode::Wild;
        assert!(should_apply(&o), "piranhas wild + ground wild");

        o.piranhas = EnemyMode::Shuffle;
        assert!(!should_apply(&o), "piranhas shuffle + ground wild");
    }

    #[test]
    fn cannons_alone_do_not_trigger() {
        // Cannons share Wild semantics but have a self-contained pool; if only
        // cannons are Wild alongside piranhas, no cross can occur.
        let mut o = opts_off();
        o.piranhas = EnemyMode::Wild;
        o.cannons = EnemyMode::Wild;
        assert!(!should_apply(&o));
    }
}
