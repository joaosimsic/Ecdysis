//! Incubator (§4.3, M5): shadow `rustc` worker pool.
//!
//! The incubator takes a `TokenStream` from `transpiler::synthesize`, writes
//! it to `fossils/gen_XXX.rs`, and shells out to `rustc` to produce
//! `fossils/gen_XXX.wasm`. The live module keeps serving the firehose while
//! incubation runs (§4.3) — that property is provided by the *caller*
//! (kernel/evolution); this crate just guarantees that compilation happens
//! off the hot path on a bounded `tokio` worker pool and that wall time is
//! observed and reported per generation.
//!
//! ## ABI / `rustc` invocation
//!
//! The transpiler emits a complete `#![no_std] #![no_main]` cdylib crate
//! root. We compile it with a single direct `rustc` invocation — no Cargo,
//! no build script, no temp project — because:
//!
//! 1. Spawning Cargo per generation would dwarf the WSS keepalive budget
//!    that §4.3 demands we stay under.
//! 2. The generated source has zero external dependencies, so Cargo's only
//!    contribution would be overhead.
//!
//! `rustc` is invoked as:
//!
//! ```text
//! rustc --edition=2021 --crate-type=cdylib \
//!       --target=wasm32-unknown-unknown \
//!       -C opt-level=3 -C lto=off -C panic=abort \
//!       -C link-arg=--export=memory \
//!       -o gen_XXX.wasm gen_XXX.rs
//! ```
//!
//! The `--export=memory` link arg defends against rustc/lld versions that do
//! not export linear memory by default; the `wasm-host` ABI requires it.
//!
//! ## Stillbirth budget
//!
//! Per the PRD, "regressions past the WSS keepalive window fail the build".
//! v0 enforces this softly: every compile is timed and logged via `tracing`,
//! and `IncubatorPool::compile_budget` (default 25s, well under any
//! reasonable Mastodon WSS keepalive) is returned as `IncubatorError::Slow`
//! when exceeded — *after* the artifact is still produced, so callers can
//! decide whether to swap or to abort. We do not silently kill `rustc`; a
//! killed compile is the worst possible Stillbirth.

use proc_macro2::TokenStream;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tracing::{info, warn};

#[derive(Debug)]
pub enum IncubatorError {
    Io(std::io::Error),
    /// `rustc` exited non-zero. Stderr is captured for the kernel's log.
    Rustc { status: i32, stderr: String },
    /// Compile produced an artifact, but wall time exceeded `compile_budget`.
    /// The wasm path is still returned so callers can choose to use it.
    Slow { wasm: PathBuf, elapsed: Duration },
}

impl std::fmt::Display for IncubatorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IncubatorError::Io(e) => write!(f, "incubator io: {e}"),
            IncubatorError::Rustc { status, stderr } => {
                write!(f, "rustc exited {status}: {stderr}")
            }
            IncubatorError::Slow { elapsed, .. } => {
                write!(f, "rustc wall time {elapsed:?} exceeded budget")
            }
        }
    }
}

impl std::error::Error for IncubatorError {}

impl From<std::io::Error> for IncubatorError {
    fn from(e: std::io::Error) -> Self { IncubatorError::Io(e) }
}

/// One successfully incubated generation. Both fossils (§7) live on disk
/// before this is returned, so the caller can hot-swap and then drop the
/// old `WasmHost` knowing the new artifact is durable.
#[derive(Debug, Clone)]
pub struct CompiledGen {
    pub generation: u32,
    pub rs_path: PathBuf,
    pub wasm_path: PathBuf,
    pub wall_time: Duration,
}

/// Bounded worker pool for `rustc`. Cheap to clone — internally an `Arc`
/// around a `Semaphore` and a generation counter, so the kernel can hand a
/// clone to every FSM task without per-task plumbing.
#[derive(Clone)]
pub struct IncubatorPool {
    inner: Arc<Inner>,
}

struct Inner {
    fossil_dir: PathBuf,
    generation: AtomicU32,
    permits: Arc<Semaphore>,
    compile_budget: Duration,
    rustc: PathBuf,
}

impl IncubatorPool {
    /// `max_workers` caps concurrent `rustc` invocations across the whole
    /// kernel. `rustc` is heavy enough that letting N FSMs each spawn their
    /// own would thrash the host long before the wasm modules ever run.
    pub fn new(fossil_dir: impl Into<PathBuf>, max_workers: usize) -> Self {
        Self {
            inner: Arc::new(Inner {
                fossil_dir: fossil_dir.into(),
                generation: AtomicU32::new(0),
                permits: Arc::new(Semaphore::new(max_workers.max(1))),
                compile_budget: Duration::from_secs(25),
                rustc: PathBuf::from("rustc"),
            }),
        }
    }

    /// Override the per-compile soft budget. Exceeding it produces
    /// `IncubatorError::Slow` *after* the artifact has been written.
    pub fn with_compile_budget(mut self, budget: Duration) -> Self {
        Arc::get_mut(&mut self.inner)
            .expect("with_compile_budget called on shared pool")
            .compile_budget = budget;
        self
    }

    /// Override the rustc binary path (useful for tests / sandboxed CI).
    pub fn with_rustc(mut self, rustc: impl Into<PathBuf>) -> Self {
        Arc::get_mut(&mut self.inner)
            .expect("with_rustc called on shared pool")
            .rustc = rustc.into();
        self
    }

    pub fn fossil_dir(&self) -> &Path { &self.inner.fossil_dir }

    /// Reserve the next generation number. Each call returns a fresh,
    /// monotonically increasing `u32`. Wraps after `u32::MAX` generations,
    /// which v0 will never reach.
    pub fn next_generation(&self) -> u32 {
        self.inner.generation.fetch_add(1, Ordering::Relaxed)
    }

    /// Incubate one generation: write `gen_XXX.rs`, await a worker permit,
    /// run `rustc`, return the compiled artifact's path. The caller (the
    /// evolution pipeline) is responsible for the `arc-swap` of the live
    /// module — see §4.3 / M6.
    pub async fn incubate(&self, source: TokenStream) -> Result<CompiledGen, IncubatorError> {
        let generation = self.next_generation();
        let stem = format!("gen_{:03}", generation);
        let rs_path = self.inner.fossil_dir.join(format!("{stem}.rs"));
        let wasm_path = self.inner.fossil_dir.join(format!("{stem}.wasm"));

        tokio::fs::create_dir_all(&self.inner.fossil_dir).await?;
        // Format: `TokenStream::to_string` produces a single line. That's
        // ugly but valid Rust; rustc doesn't care, and the fossil is meant
        // for forensic reading, not aesthetic appreciation. We add a
        // trailing newline so editors don't complain.
        let mut text = source.to_string();
        text.push('\n');
        tokio::fs::write(&rs_path, text).await?;

        let permit = self
            .inner
            .permits
            .clone()
            .acquire_owned()

            .await
            .expect("incubator semaphore closed");

        let rustc = self.inner.rustc.clone();
        let rs_path_for_task = rs_path.clone();
        let wasm_path_for_task = wasm_path.clone();
        let budget = self.inner.compile_budget;

        // Block_in_place would also work, but spawning isolates the rustc
        // child from the runtime worker so the FSM tasks keep advancing.
        let join = tokio::task::spawn_blocking(move || -> Result<Duration, IncubatorError> {
            let start = Instant::now();
            let output = std::process::Command::new(&rustc)
                .arg("--edition=2021")
                .arg("--crate-type=cdylib")
                .arg("--target=wasm32-unknown-unknown")
                .arg("-C").arg("opt-level=3")
                .arg("-C").arg("lto=off")
                .arg("-C").arg("panic=abort")
                .arg("-C").arg("link-arg=--export=memory")
                .arg("-o").arg(&wasm_path_for_task)
                .arg(&rs_path_for_task)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()?;
            let elapsed = start.elapsed();
            if !output.status.success() {
                return Err(IncubatorError::Rustc {
                    status: output.status.code().unwrap_or(-1),
                    stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                });
            }
            Ok(elapsed)
        });

        let elapsed = match join.await {
            Ok(Ok(d)) => d,
            Ok(Err(e)) => {
                drop(permit);
                return Err(e);
            }
            Err(join_err) => {
                drop(permit);
                return Err(IncubatorError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("incubator worker panicked: {join_err}"),
                )));
            }
        };
        drop(permit);

        info!(
            target: "incubator",
            generation,
            wall_ms = elapsed.as_millis() as u64,
            wasm = %wasm_path.display(),
            "incubated generation",
        );

        if elapsed > budget {
            warn!(
                target: "incubator",
                generation,
                wall_ms = elapsed.as_millis() as u64,
                budget_ms = budget.as_millis() as u64,
                "rustc wall time exceeded budget — Rebirth would risk WSS keepalive",
            );
            return Err(IncubatorError::Slow { wasm: wasm_path, elapsed });
        }

        Ok(CompiledGen {
            generation,
            rs_path,
            wasm_path,
            wall_time: elapsed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proc_macro2::TokenStream;
    use std::str::FromStr;

    #[tokio::test]
    async fn missing_rustc_surfaces_io_error() {
        let dir = std::env::temp_dir().join(format!("ecdysis-incubator-{}", std::process::id()));
        let pool = IncubatorPool::new(&dir, 1)
            .with_rustc("/definitely/not/a/real/rustc/binary");
        let src = TokenStream::from_str("fn main() {}").unwrap();
        let err = pool.incubate(src).await.unwrap_err();
        assert!(matches!(err, IncubatorError::Io(_)), "got {err:?}");
        // The .rs file should still have been written before the rustc spawn
        // failed — the fossil record is append-only by design (§7).
        assert!(dir.join("gen_000.rs").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn rustc_failure_is_classified() {
        // Use /bin/false as a stand-in for "rustc that always fails".
        // This keeps the test fast and removes any dependency on the host
        // having a wasm32 toolchain installed.
        if !std::path::Path::new("/bin/false").exists() {
            return;
        }
        let dir = std::env::temp_dir()
            .join(format!("ecdysis-incubator-fail-{}", std::process::id()));
        let pool = IncubatorPool::new(&dir, 1).with_rustc("/bin/false");
        let src = TokenStream::from_str("fn main() {}").unwrap();
        let err = pool.incubate(src).await.unwrap_err();
        assert!(matches!(err, IncubatorError::Rustc { .. }), "got {err:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn generations_are_monotonic() {
        let dir = std::env::temp_dir()
            .join(format!("ecdysis-incubator-gen-{}", std::process::id()));
        let pool = IncubatorPool::new(&dir, 1);
        assert_eq!(pool.next_generation(), 0);
        assert_eq!(pool.next_generation(), 1);
        assert_eq!(pool.next_generation(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
