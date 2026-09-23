# LMML ↔ OpenCode Runtime Handoff

**Prepared:** 2026-09-20 14:01 AEST  
**Updated:** 2026-09-23 13:30 AEST  
**Machine:** `terran`  
**Audience:** LMML implementation/operator agent  
**Status:** Prism PQ2_0 runtime is validated and serving; external harness configuration must be synchronized to the active LMML port

> **Current operational state:** The validated Prism server is listening on
> `http://127.0.0.1:8080/v1`. Earlier references to port `1200` describe the
> previous handoff state and are historical unless explicitly marked otherwise.

## Mission for the receiving agent

Make LMML the reliable source of truth for the model OpenCode is actually using, without reintroducing OpenCode's runaway filesystem snapshots or a fake second “fast” route.

The immediate engineering problem is not CPU-only inference. The Prism ROCm
runtime is working on `gfx1201`, and the remaining integration problem is that
LMML can change the model or port without automatically rewriting external
harness configuration files.

## Current live handoff: Prism on port 8080

The current TUI state is the source of truth:

| Field | Current value |
|---|---|
| Runtime | `prism` |
| Runtime selection | `auto` |
| Backend | ROCm / HIP |
| GPU target | `gfx1201` |
| Server | `127.0.0.1:8080` |
| API base URL | `http://127.0.0.1:8080/v1` |
| Readiness | `/health` returns ready |
| Model | `TERNARY-BONSAI-2-27B-DERISKED-PQ2_0.gguf` |

OpenCode, Codex, DeepSeek Harness, and any other OpenAI-compatible client must
use the active endpoint above. Changing the LMML port does not mutate client
files automatically. After the server is ready, synchronize the clients:

```bash
# Preview first. This does not write either file.
lmml runtime configure opencode --dry-run --force

# Apply after reviewing the diff. Creates timestamped backups.
lmml runtime configure opencode --yes --force

# Restart OpenCode after the client configuration changes.
```

The configure command reads LMML's persisted single-server state, verifies the
live served model before applying LMML routing, preserves unrelated OpenCode
keys, removes the stale `llamacpp_fast` provider in single-server mode, and
updates both `model` and `small_model` to the live `llamacpp/<model>` route.
If `oh-my-openagent.json` exists, it is synchronized in the same transaction.

Keep the LMML TUI open while running the configure command. The interactive
server is owned by that TUI session and normally stops when the session exits;
`lmml runtime status` reports only the separate managed `opencode` profiles.
If configuration reports that `/v1/models` is unreachable, restart the Prism
server from the TUI, wait for `Status: Ready`, and rerun the command. Do not
bypass the live-model check just to write a stale endpoint.

For other harnesses, print a configuration generated from the same live state:

```bash
lmml runtime print-config codex
lmml runtime print-config deepseek-harness
```

Copy the resulting endpoint/model values into the harness configuration, then
restart that harness. Do not point clients at the dormant managed profile ports
`4010` or `4011` unless those profiles have been deliberately configured and
started.

## Executive summary

1. LMML is CUDA-backed and the live `llama-server` is a GPU compute process.
2. The original large CPU spike was caused by OpenCode's snapshot subsystem repeatedly running `git add --all` over `/home/angelo`, then failing on nested/incomplete Git repositories.
3. OpenCode snapshots were disabled with `"snapshot": false`. No snapshot Git child is currently running, and there have been no new snapshot failures since the correction.
4. OpenCode's duplicate `llamacpp_fast` provider was removed because both “full” and “fast” previously pointed to the same single-slot server.
5. OpenCode and all Oh My OpenAgent routes were aligned to the Crown Q4 model that LMML was serving at the time of correction.
6. LMML was subsequently switched to a different Crown Q8 model. The live server now serves Q8, but OpenCode still advertises the prior Q4 model ID. Requests work because this is a single-model OpenAI-compatible server, but the integration contract is stale again. The configure command above is the supported repair path.
7. LMML's managed `opencode` and `opencode-fast` runtime profiles are stopped and have no model configured. The working server is the interactive LMML server on port `8080`, not the managed runtime servers on `4010`/`4011`.
8. `TERNARY-BONSAI-2-27B-DERISKED-PQ2_0.gguf` is valid but cannot load through LMML's upstream `ggml-org/llama.cpp` build. Its Prism-specific `PQ2_0` tensors use GGML type `142`; the installed upstream reader accepts only types `0` through `42`. LMML needs a side-by-side Prism runtime, model-aware runtime selection, and safe 32K/one-slot defaults for the first validated launch.

## Timeline

### 2026-09-19: initial diagnosis

- OpenCode advertised `Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-Q2_K_P.gguf`.
- LMML was actually serving `qwen35-crown11-aw-Q4_K_M.gguf`, an approximately 9B Q4_K_M model.
- `llama-server` was launched with `-ngl -1`, and NVIDIA reported it as a CUDA compute process using approximately 9.1 GiB of VRAM.
- OpenCode had a child process similar to:

  ```text
  git ... --git-dir /home/angelo/.local/share/opencode/snapshot/... \
      --work-tree /home/angelo add --all --sparse ...
  ```

- That Git process consumed about 150% CPU and repeatedly failed on:
  - `repos/agentq/llama-model-manager-v2.1.0`
  - `repos/agentq/llama-model-manager.backup-20260420T093109Z/`
- The repository root resolved to `/home/angelo`, so starting OpenCode below `/home/angelo/repos` did not prevent whole-home snapshot traversal.

### 2026-09-19 17:13 AEST: correction applied

- Added `"snapshot": false` to `~/.config/opencode/opencode.json`.
- Replaced the incorrect 27B model ID with `qwen35-crown11-aw-Q4_K_M.gguf`.
- Removed provider `llamacpp_fast`.
- Set both OpenCode `model` and `small_model` to the single `llamacpp` provider.
- Updated all Oh My OpenAgent agents and categories to that provider/model.
- Removed redundant fallback arrays that pointed back to the same server.
- JSON syntax and OpenCode's resolved configuration were validated.

The last snapshot failure was logged at `2026-09-19T07:10:00Z`, before the corrected config's `17:13 AEST` modification time. No snapshot Git child is running now.

### 2026-09-19 18:48 AEST onward: configuration drift returned

LMML was switched to:

```text
/home/angelo/.local/share/lmml/models/Qwen35-Uncensored-Crown11-Merged-9.0B-Q8_0.gguf
```

The LMML state and live server changed, but OpenCode and Oh My OpenAgent retained the Q4 model label.

### 2026-09-20 23:04 AEST onward: Prism PQ2_0 incompatibility confirmed

LMML attempted to start:

```text
/home/angelo/.local/share/lmml/models/TERNARY-BONSAI-2-27B-DERISKED-PQ2_0.gguf
```

The upstream server rejected the file while reading tensor descriptors:

```text
gguf_init_from_reader: tensor 'output.weight' has invalid ggml type 142.
should be in [0, 43)
```

The failure was reproduced before model allocation, CUDA offload, context allocation, or inference. It is therefore not evidence of CPU-only execution, insufficient VRAM, a damaged download, or a server-readiness timeout.

Integrity and provenance checks:

| Check | Result |
|---|---|
| Local file size | `7,206,168,928` bytes |
| Local SHA-256 | `32eb8f0ddfb8714d7ea9d10903c6dbe56c3b508f876c7280c6072dafcb1db7d5` |
| Publisher SHA-256 | Exact match |
| Model packing | Prism native ternary `PQ2_0`, group size 128 |
| GGML tensor type | `142` |
| Required runtime | PrismML `llama.cpp` fork with PQ2_0 kernels and activation transforms |
| Installed source | Upstream `https://github.com/ggml-org/llama.cpp.git` |
| Installed source commit at diagnosis | `3d82ef62d47fd74e18f36c5eccbdcf965b617b17` |
| Installed GGML type range | `0..42` (`GGML_TYPE_COUNT = 43`) |

Primary references:

- [DERISKED model card and artifact checksums](https://huggingface.co/Blackfrost-AI/TERNARY-BONSAI-2-27B-DERISKED-GGUF)
- [Publisher's validated Prism deployment kit](https://huggingface.co/Blackfrost-AI/TERNARY-BONSAI-2-27B-DERISKED-GGUF/blob/main/DEPLOYMENT_KIT_RTX_PRO_6000_BLACKWELL/README.md)
- [Prism runtime and model-format guidance](https://github.com/PrismML-Eng/Bonsai-demo/blob/main/MODEL-FORMATS.md)
- [PrismML llama.cpp fork](https://github.com/PrismML-Eng/llama.cpp)

## Live state at initial handoff (14:01 AEST)

### Hardware and LMML build

| Field | Value |
|---|---|
| GPU | NVIDIA GeForce RTX 5060 Ti |
| VRAM | 16,311 MiB |
| LMML backend | `Cuda` |
| CUDA architecture | `sm_120` |
| llama.cpp commit | `7acdbb1f191d869bad8c5da9d4a2121defa340af` |
| Installed LMML binary | `/home/angelo/.local/bin/lmml` |
| LMML source repository | `/home/angelo/repos/lmml` |

### Live server

| Field | Value |
|---|---|
| URL | `http://127.0.0.1:1200/v1` |
| Health | `ok` |
| Actual model | `Qwen35-Uncensored-Crown11-Merged-9.0B-Q8_0.gguf` |
| Parameters | 8,953,803,264 |
| Quantization | `Q8_0` |
| Model size reported by API | 9,516,533,760 bytes |
| Context | 196,608 tokens |
| GPU layers | `-1` |
| Flash attention | on |
| Batch / ubatch | 512 / 128 |
| Threads | 16 |
| Parallel slots | 1 |
| KV cache types | K=`q8_0`, V=`q8_0` |
| RAM prompt cache | 4,096 MiB |
| GPU allocation observed | approximately 12,350 MiB |

Relevant launch shape:

```text
llama-server \
  --model .../Qwen35-Uncensored-Crown11-Merged-9.0B-Q8_0.gguf \
  --host 127.0.0.1 --port 1200 \
  --ctx-size 196608 -ngl -1 \
  --batch-size 512 --ubatch-size 128 --threads 16 \
  --flash-attn on --jinja --parallel 1 \
  -ctk q8_0 -ctv q8_0 --cache-ram 4096
```

At the live check, the single slot was actively processing a 41,762-token request, with 38,035 prompt tokens reported as cached. Treat those counters as a point-in-time workload sample, not static configuration.

### OpenCode resolved configuration

File: `/home/angelo/.config/opencode/opencode.json`

```json
{
  "snapshot": false,
  "model": "llamacpp/qwen35-crown11-aw-Q4_K_M.gguf",
  "small_model": "llamacpp/qwen35-crown11-aw-Q4_K_M.gguf",
  "provider": {
    "llamacpp": {
      "options": {
        "baseURL": "http://127.0.0.1:1200/v1",
        "timeout": 7200000,
        "chunkTimeout": 300000
      }
    }
  },
  "compaction": {
    "auto": true,
    "prune": true,
    "reserved": 65536
  }
}
```

Important drift:

- OpenCode model ID: `qwen35-crown11-aw-Q4_K_M.gguf`
- Live server model: `Qwen35-Uncensored-Crown11-Merged-9.0B-Q8_0.gguf`

Recent OpenCode logs confirm it sends the Q4 label while the Q8 server handles the request.

### Oh My OpenAgent

File: `/home/angelo/.config/opencode/oh-my-openagent.json`

- Every configured agent currently uses `llamacpp/qwen35-crown11-aw-Q4_K_M.gguf`.
- Every category currently uses the same model.
- No `fallback_models` arrays remain.
- `llamaModelManager.openagentSync.fastModel` and `fullFallbackModel` both point to the Q4 label.
- Category `lmml-qwen-fast` still exists as a compatibility name, but it has no distinct fast backend.

### Standalone validator drift

File: `/home/angelo/.config/opencode/validator.ts`

This file still maps `quick`, `visual-engineering`, and `lmml-qwen-fast` to provider `llamacpp_fast`, even though that provider was removed from `opencode.json`.

No current import/registration of `validator.ts` was found in `opencode.json`, `oh-my-openagent.json`, or the local package manifest. It may be dormant, but its ownership and load path should be confirmed before editing or deleting it.

### Managed runtime profiles

`lmml runtime status` reports:

```text
profile        status     pid     url                          model
opencode       stopped    -       http://127.0.0.1:4010/v1     -
opencode-fast  stopped    -       http://127.0.0.1:4011/v1     -
```

The working OpenCode connection is therefore not represented by managed runtime state. It is attached to the interactive TUI server on `8080` in the current handoff.

Do not run `lmml runtime configure opencode --force` against the current user config without first fixing the runtime profiles and previewing the result. The installed binary currently proposes:

- unset placeholder models;
- ports `4010` and `4011`;
- two providers, including `llamacpp_fast`;
- `compaction.reserved = 32768`;
- `chunkTimeout = 300000`.

That would conflict with the working single-server contract and could overwrite the deliberate snapshot/compaction choices.

## CPU finding: resolved versus unresolved

### Resolved

The earlier persistent CPU burn was an OpenCode snapshot Git loop. With `snapshot: false`:

- there is no `git ... .local/share/opencode/snapshot` child;
- the previous repeating snapshot failure has stopped;
- future OpenCode filesystem rollback is disabled.

### Still worth profiling

During the active 41k-token agent/tool request, a three-second sample showed approximately:

| Process | CPU |
|---|---:|
| `lmml` TUI | 20% |
| `llama-server` | 112% |
| `opencode` | 212% |

This sample was taken under active inference and an active tool/subagent loop. It is not evidence of CPU-only model execution. `llama-server` simultaneously held approximately 12.35 GiB of VRAM as an NVIDIA compute process.

If CPU remains a concern, profile in two controlled states:

1. server idle, OpenCode idle, no snapshot child;
2. a fixed prompt and fixed output count under active inference.

Separate OpenCode/Bun/tool-loop CPU from llama-server CPU and GPU utilization. Do not infer inference placement from the `opencode` process's CPU percentage; OpenCode is the HTTP client/orchestrator.

## Storage state

Existing data was intentionally retained:

| Path | Approximate size |
|---|---:|
| `~/.local/share/opencode/snapshot` | 579 MiB |
| `~/.local/share/opencode/opencode.db` | 7.4 GiB |
| `~/.local/share/opencode/log/opencode.log` | 85 MiB |

Do not delete these without explicit operator approval. Disabling snapshots stops future capture but does not remove old snapshot data.

## Source-level integration hazards

The LMML source worktree is `/home/angelo/repos/lmml` and is dirty with pre-existing user changes. Do not reset, clean, or revert unrelated files.

The relevant implementation is primarily:

- `crates/lmml-build/src/lib.rs`
- `crates/lmml-tui/src/runtime_cli.rs`
- `crates/lmml-state/src/lib.rs`
- `crates/lmml-tui/src/main.rs`
- `docs/lmml-integration-contract.md`
- `docs/llama-server-integration-contract.md`

Observed code assumptions in `runtime_cli.rs`:

- `desired_opencode_config()` always requires both `opencode` and `opencode-fast` profiles.
- It routes `small_model` through `llamacpp_fast`.
- It emits `compaction.reserved = 32768`.
- `desired_provider_object()` always emits two providers.
- The current source file contains `chunkTimeout = 2400000`, while the installed binary's `runtime print-config` output contains `300000`. This indicates source/binary behavior skew that should be reconciled before deployment.

The default managed profiles in `lmml-state` are also incompatible with the live single-slot setup:

- `opencode`: port `4010`, context `65536`, parallel `4`;
- `opencode-fast`: port `4011`, context `32768`, parallel `2`.

`crates/lmml-build/src/lib.rs` currently hardcodes:

```rust
const LLAMA_CPP_URL: &str = "https://github.com/ggml-org/llama.cpp.git";
```

That source is appropriate for ordinary upstream GGUF formats but cannot load Prism `PQ2_0` type `142` or `PTQ1_0` type `143`. Repointing the existing checkout in place is not a safe fix: Prism explicitly warns not to mix its `ggml-*` libraries with a stock build, and LMML must remain able to run ordinary upstream models.

## Recommended correction: model-aware upstream and Prism runtimes

### Objective and non-goals

Add first-class, side-by-side runtime flavors so LMML selects a compatible `llama-server` from model metadata before attempting startup.

The correction must:

- preserve upstream `llama.cpp` for standard GGUFs;
- add a separately built Prism runtime for Prism ternary models;
- detect compatibility from GGUF contents, not filenames;
- prevent an incompatible binary from being launched;
- keep source trees, build outputs, shared libraries, fingerprints, and update channels isolated;
- synchronize OpenCode only after the selected server is healthy and reports the intended model;
- preserve all existing snapshot protections and unrelated OpenCode configuration.

It must not:

- merely raise `GGML_TYPE_COUNT` or reinterpret type `142` as type `42`;
- rename the GGUF to look like `Q2_0`;
- replace the upstream checkout globally with the Prism fork;
- combine Prism and upstream `libggml`, `libllama`, or `libllama-common` libraries;
- assume that ordinary BF16/F16 tensors make a Bonsai model upstream-compatible—Prism models can also require `prism.hadamard.*` activation transforms;
- update OpenCode to a model that failed to start.

### Why a separate runtime is required

`PQ2_0` is not upstream `Q2_0` under another name.

| Format | Tensor type | Group size | Compatible runtime |
|---|---:|---:|---|
| Standard upstream quantizations such as `Q4_K_M` and `Q8_0` | Upstream IDs | Varies | Upstream and normally Prism |
| Official group-64 `Q2_0` | `42` | 64 | Upstream and Prism v7+ |
| Prism `PQ2_0` | `142` | 128 | Prism v7+ with matching kernels |
| Prism `PTQ1_0` | `143` | 128 | Prism v7+ with matching kernels |

The Prism formats need both their block layouts/kernels and the associated activation transform. Teaching the upstream parser to accept the numeric ID alone would at best fail later and at worst yield incoherent output.

### 1. Introduce an explicit runtime flavor

Add a typed runtime/source identifier in LMML state and build configuration, for example:

```rust
enum LlamaRuntimeFlavor {
    Upstream,
    Prism,
}
```

Persist, per flavor:

- canonical repository URL;
- requested branch/tag/commit;
- resolved commit;
- source directory;
- build directory;
- `llama-server` binary path;
- build backend and CUDA architectures;
- CMake argument fingerprint;
- binary version/build output;
- last successful verification time.

Suggested isolated layout:

```text
~/.local/share/lmml/runtimes/upstream/llama.cpp/
~/.local/share/lmml/runtimes/prism/llama.cpp/
```

For a low-risk migration, the existing `~/.local/share/lmml/llama.cpp` checkout can remain the upstream location initially. The Prism checkout must still use its own directory and build tree. Do not rewrite the existing checkout's `origin` remote.

Use an explicit trusted source registry rather than accepting an arbitrary repository URL from model metadata:

| Flavor | Repository | Initial ref policy |
|---|---|---|
| `upstream` | `https://github.com/ggml-org/llama.cpp.git` | Existing LMML update policy |
| `prism` | `https://github.com/PrismML-Eng/llama.cpp.git` | Pin a validated Prism v7+ release/commit |

The DERISKED publisher validated Prism release `prism-b10683-d8f26ee`, commit `d8f26eec76da6d09bb708bcba51ef64b8cd868a3`. Use that as the initial known-good baseline unless a newer Prism revision is independently smoke-tested on this machine.

### 2. Add GGUF compatibility preflight

Before spawning a server, inspect enough of the GGUF header to collect:

- `general.architecture`;
- `general.file_type`;
- raw tensor type IDs;
- presence of `prism.hadamard.*` metadata;
- optional model name/context metadata needed for display.

The preflight parser must retain unknown raw tensor type integers instead of failing while converting them into the upstream enum. Runtime selection should follow rules equivalent to:

```text
if any tensor type is 142 or 143:
    require Prism
else if any metadata key starts with "prism.hadamard.":
    require Prism
else if every tensor type is supported upstream:
    prefer Upstream
else:
    fail with an unsupported-format error naming the raw type IDs
```

Do not select a runtime from `PQ2_0`, `Q2_0`, or `Prism` substrings in the filename. Filenames have changed across model generations and older `Q2_0` Prism artifacts used an incompatible legacy layout under type `42`.

Store the detected requirement with the model inventory entry, but recompute it whenever file size, modification time, or content hash changes. A stale cached compatibility decision must not survive replacement of the GGUF at the same path.

### 3. Make builds and dynamic libraries flavor-safe

Refactor the hardcoded `LLAMA_CPP_URL` into the runtime flavor definition. Build each flavor independently with LMML's detected backend.

For this machine, the Prism build should retain:

- CUDA enabled;
- CUDA architecture `sm_120`/CMake architecture `120`;
- server target enabled;
- the same host compiler compatibility checks LMML applies to upstream builds.

After build, verify all of the following before marking the runtime usable:

1. `llama-server --version` succeeds and its commit matches the selected source checkout.
2. The binary and its `libggml*`, `libllama*`, and `libmtmd*` dependencies resolve from the same flavor's build output.
3. A Prism build contains `PQ2_0`/type-142 support in its source/type registry.
4. A minimal metadata/load smoke test reaches model initialization without `invalid ggml type 142`.
5. The build fingerprint includes runtime flavor, source URL, resolved commit, CMake arguments, CUDA architecture, and binary path.

The launcher should set or preserve the appropriate runtime library search path when necessary. Never allow the Prism server binary to bind against upstream shared libraries, or the upstream server to bind against Prism shared libraries.

### 4. Select the runtime before server startup

The server-start state machine should become:

```text
select model
  -> inspect GGUF compatibility
  -> resolve required runtime flavor
  -> verify/build that flavor
  -> calculate a safe launch plan
  -> spawn the matching binary
  -> wait for process and health readiness
  -> verify /v1/models reports the selected model
  -> commit LMML/OpenCode synchronization
```

If the required runtime is unavailable, stop before spawning and show an actionable error such as:

```text
This model requires the Prism runtime: tensor type 142 (PQ2_0) was found.
The installed upstream runtime supports types 0 through 42.
Build or select the Prism runtime to continue.
```

Do not retry the same incompatible command until the 30-second readiness timeout. Parser incompatibility is deterministic and should fail immediately at preflight.

Record the resolved runtime flavor, source commit, and binary path in the server status. The TUI Server panel should display them alongside model, port, context, GPU layers, batch size, and threads.

### 5. Add safe model-specific launch defaults

The current global configuration requests `196608` context. That setting was not responsible for the type-142 parser failure, but it can become a separate memory problem after the correct runtime loads.

For the first launch of this DERISKED PQ2_0 model, use the publisher-validated baseline:

| Setting | Initial value |
|---|---:|
| Context | `32768` |
| Parallel slots | `1` |
| GPU layers | all supported layers (`-ngl 999` in the validated recipe; LMML may normalize this to its equivalent) |
| Flash attention | enabled |
| Jinja template | enabled |
| Temperature | `1.0` |
| Top-p | `0.95` |
| Top-k | `20` |

The publisher validated this recipe on a 97,887 MiB RTX PRO 6000 Blackwell, not the local 16,311 MiB RTX 5060 Ti. CUDA support exists for PQ2_0, and the 6.71 GiB weights are small enough to justify a local test, but successful 32K operation on this GPU is not guaranteed. LMML should perform its normal memory-fit calculation and clearly distinguish:

- format/runtime incompatibility;
- model-weight allocation failure;
- KV-cache/context allocation failure;
- compute-buffer allocation failure.

Only increase context after a successful 32K/one-slot smoke test and observed VRAM headroom. Do not silently fall back to CPU merely to satisfy the requested context.

### 6. Make the TUI behavior explicit

Models view:

- show `Required runtime: upstream|prism|unsupported`;
- show detected raw tensor types and relevant Prism metadata in model details;
- warn when the selected runtime is not built;
- offer a deliberate build/select action rather than launching an incompatible binary.

Build view:

- show the selected runtime flavor, repository, ref, resolved commit, source directory, and build directory;
- support building/updating upstream and Prism independently;
- never report one flavor as current based on the other flavor's fingerprint;
- require a clean rebuild when the source flavor changes.

Server view:

- show the actual runtime flavor and binary;
- surface preflight errors directly instead of converting them into generic readiness timeouts;
- keep the last successful runtime/model visible after a failed switch;
- allow rollback/restart of the last known-good pair.

Settings/state:

- add an `auto` runtime-selection mode as the safe default;
- permit an expert override only when compatibility is proven;
- version and migrate the state schema, defaulting existing installations to `upstream` without deleting their build state;
- store per-model launch overrides separately from global defaults so the Bonsai 32K baseline does not reduce other models unintentionally.

### 7. Synchronize OpenCode transactionally

Runtime support must integrate with the existing OpenCode correction rather than create another source of drift.

Use a staged transaction:

1. Preserve a timestamped backup of user-owned OpenCode and Oh My OpenAgent files.
2. Start the candidate server with the compatible runtime.
3. Require successful health and `/v1/models` checks.
4. Confirm the live process argv contains the intended GGUF and runtime binary.
5. Atomically update the OpenCode model entry, `model`, `small_model`, all active Oh My OpenAgent routes, and sync metadata.
6. Preserve `snapshot: false`, plugins, instructions, compaction settings, and unrelated providers.
7. Fully restart OpenCode only after the configuration transaction succeeds.
8. On failure, leave OpenCode pointing at the last healthy server/model and report the candidate failure.

The model ID visible to OpenCode should either match `/v1/models` or use a stable alias that has been verified with a real completion request. Runtime flavor is an LMML concern and should not require a second fake OpenCode provider.

### 8. Tests required before rollout

Unit tests:

- detect type `142` as Prism `PQ2_0`;
- detect type `143` as Prism `PTQ1_0`;
- detect `prism.hadamard.*` on ordinary tensor types as Prism-required;
- classify `Q4_K_M`, `Q8_0`, and official group-64 type-42 `Q2_0` as upstream-compatible;
- preserve and report unknown raw tensor IDs;
- invalidate cached compatibility when the file changes;
- produce distinct, actionable errors for unsupported format, missing runtime, and memory-fit failure.

Build/state tests:

- resolve separate source/build/binary paths per flavor;
- include flavor and source commit in the build fingerprint;
- never reuse one flavor's shared libraries or fingerprint for the other;
- migrate existing state to upstream without losing current paths or settings;
- preserve a dirty LMML source worktree and unrelated user configuration.

Integration tests:

- reject a PQ2_0 model before spawning the upstream server;
- select the Prism binary for a synthetic/type-142 fixture;
- select upstream for an ordinary Q4/Q8 model;
- switch Prism -> upstream -> Prism without rewriting Git remotes or mixing libraries;
- do not update OpenCode when candidate startup fails;
- update all OpenCode/Oh My OpenAgent model references after a successful switch;
- preserve `snapshot: false` and unrelated JSON keys;
- expose runtime flavor, commit, model, and binary consistently in state and the TUI.

Hardware-gated smoke test on this machine:

1. Verify the model SHA-256.
2. Launch the Prism server at 32K context, one slot, full CUDA offload, and flash attention.
3. Confirm no `invalid ggml type 142` or Hadamard-transform errors occur.
4. Confirm `/health` reports ready and `/v1/models` identifies the DERISKED PQ2_0 GGUF.
5. Confirm `nvidia-smi` reports the Prism `llama-server` as an NVIDIA compute process.
6. Run a deterministic short text completion and inspect it for coherent output.
7. Exercise a short multi-turn chat through OpenCode after synchronization.
8. Record peak VRAM, prompt-processing speed, decode speed, and context allocation.
9. Stop and report a clear memory-fit failure rather than enabling unbounded CPU fallback if 32K does not fit.

### 9. Rollout and rollback sequence

1. Do not modify or delete the current upstream source/build.
2. Land state/schema and runtime-flavor support first.
3. Add metadata preflight and UI reporting.
4. Add the Prism source/build in its isolated directory at the pinned known-good revision.
5. Run unit and integration tests with no OpenCode mutation.
6. Stop the current server cleanly before binding the Prism candidate to the configured LMML port.
7. Run the 32K hardware smoke test.
8. Synchronize OpenCode only after the candidate passes.
9. Verify end-to-end OpenCode traffic and snapshot protections.
10. If any step fails, stop the candidate and restart the last known-good upstream model/runtime pair; restore configuration only from the timestamped backup created for this transaction.

### Prism runtime acceptance criteria

The Prism workstream is complete only when all of the following hold:

- The local DERISKED PQ2_0 file still matches SHA-256 `32eb8f0ddfb8714d7ea9d10903c6dbe56c3b508f876c7280c6072dafcb1db7d5`.
- LMML identifies it as Prism-required from GGUF metadata/tensor IDs before launch.
- Upstream and Prism sources, builds, binaries, libraries, commits, and fingerprints remain isolated.
- The selected Prism binary reports the intended pinned or independently validated revision.
- Startup no longer emits `invalid ggml type 142`.
- The model produces a coherent smoke-test completion through the Prism runtime.
- CUDA offload is active; there is no silent CPU fallback.
- The initial accepted profile uses 32K context and one slot, or a lower explicitly documented memory-fit fallback.
- A failed candidate launch does not change OpenCode routing.
- A successful launch results in agreement between LMML state, process argv, `/v1/models`, OpenCode, and Oh My OpenAgent.
- Switching back to a standard Q4/Q8 GGUF selects the upstream runtime and remains functional.
- No snapshot Git traversal is reintroduced.

## Requested implementation outcome

Choose and implement one canonical integration mode.

### Preferred: single-server mode for this machine

1. Treat the interactive LMML server on `127.0.0.1:8080` as the sole OpenCode backend for the current machine. In general, use the port shown by the LMML Server screen and persisted state.
2. Keep one OpenCode provider: `llamacpp`.
3. Route both `model` and `small_model` to that provider.
4. When LMML changes the loaded model, atomically update:
   - `opencode.json` provider model key;
   - top-level `model` and `small_model`;
   - `oh-my-openagent.json` agent/category routes;
   - `llamaModelManager.openagentSync` metadata;
   - `validator.ts` only if it is confirmed active.
5. Preserve unrelated user fields, especially:
   - `snapshot: false`;
   - plugin configuration;
   - `compaction.reserved: 65536` unless deliberately re-budgeted;
   - operator instructions;
   - timeout choices.
6. Fully restart OpenCode after routing changes.

An alternative is a stable server-side model alias, if the installed llama-server supports and validates it. In that design, OpenCode would use a stable logical model ID while LMML changes the backing GGUF. Do not assume alias support—test `/v1/models` and an actual completion request first.

### Alternative: managed runtime mode

If `4010`/`4011` managed runtimes are the intended architecture:

1. Configure real model paths in both runtime profiles.
2. Validate aggregate VRAM before starting both servers.
3. Set contexts/parallelism appropriate for the 16 GiB RTX 5060 Ti.
4. Run `lmml runtime configure opencode --dry-run` and verify it preserves user-owned fields.
5. Update Oh My OpenAgent and any active validator middleware as part of the same transaction.
6. Stop using the TUI server on `8080` for OpenCode, so there is one unambiguous ownership model.

Do not keep the current hybrid state indefinitely: managed profiles claim `4010`/`4011`, while real traffic uses the interactive TUI server on `8080`.

## Acceptance criteria

The handoff is complete when all of the following hold:

- `/v1/models` and OpenCode's configured model ID agree, or both intentionally use a tested stable alias.
- `state.toml` `last_used`, the live process argv, and `/v1/models` identify the same GGUF.
- OpenCode has no provider that points to a nonexistent runtime.
- Oh My OpenAgent has no stale model or provider reference.
- `validator.ts` is either aligned and demonstrably loaded, or documented/removed as dormant.
- `snapshot` remains `false` unless a bounded, repository-scoped snapshot design replaces it.
- No snapshot Git child traverses `/home/angelo`.
- CUDA remains enabled and `llama-server` appears as an NVIDIA compute process.
- `lmml runtime configure opencode --dry-run` never proposes unset placeholder models.
- Configuration changes preserve unrelated user keys and create a timestamped backup.
- Tests cover single-server mode, dual-runtime mode, model switching, field preservation, and source/binary config parity.

## Read-only verification commands

```bash
# Resolved OpenCode contract
opencode debug config --pure \
  | jq '{snapshot,model,small_model,provider,compaction}'

# Oh My OpenAgent routing uniqueness
jq '{
  agents: ([.agents[].model] | unique),
  categories: ([.categories[].model] | unique),
  fallbacks: ([.. | objects | select(has("fallback_models"))] | length),
  sync: .llamaModelManager.openagentSync
}' ~/.config/opencode/oh-my-openagent.json

# Live model and context
curl -fsS http://127.0.0.1:8080/v1/models \
  | jq '.data[0] | {id, meta}'

# Exact server launch and GPU allocation
pgrep -af 'llama-server'
nvidia-smi --query-compute-apps=pid,process_name,used_memory --format=csv,noheader

# Snapshot loop must return no real Git child
pgrep -af 'git .*\.local/share/opencode/snapshot'

# Managed runtime state
lmml runtime status
lmml runtime print-config opencode

# Preview only; do not apply until placeholder models and routing are fixed
lmml runtime configure opencode --dry-run \
  --model-source existing \
  --small-model-source existing

# PQ2_0 artifact integrity
sha256sum \
  /home/angelo/.local/share/lmml/models/TERNARY-BONSAI-2-27B-DERISKED-PQ2_0.gguf

# Current upstream source identity and supported type ceiling
git -C /home/angelo/.local/share/lmml/llama.cpp remote get-url origin
git -C /home/angelo/.local/share/lmml/llama.cpp rev-parse HEAD
rg -n 'GGML_TYPE_COUNT' \
  /home/angelo/.local/share/lmml/llama.cpp/ggml/include/ggml.h

# Run after the isolated Prism runtime has been built
git -C /home/angelo/.local/share/lmml/runtimes/prism/llama.cpp \
  remote get-url origin
git -C /home/angelo/.local/share/lmml/runtimes/prism/llama.cpp \
  rev-parse HEAD
rg -n 'GGML_TYPE_PQ2_0|GGML_TYPE_PTQ1_0' \
  /home/angelo/.local/share/lmml/runtimes/prism/llama.cpp
/home/angelo/.local/share/lmml/runtimes/prism/llama.cpp/build/bin/llama-server \
  --version
ldd \
  /home/angelo/.local/share/lmml/runtimes/prism/llama.cpp/build/bin/llama-server

# Post-launch Prism health, model identity, process identity, and GPU allocation
curl -fsS http://127.0.0.1:8080/health
curl -fsS http://127.0.0.1:8080/v1/models \
  | jq '.data[0] | {id, meta}'
pgrep -af 'runtimes/prism/.*/llama-server'
nvidia-smi --query-compute-apps=pid,process_name,used_memory --format=csv,noheader
```

## Files intentionally changed during the correction

- `/home/angelo/.config/opencode/opencode.json`
- `/home/angelo/.config/opencode/oh-my-openagent.json`

No LMML source files, model files, existing OpenCode snapshot data, database files, or running server processes were modified as part of the correction.

## Guardrails

- Preserve the dirty LMML worktree; do not use destructive Git commands.
- Do not delete the 579 MiB snapshot store or 7.4 GiB OpenCode database without operator approval.
- Do not re-enable whole-home OpenCode snapshots.
- Do not reintroduce `llamacpp_fast` unless it represents a genuinely separate, running endpoint with a real model.
- Do not treat high OpenCode client CPU as proof of CPU inference.
- Do not apply `runtime configure --force` until its dry-run output matches the chosen architecture.
- Restart OpenCode after provider/model/category changes; restarting only llama-server does not reload client-side routing.
