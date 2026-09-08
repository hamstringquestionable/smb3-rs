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
    0xE280A526F23F2EF9, 0x5CFC4B6F55B505F9, 0x74A9232189958648, 0x9DDC56D6747419A1,
    0x26B2B38284964235, 0xD38E6CD89C356D98, 0xFE93B76C550E591E, 0xE97FD6ED6D7C69FD,
    0xED37AD9D1FAE43C2, 0x5165C56AD24CBDCE, 0x45E8CED514A354B4, 0xE9EB79071ACDCF32,
    0xA060A28A69E722FC, 0x8778B2AB1C9A6E28, 0x3B9EBD48A49F2B64, 0x409474608E3E6886,
    0x367675006DF2EE38, 0x989585727F05EF5A, 0x6958892EFCE3D548, 0xC7833220D6666BA5,
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
