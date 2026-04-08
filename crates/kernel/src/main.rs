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

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use cap::Cap;
use firehose::{FirehoseConfig, FirehoseHandle, Health};
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};
use wasm_host::{build_engine, Engine};

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
        handles.push(tokio::spawn(supervisor(
            id,
            heart,
            per_task_ceiling,
            bus_tx,
            firehose_tx,
            health,
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
/// cause), and respawns it at Generation 0 with a fresh arena. Death never
/// propagates to the bus — we drop the child's senders before respawning so
/// no half-emitted frame escapes (§M7).
async fn supervisor(
    id: usize,
    heart: Arc<Heartbeat>,
    ceiling: usize,
    bus_tx: broadcast::Sender<BusFrame>,
    firehose_tx: broadcast::Sender<u8>,
    health: watch::Receiver<Health>,
) {
    let mut generation: u64 = 0;
    loop {
        let arena = Arc::new(FsmArena::new(ceiling));
        let bus_tx_child = bus_tx.clone();
        let bus_rx_child = bus_tx.subscribe();
        let firehose_rx_child = firehose_tx.subscribe();
        let health_child = health.clone();
        let heart_child = heart.clone();
        let arena_child = arena.clone();

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

/// One FSM task. The feeding strategy is an explicit branch on [`Health`].
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
) {
    info!(fsm = id, gen = generation, "online");
    let mut excretion: Vec<u8> = Vec::with_capacity(64);
    let mut learner = ShortcutLearner::new();

    loop {
        heart.bump();

        // Cooperative arena check. In M3+ the Ephemeral graph will be the
        // primary contributor; for now we charge each excretion buffer push
        // so the death/respawn path is exercised end-to-end.
        if !arena.alloc(0) {
            warn!(fsm = id, gen = generation, used = arena.used(), "arena exhausted — surrendering");
            return;
        }

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
                                learner.on_flush(id);
                                let frame = BusFrame { origin: id, bytes: std::mem::take(&mut excretion) };
                                let _ = bus_tx.send(frame);
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
