use wasm_bindgen::prelude::*;

use crate::Options;

#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Validate that `rom` is a recognized SMB3 (USA) (Rev 1) dump. Intended to be
/// called from JS at upload time so the user sees errors immediately instead of
/// after clicking Generate. `skip_validation` mirrors the user-facing flag.
#[wasm_bindgen]
pub fn validate_rom(rom: &[u8], skip_validation: bool) -> Result<(), JsError> {
    crate::validate_rom_bytes(rom, skip_validation).map_err(|e| JsError::new(&e))
}

#[wasm_bindgen]
pub fn generate_patch(
    rom: &[u8],
    seed: u64,
    options_json: &str,
    visual_patch: Option<Vec<u8>>,
) -> Result<Vec<u8>, JsError> {
    let options: Options = parse_options(options_json)?;
    crate::generate_patch(rom, seed, &options, visual_patch.as_deref())
        .map_err(|e| JsError::new(&e))
}

#[wasm_bindgen]
pub fn generate_patched_rom(
    rom: &[u8],
    seed: u64,
    options_json: &str,
    visual_patch: Option<Vec<u8>>,
) -> Result<Vec<u8>, JsError> {
    let options: Options = parse_options(options_json)?;
    crate::generate_patched_rom(rom, seed, &options, visual_patch.as_deref())
        .map_err(|e| JsError::new(&e))
}

#[wasm_bindgen]
pub fn build_ips_patch(original: &[u8], modified: &[u8]) -> Result<Vec<u8>, JsError> {
    if original.len() != modified.len() {
        return Err(JsError::new("ROM sizes must match for diffing"));
    }
    Ok(crate::ips::build_ips_patch(original, modified))
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
