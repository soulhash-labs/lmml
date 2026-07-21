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

Current release-readiness claim for local v0.1.0: LAN install works on tested
Linux x86_64 host/target with the generated `dist/` artifacts, SHA256 integrity
checks, hard-prereq doctor gate, installed uninstaller, and clean-install smoke
test. Broader platform readiness still requires target-specific builder or CI
validation.

Current local v0.1.0 hardening state:

- Runtime/TUI correctness blockers are fixed: build cancellation owns subprocess
  groups, explicit backend overrides win over auto-detect, running server model
  swaps require confirmation and restart safely, startup auto-detect/model scan
  is wired, and first-run flow no longer depends on manual refresh.
- Installer/distribution flow is local-LAN ready for the tested Linux x86_64
  path: binary tarball install remains default, source-build bootstrap is
  explicit, `preflight.sh` is mode-aware, clean-install smoke uses the
  documented HTTP path, and release archives are reproducible enough for local
  v0.1.0.
- Signed checksum support exists for future/public release hardening, but real
  minisign release-keypair verification is not a blocker while v0.1.0 remains
  local/LAN-only.

GPU acceleration is primary and first-class. Preflight should fail GPU
acceleration failures by default, while allowing intentional CPU-only nodes only
through an explicit `LMML_GPU_MODE=cpu-only` setting.

ROCm/HIP is now implemented as a conservative v2 source-build path:
`lmml-detect` probes `hipconfig`/`rocminfo`, maps `gfx*` targets, `lmml-build`
emits `-DGGML_HIP=ON -DGPU_TARGETS=...`, and backend settings/tests cover the
mapping. ROCm-specific VRAM telemetry remains open.

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

- [x] v2 ROCm/HIP build support — `hipconfig`/`rocminfo` probe, `gfx*` target mapping, `GGML_HIP` CMake flags, backend settings, and tests.
- [ ] ROCm-specific VRAM telemetry for dashboard/load reporting
- [ ] Vulkan-specific GPU heap polling for VRAM-style dashboard telemetry

### 🟢 Minor

- [ ] Replace broad crate-level dead-code allowance with narrower module-level gates as APIs mature

---

## Phase 2 — GPU Backend Expansion (Historical / Partially Superseded)

- [x] Apple Metal support (probe + build flags)
- [x] AMD ROCm support (probe + build flags) — current v2 support covers conservative source builds; ROCm VRAM telemetry remains open
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
- [ ] Run `tests/integration/clean_install.sh` on validated Ubuntu 24.04 and 26.04 x86_64 CUDA machines before tagging each broadened Debian-family release
- [x] Add signed `SHA256SUMS` verification support before claiming checksum authenticity against tampering

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

---

## Phase 7 — Signed Checksum Release Authenticity (Completed for Local v0.1.0)

Default LAN installs still support unsigned `SHA256SUMS` as integrity-only for
trusted LAN hosts. Production/public release flows should require a signed
`SHA256SUMS.minisig`.

- [x] Add `LMML_CHECKSUM_VERIFY=optional|required|off` installer policy
- [x] Add minisign verification for `SHA256SUMS.minisig` using `LMML_MINISIGN_PUBLIC_KEY` or `LMML_MINISIGN_PUBLIC_KEY_FILE`
- [x] Keep unsigned checksum fallback warning-only for trusted LAN testing
- [x] Make `LMML_CHECKSUM_VERIFY=required` fail clearly when signature, minisign, or public key is missing
- [x] Add `LMML_SIGN_CHECKSUMS=1` packaging hook that signs `SHA256SUMS` with `LMML_MINISIGN_SECRET_KEY_FILE`
- [x] Add installer fixture tests for required signed verification failure and invalid verification mode
- [x] Keep real minisign release-keypair verification as a future/public-release task, not a local v0.1.0 blocker

### Future Public Release Follow-up

- [ ] Before publishing outside the local/LAN environment, generate and verify a real minisign release keypair and publish the public key with release instructions.

---

## Phase 8 — Local v0.1.0 Release Closure (Verification Passed; Archive Pending)

Goal: finish the local-only v0.1.0 release without broadening the readiness
claim beyond the tested LAN target.

- [x] Run the final release gate after Phase 7 changes: `cargo fmt --all -- --check`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`, script fixture tests, package, `lmml doctor`, and clean-install smoke
- [x] Re-run both documented HTTP install modes from `dist/`: default binary install and explicit `INSTALL_MODE=source`
- [x] Confirm `README.md` and `docs/release-checklist.md` still describe local/LAN v0.1.0 honestly, including GPU-primary and CPU-only opt-in semantics
- [x] Decide whether local release uses an ad hoc LAN host URL or a fixed internal host; replace placeholder URLs only if a fixed host is chosen
- [ ] Tag or otherwise archive the local v0.1.0 release once verification passes

Decision: local v0.1.0 continues to use ad hoc LAN host URLs. Keep placeholder
`your-lan-or-github` examples until a fixed internal host is chosen.

Latest local verification: 2026-06-02 on Ubuntu 26.04 Linux x86_64. `cargo fmt`, clippy,
workspace tests, script fixtures, package generation, `lmml doctor`,
`lmml smoke`, default HTTP binary install, and explicit HTTP source install all
passed. Direct terminal `./target/release/lmml doctor` detects CUDA correctly:
`NVIDIA GeForce GTX 1080 Ti` with `sm_61`. Any earlier GPU warning was limited
to the Codex tool sandbox environment, not the host driver/toolkit.

### Deferred Beyond Local v0.1.0

- [ ] Build and validate non-x86_64 target tarballs on matching builders or CI
- [ ] Run CUDA validation on Ubuntu 24.04 and 26.04 x86_64 machines before making broader Debian-family GPU claims
- [ ] Require real signed-checksum verification for any public/non-local release
- [ ] Implement ROCm-specific VRAM telemetry before claiming complete AMD GPU production readiness

---

## Phase 9 — Debian-Family Linux Validation (Planned)

Goal: convert the remaining release-readiness gaps into repeatable validation
jobs for Debian-family Linux before claiming readiness beyond the tested local
Linux x86_64 LAN target. Ubuntu 24.04/26.04 are the first concrete validation
targets. macOS validation is deferred to a later phase.

### 9A — Debian-Family Release Targets

- [ ] Add CI/build matrix entries or documented matching-builder jobs for
  Debian-family Linux targets, starting with Ubuntu 24.04 x86_64, Ubuntu 26.04
  x86_64, Ubuntu 24.04 ARM64, and Ubuntu 26.04 ARM64
- [ ] For each target, run `cargo build --release -p lmml-tui --target <target>`
  on a matching Ubuntu builder or supported Linux cross-build environment
- [ ] Run `scripts/package-release.sh` with `TARGET_TRIPLE=<target>` and confirm
  target-specific tarball, alias tarball, `latest`, and `SHA256SUMS` entries are
  generated
- [ ] Extract each tarball and verify it contains `lmml`, `README.md`, `LICENSE`,
  `RELEASE-METADATA`, `scripts/install.sh`, `scripts/preflight.sh`, and
  `scripts/uninstall.sh`
- [ ] On each matching OS/arch, run installed `lmml doctor` and `lmml smoke`
- [ ] Record per-target validation date, host/runner, target triple, and runtime
  dependency notes in `docs/release-checklist.md`

### 9B — This-Machine Ubuntu 26.04 CUDA Validation

- [x] Use this Ubuntu 26.04 x86_64 machine as the CUDA validation target; do not
  require a separate VM for local v0.1.0 validation
- [x] Confirm this machine has NVIDIA driver, CUDA toolkit, compiler, CMake, Git,
  curl/wget, Rust toolchain, sccache, and at least 4 GB free disk
- [x] Confirm direct host commands pass: `nvidia-smi`, `nvcc --version`,
  `rustc --version`, `cargo --version`, and `rustup show active-toolchain`
- [x] Serve `dist/` from a release host and run default binary install without
  `LMML_GPU_MODE=cpu-only`
- [x] Run `lmml doctor` and require `CUDA available` with GPU name and compute
  capability, not a soft GPU warning
- [x] Run `lmml smoke`
- [x] Run explicit source install from `dist/` without `LMML_GPU_MODE=cpu-only`
  and confirm preflight passes GPU-required mode
- [x] Record the exact GPU, driver version, CUDA toolkit version, and validation
  commands/output summary in `docs/release-checklist.md`

Host precheck evidence from a local validation terminal:

```text
NVIDIA GeForce GTX 1080 Ti, driver 580.159.03, 11264 MiB, compute capability 6.1
CUDA compilation tools 12.4, V12.4.131
rustc 1.96.0
cargo 1.96.0
stable-x86_64-unknown-linux-gnu (default)
sccache 0.13.0+ds-3build1 installed from apt
```

Install validation evidence:

- Served `dist/` with `python3 -m http.server 8127`
- Ran default binary install with `BASE_URL=http://127.0.0.1:8127 INSTALL_MODE=binary tests/integration/clean_install.sh`
- Ran explicit source install with `BASE_URL=http://127.0.0.1:8127 INSTALL_MODE=source tests/integration/clean_install.sh`
- Did not set `LMML_GPU_MODE=cpu-only`
- Both installs reported `CUDA available · NVIDIA GeForce GTX 1080 Ti · sm_61`
- Both installs ran `lmml smoke` and uninstalled cleanly
- Latest rerun: 2026-06-02 on Ubuntu 26.04 x86_64

### 9C — Live llama.cpp CUDA Build and Server Validation

- [x] Install lmml through the packaged LAN installer after the numeric CUDA
  architecture fix
- [x] Run a clean TUI build with CUDA backend selected by auto-detection
- [x] Confirm the resulting server binary path:
  `/home/user/.local/share/lmml/llama.cpp/build/bin/llama-server`
- [x] Start `llama-server` through lmml with a local GGUF model:
  `/home/user/.local/share/lmml/models/Qwen3.5-4B-Q6_K.gguf`
- [x] Confirm server reaches ready state at `http://127.0.0.1:1200`
- [x] Confirm llama.cpp runtime reports CUDA device usage:
  `CUDA0: NVIDIA GeForce GTX 1080 Ti`
- [x] Confirm llama.cpp runtime reports CUDA architecture:
  `CUDA : ARCHS = 610`

Live validation evidence:

```text
Binary: /home/user/.local/share/lmml/llama.cpp/build/bin/llama-server
Model: /home/user/.local/share/lmml/models/Qwen3.5-4B-Q6_K.gguf
Status: Ready { url: "http://127.0.0.1:1200" }
CUDA0: NVIDIA GeForce GTX 1080 Ti (11157 MiB, 10658 MiB free)
system_info: CUDA : ARCHS = 610
```

Latest Orion deep-profile runtime evidence:

```text
Date: 2026-06-03
Model: /home/user/.local/share/lmml/models/Qwen3.5-4B-Q8_0.gguf
Status: Ready { url: "http://127.0.0.1:1200" }
Context: 262144
parallel: 1
KV cache: q8_0 / q8_0
cache_ram: 4096 MiB
OpenCode chunkTimeout: 2400s
OpenCode timeout: 7200s
```

### Phase 9 Acceptance

- [ ] Debian-family x86_64 and ARM64 artifacts are built and smoke-tested on
  matching machines or CI, starting with Ubuntu 24.04/26.04
- [x] This-machine Ubuntu CUDA validation proves GPU-required install flow on
  the actual local release target
- [x] This-machine live llama.cpp CUDA build and server startup are validated
- [ ] Ubuntu 24.04 x86_64 CUDA install validation passes on a second machine
- [ ] README release scope is broadened only to the targets actually validated

---

## Phase 10 — macOS Release Validation (Deferred)

macOS is intentionally out of Phase 9. Validate after Debian-family Linux release
coverage is repeatable.

- [ ] Build and package `x86_64-apple-darwin` on an Intel macOS runner or host
- [ ] Build and package `aarch64-apple-darwin` on an Apple Silicon macOS runner
  or host
- [ ] Verify tarball contents for both macOS targets
- [ ] Install and run `lmml doctor` and `lmml smoke` on matching macOS machines
- [ ] Update README/release scope only for macOS targets that pass validation

---

## Phase 11 — Managed Runtime Harness Integration (Planned)

Goal: make lmml the manager for long-running local llama.cpp runtimes consumed
by coding harnesses. OpenCode is the first target because the local config
already uses OpenAI-compatible HTTP endpoints.

Decision: harnesses should use managed `llama-server` HTTP endpoints, not
`llama-cli`, except for one-shot diagnostics.

Current OpenCode local config:

- path: `~/.config/opencode/opencode.json`
- full provider base URL: `http://127.0.0.1:1200/v1`
- fast provider base URL: `http://127.0.0.1:1200/v1`
- timeout: `7200s`
- chunk timeout: `2400s`
- server context: `262144` tokens
- compaction reserve: `65536` tokens
- usable input before compaction: `196608` tokens
- practical single-agent input target: `120000-170000` tokens
- hard reject/compress threshold: about `196000` tokens
- local model: `llamacpp/Qwen3.5-4B-Q8_0.gguf`
- local fast model: `llamacpp_fast/Qwen3.5-4B-Q8_0.gguf`

Current workstation override:

- The TUI-managed server is the active runtime for OpenCode.
- Keep OpenCode on `http://127.0.0.1:1200/v1` while the TUI Server tab reports
  `Ready { url: "http://127.0.0.1:1200" }`.
- Keep the active OpenCode route on the TUI-managed server at `1200` unless an
  operator explicitly configures a different single active server.
- Current server context: `262144` tokens.
- Current OpenCode `compaction.reserved`: `65536` tokens.
- Current OpenCode local model output limit: `18000` tokens.
- Current OpenCode chunk timeout: `2400s`.
- Usable input before compaction: `196608` tokens.
- Practical single-agent input target before compaction: `120000-170000` tokens.
- Hard reject/compress threshold: about `196000` tokens.
- Current `llama-server` slot policy:
  `extra_args = ["--parallel", "1", "--slot-save-path", "/home/user/.local/share/lmml/llama-slots", "-ctk", "q8_0", "-ctv", "q8_0", "--cache-ram", "4096"]`.
- Expected local model routing:
  `model = "llamacpp/Qwen3.5-4B-Q8_0.gguf"` and
  `small_model = "llamacpp_fast/Qwen3.5-4B-Q8_0.gguf"`.
- Local evidence snapshots should stay outside the public documentation tree
  unless sanitized into generic examples.

- [x] Add runtime profile config schema for harness-managed servers
- [x] Add separate runtime state schema for PID, status, health, and log path
- [x] Add model-specific runtime profile schema so Qwen and Nemotron can carry
  different server settings and chat-template policy
- [x] Apply model-specific server settings when selecting a model in the TUI and
  when building the server config
- [x] Validate Orion Qwen3.5-4B-Q8_0 deep profile at `ctx_size=262144`,
  `parallel=1`, Q8 KV cache, `cache_ram=4096`, and port `1200`
- [x] Remove the shared external Qwen chat template from the active Orion
  profiles; use embedded GGUF templates unless a per-model override is proven
- [x] Update active OpenCode provider timeout policy to `timeout=7200s` and
  `chunkTimeout=2400s`
- [x] Fix stale OpenCode validator routing that still mapped deep/quick lanes to
  old detached-profile lane names
- [x] Route both `opencode` and `opencode-fast` through the active TUI-managed `1200` server by default
- [ ] Allow simultaneous managed `llama-server` instances for full and fast
  profiles
- [ ] Keep each profile isolated: failure of one profile must not stop another
- [x] Add stale PID handling for crashed or externally killed servers in status
  reconciliation and stop
- [ ] Add health polling every `5s` and mark unhealthy after three consecutive
  failures
- [ ] Add `docs/llama-server-integration-contract.md` implementation coverage:
  profile schema fields, `lmml-compat` flag generation, context guard, VRAM
  budget guard, prompt-cache controls, and OpenCode compaction drift detection
- [x] Add initial CLI commands: `lmml runtime status`, `lmml runtime
  print-config opencode`, and `lmml runtime configure opencode`
- [x] Add process CLI commands: `lmml runtime start|stop|logs`
- [x] Add `lmml runtime start <profile> --detach`
- [ ] Add `lmml runtime status --json`
- [ ] Add `lmml runtime validate <profile>` and `lmml runtime health <profile>`
- [ ] Add `lmml runtime restart <profile>` with confirmed cold-restart/model-swap semantics
- [ ] Add profile CLI commands: `profile presets`, `profile list`,
  `profile show`, `profile copy`, `profile validate`, and `profile set`
- [ ] Extend runtime profile schema with `compaction_reserved`, `ubatch_size`,
  `flash_attn`, `continuous_batch`, `split_mode`, `api_key`, KV cache type,
  fit, prompt-cache, slot-save, sampling, MTP, and `mmproj` fields
- [ ] Extend `lmml-compat` to probe and emit `-ctk/-ctv`, `-fit/-fitt/-fitc`,
  `-np`, `-cb`, `--split-mode`, `--cache-prompt`, `--cache-reuse`,
  `--cache-ram`, `--slot-save-path`, multimodal projector flags, and sampling
  flags
- [ ] Add built-in read-only hardware presets and `lmml profile copy`
- [ ] Promote the current Orion `Qwen3.5-4B-Q8_0` 256k profile from manual
  `state.toml` config into a built-in read-only preset
- [ ] Add profile-aware OpenCode configure output so the CLI can emit Q8 model
  limits, 65536 reserve, 2400s chunk timeout, and validator/category guidance
  without hand-editing `~/.config/opencode`
- [ ] Add a safe managed command or documented external helper for patching
  `~/.config/opencode/oh-my-openagent.json` and `validator.ts`; today this is a
  manually maintained local integration
- [ ] Add context validation guard: `ctx_size >= compaction_reserved + 4096`
- [ ] Add effective working-window warning for OpenCode/Sisyphus profiles:
  estimate `ctx_size - compaction_reserved - 20000..24000` and warn when the
  usable window cannot meet the target live prompt size
- [ ] Add VRAM budget validation for single profiles and combined running
  profiles, including tight-margin warnings
- [ ] Add profile presets for both 24GB 131k single-agent mode
  (`ctx_size=131072`, `compaction_reserved=32768`) and 24GB 256k deep-run mode
  (`ctx_size=262144`, `compaction_reserved=65536`)
- [ ] Probe/emit `-ctk q8_0 -ctv q8_0` for supported 131k+ coding profiles
  where KV cache pressure is the limiting factor
- [x] Field-test an 11GB 256k Qwen3.5-4B-Q8 profile:
  `ctx_size=262144`, `compaction_reserved=65536`, `ubatch_size=128`,
  `cache_type_k/v=q8_0`, `cache_ram_mb=4096`, `parallel=1`
- [ ] Add that field-tested 11GB 256k Qwen3.5-4B-Q8 profile as a built-in
  preset and migration target
- [ ] Keep the older experimental 11GB 256k Qwen3.5-4B-Q6 profile available
  only as a fallback/test preset:
  `ctx_size=262144`, `compaction_reserved=65536`, `ubatch_size=128`,
  `cache_type_k/v=q8_0`, `cache_ram_mb=4096`, `parallel=1`
- [ ] Add a 256k fallback profile with `batch_size=256`, `ubatch_size=64`,
  `cache_type_k/v=q4_1`, and `cache_ram_mb=6144`
- [ ] Add doctor opportunity warning when `ctx_size` is below model training
  context, including the note that 256k on <16GB VRAM needs Q8 KV cache and
  `cache_ram_mb >= 4096`
- [ ] Add log guidance for `ggml_cuda_host_alloc` pinned-memory failures during
  `--cache-ram` testing
- [ ] Add fleet profile support with validation status, per-slot context checks,
  LAN host/auth checks, and proposed-vs-validated labels
- [ ] Add Qwen3.5 9B model preset metadata:
  262k native context, 128k minimum thinking context, 9B dense/32 layers,
  multimodal requires matching `mmproj`, MTP is supported but disabled by
  default
- [ ] Add Qwen sampling presets:
  thinking/default `temperature=0.6 top_p=0.95 top_k=20 min_p=0`;
  non-thinking/fast `temperature=0.7 top_p=0.8 top_k=20 min_p=0`
- [ ] Add doctor/profile validation for Qwen multimodal mode: if a Qwen profile
  enables image/video inputs, require the configured `mmproj` file to exist and
  show the exact llama.cpp projector argument that will be used
- [ ] Add installer/source-build regression coverage for CUDA 11.x + GCC 13+
  Ubuntu hosts: preflight should recommend `g++-11`, and CUDA builds should emit
  `-DCMAKE_CUDA_HOST_COMPILER=/usr/bin/g++-11` when that compiler exists
- [x] Make `lmml runtime print-config opencode` print ready-to-paste OpenCode
  JSON
- [ ] Keep `lmml doctor` read-only for OpenCode integration; it may detect
  missing/mismatched lmml providers and recommend commands, but must not patch
  `~/.config/opencode/opencode.json`
- [x] Add `lmml runtime configure opencode --dry-run` to parse OpenCode JSON,
  preserve unrelated keys, and show a structural diff without writing
- [x] Add `lmml runtime configure opencode` as the explicit mutating command:
  create a timestamped backup, patch only lmml-owned keys, validate JSON, and
  print the rollback path
- [x] Add `lmml runtime configure opencode --path <file>` for non-default
  OpenCode config locations
- [x] Add `lmml runtime configure opencode --rollback <backup-file>` for
  reversible config recovery
- [x] Add guarded `--yes`/`--force` behavior: `--yes` may apply clean changes
  non-interactively, while conflicts still require interactive confirmation or
  explicit `--force`
- [x] Make OpenCode configure local-first by default, with
  `--model-source existing|lmml|none` and
  `--small-model-source existing|lmml|none` for explicit routing control
- [x] Allow local-first top-level routing replacement through the normal
  confirmation/`--yes` path; reserve `--force` for conflicting lmml-owned
  provider entries
- [ ] Add an OpenCode Setup Wizard in the TUI that walks users through
  `doctor` status, profile review, config preview, diff review, apply with
  backup, and verification; each mutating step requires explicit confirmation
- [ ] Add TUI Server tab profile selector and per-profile status
- [ ] Generate/copy OpenCode-compatible config snippets
- [ ] Use OpenAI-compatible chat completions as the Phase 11 harness API
  boundary
- [x] Add native Anthropic `/v1/messages` translation in `lmml-node` for
  Claude Code-style clients while keeping raw `llama-server` on OpenAI chat
  completions
- [ ] Make model changes use confirmed cold restart; restore the previous config
  if the new server fails health checks
- [ ] Connect clean install behavior: runtime start must fail clearly when no
  model is configured
- [ ] Add remaining tests for multi-profile port conflicts, restart on model
  change, ongoing health checks, and no orphaned processes
