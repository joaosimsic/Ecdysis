# Axiom Structure

### Axiom [N]: [Title]
> **The General Rule:** [One-sentence imperative or law governing the system behavior.]

**Premises:**
* **[P.1]** (Hardware/Logic): [The technical or physical constraint that makes this necessary.]
* **[P.2]** (Mechanism): [How the software specifically enforces this constraint.]

**Systems Theory Reinforcement:**
* **Concept:** *[Luhmann’s Specific Term]*
* **Validation:** [Brief explanation of how the software's behavior mirrors the sociological theory of operational closure, differentiation, or autopoiesis.]

---

# Axiom Structure: Ecdysis Systems

### Axiom 1: Managed Irritation
> **The General Rule:** The system must remain cognitively open to high-entropy external noise while filtering for pattern-gestalts to prevent structural drowning.

**Premises:**
* **[P.1]** (Logic): Raw entropy (single bytes) provides no variety for autopoiesis; only patterns (sequences) constitute information.
* **[P.2]** (Mechanism): A sliding-window pre-processor groups WebSocket noise into $n$-grams; sequences below a probability threshold are discarded before triggering FSM transitions.

**Systems Theory Reinforcement:**
* **Concept:** *Irritation / Resonance*
* **Validation:** Systems are not "open" to everything; they only resonate with noise they can categorize. Filtering ensures the system is irritated, not destroyed.

---

### Axiom 2: The Communicative Triple-Selection
> **The General Rule:** Every systemic event must synthesize Information (the pattern), Utterance (the transition), and Understanding (the mutation).

**Premises:**
* **[P.1]** (Logic): Processing data without structural change is not communication, but mere throughput.
* **[P.2]** (Mechanism): Every FSM event must return a triple-result: the identified $n$-gram, the state-path taken, and the resulting update to the transition weight (EMA).

**Systems Theory Reinforcement:**
* **Concept:** *Mitteilungssynthese*
* **Validation:** Communication is a three-part selection process. If the "Understanding" (structural change) is missing, the communicative event is void.

---

### Axiom 3: Tiered Autopoietic Speed
> **The General Rule:** The system must bifurcate its evolution into immediate phenotypic reactions and asynchronous genotypic codification.

**Premises:**
* **[P.1]** (Hardware): Real-time noise arrives in milliseconds; Rust/Wasm compilation takes seconds.
* **[P.2]** (Mechanism): Initial "Understanding" happens in a fast, interpreted heap-graph; high-frequency paths are then offloaded to the `Incubator` for background Wasm transpilation.

---

### Axiom 4: The Law of Reentry
> **The General Rule:** The system must distinguish between irritations originating from the environment and those originating from its own operations.

**Premises:**
* **[P.1]** (Logic): Failure to distinguish self-observation from environment leads to stagnant consensus and "hallucinated" state loops.
* **[P.2]** (Mechanism): Transitions are tagged with a `SourceBit`. Internal feedback (the Societal Bus) is processed with a higher decay rate ($\lambda$) to ensure environmental noise remains the primary driver.

**Systems Theory Reinforcement:**
* **Concept:** *Re-entry*
* **Validation:** The distinction between system and environment is re-introduced into the system itself, allowing for self-reflective evolution.

---

### Axiom 5: Metabolic Selection (The Governor)
> **The General Rule:** The ecosystem must proactively starve inefficient or stagnant forms to maintain the physical viability of the host.

**Premises:**
* **[P.1]** (Hardware): Fixed cgroup resources (L3 cache, RAM) make growth a zero-sum game between FSMs.
* **[P.2]** (Mechanism): The `Kernel` monitors a "Meaning-to-Energy" ratio; systems with high memory usage but low "Understanding" counts are throttled or terminated.

---

### Axiom 6: Functional Atrophy
> **The General Rule:** The system survives by choosing what to forget; non-reinforced paths must be purged to free resources for new complexity.

**Premises:**
* **[P.1]** (Logic): In a finite system, forgetting is the prerequisite for learning.
* **[P.2]** (Mechanism): The `Harvest` routine deletes any state path not utilized within a temporal window $N$, reclaiming RAM for the `Incubator`.

**Systems Theory Reinforcement:**
* **Concept:** *Reduktion von Komplexität*
* **Validation:** Systems maintain a world-view by selecting what is relevant and ignoring the rest. Atrophy is the physical manifestation of this selection.

---

### Axiom 7: Stillbirth Rejection
> **The General Rule:** Any structural change that fails to codify within the temporal window of its environment must be discarded as evolutionary lag.

**Premises:**
* **[P.1]** (Logic): Stale optimization is more dangerous than no optimization.
* **[P.2]** (Mechanism): The `Kernel` rejects Wasm modules from the `Incubator` if the compile time exceeds the current "Environmental Shift Rate."

---

### Axiom 8: Operational Closure (The Wall of Silence)
> **The General Rule:** A system consists only of its own operations; it cannot "reach out" to mutate or inspect the internal state of another system.

**Premises:**
* **[P.1]** (Hardware/Logic): Direct memory access across tasks leads to race conditions and breaks the mathematical proof of the FSM's current state.
* **[P.2]** (Mechanism): Each FSM is encapsulated in a `tokio` task with strictly private heap allocations. Inter-system communication is restricted to the `Societal Bus` (broadcast channel), where systems only see "emitted artifacts," never the "emitting logic."

**Systems Theory Reinforcement:**
* **Concept:** *Operative Geschlossenheit*
* **Validation:** Systems are "closed" because they only respond to their own internal states. By isolating memory, the FSM is forced to rely on its own interpretation of the Bus, rather than "cheating" via shared state.

---

### Axiom 9: Historical Path-Dependency (The Fossil Record)
> **The General Rule:** Every current state transition is constrained by the totality of the system’s prior communicative history; there is no "Clean Slate."

**Premises:**
* **[P.1]** (Logic): A system without a history cannot develop specialized variety; it remains a generic processor.
* **[P.2]** (Mechanism): The "Fossil Record" (generated `.rs` and `.wasm` files) ensures that every "Rebirth" or "Reboot" of an FSM is a mutation of existing code. The system must navigate the graph it has already built.

**Systems Theory Reinforcement:**
* **Concept:** *Strukturelle Kopplung (Structural Coupling)*
* **Validation:** Evolved structures restrict what the system can perceive in the future. The system becomes "locked in" to its own specialized way of seeing the world.

---

### Axiom 10: The Law of Friction (Hardware as Medium)
> **The General Rule:** Physical hardware constraints (L3 cache contention, CPU jitter) are the primary drivers of system differentiation.

**Premises:**
* **[P.1]** (Hardware): OS thread jitter and cache misses provide the "stochastic spark" needed for identical clones to diverge in their timing and perception.
* **[P.2]** (Mechanism): Bounded, lossy buffers ensure that "missing a byte" due to CPU starvation is a permanent, non-recoverable evolutionary event that forces the system to adapt to its physical limits.

**Systems Theory Reinforcement:**
* **Concept:** *Medium / Form*
* **Validation:** Hardware is the *medium* (loosely coupled) and the FSM is the *form* (tightly coupled). The friction of the medium is what prevents the system from becoming a perfect, stagnant mathematical abstraction.

---

### Axiom 11: Parasitic Structural Coupling
> **The General Rule:** In the absence of external environmental noise, the system treats the communicative output of its peers as its primary environment.

**Premises:**
* **[P.1]** (Logic): Autopoiesis cannot pause; if the external firehose drops, the system must find irritation elsewhere to avoid cessation.
* **[P.2]** (Mechanism): The `Health::BusFeeding` strategy allows FSMs to switch focus to the `Societal Bus`. To prevent "Consensus Hallucination," Axiom 4 (Reentry) is intensified during these periods to ensure peer-noise is treated as highly suspicious.

**Systems Theory Reinforcement:**
* **Concept:** *Inter-systemic Interference*
* **Validation:** Social systems emerge from the noise created by other systems. This justifies "feeding" on neighbor outputs to maintain the "noise floor" required for the system to keep moving.
