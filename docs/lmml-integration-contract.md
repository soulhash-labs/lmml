# Specification: llama-server Integration Contract (lmml-compat)
### Version 2 — Multi-tier hardware, multi-profile, full flexibility

This document defines the strict runtime, configuration, and validation contract
between the lmml manager, the underlying llama-server process, and all supported
coding harnesses (OpenCode, Claude Code, Continue, and any OpenAI-compatible client).

It is the authoritative source of truth for `lmml-compat` flag generation,
`lmml doctor` validation rules, profile schema, and VRAM budget enforcement.

---

## 1. System topology

```
┌──────────────────────────────────────────────────────────────────┐
│                      User / Developer Space                      │
│  ~/.config/opencode/opencode.json                                │
│  ~/.config/lmml/state.toml                                       │
└────────────────────────────┬─────────────────────────────────────┘
                             │ lmml runtime configure opencode
                             ▼
┌──────────────────────────────────────────────────────────────────┐
│                        lmml CLI / TUI                            │
│  Process supervisor · profile manager · state.toml writer        │
│  lmml-detect · lmml-compat · lmml-state · lmml-server           │
└──────┬───────────────────────────────────────┬───────────────────┘
       │ spawns detached process group          │ polls every 5s
       ▼                                        ▼
┌──────────────────────┐  ┌──────────────────────┐  ┌────────────────┐
│ Profile A            │  │ Profile B            │  │ GET /v1/health │
│ llama-server :4010   │  │ llama-server :4011   │  │ per profile    │
│ full coding model    │  │ fast/small model     │  └────────────────┘
│ log: per-profile     │  │ log: per-profile     │
└──────────┬───────────┘  └──────────┬───────────┘
           │                         │
           ▼                         ▼
┌──────────────────────────────────────────────────────────────────┐
│                   NVIDIA CUDA Driver Layer                       │
│  Shared VRAM pool · unified allocation · lmml-detect probes      │
└──────────────────────────────────────────────────────────────────┘
```

**Ports:** 4010 (primary / full) and 4011 (secondary / fast). These supersede
any prior references to 8080/8081.

**Logs:** Each profile writes to its own log file:
- `~/.local/share/lmml/logs/profile-opencode.log`
- `~/.local/share/lmml/logs/profile-opencode-fast.log`

The global lmml trace log remains at `~/.local/share/lmml/lmml.log`.

---

## 2. Profile schema

A profile is the central configuration unit in lmml. Every flag lmml-compat
emits is derived from a profile. Users never write raw llama-server flags.

```toml
# Example: ~/.config/lmml/state.toml  [profiles] section
# This is an editable profile copied from preset-11gb-workstation.
# To create a profile: lmml profile copy <preset-name> <my-profile-name>

[profiles.opencode]
copied_from     = "preset-11gb-workstation"
port            = 4010
model           = "~/.local/share/lmml/models/mistral-7b-q4_k_m.gguf"
ctx_size        = 32768
# compaction.reserved must match opencode.json compaction.reserved.
# 11 GB workstation: 16384. 24 GB+ workstation: 65536 (validated).
compaction_reserved = 16384
n_gpu_layers    = -1          # -1 = auto from VramFit
batch_size      = 512
ubatch_size     = 512
threads         = 8
flash_attn      = "auto"      # "auto" | "true" | "false"
                              # auto: skips on Pascal/Maxwell, emits on Volta+
                              # true: force on (user verified hardware supports it)
                              # false: force off (use if FA causes crashes)
continuous_batch = true
parallel_slots  = 1           # -np; keep at 1 for single-GPU
split_mode      = "auto"      # "auto" = omit on single-GPU, "none" on multi-GPU
                              # unless explicit tensor parallelism is requested

# KV cache quantization — reduces context VRAM footprint at minor quality cost
# "f16"  = default FP16 (highest quality, most VRAM)
# "q8_0" = 8-bit KV cache (~50% VRAM reduction, negligible quality loss)
# "q4_1" = 4-bit KV cache (~70% VRAM reduction, small quality loss)
# Recommended for ctx_size > 65536 on memory-constrained hardware.
kv_cache_type_k = "f16"       # -ctk
kv_cache_type_v = "f16"       # -ctv

# Smart VRAM fit mode
# "off"  = use n_gpu_layers exactly as set
# "auto" = let llama-server compute max layers that fit given ctx_size + KV budget
# Equivalent to --fit on in llama-server. Recommended when ctx_size > 65536
# or when running close to VRAM ceiling.
fit_mode        = "off"       # "off" | "auto"

# Speculative decoding — pairs a small draft model with the main model
# to accelerate token generation (2-3x speedup on long contexts).
# Leave empty to disable.
speculative_draft_model = ""  # path to small draft .gguf, e.g. qwen2.5-coder-0.5b-q8.gguf
speculative_draft_tokens = 5  # --draft N; tokens the draft model guesses ahead

# Prompt cache (slot save/restore) — critical for long multi-turn sessions.
# Caches the processed KV state so subsequent turns skip full re-evaluation.
# "none"   = disabled
# "memory" = in-process memory cache (fast, lost on server restart)
# "disk"   = persist to cache_path (survives restarts, slower first access)
prompt_cache    = "memory"    # "none" | "memory" | "disk"
prompt_cache_path = ""        # required when prompt_cache = "disk"
                              # e.g. ~/.local/share/lmml/cache/opencode.kvcache

api_key         = ""
extra_args      = []
tuned_for_vram_mb = 11264     # 11 GB GTX 1080 Ti

[profiles.opencode-fast]
copied_from     = "preset-11gb-workstation"
port            = 4011
model           = "~/.local/share/lmml/models/phi-3-mini-q4.gguf"
ctx_size        = 8192
compaction_reserved = 16384
n_gpu_layers    = -1
batch_size      = 256
ubatch_size     = 256
threads         = 8
flash_attn      = "auto"      # ctx 8192; auto will skip FA at this size
continuous_batch = true
parallel_slots  = 1
split_mode      = "auto"
kv_cache_type_k = "f16"
kv_cache_type_v = "f16"
fit_mode        = "off"
speculative_draft_model  = ""
speculative_draft_tokens = 5
prompt_cache    = "memory"
prompt_cache_path = ""
api_key         = ""
extra_args      = []
tuned_for_vram_mb = 11264

# Example: 24 GB workstation profile (validated)
# lmml profile copy preset-24gb-workstation opencode-quadro
[profiles.opencode-quadro]
copied_from          = "preset-24gb-workstation"
port                 = 4010
model                = "~/.local/share/lmml/models/Qwen3.5-9B-Q8_0.gguf"
mmproj               = "~/.local/share/lmml/models/Qwen3.5-9B-mmproj.gguf" # required for image/video; proposed field
ctx_size             = 262144   # 256k — validated on Quadro M6000
compaction_reserved  = 65536    # validated: 9h coding session, no exhaustion
n_gpu_layers         = -1
batch_size           = 512
ubatch_size          = 512
threads              = 16
flash_attn           = true
continuous_batch     = true
parallel_slots       = 1
split_mode           = "auto"
# KV cache quantization: q8_0 halves context VRAM at negligible quality cost.
# Validated: allows full 256k context on 24 GB without spilling to RAM.
kv_cache_type_k      = "q8_0"
kv_cache_type_v      = "q8_0"
temperature          = 0.6
top_p                = 0.95
top_k                = 20
min_p                = 0.0
mtp                  = "off"
# fit_mode auto: llama-server computes optimal ngl given ctx + KV budget.
# Recommended for 256k context; prevents manual ngl guesswork.
fit_mode             = "auto"
# Speculative decoding: Qwen2.5-Coder 0.5B as draft model gives ~2x generation
# speedup on long-context turns without any accuracy loss from the 9B model.
speculative_draft_model  = "~/.local/share/lmml/models/qwen2.5-coder-0.5b-q8.gguf"
speculative_draft_tokens = 5
# Disk prompt cache: survives server restarts, critical for 9h+ sessions.
# TTFT on cached turns drops from minutes to sub-second.
prompt_cache      = "disk"
prompt_cache_path = "~/.local/share/lmml/cache/opencode-quadro.kvcache"
api_key           = ""
extra_args        = []
tuned_for_vram_mb = 24576
# Example: RTX 5070 Ti fast tactical profile
# lmml profile copy preset-16gb-fanout opencode-5070ti
[profiles.opencode-5070ti]
copied_from          = "preset-16gb-fanout"
port                 = 4011
model                = "~/.local/share/lmml/models/Qwen3.5-4B-Q6_K.gguf"
ctx_size             = 131072
compaction_reserved  = 16384   # dynamic formula: 131072 → 16384
n_gpu_layers         = -1
batch_size           = 512
ubatch_size          = 128
threads              = 8
flash_attn           = "auto"  # 5070 Ti is Ada/Blackwell (sm_89+); auto will enable
continuous_batch     = true
parallel_slots       = 4
split_mode           = "auto"
kv_cache_type_k      = "q8_0"
kv_cache_type_v      = "q8_0"
cache_ram_mb         = 2048
fit_mode             = "auto"
prompt_cache         = "memory"  # fast agents; disk persist not needed
prompt_cache_path    = ""
api_key              = ""
extra_args           = []
tuned_for_vram_mb    = 16384
# VRAM estimate: weights ~3.5 GB + KV ~4.0 GB + overhead ~0.5 GB = ~8 GB / 16 GB
# 8 GB margin gives stable headroom for all 4 slots at hard cap (28k tokens each)
```

lmml ships with read-only preset profiles. Users **copy a preset to create an
editable profile** — presets themselves are never modified by the tool. Copying
is the only way to create a new profile; there is no blank-slate "new profile"
command. This ensures every profile starts from a validated, hardware-appropriate
baseline.

**How to copy a preset:**
```sh
lmml profile copy preset-24gb-workstation my-qwen-long-run
lmml profile edit my-qwen-long-run          # opens in TUI settings pane
```

**Copy rules:**
- `lmml profile copy <source> <name>` accepts only `preset-*` names as source
  by default. This is the safe path — you always start from a known-good baseline.
- To copy from an existing user profile: `lmml profile copy --from-profile <source> <name>`.
  This path runs `lmml doctor --strict` on the source profile before copying.
  If doctor exits 1, the copy is refused with:
  ```
  ✗ Source profile 'my-profile' failed validation. Fix it before copying.
    Run `lmml doctor` to see what needs fixing.
  ```
- Copied profiles record `copied_from` for reference only; it has no runtime effect.

**Full preset table:**

| Preset | Target hardware | VRAM | ctx_size | parallel | compaction.reserved | Suggested model | Use case |
|---|---|---|---|---|---|---|---|
| `preset-8gb-desktop` | Desktop 8 GB | 8192 MB | 16384 | 1 | 4096 | 7B Q4_K_M | Proofreading, short tasks |
| `preset-11gb-deep` | GTX 1080 Ti | 11264 MB | 262144 | 1 | 65536 | 4B Q6_K | Deep single-agent ✓ validated |
| `preset-12gb-workstation` | RTX 3060 / 4060 | 12288 MB | 32768 | 1 | 8192 | 7B Q4_K_M | Daily coding, moderate runs |
| `preset-16gb-fanout` | RTX 5070 Ti / 4080 | 16384 MB | 131072 | 4 | 16384 | 4B Q6_K | Fast tactical subagents |
| `preset-16gb-dual` | RTX 5070 Ti / 4080 | 16384 MB | 262144 | 2 | 65536 | 4B Q6_K | Two large agents |
| `preset-24gb-fanout` | Quadro M6000 / RTX 3090 | 24576 MB | 262144 | 4 | 65536 | 9B Q8_0 | Subagent swarm ✓ validated |
| `preset-24gb-deep` | Quadro M6000 / RTX 3090 | 24576 MB | 262144 | 1 | 65536 | 9B Q8_0 | Deep 9h+ coding sessions ✓ validated |
| `preset-32gb-workstation` | RTX 4090 / dual 16 GB | 32768 MB | 262144 | 4 | 65536 | 34B Q4_K_M | Heavy coding, large context |
| `preset-48gb-workstation` | A6000 / dual 24 GB | 49152 MB | 262144 | 6 | 65536 | 34B Q4_K_M | Multiplex harnesses |
| `preset-80gb-server` | A100 / H100 | 81920 MB | 262144 | 8 | 65536 | 70B Q4_K_M | Large agent runs |
| `preset-auto` | Auto-detected | detected | computed | computed | computed | — | First-run default |

**`preset-auto`** is selected during first-run when the user does not choose a
preset manually. lmml-detect probes VRAM, computes ctx_size and
compaction_reserved using the dynamic formula (see §4), and creates an initial
profile. The user can copy and tune it at any time.

**Multi-GPU: roadmap.** Presets for multi-GPU systems (tensor parallelism across
2× or 4× GPUs) are deferred to a future release. For now, multi-GPU users should
use `preset-48gb-workstation` or `preset-80gb-server` with `split_mode = "auto"`
and treat the combined VRAM as a single pool. True tensor-parallel presets will
be added when multi-GPU support is hardened in `lmml-compat`.

**Notes on `compaction.reserved` values:**
- These are preset defaults, not locked values. After copying, users may tune
  `compaction_reserved` freely in the TUI Settings pane.
- The dynamic formula (§4) recomputes a recommended value whenever `ctx_size`
  changes; the TUI shows the recommendation and lets the user accept or override.
- The 24 GB preset value (65536) is empirically validated: Quadro M6000 +
  Qwen3.5 9B Q8_0 + ctx 262144 over a 9-hour coding session, no exhaustion.
- The 11 GB deep preset value (65536) is validated at 256k context: GTX 1080 Ti +
  Qwen3.5 4B Q6_K + ctx 262144, confirmed 8556/11264 MiB VRAM at launch.
  Note: at 131072 ctx the same 65536 reserved was too conservative (choked at
  ~43k practical). The fix is 256k context, not reducing the reserve.

**Fleet model assignment (validated):**

| Machine | VRAM | Model | Quant | Role |
|---|---|---|---|---|
| Orion (GTX 1080 Ti) | 11 GB | Qwen3.5 4B | Q6_K | Deep single-agent, 256k ctx |
| Quadro M6000 | 24 GB | **Qwen3.5 9B** | **Q8_0** | Subagent swarm, parallel 4 |
| RTX 5070 Ti | 16 GB | Qwen3.5 4B | Q6_K | Fast tactical agents, parallel 4 |

**Why 4B Q6_K on the 5070 Ti, not 9B Q8_0:**
The 5070 Ti's value in the fleet is decode speed and fanout, not model quality —
the M6000 already provides the smarter model. A 4B model on a Blackwell/Ada GPU
decodes measurably faster than a 9B on Maxwell. At parallel 4 with 131k context,
9B Q8_0 leaves only ~2 GB VRAM margin (tight). 4B Q6_K leaves ~8 GB (comfortable).
Keep the fleet lanes distinct. If quality matters more than throughput on the
fast lane, use `preset-16gb-dual` with 9B Q8_0 at parallel 2 instead.

Each preset stores its `tuned_for_vram_mb` value. `lmml doctor` warns when a
profile is used on hardware with less VRAM than it was tuned for.

---

## 3. VRAM budget enforcement

### Reference table (GTX 1080 Ti, 11 GB ceiling) — Qwen3.5 4B Q6_K validated at 256k

| Model | Quant | Weights | KV 8k + FA | KV 32k + FA | KV 256k q8 + cache-ram | Fits 11 GB? |
|---|---|---|---|---|---|---|
| 4B (Qwen3.5) | Q6_K | ~3.5 GB | ~0.3 GB | ~1.0 GB | ~8.0 GB + RAM spill | **Yes — validated ✓** |
| 7B / 8B (Mistral, Qwen2.5) | Q4_K_M | ~4.1 GB | ~0.5 GB | ~2.1 GB | ~8.0 GB + RAM spill | Yes (tight) |
| 7B / 8B | Q8_0 | ~7.7 GB | ~0.5 GB | ~2.1 GB | OOM at 256k | No at 256k |
| 13B / 14B | Q4_K_M | ~7.4 GB | ~0.5 GB | ~2.1 GB | OOM at 256k | No at 256k |

**Validated 256k config on 11 GB:** `--cache-ram 4096 --cache-type-k q8_0 --cache-type-v q8_0`
Boot log confirmed: 8556 MiB used / 2602 MiB free at 256k launch. KV overflow spills
to system RAM (47.8 GB available). `n_ctx_seq < n_ctx_train (262144)` confirms model
supports 256k natively.

### Reference table (RTX 5070 Ti, 16 GB ceiling) — 4B Q6_K recommended

| Model | Quant | Weights | KV 131k/4 slots q8 | KV 262k/2 slots q8 | Fits 16 GB? |
|---|---|---|---|---|---|
| 4B (Qwen3.5) | Q6_K | ~3.5 GB | ~4.0 GB total | ~4.0 GB total | **Yes — 8 GB margin** |
| 9B (Qwen2.5) | Q8_0 | ~9.5 GB | ~4.0 GB total | ~2.0 GB total | Yes — 2 GB margin (tight at parallel 4) |
| 13B / 14B | Q4_K_M | ~7.4 GB | ~4.0 GB total | ~4.0 GB total | Yes (tight) |

**Recommended for 5070 Ti:** 4B Q6_K at `parallel 4 / ctx 131072`. 8 GB VRAM margin.
Fast decode on newer GPU architecture. Reserve 9B Q8_0 for `parallel 2 / ctx 262144`
only if subagent quality matters more than throughput.

### Reference table (Quadro M6000, 24 GB ceiling) — validated

| Model | Quant | Weights | KV 64k + FA | KV 128k + FA | KV 256k + FA | Fits 24 GB? |
|---|---|---|---|---|---|---|
| 9B (Qwen3.5) | Q8_0 | ~9.5 GB | ~2.0 GB | ~4.0 GB | ~8.0 GB | **Yes — validated ✓** |
| 13B / 14B | Q4_K_M | ~7.4 GB | ~2.0 GB | ~4.0 GB | ~8.0 GB | Yes |
| 13B / 14B | Q8_0 | ~13.5 GB | ~2.0 GB | ~4.0 GB | ~8.0 GB | No at 256k |
| 34B | Q4_K_M | ~18.5 GB | ~2.0 GB | ~4.0 GB | — | No at 128k+ |

**Validated configuration:** Qwen3.5 9B Q8_0 + ctx 262144 + compaction.reserved 65536
on Quadro M6000 (24 GB), 9-hour agentic coding session, no OOM, no context exhaustion.

Qwen3.5 9B model-channel requirements:

- Maintain at least 128k context to preserve thinking capability.
- Thinking/default sampling: `temperature=0.6`, `top_p=0.95`, `top_k=20`,
  `min_p=0`.
- Non-thinking/fast sampling: `temperature=0.7`, `top_p=0.8`, `top_k=20`,
  `min_p=0`.
- Vision/video support requires the matching `mmproj` vision encoder alongside
  the main GGUF and loaded by the runtime.
- MTP is supported by the model, but lmml presets should keep it off until a
  stability/performance profile is validated.

### Generalised VRAM budget formula

```
vram_required = model_weights_gb + kv_cache_gb + overhead_gb

kv_cache_gb   = (ctx_size * n_layers * d_head * 2 * dtype_bytes) / 1e9
                [without flash attention]

kv_cache_fa   = kv_cache_gb * 0.25   [flash attention reduces by ~75%]

overhead_gb   = 0.5   [cuda context, activation buffers]

safety_margin = 0.5   [never fill VRAM to 100%]

budget_ok     = vram_required + safety_margin <= detected_vram_gb
```

lmml-detect provides `detected_vram_gb` from `nvidia-smi`. lmml doctor runs
this calculation for every profile during preflight. If `budget_ok` is false,
doctor exits 1 with:

```
✗ VRAM budget exceeded for profile 'opencode'
  Model weights:  4.1 GB
  KV cache (32k): 2.1 GB  [flash attention enabled]
  Overhead:       0.5 GB
  Total:          6.7 GB  →  fits
  ─────────────────────────────────────────
  At ctx_size=65536:
  Total:         8.6 GB  →  fits (0.4 GB margin — tight)
  ─────────────────────────────────────────
  At ctx_size=65536 with Q8_0:
  Total:        12.2 GB  →  EXCEEDS 11 GB VRAM
  Reduce ctx_size or use a lower quantisation.
```

---

## 4. Context size validation (harness compaction guard)

### The inequality

```
c ≥ R + B

c  = profile ctx_size
R  = harness compaction.reserved  (profile-dependent — see table below)
B  = minimum generation buffer    (hardcoded: 4096)
```

### Validated compaction.reserved defaults by hardware tier

Preset defaults are a starting point. The live recommended value is always
computed dynamically from `ctx_size`, not from VRAM tier. VRAM tier determines
the maximum safe `ctx_size`; `ctx_size` determines `compaction_reserved`.

**Dynamic formula (used by lmml-compat and TUI recommendation engine):**

```
compaction_reserved =
    if ctx_size <= 16384:  max(ctx_size / 4, 4096)      # floor: 4096
    if ctx_size <= 65536:  16384
    if ctx_size <= 131072: 32768
    else:                  65536                         # ceiling: 65536
```

Rationale: OpenCode's compaction index scales with active context depth, not
raw VRAM. A user who copies a 24 GB preset but sets `ctx_size = 8192` for
raw speed should use `compaction_reserved = 4096`, not 65536 — reserving 65536
on an 8k context window wastes 87% of the usable window.

The formula is applied:
- When `preset-auto` is first created (ctx_size computed from VRAM probe).
- When the user changes `ctx_size` in the TUI Settings pane — the TUI shows
  the recommended new value and asks: "Update compaction_reserved to N? [y/N]"
- When `lmml doctor` validates a profile — mismatch between stored value and
  formula output is a soft warning, not a hard error (user may have a reason).

**Empirically validated reference points:**

| ctx_size | Formula result | Validated on |
|---|---|---|
| 8192 | 4096 | — |
| 16384 | 4096 | — |
| 32768 | 16384 | GTX 1080 Ti, 7B Q4_K_M |
| 65536 | 16384 | — |
| 131072 | 32768 | — |
| 262144 | **65536** | **Quadro M6000, Qwen2.5 9B Q8, 9h session ✓** |

### Enforcement paths

**`lmml doctor`** — inspection only, never mutates:
- Reads `compaction.reserved` from the profile (not hardcoded to 32768).
- If `c - R < 4096`, report and exit 1.
- Also flags if the profile's `compaction.reserved` differs from the value
  written in `~/.config/opencode/opencode.json` (config drift).
- Error string:
  `"Error: ctx_size (c) minus compaction.reserved (R) = N tokens — below the 4,096 minimum generation buffer. The model will hit context exhaustion immediately."`

**`lmml runtime start <profile>`** — pre-flight guard:
- Runs the same check before spawning the process.
- If violated, refuses to start and prints the same error to stderr.
- Does not mutate config; instructs the user to fix it and rerun.

### Minimum safe ctx_size values per harness and tier

| Harness | compaction.reserved | Minimum ctx_size | Recommended ctx_size (24 GB+) |
|---|---|---|---|
| OpenCode (8 GB desktop) | 8192 | 12288 | 16384 |
| OpenCode (11 GB workstation) | 16384 | 20480 | 32768 |
| OpenCode (24 GB+ workstation) | 65536 | 69632 | 262144 |
| Claude Code (proxy mode) | 0 | 4096 | 32768+ |
| Continue | 0 | 4096 | 32768+ |
| Custom / unknown | configurable | R + 4096 | — |

### Config drift detection

lmml maintains `compaction.reserved` in two places:
1. The profile in `state.toml` (what lmml knows).
2. The `compaction.reserved` key in `~/.config/opencode/opencode.json` (what
   OpenCode reads at runtime).

These must match. `lmml doctor` compares them and warns on mismatch:
```
⚠  compaction.reserved mismatch
   state.toml profile 'opencode':      65536
   opencode.json compaction.reserved:  32768
   Fix: lmml runtime configure opencode
```

---

## 5. lmml-compat: automated flag generation

Users never write raw llama-server argv. lmml-compat translates a profile
struct into the correct argv array for the detected binary version.

### Flag emission rules

| Flag | Condition | Value |
|---|---|---|
| `-m` | always | profile.model path |
| `-c` | always | profile.ctx_size |
| `--port` | always | profile.port |
| `--host` | always | profile.host |
| `-ngl` | always | profile.n_gpu_layers (-1 = auto from VramFit) |
| `-b` | always | profile.batch_size |
| `-ub` | always | profile.ubatch_size |
| `-t` | always | profile.threads |
| `-fa` | flash_attn resolved to `true` (see resolution rules below) | (flag only, no value) |
| `-cb` | always | (flag only) |
| `-np` | always | profile.parallel_slots (default 1) |
| `--split-mode none` | multi-GPU AND profile.split_mode = "none" | "none" |
| `--api-key` | profile.api_key non-empty AND binary supports it | profile.api_key |
| `--jinja` | profile.jinja = true AND binary supports it | (flag only) |
| `--chat-template` | profile.chat_template non-empty | profile.chat_template |
| extra_args | profile.extra_args non-empty | appended verbatim, last |

**`flash_attn` tri-state resolution:**

The `flash_attn` profile field is a string with three valid values:

| Value | Meaning |
|---|---|
| `"auto"` | lmml decides: probe binary capability + GPU arch + ctx_size |
| `"true"` | Always emit `-fa`; user asserts their hardware supports it |
| `"false"` | Never emit `-fa`; hard override regardless of ctx_size |

`"auto"` resolution logic (in order):
1. Check binary capability via `lmml-compat` (binary supports `-fa`?). If no → skip.
2. Check GPU compute capability:
   - sm_37 / sm_50 / sm_52 (Kepler, Maxwell): FA support is unreliable in most
     llama.cpp builds. Default to **skip** unless user sets `"true"` explicitly.
   - sm_60 / sm_61 (Pascal, e.g. GTX 1080 Ti): FA may work but is not
     guaranteed across all llama.cpp versions. Default to **skip**; warn user:
     `"⚠ Flash attention skipped on Pascal (sm_61) — set flash_attn='true' to force"`
   - sm_70+ (Volta and newer): FA is supported and stable. **Emit `-fa`** when
     `ctx_size > 8192`.
3. If GPU is unknown or lmml-detect failed: default to **skip**, soft warning.

All presets default to `flash_attn = "auto"`. Users on Pascal/Maxwell who have
verified FA works on their specific build may set `flash_attn = "true"` in
their copied profile. Users who hit FA-related crashes set `flash_attn = "false"`.

**`--split-mode` policy:**
- Single-GPU systems: flag omitted entirely. No noise.
- Multi-GPU, `split_mode = "auto"`: flag omitted (llama-server default is fine).
- Multi-GPU, `split_mode = "none"`: emits `--split-mode none`. Use only when
  you explicitly want to lock to one GPU and forfeit tensor parallelism.
- Multi-GPU, `split_mode = "row"` or `"layer"`: emits accordingly when the
  binary supports it.

### Example emitted argv (GTX 1080 Ti, opencode profile, ctx 32768)

```
llama-server \
  -m ~/.local/share/lmml/models/mistral-7b-q4_k_m.gguf \
  -c 32768 \
  --port 4010 \
  --host 127.0.0.1 \
  -ngl 99 \
  -b 512 \
  -ub 512 \
  -t 8 \
  -fa \
  -cb \
  -np 1
```

### Queuing behaviour note

`-np 1` restricts the server to one active generation slot. A second request
arriving while a generation is in progress queues on the server side. On a
7200s harness timeout this appears as a pause, not a failure. This is expected
and correct behaviour for single-GPU deployments. Users who need true
concurrency on multi-GPU systems should set `parallel_slots = 2` and accept
the KV cache split cost.

---

## 6. Process supervision and stale PID recovery

lmml is un-daemonised: it does not run a persistent background service. State
tracking is delegated entirely to `~/.config/lmml/state.toml`. llama-server
instances run as detached process groups and survive terminal session closure.

### State tracked per profile

```toml
[runtime.opencode]
status     = "Ready"       # Stopped | Starting | Ready | Unhealthy | Failed
pid        = 12345
port       = 4010
model      = "mistral-7b-q4_k_m.gguf"
started_at = "2026-06-01T14:23:00Z"
last_health = "2026-06-01T14:23:55Z"
last_health_ok = true
health_failures = 0
```

### Stale PID recovery protocol

Run on every: `lmml` startup, `lmml runtime status`, TUI entry.

```
1. Read pid from state.toml for each profile with status != "Stopped".
2. Query OS process table:
   - Linux:  /proc/<pid>/exe  →  verify path ends in "llama-server"
   - macOS:  sysctl KERN_PROC or proc_pidpath()
3. If process is missing OR exe name does not match "llama-server":
   a. Set status = "Stopped"
   b. Clear pid, port lock
   c. Write updated state.toml
   d. Log recovery event to lmml.log:
      "Stale PID <pid> for profile '<name>' cleared on startup"
4. If process is present and healthy, resume health polling.
```

### Health polling

- Interval: every 5 seconds per profile.
- Endpoint: `GET http://127.0.0.1:<port>/v1/health`
- Success: HTTP 200 with `{"status":"ok"}` or `{"status":"loading model"}`
- Failure threshold: 3 consecutive failures → status = "Unhealthy"
- On Unhealthy: TUI footer badge turns red, `lmml runtime status` reports it,
  no auto-restart (user must restart explicitly).
- Recovery: first successful health poll after Unhealthy → status = "Ready",
  `health_failures` reset to 0.

---

## 7. Multi-profile and multi-harness operation

### Simultaneous profile rules

- Each profile is an independent `ServerManager` instance with its own PID,
  port, log file, and health state.
- Failure or restart of one profile has zero effect on other profiles.
- VRAM is shared: lmml doctor warns when the combined VRAM budget of all
  running profiles exceeds detected GPU VRAM.
- Combined VRAM check formula:
  `sum(vram_required[p] for p in running_profiles) + overhead ≤ detected_vram`

### Port conflict prevention and liveness check

**Why ports are static (not auto-incrementing):**
Harness config files (`~/.config/opencode/opencode.json`) hardcode the base URL
including port. If lmml used dynamic ports, every port change would require
re-running `lmml runtime configure` and restarting OpenCode. Static ports are
intentional for harness config stability. The solution to port conflicts is
detection and clear error messages, not dynamic assignment.

**Pre-spawn liveness check (mandatory, runs before every `lmml runtime start`):**

```rust
// 1. Check if the port is free
match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
    Ok(_)  => { /* free — proceed */ }
    Err(_) => {
        // 2. Check if it's one of lmml's own processes (stale state)
        if state.profile_owns_port(port) {
            // Stale PID path — run recovery, then retry
        } else {
            // Something else owns the port
            return Err(ServerError::PortInUse { port });
        }
    }
}
```

Error shown to user:
```
✗ Profile 'opencode' cannot start — port 4010 is already in use.

  If this is a stale lmml process:
    lmml runtime stop opencode --force

  If another service owns port 4010:
    Change the port in Settings, then re-run:
    lmml runtime configure opencode
    (This will update ~/.config/opencode/opencode.json to the new port.)

  To see what is using port 4010:
    ss -tulpn | grep 4010
```

**Port registry:** `state.toml` maintains a `[port_registry]` table mapping
port → profile name. Before spawning, lmml checks both the OS (TcpListener
bind) and the registry. After a successful start, the port is locked in the
registry. On stop or stale PID recovery, the port is released.

### `lmml runtime start` as a gatekeeper

`lmml runtime start <profile>` runs a mandatory pre-flight sequence before
spawning any process. It cannot be bypassed. The sequence is:

```
1. Stale PID recovery (§6)
2. Port liveness check (§7)
3. VRAM budget check   (§3) — hard block if budget_ok = false
4. Context validation  (§4) — hard block if c - R < 4096
5. Flash attention resolution (§5) — sets resolved flag, emits warning if skipped
6. compaction_reserved drift check — hard block if profile value ≠ opencode.json value
   Error:
   ✗ Config drift detected: profile 'opencode' compaction_reserved (65536)
     differs from ~/.config/opencode/opencode.json compaction.reserved (32768).
     Fix: lmml runtime configure opencode
     (or: lmml runtime configure opencode --dry-run to preview)
7. Model file exists and is readable — hard block if missing
8. Binary exists and is executable — hard block if missing

Only if all 8 checks pass → spawn llama-server
```

**Hard blocks** (exit 1, process not spawned): VRAM exceeded, context
exhaustion, config drift, missing model, missing binary.

**Soft warnings** (process spawns with warning printed): flash attention
skipped on Pascal/Maxwell, VRAM margin < 1 GB, compaction_reserved differs
from dynamic formula recommendation.

This makes `lmml doctor` the inspection tool and `lmml runtime start` the
enforcement gate. A user cannot reach a broken 4-hour session state by
ignoring a `doctor` warning — the server simply will not start until the
drift is resolved.

When a user changes the model for a running profile:
1. lmml warns: "Profile 'opencode' is running. Changing the model requires a restart."
2. User confirms restart.
3. lmml stops the old server (SIGTERM → 3s → SIGKILL if needed).
4. lmml starts the new server on the same port.
5. lmml polls `/v1/health` until Ready or timeout (30s default).
6. State is updated only after the new server is confirmed healthy.
7. In-flight requests on the old server are not drained (cold swap).
   Document this: callers with in-flight requests will see a connection reset.

---

## 8. lmml doctor output specification

`lmml doctor` is read-only. It never patches any file. It exits 0 if all hard
checks pass; exits 1 if any hard check fails.

### Full output format

```
lmml doctor — system preflight check
──────────────────────────────────────────────────────────────────
System
  ✓  gcc 15.2.0
  ✓  cmake 4.2.3  (≥ 3.21 required)
  ✓  git 2.53.0   (≥ 2.28 required)
  ✓  CUDA 12.4  ·  GTX 1080 Ti  ·  sm_61  ·  11 GB VRAM
  ✓  sccache active
  ✓  disk: 44 GB available

VRAM budget
  ✓  profile 'opencode':       6.7 GB required  /  11 GB available  (4.3 GB free)
  ✓  profile 'opencode-fast':  2.1 GB required  /  11 GB available
  ⚠  combined running profiles: 8.8 GB  →  2.2 GB margin (tight)

Context validation
  ✓  profile 'opencode':       ctx 32768  ≥  R 16384 + B 4096 = 20480  →  OK
  ✓  profile 'opencode-quadro': ctx 262144 ≥  R 65536 + B 4096 = 69632  →  OK
  ⚠  profile 'opencode':       compaction.reserved mismatch
       state.toml:       16384
       opencode.json:    32768  ← stale from previous config
       Fix: lmml runtime configure opencode

OpenCode config
  ✓  ~/.config/opencode/opencode.json found
  ✓  provider.llamacpp present  →  baseURL http://127.0.0.1:4010/v1
  ✓  provider.llamacpp_fast present  →  baseURL http://127.0.0.1:4011/v1
  ⚠  provider.llamacpp.options.timeout = 3600000  (recommended: 7200000)
     Fix: lmml runtime configure opencode  (or --dry-run to preview)

lmml runtime status
  ●  opencode       Ready   pid 12345   http://127.0.0.1:4010/v1
  ●  opencode-fast  Ready   pid 12346   http://127.0.0.1:4011/v1

──────────────────────────────────────────────────────────────────
1 error, 2 warnings.
Run `lmml runtime configure opencode` to apply recommended fixes.
```

### Exit codes

| Exit code | Meaning |
|---|---|
| 0 | All hard checks pass (warnings are informational) |
| 1 | One or more hard checks failed |

Hard checks: missing prerequisites, context validation failure, VRAM budget
exceeded, profile model file missing.

Soft warnings: tight VRAM margin, timeout below recommendation, OpenCode
provider present but misconfigured.

---

## 9. OpenCode config management

### Detection and reporting (lmml doctor)

Doctor reports the OpenCode config state but never writes to it:
- Config found / not found
- Providers `llamacpp` and `llamacpp_fast` present / missing / mismatched
- Timeout values matching recommendation
- Prints exact fix command to run

### Read-only config preview

```sh
lmml runtime print-config opencode
```

Outputs a ready-to-paste JSON block matching the exact OpenCode provider shape,
with `compaction.reserved` taken from the active profile — not hardcoded:

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
      "name": "llama.cpp fast",
      "options": {
        "baseURL": "http://127.0.0.1:4011/v1",
        "timeout": 7200000,
        "chunkTimeout": 300000
      }
    }
  },
  "compaction": {
    "reserved": 65536
  }
}
```

The `compaction.reserved` value is read from the primary profile's
`compaction_reserved` field. On a 24 GB+ system using a copied preset, this
will be 65536. On an 11 GB system it will be 16384. It is never hardcoded.

### Surgical config mutation (opt-in)

```sh
lmml runtime configure opencode --dry-run   # preview diff, no writes
lmml runtime configure opencode             # apply with backup
lmml runtime configure opencode --path ~/.config/opencode/opencode.json
lmml runtime configure opencode --force     # overwrite conflicting providers
```

What `configure` does:
1. Locate `~/.config/opencode/opencode.json` (or `--path`).
2. Parse JSON structurally — no string replacement.
3. Create timestamped backup: `opencode.json.bak-YYYYMMDD-HHMMSS`.
4. Add or update only lmml-owned keys:
   - `provider.llamacpp`
   - `provider.llamacpp_fast`
   - `compaction.reserved` — always written from the primary profile's
     `compaction_reserved` value; never hardcoded. On a 24 GB+ system this
     will be 65536. Existing user-set values are overwritten only with `--force`.
5. Preserve all unrelated providers, plugins, model settings, user fields.
6. Write the patched file.
7. Print a diff of what changed.

What `configure` never does:
- Install OpenCode.
- Remove user-set providers that don't conflict with lmml.
- Run without a backup.
- Write if `--dry-run` is passed.

---

## 10. TUI one-keystroke harness setup wizard

The TUI provides a single entry point that walks through the full harness
setup flow interactively. Triggered by pressing `h` from any tab, or
automatically on first run when no OpenCode config is detected.

### Wizard steps

```
Step 1 of 5 — System check
  Running lmml doctor...
  ✓ All prerequisites met
  ✓ CUDA: GTX 1080 Ti · sm_61 · 11 GB
  [Continue]  [Abort]

Step 2 of 5 — Profile selection
  Select preset for this machine:
  ○ preset-8gb-desktop      (8 GB VRAM)
  ● preset-11gb-workstation (11 GB VRAM)  ← auto-detected
  ○ preset-24gb-workstation (24 GB VRAM)
  ○ preset-48gb-workstation (48 GB VRAM)
  ○ preset-80gb-server      (80 GB VRAM)
  [Use selected preset]  [Customise]

Step 3 of 5 — Model selection
  Profile 'opencode' needs a full coding model.
  Select from downloaded models:
  ● mistral-7b-q4_k_m.gguf   4.1 GB  Q4_K_M  ✓ fits (4.3 GB free)
  ○ phi-3-mini-q4.gguf       1.8 GB  Q4     ✓ fits
  [Search Hugging Face for more models]
  [Use selected]

Step 4 of 5 — OpenCode config
  OpenCode config found at ~/.config/opencode/opencode.json
  lmml providers: NOT configured

  Preview of changes (--dry-run):
  + provider.llamacpp  →  http://127.0.0.1:4010/v1
  + provider.llamacpp_fast  →  http://127.0.0.1:4011/v1
  + compaction.reserved = 32768

  Backup will be created: opencode.json.bak-20260601-142300
  [Apply]  [Skip]  [Copy to clipboard instead]

Step 5 of 5 — Start runtime
  Ready to start:
    profile opencode       → llama-server :4010  (mistral-7b-q4_k_m)
    profile opencode-fast  → llama-server :4011  (phi-3-mini-q4)

  [Start both profiles]  [Start opencode only]  [Done without starting]
```

Each step is snapshot-tested. No step mutates anything without an explicit
user confirmation (`[Apply]`, `[Start]`). The wizard is re-entrant — if
aborted at any step, no partial changes are written.

---

## 11. Verification checklist (developer / CI)

```sh
# Port binding
ss -tulpn | grep 4010
ss -tulpn | grep 4011

# Health endpoints
curl -s http://127.0.0.1:4010/v1/health | jq .
curl -s http://127.0.0.1:4011/v1/health | jq .

# Runtime status (machine-readable)
lmml runtime status --json

# Profile-specific logs
tail -f ~/.local/share/lmml/logs/profile-opencode.log
tail -f ~/.local/share/lmml/logs/profile-opencode-fast.log

# VRAM live monitoring
watch -n 1 nvidia-smi --query-gpu=memory.used,memory.free --format=csv,noheader

# Context validation
lmml doctor

# Full doctor with VRAM check
lmml doctor --vram-check
```

---

## 12. What is NOT in scope for this document

- ROCm / AMD GPU support (documented as v2 production gap in docs/todo.md).
- Windows support (Linux and macOS only in this version).
- OpenCode installation (lmml never installs OpenCode).
- Model training (profiles tagged `training` are reserved for a future release).
- Tensor parallelism configuration beyond `--split-mode` (future multi-GPU work).
