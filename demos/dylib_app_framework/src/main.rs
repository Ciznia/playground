//! Host for the framework-owned dylib-view-swap draft.
//!
//! Compare to `demos/dylib_app`'s `host/src/main.rs` in the non-framework
//! draft: everything that file hand-rolled (the Windows generation-numbered
//! copy trick, `libloading`, the terminal/event/render loop) now lives in
//! `termoxide_hot_reload`. This file's only job is to point it at where
//! `logic` lives — no app-specific type or function pointer to hand over,
//! since the host never needs to know `logic`'s `State` type.

use std::path::PathBuf;

use termoxide_hot_reload::DylibConfig;

fn main() -> anyhow::Result<()> {
    termoxide_hot_reload::run(DylibConfig { logic_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("logic") })
}
