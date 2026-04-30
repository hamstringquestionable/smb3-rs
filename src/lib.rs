pub mod ips;
pub mod randomize;
pub mod randomizer;
pub mod rom;

#[cfg(target_arch = "wasm32")]
pub mod wasm;

use rom::Rom;

pub use ips::apply_ips_patch;
pub use randomizer::{
    EnemyMode, LevelShuffle, Options,
    ITEM_RANDOM, ITEM_RANDOM_NO_WHISTLE, ITEM_RANDOM_SUIT_ONLY,
};

/// Parse, validate, randomize, and return the full Rom struct.
pub fn randomize_rom(rom_data: &[u8], seed: u64, options: &Options) -> Result<Rom, String> {
    let mut rom = Rom::from_bytes_lax(rom_data, options.skip_rom_validation)
        .map_err(|e| e.to_string())?;
    randomizer::randomize(&mut rom, seed, options);
    Ok(rom)
}

/// Generate an IPS patch from a ROM with the given seed and options.
/// Returns the IPS patch bytes.
pub fn generate_patch(rom_data: &[u8], seed: u64, options: &Options) -> Result<Vec<u8>, String> {
    let rom = randomize_rom(rom_data, seed, options)?;
    Ok(ips::build_ips_patch(rom.original_bytes(), rom.output_bytes()))
}

/// Generate a patched ROM from a ROM with the given seed and options.
/// Returns the full modified ROM bytes.
pub fn generate_patched_rom(
    rom_data: &[u8],
    seed: u64,
    options: &Options,
) -> Result<Vec<u8>, String> {
    let rom = randomize_rom(rom_data, seed, options)?;
    Ok(rom.output_bytes().to_vec())
}
