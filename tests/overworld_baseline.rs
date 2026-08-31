//! Byte-identity baseline for overworld output.
//!
//! Guards refactors that are supposed to change nothing — the tile-predicate
//! consolidation in `rom_data::tiles` being the motivating case. Each migration
//! of a call site onto a shared predicate must leave these hashes untouched; a
//! changed hash means the substituted predicate covers a different set than the
//! inlined list it replaced, which is exactly the kind of silent behaviour
//! change a "pure refactor" is not allowed to make.
//!
//! Palette randomization is excluded: it is not seed-stable (two runs of the
//! same seed differ in a handful of NES palette bytes), so it would make this
//! harness flap for reasons unrelated to what it is guarding. Topology is
//! unaffected.
//!
//! Skipped when the ROM is absent, like the other ROM-dependent tests — the
//! ROM is gitignored and CI has no copy.

use smb3_rs::{Options, generate_patched_rom};

const ROM_PATH: &str = "roms/Super Mario Bros. 3 (USA) (Rev 1).nes";
const SEEDS: u64 = 20;

/// FNV-1a. Rolled by hand rather than using `DefaultHasher`, whose output is
/// explicitly not guaranteed stable across Rust releases — a baseline that
/// changes when the toolchain updates is worse than none.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn options() -> Options {
    Options { palettes: false, palette_themed: false, ..Options::default() }
}

fn hashes(rom: &[u8]) -> Vec<u64> {
    (1..=SEEDS)
        .map(|seed| {
            let out = generate_patched_rom(rom, seed, &options(), None)
                .unwrap_or_else(|e| panic!("seed {seed} failed to generate: {e}"));
            fnv1a(&out)
        })
        .collect()
}

/// The harness is only worth anything if it is stable in the first place.
/// Runs the sweep twice in one process and requires agreement.
#[test]
fn sweep_is_deterministic() {
    let Ok(rom) = std::fs::read(ROM_PATH) else {
        eprintln!("SKIP: requires the ROM, which is not included in the repo");
        return;
    };
    assert_eq!(
        hashes(&rom),
        hashes(&rom),
        "same seeds produced different output within one run — the baseline \
         cannot guard anything until this is stable"
    );
}

/// Regenerate ONLY for a change intended to alter output, and say so
/// explicitly in the commit — never to make a red test green.
///
/// Captured 2026-08-03 before the `rom_data::tiles` migration; re-captured
/// 2026-08-05 for the always-on stomp-fairness patch, which writes 31 bytes
/// into every ROM (`stomp_fairness::apply`). These hashes cover the whole
/// output, so all 20 seeds moved. Overworld topology is untouched by that
/// change — the ROM bytes differ, the maps do not.
///
/// Re-captured again 2026-08-05 for the Wild Injections sun/lakitu pool
/// (`FLAG_KEY_VERSION` 27 → 28). The version byte is stamped into every ROM,
/// so all 20 seeds moved for that reason alone: pinning the constant back to
/// 27 reproduced the previous hashes exactly, which is the check that this
/// regeneration hides no real change. Default options leave the injection
/// mode Off, so no gameplay byte differs.
///
/// Re-captured again 2026-08-09 for the wider title-hash icon set: ICON_TILES
/// grew from 15 to 20 rows and the palette digit was dropped, so the 40-byte
/// sprite table at `FS_SEED_HASH_DATA` differs for every seed and all 20 moved.
/// Verified the same way as the v29 recapture — seeds 1-3 generated before and
/// after with `--no-palettes` (the player wardrobe is not seed-deterministic)
/// and diffed byte by byte. Every difference lands inside FS_SEED_HASH_DATA; no
/// overworld, level or enemy byte moved.
///
/// Re-captured again 2026-08-06 for the v29 flag-key re-layout (issue #158).
/// Same story, verified the same way but end-to-end rather than by pinning a
/// constant, since the whole format changed: seeds 1-3 were generated before
/// and after and diffed byte by byte. Every difference lands in one of the two
/// places that carry the flag bytes on purpose — the stamp block at 0x19DF0 and
/// the title-screen verification icons at `FS_SEED_HASH_DATA` (0x3E93D), whose
/// hash folds `to_flag_bytes()`. No overworld, level, or enemy byte moved.
/// Re-captured 2026-08-10 for the title-screen B-to-mute toggle, which writes
/// 25 bytes into every ROM (`title_screen::mute_routine` plus its hook), so all
/// 20 seeds moved. Verified the same way as the recaptures above: seeds 1-3
/// generated before and after with `--no-palettes --patched-rom` and diffed byte
/// by byte. Exactly 24 bytes differ per seed, in two places — the `JSR` operand
/// at the hook site 0x30C94 and the 22-byte routine at 0x33529, which was $FF
/// filler. No overworld, level or enemy byte moved.
///
/// Re-captured 2026-08-11 for the dealt C1 floor (`capacity::deal_c1_floors`).
/// This one DOES change overworld topology, on purpose: each world is dealt a
/// floor of 11 / 14 / 17 instead of a flat 14, so shaping resolves differently.
/// It also draws twice from the shared RNG (the pair count, then a shuffle),
/// which shifts every downstream stream position — so all 20 seeds move for
/// two reasons at once and a byte-level diff could not attribute them.
/// Verified by pinning instead, the way the v28 recapture was: making
/// `deal_c1_floors` return `[C1_FLOOR; 8]` with no draw reproduced the
/// PREVIOUS hashes exactly, which shows the difference is the deal alone and
/// that threading the floor through the shaping sites changed no decision.
/// (The `C1_FLOOR_FLAT` env arm cannot do this job — it is `cfg(test)`, and
/// this file links the library as an outside consumer.)
/// Re-captured 2026-08-17 for the treasure-box room rules. Two changes, both
/// deliberate, and every seed moves because both shift the shared RNG stream:
///
/// 1. A `$81` in a `Level_Event = 7` room that is not a real bro battle is
///    rewritten by `ObjInit_HammerBro` into an unkillable object, so it is now
///    excluded from every such room (`rewrites_hammer_bro`).
/// 2. Those rooms are cleared to be left, so they take the bro-fight pool:
///    the 8-Tank treasure room joins the Coin Ship as a `HammerBro` segment,
///    and Dry Bones moved from `STOMPABLE_ENEMIES` to `HB_NEEDS_SHELL_ENEMIES`
///    (it is stompable but revives, so only a kicked shell clears it).
///
/// Attributed rather than assumed. Against the previous build at default flags
/// (`hb_encounters` Off): **no** enemy ID in any treasure-box room differs, and
/// `0x0DA3A` (the 8-Tank slot) now holds vanilla `$82` in every seed checked.
/// That is the whole story — with `hb_encounters` Off, `hb_modes` is all-Off, so
/// routing that room through the HB path means its single entry no longer draws
/// at all where the old `ForceTankBro` row drew once. One fewer draw, and every
/// downstream consumer (chest items, later segments, the overworld) shifts. The
/// pool changes themselves only bite with `hb_encounters` on, which this sweep
/// does not exercise — `treasure_box_rooms_never_get_a_hammer_bro` and the
/// shell-pairing invariant in `enemies::tests` cover that instead.
///
/// Re-captured 2026-08-20 for the W8 bridges-out deal
/// (`overworld_build::locks::deal_bridge_spans`). Every seed moves, and for
/// the usual two reasons at once: `capacity::roll_bridges_out` draws once
/// from the shared RNG before the worlds are built, shifting every downstream
/// stream position, and the deal itself changes W8's lock placement on
/// purpose. Attribution is by census rather than by byte, since a lock moving
/// re-prices the world: `w8_bridges_out_census` shows the count going from
/// 11.5/62.3/18.0/8.2/0% to 3.0/46.1/40.2/10.8/~0.02% across 0-4 spans out,
/// the same-span rate from 78% to 33%, and `test_route_census` at 1000 seeds
/// shows W8 mean routes 2.27 -> 2.25 (linear 4% -> 5%), overall 2.548 -> 2.537
/// (linear 6.60% -> 6.91%), nothing below its dealt floor either way.
///
/// Re-captured 2026-08-24 for the `SPOILED_SEGMENT_RANGES` off-by-one
/// (issue #181). The middle row ended one byte past the next real stream's
/// page byte, so after autoscroll removal the enemy-data walker resumed
/// mid-entry and stayed out of phase for 18 slots. Attribution is by
/// pinning, as with the v28 and C1-floor recaptures: restoring the range to
/// `0x0D037..0x0D042` reproduces the hashes above exactly, so this one byte
/// is the whole difference. What it changes: the walker now sees the two
/// segments it was reading through, whose 17 entries were pinned to vanilla
/// in every seed before. They are unrelated levels that merely sit next to
/// each other in the block — `0x0D041` is 4-1's underwater sub-area and
/// `0x0D049` is 5-4. Only four of the 17 can move: the two Big Berthas at
/// `0x0D042`/`0x0D045` (water) and the two para-troopas at
/// `0x0D068`/`0x0D074` (flying). The other 13 are `$90`-`$93`, the tilting
/// and twirling platforms, which belong to no class pool and so never draw
/// at any flag setting. Those four extra draws shift the shared RNG stream,
/// which is why all 20 seeds move and the overworld moves with them.
/// Nothing was being written to a wrong byte before the fix — the 18
/// out-of-phase slots held X, Y and page bytes, all `$01..$19` and in no
/// class pool, so 500 seeds at all-Wild never wrote one. It was latent, and
/// `every_enemy_entry_point_is_a_walker_segment_start` keeps it that way.
///
/// Re-captured 2026-08-26 for the Super Princess Peach title-screen fix. The
/// B-to-mute routine moved from 0x33529 to 0x33FF0 — same 22 bytes, different
/// home, because Peach's title logo claims 0x33530..0x336A7 and the randomizer
/// writes last. Every seed moves because the ROM bytes moved. Attribution is by
/// byte diff, as with the recaptures above: seeds 1-3 generated before and
/// after with `--no-palettes --patched-rom` differ in exactly 46 bytes, in
/// three runs — the 2-byte `JSR` operand at 0x30C94, the 22 bytes at 0x33529
/// returning to `$FF` filler, and the byte-identical routine reappearing at
/// 0x33FF0. The routine's seeded music byte is unchanged per seed, so no RNG
/// draw shifted; no overworld, level or enemy byte moved.
///
/// Re-captured 2026-08-26 for the 1.2.1 bump. A version bump alone moves every
/// seed: `compute_hash` folds `CARGO_PKG_VERSION`, which is the whole point of
/// the version-guard. Attribution is by round trip *and* by byte. Round trip:
/// 1.2.0 on this same tree reproduces the previous array exactly and 1.2.1
/// reproduces this one, so the version string is the only cause. Byte: seeds
/// 1-3 generated at each version with `--no-palettes --patched-rom` differ in
/// 2-3 bytes, every one of them inside `FS_SEED_HASH_DATA` (0x3E93D+40) -- the
/// title-screen verification icons, which is exactly what is supposed to move.
///
/// One trap worth naming: `cargo` did NOT rebuild on a version-only edit to
/// `Cargo.toml` here, so the first recapture attempt printed a stale binary's
/// hashes. Force it (`touch src/lib.rs`, or `cargo clean -p smb3-rs`) before
/// trusting any number this test prints after a bump.
///
/// Re-captured 2026-08-30 for the pipe screen-squish fix: the always-on
/// MaCobra bundle gained a 9-byte splice at 0x10476 (PRG008 $A466) that exempts
/// pipe travel from the left-edge crush death. It consumes no RNG, so nothing
/// downstream shifted — verified rather than argued: for all 20 of these seeds
/// the pre- and post-fix ROMs were diffed byte for byte (`--no-palettes
/// --patched-rom`) and the diff is exactly 0x10476..0x1047F on every one of
/// them, with no second span. No map tile, pointer table or level byte moved.
///
/// Re-captured 2026-08-31 for the off-path fort rule (`Forts` may no longer
/// take a blank the player cannot walk around; `try_fort_lock` and
/// `try_pipe_move` hold the same line). Overworld topology changes on
/// purpose. TWO causes are folded into this recapture and both are named
/// here on purpose:
///
/// 1. The baseline was ALREADY stale at HEAD — PR #205 (three new king rescue
///    quotes) rewrites quote text in every ROM and did not recapture, so all
///    20 seeds had already moved before this change was written.
/// 2. This change itself, which moves fortresses and therefore locks, C1, and
///    the shaping decisions downstream of them.
///
/// Verified by pinning, the way the v28 and dealt-C1-floor recaptures were:
/// neutralising all three guards (an empty forced set in `Forts` and in
/// `try_fort_lock`, `any_fort_forced` returning false) reproduced the HEAD
/// hashes EXACTLY, all 20 seeds. So cause 2 is the guards alone — the new
/// `WorldState::forced_positions` and the census columns that read it change
/// no output — and cause 1 is inherited, not created here.
const BASELINE: [u64; SEEDS as usize] = [
    0x29A3EEA5BF4AE51F,
    0xDF1B453A02F3E8CE,
    0x3C30C7E62A89E412,
    0x6590EB228FFFFFAD,
    0xD9824148CFCFB18E,
    0x13FB87791F7E5643,
    0x2259781019F56587,
    0x05E4347FD69DA9A9,
    0xC149AFCA80118085,
    0xB2C7DFD9A190A0B5,
    0x1D86758E9A78E98E,
    0xE05F91CBFC3F6720,
    0xF29978C2C9D3AF59,
    0xE7AC193883E470EC,
    0xCDA7E9922AB91CCC,
    0x32D1C6748C543C3C,
    0xF801670EA1CCA280,
    0x14D567EBB4188B77,
    0xCBA0C0826565F4C1,
    0x78E3B157D38E6082,
];
///
/// Re-captured 2026-08-27 for the Big [?] bonus-room shuffle, which is always
/// on. Every seed moves, for two reasons that are both intended:
///
/// 1. `qol::big_q`'s lookup routine grew from 106 to 207 bytes (slot seeding
///    plus four new 13-entry payload tables), and the Big [?] exit path's
///    `JMP PRG026_AA8A` at 0x34A84 now jumps into it.
/// 2. `big_q_rooms::shuffle` draws twice from the shared RNG and rewrites the
///    per-row area/arrival tables, and 7-F1's drawn block is forced to Tanooki.
///
/// **Overworld topology is untouched**, which is the thing worth checking here.
/// The pass runs after `write_overworld`, so it cannot move a map tile — and
/// that was verified rather than argued: the 8 map grids were hashed for all 20
/// seeds with the `shuffle` call live and with it stubbed to return an empty
/// vec, and the two sets are identical. The ROM bytes differ, the maps do not.
///
/// Amended the same week: Unused Level 5's screens 6 and 7 got new arrival
/// coordinates after playtesting showed the originals killed the player. Four
/// table bytes, so every seed that draws one of those rooms moves. No RNG draw
/// changed — the pool is the same size — so this is a pure byte difference.
///
/// Re-captured 2026-08-29 again, for `shuffle_big_q_rooms`. The room shuffle is
/// now an option and **off by default**, so the pass no longer draws its two
/// values from the shared RNG on a default build, which shifts every module
/// downstream of it (`credits`, `items`, `king_quotes`, `koopalings`).
///
/// The on-path was checked against the build this option was carved out of: for
/// seeds 1-12, `--shuffle-big-q-rooms` reproduces `beta/next` byte for byte
/// apart from `[stamp]` (0x19DF0..0x19E09) and `[title_screen]` (0x3E924..),
/// both of which are derived from the flag key — and the key necessarily grew
/// one bit. The randomization itself is untouched.
///
/// Re-captured 2026-08-29 for the 7-F1 return fix: the entry and exit halves of
/// the `qol::big_q` routine now share one `bq_lookup` subroutine, which moved
/// every label and table after offset $09 and shrank it from 207 to 199 bytes.
/// Again a pure byte difference, and again verified rather than argued — for
/// each of these 20 seeds the pre- and post-fix ROMs were diffed byte for byte,
/// and the only bytes that changed outside `FS_BIG_Q_LOOKUP`'s own 224-byte
/// allocation were the exit hook's operand at 0x34A85. No map tile, pointer
/// table or level byte moved.
///
/// Re-captured 2026-08-30 for `qol::level_clock`: the level clock's frame
/// divider is now 60 rather than vanilla's 41, so a unit is a real second. It
/// is two immediate operands (0x34FC6, 0x3C517) and is applied to every seed,
/// so every hash moved. Verified rather than argued, the same way as the
/// entries above: for each of these 20 seeds the generated ROM had those two
/// bytes reverted to `$28` and re-hashed, and all 20 then reproduced the
/// previous baseline exactly — so those two bytes are the entire diff. No map
/// tile, pointer table or level byte moved.

#[test]
fn output_matches_baseline() {
    let Ok(rom) = std::fs::read(ROM_PATH) else {
        eprintln!("SKIP: requires the ROM, which is not included in the repo");
        return;
    };
    if BASELINE.iter().all(|h| *h == 0) {
        panic!(
            "BASELINE is unpopulated — run `cargo test print_baseline -- --ignored --nocapture`"
        );
    }
    let got = hashes(&rom);
    let mismatched: Vec<usize> = (0..SEEDS as usize).filter(|i| got[*i] != BASELINE[*i]).collect();
    assert!(
        mismatched.is_empty(),
        "output changed for seed(s) {:?} — if this was intentional, regenerate \
         the baseline in the same commit and explain why",
        mismatched.iter().map(|i| i + 1).collect::<Vec<_>>()
    );
}

/// Prints a `BASELINE` array to paste above. Ignored by default so it never
/// runs as part of the suite.
#[test]
#[ignore]
fn print_baseline() {
    let Ok(rom) = std::fs::read(ROM_PATH) else {
        eprintln!("SKIP: requires the ROM, which is not included in the repo");
        return;
    };
    println!("const BASELINE: [u64; SEEDS as usize] = [");
    for chunk in hashes(&rom).chunks(4) {
        let line: Vec<String> = chunk.iter().map(|h| format!("0x{h:016X}")).collect();
        println!("    {},", line.join(", "));
    }
    println!("];");
}
