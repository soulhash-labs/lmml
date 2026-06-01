# lmml — Development Learnings

> Architectural decisions, patterns, gotchas, and project state.

---

## 1. Architecture Decisions

### Stack: Rust + ratatui over Go + bubbletea or Python + Textual

**Decision:** Rust.

**Rationale:** Single self-contained binary, no runtime dependencies, excellent async support via tokio, strong typing prevents class bugs in complex state machines. The TUI screens form a state machine where ratatui's immediate-mode rendering pairs naturally with Rust's ownership model.

**Trade-off:** 3-5x slower iteration than Python. Mitigated by fast incremental compilation.

### Module Isolation: Cross-module coordination through `App::update()` only

**Decision:** No module can import another sibling module directly. All coordination flows through `app::App::update()`.

```
tui::*  ───►  app::App::update()  ◄───  probe::*, build::*, models::*, server::*
```

**Why:** Prevents circular dependencies, makes data flow explicit, simplifies testing (each module can be tested with mock state).

**Pattern:** Each background module exposes an event enum (`BuildEvent`, `ProbeEvent`, `ServerEvent`), sends events over an `mpsc::Sender`, and `App::drain_channels()` processes them in the TUI event loop.

### Progress Channel Pattern (established pattern for all background ops)

Every long-running operation follows:
1. Define event enum (`Line`, `Progress`, `Complete(Result<...>)`)
2. Spawn tokio task, send events on `mpsc::Sender`
3. `App::drain_channels()` consumes events on every tick
4. TUI renders from state updated by events

**Key: Never use `std::process::Command`** — it blocks the event loop. Always `tokio::process::Command`.

### Error Types: `thiserror` per-module + `anyhow::Context` for annotations

Each module defines its own error enum with `#[derive(thiserror::Error)]`. Call sites use `anyhow::Context` to annotate with human-readable context. Errors from background tasks are sent as `Err(...)` variants via channels, displayed as TUI status badges — never written to stderr directly.

### Module Sizing: Target < 500 LoC per module, < 800 LoC per file

Extract new modules when files grow beyond 800 lines. Move tests and doc comments with the extracted code.

---

## 2. Ratatui 0.29 Learnings

### `Line::raw()` is removed — use `Line::from()`

In ratatui 0.29, `Line::raw()` was removed. Use `Line::from(Span::raw(...))` or `Line::from(vec![...])`.

### Explicit lifetimes on widget functions

Functions that return `Block` or `Line` with input references need explicit lifetime annotations:

```rust
pub fn panel<'a>(title: Option<&'a str>) -> Block<'a> { ... }
pub fn centered_line<'a>(text: &'a str, width: u16) -> Line<'a> { ... }
```

Without these, the compiler warns "hiding a lifetime that's elided elsewhere is confusing".

### `Constraint::Length` vs `Constraint::Min`

Use `Constraint::Min(1)` for main content areas that should fill remaining space. Use `Constraint::Length(n)` for fixed-height areas. Without a `Min` constraint, layouts produce zero-height sections.

### Render nothing instead of panicking on zero-size areas

Widgets should check `area.width` and `area.height` before rendering. If either is 0, return immediately. This prevents ratatui panics from layout edge cases.

---

## 3. Rust Compilation Gotchas

### `PathBuf` doesn't impl `Display`

```rust
// ❌ Build error
format!("Path: {path}")

// ✅ Correct
format!("Path: {}", path.display())
```

### `Option<T>` doesn't impl `Display`

```rust
// ❌ Build error
format!("Exit code: {code}")  // code: Option<i32>

// ✅ Correct
format!("Exit code: {code:?}")
// or
format!("Exit code: {}", code.unwrap_or(-1))
```

### Module path resolution — sibling modules need `crate::`

In TUI modules (which are under `crate::tui`), referencing `app::config::...` fails because `app` is not a sibling of `tui` in the module tree — it's a sibling crate module. Use `crate::app::config::...`.

### `break` in `loop` that's the function's final expression

If the only `break` in a `loop` that is the last expression of a function, the `break` value `()` must match the function's return type. Fix: add `Ok(())` after the loop so it's not the tail expression.

### Unused variable suppression

Prefix with `_` for single unused bindings. For pattern matches, use `_` (not `_var`) to avoid "unused variable" warnings that clippy still catches.

### `mut` on tokio BufReader not needed

`tokio::io::BufReader::new()` returns a reader that doesn't need `mut` for `.lines()`:

```rust
// ❌ Unnecessary mut
let mut reader = tokio::io::BufReader::new(stdout);

// ✅ Correct
let reader = tokio::io::BufReader::new(stdout);
```

---

## 4. lmml-Specific Patterns

### Hardware detection contract

- **Never crash on missing tools.** If `nvidia-smi` not found, return `CudaInfo::None` with a warning string.
- **Distinguish "not found" from "error"**. `NotFound` = silently skip. `Error(msg)` = log the message, continue with other probes.
- **Version-aware flag generation.** CUDA < 11.8 can't target compute 8.9+. Flags must reflect this.
- **All results are advisory.** User can override cmake flags in settings.

### Build pipeline contract

- **Always verify the build.** After `cmake --build`, run `llama-cli --version`.
- **Streaming output, never buffer.** Every cmake line goes through the channel immediately.
- **Cancellation safety.** Kill cmake process group and clean up partial artifacts on cancel.
- **Idempotent.** Running twice is safe — cmake detects no-op changes.

### Server management contract

- **Own the subprocess lifecycle.** `ServerProcess` holds the `Child` handle. On drop, child is killed.
- **Health check required.** Poll `/v1/health` until 200 or timeout (30s).
- **Graceful shutdown.** SIGTERM → wait 5s → SIGKILL.
- **Port conflict detection.** Check port before starting.

### TUI screen screen convention

Each screen module exposes exactly two public functions:

```rust
pub fn render(area: Rect, app: &App, frame: &mut Frame);
pub fn handle_event(key: KeyEvent, app: &mut App) -> Option<Action>;
```

Where `Action` is an enum of things the app core should do (navigate, spawn task, etc.). This keeps screen logic testable without the full event loop.

---

## 5. Project State

### Current v2 Status

The active product is the crate workspace under `crates/`:

- `lmml-tui` builds the installed `lmml` binary.
- `lmml-detect`, `lmml-build`, `lmml-models`, `lmml-server`,
  `lmml-compat`, and `lmml-state` own the runtime subsystems.
- Current CLI surface is `lmml`, `lmml doctor`, and `lmml smoke`.
- `--model`, `--port`, `--build`, and `--diagnose` were historical
  root-package plan items and are not current v2 CLI flags.
- v0.1.0 release readiness means the tested LAN install flow works for the
  validated host/target: packaged tarball, `SHA256SUMS` integrity check,
  `lmml doctor` hard-prereq gate, `lmml smoke`, and installed
  `lmml-uninstall`.
- LAN HTTP checksums are integrity-only. They are not authenticity protection
  until signed checksums or HTTPS-hosted releases exist.
- ROCm/HIP remains a v2 production gap unless the current crates implement and
  test the complete probe, build, telemetry, settings, and server flow.
- Vulkan backend support is current v2 functionality: `lmml-detect` probes
  `vulkaninfo --summary`, `lmml-build` emits `-DGGML_VULKAN=ON`, and TUI state
  persists/selects the backend. Vulkan-specific GPU heap polling for VRAM-style
  telemetry remains open.

### Historical Phase 1 — MVP (Superseded Root-Package Snapshot)

The table below is retained for project memory. It describes earlier
root-package progress and may not match the current v2 crate workspace. Do not
use it as a release-readiness source without checking the current crates.

| Area | Status |
|------|--------|
| Build | Compiles, 0 errors, 20 pre-existing dead-code warnings |
| Probe | Auto-runs on launch, streams to Dashboard, graceful on missing tools |
| Model scan | Auto-scans `~/.lmml/models/` for `.gguf` files at startup |
| Navigation | 5 screens via Tab/1-5, global keybindings, screen-specific help bar |
| Config | `~/.lmml/config.toml` loaded on startup, edited via Settings screen, saved to disk |
| State | `~/.lmml/state.toml` saved on exit |
| Settings editing | Read/write via Settings screen with in-memory config + disk persistence |
| Build button | `b` key triggers clone → configure → build pipeline with streaming output |
| Download dialog | Modal input accepts `user/model` or `https://...` URLs, progress bar with speed |
| Server start/stop | Space/Enter toggles server via spawned subprocess, auto-restart on crash |
| Server cancel build | `c` key sets `AtomicBool` flag polled in build loop; kills cmake mid-flight |
| Progress bar widget | Used by download overlay and build screen |
| Build last-summary | Shows ✓ succeeded or pinned last-20 error lines on failure |

### Phase 1 Gap Closure Progress

| Batch | Items | Status |
|-------|-------|--------|
| 🔴 Settings save bug + graceful shutdown + port conflict + help bar + models UI + build progress | Settings data loss fixed, graceful shutdown implemented, port conflict wired, help bar screen-specific, search/filter/delete/favorites added, build progress bar + last-summary | ✅ Done (session 2) |
| 🟡 Probe (BLAS, ccache, install suggestions) + Config (schema migration, models.toml, hot-reload) + Download (hf://, ETA, resume) + GGUF header + Server JSON health + Build commit hash + Dashboard RAM bar + Settings build config + Models sort key/disk usage | All implemented | ✅ Done (session 3) |
| 🟢 Resume prompt + Server inline edit/model swap + Dashboard VRAM total | Build interruption state now prompts on launch; Server screen edits core config fields and cycles models with `m`; Dashboard shows CUDA VRAM total | ✅ Done (session 4) |
| 🟢 Server performance + live NVIDIA VRAM + binary symlinks + model card | Server screen has a performance panel; Dashboard polls CUDA VRAM via `nvidia-smi`; successful builds link binaries to `~/.lmml/build/bin`; Models details render through `model_card` | ✅ Done (session 5) |
| 🟢 Restart confirmation + richer metrics + config hot-reload + download retry UX + toolchain check | Server model swap can restart after confirmation; metrics fall back to `/metrics`; config reload runs on ticks; interrupted downloads explain retry/resume; missing rustfmt/clippy warnings appear in build log | ✅ Done (session 6) |
| 🟢 Metrics persistence | Server health samples are capped and persisted to `~/.lmml/metrics.toml` for later history charts/sparklines | ✅ Done (session 7) |
| 🟢 Multi-GPU + backend selection | Historical: CUDA probe parses all NVIDIA GPUs, CMake receives deduplicated architecture flags, and Settings can force auto/cpu/cuda/rocm/vulkan/metal builds. Current v2 now proves Vulkan backend selection/build mapping in crate tests; ROCm remains a v2 gap. | ⚠️ Superseded |
| 🟢 Toast notifications | Async completions and status transitions now show a short non-blocking toast instead of relying only on logs/modals | ✅ Done (session 9) |
| 🟢 CLI startup flags | Historical root-package item. Current v2 CLI exposes `lmml`, `lmml doctor`, and `lmml smoke`, not `--model`, `--port`, or `--build`. | ⚠️ Superseded |
| 🟢 Advanced model filtering | Model search supports plain terms plus `quant:`, `type:`, `size>`, and `size<` filters | ✅ Done (session 11) |
| 🟢 Diagnostic dump | Historical root-package item. Current v2 has `doctor` and `smoke`; no `--diagnose` flag. | ⚠️ Superseded |

### Remaining Low-Priority Items

- Server performance panel (active slots, KV cache) — superseded by session 5 implementation; follow-up is richer endpoint coverage when `/v1/health` omits fields
- Dashboard live VRAM usage bar — superseded by session 5 NVIDIA implementation; follow-up is cross-vendor GPU memory polling
- Binary symlink step — superseded by session 5 implementation
- `model_card` widget rendering — superseded by session 5 implementation
- Richer server metrics endpoints for active slots/KV cache when health omits them — superseded by session 6 `/metrics` fallback
- Cross-vendor GPU memory polling beyond NVIDIA — superseded by session 6 ROCm/macOS baseline; Vulkan heap polling remains separate
- Optional restart flow after model swap — superseded by session 6 confirmation modal
- Developer toolchain check for missing `rustfmt`/`clippy` — superseded by session 6 startup build-log warnings

---

## 6. Development Workflow

```bash
# Build
cargo build

# Run
cargo run

# Format & lint
cargo fmt
cargo clippy -p lmml -- -D warnings    # if clippy available

# Env vars for iteration
LMML_SKIP_PROBE=1 cargo run            # skip hardware probe on launch
LMML_FAKE_GPU=cuda cargo run           # simulate CUDA GPU for testing
LMML_LOG=debug cargo run               # tracing output to stderr

# Watch mode
cargo watch -x run
```

---

## 7. File Index

| File | Purpose |
|------|---------|
| `CLAUDE.md` | Current v2 architecture and implementation contract |
| `docs/lmml-plan.md` | Current lmml v2 design plan |
| `docs/learnings.md` | This file — development learnings and project state |
| `AGENTS.md` | Development guide for AI agents (code style, patterns, contracts) |
| `crates/lmml-tui/src/main.rs` | Binary entry point, CLI subcommands, terminal init |
| `crates/lmml-tui/src/app.rs` | TUI state, actions, key handling, persistent-state coordination |
| `crates/lmml-tui/src/event_loop.rs` | Terminal/background task multiplexing |
| `crates/lmml-tui/src/tabs/` | Ratatui tab rendering and snapshots |
| `crates/lmml-detect/src/lib.rs` | Hardware and hard-prerequisite detection |
| `crates/lmml-build/src/lib.rs` | llama.cpp source/build orchestration |
| `crates/lmml-models/src/lib.rs` | GGUF registry, scanning, metadata, download |
| `crates/lmml-server/src/lib.rs` | llama-server lifecycle |
| `crates/lmml-compat/src/lib.rs` | llama-server capability probing and argv translation |
| `crates/lmml-state/src/lib.rs` | Persistent state under XDG config/data directories |
| `scripts/package-release.sh` | Release artifact packaging and checksums |
| `scripts/install.sh` | LAN/public installer |
| `scripts/uninstall.sh` | Installed uninstaller source |
| `tests/integration/clean_install.sh` | HTTP installer smoke test with isolated HOME |

---

## 8. Session Learnings (2026-06-01)

### Config Lifecycle Bug: Never Create `Config::default()` in Render/Handle Path

**Root cause (3 instances in `tui/settings.rs`, 2 in `tui/server.rs`):**
`config_fields()`, `apply_field()`, and `s` key save handler all called `Config::default()` instead of using the loaded config. The settings screen always showed defaults, every edit started from defaults (wiping custom values), and saving wrote defaults to disk.

**Fix:** Store `Config` in `AppState`, load from disk at startup via `Config::load_or_default()`. All read/write operations go through `app.state.config`.

**Rule:** `Config::default()` should only be called in `load_config()` (for first-launch creation) and tests. Everywhere else must use the loaded config from `AppState`.

### Graceful Shutdown Requires Unsafe (on Unix)

`tokio::process::Child::kill()` sends SIGKILL directly. There's no built-in SIGTERM. Two approaches:

1. **Add `libc` crate** (chosen) — `libc::kill(pid, SIGTERM)` in an `unsafe` block. Simple, standard, well-reviewed.
2. **Call `kill` binary** — fragile, assumes POSIX environment.

`#[cfg(unix)]` guards needed since `libc::SIGTERM`/`SIGKILL` don't exist on Windows.

**Two-phase shutdown pattern:**
```
SIGTERM → 200ms poll loop → 5s deadline → SIGKILL
```
Uses `child.try_wait()` to detect clean exit immediately without waiting the full 5s.

### Port Conflict Detection: `is_port_in_use()` Exists But Never Called

The function in `server/process.rs` was never wired. A TcpStream connection attempt before spawning the server detects port conflicts.

### Cancel Build: `Arc<AtomicBool>` Flag Pattern

Checked between phases (before clone, before cmake) and inside `run_command` via `tokio::select!` polling at 500ms. `Ordering::Relaxed` is sufficient since we only need eventual consistency.

### Search/Filter: Filtered-List ↔ Full-List Index Translation

`selected_model` always indexes into the full `app.state.models` Vec. Rendering maps through a filtered list using `path` as the identity key. Navigation translates via `position(|m| m.path == sel.path)`.

### Server Screen Used `Config::default()` for Config Display

`render_config()` and the start handler both called `Config::default().server` instead of `app.state.config.server`. Settings changes were invisible on the Server screen.

---

## 9. Session Learnings (2026-06-02) — Phase 1 Gap Batch

### Background Agent Timeout Limit

**Problem:** Launched 4 parallel background agents (6-7 items each) for the ~25 remaining Phase 1 gaps. All 4 timed out with zero output.

**Root cause:** Multi-item agent prompts are too long and lack clear boundaries. Agents hit their internal timeout/context limit before producing anything.

**Fix:** Never batch > 1 item per agent prompt. For this project, all remaining work was done inline.

**Rule:** Background agents are for single-file or single-scope searches only. Implementation must be done in-line or via single-item task delegation.

### GGUF Binary Format Parsing

GGUF metadata is stored in a binary header with this layout:
```
[4 bytes] magic: "GGUF"
[4 bytes] version: u32 LE
[8 bytes] tensor_count: u64 LE
[8 bytes] metadata_count: u64 LE
--- for each metadata KV pair:
  [8 bytes] key_length: u64 LE
  [N bytes] key string
  [4 bytes] value_type: u32 LE
  [N bytes] value (type-dependent)
```

Value types: 0=bool, 1=int8, 2=int16, 3=int32, 4=int64, 5=float32, 6=float64, 8=string.

**Key insight:** `general.architecture` is the most useful metadata key — gives the actual model architecture (llama, bert, gptneox, etc.) instead of guessing from filenames.

### Download Resume via Range Header

Pattern: Check if `{filename}.part` exists → get its size → send `Range: bytes={size}-` header → use `OpenOptions::new().append(true)` to continue writing.

**Handling Content-Range response:**
```
Content-Range: bytes 100-999/2000
```
Parse the total after `/` to get full file size. Falls back to `content_length() + existing_size` if Content-Range is missing.

### Config Schema Migration

Add a `version: u32` field to Config struct. In `load_config()`, compare against `CONFIG_VERSION` constant. On mismatch:
1. Copy existing config to `config.toml.bak`
2. Call `migrate_config(&mut config, from_version)`
3. Set `config.version = CONFIG_VERSION`
4. Save

Migration functions are cumulative — each handles upgrades from `N` to `N+1`.

**Borrow checker gotcha:** Cannot read `config.version` after taking `&mut config`:
```rust
// ❌ Compiler error: cannot use config.version because it was mutably borrowed
migrate_config(&mut config, config.version);
// ✅ Fix: capture version first
let current_version = config.version;
migrate_config(&mut config, current_version);
```

### Models Sort: Sorting by Enum Variant

Sort toggle cycles through `ModelsSort` variants:
```rust
pub enum ModelsSort { Name, Size }
```

Sort function uses a tuple for stable ordering:
```rust
result.sort_by_key(|m| {
    let fav = if m.is_favorite { 0u8 } else { 1u8 };
    let sort = match sort_by {
        ModelsSort::Name => m.name.clone(),
        ModelsSort::Size => format!("{:020}", m.size_bytes),
    };
    (fav, sort)  // favorites always first, then by sort key
});
```

Zero-padded string formatting (`{:020}`) is used for numeric sorting of sizes to avoid parsing back-and-forth.

### `serde_json` Dependency Addition

Added `serde_json = "1"` to Cargo.toml for server health endpoint JSON body parsing. The `/v1/health` endpoint may return either `tokens_per_second` or `completion_tokens_per_second` depending on llama.cpp version:

```rust
serde_json::from_str::<serde_json::Value>(&body).ok()
    .and_then(|v| {
        v.get("tokens_per_second")
            .or_else(|| v.get("completion_tokens_per_second"))
            .or_else(|| v.get("predictions_per_sec"))
            .and_then(|t| t.as_f64())
    })
    .unwrap_or(0.0);
```

### Sysinfo RAM Bar Pattern

```rust
let mut sys = sysinfo::System::new_all();
sys.refresh_memory();
let used_mb = sys.used_memory() / (1024 * 1024);
let total_mb = sys.total_memory() / (1024 * 1024);
```

`sysinfo` is already in dependencies — no new crate needed for the RAM bar.

### BLAS probe via pkg-config

`pkg-config --exists openblas` checks for OpenBLAS. `pkg-config --modversion openblas` gets version. Intel MKL fallback: `mkl-static-lp64`.

The probe is best-effort and never blocks the detection pipeline — if `pkg-config` is not installed, BLAS shows as NotFound silently.

### Test Failure on Struct Field Addition

Adding a new field to `ProbeResult` (e.g., `blas: BlasProbe`) breaks test fixtures that construct it with struct literal syntax. Every `ProbeResult { ... }` in tests must include the new field or use `..Default::default()`. The cmake.rs test helper `minimal_result()` needed explicit `blas: BlasProbe::NotFound`.

---

## 10. Session Learnings (2026-06-01) — Release Packaging

### Package Name vs Binary Name Must Stay Distinct

The release command is `cargo build --release -p lmml-tui`, but the installed executable must be `lmml`. Keep the package and binary names separate:

```toml
[package]
name = "lmml-tui"

[[bin]]
name = "lmml"
```

This lets workspace commands target the crate unambiguously while preserving the user-facing command name.

### LAN Installer Environment Variables in Pipelines

This shell pattern does not pass `BASE_URL` to the installer process:

```sh
BASE_URL=http://host:8000 curl -fsSL http://host:8000/install.sh | sh
```

It only sets `BASE_URL` for `curl`. Use one of these instead:

```sh
curl -fsSL http://host:8000/install.sh | BASE_URL=http://host:8000 sh
export BASE_URL=http://host:8000
curl -fsSL http://host:8000/install.sh | sh
```

README and release docs should use the first form so copy/paste installs fetch tarballs from the LAN server.

### rustls Removes OpenSSL Runtime Packaging Risk

Default `reqwest` features pulled in OpenSSL, zlib, and zstd on Linux release builds. The release binary then depended on host shared libraries that may not exist on a clean target machine.

Fix:

```toml
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls"] }
```

For downloads/searches, add `json` and `stream` features as needed. After switching to rustls, `ldd target/release/lmml` showed only `libgcc_s`, `libm`, `libc`, and the loader on the build host.

### SHA256SUMS Must Support Multiple Tarballs

Release packaging may run once per target triple. `SHA256SUMS` must update the current tarball entry without deleting entries for other targets. Pattern:

1. Remove any existing line for the tarball name.
2. Append the new checksum line.
3. Keep both full target-triple names and LAN-friendly alias names when both files exist.

This allows `dist/` to serve both `lmml-0.1.0-x86_64-unknown-linux-gnu.tar.gz` and `lmml-0.1.0-x86_64-linux.tar.gz` with the same checksum.

### Install `lmml-uninstall` with the Binary

The tarball includes `scripts/uninstall.sh`, but users should not need the source checkout after install. The installer copies that script to `$install_dir/lmml-uninstall` next to `$install_dir/lmml`. The uninstaller removes both files.

### Doctor Is Hard-Prereq Gate, Not GPU Gate

`lmml doctor` exits `1` only when hard prerequisites fail: compiler/C++17, cmake, git, or disk space. CUDA/GPU absence is a soft warning because CPU-only mode is valid. This keeps `curl | sh` installs successful on CPU-only machines while still surfacing the acceleration gap clearly.

Update after installer hardening: `scripts/install.sh` installs the binary, then
runs the installed `lmml doctor` and exits non-zero if doctor reports hard
prereq failures. CPU-only systems still pass when the hard prerequisites are
present.

### Smoke Mode Is for Headless Install Validation

The TUI cannot be launched safely from a non-interactive clean-install script. `lmml smoke` loads/creates state, runs the same detection path, checks hard prerequisites, prints a short success line, and exits without entering alternate-screen terminal mode.

Clean-machine validation flow:

```sh
curl -fsSL http://host:8000/install.sh | BASE_URL=http://host:8000 sh
lmml doctor
timeout 5s lmml smoke
lmml-uninstall
```

### Validate Installers with Temporary HOME

Installer tests should avoid clobbering the developer's real `~/.local/bin` and config. The clean-install script exports:

```sh
HOME=$(mktemp -d)
XDG_CONFIG_HOME=$HOME/.config
XDG_DATA_HOME=$HOME/.local/share
PATH=$HOME/.local/bin:$PATH
```

This proves the default install path and state creation while leaving the real home directory untouched.

The current clean-install script fetches `install.sh` from `BASE_URL` over HTTP
and invokes the installed `lmml-uninstall`, so it tests the documented LAN path
rather than a checkout-local installer script.

### Reproducible Packaging Baseline

`scripts/package-release.sh` now requires GNU tar and normalizes release
archives with sorted entries, numeric owner/group `0:0`, fixed mtimes from
`SOURCE_DATE_EPOCH`, normalized file modes, `gzip -n`, sorted `SHA256SUMS`, and
`RELEASE-METADATA` containing version, target, git commit, rustc version, and
source epoch.

With the same `SOURCE_DATE_EPOCH` and build inputs, two package runs should
produce the same tarball checksum.

### Localhost Integration Tests Need Sandbox Escalation

Workspace tests and LAN install simulation bind local ports for mock HTTP servers and `python3 -m http.server`. In this sandbox, those fail with `Operation not permitted` unless rerun with network permission. Treat a non-escalated bind failure as an environment restriction, then rerun the same command with the appropriate permission instead of changing test code.

---

## 11. Session Learnings (2026-06-01) — Preflight and GPU Readiness

### GPU Is Primary, CPU-Only Is Explicit

lmml should treat GPU acceleration as primary and first-class in preflight. A
missing/broken GPU stack should fail default preflight because it contradicts
the normal target experience.

CPU-only operation is still valid for intentional CPU-only nodes, but it should
be explicit:

```sh
LMML_GPU_MODE=cpu-only
```

This keeps accidental driver/toolkit failures visible while allowing deliberate
CPU-only deployments.

### `lmml doctor` and `preflight.sh` Have Different Jobs

Shell preflight catches what must be true before `lmml` exists: platform,
download tool, compiler, CMake, Git, disk, Rust for source mode, and GPU
readiness policy.

Installed `lmml doctor` remains authoritative after install. It should continue
to return non-zero only for hard prerequisites, while surfacing GPU acceleration
problems clearly.

### Preserve GPU Probe Errors

When `nvidia-smi` exists but cannot communicate with the NVIDIA driver, reporting
"GPU not detected" is misleading. Preserve the actual `nvidia-smi` stderr and
show it in `lmml doctor`.

Observed host state during verification:

```text
nvcc present
nvidia-smi failed because it could not communicate with the NVIDIA driver
```

That is a driver/runtime problem, not a CUDA toolkit or lmml build problem.

### Binary Install Must Not Require Rust

The default install path is a verified binary tarball. Rust/Cargo/rustup are
hard prerequisites only for explicit source-build bootstrap mode.

### Source Build Requires an Authenticated Source Story

For LAN reliability, source mode should build from a source tarball served from
`dist/` and covered by `SHA256SUMS`. Avoid cloning an unpinned branch as a
production path.

### Auto-Fix Must Stay Narrow and Opt-In

`LMML_FIX_DEPS=1` can cover narrow apt packages such as compiler, CMake, Git,
curl, and sccache. It should not automatically install Rust, NVIDIA drivers,
CUDA, or broad GPU stacks.

### Pipeline Environment Placement Still Matters

For piped installers, environment variables must be attached to the shell
process, not the `curl` process:

```sh
curl -fsSL http://host:8000/preflight.sh | LMML_INSTALL_MODE=source bash
curl -fsSL http://host:8000/install.sh | BASE_URL=http://host:8000 INSTALL_MODE=source bash
```

### Rust Proxies Are Not Enough for Source Mode

`rustc`, `cargo`, and `rustup` can exist on `PATH` as rustup proxies while still
being unusable if no active toolchain is visible under the current
`RUSTUP_HOME`/`HOME`. Source-mode preflight must run the version/toolchain
commands and fail if they cannot execute, not just check `command -v`.

Clean-install source smokes should isolate lmml config/data with a temporary
`HOME`, but preserve `CARGO_HOME` and `RUSTUP_HOME` from the caller so the test
uses the developer toolchain instead of pretending the source host has no Rust
toolchain configured.

### Signed Checksums Need an Explicit Policy

Unsigned `SHA256SUMS` is useful for corruption detection on a trusted LAN, but
not for authenticity. The installer needs a policy knob:

```sh
LMML_CHECKSUM_VERIFY=optional|required|off
```

Use `required` for public or untrusted-network installs with a configured
minisign public key. Keep `optional` as the LAN/dev default so existing trusted
LAN bootstrap flows still work while warning when no signature verification is
performed.

For local-only v0.1.0, real minisign keypair verification is not a release
blocker. It becomes mandatory when lmml is published outside the trusted
local/LAN release environment.

---

## 12. Session Summary (2026-06-01) — What We Did

Release hardening moved from review findings to implemented fixes:

- Fixed runtime/TUI truthfulness issues: full build process-group cancellation,
  explicit backend override semantics, safe running-server model swap, and
  startup detect/model scan dispatch.
- Hardened release install paths: default binary install remains the primary
  path, source-build install is explicit, preflight is mode-aware, and CPU-only
  nodes must opt in with `LMML_GPU_MODE=cpu-only`.
- Preserved real GPU probe failure messages in `lmml doctor`, so driver/runtime
  failures are visible instead of being collapsed into generic "GPU not found"
  text.
- Made packaging more reproducible and local-LAN complete: deterministic tarball
  settings, `RELEASE-METADATA`, source tarball, `preflight.sh`, and sorted
  `SHA256SUMS`.
- Added optional signed-checksum support for future/public release needs while
  keeping local v0.1.0 unblocked by real keypair management.

Key lesson: keep local v0.1.0 claims narrow. The tested claim is local/LAN
Linux x86_64 install readiness, not broad production readiness across every
platform, GPU stack, or public distribution threat model.

### Phase 8 Verification Result

The final local v0.1.0 verification pass succeeded on 2026-06-01 for the tested
Linux x86_64 host:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace -- -D warnings`
- `cargo test --workspace` with localhost bind permission
- script fixture tests for syntax, preflight, and signed checksums
- `scripts/package-release.sh`
- `./target/release/lmml doctor`
- `./target/release/lmml smoke`
- HTTP binary clean install from `dist/`
- HTTP source-build clean install from `dist/`

Direct terminal verification of `./target/release/lmml doctor` reports CUDA
available on the host:

```text
NVIDIA GeForce GTX 1080 Ti · sm_61
```

An earlier `nvidia-smi` failure occurred only inside the Codex tool execution
environment, so it should be treated as sandbox/device visibility rather than a
host driver, CUDA toolkit, or lmml probe failure.

The release still should not be described as broadly production-ready. Remaining
non-local work includes cross-target builders, this-machine CUDA validation, real
public-release signing, and v2 ROCm/HIP production support.

### Cross-Target and CUDA Validation Must Be Evidence-Based

Building a tarball name is not the same as supporting a target. Each
Debian-family artifact needs a matching-machine install and `lmml doctor`/`lmml
smoke` result before it can be advertised. Ubuntu 24.04/26.04 are the first
concrete Debian-family targets; macOS validation is deliberately deferred until
the Linux matrix is repeatable.

The this-machine CUDA validation must run without `LMML_GPU_MODE=cpu-only`. The point
of that pass is proving that default GPU-required preflight and installed
`lmml doctor` both see CUDA correctly on the actual Ubuntu 24.04 local release
target.

That validation passed for local v0.1.0 on 2026-06-01: both binary and source
HTTP installs from `dist/` ran in default GPU-required mode and reported CUDA
available on the NVIDIA GeForce GTX 1080 Ti.
