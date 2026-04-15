# Ecdysis

## I. Invariants: The Physics of the System
These axioms represent the non-negotiable constraints of the substrate. They define the "laws of nature" for every organism.

### Axiom 1: Operational Closure (The Wall)
> **The General Rule:** A system consists only of its own operations; it cannot inspect or mutate the internal state of another system.

**Premises:**
* **[P.1]** (Hardware/Logic): Direct memory access across tasks leads to race conditions and invalidates the mathematical proof of an FSM's state.
* **[P.2]** (Mechanism): Each FSM is encapsulated in a `tokio` task with private heap allocations. Inter-system communication is restricted to the `Societal Bus` (broadcast channel).

**Systems Theory Reinforcement:**
* **Concept:** *Operative Geschlossenheit*
* **Validation:** Systems are "closed" because they only respond to their own internal state-transitions. By isolating memory, the FSM is forced to **interpret** the Bus rather than "cheating" via shared variables.

### Axiom 2: Metabolic Scarcity (The Governor)
> **The General Rule:** The ecology must proactively starve inefficient forms to maintain the physical viability of the host.

**Premises:**
* **[P.1]** (Hardware): Fixed cgroup resources (L3 cache, RAM) make computational growth a zero-sum game between FSMs.
* **[P.2]** (Mechanism): The `Kernel` monitors a **Meaning-to-Energy (MtE)** ratio. Systems with high resource consumption but low structural mutation counts are throttled or terminated.

**Systems Theory Reinforcement:**
* **Concept:** *Autopoiesis / Resource Scarcity*
* **Validation:** Real systems do not have infinite energy. Differentiation occurs because it is more efficient to be specialized than to be a resource-heavy generalist.

### Axiom 3: Stochastic Friction (The Spark)
> **The General Rule:** Physical hardware constraints are the primary drivers of system differentiation.

**Premises:**
* **[P.1]** (Hardware): OS thread jitter and L3 cache misses provide the noise needed for identical clones to diverge.
* **[P.2]** (Mechanism): Bounded, lossy buffers ensure that "missing a byte" due to CPU starvation is a permanent, non-recoverable evolutionary event.

**Systems Theory Reinforcement:**
* **Concept:** *Medium / Form*
* **Validation:** Hardware is the *medium* (loosely coupled) and the FSM is the *form* (tightly coupled). The friction of the medium prevents the system from becoming a stagnant mathematical abstraction.

---

## II. Emergent Strategies: The Biology of the Organism
The organism (the FSM) uses these rules to navigate the Physics defined above.

### Axiom 4: Functional Atrophy
> **The General Rule:** The system survives by choosing what to forget; non-reinforced paths must be purged to free resources for new complexity.

**Premises:**
* **[P.1]** (Logic): In a finite system, forgetting is the prerequisite for learning.
* **[P.2]** (Mechanism): The `Harvest` routine deletes any state path not utilized within temporal window $N$, reclaiming RAM to maintain a high **MtE** ratio.

### Axiom 5: Autonomous Fission (The Ecdysis)
> **The General Rule:** When internal complexity threatens metabolic viability, the system must bifurcate into specialized subsystems.

**Premises:**
* **[P.1]** (Logic): A monolith eventually becomes too slow to respond to the environment, failing Axiom 2.
* **[P.2]** (Mechanism): FSMs may trigger a "Split" where a high-frequency subgraph is offloaded to the `Incubator` to become a new, autonomous daughter FSM.

---

## III. Convergence Metrics: The Sociology of the Ecology
We define the system as "Luhmannian" if the following parameters are observed in the aggregate behavior of the FSMs on the `Societal Bus`.



| Parameter | Metric | Convergence Goal |
| :--- | :--- | :--- |
| **Functional Differentiation** | *Niche Variance* | Systems stop competing for the same raw $n$-grams and start specializing in unique pattern-gestalts. |
| **Operational Resonance** | *Bus/Environment Ratio* | Systems begin to trigger state-changes based on peer-signals more efficiently than raw environmental noise. |
| **Centrality Dissolution** | *Graph Topology* | The network remains polycentric; no single system becomes the "Master Router" or sovereign observer. |

---

### Direct Answer to the MtE Ratio Calculation
To answer your follow-up question regarding the **Meaning-to-Energy (MtE)** ratio:

The `Kernel` should prioritize the **magnitude of EMA (Exponential Moving Average) weight changes** rather than simple transition frequency. 

**Why?** High transition frequency without weight change is just "looping" or "idling"—it’s a system running in circles without learning. Significant weight change (mutation) indicates that the system has encountered an **Irritation** it found meaningful enough to alter its internal structure. This is the true mark of **Understanding** in Luhmann’s triple-selection.

**MtE Formula Proposal:**
$$\text{MtE} = \frac{\sum |\Delta \text{Weights}|}{\text{CPU Time} \times \text{Memory Footprint}}$$

This formula punishes stagnant "vampire" systems and rewards those that are actively evolving their state-paths to match the environmental entropy.
