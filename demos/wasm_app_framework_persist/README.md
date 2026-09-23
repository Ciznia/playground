# hot-reload draft: wasm (framework-owned host, persistent)

Built directly on top of `hot_reload_wasm_framework`: same
`termoxide_hot_reload_wasm_abi` crate, same `wasm_app!` macro, same
`AppState` + `WasmApp` impl. The only addition is opting into
[`termoxide_hot_reload::run_persistent`](../../crates/termoxide_hot_reload/src/lib.rs)
in place of `run`, so state survives a restart of the host process itself.

## What changed, end to end

- **[`src/main.rs`](src/main.rs)**: `termoxide_hot_reload::run` →
  `run_persistent`, plus one `PathBuf` for the snapshot location.
- **[`logic/src/lib.rs`](logic/src/lib.rs)**: **nothing** — not even a new
  derive. `AppState` already had `#[derive(Serialize, Deserialize)]` in
  the base draft, because the *live* wasm reload boundary already needs
  it (state has to cross into the guest as bytes on every single call,
  reload or not). Persistence adds no new requirement on top of that.
- **`termoxide_hot_reload`**: gained `run_persistent` and a shared `drive`
  helper both `run` and `run_persistent` call. `run_persistent` reads a
  snapshot file into the starting `state_bytes` and writes `state_bytes`
  back out after every change — see below for why that's *all* it needs
  to do.

## Framework side: persistence costs nothing extra here

Contrast this with the hot-lib-reloader-framework persist variant, where
`run_persistent` needed a new `S: Serialize + DeserializeOwned` bound and
an app-side derive, because host held a *live, in-process, un-encoded*
`S` value that had never been serialized before — persistence was the
first time encoding was needed at all there.

For wasm, encoding already happened, on every call, as part of the live
reload boundary itself (see the base draft's README): host has held the
app's state as an opaque `postcard`-encoded `Vec<u8>` since the very
first tick, because that's the only way state can reach the wasm guest.
`run_persistent` doesn't add an encode step — it just writes those exact
same bytes to a file:

```rust
pub fn run_persistent(config: WasmConfig, snapshot_path: PathBuf) -> Result<()> {
    let initial = std::fs::read(&snapshot_path).unwrap_or_default();
    drive(config, initial, |bytes| { let _ = std::fs::write(&snapshot_path, bytes); })
}
```

No trait bound, no derive, no app-side change beyond calling
`run_persistent`. This is the more minimal of the two persistence stories
in this set of drafts, precisely because the wasm transport already pays
the "state must be bytes" cost up front, for a different reason.

## Try it

```bash
rustup target add wasm32-unknown-unknown   # once
cd demos/wasm_app_framework_persist
cargo run
```

Quit (`q` or `Ctrl-C`) and `cargo run` again — `ticks`/`count` resume from
where they left off under a new pid. Editing
[`logic/src/lib.rs`](logic/src/lib.rs) still hot-swaps without a restart,
exactly as in the non-persistent draft.

Verified live: killed the process outright (not a graceful quit) after
`ticks` had climbed past 190, then ran again — the second run logged
`restored state from ...` and resumed at `ticks: 190`, not `0`, under a
different pid.
