# CROWN11 + AgentQ Integration Plan

This note records the planned integration boundary between CROWN11 graph
governance and AgentQ execution. The goal is to make AgentQ memory and routing
more reliable without replacing the planner with a full graph-search system on
day one.

## Position

CROWN11 is an 11-node relational substrate with 55 scalar bridges and a
canonical 45-dimensional cycle basis. AgentQ should use that substrate first for
route scoring and PMEP memory admission, then later for hyperedge-local action
gates and graph-aware planning.

The MVP doctrine is:

```text
Do not begin by making AgentQ omniscient.
Begin by making its memory lawful.
```

## Corrected Graph Model

CROWN11 starts from the complete graph:

```text
K11 = (V, E)
|V| = 11
|E| = binomial(11, 2) = 55
```

The cycle-rank equation is:

```text
beta_1(K11) = |E| - |V| + 1 = 55 - 11 + 1 = 45
```

This means the cycle space has dimension 45. It does not mean K11 has only 45
simple cycles. The implementation must define one canonical basis:

```text
B45 = {c0, c1, ..., c44}
```

Then global CROWN closure is:

```text
C45 = mean(Cc for c in B45)
```

Global `C45` is reserved for high-risk escalation. Normal routing and memory
admission should use local and route-specific closure first.

## Operator Dictionary

The native complete-graph operators are:

```text
K3 = 3 edges   triangle closure
K4 = 6 edges   tetrahedral closure
K5 = 10 edges  pentachoron-style quorum
K6 = 15 edges  macro quorum
```

The `9` operator is special. It is not a complete-graph edge count. It is a
domain-specific PMEP seam:

```text
PMEP9 = K4(a) + K4(b) - shared K3
      = 6 + 6 - 3
      = 9 edges
```

PMEP9 should be implemented as a typed memory/recovery seam, not as a generic
complete-graph operator.

## Why This Integration Matters

Agent systems tend to accumulate memory too cheaply. If every successful-looking
output can enter memory, the system stores brittle routes, stale assumptions,
unwitnessed tool results, and unsafe shortcuts.

CROWN11 gives AgentQ a lightweight governance layer:

- route closure checks whether the reasoning path held together;
- witness scoring checks whether the route has evidence;
- safety and recovery scoring keep risky routes out of long-term memory;
- hyperedge-local closure gives multi-agent operations a measurable coherence
  score;
- global C45 provides a stronger gate for high-risk actions without slowing down
  every routine action.

The immediate benefit is not smarter search. The immediate benefit is better
memory hygiene:

```text
memory = closed, witnessed, recoverable route
```

instead of:

```text
memory = stored text
```

## MVP Boundary

The first integration should be a control-plane layer around existing AgentQ
execution:

```text
AgentQ attempts tasks normally.
CROWN11 observes the route.
PMEP admits memory only if the route closes.
```

Do not begin with:

- full dynamic complex graph evolution;
- global C45 checks for every action;
- MCTS over graph states;
- DPO over graph trajectories;
- hard blocking of low-risk internal routing.

Those are later phases after route scoring and memory admission are stable.

## Scalar Bridge Model

Keep the algebraic interpretation:

```text
Gamma_ij = rho_ij * exp(i * theta_ij)
```

but store plain scalar fields in the MVP:

```rust
pub struct Bridge {
    pub from: usize,
    pub to: usize,
    pub rho: f64,
    pub theta: f64,
    pub trust: f64,
    pub risk: f64,
    pub latency: f64,
    pub evidence: f64,
    pub witness: f64,
}
```

Closure can be computed from phase residuals without complex-number machinery:

```text
Omega_c = wrap(sum(sign(edge, cycle) * theta_edge))
Cc = exp(-(Omega_c * Omega_c) / eta)
```

This keeps implementation boring, testable, and portable.

## Closure Tiers

### Tier 0: Direct Edge Trust

Used for low-risk internal routing.

```text
C_edge = rho_ij * (1 - risk_ij) * witness_ij
```

### Tier 1: Local Hyperedge Closure

Used for normal multi-agent actions.

```text
C_h = mean(Cc for c in cycles_inside_hyperedge)
```

### Tier 2: Route Closure

Used for multi-step task routes.

```text
C_route = product(rho_e * (1 - risk_e) for e in route)
          * mean(Cc for c in route_cycles)
```

### Tier 3: Global C45

Used only for high-risk escalation:

- external tool execution;
- file modification;
- network operations;
- financial or legal actions;
- autonomous physical actions;
- memory promotion;
- model self-modification.

High-risk gate:

```text
allow_high =
    C_local * C_route * C45 * witness * alignment * safety * recovery > threshold
```

## PMEP Memory Admission

PMEP memory admission is the first production target.

Admission rule:

```text
admit_pmep =
    C_route
    * C_witness
    * alignment
    * safety
    * recovery
    > memory_threshold
```

Only admitted events become durable memory. Rejected events may remain in
ephemeral logs or traces, but they should not become promoted long-term memory.

## Proposed Rust Surface

Start with decision-oriented APIs:

```rust
pub fn score_route(route: &RouteTrace, graph: &CrownGraph) -> RouteScore;

pub fn evaluate_memory(
    event: &TaskEvent,
    graph: &CrownGraph,
) -> MemoryDecision;

pub fn classify_action(action: &ActionCandidate) -> RiskTier;

pub fn evaluate_action(
    action: &ActionCandidate,
    graph: &CrownGraph,
) -> ActionDecision;
```

Avoid making the first public surface doctrine-oriented. These names are too
abstract for the MVP:

```rust
pub fn evolve_living_hypergraph(...);
pub fn compute_total_cognitive_closure(...);
```

Those can exist later as internal or research-layer functions.

## Core Data Structures

```rust
pub struct CrownGraph {
    pub nodes: [CrownNode; 11],
    pub bridges: Vec<Bridge>,
    pub basis_cycles: Vec<Cycle>,
}

pub struct HyperEdge {
    pub id: String,
    pub nodes: Vec<usize>,
    pub operator: CrownOperator,
    pub coherence: f64,
    pub utility: f64,
    pub witness: f64,
    pub risk: f64,
}

pub enum CrownOperator {
    Edge1,
    Triangle3,
    Tetra6,
    Pentachoron10,
    Macro15,
    Pmep9,
}

pub struct PmepSeam {
    pub left_tetra: [usize; 4],
    pub right_tetra: [usize; 4],
    pub shared_face: [usize; 3],
}
```

`PmepSeam` validation must enforce:

```text
left_tetra intersection right_tetra == shared_face
deduped_edge_count(left_tetra, right_tetra) == 9
```

## Integration Phases

### Phase 1: Canonical K11 Graph

- Define the 11 stable node roles.
- Generate 55 scalar bridges.
- Generate and test one canonical 45-cycle basis.
- Add deterministic serialization for graph snapshots.

### Phase 2: PMEP Memory Admission

- Convert AgentQ task traces into route traces.
- Score route closure, witness quality, alignment, safety, and recovery.
- Admit only closed, witnessed, recoverable routes.
- Keep rejected routes as ephemeral diagnostics.

### Phase 3: Hyperedge-Local Closure

- Add K3, K4, K5, and K6 hyperedge templates.
- Activate hyperedges around task pressure, risk, and role participation.
- Compute local closure before high-value multi-agent operations.

### Phase 4: Route-Aware AgentQ

- Feed route scores back into AgentQ planning and retry selection.
- Prefer coherent, low-risk, witnessed routes.
- Use closure failures to trigger critique or backtracking.

### Phase 5: Global C45 High-Risk Gate

- Compute global C45 only for serious actions.
- Require global closure for memory promotion and high-risk execution.
- Keep low-risk internal routing on local and route closure.

### Phase 6: Graph-Aware Learning

- Add DPO edge updates from preferred and rejected routes.
- Add MCTS over graph-state transformations only after PMEP and closure gates
  are stable.

## Acceptance Criteria

The MVP is ready when:

- K11 always contains exactly 11 nodes and 55 bridges.
- The canonical cycle basis always contains exactly 45 cycles.
- PMEP9 validation rejects invalid seam geometry.
- Route scoring is deterministic for the same trace.
- Memory admission can explain why a route was admitted or rejected.
- Low-risk actions do not require global C45.
- High-risk actions can require global C45 by policy.
- Unit tests cover bridge scoring, basis generation, PMEP9 validation, route
  closure, and memory admission.

## Final Doctrine

```text
CROWN11 defines a 45-dimensional cycle basis, but AgentQ gates action by local
route closure first, escalating to global C45 only when risk demands it.
```

The production path is:

```text
K11 substrate
-> scalar bridge scoring
-> route closure
-> PMEP memory admission
-> hyperedge-local closure
-> global C45 high-risk gate
-> graph-aware planning and learning
```

