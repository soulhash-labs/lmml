# lmml Managed Runtime Harness Plan

This plan defines how lmml should provide managed local llama.cpp runtimes for
coding harnesses. The first target is OpenCode because the local config already
uses OpenAI-compatible HTTP endpoints.

The detailed compatibility and validation contract is tracked in
[`docs/llama-server-integration-contract.md`](llama-server-integration-contract.md).
That contract is the Phase 11 target for `lmml-compat` flag generation, profile
schema expansion, VRAM/context guards, prompt-cache controls, and OpenCode
compaction drift detection.

Fleet-level machine roles and proposed multi-host profiles are tracked in
[`docs/lmml-fleet-profiles.md`](lmml-fleet-profiles.md). The Orion
256k profile is validated; Quadro M6000 and RTX 5070 Ti profiles remain proposed
until load-tested on those machines.

## Decision

Use `llama-server` as the harness runtime, not `llama-cli`.

`llama-cli` is useful for direct smoke tests, one-shot prompts, debugging, and
offline model checks. It is not the right default for coding harnesses because
agent workflows need a long-lived OpenAI-compatible HTTP API, stable port,
health/readiness checks, logs, restart behavior, and model/profile switching.

`llama-server` is the correct integration point because lmml already manages:

- build and binary discovery
- model selection
- server argv compatibility across llama.cpp versions
- process lifecycle
- port checks
- `/v1/health` readiness
- logs and status in the TUI
- CUDA detection and GPU layer settings

## Current OpenCode Shape

Current local OpenCode config is at:

```text
~/.config/opencode/opencode.json
```

Observed provider shape:

```json
{
  "provider": {
    "llamacpp": {
      "npm": "@ai-sdk/openai-compatible",
      "name": "lmml llama.cpp",
      "options": {
        "baseURL": "http://127.0.0.1:1200/v1",
        "timeout": 7200000,
        "chunkTimeout": 2400000
      }
    },
    "llamacpp_fast": {
      "npm": "@ai-sdk/openai-compatible",
      "name": "lmml llama.cpp fast",
      "options": {
        "baseURL": "http://127.0.0.1:1200/v1",
        "timeout": 7200000,
        "chunkTimeout": 2400000
      }
    }
  },
  "compaction": {
    "reserved": 65536
  }
}
```

Current proven route:

- TUI-managed full provider on `127.0.0.1:1200`
- TUI-managed fast provider on `127.0.0.1:1200`
- server context: `262144`
- OpenCode reserved compaction tokens: `65536`
- usable input before compaction: `196608`
- practical single-agent input target: `120000-170000`
- hard reject/compress threshold: about `196000`

Detached managed runtime profiles may still use dedicated ports in future
multi-instance work, but the current workstation-proven setup is `1200` for both
OpenCode providers.

The long-run preset is:

- request timeout: `7200s`
- chunk timeout: `2400s`
- reserved compaction tokens: `65536`

## Non-Goals

- Do not implement a native Anthropic Messages adapter in this phase. If a
  Claude/Anthropic-oriented harness can use OpenAI-compatible chat completions,
  it can consume these managed endpoints directly. Native `/v1/messages`
  translation belongs in a separate integration plan.
- Do not use `llama-cli` as the harness runtime. Keep it for diagnostics and
  direct one-shot checks.

## API Compatibility Boundary

The first runtime contract is OpenAI-compatible chat completions over
`llama-server`:

```text
POST /v1/chat/completions
GET  /v1/health
```

This matches OpenCode's current provider shape and also fits clients that can
choose an OpenAI-compatible mode even when they otherwise support Anthropic
native Messages APIs.

Anthropic-oriented clients may expose two useful modes:

- native Messages mode: `POST /v1/messages`
- OpenAI-compatible chat mode: `POST /v1/chat/completions`

Phase 11 should use the OpenAI-compatible chat mode first. Do not add native
Messages translation until there is a verified harness requirement and a
separate adapter contract.

## Runtime Profile Schema

Runtime profiles are persisted in the lmml config because they are user-edited
desired state. Live process state is persisted separately because PIDs, health
results, and log paths are runtime facts.

Config schema:

```toml
[runtime.profiles.opencode]
host         = "127.0.0.1"
port         = 1200
model        = "~/.local/share/lmml/models/Qwen3.5-4B-Q8_0.gguf"
ctx_size     = 262144
gpu_layers   = -1
batch_size   = 512
threads      = 8
parallel     = 1
compaction_reserved = 65536
extra_args   = []
autostart    = false

[runtime.profiles.opencode-fast]
host         = "127.0.0.1"
port         = 1200
model        = "~/.local/share/lmml/models/Qwen3.5-4B-Q8_0.gguf"
ctx_size     = 262144
gpu_layers   = -1
batch_size   = 512
threads      = 8
parallel     = 1
compaction_reserved = 65536
extra_args   = []
autostart    = false
```

These values describe the current workstation-proven TUI-server route. Future
detached multi-instance profiles may use `4010` and `4011`, but those ports are
not the active OpenCode route on this machine.

Rules:

- `model` may be empty after a fresh binary install. `lmml runtime start
  <profile>` must fail clearly and point the user to the TUI model flow or
  `lmml doctor`.
- `gpu_layers = -1` means offload as much as llama.cpp can fit. CPU-only nodes
  must opt into CPU mode and should use `gpu_layers = 0`.
- `extra_args` are appended after lmml-managed flags. lmml-owned flags such as
  host, port, model, context, batch, threads, and GPU layers must not be
  duplicated in `extra_args`.
- Profile names are stable identifiers and should use lowercase kebab-case.

Runtime state schema:

```toml
[runtime.state.opencode]
status          = "ready"
pid             = 12345
host            = "127.0.0.1"
port            = 1200
model           = "/home/angelo/.local/share/lmml/models/Qwen3.5-4B-Q8_0.gguf"
log_path        = "/home/angelo/.local/state/lmml/runtime/opencode.log"
started_at      = "2026-06-01T10:00:00Z"
last_health_at  = "2026-06-01T10:00:10Z"
last_health     = "ok"
failure_count   = 0
```

Status values: `stopped`, `starting`, `ready`, `unhealthy`, `failed`,
`stopping`.

Stale PID handling:

- On startup or `lmml runtime status`, lmml must check whether recorded PIDs are
  still alive and whether they still answer on the recorded port.
- If the PID is gone, mark the profile `stopped`.
- If the PID exists but the port health check fails, mark the profile
  `unhealthy`.
- Never kill an unknown process solely because its PID matches stale state.

## Planned lmml Surface

Add a harness profile layer above the existing server config:

- `lmml runtime start opencode`
- `lmml runtime start opencode --detach`
- `lmml runtime start opencode-fast`
- `lmml runtime stop <profile>`
- `lmml runtime status`
- `lmml runtime status --json`
- `lmml runtime logs <profile>`
- `lmml runtime logs <profile> --follow`
- `lmml runtime print-config opencode`
- `lmml runtime configure opencode --dry-run`
- `lmml runtime configure opencode`
- `lmml runtime configure opencode --rollback <backup-file>`
- `lmml runtime configure opencode --model-source existing|lmml|none`
- `lmml runtime configure opencode --small-model-source existing|lmml|none`
- `lmml runtime validate <profile>`
- `lmml runtime health <profile>`
- `lmml runtime restart <profile>`
- `lmml runtime restart <profile> --model <path>`
- `lmml profile presets`
- `lmml profile list`
- `lmml profile show <profile>`
- `lmml profile copy <preset-name> <profile>`
- `lmml profile copy --from-profile <source-profile> <profile>`
- `lmml profile validate <profile>`
- `lmml profile set <profile> <key> <value>`

The detailed subcommand behavior is defined in
[`docs/llama-server-integration-contract.md`](llama-server-integration-contract.md).

`start` behavior:

- Default non-detached mode streams startup logs until the profile becomes
  `ready` or fails health checks.
- `--detach` starts the server in the background, waits until `/v1/health`
  passes or startup times out, then returns.
- Current CLI implementation supports the detached path first. Foreground log
  streaming remains a later enhancement.
- Each profile owns an independent `ServerManager` instance. Starting or failing
  one profile must not stop another profile.
- If a port is already in use, only that profile fails and the error must name
  the conflicting host/port.

`status` default output:

```text
profile        status     pid     url                         model
opencode       ready      12345   http://127.0.0.1:1200/v1    Qwen3.5-4B-Q8_0.gguf
opencode-fast  ready      12345   http://127.0.0.1:1200/v1    Qwen3.5-4B-Q8_0.gguf
```

Exit codes:

- `0`: all requested profiles are `ready` or `stopped` without errors
- `1`: one or more requested profiles are `failed` or `unhealthy`
- `2`: bad arguments, unknown profile, invalid config, or missing model

`status --json` prints machine-readable profile state for scripts.

`print-config opencode` output must be ready-to-paste OpenCode JSON for the
managed profiles:

```json
{
  "provider": {
    "llamacpp": {
      "npm": "@ai-sdk/openai-compatible",
      "name": "lmml llama.cpp",
      "models": {
        "Qwen3.5-4B-Q8_0.gguf": {
          "name": "Qwen3.5-4B-Q8_0.gguf (lmml Qwen Q8 complex)"
        }
      },
      "options": {
        "baseURL": "http://127.0.0.1:1200/v1",
        "timeout": 7200000,
        "chunkTimeout": 2400000
      }
    },
    "llamacpp_fast": {
      "npm": "@ai-sdk/openai-compatible",
      "name": "lmml llama.cpp fast",
      "models": {
        "Qwen3.5-4B-Q8_0.gguf": {
          "name": "Qwen3.5-4B-Q8_0.gguf (lmml Qwen Q8 fast)"
        }
      },
      "options": {
        "baseURL": "http://127.0.0.1:1200/v1",
        "timeout": 7200000,
        "chunkTimeout": 2400000
      }
    }
  },
  "model": "llamacpp/Qwen3.5-4B-Q8_0.gguf",
  "small_model": "llamacpp_fast/Qwen3.5-4B-Q8_0.gguf",
  "compaction": {
    "auto": true,
    "prune": true,
    "reserved": 65536
  }
}
```

## OpenCode Configuration Flow

`lmml doctor` remains read-only. It may detect OpenCode configuration status and
report mismatches, but it must not mutate OpenCode files.

Doctor output should follow this shape:

```text
OpenCode config: found at ~/.config/opencode/opencode.json
lmml providers: missing
recommended: lmml runtime configure opencode --dry-run
```

Configuration changes are handled only by explicit runtime commands:

```sh
lmml runtime print-config opencode
lmml runtime configure opencode --dry-run
lmml runtime configure opencode
lmml runtime configure opencode --rollback <backup-file>
```

### Current 1200 TUI Server Override

The current workstation setup uses the TUI-managed server, not detached
runtime-profile servers, for OpenCode. The live server is:

```text
http://127.0.0.1:1200/v1
```

This is intentional. Agents and humans should not change OpenCode back to
`4010/4011` just because those are the default future managed-profile ports.
Use `4010/4011` only when `lmml runtime start opencode --detach` and
`lmml runtime start opencode-fast --detach` are the active server lifecycle.
The frozen evidence snapshot for this working route is
[`docs/opencode-1200-evidence.md`](opencode-1200-evidence.md).

For the current TUI server flow, keep these files aligned:

```toml
# ~/.config/lmml/state.toml
[server]
host = "127.0.0.1"
port = 1200

[runtime.opencode]
host = "127.0.0.1"
port = 1200
model = "/home/angelo/.local/share/lmml/models/Qwen3.5-4B-Q8_0.gguf"

[runtime.opencode-fast]
host = "127.0.0.1"
port = 1200
model = "/home/angelo/.local/share/lmml/models/Qwen3.5-4B-Q8_0.gguf"
```

OpenCode config:

```json
{
  "provider": {
    "llamacpp": {
      "options": {
        "baseURL": "http://127.0.0.1:1200/v1"
      }
    },
    "llamacpp_fast": {
      "options": {
        "baseURL": "http://127.0.0.1:1200/v1"
      }
    }
  },
  "model": "llamacpp/Qwen3.5-4B-Q8_0.gguf",
  "small_model": "llamacpp_fast/Qwen3.5-4B-Q8_0.gguf",
  "compaction": {
    "auto": true,
    "prune": true,
    "reserved": 65536
  }
}
```

Troubleshooting when OpenCode cannot see lmml while the TUI says server ready:

1. Check the TUI Server tab URL. If it is `http://127.0.0.1:1200`, OpenCode
   must use `http://127.0.0.1:1200/v1`.
2. Verify the server:
   `curl -fsS http://127.0.0.1:1200/health`.
3. Verify OpenAI-compatible model listing:
   `curl -fsS http://127.0.0.1:1200/v1/models`.
4. Verify OpenCode sees the configured providers:
   `opencode models llamacpp` and `opencode models llamacpp_fast`.
5. Restart OpenCode after editing `~/.config/opencode/opencode.json`; OpenCode
   may not reload provider config in an already-running session.
6. If truncation still happens around `40000-45000` tokens after moving to the
   256k/Q8 profile, search for hidden caps in OpenCode wrapper configs:
   `~/.config/llama-server/models.tsv` and
   `~/.config/llama-server/defaults.env` have previously carried stale context
   values on this workstation.
7. Check `~/.config/opencode/oh-my-openagent.json` and
   `~/.config/opencode/validator.ts`; both have previously carried stale
   Q6/GlyphOS/`4010`/`4011` routing after `opencode.json` was already correct.

Troubleshooting hidden thinking / empty output:

If Qwen appears to generate nothing, spends the whole `max_tokens` budget in
`reasoning_content`, repeats, or aborts before producing visible text, inspect
the rendered generation prompt and the model's embedded GGUF chat template.
lmml no longer forces the local Qwen template override on Orion because it
proved too brittle across Qwen and Nemotron profiles.

Expected normal generation prefix:

```text
<|im_start|>assistant
```

Expected deep-agent opt-in prefix:

```text
<|im_start|>assistant
<think>
```

Forbidden default prefix:

```text
<|im_start|>assistant
<think>
</think>
```

Production template policy:

- Leave `chat_template = ""` in lmml model profiles unless a model-specific
  override has been validated with that exact GGUF.
- Keep `jinja = true` so llama-server can use the GGUF's embedded
  `tokenizer.chat_template`.
- Do not share one Qwen template across Qwen and Nemotron profiles.
- If a custom template is reintroduced later, it must be scoped to one model
  profile, tested with `/props`, and removed immediately if the server reports
  bad thinking or formatting behavior.

Compaction policy for this machine:

- Server context is `262144` tokens.
- OpenCode `compaction.reserved` stays at `65536`.
- OpenCode local model output limit stays at `18000`.
- OpenCode provider timeout stays at `7200s`.
- OpenCode provider `chunkTimeout` stays at `2400s` for background SSE runs.
- Usable input before compaction is `196608` tokens.
- Operator compact target is `90000-120000` live prompt tokens.
- Operator red zone is `120000-170000` live prompt tokens.
- Hard reject/compress threshold is about `170000-190000` live prompt tokens.
- `llama-server` parallel slots stay at `1` for this 11GB validation machine.
- Slot save path is `/home/angelo/.local/share/lmml/llama-slots`.
- KV cache type is `q8_0` for both K and V.
- Host cache is `--cache-ram 4096`.
- Qwen sampling should be profile-specific. For Qwen3.5 thinking mode use
  `temperature=0.6`, `top_p=0.95`, `top_k=20`, `min_p=0`; for non-thinking
  mode use `temperature=0.7`, `top_p=0.8`, `top_k=20`, `min_p=0`.
- Qwen3.5 9B multimodal mode requires a matching `mmproj` file. Do not claim
  image/video support unless the profile has an existing `mmproj` path and
  lmml-compat emits the correct llama.cpp projector argument.
- Do not leave `--parallel` on auto for deep mode on this machine. Auto selected
  four slots at 128k and exhausted KV cache under concurrent OpenCode background
  tasks.
- Do not spawn background subagents in Orion single-slot mode. Queue, summarize,
  or execute directly in the main agent unless the operator explicitly moves
  Sisyphus to `LMML_SISYPHUS_SUBAGENTS=1` or `2` before launching OpenCode.
- Do not treat slot save/restore as live overflow. It avoids repeated prefix
  processing; it does not make a live request larger than the active slot.
- A llama-server restart alone is not enough after changing OpenCode
  compaction. Restart OpenCode/Sisyphus as well so the harness reloads
  `~/.config/opencode/opencode.json`.
- The model reports a `262144` token training context. The 256k Q8 profile is
  the current validated Orion deep profile: `ctx_size=262144`,
  `compaction.reserved=65536`, `parallel=1`, `ubatch_size=128`, `-ctk q8_0`,
  `-ctv q8_0`, and `--cache-ram 4096`.

TUI-switchable Orion Qwen Q8 profiles:

```text
orion-qwen-q8-deep:
  ctx_size=262144
  parallel=1
  subagents=0 by default
  compact target=90000-120000
  hard compress/reject=170000-190000

orion-qwen-q8-balanced:
  ctx_size=262144
  parallel=2
  subagents=1 max
  per-slot theoretical context=131072
  compact target=60000-80000 per active agent
  hard compress/reject=100000-115000 per active agent

5070ti-qwen4b-fanout4:
  ctx_size=131072
  parallel=4
  subagents=3 max
  per-slot theoretical context=32768
  compact target=16000-24000 per active agent

5070ti-qwen4b-dual:
  ctx_size=262144
  parallel=2
  subagents=1 max
  per-slot theoretical context=131072
  compact target=60000-90000 per active agent

m6000-qwen9b-deep:
  ctx_size=262144
  parallel=1
  subagents=0 by default
  compact target=120000-170000

m6000-qwen9b-fanout4:
  ctx_size=262144
  parallel=4
  subagents=3 max
  per-slot theoretical context=65536
  compact target=32000-48000 per active agent

m6000-qwen9b-fanout6:
  ctx_size=262144
  parallel=6
  subagents=5 max after validation
  per-slot theoretical context=43690
  compact target=20000-30000 per active agent

5070ti-qwen9b-deep:
  ctx_size=196608
  parallel=1
  subagents=0 by default
  compact target=90000-130000

5070ti-qwen9b-balanced2:
  ctx_size=131072
  parallel=2
  subagents=1 max
  per-slot theoretical context=65536
  compact target=32000-48000 per active agent
```

The TUI `p` key switches the saved lmml runtime profile for the selected model.
It does not mutate a running OpenCode process. Stop/start `llama-server` after
switching profiles, and restart OpenCode after changing
`LMML_SISYPHUS_SUBAGENTS` or its provider limits.
- If the 256k Q8 profile stalls or host cache allocation fails, use the fallback
  `batch_size=256`, `ubatch_size=64`, `-ctk q4_1`, `-ctv q4_1`,
  `--cache-ram 6144`.
- Watch logs for `ggml_cuda_host_alloc` failures; they mean the requested
  pinned host-memory cache was not fully available.

Patch contract:

- Parse OpenCode config structurally as JSON. Do not use string replacement.
- Preserve unrelated providers, plugins, auth, UI settings, and unknown keys.
- Patch lmml-owned provider keys by default:
  - `provider.llamacpp`
  - `provider.llamacpp_fast`
- Default top-level routing is local-first:
  - `model` is set to the lmml `llamacpp/...` model.
  - `small_model` is set to the lmml `llamacpp_fast/...` model.
- Operators can preserve cloud routing with:
  - `--model-source existing`
  - `--small-model-source existing`
- `existing` preserves a present key and does not create it when missing.
- `none` never touches the key.
- `lmml` writes the lmml-managed local route.
- Existing top-level `model` or `small_model` values are reported as routing
  conflicts when `lmml` routing is selected, but they can be replaced by the
  normal apply flow after confirmation or with `--yes`.
- Existing lmml-owned provider entries that differ from lmml's desired provider
  config require `--force`.
- Treat `compaction.reserved` as opt-in because it affects global OpenCode
  behavior.
- `--dry-run` prints a structural diff and writes nothing.
- Before any write, create a timestamped backup next to the config file.
- Write through a temporary file, validate the result as JSON, then atomically
  replace the original where the platform supports it.
- Print the backup path and rollback command after a successful write.
- `--path <file>` allows non-default config locations.
- `--yes` may apply provider changes and local-first routing changes
  non-interactively. Conflicting existing `llamacpp` providers require explicit
  `--force`.
- `--rollback <backup-file>` restores a previous backup after validating that
  the backup is readable JSON.

TUI flow:

```text
OpenCode Setup Wizard
1. Check OpenCode config
2. Review lmml runtime profiles
3. Preview provider JSON
4. Review structural diff
5. Apply with backup
6. Verify resulting config
```

The wizard is a single entry point for the guided flow, not an automatic
one-keystroke patcher. Each mutating step requires explicit confirmation.

The TUI should expose the same concepts through the Server tab:

- profile selector
- port per profile
- model per profile
- context/gpu layers/batch/thread settings per profile
- health/status per profile
- copyable OpenCode config snippet
- clear warning when the configured port/model differs from the harness config

## Detached Profile Defaults

Initial detached multi-instance defaults:

| Profile | Port | Purpose |
|---|---:|---|
| `opencode` | `4010` | full coding-agent runtime |
| `opencode-fast` | `4011` | small/fast helper runtime |

Suggested settings should be conservative and editable:

- host: `127.0.0.1`
- API base URL: `http://127.0.0.1:<port>/v1`
- GPU layers: `-1` when CUDA is available
- startup timeout: at least `30s`; long model loads may need more
- request timeout guidance: `7200s`
- chunk timeout guidance: `2400s` for local long-running background streams

## Health And Logs

- Poll `/v1/health` every `5s` while a profile is running.
- Mark a profile `unhealthy` after three consecutive failed health checks.
- Do not auto-restart unhealthy profiles in the first implementation. Surface
  the state in `runtime status`, logs, and the TUI.
- Write one log file per profile under the lmml state directory.
- `lmml runtime logs <profile>` prints the current log. `--follow` tails it.

## Model Change And Restart Semantics

For the first implementation, model changes use a cold restart with a clear
warning:

- If the profile is stopped, update config immediately.
- If the profile is running, show/require confirmation.
- On confirmation, send graceful shutdown to the old server, wait up to `30s`,
  then force kill if needed.
- Start the new server with the updated model and mark the config active only
  after `/v1/health` succeeds.
- If restart fails, restore the previous config and report the failure.

Blue/green restart is deferred because `llama-server` owns the configured port.
It can be revisited later with a local proxy layer, but this phase should not
pretend swaps are seamless while long agent requests may be in flight.

## Install And First-Run Flow

The binary installer does not guarantee that a model is already present.

After install:

- `lmml doctor` verifies host prerequisites.
- `lmml` TUI remains the primary first-run flow for detection, build, model
  scan/download, and manual server validation.
- `lmml runtime start <profile>` must fail clearly when no model is configured
  or the model path does not exist.
- `lmml runtime print-config opencode` can still print endpoint config before
  the server is running, but it should warn when profile models are unset.

## Acceptance Criteria

- Config round-trip tests cover the profile TOML schema and defaults.
- CLI tests verify `runtime status`, `status --json`, `print-config opencode`,
  and unknown-profile exit codes.
- Process tests start full and fast profiles simultaneously on separate ports;
  both answer `/v1/health`.
- Failure tests prove a port conflict or health failure in one profile does not
  stop the other profile.
- Stale PID tests prove missing PIDs become `stopped` and live-but-unhealthy
  processes become `unhealthy`.
- Log tests prove `runtime logs <profile>` can read profile logs without the
  TUI.
- Model change tests prove stopped updates, running cancellation, and confirmed
  cold restart behavior.
- Clean install smoke covers installed `lmml runtime print-config opencode` and
  the clear missing-model failure path.
- Docs clearly state that harness runtime uses `llama-server`, while
  `llama-cli` remains a diagnostic tool.
