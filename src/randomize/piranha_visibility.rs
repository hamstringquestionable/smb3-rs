//! Wild-shuffled piranha plants: keep them visible at spawn, and drop their
//! hitbox while they are invisible mid-cycle.
//!
//! Vanilla piranha plants spawn in state `Objects_Var4 = 0` (HideInPipe), which
//! skips the draw call entirely until a timer + "Mario not too close" check
//! advances them to state 1 (Emerge). That's correct behavior for pipe-mounted
//! piranhas — the hide phase lets Mario stand next to the pipe safely.
//!
//! In wild-shuffle mode, however, piranha IDs (`0xA0–0xA7`, `0x7D`, `0x7F`) can
//! land in slots that previously held a ground enemy. Two failure modes follow:
//!
//! 1. **Invisible spawn.** The piranha keeps its initial `Var4 = 0`, so the
//!    player sees nothing — then the plant pops into view once the timer
//!    expires, which feels unfair.
//! 2. **Invisible hitbox.** Even after the patch above primes spawn to state 1,
//!    the state machine cycles back to state 0 (Retract → HideInPipe) every
//!    period. During that hide phase the sprite is skipped but the per-frame
//!    `JSR Player_HitEnemy` keeps firing, so Mario can be damaged by an
//!    invisible plant standing in mid-level (vanilla pipe placements hide the
//!    hitbox inside the pipe geometry, so it never matters there).
//!
//! Two pairs of bank-local thunks fix both issues. Both are gated on the same
//! condition: `opts.piranhas == Wild` **and** at least one other enemy class
//! is also Wild — outside that case the wild pool can't put piranhas into
//! foreign slots, so vanilla behavior is preserved.
//!
//! ## Patch layout
//!
//! ### Visibility (init-time prime of `Var4 = 1`)
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
//! ### Per-frame hitbox skip (distance-based gate around the hidden state)
//!
//! Only the small-piranha bank needs an extra thunk: `ObjNorm_BigPiranha`
//! already short-circuits state 0 in vanilla (`AND #$03 / BNE main; LDA #$FF /
//! STA SprHVis,X / JMP $B79D`), so its `JSR Player_HitEnemy` at `$B79A` is
//! already unreachable from state 0.
//!
//! `ObjNorm_Piranha` (small piranhas) runs `JSR Player_HitEnemy` every frame
//! regardless of orientation. We gate that call on the piranha's distance
//! from its hidden-position Y (`Objects_Var5,X`): skip when `|Y - Var5| < 10`
//! pixels. That covers the fully-hidden state itself **plus** a ~10 frame
//! safety margin on either side of the transition (Retract end and Emerge
//! start) — `Piranha_Retract` advances Y by +1 per frame.
//!
//! Using distance instead of state has two nice properties:
//! - **Orientation-agnostic.** Upright vs ceiling piranhas use the same state
//!   handlers, just running in different raw-`Var4` slots. Distance to Var5
//!   is symmetric (Y < Var5 for upright, Y > Var5 for ceiling), and the
//!   thunk uses a two-tail compare (`CMP #$0A` + `CMP #$F6`) to catch both.
//! - **No FlipBits dispatch needed.** Vanilla emerge height is ~24 px (per
//!   the Object_BoundBox entry for piranhas), so the fully-extended state
//!   sits well outside the ±10 px window — no risk of unintended skipping
//!   during the Attack state.
//!
//! - **PRG005** patch site: cpu `$A794` (file `0x0A7A4`), bytes `20 BA D1`
//!   → `4C CD BF` (JMP `$BFCD`).
//! - **PRG005** thunk: file `0x0BFDD` (cpu `$BFCD`), 18 bytes. Bias-and-range
//!   form: `LDA Y / SEC / SBC Var5 / CLC / ADC #$0A / CMP #$15 / BCC skip` —
//!   a single BCC catches both orientations after biasing `Y - Var5` into the
//!   range `[0, 20]`.
//!
//! `Objects_Y` is zero-page `$A3`, `Objects_Var5` is `$9A` — verified from
//! `Piranha_Retract` at `$A7C4` (`LDA $A3,X / ADD #$01 / ... / CMP $9A,X`).

use crate::rom::Rom;
use crate::randomizer::{EnemyMode, Options};
use super::rom_data::{
    FS_PIRANHA_HIT_SMALL,
    FS_PIRANHA_VIS_BIG, FS_PIRANHA_VIS_SMALL,
    PIRANHA_HIT_SMALL_CPU,
    PIRANHA_VIS_BIG_CPU, PIRANHA_VIS_SMALL_CPU,
};

/// File offset of the small-piranha init tail's last 3 bytes (`STA <$91,X / RTS`).
const SMALL_PIRANHA_INIT_TAIL: usize = 0x0A662;

/// File offset of the big-piranha init tail's last 4 bytes (`INC $7FF7,X / RTS`).
const BIG_PIRANHA_INIT_TAIL: usize = 0x09783;

/// File offset of `JSR Player_HitEnemy` inside `ObjNorm_Piranha` (PRG005, cpu `$A794`).
const SMALL_PIRANHA_HIT_CALL: usize = 0x0A7A4;

/// CPU return address after the small-piranha hit-skip thunk: vanilla `INC Objects_Var3,X` at `$A797`.
const SMALL_PIRANHA_HIT_RETURN_CPU: u16 = 0xA797;

/// CPU address of `Player_HitEnemy`, mapped via `$C000` during gameplay banking.
const PLAYER_HIT_ENEMY_CPU: u16 = 0xD1BA;

/// Apply the visibility + hit-skip patches when wild-shuffle can mix piranhas
/// with other classes. No-op otherwise.
pub fn apply(rom: &mut Rom, opts: &Options) {
    if !should_apply(opts) {
        return;
    }
    rom.push_tag("piranha_visibility");
    patch_small_piranha(rom);
    patch_big_piranha(rom);
    patch_small_piranha_hit_skip(rom);
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
    let [lo, hi] = PIRANHA_VIS_SMALL_CPU.to_le_bytes();
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
    let [lo, hi] = PIRANHA_VIS_BIG_CPU.to_le_bytes();
    rom.write_range(BIG_PIRANHA_INIT_TAIL, &[0x4C, lo, hi, 0x60]);
}

/// Apply the small-piranha per-frame hit-skip thunk (see module docs).
fn patch_small_piranha_hit_skip(rom: &mut Rom) {
    debug_assert_eq!(
        rom.read_range(SMALL_PIRANHA_HIT_CALL, 3),
        &[0x20, 0xBA, 0xD1],
        "small piranha JSR Player_HitEnemy site does not match — ROM mismatch?",
    );

    let [hit_lo, hit_hi] = PLAYER_HIT_ENEMY_CPU.to_le_bytes();
    let [ret_lo, ret_hi] = SMALL_PIRANHA_HIT_RETURN_CPU.to_le_bytes();

    // Bias-and-range check: A = (Y - Var5) + 10 maps the window [-10, +10]
    // onto [0, 20], so a single BCC catches both orientations.
    let thunk: [u8; 18] = [
        0xB5, 0xA3,             // LDA Objects_Y,X
        0x38,                   // SEC
        0xF5, 0x9A,             // SBC Objects_Var5,X   ; A = Y - Var5 (mod 256)
        0x18,                   // CLC
        0x69, 0x0A,             // ADC #$0A             ; bias by +10
        0xC9, 0x15,             // CMP #$15             ; < 21?
        0x90, 0x03,             // BCC +3 → skip JSR    ; |Y - Var5| <= 10
        0x20, hit_lo, hit_hi,   // JSR Player_HitEnemy
        0x4C, ret_lo, ret_hi,   // JMP $A797
    ];
    rom.write_range(FS_PIRANHA_HIT_SMALL, &thunk);

    let [lo, hi] = PIRANHA_HIT_SMALL_CPU.to_le_bytes();
    rom.write_range(SMALL_PIRANHA_HIT_CALL, &[0x4C, lo, hi]);
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

    fn synthetic_rom() -> crate::rom::Rom {
        let mut data = vec![0u8; 393232];
        data[0..4].copy_from_slice(&[0x4E, 0x45, 0x53, 0x1A]);
        data[4] = 16;
        data[5] = 16;
        data[6] = 0x40;
        data[SMALL_PIRANHA_INIT_TAIL..SMALL_PIRANHA_INIT_TAIL + 3]
            .copy_from_slice(&[0x95, 0x91, 0x60]);
        data[BIG_PIRANHA_INIT_TAIL..BIG_PIRANHA_INIT_TAIL + 4]
            .copy_from_slice(&[0xFE, 0xF7, 0x7F, 0x60]);
        data[SMALL_PIRANHA_HIT_CALL..SMALL_PIRANHA_HIT_CALL + 3]
            .copy_from_slice(&[0x20, 0xBA, 0xD1]);
        crate::rom::Rom::from_bytes_lax(&data, true).unwrap()
    }

    #[test]
    fn hit_skip_patch_rewrites_call_site_and_thunk() {
        let mut rom = synthetic_rom();

        let mut opts = opts_off();
        opts.piranhas = EnemyMode::Wild;
        opts.ground = EnemyMode::Wild;

        apply(&mut rom, &opts);

        let [jmp_lo, jmp_hi] = PIRANHA_HIT_SMALL_CPU.to_le_bytes();
        assert_eq!(
            rom.read_range(SMALL_PIRANHA_HIT_CALL, 3),
            &[0x4C, jmp_lo, jmp_hi],
        );

        let [ret_lo, ret_hi] = SMALL_PIRANHA_HIT_RETURN_CPU.to_le_bytes();
        assert_eq!(
            rom.read_range(FS_PIRANHA_HIT_SMALL, 18),
            &[
                0xB5, 0xA3,             // LDA Y,X
                0x38,                   // SEC
                0xF5, 0x9A,             // SBC Var5,X
                0x18,                   // CLC
                0x69, 0x0A,             // ADC #$0A
                0xC9, 0x15,             // CMP #$15
                0x90, 0x03,             // BCC +3
                0x20, 0xBA, 0xD1,       // JSR Player_HitEnemy
                0x4C, ret_lo, ret_hi,   // JMP $A797
            ],
        );
    }

    #[test]
    fn hit_skip_patch_is_gated() {
        let mut rom = synthetic_rom();
        let opts = opts_off();
        apply(&mut rom, &opts);
        assert_eq!(rom.read_range(SMALL_PIRANHA_HIT_CALL, 3), &[0x20, 0xBA, 0xD1]);
    }
}
