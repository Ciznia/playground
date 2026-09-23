//! Host for the framework-owned `hot-lib-reloader` draft.
//!
//! Compare to `demos/hot_lib_reloader_app`'s `src/main.rs` in the
//! non-framework draft: everything that file hand-rolled (the Windows
//! `PATH` fixup, the rebuild watcher, the terminal/event/render loop) now
//! lives in `termoxide_hot_reload`. This file's only job is to generate
//! the `hot_lib_reloader` glue (via [`termoxide_hot_reload::hot_lib_app!`])
//! and hand the framework the three function pointers it resolves to.

use std::path::PathBuf;

use termoxide_hot_reload::HotLibConfig;

termoxide_hot_reload::hot_lib_app!("lib", "lib/src/lib.rs", lib::AppState);

fn main() -> anyhow::Result<()> {
    termoxide_hot_reload::run(HotLibConfig {
        lib_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("lib"),
        package_name: "lib",
        on_tick: hot_lib::on_tick,
        handle_event: hot_lib::handle_event,
        build_view: hot_lib::build_view,
    })
}
