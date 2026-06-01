# AGENTS.md — lmml Development Guide

## Project Overview

lmml is a turnkey TUI application for managing llama.cpp locally. It auto-detects hardware, builds llama.cpp from source with optimal flags, manages GGUF models, and controls the inference server — all from a single terminal interface.

**Stack:** Rust + ratatui + crossterm + tokio  
**Config dir:** `~/.lmml/`  
**Target:** Single self-contained binary, no runtime dependencies.

> **Architecture plan:** See [`docs/plan.md`](docs/plan.md) for the full design document, screen blueprints, and phased delivery roadmap.

---

## Development Setup

```bash
# Prerequisites
rustup update stable
cargo install cargo-watch          # optional: hot-reload during dev

# Build (debug)
cargo build

# Build (release — what users run)
cargo build --release

# Run
cargo run

# Watch mode (auto-rebuild on source change)
cargo watch -x run

# Test specific module
cargo test probe::tests

# Lint
cargo clippy -p lmml -- -D warnings

# Format
cargo fmt
```

### Environment Variables (for development)

| Variable | Purpose |
|----------|---------|
| `LMML_CONFIG_DIR` | Override `~/.lmml/` for testing with isolated configs |
| `LMML_SKIP_PROBE` | Skip hardware probe on launch (faster iteration) |
| `LMML_LOG=debug` | Enable tracing output to stderr (behind TUI) |
| `LMML_FAKE_GPU` | Simulate GPU detection for testing: `cuda|rocm|vulkan|metal` |

---

## Code Style

- Inline format args — use `format!("{x}")` not `format!("{}", x)` ([`uninlined_format_args`](https://rust-lang.github.io/rust-clippy/master/index.html#uninlined_format_args)).
- Collapse nested `if` statements where possible ([`collapsible_if`](https://rust-lang.github.io/rust-clippy/master/index.html#collapsible_if)).
- Prefer method references over closures ([`redundant_closure_for_method_calls`](https://rust-lang.github.io/rust-clippy/master/index.html#redundant_closure_for_method_calls)).
- Make `match` statements exhaustive — avoid wildcard arms.
- Do not create small helper methods that are only referenced once.
- Prefer private modules with explicitly exported public API.
- Use `tracing` for logging throughout, never `println!` or `eprintln!` in application code (except in `main.rs` setup).

---

## Module Architecture

```
src/
├── main.rs          # Entry point, panic hooks, terminal init
├── app/             # Core application state and event loop
│   ├── mod.rs       # App struct, Message enum, update() dispatch
│   ├── config.rs    # ~/.lmml/config.toml serialization
│   ├── state.rs     # Runtime session state
│   └── errors.rs    # Error types with human-friendly Display impls
├── tui/             # Ratatui UI layer
│   ├── mod.rs       # Terminal init/restore, event loop
│   ├── dashboard.rs # System overview, quick actions
│   ├── models.rs    # Model list with search, download
│   ├── server.rs    # Server start/stop, config, live logs
│   ├── build.rs     # Hardware detection display, build progress
│   ├── settings.rs  # User preferences editor
│   ├── helpers.rs   # Shared TUI utilities (centering, colors)
│   └── widgets/     # Reusable UI components
│       ├── mod.rs
│       ├── status_badge.rs  # Green/yellow/red status indicator
│       ├── progress_bar.rs  # Download/build progress
│       ├── log_viewer.rs    # Scrollable log output
│       ├── model_card.rs    # Model detail card
│       └── help_bar.rs      # Footer keybindings
├── probe/           # Hardware detection engine
│   ├── mod.rs       # ProbeResult struct, run_all() orchestrator
│   ├── os.rs        # OS + arch detection
│   ├── cuda.rs      # nvidia-smi, nvcc detection
│   ├── rocm.rs      # hipconfig, ROCm detection
│   ├── vulkan.rs    # vulkaninfo detection
│   ├── metal.rs     # macOS Metal detection
│   ├── cpu.rs       # CPU feature detection (AVX2, NEON, etc.)
│   └── cmake.rs     # ProbeResult → cmake flags + ngl
├── build/           # llama.cpp build pipeline
│   ├── mod.rs       # BuildState, run_build(), cancel_build()
│   ├── clone.rs     # git clone / pull llama.cpp
│   └── compile.rs   # cmake configure + build with streaming
├── models/          # Model management
│   ├── mod.rs       # ModelManager struct
│   ├── local.rs     # Filesystem scan for .gguf files
│   ├── download.rs  # HuggingFace download with progress
│   └── types.rs     # ModelMetadata, Quantization enum
└── server/          # llama-server process management
    ├── mod.rs       # ServerManager: start/stop/restart
    ├── process.rs   # Subprocess lifecycle management
    └── config.rs    # ServerConfig serialization
```

### Module Dependency Flow

```
main.rs
  └── app::App
        ├── tui::*       (renders screens, dispatches user input)
        ├── probe::*     (hardware detection, called by build screen)
        ├── build::*     (build pipeline, called by build screen)
        ├── models::*    (model CRUD, called by models screen)
        └── server::*    (server lifecycle, called by server screen)
```

Modules at the same level **must not** import each other directly. All cross-module coordination flows through `app::App::update()`.

---

## TUI Architecture Pattern

The app follows the **Elm-like architecture** that ratatui is built for:

```
┌──────────────────────────────────────────────────────┐
│                      Event Loop                       │
│                                                       │
│  ┌──────────┐   keystroke    ┌────────┐  new state  ┌──┐
│  │ crossterm │ ───────────► │  App   │ ───────────► │  │
│  │  events   │               │ update │              │  │
│  └──────────┘               │ (mut   │              │  │
│                              │  self) │              │  │
│  ┌──────────┐   tick timer   │        │              │  │
│  │ tokio    │ ───────────►  │        │              │  │
│  │ interval │               └────────┘              │  │
│  └──────────┘                                        │  │
│                                                      │  │
│  ┌──────────────────────────────────────────────┐   │  │
│  │              App::render()                    │   │  │
│  │  match self.current_screen {                  │   │  │
│  │      Screen::Dashboard => dashboard::render(),│   │  │
│  │      Screen::Models => models::render(),      │   │  │
│  │      ...                                      │   │  │
│  │  }                                            │   │  │
│  └──────────────────────────────────────────────┘   │  │
└──────────────────────────────────────────────────────┘  │
```

### Adding a New Screen

1. Create `src/tui/my_screen.rs` with a `pub fn render(...)` and `pub fn handle_event(...)`.
2. Add a variant to the `Screen` enum in `app/mod.rs`.
3. Add the match arm in `App::update()` for messages targeting the screen.
4. Add the match arm in `App::render()` to draw it.
5. Register keybindings in the global help bar widget.

### Screen Structure Convention

Each screen module should expose:

```rust
// Render the screen into the ratatui Frame
pub fn render(area: Rect, state: &AppState, ctx: &ScreenContext, frame: &mut Frame);

// Handle a key event for this screen, return an Action or None
pub fn handle_event(key: KeyEvent, state: &mut AppState) -> Option<Action>;
```

Where `Action` is an enum of things the app core should do (navigate, spawn task, etc.).

---

## Async & Concurrency

### Tokio Runtime
- The app uses a multi-threaded tokio runtime for background operations (builds, downloads, server health checks).
- The TUI event loop runs on the main thread. Heavy work is spawned via `tokio::spawn`.
- Use `tokio::sync::mpsc` channels to stream progress from background tasks back to the TUI.

### Progress Channel Pattern

This is the core communication mechanism between background work and the TUI. Every long-running operation follows this exact pattern:

```rust
// 1. Define events for the operation
enum BuildEvent {
    Line(String),              // a line of cmake output
    Progress { current: u32, total: u32 },  // parsed progress
    Complete(Result<(), BuildError>),
}

// 2. Spawn the task, send events back on a channel
fn start_build(tx: mpsc::Sender<BuildEvent>) {
    tokio::spawn(async move {
        let mut child = tokio::process::Command::new("cmake")
            .args(["--build", "build"])
            .stdout(Stdio::piped())
            .spawn()?;

        let reader = BufReader::new(child.stdout.take().unwrap());
        let mut lines = reader.lines();

        while let Some(line) = lines.next_line().await? {
            tx.send(BuildEvent::Line(line)).await?;
        }

        let status = child.wait().await?;
        if status.success() {
            tx.send(BuildEvent::Complete(Ok(()))).await?;
        } else {
            tx.send(BuildEvent::Complete(Err(BuildError::BuildFailed))).await?;
        }
    });
}

// 3. In the event loop, drain the channel
fn update(&mut self) {
    while let Ok(event) = self.build_rx.try_recv() {
        match event {
            BuildEvent::Line(line) => self.build_state.log_lines.push(line),
            BuildEvent::Complete(result) => self.build_state.status = result.into(),
            // ...
        }
    }
}
```

### Subprocess Management
- All subprocesses (cmake, make, llama-server) are managed via `tokio::process::Command`.
- Never use `std::process::Command` directly — it blocks the event loop.
- Kill subprocesses via `child.kill()` or `child.start_kill()` — not system-level PID hunting.
- Always wait for the child to exit after sending a kill signal.

### Shared State Pattern
```rust
struct App {
    // Owned by the TUI thread, read by render()
    state: Arc<RwLock<AppState>>,
    // Channels from background tasks
    build_rx: mpsc::Receiver<BuildEvent>,
    download_rx: mpsc::Receiver<DownloadEvent>,
    server_rx: mpsc::Receiver<ServerEvent>,
}
```

---

## Error Handling

### Error Types

Define a custom error enum per module:

```rust
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("git clone failed: {0}")]
    CloneFailed(#[source] std::io::Error),

    #[error("cmake configuration failed (exit code: {code})")]
    CmakeFailed { code: Option<i32> },

    #[error("ccache not found — install with: sudo apt install ccache")]
    CcacheMissing,
}
```

### Error Display Rules

1. **Every error must have a human-readable message** that includes a fix suggestion where possible.
2. **Chain errors with `#[source]`** so `color-eyre` can print the full chain in debug mode.
3. **Never suppress errors** — no `.ok()`, no `let _ =`, no unwrap on fallible operations.
4. **Prefer `anyhow::Context`** for annotating errors at the call site:

```rust
use anyhow::Context;

fn detect_cuda() -> anyhow::Result<CudaInfo> {
    let output = std::process::Command::new("nvidia-smi")
        .output()
        .context("Failed to run nvidia-smi — is the NVIDIA driver installed?")?;
    // ...
}
```

### Display in TUI
- Errors from background tasks are sent via channels as `Err(...)` variants and displayed as status badges or modal dialogs in the TUI — never written to stderr directly.

---

## Async Traits

Do **not** use `#[async_trait]` or `#[allow(async_fn_in_trait)]`.

Prefer native RPITIT with explicit `Send` bounds:

```rust
// Trait definition
fn foo(&self) -> impl std::future::Future<Output = T> + Send;

// Implementation (async fn is fine here)
async fn foo(&self) -> T { ... }
```

---

## Documentation

- All public types, traits, and functions **must** have doc comments.
- Doc comments should explain **what** the item does and **why** it exists, not how it works internally.
- Include a usage example in doc comments for non-trivial public APIs.
- When adding or changing a public API, update any relevant documentation alongside the code change.

### Module-level docs

Each module directory must have a `mod.rs` with a doc comment describing:

1. The module's responsibility
2. Its key types (with links)
3. Its relation to other modules

**Example:**
```rust
//! Hardware detection engine for lmml.
//!
//! Probes the system for GPUs, CPU features, and OS capabilities,
//! then maps the results to optimal llama.cpp build flags.
//!
//! The main entry point is [`run_all`], which returns a [`ProbeResult`]
//! containing all detected information and suggested cmake flags.
//!
//! ## Sub-modules
//! - [`cuda`] — NVIDIA GPU + CUDA toolkit detection
//! - [`rocm`] — AMD ROCm/HIP detection
//! - [`cpu`] — CPU feature detection (AVX2, NEON, etc.)
```

---

## Module & File Size

- Target modules under **500 LoC**, excluding tests.
- If a file exceeds ~**800 LoC**, add new functionality in a new module rather than extending the existing file.
- When extracting code into a new module, move the related tests and doc comments with it — keep invariants close to the code that owns them.

---

## Testing

### Unit Tests
- Every module should have unit tests covering its core logic.
- Test probe detection with mock outputs (read from fixture files).
- Test build pipeline flag generation — don't actually compile.
- Test model metadata parsing with sample GGUF headers.

### Integration Tests
- Integration tests go in `tests/` at the crate root.
- Test the full config round-trip (write → read → verify).
- Test server health check against a known endpoint.

### TUI Testing
- Test screen rendering with snapshot tests using `insta`.
- Test `handle_event` logic by feeding key events and asserting state changes.
- Do not test the full ratatui rendering pipeline — test the data preparation (the `render` function's inputs) and the event handling (the `handle_event` function's outputs).

```rust
#[test]
fn test_models_screen_keybindings() {
    let mut state = AppState::default();
    state.models = vec![model_a(), model_b()];

    // Press down arrow — selection moves
    let action = models::handle_event(KeyCode::Down.into(), &mut state);
    assert_eq!(state.selected_model, 1);
    assert!(action.is_none());

    // Press delete — asks for confirmation
    let action = models::handle_event(KeyCode::Char('d').into(), &mut state);
    assert_eq!(action, Some(Action::ShowConfirmDialog("Delete model?".into())));
}
```

### Assertions
- Use `pretty_assertions::assert_eq` for clearer diffs. Import it at the top of each test module that needs it.
- Compare whole objects with `assert_eq!` rather than asserting individual fields one by one.
- Avoid mutating the process environment in tests. Pass environment-derived values as arguments instead.

### Running Tests
```sh
cargo test                         # all tests
cargo test -p lmml                 # this crate
cargo test probe::tests            # specific module
cargo test -- --nocapture          # see stdout/stderr
```

### Snapshot Tests
Any change that affects user-visible output must include updated [`insta`](https://insta.rs) snapshot coverage. Add a new snapshot test if one doesn't exist, or update existing snapshots and include the reviewed `.snap` files in the same commit.

```sh
cargo test                                   # generates *.snap.new files
cargo insta pending-snapshots                # list what's pending
cargo insta accept                           # accept all

# Install if needed:
cargo install cargo-insta
```

---

## Formatting & Linting

Run `cargo fmt` and `cargo clippy` after finishing code changes — no need to ask for approval:

```sh
cargo fmt
cargo clippy -p lmml -- -D warnings
```

Do not re-run tests after formatting or linting.

---

## Patience with Rust Commands

Rust compilation and lock acquisition can be slow. Never attempt to kill a running `cargo` command by PID. Wait for it to complete.

---

## Widget Implementation Guide

Widgets in `src/tui/widgets/` are reusable ratatui components. Each widget should:

1. **Be a plain struct** with the data it needs — no logic, no state.
2. **Implement `Widget` or provide a `render()` function** that takes `Rect` and `Frame`.
3. **Accept styling as parameters** (colors, borders) — don't hardcode.
4. **Handle zero-width/height gracefully** — render nothing instead of panicking.

```rust
// Good:
pub struct StatusBadge {
    pub label: String,
    pub status: Status,   // enum: Ready | Busy | Error
    pub focused: bool,
}

impl StatusBadge {
    pub fn render(&self, area: Rect, frame: &mut Frame) {
        if area.width < self.label.len() as u16 + 4 {
            return; // not enough space, skip
        }
        let color = match self.status {
            Status::Ready => Color::Green,
            Status::Busy => Color::Yellow,
            Status::Error => Color::Red,
        };
        // ...
    }
}
```

Available widgets:
- `status_badge` — Green/yellow/red pill indicator for server status, build status, etc.
- `progress_bar` — Determinate and indeterminate progress with percentage + ETA text.
- `log_viewer` — Scrollable, styled log output with ANSI stripping and line wrapping.
- `model_card` — Structured model detail card showing metadata fields.
- `help_bar` — Bottom-of-screen keybinding legend, auto-hides on small terminals.

---

## Color & Style Convention

| Semantic | Color | Used for |
|----------|-------|----------|
| Success / Ready | Green | Server running, build complete, model loaded |
| Warning / Busy | Yellow | Build in progress, download active, low disk space |
| Error / Critical | Red | Build failed, server crashed, CUDA error |
| Info | Cyan | Log output, status messages |
| Muted | Dark gray | Help text, secondary labels, borders |
| Accent | Magenta | Selected item, focused input field |

Use `helpers.rs` for shared style functions:

```rust
// Good — centralized styles
pub fn badge_style(status: Status) -> Style {
    match status {
        Status::Ready => Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        Status::Busy => Style::default().fg(Color::Yellow),
        Status::Error => Style::default().fg(Color::Red).add_modifier(Modifier::SLOW_BLINK),
    }
}

// Bad — inline magic colors
Style::default().fg(Color::Green) // scattered everywhere
```

---

## Hardware Detection Contract

The probe engine must follow these rules:

1. **Never crash on missing tools.** If `nvidia-smi` is not found, return `CudaInfo::None` with a warning string — don't panic.
2. **Version-aware flag generation.** CUDA < 11.8 cannot target compute capability 8.9+ — flags must reflect this.
3. **Distinguish "not found" from "error".** `not found` = silently skip. `error` = log the error, continue with others.
4. **All results are advisory.** The user can always override cmake flags manually in settings.

### Probe Output Example

```
✓ OS: Linux x86_64 (kernel 6.8.0)
✓ NVIDIA CUDA 12.4 detected (GeForce GTX 1080 Ti, 11 GB VRAM)
○ ROCm: not detected
○ Vulkan: not detected
✓ CPU: AVX2, AVX-512 (partial), 16 cores / 32 threads
✓ RAM: 64 GB

→ Suggested cmake flags: -DGGML_CUDA=ON -DGGML_NATIVE=ON
→ Suggested ngl: 99
```

---

## Build Pipeline Contract

1. **Always verify the build.** After `cmake --build` completes, run `build/bin/llama-cli --version` to confirm the binary works.
2. **Streaming output.** Every line from cmake stdout/stderr must be captured and sent to the TUI via channel — never buffer and dump.
3. **Cancellation safety.** If the user cancels a build mid-way, kill the cmake process group and clean up partial build artifacts.
4. **Idempotent.** Running the build twice should be safe — `cmake` detects no-op changes.

---

## Server Management Contract

1. **Own the subprocess lifecycle.** `ServerManager` holds the `Child` handle. When it drops, the child is killed.
2. **Health check required.** After starting `llama-server`, poll `/v1/health` until it returns 200 or timeout (30s).
3. **Graceful shutdown.** Send SIGTERM, wait 5s for graceful exit, then SIGKILL.
4. **Port conflict detection.** Before starting, check if the port is in use. If so, suggest an alternative.

---

## Config File Format

`~/.lmml/config.toml`:

```toml
[general]
model_dirs = ["~/.lmml/models"]
default_model = ""
theme = "auto"

[build]
llama_cpp_path = "~/.lmml/build/llama.cpp"
extra_cmake_flags = []
jobs = 0

[server]
port = 8080
context_size = 8192
gpu_layers = 99
threads = 0
batch_size = 512
model = ""
extra_args = []
```

`~/.lmml/state.toml` (auto-managed):

```toml
[last_session]
last_model = ""
server_was_running = false
build_state = "not-started"
build_commit = ""
```

- Config is written on first launch if it doesn't exist.
- State is written on every graceful exit and after significant state changes.
- Both use the `toml` crate with `serde` for serialization.

---

## First-Time User Flow

1. `lmml` launched → `~/.lmml/` created with default config and state
2. Probe engine runs automatically → shows hardware detection results on the Build screen
3. If llama.cpp not built → Build screen is the initial view with a "Press `b` to build" prompt
4. After build completes → Dashboard shows "ready" state
5. If no models found → Models screen shows "No models found — press `d` to download from HuggingFace"
6. After model downloaded → Server screen available to start serving

This flow must never dead-end — every screen should offer a clear next action.
