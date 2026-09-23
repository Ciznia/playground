//! The FFI boundary between `host` and `logic`.
//!
//! Everything here is `#[repr(C)]` plain data — no `Vec`, `String`, `Box`, or
//! `Signal` crosses this boundary. That's deliberate, for two independent
//! reasons:
//!
//! 1. **No shared allocator.** `host` and `logic` are separate compilation
//!    units; a `Vec`/`String` allocated on one side and freed on the other
//!    is undefined behaviour unless both link the same allocator instance,
//!    which a plain `cdylib` does not guarantee. Fixed-size buffers avoid
//!    the question entirely.
//! 2. **No reactive state in `logic`.** `logic` never touches
//!    `termoxide_reactive::Signal` — if it did, its statically-linked copy
//!    of the reactive runtime would be a *different instance* from the
//!    host's, invisible to the host's `Owner`. So `logic` is pure: plain
//!    values in ([`FfiState`], [`FfiKey`]), plain values out
//!    ([`FfiHandleResult`], [`FfiFrame`]). `host` alone owns every `Signal`
//!    and reads/writes them around each call.

/// Max UTF-8 bytes a single [`FfiText`] can hold. Generous for the demo's
/// short status lines; a real app would size this to its own content.
pub const MAX_TEXT_LEN: usize = 96;
/// Max lines [`FfiFrame`] can carry in one frame.
pub const MAX_LINES: usize = 8;

/// A fixed-capacity UTF-8 string. Truncates rather than allocates.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FfiText {
    pub buf: [u8; MAX_TEXT_LEN],
    pub len: u32,
}

impl FfiText {
    pub const EMPTY: Self = Self { buf: [0; MAX_TEXT_LEN], len: 0 };

    pub fn as_str(&self) -> &str { std::str::from_utf8(&self.buf[..self.len as usize]).unwrap_or("") }
}

impl From<&str> for FfiText {
    /// Truncates to [`MAX_TEXT_LEN`] bytes, on a char boundary so
    /// [`as_str`](Self::as_str) never sees a partial UTF-8 sequence.
    fn from(s: &str) -> Self {
        let bytes = s.as_bytes();
        let len = bytes.len().min(MAX_TEXT_LEN);
        let len = (0..=len).rev().find(|&i| s.is_char_boundary(i)).unwrap_or(0);
        let mut buf = [0u8; MAX_TEXT_LEN];
        buf[..len].copy_from_slice(&bytes[..len]);
        Self { buf, len: len as u32 }
    }
}

/// The state `host` hands to every `logic` call — a snapshot of the signals
/// it owns, read just before the call.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FfiState {
    pub count: u32,
    pub ticks: u64,
    pub last_key: FfiText,
}

/// A key press, backend-agnostic like [`termoxide_event::event::KeyEvent`]
/// but flattened to plain fields for the FFI boundary.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FfiKey {
    /// One of the [`key_tag`] constants.
    pub tag: u8,
    /// `key_tag::CHAR` → the `char` as `u32`; `key_tag::F` → the function
    /// key number; unused otherwise.
    pub payload: u32,
    /// Bitwise-OR of [`modifier`] constants.
    pub modifiers: u8,
}

/// Tag values for [`FfiKey::tag`], one per
/// [`KeyCode`](termoxide_event::event::KeyCode) variant.
pub mod key_tag {
    pub const BACKSPACE: u8 = 0;
    pub const ENTER: u8 = 1;
    pub const LEFT: u8 = 2;
    pub const RIGHT: u8 = 3;
    pub const UP: u8 = 4;
    pub const DOWN: u8 = 5;
    pub const HOME: u8 = 6;
    pub const END: u8 = 7;
    pub const PAGE_UP: u8 = 8;
    pub const PAGE_DOWN: u8 = 9;
    pub const TAB: u8 = 10;
    pub const BACK_TAB: u8 = 11;
    pub const DELETE: u8 = 12;
    pub const INSERT: u8 = 13;
    pub const F: u8 = 14;
    pub const CHAR: u8 = 15;
    pub const NULL: u8 = 16;
    pub const ESC: u8 = 17;
}

/// Bit values for [`FfiKey::modifiers`], matching
/// [`KeyModifiers`](termoxide_event::event::KeyModifiers)'s own bit
/// positions so translation is a straight copy.
pub mod modifier {
    pub const SHIFT: u8 = 1 << 0;
    pub const CONTROL: u8 = 1 << 1;
    pub const ALT: u8 = 1 << 2;
    pub const SUPER: u8 = 1 << 3;
}

/// What `logic` hands back after handling a key press — the new values for
/// whatever signals it's allowed to change, plus a quit flag.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FfiHandleResult {
    pub count: u32,
    pub last_key: FfiText,
    /// Non-zero to stop the host's main loop.
    pub quit: u8,
}

/// A small fixed palette, so `logic` doesn't need `ratatui` as a dependency
/// just to say "yellow".
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FfiColor(pub u8);

impl FfiColor {
    pub const CYAN: Self = Self(1);
    pub const DEFAULT: Self = Self(0);
    pub const GREEN: Self = Self(3);
    pub const MAGENTA: Self = Self(4);
    pub const YELLOW: Self = Self(2);
}

/// One line of `logic`'s view output.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FfiLine {
    pub text: FfiText,
    pub color: FfiColor,
}

impl FfiLine {
    pub const EMPTY: Self = Self { text: FfiText::EMPTY, color: FfiColor::DEFAULT };
}

/// `logic`'s entire view output for one frame — a fixed-size array rather
/// than a `Vec` for the same cross-allocator reason as [`FfiText`].
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FfiFrame {
    pub lines: [FfiLine; MAX_LINES],
    pub line_count: u32,
}

impl FfiFrame {
    pub const EMPTY: Self = Self { lines: [FfiLine::EMPTY; MAX_LINES], line_count: 0 };
}

/// Symbol names `host` resolves from the loaded `logic` dylib. Named here
/// once so the string literal can't drift between the two crates.
pub const SYMBOL_ON_TICK: &[u8] = b"termoxide_on_tick\0";
pub const SYMBOL_HANDLE_KEY: &[u8] = b"termoxide_handle_key\0";
pub const SYMBOL_BUILD_VIEW: &[u8] = b"termoxide_build_view\0";

pub type OnTickFn = unsafe extern "C" fn(ticks: u64) -> u64;
pub type HandleKeyFn = unsafe extern "C" fn(state: FfiState, key: FfiKey) -> FfiHandleResult;
pub type BuildViewFn = unsafe extern "C" fn(state: FfiState, width: u16, height: u16) -> FfiFrame;
