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
const BASELINE: [u64; SEEDS as usize] = [
    0xACE6D29C567449F8, 0x550C792486224F4B, 0xA8C5F0A6B296759D, 0x61CC712A9D8232FF,
    0x8DA95E9C5FEA6A31, 0xE02746EC8F23D59F, 0xB7FA7BEAEF413EBA, 0xACEBCAE4470A4C11,
    0xC269A39320B27CFB, 0xEF8AFBB90E2A00AD, 0x14129EF370846280, 0xAADFE1EEB4E9E188,
    0x6546BD69DCFAB8FE, 0x94C37EDDC54B6A0D, 0x543AC2A0587F25BF, 0x97DB071E9067030A,
    0x39D2EC7C5988E1F3, 0xDA4AE0193B48CDDF, 0xA77453960E7F51DF, 0x2A1B1A9E8C0DC998,
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
