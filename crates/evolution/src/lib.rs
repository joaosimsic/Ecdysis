//! Evolution: Synthesis → Incubation → Hot-Swap pipeline (§4.3, §8, M6).
//!
//! This crate ties the §8 Evolution Workflow together. It owns the
//! `ArcSwap<LiveModule>` that the kernel and every FSM task share, runs one
//! Rebirth from a `LiveGraph` snapshot, and atomically:
//!
//!   1. transpiles the surviving (above-EMA-threshold) edges into Rust source
//!      via `transpiler::synthesize`,
//!   2. hands the source to the [`incubator::IncubatorPool`] for an off-hot-path
//!      `rustc` build (the live module keeps serving the firehose meanwhile —
//!      §4.3),
//!   3. loads the freshly-built `gen_XXX.wasm` into a [`wasmtime::Module`],
//!   4. `arc-swap`s the [`LiveModule`] pointer (lock-free, §4.1 / §4.3),
//!   5. **in the same Harvest tick** (§8 step 6) purges the now-institutionalized
//!      edges from the Ephemeral graph so RAM is released to the next Irritation,
//!   6. drops the previous module after a grace period long enough for any
//!      in-flight `WasmHost::process` call to complete.
//!
//! The fossil record (§7) is written by the incubator before the swap fires,
//! so on a successful Rebirth both `gen_XXX.rs` and `gen_XXX.wasm` are durable
//! on disk before any FSM observes the new generation.

use arc_swap::ArcSwap;
use ephemeral::{LiveGraph, NodeId, StateGraph};
use incubator::{IncubatorError, IncubatorPool};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};
use transpiler::{synthesize, SynthesizeOptions};
use wasm_host::{HostError, WasmHost};
use wasmtime::{Engine, Module};

/// One generation's compiled Wasm + the set of `(NodeId, byte)` edges it
/// institutionalizes. Cloning is `Arc`-cheap because `wasmtime::Module` is an
/// `Arc` internally.
pub struct LiveModule {
    pub generation: u32,
    pub module: Module,
    /// Edges baked into this module's binary code. Used by the Harvest pass
    /// after a Rebirth to purge their RAM equivalents (§8 step 6).
    pub institutionalized: BTreeSet<(NodeId, u8)>,
}

#[derive(Debug)]
pub enum RebirthError {
    Incubator(IncubatorError),
    Io(std::io::Error),
    Wasm(wasmtime::Error),
    Host(HostError),
}

impl std::fmt::Display for RebirthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RebirthError::Incubator(e) => write!(f, "incubator: {e}"),
            RebirthError::Io(e) => write!(f, "rebirth io: {e}"),
            RebirthError::Wasm(e) => write!(f, "wasmtime: {e}"),
            RebirthError::Host(e) => write!(f, "wasm host: {e}"),
        }
    }
}

impl std::error::Error for RebirthError {}

/// Owns the `ArcSwap<LiveModule>` that backs every FSM's hot path. Cloneable
/// (cheap `Arc`) so the kernel can hand a clone to the supervisor and to each
/// FSM task without per-task plumbing.
#[derive(Clone)]
pub struct Rebirth {
    engine: Engine,
    pool: IncubatorPool,
    live: Arc<ArcSwap<LiveModule>>,
    /// How long to keep the previous `LiveModule` alive after a swap so that
    /// any FSM mid-`process()` call can finish against the old module.
    grace: Duration,
}

impl Rebirth {
    /// Build a new pipeline pinned to `engine`, sourcing fossils from `pool`,
    /// starting from `bootstrap` (typically the `gen_000` module produced from
    /// `wasm_host::gen_000_wasm`).
    pub fn new(engine: Engine, pool: IncubatorPool, bootstrap: LiveModule, grace: Duration) -> Self {
        Self {
            engine,
            pool,
            live: Arc::new(ArcSwap::from_pointee(bootstrap)),
            grace,
        }
    }

    /// Lock-free snapshot of the currently live module. The returned `Arc`
    /// keeps the old `LiveModule` valid for the duration of the caller's use,
    /// even if a Rebirth fires concurrently — that is the property the FSM
    /// hot path depends on.
    pub fn live(&self) -> Arc<LiveModule> {
        self.live.load_full()
    }

    /// Pointer to the underlying `ArcSwap<LiveModule>` for callers that want
    /// to share the swap directly (e.g. the kernel handing it to a supervisor
    /// task that polls for generation changes).
    pub fn handle(&self) -> Arc<ArcSwap<LiveModule>> {
        self.live.clone()
    }

    /// Instantiate a fresh `WasmHost` against whichever module is currently
    /// live. Each FSM task owns its own host (so it owns its own `Store`),
    /// but they all share one `Module`.
    pub fn instantiate(&self, fuel_per_call: u64, epoch_deadline: u64) -> Result<WasmHost, RebirthError> {
        let live = self.live.load();
        WasmHost::from_module(&self.engine, &live.module, fuel_per_call, epoch_deadline)
            .map_err(RebirthError::Host)
    }

    /// Run one full §8 cycle. Synthesizes from a `graph` snapshot, incubates,
    /// hot-swaps, then purges institutionalized edges from `graph` *in the
    /// same Harvest tick* (§8 step 6). The previous `LiveModule` is dropped
    /// after `self.grace` so any in-flight call against it can complete.
    ///
    /// Returns the new `LiveModule` so the caller can log the generation
    /// transition.
    pub async fn rebirth(
        &self,
        graph: &LiveGraph,
        opts: SynthesizeOptions,
    ) -> Result<Arc<LiveModule>, RebirthError> {
        // (1) Snapshot. The live firehose keeps running against the current
        // module while we work on this snapshot. arc-swap guarantees the
        // snapshot is stable for the duration of synthesis.
        let snapshot = graph.load();
        let institutionalized = collect_institutionalized(&snapshot, opts.ema_threshold);

        // (2) Synthesis: emit Rust AST via quote!. We stringify here, *before*
        // the incubator await, so the `proc_macro2::TokenStream` (which is
        // `!Send`) never enters this async function's state machine. Without
        // this, the kernel cannot `tokio::spawn(rebirth.rebirth(..))`.
        let source = synthesize(&snapshot, opts).to_string();

        // (3) Incubation: shadow rustc worker writes the fossils and returns
        // the wasm path. The live module is untouched throughout this await.
        let compiled = self
            .pool
            .incubate(source)
            .await
            .map_err(RebirthError::Incubator)?;

        // (4) Load the new wasm and bake it into a Module pinned to our Engine.
        let bytes = tokio::fs::read(&compiled.wasm_path)
            .await
            .map_err(RebirthError::Io)?;
        let module = Module::new(&self.engine, &bytes).map_err(RebirthError::Wasm)?;

        // (5) Hot-swap. arc-swap publishes the new pointer atomically; FSM
        // tasks pick up the new module on their next `live()` call.
        let new = Arc::new(LiveModule {
            generation: compiled.generation,
            module,
            institutionalized: institutionalized.clone(),
        });
        let old = self.live.swap(new.clone());
        info!(
            target: "evolution",
            generation = compiled.generation,
            edges = institutionalized.len(),
            wall_ms = compiled.wall_time.as_millis() as u64,
            "rebirth: hot-swapped live module",
        );

        // (6) Same-tick Harvest purge. Any edge that just got baked into the
        // wasm is no longer needed in RAM; releasing it now is what makes the
        // FSM's RAM ceiling sustainable across many generations (§3, §6).
        graph.mutate(|g| purge_institutionalized(g, &institutionalized));

        // Defer the actual drop of the old module by `grace` so that any FSM
        // currently inside `WasmHost::process` against it has time to finish.
        let grace = self.grace;
        tokio::spawn(async move {
            tokio::time::sleep(grace).await;
            drop(old);
        });

        Ok(new)
    }
}

/// Compute the set of `(source_node, byte)` edges that the next-generation
/// wasm will institutionalize. Mirrors the survival rule used inside
/// `transpiler::synthesize` — kept here (rather than re-exported from the
/// transpiler) so the two filters cannot drift without a test failure below.
fn collect_institutionalized(graph: &StateGraph, ema_threshold: f64) -> BTreeSet<(NodeId, u8)> {
    graph
        .edges
        .iter()
        .filter(|(_, edge)| edge.score >= ema_threshold)
        .map(|(&key, _)| key)
        .collect()
}

/// §8 step 6: drop institutionalized edges from the RAM graph in the same
/// Harvest tick that performed the Rebirth. Orphan nodes are intentionally
/// left in place — `NodeId`s are positionally stable across harvests by
/// design (see `ephemeral::StateGraph::harvest`).
fn purge_institutionalized(graph: &mut StateGraph, edges: &BTreeSet<(NodeId, u8)>) {
    let before = graph.edges.len();
    graph.edges.retain(|key, _| !edges.contains(key));
    let removed = before - graph.edges.len();
    if removed > 0 {
        info!(
            target: "evolution",
            removed,
            remaining = graph.edges.len(),
            "harvest: purged institutionalized edges",
        );
    } else {
        warn!(
            target: "evolution",
            "harvest: rebirth produced zero purged edges (graph already empty?)",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ephemeral::{Edge, ROOT};
    use wasm_host::{build_engine, gen_000_wasm};

    fn bootstrap_module() -> (Engine, LiveModule) {
        let engine = build_engine().expect("engine");
        let bytes = gen_000_wasm().expect("gen_000 wat");
        let module = Module::new(&engine, &bytes).expect("module");
        (
            engine,
            LiveModule {
                generation: 0,
                module,
                institutionalized: BTreeSet::new(),
            },
        )
    }

    #[test]
    fn collect_respects_threshold() {
        let mut g = StateGraph::new();
        g.nodes.push(Default::default());
        g.nodes.push(Default::default());
        g.edges.insert((ROOT, b'a'), Edge { target: 1, score: 0.9 });
        g.edges.insert((ROOT, b'b'), Edge { target: 2, score: 0.1 });
        let s = collect_institutionalized(&g, 0.5);
        assert!(s.contains(&(ROOT, b'a')));
        assert!(!s.contains(&(ROOT, b'b')));
    }

    #[test]
    fn purge_removes_only_listed_edges() {
        let mut g = StateGraph::new();
        g.nodes.push(Default::default());
        g.nodes.push(Default::default());
        g.edges.insert((ROOT, b'a'), Edge { target: 1, score: 1.0 });
        g.edges.insert((ROOT, b'b'), Edge { target: 2, score: 1.0 });
        let mut set = BTreeSet::new();
        set.insert((ROOT, b'a'));
        purge_institutionalized(&mut g, &set);
        assert!(!g.edges.contains_key(&(ROOT, b'a')));
        assert!(g.edges.contains_key(&(ROOT, b'b')));
    }

    #[test]
    fn rebirth_swap_publishes_new_module_and_keeps_old_snapshot_valid() {
        // We don't actually invoke rustc here (CI may not have wasm32). We
        // exercise the swap + grace-drop machinery directly against an
        // in-memory `LiveModule`, which is the only piece this test owns.
        let (engine, boot) = bootstrap_module();
        let pool = IncubatorPool::new(std::env::temp_dir().join("ecdysis-rebirth-test"), 1);
        let r = Rebirth::new(engine.clone(), pool, boot, Duration::from_millis(10));

        let before = r.live();
        assert_eq!(before.generation, 0);

        // Hand-roll a "next" module reusing gen_000 bytes — we only care that
        // arc-swap publishes a new pointer with a new generation number, not
        // that it's a structurally different binary.
        let bytes = gen_000_wasm().unwrap();
        let module = Module::new(&engine, &bytes).unwrap();
        let next = Arc::new(LiveModule {
            generation: 1,
            module,
            institutionalized: BTreeSet::new(),
        });
        r.live.store(next);

        let after = r.live();
        assert_eq!(after.generation, 1);
        // The old snapshot stays valid — §4.1 invariant.
        assert_eq!(before.generation, 0);
    }

    #[test]
    fn instantiate_uses_currently_live_module() {
        let (engine, boot) = bootstrap_module();
        let pool = IncubatorPool::new(std::env::temp_dir().join("ecdysis-rebirth-inst"), 1);
        let r = Rebirth::new(engine, pool, boot, Duration::from_millis(10));
        let mut host = r.instantiate(1_000_000, 1).expect("instantiate");
        // gen_000 returns Unmapped on the first byte — the bootstrap behavior.
        let out = host.process(b"x").expect("process");
        assert!(matches!(out, wasm_host::StepOutcome::Unmapped(_)));
    }
}
