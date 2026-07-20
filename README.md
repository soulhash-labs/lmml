# lmml

**Local Model Manager for llama.cpp.**

lmml is a Rust terminal UI for local AI: detect hardware, build llama.cpp,
manage GGUF models, and run an OpenAI-compatible inference server for humans,
scripts, and coding agents.

Current v0.1.0 release scope is the tested Linux x86_64 LAN install flow. Other
target tarballs should be built and validated on matching builders before they
are advertised as release-ready.

This is a local/LAN release scope, not a broad production-ready claim. GPU
acceleration is the primary path; intentional CPU-only nodes are supported by
explicit opt-in during preflight/install.

## ✨ What lmml Gives You

- 🧭 Hardware-aware llama.cpp builds for NVIDIA CUDA, AMD ROCm/HIP, Intel,
  Vulkan, Metal, and CPU fallback paths.
- 🧠 Built-in model-family guidance for Qwen3.5, Qwen3.6, Gemma 4, and Hermes 4.
- ⚡ Runtime profiles for validated local-agent setups, including Qwen fanout and
  Gemma 4 MTP speculative decoding.
- 🔌 OpenAI-compatible local serving for OpenCode, LAN agents, shell scripts, and
  proxy/gateway integrations.
- 🔐 Local-first defaults: `127.0.0.1` serving unless you intentionally expose a
  LAN node.

## 🚀 How To Use

Start with [`docs/how-to-use.md`](docs/how-to-use.md). It covers the human TUI
workflow, agent/harness integration, runtime profiles, LAN install, and common
troubleshooting. For training-specific workflows, use
[`docs/training-how-to-use.md`](docs/training-how-to-use.md).
For the built-in NVIDIA/AMD/Intel local-AI GPU catalog, use
[`docs/hardware-gpu-support.md`](docs/hardware-gpu-support.md).
For the built-in Qwen3.5/Qwen3.6/Gemma 4/Hermes 4 model-family catalog, use
[`docs/llm-model-support.md`](docs/llm-model-support.md).

## 📦 Install

### One-line install (Linux / macOS)

```sh
curl -fsSL https://your-lan-or-github/install.sh | sh
```

### LAN install

If you are serving lmml on a local network:

```sh
curl -fsSL http://192.168.1.100:8000/install.sh | BASE_URL=http://192.168.1.100:8000 sh
```

Serve the packaged `dist/` directory from the release host:

```sh
cd dist && python3 -m http.server 8000
```

The LAN HTTP flow verifies `SHA256SUMS` to catch corrupt or incomplete
downloads. It is not tamper-proof: anyone who can alter the HTTP response can
alter both the tarball and checksum file. Treat it as an integrity check for a
trusted LAN release host unless you require signed checksum verification.

For a future public or non-local signed release, publish `SHA256SUMS.minisig`
from `scripts/package-release.sh` and require minisign verification during
install:

```sh
curl -fsSL https://release.example/install.sh | LMML_CHECKSUM_VERIFY=required LMML_MINISIGN_PUBLIC_KEY='RW...' sh
```

### Preflight and source-build bootstrap

The default install path above uses the verified binary tarball. For a
source-build LAN/dev bootstrap, run preflight first and then opt into source
mode explicitly:

```sh
curl -fsSL http://192.168.1.100:8000/preflight.sh | LMML_INSTALL_MODE=source bash
curl -fsSL http://192.168.1.100:8000/install.sh | BASE_URL=http://192.168.1.100:8000 INSTALL_MODE=source bash
```

GPU acceleration is primary and first-class in preflight. Intentional CPU-only
nodes must opt in explicitly:

```sh
curl -fsSL http://192.168.1.100:8000/preflight.sh | LMML_INSTALL_MODE=source LMML_GPU_MODE=cpu-only bash
curl -fsSL http://192.168.1.100:8000/install.sh | BASE_URL=http://192.168.1.100:8000 INSTALL_MODE=source LMML_GPU_MODE=cpu-only bash
```

Narrow apt fixes for compiler/CMake/Git/curl/sccache are opt-in. On Ubuntu
CUDA 11.x hosts with GCC 13+, preflight also recommends `g++-11` because
CUDA 11's device compiler can fail on glibc `_FloatN` headers unless CMake is
configured with `-DCMAKE_CUDA_HOST_COMPILER=/usr/bin/g++-11`:

```sh
curl -fsSL http://192.168.1.100:8000/preflight.sh | LMML_INSTALL_MODE=source LMML_FIX_DEPS=1 bash
```

For a Quadro M6000 24GB target running Qwen3.5 9B Q8, the piped installer can
print the proposed fanout profile math after install:

```sh
curl -fsSL http://192.168.1.100:8000/install.sh | BASE_URL=http://192.168.1.100:8000 LMML_PROFILE_HINT=quadro-m6000-qwen35-9b-q8 sh
```

That hint documents:

```text
llama-server ctx_size: 262144 tokens
OpenCode compaction.reserved: 49152 tokens for 4-slot fanout
per-slot context at parallel 4: 65536 tokens
recommended subagent soft cap: 32768 tokens
recommended extra_args: ["--parallel", "4", "--slot-save-path", "/home/angelo/.local/share/lmml/llama-slots"]
Qwen thinking sampling: temperature=0.6 top_p=0.95 top_k=20 min_p=0
Qwen non-thinking sampling: temperature=0.7 top_p=0.8 top_k=20 min_p=0
minimum context for thinking: 128000 tokens
vision/video support: requires matching mmproj vision encoder beside the GGUF
MTP support: supported by model, keep disabled until profiled
```

The Qwen3.5 9B model is natively multimodal, but llama.cpp cannot accept
image/video inputs from the text GGUF alone. Put the matching `mmproj` vision
encoder file beside the main GGUF and configure lmml/llama-server to load both
files before claiming vision support on a LAN node.

For the validated Orion GTX 1080 Ti 11GB + Qwen3.5 4B Q8 deep profile:

```sh
curl -fsSL http://192.168.1.100:8000/install.sh | BASE_URL=http://192.168.1.100:8000 LMML_PROFILE_HINT=orion-qwen35-4b-q8 sh
```

That hint documents the current local OpenCode/lmml route:

```text
llama-server ctx_size: 262144 tokens
OpenCode compaction.reserved: 65536 tokens
OpenCode usable input limit: 196608 tokens
OpenCode output limit: 18000 tokens
operator compact target: 90000-120000 live prompt tokens
operator red zone: 120000-170000 live prompt tokens
operator hard compress/reject: 170000-190000 live prompt tokens
OpenCode provider timeout: 7200 seconds
OpenCode stream chunk timeout: 2400 seconds
llama-server parallel slots: 1
recommended extra_args: ["--parallel", "1", "--slot-save-path", "/home/angelo/.local/share/lmml/llama-slots"]
recommended KV/cache args: ["-ctk", "q8_0", "-ctv", "q8_0", "--cache-ram", "4096"]
```

For a headless AMD BC-250 Vulkan target running Qwen3.5 9B Q4_K_M, use source
install so llama.cpp builds with Vulkan/RADV on that board:

```sh
curl -fsSL http://192.168.1.100:8000/preflight.sh | LMML_INSTALL_MODE=source LMML_GPU_MODE=vulkan bash
curl -fsSL http://192.168.1.100:8000/install.sh | BASE_URL=http://192.168.1.100:8000 INSTALL_MODE=source LMML_GPU_MODE=vulkan LMML_PROFILE_HINT=bc250-qwen35-9b-q4km-vulkan sh
```

That hint documents the headless LAN profile:

```text
Ubuntu Server / Debian headless: ~6-8GB
llama.cpp source + binaries: ~1GB
Qwen3.5 9B Q4_K_M GGUF: ~5.5GB
Profile: bc250-qwen9b-q4km-vulkan
Host: 0.0.0.0
Port: 8080
Context: 4096
GPU layers: 99
Threads: 6
Backend: Vulkan/RADV
```

### After install

```sh
lmml doctor    # check your system
lmml           # launch the TUI
```

For the full training procedure, dataset formatting examples, agent checklist,
and VRAM guidance, see [`docs/training-how-to-use.md`](docs/training-how-to-use.md).

Experimental fine-tuning uses llama.cpp's native C++ training binaries, not a
Python/PyTorch training script. Current upstream `llama-finetune` performs
full-model GGUF fine-tuning: lmml maps `--train-data` to `--file`, maps
`--model-base` to `--model` unless the binary advertises `--model-base`, and
passes `--output` through as the fine-tuned GGUF output:

```sh
lmml train \
  --model-base ./models/Qwen3.5-9B-BF16.gguf \
  --train-data ./data/train.txt \
  --output ./models/Qwen3.5-9B-Finetuned.gguf \
  -- --epochs 3 --ctx-size 512 --batch-size 4 --n-gpu-layers 32
```

Use an F16/BF16 base GGUF for training, then quantize the output GGUF afterward
with `llama-quantize` if you need Q8/Q4 deployment artifacts. lmml only enables
custom-fork adapter flags such as `--lora-out`, `--checkpoint-in`, and
`--checkpoint-out` when `llama-finetune --help` explicitly advertises them.

The installer runs `lmml doctor` before reporting success. Missing hard
prerequisites such as a compiler, CMake, Git, or required disk space cause the
install command to fail clearly even though the binary has already been copied.
Fix the reported prerequisites and rerun `lmml doctor`.

### Uninstall

```sh
curl -fsSL https://your-lan-or-github/uninstall.sh | sh
```

Or, after installing:

```sh
lmml-uninstall
```

## Build From Source

```sh
cargo build --release -p lmml-tui
./target/release/lmml doctor
```

## Harness Runtime Direction

For the practical agent setup path, see [`docs/how-to-use.md`](docs/how-to-use.md).

For coding harnesses such as OpenCode, Claude Code, and Hermes-compatible
clients, lmml should manage long-running `llama-server` HTTP endpoints.
`llama-cli` is reserved for one-shot diagnostics and smoke checks.

`lmml runtime configure opencode` is local-first by default: it adds the
lmml-managed providers and routes OpenCode's top-level `model` and
`small_model` to those local providers. Operators who want to keep cloud routing
active can pass `--model-source existing --small-model-source existing`.

### 🔌 Agent Client Wiring

#### OpenCode

OpenCode is the first-class local harness target today. Point its OpenAI-
compatible provider at the lmml server:

```text
baseURL: http://127.0.0.1:1200/v1
model: llamacpp/<your-gguf-model-name>
```

Use the helper when you want lmml to write the provider block:

```sh
lmml runtime configure opencode --base-url http://127.0.0.1:1200/v1
```

#### Claude Code

Claude Code can use lmml through the `lmml-node` Anthropic Messages
compatibility endpoint. Keep the TUI-managed `llama-server` on port `1200`, then
start `lmml-node` as the API adapter on port `8101`:

```sh
LMML_NODE_API_KEY=local-dev-key lmml-node --llama-url http://127.0.0.1:1200
```

In the Claude Code shell:

```sh
export ANTHROPIC_BASE_URL=http://127.0.0.1:8101
export ANTHROPIC_AUTH_TOKEN=local-dev-key
export ANTHROPIC_MODEL=Qwen3.5-4B-Q8_0.gguf
export ANTHROPIC_SMALL_FAST_MODEL=Qwen3.5-4B-Q8_0.gguf
claude
```

Direct adapter contract:

```text
Claude Code -> http://127.0.0.1:8101/v1/messages -> lmml-node -> http://127.0.0.1:1200/v1/chat/completions
```

The compatibility endpoint maps Anthropic text messages to OpenAI-compatible
chat completions, translates Anthropic tool schemas into OpenAI function tools,
maps OpenAI tool calls back to Anthropic `tool_use` blocks, and synthesizes
Anthropic SSE events when `"stream": true`. Image and document content blocks
are intentionally rejected until lmml has validated multimodal routing.

#### LAN Router / Load Balancer

For a LAN with multiple GPU machines, run `lmml-node` on each worker and
`lmml-router` on the coordinator. The router exposes the same useful endpoints
to clients and selects a ready upstream by route support, requested model, and
current LMML load metadata. It also aggregates `GET /v1/models` from currently
routable workers so OpenAI-compatible clients can inspect the coordinator as
their base URL.

Example with a main workstation and a BC-250 worker:

```sh
# Workstation worker, usually near the TUI-managed llama-server on port 1200.
LMML_NODE_API_KEY=worker-key lmml-node \
  --host 0.0.0.0 \
  --port 8101 \
  --node-name workstation \
  --llama-url http://127.0.0.1:1200

# BC-250 worker, usually beside a Vulkan llama-server on port 8080.
LMML_NODE_API_KEY=worker-key lmml-node \
  --host 0.0.0.0 \
  --port 8101 \
  --node-name bc250 \
  --llama-url http://127.0.0.1:8080

# Coordinator router.
LMML_ROUTER_API_KEY=router-key lmml-router \
  --host 0.0.0.0 \
  --port 8100 \
  --upstream workstation=http://192.168.50.178:8101 \
  --upstream bc250=http://192.168.50.176:8101 \
  --upstream-key workstation=worker-key \
  --upstream-key bc250=worker-key
```

Point OpenAI-compatible clients at `http://<router-ip>:8100/v1` and Anthropic
Messages clients at `http://<router-ip>:8100`. LAN-visible router and worker
routes require bearer auth unless explicitly started with the unsafe development
escape hatch.

Static upstreams can be replaced or supplemented with opt-in LAN discovery:

```sh
LMML_NODE_API_KEY=worker-key lmml-node \
  --host 0.0.0.0 \
  --port 8101 \
  --public-url http://192.168.50.178:8101 \
  --advertise-lan

LMML_ROUTER_API_KEY=router-key lmml-router \
  --host 0.0.0.0 \
  --port 8100 \
  --discover-lan \
  --upstream-key default=worker-key
```

The router treats advertisements as hints only. Discovered nodes are used only
after authenticated health, capability, and load probes pass.

#### Hermes

Hermes has two meanings in this repo:

- **Hermes 4 models:** lmml recognizes Hermes 4 14B, 4.3 36B, 70B, and 405B FP8
  GGUF names and displays ChatML/reasoning guidance.
- **Hermes-style clients/agents:** configure them like any OpenAI-compatible
  client when they can target a local base URL:

```text
base URL: http://127.0.0.1:1200/v1
model: <your Hermes or other GGUF filename>
```

Hermes runtime profiles should stay explicit and hardware-validated. The model
catalog can recognize the family before LMML claims a tuned runtime profile.

### OpenCode With The TUI Server On Port 1200

On this workstation, OpenCode is intentionally wired to the live lmml TUI server:

```text
http://127.0.0.1:1200/v1
```

Do not "repair" this back to the default managed runtime profile ports
`4010/4011` unless the separate `lmml runtime start opencode --detach` flow is
actually being used. If the TUI Server tab says ready at `127.0.0.1:1200`,
OpenCode providers should use `baseURL: "http://127.0.0.1:1200/v1"`.

Quick verification:

```sh
curl -fsS http://127.0.0.1:1200/health
curl -fsS http://127.0.0.1:1200/v1/models
opencode models llamacpp
opencode models llamacpp_fast
```

Expected OpenCode config shape:

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

Current proven context math:

```text
server context: 262144 tokens
OpenCode compaction.reserved: 65536 tokens
OpenCode model output limit: 18000 tokens
OpenCode stream chunk timeout: 2400 seconds
OpenCode long-run timeout: 7200 seconds
usable input before compaction: 196608 tokens
operator compact target: 90000-120000 live prompt tokens
operator red zone: 120000-170000 live prompt tokens
hard reject/compress threshold: 170000-190000 tokens
llama-server parallel slots: 1
slot save path: /home/angelo/.local/share/lmml/llama-slots
```

On the 11GB GTX 1080 Ti validation machine, the working deep profile is
single-slot Qwen Q8 at 256k with Q8 KV cache and 4096 MiB host cache. Earlier
128k testing proved that auto-selected four-slot mode exhausted KV cache during
concurrent OpenCode background work. Keep this TUI-managed 1200 server in deep
single-agent mode unless VRAM headroom and per-slot context are revalidated:

```toml
extra_args = [
  "--parallel", "1",
  "--slot-save-path", "/home/angelo/.local/share/lmml/llama-slots",
  "-ctk", "q8_0",
  "-ctv", "q8_0",
  "--cache-ram", "4096"
]
```

The TUI includes two built-in runtime profiles for `Qwen3.5-4B-Q8_0.gguf`.
Press `p` on the Models or Server tab to switch between them:

```text
orion-qwen-q8-deep:
  ctx_size: 262144
  parallel: 1
  OpenCode/Sisyphus: 0 subagents by default
  operator compact target: 90000-120000
  hard compress/reject: 170000-190000

orion-qwen-q8-balanced:
  ctx_size: 262144
  parallel: 2
  OpenCode/Sisyphus: 1 subagent max
  per-slot theoretical context: 131072
  practical per-agent target: 60000-80000
  hard compress/reject: 100000-115000

orion-qwen-q8-kvu-fanout4 / fanout6 / fanout8:
  ctx_size: 65536
  parallel: 4, 6, or 8
  KV: q4_0 key/value with --kv-unified
  target: high-concurrency experimental harness runs on 11/12GB GPUs

5060ti-qwen4b-fanout4:
  ctx_size: 131072
  parallel: 4
  OpenCode/Sisyphus: 3 subagents max
  per-slot theoretical context: 32768
  practical per-agent target: 16000-24000

5060ti-qwen4b-dual:
  ctx_size: 262144
  parallel: 2

5060ti-qwen4b-kvu-fanout4 / fanout6 / fanout8:
  ctx_size: 73728
  parallel: 4, 6, or 8
  KV: q4_0 key/value with --kv-unified

5070ti-qwen4b-fanout4:
  ctx_size: 131072
  parallel: 4
  OpenCode/Sisyphus: 3 subagents max
  per-slot theoretical context: 32768
  practical per-agent target: 16000-24000

5070ti-qwen4b-dual:
  ctx_size: 262144
  parallel: 2
  OpenCode/Sisyphus: 1 subagent max
  per-slot theoretical context: 131072
  practical per-agent target: 60000-90000

5070ti-qwen4b-kvu-fanout4 / fanout6 / fanout8:
  ctx_size: 73728
  parallel: 4, 6, or 8
  KV: q4_0 key/value with --kv-unified

m6000-qwen9b-deep:
  ctx_size: 262144
  parallel: 1
  OpenCode/Sisyphus: 0 subagents by default
  practical single-agent target: 120000-170000

m6000-qwen9b-fanout4:
  ctx_size: 262144
  parallel: 4
  OpenCode/Sisyphus: 3 subagents max
  per-slot theoretical context: 65536
  practical per-agent target: 32000-48000

m6000-qwen9b-fanout6:
  ctx_size: 262144
  parallel: 6
  OpenCode/Sisyphus: 5 subagents max after validation
  per-slot theoretical context: 43690
  practical per-agent target: 20000-30000

5070ti-qwen9b-deep:
  ctx_size: 196608
  parallel: 1
  OpenCode/Sisyphus: 0 subagents by default
  practical single-agent target: 90000-130000

5070ti-qwen9b-balanced2:
  ctx_size: 131072
  parallel: 2
  OpenCode/Sisyphus: 1 subagent max
  per-slot theoretical context: 65536
  practical per-agent target: 32000-48000

gemma4-12b-mtp-q4km:
  model: Gemma4-12B-QAT-Q4_K_M.gguf
  required draft model: mtp-gemma-4-12B-it.gguf beside the main GGUF
  ctx_size: 73728
  gpu_layers: 99
  parallel: 1
  MTP: -md <models>/mtp-gemma-4-12B-it.gguf --spec-type draft-mtp
  sampling: temperature=0.6 top_k=64 top_p=0.9 min_p=0.05 repeat_penalty=1.1

bc250-qwen9b-q4km-vulkan:
  model: Qwen3.5-9B-Q4_K_M.gguf
  backend: Vulkan/RADV
  host: 0.0.0.0
  port: 8080
  ctx_size: 4096
  gpu_layers: 99
  parallel: 1
  threads: 6
```

Switching while the server is running changes the saved profile and visible
settings, but the active `llama-server` process keeps its launch arguments until
you stop and restart it. OpenCode must also be restarted after changing
`LMML_SISYPHUS_SUBAGENTS` or `opencode.json`.

Use the GGUF embedded chat template for Qwen and Nemotron by keeping lmml
`chat_template` empty. Do not share one external Qwen template across models
unless that exact model/profile has been revalidated.

Frozen evidence for the current working setup is in
[docs/opencode-1200-evidence.md](docs/opencode-1200-evidence.md).

See [docs/runtime-harness-plan.md](docs/runtime-harness-plan.md).
