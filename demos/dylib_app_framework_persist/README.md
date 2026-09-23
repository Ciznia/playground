# hot-reload draft: dylib-view-swap (framework-owned host, persistent)

Built directly on top of `hot_reload_dylib_view_swap_framework`: same
`termoxide_hot_reload_dylib_abi` crate, same `dylib_app!` macro, same
`AppState` + `DylibApp` impl. The only addition is opting into
[`termoxide_hot_reload::run_persistent`](../../crates/termoxide_hot_reload/src/lib.rs)
in place of `run`, so state survives a restart of the host process itself.

## What changed, end to end

- **[`src/main.rs`](src/main.rs)**: `termoxide_hot_reload::run` →
  `run_persistent`, plus one `PathBuf` for the snapshot location.
- **[`logic/src/lib.rs`](logic/src/lib.rs)**: **nothing** (beyond a
  cosmetic label change) — `AppState` already had `#[derive(Serialize,
  Deserialize)]` in the base draft, because the live dylib reload
  boundary already needs it (state crosses into `logic` as bytes on every
  call, reload or not). Persistence adds no new requirement.
- **`termoxide_hot_reload`**: already carried `run_persistent` and the
  shared `drive` helper (built alongside the base draft in this same
  crate) — the persist variant only had to wire it up from `main.rs`.

## Framework side: persistence costs nothing extra here, same as wasm

Host has held `AppState` as an opaque `postcard`-encoded `Vec<u8>` since
the very first tick — that's how state crosses into `logic`'s pointer-based
exports at all. `run_persistent` doesn't add an encode step; it just
writes those same bytes to a file and reads them back on the next start:

```rust
pub fn run_persistent(config: DylibConfig, snapshot_path: PathBuf) -> Result<()> {
    let initial = fs::read(&snapshot_path).unwrap_or_default();
    drive(config, initial, |bytes| { let _ = fs::write(&snapshot_path, bytes); })
}
```

No trait bound beyond what the base draft already required, no app-side
derive to add. Same story as the wasm-framework persist variant, and for
the same underlying reason: both transports already pay the "state must
be bytes" cost for the live reload boundary, so persistence is free once
that's in place. Contrast with the hot-lib-reloader-framework persist
variant, where persistence was the *first* time encoding was needed at
all (real Rust types cross that boundary live), so it needed a new
`Serialize`/`Deserialize` bound and app-side derive that didn't otherwise
exist.

## Try it

```bash
cd demos/dylib_app_framework_persist
cargo run
```

Quit (`q` or `Ctrl-C`) and `cargo run` again — `ticks`/`count` resume from
where they left off under a new pid. Editing
[`logic/src/lib.rs`](logic/src/lib.rs) still hot-swaps without a restart,
exactly as in the non-persistent draft.

Verified live: killed the process outright (not a graceful quit) after
`ticks` had climbed past 232, then ran again — the second run logged
`restored state from ...` and resumed at `ticks: 232`, not `0`, under a
different pid.
