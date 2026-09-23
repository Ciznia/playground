# hot-reload draft: hot-lib-reloader (framework-owned host)

Same transport as the `hot_reload_hot_lib_reloader` draft — `lib` builds as
a Rust `dylib`, `hot-lib-reloader` watches and swaps it in — but the host
loop, the rebuild watcher, and the Windows toolchain-`PATH` fixup have all
moved out of the app and into a new framework crate,
[`termoxide_hot_reload`](../../crates/termoxide_hot_reload). Compare
[`src/main.rs`](src/main.rs) here (17 lines) to the non-framework draft's
`host/src/main.rs` (~150 lines): this is what's left once everything that
isn't genuinely app-specific is pulled out.

## Framework side: `crates/termoxide_hot_reload`

New crate, not present on `dev/hot_reload`. It owns everything that has no
app-specific information in it:

- **[`ensure_toolchain_dlls_are_loadable`](../../crates/termoxide_hot_reload/src/lib.rs)**
  — the Windows `PATH` fixup so a Rust `dylib`'s `std-<hash>.dll` can be
  found by the loader. Gated `#[cfg(windows)]`; a no-op elsewhere.
- **`spawn_rebuild_watcher`** — watches `<lib>/src` and `<lib>/Cargo.toml`,
  debounces, and runs `cargo build -p <package>`. `hot-lib-reloader` only
  handles the *reload* half (swapping in an already-built dylib); it
  deliberately doesn't trigger builds itself, so every hot-lib-reloader app
  needs this piece regardless — it's identical code whether it's written
  once here or copy-pasted into every app's `host`.
- **`run`** — the whole event/tick/render loop: owns the app's `State`
  value for the process's lifetime, calls the three app-supplied function
  pointers each frame, appends the `pid`/generation footer line, drives
  `Renderer::render_frame`.
- **[`hot_lib_app!`](../../crates/termoxide_hot_reload/src/lib.rs)** — a
  `macro_rules!` that expands to the `#[hot_lib_reloader::hot_module]` +
  `hot_functions_from_file!` block every hot-lib-reloader app needs,
  including the framework-fixed type re-exports (`Rect`, `Event`,
  `ViewNode` — every app on this framework uses the same three).

`termoxide_reactive`, `termoxide_event`, `termoxide_rendering` are used
here exactly as before — still unmodified, still not gaining any
hot-reload-specific code. The new framework surface is entirely the new
`termoxide_hot_reload` crate; nothing under the original `crates/`
directories changed.

## App side: state + three functions, nothing else

Two files, both far smaller than their non-framework counterparts:

- **[`src/main.rs`](src/main.rs)** (17 lines) — one macro invocation
  (`hot_lib_app!`) naming the dylib package, the swappable source file, and
  the app's state type, then one call to `termoxide_hot_reload::run`
  handing over three function pointers. No watcher, no `PATH` fixup, no
  loop.
- **[`lib/src/lib.rs`](lib/src/lib.rs)** — the actual hot-swappable code:
  a plain `#[derive(Default)] struct AppState { count, ticks, last_key }`
  and three functions operating on `&AppState`/`&mut AppState`. This is
  the entire app-author surface — no FFI, no wire format, no host
  boilerplate to maintain. Same discipline as every other draft: this
  crate must not touch `termoxide_reactive::Signal`/`Owner` (a `dylib`
  statically embeds its own copy of every dependency, so a `Signal`
  created here would be a different runtime instance than the one `run`
  owns) — state stays a plain value, passed by reference.

## What still can't move to the framework, and why

`hot_lib_app!`'s three arguments are the one remaining app-specific
surface, and each is unavoidable:

1. **The literal path to `lib/src/lib.rs`.** `hot_lib_reloader::hot_module`
   parses that file *at the app's own compile time* to generate typed
   wrapper functions matching its exact signatures. The framework crate
   has no way to know where a given app's swappable crate lives — this
   has to come from the app.
2. **The dylib package name (`"lib"`).** Passed to both `hot_module` (to
   know which built artifact to watch) and the rebuild watcher (to know
   what to `cargo build -p`). Nothing stops an app from naming its crate
   something else.
3. **The app's `State` type.** The generated `hot_lib` module's function
   signatures reference it directly (`fn handle_event(state: &mut
   AppState, event: Event) -> bool`), so it has to be in scope inside that
   module — re-exported via `pub use $state;` in the macro expansion. This
   is the one piece of the boundary that's genuinely per-app; the
   framework fixes every other type crossing it.

## What has to be done for a reload to work

All in `termoxide_hot_reload::run`, none of it in the app:

1. Fix the Windows `PATH` once, before anything touches `hot_lib`.
2. Spawn the rebuild watcher (background thread, watches `lib/src` +
   `lib/Cargo.toml`, debounced `cargo build -p lib`).
3. Own `State::default()` for the process's lifetime.
4. Each frame: forward events into `handle_event`, tick via `on_tick`
   roughly every 100ms, call `build_view`, append the footer, render.
   Calling *any* `hot_lib::*` function is what makes `hot-lib-reloader`
   check for and transparently apply a pending reload before the call runs
   — there's no separate "check for reload" step to write.
5. Unlike the wasm/dylib-view-swap drafts, rebuilds here don't block the
   render loop: the `cargo build` in step 2 runs on its own background
   thread, so the terminal keeps redrawing at the normal 16ms cadence
   while a rebuild is in flight. The swap itself is still a hard cut (no
   partial/in-between frame) — it just doesn't freeze the UI to get there.

## Try it

```bash
cd demos/hot_lib_reloader_app_framework
cargo run
```

Edit [`lib/src/lib.rs`](lib/src/lib.rs) (e.g. the string literal in
`build_view`) and save — `pid` stays put, `count`/`ticks` keep counting,
only the swapped-in text changes.
