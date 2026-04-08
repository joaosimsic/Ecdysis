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
 
# Ecdysis Axioms

## 1. Environment Axioms

### Axiom 1: Irritation Source
> **The General Rule:** The system must remain cognitively open to a continuous, high-entropy stream of external noise that it cannot fully predict or control.

**Premises:**
* **[P.1]** (Ashby's Law): To survive the complexity of the internet, the FSM must develop internal variety that mathematically matches the environment's variety.
* **[P.2]** (Mechanism): Ingestion of the Mastodon Public Federated Timeline via a non-buffered, lossy WebSocket. Every unmapped byte triggers a mandatory allocation or state transition.
* **[P.3]** (Scope): The system is forbidden from using parsers (JSON/Regex). Only raw byte transitions constitute valid internal operations. 

**Systems Theory Reinforcement:**
* **Concept:** *Irritation*
* **Validation:** Systems do not "receive information"; they are "irritated" by environmental noise. Information is an internal event created when the system chooses to categorize that irritation.

---

## 2. Operational Axioms

### Axiom 2: Operational Closure
> **The General Rule:** Systems interact only through emitted communication; direct inspection or mutation of another system’s internal state is physically impossible.

**Premises:**
* **[P.1]** (Logic): Each FSM is encapsulated in a unique `tokio` task with private heap allocations; no shared pointers are permitted.
* **[P.2]** (Mechanism): Inter-system coupling occurs exclusively through a `tokio::sync::broadcast` channel (The Societal Bus). Systems "sense" others only by the bytes dropped onto the Bus.

**Systems Theory Reinforcement:**
* **Concept:** *Operative Geschlossenheit* (Operational Closure)
* **Validation:** A system consists only of its own operations. It cannot "reach out" into its environment. By enforcing strict memory isolation, the FSM maintains its own specialized logic.

### Axiom 3: The Communicative Unit
> **The General Rule:** The smallest unit of the system is a triple-selection of Information (the byte), Utterance (the transition), and Understanding (the mutation).

**Premises:**
* **[P.1]** (**Information / The Byte**): The arriving noise is not "data" until the system selects it. A byte from the Firehose is a selection of one possibility out of 256, creating a specific "irritation."
* **[P.2]** (**Utterance / The Transition**): The act of moving from `S1` to `S2` is the system "speaking" to itself. It is the formal expression of the irritation within the system’s current logic.
* **[P.3]** (**Understanding / The Mutation**): Understanding is not "comprehension" but **structural change**. When the `ShortcutLearner` or `Incubator` codifies a frequent transition into a permanent Wasm-backed state, the system has "understood" the noise by incorporating it into its future autopoiesis.

**Systems Theory Reinforcement:**
* **Concept:** *Mitteilungssynthese* (Synthesis of Communication)
* **Validation:** Luhmann argues communication only occurs when these three components are synthesized. In Ecdysis, if a byte (Information) triggers a transition (Utterance) that leads to a re-compilation (Understanding), a complete communicative event has occurred. If any piece is missing (e.g., a byte is dropped), the event fails.

---

## 3. Evolutionary Axioms

### Axiom 4: Autopoietic Continuation through Atrophy
> **The General Rule:** The system has no objective other than the recursive production of its own next state, achieved by evolving "blindness" to non-recurring noise.

**Premises:**
* **[P.1]** (Hardware): Survival is defined by the avoidance of "Stagnation Death" (timeouts) and "OOM" (resource exhaustion). Infinite state growth is fatal.
* **[P.2]** (Mechanism): The system survives by "forgetting." The `Harvest` purge and EMA decay (λ) ensure that non-reinforced paths are deleted, while the Transpiler collapses high-frequency paths into optimized Wasm.

**Systems Theory Reinforcement:**
* **Concept:** *Sinn / Autopoiesis* (Meaning and Self-production)
* **Validation:** Meaning is the selection of one possibility over many others. By "Institutionalizing" certain paths and letting others atrophy, the system creates a world-view that allows it to process chaos without collapsing under its own complexity.

---

## 4. Temporal Axioms

### Axiom 5: Historical Path-Dependency
> **The General Rule:** Every current operation is constrained by the totality of the system’s prior communicative history.

**Premises:**
* **[P.1]** (Logic): The system cannot jump to an arbitrary state; it must navigate the graph it has built over time.
* **[P.2]** (Mechanism): The "Fossil Record" (the sequence of generated `.rs` and `.wasm` files) ensures that every "Rebirth" is a mutation of the existing DNA, not a random generation.

**Systems Theory Reinforcement:**
* **Concept:** *Strukturelle Kopplung* (Structural Coupling)
* **Validation:** Systems evolve structures that restrict what they can perceive in the future. The system becomes "locked in" to its own specialized way of seeing the world.

### Axiom 6: Resource Scarcity (The Law of Friction)
> **The General Rule:** Physical hardware constraints are the primary drivers of system differentiation and evolution.

**Premises:**
* **[P.1]** (Hardware): OS thread jitter and L3 cache contention provide the "stochastic spark" needed for identical clones to diverge.
* **[P.2]** (Mechanism): Bounded, lossy buffers ensure that "missing a byte" due to CPU starvation is a permanent, non-recoverable evolutionary event.

**Systems Theory Reinforcement:**
* **Concept:** *Interferenz* (Interference)
* **Validation:** While systems are operationally closed, they exist in a shared physical medium. The friction of the hardware is the medium through which systems "feel" the presence of the environment.

---

## 5. Death and Parasitism

### Axiom 7: The Necessity of Stillbirth
> **The General Rule:** An evolution that exceeds the temporal window of its environment is a failure and must be discarded.

**Premises:**
* **[P.1]** (Logic): If the logic required to process a byte takes longer than the "irritation" arrival rate, the system has lost its structural coupling.
* **[P.2]** (Mechanism): The `Incubator` returns `IncubatorError::Slow` if compilation/optimization exceeds the 25s threshold; the Kernel rejects these modules to prevent "evolutionary lag."

**Systems Theory Reinforcement:**
* **Concept:** *Temporalized Complexity*
* **Validation:** Systems must operate at a speed relative to their environment. A system that "thinks" slower than the noise arrives is effectively dead.

### Axiom 8: Parasitic Structural Coupling
> **The General Rule:** In the absence of primary environmental noise, the system must treat the communicative output of its peers as its own environment.

**Premises:**
* **[P.1]** (Logic): A system cannot exist in a vacuum; if the external Firehose drops, the Societal Bus becomes the new "irritation."
* **[P.2]** (Mechanism): The `Health::BusFeeding` strategy allows FSMs to learn transitions from peer frames (Shortcuts) when the external input stream is `Starved`.

**Systems Theory Reinforcement:**
* **Concept:** *Inter-systemic Interference*
* **Validation:** Social systems emerge from the noise created by other social systems. This justifies the system's ability to "feed" on the outputs of its neighbors to maintain autopoiesis.

### Axiom 9: Metabolic Ceiling
> **The General Rule:** The total complexity of the ecosystem is hard-capped by the physical energy (CPU/RAM) of the host; growth in one system necessitates the starvation of another.

**Premises:**
* **[P.1]** (Hardware): The Linux OOM Killer and the CPU scheduler act as the ultimate "Natural Selection" agents within a fixed cgroup/resource slice.
* **[P.2]** (Mechanism): The `Kernel` implements a global `ResourceQuota`. If System A (the Incubator) spikes in CPU usage to compile a new module, System B (an FSM) will experience increased latency, potentially triggering an Axiom 7 "Stillbirth" or Axiom 6 "Missing Byte."

**Systems Theory Reinforcement:**
* **Concept:** *Medium/Form Distinction*
* **Validation:** The hardware is the *medium* (the loose coupling of possibilities), and the FSM is the *form* (the tight coupling of state). A form cannot exist without a medium. When the medium is exhausted, no further forms can be "imprinted," forcing the system into a competitive evolutionary struggle for physical space.
