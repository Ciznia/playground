//! The entire app-author surface of the framework-owned dylib-view-swap
//! draft: a plain state struct and one `DylibApp` impl. No `#[repr(C)]`
//! structs to hand-maintain, no `output_ptr`, no byte cursor — see
//! `termoxide_hot_reload_dylib_abi::dylib_app!` for where all of that now
//! lives. Doesn't depend on `ratatui`/`termoxide_event`/
//! `termoxide_rendering` at all: `Event`/`KeyCode`/`Line`/`Color` here are
//! the framework's own small equivalents.

use serde::{Deserialize, Serialize};
use termoxide_hot_reload_dylib_abi::{Color, DylibApp, Event, KeyCode, Line, modifier};

#[derive(Default, Serialize, Deserialize)]
pub struct AppState {
    pub count: u32,
    pub ticks: u64,
}

pub struct App;

impl DylibApp for App {
    type State = AppState;

    fn on_tick(state: &mut AppState) { state.ticks += 1; }

    fn handle_event(state: &mut AppState, event: Event) -> bool {
        match event {
            Event::ChannelReady => false,
            Event::KeyPress { code, modifiers } => {
                let quit = code == KeyCode::Char('q') || (code == KeyCode::Char('c') && modifiers.contains(modifier::CONTROL));
                state.count += 1;
                quit
            },
        }
    }

    fn build_view(state: &AppState, _width: u16, _height: u16) -> Vec<Line> {
        vec![
            Line::new(Color::Cyan, "dylib termoxide app (framework host, persist)"),
            Line::new(Color::Yellow, format!("ticks: {} | key presses: {}", state.ticks, state.count)),
            Line::new(Color::Green, "Controls: any key counts, q or Ctrl-C quits"),
        ]
    }
}

termoxide_hot_reload_dylib_abi::dylib_app!(App);
