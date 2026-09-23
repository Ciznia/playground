//! The hot-swappable half of `dylib_app`.
//!
//! This is the crate `host` rebuilds and reloads on every save — edit
//! anything in here (the status text, the tick increment, which key quits)
//! and `host` picks it up without restarting, without losing `count`/
//! `ticks`/`last_key`, and without changing process id.
//!
//! No `termoxide_reactive`, no `Signal`, no heap type crossing back to
//! `host` — see the crate docs on `termoxide_dylib_abi` for why. Every
//! exported function is pure: state in, state out.

use termoxide_dylib_abi::{FfiColor, FfiFrame, FfiHandleResult, FfiKey, FfiLine, FfiState, FfiText, key_tag, modifier};

#[unsafe(no_mangle)]
pub extern "C" fn termoxide_on_tick(ticks: u64) -> u64 { ticks + 1 }

#[unsafe(no_mangle)]
pub extern "C" fn termoxide_handle_key(state: FfiState, key: FfiKey) -> FfiHandleResult {
    let is_char = |c: char| key.tag == key_tag::CHAR && key.payload == c as u32;
    let quit = is_char('q') || (is_char('c') && key.modifiers & modifier::CONTROL != 0);

    FfiHandleResult { count: state.count + 1, last_key: FfiText::from(describe_key(key).as_str()), quit: quit as u8 }
}

fn describe_key(key: FfiKey) -> String {
    let name = match key.tag {
        key_tag::CHAR => return format!("key {}", char::from_u32(key.payload).unwrap_or('?')),
        key_tag::ENTER => "Enter",
        key_tag::ESC => "Esc",
        key_tag::BACKSPACE => "Backspace",
        key_tag::TAB => "Tab",
        key_tag::BACK_TAB => "Shift+Tab",
        key_tag::UP => "Up",
        key_tag::DOWN => "Down",
        key_tag::LEFT => "Left",
        key_tag::RIGHT => "Right",
        key_tag::HOME => "Home",
        key_tag::END => "End",
        key_tag::PAGE_UP => "PageUp",
        key_tag::PAGE_DOWN => "PageDown",
        key_tag::DELETE => "Delete",
        key_tag::INSERT => "Insert",
        key_tag::F => return format!("F{}", key.payload),
        key_tag::NULL => "Null",
        _ => "?",
    };
    format!("key {name}")
}

#[unsafe(no_mangle)]
pub extern "C" fn termoxide_build_view(state: FfiState, _width: u16, _height: u16) -> FfiFrame {
    let mut lines = [FfiLine::EMPTY; termoxide_dylib_abi::MAX_LINES];
    lines[0] = FfiLine { text: FfiText::from("dylib termoxide app"), color: FfiColor::CYAN };
    lines[1] = FfiLine {
        text: FfiText::from(
            format!("ticks: {} | key presses: {} | last key: {}", state.ticks, state.count, state.last_key.as_str())
                .as_str(),
        ),
        color: FfiColor::YELLOW,
    };
    lines[2] = FfiLine { text: FfiText::from("Controls: any key counts, q or Ctrl-C quits"), color: FfiColor::GREEN };

    FfiFrame { lines, line_count: 3 }
}
