//! Host for the framework-owned wasm draft.
//!
//! Compare to `demos/wasm_app`'s `host/src/main.rs` in the non-framework
//! draft: everything that file hand-rolled (the rebuild watcher, wasmtime
//! module loading, the terminal/event/render loop) now lives in
//! `termoxide_hot_reload`. This file's only job is to point it at where
//! `logic` lives — there's no app-specific type or function pointer to
//! hand over, since the host never needs to know `logic`'s `State` type
//! (see `termoxide_hot_reload`'s crate docs).

use std::path::PathBuf;

use termoxide_hot_reload::WasmConfig;

fn main() -> anyhow::Result<()> {
    termoxide_hot_reload::run(WasmConfig { logic_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("logic") })
}
