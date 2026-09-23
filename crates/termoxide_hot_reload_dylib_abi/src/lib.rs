//! Shared between the dylib guest (an app's `logic` crate, built as a
//! `cdylib`) and the native host's generic runner in
//! `termoxide_hot_reload`. Unlike the wasm form of this framework, guest
//! and host run in the *same* process address space — no sandbox, no
//! separate linear memory to read/write through. `state_ptr`/`output_ptr`
//! return real pointers into the guest's own static buffers, and host
//! dereferences them directly.
//!
//! Compare this to `termoxide_dylib_abi` in the non-framework
//! `dylib_view_swap` draft: that crate defined `#[repr(C)]` structs
//! (`FfiState`, `FfiFrame`, `FfiLine`, ...) that both sides had to keep in
//! sync by hand, field for field, on pain of undefined behavior. Here,
//! [`dylib_app!`] generates the marshaling generically over any
//! `$app::State: Serialize + DeserializeOwned`, the same `postcard`-based
//! approach the wasm-framework drafts use — there's no per-app `#[repr(C)]`
//! type to maintain at all.

// Re-exported so `dylib_app!`'s expansion can reach `postcard` through
// `$crate::postcard` without the app's `logic` crate needing its own
// direct dependency on it.
pub use postcard;
use serde::{Serialize, de::DeserializeOwned};

/// Bytes reserved for the state buffer both directions of every call
/// read and write through.
pub const STATE_CAPACITY: usize = 4096;
/// Bytes reserved for `build_view`'s rendered-line output.
pub const OUTPUT_CAPACITY: usize = 2048;
/// Max bytes one line's text may occupy — small enough a `u8` length
/// prefix always holds it.
pub const MAX_LINE_TEXT_LEN: usize = 120;

pub const SYMBOL_STATE_PTR: &[u8] = b"state_ptr\0";
pub const SYMBOL_OUTPUT_PTR: &[u8] = b"output_ptr\0";
pub const SYMBOL_ON_TICK: &[u8] = b"on_tick\0";
pub const SYMBOL_HANDLE_EVENT: &[u8] = b"handle_event\0";
pub const SYMBOL_BUILD_VIEW: &[u8] = b"build_view\0";

/// Function pointer types `host` resolves via `libloading`, matching the
/// exports [`dylib_app!`] generates exactly.
pub type StatePtrFn = unsafe extern "C" fn() -> *mut u8;
pub type OutputPtrFn = unsafe extern "C" fn() -> *mut u8;
pub type OnTickFn = unsafe extern "C" fn(usize) -> usize;
pub type HandleEventFn = unsafe extern "C" fn(usize, i32, i32, i32, i32) -> usize;
pub type BuildViewFn = unsafe extern "C" fn(usize, u16, u16) -> usize;

/// A line's color, framework-fixed like `Rect`/`Event`/`ViewNode` are for
/// the hot-lib-reloader draft.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Color {
    #[default]
    Default,
    Cyan,
    Yellow,
    Green,
    Magenta,
}

impl Color {
    pub fn to_tag(self) -> u8 {
        match self {
            Color::Default => 0,
            Color::Cyan => 1,
            Color::Yellow => 2,
            Color::Green => 3,
            Color::Magenta => 4,
        }
    }

    pub fn from_tag(tag: u8) -> Self {
        match tag {
            1 => Color::Cyan,
            2 => Color::Yellow,
            3 => Color::Green,
            4 => Color::Magenta,
            _ => Color::Default,
        }
    }
}

/// One line of `build_view`'s output.
pub struct Line {
    pub color: Color,
    pub text: String,
}

impl Line {
    pub fn new(color: Color, text: impl Into<String>) -> Self { Self { color, text: text.into() } }
}

/// Encode `lines` into `buf` as `[color, len, text_bytes...]*`,
/// self-delimiting by construction. Returns the byte count written,
/// truncating (dropping whole lines, never splitting UTF-8) rather than
/// overflowing `buf`.
pub fn encode_lines(lines: &[Line], buf: &mut [u8]) -> usize {
    let mut cursor = 0usize;
    for line in lines {
        let bytes = line.text.as_bytes();
        let len = bytes.len().min(MAX_LINE_TEXT_LEN);
        let len = (0..=len).rev().find(|&i| line.text.is_char_boundary(i)).unwrap_or(0);
        if cursor + 2 + len > buf.len() {
            break;
        }
        buf[cursor] = line.color.to_tag();
        buf[cursor + 1] = len as u8;
        buf[cursor + 2..cursor + 2 + len].copy_from_slice(&bytes[..len]);
        cursor += 2 + len;
    }
    cursor
}

/// Decode the wire format [`encode_lines`] writes — used host-side.
pub fn decode_lines(bytes: &[u8]) -> Vec<(Color, String)> {
    let mut lines = Vec::new();
    let mut cursor = 0usize;
    while cursor + 2 <= bytes.len() {
        let color = Color::from_tag(bytes[cursor]);
        let len = bytes[cursor + 1] as usize;
        let end = (cursor + 2 + len).min(bytes.len());
        let text = std::str::from_utf8(&bytes[cursor + 2..end]).unwrap_or("").to_string();
        lines.push((color, text));
        cursor = end;
    }
    lines
}

/// A key press, decoded from the scalar tag/payload the host encodes it
/// as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyCode {
    Backspace,
    Enter,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Tab,
    BackTab,
    Delete,
    Insert,
    F(u8),
    Char(char),
    Null,
    Esc,
}

pub mod key_tag {
    pub const BACKSPACE: i32 = 0;
    pub const ENTER: i32 = 1;
    pub const LEFT: i32 = 2;
    pub const RIGHT: i32 = 3;
    pub const UP: i32 = 4;
    pub const DOWN: i32 = 5;
    pub const HOME: i32 = 6;
    pub const END: i32 = 7;
    pub const PAGE_UP: i32 = 8;
    pub const PAGE_DOWN: i32 = 9;
    pub const TAB: i32 = 10;
    pub const BACK_TAB: i32 = 11;
    pub const DELETE: i32 = 12;
    pub const INSERT: i32 = 13;
    pub const F: i32 = 14;
    pub const CHAR: i32 = 15;
    pub const NULL: i32 = 16;
    pub const ESC: i32 = 17;
}

pub mod modifier {
    pub const SHIFT: u8 = 1 << 0;
    pub const CONTROL: u8 = 1 << 1;
    pub const ALT: u8 = 1 << 2;
    pub const SUPER: u8 = 1 << 3;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Modifiers(pub u8);

impl Modifiers {
    pub fn contains(self, bit: u8) -> bool { self.0 & bit != 0 }
}

pub fn decode_key_code(tag: i32, payload: i32) -> KeyCode {
    match tag {
        key_tag::BACKSPACE => KeyCode::Backspace,
        key_tag::ENTER => KeyCode::Enter,
        key_tag::LEFT => KeyCode::Left,
        key_tag::RIGHT => KeyCode::Right,
        key_tag::UP => KeyCode::Up,
        key_tag::DOWN => KeyCode::Down,
        key_tag::HOME => KeyCode::Home,
        key_tag::END => KeyCode::End,
        key_tag::PAGE_UP => KeyCode::PageUp,
        key_tag::PAGE_DOWN => KeyCode::PageDown,
        key_tag::TAB => KeyCode::Tab,
        key_tag::BACK_TAB => KeyCode::BackTab,
        key_tag::DELETE => KeyCode::Delete,
        key_tag::INSERT => KeyCode::Insert,
        key_tag::F => KeyCode::F(payload as u8),
        key_tag::CHAR => KeyCode::Char(char::from_u32(payload as u32).unwrap_or('\u{FFFD}')),
        key_tag::ESC => KeyCode::Esc,
        _ => KeyCode::Null,
    }
}

pub fn encode_key_code(code: KeyCode) -> (i32, i32) {
    match code {
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
    }
}

/// An input event, as delivered to [`DylibApp::handle_event`].
#[derive(Clone, Copy, Debug)]
pub enum Event {
    ChannelReady,
    KeyPress { code: KeyCode, modifiers: Modifiers },
}

/// The entire app-author surface for a `dylib_app!`-based draft: a state
/// type plus three pure functions operating on it.
pub trait DylibApp {
    type State: Default + Serialize + DeserializeOwned;

    fn on_tick(state: &mut Self::State);
    fn handle_event(state: &mut Self::State, event: Event) -> bool;
    fn build_view(state: &Self::State, width: u16, height: u16) -> Vec<Line>;
}

/// Generates the `#[no_mangle] extern "C"` exports a `dylib_app!`-based
/// `logic` crate needs — the state/output buffers, and `state_ptr`/
/// `output_ptr`/`on_tick`/`handle_event`/`build_view`, all marshaling
/// through `postcard` and [`encode_lines`] generically over `$app::State`.
#[macro_export]
macro_rules! dylib_app {
    ($app:ty) => {
        static mut __TERMOXIDE_STATE_BUF: [u8; $crate::STATE_CAPACITY] = [0; $crate::STATE_CAPACITY];
        static mut __TERMOXIDE_OUTPUT_BUF: [u8; $crate::OUTPUT_CAPACITY] = [0; $crate::OUTPUT_CAPACITY];

        #[unsafe(no_mangle)]
        pub extern "C" fn state_ptr() -> *mut u8 {
            #[allow(static_mut_refs)]
            unsafe {
                __TERMOXIDE_STATE_BUF.as_mut_ptr()
            }
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn output_ptr() -> *mut u8 {
            #[allow(static_mut_refs)]
            unsafe {
                __TERMOXIDE_OUTPUT_BUF.as_mut_ptr()
            }
        }

        fn __termoxide_decode_state(len: usize) -> <$app as $crate::DylibApp>::State {
            #[allow(static_mut_refs)]
            let bytes = unsafe { &__TERMOXIDE_STATE_BUF[..len] };
            $crate::postcard::from_bytes(bytes).unwrap_or_default()
        }

        fn __termoxide_encode_state(state: &<$app as $crate::DylibApp>::State) -> usize {
            #[allow(static_mut_refs)]
            let buf = unsafe { &mut __TERMOXIDE_STATE_BUF[..] };
            $crate::postcard::to_slice(state, buf).map(|written| written.len()).unwrap_or(0)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn on_tick(state_len: usize) -> usize {
            let mut state = __termoxide_decode_state(state_len);
            <$app as $crate::DylibApp>::on_tick(&mut state);
            __termoxide_encode_state(&state)
        }

        /// `event_kind` is `0` for `ChannelReady`, `1` for a key press
        /// (with `key_tag`/`key_payload`/`key_modifiers` then meaningful).
        /// Returns `(new_state_len << 1) | quit_bit`.
        #[unsafe(no_mangle)]
        pub extern "C" fn handle_event(
            state_len: usize,
            event_kind: i32,
            key_tag: i32,
            key_payload: i32,
            key_modifiers: i32,
        ) -> usize {
            let mut state = __termoxide_decode_state(state_len);
            let event = if event_kind == 0 {
                $crate::Event::ChannelReady
            } else {
                $crate::Event::KeyPress {
                    code: $crate::decode_key_code(key_tag, key_payload),
                    modifiers: $crate::Modifiers(key_modifiers as u8),
                }
            };
            let quit = <$app as $crate::DylibApp>::handle_event(&mut state, event);
            let new_len = __termoxide_encode_state(&state);
            (new_len << 1) | usize::from(quit)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn build_view(state_len: usize, width: u16, height: u16) -> usize {
            let state = __termoxide_decode_state(state_len);
            let lines = <$app as $crate::DylibApp>::build_view(&state, width, height);
            #[allow(static_mut_refs)]
            let buf = unsafe { &mut __TERMOXIDE_OUTPUT_BUF[..] };
            $crate::encode_lines(&lines, buf)
        }
    };
}
