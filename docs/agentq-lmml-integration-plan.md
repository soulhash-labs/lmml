# AgentQ and LMML Integration Plan

Date: 2026-07-13

## Summary Decision

Do not integrate the ZIP code directly into `/home/angelo/repos/agentq`.

Use the ZIP files as design/reference material only. The useful product direction is valid: add a headless LMML node API and let AgentQ discover and route to those LMML nodes. The supplied code packs are not merge-ready, and the AgentQ repo already contains the canonical Python protocol implementation plus a Rust `agentq-core` engine. Copying the ZIP protocol/orchestrator crates into AgentQ would duplicate existing concepts and introduce incompatible behavior.

The clean integration boundary is:

- `lmml` owns llama.cpp build, server lifecycle, model inventory, hardware detection, stable node HTTP APIs, and the AgentQ bridge endpoints exposed by LMML workers.
- `agentq` owns AgentQ packet semantics, identity, permissions, daemon registry, scheduling, command governance, routing, and operator/brain logic.
- AgentQ should call LMML nodes through a small Python client/adapter, not absorb the LMML node runtime.
- LMML should not duplicate AgentQ orchestration. It should expose safe, stable compute endpoints.

## Reviewed Artifacts

Local LMML artifacts:

- `crown11-agentq-lan-swarm.zip`
- `lmml-node-update.zip`
- `lmml-agentq-wired.zip`
- `production Rust workspace CROWN11 agentq swarm.md`
- `wired LMML + AgentQ Rust pack.md`

Current LMML workspace:

- `Cargo.toml`
- `crates/lmml-detect`
- `crates/lmml-build`
- `crates/lmml-server`
- `crates/lmml-state`
- `crates/lmml-tui`

AgentQ repo:

- `/home/angelo/repos/agentq/pyproject.toml`
- `/home/angelo/repos/agentq/src/agentq/protocol/agent_protocol.py`
- `/home/angelo/repos/agentq/src/agentq/daemon/agentqd.py`
- `/home/angelo/repos/agentq/src/agentq/daemon/api.py`
- `/home/angelo/repos/agentq/src/agentq/daemon/registry.py`
- `/home/angelo/repos/agentq/src/agentq/daemon/announce.py`
- `/home/angelo/repos/agentq/src/agentq/routing/routing_bridge.py`
- `/home/angelo/repos/agentq/src/agentq/compute/provider.py`
- `/home/angelo/repos/agentq/src/agentq/scheduling/bus.py`
- `/home/angelo/repos/agentq/engine/Cargo.toml`
- `/home/angelo/repos/agentq/engine/agentq-core/src/lib.rs`
- `/home/angelo/repos/agentq/engine/agentq-core/src/packet.rs`

## Current State

### LMML

LMML is already a multi-crate Rust workspace:

- `lmml-detect` probes toolchain, CUDA, Vulkan, CPU, RAM, disk, and recommends `BuildBackend`.
- `lmml-build` maps `BuildBackend` to CMake flags and verifies llama.cpp binaries.
- `lmml-server` owns `llama-server` process lifecycle, port checks, readiness polling, graceful shutdown, and runtime monitoring.
- `lmml-compat` owns llama.cpp CLI compatibility and flag spelling.
- `lmml-tui` is currently the default binary.

Baseline verification:

```sh
cargo check --workspace
```

Result: passes in the current lmml workspace.

### AgentQ

AgentQ is primarily a Python package with a separate Rust engine workspace under `engine/`.

Important existing AgentQ capabilities:

- `src/agentq/protocol/agent_protocol.py` already implements the 31-byte packet format, 25-byte identity block, packet encode/decode, checksum behavior, permission bitmasks, mode restrictions, and evolution permissions.
- `src/agentq/__init__.py` exports the protocol types and functions as the canonical Python API.
- `src/agentq/daemon/registry.py` stores `AgentIdentity` as the source of truth for daemon agent records.
- `src/agentq/daemon/announce.py` sends AgentQ packet heartbeats over UDP multicast.
- `src/agentq/routing/routing_bridge.py` already bridges routing decisions to AgentQ packets and permission checks.
- `src/agentq/scheduling/bus.py` provides an in-process `ComputeBus`, but it does not yet route work to remote LMML node HTTP APIs.
- `engine/agentq-core` already has a no-std Rust engine and a 31-byte packet header type. It is not a daemon/runtime equivalent to the ZIP CROWN11 workspace.

AgentQ worktree status is dirty with many existing modified and untracked files. Any future AgentQ edits should be made in a dedicated branch after preserving or understanding that state.

## ZIP Pack Findings

### `lmml-agentq-wired.zip`

Intent:

- Adds `lmml-api`
- Adds `lmml-node`
- Adds `lmml-agentq`
- Adds `crown-agentq-protocol`
- Adds `crown-agentq-crown`
- Adds AgentQ endpoints to LMML node:
  - `GET /v1/agentq/identity`
  - `POST /v1/agentq/packet`
  - `POST /v1/agentq/packet/json`
  - `POST /v1/agentq/infer`
  - `GET /v1/crown/status`

Compile status:

```sh
cd /tmp/lmml-upgrade-review/lmml-agentq-wired
cargo check --workspace
```

Result: fails. `crown-agentq-protocol/src/permissions.rs` does not cover `Action::Reject`.

Other issues:

- Uses `async-trait` in workspace dependencies, which conflicts with the lmml rule forbidding `#[async_trait]`. The dependency may not be used, but it should not be introduced.
- `lmml-node/src/load.rs` imports removed `sysinfo::{CpuExt, SystemExt}` traits.
- `lmml-node/src/managed.rs` reimplements a weaker server manager instead of reusing `lmml-server`.
- `NodeConfig::default` binds to `0.0.0.0:8101` with `api_key = None`, which exposes inference endpoints on a LAN by default.
- AgentQ packet permission checks are based on self-asserted packet identity scopes plus optional HTTP bearer auth. That is not strong authentication.
- The root Cargo patch only adds `lmml-api` and `lmml-node`, but the wired pack also needs `lmml-agentq`, `crown-agentq-protocol`, and `crown-agentq-crown`.

### `crown11-agentq-lan-swarm.zip`

Intent:

- Standalone Rust CROWN11/AgentQ daemon/orchestrator workspace.
- Includes protocol, CROWN graph, compute router, MCTS, orchestrator, and daemon crates.

Compile status:

```sh
cd /tmp/lmml-upgrade-review/crown11-agentq-lan-swarm
cargo check --workspace
```

Result: fails with the same `Action::Reject` non-exhaustive match.

Integration assessment:

- Do not merge this into AgentQ wholesale. AgentQ already has daemon, registry, packet protocol, routing, scheduling, and Crown/brain concepts in Python and Rust engine modules.
- Treat this pack as a design sketch for possible future Rust services only after reconciling with AgentQ's existing Python daemon and `agentq-core` engine.

### `lmml-node-update.zip`

Intent:

- Adds a headless LMML node API without AgentQ wiring.
- Adds shared DTO crate `lmml-api`.

Compile status:

```sh
cd /tmp/lmml-upgrade-review/lmml-node-update
cargo check --workspace
```

Result: fails because `lmml-node/src/load.rs` imports `sysinfo::{CpuExt, SystemExt}` with a resolved `sysinfo` version where those root traits are unavailable.

Integration assessment:

- This is the best conceptual starting point, but not a drop-in patch.
- Rebuild the node API around current lmml crates instead of copying it unchanged.

## Cross-Repo Integration Decision

### Should ZIP code be integrated into AgentQ?

No, not directly.

Reasons:

1. AgentQ already has the canonical packet protocol in `agent_protocol.py`.
2. AgentQ already exports `AgentIdentity`, `create_packet`, `decode_packet`, `PermissionRegistry`, and `PermissionEnforcer`.
3. AgentQ already has daemon registry, announce, routing, governance, and compute scheduling modules.
4. `engine/agentq-core` already provides a Rust no-std execution engine and packet header support.
5. The ZIP protocol crate is not source-compatible with AgentQ's Python permission matrix:
   - ZIP Rust derives broad generic permissions for every action/destination.
   - AgentQ Python uses a sparse explicit permission matrix and returns allow-by-default when no specific requirement exists.
6. The ZIP Rust code currently does not compile.
7. Importing it would create two AgentQ protocol sources of truth.

### What should be integrated into AgentQ?

Add only an AgentQ-side LMML node client/adapter:

- `src/agentq/lmml/node_client.py`
- `src/agentq/lmml/node_registry.py`
- optional `src/agentq/lmml/routing.py`
- tests under `tests/test_lmml_node_client.py` and `tests/test_lmml_node_routing.py`

Responsibilities:

- Discover or accept configured LMML nodes.
- Call `GET /v1/health`.
- Call `GET /v1/capabilities`.
- Call `GET /v1/load`.
- Call `POST /v1/infer`.
- Optionally call `POST /v1/agentq/infer` once LMML exposes it.
- Convert LMML capabilities into AgentQ daemon registry capability records.
- Feed LMML node load/capability into existing AgentQ routing and `ComputeBus` decisions.

Do not duplicate packet encoding in this adapter. Use existing AgentQ Python APIs for AgentQ packets.

## Target Architecture

```text
AgentQ daemon / router
  |
  | Python client calls stable HTTP DTOs
  v
LMML node API
  |
  | lmml-server manages process lifecycle
  v
local llama-server
  |
  v
GGUF model on local hardware
```

AgentQ-to-LMML responsibilities:

- AgentQ selects the right node and task shape.
- AgentQ sends a normalized inference request to LMML.
- LMML handles local model/runtime details.
- LMML returns stable inference/load/health DTOs.

LMML-to-AgentQ responsibilities:

- LMML optionally advertises AgentQ-compatible identity and node metadata.
- LMML optionally accepts AgentQ inference envelopes.
- LMML does not run AgentQ's planner or daemon logic.

## Detailed Implementation Plan

### Phase 0: Stabilize Inputs Before Any Merge

Goal: prevent broken code from entering either repo.

Tasks:

1. Keep ZIP extraction in `/tmp` only.
2. Do not apply ZIP patches directly.
3. Record current LMML baseline:

   ```sh
   cargo check --workspace
   ```

4. Record current AgentQ baseline in a separate AgentQ task before editing that repo:

   ```sh
   cd /home/angelo/repos/agentq
   python3 -m pytest tests/test_governance.py tests/test_glyphos_router.py
   cd engine
   cargo check --workspace
   ```

5. Preserve the dirty AgentQ worktree before making AgentQ changes:

   ```sh
   git -C /home/angelo/repos/agentq status --short
   ```

Deliverable:

- This plan file.
- No code merged from ZIP packs.

### Phase 1: Add LMML API Contracts

Goal: create a stable DTO crate used by LMML node, future LMML TUI cluster views, and AgentQ Python clients.

LMML changes:

1. Add `crates/lmml-api`.
2. Start with the useful DTOs from the ZIP, adapted to current lmml style:
   - `HealthResponse`
   - `NodeCapabilities`
   - `LoadResponse`
   - `ModelDescriptor`
   - `GpuDescriptor`
   - `InferRequest`
   - `InferResponse`
   - `EmbeddingRequest`
   - `EmbeddingResponse`
   - `ServerControlRequest`
   - `ServerControlResponse`
   - `ErrorResponse`
3. Use per-crate dependencies, consistent with current LMML manifests.
4. Use `thiserror = "2"` if needed, consistent with current crates.
5. Add API/protocol version fields before AgentQ integration:
   - `NodeCapabilities.api_version`
   - `NodeCapabilities.lmml_version`
   - `NodeCapabilities.llama_cpp_commit`
   - `NodeCapabilities.auth_required`
6. Add request IDs to inference contracts:
   - `InferRequest.request_id`
   - `InferResponse.request_id`
7. Treat `/v1/infer` as the stable LMML-native contract. Keep `/v1/chat/completions` as proxy compatibility only.
8. Add doc comments for every public item.
9. Add serde round-trip unit tests.

Do not add:

- `async-trait`
- AgentQ packet dependencies
- server process management

Verification:

```sh
cargo fmt
cargo test -p lmml-api
cargo check --workspace
cargo clippy -p lmml -- -D warnings
```

Note: the final clippy package name may need adjustment if the root package remains `lmml-tui` only.

### Phase 2: Add Headless LMML Node Binary

Goal: expose a safe local/LAN HTTP API for LMML workers.

Split this into smaller mergeable milestones. Do not build inference proxying,
server control, and AgentQ bridge behavior in one patch.

#### Phase 2A: Health, Capabilities, and Load

Goal: expose safe read-only node state.

1. Add `crates/lmml-node`.
2. Add it to root workspace members.
3. Keep `lmml-tui` as `default-members`.
4. Implement read-only endpoints:
   - `GET /v1/health`
   - `GET /v1/capabilities`
   - `GET /v1/load`
   - `GET /v1/models`
5. Reuse `lmml-models` for local model inventory when practical.
6. Reuse `lmml-detect` for hardware/backend detection.
7. Reuse `lmml-state` or a node-specific config module for persisted node configuration.

#### Phase 2B: Stable Inference Proxy

Goal: make LMML nodes useful as compute workers while keeping `/v1/infer`
canonical.

1. Implement stable LMML-native inference:
   - `POST /v1/infer`
2. Add proxy compatibility endpoints only after `/v1/infer` works:
   - `POST /v1/chat/completions`
   - `POST /v1/embeddings`
3. Keep `/v1/chat/completions` a pass-through compatibility surface, not the stable internal AgentQ/LMML contract.
4. Propagate request IDs from `InferRequest` to `InferResponse`.

#### Phase 2C: Explicit Server Control

Goal: expose controlled lifecycle operations without making them available by
default.

1. Implement only after Phase 2A and Phase 2B pass.
2. Add `POST /v1/server/control`.
3. Keep `POST /v1/server/control` disabled by default.
4. Require an explicit config flag such as `supports_server_control = true`.
5. Keep server-control routes behind bearer auth when enabled.
6. Reuse `lmml-server` for managed `llama-server` lifecycle.
7. Reuse `lmml-compat` for llama.cpp CLI flag generation.

Security defaults:

- Default bind should be `127.0.0.1:8101`.
- If `bind` is non-local, require an API key unless an explicit unsafe development flag is set.
- Do not expose unauthenticated `/v1/infer` on `0.0.0.0`.
- Keep `/v1/health` optionally public only if it returns no sensitive model paths.
- Use constant-time API key comparison.
- Redact API keys from logs.

Managed server behavior:

- Use `lmml-server::ServerManager`.
- Check port availability before spawn.
- Poll `/health` and `/v1/health` until ready.
- Stop with graceful shutdown semantics.
- Stream logs through channels or tracing.

Tests:

- Config defaults: local bind, auth behavior.
- Auth middleware: allowed/denied cases.
- `/v1/health`: degraded when llama-server unavailable.
- `/v1/capabilities`: includes `lmml_version`, `api_version`, `llama_cpp_commit`, and `auth_required`.
- `/v1/infer`: rejects empty prompt.
- `/v1/infer`: preserves or assigns request IDs.
- Server control disabled by default.
- DTO serialization with `lmml-api`.

Verification:

```sh
cargo fmt
cargo test -p lmml-node
cargo check --workspace
cargo clippy -p lmml-node -- -D warnings
```

### Phase 3: AgentQ-Side LMML Client

Goal: let AgentQ use LMML nodes without importing the Rust ZIP workspaces.

AgentQ changes:

1. Add `src/agentq/lmml/node_client.py`.
2. Add `src/agentq/lmml/node_registry.py`.
3. Add tests:
   - `tests/test_lmml_node_client.py`
   - `tests/test_lmml_node_routing.py`
4. Keep dependencies stdlib-only if possible:
   - use `urllib.request`
   - use `dataclasses`
   - use `json`
   - use `typing`
5. Support optional bearer token.
6. Implement:
   - `health()`
   - `capabilities()`
   - `load()`
   - `infer()`
   - `score_node_for_task()`
7. Map LMML `NodeCapabilities` into AgentQ registry `capability` dictionaries.
8. Integrate with AgentQ daemon through existing patterns:
   - add a local config section for LMML nodes
   - surface node status through existing daemon API or monitor panels
   - feed node capabilities into `CapabilityIndex` where useful

Do not:

- Reimplement AgentQ packet encoding.
- Add the ZIP Rust protocol crate to AgentQ.
- Replace AgentQ `ComputeBus`.

Validation:

```sh
cd /home/angelo/repos/agentq
python3 -m pytest tests/test_lmml_node_client.py tests/test_lmml_node_routing.py
```

### Phase 4: Optional AgentQ Bridge Endpoints in LMML

Goal: let LMML nodes accept AgentQ-shaped requests after the base node API is safe.

LMML changes:

1. Add `crates/lmml-agentq` only after Phase 2 passes.
2. Prefer one of these approaches:
   - Implement only JSON AgentQ envelopes in LMML, leaving binary packet construction to AgentQ.
   - If binary packets are required, generate or port a minimal Rust implementation from AgentQ's canonical protocol tests, not from the broken ZIP crate unchanged.
3. Add endpoints:
   - `GET /v1/agentq/identity`
   - `POST /v1/agentq/infer`
   - optionally `POST /v1/agentq/packet`
   - optionally `POST /v1/agentq/packet/json`
   - `GET /v1/crown/status`
4. Add cross-language parity tests:
   - Python AgentQ creates a packet.
   - Rust LMML decodes it.
   - Rust LMML returns an encoded packet.
   - Python AgentQ decodes the reply.

Important compatibility decision:

- AgentQ Python currently uses an explicit sparse permission matrix where unknown action/destination pairs can be allowed.
- ZIP Rust uses generic action and destination requirements, which is stricter and behaviorally different.
- Pick one canonical policy before enabling binary packet routes. The recommended source of truth is AgentQ Python's current behavior, because that is what the installed AgentQ package and tests already use.

Security:

- Do not treat packet identity as authentication.
- Require HTTP bearer auth for non-local packet endpoints.
- Consider later signed packet identities or a trusted-node registry.

Validation:

```sh
cargo test -p lmml-agentq
cargo check --workspace
cd /home/angelo/repos/agentq
python3 -m pytest tests/test_lmml_agentq_bridge.py
```

### Phase 5: HIP/ROCm Backend Support

Goal: add AMD ROCm/HIP support for BC-250-style nodes without disturbing existing Vulkan support.

This phase is useful but orthogonal to AgentQ integration. Keep it separate
from LMML node API and AgentQ client work so backend probing risk does not
block the node protocol.

Current state:

- LMML already supports `BuildBackend::Vulkan`.
- `lmml-build` already maps Vulkan to `-DGGML_VULKAN=ON`.
- The ZIP patch's Vulkan addition is stale.

Tasks:

1. Add `BuildBackend::Hip { targets: Vec<String> }`.
2. Detect ROCm/HIP tools in `lmml-detect`.
3. Prefer HIP only when ROCm is healthy and target detection is reliable.
4. Keep Vulkan fallback for AMD nodes when ROCm is absent or brittle.
5. Map HIP in `lmml-build`:

   ```text
   -DGGML_HIP=ON
   -DAMDGPU_TARGETS=<targets>
   ```

6. Update TUI backend serialization/state helpers:
   - backend name
   - backend arch/target persistence
   - backend cycling
   - fallback backend logic
7. Add tests and snapshots for HIP detection and display.

Validation:

```sh
cargo test -p lmml-detect
cargo test -p lmml-build
cargo test -p lmml-tui
cargo fmt
cargo clippy -p lmml -- -D warnings
```

### Phase 6: TUI Cluster View

Goal: expose LMML node state to human operators.

LMML changes:

1. Add a cluster tab or dashboard section in `lmml-tui`.
2. Consume `lmml-api` DTOs.
3. Show:
   - node name
   - status
   - backend
   - model list
   - current load
   - active requests
   - AgentQ capability flag if enabled
4. Keep TUI coordination through `App::update()`.
5. Add snapshot tests.

Validation:

```sh
cargo test -p lmml-tui
cargo insta pending-snapshots
cargo insta accept
cargo fmt
cargo clippy -p lmml -- -D warnings
```

## Code That Should Not Be Copied

Do not copy these ZIP parts directly:

- `crown-agentq-protocol` into AgentQ: duplicates `agent_protocol.py` and currently fails to compile.
- `crown-agentq-crown` into AgentQ wholesale: AgentQ already has Crown/brain/engine modules and tests; selectively compare concepts instead.
- `crown-agentq-orchestrator` into AgentQ: AgentQ already has daemon/routing/scheduling/governance machinery.
- `crown-agentqd` into AgentQ: duplicates Python `agentqd`.
- `lmml-node/src/managed.rs`: weaker than existing `lmml-server`.
- ZIP root workspace dependency layout: does not match current LMML per-crate manifest style.

## Code Worth Harvesting Selectively

Useful ideas from ZIP packs:

- `lmml-api` DTO shape, after cleanup and doc comments.
- `lmml-node` route list and request/response concepts.
- AgentQ capability advertisement fields in `NodeCapabilities`.
- AgentQ JSON inference envelope shape.
- CROWN node metadata endpoint shape.
- Example node config fields, after secure defaults.

Fix before harvesting:

- Remove `async-trait`.
- Fix `Action::Reject`.
- Fix `sysinfo` imports.
- Replace `ManagedServer` with `lmml-server`.
- Default to localhost bind or required auth.
- Align permission behavior with AgentQ Python.

## Compatibility Notes

### Packet Format

AgentQ Python and ZIP Rust agree broadly on:

- 31-byte overhead
- 6-byte header
- 25-byte identity block
- `u16` payload length
- optional 4-byte MD5 checksum

But AgentQ Python is the canonical implementation in the installed package.

### Permission Model

AgentQ Python:

- Uses explicit `PERMISSION_MATRIX`.
- Unknown action/destination pairs can be allowed because required permissions are `0`.
- Mode restrictions apply only to required permissions.

ZIP Rust:

- Assigns generic required permissions for every action and every destination.
- Is stricter for many routes.
- Currently misses `Action::Reject`.

Recommendation:

- Add parity tests before enabling Rust binary packet handling in LMML.
- Keep AgentQ Python as policy source of truth unless intentionally changing AgentQ semantics.

### Authentication

AgentQ packet identity is not authentication.

Required production behavior:

- HTTP bearer auth for LMML node APIs exposed beyond localhost.
- Optional later packet signatures.
- Optional trusted AgentQ node registry.
- No unauthenticated inference on LAN by default.

## Verification Matrix

LMML after Phase 1:

```sh
cargo test -p lmml-api
cargo check --workspace
```

LMML after Phase 2:

```sh
cargo test -p lmml-node
cargo test -p lmml-server
cargo check --workspace
```

AgentQ after Phase 3:

```sh
cd /home/angelo/repos/agentq
python3 -m pytest tests/test_lmml_node_client.py tests/test_lmml_node_routing.py
python3 -m pytest tests/test_governance.py tests/test_glyphos_router.py
```

Cross-language bridge after Phase 4:

```sh
cargo test -p lmml-agentq
cd /home/angelo/repos/agentq
python3 -m pytest tests/test_lmml_agentq_bridge.py
```

Full LMML check before merging:

```sh
cargo fmt
cargo clippy -p lmml -- -D warnings
```

Do not rerun tests after formatting/linting unless formatting changes generated files or snapshots.

## Proposed Milestones

1. `M1`: `lmml-api` merged with tests.
2. `M2`: `lmml-node` exposes secure local endpoints and proxies inference.
3. `M3`: AgentQ Python client can call LMML node health/capabilities/infer.
4. `M4`: AgentQ daemon can register LMML nodes as compute-capable records.
5. `M5`: Optional LMML AgentQ bridge endpoints with Python/Rust packet parity tests.
6. `M6`: HIP/ROCm backend support and BC-250 profile validation.
7. `M7`: LMML TUI cluster view.

## Immediate Next Step

Start with `M1` in the LMML repo. Do not touch AgentQ until LMML has a stable node API to consume.

The first concrete implementation task should be:

```text
Add crates/lmml-api with stable DTOs, version fields, and request IDs.
```

After `lmml-api` lands, implement `lmml-node` Phase 2A first: `/v1/health`, `/v1/capabilities`, `/v1/load`, and `/v1/models`. Once `lmml-node` has a working `/v1/health`, `/v1/capabilities`, and `/v1/infer`, add the AgentQ Python client in a separate AgentQ branch.
