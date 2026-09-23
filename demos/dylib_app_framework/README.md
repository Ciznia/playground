# hot-reload draft: dylib-view-swap (framework-owned host)

Same transport as the `hot_reload_dylib_view_swap` draft — `logic` builds
as a `cdylib`, loaded and swapped via `libloading` — but the `#[repr(C)]`
wire format, the FFI marshaling, and the whole host loop have moved into
two new framework crates. The app author's entire surface is a state
struct and one trait impl.

## Framework side: two crates

- **[`termoxide_hot_reload_dylib_abi`](../../crates/termoxide_hot_reload_dylib_abi)**
  — defines the wire format (state buffer, output buffer, line encoding),
  the framework-fixed `Event`/`KeyCode`/`Color` types, the
  [`DylibApp`](../../crates/termoxide_hot_reload_dylib_abi/src/lib.rs)
  trait, and the [`dylib_app!`](../../crates/termoxide_hot_reload_dylib_abi/src/lib.rs)
  macro that generates every `#[no_mangle] extern "C"` export a guest
  needs.
- **[`termoxide_hot_reload`](../../crates/termoxide_hot_reload)** — the
  host runtime: the Windows generation-numbered-copy trick, `libloading`,
  the rebuild watcher, the terminal/event/render loop. `run` takes **no
  type parameter**, for the same reason as the wasm-framework crate: host
  only ever holds the app's state as an opaque `postcard`-encoded
  `Vec<u8>` and never deserializes it.

## App side: a struct and a trait impl

[`logic/src/lib.rs`](logic/src/lib.rs), in full:

```rust
#[derive(Default, Serialize, Deserialize)]
pub struct AppState { pub count: u32, pub ticks: u64 }

pub struct App;

impl DylibApp for App {
    type State = AppState;
    fn on_tick(state: &mut AppState) { state.ticks += 1; }
    fn handle_event(state: &mut AppState, event: Event) -> bool { /* ... */ }
    fn build_view(state: &AppState, width: u16, height: u16) -> Vec<Line> { /* ... */ }
}

termoxide_hot_reload_dylib_abi::dylib_app!(App);
```

Compare this to `demos/dylib_app`'s `abi`/`logic` crates in the
non-framework draft: `termoxide_dylib_abi` hand-defines `#[repr(C)]`
structs (`FfiState`, `FfiFrame`, `FfiLine`, fixed `[u8; N]` text buffers)
that both sides must keep in sync field-for-field, on pain of undefined
behavior if they ever drift. Here there's no `#[repr(C)]` type at all —
`postcard` handles the encoding generically, the same mechanism the
wasm-framework drafts use.

## Why the host needs no generics here, same reasoning as wasm

Exactly as with the wasm form of this framework: `logic`'s `dylib_app!`
expansion encodes `AppState` to bytes via `postcard` before host ever
sees it, and host only ever holds that opaque blob, passing it back in
unexamined. `run`/`run_persistent` need no type parameter and no per-app
trait bound.

## The one real difference from wasm: no sandbox, so the Windows dylib lock is back

Wasm never keeps an OS handle open on the built `.wasm` file — it's read
into memory once and forgotten. A native `cdylib` loaded via `libloading`
*does* hold a handle for as long as it's loaded, so Windows won't let the
linker overwrite it on the next build. `termoxide_hot_reload` carries the
same fix the non-framework `dylib_view_swap` draft uses: every reload
copies the freshly built `.dll` to a new, incrementally-numbered file
(`logic-1.dll`, `logic-2.dll`, …) under a per-process temp directory, and
loads *that* — `cargo build`'s own output path is therefore never a file
this process currently has open.

## A real bug this caught during live-testing

Filtering `cargo build --message-format=json`'s output for the `logic`
artifact by "ends with `.dll`" is **not enough** once `logic` depends on
a proc-macro crate — which it does here, for `#[derive(Serialize,
Deserialize)]` via `serde_derive`. Proc-macro crates are *also* compiled
to a native `.dll` as part of the same build (they run on the host during
compilation, regardless of what target the main crate builds for), and
that compiler-artifact message can appear before `logic`'s own. The first
version of this crate picked up `serde_derive`'s `.dll` instead, copied
*that* into the generation slot, and failed at symbol resolution
(`GetProcAddress` — "the specified procedure could not be found") because
none of `state_ptr`/`on_tick`/etc. exist in a proc-macro's exports.
Fixed by filtering on the artifact's `target.kind` containing `"cdylib"`
instead of the filename alone (see `is_cdylib_artifact` in
`termoxide_hot_reload/src/lib.rs`) — caught only because this draft was
run for real, not just compiled; `cargo build`/`cargo clippy` both stayed
clean throughout.

## Try it

```bash
cd demos/dylib_app_framework
cargo run
```

Edit [`logic/src/lib.rs`](logic/src/lib.rs) (e.g. the string literal in
`build_view`) and save — verified live: pid and generation footer confirm
the swap while `ticks`/`count` keep counting underneath it.
