//! The host/guest protocol for `wasm_app`.
//!
//! `logic` compiles to `wasm32-unknown-unknown`, so the boundary here is
//! stricter than the dylib draft's: core wasm exports only take/return
//! `i32`/`i64` scalars — no passing a struct by value. Anything richer
//! (the rendered status text) goes through `logic`'s own linear memory
//! instead: `logic` writes bytes at a fixed offset, `host` reads them back
//! with `wasmtime::Memory::data()` after the call returns.
//!
//! Everything that fits in a scalar (the tick counter, key code, quit flag)
//! stays a plain argument/return value — memory is only used for the one
//! thing that's genuinely variable-length: the frame's text.

/// Total bytes reserved for `build_view`'s output. Generous for a handful
/// of short status lines; `build_view` must not write past this.
pub const OUTPUT_CAPACITY: usize = 2048;
/// Max bytes one line's text may occupy — small enough that a `u8` length
/// prefix (see the wire format below) can always hold it.
pub const MAX_LINE_TEXT_LEN: usize = 120;

/// Wire format `build_view` writes starting at the address `output_ptr`
/// returns (not a fixed offset — address 0 is a poor choice to write to on
/// any target, wasm included, since compilers reasonably assume nothing
/// meaningful ever lives there). The exported function's `i32` return
/// value is the total byte count written:
///
/// ```text
/// repeat until `total_bytes` bytes are consumed:
///   u8  color tag (see `color`)
///   u8  text length N (<= MAX_LINE_TEXT_LEN)
///   N bytes of UTF-8 text
/// ```
///
/// Self-delimiting by construction — `host` doesn't need a separate line
/// count, just the total byte length `build_view` returned.
pub mod color {
    pub const CYAN: u8 = 1;
    pub const DEFAULT: u8 = 0;
    pub const GREEN: u8 = 3;
    pub const MAGENTA: u8 = 4;
    pub const YELLOW: u8 = 2;
}

/// Tag values for `handle_key`'s `tag` parameter — one per `KeyCode`
/// variant, matching the dylib draft's `key_tag` module so the two are
/// easy to compare.
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

/// Bit values for `handle_key`'s `modifiers` parameter.
pub mod modifier {
    pub const SHIFT: i32 = 1 << 0;
    pub const CONTROL: i32 = 1 << 1;
    pub const ALT: i32 = 1 << 2;
    pub const SUPER: i32 = 1 << 3;
}

/// Unpack `handle_key`'s return value: `(new_count << 1) | quit_bit`. A
/// single scalar return is all core wasm gives a function, so the two
/// results share one `i32` rather than needing the multi-value proposal.
pub fn unpack_handle_key_result(packed: i32) -> (i32, bool) { (packed >> 1, packed & 1 != 0) }

/// Guest-side counterpart of [`unpack_handle_key_result`].
pub fn pack_handle_key_result(new_count: i32, quit: bool) -> i32 { (new_count << 1) | i32::from(quit) }

/// Names of the functions `host` resolves from the `logic` wasm instance.
pub const EXPORT_ON_TICK: &str = "on_tick";
pub const EXPORT_HANDLE_KEY: &str = "handle_key";
pub const EXPORT_BUILD_VIEW: &str = "build_view";
/// Returns the address, in the guest's exported `memory`, of the buffer
/// `build_view` writes into. `host` calls this once after loading and
/// caches the result — the buffer is a `static`, so its address never
/// changes for the instance's lifetime.
pub const EXPORT_OUTPUT_PTR: &str = "output_ptr";
pub const EXPORT_MEMORY: &str = "memory";
