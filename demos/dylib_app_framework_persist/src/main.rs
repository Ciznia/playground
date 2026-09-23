//! Host for the framework-owned, persistent dylib-view-swap draft.
//!
//! Identical to `dylib_app_framework`'s `src/main.rs` except one call:
//! `termoxide_hot_reload::run_persistent` in place of `run`, plus the
//! fixed snapshot path. `logic/src/lib.rs` needs no change at all — same
//! reasoning as the wasm-framework persist variant: host was already
//! holding `AppState` as opaque, already-`postcard`-encoded bytes for the
//! live reload boundary, so persistence just writes those same bytes to
//! disk.

use std::path::PathBuf;

use termoxide_hot_reload::DylibConfig;

fn main() -> anyhow::Result<()> {
    termoxide_hot_reload::run_persistent(
        DylibConfig { logic_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("logic") },
        std::env::temp_dir().join("termoxide-dylib-framework.state.postcard"),
    )
}
