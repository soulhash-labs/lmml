# OpenCode 1200 Evidence Snapshot

## Current Superseding Snapshot — 2026-06-03 Orion Qwen Q8 256k

The current active route supersedes the earlier Q6/128k snapshot below:

```text
OpenCode -> http://127.0.0.1:1200/v1 -> lmml TUI llama-server
model: Qwen3.5-4B-Q8_0.gguf
server ctx_size: 262144
OpenCode compaction.reserved: 65536
OpenCode input limit: 196608
OpenCode output limit: 18000
OpenCode timeout: 7200s
OpenCode chunkTimeout: 2400s
llama-server parallel: 1
KV cache: q8_0 / q8_0
cache_ram: 4096 MiB
slot save path: /home/angelo/.local/share/lmml/llama-slots
```

Verified `/v1/models` metadata:

```text
id: Qwen3.5-4B-Q8_0.gguf
n_ctx: 262144
n_ctx_train: 262144
n_params: 4205751296
size: 4471435264
```

Current required OpenCode provider shape:

```json
{
  "provider": {
    "llamacpp": {
      "options": {
        "baseURL": "http://127.0.0.1:1200/v1",
        "timeout": 7200000,
        "chunkTimeout": 2400000
      },
      "models": {
        "Qwen3.5-4B-Q8_0.gguf": {
          "name": "Qwen3.5-4B-Q8_0.gguf (lmml Qwen Q8 complex)",
          "limit": {
            "context": 262144,
            "input": 196608,
            "output": 18000
          }
        }
      }
    },
    "llamacpp_fast": {
      "options": {
        "baseURL": "http://127.0.0.1:1200/v1",
        "timeout": 7200000,
        "chunkTimeout": 2400000
      },
      "models": {
        "Qwen3.5-4B-Q8_0.gguf": {
          "name": "Qwen3.5-4B-Q8_0.gguf (lmml Qwen Q8 fast)",
          "limit": {
            "context": 262144,
            "input": 196608,
            "output": 18000
          }
        }
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

Operational lessons from this snapshot:

- Restart OpenCode after editing provider/category/validator files.
- Keep `oh-my-openagent.json` and `validator.ts` aligned; both previously kept
  stale Q6/GlyphOS/`4010`/`4011` routing after `opencode.json` was corrected.
- `SSE read timed out` on a background task is a client stream timeout; use
  `chunkTimeout=2400000` for 40-minute chunks and `timeout=7200000` for 2-hour
  long runs.
- Keep lmml `chat_template = ""` and `jinja = true` for this profile so
  llama-server uses the GGUF embedded template.
- `forcing full prompt re-processing` means cache invalidation or insufficient
  reusable prefix; it is slower but not a context failure by itself.

---

## Historical Snapshot — 128k/Q6 Port 1200 Setup

Captured: `2026-06-02T14:39:55+10:00`

This snapshot records the working OpenCode integration on this machine. The
active runtime is the lmml TUI-managed `llama-server`, not the detached
`lmml runtime start opencode --detach` profile flow.

Update: the original live snapshot used a `65535` context argument. A later
target used `98304`, and the current live server has been raised again to
`131072`. The `65535` evidence below remains as historical proof of the first
verified live run.

## Frozen Working Route

```text
OpenCode -> http://127.0.0.1:1200/v1 -> lmml TUI llama-server
```

Do not change this machine back to `4010/4011` while the TUI Server tab reports:

```text
Ready { url: "http://127.0.0.1:1200" }
```

Use `4010/4011` only when detached runtime profiles are actually started and
verified separately.

## Active Model

```text
Qwen3.5-4B-Q6_K.gguf
```

Model path:

```text
/home/angelo/.local/share/lmml/models/Qwen3.5-4B-Q6_K.gguf
```

Server binary:

```text
/home/angelo/.local/share/lmml/llama.cpp/build/bin/llama-server
```

## Runtime Settings

OpenCode does not own the llama-server context window. OpenCode points at the
OpenAI-compatible endpoint and owns harness settings such as compaction. The
active server context, GPU layers, batch sizes, and threads are set on the
lmml-managed `llama-server` process.

Live process command:

```text
/home/angelo/.local/share/lmml/llama.cpp/build/bin/llama-server --model /home/angelo/.local/share/lmml/models/Qwen3.5-4B-Q6_K.gguf --host 127.0.0.1 --port 1200 --ctx-size 65535 -ngl -1 --batch-size 512 --ubatch-size 512 --threads 8
```

Effective llama-server settings:

```text
host: 127.0.0.1
port: 1200
ctx_size arg: 65535
effective slot n_ctx: 65536
gpu_layers: -1
batch_size: 512
ubatch_size: 512
threads: 8
```

Current live llama-server target settings:

```text
host: 127.0.0.1
port: 1200
ctx_size arg: 131072
gpu_layers: -1
batch_size: 512
ubatch_size: 512
threads: 8
```

OpenCode-side harness settings:

```json
{
  "compaction": {
    "auto": true,
    "prune": true,
    "reserved": 32768
  }
}
```

Compaction target:

```text
server context: 131072 tokens
OpenCode reserved: 32768 tokens
OpenCode model output limit: 18000 tokens
usable input before compaction: 98304 tokens
practical single-agent input target: 80000-90000 tokens
hard reject/compress threshold: 96000-100000 tokens
llama-server parallel slots: 1
slot save path: /home/angelo/.local/share/lmml/llama-slots
```

This is the intended deep single-agent setting for this 11 GB GTX 1080 Ti
workstation. Keep `compaction.reserved = 32768` with an OpenCode model output
limit of `18000` while the server context is `131072`. The practical live prompt
target is `80000-90000` tokens, and requests near `96000-100000` tokens should be
compressed or rejected before they hit llama.cpp. The earlier `reserved = 24000`
and `reserved = 65536` experiments are superseded for this mode.

The slot count is also part of the proven setup. With `--parallel -1` auto,
llama.cpp selected `n_parallel = 4`. At 128k context on this 11GB GTX 1080 Ti,
concurrent OpenCode background tasks hit:

```text
failed to find free space in the KV cache
failed to prepare attention ubatches
Context size has been exceeded
```

The fix is to run the 1200 server as a single-slot long-context server:

```toml
[server]
extra_args = [
    "--parallel",
    "1",
    "--slot-save-path",
    "/home/angelo/.local/share/lmml/llama-slots",
    "--cache-reuse",
    "256",
]
```

This preserves the 128k live slot while avoiding four simultaneous long-context
KV allocations. Slot save/restore is persistence and prefix reuse, not live
context overflow.

If a full OpenCode restart with `compaction.reserved = 32768` still truncates
around `40000-45000` tokens, look for a second upstream cap before changing
llama.cpp again. On this machine, stale legacy caps were found in:

```text
/home/angelo/.config/llama-server/models.tsv
/home/angelo/.config/llama-server/defaults.env
```

Those files are not the active route when OpenCode talks directly to
`http://127.0.0.1:1200/v1`, but they matter if an LMM wrapper participates in
the request path.

Live single-slot cache behavior captured after the fix:

```text
slot prompt_clear: id 3 | clearing prompt with 58452 tokens
srv update_slots: all slots are idle
slot get_available: id 0 | selected slot by LRU
srv get_available: updating prompt cache
srv update: cache state: 2 prompts, 4325.506 MiB (limits: 8192.000 MiB)
prompt 0: 30833 tokens, checkpoints: 9
prompt 1: 75358 tokens, checkpoints: 9
slot launch_slot: id 0 | processing task
slot print_timing: prompt processing, n_tokens = 4096
slot print_timing: prompt processing, n_tokens = 4608
```

This shows the current server reusing prompt cache within the 8192 MiB cache
RAM limit and launching work on slot `0` instead of allocating four concurrent
long-context slots. The erased invalidated checkpoint warnings in this capture
are normal prompt-cache maintenance during a new request, not the previous
`failed to find free space in the KV cache` failure.

## Verified Commands

### Health

```sh
curl -fsS http://127.0.0.1:1200/health
```

Output:

```json
{"status":"ok"}
```

### OpenAI-Compatible Models Endpoint

```sh
curl -fsS http://127.0.0.1:1200/v1/models
```

Relevant output:

```json
{
  "object": "list",
  "data": [
    {
      "id": "Qwen3.5-4B-Q6_K.gguf",
      "owned_by": "llamacpp"
    }
  ]
}
```

### OpenCode Provider Model Listing

```sh
opencode models llamacpp
```

Output:

```text
llamacpp/Qwen3.5-4B-Q6_K.gguf
```

```sh
opencode models llamacpp_fast
```

Output:

```text
llamacpp_fast/Qwen3.5-4B-Q6_K.gguf
```

## OpenCode Config Evidence

Path:

```text
/home/angelo/.config/opencode/opencode.json
```

Required provider shape:

```json
{
  "provider": {
    "llamacpp": {
      "name": "lmml llama.cpp",
      "options": {
        "baseURL": "http://127.0.0.1:1200/v1"
      },
      "models": {
        "Qwen3.5-4B-Q6_K.gguf": {
          "name": "Qwen3.5-4B-Q6_K.gguf (GlyphOS full)",
          "limit": {
            "context": 131072,
            "input": 98304,
            "output": 18000
          }
        }
      }
    },
    "llamacpp_fast": {
      "name": "lmml llama.cpp fast",
      "options": {
        "baseURL": "http://127.0.0.1:1200/v1"
      },
      "models": {
        "Qwen3.5-4B-Q6_K.gguf": {
          "name": "Qwen3.5-4B-Q6_K.gguf (GlyphOS fast)",
          "limit": {
            "context": 131072,
            "input": 98304,
            "output": 18000
          }
        }
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

Backup created before the fix:

```text
/home/angelo/.config/opencode/opencode.json.bak.lmml-1200-fix
```

## lmml State Evidence

Path:

```text
/home/angelo/.config/lmml/state.toml
```

Required state shape:

```toml
[build]
binary = "/home/angelo/.local/share/lmml/llama.cpp/build/bin/llama-server"

[model]
last_used = "/home/angelo/.local/share/lmml/models/Qwen3.5-4B-Q6_K.gguf"

[server]
host = "127.0.0.1"
port = 1200
ctx_size = 131072
extra_args = [
    "--parallel",
    "1",
    "--slot-save-path",
    "/home/angelo/.local/share/lmml/llama-slots",
    "--cache-reuse",
    "256",
]

[runtime.opencode]
host = "127.0.0.1"
port = 1200
model = "/home/angelo/.local/share/lmml/models/Qwen3.5-4B-Q6_K.gguf"
ctx_size = 131072
extra_args = [
    "--parallel",
    "1",
    "--slot-save-path",
    "/home/angelo/.local/share/lmml/llama-slots",
    "--cache-reuse",
    "256",
]

[runtime.opencode-fast]
host = "127.0.0.1"
port = 1200
model = "/home/angelo/.local/share/lmml/models/Qwen3.5-4B-Q6_K.gguf"
ctx_size = 131072
extra_args = [
    "--parallel",
    "1",
    "--slot-save-path",
    "/home/angelo/.local/share/lmml/llama-slots",
    "--cache-reuse",
    "256",
]
```

Backup created before the fix:

```text
/home/angelo/.config/lmml/state.toml.bak.lmml-opencode-1200-fix
/home/angelo/.config/lmml/state.toml.bak.lmml-context-65535-fix
/home/angelo/.config/lmml/state.toml.bak.lmml-context-98304
/home/angelo/.config/opencode/opencode.json.bak.lmml-compaction-65536
```

## Human And Agent Rule

If OpenCode cannot see lmml but the TUI server is ready at `1200`, fix
OpenCode and lmml state to match this snapshot. Do not switch to `4010/4011`
unless detached runtime profiles are started, healthy, and intentionally replacing
the TUI server route.

## 256k Profile Test

Captured: `2026-06-03T11:35:51+10:00`

The Qwen3.5-4B-Q6_K model reports a `262144` token training context. A live
256k test profile was started manually on port `1200` with:

```text
--ctx-size 262144
--parallel 1
--batch-size 512
--ubatch-size 128
--gpu-layers -1
--cache-ram 4096
-ctk q8_0
-ctv q8_0
--slot-save-path /home/angelo/.local/share/lmml/llama-slots
--cont-batching
```

OpenCode was also updated for the test:

```text
context: 262144
input: 196608
output: 18000
compaction.reserved: 65536
```

Startup passed:

```text
CUDA0: NVIDIA GeForce GTX 1080 Ti (11157 MiB, 10260 MiB free)
new slot, n_ctx = 262144
prompt cache is enabled, size limit: 4096 MiB
server is listening on http://127.0.0.1:1200
```

Live verification:

```text
/health: {"status":"ok"}
/props: total_slots = 1, n_ctx = 262144
nvidia-smi after load: 8737 MiB used, 2420 MiB free, 11264 MiB total
```

A small `/v1/chat/completions` request decoded successfully:

```text
prompt eval: 21 tokens at 115.51 tok/s
decode: 16 tokens at 50.09 tok/s
stop processing: n_tokens = 36, truncated = 0
```

No OOM, pinned-memory, or context-exceeded error appeared in the startup/decode
log. One warning matters for the final profile:

```text
cache_reuse is not supported by this context, it will be disabled
```

Therefore the 256k profile should rely on the 4096 MiB prompt cache and context
checkpoints. Do not treat `--cache-reuse 256` as active for this setup.

The live server was then restarted without `--cache-reuse`. The cleaned process
remained healthy:

```text
process args: --ctx-size 262144 --parallel 1 --batch-size 512 --ubatch-size 128
              --cache-ram 4096 -ctk q8_0 -ctv q8_0 --cont-batching
/health: {"status":"ok"}
/props: total_slots = 1, n_ctx = 262144
nvidia-smi after clean load: 8556 MiB used, 2602 MiB free, 11264 MiB total
```

The cleaned process decoded a small request successfully:

```text
prompt eval: 21 tokens at 120.46 tok/s
decode: 32 tokens at 51.57 tok/s
```
