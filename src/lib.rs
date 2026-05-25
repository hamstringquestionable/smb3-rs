pub mod ips;
pub mod randomize;
pub mod randomizer;
pub mod rom;

#[cfg(target_arch = "wasm32")]
pub mod wasm;

use rom::Rom;

pub use ips::apply_ips_patch;
pub use randomizer::{
    EnemyMode, Options, STARTING_LIVES_VALUES,
    ITEM_RANDOM, ITEM_RANDOM_NO_WHISTLE, ITEM_RANDOM_SUIT_ONLY,
};

/// Validate a ROM blob without doing any randomization. Returns Ok if the bytes
/// match the expected SMB3 (USA) (Rev 1) layout and payload CRC. When
/// `skip_validation` is true, only the bare-minimum size check runs (matching
/// the contract of `Rom::from_bytes_lax`).
///
/// Exposed for callers that want to fail fast at upload time (e.g. the web UI
/// validates as soon as the user picks a file, before they hit Generate).
pub fn validate_rom_bytes(bytes: &[u8], skip_validation: bool) -> Result<(), String> {
    Rom::from_bytes_lax(bytes, skip_validation)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Parse, validate, optionally apply a visual patch, randomize, and return the
/// full Rom struct. Visual-patch bytes are applied before randomization, so the
/// resulting IPS diff (`original` → `data`) captures both visual and
/// randomization changes in a single output.
pub fn randomize_rom(
    rom_data: &[u8],
    seed: u64,
    options: &Options,
    visual_patch: Option<&[u8]>,
) -> Result<Rom, String> {
    let mut rom = Rom::from_bytes_lax(rom_data, options.skip_rom_validation)
        .map_err(|e| e.to_string())?;
    if let Some(patch) = visual_patch {
        rom.apply_ips_patch(patch)?;
    }
    randomizer::randomize(&mut rom, seed, options);
    Ok(rom)
}

/// Generate an IPS patch from a ROM with the given seed and options.
/// The IPS captures any visual-patch bytes plus randomization changes.
pub fn generate_patch(
    rom_data: &[u8],
    seed: u64,
    options: &Options,
    visual_patch: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    let rom = randomize_rom(rom_data, seed, options, visual_patch)?;
    Ok(ips::build_ips_patch(rom.original_bytes(), rom.output_bytes()))
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
    let mut rom = Rom::from_bytes_lax(rom_data, options.skip_rom_validation)
        .map_err(|e| e.to_string())?;
    if let Some(patch) = visual_patch {
        rom.apply_ips_patch(patch)?;
    }
    let mut capture: Option<randomize::overworld_build::BuildResult> = None;
    randomizer::randomize_with_overworld_capture(&mut rom, seed, options, &mut capture);
    let build = capture.ok_or_else(|| "overworld capture not populated".to_string())?;
    Ok((rom, build))
}
