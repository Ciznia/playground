//! Long-lived host for the dylib/view-swap hot-reload draft.
//!
//! Unlike the `hot_reload_process_supervisor` draft, there is no supervisor
//! process here and no process restart at all: `host` is started once (`cargo
//! run`) and stays up for the whole session. It owns the reactive `Owner`
//! and every `Signal` for the app's lifetime, the terminal, and the input
//! reader. The only thing that changes on a reload is which `logic` dylib
//! generation supplies `on_tick`/`handle_key`/`build_view` — see
//! `termoxide_dylib_abi` for why that boundary is plain `#[repr(C)]` data
//! rather than `termoxide::App`.
//!
//! This intentionally does **not** go through `termoxide::run_with_app`: that
//! entry point assumes the whole `App` is one static Rust type, which is
//! exactly what this draft is trying not to require. `host` hand-rolls a
//! much simpler synchronous loop instead — no `tokio`, no `RenderEffect`;
//! it redraws unconditionally every ~16ms, which is enough for a draft
//! though not what a production integration would want. Framework crates
//! (`termoxide_reactive`, `termoxide_event`, `termoxide_rendering`) are used
//! directly and unmodified.
//!
//! # Windows dylib lock
//!
//! Same problem as the process-supervisor draft's `.exe`, one level lower:
//! a process can't have the linker overwrite a `.dll` it currently has
//! loaded. `host` never loads the dylib cargo actually builds
//! (`target/debug/logic.dll`) — every reload copies that fresh build to a
//! new, incrementally-numbered file first (`logic-1.dll`, `logic-2.dll`, …)
//! and loads *that*. `cargo build`'s own output path is therefore never the
//! file a `Library` currently has open, so nothing blocks the next rebuild.
//!
//! No application state survives a reload: `count`/`ticks`/`last_key` are
//! never at risk from a `logic` reload (they live entirely in `host`), but
//! a full restart of `host` itself — needed when `host/src/main.rs`
//! changes, since `host` isn't hot-swappable — does lose them. See the
//! `hot_reload_dylib_view_swap_persist` draft worktree for the opt-in
//! state snapshot built on top of this one.

use std::{
    env, fs,
    io::stdout,
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
use termoxide_dylib_abi::{
    BuildViewFn, FfiFrame, FfiKey, FfiState, FfiText, HandleKeyFn, OnTickFn, SYMBOL_BUILD_VIEW, SYMBOL_HANDLE_KEY,
    SYMBOL_ON_TICK, key_tag, modifier,
};
use termoxide_event::{
    EventStream,
    event::{Event, KeyCode, KeyModifiers},
};
use termoxide_reactive::{Owner, Signal};
use termoxide_rendering::{renderer::Renderer, view_node::ViewNode};

/// How often the loop redraws and checks for a completed tick, regardless of
/// input. No reactive redraw-on-write here (see the module docs), so this
/// doubles as the tick check cadence.
const FRAME_INTERVAL: Duration = Duration::from_millis(16);
const TICK_INTERVAL: Duration = Duration::from_millis(100);
/// Quiet period after the first file-system event before rebuilding, so a
/// burst of saves collapses into one rebuild.
const DEBOUNCE: Duration = Duration::from_millis(300);

// ─────────────────────────────────────────────────────────────────────────
//  Loading the logic dylib
// ─────────────────────────────────────────────────────────────────────────

/// A loaded `logic` generation: the library (kept alive so the function
/// pointers below stay valid) plus the three resolved entry points.
struct Logic {
    _lib: libloading::Library,
    on_tick: OnTickFn,
    handle_key: HandleKeyFn,
    build_view: BuildViewFn,
    generation: u32,
}

impl Logic {
    /// # Safety
    ///
    /// `path` must be a `logic`-dylib build produced by this same workspace
    /// (so its exported symbols match `termoxide_dylib_abi`'s function
    /// types) — loading and calling into arbitrary native code is exactly
    /// what makes this whole approach unsafe in the general case. That
    /// trust boundary is inherent to dylib hot-reloading, not specific to
    /// this draft.
    unsafe fn load(path: &Path, generation: u32) -> Result<Self> {
        unsafe {
            let lib = libloading::Library::new(path).with_context(|| format!("failed to load {}", path.display()))?;
            let on_tick = *lib.get::<OnTickFn>(SYMBOL_ON_TICK).context("missing termoxide_on_tick export")?;
            let handle_key =
                *lib.get::<HandleKeyFn>(SYMBOL_HANDLE_KEY).context("missing termoxide_handle_key export")?;
            let build_view =
                *lib.get::<BuildViewFn>(SYMBOL_BUILD_VIEW).context("missing termoxide_build_view export")?;
            Ok(Self { _lib: lib, on_tick, handle_key, build_view, generation })
        }
    }
}

/// Build `logic`, copy the result into a fresh generation file under
/// `run_dir`, and load it. See the module docs for why the copy step is
/// required on Windows.
fn build_and_load(manifest_path: &Path, run_dir: &Path, next_generation: u32) -> Result<Option<Logic>> {
    let Some(built) = build_logic(manifest_path)? else {
        return Ok(None);
    };

    let dest = run_dir.join(format!("logic-{next_generation}.{}", env::consts::DLL_EXTENSION));
    fs::copy(&built, &dest).with_context(|| format!("failed to copy {} to {}", built.display(), dest.display()))?;

    // Safety: `dest` is a `logic` dylib this same workspace just built, so
    // its exports match `termoxide_dylib_abi`.
    Ok(Some(unsafe { Logic::load(&dest, next_generation)? }))
}

/// Run `cargo build` for the `logic` crate and return its dylib artifact
/// path, or `None` if the build failed (already reported to stderr).
fn build_logic(manifest_path: &Path) -> Result<Option<PathBuf>> {
    let output = Command::new("cargo")
        .args(["build", "--message-format=json-render-diagnostics", "--manifest-path"])
        .arg(manifest_path)
        .output()
        .context("failed to run `cargo build` for logic")?;

    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Ok(message) = serde_json::from_str::<serde_json::Value>(line)
            && let Some(rendered) = message.pointer("/message/rendered").and_then(serde_json::Value::as_str)
        {
            eprint!("{rendered}");
        }
    }

    if !output.status.success() {
        eprintln!("[dylib_app host] logic build failed; keeping the currently loaded generation");
        return Ok(None);
    }

    let dylib_suffix = format!(".{}", env::consts::DLL_EXTENSION);
    let path = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|message| message["reason"] == "compiler-artifact")
        .find_map(|message| {
            message["filenames"]
                .as_array()?
                .iter()
                .find_map(|f| f.as_str().filter(|s| s.ends_with(&dylib_suffix)).map(PathBuf::from))
        })
        .context("cargo build succeeded but reported no dylib artifact for logic")?;

    Ok(Some(path))
}

// ─────────────────────────────────────────────────────────────────────────
//  FFI translation
// ─────────────────────────────────────────────────────────────────────────

fn to_ffi_key(key: termoxide_event::event::KeyEvent) -> FfiKey {
    let (tag, payload) = match key.code {
        KeyCode::Backspace => (key_tag::BACKSPACE, 0),
        KeyCode::Enter => (key_tag::ENTER, 0),
        KeyCode::Left => (key_tag::LEFT, 0),
        KeyCode::Right => (key_tag::RIGHT, 0),
        KeyCode::Up => (key_tag::UP, 0),
        KeyCode::Down => (key_tag::DOWN, 0),
        KeyCode::Home => (key_tag::HOME, 0),
        KeyCode::End => (key_tag::END, 0),
        KeyCode::PageUp => (key_tag::PAGE_UP, 0),
        KeyCode::PageDown => (key_tag::PAGE_DOWN, 0),
        KeyCode::Tab => (key_tag::TAB, 0),
        KeyCode::BackTab => (key_tag::BACK_TAB, 0),
        KeyCode::Delete => (key_tag::DELETE, 0),
        KeyCode::Insert => (key_tag::INSERT, 0),
        KeyCode::F(n) => (key_tag::F, u32::from(n)),
        KeyCode::Char(c) => (key_tag::CHAR, c as u32),
        KeyCode::Null => (key_tag::NULL, 0),
        KeyCode::Esc => (key_tag::ESC, 0),
    };

    let mut modifiers = 0u8;
    for (flag, bit) in [
        (KeyModifiers::SHIFT, modifier::SHIFT),
        (KeyModifiers::CONTROL, modifier::CONTROL),
        (KeyModifiers::ALT, modifier::ALT),
        (KeyModifiers::SUPER, modifier::SUPER),
    ] {
        if key.modifiers.contains(flag) {
            modifiers |= bit;
        }
    }

    FfiKey { tag, payload, modifiers }
}

fn color_from_tag(tag: u8) -> Color {
    match tag {
        1 => Color::Cyan,
        2 => Color::Yellow,
        3 => Color::Green,
        4 => Color::Magenta,
        _ => Color::Reset,
    }
}

/// Turn `logic`'s FFI frame into a real [`ViewNode`] tree, and append one
/// line `host` writes itself — proof, alongside the pid, that the footer
/// (and the process) survive a reload the dylib has no part in.
fn frame_to_view_node(frame: &FfiFrame, viewport: Rect, generation: u32) -> ViewNode {
    let mut children = Vec::new();

    for (i, line) in frame.lines.iter().take(frame.line_count as usize).enumerate() {
        let row = i as u16;
        if row >= viewport.height {
            break;
        }
        let area = Rect::new(viewport.x, viewport.y.saturating_add(row), viewport.width, 1);
        children.push(ViewNode::text(area, line.text.as_str().to_string(), Style::default().fg(color_from_tag(line.color.0))));
    }

    let footer_row = frame.line_count.min(u32::from(u16::MAX)) as u16;
    if footer_row < viewport.height {
        let area = Rect::new(viewport.x, viewport.y.saturating_add(footer_row), viewport.width, 1);
        let text = format!("pid: {} | logic generation: {generation} (host-owned, never resets)", std::process::id());
        children.push(ViewNode::text(area, text, Style::default().fg(Color::Magenta)));
    }

    ViewNode::container(viewport, children)
}

// ─────────────────────────────────────────────────────────────────────────
//  main
// ─────────────────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().context("no parent dir")?.join("logic").join("Cargo.toml");
    if !manifest_path.exists() {
        bail!("expected the logic crate's manifest at {}", manifest_path.display());
    }
    let logic_src_dir = manifest_path.parent().context("no parent dir")?.join("src");

    let run_dir = env::temp_dir().join(format!("termoxide-dylib-app-{}", std::process::id()));
    fs::create_dir_all(&run_dir).with_context(|| format!("failed to create {}", run_dir.display()))?;

    // Created exactly once, for the whole process lifetime — this is the
    // entire point of this draft. `logic` never touches these.
    let owner = Owner::new();
    owner.set();
    let count = Signal::new(0u32);
    let ticks = Signal::new(0u64);
    let last_key = Signal::new(String::from("waiting for input"));

    eprintln!("[dylib_app host] building logic...");
    let mut generation = 0u32;
    generation += 1;
    let mut logic = build_and_load(&manifest_path, &run_dir, generation)?
        .context("initial logic build failed; fix the error above and re-run")?;
    eprintln!("[dylib_app host] logic generation {generation} loaded, starting");

    let (tx, rx) = mpsc::channel();
    let mut watcher: RecommendedWatcher =
        notify::recommended_watcher(move |event| { let _ = tx.send(event); }).context("failed to start file watcher")?;
    watcher
        .watch(&logic_src_dir, RecursiveMode::Recursive)
        .with_context(|| format!("failed to watch {}", logic_src_dir.display()))?;
    watcher
        .watch(&manifest_path, RecursiveMode::NonRecursive)
        .with_context(|| format!("failed to watch {}", manifest_path.display()))?;

    let events = EventStream::new();
    let terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let mut renderer = Renderer::new(terminal)?;

    let mut last_tick = Instant::now();
    let mut quit_requested = false;

    while !quit_requested {
        let frame_start = Instant::now();

        // Non-blocking: rebuild only once a change has actually landed.
        if rx.try_recv().is_ok() {
            while rx.recv_timeout(DEBOUNCE).is_ok() {}
            eprintln!("[dylib_app host] change detected, rebuilding logic...");
            generation += 1;
            match build_and_load(&manifest_path, &run_dir, generation) {
                Ok(Some(new_logic)) => {
                    logic = new_logic;
                    eprintln!(
                        "[dylib_app host] logic reloaded (generation {generation}); count/ticks/last_key untouched"
                    );
                },
                Ok(None) => generation -= 1, // build failed; no new generation was actually loaded
                Err(e) => {
                    generation -= 1;
                    eprintln!("[dylib_app host] failed to load rebuilt logic: {e:#}");
                },
            }
        }

        for event in events.poll_events() {
            match event {
                Event::ChannelReady => last_key.set(String::from("waiting for input")),
                Event::KeyPress(key) => {
                    let state = FfiState {
                        count: count.get_untracked(),
                        ticks: ticks.get_untracked(),
                        last_key: FfiText::from(last_key.get_untracked().as_str()),
                    };
                    // Safety: `logic.handle_key` was resolved from a dylib
                    // this same workspace built against `termoxide_dylib_abi`.
                    let result = unsafe { (logic.handle_key)(state, to_ffi_key(key)) };
                    count.set(result.count);
                    last_key.set(result.last_key.as_str().to_string());
                    quit_requested = result.quit != 0;
                },
            }
        }

        if last_tick.elapsed() >= TICK_INTERVAL {
            // Safety: see above.
            let new_ticks = unsafe { (logic.on_tick)(ticks.get_untracked()) };
            ticks.set(new_ticks);
            last_tick = Instant::now();
        }

        let viewport = renderer.viewport();
        let state = FfiState {
            count: count.get_untracked(),
            ticks: ticks.get_untracked(),
            last_key: FfiText::from(last_key.get_untracked().as_str()),
        };
        // Safety: see above.
        let frame = unsafe { (logic.build_view)(state, viewport.width, viewport.height) };
        let mut root = frame_to_view_node(&frame, viewport, logic.generation);
        renderer.render_frame(&mut root)?;

        if let Some(remaining) = FRAME_INTERVAL.checked_sub(frame_start.elapsed()) {
            std::thread::sleep(remaining);
        }
    }

    let _ = events.teardown();
    Ok(())
}
