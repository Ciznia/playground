//! Host for the `hot-lib-reloader`-based draft.
//!
//! Same shape as the other host-resident drafts (`dylib_view_swap`,
//! `wasm`): one long-lived process, `Owner`/every `Signal` created once and
//! never re-created, only `lib`'s functions swap out on a reload. What's
//! different is how little of the swap machinery this file has to write
//! itself — `hot_lib_reloader::hot_module` generates the watch-reload-and-
//! re-resolve-symbols machinery `dylib_view_swap`'s `host` hand-rolled with
//! `libloading` directly, Windows copy-before-load trick included. Compare
//! `Logic`/`build_and_load`/`spawn` in that draft's `host/src/main.rs` to
//! the four-line `hot_module` block below.
//!
//! # What this crate still has to do itself
//!
//! `hot-lib-reloader` only handles the *reload* half: it watches the
//! already-built `target/debug/lib.dll` and swaps it in when that file
//! changes. It deliberately does not invoke `cargo build` for you — the
//! project's own docs recommend running `cargo watch -w lib -x 'build -p
//! lib'` in a second terminal. To keep this a single `cargo run`, like
//! every other draft in this set, [`spawn_rebuild_watcher`] does that one
//! remaining piece itself: watch `lib/src` and `lib/Cargo.toml`, and shell
//! out to `cargo build -p lib` on change. It's the same few lines every
//! other draft's watcher already needed.
//!
//! No application state survives a restart of `host` itself (needed when
//! `src/main.rs` changes, since `host` isn't hot-swappable — only `lib`
//! is). A `lib` reload never touches `count`/`ticks`/`last_key` though, so
//! that's the only state loss possible. See the
//! `hot_reload_hot_lib_reloader_persist` draft worktree for the opt-in
//! state snapshot built on top of this one.
//!
//! # A Windows-specific rough edge: `lib.dll` won't load at all
//!
//! `hot-lib-reloader` requires `lib`'s crate-type to include `dylib` (not
//! `cdylib` — see `lib/Cargo.toml`). On Windows, a Rust `dylib` links
//! *dynamically* against the toolchain's own `std-<hash>.dll`, which lives
//! in the rustup toolchain's `bin` directory
//! (`~/.rustup/toolchains/<toolchain>/bin/`) — not `~/.cargo/bin`, the
//! shim directory that's normally the only rustup path on `PATH`. Without
//! it on `PATH`, loading `lib.dll` fails with a bare "module not found"
//! (`LoadLibraryExW` error 126) that gives no hint it's a *different*
//! missing DLL, not `lib.dll` itself — confirmed with `dumpbin
//! /dependents` while building this draft. [`ensure_toolchain_dlls_are_loadable`]
//! fixes it by asking `rustc` for its own sysroot and prepending that
//! `bin` directory to this process's `PATH` before the first `hot_lib`
//! call. This is very likely specific to `dylib` (vs. the `cdylib` the
//! other native drafts use) and to Windows — worth confirming if this
//! draft ever runs on Linux/macOS.

use std::{
    env,
    io::stdout,
    path::PathBuf,
    process::Command,
    sync::mpsc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::Rect,
    style::{Color, Style},
};
use termoxide_event::EventStream;
use termoxide_reactive::{Owner, Signal};
use termoxide_rendering::{renderer::Renderer, view_node::ViewNode};

#[hot_lib_reloader::hot_module(dylib = "lib")]
mod hot_lib {
    hot_functions_from_file!("lib/src/lib.rs");

    // The macro copies each function's signature verbatim into this
    // module, so every type appearing in one needs to be in scope here.
    pub use ratatui::layout::Rect;
    pub use termoxide_event::event::Event;
    pub use termoxide_rendering::view_node::ViewNode;
}

const TICK_INTERVAL: Duration = Duration::from_millis(100);
const FRAME_INTERVAL: Duration = Duration::from_millis(16);
const DEBOUNCE: Duration = Duration::from_millis(300);

/// The one piece of rebuild-triggering `hot-lib-reloader` leaves to us —
/// see the module docs. Runs for the process's lifetime; nothing to join.
fn spawn_rebuild_watcher(lib_dir: PathBuf) -> Result<()> {
    let (tx, rx) = mpsc::channel();
    let mut watcher: RecommendedWatcher =
        notify::recommended_watcher(move |event| { let _ = tx.send(event); }).context("failed to start file watcher")?;
    watcher
        .watch(&lib_dir.join("src"), RecursiveMode::Recursive)
        .with_context(|| format!("failed to watch {}", lib_dir.join("src").display()))?;
    watcher
        .watch(&lib_dir.join("Cargo.toml"), RecursiveMode::NonRecursive)
        .with_context(|| format!("failed to watch {}", lib_dir.join("Cargo.toml").display()))?;

    std::thread::spawn(move || {
        let _watcher = watcher; // kept alive for the thread's lifetime
        loop {
            if rx.recv().is_err() {
                break;
            }
            while rx.recv_timeout(DEBOUNCE).is_ok() {}

            eprintln!("[hot_lib_reloader_app] change detected, rebuilding lib...");
            match Command::new("cargo").args(["build", "-p", "lib"]).status() {
                Ok(status) if status.success() => {
                    eprintln!("[hot_lib_reloader_app] lib rebuilt; hot-lib-reloader will pick it up shortly")
                },
                Ok(_) => eprintln!("[hot_lib_reloader_app] lib build failed; keeping the currently loaded version"),
                Err(e) => eprintln!("[hot_lib_reloader_app] failed to run cargo build: {e}"),
            }
        }
    });

    Ok(())
}

/// Puts the rustup toolchain's `bin` directory (where `std-<hash>.dll`
/// lives) on this process's `PATH`, so `LoadLibraryExW` can find it when
/// `hot-lib-reloader` loads `lib.dll`. See the module docs for why this is
/// needed at all. Safe to call more than once; a no-op after the first.
fn ensure_toolchain_dlls_are_loadable() -> Result<()> {
    let output = Command::new("rustc").args(["--print", "sysroot"]).output().context("failed to run `rustc --print sysroot`")?;
    if !output.status.success() {
        bail!("`rustc --print sysroot` failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    let toolchain_bin = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()).join("bin");

    let mut paths: Vec<PathBuf> = env::var_os("PATH").map(|p| env::split_paths(&p).collect()).unwrap_or_default();
    if !paths.contains(&toolchain_bin) {
        paths.insert(0, toolchain_bin);
        let new_path = env::join_paths(paths).context("failed to rebuild PATH")?;
        // Safety: called once, at the very start of `main`, before any
        // other thread that might read `PATH` concurrently has been spawned.
        unsafe {
            env::set_var("PATH", new_path);
        }
    }

    Ok(())
}

fn main() -> Result<()> {
    ensure_toolchain_dlls_are_loadable()?;

    let owner = Owner::new();
    owner.set();
    let count = Signal::new(0u32);
    let ticks = Signal::new(0u64);
    let last_key = Signal::new(String::from("waiting for input"));

    let lib_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("lib");
    spawn_rebuild_watcher(lib_dir)?;

    let events = EventStream::new();
    let terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let mut renderer = Renderer::new(terminal)?;

    let mut last_tick = Instant::now();
    let mut quit_requested = false;

    while !quit_requested {
        let frame_start = Instant::now();

        for event in events.poll_events() {
            // Calling any `hot_lib` function is what makes hot-lib-reloader
            // check for and apply a pending reload, transparently, before
            // running the call against whichever generation is current.
            let (new_count, new_last_key, quit) = hot_lib::handle_event(count.get_untracked(), event);
            count.set(new_count);
            last_key.set(new_last_key);
            quit_requested = quit;
        }

        if last_tick.elapsed() >= TICK_INTERVAL {
            let new_ticks = hot_lib::on_tick(ticks.get_untracked());
            ticks.set(new_ticks);
            last_tick = Instant::now();
        }

        let viewport = renderer.viewport();
        let mut root = hot_lib::build_view(viewport, count.get_untracked(), ticks.get_untracked(), last_key.get_untracked());

        // Appended by `host`, not `lib` — proof, alongside the pid, that
        // this line (and the process) survive a reload `lib` has no part in.
        let footer_row = root.children.len() as u16;
        if footer_row < viewport.height {
            let area = Rect::new(viewport.x, viewport.y.saturating_add(footer_row), viewport.width, 1);
            let text = format!("pid: {} (host-owned, never resets)", std::process::id());
            root.children.push(ViewNode::text(area, text, Style::default().fg(Color::Magenta)));
        }

        renderer.render_frame(&mut root)?;

        if let Some(remaining) = FRAME_INTERVAL.checked_sub(frame_start.elapsed()) {
            std::thread::sleep(remaining);
        }
    }

    let _ = events.teardown();
    Ok(())
}
