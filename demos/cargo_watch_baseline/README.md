# Baseline: what `cargo-watch` alone gets you

Not a draft — there's no code here. This is the "why we built the rest of
these" comparison point: what a TermOxide app gets from off-the-shelf
tooling with zero custom integration, and exactly where it falls short
Every claim below was run live against `demos/external_app`, not assumed.

## Command

```bash
cargo install cargo-watch   # one-time
cargo watch -w demos/external_app -x 'run --manifest-path demos/external_app/Cargo.toml'
```

That's the entire "hot reload" setup. No `termoxide` changes, no supervisor
process to write, no ABI to design.

## What actually happened, tested live

1. **It restarts the app on save**, and — contrary to what I expected
   going in — it did **not** hit the Windows file-lock error the
   `process_supervisor` draft's first version hit
   (`Accès refusé (os error 5)` when a rebuild tries to overwrite a
   `.exe` its own old instance still has open). `cargo-watch` (via the
   `watchexec` crate underneath) kills the old process and waits for it
   to fully exit *before* running the next `cargo run`, rather than
   trying to rebuild while the old instance is still up. That's the
   conservative ordering — no overlap between "still running" and
   "being rebuilt", so nothing is ever open when the linker needs to
   write it. It costs the "keep the old UI up during compile" property
   the `process_supervisor` draft's blue/green slots exist specifically
   to preserve, but it does dodge the lock bug entirely.

2. **No state survives a reload.** Expected and confirmed: `ticks`
   resets to 0 every time, because the whole point is a fresh process
   with fresh `Signal`s. There's no hook for anything else — persistence
   would have to be built entirely outside `cargo-watch`, which is
   exactly what the `process_supervisor_persist` draft is.

3. **The shutdown is not graceful, confirmed from the raw terminal
   byte stream.** Across a full reload cycle, the old process's
   `[?1049h` (enter alternate screen) was never followed by a matching
   `[?1049l` (leave it) before the next process's own `[?1049h` showed
   up. That means the old instance's `Drop` impls — the ones that
   restore the terminal — never ran; `watchexec` kills it outright
   (`TerminateProcess` on Windows) rather than asking it to exit. Every
   other draft in this set deliberately avoids this, either through a
   sentinel file the app polls for (`process_supervisor`) or by never
   restarting the process holding the terminal at all (`dylib_view_swap`,
   `wasm`, `hot_lib_reloader`).

## Where this sits relative to the other drafts

| | `cargo-watch` alone | `process_supervisor` |
|---|---|---|
| Extra code needed | none | a `termoxide_watch` binary + a sentinel-poll hook in `termoxide::run_with_app` |
| Windows rebuild-lock safe | yes, by luck of kill-then-build ordering | yes, by design (blue/green slots) |
| Graceful shutdown | no — hard kill | yes — sentinel file, `Drop` runs normally |
| State survives reload | never | opt-in (`process_supervisor_persist`) |
| Old UI visible during compile | no | yes |

It's a legitimate starting point for a five-minute "just try hot reload"
moment, and worth keeping in the back pocket for exactly that. It stops
being enough the moment you care about not losing what you were looking
at, or about the terminal not occasionally needing a manual `reset` after
a kill mid-redraw.
