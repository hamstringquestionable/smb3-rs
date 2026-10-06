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
/// Recaptured 2026-10-06 for the island-edge blank fix (W4's cells under the
/// strip end `0xAB` blank to land) — see `docs/overworld_baseline_log.md`.
#[rustfmt::skip]
const BASELINE: [u64; SEEDS as usize] = [
    0xF2F51030B5EBE18A, 0xDF9E6BB7382B6F41, 0x3ED44B476C7D28C3, 0xF1BE679407295D03,
    0xE57D2832DE0FF75E, 0x8D01E3EAC3C63659, 0x3C41C71E1B5DE551, 0xEA47E6185EE9E045,
    0x5F63448F5843225C, 0x298833353AE328D7, 0x8AAA707A26A3C64B, 0xF68E172E9BC273C0,
    0x573C7993313C061E, 0x33DE5F7EAF29FF6E, 0xFF11913115AFB546, 0xA092365D9C285B10,
    0xA695EEDEF43C5B32, 0x941EA05F3F65EE73, 0xD17FBFDFEA84ECE6, 0x8CC9D1B1A1DA7C75,
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
