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

#[wasm_bindgen]
pub fn apply_visual_patch(rom: &[u8], patch: &[u8]) -> Result<Vec<u8>, JsError> {
    crate::apply_ips_patch(rom, patch).map_err(|e| JsError::new(&e))
}

fn parse_options(json: &str) -> Result<Options, JsError> {
    serde_json::from_str(json).map_err(|e| JsError::new(&format!("Invalid options: {e}")))
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
