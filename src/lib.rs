pub mod ips;
pub mod randomize;
pub mod randomizer;
pub mod rom;

/// Playtest ROM assembly. Native-only: it exists to serve the CLI and has no
/// role in the web build, which never needs a ROM the randomizer wouldn't make.
#[cfg(not(target_arch = "wasm32"))]
pub mod testrom;

#[cfg(target_arch = "wasm32")]
pub mod wasm;

use rom::Rom;

pub use ips::apply_ips_patch;
pub use randomizer::{
    DejaVuMode, EnemyMode, FireFlowerMode, HazardLimit, HintMode, ITEM_RANDOM,
    ITEM_RANDOM_NO_WHISTLE, ITEM_RANDOM_SUIT_ONLY, ITEMS, Options, PiranhaMode,
    STARTING_LIVES_VALUES, Tri, WildChaser, current_flag_key_version, flag_key_fields,
    flag_key_version_of, item_display_name, item_id,
};

/// Which SMB3 (USA) revision a ROM blob turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RomRevision {
    /// Rev 1 (PRG1) — what the randomizer targets, used as supplied.
    Rev1,
    /// Rev 0 (PRG0) — accepted, and converted to Rev 1 at load time. Callers
    /// should tell the user, since the ROM they get back is not the revision
    /// they handed over.
    Prg0Converted,
}

/// Validate a ROM blob without doing any randomization. Returns which revision
/// it is if the bytes match the expected SMB3 (USA) layout and one of the two
/// known payload CRCs. When `skip_validation` is true, only the bare-minimum
/// size check runs (matching the contract of `Rom::from_bytes_lax`) — and with
/// no CRC computed there is no revision to report, so the result is `Rev1`
/// whatever the bytes are.
///
/// Exposed for callers that want to fail fast at upload time (e.g. the web UI
/// validates as soon as the user picks a file, before they hit Generate) and
/// to warn about the Rev 0 conversion at the same moment.
pub fn validate_rom_bytes(bytes: &[u8], skip_validation: bool) -> Result<RomRevision, String> {
    Rom::from_bytes_lax(bytes, skip_validation)
        .map(
            |rom| {
                if rom.converted_from_prg0() {
                    RomRevision::Prg0Converted
                } else {
                    RomRevision::Rev1
                }
            },
        )
        .map_err(|e| e.to_string())
}

/// Parse, validate, optionally apply a visual patch, randomize, and return the
/// full Rom struct. Visual-patch bytes are applied before randomization, so the
/// resulting IPS diff (`ips_baseline_bytes` → `data`) captures both visual and
/// randomization changes in a single output.
pub fn randomize_rom(
    rom_data: &[u8],
    seed: u64,
    options: &Options,
    visual_patch: Option<&[u8]>,
) -> Result<Rom, String> {
    match visual_patch {
        Some(patch) => {
            randomize_rom_with_patches(rom_data, seed, options, &[("visual_patch", patch)])
        }
        None => randomize_rom_with_patches(rom_data, seed, options, &[]),
    }
}

/// [`randomize_rom`] with more than one visual patch, applied in order and each
/// tagged in the write log so a collision between two of them — or between one
/// and the randomizer — is attributable rather than an anonymous byte change.
///
/// This is the single entry point every caller goes through, which is the point
/// of it: patches must land *after* the `Rom` is built, because that is where a
/// Rev 0 input becomes Rev 1. A caller that patches the raw bytes first moves
/// the payload CRC, and the revision check then recognizes neither revision.
pub fn randomize_rom_with_patches(
    rom_data: &[u8],
    seed: u64,
    options: &Options,
    visual_patches: &[(&str, &[u8])],
) -> Result<Rom, String> {
    let mut rom =
        Rom::from_bytes_lax(rom_data, options.skip_rom_validation).map_err(|e| e.to_string())?;
    for (tag, patch) in visual_patches {
        rom.apply_ips_patch(patch, tag)?;
    }
    randomizer::randomize(&mut rom, seed, options);
    Ok(rom)
}

/// Generate an IPS patch from a ROM with the given seed and options.
/// The IPS captures any visual-patch bytes plus randomization changes — and,
/// for a Rev 0 input, the Rev 0 -> Rev 1 conversion as well, so the patch
/// applies to the ROM that was supplied.
pub fn generate_patch(
    rom_data: &[u8],
    seed: u64,
    options: &Options,
    visual_patch: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    let rom = randomize_rom(rom_data, seed, options, visual_patch)?;
    Ok(ips::build_ips_patch(rom.ips_baseline_bytes(), rom.output_bytes()))
}

/// Generate a fully patched ROM (visual patch + randomization applied).
pub fn generate_patched_rom(
    rom_data: &[u8],
    seed: u64,
    options: &Options,
    visual_patch: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    let rom = randomize_rom(rom_data, seed, options, visual_patch)?;
    Ok(rom.output_bytes().to_vec())
}

/// Same as [`randomize_rom`] but also returns a snapshot of the overworld
/// `BuildResult` captured just before the writer stamps it onto the ROM.
/// Used by the must-clear progression analyzer (and the future WASM
/// single-seed dump endpoint) so the topology being analyzed matches what
/// a real playthrough with the same seed + options would produce.
#[allow(dead_code)] // exposed for internal tests; WASM hook to follow.
pub(crate) fn randomize_rom_with_overworld_capture(
    rom_data: &[u8],
    seed: u64,
    options: &Options,
    visual_patch: Option<&[u8]>,
) -> Result<(Rom, randomize::overworld_build::BuildResult), String> {
    let mut rom =
        Rom::from_bytes_lax(rom_data, options.skip_rom_validation).map_err(|e| e.to_string())?;
    if let Some(patch) = visual_patch {
        rom.apply_ips_patch(patch, "visual_patch")?;
    }
    let mut capture: Option<randomize::overworld_build::BuildResult> = None;
    randomizer::randomize_with_overworld_capture(&mut rom, seed, options, &mut capture);
    let build = capture.ok_or_else(|| "overworld capture not populated".to_string())?;
    Ok((rom, build))
}
