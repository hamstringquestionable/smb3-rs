use wasm_bindgen::prelude::*;

use crate::Options;

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

fn parse_options(json: &str) -> Result<Options, JsError> {
    serde_json::from_str(json).map_err(|e| JsError::new(&format!("Invalid options: {e}")))
}
