# lmml — Local LLM Manager

> A turnkey TUI app for managing llama.cpp: auto-detect your hardware, build from source, manage models, and run the inference server — all from one terminal.

---

## 1. Philosophy & Niceties

This tool is built for **the person who wants local LLMs to Just Work™** without wrangling cmake flags, CUDA paths, and server configs by hand. Every design decision flows from that.

**Niceties baked in from day one:**

- **No silent failures.** If `nvidia-smi` isn't found, you see `✗ NVIDIA CUDA not detected` — not a crash, not a lie. The TUI shows green/yellow/red status badges so you know what's working at a glance.
- **Graceful degradation.** No GPU? We build CPU-only and move on. Partial CUDA install? We tell you what's missing. You never hit a wall — you hit a helpful message.
- **Build progress you can watch.** When llama.cpp compiles, you see real-time cmake output streaming into the TUI. Not a spinner. Not a black box. Actual progress, with the last error visible if it fails.
- **Sensible defaults, but everything tweakable.** We pick `-ngl 99` for your GTX 1080 Ti out of the box. But every knob is in the config file and TUI settings pane.
- **Stateful.** Close the TUI and reopen — your last-used model, server config, and build state are right where you left them. No re-typing flags.
- **Human error messages.** `"llama-server failed to start — port 8080 is already in use"` instead of `"Error: EADDRINUSE"`. `"No .gguf models found — want to download one?"` instead of a blank screen.
- **Clean exits.** Ctrl+C stops the server gracefully, saves state, restores your terminal. No orphaned processes, no garbled scrollback.

---

## 2. High-Level Architecture

```
┌────────────────────────────────────────────────────────────┐
│                     TERMINAL USER INTERFACE                  │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌─────────────┐ │
│  │Dashboard │  │ Models   │  │ Server   │  │ Settings     │ │
│  │ at-a-gl │  │ manager  │  │ control  │  │ + Build Log  │ │
│  │ -ance   │  │          │  │          │  │              │ │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘  └──────┬──────┘ │
│       │              │              │               │        │
│       └──────────────┴──────────────┴───────────────┘        │
│                          │  ratatui + crossterm               │
├──────────────────────────┼───────────────────────────────────┤
│                     APP CORE (tokio async)                    │
│                          │                                    │
│  ┌──────────────────────────────────────────────────────┐    │
│  │                    Dispatcher                         │    │
│  │  Routes events from TUI → backend, streams progress   │    │
│  │  back to TUI. Single-threaded UI, multi-threaded ops. │    │
│  └──┬──────┬──────┬──────┬──────┬───────────────────────┘    │
│     │      │      │      │      │                            │
│     ▼      ▼      ▼      ▼      ▼                            │
│  ┌────┐ ┌────┐ ┌────┐ ┌────┐ ┌──────┐                      │
│  │Probe│ │Build│ │Model│ │Srvr │ │Config│                     │
│  │Engine││Pipe-│ │Mgr  │ │Mgr  │ │(.toml)│                   │
│  │     │ │line │ │     │ │     │ │      │                     │
│  └──┬──┘ └──┬──┘ └──┬─┘ └──┬──┘ └──────┘                     │
│     │       │       │       │                                 │
├─────┼───────┼───────┼───────┼────────────────────────────────┤
│     ▼       ▼       ▼       ▼                                 │
│  ┌──────────────────────────────────────────────────────┐    │
│  │              SYSTEM LAYER (std::process)              │    │
│  │  git, cmake, make, nvidia-smi, vulkaninfo, ldconfig  │    │
│  └──────────────────────────────────────────────────────┘    │
│  ┌──────────────────────────────────────────────────────┐    │
│  │              STORAGE LAYER (~/.lmml/)                 │    │
│  │  config.toml  │  state.toml  │  models.toml  │ build/│   │
│  └──────────────────────────────────────────────────────┘    │
└──────────────────────────────────────────────────────────────┘
```

### Data Flow

```
User Input (keyboard)
    │
    ▼
ratatui event loop ──► App::update(msg)
                            │
                            ▼
                    ┌─── Matches message ───┐
                    │                       │
            ┌───────┴───────┐       ┌───────┴───────┐
            │  UI action     │       │  Backend op   │
            │ (navigate,     │       │ (build,       │
            │  scroll,       │       │  download,    │
            │  select)       │       │  start server)│
            └───────┬───────┘       └───────┬───────┘
                    │                       │
                    ▼                       ▼
            ratatui redraw           tokio::spawn(task)
                                         │
                                         ▼
                                  Progress channel ──► UI update
                                         │
                                         ▼
                                  Complete / Error ──► Status badge
```

---

## 3. Module Tree

```
lmml/
├── Cargo.toml
├── docs/
│   └── plan.md                          ← you are here
├── src/
│   ├── main.rs                          # Entry point: panic hooks, init, run TUI
│   │
│   ├── app/
│   │   ├── mod.rs                       # App struct, Message enum, update()
│   │   ├── config.rs                    # Config struct, load/save ~/.lmml/config.toml
│   │   ├── state.rs                     # Runtime state (selected model, server pid, etc.)
│   │   └── errors.rs                    # Error types with human-friendly Display impls
│   │
│   ├── tui/
│   │   ├── mod.rs                       # Tui struct: terminal init/restore, event loop
│   │   ├── dashboard.rs                 # Dashboard screen (system overview, quick actions)
│   │   ├── models.rs                    # Model list screen (search, sort, download)
│   │   ├── server.rs                    # Server control screen (start/stop, config, logs)
│   │   ├── build.rs                     # Build screen (detected hw, cmake flags, build log)
│   │   ├── settings.rs                  # Settings screen (paths, defaults, theme)
│   │   ├── widgets/
│   │   │   ├── mod.rs
│   │   │   ├── status_badge.rs          # Green/yellow/red status indicator
│   │   │   ├── progress_bar.rs          # Download/build progress bar widget
│   │   │   ├── log_viewer.rs            # Scrollable log output widget
│   │   │   ├── model_card.rs            # Model detail card (size, quant, params)
│   │   │   └── help_bar.rs              # Footer with keybindings
│   │   └── helpers.rs                   # Shared TUI utilities
│   │
│   ├── probe/
│   │   ├── mod.rs                       # ProbeResult struct, run_all()
│   │   ├── os.rs                        # OS + arch detection
│   │   ├── cuda.rs                      # nvidia-smi, nvcc detection, CUDA version
│   │   ├── rocm.rs                      # hipconfig, ROCm detection
│   │   ├── vulkan.rs                    # vulkaninfo, Vulkan SDK detection
│   │   ├── metal.rs                     # macOS Metal detection
│   │   ├── cpu.rs                       # CPU features (AVX2, AVX512, AMX, NEON)
│   │   └── cmake.rs                     # Map ProbeResult → cmake flags + ngl
│   │
│   ├── build/
│   │   ├── mod.rs                       # BuildState, run_build(), cancel_build()
│   │   ├── clone.rs                     # git clone / pull llama.cpp
│   │   └── compile.rs                   # cmake configure + build with streaming output
│   │
│   ├── models/
│   │   ├── mod.rs                       # ModelManager struct, scan(), download()
│   │   ├── local.rs                     # Scan filesystem for .gguf files, parse metadata
│   │   ├── download.rs                  # HuggingFace download with progress reporting
│   │   └── types.rs                     # ModelMetadata, Quantization enum, Source enum
│   │
│   └── server/
│       ├── mod.rs                       # ServerManager: start/stop/restart, health check
│       ├── process.rs                   # llma-server subprocess management
│       └── config.rs                    # ServerConfig (port, ctx, ngl, threads, etc.)
│
└── examples/                           # (future)
```

---

## 4. Component Details

### 4.1 Probe Engine (`src/probe/`)

**Purpose:** Answer one question: *"What hardware am I on, and what's the optimal way to build llama.cpp for it?"*

**Detection Matrix:**

| Backend | How we detect | Key indicators | Fallback |
|---------|--------------|----------------|----------|
| **CUDA** | `nvidia-smi` + `nvcc --version` | GPU model, VRAM, CUDA version → `-DGGML_CUDA=ON` | CPU-only |
| **ROCm** | `hipconfig --full` | GPU target (gfx1030, etc.) → `-DGGML_HIP=ON -DGPU_TARGETS=...` | CPU-only |
| **Vulkan** | `vulkaninfo` | Vulkan SDK available → `-DGGML_VULKAN=ON` | CPU-only |
| **Metal** | `sw_vers -productVersion` (macOS) | macOS → `-DGGML_METAL=ON` | CPU-only |
| **CPU** | `/proc/cpuinfo` or equivalent | AVX2, AVX-512, NEON, AMX → add `-DGGML_NATIVE=ON` | Generic |
| **BLAS** | `pkg-config --exists openblas` | OpenBLAS/MKL found → `-DGGML_BLAS=ON` | None |

**Output:**
```rust
struct ProbeResult {
    os: Os,
    arch: Arch,
    cuda: Option<CudaInfo>,       // None if not found
    rocm: Option<RocmInfo>,
    vulkan: bool,
    metal: bool,
    cpu_features: CpuFeatures,    // avx2, avx512, neon, etc.
    suggested_cmake_flags: Vec<String>,
    suggested_ngl: u32,           // 0 for CPU, 99 for most GPUs
    ram_gb: u32,
    vram_gb: Option<u32>,         // None for CPU-only
    warnings: Vec<String>,        // "CUDA found but VRAM < 4GB"
}
```

**Niceties:**
- Each check logs a human-readable line: `✓ CUDA 12.4 detected (GeForce GTX 1080 Ti, 11 GB VRAM)`
- Partial installs get helpful suggestions: `✗ Vulkan not found — install with: sudo apt install libvulkan-dev`
- Warnings about known issues: `⚠ CUDA 11.7 detected — older version, expect slower compilation`

---

### 4.2 Build Pipeline (`src/build/`)

**Purpose:** Get llama.cpp cloned, configured, and compiled with zero user friction.

**Flow:**
```
1. Check ~/.lmml/build/llama.cpp exists → git pull, else git clone
2. mkdir build dir
3. cmake -B build <flags from ProbeEngine>
4. cmake --build build --config Release -j $(nproc)
5. Verify: build/bin/llama-cli --version
6. Symlink binaries to ~/.lmml/build/bin/
```

**Niceties:**
- **Streaming build output**: Every cmake line shows in the TUI log viewer. If it fails, the last 20 lines are pinned.
- **Build caching**: Detects ccache automatically. Shows `✓ ccache found — rebuilds will be faster`.
- **Incremental rebuilds**: If you rebuild after a config change, only changed files recompile.
- **Resume on reboot**: Build state persists. If you closed the TUI mid-build, next launch asks: *"Build was in progress — resume?"*
- **Cancellation**: Ctrl+C mid-build kills cmake gracefully, cleans partial artifacts.

---

### 4.3 Model Manager (`src/models/`)

**Purpose:** Find, browse, download, and organize GGUF models.

**Local model scanning:**
- Recursively scan configured directories (default: `~/.lmml/models/` + any user paths)
- Parse GGUF metadata via `llama.cpp/scripts/gguf-parser` or header inspection
- Extract: model name, architecture, parameter count, quantization, file size

**Model metadata (displayed in TUI):**

```
┌─ Llama-3.1-8B-Instruct-Q4_K_M.gguf ──────────────────┐
│                                                        │
│  Architecture : llama                                  │
│  Parameters   : 8.03B                                  │
│  Quantization : Q4_K_M                                 │
│  File size    : 4.92 GB                                │
│  Source       : HuggingFace meta-llama/Llama-3.1-8B   │
│  Last used    : 2026-05-31 14:22                       │
│  Status       : ✓ Loaded                               │
│                                                        │
│  [Load]  [Delete]  [Set as default]                    │
└────────────────────────────────────────────────────────┘
```

**HuggingFace download:**
- Use `reqwest` to download GGUF files with progress streaming
- Display ETA, speed, and percentage in a real-time progress bar
- Support for `hf://user/model:quant` syntax
- Resume interrupted downloads

**Niceties:**
- **Download queue**: Queue multiple models, they download sequentially with a progress overview
- **Favorites / pinning**: Star your frequently-used models, they float to the top
- **Smart sorting**: Sort by last-used, size, param count, quantization level
- **Search**: Fuzzy-find across model names as you type
- **Disk usage warning**: Shows `▰▰▰▰▰▰▰▰▱▱ 72% (142 GB / 200 GB used)` before you start a 10GB download
- **Duplicate detection**: Same model already in a different dir? Warn before downloading again

---

### 4.4 Server Manager (`src/server/`)

**Purpose:** Launch, configure, and monitor `llama-server` as a managed subprocess.

**Server Config (persisted to `~/.lmml/config.toml`):**
```toml
[server]
port = 8080
context_size = 8192
gpu_layers = 99
threads = 8
batch_size = 512
model = "Llama-3.1-8B-Instruct-Q4_K_M.gguf"
extra_args = []
```

**TUI Server Screen:**
```
┌─ Server Control ───────────────────────────────────────┐
│                                                        │
│  Status: ● Running  (pid: 12453, uptime: 2h 14m)       │
│  Port:  [8080]  │  Context: [ 8192 ]  │  GPU: [99]     │
│  Threads: [8]  │  Batch: [512]                          │
│                                                        │
│  Model: Llama-3.1-8B-Instruct-Q4_K_M.gguf  [Change]   │
│                                                        │
│  [Stop]  [Restart]  [Quick Swap...]                    │
│                                                        │
│  ── Live Log ────────────────────────────────────────── │
│  14:22:03  INFO  initializing slot                   │
│  14:22:04  INFO  HTTP server listening on port 8080   │
│  14:22:05  INFO  POST /v1/chat/completions 200 2.3s   │
│                                                        │
└────────────────────────────────────────────────────────┘
```

**Health monitoring:**
- Periodically hit `GET /v1/health` every 5 seconds
- Track: request latency, tokens/second, active connections
- Auto-restart on crash (with backoff: 1s, 5s, 30s, stop)
- Show warning if VRAM is nearly full

**Quick Model Swap:**
1. Stop server
2. Swap model file
3. Restart with same config
4. All in one keystroke, takes ~2 seconds

**Niceties:**
- **Port conflict detection**: Warn before starting: `Port 8080 in use by: "docker-proxy" (pid 3124)`. Suggest next available.
- **Startup timeout**: If server doesn't respond to health check within 30s, show last 10 log lines and suggest fixes.
- **Graceful shutdown**: Sends SIGTERM, waits 5s, then SIGKILL. Logs the event.
- **Performance snapshot**: `⚡ 42.3 tok/s  •  slot 1/4  •  KV cache: 62%`

---

### 4.5 Config System (`src/app/config.rs`)

**Location:** `~/.lmml/`

```
~/.lmml/
├── config.toml       # User preferences
├── state.toml        # Auto-saved session state
├── models.toml       # Model metadata cache
├── build/            # llama.cpp clone + build artifacts
│   ├── llama.cpp/    # git clone
│   ├── build/        # cmake build directory
│   └── bin/          # symlinks to compiled binaries
└── models/           # Default model storage directory
    └── *.gguf
```

**`config.toml`:**
```toml
# ~~ lmml configuration ~~
# Generated by lmml. Edit freely — comments are preserved.

[general]
model_dirs = ["~/.lmml/models", "/mnt/models"]
default_model = "Llama-3.1-8B-Instruct-Q4_K_M.gguf"
theme = "auto"                          # "auto" | "light" | "dark"

[build]
llama_cpp_path = "~/.lmml/build/llama.cpp"
extra_cmake_flags = []
jobs = 0                                # 0 = auto (nproc)

[server]
port = 8080
context_size = 8192
gpu_layers = 99
threads = 0                             # 0 = auto (nproc - 1)
batch_size = 512
extra_args = []
```

**`state.toml`** (auto-managed, don't edit):
```toml
[last_session]
last_model = "Llama-3.1-8B-Instruct-Q4_K_M.gguf"
server_running = false
server_port = 8080
build_state = "built"                   # "not-started" | "building" | "built" | "failed"
build_commit = "abc123def"
```

**Niceties:**
- **File watcher**: If you edit `config.toml` while the TUI is open, it hot-reloads with a notification: `⚡ Config reloaded — new port: 8081`
- **Migration on version bump**: If the schema changes, old config is backed up to `config.toml.bak` and a new one is written with defaults for missing fields.
- **Sensible defaults on first run**: `state.toml` starts clean. If `~/.lmml/` doesn't exist, it's created with a welcome message.

---

## 5. TUI Screen Blueprints

### 5.1 Dashboard (landing screen)

```
┌─ lmml ──────────────────────────────────────────── ● ● ● ─┐
│                                                             │
│  ╔═══════════════════════════════════════════════════════╗  │
│  ║                    SYSTEM OVERVIEW                    ║  │
│  ║   ● NVIDIA CUDA 12.4  —  GTX 1080 Ti  (11 GB VRAM)  ║  │
│  ║   ● CPU: AMD Ryzen 9 7950X  —  16C/32T              ║  │
│  ║   ○ RAM: 27.4 GB / 64 GB  ▰▰▰▰▱▱▱▱ 43%             ║  │
│  ║   ○ VRAM: 6.2 GB / 11 GB  ▰▰▰▰▰▱▱▱ 56%             ║  │
│  ╚═══════════════════════════════════════════════════════╝  │
│                                                             │
│  ┌─ Built ─────────────┐  ┌─ Server ────────────┐          │
│  │  ✓ llama.cpp built  │  │  ● Running on :8080 │          │
│  │  Commit: abc123def  │  │  Model: Llama-3.1-8B │          │
│  │  [Rebuild]          │  │  ⚡ 42.3 tok/s      │          │
│  └─────────────────────┘  └──────────────────────┘          │
│                                                             │
│  ┌─ Models ────────────────────────────────────────────┐   │
│  │  ★ Llama-3.1-8B-Instruct-Q4_K_M  (4.92 GB)  ✓     │   │
│  │    Mistral-7B-v0.3-Q4_K_M         (4.08 GB)       │   │
│  │    Phi-3-mini-4k-instruct-Q4_K_M  (2.34 GB)       │   │
│  │    ... 3 more models                               │   │
│  └────────────────────────────────────────────────────┘   │
│                                                             │
│  [Tab] Navigate  [M] Models  [S] Server  [B] Build  [q] Quit │
└─────────────────────────────────────────────────────────────┘
```

### 5.2 Models Screen

```
┌─ Models ─────────────────────────────────────────── ● ● ● ─┐
│  Search: [llama___________________________]  Sort: [Name v] │
│                                                             │
│  ┌──── Models ────────────────────┐  ┌── Details ─────────┐│
│  │ ★ Llama-3.1-8B-Q4_K_M  4.92G  │  │ Model: Llama-3.1-8B││
│  │   Mistral-7B-Q4_K_M    4.08G  │  │ Quant: Q4_K_M      ││
│  │ ▶ Phi-3-mini-Q4_K_M    2.34G  │  │ Size: 4.92 GB      ││
│  │   Llama-3.2-3B-Q4_K_M  1.92G  │  │ Params: 8.03B      ││
│  │   Gemma-2-2B-Q4_K_M    1.34G  │  │ Source: HF ggml-org ││
│  │                                │  │ Added: 2026-05-28  ││
│  │   Storage: 14.6 GB / 200 GB   │  │                    ││
│  │   ▰▰▰▰▰▰▰▱▱▱ 7%              │  │ [Load] [Delete]   ││
│  └────────────────────────────────┘  └────────────────────┘│
│                                                             │
│  [↓/↑] Navigate  [Enter] Select  [d] Download  [Del] Delete│
└─────────────────────────────────────────────────────────────┘
```

### 5.3 Server Screen

```
┌─ Server ─────────────────────────────────────────── ● ● ● ─┐
│                                                             │
│  Status: ● Running  —  pid: 12453  —  uptime: 2h 14m       │
│                                                             │
│  ┌─ Configuration ──────────────────────────────────────┐  │
│  │  Port    [ 8080  ▼]  Context [ 8192 ▼]  GPU [ 99 ▼] │  │
│  │  Threads [    8  ▼]  Batch  [  512 ▼]               │  │
│  │  Extra: [--mlock --no-mmap________________________]  │  │
│  │                                                      │  │
│  │  Model: Llama-3.1-8B-Instruct-Q4_K_M.gguf  [Swap]  │  │
│  │                                                      │  │
│  │  [ Stop ]  [ Restart ]  [ Quick Model Swap... ]     │  │
│  └──────────────────────────────────────────────────────┘  │
│                                                             │
│  ┌─ Performance ───────────────────────────────────────┐  │
│  │  ⚡ 42.3 tok/s  │  Active slots: 1/4  │  KV: 62%   │  │
│  │  Requests: 1,234  │  Avg latency: 2.3s              │  │
│  └──────────────────────────────────────────────────────┘  │
│                                                             │
│  ┌─ Live Log ──────────────────────────────────────────┐  │
│  │  14:22:03  INFO  initializing slot 0               │  │
│  │  14:22:04  INFO  HTTP server listening on port 8080 │  │
│  │  14:22:05  POST /v1/chat/completions 200 2.3s      │  │
│  │  14:22:10  POST /v1/chat/completions 200 1.8s      │  │
│  └──────────────────────────────────────────────────────┘  │
│                                                             │
│  [↓/↑] Scroll log  [Enter] Edit config  [s] Start/Stop     │
└─────────────────────────────────────────────────────────────┘
```

### 5.4 Build Screen

```
┌─ Build ──────────────────────────────────────────── ● ● ● ┐
│                                                             │
│  ┌─ Hardware Detection ────────────────────────────────┐  │
│  │  ✓ OS: Linux x86_64                                │  │
│  │  ✓ NVIDIA CUDA 12.4  —  GTX 1080 Ti  (11 GB VRAM) │  │
│  │  ✓ CPU: AVX2, AVX-512 (partial), AMD Ryzen 9       │  │
│  │  ○ ROCm: not detected                              │  │
│  │  ○ Vulkan: not detected                            │  │
│  └──────────────────────────────────────────────────────┘  │
│                                                             │
│  ┌─ Build Configuration ───────────────────────────────┐  │
│  │  cmake -B build \                                  │  │
│  │    -DGGML_CUDA=ON \                                │  │
│  │    -DCMAKE_BUILD_TYPE=Release \                     │  │
│  │    -DGGML_NATIVE=ON                                 │  │
│  │                                                      │  │
│  │  cmake --build build -j 16                          │  │
│  └──────────────────────────────────────────────────────┘  │
│                                                             │
│  ┌─ Build Progress ────────────────────────────────────┐  │
│  │  ▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▱▱▱▱▱▱▱▱▱ 68%           │  │
│  │  [================================>       ] 3:42   │  │
│  │                                                      │  │
│  │  [  4/342] Building CUDA kernel ggml-cuda.cu       │  │
│  │  [ 12/342] Building CXX object ggml/CMakeFiles/... │  │
│  │                                                      │  │
│  │  ⚠  Last build: 3 errors, 12 warnings              │  │
│  │     ⚠  warning: ignoring unknown flag --no-mmap    │  │
│  └──────────────────────────────────────────────────────┘  │
│                                                             │
│  [b] Build / Rebuild  [c] Cancel  [r] Redetect hardware    │
└─────────────────────────────────────────────────────────────┘
```

### 5.5 Settings Screen

```
┌─ Settings ────────────────────────────────────────── ● ● ● ┐
│                                                             │
│  ┌─ General ────────────────────────────────────────────┐  │
│  │  Model directories:                                  │  │
│  │    [~/.lmml/models_________________________________] │  │
│  │    [/mnt/models____________________________________] │  │
│  │                                                      │  │
│  │  Default model: [Llama-3.1-8B-Instruct-Q4_K_M  ▼ ] │  │
│  │  Theme: [○ Auto  ● Dark  ○ Light]                   │  │
│  └──────────────────────────────────────────────────────┘  │
│                                                             │
│  ┌─ Build ──────────────────────────────────────────────┐  │
│  │  llama.cpp path: [~/.lmml/build/llama.cpp__________] │  │
│  │  Extra cmake flags: [-DGGML_CUDA=ON_______________] │  │
│  │  Build jobs: [auto ▼]                               │  │
│  └──────────────────────────────────────────────────────┘  │
│                                                             │
│  ┌─ About ─────────────────────────────────────────────┐  │
│  │  lmml v0.1.0                                       │  │
│  │  Rust + ratatui + llama.cpp                        │  │
│  │  [View license]  [Check for updates]               │  │
│  └──────────────────────────────────────────────────────┘  │
│                                                             │
│  [Enter] Edit  [s] Save  [q] Back                          │
└─────────────────────────────────────────────────────────────┘
```

---

## 6. Error Handling Strategy

### Categories

| Category | Example | UX |
|----------|---------|----|
| **Recoverable** | Port in use, build warning, download interrupted | Inline warning badge + suggested action (`Port 8080 in use — try 8081?`) |
| **Unrecoverable** | No disk space, cmake not installed, permission denied | Modal dialog with problem description, command to fix, and option to retry or quit |
| **Transient** | Network timeout, server not ready yet | Spinner/progress bar + auto-retry with exp. backoff |
| **Silent** | Model metadata parse failed, one GPU probe failed | Logged, but TUI shows partial results — never fail the whole probe because one backend errored |

### Display principles

1. **Every error has a fix suggestion.** Not just "build failed" — `Build failed at 67% — CUDA compute capability 8.9 requires CUDA >= 11.8. Install CUDA 12.x and try again.`
2. **Never panic to the terminal.** All panics are caught by `color-eyre` and shown in the TUI before exit.
3. **Partial results are shown, not hidden.** If 3/4 hardware probes succeed, you see 3 green checks and 1 red one — not a wall of nothing.

---

## 7. Phased Delivery

### Phase 1 — MVP (build this first)

| Module | Deliverable | Depends on |
|--------|------------|------------|
| `src/main.rs` + `app/mod.rs` | App skeleton with ratatui event loop, basic navigation | Nothing |
| `probe/` | OS + CUDA detection → cmake flags | Nothing |
| `build/` | Clone llama.cpp, cmake + build with streaming | `probe/` |
| `tui/build.rs` | Build screen with detection results + build log | `build/` + `probe/` |
| `models/local.rs` | Scan directories for `.gguf` files, parse metadata | Nothing |
| `models/download.rs` | Download from HuggingFace with progress | Nothing |
| `tui/models.rs` | Model list screen with download | `models/` |
| `server/` | Start/stop llama-server, config, health check | `models/` |
| `tui/server.rs` | Server control screen | `server/` |
| `tui/dashboard.rs` | Dashboard overview screen | All above |
| `app/config.rs` | Config persistence (TOML) | Nothing |
| `tui/settings.rs` | Settings screen | `app/config.rs` |

#### Phase 1 — Gap Review (2026-06-01)

The following gaps exist between the plan's MVP specification and the current implementation:

| Module | Gap | Severity | Status |
|--------|-----|----------|--------|
| `probe/` | Missing BLAS detection | Minor | ✅ Done — `src/probe/blas.rs` with pkg-config detection |
| `probe/` | No install suggestions | Minor | ✅ Done — apt-get/brew suggestions for CUDA, ROCm, Vulkan, BLAS |
| `build/` | No ccache detection | Minor | ✅ Done — `which ccache` check in `run_all()` |
| `build/` | No binary symlink step | Medium | ✅ Done — successful builds link `llama-cli` and `llama-server` to `~/.lmml/build/bin/` |
| `build/` | No resume-on-reboot prompt | Medium | ✅ Done — prior `running` state opens Build screen with resume/skip prompt |
| `build/` | No last-20-lines pinning on failure | Minor | ✅ Done in earlier session (build screen shows last error lines) |
| `tui/build.rs` | No progress bar in build screen | Minor | ✅ Done in earlier session |
| `tui/build.rs` | No "Last build" summary | Minor | ✅ Done in earlier session |
| `models/local.rs` | No GGUF header inspection | Medium | ✅ Done — `read_gguf_header()` binary parsing |
| `models/download.rs` | No `hf://` prefix support | Minor | ✅ Done |
| `models/download.rs` | No download resume | Medium | ✅ Done — `Range` header + `.part` append |
| `models/download.rs` | No ETA display | Minor | ✅ Done — `eta_secs` in `DownloadEvent::Progress` |
| `tui/models.rs` | No search/filter | Minor | ✅ Done in earlier session |
| `tui/models.rs` | No sorting | Minor | ✅ Done — `s` key cycles Name/Size |
| `tui/models.rs` | No favorites toggle UI | Minor | ✅ Done in earlier session — `f` key |
| `tui/models.rs` | No delete key | Minor | ✅ Done in earlier session — `Del` key |
| `tui/models.rs` | No disk usage display | Minor | ✅ Done — "N models — X.XX GB" summary line |
| `server/` | No graceful shutdown wait | Minor | ✅ Done in earlier session — SIGTERM → 5s → SIGKILL |
| `server/` | Port conflict detection unused | Minor | ✅ Done in earlier session — called before server start |
| `server/` | No performance parsing | Medium | ✅ Done — JSON body parsed for `tokens_per_second` |
| `tui/server.rs` | No performance panel | Minor | ✅ Done — panel shows tok/s, latency, slots, and KV cache when endpoint reports them |
| `tui/server.rs` | No config editing in screen | Minor | ✅ Done — arrow/Enter edits server fields inline and saves config |
| `tui/server.rs` | No quick model swap | Minor | ✅ Done — `m` cycles selected model from Server screen |
| `tui/dashboard.rs` | No RAM/VRAM usage bars | Minor | ✅ Done for RAM + NVIDIA CUDA — RAM via sysinfo, VRAM via `nvidia-smi`; cross-vendor GPU polling pending |
| `tui/dashboard.rs` | No commit hash display | Minor | ✅ Done — build commit hash in status line |
| `app/config.rs` | No `models.toml` metadata cache | Minor | ✅ Done — `ModelsCache` struct + load/save functions |
| `app/config.rs` | No schema migration | Minor | ✅ Done — `CONFIG_VERSION` + backup-on-change |
| `tui/settings.rs` | No theme selector | Minor | ✅ Done — theme input validates auto/dark/light |
| `tui/settings.rs` | No build config section | Minor | ✅ Done — llama_cpp_path, extra flags, jobs |
| `tui/settings.rs` | Save overwrites config defaults | **Major** | ✅ Fixed — all read/write through `app.state.config` |
| All screens | No help bar rendering | Minor | ✅ Done in earlier session — screen-specific help bar |
| `tui/widgets/model_card.rs` | Widget exists but unused | Minor | ✅ Done — model detail pane renders through `ModelCard` |

**Closure summary:** Core Phase 1 gaps are closed. Remaining work is follow-up hardening and polish tracked in `docs/todo.md`.

### Phase 2 — GPU Backend Expansion

| Module | Deliverable |
|--------|------------|
| `probe/rocm.rs` | AMD ROCm/HIP detection → cmake flags |
| `probe/vulkan.rs` | Vulkan SDK detection |
| `probe/metal.rs` | macOS Metal detection |
| `probe/cpu.rs` | Advanced CPU feature detection |
| `probe/cmake.rs` | Multi-backend cmake flag generation |

### Phase 3 — Hardening

#### Implementation Status (as of 2026-06-02)

| Item | Status | Notes |
|------|--------|-------|
| Auto-restart server on crash | ✅ Done | Wraps server start + health check in a restart loop. Re-spawns process on crash. Exponential backoff: 1s → 2s → 4s → ... → 30s cap. |
| Cancel build mid-flight | ✅ Done | `Arc<AtomicBool>` flag polled every 500ms inside `run_command`. Kills cmake child process. Checks flag between phases too. |
| Progress bar on download | ✅ Done | Wired to models download overlay. Shows speed + ETA in human-readable format. |
| Unit tests (Phase 1 modules) | ✅ Done | 17 tests — probe cmake flags, config path resolution, download parsing, GGUF header, config schema. |
| Graceful server shutdown | ✅ Done | SIGTERM → 200ms poll loop → 5s deadline → SIGKILL. `#[cfg(unix)]` guarded. |
| Port conflict detection | ✅ Done | `is_port_in_use()` called in server start handler before spawning subprocess. |
| GGUF header inspection | ✅ Done | Binary GGUF parser in `local.rs` extracts architecture, context_length from KV pairs. |
| Download resume | ✅ Done | `Range` header + `.part` file append. Handles `Content-Range` response. |
| Server health JSON parsing | ✅ Done | Parses `tokens_per_second` / `completion_tokens_per_second` from `/v1/health` body. |
| Config schema migration | ✅ Done | `CONFIG_VERSION = 1` with backup to `config.toml.bak` on version change. |
| Config hot-reload polling | ✅ Done | `check_config_reload()` compares file modification timestamps. |
| BLAS detection | ✅ Done | pkg-config based OpenBLAS/MKL detection in `src/probe/blas.rs`. |
| Install suggestions | ✅ Done | Per-platform apt-get/brew commands for missing CUDA, ROCm, Vulkan, BLAS. |
| ccache detection | ✅ Done | `which ccache` check in probe output. |
| Models sort + disk usage | ✅ Done | `s` key cycles Name/Size sort. "N models — X.XX GB" summary line. |
| Dashboard RAM bar | ✅ Done | Real-time `sysinfo` gauge with color coding (green/yellow/red at 70%/90%). |
| Build commit hash | ✅ Done | `git rev-parse --short HEAD` captured after clone/pull, displayed on dashboard. |
| Settings build config | ✅ Done | llama_cpp_path, extra_cmake_flags, jobs fields with theme validation. |

#### Remaining Gaps

- **Build resume on reboot** — ✅ Done: build start persists `running`; next launch opens Build screen with resume/skip prompt.
  - Requires: persisted state check in `main.rs`
  - Estimated effort: medium (2h)

- **Server performance panel** — ✅ Done: Server screen includes a performance panel. Follow-up: query richer endpoints when `/v1/health` omits active slots or KV cache.

- **Dashboard VRAM usage bar** — ✅ Done for NVIDIA CUDA via `nvidia-smi`; follow-up: support ROCm/Metal/Vulkan memory sources.

- **Performance metrics persistence** — Track tok/s, request latency, active connections over time. Persist to `~/.lmml/metrics.toml`. Display historical charts or sparklines in server screen.
  - Estimated effort: large (4-6h) including data model, serialization, TUI rendering

- **Multiple server instances** — Allow running llama-server on multiple ports simultaneously. Requires per-instance state management, port allocation, and multi-instance UI.
  - Estimated effort: large (6-8h) — architectural change

- **Binary symlink step** — ✅ Done: verified builds link binaries into `~/.lmml/build/bin/`.

### Phase 4 — Advanced

- Fine-tuning launcher (LoRA via existing llama.cpp tools)
- Model merging / quantization via external scripts
- System tray background mode
- opencode / API integration

---

## 8. Development Setup

```bash
# Prerequisites
rustup update stable
cargo install cargo-watch          # hot-reload during dev

# Build
cargo build

# Run
cargo run

# Watch mode (auto-rebuild on changes)
cargo watch -x run

# Test
cargo test
cargo clippy -- -D warnings
```

**First run experience:**
1. `lmml` launched with no `~/.lmml/` → creates directory structure
2. Probe Engine auto-runs → shows detected hardware
3. If llama.cpp not built → shows build screen with detected flags
4. User presses `[b]` → builds llama.cpp with progress
5. On success → dashboard shows ready state, prompts to download a model
6. User downloads a model → can start server immediately

---

## 9. Key Dependencies (Cargo.toml)

```toml
[package]
name = "lmml"
version = "0.1.0"
edition = "2024"

[dependencies]
# TUI
ratatui = "0.29"
crossterm = "0.28"

# Async runtime
tokio = { version = "1", features = ["full", "process"] }

# Serialization
serde = { version = "1", features = ["derive"] }
toml = "0.8"

# HTTP / downloads
reqwest = { version = "0.12", features = ["stream"] }

# System
sysinfo = "0.33"

# Error handling
color-eyre = "0.6"
tracing = "0.1"
tracing-subscriber = "0.3"

# Utilities
fuzzy-matcher = "0.3"             # fuzzy model search
chrono = "0.4"                     # timestamps
human-size = "0.5"                 # "4.92 GB" formatting
indicatif = "0.17"                 # progress bars (for download widget)
```

---

## 10. Risks & Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| llama.cpp build takes 30+ min | Bad first impression | Show estimated time, use ccache, parallel jobs, show progress |
| CUDA/ROCm detection fragile across distros | Wrong build flags | Fall back to CPU-only + show warning, allow manual override |
| Server subprocess dies silently | User thinks server is running | Health check every 5s, badge turns red immediately |
| Large model download interrupted | Wasted bandwidth | Support resume via `Range` headers, track partial files |
| Config file format changes in future version | User config broken | Version the schema, auto-migrate with backup on version bump |
| TUI too complex for new users | Overwhelming | Dashboard as landing page, help bar always visible, `?` key for full help |

---

*This plan is a living document. As we build, things will change. Every deviation gets documented here with the rationale.*
