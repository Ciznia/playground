//! Long-lived host for the WASM hot-reload draft.
//!
//! Same shape as the `hot_reload_dylib_view_swap` draft — one process, up
//! for the whole session, owning the reactive `Owner` and every `Signal`;
//! only `logic` (here compiled to `wasm32-unknown-unknown` and run through
//! `wasmtime`) gets swapped out on a reload. See that draft's `host` for
//! the fuller comparison; this doc only covers what's different.
//!
//! # No Windows dylib-lock dance
//!
//! The dylib draft has to copy every rebuild to a new, incrementally
//! numbered `.dll` file, because Windows won't let the linker overwrite one
//! this process still has loaded. That problem doesn't exist here: `host`
//! never keeps an OS handle open on the `.wasm` file at all — it reads the
//! bytes once with `std::fs::read` into a `wasmtime::Module`, and from that
//! point on the file is just bytes on disk again, free for `cargo build` to
//! overwrite on the next change. So every reload rebuilds and reloads from
//! the *same* canonical path, no generation bookkeeping required.
//!
//! # A stricter, narrower boundary
//!
//! Core wasm functions only take/return `i32`/`i64` — no passing a
//! `#[repr(C)]` struct by value the way the dylib draft's `on_tick`/
//! `handle_key`/`build_view` do. Only `build_view`'s output (the rendered
//! status text, the one genuinely variable-length value) goes through
//! `logic`'s own linear memory; everything else is a plain scalar. See
//! `termoxide_wasm_abi` for the exact wire format. One real feature was cut
//! rather than fought: this draft drops the "last key" label the other two
//! show, precisely because moving a variable-length string *into* the
//! guest would have meant a second memory-write path in the other
//! direction — doable, but more machinery than this draft needed to prove
//! the core idea.
//!
//! No application state survives a reload: a `logic` reload never touches
//! `count`/`ticks` (they live entirely in `host`), but a full restart of
//! `host` itself — needed when `host/src/main.rs` changes, since `host`
//! isn't hot-swappable — does lose them. See the `hot_reload_wasm_persist`
//! draft worktree for the opt-in state snapshot built on top of this one.

use std::{io::stdout, path::{Path, PathBuf}, process::Command, sync::mpsc, time::{Duration, Instant}};

use anyhow::{Context, Result, bail};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::Rect,
    style::{Color, Style},
};
use termoxide_event::{
    EventStream,
    event::{Event, KeyCode, KeyModifiers},
};
use termoxide_reactive::{Owner, Signal};
use termoxide_rendering::{renderer::Renderer, view_node::ViewNode};
use termoxide_wasm_abi::{EXPORT_BUILD_VIEW, EXPORT_HANDLE_KEY, EXPORT_MEMORY, EXPORT_ON_TICK, EXPORT_OUTPUT_PTR, key_tag, modifier, unpack_handle_key_result};
use wasmtime::{Engine, Instance, Memory, Module, Store, TypedFunc};

const FRAME_INTERVAL: Duration = Duration::from_millis(16);
const TICK_INTERVAL: Duration = Duration::from_millis(100);
const DEBOUNCE: Duration = Duration::from_millis(300);

// ─────────────────────────────────────────────────────────────────────────
//  Loading the logic wasm module
// ─────────────────────────────────────────────────────────────────────────

struct Logic {
    store: Store<()>,
    on_tick: TypedFunc<i64, i64>,
    handle_key: TypedFunc<(i32, i32, i32, i32), i32>,
    build_view: TypedFunc<(i32, i64), i32>,
    output_ptr: usize,
    memory: Memory,
    generation: u32,
}

impl Logic {
    fn load(engine: &Engine, wasm_path: &Path, generation: u32) -> Result<Self> {
        let module = Module::from_file(engine, wasm_path)
            .map_err(|e| anyhow::anyhow!("failed to load {}: {e}", wasm_path.display()))?;
        let mut store = Store::new(engine, ());
        // No host functions needed: `logic` never calls back into `host`.
        let instance = Instance::new(&mut store, &module, &[])
            .map_err(|e| anyhow::anyhow!("failed to instantiate {}: {e}", wasm_path.display()))?;

        let on_tick = instance
            .get_typed_func::<i64, i64>(&mut store, EXPORT_ON_TICK)
            .map_err(|e| anyhow::anyhow!("missing {EXPORT_ON_TICK} export: {e}"))?;
        let handle_key = instance
            .get_typed_func::<(i32, i32, i32, i32), i32>(&mut store, EXPORT_HANDLE_KEY)
            .map_err(|e| anyhow::anyhow!("missing {EXPORT_HANDLE_KEY} export: {e}"))?;
        let build_view = instance
            .get_typed_func::<(i32, i64), i32>(&mut store, EXPORT_BUILD_VIEW)
            .map_err(|e| anyhow::anyhow!("missing {EXPORT_BUILD_VIEW} export: {e}"))?;
        let output_ptr_fn = instance
            .get_typed_func::<(), i32>(&mut store, EXPORT_OUTPUT_PTR)
            .map_err(|e| anyhow::anyhow!("missing {EXPORT_OUTPUT_PTR} export: {e}"))?;
        let memory = instance.get_memory(&mut store, EXPORT_MEMORY).with_context(|| format!("missing {EXPORT_MEMORY} export"))?;

        // The output buffer is a `static`, so its address is fixed for this
        // instance's whole lifetime — ask once, not on every `build_view` call.
        let output_ptr = output_ptr_fn.call(&mut store, ())? as usize;

        Ok(Self { store, on_tick, handle_key, build_view, output_ptr, memory, generation })
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
        .args(["build", "--target", "wasm32-unknown-unknown", "--message-format=json-render-diagnostics", "--manifest-path"])
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
        eprintln!("[wasm_app host] logic build failed; keeping the currently loaded generation");
        return Ok(None);
    }

    let path = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|message| message["reason"] == "compiler-artifact")
        .find_map(|message| {
            message["filenames"].as_array()?.iter().find_map(|f| f.as_str().filter(|s| s.ends_with(".wasm")).map(PathBuf::from))
        })
        .context("cargo build succeeded but reported no .wasm artifact for logic")?;

    Ok(Some(path))
}

// ─────────────────────────────────────────────────────────────────────────
//  FFI translation
// ─────────────────────────────────────────────────────────────────────────

fn to_ffi_key(key: termoxide_event::event::KeyEvent) -> (i32, i32, i32) {
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
        KeyCode::F(n) => (key_tag::F, i32::from(n)),
        KeyCode::Char(c) => (key_tag::CHAR, c as i32),
        KeyCode::Null => (key_tag::NULL, 0),
        KeyCode::Esc => (key_tag::ESC, 0),
    };

    let mut modifiers = 0i32;
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

    (tag, payload, modifiers)
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

/// Read `logic`'s wire-format output back out of its linear memory and turn
/// it into a real [`ViewNode`] tree, appending the host-owned footer line.
fn frame_to_view_node(bytes: &[u8], viewport: Rect, generation: u32) -> ViewNode {
    let mut children = Vec::new();
    let mut cursor = 0usize;
    let mut row = 0u16;

    while cursor + 2 <= bytes.len() && row < viewport.height {
        let color = bytes[cursor];
        let len = bytes[cursor + 1] as usize;
        let end = (cursor + 2 + len).min(bytes.len());
        let text = std::str::from_utf8(&bytes[cursor + 2..end]).unwrap_or("").to_string();

        let area = Rect::new(viewport.x, viewport.y.saturating_add(row), viewport.width, 1);
        children.push(ViewNode::text(area, text, Style::default().fg(color_from_tag(color))));

        cursor = end;
        row += 1;
    }

    if row < viewport.height {
        let area = Rect::new(viewport.x, viewport.y.saturating_add(row), viewport.width, 1);
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

    let owner = Owner::new();
    owner.set();
    let count = Signal::new(0u32);
    let ticks = Signal::new(0u64);

    let engine = Engine::default();

    eprintln!("[wasm_app host] building logic...");
    let mut generation = 0u32;
    generation += 1;
    let mut logic =
        build_and_load(&engine, &manifest_path, generation)?.context("initial logic build failed; fix the error above and re-run")?;
    eprintln!("[wasm_app host] logic generation {generation} loaded, starting");

    let (tx, rx) = mpsc::channel();
    let mut watcher: RecommendedWatcher =
        notify::recommended_watcher(move |event| { let _ = tx.send(event); }).context("failed to start file watcher")?;
    watcher.watch(&logic_src_dir, RecursiveMode::Recursive).with_context(|| format!("failed to watch {}", logic_src_dir.display()))?;
    watcher.watch(&manifest_path, RecursiveMode::NonRecursive).with_context(|| format!("failed to watch {}", manifest_path.display()))?;

    let events = EventStream::new();
    let terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let mut renderer = Renderer::new(terminal)?;

    let mut last_tick = Instant::now();
    let mut quit_requested = false;

    while !quit_requested {
        let frame_start = Instant::now();

        if rx.try_recv().is_ok() {
            while rx.recv_timeout(DEBOUNCE).is_ok() {}
            eprintln!("[wasm_app host] change detected, rebuilding logic...");
            generation += 1;
            match build_and_load(&engine, &manifest_path, generation) {
                Ok(Some(new_logic)) => {
                    logic = new_logic;
                    eprintln!("[wasm_app host] logic reloaded (generation {generation}); count/ticks untouched");
                },
                Ok(None) => generation -= 1,
                Err(e) => {
                    generation -= 1;
                    eprintln!("[wasm_app host] failed to load rebuilt logic: {e:#}");
                },
            }
        }

        for event in events.poll_events() {
            if let Event::KeyPress(key) = event {
                let (tag, payload, modifiers) = to_ffi_key(key);
                let packed = logic.handle_key.call(&mut logic.store, (count.get_untracked() as i32, tag, payload, modifiers))?;
                let (new_count, quit) = unpack_handle_key_result(packed);
                count.set(new_count as u32);
                quit_requested = quit;
            }
        }

        if last_tick.elapsed() >= TICK_INTERVAL {
            let new_ticks = logic.on_tick.call(&mut logic.store, ticks.get_untracked() as i64)?;
            ticks.set(new_ticks as u64);
            last_tick = Instant::now();
        }

        let viewport = renderer.viewport();
        let written = logic.build_view.call(&mut logic.store, (count.get_untracked() as i32, ticks.get_untracked() as i64))? as usize;
        let written = written.min(termoxide_wasm_abi::OUTPUT_CAPACITY);
        let bytes = logic.memory.data(&logic.store)[logic.output_ptr..logic.output_ptr + written].to_vec();
        let mut root = frame_to_view_node(&bytes, viewport, logic.generation);
        renderer.render_frame(&mut root)?;

        if let Some(remaining) = FRAME_INTERVAL.checked_sub(frame_start.elapsed()) {
            std::thread::sleep(remaining);
        }
    }

    let _ = events.teardown();
    Ok(())
}
