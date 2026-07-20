# LMML LAN Client Install Guide

This guide is for machines on the same LAN that need to install `lmml` from the
release files served by the LMML release host.

## Current Release Host

Current checked host:

```text
http://192.168.50.176:8000
```

If the release host IP changes, replace `192.168.50.176` in the commands below
with the current host IP.

## Release Host Setup

On the machine that has the LMML repository and release files:

```sh
cd /home/angelo/repos/lmml/dist
python3 -m http.server 8000 --bind 0.0.0.0
```

Verify from the release host:

```sh
curl -fsS http://127.0.0.1:8000/latest
curl -fsSI http://127.0.0.1:8000/install.sh
```

Verify from a LAN client:

```sh
curl -fsS http://192.168.50.176:8000/latest
curl -fsSI http://192.168.50.176:8000/install.sh
```

## Default Binary Install

Run this on the LAN client:

```sh
curl -fsSL http://192.168.50.176:8000/install.sh | BASE_URL=http://192.168.50.176:8000 sh
```

This installs the packaged `lmml` binary, verifies release checksums, installs
`lmml-uninstall`, and runs `lmml doctor` plus `lmml smoke`.

## Hardware/Profile Hints

Profile hints do not download model files. They print post-install guidance and
make the expected runtime profile explicit.

### Orion Qwen3.5 4B Q8

```sh
curl -fsSL http://192.168.50.176:8000/install.sh | BASE_URL=http://192.168.50.176:8000 LMML_PROFILE_HINT=orion-qwen35-4b-q8 sh
```

### Quadro M6000 Qwen3.5 9B Q8

```sh
curl -fsSL http://192.168.50.176:8000/install.sh | BASE_URL=http://192.168.50.176:8000 LMML_PROFILE_HINT=quadro-m6000-qwen35-9b-q8 sh
```

### Gemma4 12B QAT Q4_K_M with MTP

```sh
curl -fsSL http://192.168.50.176:8000/install.sh | BASE_URL=http://192.168.50.176:8000 LMML_PROFILE_HINT=gemma4-12b-mtp-q4km sh
```

Gemma4 MTP requires both GGUF files in the LMML model directory:

```text
Gemma4-12B-QAT-Q4_K_M.gguf
mtp-gemma-4-12B-it.gguf
```

After install, put both files in `~/.local/share/lmml/models`, select
`Gemma4-12B-QAT-Q4_K_M.gguf` in the TUI, and press `p` until the profile is
`gemma4-12b-mtp-q4km`.

### AMD BC-250 Qwen3.5 9B Q4_K_M Vulkan

BC-250 should use a source install so llama.cpp is built locally with the Vulkan
backend:

```sh
curl -fsSL http://192.168.50.176:8000/preflight.sh | LMML_INSTALL_MODE=source LMML_GPU_MODE=vulkan bash
curl -fsSL http://192.168.50.176:8000/install.sh | BASE_URL=http://192.168.50.176:8000 INSTALL_MODE=source LMML_GPU_MODE=vulkan LMML_PROFILE_HINT=bc250-qwen35-9b-q4km-vulkan sh
```

Expected footprint:

```text
Ubuntu Server / Debian headless: ~6-8GB
llama.cpp source + compiled binaries: ~1GB
Qwen3.5 9B Q4_K_M GGUF: ~5.5GB
```

Put the model in `~/.local/share/lmml/models/Qwen3.5-9B-Q4_K_M.gguf`. If the
downloaded file is named `qwen-9b-q4_k_m.gguf`, rename it or create a symlink so
the built-in profile matches.

The built-in profile is:

```text
bc250-qwen9b-q4km-vulkan
host: 0.0.0.0
port: 8080
ctx_size: 4096
gpu layers: 99
threads: 6
parallel: 1
```

Equivalent raw llama.cpp command shape:

```sh
~/.local/share/lmml/llama.cpp/build/bin/llama-server \
  -m ~/.local/share/lmml/models/Qwen3.5-9B-Q4_K_M.gguf \
  --host 0.0.0.0 \
  --port 8080 \
  -ngl 99 \
  -fa \
  -c 4096
```

Optional systemd service for a dedicated headless BC-250 node:

```ini
[Unit]
Description=LMML llama.cpp Vulkan Server for Qwen3.5 9B
After=network.target

[Service]
Type=simple
User=yourusername
WorkingDirectory=/home/yourusername/.local/share/lmml/llama.cpp
ExecStart=/home/yourusername/.local/share/lmml/llama.cpp/build/bin/llama-server -m /home/yourusername/.local/share/lmml/models/Qwen3.5-9B-Q4_K_M.gguf --host 0.0.0.0 --port 8080 -ngl 99 -fa -c 4096
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

Install and start it:

```sh
sudo install -m 0644 llama-server.service /etc/systemd/system/llama-server.service
sudo systemctl daemon-reload
sudo systemctl enable --now llama-server
curl -fsS http://127.0.0.1:8080/health
```

Use `0.0.0.0:8080` only on a trusted LAN. Add firewall rules or keep the host on
`127.0.0.1` when the network is shared or untrusted.

## LAN AI Router

`lmml-router` lets one machine act as the LAN coordinator while GPU machines
serve as LMML workers. Use this when agents should send requests to one endpoint
and let LMML pick between a workstation GPU and a BC-250 node.

Each worker needs a running `llama-server` and a running `lmml-node`:

```sh
LMML_NODE_API_KEY=worker-key lmml-node \
  --host 0.0.0.0 \
  --port 8101 \
  --node-name workstation \
  --llama-url http://127.0.0.1:1200

LMML_NODE_API_KEY=worker-key lmml-node \
  --host 0.0.0.0 \
  --port 8101 \
  --node-name bc250 \
  --llama-url http://127.0.0.1:8080
```

Run the router on the coordinator:

```sh
LMML_ROUTER_API_KEY=router-key lmml-router \
  --host 0.0.0.0 \
  --port 8100 \
  --upstream workstation=http://192.168.50.178:8101 \
  --upstream bc250=http://192.168.50.176:8101 \
  --upstream-key workstation=worker-key \
  --upstream-key bc250=worker-key
```

Agents and clients can then use:

```text
OpenAI-compatible base URL: http://<router-ip>:8100/v1
Anthropic Messages base URL: http://<router-ip>:8100
LMML-native inference:        POST http://<router-ip>:8100/v1/infer
```

Router selection is intentionally conservative: it probes worker health,
capabilities, and load; filters by supported endpoint and requested model; then
chooses the ready worker with the lowest reported running request count.
The router also exposes `GET /v1/models`, aggregated from currently routable
workers, for clients that inspect model metadata before sending requests.

For smaller LANs, static `--upstream` entries are simplest. For workstations
that come and go, enable opt-in LAN discovery:

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

Discovered nodes must advertise authenticated APIs. The router ignores
unauthenticated advertisements and still verifies each candidate through
authenticated `/v1/health`, `/v1/capabilities`, and `/v1/load` probes before
routing requests.

## Source Install

Use source install when the client needs to build `llama.cpp` locally for its
own GPU/backend:

```sh
curl -fsSL http://192.168.50.176:8000/preflight.sh | LMML_INSTALL_MODE=source bash
curl -fsSL http://192.168.50.176:8000/install.sh | BASE_URL=http://192.168.50.176:8000 INSTALL_MODE=source bash
```

For CPU-only source install:

```sh
curl -fsSL http://192.168.50.176:8000/preflight.sh | LMML_INSTALL_MODE=source LMML_GPU_MODE=cpu-only bash
curl -fsSL http://192.168.50.176:8000/install.sh | BASE_URL=http://192.168.50.176:8000 INSTALL_MODE=source LMML_GPU_MODE=cpu-only bash
```

## Signed Checksum Policy

Default LAN installs use `LMML_CHECKSUM_VERIFY=optional`: unsigned checksums are
accepted for trusted LAN testing, but `SHA256SUMS` is still checked.

Require minisign verification when a signed checksum is published:

```sh
curl -fsSL http://192.168.50.176:8000/install.sh | BASE_URL=http://192.168.50.176:8000 LMML_CHECKSUM_VERIFY=required LMML_MINISIGN_PUBLIC_KEY='<public-key>' sh
```

Disable checksum verification only for local debugging:

```sh
curl -fsSL http://192.168.50.176:8000/install.sh | BASE_URL=http://192.168.50.176:8000 LMML_CHECKSUM_VERIFY=off sh
```

## Client Verification

After install on the LAN client:

```sh
lmml doctor
lmml smoke
lmml
```

If `lmml` is not found, add the install directory to `PATH`:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

## Common Failures

- `curl: Failed to connect`: the release host is not serving on `0.0.0.0:8000`,
  the host IP changed, or a firewall blocks TCP `8000`.
- `BASE_URL` downloads from GitHub instead of LAN: use the pipe form shown in
  this guide. Do not put `BASE_URL=...` before `curl`; that only affects `curl`.
- Source build fails before CMake: run `preflight.sh` first and fix missing
  compiler, Git, CMake, CUDA, or CPU-only mode requirements.
- Gemma4 MTP starts without speedup: confirm `mtp-gemma-4-12B-it.gguf` is beside
  `Gemma4-12B-QAT-Q4_K_M.gguf` and the active TUI profile is
  `gemma4-12b-mtp-q4km`.
