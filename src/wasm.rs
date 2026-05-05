use wasm_bindgen::prelude::*;

use crate::Options;

#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[wasm_bindgen]
pub fn generate_patch(rom: &[u8], seed: u64, options_json: &str) -> Result<Vec<u8>, JsError> {
    let options: Options = parse_options(options_json)?;
    crate::generate_patch(rom, seed, &options).map_err(|e| JsError::new(&e))
}

#[wasm_bindgen]
pub fn generate_patched_rom(rom: &[u8], seed: u64, options_json: &str) -> Result<Vec<u8>, JsError> {
    let options: Options = parse_options(options_json)?;
    crate::generate_patched_rom(rom, seed, &options).map_err(|e| JsError::new(&e))
}

/// SMB3 USA Rev 1 CHR ROM region: file offset range that holds tile graphics.
/// 16-byte iNES header + 256 KB PRG + 128 KB CHR = 393,232 bytes total.
const CHR_START: usize = 0x40010;
const ROM_END: usize = 0x60010;

#[wasm_bindgen]
pub fn apply_visual_patch(rom: &[u8], patch: &[u8]) -> Result<Vec<u8>, JsError> {
    crate::ips::validate_region(patch, CHR_START, ROM_END).map_err(|e| JsError::new(&e))?;
    crate::apply_ips_patch(rom, patch).map_err(|e| JsError::new(&e))
}

/// Standalone CHR-region validation, separate from apply, so the JS layer
/// can give the user instant feedback at file-select time.
#[wasm_bindgen]
pub fn validate_visual_patch(patch: &[u8]) -> Result<(), JsError> {
    crate::ips::validate_region(patch, CHR_START, ROM_END).map_err(|e| JsError::new(&e))
}

fn parse_options(json: &str) -> Result<Options, JsError> {
    serde_json::from_str(json).map_err(|e| JsError::new(&format!("Invalid options: {e}")))
}

/// Serialize the canonical Options::default() as JSON so the JS layer can
/// assert its schema covers every field (and only the fields) the Rust
/// source of truth knows about. Drift is reported on page load.
#[wasm_bindgen]
pub fn default_options_json() -> Result<String, JsError> {
    serde_json::to_string(&Options::default())
        .map_err(|e| JsError::new(&format!("Serialize error: {e}")))
}

#[wasm_bindgen]
pub fn encode_flag_key(options_json: &str) -> Result<String, JsError> {
    let options: Options = parse_options(options_json)?;
    Ok(options.to_flag_key())
}

#[wasm_bindgen]
pub fn decode_flag_key(key: &str) -> Result<String, JsError> {
    let options = Options::from_flag_key(key).map_err(|e| JsError::new(&e))?;
    serde_json::to_string(&options).map_err(|e| JsError::new(&format!("Serialize error: {e}")))
}
