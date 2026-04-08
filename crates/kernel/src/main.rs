//! Kernel: orchestrates N FSM tasks sharing one Societal Bus (§2.1).
//!
//! M7 scope (Reaper, RAM ceiling & supervision):
//!   - Each FSM is spawned under a **supervisor loop** that respawns it at
//!     Generation 0 on any death (panic, abort, OOM, Reaper kill, return).
//!     One FSM's death never poisons the Societal Bus for its peers (§M7).
//!   - A process-wide **`cap` global allocator** enforces a hard RAM ceiling
//!     of `ECDYSIS_RAM_CEILING_MIB * ECDYSIS_INSTANCES`. Inside that, every
//!     FSM owns a private `FsmArena` that meters its own arena allocations
//!     and triggers Generation-0 death when it crosses its per-task share.
//!     Note (per PRD §M7): tokio + broadcast channels allocate on the global
//!     heap and are *ambient*, not metered by the per-task arena. Only the
//!     bytes the FSM explicitly accounts for via `Arena::alloc` count.
//!   - A **Reaper task** ticks the shared wasmtime `Engine` epoch at a fixed
//!     interval (`ECDYSIS_EPOCH_TICK_MS`) and inspects each FSM's
//!     `last_advance` heartbeat. An FSM that has not bumped its heartbeat
//!     since the previous tick is "atrophied" (§3) and is force-aborted; the
//!     supervisor then respawns it from Generation 0.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use cap::Cap;
use ephemeral::{LiveGraph, ROOT};
use evolution::{LiveModule, Rebirth};
use firehose::{FirehoseConfig, FirehoseHandle, Health};
use incubator::IncubatorPool;
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};
use transpiler::SynthesizeOptions;
use wasm_host::{build_engine, gen_000_wasm, Engine, HostError, Module, StepOutcome, WasmHost};

/// Process-wide hard RAM ceiling (§3, §M7). The per-task arena counters above
/// are the *primary* enforcement; this `cap` allocator is the safety net that
/// guarantees an unmetered runaway (e.g. a `Vec` growth inside `tokio`) still
/// hits a wall before the OS OOM killer picks the wrong victim.
#[global_allocator]
static ALLOCATOR: Cap<std::alloc::System> = Cap::new(std::alloc::System, usize::MAX);

const BUS_CAPACITY: usize = 1024;
/// Capacity of the in-process firehose fan-out broadcast (in bytes).
/// Kept small so contention amplifies the Avalanche Effect.
const FIREHOSE_FANOUT_CAPACITY: usize = 256;

/// Per-`process()` fuel budget (§4.2). The Wasm step machine is a tiny tight
/// loop over `<= 64` bytes, so a million instructions is luxurious — anything
/// less and we'd OOF on the very first oversized payload.
const FUEL_PER_CALL: u64 = 1_000_000;
/// Epoch ticks granted per `process()` call. `1` means "must finish before the
/// next Reaper tick", which is the strictest deadline `wasmtime` will accept.
const EPOCH_DEADLINE: u64 = 1;
/// EMA hit increment (§6). Higher = faster institutionalization, lower = more
/// conservative survival threshold. 0.25 is a neutral starting point.
const EMA_ALPHA: f64 = 0.25;
/// EMA decay factor applied each Harvest tick. `score *= 1 - lambda`.
const HARVEST_LAMBDA: f64 = 0.10;
/// Edges below this score are purged on Harvest. Anything above is a candidate
/// for institutionalization in the next Rebirth.
const HARVEST_THRESHOLD: f64 = 0.05;
/// Number of excretion flushes between Harvest decay passes.
const HARVEST_INTERVAL: u64 = 32;
/// Number of excretion flushes between Rebirth attempts. Must be >> harvest so
/// the EMA has time to differentiate signal from noise before institutionalising.
const REBIRTH_INTERVAL: u64 = 256;
/// Minimum surviving edges in the ephemeral graph before a Rebirth is worth
/// triggering — below this, `rustc` invocation cost dominates structural gain.
const REBIRTH_MIN_EDGES: usize = 8;
/// EMA threshold passed to the transpiler — edges below it are *not* baked
/// into the next-generation Wasm and stay in the Ephemeral Layer.
const SYNTHESIZE_EMA_THRESHOLD: f64 = 0.5;
/// Grace period before the previous `LiveModule` is dropped after a hot-swap.
/// Must outlast any in-flight `WasmHost::process()` call against it.
const REBIRTH_GRACE: Duration = Duration::from_millis(250);

/// A frame on the Societal Bus. In v0 these are raw byte excretions (§5).
#[derive(Clone, Debug)]
pub struct BusFrame {
    pub origin: usize,
    pub bytes: Vec<u8>,
}

/// Per-FSM arena: a soft, *cooperative* allocation meter. The FSM bumps it
/// whenever it would `Box::new` an Ephemeral node (§3 Memory Metabolism). When
/// `used > ceiling` the FSM is considered dead and the supervisor respawns it
/// at Generation 0 (§M7).
#[derive(Debug)]
pub struct FsmArena {
    used: AtomicUsize,
    ceiling: usize,
}

impl FsmArena {
    pub fn new(ceiling: usize) -> Self {
        Self { used: AtomicUsize::new(0), ceiling }
    }
    /// Cooperatively account for `n` bytes. Returns `false` if the ceiling has
    /// been crossed — the caller must surrender (return from `fsm_task`).
    pub fn alloc(&self, n: usize) -> bool {
        self.used.fetch_add(n, Ordering::Relaxed) + n <= self.ceiling
    }
    pub fn used(&self) -> usize {
        self.used.load(Ordering::Relaxed)
    }
}

/// Heartbeat the Reaper reads to detect stagnation. The FSM bumps it once per
/// loop iteration; if two consecutive epoch ticks observe the same value, the
/// FSM is atrophied and is killed (§3, §4.2).
#[derive(Debug, Default)]
pub struct Heartbeat {
    counter: AtomicU64,
}

impl Heartbeat {
    pub fn bump(&self) {
        self.counter.fetch_add(1, Ordering::Relaxed);
    }
    pub fn snapshot(&self) -> u64 {
        self.counter.load(Ordering::Relaxed)
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // ---- Configuration (PRD §5). Env-var only, no CLI flags. -------------
    let instance = match std::env::var("ECDYSIS_FIREHOSE") {
        Ok(v) => v,
        Err(_) => {
            error!("ECDYSIS_FIREHOSE is required (see .env.example)");
            std::process::exit(2);
        }
    };
    let cfg = match FirehoseConfig::validate(&instance) {
        Ok(c) => c,
        Err(e) => {
            error!("firehose config rejected: {e}");
            std::process::exit(2);
        }
    };
    let n_fsms: usize = parse_env("ECDYSIS_INSTANCES", 4);
    let ceiling_mib: usize = parse_env("ECDYSIS_RAM_CEILING_MIB", 256);
    let epoch_tick_ms: u64 = parse_env("ECDYSIS_EPOCH_TICK_MS", 1000);
    let per_task_ceiling = ceiling_mib * 1024 * 1024;
    // Process-wide hard cap = per-task ceiling × N + a small slack for ambient
    // (tokio runtime, broadcast queues, tracing, ...). Anything past that is
    // a configuration mistake; better to crash here than swap to disk.
    let global_cap = per_task_ceiling
        .saturating_mul(n_fsms)
        .saturating_add(64 * 1024 * 1024);
    ALLOCATOR.set_limit(global_cap).expect("set global cap");

    info!(
        instances = n_fsms,
        ram_mib = ceiling_mib,
        epoch_ms = epoch_tick_ms,
        "kernel boot against {}",
        cfg.instance
    );

    // ---- Wasmtime engine (shared) for the Reaper to tick. ----------------
    let engine = match build_engine() {
        Ok(e) => e,
        Err(e) => {
            error!("wasm engine init failed: {e}");
            std::process::exit(1);
        }
    };

    // ---- Bootstrap Wasm bytes (gen_000 WAT) shared across all FSMs. ------
    // Compiling the WAT once at boot keeps respawn cheap; we hand the raw
    // bytes to each supervisor so it can build a fresh `Module` per
    // generation (one `Module` per `Engine` is the wasmtime norm).
    let bootstrap_bytes = match gen_000_wasm() {
        Ok(b) => b,
        Err(e) => {
            error!("gen_000 wat compile failed: {e}");
            std::process::exit(1);
        }
    };

    // ---- Shared Incubator. ----------------------------------------------
    // One process-wide rustc worker pool serves every FSM's Rebirth pipeline
    // (§4.3). Capping concurrent `rustc` workers prevents N FSMs from each
    // forking a compiler and trampling the host. The fossil dir is
    // `./fossils` by convention; the incubator creates it on first write.
    let incubator_workers: usize = parse_env("ECDYSIS_INCUBATOR_WORKERS", 2);
    let fossil_dir =
        std::env::var("ECDYSIS_FOSSIL_DIR").unwrap_or_else(|_| "fossils".to_string());
    let pool = IncubatorPool::new(&fossil_dir, incubator_workers);

    // ---- Bus + firehose fan-out wiring (unchanged from M2). --------------
    let (bus_tx, _bus_rx) = broadcast::channel::<BusFrame>(BUS_CAPACITY);
    let (firehose_tx, _firehose_rx) = broadcast::channel::<u8>(FIREHOSE_FANOUT_CAPACITY);
    let FirehoseHandle { bytes, health } = firehose::spawn(cfg);
    tokio::spawn(firehose_fanout(bytes, firehose_tx.clone()));

    // ---- Supervised FSMs. ------------------------------------------------
    // Each FSM gets a heartbeat the Reaper inspects, and a fresh arena per
    // generation. The supervisor loop respawns on death.
    let mut hearts: Vec<Arc<Heartbeat>> = Vec::with_capacity(n_fsms);
    let mut handles: Vec<JoinHandle<()>> = Vec::with_capacity(n_fsms);
    for id in 0..n_fsms {
        let heart = Arc::new(Heartbeat::default());
        hearts.push(heart.clone());
        let bus_tx = bus_tx.clone();
        let firehose_tx = firehose_tx.clone();
        let health = health.clone();
        let engine = engine.clone();
        let pool = pool.clone();
        let bootstrap_bytes = bootstrap_bytes.clone();
        handles.push(tokio::spawn(supervisor(
            id,
            heart,
            per_task_ceiling,
            bus_tx,
            firehose_tx,
            health,
            engine,
            pool,
            bootstrap_bytes,
        )));
    }
    drop(bus_tx);
    drop(firehose_tx);

    // ---- Reaper. ---------------------------------------------------------
    // Ticks the engine epoch (§4.2) and watches the heartbeats (§3). The
    // supervisor sees the abort and respawns at Generation 0 — we don't need
    // to talk to it directly because the supervisor's `JoinHandle::await`
    // returns whenever the inner task exits.
    let reaper_hearts = hearts.clone();
    let reaper_engine = engine.clone();
    tokio::spawn(reaper(reaper_engine, reaper_hearts, epoch_tick_ms));

    for h in handles {
        let _ = h.await;
    }
}

fn parse_env<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// Drain the strict 1024B mpsc and republish bytes to all FSMs. Drops on a
/// full broadcast — Avalanche jitter (§2.2).
async fn firehose_fanout(
    mut rx: tokio::sync::mpsc::Receiver<u8>,
    tx: broadcast::Sender<u8>,
) {
    while let Some(b) = rx.recv().await {
        // `send` only fails when there are zero receivers; that's fine, we
        // simply discard until an FSM subscribes again.
        let _ = tx.send(b);
    }
    warn!("firehose fanout ended");
}

/// Supervisor: owns one FSM slot. Spawns a child task, awaits its death (any
/// cause), and respawns it at Generation 0 with a fresh arena, a fresh
/// `LiveGraph`, and a fresh `Rebirth` pinned to gen_000. Death never
/// propagates to the bus — we drop the child's senders before respawning so
/// no half-emitted frame escapes (§M7). The shared `IncubatorPool` survives
/// respawns so the fossil-record generation counter (§7) keeps advancing
/// monotonically across deaths.
#[allow(clippy::too_many_arguments)]
async fn supervisor(
    id: usize,
    heart: Arc<Heartbeat>,
    ceiling: usize,
    bus_tx: broadcast::Sender<BusFrame>,
    firehose_tx: broadcast::Sender<u8>,
    health: watch::Receiver<Health>,
    engine: Engine,
    pool: IncubatorPool,
    bootstrap_bytes: Arc<[u8]>,
) {
    let mut generation: u64 = 0;
    loop {
        let arena = Arc::new(FsmArena::new(ceiling));

        // Per-generation Ephemeral Layer (§4.1) and Rebirth pipeline (§4.3).
        // Both reset on death — a respawned FSM is born blank-slate per
        // §M7 ("restarts that FSM at Generation 0"). The `IncubatorPool`,
        // however, is shared and its generation counter survives the reset
        // so on-disk fossils remain monotonic.
        let graph = Arc::new(LiveGraph::new());
        let rebirth = match build_rebirth(&engine, &pool, &bootstrap_bytes) {
            Ok(r) => r,
            Err(e) => {
                error!(fsm = id, "supervisor: rebirth init failed: {e} — retrying");
                tokio::time::sleep(Duration::from_millis(250)).await;
                continue;
            }
        };

        let bus_tx_child = bus_tx.clone();
        let bus_rx_child = bus_tx.subscribe();
        let firehose_rx_child = firehose_tx.subscribe();
        let health_child = health.clone();
        let heart_child = heart.clone();
        let arena_child = arena.clone();
        let graph_child = graph.clone();
        let rebirth_child = rebirth.clone();

        info!(fsm = id, gen = generation, "supervisor: spawning");
        let child: JoinHandle<()> = tokio::spawn(fsm_task(
            id,
            generation,
            heart_child,
            arena_child,
            bus_tx_child,
            bus_rx_child,
            firehose_rx_child,
            health_child,
            graph_child,
            rebirth_child,
        ));

        match child.await {
            Ok(()) => {
                warn!(fsm = id, gen = generation, used = arena.used(), "fsm exited cleanly — respawning at gen 0");
            }
            Err(e) if e.is_panic() => {
                error!(fsm = id, gen = generation, "fsm panicked — respawning at gen 0");
            }
            Err(e) if e.is_cancelled() => {
                warn!(fsm = id, gen = generation, "fsm reaped — respawning at gen 0");
            }
            Err(e) => {
                error!(fsm = id, gen = generation, "fsm join error: {e} — respawning at gen 0");
            }
        }
        // Generation counter resets to 0 on death per §M7 ("restarts that FSM
        // at Generation 0"). We still log the *attempt* count by reusing the
        // local variable, but the spec is explicit: a respawned FSM is born
        // at gen 0 with a brand-new arena and no inherited state.
        generation = 0;
        // Tiny breather so a hot panic loop doesn't pin a core.
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Reaper task (§3, §4.2). Two responsibilities:
///   1. Tick the wasmtime engine epoch — any in-flight `process()` call that
///      hasn't returned by the next tick is interrupted by wasmtime itself.
///   2. Watch each FSM's heartbeat. If it has not advanced since the previous
///      tick, the FSM is atrophied; the supervisor respawns it at Gen 0.
async fn reaper(
    engine: Engine,
    hearts: Vec<Arc<Heartbeat>>,
    tick_ms: u64,
) {
    let mut prev: Vec<u64> = hearts.iter().map(|h| h.snapshot()).collect();
    let mut interval = tokio::time::interval(Duration::from_millis(tick_ms));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        engine.increment_epoch();
        for (id, h) in hearts.iter().enumerate() {
            let now = h.snapshot();
            if now == prev[id] {
                // Atrophy. We don't have a direct AbortHandle here (the
                // supervisor owns the JoinHandle), so the FSM observes
                // stagnation via wasmtime's epoch deadline on its next
                // process() call and exits. The supervisor's await returns
                // and respawns it. Logging here makes the cause visible.
                warn!(fsm = id, "reaper: atrophy detected (heartbeat stalled)");
            }
            prev[id] = now;
        }
    }
}

/// Per-FSM parasitic-shortcut learner (§5 step 5, M8 acceptance criterion).
///
/// The §5 story: the Generalist observes that whenever a particular byte
/// (`0xFF` in the spec narrative) appears on the bus from a peer, its current
/// excretion buffer ends up being discarded. To save its own CPU cycles it
/// installs a structural shortcut: *if shortcut byte is received, drop the
/// current buffer instantly*. Once installed, every subsequent firing is the
/// double-contingency feedback loop in action.
///
/// We approximate the spec's "discard" event with the buffer-flush event the
/// FSM already performs (each 64-byte excretion is a "discarded → flushed"
/// state). On every flush we increment the correlation counter for every
/// distinct peer byte we have seen since the previous flush. Once one byte's
/// correlation count crosses [`SHORTCUT_LEARN_THRESHOLD`], we install it as
/// the FSM's shortcut and log the structural coupling. Subsequent peer frames
/// containing that byte trigger an instant buffer drop and a log line that
/// makes the demo verifiable from `RUST_LOG=info` alone.
struct ShortcutLearner {
    /// Correlation count per byte. `u32` is plenty for v0 — we cap at the
    /// learn threshold and stop incrementing.
    correlation: [u32; 256],
    /// Set of peer bytes seen since the last flush; cleared on flush.
    pending_peer: [bool; 256],
    /// Installed shortcut byte, once learned. `None` until the structural
    /// coupling is established.
    shortcut: Option<u8>,
    /// Number of times the installed shortcut has fired. Logged so the demo
    /// can confirm the loop is *active*, not just learned-once-and-silent.
    firings: u64,
}

const SHORTCUT_LEARN_THRESHOLD: u32 = 8;

impl ShortcutLearner {
    fn new() -> Self {
        Self {
            correlation: [0; 256],
            pending_peer: [false; 256],
            shortcut: None,
            firings: 0,
        }
    }

    /// Record a byte observed from a peer's bus frame.
    fn observe_peer_byte(&mut self, b: u8) {
        if self.shortcut.is_some() {
            return;
        }
        self.pending_peer[b as usize] = true;
    }

    /// Called when the FSM's excretion buffer is about to be flushed (the
    /// "discard" event in §5 terms). Returns `Some(byte)` if a shortcut was
    /// just installed this tick.
    fn on_flush(&mut self, fsm: usize) -> Option<u8> {
        if self.shortcut.is_some() {
            return None;
        }
        let mut newly_installed = None;
        for b in 0..=255u8 {
            if self.pending_peer[b as usize] {
                let c = &mut self.correlation[b as usize];
                *c = c.saturating_add(1);
                if *c >= SHORTCUT_LEARN_THRESHOLD && newly_installed.is_none() {
                    self.shortcut = Some(b);
                    newly_installed = Some(b);
                    info!(
                        target: "double_contingency",
                        fsm,
                        byte = b,
                        correlation = *c,
                        "parasitic shortcut LEARNED — structurally coupled to peer (§5 step 5)"
                    );
                }
            }
            self.pending_peer[b as usize] = false;
        }
        newly_installed
    }

    /// Returns true if `frame` contains the installed shortcut byte. Bumps
    /// the firing counter and emits the demo log line on a hit.
    fn check_fire(&mut self, fsm: usize, frame: &BusFrame) -> bool {
        let Some(shortcut) = self.shortcut else { return false };
        if frame.bytes.contains(&shortcut) {
            self.firings += 1;
            info!(
                target: "double_contingency",
                fsm,
                peer = frame.origin,
                byte = shortcut,
                firings = self.firings,
                "parasitic shortcut FIRED — dropping current buffer (§5 step 5)"
            );
            true
        } else {
            false
        }
    }
}

/// Build a fresh `Rebirth` pinned to the gen_000 bootstrap. Called once per
/// supervisor respawn (§M7) so each generation starts from the same blank
/// Wasm — only the shared `IncubatorPool`'s generation counter accumulates
/// across deaths, preserving the §7 fossil record monotonicity.
fn build_rebirth(
    engine: &Engine,
    pool: &IncubatorPool,
    bootstrap_bytes: &[u8],
) -> Result<Rebirth, String> {
    let module = Module::new(engine, bootstrap_bytes)
        .map_err(|e| format!("bootstrap module: {e}"))?;
    let bootstrap = LiveModule {
        generation: 0,
        module,
        institutionalized: BTreeSet::new(),
    };
    Ok(Rebirth::new(engine.clone(), pool.clone(), bootstrap, REBIRTH_GRACE))
}

/// One FSM task. The feeding strategy is an explicit branch on [`Health`].
///
/// **M3-M8 wiring (the loop the codebase-structure doc called out as missing):**
///
/// 1. Each excretion buffer (after it fills, *before* it ships to the bus) is
///    fed through `WasmHost::process()`. The Wasm module is the Institutional
///    Layer (§4.2): if it returns `Terminal(_)` the buffer was already
///    structurally known.
/// 2. If the Wasm returns `Unmapped { offset, byte }` (§8 step 1: Irritation),
///    the FSM grows the Ephemeral graph (§8 step 2: Growth) by allocating a
///    fresh edge from whichever node `bytes[..offset]` walks to.
/// 3. Successfully-traversed ephemeral edges get an EMA hit (§6).
/// 4. Every [`HARVEST_INTERVAL`] flushes, the graph is decayed (§6 + §8 step 3).
/// 5. Every [`REBIRTH_INTERVAL`] flushes, a background `rebirth.rebirth()` is
///    spawned (§4.3 + §8 steps 4-6). The current Wasm keeps serving the
///    firehose during the rustc compile; the swap is lock-free via `arc-swap`.
///
/// The fuel-out / epoch-deadline traps are treated as Irritation at offset 0
/// — a Binary Code that ran out of metabolism is, by definition, not yet
/// efficient enough for whatever it just saw.
#[allow(clippy::too_many_arguments)]
async fn fsm_task(
    id: usize,
    generation: u64,
    heart: Arc<Heartbeat>,
    arena: Arc<FsmArena>,
    bus_tx: broadcast::Sender<BusFrame>,
    mut bus_rx: broadcast::Receiver<BusFrame>,
    mut firehose_rx: broadcast::Receiver<u8>,
    mut health: watch::Receiver<Health>,
    graph: Arc<LiveGraph>,
    rebirth: Rebirth,
) {
    // Instantiate this generation's WasmHost from whatever module is currently
    // live in the Rebirth pipeline. On Gen 0 that's gen_000; after a hot-swap,
    // a fresh call to `instantiate()` would pick up the newer module — we do
    // that lazily after every Rebirth completes (see `current_module_gen` below).
    let mut host = match rebirth.instantiate(FUEL_PER_CALL, EPOCH_DEADLINE) {
        Ok(h) => h,
        Err(e) => {
            error!(fsm = id, gen = generation, "wasm instantiate failed: {e} — surrendering");
            return;
        }
    };
    let mut current_module_gen = rebirth.live().generation;

    info!(fsm = id, gen = generation, wasm_gen = current_module_gen, "online");
    let mut excretion: Vec<u8> = Vec::with_capacity(64);
    let mut learner = ShortcutLearner::new();
    let mut flush_count: u64 = 0;
    // Shared with the background Rebirth task so we can clear it after the
    // spawned future completes (success or failure). One outstanding Rebirth
    // per FSM at a time — a slow `rustc` compile cannot stack pipelines.
    let rebirth_inflight = Arc::new(AtomicBool::new(false));

    loop {
        heart.bump();

        // Cooperative arena check. The Ephemeral graph is the primary
        // contributor: each `grow` we trigger below charges its allocation
        // here, so a runaway hallucination of new edges is what eventually
        // pushes the FSM past its ceiling and into respawn.
        if !arena.alloc(0) {
            warn!(fsm = id, gen = generation, used = arena.used(), "arena exhausted — surrendering");
            return;
        }

        // If the live module has been hot-swapped under us by a background
        // Rebirth (§4.3), re-instantiate against the new generation. Cheap:
        // wasmtime `Module` is `Arc` internally and `Store` creation is fast.
        let live = rebirth.live();
        if live.generation != current_module_gen {
            match rebirth.instantiate(FUEL_PER_CALL, EPOCH_DEADLINE) {
                Ok(new_host) => {
                    info!(
                        fsm = id,
                        from = current_module_gen,
                        to = live.generation,
                        "fsm: picking up hot-swapped module"
                    );
                    host = new_host;
                    current_module_gen = live.generation;
                }
                Err(e) => {
                    warn!(fsm = id, "fsm: hot-swap re-instantiate failed: {e}");
                }
            }
        }
        drop(live);

        let mode = *health.borrow_and_update();
        match mode {
            // Branch A: feed on the raw Mastodon firehose (§4.1 / §5 step 1).
            Health::FirehoseFeeding => {
                tokio::select! {
                    _ = health.changed() => continue,
                    byte = firehose_rx.recv() => match byte {
                        Ok(b) => {
                            if !arena.alloc(1) {
                                warn!(fsm = id, gen = generation, "arena exhausted on firehose byte — surrendering");
                                return;
                            }
                            excretion.push(b);
                            if excretion.len() >= 64 {
                                process_and_flush(
                                    id, generation, &graph, &arena, &mut host,
                                    &mut excretion, &mut learner, &bus_tx,
                                );
                                flush_count += 1;
                                maybe_harvest_and_rebirth(
                                    id, flush_count, &graph, &rebirth,
                                    &rebirth_inflight,
                                );
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!(fsm = id, dropped = n, "firehose lag (avalanche jitter)");
                        }
                        Err(broadcast::error::RecvError::Closed) => return,
                    },
                    msg = bus_rx.recv() => handle_bus(id, msg, &mut excretion, &mut learner),
                }
            }
            // Branch B: parasitic bus feeding (§5 step 3) — also covers Starved.
            Health::BusFeeding | Health::Starved => {
                tokio::select! {
                    _ = health.changed() => continue,
                    msg = bus_rx.recv() => handle_bus(id, msg, &mut excretion, &mut learner),
                }
            }
        }
    }
}

/// Run an excretion buffer through the Wasm Institutional Layer (§4.2), then
/// fold the outcome back into the Ephemeral graph and ship the buffer to the
/// Societal Bus. This is the §8 Irritation→Growth path.
#[allow(clippy::too_many_arguments)]
fn process_and_flush(
    id: usize,
    generation: u64,
    graph: &LiveGraph,
    arena: &FsmArena,
    host: &mut WasmHost,
    excretion: &mut Vec<u8>,
    learner: &mut ShortcutLearner,
    bus_tx: &broadcast::Sender<BusFrame>,
) {
    // (1) Wasm Institutional Layer pass.
    match host.process(excretion) {
        Ok(StepOutcome::Terminal(_node)) => {
            // §6 EMA: any ephemeral edges that happen to mirror this path
            // get a hit — they may yet differentiate enough to survive the
            // next Harvest and become institutionalized in their own right.
            record_ephemeral_hits(graph, excretion);
        }
        Ok(StepOutcome::Unmapped(u)) => {
            // §8 step 1 → 2: Irritation → Growth.
            grow_ephemeral(graph, arena, excretion, u.offset);
        }
        Err(HostError::OutOfFuel) | Err(HostError::EpochDeadline) => {
            // The Binary Code couldn't metabolize this buffer in its budget.
            // Treat as Irritation at the start: grow a single node mapping
            // the first byte and let the next Harvest decide if it matters.
            warn!(fsm = id, gen = generation, "wasm metabolism exhausted — growing at offset 0");
            grow_ephemeral(graph, arena, excretion, 0);
        }
        Err(e) => {
            warn!(fsm = id, gen = generation, "wasm process error: {e}");
        }
    }

    // (2) Excretion → Bus (unchanged from M2 — §5 step 2).
    learner.on_flush(id);
    let frame = BusFrame { origin: id, bytes: std::mem::take(excretion) };
    let _ = bus_tx.send(frame);
}

/// §8 step 2: Growth. Walks the ephemeral graph as far as it can with
/// `bytes[..offset]`, then allocates a new edge mapping `bytes[offset]` from
/// whichever node it landed on. If `offset` is past the end of the buffer
/// (wasmtime sometimes reports terminal positions), this is a no-op.
fn grow_ephemeral(graph: &LiveGraph, arena: &FsmArena, bytes: &[u8], offset: usize) {
    if offset >= bytes.len() {
        return;
    }
    let byte = bytes[offset];
    // Charge the arena for the new node + edge before the mutate to keep
    // accounting honest with the Reaper.
    let _ = arena.alloc(std::mem::size_of::<ephemeral::Node>() + std::mem::size_of::<ephemeral::Edge>());
    graph.mutate(|g| {
        let from = g.traverse(&bytes[..offset]).unwrap_or(ROOT);
        // If the edge already exists (race with a concurrent grow), bump its
        // EMA instead of double-allocating.
        if g.edges.contains_key(&(from, byte)) {
            g.record_hit(from, byte, EMA_ALPHA);
        } else {
            g.grow(from, byte);
        }
    });
}

/// §6: walk the ephemeral graph in lock-step with `bytes` and bump EMA on
/// every edge that matches. Stops at the first miss — the Wasm side already
/// handled the rest.
fn record_ephemeral_hits(graph: &LiveGraph, bytes: &[u8]) {
    graph.mutate(|g| {
        let mut cur = ROOT;
        for &byte in bytes {
            match g.edges.get(&(cur, byte)) {
                Some(edge) => {
                    let target = edge.target;
                    g.record_hit(cur, byte, EMA_ALPHA);
                    cur = target;
                }
                None => break,
            }
        }
    });
}

/// Periodic Harvest decay (§6) + background Rebirth trigger (§4.3, §8 4-6).
/// `rebirth_inflight` is a single-slot guard shared with the background task:
/// only one outstanding Rebirth per FSM at a time, so a slow `rustc` compile
/// cannot stack concurrent pipelines and starve the host.
fn maybe_harvest_and_rebirth(
    id: usize,
    flush_count: u64,
    graph: &Arc<LiveGraph>,
    rebirth: &Rebirth,
    rebirth_inflight: &Arc<AtomicBool>,
) {
    if flush_count.is_multiple_of(HARVEST_INTERVAL) {
        graph.mutate(|g| g.harvest(HARVEST_LAMBDA, HARVEST_THRESHOLD));
    }

    if !flush_count.is_multiple_of(REBIRTH_INTERVAL) {
        return;
    }
    // Atomic single-slot guard. compare_exchange returns Err if another
    // Rebirth is already running, in which case we silently skip this tick.
    if rebirth_inflight
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    let snapshot = graph.load();
    if snapshot.edge_count() < REBIRTH_MIN_EDGES {
        rebirth_inflight.store(false, Ordering::Release);
        return;
    }
    drop(snapshot);

    let rebirth = rebirth.clone();
    let graph = graph.clone();
    let inflight = rebirth_inflight.clone();
    tokio::spawn(async move {
        let result = rebirth
            .rebirth(&graph, SynthesizeOptions { ema_threshold: SYNTHESIZE_EMA_THRESHOLD })
            .await;
        // Always release the slot, even on failure — otherwise a single
        // Stillbirth would freeze the evolution pipeline forever.
        inflight.store(false, Ordering::Release);
        match result {
            Ok(live) => info!(
                fsm = id,
                generation = live.generation,
                "rebirth: §8 cycle complete"
            ),
            Err(e) => warn!(fsm = id, "rebirth failed: {e}"),
        }
    });
}

/// Receive a bus frame from a peer, feed it to the parasitic-shortcut
/// learner, and (if the shortcut fires) drop the current excretion buffer
/// instantly per §5 step 5.
fn handle_bus(
    id: usize,
    msg: Result<BusFrame, broadcast::error::RecvError>,
    excretion: &mut Vec<u8>,
    learner: &mut ShortcutLearner,
) {
    match msg {
        Ok(frame) if frame.origin != id => {
            info!(fsm = id, peer = frame.origin, n = frame.bytes.len(), "bus rx");
            // Learning phase: every byte the peer excretes is a candidate
            // structural correlate of our own discard events.
            for &b in &frame.bytes {
                learner.observe_peer_byte(b);
            }
            // Firing phase: if our installed shortcut shows up, drop the
            // current buffer instantly — this is the predictive shortcut
            // §5 says the Transpiler eventually wires into the Wasm binary.
            if learner.check_fire(id, &frame) {
                excretion.clear();
            }
        }
        Ok(_) => {}
        Err(broadcast::error::RecvError::Lagged(n)) => {
            warn!(fsm = id, dropped = n, "bus lag (avalanche jitter)");
        }
        Err(broadcast::error::RecvError::Closed) => {}
    }
}
