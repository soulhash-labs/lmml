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
