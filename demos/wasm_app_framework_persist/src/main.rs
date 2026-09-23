//! Host for the framework-owned, persistent wasm draft.
//!
//! Identical to `wasm_app_framework`'s `src/main.rs` except one call:
//! `termoxide_hot_reload::run_persistent` in place of `run`, plus the
//! fixed snapshot path. Unlike the hot-lib-reloader-framework persist
//! variant, `logic/src/lib.rs` needs **no change at all** — `AppState`
//! doesn't gain a derive, because host was already holding the app's
//! state as opaque, already-encoded bytes (see
//! `termoxide_hot_reload::run_persistent`'s docs for why).

use std::path::PathBuf;

use termoxide_hot_reload::WasmConfig;

fn main() -> anyhow::Result<()> {
    termoxide_hot_reload::run_persistent(
        WasmConfig { logic_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("logic") },
        std::env::temp_dir().join("termoxide-wasm-framework.state.postcard"),
    )
}
