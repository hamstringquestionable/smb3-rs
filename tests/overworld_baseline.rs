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
const BASELINE: [u64; SEEDS as usize] = [
    0xBB2A7CBF02AF4BBA, 0x761607361C034EDA, 0xC4E3267B24430DB5, 0x871D2AEE7C896AB6,
    0xF27E5B34E1EEABDD, 0x9D9F280937717403, 0xE879479EEA8398FF, 0x2991D7B20C48097E,
    0xEFF71177664A5822, 0x4EF2AB92BC810522, 0xD188D3A10F375DB5, 0x833E8EFC349D2154,
    0x280B67E04F3F5F87, 0xCD041C077C531F96, 0x675C1986FEEDE464, 0x3286199B722396FA,
    0xD108207D0F8A21EC, 0x13D64C84EB019B01, 0x6C77A703B2285A5A, 0x437324880DC9D14B,
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
