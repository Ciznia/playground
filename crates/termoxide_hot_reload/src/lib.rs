//! Generic host runtime for the wasm-based hot-reload transport, moved out
//! of `demos/wasm_app`'s `host/src/main.rs` and into a reusable framework
//! crate.
//!
//! # The key difference from the hot-lib-reloader framework crate: no
//! generics on the host at all
//!
//! In `termoxide_hot_reload`'s hot-lib-reloader form, `run::<S>` still had
//! to be generic over the app's state type `S`, because host held a real
//! `S` value and called real Rust function pointers operating on it
//! (same-toolchain trust makes that possible there). Core wasm can't pass
//! a real Rust type across the boundary at all — every app's `logic`
//! crate uses `termoxide_hot_reload_wasm_abi::wasm_app!` to encode its
//! state to bytes via `postcard` before it ever leaves the guest. Host
//! only ever holds that encoded blob (`Vec<u8>`) and passes it back in
//! unexamined; it never deserializes it, so it genuinely never needs to
//! know the app's state *type* at all. [`run`] below takes no type
//! parameter and needs no per-app trait impl — same function, byte for
//! byte, for every `wasm_app!`-based app.

use std::{
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
use termoxide_hot_reload_wasm_abi::{
    Color,
    EXPORT_BUILD_VIEW,
    EXPORT_HANDLE_EVENT,
    EXPORT_MEMORY,
    EXPORT_ON_TICK,
    EXPORT_OUTPUT_PTR,
    EXPORT_STATE_PTR,
    KeyCode,
    decode_lines,
    encode_key_code,
};
use termoxide_rendering::{renderer::Renderer, view_node::ViewNode};
use wasmtime::{Engine, Instance, Memory, Module, Store, TypedFunc};

const FRAME_INTERVAL: Duration = Duration::from_millis(16);
const TICK_INTERVAL: Duration = Duration::from_millis(100);
const DEBOUNCE: Duration = Duration::from_millis(300);

// ─────────────────────────────────────────────────────────────────────────
//  Loading the logic wasm module
// ─────────────────────────────────────────────────────────────────────────

struct Logic {
    store: Store<()>,
    state_ptr: usize,
    output_ptr: usize,
    memory: Memory,
    on_tick: TypedFunc<i32, i32>,
    handle_event: TypedFunc<(i32, i32, i32, i32, i32), i32>,
    build_view: TypedFunc<(i32, i32, i32), i32>,
    generation: u32,
}

impl Logic {
    fn load(engine: &Engine, wasm_path: &Path, generation: u32) -> Result<Self> {
        let module = Module::from_file(engine, wasm_path)
            .map_err(|e| anyhow::anyhow!("failed to load {}: {e}", wasm_path.display()))?;
        let mut store = Store::new(engine, ());
        let instance = Instance::new(&mut store, &module, &[])
            .map_err(|e| anyhow::anyhow!("failed to instantiate {}: {e}", wasm_path.display()))?;

        let state_ptr_fn = instance
            .get_typed_func::<(), i32>(&mut store, EXPORT_STATE_PTR)
            .map_err(|e| anyhow::anyhow!("missing {EXPORT_STATE_PTR} export: {e}"))?;
        let output_ptr_fn = instance
            .get_typed_func::<(), i32>(&mut store, EXPORT_OUTPUT_PTR)
            .map_err(|e| anyhow::anyhow!("missing {EXPORT_OUTPUT_PTR} export: {e}"))?;
        let on_tick = instance
            .get_typed_func::<i32, i32>(&mut store, EXPORT_ON_TICK)
            .map_err(|e| anyhow::anyhow!("missing {EXPORT_ON_TICK} export: {e}"))?;
        let handle_event = instance
            .get_typed_func::<(i32, i32, i32, i32, i32), i32>(&mut store, EXPORT_HANDLE_EVENT)
            .map_err(|e| anyhow::anyhow!("missing {EXPORT_HANDLE_EVENT} export: {e}"))?;
        let build_view = instance
            .get_typed_func::<(i32, i32, i32), i32>(&mut store, EXPORT_BUILD_VIEW)
            .map_err(|e| anyhow::anyhow!("missing {EXPORT_BUILD_VIEW} export: {e}"))?;
        let memory = instance
            .get_memory(&mut store, EXPORT_MEMORY)
            .with_context(|| format!("missing {EXPORT_MEMORY} export"))?;

        // Both buffers are `static`s, so their addresses are fixed for
        // this instance's whole lifetime — ask once, not on every call.
        let state_ptr = state_ptr_fn.call(&mut store, ())? as usize;
        let output_ptr = output_ptr_fn.call(&mut store, ())? as usize;

        Ok(Self {
            store,
            state_ptr,
            output_ptr,
            memory,
            on_tick,
            handle_event,
            build_view,
            generation,
        })
    }
}

/// Build `logic` and load the result. Returns `Ok(None)` when the build
/// itself failed (already reported to stderr).
fn build_and_load(engine: &Engine, manifest_path: &Path, next_generation: u32) -> Result<Option<Logic>> {
    let Some(wasm_path) = build_logic(manifest_path)? else {
        return Ok(None);
    };
    Ok(Some(Logic::load(engine, &wasm_path, next_generation)?))
}

fn build_logic(manifest_path: &Path) -> Result<Option<PathBuf>> {
    let output = Command::new("cargo")
        .args([
            "build",
            "--target",
            "wasm32-unknown-unknown",
            "--message-format=json-render-diagnostics",
            "--manifest-path",
        ])
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

    let path = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|message| message["reason"] == "compiler-artifact")
        .find_map(|message| {
            message["filenames"]
                .as_array()?
                .iter()
                .find_map(|f| f.as_str().filter(|s| s.ends_with(".wasm")).map(PathBuf::from))
        })
        .context("cargo build succeeded but reported no .wasm artifact for logic")?;

    Ok(Some(path))
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
        (KeyModifiers::SHIFT, termoxide_hot_reload_wasm_abi::modifier::SHIFT),
        (KeyModifiers::CONTROL, termoxide_hot_reload_wasm_abi::modifier::CONTROL),
        (KeyModifiers::ALT, termoxide_hot_reload_wasm_abi::modifier::ALT),
        (KeyModifiers::SUPER, termoxide_hot_reload_wasm_abi::modifier::SUPER),
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
/// append the framework-owned footer line — proof, alongside the pid,
/// that it (and the process) survive a reload the app's `logic` crate has
/// no part in.
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

/// Everything the framework needs from a wasm-based app that it can't
/// infer generically: only where the swappable crate lives. Unlike the
/// hot-lib-reloader form of `termoxide_hot_reload`, there's no app state
/// type or function pointers to hand over here — [`run`] never touches
/// the app's `State` type at all (see the module docs).
pub struct WasmConfig {
    /// Directory of the hot-swappable `logic` crate (its `Cargo.toml`'s
    /// parent).
    pub logic_dir: PathBuf,
}

/// Runs the whole host loop for a wasm-based app: rebuild watcher,
/// wasmtime module loading, terminal setup, and the event/tick/render
/// loop. Takes no type parameter — see the module docs for why the host
/// never needs to know the app's `State` type to do this.
pub fn run(config: WasmConfig) -> Result<()> {
    let manifest_path = config.logic_dir.join("Cargo.toml");
    if !manifest_path.exists() {
        bail!("expected the logic crate's manifest at {}", manifest_path.display());
    }
    let logic_src_dir = config.logic_dir.join("src");

    let engine = Engine::default();

    eprintln!("[termoxide_hot_reload] building logic...");
    let mut generation = 0u32;
    generation += 1;
    let mut logic = build_and_load(&engine, &manifest_path, generation)?
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
    let mut state_bytes: Vec<u8> = Vec::new();

    let mut last_tick = Instant::now();
    let mut quit_requested = false;

    while !quit_requested {
        let frame_start = Instant::now();

        if rx.try_recv().is_ok() {
            while rx.recv_timeout(DEBOUNCE).is_ok() {}
            eprintln!("[termoxide_hot_reload] change detected, rebuilding logic...");
            generation += 1;
            match build_and_load(&engine, &manifest_path, generation) {
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
            logic.memory.write(&mut logic.store, logic.state_ptr, &state_bytes)?;
            let packed = match event {
                TermoxideEvent::ChannelReady => {
                    logic
                        .handle_event
                        .call(&mut logic.store, (state_bytes.len() as i32, 0, 0, 0, 0))?
                },
                TermoxideEvent::KeyPress(key) => {
                    let (tag, payload) = encode_key_code(to_abi_key_code(key.code));
                    let modifiers = to_modifiers_byte(key.modifiers);
                    logic
                        .handle_event
                        .call(&mut logic.store, (state_bytes.len() as i32, 1, tag, payload, modifiers))?
                },
            };
            let new_len = (packed >> 1) as usize;
            quit_requested = packed & 1 != 0;
            state_bytes = logic.memory.data(&logic.store)[logic.state_ptr..logic.state_ptr + new_len].to_vec();
        }

        if last_tick.elapsed() >= TICK_INTERVAL {
            logic.memory.write(&mut logic.store, logic.state_ptr, &state_bytes)?;
            let new_len = logic.on_tick.call(&mut logic.store, state_bytes.len() as i32)? as usize;
            state_bytes = logic.memory.data(&logic.store)[logic.state_ptr..logic.state_ptr + new_len].to_vec();
            last_tick = Instant::now();
        }

        let viewport = renderer.viewport();
        logic.memory.write(&mut logic.store, logic.state_ptr, &state_bytes)?;
        let written = logic.build_view.call(
            &mut logic.store,
            (state_bytes.len() as i32, i32::from(viewport.width), i32::from(viewport.height)),
        )? as usize;
        let written = written.min(termoxide_hot_reload_wasm_abi::OUTPUT_CAPACITY);
        let bytes = logic.memory.data(&logic.store)[logic.output_ptr..logic.output_ptr + written].to_vec();
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
