# lmml — Todo / Roadmap

> Current task tracking for the lmml project.

---

## Current v2 Status

The active implementation is the Rust workspace under `crates/`, with the
user-facing binary built from package `lmml-tui` and installed as `lmml`.

Current CLI surface:

- `lmml` launches the TUI.
- `lmml doctor` runs hard-prerequisite preflight checks and exits non-zero when
  compiler/C++17, CMake, Git, or disk prerequisites fail.
- `lmml smoke` runs the headless clean-install startup check.

Not current v2 CLI surface: `--model`, `--port`, `--build`, and `--diagnose`.
Those entries below are historical root-package plan items and should not be
used in release notes.

Current production-readiness claim for v0.1.0: LAN install works on tested
Linux x86_64 host/target with the generated `dist/` artifacts, SHA256 integrity
checks, hard-prereq doctor gate, installed uninstaller, and clean-install smoke
test. Broader platform readiness still requires target-specific builder or CI
validation.

GPU acceleration is primary and first-class. Preflight should fail GPU
acceleration failures by default, while allowing intentional CPU-only nodes only
through an explicit `LMML_GPU_MODE=cpu-only` setting.

ROCm/HIP is not production-ready in v2. Treat any older completed ROCm rows as
superseded until `lmml-detect`, `lmml-build`, telemetry, settings, and tests
prove the full path.

Vulkan backend selection/build support is current v2 functionality: the detect
crate probes `vulkaninfo --summary`, the build crate emits `-DGGML_VULKAN=ON`,
and TUI/backend persistence tests cover the mapping. Vulkan-specific GPU heap
polling for VRAM-style telemetry remains open.

---

## Historical Root-Package Roadmap

The sections below preserve earlier planning context. Some checked items refer
to the pre-v2/root-package architecture and have been superseded by the current
crate workspace. Prefer the “Current v2 Status” section above when making
release claims.

## Phase 1 — MVP (Historical)

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
- [x] CLI flags (`--model`, `--port`, `--build`, `--diagnose`) — historical root-package item; not present in current v2 binary
- [x] Advanced model filtering (size ranges, quantization facets)

---

## Phase 1 — Known Gaps (Remaining)

### 🟡 Medium Priority

- [ ] v2 ROCm/HIP production support — `hipconfig`/`rocminfo` probe, `gfx*` target mapping, `GGML_HIP` CMake flags, ROCm VRAM telemetry, backend settings, and tests. Supersedes older completed root-package ROCm entries for the v2 crate architecture.
- [ ] Vulkan-specific GPU heap polling for VRAM-style dashboard telemetry

### 🟢 Minor

- [ ] Replace broad crate-level dead-code allowance with narrower module-level gates as APIs mature

---

## Phase 2 — GPU Backend Expansion (Historical / Partially Superseded)

- [x] Apple Metal support (probe + build flags)
- [x] AMD ROCm support (probe + build flags) — historical entry; v2 ROCm production support remains open
- [x] Vulkan backend support (probe + build flags) — current v2 support is covered by `lmml-detect`, `lmml-build`, and `lmml-tui` tests
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
- [x] CLI flags (`--model`, `--port`, `--build`) — historical root-package item; current v2 exposes subcommands, not these flags
- [x] `~/.lmml/` diagnostic dump for issue reporting — historical root-package item; current v2 exposes `doctor` and `smoke`
- [ ] Crash recovery: persist and restore scrollback
- [x] Developer toolchain check for missing `rustfmt`/`clippy` components

---

## Phase 5 — Release Packaging & Distribution (Complete)

- [x] Pre-release baseline commit after `cargo fmt --all -- --check`, `cargo clippy --workspace -- -D warnings`, and `cargo test --workspace`
- [x] Release binary build via `cargo build --release -p lmml-tui`
- [x] Dynamic-link audit via `ldd target/release/lmml`
- [x] Switched HTTP dependencies to rustls TLS to remove OpenSSL/zlib/zstd runtime links
- [x] Documented remaining Linux runtime links (`libgcc_s`, `libm`, `libc`, loader) in `docs/release-checklist.md`
- [x] `lmml doctor` subcommand reuses `lmml-detect::SystemProfile::detect()` and exits non-zero only for hard prerequisite failures
- [x] `lmml smoke` subcommand supports headless clean-install validation
- [x] `scripts/package-release.sh` builds a versioned tarball from `crates/lmml-tui/Cargo.toml`
- [x] `scripts/package-release.sh` writes `dist/latest`, `dist/SHA256SUMS`, full target-triple tarballs, and LAN-friendly alias tarballs
- [x] `scripts/install.sh` detects Linux/macOS x86_64/aarch64 targets, checks hard prerequisites, downloads by `BASE_URL`, verifies SHA256, installs `lmml`, installs `lmml-uninstall`, runs `lmml doctor`, and exits non-zero if doctor reports hard prerequisite failures
- [x] `scripts/uninstall.sh` removes `lmml` and `lmml-uninstall`, then optionally removes config/data directories
- [x] `Makefile` release helpers added (`release`, `dist-serve`, `doctor`, `clean-release`)
- [x] `README.md` updated with one-line install, LAN install, doctor, launch, and uninstall paths
- [x] `tests/integration/clean_install.sh` validates documented HTTP install → doctor → smoke → installed `lmml-uninstall` with an isolated temporary HOME
- [x] LAN-style local install simulation passed from `dist/` over `python3 -m http.server`
- [x] `scripts/package-release.sh` uses sorted GNU tar entries, normalized owner/group/modes/mtimes, `gzip -n`, `RELEASE-METADATA`, and sorted `SHA256SUMS` for repeatable v0.1.0 artifacts when `SOURCE_DATE_EPOCH` is fixed

### Release Packaging Follow-ups

- [ ] Build and publish the remaining cross-target tarballs on matching builders or CI runners: `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin`
- [ ] Replace README placeholder release URL (`https://your-lan-or-github/install.sh`) with the real public release URL when hosting is chosen
- [ ] Run `tests/integration/clean_install.sh` on a clean Ubuntu 24.04 x86_64 VM with CUDA drivers before tagging each release
- [ ] Add signed `SHA256SUMS` verification or HTTPS-hosted public releases before claiming checksum authenticity against tampering

---

## Phase 6 — Preflight and Source-Build Bootstrap (Completed)

Default install remains the verified binary tarball. Source-build install is a
fallback/dev/LAN bootstrap path.

- [x] Add `scripts/preflight.sh` as a Bash-only, read-only default checker
- [x] Make preflight mode-aware with `LMML_INSTALL_MODE=binary|source`
- [x] Make GPU acceleration primary/first-class by default with `LMML_GPU_MODE=required`
- [x] Add explicit intentional CPU-only node mode with `LMML_GPU_MODE=cpu-only`
- [x] Add `INSTALL_MODE=binary|source` to `scripts/install.sh`, rejecting unknown modes clearly
- [x] Keep `INSTALL_MODE=binary` as the default path and avoid requiring Rust for binary install
- [x] Package a checksummed source tarball into `dist/`
- [x] Implement `INSTALL_MODE=source` from the source tarball, not from an unpinned branch
- [x] Include `preflight.sh` in `dist/` and release tarballs
- [x] Add syntax tests for `scripts/preflight.sh`, `scripts/install.sh`, and `scripts/package-release.sh`
- [x] Add preflight fixture tests for binary/source mode, missing Rust, GPU-required failure, and CPU-only pass
- [x] Add clean source-install smoke with isolated `HOME`
- [x] Document exact pipeline syntax for binary, source, auto-fix, GPU-required, and CPU-only flows
