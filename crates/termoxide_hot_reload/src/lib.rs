//! Generic host runtime for the dylib-view-swap hot-reload transport,
//! moved out of `demos/dylib_app`'s `host/src/main.rs` and into a reusable
//! framework crate.
//!
//! Same "host never needs to know the app's `State` type" property as the
//! wasm form of this framework (see that crate's docs) — every app's
//! `logic` crate uses `termoxide_hot_reload_dylib_abi::dylib_app!` to
//! encode its state to bytes via `postcard` before host ever touches it.
//! [`run`]/[`run_persistent`] below take no type parameter.
//!
//! # The one real difference from wasm: no sandbox
//!
//! `logic` here is a `cdylib` loaded directly into this process via
//! `libloading` — no `wasmtime::Memory` indirection. `state_ptr`/
//! `output_ptr` are real pointers into the guest's own static buffers;
//! host reads/writes them with `std::ptr::copy_nonoverlapping`/
//! `std::slice::from_raw_parts` directly. That also means the Windows
//! dylib-lock problem is back (wasm never keeps an OS handle open on the
//! built artifact; a native dylib load does) — see [`build_and_load`] for
//! the same generation-numbered-copy trick `dylib_view_swap` uses.

use std::{
    env,
    fs,
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
    style::{Color as RatatuiColor, Style},
};
use termoxide_event::{
    EventStream,
    event::{Event as TermoxideEvent, KeyCode as TermoxideKeyCode, KeyModifiers},
};
use termoxide_hot_reload_dylib_abi::{
    BuildViewFn,
    Color,
    HandleEventFn,
    KeyCode,
    OnTickFn,
    OutputPtrFn,
    SYMBOL_BUILD_VIEW,
    SYMBOL_HANDLE_EVENT,
    SYMBOL_ON_TICK,
    SYMBOL_OUTPUT_PTR,
    SYMBOL_STATE_PTR,
    StatePtrFn,
    decode_lines,
    encode_key_code,
};
use termoxide_rendering::{renderer::Renderer, view_node::ViewNode};

const FRAME_INTERVAL: Duration = Duration::from_millis(16);
const TICK_INTERVAL: Duration = Duration::from_millis(100);
const DEBOUNCE: Duration = Duration::from_millis(300);

// ─────────────────────────────────────────────────────────────────────────
//  Loading the logic dylib
// ─────────────────────────────────────────────────────────────────────────

/// A loaded `logic` generation: the library (kept alive so the pointers
/// and function pointers below stay valid) plus the resolved entry
/// points.
struct Logic {
    _lib: libloading::Library,
    state_ptr: *mut u8,
    output_ptr: *mut u8,
    on_tick: OnTickFn,
    handle_event: HandleEventFn,
    build_view: BuildViewFn,
    generation: u32,
}

impl Logic {
    /// # Safety
    ///
    /// `path` must be a `logic`-dylib build produced by a `dylib_app!`
    /// invocation in this same workspace (so its exported symbols match
    /// `termoxide_hot_reload_dylib_abi`'s function types) — loading and
    /// calling into arbitrary native code is exactly what makes this
    /// whole approach unsafe in the general case, same as the
    /// non-framework `dylib_view_swap` draft.
    unsafe fn load(path: &Path, generation: u32) -> Result<Self> {
        unsafe {
            let lib = libloading::Library::new(path).with_context(|| format!("failed to load {}", path.display()))?;
            let state_ptr_fn = *lib.get::<StatePtrFn>(SYMBOL_STATE_PTR).context("missing state_ptr export")?;
            let output_ptr_fn = *lib.get::<OutputPtrFn>(SYMBOL_OUTPUT_PTR).context("missing output_ptr export")?;
            let on_tick = *lib.get::<OnTickFn>(SYMBOL_ON_TICK).context("missing on_tick export")?;
            let handle_event = *lib
                .get::<HandleEventFn>(SYMBOL_HANDLE_EVENT)
                .context("missing handle_event export")?;
            let build_view = *lib.get::<BuildViewFn>(SYMBOL_BUILD_VIEW).context("missing build_view export")?;

            // Both buffers are `static`s, so their addresses are fixed for
            // this instance's whole lifetime — ask once, not on every call.
            let state_ptr = state_ptr_fn();
            let output_ptr = output_ptr_fn();

            Ok(Self { _lib: lib, state_ptr, output_ptr, on_tick, handle_event, build_view, generation })
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

    // Safety: `dest` is a `logic` dylib this same workspace just built via
    // `dylib_app!`, so its exports match `termoxide_hot_reload_dylib_abi`.
    Ok(Some(unsafe { Logic::load(&dest, next_generation)? }))
}

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
        eprintln!("[termoxide_hot_reload] logic build failed; keeping the currently loaded generation");
        return Ok(None);
    }

    let dylib_suffix = format!(".{}", env::consts::DLL_EXTENSION);
    let path = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|message| message["reason"] == "compiler-artifact")
        .filter(is_cdylib_artifact)
        .find_map(|message| {
            message["filenames"]
                .as_array()?
                .iter()
                .find_map(|f| f.as_str().filter(|s| s.ends_with(&dylib_suffix)).map(PathBuf::from))
        })
        .context("cargo build succeeded but reported no dylib artifact for logic")?;

    Ok(Some(path))
}

/// Match on the `cdylib` target kind, not just a `.dll` filename: a
/// proc-macro dependency (e.g. `serde_derive`, needed for `#[derive(
/// Serialize, Deserialize)]` on the app's state) is *also* compiled to a
/// native `.dll` as part of the same build, and its compiler-artifact
/// message can appear before `logic`'s own — filtering on filename suffix
/// alone silently picks up the wrong artifact.
fn is_cdylib_artifact(message: &serde_json::Value) -> bool {
    message["target"]["kind"]
        .as_array()
        .is_some_and(|kinds| kinds.iter().any(|k| k == "cdylib"))
}

// ─────────────────────────────────────────────────────────────────────────
//  FFI translation
// ─────────────────────────────────────────────────────────────────────────

fn to_abi_key_code(code: TermoxideKeyCode) -> KeyCode {
    match code {
        TermoxideKeyCode::Backspace => KeyCode::Backspace,
        TermoxideKeyCode::Enter => KeyCode::Enter,
        TermoxideKeyCode::Left => KeyCode::Left,
        TermoxideKeyCode::Right => KeyCode::Right,
        TermoxideKeyCode::Up => KeyCode::Up,
        TermoxideKeyCode::Down => KeyCode::Down,
        TermoxideKeyCode::Home => KeyCode::Home,
        TermoxideKeyCode::End => KeyCode::End,
        TermoxideKeyCode::PageUp => KeyCode::PageUp,
        TermoxideKeyCode::PageDown => KeyCode::PageDown,
        TermoxideKeyCode::Tab => KeyCode::Tab,
        TermoxideKeyCode::BackTab => KeyCode::BackTab,
        TermoxideKeyCode::Delete => KeyCode::Delete,
        TermoxideKeyCode::Insert => KeyCode::Insert,
        TermoxideKeyCode::F(n) => KeyCode::F(n),
        TermoxideKeyCode::Char(c) => KeyCode::Char(c),
        TermoxideKeyCode::Null => KeyCode::Null,
        TermoxideKeyCode::Esc => KeyCode::Esc,
    }
}

fn to_modifiers_byte(modifiers: KeyModifiers) -> i32 {
    let mut bits = 0u8;
    for (flag, bit) in [
        (KeyModifiers::SHIFT, termoxide_hot_reload_dylib_abi::modifier::SHIFT),
        (KeyModifiers::CONTROL, termoxide_hot_reload_dylib_abi::modifier::CONTROL),
        (KeyModifiers::ALT, termoxide_hot_reload_dylib_abi::modifier::ALT),
        (KeyModifiers::SUPER, termoxide_hot_reload_dylib_abi::modifier::SUPER),
    ] {
        if modifiers.contains(flag) {
            bits |= bit;
        }
    }
    i32::from(bits)
}

fn ratatui_color(color: Color) -> RatatuiColor {
    match color {
        Color::Cyan => RatatuiColor::Cyan,
        Color::Yellow => RatatuiColor::Yellow,
        Color::Green => RatatuiColor::Green,
        Color::Magenta => RatatuiColor::Magenta,
        Color::Default => RatatuiColor::Reset,
    }
}

/// Turn `build_view`'s decoded output into a real [`ViewNode`] tree, and
/// append the framework-owned footer line.
fn frame_to_view_node(lines: &[(Color, String)], viewport: Rect, generation: u32) -> ViewNode {
    let mut children = Vec::new();

    for (i, (color, text)) in lines.iter().enumerate() {
        let row = i as u16;
        if row >= viewport.height {
            break;
        }
        let area = Rect::new(viewport.x, viewport.y.saturating_add(row), viewport.width, 1);
        children.push(ViewNode::text(area, text.clone(), Style::default().fg(ratatui_color(*color))));
    }

    let footer_row = lines.len().min(usize::from(u16::MAX)) as u16;
    if footer_row < viewport.height {
        let area = Rect::new(viewport.x, viewport.y.saturating_add(footer_row), viewport.width, 1);
        let text = format!(
            "pid: {} | logic generation: {generation} (host-owned, never resets)",
            std::process::id()
        );
        children.push(ViewNode::text(area, text, Style::default().fg(RatatuiColor::Magenta)));
    }

    ViewNode::container(viewport, children)
}

// ─────────────────────────────────────────────────────────────────────────
//  main
// ─────────────────────────────────────────────────────────────────────────

/// Everything the framework needs from a dylib-based app that it can't
/// infer generically: only where the swappable crate lives. No app state
/// type or function pointers to hand over — same reasoning as the wasm
/// form of this crate (see the module docs).
pub struct DylibConfig {
    /// Directory of the hot-swappable `logic` crate (its `Cargo.toml`'s
    /// parent).
    pub logic_dir: PathBuf,
}

pub fn run(config: DylibConfig) -> Result<()> { drive(config, Vec::new(), |_| {}) }

/// Same as [`run`], but opts into surviving a restart of the host process
/// itself: the app's opaque state bytes are read back from `snapshot_path`
/// on startup if present, and written back to that same path after every
/// state change. As with the wasm form of this crate, no encoding step is
/// needed beyond what the live reload boundary already does — see that
/// crate's docs for why.
pub fn run_persistent(config: DylibConfig, snapshot_path: PathBuf) -> Result<()> {
    let initial = fs::read(&snapshot_path).unwrap_or_default();
    if !initial.is_empty() {
        eprintln!("[termoxide_hot_reload] restored state from {}", snapshot_path.display());
    }
    eprintln!("[termoxide_hot_reload] state persistence on ({})", snapshot_path.display());

    drive(config, initial, |bytes| {
        let _ = fs::write(&snapshot_path, bytes);
    })
}

/// Shared loop body for [`run`] and [`run_persistent`].
fn drive(config: DylibConfig, initial_state: Vec<u8>, mut after_state_change: impl FnMut(&[u8])) -> Result<()> {
    let manifest_path = config.logic_dir.join("Cargo.toml");
    if !manifest_path.exists() {
        bail!("expected the logic crate's manifest at {}", manifest_path.display());
    }
    let logic_src_dir = config.logic_dir.join("src");

    let run_dir = env::temp_dir().join(format!("termoxide-dylib-app-framework-{}", std::process::id()));
    fs::create_dir_all(&run_dir).with_context(|| format!("failed to create {}", run_dir.display()))?;

    eprintln!("[termoxide_hot_reload] building logic...");
    let mut generation = 0u32;
    generation += 1;
    let mut logic = build_and_load(&manifest_path, &run_dir, generation)?
        .context("initial logic build failed; fix the error above and re-run")?;
    eprintln!("[termoxide_hot_reload] logic generation {generation} loaded, starting");

    let (tx, rx) = mpsc::channel();
    let mut watcher: RecommendedWatcher = notify::recommended_watcher(move |event| {
        let _ = tx.send(event);
    })
    .context("failed to start file watcher")?;
    watcher
        .watch(&logic_src_dir, RecursiveMode::Recursive)
        .with_context(|| format!("failed to watch {}", logic_src_dir.display()))?;
    watcher
        .watch(&manifest_path, RecursiveMode::NonRecursive)
        .with_context(|| format!("failed to watch {}", manifest_path.display()))?;

    let events = EventStream::new();
    let terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let mut renderer = Renderer::new(terminal)?;

    // The one thing host holds across reloads and restarts of `logic`:
    // the app's last known serialized state, entirely opaque to host.
    let mut state_bytes: Vec<u8> = initial_state;

    let mut last_tick = Instant::now();
    let mut quit_requested = false;

    while !quit_requested {
        let frame_start = Instant::now();

        if rx.try_recv().is_ok() {
            while rx.recv_timeout(DEBOUNCE).is_ok() {}
            eprintln!("[termoxide_hot_reload] change detected, rebuilding logic...");
            generation += 1;
            match build_and_load(&manifest_path, &run_dir, generation) {
                Ok(Some(new_logic)) => {
                    logic = new_logic;
                    eprintln!("[termoxide_hot_reload] logic reloaded (generation {generation}); state untouched");
                },
                Ok(None) => generation -= 1,
                Err(e) => {
                    generation -= 1;
                    eprintln!("[termoxide_hot_reload] failed to load rebuilt logic: {e:#}");
                },
            }
        }

        for event in events.poll_events() {
            // Safety: `state_bytes.len()` never exceeds `STATE_CAPACITY`
            // (every write below is bounded the same way), and
            // `logic.state_ptr` was resolved from a dylib this same
            // workspace built via `dylib_app!`.
            let packed = unsafe {
                std::ptr::copy_nonoverlapping(state_bytes.as_ptr(), logic.state_ptr, state_bytes.len());
                match event {
                    TermoxideEvent::ChannelReady => (logic.handle_event)(state_bytes.len(), 0, 0, 0, 0),
                    TermoxideEvent::KeyPress(key) => {
                        let (tag, payload) = encode_key_code(to_abi_key_code(key.code));
                        let modifiers = to_modifiers_byte(key.modifiers);
                        (logic.handle_event)(state_bytes.len(), 1, tag, payload, modifiers)
                    },
                }
            };
            let new_len = packed >> 1;
            quit_requested = packed & 1 != 0;
            // Safety: see above; `new_len` was just reported by `logic`
            // itself as the byte count it wrote into this same buffer.
            state_bytes = unsafe { std::slice::from_raw_parts(logic.state_ptr, new_len) }.to_vec();
            after_state_change(&state_bytes);
        }

        if last_tick.elapsed() >= TICK_INTERVAL {
            // Safety: see above.
            let new_len = unsafe {
                std::ptr::copy_nonoverlapping(state_bytes.as_ptr(), logic.state_ptr, state_bytes.len());
                (logic.on_tick)(state_bytes.len())
            };
            state_bytes = unsafe { std::slice::from_raw_parts(logic.state_ptr, new_len) }.to_vec();
            after_state_change(&state_bytes);
            last_tick = Instant::now();
        }

        let viewport = renderer.viewport();
        // Safety: see above.
        let written = unsafe {
            std::ptr::copy_nonoverlapping(state_bytes.as_ptr(), logic.state_ptr, state_bytes.len());
            (logic.build_view)(state_bytes.len(), viewport.width, viewport.height)
        };
        let written = written.min(termoxide_hot_reload_dylib_abi::OUTPUT_CAPACITY);
        // Safety: see above.
        let bytes = unsafe { std::slice::from_raw_parts(logic.output_ptr, written) }.to_vec();
        let lines = decode_lines(&bytes);
        let mut root = frame_to_view_node(&lines, viewport, logic.generation);
        renderer.render_frame(&mut root)?;

        if let Some(remaining) = FRAME_INTERVAL.checked_sub(frame_start.elapsed()) {
            std::thread::sleep(remaining);
        }
    }

    let _ = events.teardown();
    Ok(())
}
