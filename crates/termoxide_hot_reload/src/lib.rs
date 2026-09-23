//! Generic host runtime for the `hot-lib-reloader`-based hot-reload
//! transport, moved out of `demos/hot_lib_reloader_app`'s `host/src/main.rs`
//! and into a reusable framework crate.
//!
//! Compare this to the `hot_reload_hot_lib_reloader` (non-framework) draft:
//! there, every app author had to hand-write the toolchain-`PATH` fixup,
//! the rebuild watcher, and the whole event/tick/render loop themselves.
//! Here, an app's `main.rs` is just [`hot_lib_app!`] (generating the
//! `hot_lib_reloader` glue) plus one call to [`run`]. The app's only real
//! surface is its `State` type and three plain functions operating on it —
//! see the `demos/hot_lib_reloader_app_framework/lib` crate for what that
//! looks like in practice.
//!
//! # What's still unavoidably app-side
//!
//! [`hot_lib_app!`] still needs a literal path to the app's `lib/src/lib.rs`
//! — `hot_lib_reloader::hot_module` parses that file at the app's own
//! compile time to generate typed wrapper functions matching its exact
//! signatures, so the path can't be hidden inside this crate (it has no
//! way to know where the *app's* swappable crate lives). Every other piece
//! — the loop, the watcher, the Windows `PATH` fixup — has no app-specific
//! information in it and lives here instead.
//!
//! # Opt-in persistence: [`run_persistent`]
//!
//! A `lib` reload never touches `S` — it's owned by [`run`]/[`run_persistent`]
//! for the whole process lifetime, so it's never at risk from a hot swap.
//! The only way this draft loses state is a full restart of the *host*
//! process (needed when the app's `main.rs` changes, since that isn't
//! hot-swappable). [`run_persistent`] opts into surviving that: it writes
//! `S` to a snapshot file after every tick and event, and reads it back on
//! startup if present.
//!
//! Persistence is the one place a wire format is unavoidable — the state
//! has to cross an actual process boundary (this run of `host` writing a
//! file, a *later* run of `host` reading it back), unlike the live reload
//! boundary, which stays in-process the whole time. Rather than have every
//! app hand-write that serialization, `S` only needs to derive
//! `serde::Serialize`/`Deserialize`; [`run_persistent`] handles encoding
//! (via `postcard`) generically.

use std::{
    env,
    path::{Path, PathBuf},
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
use serde::{Serialize, de::DeserializeOwned};
use termoxide_event::{EventStream, event::Event};
use termoxide_rendering::{renderer::Renderer, view_node::ViewNode};

const TICK_INTERVAL: Duration = Duration::from_millis(100);
const FRAME_INTERVAL: Duration = Duration::from_millis(16);
const DEBOUNCE: Duration = Duration::from_millis(300);

/// Generates the `hot_lib_reloader::hot_module` boilerplate every
/// hot-lib-reloader app needs, so the app itself only names its dylib
/// package, the swappable source file, and its own state type:
///
/// ```ignore
/// termoxide_hot_reload::hot_lib_app!("lib", "lib/src/lib.rs", lib::AppState);
/// ```
///
/// The other re-exported types (`Rect`, `Event`, `ViewNode`) are fixed by
/// this framework, not chosen per app — every hot-lib-reloader app built on
/// top of `termoxide_hot_reload` uses the same three types across the
/// reload boundary. `State` can't be fixed the same way (it's the one
/// piece of this that's genuinely app-specific), so it's the macro's one
/// required argument beyond the file paths.
#[macro_export]
macro_rules! hot_lib_app {
    ($dylib:literal, $path:literal, $state:path) => {
        #[hot_lib_reloader::hot_module(dylib = $dylib)]
        mod hot_lib {
            hot_functions_from_file!($path);

            pub use ratatui::layout::Rect;
            pub use termoxide_event::event::Event;
            pub use termoxide_rendering::view_node::ViewNode;
            pub use $state;
        }
    };
}

/// Puts the rustup toolchain's `bin` directory (where `std-<hash>.dll`
/// lives) on this process's `PATH`, so Windows's loader can find it when
/// `hot-lib-reloader` loads a Rust `dylib` (as opposed to `cdylib`) built
/// against that toolchain. A no-op on non-Windows targets, where this
/// hasn't been observed to be necessary. Safe to call more than once.
pub fn ensure_toolchain_dlls_are_loadable() -> Result<()> {
    #[cfg(windows)]
    {
        let output = Command::new("rustc")
            .args(["--print", "sysroot"])
            .output()
            .context("failed to run `rustc --print sysroot`")?;
        if !output.status.success() {
            bail!("`rustc --print sysroot` failed: {}", String::from_utf8_lossy(&output.stderr));
        }
        let toolchain_bin = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()).join("bin");

        let mut paths: Vec<PathBuf> = env::var_os("PATH").map(|p| env::split_paths(&p).collect()).unwrap_or_default();
        if !paths.contains(&toolchain_bin) {
            paths.insert(0, toolchain_bin);
            let new_path = env::join_paths(paths).context("failed to rebuild PATH")?;
            // Safety: called once, at the very start of `run`, before any
            // other thread that might read `PATH` concurrently has been
            // spawned.
            unsafe {
                env::set_var("PATH", new_path);
            }
        }
    }

    Ok(())
}

/// Watches `lib_dir/src` and `lib_dir/Cargo.toml`, running `cargo build -p
/// <package_name>` on change (debounced). `hot-lib-reloader` only watches
/// the already-built dylib and swaps it in when that file changes — it
/// deliberately doesn't invoke `cargo build` itself, so this is the one
/// piece of the rebuild pipeline every hot-lib-reloader app still needs.
/// Runs for the process's lifetime; nothing to join.
fn spawn_rebuild_watcher(lib_dir: PathBuf, package_name: &'static str) -> Result<()> {
    let (tx, rx) = mpsc::channel();
    let mut watcher: RecommendedWatcher = notify::recommended_watcher(move |event| {
        let _ = tx.send(event);
    })
    .context("failed to start file watcher")?;
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

            eprintln!("[termoxide_hot_reload] change detected, rebuilding {package_name}...");
            match Command::new("cargo").args(["build", "-p", package_name]).status() {
                Ok(status) if status.success() => {
                    eprintln!("[termoxide_hot_reload] {package_name} rebuilt; hot-lib-reloader will pick it up shortly")
                },
                Ok(_) => {
                    eprintln!(
                        "[termoxide_hot_reload] {package_name} build failed; keeping the currently loaded version"
                    )
                },
                Err(e) => eprintln!("[termoxide_hot_reload] failed to run cargo build: {e}"),
            }
        }
    });

    Ok(())
}

/// Everything the framework needs from a hot-lib-reloader-based app that it
/// can't infer generically: where the swappable crate lives, and the three
/// function pointers resolved from the app's own `hot_lib_app!`-generated
/// module (e.g. `hot_lib::on_tick`, `hot_lib::handle_event`,
/// `hot_lib::build_view`).
pub struct HotLibConfig<S> {
    /// Directory of the hot-swappable crate (its `Cargo.toml`'s parent).
    pub lib_dir: PathBuf,
    /// That crate's package name, passed to `cargo build -p <name>`.
    pub package_name: &'static str,
    pub on_tick: fn(&mut S),
    pub handle_event: fn(&mut S, Event) -> bool,
    pub build_view: fn(&S, Rect) -> ViewNode,
}

/// Shared loop body for [`run`] and [`run_persistent`]: terminal setup,
/// event/tick/render loop, teardown. `after_change` is called with the
/// latest state right after every event and tick — [`run`] gives it a
/// no-op, [`run_persistent`] gives it the snapshot writer, so persistence
/// adds no branching to the loop itself.
fn drive<S>(config: HotLibConfig<S>, mut state: S, mut after_change: impl FnMut(&S)) -> Result<()> {
    let events = EventStream::new();
    let terminal = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;
    let mut renderer = Renderer::new(terminal)?;

    let mut last_tick = Instant::now();
    let mut quit_requested = false;

    while !quit_requested {
        let frame_start = Instant::now();

        for event in events.poll_events() {
            quit_requested = (config.handle_event)(&mut state, event);
            after_change(&state);
        }

        if last_tick.elapsed() >= TICK_INTERVAL {
            (config.on_tick)(&mut state);
            after_change(&state);
            last_tick = Instant::now();
        }

        let viewport = renderer.viewport();
        let mut root = (config.build_view)(&state, viewport);

        // Appended by the framework, not the app — proof, alongside the
        // pid, that this line (and the process) survive a reload the app's
        // `lib` crate has no part in.
        let footer_row = root.children.len() as u16;
        if footer_row < viewport.height {
            let area = Rect::new(viewport.x, viewport.y.saturating_add(footer_row), viewport.width, 1);
            let text = format!("pid: {} (host-owned, never resets)", std::process::id());
            root.children
                .push(ViewNode::text(area, text, Style::default().fg(Color::Magenta)));
        }

        renderer.render_frame(&mut root)?;

        if let Some(remaining) = FRAME_INTERVAL.checked_sub(frame_start.elapsed()) {
            std::thread::sleep(remaining);
        }
    }

    let _ = events.teardown();
    Ok(())
}

/// Runs the whole host loop for a hot-lib-reloader-based app: the Windows
/// `PATH` fixup, the rebuild watcher, terminal setup, and the event/tick/
/// render loop, until [`HotLibConfig::handle_event`] reports quit.
///
/// `S` is the app's own state type, owned here for the process's whole
/// lifetime — exactly like every other draft in this set, a reload only
/// swaps the *functions* that operate on `S`, never `S` itself, so state
/// is never at risk from a `lib` reload. It's still lost on a restart of
/// the host process itself, needed when the app's `main.rs` changes (not
/// hot-swappable) — see [`run_persistent`] for the opt-in snapshot on top
/// of this.
pub fn run<S: Default>(config: HotLibConfig<S>) -> Result<()> {
    ensure_toolchain_dlls_are_loadable()?;
    spawn_rebuild_watcher(config.lib_dir.clone(), config.package_name)?;
    drive(config, S::default(), |_| {})
}

fn read_snapshot<S: DeserializeOwned>(path: &Path) -> Option<S> {
    let bytes = std::fs::read(path).ok()?;
    postcard::from_bytes(&bytes).ok()
}

fn write_snapshot<S: Serialize>(path: &Path, state: &S) {
    // Best-effort: a failed write here shouldn't take down a running app
    // over a dev-time convenience feature.
    if let Ok(bytes) = postcard::to_allocvec(state) {
        let _ = std::fs::write(path, bytes);
    }
}

/// Same as [`run`], but opts into surviving a restart of the host process
/// itself: `S` is read back from `snapshot_path` on startup if present
/// (falling back to `S::default()` otherwise), and written back to that
/// same path after every tick and event. Encoding is handled generically
/// via `postcard` — the app only needs `S: Serialize + DeserializeOwned`
/// (an ordinary `#[derive(Serialize, Deserialize)]`), no hand-written
/// pack/unpack code.
///
/// `snapshot_path` is fixed rather than pid-keyed, deliberately — the
/// whole point is finding the file across a pid change.
pub fn run_persistent<S: Default + Serialize + DeserializeOwned>(
    config: HotLibConfig<S>,
    snapshot_path: PathBuf,
) -> Result<()> {
    ensure_toolchain_dlls_are_loadable()?;
    spawn_rebuild_watcher(config.lib_dir.clone(), config.package_name)?;

    let state = match read_snapshot::<S>(&snapshot_path) {
        Some(state) => {
            eprintln!("[termoxide_hot_reload] restored state from {}", snapshot_path.display());
            state
        },
        None => S::default(),
    };
    eprintln!("[termoxide_hot_reload] state persistence on ({})", snapshot_path.display());

    drive(config, state, |state| write_snapshot(&snapshot_path, state))
}
