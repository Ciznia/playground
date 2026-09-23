//! Dev-time supervisor for hot reload.
//!
//! `termoxide-watch` never touches a running app's terminal or process
//! image. It watches a crate's `src/` and `Cargo.toml`, and on change:
//!
//! 1. Rebuilds the target binary, while the current instance keeps running and stays on screen.
//! 2. Copies the freshly built binary into whichever of two alternating "slots" ([`Slot::Blue`] / [`Slot::Green`])
//!    isn't the one currently running — see [`Slot::path`]. The running instance always executes from a slot copy,
//!    never from cargo's own output path, so the linker is always free to overwrite that path on the next rebuild — on
//!    Windows especially, it cannot overwrite a binary that's still running (`Accès refusé (os error 5)`), which is
//!    exactly what rebuilding in place while the old process is up would attempt. A failed build is reported and
//!    otherwise ignored — the running app, on its own unrelated slot, is untouched.
//! 3. Asks the running app to shut down gracefully (by writing the path in `TERMOXIDE_RELOAD_SENTINEL`, which
//!    `termoxide::run_with_app` polls — see `crates/termoxide/src/lib.rs`), falling back to a hard kill if it doesn't
//!    exit in time.
//! 4. Spawns the other slot's freshly copied binary with inherited stdio, so it owns the real terminal exactly as it
//!    would under a plain `cargo run`.
//!
//! State does not survive a reload by default — the new process starts from
//! scratch, same as a normal `cargo run`. Passing `--persist-state` opts a
//! session into carrying it across: this process picks one file path (see
//! [`state_path`]) and passes it to every spawn via `TERMOXIDE_RELOAD_STATE`,
//! which `termoxide::run_with_app` reads. The app itself still decides
//! whether it has anything worth saving — `termoxide`'s `App::snapshot` /
//! `App::restore` default to doing nothing, so the flag alone does not
//! magically persist a state type the app never opted in to encoding.
//!
//! ## Ctrl-C
//!
//! While the running app is alive it has the terminal in raw mode, which
//! disables OS-level signal generation for Ctrl-C on that console —
//! crossterm decodes it as an ordinary key press instead, and the app
//! handles it (or not) through its normal event loop exactly as it would
//! without this tool. So a Ctrl-C handler here only ever fires in the
//! windows where no child is holding raw mode: before the first spawn, and
//! after a child has exited on its own (e.g. the user quit it directly) and
//! this process is left idling. In both cases it stops watching and exits
//! rather than being torn down mid-operation by the OS's default handling.
//! A second Ctrl-C forces an immediate exit if the first is taking too long.
//!
//! # Usage
//!
//! ```text
//! cargo run -p termoxide_watch -- --manifest-path demos/external_app/Cargo.toml
//! ```

use std::{
    env,
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

/// How long to wait for the app to exit on its own after asking it to,
/// before falling back to a hard kill.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

/// How long to wait after the first file-system event before rebuilding, so
/// that a burst of saves (editors often write, then rename) collapses into
/// one rebuild instead of several.
const DEBOUNCE: Duration = Duration::from_millis(300);

/// How often the idle wait re-checks the Ctrl-C flag between file events.
const INTERRUPT_POLL: Duration = Duration::from_millis(200);

struct Args {
    manifest_path: PathBuf,
    bin: Option<String>,
    /// `--persist-state`: opt this session into carrying app state across a
    /// reload. See the module docs and [`state_path`].
    persist_state: bool,
}

fn parse_args() -> Result<Args> {
    let mut manifest_path = None;
    let mut bin = None;
    let mut persist_state = false;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--manifest-path" => {
                manifest_path = Some(PathBuf::from(args.next().context("--manifest-path needs a value")?));
            },
            "--bin" => {
                bin = Some(args.next().context("--bin needs a value")?);
            },
            "--persist-state" => persist_state = true,
            other => {
                bail!(
                    "unrecognized argument: {other}\n\nUsage: termoxide-watch --manifest-path <path/to/Cargo.toml> \
                     [--bin <name>] [--persist-state]"
                )
            },
        }
    }

    Ok(Args {
        manifest_path: manifest_path.context(
            "missing required --manifest-path <path/to/Cargo.toml>\n\nUsage: termoxide-watch --manifest-path \
             <path/to/Cargo.toml> [--bin <name>] [--persist-state]",
        )?,
        bin,
        persist_state,
    })
}

/// Path a persisted state snapshot lives at for the whole watch session.
///
/// Stable across every spawn in this session (keyed by this supervisor's own
/// pid, not the child's, which changes every reload) so the file the old
/// instance wrote is the same one the new instance reads.
fn state_path() -> PathBuf { env::temp_dir().join(format!("termoxide-reload-{}.state", std::process::id())) }

/// The binary this session watches, and where its source and build output
/// live.
struct Target {
    bin_name: String,
    crate_dir: PathBuf,
    /// Where the slot copies live: `<target_directory>/termoxide-watch/`.
    /// Read from `cargo metadata` rather than assumed to be `<crate_dir>/target`,
    /// since `CARGO_TARGET_DIR` or a workspace can relocate it.
    run_dir: PathBuf,
}

/// Ask `cargo metadata` for the manifest's binary targets and settle on one:
/// the explicit `--bin`, or the sole binary the crate declares.
fn resolve_target(args: &Args) -> Result<Target> {
    let output = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1", "--manifest-path"])
        .arg(&args.manifest_path)
        .output()
        .context("failed to run `cargo metadata`")?;
    if !output.status.success() {
        bail!("cargo metadata failed:\n{}", String::from_utf8_lossy(&output.stderr));
    }

    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("failed to parse `cargo metadata` output")?;
    let package = metadata["packages"]
        .as_array()
        .and_then(|packages| packages.first())
        .with_context(|| format!("cargo metadata reported no package for {}", args.manifest_path.display()))?;

    let bin_names: Vec<&str> = package["targets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|target| target["kind"].as_array().is_some_and(|kinds| kinds.iter().any(|k| k == "bin")))
        .filter_map(|target| target["name"].as_str())
        .collect();

    let bin_name = match (args.bin.as_deref(), bin_names.as_slice()) {
        (Some(name), _) => name.to_string(),
        (None, [only]) => (*only).to_string(),
        (None, []) => bail!("crate at {} declares no [[bin]] target", args.manifest_path.display()),
        (None, many) => {
            bail!(
                "crate at {} declares multiple binaries ({}); pick one with --bin",
                args.manifest_path.display(),
                many.join(", ")
            )
        },
    };

    let crate_dir = args
        .manifest_path
        .parent()
        .with_context(|| format!("{} has no parent directory", args.manifest_path.display()))?
        .to_path_buf();

    let target_directory = metadata["target_directory"]
        .as_str()
        .context("cargo metadata reported no target_directory")?;

    Ok(Target {
        bin_name,
        crate_dir,
        run_dir: PathBuf::from(target_directory).join("termoxide-watch"),
    })
}

/// One of two alternating homes for a built binary.
///
/// The running app always executes from a slot copy, so cargo's own output
/// path is never the file a process currently has open — see the module
/// docs for why that matters on Windows. Each successful rebuild copies
/// into [`other`](Self::other), so the app being replaced and the one about
/// to run are never the same file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Blue,
    Green,
}

impl Slot {
    fn other(self) -> Self {
        match self {
            Self::Blue => Self::Green,
            Self::Green => Self::Blue,
        }
    }

    fn path(self, run_dir: &Path, bin_name: &str) -> PathBuf {
        let label = match self {
            Self::Blue => "blue",
            Self::Green => "green",
        };
        run_dir.join(format!("{bin_name}-{label}{}", env::consts::EXE_SUFFIX))
    }
}

/// Build the target binary and return its path, or `None` if the build
/// failed (already reported to stderr — the caller should leave whatever is
/// currently running alone and keep watching).
fn build(manifest_path: &Path, bin_name: &str) -> Result<Option<PathBuf>> {
    let output = Command::new("cargo")
        .args(["build", "--message-format=json-render-diagnostics", "--manifest-path"])
        .arg(manifest_path)
        .args(["--bin", bin_name])
        .output()
        .context("failed to run `cargo build`")?;

    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Ok(message) = serde_json::from_str::<serde_json::Value>(line)
            && let Some(rendered) = message.pointer("/message/rendered").and_then(serde_json::Value::as_str)
        {
            eprint!("{rendered}");
        }
    }

    if !output.status.success() {
        eprintln!("[termoxide-watch] build failed; leaving the running app as-is and watching for the next change");
        return Ok(None);
    }

    let executable = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|message| message["reason"] == "compiler-artifact" && message["target"]["name"] == bin_name)
        .find_map(|message| message["executable"].as_str().map(PathBuf::from))
        .context("cargo build succeeded but reported no executable for the target binary")?;

    Ok(Some(executable))
}

/// A spawned app instance and the sentinel path it was told to watch.
struct RunningApp {
    child: Child,
    sentinel: PathBuf,
}

/// Launch `executable` with inherited stdio, so it takes the real terminal
/// exactly as it would under `cargo run`.
///
/// `state` is `Some` only under `--persist-state`; when set it's forwarded
/// as `TERMOXIDE_RELOAD_STATE`, which is itself the opt-in `termoxide::run_with_app`
/// checks for — omitting the env var entirely (rather than, say, passing it
/// empty) is what keeps a plain session from touching the filesystem for
/// this at all.
fn spawn(executable: &Path, crate_dir: &Path, state: Option<&Path>) -> Result<RunningApp> {
    let sentinel = env::temp_dir().join(format!("termoxide-reload-{}.signal", std::process::id()));
    let _ = fs::remove_file(&sentinel);

    let mut command = Command::new(executable);
    command
        .current_dir(crate_dir)
        .env("TERMOXIDE_RELOAD_SENTINEL", &sentinel)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    if let Some(state) = state {
        command.env("TERMOXIDE_RELOAD_STATE", state);
    }

    let child = command
        .spawn()
        .with_context(|| format!("failed to launch {}", executable.display()))?;

    Ok(RunningApp { child, sentinel })
}

/// Ask the running app to shut down by writing its sentinel file, then wait
/// for it to exit on its own. Falls back to a hard kill after
/// [`SHUTDOWN_TIMEOUT`] — e.g. the binary predates this hook and never
/// learned to look for the sentinel.
fn stop(mut app: RunningApp) -> Result<()> {
    fs::write(&app.sentinel, b"reload").context("failed to write the reload sentinel")?;

    let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
    loop {
        if app.child.try_wait().context("failed to poll the running app")?.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            eprintln!("[termoxide-watch] app did not exit within {SHUTDOWN_TIMEOUT:?}; killing it");
            let _ = app.child.kill();
            let _ = app.child.wait();
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    let _ = fs::remove_file(&app.sentinel);
    Ok(())
}

/// Install a Ctrl-C handler and return the flag it sets.
///
/// The first Ctrl-C just raises the flag, for the main loop to notice at its
/// next chance (see [`wait_for_change`]). A second one means the first
/// didn't get acted on fast enough for the user's patience, so it exits
/// immediately rather than risk hanging forever.
fn install_interrupt_handler() -> Result<Arc<AtomicBool>> {
    let interrupted = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&interrupted);
    ctrlc::set_handler(move || {
        if flag.swap(true, Ordering::SeqCst) {
            eprintln!("[termoxide-watch] second Ctrl-C, forcing immediate exit");
            std::process::exit(130);
        }
        eprintln!("[termoxide-watch] Ctrl-C received, shutting down (press again to force)");
    })
    .context("failed to install Ctrl-C handler")?;
    Ok(interrupted)
}

/// Wait for either the next file-system event or Ctrl-C.
///
/// Returns `true` when a change came in and the caller should rebuild,
/// `false` when it should stop watching instead (Ctrl-C, or the watcher
/// itself was dropped).
fn wait_for_change(rx: &mpsc::Receiver<notify::Result<notify::Event>>, interrupted: &AtomicBool) -> bool {
    loop {
        if interrupted.load(Ordering::SeqCst) {
            return false;
        }
        match rx.recv_timeout(INTERRUPT_POLL) {
            Ok(_) => return true,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return false,
        }
    }
}

/// Build, then copy the result into `slot`. Returns `Ok(None)` when the
/// build itself failed (already reported by [`build`]) rather than erroring,
/// so the caller can leave whatever is currently running alone.
fn build_into_slot(manifest_path: &Path, bin_name: &str, run_dir: &Path, slot: Slot) -> Result<Option<PathBuf>> {
    let Some(built) = build(manifest_path, bin_name)? else {
        return Ok(None);
    };

    let slot_path = slot.path(run_dir, bin_name);
    fs::create_dir_all(run_dir).with_context(|| format!("failed to create {}", run_dir.display()))?;
    fs::copy(&built, &slot_path)
        .with_context(|| format!("failed to copy {} to {}", built.display(), slot_path.display()))?;

    Ok(Some(slot_path))
}

fn main() -> Result<()> {
    let args = parse_args()?;
    let target = resolve_target(&args)?;
    let interrupted = install_interrupt_handler()?;

    eprintln!(
        "[termoxide-watch] watching {} (bin: {})",
        target.crate_dir.display(),
        target.bin_name
    );

    // `state` (not just `args.persist_state`) is what actually gets passed to
    // `spawn`, so state persistence is only ever one flag away from fully off.
    let state = args.persist_state.then(state_path);
    if let Some(state) = &state {
        // A leftover file from an earlier, unrelated watch session (pid
        // reuse, or this one crashed last time) must not leak into a fresh
        // session's first launch — persistence should only ever carry state
        // across reloads *within* one `termoxide-watch` run.
        let _ = fs::remove_file(state);
        eprintln!("[termoxide-watch] state persistence on ({})", state.display());
    }

    let mut slot = Slot::Blue;
    let slot_path = build_into_slot(&args.manifest_path, &target.bin_name, &target.run_dir, slot)?
        .context("initial build failed; fix the error above and re-run termoxide-watch")?;
    let mut running = spawn(&slot_path, &target.crate_dir, state.as_deref())?;

    let (tx, rx) = mpsc::channel();
    // Kept alive for the rest of `main`; dropping it would stop the watch.
    let mut watcher: RecommendedWatcher = notify::recommended_watcher(move |event| {
        let _ = tx.send(event);
    })
    .context("failed to start file watcher")?;
    let src_dir = target.crate_dir.join("src");
    watcher
        .watch(&src_dir, RecursiveMode::Recursive)
        .with_context(|| format!("failed to watch {}", src_dir.display()))?;
    watcher
        .watch(&args.manifest_path, RecursiveMode::NonRecursive)
        .with_context(|| format!("failed to watch {}", args.manifest_path.display()))?;

    while wait_for_change(&rx, &interrupted) {
        // Drain until a quiet period, so a burst of saves collapses into a
        // single rebuild.
        while rx.recv_timeout(DEBOUNCE).is_ok() {}

        eprintln!("[termoxide-watch] change detected, rebuilding...");
        // Builds into the *other* slot while `running` (on the current
        // slot) keeps executing — cargo never touches the file `running`
        // has open, so this is safe even on Windows. Only swap once the
        // new binary is actually ready.
        let next_slot = slot.other();
        if let Some(slot_path) = build_into_slot(&args.manifest_path, &target.bin_name, &target.run_dir, next_slot)? {
            stop(running)?;
            running = spawn(&slot_path, &target.crate_dir, state.as_deref())?;
            slot = next_slot;
            eprintln!("[termoxide-watch] reloaded");
        }
        // `None` means `build` already reported the failure; `running` on
        // the current slot is untouched, so the app never went down.
    }

    eprintln!("[termoxide-watch] stopping the running app and exiting");
    stop(running)?;
    Ok(())
}
