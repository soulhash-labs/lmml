# lmml Fleet Profile Reference
### Three-Machine Architecture — Fleet Profiles

This document is the canonical reference for the three-machine local LLM fleet.
Each machine has a distinct role. Do not run all machines as 256k single-agent
boxes — that wastes the fleet. The strongest architecture is one deep context
engine plus two fanout engines.

Validation status:

- `Validated`: tested on the named hardware with health, props, VRAM, and a
  decode request.
- `Proposed`: plausible from hardware/profile math, but not yet load-tested.

---

## Fleet Overview

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         Three-Lane Architecture                         │
│                                                                         │
│  Orion               Quadro M6000           RTX 5070 Ti               │
│  GTX 1080 Ti 11GB     24GB VRAM              16GB VRAM                 │
│                                                                         │
│  "Big Brain"          "Subagent Swarm"        "Tactical Fast Lane"     │
│  256k deep context    256k / 4 slots          128k / 4 slots           │
│  1 resident agent     medium-depth fanout     fast small agents        │
│  80k–196k working     32k per slot            16k per slot             │
└─────────────────────────────────────────────────────────────────────────┘
```

**Rule:** Never mix deep-context single-agent mode with parallel fanout on the
same server. Define separate profiles and do not attempt to serve both workloads
from a single `llama-server` instance.

---

## Machine 1 — Orion (GTX 1080 Ti, 11 GB VRAM, 47.8 GB RAM)

Status: `Validated`

### Role: deep single-agent resident context

This machine runs one heavyweight reasoning/code agent with a very large working
context. Subagents must be serialized, summarized, or capped — this profile
does not support parallel fanout.

### Confirmed hardware state at 256k launch

```
VRAM used:  8556 MiB
VRAM free:  2602 MiB
VRAM total: 11264 MiB
n_slots:    1
n_ctx:      262144
KV cache:   q8_0 / q8_0
```

Model capability confirmed: `n_ctx_seq (131072) < n_ctx_train (262144)` —
the model was trained at 256k; previous 128k cap was a launch config limit,
not a model limit.

### Profile: opencode-orion-deep (canonical, validated)

```toml
[profiles.opencode-orion-deep]
copied_from          = "preset-11gb-workstation"
port                 = 1200
model                = "~/.local/share/lmml/models/Qwen3.5-4B-Q8_0.gguf"
ctx_size             = 262144
compaction_reserved  = 65536
n_gpu_layers         = -1
batch_size           = 512
ubatch_size          = 128      # reduced from 512 for long-context stability
threads              = 8
flash_attn           = "auto"   # skips on Pascal sm_61; set "true" if verified
continuous_batch     = true
parallel_slots       = 1        # single-agent mode only
split_mode           = "auto"
kv_cache_type_k      = "q8_0"  # required: FP16 at 256k OOMs on 11 GB
kv_cache_type_v      = "q8_0"
cache_ram_mb         = 4096     # spills KV overflow to system RAM (47.8 GB free)
fit_mode             = "auto"
prompt_cache         = "disk"
prompt_cache_path    = "~/.local/share/lmml/cache/orion-deep.kvcache"
tuned_for_vram_mb    = 11264
opencode_timeout_s   = 7200
opencode_chunk_timeout_s = 2400
opencode_output_limit = 18000
```

### llama-server argv (emitted by lmml-compat)

```sh
llama-server \
  --model ~/.local/share/lmml/models/Qwen3.5-4B-Q8_0.gguf \
  --host 127.0.0.1 \
  --port 1200 \
  --ctx-size 262144 \
  --parallel 1 \
  --batch-size 512 \
  --ubatch-size 128 \
  --threads 8 \
  --gpu-layers -1 \
  --cache-ram 4096 \
  --cache-type-k q8_0 \
  --cache-type-v q8_0
```

Note: `--cache-reuse` is explicitly excluded. Confirmed unsupported in this
context. Prompt cache/checkpoints via `--slot-save-path` are the correct
persistence mechanism.

### Operational envelope

```
Green zone:    0 – 120k tokens    normal large task, comfortable
Yellow zone:   120k – 170k tokens deep repo task, monitor VRAM
Red zone:      170k – 196k tokens emergency max, expect slowdown
Hard cap:      > 196k tokens      compact / reject

Reserve:       65536 tokens       (compaction.reserved, OpenCode safety margin)
Absolute ctx:  262144 tokens
Working max:   196608 tokens      (262144 − 65536)
```

### Wrapper environment (OpenCode / LMM)

```sh
export LMM_MAX_CONTEXT_TOKENS=262144
export LMM_CONTEXT_SAFETY_MARGIN=65536
export LMM_AGENT_SOFT_CONTEXT_LIMIT=196608
export LMM_AGENT_HARD_CONTEXT_LIMIT=209715    # 80% of 262144; reject above this
export LMM_PARALLEL_SUBAGENTS=1               # serialize all subagents
```

### OpenCode route

```json
{
  "model": "llamacpp/Qwen3.5-4B-Q8_0.gguf",
  "small_model": "llamacpp_fast/Qwen3.5-4B-Q8_0.gguf",
  "provider": {
    "llamacpp": {
      "options": {
        "baseURL": "http://127.0.0.1:1200/v1",
        "timeout": 7200000,
        "chunkTimeout": 2400000
      }
    },
    "llamacpp_fast": {
      "options": {
        "baseURL": "http://127.0.0.1:1200/v1",
        "timeout": 7200000,
        "chunkTimeout": 2400000
      }
    }
  }
}
```

Keep `~/.config/opencode/oh-my-openagent.json` and
`~/.config/opencode/validator.ts` aligned with the same Q8 model and port
`1200`. Those files can preserve stale Q6/GlyphOS/`4010`/`4011` routing even
after `opencode.json` is correct.

### OpenCode compaction config

```json
{
  "compaction": {
    "reserved": 65536
  }
}
```

### Why 65536 is now correct at 256k

At 128k context: `131072 − 65536 = 65536` theoretical → ~43k practical (choking).
At 256k context: `262144 − 65536 = 196608` theoretical → ~174k practical (correct).

The same reserve value that was choking the agent at 128k is the right safety
margin at 256k. The fix was not reducing the reserve — it was doubling the
context window.

### Subagent policy on Orion

This profile does not support parallel subagents. OpenCode/LMM must be
configured to:
- Serialize subagent execution (one at a time, queued).
- Cap each subagent payload to 32k tokens maximum before dispatch.
- Summarize completed subagent results before adding to main context.

If parallel fanout is needed, use the Quadro M6000 (Machine 2).

### Validation test

Run the same OpenCode/Sisyphus workflow that previously truncated around
42k-46k. Expected: continue past that wall and compact/reject only near the
196k OpenCode input budget. Watch the logs for client-side SSE timeouts as a
separate issue; the current chunk timeout target is `2400s`.

---

## Machine 2 — Quadro M6000 (24 GB VRAM, 112 GB RAM)

Status: `Proposed`

### Role: subagent swarm / medium-depth fanout

This machine runs multiple concurrent subagents with enough per-slot context
for real work. The 24 GB VRAM gives the headroom that 11 GB cannot.

Important hardware note: Quadro M6000 is Maxwell-era. Keep `flash_attn = "auto"`
and expect it to be disabled unless a specific build/backend proves otherwise.

Qwen3.5 9B model facts from the model channel:

- 9B dense parameters, 32 layers.
- Hybrid Gated DeltaNet linear attention plus full softmax attention at a 3:1
  ratio.
- 262k native context, extendable to 1M with YaRN in runtimes that support it.
- Native multimodal text/image/video support, but vision/video requires the
  matching `mmproj` vision encoder file alongside the main GGUF.
- Multi-token prediction support exists, but keep MTP disabled until lmml has a
  stability/performance profile for it.
- 248k vocabulary and 201 languages.
- Maintain at least 128k context to preserve thinking capability.
- For production/high-throughput serving, use vLLM, SGLang, or KTransformers;
  lmml/llama.cpp remains the local LAN control plane profile here.

Official Qwen sampling presets:

```text
thinking/default:     temperature=0.6 top_p=0.95 top_k=20 min_p=0
non-thinking/fast:    temperature=0.7 top_p=0.8  top_k=20 min_p=0
```

### Profile: opencode-m6000-fanout (recommended, balanced)

```toml
[profiles.opencode-m6000-fanout]
copied_from          = "preset-24gb-workstation"
host                 = "0.0.0.0"
port                 = 1200
model                = "~/.local/share/lmml/models/Qwen3.5-9B-Q8_0.gguf"
mmproj               = "~/.local/share/lmml/models/Qwen3.5-9B-mmproj.gguf" # required for image/video; proposed field
ctx_size             = 262144
compaction_reserved  = 49152    # below per-slot context; 262144/4 = 65536
n_gpu_layers         = -1
batch_size           = 512
ubatch_size          = 128
threads              = 8
flash_attn           = "auto"
continuous_batch     = true
parallel_slots       = 4
split_mode           = "auto"
kv_cache_type_k      = "q8_0"
kv_cache_type_v      = "q8_0"
cache_ram_mb         = 4096
fit_mode             = "auto"
prompt_cache         = "disk"
prompt_cache_path    = "~/.local/share/lmml/cache/m6000-fanout.kvcache"
tuned_for_vram_mb    = 24576
temperature          = 0.6
top_p                = 0.95
top_k                = 20
min_p                = 0.0
mtp                  = "off"
```

### llama-server argv

```sh
llama-server \
  --model /path/to/Qwen3.5-9B-Q8_0.gguf \
  --host 127.0.0.1 \
  --port 1200 \
  --ctx-size 262144 \
  --parallel 4 \
  --batch-size 512 \
  --ubatch-size 128 \
  --threads 8 \
  --gpu-layers -1 \
  --cache-ram 4096 \
  --cache-type-k q8_0 \
  --cache-type-v q8_0
```

When `mmproj` support lands in lmml-compat, the same profile should add the
llama.cpp multimodal projector flag for the matching vision encoder. Until that
file is present and loaded, the Qwen 9B profile is text-only in practice.

### Slot math

```
262144 / 4 = 65536 tokens per slot (theoretical)
```

### Per-slot operational envelope

```
Green zone:    0 – 32k tokens per slot    normal subagent work
Yellow zone:   32k – 48k tokens           deep subagent, monitor
Red zone:      48k – 56k tokens           near limit, compact soon
Hard cap:      ~56k tokens per slot       reject above this

Recommended subagent payload cap: 32768 tokens
```

### Wrapper environment

```sh
export LMM_MAX_CONTEXT_TOKENS=262144
export LMM_CONTEXT_SAFETY_MARGIN=49152
export LMM_SUBAGENT_SOFT_CONTEXT_LIMIT=32768
export LMM_SUBAGENT_HARD_CONTEXT_LIMIT=49152
export LMM_PARALLEL_SUBAGENTS=4
```

### Profile: opencode-m6000-aggressive (fanout × 6, if VRAM stable)

Use only after confirming VRAM stability under `parallel 4` load.
Suitable for many small scoped agents: lint, test, docs, diff-review, search.

```sh
llama-server \
  --model /path/to/Qwen3.5-9B-Q8_0.gguf \
  --host 127.0.0.1 \
  --port 1200 \
  --ctx-size 262144 \
  --parallel 6 \
  --batch-size 512 \
  --ubatch-size 96 \
  --threads 8 \
  --gpu-layers -1 \
  --cache-ram 4096 \
  --cache-type-k q8_0 \
  --cache-type-v q8_0
```

Slot math: `262144 / 6 ≈ 43690` tokens per slot.

Per-slot operational envelope:
```
Green zone:    0 – 20k tokens
Yellow zone:   20k – 30k tokens
Red zone:      30k – 36k tokens
Hard cap:      ~36k tokens
```

Wrapper caps for aggressive mode:
```sh
export LMM_SUBAGENT_SOFT_CONTEXT_LIMIT=20480
export LMM_SUBAGENT_HARD_CONTEXT_LIMIT=32768
export LMM_PARALLEL_SUBAGENTS=6
```

---

## Machine 3 — RTX 5070 Ti (16 GB VRAM, 64 GB RAM)

Status: `Proposed`

### Role: fast tactical subagent execution

Newer GPU architecture, faster decode, but 16 GB VRAM is less forgiving for
high-parallel long-context KV. This machine handles fast, scoped subagents.

Before using this profile, `lmml doctor` must verify the actual GPU compute
capability and CUDA toolkit compatibility. Do not assume support from the card
name alone.

### Profile: opencode-5070ti-fanout (recommended, stable)

```toml
[profiles.opencode-5070ti-fanout]
copied_from          = "preset-16gb-workstation"
host                 = "0.0.0.0"
port                 = 1200
model                = "~/.local/share/lmml/models/Qwen3.5-4B-Q6_K.gguf"
ctx_size             = 131072
compaction_reserved  = 16384    # 131072/4 = 32768 per slot; reserve 16384
n_gpu_layers         = -1
batch_size           = 512
ubatch_size          = 128
threads              = 8
flash_attn           = "auto"   # Blackwell/Ada: FA supported; auto will enable
continuous_batch     = true
parallel_slots       = 4
split_mode           = "auto"
kv_cache_type_k      = "q8_0"
kv_cache_type_v      = "q8_0"
cache_ram_mb         = 2048
fit_mode             = "auto"
prompt_cache         = "memory"  # faster; smaller sessions don't need disk persist
tuned_for_vram_mb    = 16384
```

### llama-server argv

```sh
llama-server \
  --model /path/to/Qwen3.5-4B-Q6_K.gguf \
  --host 127.0.0.1 \
  --port 1200 \
  --ctx-size 131072 \
  --parallel 4 \
  --batch-size 512 \
  --ubatch-size 128 \
  --threads 8 \
  --gpu-layers -1 \
  --cache-ram 2048 \
  --cache-type-k q8_0 \
  --cache-type-v q8_0
```

### Slot math

```
131072 / 4 = 32768 tokens per slot (theoretical)
```

### Per-slot operational envelope

```
Green zone:    0 – 16k tokens    fast scoped agent, comfortable
Yellow zone:   16k – 24k tokens  moderate subagent, watch VRAM
Red zone:      24k – 28k tokens  near limit
Hard cap:      ~28k tokens

Recommended subagent payload cap: 16384 tokens
```

### Wrapper environment

```sh
export LMM_MAX_CONTEXT_TOKENS=131072
export LMM_CONTEXT_SAFETY_MARGIN=32768
export LMM_AGENT_SOFT_CONTEXT_LIMIT=98304
export LMM_SUBAGENT_SOFT_CONTEXT_LIMIT=16384
export LMM_SUBAGENT_HARD_CONTEXT_LIMIT=24576
export LMM_PARALLEL_SUBAGENTS=4
```

### Profile: opencode-5070ti-dual (two large agents, if needed)

When subagents need more depth but fanout can be reduced:

```sh
llama-server \
  --model /path/to/Qwen3.5-4B-Q6_K.gguf \
  --host 127.0.0.1 \
  --port 1200 \
  --ctx-size 262144 \
  --parallel 2 \
  --batch-size 512 \
  --ubatch-size 128 \
  --threads 8 \
  --gpu-layers -1 \
  --cache-ram 2048 \
  --cache-type-k q8_0 \
  --cache-type-v q8_0
```

Slot math: `262144 / 2 = 131072` tokens per slot. This is two large agents, not
a fanout swarm. Per-slot envelope:
```
Green zone:    0 – 64k tokens
Yellow zone:   64k – 96k tokens
Red zone:      96k – 112k tokens
Hard cap:      ~112k tokens
```

Wrapper caps:
```sh
export LMM_MAX_CONTEXT_TOKENS=262144
export LMM_CONTEXT_SAFETY_MARGIN=65536
export LMM_SUBAGENT_SOFT_CONTEXT_LIMIT=65536
export LMM_SUBAGENT_HARD_CONTEXT_LIMIT=98304
export LMM_PARALLEL_SUBAGENTS=2
```

---

## Fleet Summary Table

| Machine | Status | Model | Profile | ctx_size | parallel | Per-slot ctx | VRAM | KV | Role |
|---|---|---|---|---:|---:|---:|---|---|---|
| Orion (1080 Ti 11GB) | Validated | Qwen3.5 4B Q6_K | opencode-orion-deep | 262144 | 1 | 262144 | 8.5/11 GB | q8_0 | Deep resident agent |
| Quadro M6000 24GB | Proposed | **Qwen3.5 9B Q8_0** | opencode-m6000-fanout | 262144 | 4 | 65536 | needs test | q8_0 | Subagent swarm |
| Quadro M6000 24GB | Proposed | **Qwen3.5 9B Q8_0** | opencode-m6000-aggressive | 262144 | 6 | 43690 | needs test | q8_0 | Many small agents |
| RTX 5070 Ti 16GB | Proposed | Qwen3.5 4B Q6_K | opencode-5070ti-fanout | 131072 | 4 | 32768 | needs test | q8_0 | Fast tactical agents |
| RTX 5070 Ti 16GB | Proposed | Qwen3.5 4B Q6_K | opencode-5070ti-dual | 262144 | 2 | 131072 | needs test | q8_0 | Two large agents |

---

## Subagent payload caps by machine

These are the hard limits to configure in the OpenCode/LMM wrapper for each
machine's primary profile. Dispatching payloads above the soft cap risks queue
pile-up or KV overflow under concurrent load.

| Machine | Profile | Soft cap | Hard cap | Max parallel |
|---|---|---|---|---|
| Orion | deep | 32768 (per dispatch) | serialized | 1 |
| Quadro M6000 | fanout | 32768 | 49152 | 4 |
| Quadro M6000 | aggressive | 20480 | 32768 | 6 |
| RTX 5070 Ti | fanout | 16384 | 24576 | 4 |
| RTX 5070 Ti | dual | 65536 | 98304 | 2 |

---

## KV cache VRAM estimates (q8_0, all machines)

```
Validated Orion data:
  Qwen3.5 4B Q6_K, ctx 262144, parallel 1, q8_0 KV:
  8556 MiB used / 2602 MiB free / 11264 MiB total
```

Do not treat fanout KV as `ctx_size * parallel_slots`. In llama-server:

```
total context budget = ctx_size
per-slot context     = ctx_size / parallel_slots
runtime pressure increases with parallelism
```

For proposed profiles, estimate:

```
vram_required = model_weights + kv_cache(ctx_size, kv_type) + runtime_overhead

Model weights by machine:
  Orion / 5070 Ti:  Qwen3.5 4B Q6_K  ~3.5 GB
  Quadro M6000:      Qwen3.5 9B Q8_0  ~9.5 GB
CUDA overhead: ~0.5 GB
```

---

## lmml doctor checks to add for fleet profiles

When `parallel_slots > 1`, doctor should validate:

```
ctx_size / parallel_slots ≥ subagent_hard_context_limit + 8192
compaction_reserved < ctx_size / parallel_slots

If violated:
✗ Profile 'opencode-m6000-fanout': per-slot context (65536) is below
  subagent_hard_context_limit (49152) + safety buffer (8192).
  Reduce parallel_slots or increase ctx_size.
```

When `parallel_slots > 1` and `cache_ram_mb < parallel_slots * 512`:
```
⚠  Profile 'opencode-m6000-fanout': cache_ram_mb (4096) may be insufficient
   for 4 concurrent slots under heavy load.
   Recommended minimum: 4096 MB (current: 4096 MB — OK).
```

When `parallel_slots > 1` on hardware with < 16 GB VRAM:
```
⚠  Profile uses parallel_slots = 4 on 11 GB VRAM.
   KV cache at this fanout may exceed VRAM.
   Recommended: parallel_slots ≤ 2 on hardware < 16 GB VRAM,
   or use kv_cache_type_k/v = "q4_1" to reduce KV footprint.
```

---

## Deployment rule summary

```
1. Orion runs one profile at a time: deep mode OR fanout mode. Never both.
2. Quadro M6000 is the primary subagent server. Start with parallel 4.
   Only move to parallel 6 after confirming VRAM stability under load.
3. RTX 5070 Ti handles fast tactical work. 128k / parallel 4 is the daily driver.
   Use 256k / parallel 2 only when subagents need deeper context.
4. Do not run compaction.reserved above ctx_size / parallel_slots.
   That wastes the entire working window of every slot.
5. Always restart OpenCode/LMM (not just llama-server) after changing
   compaction.reserved. The value is read at session init.
6. Watch for pinned memory failure in logs:
   "ggml_cuda_host_alloc: failed to allocate N MiB of pinned memory"
   If seen, reduce cache_ram_mb or the system fell back to slower unpinned RAM.
7. For LAN-serving profiles, use `host = "0.0.0.0"` or a concrete LAN IP,
   open the firewall for the selected port, and configure auth/API keys before
   exposing beyond localhost.
8. Treat proposed fleet profiles as hypotheses until each machine has health,
   props, VRAM, and decode evidence.
```
