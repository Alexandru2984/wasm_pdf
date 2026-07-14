//! JavaScript boundary for the dedicated PDF Web Worker.

use wasm_bindgen::prelude::*;

/// Installs readable panic messages in the browser console.
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}
