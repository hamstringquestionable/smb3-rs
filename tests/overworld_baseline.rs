//! Baseline for overworld output.
//!
//! A committed hash, per seed, of the bytes that describe the map a player
//! walks — terrain, where each tile leads, the sprites standing on it, where
//! pipes come out, and which fortress opens which lock. See
//! [`overworld_fingerprint`] for exactly what is covered and why.
//!
//! It moves for a real topology change, and for an RNG-stream shift from a new
//! draw anywhere upstream — because that genuinely re-deals every seed's map.
//! It does not move for a version bump, a new king quote, a relocated title
//! routine, or an always-on splice, all of which used to force a recapture back
//! when this hashed the whole ROM.
//!
//! **Whole-ROM byte identity is a different question and lives in
//! `tests/rom_identity.rs`** — capture on one tree, compare on another, no
//! committed constant to go stale. Reach for that one when the claim is "this
//! refactor changes nothing at all."
//!
//! Palette randomization is outside the fingerprint's regions, so the
//! not-seed-stable palette bytes cannot make this flap. The sweep still runs
//! with palettes off, so the two harnesses generate the same ROMs.
//!
//! Skipped when the ROM is absent, like the other ROM-dependent tests — the ROM
//! is gitignored and CI has no copy.

use smb3_rs::randomize::rom_data::overworld_fingerprint;
use smb3_rs::rom::Rom;
use smb3_rs::{Options, generate_patched_rom};

const ROM_PATH: &str = "roms/Super Mario Bros. 3 (USA) (Rev 1).nes";
const SEEDS: u64 = 20;

fn options() -> Options {
    Options { palettes: false, palette_themed: false, ..Options::default() }
}

fn fingerprints(rom: &[u8]) -> Vec<u64> {
    (1..=SEEDS)
        .map(|seed| {
            let out = generate_patched_rom(rom, seed, &options(), None)
                .unwrap_or_else(|e| panic!("seed {seed} failed to generate: {e}"));
            let parsed = Rom::from_bytes_lax(&out, true)
                .unwrap_or_else(|e| panic!("seed {seed} produced an unreadable ROM: {e}"));
            overworld_fingerprint(&parsed)
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
        fingerprints(&rom),
        fingerprints(&rom),
        "same seeds produced different overworlds within one run — the baseline \
         cannot guard anything until this is stable"
    );
}

/// Regenerate ONLY for a change intended to alter the overworld, and add a
/// dated entry to `docs/overworld_baseline_log.md` in the same commit saying
/// what moved and how you established it — never to make a red test green.
///
/// Captured 2026-09-08, the first capture of the narrowed fingerprint.
#[rustfmt::skip]
const BASELINE: [u64; SEEDS as usize] = [
    0xE5A2F00297D3BE5F, 0x5527C670B0E5F2C6, 0xBE65260A088EF73C, 0xE9AFEB4B9F5EEE43,
    0x703873E7132730E9, 0x45BD1D7B5F67F844, 0xA1F93503B38F6A66, 0x155D7E25DF5DC1ED,
    0x2591E13B5B00DECA, 0x1401007560BA0536, 0x12C446F2CE54AB4A, 0x3855943445F69287,
    0x10A6499058D8AB92, 0xE532B927D590C205, 0x551BE6350CB161B8, 0xEAB4EE609EA95F1C,
    0x9290E6451D5E6643, 0x1B5A89FD1AEFCFCD, 0xBC84B04EBE23553A, 0x56D22D2772AE929E,
];

#[test]
fn output_matches_baseline() {
    let Ok(rom) = std::fs::read(ROM_PATH) else {
        eprintln!("SKIP: requires the ROM, which is not included in the repo");
        return;
    };
    if BASELINE.iter().all(|h| *h == 0) {
        panic!(
            "BASELINE is unpopulated — run `cargo test --test overworld_baseline \
             -- --ignored print_baseline --nocapture`"
        );
    }
    let got = fingerprints(&rom);
    let mismatched: Vec<usize> = (0..SEEDS as usize).filter(|i| got[*i] != BASELINE[*i]).collect();
    assert!(
        mismatched.is_empty(),
        "the overworld changed for seed(s) {:?} — if that was intentional, \
         regenerate the baseline in the same commit and add a dated entry to \
         docs/overworld_baseline_log.md saying what moved",
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
    for chunk in fingerprints(&rom).chunks(4) {
        let line: Vec<String> = chunk.iter().map(|h| format!("0x{h:016X}")).collect();
        println!("    {},", line.join(", "));
    }
    println!("];");
}
