//! Whole-ROM byte identity, on demand.
//!
//! The question this answers is **"I changed no behaviour — prove it."** A pure
//! refactor, a module split, a helper extracted: the generated ROM must come
//! out byte for byte identical for every seed, and if it does not, the
//! substituted code covers a different set than what it replaced.
//!
//! # Using it
//!
//! ```sh
//! git stash                                              # or check out the base commit
//! cargo test --test rom_identity -- --ignored capture
//! git stash pop                                          # ...now make the change
//! cargo test --test rom_identity -- --ignored compare
//! ```
//!
//! `capture` writes to `target/rom_identity_capture.txt`, which is gitignored
//! by virtue of living in `target/`. (`cargo clean` takes it with everything
//! else — recapture if that happens.)
//!
//! # Why there is no committed baseline here
//!
//! There used to be, and it was the wrong shape for this job. A whole-ROM hash
//! moves for *any* byte anywhere: a version bump, a new king quote, a title
//! routine changing address, an always-on splice that touches no gameplay. As a
//! suite gate that meant a recapture — and a paragraph proving the diff was
//! harmless — roughly every week, and twice the baseline silently went stale
//! instead, which is strictly worse than not having one.
//!
//! A refactor check does not need a *stored* answer, only two trees. Capturing
//! on demand is both less maintenance and more capable: it compares against
//! whatever commit you point it at, not only the last one somebody blessed.
//!
//! What the suite still gates on is `tests/overworld_baseline.rs`, which hashes
//! the overworld alone and so stays green through ordinary feature work.
//!
//! Skipped when the ROM is absent, like the other ROM-dependent tests — the ROM
//! is gitignored and CI has no copy.

use smb3_rs::{DejaVuMode, HintMode, Options, PiranhaMode, generate_patched_rom};
use std::path::Path;

const ROM_PATH: &str = "roms/Super Mario Bros. 3 (USA) (Rev 1).nes";
const CAPTURE_PATH: &str = "target/rom_identity_capture.txt";
const SEEDS: u64 = 20;

/// Seeds per arm beyond the first. The default arm keeps the full [`SEEDS`]
/// sweep; the rest exist to reach code the defaults leave switched off, and a
/// short sweep over each buys more coverage per second than a longer one over
/// the same arm.
const ARM_SEEDS: u64 = 8;

/// FNV-1a. Rolled by hand rather than using `DefaultHasher`, whose output is
/// explicitly not guaranteed stable across Rust releases.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Palette randomization is excluded: it is not seed-stable (two runs of the
/// same seed differ in a handful of NES palette bytes), so it would make this
/// harness flap for reasons unrelated to what it is guarding.
fn options() -> Options {
    Options { palettes: false, palette_themed: false, ..Options::default() }
}

/// The option arms the sweep covers, and how many seeds each gets.
///
/// **The defaults alone were not enough, and this records what that cost.** A
/// refactor of the map walker — a blocked set in place of painted lock tiles —
/// came out byte-identical on the default arm while quietly changing canoe
/// gating. `overworld_baseline` caught it; this harness would not have. Arms
/// two onward are the paths `Options::default()` leaves off that the overworld
/// writer, the deck and the maze all run through.
///
/// **The world maze is here for a second reason.** It has no committed baseline
/// of any other kind, has never been playtested end to end, and every guarantee
/// it holds is a property test. An identity oracle is the only thing that will
/// tell a future refactor that it moved the mode's output.
fn arms() -> Vec<(&'static str, Options, u64)> {
    let base = options;
    vec![
        ("default", base(), SEEDS),
        ("piranha", Options { piranha_shuffle: PiranhaMode::Wild, ..base() }, ARM_SEEDS),
        ("start<->airship", Options { swap_start_airship: true, ..base() }, ARM_SEEDS),
        (
            "deja vu",
            Options { deja_vu: DejaVuMode::Double, deja_vu_forts: true, ..base() },
            ARM_SEEDS,
        ),
        ("friendlier", Options { friendlier_levels: true, ..base() }, ARM_SEEDS),
        ("no hb shuffle", Options { shuffle_hammer_bros: false, ..base() }, ARM_SEEDS),
        ("beta stages", Options { include_beta_stages: true, ..base() }, ARM_SEEDS),
        ("maze, hints some", Options { world_maze: true, ..base() }, ARM_SEEDS),
        (
            "maze, hints full",
            Options { world_maze: true, hints: HintMode::Full, ..base() },
            ARM_SEEDS,
        ),
        ("maze, no hints", Options { world_maze: true, hints: HintMode::Off, ..base() }, ARM_SEEDS),
    ]
}

/// How many hashes a full sweep produces.
fn sweep_len() -> usize {
    arms().iter().map(|(_, _, n)| *n as usize).sum()
}

/// Which arm and seed hash `i` came from, so a failure names the thing that
/// moved rather than an index into a list.
fn label(i: usize) -> String {
    let mut at = i;
    for (name, _, n) in arms() {
        if at < n as usize {
            return format!("{name} seed {}", at + 1);
        }
        at -= n as usize;
    }
    format!("index {i}")
}

fn rom() -> Option<Vec<u8>> {
    std::fs::read(ROM_PATH).ok()
}

fn hashes(rom: &[u8]) -> Vec<u64> {
    let mut out = Vec::with_capacity(sweep_len());
    for (name, opts, n) in arms() {
        for seed in 1..=n {
            let patched = generate_patched_rom(rom, seed, &opts, None)
                .unwrap_or_else(|e| panic!("{name} seed {seed} failed to generate: {e}"));
            out.push(fnv1a(&patched));
        }
    }
    out
}

/// The harness is only worth anything if it is stable in the first place.
/// Runs the sweep twice in one process and requires agreement. Not ignored:
/// it costs one extra sweep and it is the reason a `compare` failure can be
/// believed.
#[test]
fn sweep_is_deterministic() {
    let Some(rom) = rom() else {
        eprintln!("SKIP: requires the ROM, which is not included in the repo");
        return;
    };
    assert_eq!(
        hashes(&rom),
        hashes(&rom),
        "same seeds produced different output within one run — no identity \
         check can mean anything until this is stable"
    );
}

/// Record this tree's output. Run on the base commit, before the change.
#[test]
#[ignore]
fn capture() {
    let Some(rom) = rom() else {
        eprintln!("SKIP: requires the ROM, which is not included in the repo");
        return;
    };
    let body: String =
        hashes(&rom).iter().map(|h| format!("{h:016X}\n")).collect::<Vec<_>>().concat();
    std::fs::write(CAPTURE_PATH, &body).unwrap_or_else(|e| panic!("{CAPTURE_PATH}: {e}"));
    eprintln!("captured {SEEDS} seeds to {CAPTURE_PATH}");
}

/// Compare this tree against the capture. Run after the change.
///
/// Fails loudly rather than skipping when there is no capture: "no baseline
/// recorded" reported as a pass is how a refactor guard gets trusted for
/// nothing.
#[test]
#[ignore]
fn compare() {
    let Some(rom) = rom() else {
        eprintln!("SKIP: requires the ROM, which is not included in the repo");
        return;
    };
    assert!(
        Path::new(CAPTURE_PATH).exists(),
        "no capture at {CAPTURE_PATH} — run `cargo test --test rom_identity -- \
         --ignored capture` on the tree you mean to compare against first"
    );
    let text = std::fs::read_to_string(CAPTURE_PATH).unwrap();
    let before: Vec<u64> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| u64::from_str_radix(l.trim(), 16).expect("capture file is corrupt"))
        .collect();
    assert_eq!(
        before.len(),
        sweep_len(),
        "capture holds {} hashes, this tree sweeps {} — recapture",
        before.len(),
        sweep_len()
    );

    let after = hashes(&rom);
    let moved: Vec<String> =
        (0..sweep_len()).filter(|i| after[*i] != before[*i]).map(label).collect();
    assert!(
        moved.is_empty(),
        "{} of {} outputs changed: {moved:?}\n\
         The ROM is not byte-identical, so this was not a pure refactor. Either \
         the change has an effect that was not intended, or it was never meant \
         to be identity-preserving — in which case attribute the difference and \
         say so, do not recapture to make this quiet.",
        moved.len(),
        sweep_len()
    );
    eprintln!("all {SEEDS} seeds byte-identical");
}
