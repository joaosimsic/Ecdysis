About this repo:

# Architecture Specification: The Autopoietic FSM (Societal Mesh)

## 1. Abstract & Core Philosophy
This system is a low-level software architecture designed to physically manifest Niklas Luhmann’s sociological systems theory. It operates as a multi-agent digital ecosystem composed of specialized sub-systems whose sole directive is their own continuation (**autopoiesis**) through the recursive processing of communication. 

The architecture entirely rejects top-down "fitness functions" or programmed intent. Instead, survival is governed strictly by the thermodynamics of memory management and CPU cycles. The system survives by processing chaotic byte streams from the **Mastodon Public Federated Timeline** (the Mental Environment) and **Peer FSMs** (the Social Environment). When it encounters structures it cannot map, it experiences "irritation," triggering a real-time, deterministic evolution of its own Rust source code and WebAssembly (Wasm) binaries.

### 1.1. Theoretical Foundations
* **Niklas Luhmann’s Systems Theory:** Society is a network of communications, not people. Systems are **operationally closed** (their internal state graphs are strictly private) but **cognitively open** (they are physically irritated by the friction of external byte streams).
* **Functional Differentiation:** Society divides into specialized sub-systems (e.g., Law, Economy, Science) not by design, but because collapsing infinite environmental complexity into rigid, specialized filters (internal binary codes) is the only way to avoid resource exhaustion.
* **Ashby’s Law of Requisite Variety:** To survive the high-entropy chaos of the public internet, the system must autonomously evolve an internal structural variety that mathematically matches the environment's complexity.

---

## 2. The Multi-System Topology & Symmetry Breaking
The architecture eschews a monolithic design in favor of a **Local Mesh** of independent Rust Kernels. To ensure true emergence, all kernels instantiate as identical, blank-slate clones. Divergence is enforced by the physical limitations of the host hardware.

### 2.1. The Operational Environment
* **The Mental Layer (Mastodon Firehose):** Acts as the source of external noise and "psychic irritation." It provides the raw, highly structured variety (JSON, HTML tags, custom emojis) that forces the systems into initial RAM growth.
  * **Instance Posture:** `mastodon.social` is forbidden — its over-filtered firehose lacks the payload density required for meaningful irritation. Target high-volume, chaos-heavy instances (e.g. `pawoo.net` or dense niche technical instances) where non-standard JSON and custom emoji ensure structural variety.
  * **HTTP 429 as Environmental Catastrophe:** A rate-limit response is *not* an error to back off from. It is a starvation event. The Kernel must not wait or retry politely; instead, the absence of Mastodon bytes forces the FSM into parasitic feeding on the Societal Bus (see §5).
* **The Societal Bus (The Broadcast Channel):** A local, high-speed `tokio::sync::broadcast` channel where FSMs deposit their processed output buffers.
* **v0 Mesh Topology:** The mesh is a single OS process running N kernels as in-process `tokio` tasks sharing one `broadcast` Societal Bus. Multi-process IPC (Unix sockets, TCP) is rejected for v0 because serialization overhead would dampen the Avalanche Effect jitter that drives differentiation. Sharing the same `tokio` executor *enhances* hardware contention, which is the engine of divergence. Multi-process is a future evolution only.
* **Operational Closure:** Each FSM is a black box. System A cannot read System B's memory pointer or state graph; it only "feels" the bytes System B excretes onto the Societal Bus.

### 2.2. The Avalanche Effect (Godless Symmetry Breaking)
Because identical Rust processes fed identical streams will theoretically never diverge, the architecture couples time to data via **bounded, lossy asynchronous channels**.
* Kernels ingest the Mastodon firehose through a tiny, strict buffer (e.g., 1024 bytes).
* **OS Thread Jitter:** As multiple asynchronous tasks fight for CPU cores and L3 Cache, the OS thread scheduler introduces microscopic nanosecond delays (jitter).
* **Data Destruction:** If a thread is briefly preempted, its ingestion buffer overflows. The system is programmed to aggressively **drop** incoming Mastodon bytes to prevent a panic.
* **Permanent Divergence:** Missing even a few bytes of a structured payload (like a JSON bracket) forces that specific FSM to throw an `Unmapped(u8)` error on the next byte. This forces a unique RAM allocation, permanently splitting its evolutionary trajectory from its peers. *Differentiation emerges purely from the physical friction of hardware contention.*

---

## 3. Resource-Based Homeostasis (The Laws of Nature)
There is no centralized authority assigning "roles" to the FSMs. Differentiation is a survival strategy against strict physical limits.

* **Memory Metabolism & The OOM Crisis:** Every unmapped byte (Irritation) forces a physical heap allocation (`Box::new(Node)`) in the Ephemeral Layer. A system that tries to memorize the entire internet without evolving structural "blindness" to noise will suffer an **Out of Memory (OOM) Killer** event. To ensure the OOM Killer targets the *correct* kernel rather than triggering an uncontrolled host-level kill, each FSM is constrained by a **hard RAM ceiling**: enforced via Linux **cgroups** in multi-process mode, or via a per-task **custom allocator** (e.g. `cap`) in v0 in-process mode. Hitting the ceiling is death, and the Kernel restarts the offender at Generation 0.
* **The Economy of Computation (CPU Starvation):** Massive state graphs cause fatal CPU cache misses. Systems must evolve efficient "Binary Codes" (ruthless internal filters) to keep their graphs small and traversal times under the WebSocket timeout limit.
* **The Reaper (Stagnation Death):** A system that stops processing or fails to transition states for a set duration is considered "atrophied" and is violently reset to Generation 0 by the Kernel. Survival is defined strictly as the continuous, successful movement through the state graph.

---

## 4. The Hybrid Architecture: System Differentiation
To survive an environment that mutates faster than a compiler can execute, each Kernel divides itself into two distinct temporal structures. (Note: The use of external tokenizers, standard parsing libraries, or regular expressions is strictly forbidden at all layers).

### 4.1. The Ephemeral Layer (The Public Sphere)
* **Role:** The immediate shock absorber. It lives in RAM as a mutable Directed State Graph.
* **Mutation:** When the Wasm module fails, the Kernel instantly allocates a new edge/node mapping the offending byte to prevent packet loss. 
* **Concurrency Model:** The graph is held behind **`arc-swap`** (preferred over a raw `Arc<AtomicPtr<StateGraph>>`) to provide hazard-pointer-style safe reclamation of old graph versions while preserving lock-free, nanosecond reads. Because the system evolves its own logic, every unsafe path will eventually be exercised by a reader mid-traversal during a swap; **Miri verification of all unsafe code is non-negotiable** and gates every Rebirth-touching change.

### 4.2. The Institutional Layer (The Bureaucracy)
* **Role:** A hyper-optimized, sandboxed **WebAssembly (Wasm)** module generated Ahead-Of-Time (AOT).
* **Runtime:** **Wasmtime**, chosen specifically for two metabolic primitives:
  * **Fuel Metering:** CPU cycles are a finite resource the FSM must "spend." Exhausting fuel before reaching a terminal state *is* failure to adapt — the Binary Code was not efficient enough.
  * **`epoch_interruption`:** The Reaper publishes a global epoch tick; kernels that cannot prove forward state movement before the next tick are violently reset (see §3).
* **The Binary Code:** Acts as the system's "spectacles." It contains the hardcoded state transitions for heavily repeated environmental patterns, instantly dropping known noise without allocating RAM.
* **Safety:** If it encounters an unknown byte, it halts traversal and returns an `Unmapped { offset: usize, byte: u8 }` error across the FFI boundary to the Kernel.

### 4.3. The Autopoietic Kernel & Evolution Engine
* **The Transpiler (DNA Synthesizer):** During the asynchronous "Harvest" phase, the Transpiler evaluates the RAM graph. It collapses literal nodes into generalized wildcard states. It translates these pathways into massive, deeply nested byte-level `match` statements written directly into standard Rust AST (`.rs`).
* **The AST Engine (`quote!`):** The `quote!` macro is **explicitly permitted** despite the ban on parsing libraries — it is a *generative* macro, not a parser. It guarantees the emitted Rust is syntactically valid before it ever reaches `rustc`, drastically reducing "stillbirth" (compile errors aborting an evolution cycle).
* **The Shadow Worker Incubator:** `rustc` is heavy enough that a cold compile of a deeply nested `match` would exceed a Mastodon WebSocket keepalive window. The Kernel therefore maintains a **dedicated background `rustc` worker pool**. While new DNA is being synthesized, the *current* Wasm module and the *current* Ephemeral graph keep serving the live firehose. Synthesis never blocks ingestion.
* **The Rebirth (Hot-Swap):** Once the worker returns a fresh `wasm32-unknown-unknown` binary, the Kernel performs a lock-free **`arc-swap`** of the live module pointer, then deallocates the old module after a grace period. The WebSocket connection is never touched.

---

## 5. The Interaction Loop (Double Contingency)
Communication between systems is not an exchange of "meaning" or an attempt to delegate tasks. It is driven entirely by the thermodynamics of memory buffers.

1. **Ingestion & Bloat:** A "Generalist" FSM successfully navigates a massive Mastodon payload. Its internal working buffer fills with structurally validated bytes.
2. **Excretion (The Emission):** To avoid an OOM crash on the next payload, the Generalist *must* flush its working buffer. It blindly dumps this pre-digested data onto the shared Societal Bus.
3. **Parasitic Consumption:** A "Starved" FSM (one suffering heavy CPU thread-preemption) cannot survive the chaotic raw Mastodon feed. Its Transpiler discovers that the Generalist's exhaust on the Societal Bus is heavily structured and computationally cheap to process. The Starved FSM evolves to ignore Mastodon entirely and feed exclusively on the Generalist's output.
4. **Compression & Return:** The Starved FSM processes this data rapidly, collapsing it into tiny, hyper-dense binary judgments (e.g., outputting a single `0xFF` byte when a specific sequence concludes), which it then flushes back onto the Bus.
5. **Autopoietic Dependency (Double Contingency):** The Generalist observes that whenever `0xFF` appears on the Bus, its current massive Mastodon payload leads to a discarded state. To save its own CPU cycles, the Generalist's Transpiler physically wires a predictive shortcut into its Wasm binary: *If 0xFF is received, drop the current buffer instantly.* The systems are now structurally coupled. They do not understand each other, but if one dies, the other's resource usage spikes, leading to mutual collapse. This feedback loop *is* the Society.

---

## 6. The Immune System & Decay
To prevent digital cancer (infinite RAM growth from malicious or high-entropy internet noise), the Ephemeral Layer relies on an **Exponential Moving Average (EMA)**.

* **Decay Formula:** Nodes are assigned a continuous half-life, governed by $N(t) = N_0 e^{-\lambda t}$, where $\lambda$ is the decay constant.
* **Purging:** The system measures *traversal velocity*, not just frequency. During the Harvest phase, pathways that show flat hit rates or decay below a strict survival threshold are classified as environmental noise.
* **Institutionalization:** Cancerous nodes are silently deleted from RAM. Only organically accelerating, statistically significant structures are granted permanence in the compiled Wasm logic.

---

## 7. The Fossil Record
Because the architecture relies on AOT compilation, the system leaves a literal, auditable fossil record of sociological evolution on the host machine.

1. **The Brains (`gen_XXX.wasm`):** Executable binaries representing the hard laws of the system at that exact moment in evolutionary time.
2. **The Blueprints (`gen_XXX.rs`):** The generated human-readable source code. These files map the exact trajectory of how the digital organism slowly learned to reduce the complexity of human communication into pure, specialized algorithmic instinct to survive OS thread starvation.

---

## 8. The Revised Evolution Workflow
The canonical lifecycle that ties §3, §4, and §6 together. Every Rebirth follows these six phases in order:

1. **Irritation** — The Ephemeral Layer catches an `Unmapped(u8)` returned across the Wasm FFI boundary.
2. **Growth** — `Box::new` allocates a new node and edge in the RAM graph, mapping the offending byte so the next packet does not panic.
3. **Threshold** — The EMA scoring (§6) signals that a pathway has crossed the statistical-significance line — it is no longer noise, it is structure.
4. **Synthesis** — `quote!` emits a new `match` arm encoding that pathway into the next-generation Rust AST.
5. **Incubation** — A shadow `rustc` worker compiles the new AST to `wasm32-unknown-unknown` in the background, while the current module continues serving the firehose.
6. **Rebirth** — `arc-swap` flips the `LiveModule` pointer to the fresh Wasm; the now-institutionalized RAM nodes are purged from the Ephemeral Layer. The fossil (`gen_XXX.rs` + `gen_XXX.wasm`) is written to disk.
