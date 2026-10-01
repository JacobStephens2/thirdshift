# Design It Twice

To explore alternative interfaces for a chosen deepening candidate, use this parallel sub-agent pattern. Based on "Design It Twice" (Ousterhout): your first idea is unlikely to be the best.

Uses the vocabulary in [SKILL.md](SKILL.md): **module**, **interface**, **seam**, **adapter**, **leverage**.

## Process

### 1. Frame the problem space

Before spawning sub-agents, write down the problem space for the chosen candidate:

- The constraints any new interface would need to satisfy
- The dependencies it would rely on, and which category they fall into (see [DEEPENING.md](DEEPENING.md))
- A rough illustrative code sketch to ground the constraints, not a proposal, just a way to make the constraints concrete

Nobody is there to read it, so don't wait: proceed to Step 2. It is what you compare the designs against in Step 3.

### 2. Spawn sub-agents

Spawn 3+ sub-agents in parallel. Each must produce a **radically different** interface for the deepened module.

Prompt each sub-agent with a separate technical brief (file paths, coupling details, dependency category from [DEEPENING.md](DEEPENING.md), what sits behind the seam). The brief is independent of the problem-space explanation in Step 1. Give each agent a different design constraint:

- Agent 1: "Minimize the interface: aim for 1–3 entry points max. Maximise leverage per entry point."
- Agent 2: "Maximise flexibility: support many use cases and extension."
- Agent 3: "Optimise for the most common caller: make the default case trivial."
- Agent 4 (if applicable): "Design around ports & adapters for cross-seam dependencies."

Include both [SKILL.md](SKILL.md) vocabulary and CONTEXT.md vocabulary in the brief so each sub-agent names things consistently with the architecture language and the project's domain language.

Each sub-agent outputs:

1. Interface (types, methods, params, plus invariants, ordering, error modes)
2. Usage example showing how callers use it
3. What the implementation hides behind the seam
4. Dependency strategy and adapters (see [DEEPENING.md](DEEPENING.md))
5. Trade-offs: where leverage is high, where it's thin

### 3. Compare and decide

Compare the designs in prose. Contrast by **depth** (leverage at the interface), **locality** (where change concentrates), and **seam placement**.

After comparing, decide: which design is strongest and why. If elements from different designs would combine well, take a hybrid. Be opinionated: the choice is yours to make, and nobody picks from a menu. Record the decision and its reason, with the designs you turned down and why.
