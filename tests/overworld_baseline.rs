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
    let Ok(rom) = std::fs::read(ROM_PATH) else { return };
    assert_eq!(
        hashes(&rom),
        hashes(&rom),
        "same seeds produced different output within one run — the baseline \
         cannot guard anything until this is stable"
    );
}

/// Baseline captured 2026-08-03, before any call site was migrated onto
/// `rom_data::tiles`. Regenerate ONLY for a change intended to alter output,
/// and say so explicitly in the commit — never to make a red test green.
const BASELINE: [u64; SEEDS as usize] = [
    0x3CC73D4F8B706AB2, 0x0AEF65A488DCD062, 0x0D056AF8D18E2BFB, 0xB3D5BE313EA37464,
    0xC7B499E3B7AA3B6D, 0xB5846D1438CE3405, 0x03A24CDEBF64925D, 0x10AA8775674EC492,
    0xEF8F05273F6202B2, 0x1309B3871D9DD47A, 0x1A760A893B1EA373, 0xB503C9D7095AF6A6,
    0x43E966B75F3514FF, 0x5968F3BEF7300364, 0x192F85ECF86D75C6, 0x538E5F94D3045FD0,
    0xB6EDBAACEE372D9C, 0xBC57F481A392D55D, 0x15C89DC8756F6CCA, 0x2F1856871DD3784F,
];

#[test]
fn output_matches_baseline() {
    let Ok(rom) = std::fs::read(ROM_PATH) else { return };
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
        println!("ROM not found at {ROM_PATH}");
        return;
    };
    println!("const BASELINE: [u64; SEEDS as usize] = [");
    for chunk in hashes(&rom).chunks(4) {
        let line: Vec<String> = chunk.iter().map(|h| format!("0x{h:016X}")).collect();
        println!("    {},", line.join(", "));
    }
    println!("];");
}
