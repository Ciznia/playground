# hot-reload draft: hot-lib-reloader (framework-owned host, persistent)

Built directly on top of `hot_reload_hot_lib_reloader_framework`: same
`termoxide_hot_reload` crate, same `hot_lib_app!` macro, same `AppState` +
three-function app shape. The only addition is opting into
[`termoxide_hot_reload::run_persistent`](../../crates/termoxide_hot_reload/src/lib.rs)
in place of `run`, so state survives a restart of the host process itself
(not just a `lib` reload, which already never touched state in the base
draft).

## What changed, end to end

- **[`src/main.rs`](src/main.rs)**: `termoxide_hot_reload::run` →
  `run_persistent`, plus one `PathBuf` for the snapshot location. That's
  the entire host-side diff.
- **[`lib/src/lib.rs`](lib/src/lib.rs)**: `AppState` gains
  `#[derive(Serialize, Deserialize)]`. Nothing else in the app changes —
  no snapshot path, no read/write code, no hand-written wire format.
- **`termoxide_hot_reload`**: gained `run_persistent`, `read_snapshot`,
  `write_snapshot`, and a `postcard`/`serde` dependency. The event/tick/
  render loop itself (`drive`) didn't change — `run` and `run_persistent`
  both call it, `run_persistent` just passes an `after_change` hook that
  writes a snapshot instead of a no-op.

## Framework side: encoding is generic, not hand-written per app

This is the one place in the whole hot-lib-reloader draft where a wire
format is unavoidable — unlike the live reload boundary (real Rust types,
in-process, no serialization needed), *persistence* genuinely crosses a
process boundary: this run of `host` writes a file, a **later, different**
run of `host` reads it back. There's no way around encoding `S` to bytes
for that.

What moved to the framework is *how* that encoding happens. `run_persistent`
only requires `S: Serialize + DeserializeOwned` — an ordinary
`#[derive(Serialize, Deserialize)]`, the same derive the app already needed
for nothing else. `termoxide_hot_reload::write_snapshot`/`read_snapshot`
do the actual encode/decode via `postcard`, generically over any `S`. The
app never writes pack/unpack code, a cursor, a length prefix, or an output
buffer — contrast this with the hand-rolled wire formats the `dylib`/`wasm`
drafts' `abi` crates define, which every app on those transports has to
maintain itself.

## Try it

```bash
cd demos/hot_lib_reloader_app_framework_persist
cargo run
```

Press a few keys, then quit (`q` or `Ctrl-C`) and `cargo run` again —
`count`/`ticks`/`last_key` pick up where they left off, even though the
process has a new pid. Editing [`lib/src/lib.rs`](lib/src/lib.rs) still
hot-swaps without a restart, exactly as in the non-persistent draft; the
persistence only matters across an actual restart of `host`.

Verified live: killed the process outright (not a graceful quit) after
`ticks` had climbed past 270, then ran again — the second run logged
`restored state from ...` and resumed at `ticks: 270`, not `0`, under a
different pid.
