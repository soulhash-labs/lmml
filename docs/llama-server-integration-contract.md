# llama-server Integration Contract

This document defines the target contract between lmml, `lmml-compat`,
managed `llama-server` profiles, OpenAI-compatible coding harnesses such as
OpenCode and Continue, and Anthropic Messages clients routed through
`lmml-node`.

Current status: planned Phase 11 contract. Some fields already exist in the v2
state schema and `lmml-compat`; others are explicit implementation tasks.
Do not describe the whole contract as shipped until the matching todo items and
tests are complete.

## Topology

lmml manages long-running `llama-server` processes. Coding harnesses talk to
those processes over OpenAI-compatible HTTP, not `llama-cli`.

Anthropic Messages clients use `lmml-node` as a compatibility adapter:

```text
Claude Code -> lmml-node /v1/messages -> llama-server /v1/chat/completions
```

`lmml-node` translates text messages, tool schemas, tool calls, non-streaming
responses, synthesized SSE responses, and Anthropic-shaped errors. Raw
`llama-server` remains OpenAI-compatible; it does not own `/v1/messages`.

Default OpenCode provider targets:

| Profile | Port | Purpose |
|---|---:|---|
| `opencode` | `1200` | primary/full coding-agent runtime |
| `opencode-fast` | `1200` | secondary fast/small lane using the active TUI server by default |

Each provider lane declares:

- one detached `llama-server` process group
- one static host/port
- one model
- one profile-specific log file
- one runtime state entry with PID, status, health result, and failure count

Ports are static because harness config files store base URLs. lmml must detect
port conflicts and fail clearly rather than silently choosing a new port.

Current workstation-proven OpenCode route:

```text
OpenCode -> http://127.0.0.1:1200/v1 -> lmml TUI llama-server
server context: 131072 tokens
OpenCode compaction.reserved: 32768 tokens
usable input before compaction: 98304 tokens
practical single-agent input target: 80000-90000 tokens
hard reject/compress threshold: 96000-100000 tokens
```

Both OpenCode provider lanes should target the active TUI-managed `1200` server
by default.

## Profile Schema Target

Existing v2 profiles currently live under `runtime.opencode` and
`runtime.opencode-fast`. The target schema should grow toward these fields while
remaining backward-compatible through `#[serde(default)]`.

```toml
[runtime.opencode]
copied_from = "preset-12gb-workstation"
host = "127.0.0.1"
port = 1200
model = "~/.local/share/lmml/models/mistral-7b-q4_k_m.gguf"
ctx_size = 32768
compaction_reserved = 16384
gpu_layers = -1
batch_size = 512
ubatch_size = 512
threads = 8
flash_attn = "auto"
continuous_batch = true
parallel = 1
split_mode = "auto"
api_key = ""
cache_type_k = "f16"
cache_type_v = "f16"
fit = "on"
fit_target_mb = 1024
fit_ctx = 4096
prompt_cache = true
cache_ram_mb = 8192
cache_reuse = 0
slot_save_path = ""
extra_args = []
autostart = false
tuned_for_vram_mb = 12288
```

Field meanings:

- `compaction_reserved`: harness-level reserved tokens. For OpenCode this must
  match `opencode.json` when lmml has configured OpenCode.
- `gpu_layers`: `-1` means automatic/offload as much as allowed, `0` means
  intentional CPU-only.
- `flash_attn`: `auto`, `true`, or `false`.
- `continuous_batch`: maps to continuous batching flags when supported.
- `parallel`: maps to `-np` / `--parallel`.
- `split_mode`: `auto`, `none`, `layer`, `row`, or `tensor`.
- `cache_type_k` / `cache_type_v`: maps to `-ctk` and `-ctv`.
- `fit`, `fit_target_mb`, `fit_ctx`: maps to `-fit`, `-fitt`, and `-fitc`
  where supported.
- `prompt_cache`, `cache_ram_mb`, `cache_reuse`, and `slot_save_path`: prompt
  cache controls for long-running coding sessions.
- `extra_args`: appended last, but must not duplicate lmml-owned flags.

## Built-In Presets

lmml should ship read-only presets. Operators copy presets into editable
profiles; presets themselves are never modified.

Fleet-specific profile composition is tracked in
[`docs/lmml-fleet-profiles.md`](lmml-fleet-profiles.md). Public profiles must
stay generic and separate validated profiles from proposed profiles.

```sh
lmml profile copy preset-12gb-workstation opencode
lmml profile copy preset-12gb-workstation opencode-fast
```

Target presets:

| Preset | Target hardware | VRAM | ctx_size | compaction_reserved | flash_attn | Use case |
|---|---|---:|---:|---:|---|---|
| `preset-8gb-desktop` | 8 GB desktop | 8192 | 16384 | 4096 | `auto` | short local tasks |
| `preset-11gb-256k-qwen35-4b-q8` | GTX 1080 Ti / Pascal | 11264 | 262144 | 65536 | `auto` | validated Orion 256k single-agent Qwen3.5 4B Q8 |
| `preset-12gb-workstation` | RTX 3060/4060 class | 12288 | 32768 | 16384 | `auto` | daily coding |
| `preset-16gb-workstation` | 16 GB workstation | 16384 | 65536 | 16384 | `auto` | longer coding |
| `preset-24gb-single-agent` | 24 GB workstation | 24576 | 131072 | 32768 | `auto` | 80k single-agent work |
| `preset-24gb-deeprun` | 24 GB workstation | 24576 | 262144 | 65536 | `auto` | long deep runs |
| `preset-32gb-workstation` | 32 GB workstation | 32768 | 262144 | 65536 | `auto` | heavy coding |
| `preset-48gb-workstation` | 48 GB workstation | 49152 | 262144 | 65536 | `auto` | multiple harnesses |
| `preset-80gb-server` | A100/H100 class | 81920 | 262144 | 65536 | `auto` | large model runs |
| `preset-auto` | detected hardware | detected | computed | computed | `auto` | first-run default |

The Quadro M6000 24 GB, Qwen3.5 9B Q8, `ctx_size = 262144`,
`compaction_reserved = 65536` result is treated as empirical evidence for the
24 GB tier, not proof that every 24 GB card/model combination will fit. Its
practical long-run history target is `196608` tokens with `parallel = 1`. If
runtime memory pressure appears, fall back to `ctx_size = 196608`,
`compaction_reserved = 65536`, `parallel = 1` (`131072` practical), then
`ctx_size = 131072`, `compaction_reserved = 32768`, `parallel = 1` (`98304`
practical).

Qwen3.5 9B profile metadata:

```text
native context: 262144 tokens
minimum context for thinking: 128000 tokens
thinking sampling: temperature=0.6 top_p=0.95 top_k=20 min_p=0
non-thinking sampling: temperature=0.7 top_p=0.8 top_k=20 min_p=0
vision/video: requires matching mmproj file loaded with the main GGUF
MTP: supported by model, disabled by default until profiled
```

The runtime schema should add `temperature`, `top_p`, `top_k`, `min_p`,
`mtp`, and `mmproj` fields. `mmproj` must be validated as an existing file
before lmml advertises image/video support for a profile.

Gemma4 12B QAT Q4_K_M profile metadata:

```text
main model: Gemma4-12B-QAT-Q4_K_M.gguf
MTP draft model: mtp-gemma-4-12B-it.gguf
validated llama.cpp mode: -md mtp-gemma-4-12B-it.gguf --spec-type draft-mtp
sampling: temperature=0.6 top_k=64 top_p=0.9 min_p=0.05 repeat_penalty=1.1
profile name: gemma4-12b-mtp-q4km
```

The Gemma4 profile requires both GGUF files under the configured lmml model
directory. It keeps MTP enabled only for the dedicated Gemma4 profile instead
of changing global defaults.

For `ctx_size = 131072` single-agent coding, `compaction_reserved = 32768` is
the validated operating point. `compaction_reserved = 65536` is appropriate for
the 256k Quadro M6000 deep-run profile, but it is too conservative for an 80k
single-agent target at 131k context because OpenCode/tooling overhead can reduce
the observed working wall to roughly 40k-45k tokens.

The GTX 1080 Ti / 11 GB `preset-11gb-256k-qwen35-4b-q8` is the current
validated Orion deep profile. It exists because the Qwen3.5 4B validation model
reports a training context of 262144 tokens, and the Q8 GGUF loaded and decoded
successfully at that context with Q8 KV cache plus host cache. At 256k, FP16 KV
cache is expected to OOM on 11 GB hardware. The preset therefore requires:

```toml
[profiles.opencode-256k-11gb]
copied_from          = "preset-11gb-256k-qwen35-4b-q8"
port                 = 1200
model                = "~/.local/share/lmml/models/Qwen3.5-4B-Q8_0.gguf"
ctx_size             = 262144
compaction_reserved  = 65536
gpu_layers           = -1
batch_size           = 512
ubatch_size          = 128
threads              = 8
flash_attn           = "auto"
continuous_batch     = true
parallel             = 1
cache_type_k         = "q8_0"
cache_type_v         = "q8_0"
cache_ram_mb         = 4096
cache_reuse          = 0
slot_save_path       = "~/.local/share/lmml/llama-slots"
tuned_for_vram_mb    = 11264
opencode_timeout_s   = 7200
opencode_chunk_timeout_s = 2400
opencode_output_limit = 18000
```

Fallback if the 256k Q8 profile stalls or cannot allocate enough host cache:

```toml
batch_size           = 256
ubatch_size          = 64
cache_type_k         = "q4_1"
cache_type_v         = "q4_1"
cache_ram_mb         = 6144
```

The 256k context math is different from the 131k profile:

```text
262144 - 65536 = 196608 theoretical working room
196608 - ~22500 overhead = ~174000 estimated usable working room
```

For a focused single-agent session with minimal tool overhead,
`compaction_reserved = 49152` is a possible reclaim-tokens experiment. Start at
`65536`, measure the observed wall in server logs, then reduce to `49152` only
if the session has enough output/tool headroom.

Field test result: the 256k Q8 profile loaded and decoded successfully on the
GTX 1080 Ti with about 2.4 GiB VRAM free after load. The server reported
`cache_reuse is not supported by this context, it will be disabled`, so the
validated 256k profile should use prompt cache/checkpoints and leave
`cache_reuse = 0`.

The active OpenCode route for this profile is `http://127.0.0.1:1200/v1` for
both `llamacpp` and `llamacpp_fast`. The active top-level model keys are:

```text
model:       llamacpp/Qwen3.5-4B-Q8_0.gguf
small_model: llamacpp_fast/Qwen3.5-4B-Q8_0.gguf
```

`opencode.json` alone may not be the whole integration. If a client
has validator, category, or provider extension files, keep them aligned with the
same model and `1200` base URL after provider config is corrected.

For 128k+ contexts, slot count is part of the memory budget. The 11GB GTX 1080
Ti validation machine failed with auto `n_parallel = 4` under concurrent
OpenCode tasks. Long-context presets should prefer `parallel = 1` until lmml has
a VRAM budget validator that accounts for model weights, KV cache, prompt cache,
batch buffers, and concurrent slots together.

## Context Guard

lmml must protect coding harnesses from immediate context exhaustion:

```text
ctx_size >= compaction_reserved + 4096
```

`lmml doctor` reports violations read-only. `lmml runtime start <profile>` must
block before spawning when the guard fails.

Recommended `compaction_reserved` is derived from `ctx_size`:

```text
if ctx_size <= 16384:  max(ctx_size / 4, 4096)
if ctx_size <= 65536:  16384
if ctx_size <= 98304:  32768
if ctx_size <= 131072: 32768
else:                  65536
```

Mismatch between the stored value and the recommendation is a soft warning
because users may intentionally override it. Mismatch between lmml's OpenCode
profile value and `opencode.json` is a hard `runtime start` block for OpenCode
profiles because it means the harness and server configuration have drifted.

Doctor should also estimate the effective harness working window, not only the
hard minimum guard. For OpenCode/Sisyphus-style coding sessions, use a
conservative default overhead estimate of `20000-24000` tokens for system
prompts, tool schemas, response reserve, reasoning metadata, and prompt-cache
margin.

Example warning:

```text
warning: profile 'opencode' has limited effective working context
ctx_size:             131072
compaction.reserved:  65536
theoretical window:   65536
estimated overhead:   ~22500
estimated usable:     ~43000 tokens

For a working window above 65k, reduce compaction.reserved to 32768
or increase ctx_size to 196608+.
```

If an 80k single-agent profile still truncates around 40k-45k after
`compaction_reserved = 32768` and a full OpenCode restart, check for stale caps
in external LMM/OpenCode configuration:

```sh
grep -RInE '65536|43000|40960|contextWindow|max_input|maxInput|ctx|input.*token|compaction|reserve' \
  ~/.config ~/.local/share/lmml ~/repos 2>/dev/null | \
  grep -v '.git' | grep -v 'node_modules' | head -200
```

On the validation machine, stale legacy candidates existed in
`~/.config/llama-server/models.tsv` and `~/.config/llama-server/defaults.env`.
Those files are not the active OpenCode route when OpenCode uses
`http://127.0.0.1:1200/v1`, but they are exactly the kind of hidden cap that
must be eliminated if a wrapper still participates in the request path.

Doctor should also surface context capability left on the table when
`llama-server` logs or GGUF metadata show the configured context is below the
model's training context:

```text
warning: profile 'opencode' is below the model training context
ctx_size:          131072
model train ctx:   262144

The model supports up to 262144 tokens. To test the full training context,
set ctx_size = 262144. On hardware with less than 16 GB VRAM, also set
cache_type_k = "q8_0", cache_type_v = "q8_0", and cache_ram_mb >= 4096.
```

## VRAM Budget Guard

`lmml doctor` and `lmml runtime start` should estimate whether a profile fits
the detected GPU memory:

```text
vram_required = model_weights + kv_cache + runtime_overhead
budget_ok = vram_required + safety_margin <= detected_vram
```

Use conservative defaults until model metadata support is strong enough for
exact estimates:

- runtime overhead: `0.5 GiB`
- safety margin: `0.5 GiB`
- tight-margin warning threshold: `< 1.0 GiB`

KV cache estimate should account for:

- `ctx_size`
- model layer/head metadata when available
- `cache_type_k` / `cache_type_v`
- flash-attention resolution
- `parallel` slot count
- combined running profiles on the same GPU

For 131k+ coding profiles on 24 GB-class cards, prefer `cache_type_k = "q8_0"`
and `cache_type_v = "q8_0"` when the probed `llama-server` binary supports
`-ctk` and `-ctv`. This roughly halves KV cache pressure compared with FP16 and
is usually a better tradeoff for long coding context than shrinking the live
window. Keep FP16 as the compatibility fallback when the flags are unavailable
or when a model/backend combination shows quality or stability regressions.

For 256k contexts on 11 GB-class cards, `q8_0` KV cache plus `cache_ram_mb >=
4096` is required for the test profile. Watch server logs for pinned host-memory
allocation failures such as:

```text
ggml_cuda_host_alloc: failed to allocate 4096 MiB of pinned memory
```

If that appears, reduce `cache_ram_mb`, try the Q4 fallback profile, or accept
slower unpinned host-memory behavior if the server remains healthy.

If a single profile exceeds detected VRAM, `doctor` exits `1` and
`runtime start` refuses to spawn. If combined running profiles are tight, report
a warning unless the combined budget exceeds detected VRAM.

CPU/system-RAM spill is allowed by llama.cpp, but lmml should surface it as a
performance cliff, not as a success equivalent to fitting in VRAM.

## lmml-compat Flag Generation

Users do not write raw `llama-server` flags. `lmml-compat` translates profile
fields into argv based on detected binary capabilities.

Local `llama-server --help` on 2026-06-02 confirmed support in the current
installed build for:

- `-ctk` / `--cache-type-k`
- `-ctv` / `--cache-type-v`
- `-fit`, `-fitt`, `-fitc`
- `-np` / `--parallel`
- `-cb` / `--cont-batching`
- `--split-mode`
- `--api-key`
- `--cache-prompt`, `--cache-reuse`, `--cache-ram`
- `--slot-save-path`
- `-md` / `--model-draft` / `--spec-draft-model`
- `--spec-type`
- `--temp` / `--temperature`
- `--top-k`, `--top-p`, `--min-p`, `--repeat-penalty`

Target emission rules:

| Profile field | Flag |
|---|---|
| `model` | `-m` / `--model` |
| `ctx_size` | `-c` / `--ctx-size` |
| `host` | `--host` |
| `port` | `--port` |
| `gpu_layers` | `-ngl` / `--gpu-layers` / `--n-gpu-layers` |
| `batch_size` | `-b` / `--batch-size` |
| `ubatch_size` | `-ub` / `--ubatch-size` |
| `threads` | `-t` / `--threads` |
| resolved `flash_attn` | `-fa` / `--flash-attn` |
| `continuous_batch` | `-cb` / `--cont-batching` or no-cont variant |
| `parallel` | `-np` / `--parallel` |
| `split_mode != auto` | `--split-mode <mode>` |
| `api_key` | `--api-key` |
| `cache_type_k` | `-ctk` / `--cache-type-k` |
| `cache_type_v` | `-ctv` / `--cache-type-v` |
| `fit` | `-fit <on|off>` |
| `fit_target_mb` | `-fitt <MiB>` |
| `fit_ctx` | `-fitc <tokens>` |
| `prompt_cache` | `--cache-prompt` / `--no-cache-prompt` |
| `draft_model` | `-md` / `--model-draft` |
| `spec_type` | `--spec-type <type>` |
| sampling extras | `--temp`, `--top-k`, `--top-p`, `--min-p`, `--repeat-penalty` |
| `cache_reuse` | `--cache-reuse <tokens>` |
| `cache_ram_mb` | `--cache-ram <MiB>` |
| `slot_save_path` | `--slot-save-path <path>` |
| `extra_args` | appended last |

If a requested optional flag is unsupported by the probed binary, `lmml-compat`
must omit it and return a warning. Required flags such as model, host, port,
context, GPU layers, and batch size must fail validation if no supported spelling
exists.

## Runtime CLI Contract

The runtime CLI is the headless control plane for harness integration. The TUI
may wrap these commands, but it must preserve the same safety contract.

Current implemented commands:

```sh
lmml runtime status
lmml runtime start <profile> --detach
lmml runtime stop <profile>
lmml runtime logs <profile>
lmml runtime logs <profile> --follow
lmml runtime print-config opencode
lmml runtime configure opencode --dry-run
lmml runtime configure opencode [--yes] [--force]
lmml runtime configure opencode --path <file>
lmml runtime configure opencode --rollback <backup-file>
lmml runtime configure opencode --model-source existing|lmml|none
lmml runtime configure opencode --small-model-source existing|lmml|none
```

Target commands to add:

```sh
lmml runtime status --json
lmml runtime health <profile>
lmml runtime validate <profile>
lmml runtime restart <profile>
lmml runtime restart <profile> --model <path>

lmml profile list
lmml profile presets
lmml profile show <profile>
lmml profile copy <preset-name> <profile>
lmml profile copy --from-profile <source-profile> <profile>
lmml profile validate <profile>
lmml profile set <profile> <key> <value>
```

### `lmml runtime status`

Purpose: read persisted runtime state, reconcile stale PIDs, and print a stable
human-readable table.

Behavior:

- Uses a non-creating read path when state does not exist.
- Reconciles stale PIDs in memory; when a stale PID is confirmed, future
  implementation should persist the correction.
- Does not start, stop, or configure external harnesses.
- Returns `1` if any profile is `failed` or `unhealthy`.

Target status table:

```text
profile        status     pid     url                         model
opencode       ready      12345   http://127.0.0.1:1200/v1    model.gguf
opencode-fast  ready      12345   http://127.0.0.1:1200/v1    model.gguf
```

### `lmml runtime status --json`

Purpose: machine-readable status for scripts, CI, and harness launchers.

Target runtime status JSON output:

```json
{
  "profiles": [
    {
      "name": "opencode",
      "status": "ready",
      "pid": 12345,
      "url": "http://127.0.0.1:1200/v1",
      "model": "/home/user/.local/share/lmml/models/mistral-7b-q4_k_m.gguf",
      "log_path": "/home/user/.local/share/lmml/logs/profile-opencode.log",
      "last_health": "ok",
      "failure_count": 0
    }
  ]
}
```

Exit codes:

- `0`: all profiles are `ready` or `stopped`
- `1`: one or more profiles are `failed` or `unhealthy`
- `2`: state/config cannot be parsed

### `lmml runtime validate <profile>`

Purpose: run the same pre-spawn gate as `runtime start`, without spawning.

Checks:

1. profile exists
2. model path exists and is readable
3. `llama-server` binary exists and is executable
4. port is available or owned by this profile
5. context guard passes
6. OpenCode compaction drift is absent for OpenCode profiles
7. single-profile VRAM budget passes
8. combined running-profile VRAM budget is not exceeded
9. requested `lmml-compat` flags are supported or safely warnable

Exit codes:

- `0`: hard checks pass
- `1`: hard validation failure
- `2`: bad arguments or unknown profile

### `lmml runtime health <profile>`

Purpose: query the profile's health endpoint once and update/report status.

Behavior:

- Reads the profile URL from lmml state/config.
- Calls `GET /v1/health`.
- Accepts HTTP 200 with `status = ok` or `status = loading model`.
- Prints the raw status and normalized lmml status.
- Does not restart the process.

### `lmml runtime start <profile> --detach`

Purpose: start one managed `llama-server` profile as a detached process group.

Current behavior: `--detach` is required; foreground log streaming is not yet
implemented.

Pre-spawn sequence:

1. load/create lmml state
2. validate the profile
3. refuse double-start when an existing managed PID is alive
4. check port availability
5. probe `llama-server` capabilities
6. build argv through `lmml-compat`
7. spawn a new process group
8. write profile-specific log
9. poll `/v1/health` until ready or timeout
10. persist `ready` only after health succeeds

Failure behavior:

- If readiness fails, terminate the whole process group.
- If cleanup fails, persist `failed` with the cleanup reason.
- Never overwrite a live managed PID with a new process.

### `lmml runtime stop <profile>`

Purpose: stop one managed profile without touching other profiles.

Behavior:

- Validates the profile name.
- If no PID is recorded, marks profile stopped.
- If PID is stale/missing, clears it.
- If PID exists but does not look like the managed `llama-server`, refuses to
  kill and marks the profile unhealthy.
- Sends SIGTERM to the process group, waits, then sends SIGKILL if needed.
- Verifies the process is gone or a zombie before persisting `stopped`.

Future flag:

```sh
lmml runtime stop <profile> --force
```

`--force` may clear stale lmml state and port registry entries, but still must
not kill an unknown non-lmml process.

### `lmml runtime restart <profile>`

Purpose: cold restart a profile on the same static port.

Target behavior:

- Validate the new desired config first.
- Warn that in-flight requests will be disconnected.
- Stop the old server.
- Start the new server on the same port.
- Persist new active state only after `/v1/health` succeeds.
- If restart fails, restore the previous config and report the failure.

For model swaps:

```sh
lmml runtime restart opencode --model ~/.local/share/lmml/models/new.gguf
```

The profile config is committed only after the new server is healthy.

### `lmml runtime logs <profile> [--follow]`

Purpose: inspect profile logs without opening the TUI.

Behavior:

- Validates profile names before constructing log paths.
- Non-follow mode prints the current log file and exits.
- `--follow` starts at the current end of file and tails new output.
- `--follow` waits for the log file to appear instead of failing immediately.

### `lmml runtime print-config opencode`

Purpose: print ready-to-paste OpenCode JSON without mutating any file.

Rules:

- Uses current profile URLs.
- Uses current profile model names.
- Uses `compaction_reserved` from the primary profile once that schema field is
  implemented.
- Warns if profile models are unset.
- May print a complete local-first example including `model` and `small_model`.

### `lmml runtime configure opencode`

Purpose: surgically patch OpenCode config after review.

Required options:

```sh
lmml runtime configure opencode --dry-run
lmml runtime configure opencode --yes
lmml runtime configure opencode --force
lmml runtime configure opencode --path <file>
lmml runtime configure opencode --rollback <backup-file>
lmml runtime configure opencode --model-source existing|lmml|none
lmml runtime configure opencode --small-model-source existing|lmml|none
```

Defaults:

- provider entries are lmml-owned
- top-level `model` routing is local-first
- top-level `small_model` routing is local-first
- unrelated user config is preserved

Conflict policy:

- Existing lmml-owned provider conflicts require `--force`.
- Top-level cloud routing can be replaced by the normal confirmation flow or
  `--yes` because local-first is the default.
- Operators can preserve cloud routing with
  `--model-source existing --small-model-source existing`.

### `lmml profile presets`

Purpose: list read-only built-in presets.

Target output:

```text
preset-8gb-desktop       8 GB   ctx 16384    reserved 4096
preset-12gb-workstation  12 GB  ctx 32768    reserved 16384
preset-12gb-proven-qwen  11 GB  ctx 131072   reserved 32768
preset-11gb-256k-qwen    11 GB  ctx 262144   reserved 65536  q8 KV + cache_ram
preset-24gb-deeprun      24 GB  ctx 262144   reserved 65536
```

### `lmml profile copy`

Purpose: create an editable user profile from a validated baseline.

Commands:

```sh
lmml profile copy preset-12gb-workstation opencode
lmml profile copy --from-profile opencode my-derived-profile
```

Rules:

- Preset sources are accepted by default.
- Copying from another user profile requires `--from-profile`.
- Copying from a user profile runs strict validation first.
- New profiles record `copied_from` for audit/debugging only.

### `lmml profile set`

Purpose: scriptable profile edits without raw TOML editing.

Example:

```sh
lmml profile set opencode ctx_size 131072
lmml profile set opencode compaction_reserved 32768
lmml profile set opencode cache_type_k q8_0
lmml profile set opencode cache_type_v q8_0
```

Rules:

- Validates key names and value types.
- Recomputes and displays recommended `compaction_reserved` when `ctx_size`
  changes.
- Does not rewrite external harness config; tells the user to run
  `lmml runtime configure opencode`.

## Flash Attention Resolution

`flash_attn = "auto"` resolves in this order:

1. If the binary lacks flash-attention flags, skip and warn.
2. If GPU architecture is unknown, skip and warn.
3. For Maxwell/Pascal-era GPUs, including `sm_61`, skip by default and warn.
4. For Volta/newer GPUs, enable when `ctx_size > 8192`.
5. `flash_attn = "true"` forces emission when the binary supports the flag.
6. `flash_attn = "false"` suppresses the flag.

This rule is conservative. Users may force flash attention after verifying their
specific GPU and llama.cpp build.

## Prompt Cache And Long-Run Features

Long coding sessions should use llama.cpp prompt/cache features explicitly, not
through opaque `extra_args`.

Target managed fields:

- `prompt_cache`
- `cache_ram_mb`
- `cache_reuse`
- `slot_save_path`

`slot_save_path` should be profile-specific when enabled. The default can remain
empty until lmml adds UI for durable slot state.

Speculative decoding is valuable but should remain a later contract slice. It
needs a draft model profile, model compatibility checks, and clear failure
behavior before becoming a first-class preset field.

## Process Supervision

lmml remains un-daemonised. Runtime processes are detached process groups and
state is persisted in `state.toml`.

Required behavior:

- stale PID reconciliation on TUI startup and `lmml runtime status`
- process command/name validation before stop
- process-group SIGTERM then SIGKILL
- no killing unknown processes solely because a PID matches stale state
- per-profile logs
- profile isolation: one profile failing must not stop another

Health polling target:

- `GET /v1/health` every `5s`
- mark `unhealthy` after 3 consecutive failed polls
- recover to `ready` after the first successful poll
- no auto-restart in the first implementation

## OpenCode Management

`lmml doctor` remains read-only. It may report OpenCode drift and recommend
commands, but it must not edit `opencode.json`.

`lmml runtime print-config opencode` and `lmml runtime configure opencode`
should use the primary profile's `compaction_reserved` value. This value must
not be hardcoded to `32768` or `65536`.

`configure` remains surgical:

- parse JSON structurally
- preserve unrelated user config
- back up before writing
- patch lmml-owned providers
- local-first routing by default
- explicit routing flags for preserving cloud model choices
- write `compaction.reserved` only through the same reviewed configure flow

## Implementation Phases

1. Extend profile schema and `lmml-compat` with cache, fit, split, flash-attn
   tri-state, API key, and compaction fields.
2. Add preset definitions and `lmml profile copy`.
3. Add context and VRAM budget validators shared by `doctor` and
   `runtime start`.
4. Add OpenCode compaction drift detection and configure/print-config wiring.
5. Add ongoing runtime health polling and `runtime status --json`.
6. Add multi-profile process tests for full and fast runtimes.
7. Add TUI wizard/profile editor after the CLI contract is stable.
