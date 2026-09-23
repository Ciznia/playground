//! The hot-swappable half of `hot_lib_reloader_app`.
//!
//! Unlike the hand-rolled `dylib_view_swap` draft, this crate isn't
//! restricted to `#[repr(C)]` scalars: `hot-lib-reloader` builds it as a
//! plain Rust `dylib` (not `cdylib`) and calls into it with the ordinary
//! Rust ABI, trusting that `host` and `lib` are always built by the same
//! rustc invocation (true here — same workspace, same toolchain). That
//! lets these functions take and return real `termoxide` types —
//! `ratatui::layout::Rect`, `termoxide_event::event::Event`,
//! `termoxide_rendering::view_node::ViewNode` — directly, no separate FFI
//! wire format to invent or maintain.
//!
//! What doesn't change from the other drafts: this crate still must not
//! touch `termoxide_reactive::Signal` or `Owner`. Rust `dylib`s statically
//! embed their own copy of every dependency by default, so a `Signal`
//! created or read here would be talking to a *different* reactive runtime
//! instance than the one `host` owns — invisible to it, and vice versa.
//! State stays in `host`; these functions are pure, plain values in and
//! out, same discipline as the other host-resident drafts.

use ratatui::{
    layout::Rect,
    style::{Color, Style},
};
use termoxide_event::event::{Event, KeyCode, KeyModifiers};
use termoxide_rendering::{
    builder::{Container, NodeBuilder, el, text},
    view_node::ViewNode,
};

#[unsafe(no_mangle)]
pub fn on_tick(ticks: u64) -> u64 { ticks + 1 }

/// Returns `(new_count, new_last_key_label, quit)`.
#[unsafe(no_mangle)]
pub fn handle_event(count: u32, event: Event) -> (u32, String, bool) {
    match event {
        Event::ChannelReady => (count, "waiting for input".to_string(), false),
        Event::KeyPress(key) => {
            let quit = key.code == KeyCode::Char('q') || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL));
            (count + 1, format!("key {}+{:?}", key.modifiers, key.code), quit)
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
pub fn build_view(viewport: Rect, count: u32, ticks: u64, last_key: String) -> ViewNode {
    let children: Vec<ViewNode> = [
        line(viewport, 0, "hot-lib-reloader termoxide app".to_string(), Style::default().fg(Color::Cyan)),
        line(
            viewport,
            1,
            format!("ticks: {ticks} | key presses: {count} | last key: {last_key}"),
            Style::default().fg(Color::Yellow),
        ),
        line(viewport, 2, "Controls: any key counts, q or Ctrl-C quits".to_string(), Style::default().fg(Color::Green)),
    ]
    .into_iter()
    .flatten()
    .collect();

    el(Container).area(viewport).children(children).build()
}
