# hot-reload draft: wasm (framework-owned host)

Same transport as the `hot_reload_wasm` draft — `logic` compiles to
`wasm32-unknown-unknown` and runs sandboxed in `wasmtime` — but every
piece of the wire format, the FFI marshaling, and the host loop has moved
into two new framework crates. The app author's entire surface is a state
struct and one trait impl: no `abi` crate to hand-write, no `output_ptr`,
no byte cursor.

## Framework side: two crates

- **[`termoxide_hot_reload_wasm_abi`](../../crates/termoxide_hot_reload_wasm_abi)**
  — compiles for *both* `wasm32-unknown-unknown` and native, since guest
  and host both need it. Defines the wire format (state buffer, output
  buffer, line encoding), the framework-fixed `Event`/`KeyCode`/`Color`
  types, the [`WasmApp`](../../crates/termoxide_hot_reload_wasm_abi/src/lib.rs)
  trait, and the [`wasm_app!`](../../crates/termoxide_hot_reload_wasm_abi/src/lib.rs)
  macro that generates every `#[no_mangle] extern "C"` export a guest
  needs from one line.
- **[`termoxide_hot_reload`](../../crates/termoxide_hot_reload)** — the
  native-only host runtime: `wasmtime` module loading, the rebuild
  watcher, the terminal/event/render loop. `run` takes **no type
  parameter** — see below for why that's possible here but wasn't for the
  hot-lib-reloader form of this same crate.

## App side: a struct and a trait impl

[`logic/src/lib.rs`](logic/src/lib.rs), in full:

```rust
#[derive(Default, Serialize, Deserialize)]
pub struct AppState { pub count: u32, pub ticks: u64 }

pub struct App;

impl WasmApp for App {
    type State = AppState;
    fn on_tick(state: &mut AppState) { state.ticks += 1; }
    fn handle_event(state: &mut AppState, event: Event) -> bool { /* ... */ }
    fn build_view(state: &AppState, width: u16, height: u16) -> Vec<Line> { /* ... */ }
}

termoxide_hot_reload_wasm_abi::wasm_app!(App);
```

Compare this to `demos/wasm_app`'s `logic/src/lib.rs` in the non-framework
draft, which hand-writes `output_ptr`, a `write_line` cursor function, and
raw `#[repr]`-free scalar exports. `logic` here doesn't even depend on
`ratatui`/`termoxide_event`/`termoxide_rendering` — `Event`/`KeyCode`/
`Line`/`Color` are the abi crate's own small, wasm-boundary-safe
equivalents.

[`src/main.rs`](src/main.rs), in full, has no app-specific information at
all:

```rust
fn main() -> anyhow::Result<()> {
    termoxide_hot_reload::run(WasmConfig { logic_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("logic") })
}
```

## Why the host needs no generics here, unlike hot-lib-reloader

`termoxide_hot_reload`'s hot-lib-reloader form still had to be generic
over the app's state type (`run::<S>`), because host held a real `S`
value in-process and called real function pointers over it. Core wasm
can't pass a real Rust type across the boundary at all — every app's
`logic` crate already encodes its state to bytes via `postcard` (inside
`wasm_app!`'s generated code) before it ever leaves the guest. Host only
ever holds that encoded blob (`Vec<u8>`) and hands it back in unexamined
on the next call — it never deserializes it, so it genuinely never needs
to know the app's `State` type. [`run`](../../crates/termoxide_hot_reload/src/lib.rs)
takes no type parameter and needs no per-app trait bound: the exact same
function, byte for byte, runs every `wasm_app!`-based app.

## What this avoids, concretely

The user-supplied "avoid a constraint like the app author hand-writing a
serializer" goal, in practice: `AppState` needs `#[derive(Serialize,
Deserialize)]` — that's the entire encoding cost paid by the app. Nobody
writes `write_line`, a length-prefixed cursor, or an `output_ptr`
function by hand. `postcard::to_slice`/`from_bytes`, called generically
inside `wasm_app!`'s macro expansion, do the state encode/decode; the
line-output wire format ([`encode_lines`]/[`decode_lines`] in the abi
crate) is written once, by the framework, and reused by every app.

## What has to be done for a reload to work

All in `termoxide_hot_reload::run`, none of it in the app:

1. Watch `logic/src` + `logic/Cargo.toml`, debounce, rebuild via `cargo
   build --target wasm32-unknown-unknown` (same blocking-rebuild
   characteristic as the non-framework `wasm` draft: the `cargo build`
   call is inline in the render loop, so the terminal freezes on the last
   frame for the whole build — this crate ports that behavior rather than
   fixing it).
2. On success, `Module::from_file` + instantiate, resolve `state_ptr`/
   `output_ptr`/`on_tick`/`handle_event`/`build_view`/`memory` by the
   fixed names in `termoxide_hot_reload_wasm_abi`.
3. Each frame: write the host's last-known opaque state bytes into the
   guest's state buffer, call the relevant export, read the (possibly
   resized) state bytes back out, then do the same read-write-call-read
   dance for `build_view`, decode its line output, append the `pid`/
   generation footer, render.
4. No Windows file-lock dance needed, same as the non-framework draft:
   `host` never keeps an OS handle open on the `.wasm` file.

## Try it

```bash
rustup target add wasm32-unknown-unknown   # once
cd demos/wasm_app_framework
cargo run
```

Edit [`logic/src/lib.rs`](logic/src/lib.rs) (e.g. the string literal in
`build_view`) and save — verified live: pid and the running `ticks` count
(confirmed still advancing via the raw terminal diff stream — ratatui
only redraws changed cells, so grepping full "ticks: N" strings in the
output undercounts) both survive the swap untouched.
