# lmml

lmml is a Rust terminal UI for managing llama.cpp locally: detect hardware,
build llama.cpp, manage GGUF models, and run the inference server.

Current v0.1.0 release scope is the tested Linux x86_64 LAN install flow. Other
target tarballs should be built and validated on matching builders before they
are advertised as release-ready.

This is a local/LAN release scope, not a broad production-ready claim. GPU
acceleration is the primary path; intentional CPU-only nodes are supported by
explicit opt-in during preflight/install.

## Install

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

Narrow apt fixes for compiler/CMake/Git/curl/sccache are opt-in:

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

### After install

```sh
lmml doctor    # check your system
lmml           # launch the TUI
```

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

For coding harnesses such as OpenCode and Claude Code, lmml should manage
long-running `llama-server` HTTP endpoints. `llama-cli` is reserved for one-shot
diagnostics and smoke checks.

`lmml runtime configure opencode` is local-first by default: it adds the
lmml-managed providers and routes OpenCode's top-level `model` and
`small_model` to those local providers. Operators who want to keep cloud routing
active can pass `--model-source existing --small-model-source existing`.

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
