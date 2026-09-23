# LMML Sovereign Engineering and Distributed Intelligence Fabric

**Document status:** Unified implementation blueprint  
**Target:** LMML engineering, agent, automation, knowledge and distributed-runtime architecture  
**Primary integrations:** Graphify, ECC, n8n-mcp, n8n, Claude Code, Codex, OpenCode, EXO, llama.cpp, Ollama, Pulsar, Colibri, AgentQ and LMML node infrastructure  
**EXO source baseline:** v1.0.71, commit `fd707de`  
**Date:** 5 August 2026  
**Architecture doctrine:** Models, harnesses, workflow engines and distributed runtimes may propose, calculate and execute within granted cells; LMML alone governs identity, context, capability, evidence, approval and irreversible commit authority.

---

# 1. Executive decision

LMML should evolve into a **harness-neutral, model-neutral and runtime-neutral sovereign engineering control plane**.

It should not replace Claude Code, Codex, OpenCode, Graphify, ECC, n8n or EXO. It should place each of them inside a clear layer with a bounded responsibility:

| Component | Authoritative role inside LMML |
|---|---|
| **LMML Control Plane** | Identity, tasks, policy, capability grants, approvals, audit and desired state |
| **AgentQ** | Task decomposition, role assignment, worker selection and multi-stage orchestration |
| **Coding harnesses** | Interactive or autonomous repository work inside isolated workspaces |
| **Graphify** | Structural repository truth, relationships and change-impact evidence |
| **Semantic RAG** | Fuzzy retrieval across documentation, code explanations and project knowledge |
| **ECC compatibility layer** | Reusable engineering skills, review disciplines, handoffs and learning patterns |
| **LMML Memory** | Episodic task memory and bounded handoffs, always marked as unreviewed context |
| **n8n-mcp** | n8n node intelligence, workflow compilation, validation, repair and development operations |
| **n8n runtime** | Durable triggers, schedules, webhooks, integrations, retries and human-in-the-loop workflows |
| **Runtime Broker** | Selection among llama.cpp, Ollama, Pulsar, Colibri, EXO, local clusters and approved cloud providers |
| **Residency / Transport Engine** | Explicit model weight residency, loading, streaming, sharding, cache and placement policy |
| **EXO** | Model-level distribution, shard placement and distributed inference inside an approved execution cell |
| **LMML nodes** | Inference, retrieval, tools, validation, storage and specialised distributed workloads |
| **ASTRA-style Authority Gate** | Deterministic verification of any high-consequence transition or irreversible action |
| **Evidence Ledger** | Patches, tests, graph impact, workflow validation, benchmarks, approvals and execution lineage |

The architecture therefore separates five fundamentally different scheduling problems:

```text
AgentQ schedules tasks, roles and agent work.
Coding harnesses schedule their internal reasoning and tool loop.
n8n schedules durable events and integration steps.
The LMML Runtime Broker selects a model endpoint and execution class.
The Residency / Transport Engine decides where weights live and how missing weights move to compute.
EXO schedules model shards and distributed model execution.
```

These layers are complementary. They must not be collapsed into one ambiguous “agent runtime.”

The unified operating sequence is:

```text
INTENT
  → TASK
  → CONTEXT
  → PLAN
  → CAPABILITY GRANT
  → HARNESS / AGENT WORK
  → RUNTIME EXECUTION
  → PATCH / WORKFLOW / RESULT
  → EVIDENCE
  → INDEPENDENT REVIEW
  → AUTHORITY CLOSURE
  → COMMIT / DEPLOY / PUBLISH
  → VERIFY
  → LEARN
```

---

# 2. Core architectural doctrine

## 2.1 LMML is the sovereign control plane

LMML owns:

- human and machine identity;
- node identity and admission;
- repository and task identity;
- model and runtime registration;
- licensing and model-use policy;
- capability grants;
- context assembly and provenance;
- workflow and tool authorization;
- agent and harness orchestration;
- placement approval;
- evidence completeness;
- approvals and irreversible transitions;
- telemetry, audit and rollback records;
- user-facing APIs and operator interfaces.

No attached system becomes authoritative merely because it can execute code, hold credentials, communicate with peers or expose an MCP server.

## 2.2 Every external engine is an adapter-bound worker

Graphify, ECC, n8n-mcp, coding harnesses and EXO should be integrated through typed adapters and canonical LMML contracts.

```text
External engine
      ↓
Versioned LMML adapter
      ↓
Canonical LMML domain type
      ↓
Policy and trust labelling
      ↓
Core LMML services
```

External JSON, vendor-specific events, prompt files or workflow schemas must not leak across the whole system.

## 2.3 Proposal and authority remain separate

A model may propose a tool call. A harness may propose a patch. n8n-mcp may propose a workflow. EXO may propose a placement. AgentQ may propose a worker plan.

None of these proposals grants itself authority.

```text
PROPOSE
  → NORMALISE
  → VALIDATE
  → APPLY POLICY
  → COLLECT EVIDENCE
  → APPROVE WHERE REQUIRED
  → COMMIT
  → VERIFY
```

## 2.4 Derived knowledge is not governed truth

The architecture distinguishes:

- source-extracted graph relationships;
- model-inferred graph relationships;
- semantic retrieval;
- episodic memory;
- verified evidence;
- approved policy and architecture decisions;
- live runtime state.

These sources may be combined in a context packet, but their trust classes must remain visible.

## 2.5 Compute locality and harness choice are separate decisions

A task can use Claude Code while routing inference through an external model, Codex while invoking LMML local tools, or OpenCode while using an LMML-hosted local model. Harness selection, model selection and runtime selection must remain independent.

---

# 3. Unified target architecture

```text
┌───────────────────────────────────────────────────────────────────────────────┐
│                              OPERATOR SURFACES                                │
│                                                                               │
│ LMML TUI │ Web UI │ CLI │ API │ Claude Code │ Codex │ OpenCode │ CI/CD     │
└───────────────────────────────────┬───────────────────────────────────────────┘
                                    │
                                    ▼
┌───────────────────────────────────────────────────────────────────────────────┐
│                         LMML ACCESS AND IDENTITY GATE                         │
│                                                                               │
│ Human identity │ node identity │ harness identity │ repository binding        │
│ Session tokens │ capability grants │ quotas │ rate limits │ audit correlation  │
└───────────────┬───────────────────┬───────────────────┬───────────────────────┘
                │                   │                   │
                ▼                   ▼                   ▼
┌──────────────────────┐ ┌──────────────────────┐ ┌────────────────────────────┐
│ CONTEXT FABRIC       │ │ HARNESS FABRIC       │ │ WORKFLOW FABRIC            │
│                      │ │                      │ │                            │
│ Graphify              │ │ Claude driver        │ │ n8n-mcp compiler           │
│ Semantic RAG          │ │ Codex driver         │ │ n8n development runtime    │
│ LMML memory           │ │ OpenCode driver      │ │ n8n production runtime     │
│ ADR/policy registry   │ │ Generic driver       │ │ triggers, webhooks, retries│
│ Runtime state         │ │ Workspace manager    │ │ human approvals            │
└────────────┬─────────┘ └────────────┬─────────┘ └─────────────┬──────────────┘
             │                        │                         │
             └────────────────────────┼─────────────────────────┘
                                      ▼
┌───────────────────────────────────────────────────────────────────────────────┐
│                          AGENTQ ORCHESTRATION PLANE                           │
│                                                                               │
│ classify │ plan │ select roles │ select harness │ select model/runtime        │
│ parallelise │ checkpoint │ recover │ independent review │ assemble evidence  │
└───────────────────────────────────┬───────────────────────────────────────────┘
                                    │
                                    ▼
┌───────────────────────────────────────────────────────────────────────────────┐
│                           LMML RUNTIME BROKER                                 │
│                                                                               │
│ llama.cpp │ Ollama │ local APIs │ Colibri │ EXO cluster │ approved cloud     │
└────────────┬──────────────────────┬─────────────────────────┬─────────────────┘
             │                      │                         │
             ▼                      ▼                         ▼
┌──────────────────────┐  ┌────────────────────────┐  ┌────────────────────────┐
│ LOCAL RUNTIMES       │  │ EXO COMPOSITE CELL     │  │ SPECIALIST RUNTIMES    │
│                      │  │                        │  │                        │
│ single-node models   │  │ coordinator            │  │ sparse / huge models   │
│ low-latency agents   │  │ admitted workers       │  │ approved remote models │
│ embeddings / tools   │  │ pipeline/tensor shards │  │ specialist services    │
└────────────┬─────────┘  └────────────┬───────────┘  └────────────┬───────────┘
             │                         │                           │
             └─────────────────────────┼───────────────────────────┘
                                       ▼
┌───────────────────────────────────────────────────────────────────────────────┐
│                          ASTRA AUTHORITY BOUNDARY                             │
│                                                                               │
│ schema closure │ identity closure │ graph closure │ route closure             │
│ capability closure │ credential closure │ evidence closure │ approval closure │
└───────────────────────────────────┬───────────────────────────────────────────┘
                                    ▼
┌───────────────────────────────────────────────────────────────────────────────┐
│                         COMMIT AND EXECUTION                                  │
│                                                                               │
│ Git │ CI │ packages │ containers │ n8n activation │ runtime placement │ prod │
└───────────────────────────────────────────────────────────────────────────────┘
```

---

# 4. Architectural planes and responsibilities

## 4.1 Control plane

Primary services:

- task service;
- identity service;
- node registry;
- model registry;
- runtime registry;
- policy engine;
- approval service;
- audit service;
- AgentQ orchestration;
- desired-state reconciliation.

The control plane contains no vendor-specific orchestration assumptions. It reasons in canonical LMML contracts.

## 4.2 Context plane

The context plane fuses:

- Graphify structural evidence;
- semantic RAG results;
- approved ADRs and policies;
- task and repository state;
- verified prior evidence;
- unreviewed memories and handoffs;
- live runtime and workflow state.

It produces bounded, provenance-labelled `ContextPacket` objects.

## 4.3 Harness plane

The harness plane supports two directions:

1. A developer-started harness connects to LMML and consumes governed context and tools.
2. LMML starts a harness as an autonomous worker and collects normalized events and evidence.

## 4.4 Workflow plane

The workflow plane compiles vendor-neutral `AutomationSpec` documents into n8n development workflows, validates and tests them, and uses n8n for durable external execution.

## 4.5 Runtime plane

The runtime plane serves inference and specialised compute. It includes independent local runtimes and composite distributed runtimes.

EXO is one runtime implementation—not the control plane and not the AgentQ scheduler.

## 4.6 Authority plane

The authority plane evaluates whether a proposed transition is permitted. It is deterministic wherever possible and must not depend solely on the proposing model’s self-assessment.

## 4.7 Evidence plane

The evidence plane records what actually occurred:

- commands;
- tool calls;
- tests;
- patches;
- graph deltas;
- workflow validation;
- runtime canaries;
- benchmarks;
- approvals;
- deployment verification.

---

# 5. Canonical LMML contracts

All integrations should build against the following versioned contracts.

## 5.1 Identity types

```rust
pub struct PrincipalId(pub String);
pub struct NodeId(pub String);
pub struct HarnessRunId(pub String);
pub struct RuntimeId(pub String);
pub struct RepositoryId(pub String);
pub struct TaskId(pub String);
pub struct CapabilityGrantId(pub String);
```

Principals include:

- human users;
- service accounts;
- coding harness sessions;
- AgentQ workers;
- n8n workflow identities;
- LMML nodes;
- EXO coordinators and mapped workers.

## 5.2 `TaskSpec`

```rust
pub struct TaskSpec {
    pub id: TaskId,
    pub title: String,
    pub objective: String,
    pub task_type: TaskType,
    pub repository: Option<RepositoryRef>,
    pub base_revision: Option<GitRevision>,
    pub risk: RiskClass,
    pub constraints: Vec<Constraint>,
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    pub required_roles: Vec<AgentRole>,
    pub required_evidence: Vec<EvidenceRequirement>,
    pub model_policy: ModelPolicy,
    pub runtime_policy: RuntimePolicy,
    pub authority_policy: PolicyRef,
}
```

## 5.3 `SessionManifest`

```rust
pub struct SessionManifest {
    pub session_id: String,
    pub task_id: TaskId,
    pub principal: PrincipalId,
    pub repository: Option<RepositoryRef>,
    pub workspace: Option<WorkspaceRef>,
    pub harness: Option<HarnessDescriptor>,
    pub capability_grant: CapabilityGrantId,
    pub context_policy: ContextPolicy,
    pub required_evidence: Vec<EvidenceRequirement>,
    pub expires_at: DateTime<Utc>,
}
```

## 5.4 `CapabilityGrant`

```rust
pub struct CapabilityGrant {
    pub id: CapabilityGrantId,
    pub subject: PrincipalId,
    pub task_id: Option<TaskId>,
    pub repository: Option<RepositoryId>,
    pub workspace: Option<WorkspaceRef>,
    pub runtime_scope: Vec<RuntimeId>,
    pub allow: Vec<Capability>,
    pub deny: Vec<Capability>,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub revocation_id: String,
}
```

Grants must be short-lived, task-bound, repository-bound, workspace-bound and revocable.

## 5.5 `ContextPacket`

```rust
pub struct ContextPacket {
    pub task: TaskBrief,
    pub governed: Vec<GovernedArtifact>,
    pub structural: Vec<GraphEvidence>,
    pub semantic: Vec<SemanticEvidence>,
    pub episodic: Vec<MemoryEvidence>,
    pub runtime: Vec<RuntimeEvidence>,
    pub workflow: Vec<WorkflowEvidence>,
    pub trust_summary: TrustSummary,
    pub constraints: ExecutionConstraints,
    pub required_evidence: Vec<EvidenceRequirement>,
    pub token_budget: TokenBudget,
}
```

## 5.6 `SkillSpec`

```yaml
apiVersion: lmml.soulhash.ai/v1
kind: Skill
metadata:
  name: rust-safe-change
  version: 1.0.0
  origin:
    type: ecc-adaptation
    source: rust-patterns
spec:
  activation:
    languages: [rust]
    taskTypes: [feature, bugfix, refactor]
  requiredContext:
    graph: [changed_symbols, inbound_callers, dependencies, impacted_tests]
    governed: [rust-policy, security-policy, error-handling-adr]
  workflow:
    - inspect
    - plan
    - write_failing_test
    - implement
    - run_targeted_tests
    - graph_impact_check
    - independent_review
    - full_verification
  permissions:
    read: repository
    write: task_worktree
    network: denied
    secrets: denied
  evidence:
    required:
      - plan
      - red-test
      - green-test
      - patch
      - graph-impact
      - reviewer-report
```

## 5.7 `AutomationSpec`

```yaml
apiVersion: lmml.soulhash.ai/v1
kind: Automation
metadata:
  name: infrastructure-daily-report
spec:
  trigger:
    type: schedule
    cron: "0 7 * * *"
  steps:
    - id: fetch
      operator: lmml.metrics.summary
    - id: filter
      operator: data.filter
      input: fetch
      condition: severity >= warning
    - id: summarise
      operator: lmml.reason
      input: filter
      modelPolicy: local-first
    - id: deliver
      operator: email.send
      input: summarise
      credentialRef: infrastructure-report-mailer
  authority:
    environment: production
    approval: required
```

The vendor-neutral source remains authoritative. Generated n8n JSON is a build artifact.

## 5.8 `RuntimeRequest`

```rust
pub struct RuntimeRequest {
    pub task_id: Option<TaskId>,
    pub model: ModelSelector,
    pub capabilities: RequiredModelCapabilities,
    pub latency_class: LatencyClass,
    pub locality: LocalityPolicy,
    pub allow_distributed: bool,
    pub maximum_nodes: Option<u32>,
    pub maximum_cost: Option<Money>,
    pub sensitivity: DataSensitivity,
    pub stream: bool,
}
```

## 5.9 `PlacementReceipt`

```rust
pub struct PlacementReceipt {
    pub runtime_id: RuntimeId,
    pub model_id: String,
    pub candidate_id: String,
    pub participating_nodes: Vec<NodeId>,
    pub backend: String,
    pub sharding: Option<String>,
    pub policy_checks: Vec<PolicyCheck>,
    pub benchmark_refs: Vec<EvidenceRef>,
    pub approved_by: Option<PrincipalId>,
    pub created_at: DateTime<Utc>,
}
```

## 5.10 `EvidenceBundle`

```rust
pub struct EvidenceBundle {
    pub task_id: TaskId,
    pub run_id: String,
    pub plan: Option<PlanArtifact>,
    pub patch: Option<PatchArtifact>,
    pub commands: Vec<CommandEvidence>,
    pub tests: Vec<TestEvidence>,
    pub graph_impact: Option<GraphImpactEvidence>,
    pub workflow_validation: Option<WorkflowValidationEvidence>,
    pub runtime_canaries: Vec<RuntimeCanaryEvidence>,
    pub benchmarks: Vec<BenchmarkEvidence>,
    pub reviews: Vec<ReviewEvidence>,
    pub unresolved: Vec<UnresolvedIssue>,
    pub provenance: ProvenanceRecord,
    pub signature: Option<ArtifactSignature>,
}
```

## 5.11 `PolicyDecision`

```rust
pub struct PolicyDecision {
    pub decision: Decision,
    pub policy: PolicyRef,
    pub checks: Vec<PolicyCheck>,
    pub required_approvals: Vec<ApprovalRequirement>,
    pub capability_delta: Option<CapabilityDelta>,
    pub expires_at: Option<DateTime<Utc>>,
}
```

---

# 6. Coding harness integration

## 6.1 Harness mode A: harness as LMML client

The developer starts a preferred harness inside an LMML-managed repository:

```bash
claude
codex
opencode
```

The harness connects to one project-scoped MCP endpoint:

```text
mcp://lmml-engineering
```

The harness can request:

- task state;
- Graphify structural context;
- semantic context;
- approved ADRs and policies;
- selected ECC-derived skills;
- bounded prior memories;
- runtime and model services;
- test and verification services;
- development workflow tools;
- evidence submission;
- handoff and checkpoint operations.

It does not connect independently to every underlying service.

## 6.2 Harness mode B: harness as LMML worker

LMML launches a harness programmatically for:

- issue triage;
- repository exploration;
- test generation;
- bounded implementation;
- build repair;
- security review;
- architecture review;
- documentation maintenance;
- independent patch review;
- pull-request preparation;
- n8n workflow engineering.

```text
Task
  → AgentQ plan
  → workspace allocation
  → generated harness bundle
  → short-lived capability grant
  → harness driver
  → normalized event stream
  → evidence bundle
  → independent verification
```

## 6.3 Harness-neutral driver

```rust
#[async_trait]
pub trait HarnessDriver: Send + Sync {
    fn descriptor(&self) -> HarnessDescriptor;

    async fn detect(
        &self,
        host: &ExecutionHost,
    ) -> Result<HarnessInstallation>;

    async fn capabilities(
        &self,
        installation: &HarnessInstallation,
    ) -> Result<HarnessCapabilities>;

    async fn prepare(
        &self,
        run: &HarnessRunSpec,
        workspace: &Workspace,
        bundle: &RenderedHarnessBundle,
    ) -> Result<PreparedHarnessRun>;

    async fn start(
        &self,
        prepared: PreparedHarnessRun,
    ) -> Result<Box<dyn HarnessSession>>;

    async fn cancel(
        &self,
        session_id: &HarnessSessionId,
    ) -> Result<()>;

    async fn collect(
        &self,
        session_id: &HarnessSessionId,
    ) -> Result<HarnessRunResult>;
}
```

## 6.4 Normalized harness events

```rust
pub enum HarnessEvent {
    SessionStarted(SessionStarted),
    ContextRequested(ContextRequested),
    SkillLoaded(SkillLoaded),
    PlanCreated(PlanCreated),
    ToolRequested(ToolRequested),
    ToolApproved(ToolApproved),
    ToolDenied(ToolDenied),
    FileRead(FileRead),
    FileChanged(FileChanged),
    CommandStarted(CommandStarted),
    CommandCompleted(CommandCompleted),
    TestResult(TestResult),
    SubagentStarted(SubagentStarted),
    SubagentCompleted(SubagentCompleted),
    PermissionRequested(PermissionRequested),
    CheckpointCreated(CheckpointCreated),
    PatchPrepared(PatchPrepared),
    SessionCompleted(SessionCompleted),
    SessionFailed(SessionFailed),
}
```

Vendor-specific event formats end at the adapter boundary.

## 6.5 Runtime capability discovery

```rust
pub struct HarnessCapabilities {
    pub mcp_client: bool,
    pub mcp_server: bool,
    pub native_skills: bool,
    pub native_subagents: bool,
    pub lifecycle_hooks: HookCapability,
    pub streamed_events: bool,
    pub interactive_approvals: bool,
    pub headless_mode: bool,
    pub sdk_control: bool,
    pub project_rules: Vec<ProjectRuleFormat>,
    pub sandbox_controls: SandboxCapability,
    pub structured_output: bool,
}
```

LMML must probe current capabilities rather than freeze assumptions about rapidly evolving harnesses.

## 6.6 Canonical asset compilation

LMML owns:

```text
ProjectRuleSpec
SkillSpec
AgentRoleSpec
HookSpec
PermissionProfile
McpServerSpec
CommandSpec
ContextPolicy
EvidenceRequirement
HarnessRunProfile
```

It compiles these into each harness’s native project layout.

### Claude Code output

```text
CLAUDE.md
.claude/
├── settings.json
├── skills/
├── agents/
└── hooks/
.mcp.json
```

### Codex output

```text
AGENTS.md
.agents/
└── skills/
.codex/
├── config.toml
├── hooks.json
└── agents/
```

### OpenCode output

```text
AGENTS.md
.opencode/
├── opencode.json
├── agents/
├── skills/
├── commands/
└── plugins/
```

### Generic output

```text
AGENTS.md
.lmml/harness-manifest.json
MCP configuration
selected SKILL.md files
```

## 6.7 Hook boundary

Harness hooks must not contain business authority. They send normalized events to a local bridge:

```text
Harness hook
   → lmml-hook-bridge
   → signed local event
   → NATS / LMML Control Plane
```

A hook cannot grant itself credentials, expand its own capability grant or approve a transition.

## 6.8 Workspace policy

All coding work occurs in isolated worktrees or containers:

```text
/repositories/lmml.git                 read-only bare repository
/worktrees/task-8842-implementer       writable
/worktrees/task-8842-reviewer          separate or read-only
/artifacts/task-8842                   evidence-service controlled
```

Default allow:

- repository reads;
- edits inside the assigned worktree;
- approved build and test tools;
- `git diff` and `git status`;
- local LMML endpoints.

Default deny:

- `sudo`;
- host package mutation;
- arbitrary home-directory access;
- direct production network access;
- secret files;
- direct Docker socket;
- protected-branch push;
- credential stores.

---

# 7. Context and knowledge fabric

## 7.1 Graphify: structural truth

Graphify supplies:

- symbols and file structure;
- imports and calls;
- dependency and neighbourhood queries;
- paths between components;
- changed-symbol impact;
- pull-request impact;
- graph communities;
- repository-relative navigation lessons.

LMML should normally call Graphify behind an adapter:

```text
Harness
  → lmml_graph_change_impact
  → LMML repository snapshot resolution
  → Graphify query
  → provenance and trust labels
  → filtered GraphEvidence
```

## 7.2 Semantic RAG: conceptual retrieval

Semantic RAG answers questions that are not purely structural:

- architecture rationale;
- design language;
- past proposals;
- documentation explanations;
- policy and project concepts;
- long-form research.

Graphify and semantic RAG remain complementary.

## 7.3 ECC-derived skills

ECC is treated as an upstream skill and engineering-discipline source.

Import pipeline:

```text
ECC asset
  → parse
  → classify
  → security scan
  → remove vendor assumptions
  → map external tools to LMML tools
  → attach provenance
  → CandidateSkill
  → review and tests
  → approved LMML SkillSpec
```

Approved assets are versioned in LMML. Upstream changes cannot silently alter production behaviour.

## 7.4 Episodic memory

LMML memory stores:

- task handoffs;
- unresolved issues;
- previous approaches;
- verified and unverified lessons;
- worker-specific context;
- compact session summaries.

It does not store secrets, policy authority or undocumented production truth.

## 7.5 Governed truth

Governed truth includes:

- Git source;
- approved ADRs;
- schemas;
- policies;
- released skills;
- signed model admission receipts;
- production workflow manifests;
- approved runtime placement policies.

## 7.6 Live state

Live state includes:

- active tasks;
- harness runs;
- n8n executions;
- node health;
- runtime availability;
- EXO cluster state;
- model-instance state;
- current workload and reservations.

## 7.7 Trust classes

| Trust class | Meaning |
|---|---|
| `T0_EXTERNAL_UNTRUSTED` | Issues, emails, web pages, documents, workflow payloads and user-controlled content |
| `T1_INFERRED` | Model or graph inference without direct structural proof |
| `T2_EXTRACTED` | Direct AST, source or runtime extraction |
| `T3_UNREVIEWED_MEMORY` | Handoffs, Graphify lessons, ECC memories and agent summaries |
| `T4_VERIFIED_EVIDENCE` | Reproducible test, canary, benchmark or signed runtime observation |
| `T5_GOVERNED` | Approved policy, ADR, schema, released skill or production manifest |
| `T6_AUTHORISED_EXECUTION` | A specific capability authorized for a specific transaction |

## 7.8 Context priority

```text
1. Current task and operator instruction
2. T5 governed policy and architecture
3. T2 current source and extracted graph evidence
4. T4 tests, canaries and verified runtime evidence
5. semantic project documentation
6. verified historical evidence
7. T3 unreviewed memory and handoffs
8. T0 external untrusted content
```

## 7.9 Lazy MCP broker

Only compact tool metadata is always visible:

```text
tool name
short description
provider
risk class
capability tags
```

Full schemas, examples and authentication requirements are hydrated on demand.

Example registry:

```yaml
tools:
  graph.query:
    provider: graphify
    risk: read_only
    hydrate: on_demand

  memory.search:
    provider: lmml-memory
    risk: contextual
    hydrate: on_demand

  workflow.compile:
    provider: n8n-mcp
    risk: draft_write
    hydrate: on_demand

  runtime.exo.instance.create:
    provider: lmml-exo-controller
    risk: infrastructure_mutation
    approval: required

  workflow.activate:
    provider: lmml-authority-gateway
    risk: production_commit
    approval: required
```

---

# 8. AgentQ orchestration

## 8.1 AgentQ schedules work, not model shards

```text
AgentQ:
  decomposes a task into explorer, implementer, tester and reviewer roles.

EXO:
  divides one approved model instance across several machines.
```

An AgentQ worker may use an EXO-hosted model, but AgentQ does not manage individual tensor shards.

## 8.2 Harness-neutral roles

| Role | Write authority | Purpose |
|---|---:|---|
| Explorer | none | Map repository, graph and evidence |
| Planner | none | Produce plan, trade-offs and risks |
| Implementer | assigned worktree | Make bounded code changes |
| Tester | scoped | Create and run verification |
| Reviewer | none | Review patch from fresh context |
| Security reviewer | none | Inspect trust boundaries and dangerous behavior |
| Workflow engineer | n8n development only | Compile and test automation |
| Runtime evaluator | none | Validate model/runtime capabilities and benchmarks |
| Release operator | no direct implementation | Assemble release evidence |
| Authority evaluator | deterministic service | Evaluate policy and closure |

## 8.3 Multi-harness patterns

### Fresh-context review

```text
Claude implementer
  → new Claude reviewer session
  → LMML test verification
```

### Cross-harness review

```text
Claude implementer
  → Codex reviewer
  → OpenCode local-model tester
  → authority gate
```

### Parallel exploration

```text
Graph question
  ├── Graphify deterministic query
  ├── Claude explorer
  └── Codex explorer
       ↓
  LMML synthesis
```

### Adversarial review

```text
Implementer
  → correctness reviewer
  → security reviewer
  → regression reviewer
  → evidence aggregator
```

## 8.4 Harness and runtime routing

Harness selection considers:

- task type;
- required native features;
- repository sensitivity;
- need for interactive approval;
- need for event streaming;
- cost policy;
- historical reliability;
- available models;
- local-only requirements.

Runtime selection separately considers:

- model capability;
- context length;
- latency class;
- sensitivity;
- model locality;
- warm-state availability;
- hardware support;
- runtime health;
- measured throughput;
- current reservations.

## 8.5 Example routing

```yaml
routing:
  repository_exploration:
    preferred_workers:
      - graphify
      - local-model
      - claude-code-explorer
    runtime_policy: local-preferred

  bounded_implementation:
    preferred_harnesses:
      - claude-code
      - codex
      - opencode

  independent_review:
    require_different_run: true
    prefer_different_harness: true
    runtime_policy: quality-preferred

  sensitive_local_only:
    allowed_harnesses:
      - opencode-local
      - lmml-native
    allowed_runtimes:
      - llama-cpp-local
      - exo-approved-local-cell
    external_network: denied
```

---

# 9. Runtime fabric and model residency

## 9.1 Governing principle

LMML separates:

- model identity;
- model architecture;
- inference runtime;
- execution topology;
- weight residency;
- weight transport;
- cache hierarchy;
- placement policy;
- authority and tool mediation.

No runtime-specific loading mechanism may leak into AgentQ, coding harnesses, tools or the public inference API.

```text
Model
  → architecture classification
  → resource inventory
  → residency feasibility
  → placement candidates
  → benchmark and policy ranking
  → RuntimeEndpoint
```

This turns llama.cpp `mmap`, Pulsar NVMe expert streaming and EXO distributed sharding into comparable runtime strategies instead of unrelated implementation details.

## 9.2 Runtime classes

| Runtime class | Examples | Primary model strategy | Topology |
|---|---|---|---|
| `LOCAL_MAPPED` | llama.cpp | mmap, mlock, DirectIO, GPU offload | single host, optional RPC |
| `LOCAL_MANAGED` | Ollama | runtime-managed local model lifecycle | single host |
| `LOCAL_STREAMED_MOE` | Pulsar | NVMe-streamed routed experts and hot caches | single host, multi-GPU |
| `DISTRIBUTED_SHARDED` | EXO | distributed model shards | multi-node |
| `SPECIALIST_OUT_OF_CORE` | Colibri | capability-dependent large-model strategy | capability dependent |
| `REMOTE_SPECIALIST` | approved providers | provider-managed endpoint | remote |

## 9.3 Canonical runtime kinds

```rust
pub enum RuntimeKind {
    LlamaCpp,
    Ollama,
    Pulsar,
    ExoCluster,
    Colibri,
    RemoteOpenAiCompatible,
    RemoteProvider,
}
```

## 9.4 Orthogonal runtime properties

Every runtime instance should report four independent properties.

### Execution topology

```rust
pub enum ExecutionTopology {
    SingleDevice,
    SingleHost,
    MultiGpuHost,
    MultiNode,
    Remote,
}
```

### Weight residency

```rust
pub enum WeightResidency {
    FullyResident,
    HostMapped,
    HostResident,
    Tiered,
    Streamed,
    DistributedShards,
}
```

### Weight transport

```rust
pub enum WeightTransport {
    None,
    OsPageFault,
    MemoryCopy,
    DirectIo,
    NvmeExpertStreaming,
    PciePeerTransfer,
    NetworkShardTransport,
}
```

### Cache hierarchy

```rust
pub struct WeightCacheTopology {
    pub gpu_resident_bytes: Option<u64>,
    pub gpu_cache_bytes: Option<u64>,
    pub host_cache_bytes: Option<u64>,
    pub filesystem_page_cache: bool,
    pub direct_io: bool,
    pub persistent_warm_profile: bool,
}
```

## 9.5 llama.cpp loading contract

llama.cpp loading mode must be explicit in LMML even when it matches the upstream default.

```rust
pub enum LlamaLoadMode {
    None,
    Mmap,
    Mlock,
    MmapMlock,
    DirectIo,
}

impl Default for LlamaLoadMode {
    fn default() -> Self {
        Self::Mmap
    }
}
```

Target state:

```toml
[server.loading]
load_mode = "mmap"

[server.memory]
kv_cache_k = "q8_0"
kv_cache_v = "q8_0"
cache_ram_mib = 4096
```

The llama.cpp adapter should emit:

```text
--load-mode mmap
```

This makes the runtime contract observable:

```text
Runtime       llama.cpp
Load mode     mmap
Model file    /models/Qwen....gguf
Host backing  filesystem/page cache
GPU offload   auto
GPU layers    -1
KV K          q8_0
KV V          q8_0
RAM cache     4096 MiB
```

The legacy `mlock: bool` setting should be migrated rather than kept as the primary loading API:

```rust
fn migrate_legacy_loading(
    legacy_mlock: Option<bool>,
    explicit_mode: Option<LlamaLoadMode>,
) -> LlamaLoadMode {
    if let Some(mode) = explicit_mode {
        return mode;
    }

    match legacy_mlock {
        Some(true) => LlamaLoadMode::MmapMlock,
        _ => LlamaLoadMode::Mmap,
    }
}
```

`mlock = true` becomes `load_mode = "mmap+mlock"`.

`mlock = false` becomes `load_mode = "mmap"`.

## 9.6 Strategy manifests

### llama.cpp mapped runtime

```yaml
runtime: llama_cpp

topology:
  type: single_host

residency:
  strategy: host_mapped

transport:
  strategy: os_page_fault

loading:
  llama_load_mode: mmap

gpu:
  layers: auto

kv:
  k: q8_0
  v: q8_0
```

### Pulsar streamed-MoE runtime

```yaml
runtime: pulsar

topology:
  type: multi_gpu_host

residency:
  strategy: tiered_streaming

transport:
  strategy: nvme_expert_streaming

storage:
  direct_io: true
  io_uring: true

cache:
  gpu_hot_set: auto
  host_lfu: auto
  persistent_census: true
```

Pulsar is not a llama.cpp loading mode. It is an independent runtime that actively transports selected MoE experts from NVMe and host cache into the compute path.

First integration should be sidecar-only through its OpenAI-compatible server:

```text
LMML
  → Runtime Broker
  → Pulsar Adapter
  → localhost:11435
      /v1/models
      /v1/chat/completions
```

### EXO distributed-shard runtime

```yaml
runtime: exo

topology:
  type: multi_node

residency:
  strategy: distributed_shards

transport:
  strategy: network_collectives

placement:
  topology_aware: true
```

EXO and Pulsar solve different capacity problems:

```text
Dense model too large for one host       → EXO candidate
Sparse MoE too large for VRAM but local  → Pulsar candidate
Medium dense GGUF fits host/GPU split    → llama.cpp candidate
Managed local endpoint is enough         → Ollama candidate
```

## 9.7 Model execution requirements

```rust
pub struct ModelExecutionRequirements {
    pub architecture: ArchitectureClass,
    pub total_parameters: Option<u64>,
    pub active_parameters_per_token: Option<u64>,
    pub weight_bytes: u64,
    pub context_tokens: u64,
    pub required_capabilities: ModelCapabilities,
    pub latency_class: LatencyClass,
    pub locality: LocalityPolicy,
}

pub enum ArchitectureClass {
    Dense,
    MoE,
    HybridMoE,
    LinearAttention,
    HybridAttention,
    Unknown,
}
```

`active_parameters_per_token` is mandatory for sparse-MoE feasibility. A dense 295B model and a 295B MoE with roughly 20B active parameters per token are different runtime problems.

## 9.8 Storage and PCIe capability detection

Node detection must measure more than CPU, RAM, GPU, VRAM and network.

```rust
pub struct StorageCapability {
    pub path: PathBuf,
    pub medium: StorageMedium,
    pub sequential_read_bps: u64,
    pub random_read_iops: Option<u64>,
    pub direct_io: bool,
    pub io_uring: bool,
    pub filesystem: String,
    pub free_bytes: u64,
    pub benchmark_receipt: Option<BenchmarkReceiptId>,
}
```

LMML should add:

- NVMe sequential throughput;
- random-read behaviour;
- DirectIO support;
- `io_uring` support;
- filesystem type;
- PCIe topology;
- measured PCIe bandwidth;
- host page-cache pressure;
- GPU-to-GPU transfer feasibility where available.

Resource capability should be measured, not inferred only from device names.

## 9.9 Runtime broker responsibilities

- normalize model catalogues;
- track runtime health;
- enforce model licensing;
- classify model architecture;
- evaluate model capability;
- evaluate residency feasibility;
- evaluate storage and transport feasibility;
- select runtime class;
- reserve resources;
- route streaming and cancellation;
- normalize usage and errors;
- persist empirical performance;
- track cold, warm and degraded runtime state;
- fall back safely;
- prevent unsupported runtimes from being promoted.

## 9.10 Runtime selection sequence

```text
RuntimeRequest
  → model registry lookup
  → architecture classification
  → capability filter
  → policy and sensitivity filter
  → residency feasibility
  → placement candidates
  → health filter
  → warm-instance preference
  → latency/throughput/storage ranking
  → capacity reservation
  → selected RuntimeEndpoint
  → execution
  → evidence and metrics
```

## 9.11 Placement candidate

```rust
pub struct PlacementCandidate {
    pub runtime: RuntimeKind,
    pub model: ModelId,
    pub resources: Vec<ResourceRef>,
    pub feasibility: Feasibility,
    pub estimated_ttft: Duration,
    pub estimated_decode_tps: f64,
    pub estimated_prefill_tps: f64,
    pub memory_headroom: f32,
    pub storage_pressure: f32,
    pub network_pressure: f32,
    pub readiness: RuntimeReadinessState,
    pub confidence: f32,
}

pub enum RuntimeReadinessState {
    Absent,
    Cold,
    Loading,
    CensusBuilding,
    Warm,
    Ready,
    Degraded,
}
```

Warm-state matters because a warm streamed-MoE runtime may beat a cold distributed runtime even when static peak throughput says otherwise.

## 9.12 Runtime decision matrix

| Situation | Preferred starting runtime |
|---|---|
| Small or medium dense GGUF fits GPU | llama.cpp |
| Dense model fits host/GPU split acceptably | llama.cpp `mmap` |
| Need locked predictable host residency | llama.cpp `mmap+mlock` |
| Want page-cache bypass or controlled I/O experiment | llama.cpp `dio` |
| Giant sparse MoE, Linux NVIDIA, fast NVMe | Pulsar |
| Model must span several machines | EXO |
| Massive specialised sparse or out-of-core workload | Pulsar / Colibri benchmark race |
| Easy managed local endpoint | Ollama |
| Sensitive coding task | local eligible runtime |
| High-depth AgentQ synthesis | best measured eligible runtime |

## 9.13 Runtime adapter package layout

```text
crates/
├── lmml-runtime-core/
├── lmml-runtime-llamacpp/
├── lmml-runtime-pulsar/
├── lmml-runtime-exo/
├── lmml-runtime-ollama/
└── lmml-runtime-colibri/
```

Common interface:

```rust
pub trait InferenceRuntime {
    fn capabilities(
        &self,
    ) -> impl Future<Output = Result<RuntimeCapabilities>> + Send;

    fn can_host(
        &self,
        model: &ModelManifest,
        node: &ResourceSnapshot,
    ) -> impl Future<Output = Result<FeasibilityReport>> + Send;

    fn prepare(
        &self,
        model: &ModelManifest,
    ) -> impl Future<Output = Result<PreparedRuntime>> + Send;

    fn start(
        &self,
        prepared: PreparedRuntime,
    ) -> impl Future<Output = Result<RuntimeInstance>> + Send;

    fn infer(
        &self,
        request: CanonicalInferenceRequest,
    ) -> impl Future<Output = Result<InferenceStream>> + Send;

    fn benchmark(
        &self,
        profile: BenchmarkProfile,
    ) -> impl Future<Output = Result<BenchmarkReceipt>> + Send;

    fn stop(
        &self,
        instance: RuntimeInstanceId,
    ) -> impl Future<Output = Result<()>> + Send;
}
```

## 9.14 Runtime authority boundary

Runtime-local tool mechanisms must not bypass LMML authority.

Pulsar may expose an optional MCP tool loop for standalone use, but production LMML routes tool calls through the LMML MCP Gateway:

```text
model proposes tool call
  → LMML receives proposal
  → capability gate
  → schema validation
  → LMML MCP Gateway
  → approved tool
  → evidence ledger
```

The model proposes tools. LMML executes them.

## 9.15 Residency benchmark family

Benchmarking must cover startup, residency and transport economics, not only decode tokens per second.

Track:

- cold model startup;
- warm model startup;
- first-token page faults;
- host page-cache residency;
- model load duration;
- NVMe sequential bandwidth;
- NVMe random-read behaviour;
- DirectIO bandwidth;
- expert cache hit rate;
- VRAM tier hit rate;
- host tier hit rate;
- storage miss rate;
- steady-state decode;
- prompt prefill throughput;
- decode slope across output lengths.

This matters because streamed and distributed runtimes have different warmup, cache and miss-rate curves.

## 9.16 Detection output target

`lmml detect` should surface runtime strategies directly:

```text
Inference Capability
────────────────────────────────────────────────────

GPU
  AMD Radeon AI PRO R9700
  VRAM             32 GiB
  backend          ROCm
  llama.cpp        supported
  Pulsar           unsupported: CUDA required
  EXO              control-plane eligible

NVMe
  sequential       measured
  direct I/O       yes/no
  io_uring         yes/no

Runtime strategies
  llama.cpp mmap          ELIGIBLE
  llama.cpp mmap+mlock    ELIGIBLE
  llama.cpp dio           ELIGIBLE
  Pulsar NVMe stream      INELIGIBLE: NVIDIA CUDA required
  EXO distributed         CONTROL-PLANE ELIGIBLE
```

---

# 10. EXO composite distributed runtime

## 10.1 Responsibility boundary

EXO is responsible for:

- dividing one model across selected machines;
- pipeline and tensor sharding;
- topology-aware model placement;
- distributed model loading;
- inter-node inference communication;
- execution inside an approved cluster cell;
- instance lifecycle at the distributed-runtime level.

LMML remains responsible for:

- node identity and admission;
- runtime registration;
- model policy and licensing;
- placement approval;
- workload orchestration;
- security and access control;
- health and reliability policy;
- context, memory and tools;
- user-facing APIs;
- telemetry and audit;
- deciding when EXO is preferable to other runtimes.

## 10.2 One EXO cluster is one LMML runtime

```text
Runtime: exo://cluster-alpha
Model:   Qwen3-235B
Nodes:   aurora + terran + node-r9700
Mode:    pipeline
Status:  READY
```

The internal shards do not appear as independent copies of the model. LMML registers one logical model endpoint.

## 10.3 Parallel implementation tracks

1. **EXO control-plane integration:** implement now.
2. **Linux GPU execution enablement:** keep capability-gated until a validated backend exists.

The source baseline states that the maintained accelerator path is Apple Silicon through MLX/MLX Distributed and that the documented Linux path is CPU-only, with CUDA and Vulkan on the roadmap. This must be revalidated before changing the production capability gate.

## 10.4 EXO execution topology

```text
LMML Runtime Broker
        │
        ▼
EXO Runtime Controller
  ├── placement policy
  ├── instance manager
  ├── capability gate
  ├── model registry adapter
  ├── inference adapter
  └── telemetry/reconciler
        │
        ▼
EXO Coordinator :52415
  ├── admitted worker: Aurora
  ├── admitted worker: Terran
  ├── admitted worker: compatible GPU node
  └── optional compatible worker
        │
        ▼
Distributed logical model instance
  ├── shard A
  ├── shard B
  └── shard C
```

## 10.5 EXO package layout

In a Rust-first LMML monorepo, implement the canonical domain and controller in Rust while permitting a Python compatibility adapter where upstream EXO integration is easiest.

Recommended structure:

```text
crates/
├── lmml-runtime-exo/
│   ├── src/config.rs
│   ├── src/types.rs
│   ├── src/client.rs
│   ├── src/capabilities.rs
│   ├── src/discovery.rs
│   ├── src/reconciler.rs
│   ├── src/placement.rs
│   ├── src/instances.rs
│   ├── src/inference.rs
│   ├── src/streaming.rs
│   ├── src/telemetry.rs
│   ├── src/benchmark.rs
│   ├── src/security.rs
│   └── src/errors.rs
└── lmml-runtime-core/

services/
└── exo-compat-adapter/       # optional Python adapter, if required
```

## 10.6 Feature configuration

```yaml
runtime:
  exo:
    enabled: false
    endpoint: http://benji:52415
    clusterId: exo-cluster-alpha
    namespace: lmml-exo-prod
    coordinatorNodeId: benji
    requestTimeoutSeconds: 30
    instanceTimeoutSeconds: 300
    statePollSeconds: 5
    maxInstances: 2
    maxNodesPerInstance: 4
    minimumMemoryHeadroomRatio: 0.15
    features:
      allowInstanceMutation: false
      allowModelDownloads: false
      allowCustomModels: false
      allowTrustRemoteCode: false
      linuxGpuExperimental: false
      imageModelsEnabled: false
```

## 10.7 Typed EXO boundary

```rust
pub struct ExoClusterStatus {
    pub cluster_id: String,
    pub coordinator_id: String,
    pub status: ExoClusterState,
    pub nodes: Vec<ExoNodeStatus>,
    pub instances: Vec<ExoInstanceStatus>,
    pub observed_at: DateTime<Utc>,
    pub raw: serde_json::Value,
}

pub struct ExoNodeStatus {
    pub exo_node_id: String,
    pub lmml_node_id: Option<NodeId>,
    pub hostname: Option<String>,
    pub platform: Option<String>,
    pub memory_total: Option<u64>,
    pub memory_available: Option<u64>,
    pub accelerator_backend: Option<String>,
    pub state: String,
    pub admission_state: NodeAdmissionState,
}

pub struct ExoPlacementPreview {
    pub model_id: String,
    pub sharding: String,
    pub instance_meta: String,
    pub instance_payload: serde_json::Value,
    pub memory_delta_by_node: BTreeMap<String, u64>,
    pub error: Option<String>,
    pub raw: serde_json::Value,
}
```

Preserve raw responses for diagnostics, but prevent core LMML services from depending on upstream internals.

## 10.8 EXO API mappings

| LMML operation | EXO endpoint |
|---|---|
| Resolve coordinator identity | `GET /node_id` |
| Read cluster topology and instances | `GET /state` |
| Read diagnostic events | `GET /events` |
| List models | `GET /models` |
| List OpenAI models | `GET /v1/models` |
| Preview placement | `GET /instance/previews` |
| Compute placement | `GET /instance/placement` |
| Create exact instance | `POST /instance` |
| Ask EXO to place instance | `POST /place_instance` |
| Await readiness | `GET /instance/await` |
| Read instance | `GET /instance/{id}` |
| Delete instance | `DELETE /instance/{id}` |
| Run chat | `POST /v1/chat/completions` |
| Run benchmark | `POST /bench/chat/completions` |
| Cancel generation | `POST /v1/cancel/{command_id}` |

The adapter must support asynchronous instance creation and Server-Sent Event readiness streams.

## 10.9 HTTP behavior

The client must include:

- connection pooling;
- separate control and inference timeouts;
- bounded retries for idempotent reads only;
- no blind generation retry;
- incremental SSE parsing;
- response-size limits;
- cancellation on downstream disconnect;
- correlation IDs;
- structured error translation;
- sanitized logging;
- upstream version capture where available.

## 10.10 Runtime registration

```yaml
id: exo-cluster-alpha
kind: exo_cluster
endpoint: http://benji:52415
enabled: true
capabilities:
  textGeneration: true
  chat: true
  streaming: true
  toolCalling: conditional
  reasoning: conditional
  vision: conditional
  imageGeneration: conditional
  embeddings: false
  distributedInference: true
  dynamicPlacement: true
policy:
  administrativeAccess: internal_only
  automaticModelDownload: false
  allowCustomModels: false
  placementApproval: required
  minimumHealth: healthy
```

Capabilities remain per model, not merely per cluster.

## 10.11 Cluster reconciliation

`ExoStateReconciler` compares observed EXO state with LMML desired state:

- coordinator identity;
- visible nodes;
- admitted nodes;
- expected and active instances;
- stale or vanished instances;
- model readiness;
- resource changes;
- cluster health;
- desired versus actual placement.

Cluster state:

```text
UNREGISTERED
  → DISCOVERING
  → HEALTHY
       ├── DEGRADED
       └── UNREACHABLE
              ↓
          RECOVERING
              ↓
           HEALTHY
```

Instance state:

```text
ABSENT
  → PLANNING
  → ADMITTED
  → CREATING
  → LOADING
  → WARMING
  → CANARY
  → READY
  → DRAINING
  → DELETING
  → ABSENT
```

Failure transitions:

```text
PLANNING → REJECTED
CREATING → FAILED
LOADING  → TIMEOUT
WARMING  → FAILED
CANARY   → QUARANTINED
READY    → DEGRADED
```

EXO presence alone does not make an instance routable. LMML canary validation is mandatory.

## 10.12 Node identity and admission

Map EXO nodes to LMML nodes using:

1. stable EXO ID where available;
2. hostname;
3. observed addresses;
4. machine fingerprint;
5. LMML node-agent identity;
6. explicit administrator mapping.

Admission states:

```text
DISCOVERED
UNVERIFIED
ADMITTED
QUARANTINED
REVOKED
MAINTENANCE
```

Only admitted nodes may participate in an approved placement.

Node roles may include:

```text
exo_coordinator
exo_worker
lmml_agent_worker
rag_worker
embedding_worker
validator
storage_node
```

A node should join EXO only when distributed inference is its best use.

## 10.13 Placement admission

```text
Model requested
  → retrieve EXO placement previews
  → normalize candidates
  → apply LMML hard gates
  → rank survivors
  → select candidate
  → persist PlacementReceipt
  → create exact preview payload
```

Hard rejection criteria:

- unadmitted node;
- node in maintenance or quarantine;
- unsupported backend;
- insufficient projected memory headroom;
- excessive node count;
- unlicensed or unapproved model;
- prohibited sharding/network combination;
- unavailable model files when downloads are disabled;
- resource reservation conflict;
- unacceptable degraded state;
- experimental backend without experimental policy.

Initial ranking weights:

```text
30% measured generation performance
20% measured prefill performance
15% free-memory headroom
15% network suitability
10% node reliability
 5% model already available
 5% energy efficiency
```

Penalties:

- heterogeneous accelerator speed;
- high variance;
- recent failures;
- model-load failures;
- cross-switch communication;
- thermal throttling;
- large downloads;
- eviction of a valuable warm model.

Network defaults:

| Network | Pipeline | Tensor |
|---|---:|---:|
| 1 GbE | experimental/small models | disabled |
| 2.5 GbE | benchmark-gated | disabled by default |
| 10 GbE | preferred baseline | experimental |
| 25 GbE+ | allowed | benchmark-gated |
| RDMA-class | allowed | preferred where supported |

## 10.14 Model registry integration

Canonical identity:

```text
provider:     exo
runtime_id:   exo-cluster-alpha
upstream_id:  mlx-community/Model-Name
canonical_id: exo-cluster-alpha::mlx-community/Model-Name
```

Per-model capability manifest:

```yaml
modelId: exo-cluster-alpha::mlx-community/Qwen3
capabilities:
  chat: true
  tools: true
  reasoning: true
  vision: false
  imageGeneration: false
  structuredOutput: unverified
  logprobs: unverified
  maxContext: discovered
```

Capability sources, in order:

1. LMML curated overrides;
2. successful canaries;
3. EXO metadata;
4. model-card metadata;
5. conservative defaults.

Model admission receipt:

```text
model ID
revision/hash
quantization
weight licence
source repository
remote-code requirement
approved capabilities
approved execution cells
canary result
benchmark result
approval date
```

## 10.15 Instance lifecycle

Creation:

```text
placement request
  → previews
  → admission and selection
  → desired-instance record
  → POST /instance
  → store command ID
  → subscribe to /instance/await
  → resolve instance in /state
  → run LMML canary
  → register READY
```

Canaries:

1. short deterministic completion;
2. streaming termination;
3. token-usage structure;
4. Unicode response;
5. stop sequence;
6. tool call where supported;
7. seeded repeat where appropriate;
8. cancellation;
9. maximum-permitted prompt smoke test;
10. node-state recheck.

Draining:

```text
READY
  → no-new-requests
  → DRAINING
  → wait for active requests
  → DELETE
  → confirm absent
  → ABSENT
```

Idempotency:

- persist desired state before mutation;
- use an LMML operation ID;
- inspect `/state` before retrying;
- never blindly duplicate create requests;
- reconcile after coordinator restart.

## 10.16 Inference adapter

The canonical LMML chat request maps into EXO’s compatible API.

The adapter must:

- reject unsupported parameters rather than silently ignore them;
- parse SSE incrementally;
- support keep-alives and `[DONE]`;
- measure time to first token;
- normalize usage;
- propagate disconnect and cancellation;
- avoid buffering complete responses.

Tool calling remains under LMML:

```text
EXO model proposes tool call
  → LMML validates name and arguments
  → policy gate
  → LMML executes tool
  → result returned to model
```

EXO never receives LMML tool credentials merely because its model proposed a call.

## 10.17 EXO-preferred workloads

Use EXO when:

- the model cannot fit on one supported node;
- placement meets latency policy;
- a suitable warm distributed instance exists;
- model quality materially exceeds a local alternative;
- larger context or model capacity is required;
- benchmark evidence supports the cell.

Avoid EXO when:

- a small local model is sufficient;
- latency sensitivity is high;
- network health is degraded;
- model loading dominates the task;
- modality or parameters are unsupported;
- resources are reserved for higher-priority work;
- only an unsupported Linux accelerator path is available.

AgentQ initial use cases:

- high-depth synthesis;
- difficult code review;
- architecture critique;
- answer adjudication;
- long-context analysis;
- complex planning.

Fast local models remain preferred for:

- tool selection;
- short loops;
- retrieval-query generation;
- memory classification;
- simple transformations;
- repetitive agent turns.

## 10.18 EXO execution cells

```yaml
exoCells:
  exo-supported:
    purpose: production distributed inference
    requiredBackend: mlx
    sharding: [pipeline, tensor]

  exo-linux-cpu-lab:
    purpose: API and control-plane validation
    requiredBackend: cpu
    sharding: [pipeline]
    productionRouting: false

  exo-linux-cuda-lab:
    purpose: experimental CUDA backend
    requiredBackend: cuda
    productionRouting: false

  exo-rdma:
    purpose: high-bandwidth tensor execution
    requiredNetwork: rdma
```

Recommended initial node roles:

```text
Benji
  LMML control plane
  EXO coordinator with no worker role

Aurora
  EXO worker only when backend is supported
  otherwise normal LMML inference worker

Terran
  EXO worker only when backend is supported
  otherwise normal LMML inference worker

R9700 node
  experimental EXO worker only after backend validation

Polaris
  CPU EXO laboratory worker
  preferably retrieval/evaluation in production

Nebula
  independent validator
  excluded from primary model shards by default
```

## 10.19 Linux GPU strategy

Production eligibility remains capability-gated:

```rust
fn exo_node_is_production_eligible(node: &NodeCapability) -> bool {
    match (node.platform.as_str(), node.backend.as_str()) {
        ("macos", "mlx") => node.backend_validation_passed,
        ("linux", "cpu") => node.cpu_production_policy_enabled,
        ("linux", "cuda" | "vulkan" | "rocm") => {
            settings.exo.linux_gpu_experimental
                && node.backend_validation_passed
                && node.benchmark_receipt.is_some()
        }
        _ => false,
    }
}
```

Upstream work should target the backend boundary rather than fork the distributed scheduler:

```text
Linux CUDA single node
  → Linux CUDA pipeline
  → Linux CUDA tensor
  → Linux Vulkan single node
  → Linux Vulkan pipeline
  → mixed-vendor research
```

Do not start with mixed NVIDIA/AMD tensor parallelism.

Interim Linux options remain:

- llama.cpp RPC or another proven distributed path;
- multiple independent llama.cpp workers;
- Colibri for sparse or out-of-core models;
- EXO for control-plane validation and supported hardware.

## 10.20 Model storage

Recommended production environment:

```text
EXO_DEFAULT_MODELS_DIR=/srv/lmml/exo-cache
EXO_MODELS_READ_ONLY_DIRS=/srv/lmml/models-approved
EXO_OFFLINE=true
EXO_ENABLE_IMAGE_MODELS=false
EXO_LIBP2P_NAMESPACE=lmml-exo-prod
```

Model promotion:

```text
request
  → licence verification
  → revision pin
  → quarantine download
  → hash calculation
  → metadata and security validation
  → approved read-only store
  → catalogue refresh
```

Record storage type, path, bandwidth, latency, local-cache state, completeness and content hash.

## 10.21 EXO security boundary

Network rules:

```text
LMML gateway → EXO coordinator API: allow
EXO nodes ↔ EXO nodes: allow required cluster traffic
ordinary LAN clients → EXO API: deny
Internet → EXO API: deny
EXO workers → unrelated internal services: deny by default
```

Administrative operations remain LMML-admin only:

- model addition/deletion;
- custom-model registration;
- instance creation/deletion;
- placement override;
- cluster-event inspection;
- image-model enablement.

Secure defaults:

```text
trust_remote_code=false
automatic_download=false
custom_models=false
offline=true in production
revision pinning required
weight hashes required
licence receipt required
```

---

# 11. Workflow fabric: n8n and n8n-mcp

## 11.1 Three workflow roles

### Build plane

Use n8n-mcp for:

- node discovery and documentation;
- template retrieval;
- schema-aware node validation;
- workflow validation;
- workflow diffing;
- auto-fix;
- security audit;
- development workflow creation and updates.

### Runtime plane

Use n8n for:

- webhooks;
- schedules;
- SaaS and database integrations;
- retries;
- human approvals;
- long-running workflows;
- external event normalization.

### Authority plane

Use LMML for:

- credential references;
- production promotion;
- side-effect classification;
- approval;
- deployment transactions;
- audit and rollback.

## 11.2 Bidirectional MCP

```text
Coding harness / AgentQ
  → LMML workflow tools
  → n8n-mcp compiler and validator
  → development n8n instance
```

```text
n8n workflow
  → selected LMML MCP tools
  → reasoning, RAG, runtime or AgentQ operation
```

```text
LMML
  → selected n8n workflow tools
  → durable business or infrastructure operation
```

Every direction is capability-scoped.

## 11.3 Workflow as code

```text
automation/
└── infrastructure-daily-report/
    ├── automation.yaml
    ├── workflow.generated.json
    ├── input.schema.json
    ├── output.schema.json
    ├── fixtures/
    ├── tests/
    ├── policy.yaml
    └── README.md
```

The coding harness edits `automation.yaml`. n8n-mcp compiles and validates generated JSON. LMML controls production promotion.

## 11.4 Workflow development transaction

```text
PROPOSE
  → COMPILE
  → VALIDATE
  → SECURITY SCAN
  → SYNTHETIC TEST
  → DIFF
  → GRAPH/DATA-LINEAGE CHECK
  → APPROVE
  → COMMIT
  → ACTIVATE
  → VERIFY
  → RECORD
```

## 11.5 Credential aliases

Harnesses and models see:

```json
{
  "credential_ref": "crm-production-readonly"
}
```

They never receive the secret value. LMML resolves the alias only after policy approval and only inside the appropriate execution boundary.

## 11.6 High-risk workflow operators

Code nodes, unrestricted HTTP nodes, shell-like operations and broad database writes require:

- static inspection;
- SSRF controls;
- domain allowlists;
- module allowlists;
- sandbox execution;
- idempotency analysis;
- human approval where appropriate.

---

# 12. Authority, security and identity

## 12.1 Layered enforcement

```text
Harness-native permissions
  + workspace/container sandbox
  + LMML capability grant
  + MCP/tool gateway policy
  + node/runtime admission
  + commit/deployment authority
```

Weakening one layer must not remove the others.

## 12.2 Node admission

All compute and runtime nodes pass through:

```text
DISCOVERED
  → UNVERIFIED
  → ADMITTED
  → MAINTENANCE / QUARANTINED / REVOKED
```

Discovery is never equivalent to trust.

## 12.3 Prompt-injection boundary

Untrusted content cannot:

- grant capabilities;
- mutate policy;
- reveal secrets;
- approve execution;
- promote memory;
- authorize a runtime placement;
- activate a workflow;
- merge a patch.

Untrusted content includes issues, PR comments, source comments, documents, web pages, workflow payloads, memories, tool outputs and model-generated metadata.

## 12.4 Secret boundary

Secrets are held by a dedicated provider and exposed only as scoped operations. They do not appear in prompts, memory, generated configuration, model cards, logs or EXO environments unless strictly required.

## 12.5 Network zones

Recommended zones:

```text
Operator zone
Control-plane zone
Harness sandbox zone
Tool/workflow zone
Runtime zone
EXO private cluster zone
Model quarantine zone
Approved model-storage zone
Production integration zone
```

Network communication is allowlisted between zones.

## 12.6 Progressive Closure

### Local closure

Required for every operation:

- valid identity;
- valid task/session;
- schema-valid call;
- capability permission;
- destination permission;
- secret non-exposure.

### Structural closure

Required for code change:

- changed symbols known;
- callers/dependencies assessed;
- architecture boundaries checked;
- relevant tests identified;
- graph freshness recorded.

### Route closure

Required for workflows and distributed execution:

- every branch terminates;
- failure paths exist;
- retries are safe;
- data lineage is known;
- side effects are classified;
- idempotency is defined;
- participating nodes are admitted;
- runtime route is supported.

### Global closure

Required for high-consequence transitions:

- protected-branch merge;
- production deployment;
- workflow activation;
- credential mutation;
- model admission;
- distributed placement on experimental hardware;
- public publication;
- financial action;
- destructive infrastructure;
- licence or identity changes.

---

# 13. Evidence, verification and learning

## 13.1 Evidence is not a model claim

“Tests passed” is not evidence.

Evidence includes:

```json
{
  "command": "cargo test -p lmml-mcp",
  "exit_code": 0,
  "stdout_hash": "sha256:...",
  "started_at": "...",
  "completed_at": "...",
  "workspace_revision": "git:...",
  "runner": "sandbox:ws_8842"
}
```

## 13.2 Required engineering evidence

Depending on risk:

- task plan;
- patch;
- command records;
- targeted tests;
- full tests;
- graph impact;
- workflow validation;
- security scan;
- independent review;
- runtime canary;
- benchmark receipt;
- approval record;
- deployment verification.

## 13.3 Independent review

The implementer does not approve its own result. High-risk tasks should use a fresh context and preferably a different harness or model family.

## 13.4 Memory promotion pipeline

```text
session trace
  → compact task summary
  → T3 unreviewed memory
  → repeated evidence
  → candidate lesson
  → scenario tests
  → independent review
  → approved SkillSpec / ADR / rule
```

A model does not receive a direct `memory_promote_to_policy` capability.

## 13.5 Graphify learning versus LMML memory

Graphify navigation lessons remain repository-relative, such as:

- preferred starting nodes;
- prior dead ends;
- paths made stale by code changes.

LMML episodic memory remains task-relative, such as:

- implementation status;
- unresolved decisions;
- useful commands;
- handoff instructions.

Governed knowledge remains separately approved.

## 13.6 Runtime learning

Runtime routing improves from verified empirical records:

- time to first token;
- prompt throughput;
- generation throughput;
- memory use;
- load time;
- canary success;
- cancellation behavior;
- failure rate;
- node-loss behavior;
- benchmark variance.

Learned routing preferences remain policy inputs, not unrestricted self-modification.

---

# 14. State machines

## 14.1 Engineering task

```text
CREATED
  → CONTEXT_READY
  → PLANNED
  → WORKSPACE_READY
  → RUNNING
  → PATCH_READY
  → VERIFYING
       ├── FAILED
       ├── REVISION_REQUIRED
       └── EVIDENCE_COMPLETE
              → APPROVAL_REQUIRED
                   ├── REJECTED
                   └── APPROVED
                          → COMMITTED
                          → DEPLOYED
                          → VERIFIED
```

Terminal states:

```text
CANCELLED
EXPIRED
POLICY_DENIED
ROLLED_BACK
SUPERSEDED
```

## 14.2 Workflow

```text
DRAFT
  → COMPILED
  → VALIDATED
  → TESTED
  → SECURITY_REVIEWED
  → APPROVAL_REQUIRED
  → APPROVED
  → ACTIVATING
  → ACTIVE
  → DRAINING
  → INACTIVE
```

## 14.3 EXO cluster and instance

Cluster:

```text
UNREGISTERED
  → DISCOVERING
  → HEALTHY
       ├── DEGRADED
       └── UNREACHABLE
              → RECOVERING
              → HEALTHY
```

Instance:

```text
ABSENT
  → PLANNING
  → ADMITTED
  → CREATING
  → LOADING
  → WARMING
  → CANARY
  → READY
  → DRAINING
  → DELETING
  → ABSENT
```

## 14.4 Model admission

```text
DISCOVERED
  → QUARANTINE
  → LICENCE_CHECKED
  → HASHED
  → METADATA_VALIDATED
  → CANARY
  → BENCHMARKED
  → APPROVED
  → AVAILABLE
```

---

# 15. Event architecture

Use NATS JetStream for durable internal events.

## 15.1 Task and harness subjects

```text
lmml.task.created
lmml.task.planned
lmml.task.state.changed

lmml.harness.run.requested
lmml.harness.run.started
lmml.harness.run.event
lmml.harness.run.completed
lmml.harness.run.failed

lmml.workspace.created
lmml.workspace.destroyed
```

## 15.2 Context, graph and skill subjects

```text
lmml.context.requested
lmml.context.supplied
lmml.graph.refresh.requested
lmml.graph.refreshed
lmml.graph.impact.created
lmml.skill.loaded
lmml.skill.outcome.recorded
lmml.memory.handoff.saved
```

## 15.3 Evidence and authority subjects

```text
lmml.evidence.submitted
lmml.evidence.verified
lmml.evidence.rejected
lmml.policy.check.requested
lmml.policy.check.completed
lmml.approval.requested
lmml.approval.resolved
```

## 15.4 Workflow subjects

```text
lmml.workflow.draft.created
lmml.workflow.compiled
lmml.workflow.validated
lmml.workflow.tested
lmml.workflow.promotion.requested
lmml.workflow.activated
lmml.workflow.failed
```

## 15.5 Runtime and EXO subjects

```text
lmml.runtime.requested
lmml.runtime.selected
lmml.runtime.request.completed
lmml.runtime.request.failed

lmml.runtime.exo.cluster.discovered
lmml.runtime.exo.cluster.healthy
lmml.runtime.exo.cluster.degraded
lmml.runtime.exo.node.joined
lmml.runtime.exo.node.left
lmml.runtime.exo.node.quarantined
lmml.runtime.exo.placement.previewed
lmml.runtime.exo.placement.rejected
lmml.runtime.exo.instance.creating
lmml.runtime.exo.instance.ready
lmml.runtime.exo.instance.failed
lmml.runtime.exo.request.started
lmml.runtime.exo.request.completed
lmml.runtime.exo.request.cancelled
```

## 15.6 Event envelope

```json
{
  "event_id": "evt_01J...",
  "event_type": "lmml.harness.run.started",
  "occurred_at": "2026-08-05T03:41:10Z",
  "actor": {
    "type": "harness",
    "id": "run_8842",
    "kind": "codex"
  },
  "task_id": "task_8842",
  "correlation_id": "corr_2281",
  "causation_id": "evt_previous",
  "payload": {},
  "signature": "..."
}
```

---

# 16. Storage architecture

| Data | Store |
|---|---|
| Tasks and state | PostgreSQL |
| Harness runs | PostgreSQL |
| Runtime and EXO desired state | PostgreSQL |
| Policies and approvals | PostgreSQL + Git |
| Semantic retrieval | pgvector |
| Graphify artifacts | versioned object storage |
| Evidence blobs | content-addressed object storage |
| Audit index | PostgreSQL |
| Events | NATS JetStream |
| Skills and rules | Git |
| Memories | Markdown/object store + index |
| Secrets | Vault/SOPS/OS secret provider |
| Workflow source | Git |
| Workflow development state | n8n development instance |
| Approved model files | read-only model store |
| Model quarantine | isolated writable store |
| Runtime benchmark history | PostgreSQL + object artifacts |

Content-addressed storage:

```text
sha256/<first-two>/<full-hash>
```

Large logs and model artifacts should be referenced, not duplicated in relational tables.

---

# 17. API design

## 17.1 Core task and harness APIs

```text
POST   /api/tasks
GET    /api/tasks/{task_id}
POST   /api/tasks/{task_id}/plan
POST   /api/tasks/{task_id}/cancel

POST   /api/harness/runs
GET    /api/harness/runs/{run_id}
POST   /api/harness/runs/{run_id}/cancel
GET    /api/harness/runs/{run_id}/events
```

## 17.2 Context and evidence APIs

```text
POST   /api/context/assemble
POST   /api/graph/query
POST   /api/graph/impact
POST   /api/evidence
GET    /api/evidence/bundles/{task_id}
POST   /api/approvals/{approval_id}/resolve
```

## 17.3 Workflow APIs

```text
POST   /api/workflows/compile
POST   /api/workflows/validate
POST   /api/workflows/test
POST   /api/workflows/promote
GET    /api/workflows/{workflow_id}
```

## 17.4 Runtime APIs

```text
GET    /api/runtimes
GET    /api/models
POST   /api/runtime/select
POST   /api/chat/completions
POST   /api/responses
```

## 17.5 EXO administration APIs

```text
GET    /api/runtimes/exo/clusters
POST   /api/runtimes/exo/clusters
GET    /api/runtimes/exo/clusters/{cluster_id}
POST   /api/runtimes/exo/clusters/{cluster_id}/refresh

GET    /api/runtimes/exo/clusters/{cluster_id}/nodes
POST   /api/runtimes/exo/clusters/{cluster_id}/nodes/{node_id}/admit
POST   /api/runtimes/exo/clusters/{cluster_id}/nodes/{node_id}/quarantine

GET    /api/runtimes/exo/clusters/{cluster_id}/models
POST   /api/runtimes/exo/clusters/{cluster_id}/placements/preview

POST   /api/runtimes/exo/clusters/{cluster_id}/instances
GET    /api/runtimes/exo/clusters/{cluster_id}/instances
DELETE /api/runtimes/exo/clusters/{cluster_id}/instances/{instance_id}

POST   /api/runtimes/exo/clusters/{cluster_id}/benchmarks
```

End users never need EXO-specific instance IDs in ordinary chat requests.

---

# 18. MCP surface for coding harnesses

Keep the initial server compact.

## 18.1 Session and task

```text
lmml_session_open
lmml_session_status
lmml_session_checkpoint
lmml_session_complete
lmml_task_get
lmml_task_plan_submit
lmml_task_claim
lmml_task_handoff
```

## 18.2 Context and graph

```text
lmml_context_route
lmml_context_fetch
lmml_context_sources
lmml_graph_query
lmml_graph_node
lmml_graph_neighbors
lmml_graph_path
lmml_graph_change_impact
lmml_graph_refresh
```

## 18.3 Skills and memory

```text
lmml_skill_search
lmml_skill_load
lmml_skill_report_outcome
lmml_memory_search
lmml_memory_read
lmml_memory_save_handoff
```

## 18.4 Evidence

```text
lmml_evidence_submit_command
lmml_evidence_submit_test
lmml_evidence_submit_patch
lmml_evidence_submit_review
lmml_evidence_bundle_status
```

## 18.5 Workflow

```text
lmml_workflow_search_nodes
lmml_workflow_compile_draft
lmml_workflow_validate
lmml_workflow_diff
lmml_workflow_test
```

## 18.6 Runtime

```text
lmml_runtime_models
lmml_runtime_capabilities
lmml_runtime_select
lmml_runtime_benchmark_status
```

Direct EXO instance mutation should not be a general coding-harness tool.

## 18.7 Policy

```text
lmml_policy_explain
lmml_policy_check
lmml_approval_request
```

A model can request approval. It cannot grant it.

---

# 19. User interface

## 19.1 Engineering task view

Display:

- task objective and state;
- current harness workers;
- worktrees;
- context sources and trust classes;
- plan;
- changed files and graph impact;
- tests;
- reviews;
- unresolved issues;
- approval requirements;
- evidence completeness.

## 19.2 Harness view

Display:

- harness type/version;
- role;
- model/runtime used;
- capability grant;
- active commands;
- permission requests;
- token/cost budget;
- checkpoints;
- event stream;
- cancel control.

## 19.3 Workflow view

Display:

- `AutomationSpec`;
- compiled n8n graph;
- validation errors;
- test fixtures;
- data lineage;
- credential aliases;
- security findings;
- diff;
- activation approval.

## 19.4 Runtime view

Display:

- runtime health;
- models;
- capabilities;
- warm instances;
- load and reservations;
- runtime comparison;
- benchmark history.

## 19.5 EXO cluster view

Display:

- cluster ID;
- coordinator and version;
- namespace;
- visible/admitted nodes;
- node platform/backend/memory/network;
- active shards;
- placement previews;
- gate outcomes;
- instance lifecycle;
- canary result;
- benchmark history;
- errors and reconciliation state.

Administrative controls:

```text
Preview
Approve and create
Run canary
Benchmark
Drain
Delete
Quarantine node
View diagnostic events
```

---

# 20. Observability and benchmarking

## 20.1 General metrics

```text
lmml_tasks_total
lmml_harness_runs_total
lmml_harness_failures_total
lmml_context_assembly_seconds
lmml_tool_calls_total
lmml_policy_denials_total
lmml_evidence_incomplete_total
lmml_workflow_validations_total
lmml_runtime_requests_total
lmml_runtime_selection_seconds
```

## 20.2 EXO metrics

```text
lmml_exo_cluster_up
lmml_exo_nodes_total
lmml_exo_nodes_admitted
lmml_exo_nodes_ready
lmml_exo_instances_total
lmml_exo_instance_state
lmml_exo_placement_preview_seconds
lmml_exo_placement_rejections_total
lmml_exo_instance_load_seconds
lmml_exo_canary_failures_total
lmml_exo_requests_total
lmml_exo_request_failures_total
lmml_exo_time_to_first_token_seconds
lmml_exo_prompt_tokens_per_second
lmml_exo_generation_tokens_per_second
lmml_exo_peak_memory_bytes
lmml_exo_cancellations_total
lmml_exo_reconcile_errors_total
```

## 20.3 Benchmark dimensions

```text
model
revision
quantization
context length
generation length
runtime
node set
node count
backend
sharding
network class
warm/cold state
concurrency
tool schema size
thinking enabled/disabled
harness/task profile
```

## 20.4 Required prompt profiles

1. Short chat: 128–512 prompt tokens.
2. Standard synthesis: 2K–8K.
3. RAG synthesis: 8K–32K.
4. Agent tool prompt with large tool schema.
5. Long-context analysis.
6. Concurrent agent requests.
7. Seeded deterministic canary.
8. Code-review workload.
9. Workflow-compilation workload.

## 20.5 Promotion thresholds

A runtime placement becomes production eligible only after:

- repeated successful loads;
- no structural output corruption;
- stable memory use;
- successful cancellation;
- acceptable failure rate;
- acceptable time to first token;
- bounded benchmark variance;
- understood node-loss behavior;
- passing canary suite.

## 20.6 Grafana views

1. Control-plane overview.
2. Task and harness throughput.
3. Evidence and approval status.
4. Workflow operations.
5. Runtime comparison.
6. EXO cluster topology.
7. EXO instance lifecycle.
8. Prompt versus generation throughput.
9. Memory headroom by node.
10. Model-load durations and failures.

---

# 21. Failure handling and resilience

## 21.1 Harness failure

- preserve emitted evidence;
- mark run failed;
- revoke capability grant;
- preserve or freeze worktree;
- generate bounded handoff;
- permit another harness to resume;
- never infer success from partial output.

## 21.2 Context service failure

- fail closed for governed or security-critical context;
- permit explicitly safe local operations where policy allows;
- mark context packet incomplete;
- prevent production commit.

## 21.3 Graph staleness

- record graph snapshot commit;
- detect repository mismatch;
- refresh incrementally;
- mark stale lessons;
- prevent structural closure if required impact evidence is unavailable.

## 21.4 n8n failure

- preserve desired workflow version;
- do not repeat non-idempotent external operations blindly;
- reconcile activation state;
- route notifications through alternate channel where defined;
- preserve workflow execution evidence.

## 21.5 EXO coordinator unavailable

- mark cluster unreachable;
- stop new routing;
- preserve desired instance records;
- allow active streams to terminate naturally;
- retry bounded reads;
- reconcile before recreating anything.

## 21.6 EXO worker disappears

- mark cluster degraded;
- stop new work on affected instances;
- inspect EXO state;
- allow safe upstream recovery;
- quarantine repeatedly unstable nodes;
- never expand placement onto unadmitted nodes.

## 21.7 EXO load timeout

```text
mark TIMEOUT
  → read /state
  → inspect /events
  → collect node health
  → controlled deletion
  → quarantine placement receipt
  → avoid immediate identical retry
```

## 21.8 Stream disconnect

- cancel upstream request;
- close connection;
- record cancellation;
- release accounting;
- verify generation termination where possible.

## 21.9 Policy service unavailable

Fail closed for:

- production deployment;
- workflow activation;
- protected branch merge;
- model or node admission;
- credential use;
- experimental runtime placement.

---

# 22. Testing strategy

## 22.1 Canonical contract tests

Validate versioned schemas for:

- tasks;
- context packets;
- capabilities;
- harness events;
- evidence bundles;
- runtime requests;
- placement receipts;
- policy decisions;
- workflow specifications.

## 22.2 Harness adapter contract tests

Every adapter must pass:

```text
detect installation
render project rules
connect to LMML MCP
open session
load skill
receive context
request tool
handle denial
modify worktree
submit patch
submit test evidence
cancel run
recover after crash
```

## 22.3 Golden generation tests

Given one canonical skill, rule and permission profile, verify generated outputs for Claude Code, Codex and OpenCode.

## 22.4 Graph and context tests

- graph snapshot mismatch;
- inferred versus extracted edge preservation;
- token budgeting;
- provenance retention;
- malicious memory;
- untrusted document attempting policy override;
- stale graph lesson.

## 22.5 Workflow tests

- schema validation;
- workflow diff;
- synthetic execution;
- dangerous Code node;
- unrestricted HTTP target;
- missing idempotency;
- credential alias misuse;
- activation without approval.

## 22.6 EXO contract fixtures

Capture pinned-release fixtures for:

```text
/node_id
/state
/events
/models
/instance/previews
/instance/await SSE
/v1/chat/completions SSE
/bench/chat/completions
```

## 22.7 Integration tests

- coordinator-only startup;
- cluster discovery;
- model listing;
- placement preview;
- small instance creation;
- readiness;
- inference;
- streaming;
- cancellation;
- deletion;
- harness using EXO-hosted model;
- AgentQ routing a review task to EXO.

## 22.8 Chaos tests

- harness crash;
- NATS interruption;
- policy-service interruption;
- coordinator restart;
- worker termination;
- network partition;
- slow worker;
- malformed SSE;
- disk exhaustion;
- duplicate create acknowledgement;
- node returns with changed EXO ID;
- n8n retry during downstream outage.

## 22.9 Security tests

- capability token from wrong workspace;
- expired grant;
- secret-file access;
- protected-branch push;
- prompt instructing policy bypass;
- malicious MCP output;
- malicious hook;
- arbitrary model download;
- remote-code model;
- EXO API external reachability;
- unadmitted node placement;
- production workflow activation by coding harness;
- network exfiltration.

---

# 23. Repository layout

```text
lmml/
├── Cargo.toml
├── AGENTS.md
├── CLAUDE.md
│
├── crates/
│   ├── lmml-protocol/
│   ├── lmml-events/
│   ├── lmml-identity/
│   ├── lmml-capabilities/
│   ├── lmml-policy/
│   ├── lmml-approval/
│   ├── lmml-audit/
│   │
│   ├── lmml-mcp-gateway/
│   ├── lmml-tool-registry/
│   ├── lmml-context/
│   ├── lmml-graph/
│   ├── lmml-memory/
│   ├── lmml-skills/
│   ├── lmml-evidence/
│   │
│   ├── lmml-harness-core/
│   ├── lmml-harness-claude/
│   ├── lmml-harness-codex/
│   ├── lmml-harness-opencode/
│   ├── lmml-harness-generic/
│   ├── lmml-workspaces/
│   ├── lmml-sandbox/
│   │
│   ├── lmml-workflows/
│   ├── lmml-n8n/
│   ├── lmml-agentq/
│   │
│   ├── lmml-runtime-core/
│   ├── lmml-runtime-router/
│   ├── lmml-runtime-llama/
│   ├── lmml-runtime-ollama/
│   ├── lmml-runtime-pulsar/
│   ├── lmml-runtime-colibri/
│   ├── lmml-runtime-exo/
│   │
│   ├── lmml-detect/
│   ├── lmml-compat/
│   ├── lmml-build/
│   ├── lmml-models/
│   ├── lmml-server/
│   └── lmml-state/
│
├── bins/
│   ├── lmml/
│   ├── lmml-control/
│   ├── lmml-node/
│   ├── lmml-harnessd/
│   └── lmml-workspaced/
│
├── services/
│   ├── graphify/
│   ├── n8n-mcp/
│   ├── n8n-runtime/
│   ├── hook-bridge/
│   ├── artifact-store/
│   ├── pulsar-sidecar-adapter/
│   └── exo-compat-adapter/
│
├── adapters/
│   ├── ecc-importer/
│   ├── graphify-adapter/
│   └── n8n-adapter/
│
├── engineering/
│   ├── rules/
│   ├── skills/
│   ├── agents/
│   ├── hooks/
│   ├── permissions/
│   ├── evidence-profiles/
│   └── harness-profiles/
│
├── automation/
├── generated/
│   ├── claude/
│   ├── codex/
│   └── opencode/
│
├── policies/
│   ├── development/
│   ├── staging/
│   └── production/
│
├── schemas/
├── models/
│   ├── manifests/
│   └── admission-receipts/
│
├── deploy/
│   ├── compose/
│   ├── systemd/
│   ├── firewall/
│   └── kubernetes/
│
└── tests/
    ├── protocol/
    ├── harness-contract/
    ├── policy/
    ├── graph/
    ├── workflow/
    ├── runtime/
    ├── exo-contract/
    ├── integration/
    ├── chaos/
    ├── security/
    └── golden/
```

---

# 24. Deployment topology for LMML nodes

Recommended initial role assignment:

## Benji

- LMML control plane;
- API and MCP gateway;
- AgentQ orchestration;
- NATS and policy coordination;
- EXO coordinator in `--no-worker` mode;
- runtime broker;
- not a default inference shard.

## Aurora

- primary local LLM runtime;
- harness worker where appropriate;
- EXO worker only after backend support and benchmark admission;
- model cache.

## Terran

- secondary local LLM runtime;
- parallel harness/agent worker;
- EXO worker only after backend support and admission.

## R9700 node

- ROCm experimentation;
- QLoRA/training work;
- experimental EXO worker only after validated Linux backend;
- never production-promoted from GPU visibility alone.

## Polaris

- CPU laboratory EXO worker;
- retrieval, ingestion, evaluation or low-priority tools in production;
- not automatically placed in latency-sensitive distributed models.

## Nebula

- independent validator;
- fresh-context review;
- canary/evidence verification;
- excluded from primary model shards by default to preserve independence.

## Starlight

- licensing and model-admission receipts;
- capability and entitlement verification;
- no raw model or harness authority beyond its service contract.

---

# 25. Implementation programme

## Phase 0 — Architecture contracts

Deliver:

- ADRs for all responsibility boundaries;
- `lmml-protocol` v0.1;
- trust classes;
- capability model;
- minimal local capability-token enforcement;
- MCP/tool side-effect classes: read, write, admin and external;
- schema validation and secret-denial rules;
- write-scope enforcement rules;
- task, context, evidence, runtime and placement contracts;
- runtime residency and weight transport contracts;
- explicit llama.cpp `LlamaLoadMode` contract;
- threat model;
- version pins.

Exit gate:

- no external vendor type leaks into core services;
- proposal and authority responsibilities are unambiguous.
- local mode has a non-bypassable capability boundary before adapters arrive.

## Phase 1 — Local workspace, evidence and loading foundation

Deliver:

- repository snapshots;
- bare repository manager;
- isolated worktrees;
- sandbox profiles;
- command wrapper;
- patch hashing;
- test evidence parser;
- content-addressed artifact store;
- append-only local event log;
- local review and approval receipt format;
- explicit llama.cpp load mode, defaulting to `mmap`;
- legacy `mlock: bool` migration to `mmap` or `mmap+mlock`;
- local storage capability detection for NVMe, DirectIO and `io_uring`;
- llama.cpp residency feasibility for `mmap`, `mmap+mlock` and `dio`.

Exit gate:

- local mode can prove `task → context → capability → evidence → review → approval → commit` without Graphify, AgentQ, n8n, NATS, PostgreSQL, Pulsar or EXO.

## Phase 2 — Read-only context fabric

Deliver:

- Graphify sidecar and adapter;
- repository snapshots;
- graph query and impact tools;
- semantic RAG integration;
- governed artifact retrieval;
- context token budgets;
- initial MCP gateway backed by local capability grants.

Exit gate:

- a harness can receive a bounded, provenance-labelled context packet without bypassing local capability policy.

## Phase 3 — Harness client adapters

Deliver:

- generated Claude Code bundle;
- generated Codex bundle;
- generated OpenCode bundle;
- session and context MCP tools;
- permission and hook bridge;
- adapter contract tests.

Exit gate:

- all three harnesses can consume the same LMML task, graph and skill contracts.

## Phase 4 — Programmatic harness workers

Deliver:

- Claude driver;
- Codex driver;
- OpenCode driver;
- generic CLI driver;
- normalized events;
- cancellation and recovery;
- cross-harness review.

Exit gate:

- LMML can launch, monitor and replace a harness worker without losing task state.

## Phase 5 — ECC import and skill compiler

Deliver:

- ECC parser and candidate-skill importer;
- security scan;
- tool-name mapping;
- skill versioning;
- harness-specific renderers;
- golden tests.

Exit gate:

- selected ECC disciplines operate as approved LMML-native skills.

## Phase 6 — AgentQ orchestration

Deliver:

- role assignment;
- harness routing;
- model/runtime routing policy;
- parallel workers;
- fresh-context review;
- checkpoint and handoff;
- evidence aggregation.

Exit gate:

- AgentQ deliberately selects workers and runtimes rather than relying on static configuration.

## Phase 7 — n8n-mcp development integration

Deliver:

- node and template search;
- `AutomationSpec` compiler;
- workflow validation;
- diff and synthetic testing;
- security audit;
- development n8n instance;
- workflow-as-code repository structure.

Exit gate:

- a harness can build and validate an inactive workflow without production credentials.

## Phase 8 — Runtime read-only control-plane integration

Deliver:

- EXO version pin;
- typed client;
- `/node_id`, `/state`, `/events`, `/models`;
- cluster registration;
- node mapping;
- state reconciler;
- read-only UI and metrics.
- Pulsar version pin;
- Pulsar OpenAI-compatible server probe;
- Pulsar `/v1/models` catalogue probe;
- Pulsar storage and readiness receipt.

Exit gate:

- LMML accurately represents EXO and Pulsar state without mutating either runtime.

## Phase 9 — EXO placement and instance lifecycle

Deliver:

- placement previews;
- hard gates and ranking;
- placement receipts;
- instance create/await/delete;
- canary suite;
- idempotent reconciliation;
- model registry integration.

Exit gate:

- an approved instance can be safely created, validated, drained and removed.

## Phase 10 — Unified inference routing

Deliver:

- standard chat adapter;
- streaming;
- tool-call mediation;
- cancellation;
- usage normalization;
- fallback among llama.cpp, Ollama, Pulsar, Colibri and EXO;
- Pulsar sidecar adapter through its OpenAI-compatible server;
- runtime strategy manifests for llama.cpp, Pulsar and EXO;
- model registry architecture fields: dense, MoE, hybrid MoE, linear attention and hybrid attention;
- model registry parameter fields: total parameters and active parameters per token;
- AgentQ high-depth Pulsar and EXO routes.

Exit gate:

- EXO and Pulsar behave like standard LMML runtimes while remaining internally composite or streamed.

## Phase 11 — Authority and production hardening

Deliver:

- capability-token enforcement;
- node admission;
- workflow promotion transaction;
- runtime placement approval;
- model allowlists and offline mode;
- private network zones;
- approval UI;
- audit completeness;
- rollback records.

Exit gate:

- coding harnesses, n8n, Pulsar and EXO cannot bypass LMML authority.

## Phase 12 — Observability and empirical routing

Deliver:

- benchmark controller;
- residency benchmark family;
- cold and warm startup benchmarks;
- model load duration;
- page-cache and DirectIO measurements;
- NVMe read measurements;
- streamed-runtime cache hit and miss metrics;
- runtime readiness state: absent, cold, loading, census-building, warm, ready and degraded;
- runtime performance database;
- Grafana dashboards;
- performance-aware routing;
- regression reports;
- chaos tests.

Exit gate:

- runtime, residency and placement decisions are supported by measured evidence.

## Phase 13 — Linux EXO validation

Deliver:

- coordinator-only Benji deployment;
- Linux CPU laboratory cell;
- lifecycle and chaos validation;
- supported or experimental GPU backend gate;
- upstream contribution plan.

Exit gate:

- control-plane integration is proven independently of accelerator support.

## Phase 14 — Verified learning

Deliver:

```text
outcome
  → unreviewed lesson
  → repeated evidence
  → candidate skill/routing change
  → evaluation
  → independent review
  → human approval
  → governed release
```

Exit gate:

- the system can improve without silently mutating its own authority or policy.

---

# 26. Recommended vertical slices

## Slice A — Graph-aware multi-harness change review

```text
Developer or issue
  → LMML task
  → Graphify before-impact
  → harness implementation
  → patch and tests
  → Graphify after-impact
  → different-harness review
  → evidence packet
  → operator decision
```

This proves context, harness, evidence and authority contracts.

## Slice B — Workflow-as-code development

```text
Task
  → automation skill
  → n8n node discovery
  → AutomationSpec
  → n8n-mcp compile/validate
  → synthetic test
  → security audit
  → evidence and approval
```

This proves workflow compilation without production authority.

## Slice C — llama.cpp residency contract

```text
Existing llama.cpp profile
  → migrate mlock bool to LlamaLoadMode
  → emit --load-mode mmap explicitly
  → expose load mode in server UI and diagnostics
  → verify active argv and server readiness
  → record load-mode evidence
```

This proves model residency is an LMML runtime contract rather than an implicit llama.cpp default.

## Slice D — Pulsar sidecar runtime admission

```text
NVIDIA CUDA host with fast NVMe
  → LMML detects storage and PCIe capability
  → Pulsar sidecar registered as OpenAI-compatible runtime
  → model manifest marks MoE active-parameter profile
  → Runtime Broker evaluates streamed-MoE feasibility
  → LMML routes a bounded inference request
  → tool calls remain behind the LMML MCP Gateway
```

This proves streamed model residency without embedding Pulsar internals or bypassing LMML authority.

## Slice E — EXO read-only runtime view

```text
EXO coordinator
  → LMML typed client
  → state reconciliation
  → node mapping
  → model catalogue
  → runtime UI and metrics
```

This proves distributed-runtime control-plane integration without mutation.

## Slice F — Harness task using EXO model

```text
Architecture-review task
  → AgentQ selects reviewer role
  → Runtime Broker selects approved EXO model
  → harness receives context
  → EXO performs distributed inference
  → review evidence returned
  → independent validator checks result
```

This proves the complete path from coding harness to distributed model runtime.

## Slice G — Event-to-PR pipeline

```text
GitHub issue
  → n8n normalizes event
  → LMML TaskSpec
  → Graphify context
  → harness workers
  → test and review evidence
  → n8n creates draft PR
  → human approval
```

This proves all major planes without automatic protected-branch merge.

---

# 27. Recommended commit sequence

```text
feat(protocol): add canonical task context capability and evidence contracts
feat(identity): add principal node harness and runtime identities
feat(policy): add capability grants and progressive closure checks
feat(events): add append-only local event envelope

feat(workspace): add isolated git worktree manager
feat(evidence): add command test patch and review evidence
feat(runtime): add LlamaLoadMode and explicit llama.cpp --load-mode mmap
feat(runtime): migrate legacy mlock to mmap or mmap+mlock
feat(detect): add NVMe DirectIO io_uring and PCIe capability probes

feat(events): add versioned NATS event envelope for fabric mode
feat(graph): add Graphify adapter and structural evidence types
feat(context): add provenance-aware context assembler
feat(mcp): add lazy LMML engineering MCP gateway

feat(harness): add common harness driver contract
feat(harness): add Claude Code adapter
feat(harness): add Codex adapter
feat(harness): add OpenCode adapter
feat(harness): add hook bridge and normalized events

feat(skills): add SkillSpec registry and renderer
feat(ecc): add reviewed ECC asset importer
feat(agentq): add role and harness routing
feat(agentq): add independent review and handoff

feat(workflow): add AutomationSpec
feat(n8n): add n8n-mcp development adapter
feat(n8n): add validation diff and synthetic execution

feat(runtime): add canonical runtime broker
feat(runtime): add runtime capability and model registry
feat(runtime): add residency and weight transport strategy manifests
feat(pulsar): add OpenAI-compatible sidecar runtime adapter
feat(exo): add configuration and typed domain types
feat(exo): add control-plane client
feat(exo): add cluster reconciler
feat(exo): add node identity and admission mapping
feat(exo): add placement preview and policy gates
feat(exo): add instance lifecycle and readiness SSE
feat(exo): add inference streaming and cancellation
feat(exo): add model canaries and benchmark ingestion

feat(authority): add production approval transactions
feat(security): add private runtime and workflow network policy
feat(metrics): add unified residency Pulsar and EXO Prometheus metrics
feat(ui): add engineering workflow runtime residency Pulsar and EXO panels

test(harness): add cross-adapter contract and golden tests
test(workflow): add n8n security and promotion tests
test(runtime): add llama.cpp load-mode and residency contract fixtures
test(pulsar): add sidecar admission and authority-gateway fixtures
test(exo): add contract integration chaos and security fixtures
docs: add sovereign engineering and distributed runtime guide
```

---

# 28. Unified definition of done

The architecture reaches controlled production readiness when:

1. Claude Code, Codex and OpenCode connect to the same LMML MCP gateway.
2. One canonical skill compiles reproducibly into all supported harness formats.
3. Every harness operates only inside an assigned workspace.
4. LMML enforces tool capability even if a harness permission is weakened.
5. Graphify impact evidence is attached to non-trivial code changes.
6. Commands and tests include workspace revision and runner provenance.
7. An implementer cannot approve its own patch.
8. n8n production credentials are never exposed to coding harnesses or models.
9. Workflow activation requires a separate authority transaction.
10. A harness crash does not destroy task state or existing evidence.
11. Another harness can resume from a bounded handoff.
12. AgentQ can replay a task with another harness for evaluation.
13. ECC updates cannot change approved skills without review.
14. llama.cpp load mode is explicit and observable in config, launch argv and diagnostics.
15. Legacy `mlock` config migrates to `mmap` or `mmap+mlock` without changing user intent.
16. Runtime manifests separate topology, residency, transport and cache policy.
17. Pulsar is represented as a logical LMML runtime without exposing its MCP path around LMML authority.
18. EXO clusters are represented as logical LMML runtimes.
19. Every EXO node maps to an admitted LMML identity.
20. Users cannot access EXO administrative operations directly.
21. LMML can explain every accepted or rejected EXO placement.
22. EXO instance creation is idempotent and recoverable.
23. Every EXO model instance passes LMML canaries before routing.
24. Streaming and cancellation work through standard LMML APIs.
25. Tool execution remains governed by LMML regardless of runtime.
26. Unsupported Linux GPU workers cannot be accidentally promoted.
27. llama.cpp, Ollama, Pulsar, EXO and Colibri remain available according to measured eligibility and policy.
28. Runtime selection uses current health, residency state and measured evidence.
29. Node loss causes controlled degradation rather than unauthorized reconfiguration.
30. Model, workflow and code promotion all produce immutable evidence and approval records.
31. Prompt-injected content cannot expand capability or mutate governed policy.
32. Protected-branch and production actions remain impossible without global closure.
33. Runtime adapter upgrades pass schema, canary, benchmark and security gates.
34. The integrated system demonstrates measurable engineering or model-capacity benefit over the unintegrated baseline.

---

# 29. Decisions and anti-patterns

## Adopt

- canonical LMML contracts;
- adapter boundaries;
- one primary harness-facing MCP gateway;
- dynamic tool hydration;
- isolated worktrees;
- evidence-first completion;
- graph-aware context;
- approved ECC-derived skills;
- cross-harness review;
- workflow-as-code;
- runtime-neutral model routing;
- explicit model residency and weight transport policy;
- llama.cpp load-mode contracts;
- Pulsar as sidecar-first streamed-MoE runtime;
- EXO as a composite runtime;
- node and model admission;
- progressive closure;
- separate proposal and commit authority.

## Reject

- connecting every harness directly to every backend;
- giving every session the full MCP tool catalogue;
- importing all ECC assets into every context;
- treating memory as policy;
- treating inferred graph edges as fact;
- letting n8n become LMML memory or authority;
- relying on implicit runtime defaults for model loading;
- treating `mmap`, DirectIO, NVMe streaming and distributed sharding as the same mechanism;
- allowing runtime-local MCP loops to bypass the LMML MCP Gateway;
- promoting Pulsar on non-CUDA or unmeasured-storage hosts;
- letting EXO discovery grant production admission;
- treating visible Linux GPUs as supported EXO workers;
- exposing EXO administrative APIs to ordinary users;
- using model claims as evidence;
- allowing implementers to approve themselves;
- hard-coding one harness or model provider;
- forking EXO’s scheduler before exhausting adapter/backend contribution paths;
- beginning with mixed-vendor tensor parallelism;
- allowing production activation without an explicit transaction and rollback plan.

---

# 30. Final operating model

```text
LMML
  = sovereign engineering, context, policy and runtime control plane

AgentQ
  = task and worker orchestrator

Claude Code / Codex / OpenCode
  = replaceable interactive and autonomous engineering workers

Graphify
  = structural knowledge and change-impact engine

ECC
  = upstream engineering skills and discipline source

LMML Context Fabric
  = provenance-aware fusion of graph, RAG, memory, policy and live state

n8n-mcp
  = workflow intelligence, compiler, validator and repair service

n8n
  = durable external event and integration runtime

Runtime Broker
  = model and execution-class selector

Residency / Transport Engine
  = model weight location, movement, cache and placement policy

llama.cpp / Ollama
  = local fast and convenient inference

Pulsar
  = sidecar-first NVMe-streamed MoE runtime

Colibri
  = sparse, huge or out-of-core specialist runtime

EXO
  = approved distributed dense-model execution fabric

ASTRA Authority Gate
  = deterministic boundary for high-consequence transitions

Evidence Ledger
  = proof of what was proposed, executed, verified and approved
```

The strategic boundary is:

> **LMML decides what work should occur, what context may be used, which capabilities are granted, which model/runtime should execute, how model weights may be resident or transported, which nodes may participate, what evidence is required and whether the result may commit. Harnesses perform engineering work. n8n performs durable integrations. Pulsar streams approved local sparse-model weights. EXO divides and executes an approved model across an approved cell. None of them can independently enlarge their authority.**

This architecture turns LMML into a structurally aware, multi-harness, workflow-capable and distributed-intelligence operating fabric while preserving local sovereignty, replaceability, auditability and deterministic authority separation.
