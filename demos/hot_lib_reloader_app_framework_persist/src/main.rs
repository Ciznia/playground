//! Host for the framework-owned, persistent `hot-lib-reloader` draft.
//!
//! Identical to `hot_lib_reloader_app_framework`'s `src/main.rs` except one
//! call: `termoxide_hot_reload::run_persistent` in place of `run`, plus the
//! fixed snapshot path. That's the entire host-side cost of opting into
//! surviving a restart — see `termoxide_hot_reload::run_persistent` for
//! where the actual snapshot read/write lives, and `lib/src/lib.rs` for
//! the one line (`#[derive(Serialize, Deserialize)]`) it costs the app.

use std::path::PathBuf;

use termoxide_hot_reload::HotLibConfig;

termoxide_hot_reload::hot_lib_app!("lib", "lib/src/lib.rs", lib::AppState);

fn main() -> anyhow::Result<()> {
    termoxide_hot_reload::run_persistent(
        HotLibConfig {
            lib_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("lib"),
            package_name: "lib",
            on_tick: hot_lib::on_tick,
            handle_event: hot_lib::handle_event,
            build_view: hot_lib::build_view,
        },
        std::env::temp_dir().join("termoxide-hot-lib-reloader-framework.state.postcard"),
    )
}
