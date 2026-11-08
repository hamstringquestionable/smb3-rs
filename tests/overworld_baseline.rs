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
/// Recaptured 2026-11-08 for the sprite-slot ledger's phase B (Hammer Bros
/// fill from slot 1, World 8's tank in slot 0 — map-object tables only, all 20
/// seeds) — see `docs/overworld_baseline_log.md`.
#[rustfmt::skip]
const BASELINE: [u64; SEEDS as usize] = [
    0x66DD79DA48E339FA, 0xB3BB70D529AA86ED, 0x0D55D984BDDA34D8, 0x4EE7C46B64B1D4E6,
    0xDE40AE41331AA4C1, 0xD53838D2CFF0424D, 0xA17AC8A46E4DB9DC, 0xA18B541B9DA16F36,
    0x9DBF416A35463A8A, 0x908D21F248035BE1, 0x584A4945F0726467, 0x3AC41831051A8C90,
    0x90576F14D09EFE4C, 0x814331EE03E74C4E, 0x7EEDA21C00FC94B9, 0xC4DA57BE74D56E86,
    0xDFCCA3BC326B74C2, 0x068ACA9BCA14EB15, 0x24D938C0E98364A8, 0x44D298D677D94A63,
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
