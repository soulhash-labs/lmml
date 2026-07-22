# lmml Plan

This document is the public-safe architecture and delivery plan for `lmml`.
It replaces an older internal planning note that mixed current design, host-local
validation notes, and historical milestone status.

Sanitization rules for this document:

- Do not include personal filesystem paths, private hostnames, real LAN
  addresses, credentials, access tokens, or deployment-specific ports beyond
  documented defaults.
- Do not include proprietary engine details or unrelated project internals.
- Keep release claims narrow: a target is release-ready only after it is built
  and validated on matching hardware or CI.
- Keep examples generic and reproducible.

## Goal

`lmml` is a Rust toolkit for managing `llama.cpp` locally. It detects hardware,
builds or installs `llama.cpp`, manages GGUF models, runs `llama-server`,
exposes stable local node APIs, and optionally routes requests across trusted
LAN workers.

The main user-facing binary is `lmml`. Headless and LAN flows use `lmml-node`
and `lmml-router`.

## Current Workspace

```text
lmml/
|-- Cargo.toml
|-- crates/
|   |-- lmml-api       # stable node/router DTOs and wire contracts
|   |-- lmml-build     # llama.cpp clone/build/fingerprint logic
|   |-- lmml-compat    # llama.cpp flag and capability compatibility
|   |-- lmml-detect    # hardware and prerequisite detection
|   |-- lmml-models    # GGUF registry, metadata, search, downloads
|   |-- lmml-node      # authenticated worker API around llama-server
|   |-- lmml-router    # LAN coordinator and load-aware route selection
|   |-- lmml-server    # llama-server lifecycle and health management
|   |-- lmml-state     # persistent state and runtime profiles
|   `-- lmml-tui       # TUI binary and runtime CLI helpers
|-- docs/
`-- scripts/
```

Crate boundary rules:

- `lmml-tui` orchestrates user workflows and should not own low-level probe,
  build, model, or server logic.
- `lmml-detect` owns local hardware/prerequisite detection only.
- `lmml-build` owns CMake argument generation and build fingerprints.
- `lmml-compat` owns llama.cpp flag compatibility.
- `lmml-server` owns local `llama-server` process lifecycle.
- `lmml-node` owns node API behavior and proxy compatibility.
- `lmml-router` owns LAN routing decisions.
- `lmml-api` owns stable DTOs shared between node, router, and future clients.

## Hardware Detection

`lmml-detect` probes the local system without treating missing optional GPU
tools as fatal. Detection produces a `SystemProfile`, warnings, missing hard
prerequisites, and a recommended `BuildBackend`.

Detected areas:

- compiler and C++17 support
- CMake and Git versions
- CUDA toolkit and NVIDIA GPU compute capabilities
- ROCm/HIP tooling and `gfx*` targets
- Vulkan runtime availability
- macOS Metal support
- CPU features
- memory and disk capacity
- build-cache tools such as `sccache`

Recommended backend priority:

```text
CUDA -> Metal -> ROCm/HIP -> Vulkan -> CPU AVX2 -> CPU AVX -> CPU fallback
```

Backend selection is advisory. Users may override it in settings or through
install-time environment variables.

## Build Backend Matrix

`lmml-build` converts `BuildBackend` into upstream llama.cpp CMake flags.

| Backend | Generated llama.cpp flags |
|---|---|
| CUDA | `-DGGML_CUDA=ON`, plus `-DCMAKE_CUDA_ARCHITECTURES=...` |
| Metal | `-DGGML_METAL=ON` |
| ROCm/HIP | `-DGGML_HIP=ON`, plus `-DGPU_TARGETS=...` when targets are known |
| Vulkan | `-DGGML_VULKAN=ON` |
| CPU AVX2 | `-DGGML_AVX2=ON` |
| CPU AVX | `-DGGML_AVX=ON` |
| CPU fallback | base release/server flags only |

ROCm/HIP support is intentionally conservative:

- use `hipconfig` and `rocminfo`;
- auto-select HIP only when a real `gfx*` target is visible;
- normalize known target aliases such as `gfx1035` to `gfx1030`;
- pass `HIPCXX` and `HIP_PATH` hints when detected;
- allow operators to use Vulkan instead when Mesa/RADV is the better local path.

Open follow-up: ROCm-specific VRAM telemetry must be added before claiming
complete AMD operational telemetry.

## Model Management

`lmml-models` owns local GGUF model discovery and model metadata handling.

Responsibilities:

- scan configured model directories;
- preserve external aliases and symlinks;
- parse basic GGUF metadata when available;
- derive family/profile guidance for known model families;
- estimate VRAM fit and recommended GPU offload;
- search and download GGUF models with resumable downloads;
- delete models only after confirmation through the TUI.

Model management must not bundle model weights.

## Server Lifecycle

`lmml-server` owns managed `llama-server` processes.

Required behavior:

- assemble runtime arguments through `lmml-compat`;
- detect port conflicts before spawning;
- stream logs back to the TUI;
- poll HTTP health endpoints instead of relying on log strings;
- shut down child processes gracefully;
- avoid orphaned process groups;
- report failures as typed errors.

`llama-cli` is useful for one-shot diagnostics, but long-running agent and
developer workflows should use HTTP `llama-server` endpoints.

## Node API

`lmml-node` is the stable worker API around an existing local `llama-server`.
It is a proxy and compatibility boundary, not a distributed scheduler.

Current routes:

| Route | Purpose |
|---|---|
| `GET /v1/health` | public health probe |
| `GET /v1/capabilities` | authenticated node capability document |
| `GET /v1/load` | authenticated node load document |
| `GET /v1/models` | authenticated, path-safe model list |
| `POST /v1/infer` | canonical LMML inference contract |
| `POST /v1/chat/completions` | raw OpenAI-compatible pass-through |
| `POST /v1/responses` | raw OpenAI Responses-compatible pass-through |
| `POST /v1/embeddings` | raw OpenAI-compatible pass-through |
| `POST /v1/messages` | Anthropic Messages compatibility adapter |
| `POST /v1/server/control` | gated server-control surface |

Security defaults:

- bind to localhost by default;
- require bearer authentication for LAN binds;
- authorize protected routes before reading request bodies;
- hide local model paths in LAN responses;
- keep server control disabled by default;
- require an API key when server control is enabled, even on localhost.

`/v1/infer` remains the canonical LMML-native route. OpenAI and Anthropic routes
exist for client compatibility.

## Router API

`lmml-router` coordinates a trusted set of `lmml-node` workers. It makes routing
decisions; workers do not make distributed scheduling decisions.

Router responsibilities:

- authenticate client requests;
- probe upstream health, capabilities, models, and load;
- select only ready workers that support the requested route;
- prefer requested model matches;
- use node load plus router in-flight counters for conservative balancing;
- aggregate models and capabilities across ready workers;
- merge static upstreams with opt-in discovered nodes;
- expire discovered nodes after missed advertisements.

Current routed endpoints:

- `/v1/infer`
- `/v1/chat/completions`
- `/v1/responses`
- `/v1/messages`
- `/v1/embeddings`
- `/v1/models`

Discovery is opt-in:

- workers advertise with `lmml-node --advertise-lan`;
- router listens with `lmml-router --discover-lan`;
- discovered workers must advertise authenticated APIs;
- router verifies candidates through authenticated probes before routing.

Future routing improvements:

- richer GPU memory/load telemetry;
- route policy tags such as foreground, background, coding, embedding, and batch;
- signed advertisements for untrusted or larger networks.

## Client Integration

Supported client patterns:

- OpenAI Chat Completions clients through `/v1/chat/completions`;
- OpenAI Responses clients through `/v1/responses`;
- Codex profiles through generated `wire_api = "responses"` config;
- OpenCode-compatible local provider config;
- Anthropic Messages clients through `/v1/messages`;
- direct LMML-native clients through `/v1/infer`.

The runtime CLI may print config snippets for user review. Automatic writes
should be limited to formats where the repository already has safe, tested
merge behavior.

Codex config generation is intentionally print-only:

```sh
lmml runtime print-config codex
```

Operators review the generated TOML before adding it to their Codex profile
configuration.

## TUI

`lmml-tui` provides the local operator interface.

Primary screens:

- Detect: hardware, toolchain, warnings, and install hints.
- Build: selected backend, CMake flags, build/update status, and build logs.
- Models: local GGUF inventory, metadata, downloads, aliases, and deletion.
- Server: local server config, status, logs, start/stop/restart.
- Settings: persistent configuration and runtime profile controls.

Key TUI expectations:

- no blank first-run state;
- every missing prerequisite has a human-readable action;
- long-running work is async and streamed back through channels;
- build/server/download failures remain visible;
- destructive actions require confirmation;
- terminal state is restored on panic or clean exit.

## State

`lmml-state` stores configuration and runtime state using XDG-aware paths.

Persisted areas:

- build source path, backend, architecture/target hints, fingerprints, and ref;
- model directory, selected model, aliases, and runtime profile choices;
- server host, port, context, GPU layers, batch settings, auth, and extra args;
- cached system profile summary;
- detached runtime profile state for agent/client workflows.

State files must be backward-compatible through `serde(default)` where practical.

## Release And Install Plan

Release claims are validation-bound. A package is advertised for a target only
after it is built and smoke-tested on matching hardware or CI.

Required local gates:

```sh
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
bash -n scripts/*.sh
cargo build --release -p lmml-tui -p lmml-node -p lmml-router
scripts/package-release.sh
```

Installer expectations:

- binary install remains the default;
- source install is explicit and preflighted;
- source install builds from the checksummed source package;
- checksum verification is always performed;
- signed checksum verification is supported for public or non-local release;
- installer exits non-zero when hard prerequisites fail.

Preflight modes:

- `LMML_INSTALL_MODE=binary`
- `LMML_INSTALL_MODE=source`
- `LMML_GPU_MODE=required`
- `LMML_GPU_MODE=rocm`
- `LMML_GPU_MODE=vulkan`
- `LMML_GPU_MODE=cpu-only`

CPU-only mode must be explicit.

## Testing Plan

Unit tests:

- detection parsing and backend recommendation;
- CMake flag assembly for every backend;
- llama.cpp capability parsing and argument assembly;
- model registry and metadata behavior;
- server lifecycle and health polling;
- state round-trip and migration behavior;
- node/router auth-before-body behavior;
- compatibility route proxying and error mapping;
- runtime CLI config rendering.

Integration tests:

- stub `llama-server` health and failure cases;
- node-to-router proxy chain;
- release package content checks;
- installer and preflight shell checks.

Snapshot tests:

- core TUI tabs;
- common error states;
- overlays and settings views.

## Current Open Work

High priority:

- add ROCm-specific VRAM telemetry for load reporting and dashboard accuracy;
- keep release validation current for each advertised target;
- expand clean install smoke coverage for wheel/source-like LAN flows;
- keep Codex/OpenCode/Anthropic compatibility tests aligned with client changes.

Medium priority:

- improve route scoring with tokens/sec and free-memory signals;
- add signed LAN discovery advertisements;
- broaden Vulkan memory telemetry;
- improve multimodal profile validation;
- keep training support feature-detected against the local `llama-finetune`.

Deferred:

- broad non-Linux release claims until matching artifacts are validated;
- public cloud deployment assumptions;
- bundled model weights;
- unrelated proprietary engine integrations.

## Commit Scope Guidance

Keep future changes small and verifiable:

- API contract changes must include node/router/client tests.
- Backend detection changes must include mock-output tests.
- Build-flag changes must include CMake argument tests.
- TUI behavior changes should include unit or snapshot coverage.
- Documentation claim changes should point at current implemented behavior or
  clearly label future work.
