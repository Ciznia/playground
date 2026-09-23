//! The entire app-author surface of the framework-owned hot-lib-reloader
//! draft: a plain state struct and three functions operating on it. No
//! host, no watcher, no `hot_module` boilerplate, no FFI/wire format — see
//! `termoxide_hot_reload` for where all of that now lives.
//!
//! Same discipline as every other draft: this crate must not touch
//! `termoxide_reactive::Signal`/`Owner`. A Rust `dylib` statically embeds
//! its own copy of every dependency, so a `Signal` created here would be
//! talking to a different reactive runtime instance than the framework's
//! `run` loop — state stays a plain value in `AppState`, owned by the host.

use ratatui::{
    layout::Rect,
    style::{Color, Style},
};
use termoxide_event::event::{Event, KeyCode, KeyModifiers};
use termoxide_rendering::{
    builder::{Container, NodeBuilder, el, text},
    view_node::ViewNode,
};

#[derive(Default)]
pub struct AppState {
    pub count: u32,
    pub ticks: u64,
    pub last_key: String,
}

#[unsafe(no_mangle)]
pub fn on_tick(state: &mut AppState) { state.ticks += 1; }

#[unsafe(no_mangle)]
pub fn handle_event(state: &mut AppState, event: Event) -> bool {
    match event {
        Event::ChannelReady => {
            state.last_key = "waiting for input".to_string();
            false
        },
        Event::KeyPress(key) => {
            let quit = key.code == KeyCode::Char('q') || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL));
            state.count += 1;
            state.last_key = format!("key {}+{:?}", key.modifiers, key.code);
            quit
        },
    }
}

fn line(viewport: Rect, row: u16, content: String, style: Style) -> Option<ViewNode> {
    if row >= viewport.height {
        return None;
    }
    Some(text(content).area(Rect::new(viewport.x, viewport.y.saturating_add(row), viewport.width, 1)).style(style).build())
}

#[unsafe(no_mangle)]
pub fn build_view(state: &AppState, viewport: Rect) -> ViewNode {
    let children: Vec<ViewNode> = [
        line(viewport, 0, "hot-lib-reloader termoxide app (framework host)".to_string(), Style::default().fg(Color::Cyan)),
        line(
            viewport,
            1,
            format!("ticks: {} | key presses: {} | last key: {}", state.ticks, state.count, state.last_key),
            Style::default().fg(Color::Yellow),
        ),
        line(viewport, 2, "Controls: any key counts, q or Ctrl-C quits".to_string(), Style::default().fg(Color::Green)),
    ]
    .into_iter()
    .flatten()
    .collect();

    el(Container).area(viewport).children(children).build()
}
