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

use smb3_rs::{generate_patched_rom, Options};

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
const BASELINE: [u64; SEEDS as usize] = [
    0x23BB0B1661EB4991, 0x8BCB87C002850E1B, 0xD35E65FB6393BA7B, 0x28316C60D5B50C8C,
    0x71D46A423D87F88D, 0x16C8AF59B8BB0589, 0x9DE59CE91689D794, 0x4C5689DE1274B707,
    0xCF0D452FCB5F3BA5, 0xA0DA1A4EA05C9840, 0x913CB348A60E1204, 0xFB9B9EACB78D1131,
    0x9F479C3584CDB1C9, 0x704A60C34AA3185F, 0xFEBF4CA438E54465, 0x46495F783B0C2C96,
    0x8B9271CB5F45BD14, 0x7931621C85178986, 0xFC800FBE4F077B0A, 0xEADB5C1E7B634CA5,
];

#[test]
fn output_matches_baseline() {
    let Ok(rom) = std::fs::read(ROM_PATH) else {
        eprintln!("SKIP: requires the ROM, which is not included in the repo");
        return;
    };
    if BASELINE.iter().all(|h| *h == 0) {
        panic!("BASELINE is unpopulated — run `cargo test print_baseline -- --ignored --nocapture`");
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
