# lmml — Rust TUI for llama.cpp
### Detailed Production Plan (v2)

> A turnkey TUI app for managing llama.cpp: auto-detect hardware, build from source,
> manage GGUF models, and run the inference server — all from one terminal.

---

## 0. Project layout

```
lmml/
├── Cargo.toml                  # workspace
├── AGENTS.md
├── crates/
│   ├── lmml-detect/            # hardware & prerequisite detection
│   ├── lmml-compat/            # llama.cpp version + flag compatibility layer
│   ├── lmml-build/             # clone, sccache, cmake build, incremental logic
│   ├── lmml-models/            # GGUF registry, metadata, HF search, download
│   ├── lmml-server/            # llama-server lifecycle, health polling
│   ├── lmml-state/             # persistent AppState (serde + toml, XDG)
│   └── lmml-tui/               # ratatui TUI binary — orchestrates all above
└── docs/
    └── architecture.md
```

Each crate has one clear responsibility. `lmml-tui` is the only binary crate.
All library crates are independently testable with no TUI dependency.

---

## 1. Hardware & prerequisite detection (`lmml-detect`)

### 1.1 What it detects

| Check | Method | Result type |
|---|---|---|
| `gcc` / `clang` | `which` + `--version` + C++17 compile probe | `CompilerInfo` |
| `cmake` | `which` + `--version`, enforce ≥ 3.21 | `CmakeInfo` |
| `git` | `which` + `--version`, enforce ≥ 2.28 | `GitInfo` |
| `nvcc` | `which` + `--version` | `NvccInfo` |
| CUDA toolkit version | parse `nvcc --version` output | `CudaVersion` |
| GPU list + compute caps | `nvidia-smi --query-gpu=name,memory.total,compute_cap --format=csv,noheader` | `Vec<GpuInfo>` |
| CUDA arch compatibility | cross-check nvcc toolkit vs GPU `compute_cap` | `CudaCompatibility` |
| `sccache` | `which sccache` | `Option<PathBuf>` |
| Metal (macOS) | `cfg!(target_os="macos")` + `system_profiler SPDisplaysDataType` | `MetalSupport` |
| CPU features | `/proc/cpuinfo` (Linux) / `sysctl` (macOS) | `CpuFeatures` |
| Available RAM | `sysinfo` crate | `MemInfo` |
| Available disk | `statvfs` on build dir parent | `DiskInfo` |

### 1.2 CUDA architecture support

lmml supports every CUDA-capable GPU from sm_37 (Kepler, GTX 700 series, 2013) through
the latest Ada Lovelace / Hopper / Blackwell generation. The full architecture map:

| Compute capability | Architecture | Example cards |
|---|---|---|
| sm_37 | Kepler (GK210) | Tesla K80 |
| sm_50 / sm_52 / sm_53 | Maxwell | GTX 750 Ti, GTX 970/980 |
| sm_60 / sm_61 / sm_62 | Pascal | GTX 1060/1070/1080, Titan X |
| sm_70 / sm_72 | Volta | Tesla V100, Titan V |
| sm_75 | Turing | RTX 2060/2070/2080, GTX 1660 |
| sm_80 / sm_86 / sm_87 | Ampere | RTX 3060–3090, A100, A10 |
| sm_89 | Ada Lovelace | RTX 4060–4090 |
| sm_90 / sm_90a | Hopper | H100, H200 |
| sm_100 / sm_100a | Blackwell | RTX 5080/5090, B100 |

Detection logic:

```rust
/// Maps a raw compute_cap string (e.g. "8.6") to the canonical sm_ arch string.
pub fn compute_cap_to_arch(cap: &str) -> Option<&'static str> {
    match cap {
        "3.7" => Some("sm_37"),
        "5.0" => Some("sm_50"),
        "5.2" => Some("sm_52"),
        "5.3" => Some("sm_53"),
        "6.0" => Some("sm_60"),
        "6.1" => Some("sm_61"),
        "6.2" => Some("sm_62"),
        "7.0" => Some("sm_70"),
        "7.2" => Some("sm_72"),
        "7.5" => Some("sm_75"),
        "8.0" => Some("sm_80"),
        "8.6" => Some("sm_86"),
        "8.7" => Some("sm_87"),
        "8.9" => Some("sm_89"),
        "9.0" => Some("sm_90"),
        "9.0a" => Some("sm_90a"),
        "10.0" => Some("sm_100"),
        "10.0a" => Some("sm_100a"),
        _ => None,
    }
}
```

Multi-GPU systems: collect all unique `sm_` values and pass them as a
semicolon-separated list to `-DCMAKE_CUDA_ARCHITECTURES="sm_75;sm_86"`.
This compiles a single binary that runs optimally on every card in the system.

### 1.3 CUDA compatibility cross-check

nvcc can only compile for architectures it knows about. An sm_90 card with CUDA 11.x
toolkit produces a binary that crashes at load time. lmml catches this before building:

```rust
pub enum CudaCompatibility {
    /// nvcc version supports all detected GPU architectures.
    Compatible { archs: Vec<&'static str> },
    /// nvcc is too old for one or more GPUs.
    ToolkitTooOld {
        gpu_arch:        &'static str,
        minimum_toolkit: &'static str,
        found_toolkit:   String,
    },
    /// nvcc found but no CUDA-capable GPUs detected.
    NoGpu,
    /// nvcc not found; CUDA backend unavailable.
    NvccMissing,
}
```

Minimum toolkit versions per architecture family:

| sm_ range | Minimum CUDA toolkit |
|---|---|
| sm_37 – sm_75 | 9.0 |
| sm_80 – sm_87 | 11.1 |
| sm_89 | 11.8 |
| sm_90 / sm_90a | 12.0 |
| sm_100 / sm_100a | 12.4 |

### 1.4 C++17 compile probe

```rust
// Runs: echo '#include <filesystem>' | $compiler -std=c++17 -x c++ - -fsyntax-only
// Returns Err with a human-readable message if the compiler rejects it.
pub fn probe_cpp17(compiler: &Path) -> Result<(), CompilerProbeError>;
```

### 1.5 Disk space check

llama.cpp source + build artifacts require at least 4 GB. lmml checks before starting:

```rust
pub struct DiskInfo {
    pub available_bytes: u64,
    pub path:            PathBuf,
}

impl DiskInfo {
    /// Returns Err if less than `min_bytes` are available.
    pub fn require(&self, min_bytes: u64) -> Result<(), InsufficientDiskError>;
}
```

### 1.6 API shape

```rust
/// Complete picture of hardware and toolchain capabilities on this machine.
pub struct SystemProfile {
    pub compiler:       Option<CompilerInfo>,
    pub cmake:          Option<CmakeInfo>,
    pub git:            Option<GitInfo>,
    pub cuda:           CudaCompatibility,
    pub gpus:           Vec<GpuInfo>,
    pub sccache:        Option<PathBuf>,
    pub metal:          MetalSupport,
    pub cpu:            CpuFeatures,
    pub memory:         MemInfo,
    pub disk:           DiskInfo,
}

impl SystemProfile {
    /// Run all probes concurrently and return the combined profile.
    pub fn detect() -> impl std::future::Future<Output = SystemProfile> + Send;

    /// The recommended backend given what's available.
    pub fn recommended_backend(&self) -> BuildBackend;

    /// All unmet hard prerequisites (tools that must be present to proceed).
    pub fn missing_prerequisites(&self) -> Vec<MissingPrerequisite>;

    /// Soft warnings (tools present but suboptimal versions).
    pub fn warnings(&self) -> Vec<DetectionWarning>;
}

pub enum BuildBackend {
    Cuda { archs: Vec<&'static str> },  // e.g. ["sm_75", "sm_86"]
    Metal,
    CpuAvx2,
    CpuAvx,
    CpuFallback,
}

pub struct MissingPrerequisite {
    pub name:    &'static str,
    pub install: &'static str,  // e.g. "sudo apt install cmake"
}

pub struct DetectionWarning {
    pub message: String,        // human-readable, e.g. "git 2.25 detected; 2.28+ recommended"
}
```

All probes run concurrently via `tokio::join!`. Results cached in `lmml-state`
so the TUI starts instantly on subsequent launches.

---

## 2. llama.cpp version compatibility (`lmml-compat`)

llama.cpp changes CLI flags and server API frequently. This crate insulates the
rest of lmml from upstream churn. It is the **only** place that knows about
llama.cpp flag names.

### 2.1 Capability detection

```rust
pub struct LlamaBinaryCapabilities {
    pub version:       Option<String>,   // from `llama-server --version`
    pub flash_attn:    bool,
    pub mlock:         bool,
    pub api_key:       bool,
    pub ubatch_size:   bool,
    pub chat_template: bool,
    pub jinja:         bool,
    pub reranking:     bool,
}

impl LlamaBinaryCapabilities {
    /// Run `llama-server --help` and parse the flag list.
    pub fn probe(
        binary: &Path,
    ) -> impl std::future::Future<Output = Result<Self, CompatError>> + Send;
}
```

### 2.2 Flag assembly

```rust
/// Stable internal config, version-independent.
pub struct ServerConfig {
    pub model:         PathBuf,
    pub port:          u16,
    pub host:          String,
    pub ctx_size:      u32,
    pub n_gpu_layers:  i32,
    pub batch_size:    u32,
    pub ubatch_size:   u32,
    pub threads:       usize,
    pub flash_attn:    bool,
    pub mlock:         bool,
    pub api_key:       Option<String>,
    pub chat_template: Option<String>,
    pub jinja:         bool,
    pub extra_args:    Vec<String>,
}

/// Translate a ServerConfig into the correct argv for the detected binary.
pub fn build_argv(
    config: &ServerConfig,
    caps:   &LlamaBinaryCapabilities,
) -> Vec<String>;
```

Flags unsupported by the detected binary are silently omitted and surfaced
as `DetectionWarning` entries shown in the TUI Settings tab.

---

## 3. Build management (`lmml-build`)

### 3.1 Responsibilities

- Clone `ggml-org/llama.cpp` at a pinned commit or user-specified ref.
- Auto-enable `sccache` if detected (cuts repeat build time from ~8 min to ~30 sec).
- Translate `BuildBackend` + user overrides into cmake flags.
- Detect whether a rebuild is needed (source commit + full cmake invocation hash).
- Verify the binary on startup (exists + executable) before trusting `BuildState`.
- Stream build output line-by-line to the TUI.
- Retain last 500 log lines in state for post-failure scrollback.
- Support a clean-build mode (delete build dir, start fresh).

### 3.2 sccache integration

If `sccache` is on `$PATH`, inject automatically:

```
-DCMAKE_C_COMPILER_LAUNCHER=sccache
-DCMAKE_CXX_COMPILER_LAUNCHER=sccache
```

No user configuration needed. The TUI shows a `⚡ sccache active` badge when enabled.

### 3.3 cmake flag matrix

| Backend | Flags added |
|---|---|
| CUDA (single arch) | `-DGGML_CUDA=ON -DCMAKE_CUDA_ARCHITECTURES=sm_86` |
| CUDA (multi-GPU) | `-DGGML_CUDA=ON -DCMAKE_CUDA_ARCHITECTURES="sm_75;sm_86"` |
| Metal | `-DGGML_METAL=ON` |
| AVX2 | `-DGGML_AVX2=ON` |
| AVX | `-DGGML_AVX=ON` |
| Fallback | (only base flags) |
| All | `-DCMAKE_BUILD_TYPE=Release -DLLAMA_BUILD_SERVER=ON` |
| sccache active | `+ -DCMAKE_C_COMPILER_LAUNCHER=sccache -DCMAKE_CXX_COMPILER_LAUNCHER=sccache` |

### 3.4 Incremental rebuild detection

A rebuild is triggered when any of the following change:

- The resolved git commit hash of the source tree.
- A SHA-256 of the full cmake invocation string (flags + paths).
- The binary is absent or not executable (partial/interrupted build recovery).

```rust
pub struct BuildFingerprint {
    pub commit:         String,
    pub cmake_hash:     [u8; 32],   // SHA-256 of full cmake argv
    pub binary:         PathBuf,
}

impl BuildFingerprint {
    pub fn needs_rebuild(&self) -> bool;
}
```

### 3.5 API shape

```rust
pub enum BuildEvent {
    Cloning { url: String },
    CmakeConfiguring,
    Compiling { line: String },
    Linking,
    Completed { binary: PathBuf, elapsed: Duration },
    Failed { last_error: String, log_tail: Vec<String> },
}

pub trait BuildRunner {
    fn run(
        &self,
        config: BuildConfig,
    ) -> impl std::future::Future<Output = tokio::sync::mpsc::Receiver<BuildEvent>> + Send;
}
```

### 3.6 Update flow

```rust
pub enum UpdateCheck {
    UpToDate { current: String },
    Available { current: String, latest: String, commits_behind: usize },
    Unreachable { reason: String },
}

pub fn check_for_update(
    source_dir: &Path,
) -> impl std::future::Future<Output = UpdateCheck> + Send;
```

The TUI Build tab shows the update status and offers "Update and rebuild" when
a newer commit is available. Users can choose between "track main" and "pin to
release tag" in Settings.

---

## 4. Model management (`lmml-models`)

### 4.1 Responsibilities

- Scan a configurable models directory (default `~/.local/share/lmml/models/`).
- Support symlinked paths and alias entries for models stored elsewhere.
- Parse GGUF metadata header to extract name, architecture, quant, and context length.
- Compute a per-model VRAM fit estimate given the detected GPU(s).
- Search Hugging Face for GGUF models by keyword, architecture, and quant tier.
- Download with progress streaming; resume interrupted downloads.
- Delete with confirmation.

### 4.2 GGUF metadata parsing

Read the GGUF binary header (magic `GGUF`, version, tensor count, metadata KV pairs)
to extract:

| Field | GGUF key |
|---|---|
| Model name | `general.name` |
| Architecture | `general.architecture` |
| Context length | `<arch>.context_length` |
| Embedding length | `<arch>.embedding_length` |
| Layer count | `<arch>.block_count` |
| Quantisation | derived from tensor dtype field |
| File size | filesystem |

### 4.3 VRAM fit estimation

Given GPU VRAM (from detection) and model file size + quant, compute:

```rust
pub enum VramFit {
    /// Entire model fits; full GPU acceleration.
    Full { vram_used_mb: u64, vram_free_mb: u64 },
    /// Partial offload; specify how many layers fit.
    Partial { recommended_ngl: i32, cpu_layers: i32 },
    /// Model too large even for partial offload at this quant.
    TooLarge { model_mb: u64, vram_mb: u64 },
    /// No GPU available; CPU only.
    CpuOnly,
}

impl ModelEntry {
    pub fn vram_fit(&self, gpus: &[GpuInfo]) -> VramFit;

    /// Returns the recommended -ngl value for this model on these GPUs.
    pub fn recommended_ngl(&self, gpus: &[GpuInfo]) -> i32;
}
```

VRAM fit is shown next to each model in the list:
```
  mistral-7b-q4_k_m.gguf   4.1 GB  Q4_K_M  ✓ fits (2.1 GB free)
  mixtral-8x7b-q4_k_m.gguf 24.6 GB Q4_K_M  ⚠ partial (32 layers on GPU)
  llama-70b-q8_0.gguf       74.2 GB Q8_0    ✗ too large (11 GB VRAM)
```

### 4.4 Hugging Face search

```rust
pub struct HfSearchQuery {
    pub keywords:      String,
    pub architecture:  Option<String>,   // e.g. "llama", "mistral"
    pub quant_filter:  Option<QuantTier>,
    pub max_results:   usize,
}

pub enum QuantTier { Q4, Q5, Q6, Q8, F16, F32 }

pub struct HfModelResult {
    pub repo_id:       String,
    pub filename:      String,
    pub size_bytes:    u64,
    pub downloads:     u64,
    pub url:           String,
}

pub fn search_huggingface(
    query: HfSearchQuery,
) -> impl std::future::Future<Output = Result<Vec<HfModelResult>, HfError>> + Send;
```

The TUI Models tab has a search pane (press `/` to open) that queries this API and
lets the user select and download directly — no URL copy-pasting required.

### 4.5 Download with resume

Downloads use HTTP `Range` requests. If interrupted, the next attempt resumes from
the byte offset of the partially downloaded file.

```rust
pub struct DownloadProgress {
    pub bytes_received: u64,
    pub total_bytes:    Option<u64>,
    pub resumed_from:   u64,          // 0 on fresh download
}
```

### 4.6 API shape

```rust
pub struct ModelEntry {
    pub path:           PathBuf,
    pub name:           String,
    pub size_bytes:     u64,
    pub quant:          String,
    pub context_length: Option<u32>,
    pub architecture:   Option<String>,
    pub aliased:        bool,         // true if symlink / external path
}

pub struct ModelRegistry {
    pub models_dir: PathBuf,
    pub aliases:    Vec<PathBuf>,
}

impl ModelRegistry {
    pub fn scan(&self) -> impl std::future::Future<Output = Vec<ModelEntry>> + Send;

    pub fn add_alias(&self, path: PathBuf) -> Result<(), RegistryError>;

    pub fn download(
        &self,
        url: &str,
        on_progress: impl Fn(DownloadProgress) + Send + 'static,
    ) -> impl std::future::Future<Output = Result<ModelEntry, DownloadError>> + Send;

    pub fn delete(&self, entry: &ModelEntry) -> Result<(), RegistryError>;
}
```

---

## 5. Server lifecycle (`lmml-server`)

### 5.1 Responsibilities

- Start / stop / restart `llama-server` as a managed child process.
- Use `lmml-compat` to assemble the correct argv for the detected binary.
- Stream server stdout/stderr into the TUI log pane.
- Detect readiness via **HTTP health polling** (`GET /health`) — not log-line watching.
- Detect port conflicts before spawning.
- Auto-compute `-ngl` from `VramFit` if not manually overridden.
- Kill the server on clean exit; no orphaned processes.
- Expose a `watch::Receiver<ServerStatus>` so the TUI footer updates reactively.

### 5.2 Health polling

Polling `GET /health` is more robust than watching for a log line: it survives
llama.cpp log format changes across versions.

```rust
async fn wait_for_ready(port: u16, timeout: Duration) -> Result<(), ServerError> {
    let url = format!("http://127.0.0.1:{port}/health");
    let deadline = Instant::now() + timeout;
    loop {
        if Instant::now() > deadline {
            return Err(ServerError::StartupTimeout);
        }
        match reqwest::get(&url).await {
            Ok(r) if r.status().is_success() => return Ok(()),
            _ => tokio::time::sleep(Duration::from_millis(250)).await,
        }
    }
}
```

### 5.3 Port conflict detection

```rust
async fn check_port_free(port: u16) -> Result<(), ServerError> {
    match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
        Ok(_)  => Ok(()),
        Err(_) => Err(ServerError::PortInUse { port }),
    }
}
```

Error shown in TUI: `"llama-server failed to start — port 8080 is already in use"`

### 5.4 API shape

```rust
pub enum ServerStatus {
    Stopped,
    Starting { elapsed: Duration },
    Ready { url: String },
    Failed { reason: String },
}

pub struct ServerHandle {
    status_rx: tokio::sync::watch::Receiver<ServerStatus>,
}

impl ServerHandle {
    pub fn status(&self) -> ServerStatus;
    pub fn stop(&self) -> impl std::future::Future<Output = ()> + Send;
}

pub struct ServerManager {
    pub binary: PathBuf,
    pub caps:   LlamaBinaryCapabilities,
}

impl ServerManager {
    pub fn start(
        &self,
        model:   &ModelEntry,
        config:  &ServerConfig,
        log_tx:  tokio::sync::mpsc::Sender<String>,
    ) -> impl std::future::Future<Output = Result<ServerHandle, ServerError>> + Send;
}
```

### 5.5 Configurable server flags

All of these are exposed in the TUI Settings tab and persisted in `lmml-state`.

```toml
[server]
port           = 8080
host           = "127.0.0.1"
ctx_size       = 4096
n_gpu_layers   = -1          # -1 = auto from VramFit
batch_size     = 512
ubatch_size    = 512
threads        = 8
flash_attn     = true        # auto-enabled when supported
mlock          = false
api_key        = ""          # empty = no auth
jinja          = false
chat_template  = ""
extra_args     = []
```

---

## 6. Persistent state (`lmml-state`)

### 6.1 What's persisted

```toml
# ~/.config/lmml/state.toml  (XDG_CONFIG_HOME respected)

[build]
source_dir    = "~/.local/share/lmml/llama.cpp"
binary        = "~/.local/share/lmml/bin/llama-server"
commit        = "abc1234"
cmake_hash    = "e3b0c44298fc..."   # SHA-256 of cmake invocation
backend       = "Cuda"
archs         = ["sm_75", "sm_86"]
sccache_used  = true
last_built    = "2025-05-01T12:00:00Z"
track_mode    = "main"              # or "tag"

[model]
last_used     = "~/.local/share/lmml/models/mistral-7b-q4_k_m.gguf"
models_dir    = "~/.local/share/lmml/models"
aliases       = []

[server]
port          = 8080
host          = "127.0.0.1"
ctx_size      = 4096
n_gpu_layers  = -1
batch_size    = 512
ubatch_size   = 512
threads       = 8
flash_attn    = true
mlock         = false
api_key       = ""
jinja         = false
chat_template = ""
extra_args    = []

[system_profile]
# cached probe results; re-run with 'd' in the Detect tab
cuda_toolkit  = "12.4"
gpu_names     = ["NVIDIA GeForce GTX 1080 Ti"]
gpu_archs     = ["sm_61"]
vram_mb       = [11264]
sccache       = true
```

### 6.2 API shape

```rust
pub struct AppState {
    pub build:          BuildState,
    pub model:          ModelState,
    pub server:         ServerConfig,
    pub system_profile: Option<SystemProfile>,
}

impl AppState {
    pub fn load()  -> Result<Self, StateError>;
    pub fn save(&self) -> Result<(), StateError>;
    pub fn path()  -> PathBuf;   // respects $XDG_CONFIG_HOME
    pub fn reset() -> Result<(), StateError>;
}
```

---

## 7. TUI architecture (`lmml-tui`)

### 7.1 Module structure

The TUI is broken into focused modules to prevent any single file from growing
beyond ~500 LoC.

```
lmml-tui/src/
├── main.rs           # entry point, tokio runtime setup
├── app.rs            # App struct + state slices (orchestration only, no rendering)
├── event_loop.rs     # multiplexes terminal events + tokio task messages + channels
├── action.rs         # Action enum dispatched through the event loop
├── tabs/
│   ├── mod.rs
│   ├── detect.rs
│   ├── build.rs
│   ├── models.rs
│   ├── server.rs
│   └── settings.rs
├── widgets/
│   ├── status_badge.rs
│   ├── log_pane.rs
│   ├── progress_bar.rs
│   ├── confirm_dialog.rs
│   ├── input_dialog.rs
│   └── help_overlay.rs
└── footer.rs
```

### 7.2 Event loop architecture

```rust
pub enum AppEvent {
    // Terminal input
    Key(crossterm::event::KeyEvent),
    Resize(u16, u16),

    // Background task completions
    DetectComplete(SystemProfile),
    BuildEvent(BuildEvent),
    ServerStatus(ServerStatus),
    ServerLog(String),
    DownloadProgress(DownloadProgress),
    ModelScanComplete(Vec<ModelEntry>),
    HfSearchResults(Vec<HfModelResult>),
    UpdateCheckResult(UpdateCheck),
}

pub enum Action {
    RunDetect,
    StartBuild,
    CancelBuild,
    StartServer,
    StopServer,
    SelectModel(PathBuf),
    OpenHfSearch,
    SearchHf(HfSearchQuery),
    DownloadModel(HfModelResult),
    DeleteModel(ModelEntry),
    AddModelAlias,
    CheckForUpdate,
    UpdateAndRebuild,
    SaveSettings,
    ShowHelp,
    Quit,
}
```

Each tab receives `&mut App` and `&mut Frame`, renders itself, and maps key presses
to `Action` values. The event loop dispatches `Action` to the appropriate background
task or state mutation. No rendering logic lives in `app.rs`.

### 7.3 Layout

```
┌─────────────────────────────────────────────────────────────┐
│  lmml  v0.1.0        [1]Detect [2]Build [3]Models [4]Server │  tab bar
│                      [5]Settings                     [?]Help │
├────────────────────────────┬────────────────────────────────┤
│                            │                                │
│  LEFT PANE                 │  RIGHT PANE                    │
│  (context-sensitive)       │  (log / output stream)         │
│                            │                                │
│  • Status badges           │  real-time cmake / server      │
│  • Model list              │  stdout with scrollback        │
│  • HF search results       │  (last 500 lines retained)     │
│  • Config fields           │                                │
│                            │                                │
├────────────────────────────┴────────────────────────────────┤
│  ● Server: Ready  http://localhost:8080    ⚡ sccache  [Q]  │  footer
└─────────────────────────────────────────────────────────────┘
```

### 7.4 Tabs

| Tab | Left pane | Right pane |
|---|---|---|
| **Detect** | System profile with green/yellow/red badges per tool; missing prereq hints; CUDA arch list | Raw probe output |
| **Build** | Build config summary; cmake flags; sccache status; update check badge; "Build" / "Update & Rebuild" / "Clean Build" actions | Streaming build log with scrollback |
| **Models** | Scrollable model list (size, quant, VRAM fit badge); `/` to open HF search | Selected model metadata; download progress bar |
| **Server** | Server config summary; start/stop button; status badge | Server stdout/stderr |
| **Settings** | All config fields editable inline; llama-server flag capability warnings | Saved state path; "Reset to defaults" |

### 7.5 Status badges

```
  ✓  gcc 15.2.0            (green)
  ✓  cmake 4.2.3            (green)
  ✓  CUDA 12.4              (green)
  ✓  GTX 1080 Ti · sm_61 · 11 GB  (green)
  ✓  sccache active         (green)
  ✗  nvcc not found         (red)    → "sudo apt install nvidia-cuda-toolkit"
  ⚠  CUDA toolkit 11.0      (yellow) → "sm_89 requires CUDA ≥ 11.8; upgrade toolkit"
  ⚠  git 2.25               (yellow) → "git ≥ 2.28 recommended"
  ✗  disk: only 1.2 GB free (red)    → "need ≥ 4 GB to build"
```

### 7.6 First-run onboarding

When no state file exists, lmml shows a guided modal sequence instead of dropping
the user into an empty interface:

```
┌─────────────────────────────────────────┐
│  Welcome to lmml                        │
│                                         │
│  Scanning your system...                │
│                                         │
│  ✓  gcc, cmake, git found               │
│  ✓  CUDA 12.4 · RTX 3090 · sm_86       │
│  ✓  44 GB disk available                │
│                                         │
│  Ready to build llama.cpp with CUDA.    │
│                                         │
│  [Build now]   [Choose backend]  [Skip] │
└─────────────────────────────────────────┘
```

Subsequent screens: choose model directory → optionally download a starter model
→ configure server port → done.

### 7.7 Key bindings

| Key | Action |
|---|---|
| `1`–`5` | Jump to tab |
| `Tab` / `Shift+Tab` | Cycle tabs |
| `↑` / `↓` | Navigate lists |
| `Enter` | Select / confirm |
| `d` | Re-run detect |
| `b` | Start build |
| `u` | Check for update |
| `s` | Start / stop server |
| `/` | Open HF model search |
| `D` | Download selected HF result |
| `a` | Add model alias (external path) |
| `x` | Delete selected model (confirm dialog) |
| `e` | Edit settings field |
| `?` | Show keybinding help overlay |
| `Ctrl+C` / `q` | Quit (stops server gracefully, saves state) |

### 7.8 Error display

| Severity | Display |
|---|---|
| Missing prerequisite | Red badge + inline install hint on Detect tab |
| Build failure | Red line at bottom of build log + last error retained in scrollback |
| Server port conflict | Modal: "port 8080 is in use — change port or stop the other process" |
| Server crash | Footer badge turns red; log pane shows last stderr lines |
| Download failure | Inline error below progress bar; partial file retained for resume |
| Flag unsupported by binary | Yellow warning on Settings tab: "`--flash-attn` not available in this build" |

---

## 8. Observability

### 8.1 Structured logging

- All background tasks use `tracing` spans with `instrument` attributes.
- Log file written to `~/.local/share/lmml/lmml.log` at DEBUG level via
  `tracing_appender` (non-blocking, rolling daily, keep last 7 days).
- TUI log pane shows INFO and above only, filtered to the active tab's subsystem.
- On crash or unexpected exit, the log file path is printed to stderr so the user
  knows where to look.

### 8.2 Panic hook

```rust
// installed in main() before the TUI starts
std::panic::set_hook(Box::new(|info| {
    // restore terminal first so the panic message is readable
    let _ = crossterm::terminal::disable_raw_mode();
    let _ = crossterm::execute!(
        std::io::stderr(),
        crossterm::terminal::LeaveAlternateScreen
    );
    eprintln!("lmml crashed: {info}");
    eprintln!("Log file: {}", AppState::log_path().display());
}));
```

---

## 9. Dependency choices

| Purpose | Crate |
|---|---|
| Async runtime | `tokio` (full) |
| TUI framework | `ratatui` |
| Terminal backend | `crossterm` |
| Serialisation | `serde` + `toml` |
| HTTP (HF search + download + health poll) | `reqwest` (stream + json features) |
| System info | `sysinfo` |
| Disk space | `nix` (Linux/macOS `statvfs`) |
| Hashing (cmake fingerprint) | `sha2` |
| Error handling | `thiserror` (libraries), `anyhow` (binary) |
| Logging | `tracing` + `tracing-subscriber` + `tracing-appender` |
| Testing | `pretty_assertions`, `insta`, `tokio::test`, `tempfile` |
| Text wrapping | `textwrap` |

---

## 10. Error handling conventions

- Library crates use `thiserror` enums — typed, matchable, no `anyhow`.
- `lmml-tui` (the binary) uses `anyhow` for top-level `main` error propagation only.
- No `unwrap()` or `expect()` outside tests. Every `?` propagates a typed error.
- All probe / build / server failures are `Err`, not panics — rendered by the TUI.
- The panic hook (§8.2) ensures the terminal is always restored before any crash output.

---

## 11. Testing strategy

### Unit tests (per crate)

- **lmml-detect**: mock command runner trait; test `recommended_backend()` for every
  backend combination; test `compute_cap_to_arch` exhaustively; test the C++17 probe
  against a fake compiler that returns success/failure; test disk space enforcement.
- **lmml-compat**: test `build_argv` output for every capability combination; test
  `--help` parser against fixture strings from known llama.cpp versions.
- **lmml-build**: test cmake flag assembly for every backend; test fingerprint
  change detection; test partial-build recovery (binary absent/non-executable).
- **lmml-models**: test GGUF header parser against real fixture files (committed to
  `tests/fixtures/`); test VRAM fit calculation across all fit tiers; test registry
  scan with a `tempfile` directory; test HF search response parsing against fixture JSON.
- **lmml-server**: test port conflict detection; test health poller with a mock HTTP
  server; test argv assembly delegates to `lmml-compat`.
- **lmml-state**: round-trip serialise/deserialise `AppState`; test XDG path
  resolution under a custom `$XDG_CONFIG_HOME`; test `reset()`.

### TUI snapshot tests

One snapshot per tab × meaningful state combination:

| Tab | States captured |
|---|---|
| Detect | fresh / probing / complete (all green) / missing prereqs / CUDA warning |
| Build | idle / building (progress) / complete / failed / sccache active / update available |
| Models | empty / populated / VRAM fit badges / HF search open / downloading |
| Server | stopped / starting / ready / failed / port conflict |
| Settings | default / unsupported flag warning |
| First-run | onboarding modal step 1 / step 2 |

Run: `cargo test -p lmml-tui && cargo insta accept -p lmml-tui`

### Integration smoke tests

- Spawn a `llama-server` stub that prints nothing but responds `200 OK` to `/health`.
  Assert `ServerStatus::Ready` within 2 seconds.
- Spawn a stub that never starts. Assert `ServerStatus::Failed` with a timeout error.
- Spawn a stub that binds the target port before the manager tries. Assert port-conflict
  error with the correct message.

---

## 12. Milestones

| # | Deliverable | Crates |
|---|---|---|
| 1 | `lmml-detect` with full CUDA arch matrix, C++17 probe, disk check, unit tests | `lmml-detect` |
| 2 | `lmml-compat` with flag detection + argv assembly, unit tests | `lmml-compat` |
| 3 | `lmml-build` with sccache, fingerprint, update check, streaming | `lmml-build` |
| 4 | `lmml-state` with full schema, XDG paths, round-trip tests | `lmml-state` |
| 5 | `lmml-tui` skeleton: event loop, Action dispatch, tab routing, footer, `?` overlay | `lmml-tui` |
| 6 | TUI Detect tab: badges, CUDA arch list, install hints, first-run onboarding | `lmml-tui`, `lmml-detect` |
| 7 | TUI Build tab: streaming log, sccache badge, update check, clean-build action | `lmml-tui`, `lmml-build` |
| 8 | `lmml-models`: GGUF parse, VRAM fit, registry scan + TUI Models tab | `lmml-models`, `lmml-tui` |
| 9 | HF search + download with resume + progress bar in Models tab | `lmml-models`, `lmml-tui` |
| 10 | `lmml-server`: health polling, port conflict, argv via compat + TUI Server tab | `lmml-server`, `lmml-tui` |
| 11 | TUI Settings tab: all fields editable, flag-unsupported warnings | `lmml-tui`, `lmml-state`, `lmml-compat` |
| 12 | Observability: `tracing` spans, rolling log file, panic hook | all |
| 13 | Full snapshot test suite + CI | all |
| 14 | Model alias support (symlinks + external paths) | `lmml-models`, `lmml-tui` |

---

## 13. Niceties checklist

- [ ] No silent failures — every missing/incompatible tool shows a badge and install hint.
- [ ] CUDA sm_37 through sm_100a all detected and mapped to correct cmake arch flags.
- [ ] Multi-GPU: all unique sm_ values passed as a semicolon-separated list.
- [ ] CUDA toolkit × GPU compute_cap compatibility cross-checked before building.
- [ ] `sccache` auto-enabled when present; ~8 min → ~30 sec repeat builds.
- [ ] Partial/interrupted build detected and recovered from on startup.
- [ ] VRAM fit badge next to every model (full / partial + recommended ngl / too large).
- [ ] HF model search from within the TUI — no URL copy-pasting.
- [ ] Interrupted downloads resume from byte offset.
- [ ] Server readiness via `/health` polling — survives llama.cpp log format changes.
- [ ] Port conflict detected before spawning with a clear human error message.
- [ ] `--flash-attn`, `--mlock`, `--api-key`, `--batch-size` all exposed and persisted.
- [ ] Flags unsupported by the detected binary shown as warnings, not crashes.
- [ ] Update check with "Update and rebuild" flow; pin-to-tag vs track-main mode.
- [ ] First-run onboarding modal sequence — no blank screen on fresh install.
- [ ] Stateful — reopen TUI and last model + server config + build state are restored.
- [ ] Rolling debug log at `~/.local/share/lmml/lmml.log`; path printed on crash.
- [ ] Panic hook restores terminal before printing crash info.
- [ ] Clean exit — Ctrl+C stops server gracefully, saves state, no orphaned processes.
- [ ] `?` key shows all keybindings in a modal overlay.
- [ ] Confirm dialog before destructive actions (model delete, clean build).
- [ ] Snapshot CI catches any unintentional TUI regressions.
