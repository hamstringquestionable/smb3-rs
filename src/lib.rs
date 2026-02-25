pub mod ips;
pub mod randomize;
pub mod randomizer;
pub mod rom;

#[cfg(target_arch = "wasm32")]
pub mod wasm;

use rom::Rom;

pub use randomizer::{FortressRedistribute, LevelShuffle, Options};

/// Generate an IPS patch from a ROM with the given seed and options.
/// Returns the IPS patch bytes.
pub fn generate_patch(rom_data: &[u8], seed: u64, options: &Options) -> Result<Vec<u8>, String> {
    let mut rom = Rom::from_bytes(rom_data).map_err(|e| e.to_string())?;
    randomizer::randomize(&mut rom, seed, options);
    Ok(ips::build_ips_patch(&rom.original, &rom.data))
}

/// Generate a patched ROM from a ROM with the given seed and options.
/// Returns the full modified ROM bytes.
pub fn generate_patched_rom(
    rom_data: &[u8],
    seed: u64,
    options: &Options,
) -> Result<Vec<u8>, String> {
    let mut rom = Rom::from_bytes(rom_data).map_err(|e| e.to_string())?;
    randomizer::randomize(&mut rom, seed, options);
    Ok(rom.data)
}
