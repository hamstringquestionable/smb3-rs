//! Byte parity between the ways a player can get the same seed.
//!
//! Three shapes produce output here: the library's `generate_patch`, its
//! `generate_patched_rom`, and the `smb3-rs` binary. They used to answer "what
//! did we start from" separately — the CLI diffed against its own copy of the
//! input while the library used the `Rom`'s baseline — and a Rev 0 input made
//! the difference visible, because the bytes the user supplied are no longer
//! the bytes the randomizer worked on.
//!
//! The identity that protects a player is: **the patch and the ROM must agree**.
//! Downloading `seed.ips` and applying it to your own ROM has to land on the
//! same bytes as downloading `seed.nes`. That is asserted here for both
//! revisions, with and without a visual patch layered on, and separately for
//! the library and for the binary — so neither path can drift from the other
//! without a test going red.

use std::path::Path;
use std::process::Command;

const REV1: &str = "roms/Super Mario Bros. 3 (USA) (Rev 1).nes";
const PRG0: &str = "roms/Super Mario Bros. 3 (USA).nes";
const TOAD: &str = "web/visual-patches/super-toad-josuecr4ft.ips";

const SEED: u64 = 0xC0FFEE;

/// Both dumps, or `None` when the machine doesn't have them. These tests guard
/// the machine the change is written on; CI has no ROM.
fn dumps() -> Option<(Vec<u8>, Vec<u8>)> {
    match (std::fs::read(REV1), std::fs::read(PRG0)) {
        (Ok(rev1), Ok(prg0)) => Some((rev1, prg0)),
        _ => {
            eprintln!("skipping: both USA dumps must be present");
            None
        }
    }
}

fn options() -> smb3_rs::Options {
    // Palettes off: the cosmetic roll is the one thing that is deliberately not
    // in the flag key, so leaving it on makes two runs of "the same" options
    // incomparable. See `testing_no_palettes_determinism`.
    smb3_rs::Options { palettes: false, ..Default::default() }
}

/// The library's patch and its ROM must describe the same bytes, for either
/// revision, with or without a visual patch underneath.
#[test]
fn library_patch_and_rom_agree() {
    let Some((rev1, prg0)) = dumps() else { return };
    let toad = std::fs::read(TOAD).expect("bundled Toad swap must exist");
    let opts = options();

    for (name, input) in [("Rev 1", &rev1), ("Rev 0", &prg0)] {
        for (what, visual) in [("no visual patch", None), ("Toad", Some(toad.as_slice()))] {
            let patch = smb3_rs::generate_patch(input, SEED, &opts, visual)
                .unwrap_or_else(|e| panic!("{name}, {what}: generate_patch: {e}"));
            let rom = smb3_rs::generate_patched_rom(input, SEED, &opts, visual)
                .unwrap_or_else(|e| panic!("{name}, {what}: generate_patched_rom: {e}"));

            let applied = smb3_rs::apply_ips_patch(input, &patch)
                .unwrap_or_else(|e| panic!("{name}, {what}: the patch does not apply: {e}"));

            assert_eq!(
                applied.len(),
                rom.len(),
                "{name}, {what}: patched input and patched ROM differ in length"
            );
            assert!(
                applied == rom,
                "{name}, {what}: applying the .ips to the supplied ROM does not reproduce the .nes"
            );
        }
    }
}

/// A Rev 0 input is converted before anything reads it, so it must randomize to
/// exactly the bytes a Rev 1 input does — same seed, same game.
#[test]
fn both_revisions_randomize_to_the_same_rom() {
    let Some((rev1, prg0)) = dumps() else { return };
    let opts = options();

    let from_rev1 = smb3_rs::generate_patched_rom(&rev1, SEED, &opts, None).unwrap();
    let from_prg0 = smb3_rs::generate_patched_rom(&prg0, SEED, &opts, None).unwrap();
    assert!(from_rev1 == from_prg0, "Rev 0 and Rev 1 inputs produced different ROMs");

    // ...while the patches differ, because they start from different bytes.
    // A Rev 0 patch that equalled the Rev 1 one would be the bug: it would
    // silently misapply to the ROM the player owns.
    let patch_rev1 = smb3_rs::generate_patch(&rev1, SEED, &opts, None).unwrap();
    let patch_prg0 = smb3_rs::generate_patch(&prg0, SEED, &opts, None).unwrap();
    assert_ne!(patch_rev1, patch_prg0, "the Rev 0 patch must also carry the revision conversion");
}

/// Same identity, through the binary. This is the half that regressed before:
/// the CLI built its patch against bytes it tracked itself.
#[test]
fn cli_patch_and_rom_agree() {
    let Some((rev1, prg0)) = dumps() else { return };
    let exe = env!("CARGO_BIN_EXE_smb3-rs");
    let dir = std::env::temp_dir().join(format!("smb3rs-parity-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");

    for (name, rom_path, input) in [("Rev 1", REV1, &rev1), ("Rev 0", PRG0, &prg0)] {
        for (what, extra) in [("plain", Vec::new()), ("Toad", vec!["--toad"])] {
            let tag = format!("{}-{}", name.replace(' ', ""), what);
            let ips = dir.join(format!("{tag}.ips"));
            let nes = dir.join(format!("{tag}.nes"));

            run_cli(exe, rom_path, &ips, &extra, false, name, what);
            run_cli(exe, rom_path, &nes, &extra, true, name, what);

            let patch = std::fs::read(&ips).expect("cli wrote no .ips");
            let rom = std::fs::read(&nes).expect("cli wrote no .nes");
            let applied = smb3_rs::apply_ips_patch(input, &patch)
                .unwrap_or_else(|e| panic!("{name}, {what}: the CLI's patch does not apply: {e}"));

            assert!(
                applied == rom,
                "{name}, {what}: the CLI's .ips does not reproduce its own .nes"
            );
        }
    }

    let _ = std::fs::remove_dir_all(&dir);
}

fn run_cli(
    exe: &str,
    rom_path: &str,
    out: &Path,
    extra: &[&str],
    patched_rom: bool,
    name: &str,
    what: &str,
) {
    let mut cmd = Command::new(exe);
    cmd.arg(rom_path)
        .args(["--seed", &SEED.to_string()])
        .arg("--no-palettes")
        .args(["--output", out.to_str().unwrap()])
        .args(extra);
    if patched_rom {
        cmd.arg("--patched-rom");
    }
    let output = cmd.output().expect("failed to run the smb3-rs binary");
    assert!(
        output.status.success(),
        "{name}, {what}: CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
