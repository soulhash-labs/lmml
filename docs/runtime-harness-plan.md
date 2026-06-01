# lmml Managed Runtime Harness Plan

This plan defines how lmml should provide managed local llama.cpp runtimes for
coding harnesses. The first target is OpenCode because the local config already
uses OpenAI-compatible HTTP endpoints.

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
      "name": "llama.cpp",
      "options": {
        "baseURL": "http://127.0.0.1:4010/v1",
        "timeout": 7200000,
        "chunkTimeout": 300000
      }
    },
    "llamacpp_fast": {
      "npm": "@ai-sdk/openai-compatible",
      "name": "GlyphOS fast",
      "options": {
        "baseURL": "http://127.0.0.1:4011/v1",
        "timeout": 7200000,
        "chunkTimeout": 300000
      }
    }
  },
  "compaction": {
    "reserved": 32768
  }
}
```

That means lmml should support at least two managed runtime profiles:

- full profile on `127.0.0.1:4010`
- fast/small profile on `127.0.0.1:4011`

The long-run preset is:

- request timeout: `7200s`
- chunk timeout: `300s`
- reserved compaction tokens: `32768`

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
port         = 4010
model        = "~/.local/share/lmml/models/Qwen3.5-4B-Q6_K.gguf"
ctx_size     = 65536
gpu_layers   = -1
batch_size   = 512
threads      = 8
parallel     = 4
extra_args   = []
autostart    = false

[runtime.profiles.opencode-fast]
host         = "127.0.0.1"
port         = 4011
model        = "~/.local/share/lmml/models/Qwen3.5-4B-Q6_K.gguf"
ctx_size     = 32768
gpu_layers   = -1
batch_size   = 512
threads      = 8
parallel     = 2
extra_args   = []
autostart    = false
```

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
port            = 4010
model           = "/home/angelo/.local/share/lmml/models/Qwen3.5-4B-Q6_K.gguf"
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
opencode       ready      12345   http://127.0.0.1:4010/v1    Qwen3.5-4B-Q6_K.gguf
opencode-fast  stopped    -       http://127.0.0.1:4011/v1    Qwen3.5-4B-Q6_K.gguf
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
        "Qwen3.5-4B-Q6_K.gguf": {
          "name": "Qwen3.5-4B-Q6_K.gguf (lmml full)"
        }
      },
      "options": {
        "baseURL": "http://127.0.0.1:4010/v1",
        "timeout": 7200000,
        "chunkTimeout": 300000
      }
    },
    "llamacpp_fast": {
      "npm": "@ai-sdk/openai-compatible",
      "name": "lmml llama.cpp fast",
      "models": {
        "Qwen3.5-4B-Q6_K.gguf": {
          "name": "Qwen3.5-4B-Q6_K.gguf (lmml fast)"
        }
      },
      "options": {
        "baseURL": "http://127.0.0.1:4011/v1",
        "timeout": 7200000,
        "chunkTimeout": 300000
      }
    }
  },
  "model": "llamacpp/Qwen3.5-4B-Q6_K.gguf",
  "small_model": "llamacpp_fast/Qwen3.5-4B-Q6_K.gguf",
  "compaction": {
    "auto": true,
    "prune": true,
    "reserved": 32768
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

## Profile Defaults

Initial local defaults:

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
- chunk timeout guidance: `300s`

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
