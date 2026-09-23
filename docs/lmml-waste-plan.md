# LMML-WASTE Integration Plan (Revised for Workspace Structure)

## Executive Summary

Add WASTE as a second runtime backend to LMML, managing `.waste` containers alongside `.gguf` models. The integration follows a phased approach aligned with the actual workspace structure:

**Revised Phase Order:**
1. **External WASTE HTTP/server management** (Weeks 1-2) - Python serving prototype
2. **.waste model discovery and profiles** (Weeks 3-4) - Directory containers
3. **WASTE stats/log parsing through actual API** (Week 5) - ctx-aware FFI
4. **libwaste native Rust FFI** (Week 6) - Release path (self-contained)
5. **ROCm/VRAM WASTE fork as benchmark-gated research spike** (Weeks 7-8)
6. **Crown11 advisor as optional policy layer** (Week 9)
7. **llama.cpp-native bridge only after WASTE hybrid benchmarks justify it** (Future)

**Key Decisions:**
- Do NOT force WASTE into llama.cpp initially. Use LMML as the supervisor/router.
- Runtime backend belongs on `ModelRuntimeProfile`, NOT `BuildState`
- `.waste` directories are atomic model containers (do NOT recurse)
- Python serving is prototype-only; release path is `waste` binary or FFI

---

## Workspace Architecture

LMML is now a **10-crate workspace** under `crates/`:

```
crates/
├── lmml-state      # Persistent state (serde, toml)
├── lmml-models     # GGUF model registry
├── lmml-server     # llama-server lifecycle (tokio::process)
├── lmml-node       # Headless HTTP API worker (axum)
├── lmml-router     # LAN coordinator/load router
├── lmml-api        # Shared HTTP API DTOs
├── lmml-detect     # Hardware detection (sysinfo)
├── lmml-build      # llama.cpp build pipeline
├── lmml-compat     # llama.cpp flag compatibility
└── lmml-tui        # Terminal UI binary
```

**Integration Strategy:**
- **lmml-state**: Add backend/profile schema for WASTE
- **lmml-models**: Add `.waste` discovery and manifest parsing
- **lmml-server**: Generalize process manager or create new `lmml-runtime`
- **lmml-node/router**: Add backend capability advertisement and routing
- **lmml-tui**: Backend-aware display (only after CLI/runtime works)

---

## Phase 1: External WASTE HTTP/Server Management (Weeks 1-2)

### 1.1 Runtime Backend Schema (NOT Build Backend)

**File:** `crates/lmml-state/src/lib.rs`

**Critical Distinction:** `BuildState.backend` is for **llama.cpp build configuration only**. Using it for WASTE confuses "what llama.cpp was built for" with "what backend is serving."

**Solution:** Add `RuntimeBackend` to `ModelRuntimeProfile` and runtime state:
```rust
// New enum in lmml-state
pub enum RuntimeBackend {
    LlamaCpp,
    Waste,
    // Future: ExternalOpenAI, Vllm
}

// Extend ModelRuntimeProfile (lines 722-755)
pub struct ModelRuntimeProfile {
    pub name: String,
    pub model: PathBuf,
    pub backend: RuntimeBackend,  // NEW: runtime backend, not build backend

    // Backend-specific config
    pub llama_config: Option<ServerConfig>,
    pub waste_config: Option<WasteServerConfig>,
}
```

**Rationale:** Runtime backend belongs on `ModelRuntimeProfile`/runtime state, NOT on `[build]` in state.toml.

### 1.2 WASTE Server Process Manager

**New crate:** `crates/lmml-runtime/` (or extend `lmml-server`)

**Pattern:** Follow `lmml-server` async subprocess model:
- Use `tokio::process::Command` (NOT `std::process::Command`)
- Pipe stdout/stderr for async log streaming
- Health polling via HTTP (`/health`, `/v1/health`)
- `watch::Sender<ServerStatus>` for state broadcasting

**File:** `crates/lmml-runtime/src/waste.rs`

```rust
// Follow lmml-server patterns
pub struct WasteServerManager {
    pub binary: PathBuf,  // "python3" or "waste"
    pub model_path: PathBuf,
    pub config: WasteServerConfig,
}

impl WasteServerManager {
    pub async fn start(
        &self,
        timeout: Duration,
    ) -> Result<WasteServerHandle, WasteServerError> {
        // Build command: python3 -m serve /path/to/model.waste --port 8000
        // OR: waste serve /path/to/model.waste --port 8000 (if CLI exists)
        let mut child = tokio::process::Command::new(&self.binary)
            .args(self.build_argv())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        // Spawn log readers (follow lmml-server pattern)
        if let Some(stdout) = child.stdout.take() {
            spawn_log_reader(stdout, log_tx.clone());
        }
        if let Some(stderr) = child.stderr.take() {
            spawn_log_reader(stderr, log_tx.clone());
        }

        Ok(WasteServerHandle { /* ... */ })
    }
}

pub struct WasteServerHandle {
    inner: Arc<WasteServerInner>,
}

impl WasteServerHandle {
    pub fn status(&self) -> ServerStatus;
    pub fn subscribe(&self) -> watch::Receiver<ServerStatus>;
    pub async fn stop(&self);  // SIGTERM, wait 5s, then SIGKILL
}
```

### 1.3 WASTE Server Config

**File:** `crates/lmml-runtime/src/waste.rs`

```rust
pub struct WasteServerConfig {
    pub model_path: PathBuf,
    pub port: u16,
    pub host: String,
    pub ram_budget_gb: usize,
    pub threads: usize,
    // Phase 3 additions:
    pub gpu_resident: Option<GpuResidentMode>,
    pub gpu_cache_gb: Option<usize>,
    pub io_staging_gb: Option<usize>,
    pub gpu_backend: Option<String>,
}

pub enum GpuResidentMode {
    None,
    Trunk,
    HotExperts,
    Hybrid,
}
```

### 1.4 WASTE Server Launch Command (Prototype vs. Release)

**Current upstream pattern** (from WASTE README):
```bash
python3 -m serve ~/models/k3.waste --port 8000
```

**Prototype Path (Phase 1-2):** Use `python3 -m serve`
- **Pros:** Quick to implement, no binary packaging needed
- **Cons:** Requires Python runtime dependency (conflicts with LMML's "single self-contained binary" target)

**Release Path (Phase 4+):** Fork WASTE to add `waste serve` companion or embed libwaste in Rust
```bash
# Option A: Fork WASTE to add `waste serve` CLI command
waste serve /path/to/model.waste --port 8000 --budget 46G

# Option B: Embed libwaste and implement serving in Rust (lmml-runtime)
# LMML spawns its own server process using libwaste FFI
```
- **Pros:** Self-contained binary, matches LMML's design goal
- **Cons:** Requires WASTE fork or Rust FFI integration

**Implementation Strategy:**
1. **Phase 1-2:** Use `python3 -m serve` for prototype
2. **Phase 4:** Migrate to `waste` binary or libwaste FFI for release

**Code:**
```rust
impl WasteServerManager {
    fn build_argv(&self) -> Vec<String> {
        // Prototype: python3 -m serve
        if cfg!(feature = "prototype-python") {
            vec![
                "-m".to_string(),
                "serve".to_string(),
                self.model_path.to_string_lossy().to_string(),
                "--port".to_string(),
                self.config.port.to_string(),
            ]
        } else {
            // Release: fork-provided `waste serve`
            vec![
                "serve".to_string(),
                self.model_path.to_string_lossy().to_string(),
                "--port".to_string(),
                self.config.port.to_string(),
            ]
        }
    }
}
```

**Explicit Label:** Python serving is **prototype-only**; final release path is `waste` binary or libwaste embedding.

### 1.5 CLI Integration

**New subcommands in `lmml-tui`:**
```bash
# WASTE server management
lmml waste start <model-path> --port 8000
lmml waste stop
lmml waste status

# Preflight checks
lmml waste doctor
```

---

## Phase 2: .waste Model Discovery & Profiles (Weeks 3-4)

### 2.1 Model Format Extension - .waste as Directory Containers

**File:** `crates/lmml-models/src/lib.rs`

**Critical:** `.waste` containers are **directories**, not files. LMML's parser rejects non-files and recurses into directories (line 486, 605).

**Current behavior:**
```rust
// parse_model_file() - line 486
if !metadata.is_file() {
    return None;  // <-- Rejects directories
}

// scan_path() - line 605
// Recurses into directories, skipping files starting with '.'
```

**Solution:** Treat `*.waste/` as **terminal model containers** before recursion:
```rust
pub async fn parse_model_file(path: impl AsRef<Path>, aliased: bool) -> Option<ModelEntry> {
    let path = path.as_ref();
    let metadata = tokio::fs::metadata(path).await.ok()?;

    // Check for .waste directory FIRST (before file check)
    if metadata.is_dir() {
        if path.file_name()?.to_string_lossy().ends_with(".waste") {
            // Treat as atomic WASTE container - do NOT recurse
            return parse_waste_container(path, aliased).await;
        }
        // Otherwise, recurse into directory (existing behavior)
        return None;
    }

    if !metadata.is_file() {
        return None;
    }

    // Existing GGUF logic...
    if path.extension().is_none_or(|ext| ext != "gguf") {
        return None;
    }
    // ...
}
```

**Key Change:** `.waste` directories are **atomic model containers** - do NOT recurse into them looking for GGUF files.

### 2.2 WASTE Model Info Parsing (info + plan)

**File:** `crates/lmml-models/src/waste.rs` (new)

**Pattern:** Use `waste info --json` for engine/model facts, `waste plan --json` for RAM budget:
```rust
pub async fn parse_waste_metadata(path: impl AsRef<Path>) -> Result<ModelEntry, WasteModelError> {
    let path = path.as_ref();

    // Step 1: Get engine/model facts from `waste info --json`
    let info_output = tokio::process::Command::new("waste")
        .args(["info", "--json"])
        .arg(path)
        .output()
        .await?;
    let info: WasteInfo = serde_json::from_slice(&info_output.stdout)?;

    // Step 2: Get RAM budget from `waste plan --json`
    let plan_output = tokio::process::Command::new("waste")
        .args(["plan", "--json"])
        .arg(path)
        .output()
        .await?;
    let plan: WastePlan = serde_json::from_slice(&plan_output.stdout)?;

    // Step 3: Calculate container size via directory walk
    let container_size = walk_directory_size(path).await?;

    Ok(ModelEntry {
        path: path.to_path_buf(),
        name: waste_model_name(path),
        size_bytes: container_size,  // From directory walk
        format: ModelFormat::WasteContainer,
        quant: info.quantization,
        context_length: Some(plan.ctx),
        architecture: Some(info.arch),
        ram_budget_gb: Some(bytes_to_gib(plan.recommended_bytes).ceil() as usize),
        aliased: false,
    })
}

#[derive(Deserialize)]
struct WasteInfo {
    engine: String,              // WASTE engine version string
    arch: String,                // model architecture
    layers: u32,                 // number of layers
    experts: u32,                // number of experts (for MoE)
    top_k: u32,                  // top-k routing
    hidden: u32,                 // hidden size
    params_total: u64,           // total language-model parameters
    params_active: u64,          // parameters touched per token
    quantization: String,        // quantization format
    expert_cache_bytes: u64,     // resolved expert cache size in bytes
    // ... other engine/model facts from waste info --json
}

#[derive(Deserialize)]
struct WastePlan {
    ctx: u32,                    // context tokens used for the plan
    trunk_bytes: u64,            // resident trunk bytes
    state_bytes: u64,            // recurrent state + KV bytes
    scratch_bytes: u64,          // scratch/activation bytes
    min_expert_cache: u64,       // minimum expert cache bytes
    floor_bytes: u64,            // Minimum RAM floor in bytes
    recommended_bytes: u64,      // Recommended RAM in bytes
    physical_ram_bytes: u64,     // detected physical RAM, or 0 if unknown
    // ... other planning facts from waste plan --json
}

// Helper to convert bytes to GiB for display
fn bytes_to_gib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0 * 1024.0)
}

fn waste_model_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.strip_suffix(".waste").unwrap_or(name))
        .unwrap_or("waste-model")
        .to_string()
}

async fn walk_directory_size(path: &Path) -> Result<u64, WasteModelError> {
    // Walk directory to calculate total size
    // (container size is sum of all files in .waste/)
    todo!()
}
```

**Key Split:**
- `waste info --json` → engine/model facts (`engine`, `arch`, layer/expert counts, parameter counts, quantization)
- `waste plan --json` → RAM planning (`ctx`, `floor_bytes`, `recommended_bytes`, `physical_ram_bytes`)
- Directory walk → container_size_bytes

### 2.3 Profile Schema Extension

**File:** `crates/lmml-state/src/lib.rs`

**Current:** `ModelRuntimeProfile` (lines 722-755):
```rust
pub struct ModelRuntimeProfile {
    pub name: String,
    pub model: PathBuf,
    pub server: ServerConfig,  // llama-server specific
}
```

**Extend:** Add backend-specific config:
```rust
pub struct ModelRuntimeProfile {
    pub name: String,
    pub model: PathBuf,
    pub backend: String,  // "llama_cpp" or "waste"

    // Backend-specific config
    pub llama_config: Option<ServerConfig>,
    pub waste_config: Option<WasteServerConfig>,
}
```

**TOML example:**
```toml
[[model.profiles]]
name = "k3-waste"
model = "/models/k3.waste"
backend = "waste"

[model.profiles.waste_config]
port = 1200
ram_budget_gb = 46
threads = 20
```

### 2.4 Model List CLI Enhancement

**Enhanced command:**
```bash
lmml models list
```

**Expected output:**
```
NAME                    FORMAT    SIZE        CONTEXT  RAM BUDGET
──────────────────────────────────────────────────────────────────
llama-3.2-gguf.gguf     GGUF      8.2 GiB     8192     -
k3.waste                WASTE     982 GiB     16384    46 GiB  # K3 is ~982 GiB container
mistral-7b.gguf         GGUF      4.1 GiB     4096     -
```

**Note:** K3 is ~982 GiB container (not 27.1 GiB as originally planned). The 27 GiB is the resident trunk.

---

## Phase 3: WASTE Stats/Log Parsing (Week 5)

### 3.1 WASTE Stats via C API (ctx-aware)

**File:** `crates/lmml-runtime/src/waste_stats.rs`

**Pattern:** Use `waste_get_stats(ctx, out)` from the public C API:
```rust
// From https://raw.githubusercontent.com/sqliteai/waste/main/src/waste.h
// Correct signature: waste_get_stats(const waste_ctx *ctx, waste_stats *out)
use std::os::raw::c_int;

#[repr(C)]
#[derive(Default)]
struct RawWasteStats {
    tokens_generated: u64,
    experts_hit: u64,
    experts_missed: u64,
    bytes_read: u64,
    sec_total: f64,
    sec_io: f64,
    direct_io: c_int,
}

#[repr(C)]
struct WasteCtx;  // Opaque context handle

extern "C" {
    fn waste_get_stats(ctx: *const WasteCtx, out: *mut RawWasteStats) -> c_int;
}

pub struct WasteRuntimeStats {
    pub tokens_per_second: f64,
    pub expert_hit_rate: f64,
    pub bytes_read: u64,
    pub read_gb_per_token: f64,
    pub io_seconds: f64,
    pub direct_io: bool,
}

pub async fn fetch_waste_stats(handle: &WasteServerHandle) -> Result<WasteRuntimeStats, WasteError> {
    // In HTTP-server mode, LMML does NOT have the waste_ctx
    // Options:
    // 1. Parse log output for stats lines (Phase 1-2)
    // 2. HTTP endpoint if WASTE exposes one (Phase 1-2)
    // 3. Embed libwaste directly with FFI (Phase 4)
    todo!()
}
```

**Key Correction:** `waste_get_stats()` requires a `waste_ctx` parameter. In HTTP-server mode, LMML doesn't own the context, so stats must come from logs or HTTP endpoint unless LMML embeds libwaste directly.

### 3.2 Log Parsing

**Pattern:** Parse the managed WASTE HTTP runtime log output for key metrics:
```rust
pub fn parse_waste_log_line(line: &str) -> Option<WasteLogEvent> {
    if line.contains("experts") && line.contains("hit") {
        Some(WasteLogEvent::ExpertCacheSummary)
    } else if line.contains("tokens/s") {
        Some(WasteLogEvent::TokensPerSecond)
    } else {
        None
    }
}
```

---

## Phase 4: libwaste Native Rust FFI (Release Path, Week 6)

### 4.1 FFI Binding

**File:** `crates/lmml-runtime/src/waste_ffi.rs`

**Pattern:** Bind `libwaste.so` via Rust FFI with ctx-aware API:
```rust
// From upstream: waste_status waste_get_stats(const waste_ctx *ctx, waste_stats *out)
// Fields: tokens_generated, experts_hit, experts_missed, bytes_read, sec_total, sec_io, direct_io
use std::os::raw::c_int;

#[repr(C)]
#[derive(Default)]
struct WasteStats {
    tokens_generated: u64,
    experts_hit: u64,
    experts_missed: u64,
    bytes_read: u64,
    sec_total: f64,
    sec_io: f64,
    direct_io: c_int,
}

#[repr(C)]
struct WasteCtx;  // Opaque context handle

extern "C" {
    fn waste_get_stats(ctx: *const WasteCtx, out: *mut WasteStats) -> WasteStatus;
}

type WasteStatus = c_int;
const WASTE_OK: WasteStatus = 0;
// Upstream error codes are negative: WASTE_E_IO = -1, WASTE_E_FORMAT = -2,
// WASTE_E_RAM_BUDGET = -3, WASTE_E_OOM = -4, WASTE_E_ARG = -5, etc.

pub fn get_waste_stats(ctx: &WasteCtx) -> Option<WasteStats> {
    unsafe {
        let mut stats = WasteStats::default();
        let status = waste_get_stats(ctx as *const _, &mut stats as *mut _);
        if status == WASTE_OK {
            Some(stats)
        } else {
            None
        }
    }
}
```

**Note:** In HTTP-server mode (Phase 1-2), LMML doesn't own the `waste_ctx`, so this FFI is most useful when LMML embeds libwaste directly (Phase 4 release path).

---

## Phase 5: ROCm/VRAM WASTE Fork (Weeks 7-8)

### 5.1 VRAM + RAM Design

**Target Hardware:** AMD R9700 (32 GiB VRAM)

**Memory Layout Strategy:**
```
NVMe .waste banks
    → pinned host RAM read-ahead slabs
    → RAM expert cache
    → optional VRAM hot-expert/trunk cache
    → HIP kernels for selected expert matvec/dequant
```

**Key Insight:** K3's WASTE trunk is ~27 GiB resident. On a 32 GiB R9700, putting the trunk in VRAM leaves too little room for:
- Scratch buffers
- Activations
- KV/recurrent state
- Expert cache

**Recommended Layout:** RAM trunk + RAM expert cache + VRAM hot expert promotion + HIP matvec

### 5.2 WASTE Fork Configuration

**New CLI flags:**
```bash
python3 -m serve /path/to/model.waste \
  --gpu-resident=hybrid \
  --gpu-cache-gb=8 \
  --ram-budget-gb=46 \
  --io-staging-gb=4 \
  --gpu-backend=rocm
```

**Flag definitions:**
| Flag | Values | Default | Description |
|------|--------|---------|-------------|
| `--gpu-resident` | `none`, `trunk`, `hot-experts`, `hybrid` | `none` | What to keep in VRAM |
| `--gpu-cache-gb` | N (GiB) | 0 | VRAM cache size for hot experts |
| `--ram-budget-gb` | N (GiB) | auto | Total RAM budget for WASTE |
| `--io-staging-gb` | N (GiB) | 2 | Pinned RAM for read-ahead |
| `--gpu-backend` | `rocm`, `cuda`, `vulkan` | auto | GPU backend |

### 5.3 Benchmark-Gated Decision

**Success Criteria:**
- VRAM cache improves tokens/s by ≥20%
- Hybrid mode uses ≤8 GiB VRAM for hot experts
- Pinned RAM staging reduces I/O latency

**Decision Point:** If benchmarks don't meet criteria, skip the ROCm fork and stick with CPU-only WASTE.

---

## Phase 6: Crown11 Advisor (Week 9)

### 6.1 Crown11 Use Case

**Purpose:** Policy/planning advisor, NOT kernel acceleration.

**Crown11 Capabilities** (from `~/repos/crown11_module`):
- K11 validation
- Operator DAGs
- L1/L2/L4 logic
- `GpuCache` concept (policy, not implementation)

**Note:** Crown11 is crate `crown-ai` (proprietary in Cargo.toml). Keep as optional/local advisor until API and licensing are clarified.

### 6.2 Runtime Placement Advisor

**File:** `crates/lmml-tui/src/advisor.rs` (new)

```rust
pub struct RuntimeAdvisor;

impl RuntimeAdvisor {
    pub fn recommend_backend(model_ram_gb: usize, vram_gb: usize) -> &str {
        if model_ram_gb > vram_gb * 2 {
            "waste"  // Large models benefit from WASTE streaming
        } else {
            "llama_cpp"  // Smaller models fit in VRAM
        }
    }

    pub fn recommend_gpu_resident_mode(model_ram_gb: usize, vram_gb: usize) -> GpuResidentMode {
        if vram_gb >= model_ram_gb + 8 {
            GpuResidentMode::Trunk  // Plenty of VRAM
        } else if vram_gb >= 8 {
            GpuResidentMode::HotExperts  // Moderate VRAM
        } else {
            GpuResidentMode::Hybrid  // Limited VRAM
        }
    }
}
```

### 6.3 Profile Advisor CLI

```bash
lmml advisor recommend --model /models/k3.waste
```

**Expected output:**
```
Recommended backend: waste
Recommended GPU resident mode: hybrid
Recommended VRAM cache: 8 GiB
Recommended RAM budget: 46 GiB
Recommended IO staging: 4 GiB

Reasoning:
- Model size (982 GiB container, 27 GiB resident) exceeds available VRAM (32 GiB)
- NVMe storage detected, suitable for streaming
- AMD ROCm backend available
```

---

## Phase 7: Native llama.cpp Bridge (Future)

### 7.1 Bridge Strategy

**Order of integration:**

1. **HTTP bridge** (Phase 1-2) ✅
   - LMML routes both `llama-server` and the managed WASTE HTTP runtime
   - Clients see unified OpenAI-compatible API

2. **llama-server-compatible WASTE personality** (Phase 3)
   - WASTE fork or embedded Rust server matches llama.cpp server endpoints/metrics/health
   - LMML and clients don't care which backend

3. **Native llama.cpp adapter** (Phase 7, later)
   - C/C++ shim exposing llama-like context API backed by `libwaste`
   - Only after benchmarks justify the complexity

### 7.2 Why Not Native First?

**Challenges:**
- WASTE is not just another quantization format
- WASTE changes execution around disk-streamed MoE expert records
- llama.cpp/ggml expects tensors and graph execution in a different shape
- Modifying ggml internals is high effort

**Recommendation:** Avoid modifying ggml internals until the WASTE hybrid backend proves itself.

---

## Implementation Checklist

### Phase 1: External WASTE HTTP/Server Management

- [ ] Add `RuntimeBackend` enum to `lmml-state`
- [ ] Add `backend: RuntimeBackend` field to `ModelRuntimeProfile`
- [ ] Create `crates/lmml-runtime/` or extend `lmml-server`
- [ ] Implement `WasteServerManager` following `lmml-server` patterns
- [ ] Implement `WasteServerHandle` with `watch::Sender<ServerStatus>`
- [ ] Use `python3 -m serve` for prototype (Phase 1-2)
- [ ] Add CLI: `lmml waste start/stop/status`
- [ ] Add CLI: `lmml waste doctor`

### Phase 2: .waste Model Discovery & Profiles

- [ ] Extend `parse_model_file()` in `lmml-models` to accept `.waste`
- [ ] Add `ModelFormat` enum (optional)
- [ ] Create `crates/lmml-models/src/waste.rs` with `parse_waste_metadata()`
- [ ] Extend `ModelRuntimeProfile` with backend field
- [ ] Add `WasteServerConfig` to `lmml-state`
- [ ] Enhance `lmml models list` to show format and RAM budget

### Phase 3: WASTE Stats/Log Parsing

- [ ] Implement `fetch_waste_stats()` via C API or HTTP
- [ ] Parse managed WASTE HTTP runtime log output for metrics
- [ ] Display stats in TUI server screen

### Phase 4: libwaste FFI (Release Path)

- [ ] Bind `libwaste.so` via Rust FFI
- [ ] Implement `waste_get_stats(ctx, out)` wrapper (ctx-aware)
- [ ] Migrate from Python serving to `waste` binary or FFI
- [ ] Compare FFI performance vs. log parsing

### Phase 5: ROCm/VRAM WASTE Fork

- [ ] Fork WASTE repo with ROCm backend support
- [ ] Add `--gpu-resident` CLI flag
- [ ] Add `--gpu-cache-gb` CLI flag
- [ ] Add `--io-staging-gb` CLI flag
- [ ] Add `--gpu-backend` CLI flag
- [ ] Implement pinned RAM staging in WASTE
- [ ] Implement VRAM cache policy in WASTE
- [ ] Benchmark: tokens/s improvement ≥20%

### Phase 6: Crown11 Advisor

- [ ] Add `crown-ai` as optional dependency
- [ ] Create `RuntimeAdvisor` in `lmml-tui`
- [ ] Implement backend recommendation logic
- [ ] Add `lmml advisor recommend` CLI command
- [ ] Integrate advisor into TUI (optional screen or modal)

### Phase 7: Native Bridge (Future)

- [ ] Benchmark HTTP bridge performance
- [ ] Profile llama.cpp ggml internals
- [ ] Design C/C++ shim API
- [ ] Implement native adapter (if justified)

---

## TUI Integration Plan

### Server Screen

**Current:** Shows llama-server metrics (VRAM, GPU layers, KV cache)

**Enhanced:** Show backend-specific metrics based on `ModelRuntimeProfile.backend` or live runtime state

```rust
// crates/lmml-tui/src/server.rs
match active_profile.backend {
    RuntimeBackend::LlamaCpp => render_llama_metrics(&metrics.backend_specific),
    RuntimeBackend::Waste => render_waste_metrics(&metrics.backend_specific),
}
```

**Key:** Use `ModelRuntimeProfile.backend` or live runtime state, NOT `BuildState.backend`.

**WASTE metrics display:**
```
┌─────────────────────────────────────────────────┐
│ WASTE Server: k3.waste                          │
├─────────────────────────────────────────────────┤
│ RAM Budget:        46 GiB / 64 GiB              │
│ Expert Cache:      12 GiB (85% hit rate)        │
│ Tokens/s:          0.52                         │  # K3 on consumer setup
│ Read GB/token:     0.8                          │
│ GPU Resident:      hybrid                       │
│ GPU Cache:         8 GiB                        │
└─────────────────────────────────────────────────┘
```

**Note:** K3 reports ~0.49-0.52 tok/s on published consumer-machine setup (not 12.5 tok/s as originally planned). Use Kimi-Linear for fast examples.

### Models Screen

**Current:** Shows `.gguf` models

**Enhanced:** Show format badge and RAM budget

```
┌─────────────────────────────────────────────────────────┐
│ Models                                                  │
├──────────────┬─────────┬─────────┬──────────┬───────────┤
│ Name         │ Format  │ Size    │ Context  │ RAM Budget│
├──────────────┼─────────┼─────────┼──────────┼───────────┤
│ llama-3.2    │ GGUF    │ 8.2 GiB │ 8192     │ -         │
│ k3           │ WASTE   │ 982 GiB │ 16384    │ 46 GiB    │
│ kimini-linear│ WASTE   │ 12 GiB  │ 8192     │ 8 GiB     │
│ mistral-7b   │ GGUF    │ 4.1 GiB │ 4096     │ -         │
└──────────────┴─────────┴─────────┴──────────┴───────────┘
```

### Build Screen

**Current:** Shows llama.cpp build

**Enhanced:** Add WASTE build option

```
┌─────────────────────────────────────────────────┐
│ Build                                           │
├─────────────────────────────────────────────────┤
│ Backend: [llama.cpp] [WASTE]                    │
│                                                 │
│ Hardware Detection:                             │
│ ✓ OS: Linux x86_64                              │
│ ✓ GPU: AMD Radeon RX 7900 XTX (24 GB)          │
│ ✓ RAM: 64 GB                                    │
│ ✓ Storage: NVMe                                 │
│                                                 │
│ [Build llama.cpp]  [Build WASTE]                │
└─────────────────────────────────────────────────┘
```

---

## Risk Assessment

### High Risk

1. **Python serving vs. self-contained binary**
   - **Risk:** `python3 -m serve` conflicts with LMML's "single self-contained binary" target
   - **Mitigation:** Label Python as prototype-only; migrate to `waste` binary or FFI in Phase 4

2. **Model sizing confusion**
   - **Risk:** K3 is 982 GiB container, not 27 GiB (27 GiB is resident trunk)
   - **Mitigation:** Clarify in docs, use Kimi-Linear for fast examples

3. **VRAM cache policy tuning**
   - **Risk:** Poor cache policy could degrade performance
   - **Mitigation:** Benchmark-gated decision, skip if criteria not met

### Medium Risk

1. **HTTP API compatibility**
   - **Risk:** The managed WASTE HTTP runtime may not fully match OpenAI API
   - **Mitigation:** Test with real clients early

2. **Storage I/O bottlenecks**
   - **Risk:** WASTE streaming may suffer on slow storage
   - **Mitigation:** Add storage warning in `lmml waste doctor`

3. **Crown11 API/licensing**
   - **Risk:** Crown11 is proprietary crate `crown-ai`, API may not compile as written
   - **Mitigation:** Keep as optional/local advisor until clarified

### Low Risk

1. **Runtime backend schema migration**
   - **Risk:** Existing model profiles must continue to deserialize as llama.cpp profiles.
   - **Mitigation:** Add `RuntimeBackend::LlamaCpp` as the serde default and preserve legacy `server` fields during migration.

---

## Success Metrics

### Phase 1
- [ ] Can start/stop WASTE runtime via CLI
- [ ] Health check returns 200 for the prototype Python WASTE server
- [ ] Log streaming works via `mpsc::Sender<String>`
- [ ] Python serving works as prototype path

### Phase 2
- [ ] Can list `.waste` models alongside `.gguf`
- [ ] Profile can specify backend
- [ ] TUI shows correct model format

### Phase 3
- [ ] Stats endpoint or log parsing returns valid data
- [ ] TUI displays WASTE metrics

### Phase 4 (Release Path)
- [ ] FFI binding compiles and returns stats
- [ ] Performance comparable to log parsing

### Phase 5
- [ ] VRAM cache improves tokens/s by ≥20%
- [ ] Hybrid mode uses ≤8 GiB VRAM for hot experts
- [ ] Pinned RAM staging reduces I/O latency

### Phase 6
- [ ] Crown11 advisor recommends correct backend
- [ ] Profile recommendation matches user preference

### Phase 7
- [ ] Native adapter matches HTTP bridge performance
- [ ] ggml integration doesn't break existing workflows

---

## Appendix: Example Config Files

### ~/.lmml/config.toml

```toml
[general]
model_dirs = ["~/.lmml/models", "/models"]
default_model = ""
theme = "auto"

[build]
llama_cpp_path = "~/.lmml/build/llama.cpp"
waste_path = "~/.lmml/build/waste"
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

[[model.profiles]]
name = "llama-3.2"
model = "~/.lmml/models/llama-3.2.gguf"
backend = "llama_cpp"

[model.profiles.llama_config]
port = 8080
gpu_layers = 99
context_size = 8192

[[model.profiles]]
name = "k3-waste"
model = "/models/k3.waste"
backend = "waste"

[model.profiles.waste_config]
port = 1200
ram_budget_gb = 46
threads = 20
gpu_resident = "hybrid"
gpu_cache_gb = 8
```

### ~/.lmml/state.toml

```toml
[build]
backend = "Auto"  # llama.cpp build backend (NOT WASTE)
archs = []
sccache_used = false

[model]
last_model = "k3-waste"
server_was_running = true

[[model.profiles]]
name = "k3-waste"
model = "/models/k3.waste"
backend = "Waste"  # Runtime backend, NOT build backend

[model.profiles.waste_config]
port = 1200
ram_budget_gb = 46
threads = 20
```

**Key Distinction:**
- `[build].backend` → llama.cpp build configuration (Auto, Cuda, Metal, etc.)
- `[[model.profiles]].backend` → Runtime backend for serving (LlamaCpp, Waste)

---

## Implementation Gates (Revised)

**First Implementation Gates (Plan Coverage):**

1. ⬜ **Add WASTE as RuntimeBackend** - Plan adds `RuntimeBackend` enum to `ModelRuntimeProfile`
2. ⬜ **Treat .waste as atomic containers** - Plan updates `parse_model_file()` to handle `.waste/` directories
3. ⬜ **Use waste info + waste plan for metadata** - Plan splits into two CLI calls
4. ⬜ **Label Python serving as prototype** - Plan explicitly labels Python as prototype, release path in Phase 4
5. ⬜ **FFI with ctx-aware API** - Plan corrects `waste_get_stats(ctx, out)` signature

**Plan coverage verified. Ready for Phase 1 implementation.**
