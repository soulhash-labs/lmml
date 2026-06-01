# lmml — Todo / Roadmap

> Current task tracking for the lmml project.

---

## Phase 1 — MVP (Complete)

- [x] Project scaffold (Cargo.toml, module structure, AGENTS.md)
- [x] TUI infrastructure (terminal init/restore, event loop, screen dispatch)
- [x] Hardware probe engine (OS, CUDA, ROCm, Vulkan, Metal, CPU, cmake flags)
- [x] Build pipeline (git clone, cmake configure + build, streaming output)
- [x] Model management (filesystem scan for `.gguf`, HuggingFace download with progress)
- [x] Server lifecycle (start/stop/restart, health check, auto-restart on crash)
- [x] Config persistence (`~/.lmml/config.toml` read/write, `state.toml` auto-save)
- [x] Settings screen (read/write all config fields, persisted to disk)
- [x] 5-screen navigation (Dashboard, Models, Server, Build, Settings)
- [x] Screen-specific help bar keybinding hints
- [x] Models search/filter (`/` key, case-insensitive name + quant match)
- [x] Models delete key (`Del`) + favorites toggle (`f`)
- [x] Build progress bar + last-build summary with error line pinning
- [x] Dashboard build status + RAM display
- [x] 18 unit tests (probe cmake flags, config path resolution, downloads, GGUF parsing)
- [x] Graceful server shutdown (SIGTERM → 5s → SIGKILL)
- [x] Port conflict detection before server start
- [x] BLAS detection (pkg-config openblas)
- [x] Hardware install suggestions ("install with: sudo apt install libvulkan-dev")
- [x] ccache detection ("ccache found — rebuilds will be faster")
- [x] GGUF header inspection — binary metadata parser with architecture detection
- [x] Download resume — `Range` header + `.part` file tracking
- [x] Download ETA display
- [x] `hf://` prefix support for downloads
- [x] Server tok/s parsing — JSON body parsing for `tokens_per_second`
- [x] Build commit hash capture + display on dashboard
- [x] Dashboard real-time RAM usage bar (sysinfo)
- [x] Settings build config section — llama_cpp_path, extra cmake flags, jobs fields
- [x] Build launcher honors build settings — uses configured llama.cpp path, extra CMake flags, and jobs
- [x] Config schema migration — version field + backup on change
- [x] `models.toml` metadata cache struct + load/save
- [x] Config hot-reload polling (`check_config_reload()`)
- [x] Models sort toggle — `s` key cycles Name / Size
- [x] Models disk usage display ("N models — X.XX GB")
- [x] Theme validation (auto/dark/light)
- [x] Build resume prompt — launch opens Build screen with resume/skip prompt if prior build was interrupted
- [x] Server config editing in server screen — arrow/Enter edits port, context, GPU layers, threads, batch
- [x] Quick model swap in server screen (`m` key)
- [x] Dashboard VRAM total display from CUDA probe result
- [x] Server performance panel — shows tok/s, latency, active slots, KV cache when reported
- [x] Dashboard live VRAM usage bar — polls `nvidia-smi` for used/total VRAM on CUDA systems
- [x] Binary symlink step — links built binaries to `~/.lmml/build/bin/`
- [x] `model_card` widget rendering on model detail
- [x] Server model swap restart confirmation — `m` prompts to restart a running server
- [x] Server metrics fallback — queries `/metrics` when `/v1/health` omits slots or KV cache fields
- [x] GPU memory polling cross-vendor baseline — NVIDIA, ROCm, and macOS unified memory
- [x] Dead-code warning gate — future-facing APIs are explicitly allowed at crate level
- [x] Multi-GPU CUDA detection and architecture flags — parses all NVIDIA GPUs and emits `CMAKE_CUDA_ARCHITECTURES`
- [x] Backend selection UI in settings — auto/cpu/cuda/rocm/vulkan/metal backend override
- [x] Toast notifications for async events (download complete, build done, server status, config reload)
- [x] CLI flags (`--model`, `--port`, `--build`, `--diagnose`)
- [x] Advanced model filtering (size ranges, quantization facets)

---

## Phase 1 — Known Gaps (Remaining)

### 🟡 Medium Priority

- [ ] Vulkan-specific GPU heap polling for VRAM-style dashboard telemetry

### 🟢 Minor

- [ ] Replace broad crate-level dead-code allowance with narrower module-level gates as APIs mature

---

## Phase 2 — GPU Backend Expansion (In Progress)

- [x] Apple Metal support (probe + build flags)
- [x] AMD ROCm support (probe + build flags)
- [x] Vulkan backend support (probe + build flags)
- [x] Multi-GPU detection and flag generation
- [x] BLAS backend detection (OpenBLAS baseline)
- [x] Cross-compilation awareness (CUDA arch flags per GPU gen)
- [x] Backend selection UI in settings
- [ ] Benchmark mode: compare tok/s across backends

---

## Phase 3 — Hardening (Remaining)

- [x] Download cancellation/retry UX — interrupted downloads keep `.part` files and surface retry/resume text in the TUI
- [x] Wire config hot-reload polling into app loop — reloads config on tick when the file changes
- [x] Performance metrics persistence (tok/s history) — appends recent health samples to `~/.lmml/metrics.toml`
- [ ] Multiple server instances

---

## Phase 4 — Polish & DX (In Progress)

- [ ] Modal confirmation dialogs (delete model, stop server, cancel build)
- [x] Toast notifications for async events (download complete, build done)
- [x] Advanced model filtering (size ranges, quantization facets)
- [ ] Edit model path / rename in TUI
- [ ] Config presets (gaming, workstation, server)
- [x] CLI flags (`--model`, `--port`, `--build`)
- [x] `~/.lmml/` diagnostic dump for issue reporting
- [ ] Crash recovery: persist and restore scrollback
- [x] Developer toolchain check for missing `rustfmt`/`clippy` components
