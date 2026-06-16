# lmml How-To Use Guide

This guide is for two audiences:

- **Humans** operating lmml directly from the terminal UI.
- **Agents and coding harnesses** that use lmml as a local `llama-server` provider.

lmml's job is to make a local llama.cpp stack reproducible: detect hardware,
build the correct llama.cpp binaries for that machine, manage GGUF models, and
run an OpenAI-compatible server.

## Mental Model

```text
Human / Agent / Harness
        |
        v
OpenAI-compatible HTTP client
        |
        v
lmml-managed llama-server
        |
        v
local GGUF model + local llama.cpp build
```

Important boundaries:

- The LAN installer installs **lmml**, not a universal prebuilt llama.cpp.
- lmml builds llama.cpp locally because CUDA architecture, compiler, GPU backend,
  and training binaries are machine-specific.
- Harnesses should talk to `llama-server` over HTTP. They should not shell out to
  `llama-cli` for normal agent work.
- Fine-tuning is a separate workflow from serving. Current upstream
  `llama-finetune` performs full-model GGUF fine-tuning unless the installed
  binary explicitly advertises custom adapter flags.

## Quick Start For Humans

### 1. Install lmml

From a LAN release host:

```sh
curl -fsSL http://192.168.50.178:8000/install.sh | BASE_URL=http://192.168.50.178:8000 sh
```

Optional profile hints print recommended settings after install:

```sh
curl -fsSL http://192.168.50.178:8000/install.sh | BASE_URL=http://192.168.50.178:8000 LMML_PROFILE_HINT=orion-qwen35-4b-q8 sh
curl -fsSL http://192.168.50.178:8000/install.sh | BASE_URL=http://192.168.50.178:8000 LMML_PROFILE_HINT=quadro-m6000-qwen35-9b-q8 sh
```

### 2. Run preflight checks

```sh
lmml doctor
lmml smoke
```

`doctor` checks the system. `smoke` checks that lmml can start headlessly.

### 3. Launch the TUI

```sh
lmml
```

Use the numbered tabs:

```text
1 Detect   2 Build   3 Models   4 Server   5 Settings
```

Recommended first-run order:

1. **Detect**: confirm OS, CPU, GPU, CUDA/ROCm/Vulkan/Metal, RAM, and tools.
2. **Build**: press `b` to build llama.cpp, or `B` for a clean rebuild.
3. **Models**: add/download/select a GGUF model.
4. **Server**: press `s` to start/stop the local server.
5. **Settings**: review persistent paths and server defaults.

### 4. Select a model and profile

On the Models tab, select a GGUF model. lmml applies the first matching runtime
profile automatically.

Press `p` on the Models or Server tab to cycle runtime profiles for the selected
model. Restart the server after changing profiles.

### 5. Start serving

On the Server tab, press `s`.

A healthy local server usually reports:

```text
Status: Ready { url: "http://127.0.0.1:1200" }
```

Verify from a shell:

```sh
curl -fsS http://127.0.0.1:1200/health
curl -fsS http://127.0.0.1:1200/v1/models
```

## Quick Start For Agents And Harnesses

Agents should use lmml as a local OpenAI-compatible HTTP endpoint.

### Preferred endpoint

For the TUI-managed server:

```text
http://127.0.0.1:1200/v1
```

Use this when the lmml Server tab says ready on port `1200`.

### OpenCode route

Configure OpenCode to use the TUI server:

```sh
lmml runtime configure opencode --yes --force
```

Then verify:

```sh
opencode models llamacpp
opencode models llamacpp_fast
```

If OpenCode is intentionally using the TUI-managed server, provider `baseURL`
should be:

```json
"baseURL": "http://127.0.0.1:1200/v1"
```

Do not change this back to `4010`/`4011` unless using the separate detached
runtime flow:

```sh
lmml runtime start opencode --detach
lmml runtime status
lmml runtime logs opencode --follow
```

### Agent context policy

Agents must treat context budget as a shared operational limit, not an abstract
model maximum.

Recommended defaults:

| Hardware tier | Profile style | Practical agent context |
| --- | --- | --- |
| 11/12GB GPU | deep single-agent | 90k-120k target, 170k-190k hard red zone |
| 11/12GB GPU | KV-unified fanout | about 64k total shared pool |
| 16GB GPU | KV-unified fanout | about 74k total shared pool |
| 24GB GPU | M6000 fanout | about 84k total shared pool |

Rules for agents:

- Compact early. Do not wait for server rejection.
- Use one deep agent for long context on 11GB cards.
- Use fanout profiles only when concurrency is more valuable than per-agent
  context depth.
- Restart the harness after provider/model changes; a llama-server restart alone
  does not reload client-side config.

## Runtime Profiles

lmml ships named profiles so humans and agents can agree on behavior.

### Stable deep profiles

Use these for long, single-agent sessions:

```text
orion-qwen-q8-deep       4B Q8, 262144 ctx, parallel 1, q8 KV
m6000-qwen9b-deep        9B Q8, 262144 ctx, parallel 1, q8 KV
5060ti-qwen9b-deep       9B Q8, 196608 ctx, parallel 1, q8 KV
5070ti-qwen9b-deep       9B Q8, 196608 ctx, parallel 1, q8 KV
```

### KV-unified fanout profiles

Use these for concurrent agent harnesses. They intentionally trade context depth
for slot count:

```text
orion-qwen-q8-kvu-fanout4 / fanout6 / fanout8    ctx 65536, q4 KV, --kv-unified
5060ti-qwen4b-kvu-fanout4 / fanout6 / fanout8    ctx 73728, q4 KV, --kv-unified
5070ti-qwen4b-kvu-fanout4 / fanout6 / fanout8    ctx 73728, q4 KV, --kv-unified
m6000-qwen9b-kvu-fanout4 / fanout6 / fanout8     ctx 86016, q4 KV, --kv-unified
5060ti-qwen9b-kvu-fanout4 / fanout6 / fanout8    ctx 73728, q4 KV, --kv-unified
5070ti-qwen9b-kvu-fanout4 / fanout6 / fanout8    ctx 73728, q4 KV, --kv-unified
```

These are experimental operating profiles. Watch VRAM, pinned host allocation,
prompt-processing speed, and llama-server logs before relying on them for long
runs.

## Fine-Tuning Workflow

For the full human/agent training guide, including dataset formatting examples
and VRAM expectations, see [`training-how-to-use.md`](training-how-to-use.md).

Current upstream llama.cpp behavior:

- `llama-finetune` fine-tunes and writes a full output GGUF.
- It uses `--model`, `--file`, and `--output`.
- It does not advertise `--lora-out` on official upstream builds.

Use an F16/BF16/F32 base GGUF, not a quantized deployment model:

```sh
lmml train \
  --model-base ./models/Qwen3.5-9B-BF16.gguf \
  --train-data ./data/train.txt \
  --output ./models/Qwen3.5-9B-Finetuned.gguf \
  -- --epochs 3 --ctx-size 512 --batch-size 4 --n-gpu-layers 32
```

Then quantize for serving if needed:

```sh
/home/angelo/.local/share/lmml/llama.cpp/build/bin/llama-quantize \
  ./models/Qwen3.5-9B-Finetuned.gguf \
  ./models/Qwen3.5-9B-Finetuned-Q8_0.gguf \
  Q8_0
```

lmml only enables custom adapter flags such as `--lora-out`, `--checkpoint-in`,
and `--checkpoint-out` when `llama-finetune --help` explicitly advertises them.

## Model File Rules

- Use text GGUF files for normal text serving.
- For multimodal Qwen profiles, keep the matching `mmproj` file beside the main
  GGUF. The `mmproj` file is the vision encoder, not the primary text model.
- Do not accidentally select an `mmproj` file as the main model for text serving.
- Use F16/BF16/F32 models for training, then quantize the trained output for
  deployment.

## Common Operations

### CUDA host compiler workaround

On Ubuntu hosts where CUDA 11.x or CUDA 13.x is paired with GCC 13+, nvcc can
fail while compiling `CMakeCUDACompilerId.cu` with CUDA/glibc math header errors
such as incompatible `rsqrt` / `rsqrtf` exception specifications. lmml detects
that combination and, when `/usr/bin/g++-11` exists, passes:

```text
-DCMAKE_CUDA_HOST_COMPILER=/usr/bin/g++-11
```

Install `g++-11` if the preflight/build warning asks for it.

### Update llama.cpp and rebuild

In the TUI Build tab:

- Press `u` to check for updates.
- Press `b` to build.
- Press `B` for a clean rebuild.

lmml's default tracking mode follows upstream `ggml-org/llama.cpp` main/master.
If a checkout already exists, lmml uses `git pull --ff-only` before building.

### Serve release files over LAN

On the release host:

```sh
cd /home/angelo/repos/lmml/dist
python3 -m http.server 8000 --bind 0.0.0.0
```

On another LAN machine:

```sh
curl -fsSL http://192.168.50.178:8000/install.sh | BASE_URL=http://192.168.50.178:8000 sh
```

### Package a new release

```sh
scripts/package-release.sh
```

Then verify:

```sh
curl -fsS http://127.0.0.1:8000/latest
curl -fsS http://127.0.0.1:8000/SHA256SUMS
```

## Troubleshooting

### Build panel says complete but log shows an old percentage

This was a display issue in older lmml builds: the log pane could show an older
slice of the retained CMake output. Current lmml renders the newest visible log
lines and appends `Build complete: <binary>` when completion is received.

### `--cache-idle-slots requires --kv-unified`

This warning means llama.cpp disabled idle-slot cache behavior because
`--kv-unified` was not present. It is not fatal. For single-slot deep profiles,
it is usually safe to ignore. For 4/6/8-slot agent fanout, use a `kvu` profile.

### Server fails to start because a path is not a directory

Check `--slot-save-path`. It must point to a directory:

```sh
mkdir -p /home/angelo/.local/share/lmml/llama-slots
```

### OpenCode cannot start a new session

Check the provider endpoint and timeout:

```sh
curl -fsS http://127.0.0.1:1200/health
curl -fsS http://127.0.0.1:1200/v1/models
```

OpenCode provider timeout should be long enough for local large-context prompt
processing. Current local policy uses 7200 seconds for long-running background
operations.

### Training rejects LoRA flags

Your installed `llama-finetune` probably does not advertise those flags. Check:

```sh
/home/angelo/.local/share/lmml/llama.cpp/build/bin/llama-finetune --help | grep -E 'lora-out|checkpoint'
```

If nothing appears, use full-model GGUF fine-tuning with `--output`.

## Deeper References

- `docs/training-how-to-use.md` — training workflow, dataset formatting, and VRAM guidance.
- `docs/runtime-harness-plan.md` — harness routing and context budget details.
- `docs/llama-server-integration-contract.md` — server integration contract.
- `docs/lmml-fleet-profiles.md` — fleet profile rationale.
- `docs/learnings.md` — operational notes and decisions.
- `docs/release-checklist.md` — packaging and release validation.
