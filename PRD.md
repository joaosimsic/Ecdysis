# Implementation Plan: Autopoietic FSM (Societal Mesh) v0

## Context
The repo currently contains only `CLAUDE.md` (the architecture spec). This PRD operationalizes that spec into a buildable v0: a single-process Rust mesh of N kernels that ingest the Mastodon firehose, mutate a RAM state graph on `Unmapped` bytes, and periodically self-recompile to Wasm via a shadow `rustc` worker. The acceptance bar is a working end-to-end demonstration of the six-phase Evolution Workflow (§8): Irritation → Growth → Threshold → Synthesis → Incubation → Rebirth.

---

## 1. Workspace Layout
Single Cargo workspace at repo root.

```
Ecdysis/
├── Cargo.toml                # [workspace]
├── crates/
│   ├── kernel/               # bin: spawns N FSM tasks + Societal Bus (orchestration only)
│   ├── evolution/            # Rebirth lifecycle (Synthesis→Incubation→Hot-Swap), supervision tree
│   ├── ephemeral/            # RAM state graph + arc-swap + EMA decay
│   ├── transpiler/           # graph → Rust AST via `quote!` (with match-flattening)
│   ├── incubator/            # background rustc worker pool
│   ├── wasm-host/            # wasmtime runtime (fuel + epoch_interruption)
│   ├── firehose/             # Mastodon WSS client, lossy 1024B buffer
│   └── fsm-runtime/          # crate compiled into each gen_XXX.wasm
└── fossils/                  # gen_XXX.rs + gen_XXX.wasm output dir
```

Each crate maps 1:1 to a numbered section of `CLAUDE.md` so contributors can navigate by §.

## 2. Key Dependencies
- `tokio` (rt-multi-thread, sync, macros) — executor + `broadcast`
- `arc-swap` — §4.1 lock-free graph swap, §4.3 LiveModule swap
- `wasmtime` — fuel metering + `epoch_interruption`
- `quote`, `proc-macro2` — §4.3 generative AST (NOT `syn`, no parsing)
- `cap` — per-task allocator for v0 RAM ceiling (§3)
- `tokio-tungstenite` + `rustls` — Mastodon WSS firehose
- **Forbidden**: `serde_json`, `regex`, `nom`, any tokenizer/parser/lexer (§4)

## 3. Milestones

### M1 — Skeleton & Societal Bus
- Workspace, crates, CI (`cargo check`, `cargo miri test` on `ephemeral`).
- `kernel` spawns N=4 dummy FSM tasks sharing one `tokio::sync::broadcast::channel`.
- Each task logs its task id; verifies bus wiring.

### M2 — Firehose Ingestion (Mental Layer)
- `firehose` crate: WSS connect to a chaos-heavy instance (config: `pawoo.net`; **block `mastodon.social`** at config-validation time).
- **Strict 1024B bounded mpsc** between socket reader and FSM consumer; on overflow → drop (no backpressure). This is the Avalanche source (§2.2).
- HTTP 429 handler does **not** retry; instead emits a `Starvation` signal that flips the FSM to bus-only feeding (§5).
- `firehose` exposes a `Health` enum (`FirehoseFeeding | BusFeeding | Starved`) consumed by the kernel as an explicit strategy toggle — the two feeding modes are first-class branches in the FSM loop, not implicit fallbacks.

### M3 — Ephemeral Layer (RAM Graph)
- `StateGraph { nodes: Vec<Node>, edges: HashMap<(NodeId,u8), NodeId> }` behind `ArcSwap<StateGraph>`.
- Hot path = lock-free read; mutation = clone-modify-swap.
- Byte-level traversal returns `Result<NodeId, Unmapped { offset, byte }>`.
- EMA scoring per edge: `score = α*hit + (1-α)*score_prev`, decayed on each Harvest tick (§6).
- **All `unsafe` paths gated by `cargo miri test`** in CI (§4.1 hard requirement).

### M4 — Wasm Host (Institutional Layer)
- `wasm-host` loads a `gen_XXX.wasm` via wasmtime with:
  - `Config::consume_fuel(true)` — per-payload fuel budget.
  - `Config::epoch_interruption(true)` — Reaper publishes ticks from kernel.
- FFI: `process(ptr, len) -> i64` where negative = `Unmapped` packed as `(offset<<8)|byte`.
- Bootstrap `gen_000.wasm`: a trivial module that returns `Unmapped { 0, byte0 }` for every input — guarantees the first packet triggers Growth.

### M5 — Transpiler & Incubator
- `transpiler::synthesize(&StateGraph) -> proc_macro2::TokenStream` emits byte `match` arms covering every edge above EMA threshold; wildcards collapse low-entropy branches.
- **Match-Flattening (mandatory)**: never emit a single monolithic `match` for the whole graph. Partition the graph into sub-regions and emit one `fn step_NNNN(byte) -> NodeId` per region (or a `static LUT: [u16; 256]` for dense low-irritation sectors) so `rustc`/LLVM optimization stays linear in graph size. Cap per-function arm count (e.g. ≤256) and recurse via tail calls.
- `incubator`: a `tokio::task` worker pool that writes `fossils/gen_XXX.rs`, shells out to `rustc --target wasm32-unknown-unknown -O`, returns the `.wasm` path. Compile wall-time is logged per generation; regressions past the WSS keepalive window fail the build.
- The live module keeps serving while incubation runs (§4.3).

### M6 — Rebirth Hot-Swap
- Kernel holds `ArcSwap<LiveModule>`; on incubator success, `store(new)`; old module dropped after a grace period (`tokio::time::sleep` long enough for in-flight calls).
- Institutionalized nodes are purged from the Ephemeral graph in the same Harvest tick (§8 step 6).
- Persist `gen_XXX.rs` + `gen_XXX.wasm` to `fossils/` (§7).

### M7 — Reaper, RAM Ceiling & Supervision
- Per-task `cap` allocator with hard ceiling (config, e.g. 256 MiB). Hitting it = death → kernel restarts that FSM at Generation 0.
- **Supervision Tree**: the kernel owns each FSM as a supervised child task. On `cap` trigger, OOM, panic, or Reaper kill, the supervisor (a) aborts the offending `tokio` task, (b) drains its `broadcast` sender so peers see no poisoned frames, (c) re-spawns a fresh Generation-0 FSM with a new allocator arena. One FSM's death must never propagate to peers on the Societal Bus.
- Note: `tokio` and `broadcast` channels allocate on the global heap; the per-task `cap` only meters the FSM's own arena. Document which allocations are metered vs. ambient so contributors don't expect `cap` to catch every byte.
- Reaper task publishes wasmtime epoch ticks at fixed interval; FSMs that don't advance state between ticks are reset (§3).

### M8 — Double Contingency Demo
- Run N=4 kernels for ≥10 min against pawoo.net.
- Verify in logs: at least one FSM evolves a `0xFF`-style shortcut that fires on bus input from a peer (§5 step 5). This is the v0 acceptance criterion.

## 4. Critical Files (to be created)
- `crates/kernel/src/main.rs` — task spawn, bus wiring, reaper loop, supervision tree (orchestration only; no Rebirth logic).
- `crates/evolution/src/lib.rs` — Synthesis → Incubation → Hot-Swap pipeline, fossil writer.
- `crates/ephemeral/src/graph.rs` — `StateGraph`, `ArcSwap`, traversal, EMA.
- `crates/transpiler/src/lib.rs` — `synthesize()` using `quote!`.
- `crates/incubator/src/lib.rs` — rustc worker pool.
- `crates/wasm-host/src/lib.rs` — wasmtime config, FFI, hot-swap.
- `crates/firehose/src/lib.rs` — WSS + lossy bounded channel + 429 handler.
- `crates/fsm-runtime/src/lib.rs` — minimal `gen_000` template.

## 5. Verification
- **Unit**: `cargo test -p ephemeral` covers traversal + EMA decay math.
- **Miri**: `cargo +nightly miri test -p ephemeral` — gates every PR touching unsafe / `arc-swap` paths.
- **Integration**: `cargo run -p kernel -- --instances 4 --firehose pawoo.net` and assert (a) `fossils/gen_001.wasm` appears within ~2 min, (b) RSS for each kernel stays below the `cap` ceiling, (c) at least one parasitic shortcut is logged (M8 criterion).
- **Negative**: starting kernel with `--firehose mastodon.social` must exit non-zero at config validation.

## 6. Out of Scope for v0
- Multi-process IPC (Unix sockets/TCP) — explicitly deferred per §2.1.
- Linux cgroups RAM enforcement — `cap` allocator suffices for v0.
- Any GUI / observability dashboard beyond `tracing` logs.
