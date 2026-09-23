//! The hot-swappable half of `wasm_app`, compiled to `wasm32-unknown-unknown`.
//!
//! Same idea as the dylib draft's `logic` crate — pure functions, no state
//! of its own carried between calls — but through a plain-wasm boundary
//! instead of a native one: exports take/return only `i32`/`i64` scalars,
//! and the one genuinely variable-length value (the rendered status text)
//! goes through this module's own linear memory instead. See
//! `termoxide_wasm_abi` for the wire format.

use termoxide_wasm_abi::{color, key_tag, modifier, pack_handle_key_result};

/// Where `build_view` writes its output — a `static`, so its address is
/// fixed for the module's lifetime and `host` only needs to ask for it
/// once (via [`output_ptr`]).
static mut OUTPUT: [u8; termoxide_wasm_abi::OUTPUT_CAPACITY] = [0; termoxide_wasm_abi::OUTPUT_CAPACITY];

#[unsafe(no_mangle)]
pub extern "C" fn output_ptr() -> i32 {
    #[allow(static_mut_refs)]
    let ptr = &raw const OUTPUT as *const u8;
    ptr as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn on_tick(ticks: i64) -> i64 { ticks + 1 }

#[unsafe(no_mangle)]
pub extern "C" fn handle_key(count: i32, tag: i32, payload: i32, modifiers: i32) -> i32 {
    let is_char = |c: char| tag == key_tag::CHAR && payload == c as i32;
    let quit = is_char('q') || (is_char('c') && modifiers & modifier::CONTROL != 0);
    pack_handle_key_result(count + 1, quit)
}

/// Encode one line into `OUTPUT[cursor..]` as `[color, len, text_bytes...]`
/// (see the wire format doc on `termoxide_wasm_abi`), truncating text that
/// would overflow the line-length or output-buffer limits. Returns the new
/// cursor.
fn write_line(cursor: usize, color: u8, text: &str) -> usize {
    let max_len = termoxide_wasm_abi::MAX_LINE_TEXT_LEN;
    let bytes = text.as_bytes();
    let len = bytes.len().min(max_len);
    let len = (0..=len).rev().find(|&i| text.is_char_boundary(i)).unwrap_or(0);

    if cursor + 2 + len > termoxide_wasm_abi::OUTPUT_CAPACITY {
        return cursor; // Out of room: drop the line rather than corrupt the buffer.
    }

    #[allow(static_mut_refs)]
    unsafe {
        OUTPUT[cursor] = color;
        OUTPUT[cursor + 1] = len as u8;
        OUTPUT[cursor + 2..cursor + 2 + len].copy_from_slice(&bytes[..len]);
    }
    cursor + 2 + len
}

#[unsafe(no_mangle)]
pub extern "C" fn build_view(count: i32, ticks: i64) -> i32 {
    let mut cursor = 0;
    cursor = write_line(cursor, color::CYAN, "wasm termoxide app");
    cursor = write_line(cursor, color::YELLOW, &format!("ticks: {ticks} | key presses: {count}"));
    cursor = write_line(cursor, color::GREEN, "Controls: any key counts, q or Ctrl-C quits");
    cursor as i32
}
