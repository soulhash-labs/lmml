//! Persistent state storage for lmml.
//!
//! This crate owns the `state.toml` schema described in the v2 architecture
//! plan. It persists build, model, server, and cached system profile state under
//! the XDG config directory so the TUI can restore the last session quickly.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

const APP_DIR_NAME: &str = "lmml";
const STATE_FILE_NAME: &str = "state.toml";
const LOG_FILE_NAME: &str = "lmml.log";
const OPENCODE_PROFILE: &str = "opencode";
const OPENCODE_FAST_PROFILE: &str = "opencode-fast";

/// Complete persisted application state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppState {
    /// llama.cpp build state.
    pub build: BuildState,
    /// Local model registry state.
    pub model: ModelState,
    /// llama-server runtime configuration.
    pub server: ServerConfig,
    /// Managed runtime profiles for coding harnesses.
    pub runtime: RuntimeConfig,
    /// Cached system profile summary from the detection crate.
    pub system_profile: Option<SystemProfile>,
}

impl AppState {
    /// Load state from the default XDG path, creating defaults if missing.
    pub fn load() -> Result<Self, StateError> {
        Self::load_from_path(Self::path())
    }

    /// Load state from the default XDG path without creating it when missing.
    pub fn load_existing_or_default() -> Result<Self, StateError> {
        Self::load_existing_or_default_from_path(Self::path())
    }

    /// Save state to the default XDG path.
    pub fn save(&self) -> Result<(), StateError> {
        self.save_to_path(Self::path())
    }

    /// Return the default state path, respecting `$XDG_CONFIG_HOME`.
    pub fn path() -> PathBuf {
        state_path_from_env(
            env::var_os("XDG_CONFIG_HOME"),
            env::var_os("HOME"),
            env::var_os("USERPROFILE"),
        )
    }

    /// Return the default debug log path, respecting `$XDG_DATA_HOME`.
    pub fn log_path() -> PathBuf {
        default_data_dir_from_env(
            env::var_os("XDG_DATA_HOME"),
            env::var_os("HOME"),
            env::var_os("USERPROFILE"),
        )
        .join(LOG_FILE_NAME)
    }

    /// Return the default runtime log directory, respecting `$XDG_STATE_HOME`.
    pub fn runtime_log_dir() -> PathBuf {
        default_state_dir_from_env(
            env::var_os("XDG_STATE_HOME"),
            env::var_os("HOME"),
            env::var_os("USERPROFILE"),
        )
        .join("runtime")
    }

    /// Reset the default state file to default values.
    pub fn reset() -> Result<(), StateError> {
        let state = Self::default();
        state.save()
    }

    /// Load state from a specific path, creating defaults if missing.
    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, StateError> {
        let path = path.as_ref();
        if !path.exists() {
            let state = Self::default();
            state.save_to_path(path)?;
            return Ok(state);
        }

        let content = fs::read_to_string(path).map_err(|source| StateError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let mut state: Self = toml::from_str(&content).map_err(|source| StateError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
        state.repair_legacy_build_binary();
        Ok(state)
    }

    /// Load state from a specific path without creating it when missing.
    pub fn load_existing_or_default_from_path(path: impl AsRef<Path>) -> Result<Self, StateError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(path).map_err(|source| StateError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let mut state: Self = toml::from_str(&content).map_err(|source| StateError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
        state.repair_legacy_build_binary();
        Ok(state)
    }

    /// Save state to a specific path.
    pub fn save_to_path(&self, path: impl AsRef<Path>) -> Result<(), StateError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| StateError::CreateDir {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let content = toml::to_string_pretty(self).map_err(StateError::Serialize)?;
        fs::write(path, content).map_err(|source| StateError::Write {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Reset a specific state file to default values.
    pub fn reset_path(path: impl AsRef<Path>) -> Result<(), StateError> {
        Self::default().save_to_path(path)
    }

    fn repair_legacy_build_binary(&mut self) {
        let legacy = legacy_server_binary(&self.build.source_dir);
        if self.build.binary == legacy {
            self.build.binary = expected_server_binary(&self.build.source_dir);
        }
    }
}

/// Persisted llama.cpp build state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BuildState {
    /// Source checkout directory.
    pub source_dir: PathBuf,
    /// Built `llama-server` binary path.
    pub binary: PathBuf,
    /// Resolved source commit hash.
    pub commit: String,
    /// Hex-encoded SHA-256 hash of the CMake invocation.
    pub cmake_hash: String,
    /// Persisted backend name, such as `Cuda`, `Metal`, or `CpuAvx2`.
    pub backend: String,
    /// CUDA architectures used for the build.
    pub archs: Vec<String>,
    /// Whether sccache was injected into the last build.
    pub sccache_used: bool,
    /// Timestamp string for the last completed build.
    pub last_built: String,
    /// Source tracking mode.
    pub track_mode: TrackMode,
}

impl Default for BuildState {
    fn default() -> Self {
        let data_dir = default_data_dir_from_env(
            env::var_os("XDG_DATA_HOME"),
            env::var_os("HOME"),
            env::var_os("USERPROFILE"),
        );
        let source_dir = data_dir.join("llama.cpp");
        Self {
            binary: expected_server_binary(&source_dir),
            source_dir,
            commit: String::new(),
            cmake_hash: String::new(),
            backend: "Auto".to_string(),
            archs: Vec::new(),
            sccache_used: false,
            last_built: String::new(),
            track_mode: TrackMode::Main,
        }
    }
}

/// Source tracking mode for llama.cpp updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackMode {
    /// Track upstream `main`.
    Main,
    /// Pin to a selected release tag.
    Tag,
}

/// Persisted model registry state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelState {
    /// Last selected model path.
    pub last_used: PathBuf,
    /// Default model directory.
    pub models_dir: PathBuf,
    /// External model paths or directories.
    pub aliases: Vec<PathBuf>,
    /// Per-model server profiles that override global server settings.
    pub profiles: Vec<ModelRuntimeProfile>,
    /// Active model runtime profile name.
    pub active_profile: String,
}

impl Default for ModelState {
    fn default() -> Self {
        let data_dir = default_data_dir_from_env(
            env::var_os("XDG_DATA_HOME"),
            env::var_os("HOME"),
            env::var_os("USERPROFILE"),
        );
        Self {
            last_used: PathBuf::new(),
            models_dir: data_dir.join("models"),
            aliases: Vec::new(),
            profiles: Vec::new(),
            active_profile: String::new(),
        }
    }
}

impl ModelState {
    /// Return the configured runtime profile for a model path.
    pub fn runtime_profile_for_path(&self, path: &Path) -> Option<&ModelRuntimeProfile> {
        let mut matching = self.runtime_profiles_for_path(path);
        if matching.is_empty() {
            return None;
        }
        matching
            .iter()
            .copied()
            .find(|profile| profile.name == self.active_profile)
            .or_else(|| matching.drain(..).next())
    }

    /// Return all configured runtime profiles for a model path.
    pub fn runtime_profiles_for_path(&self, path: &Path) -> Vec<&ModelRuntimeProfile> {
        self.profiles
            .iter()
            .filter(|profile| profile.matches_model_path(path))
            .collect()
    }

    /// Cycle to the next runtime profile that matches a model path.
    pub fn cycle_runtime_profile_for_path(&mut self, path: &Path) -> Option<&ModelRuntimeProfile> {
        let matching_names: Vec<String> = self
            .profiles
            .iter()
            .filter(|profile| profile.matches_model_path(path))
            .map(|profile| profile.name.clone())
            .collect();
        if matching_names.is_empty() {
            return None;
        }

        let next_index = matching_names
            .iter()
            .position(|name| name == &self.active_profile)
            .map(|index| (index + 1) % matching_names.len())
            .unwrap_or(0);
        self.active_profile = matching_names[next_index].clone();
        self.runtime_profile_for_path(path)
    }

    /// Add built-in model profiles that are missing from persisted state.
    pub fn ensure_builtin_profiles(&mut self) {
        let data_dir = default_data_dir_from_env(
            env::var_os("XDG_DATA_HOME"),
            env::var_os("HOME"),
            env::var_os("USERPROFILE"),
        );
        let slot_save_path = data_dir.join("llama-slots").to_string_lossy().into_owned();
        let builtins = builtin_model_profiles(slot_save_path);

        for builtin in builtins {
            let exists = self
                .profiles
                .iter()
                .any(|profile| profile.name == builtin.name && profile.model == builtin.model);
            if !exists {
                self.profiles.push(builtin);
            }
        }

        if self.active_profile.is_empty() {
            self.active_profile = "orion-qwen-q8-deep".to_string();
        }
    }
}

fn builtin_model_profiles(slot_save_path: String) -> Vec<ModelRuntimeProfile> {
    vec![
        ModelRuntimeProfile {
            name: "orion-qwen-q8-deep".to_string(),
            model: PathBuf::from("Qwen3.5-4B-Q8_0.gguf"),
            server: qwen_server_config(262_144, 1, 128, 4_096, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "orion-qwen-q8-balanced".to_string(),
            model: PathBuf::from("Qwen3.5-4B-Q8_0.gguf"),
            server: qwen_server_config(262_144, 2, 128, 4_096, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "orion-qwen-q8-kvu-fanout4".to_string(),
            model: PathBuf::from("Qwen3.5-4B-Q8_0.gguf"),
            server: qwen_kv_unified_server_config(65_536, 4, 128, 4_096, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "orion-qwen-q8-kvu-fanout6".to_string(),
            model: PathBuf::from("Qwen3.5-4B-Q8_0.gguf"),
            server: qwen_kv_unified_server_config(65_536, 6, 96, 4_096, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "orion-qwen-q8-kvu-fanout8".to_string(),
            model: PathBuf::from("Qwen3.5-4B-Q8_0.gguf"),
            server: qwen_kv_unified_server_config(65_536, 8, 64, 4_096, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "5060ti-qwen4b-fanout4".to_string(),
            model: PathBuf::from("Qwen3.5-4B-Q8_0.gguf"),
            server: qwen_server_config(131_072, 4, 128, 2_048, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "5060ti-qwen4b-dual".to_string(),
            model: PathBuf::from("Qwen3.5-4B-Q8_0.gguf"),
            server: qwen_server_config(262_144, 2, 128, 2_048, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "5060ti-qwen4b-kvu-fanout4".to_string(),
            model: PathBuf::from("Qwen3.5-4B-Q8_0.gguf"),
            server: qwen_kv_unified_server_config(73_728, 4, 128, 6_144, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "5060ti-qwen4b-kvu-fanout6".to_string(),
            model: PathBuf::from("Qwen3.5-4B-Q8_0.gguf"),
            server: qwen_kv_unified_server_config(73_728, 6, 96, 6_144, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "5060ti-qwen4b-kvu-fanout8".to_string(),
            model: PathBuf::from("Qwen3.5-4B-Q8_0.gguf"),
            server: qwen_kv_unified_server_config(73_728, 8, 64, 6_144, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "5070ti-qwen4b-fanout4".to_string(),
            model: PathBuf::from("Qwen3.5-4B-Q8_0.gguf"),
            server: qwen_server_config(131_072, 4, 128, 2_048, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "5070ti-qwen4b-dual".to_string(),
            model: PathBuf::from("Qwen3.5-4B-Q8_0.gguf"),
            server: qwen_server_config(262_144, 2, 128, 2_048, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "5070ti-qwen4b-kvu-fanout4".to_string(),
            model: PathBuf::from("Qwen3.5-4B-Q8_0.gguf"),
            server: qwen_kv_unified_server_config(73_728, 4, 128, 6_144, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "5070ti-qwen4b-kvu-fanout6".to_string(),
            model: PathBuf::from("Qwen3.5-4B-Q8_0.gguf"),
            server: qwen_kv_unified_server_config(73_728, 6, 96, 6_144, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "5070ti-qwen4b-kvu-fanout8".to_string(),
            model: PathBuf::from("Qwen3.5-4B-Q8_0.gguf"),
            server: qwen_kv_unified_server_config(73_728, 8, 64, 6_144, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "m6000-qwen9b-deep".to_string(),
            model: PathBuf::from("Qwen3.5-9B-Q8_0.gguf"),
            server: m6000_qwen_server_config(262_144, 1, 128, 4_096, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "m6000-qwen9b-fanout1".to_string(),
            model: PathBuf::from("Qwen3.5-9B-Q8_0.gguf"),
            server: m6000_qwen_server_config(262_144, 2, 128, 4_096, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "m6000-qwen9b-fanout2".to_string(),
            model: PathBuf::from("Qwen3.5-9B-Q8_0.gguf"),
            server: m6000_qwen_server_config(262_144, 2, 128, 4_096, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "m6000-qwen9b-fanout3".to_string(),
            model: PathBuf::from("Qwen3.5-9B-Q8_0.gguf"),
            server: m6000_qwen_server_config(262_144, 3, 128, 4_096, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "m6000-qwen9b-fanout4".to_string(),
            model: PathBuf::from("Qwen3.5-9B-Q8_0.gguf"),
            server: m6000_qwen_server_config(262_144, 4, 128, 4_096, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "m6000-qwen9b-fanout6".to_string(),
            model: PathBuf::from("Qwen3.5-9B-Q8_0.gguf"),
            server: m6000_qwen_server_config(262_144, 6, 96, 8_192, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "m6000-qwen9b-mtp-deep".to_string(),
            model: PathBuf::from("Qwen3.5-9B-Q8_0.gguf"),
            server: m6000_qwen_mtp_deep_server_config(&slot_save_path),
        },
        ModelRuntimeProfile {
            name: "m6000-qwen9b-mtp-vision".to_string(),
            model: PathBuf::from("Qwen3.5-9B-Q8_0.gguf"),
            server: m6000_qwen_mtp_vision_server_config(&slot_save_path),
        },
        ModelRuntimeProfile {
            name: "m6000-qwen9b-kvu-fanout4".to_string(),
            model: PathBuf::from("Qwen3.5-9B-Q8_0.gguf"),
            server: qwen_kv_unified_server_config(86_016, 4, 128, 8_192, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "m6000-qwen9b-kvu-fanout6".to_string(),
            model: PathBuf::from("Qwen3.5-9B-Q8_0.gguf"),
            server: qwen_kv_unified_server_config(86_016, 6, 96, 8_192, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "m6000-qwen9b-kvu-fanout8".to_string(),
            model: PathBuf::from("Qwen3.5-9B-Q8_0.gguf"),
            server: qwen_kv_unified_server_config(86_016, 8, 64, 8_192, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "5060ti-qwen9b-deep".to_string(),
            model: PathBuf::from("Qwen3.5-9B-Q8_0.gguf"),
            server: qwen_server_config(196_608, 1, 128, 4_096, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "5060ti-qwen9b-balanced2".to_string(),
            model: PathBuf::from("Qwen3.5-9B-Q8_0.gguf"),
            server: qwen_server_config(131_072, 2, 128, 4_096, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "5060ti-qwen9b-kvu-fanout4".to_string(),
            model: PathBuf::from("Qwen3.5-9B-Q8_0.gguf"),
            server: qwen_kv_unified_server_config(73_728, 4, 128, 6_144, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "5060ti-qwen9b-kvu-fanout6".to_string(),
            model: PathBuf::from("Qwen3.5-9B-Q8_0.gguf"),
            server: qwen_kv_unified_server_config(73_728, 6, 96, 6_144, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "5060ti-qwen9b-kvu-fanout8".to_string(),
            model: PathBuf::from("Qwen3.5-9B-Q8_0.gguf"),
            server: qwen_kv_unified_server_config(73_728, 8, 64, 6_144, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "5070ti-qwen9b-deep".to_string(),
            model: PathBuf::from("Qwen3.5-9B-Q8_0.gguf"),
            server: qwen_server_config(196_608, 1, 128, 4_096, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "5070ti-qwen9b-balanced2".to_string(),
            model: PathBuf::from("Qwen3.5-9B-Q8_0.gguf"),
            server: qwen_server_config(131_072, 2, 128, 4_096, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "5070ti-qwen9b-kvu-fanout4".to_string(),
            model: PathBuf::from("Qwen3.5-9B-Q8_0.gguf"),
            server: qwen_kv_unified_server_config(73_728, 4, 128, 6_144, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "5070ti-qwen9b-kvu-fanout6".to_string(),
            model: PathBuf::from("Qwen3.5-9B-Q8_0.gguf"),
            server: qwen_kv_unified_server_config(73_728, 6, 96, 6_144, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "5070ti-qwen9b-kvu-fanout8".to_string(),
            model: PathBuf::from("Qwen3.5-9B-Q8_0.gguf"),
            server: qwen_kv_unified_server_config(73_728, 8, 64, 6_144, &slot_save_path),
        },
        ModelRuntimeProfile {
            name: "bc250-qwen9b-q4km-vulkan".to_string(),
            model: PathBuf::from("Qwen3.5-9B-Q4_K_M.gguf"),
            server: bc250_qwen9b_vulkan_server_config(&slot_save_path),
        },
        ModelRuntimeProfile {
            name: "gemma4-12b-mtp-q4km".to_string(),
            model: PathBuf::from("Gemma4-12B-QAT-Q4_K_M.gguf"),
            server: gemma4_mtp_server_config(&slot_save_path),
        },
    ]
}

fn bc250_qwen9b_vulkan_server_config(slot_save_path: &str) -> ServerConfig {
    ServerConfig {
        port: 8080,
        host: "0.0.0.0".to_string(),
        ctx_size: 4096,
        n_gpu_layers: 99,
        batch_size: 512,
        ubatch_size: 128,
        threads: 6,
        flash_attn: true,
        mlock: false,
        api_key: String::new(),
        jinja: true,
        chat_template: String::new(),
        extra_args: vec![
            "--parallel".to_string(),
            "1".to_string(),
            "--slot-save-path".to_string(),
            slot_save_path.to_string(),
            "--cache-ram".to_string(),
            "1024".to_string(),
        ],
    }
}

fn gemma4_mtp_server_config(slot_save_path: &str) -> ServerConfig {
    let draft_model = Path::new(slot_save_path)
        .parent()
        .unwrap_or_else(|| Path::new(slot_save_path))
        .join("models")
        .join("mtp-gemma-4-12B-it.gguf");
    ServerConfig {
        port: 1200,
        host: "127.0.0.1".to_string(),
        ctx_size: 73_728,
        n_gpu_layers: 99,
        batch_size: 512,
        ubatch_size: 128,
        threads: 8,
        flash_attn: true,
        mlock: false,
        api_key: String::new(),
        jinja: true,
        chat_template: String::new(),
        extra_args: vec![
            "--parallel".to_string(),
            "1".to_string(),
            "--slot-save-path".to_string(),
            slot_save_path.to_string(),
            "-md".to_string(),
            draft_model.display().to_string(),
            "--spec-type".to_string(),
            "draft-mtp".to_string(),
            "--temp".to_string(),
            "0.6".to_string(),
            "--top-k".to_string(),
            "64".to_string(),
            "--top-p".to_string(),
            "0.9".to_string(),
            "--min-p".to_string(),
            "0.05".to_string(),
            "--repeat-penalty".to_string(),
            "1.1".to_string(),
        ],
    }
}

fn m6000_qwen_mtp_deep_server_config(slot_save_path: &str) -> ServerConfig {
    let mut server = m6000_qwen_server_config(262_144, 1, 128, 4_096, slot_save_path);
    server.extra_args.extend([
        "--spec-type".to_string(),
        "draft-mtp".to_string(),
        "--temp".to_string(),
        "0.6".to_string(),
        "--top-p".to_string(),
        "0.95".to_string(),
        "--top-k".to_string(),
        "20".to_string(),
        "--min-p".to_string(),
        "0".to_string(),
    ]);
    server
}

fn m6000_qwen_mtp_vision_server_config(slot_save_path: &str) -> ServerConfig {
    let mut server = m6000_qwen_mtp_deep_server_config(slot_save_path);
    let mmproj = Path::new(slot_save_path)
        .parent()
        .unwrap_or_else(|| Path::new(slot_save_path))
        .join("models")
        .join("mmproj-Qwen3.5-9B-BF16.gguf");
    server
        .extra_args
        .extend(["--mmproj".to_string(), mmproj.display().to_string()]);
    server
}

fn m6000_qwen_server_config(
    ctx_size: u32,
    parallel: usize,
    ubatch_size: u32,
    cache_ram_mb: u32,
    slot_save_path: &str,
) -> ServerConfig {
    let mut server = qwen_server_config(
        ctx_size,
        parallel,
        ubatch_size,
        cache_ram_mb,
        slot_save_path,
    );
    server.flash_attn = false;
    server
}

fn qwen_kv_unified_server_config(
    ctx_size: u32,
    parallel: usize,
    ubatch_size: u32,
    cache_ram_mb: u32,
    slot_save_path: &str,
) -> ServerConfig {
    let mut server = qwen_server_config(
        ctx_size,
        parallel,
        ubatch_size,
        cache_ram_mb,
        slot_save_path,
    );
    for value in &mut server.extra_args {
        if value == "q8_0" {
            *value = "q4_0".to_string();
        }
    }
    server.extra_args.push("--kv-unified".to_string());
    server
}

fn qwen_server_config(
    ctx_size: u32,
    parallel: usize,
    ubatch_size: u32,
    cache_ram_mb: u32,
    slot_save_path: &str,
) -> ServerConfig {
    ServerConfig {
        port: 1200,
        host: "127.0.0.1".to_string(),
        ctx_size,
        n_gpu_layers: -1,
        batch_size: 512,
        ubatch_size,
        threads: 8,
        flash_attn: true,
        mlock: false,
        api_key: String::new(),
        jinja: true,
        chat_template: String::new(),
        extra_args: vec![
            "--parallel".to_string(),
            parallel.to_string(),
            "--slot-save-path".to_string(),
            slot_save_path.to_string(),
            "-ctk".to_string(),
            "q8_0".to_string(),
            "-ctv".to_string(),
            "q8_0".to_string(),
            "--cache-ram".to_string(),
            cache_ram_mb.to_string(),
        ],
    }
}

/// Server settings that should apply when a matching model is selected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelRuntimeProfile {
    /// Human-readable profile name.
    pub name: String,
    /// Exact GGUF path or file name this profile applies to.
    pub model: PathBuf,
    /// Server settings to apply for this model.
    pub server: ServerConfig,
}

impl ModelRuntimeProfile {
    /// Return whether this profile applies to a selected model path.
    pub fn matches_model_path(&self, path: &Path) -> bool {
        if self.model == path {
            return true;
        }

        let Some(profile_name) = self.model.file_name() else {
            return false;
        };
        path.file_name() == Some(profile_name)
    }
}

impl Default for ModelRuntimeProfile {
    fn default() -> Self {
        Self {
            name: String::new(),
            model: PathBuf::new(),
            server: ServerConfig::default(),
        }
    }
}

/// Persisted llama-server configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    /// HTTP listen port.
    pub port: u16,
    /// HTTP listen host.
    pub host: String,
    /// Context size in tokens.
    pub ctx_size: u32,
    /// GPU layers to offload; `-1` means auto.
    pub n_gpu_layers: i32,
    /// Prompt processing batch size.
    pub batch_size: u32,
    /// Physical micro-batch size.
    pub ubatch_size: u32,
    /// Worker thread count.
    pub threads: usize,
    /// Enable flash attention when supported.
    pub flash_attn: bool,
    /// Lock model memory with mlock.
    pub mlock: bool,
    /// API key. Empty means no auth.
    pub api_key: String,
    /// Enable Jinja template processing.
    pub jinja: bool,
    /// Chat template string or path.
    pub chat_template: String,
    /// Extra llama-server argv entries.
    pub extra_args: Vec<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 8080,
            host: "127.0.0.1".to_string(),
            ctx_size: 4096,
            n_gpu_layers: -1,
            batch_size: 512,
            ubatch_size: 512,
            threads: 8,
            flash_attn: true,
            mlock: false,
            api_key: String::new(),
            jinja: false,
            chat_template: String::new(),
            extra_args: Vec::new(),
        }
    }
}

/// Persisted managed runtime profile configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeConfig {
    /// Runtime profile used by OpenCode as the primary model.
    pub opencode: RuntimeProfile,
    /// Runtime profile used by OpenCode as the fast/small model.
    #[serde(rename = "opencode-fast")]
    pub opencode_fast: RuntimeProfile,
    /// Runtime process state for known profiles.
    pub state: RuntimeState,
}

impl RuntimeConfig {
    /// Return a configured runtime profile by stable name.
    pub fn profile(&self, name: &str) -> Option<&RuntimeProfile> {
        match name {
            OPENCODE_PROFILE => Some(&self.opencode),
            OPENCODE_FAST_PROFILE => Some(&self.opencode_fast),
            _ => None,
        }
    }

    /// Return mutable runtime profile by stable name.
    pub fn profile_mut(&mut self, name: &str) -> Option<&mut RuntimeProfile> {
        match name {
            OPENCODE_PROFILE => Some(&mut self.opencode),
            OPENCODE_FAST_PROFILE => Some(&mut self.opencode_fast),
            _ => None,
        }
    }

    /// Return all built-in profile names in stable display order.
    pub fn profile_names() -> [&'static str; 2] {
        [OPENCODE_PROFILE, OPENCODE_FAST_PROFILE]
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            opencode: RuntimeProfile::opencode_default(),
            opencode_fast: RuntimeProfile::opencode_fast_default(),
            state: RuntimeState::default(),
        }
    }
}

/// Desired state for a managed llama-server profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeProfile {
    /// HTTP listen host.
    pub host: String,
    /// HTTP listen port.
    pub port: u16,
    /// GGUF model path. Empty means not configured yet.
    pub model: PathBuf,
    /// Context size in tokens.
    pub ctx_size: u32,
    /// GPU layers to offload; `-1` means auto, `0` means CPU-only.
    pub gpu_layers: i32,
    /// Prompt processing batch size.
    pub batch_size: u32,
    /// Worker thread count.
    pub threads: usize,
    /// Parallel slots for coding-agent requests.
    pub parallel: usize,
    /// Extra llama-server argv entries appended after lmml-managed flags.
    pub extra_args: Vec<String>,
    /// Whether lmml should start this profile automatically in future flows.
    pub autostart: bool,
}

impl RuntimeProfile {
    fn opencode_default() -> Self {
        Self {
            port: 1200,
            ctx_size: 65_536,
            parallel: 4,
            ..Self::default()
        }
    }

    fn opencode_fast_default() -> Self {
        Self {
            port: 1200,
            ctx_size: 32_768,
            parallel: 2,
            ..Self::default()
        }
    }

    /// Return the OpenAI-compatible API base URL for this profile.
    pub fn api_base_url(&self) -> String {
        format!("http://{}:{}/v1", format_url_host(&self.host), self.port)
    }

    /// Return a display name for the configured model.
    pub fn model_name(&self) -> String {
        self.model
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_string()
    }
}

impl Default for RuntimeProfile {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 1200,
            model: PathBuf::new(),
            ctx_size: 65_536,
            gpu_layers: -1,
            batch_size: 512,
            threads: 8,
            parallel: 4,
            extra_args: Vec::new(),
            autostart: false,
        }
    }
}

/// Live process state for managed runtime profiles.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeState {
    /// Live state for the OpenCode primary profile.
    pub opencode: RuntimeProfileState,
    /// Live state for the OpenCode fast/small profile.
    #[serde(rename = "opencode-fast")]
    pub opencode_fast: RuntimeProfileState,
}

impl RuntimeState {
    /// Return runtime state by stable profile name.
    pub fn profile(&self, name: &str) -> Option<&RuntimeProfileState> {
        match name {
            OPENCODE_PROFILE => Some(&self.opencode),
            OPENCODE_FAST_PROFILE => Some(&self.opencode_fast),
            _ => None,
        }
    }

    /// Return mutable runtime state by stable profile name.
    pub fn profile_mut(&mut self, name: &str) -> Option<&mut RuntimeProfileState> {
        match name {
            OPENCODE_PROFILE => Some(&mut self.opencode),
            OPENCODE_FAST_PROFILE => Some(&mut self.opencode_fast),
            _ => None,
        }
    }
}

/// Runtime status value persisted for a managed profile.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeStatus {
    /// No process is running.
    #[default]
    Stopped,
    /// A process is starting and health checks have not passed yet.
    Starting,
    /// The process answered its health endpoint.
    Ready,
    /// The process exists but health checks failed.
    Unhealthy,
    /// Start or runtime management failed.
    Failed,
    /// lmml is stopping the process.
    Stopping,
}

/// Live process facts for a managed runtime profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeProfileState {
    /// Runtime status.
    pub status: RuntimeStatus,
    /// Child process ID when known.
    pub pid: Option<u32>,
    /// Last host bound by the profile.
    pub host: String,
    /// Last port bound by the profile.
    pub port: u16,
    /// Last model path served by the profile.
    pub model: PathBuf,
    /// Log file path for the profile.
    pub log_path: PathBuf,
    /// Start timestamp string.
    pub started_at: String,
    /// Last health-check timestamp string.
    pub last_health_at: String,
    /// Last health result string.
    pub last_health: String,
    /// Consecutive health-check failure count.
    pub failure_count: u32,
}

impl Default for RuntimeProfileState {
    fn default() -> Self {
        Self {
            status: RuntimeStatus::Stopped,
            pid: None,
            host: String::new(),
            port: 0,
            model: PathBuf::new(),
            log_path: PathBuf::new(),
            started_at: String::new(),
            last_health_at: String::new(),
            last_health: String::new(),
            failure_count: 0,
        }
    }
}

/// Cached system profile summary used for fast TUI startup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemProfile {
    /// CUDA toolkit version, if detected.
    pub cuda_toolkit: Option<String>,
    /// Detected GPU names.
    pub gpu_names: Vec<String>,
    /// Detected GPU CUDA architectures.
    pub gpu_archs: Vec<String>,
    /// Whether ROCm/HIP was detected as an auto-selectable backend.
    #[serde(default)]
    pub rocm_available: bool,
    /// Detected ROCm/HIP `gfx*` GPU targets.
    #[serde(default)]
    pub rocm_targets: Vec<String>,
    /// Total VRAM per GPU in MiB.
    pub vram_mb: Vec<u64>,
    /// Whether sccache was detected.
    pub sccache: bool,
}

/// Errors returned while loading or saving state.
#[derive(Debug, Error)]
pub enum StateError {
    /// Could not create the parent directory for a state file.
    #[error("failed to create state directory {path}: {source}")]
    CreateDir {
        /// Directory path.
        path: PathBuf,
        /// Source IO error.
        #[source]
        source: std::io::Error,
    },
    /// Could not read a state file.
    #[error("failed to read state at {path}: {source}")]
    Read {
        /// State file path.
        path: PathBuf,
        /// Source IO error.
        #[source]
        source: std::io::Error,
    },
    /// Could not parse TOML state.
    #[error("failed to parse state at {path}: {source}")]
    Parse {
        /// State file path.
        path: PathBuf,
        /// TOML parse error.
        #[source]
        source: toml::de::Error,
    },
    /// Could not serialize TOML state.
    #[error("failed to serialize state: {0}")]
    Serialize(#[source] toml::ser::Error),
    /// Could not write a state file.
    #[error("failed to write state at {path}: {source}")]
    Write {
        /// State file path.
        path: PathBuf,
        /// Source IO error.
        #[source]
        source: std::io::Error,
    },
}

fn state_path_from_env(
    xdg_config_home: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
    userprofile: Option<std::ffi::OsString>,
) -> PathBuf {
    default_config_dir_from_env(xdg_config_home, home, userprofile).join(STATE_FILE_NAME)
}

fn default_config_dir_from_env(
    xdg_config_home: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
    userprofile: Option<std::ffi::OsString>,
) -> PathBuf {
    if let Some(path) = xdg_config_home.filter(|path| !path.is_empty()) {
        return PathBuf::from(path).join(APP_DIR_NAME);
    }
    if let Some(path) = home.filter(|path| !path.is_empty()) {
        return PathBuf::from(path).join(".config").join(APP_DIR_NAME);
    }
    if let Some(path) = userprofile.filter(|path| !path.is_empty()) {
        return PathBuf::from(path).join(".config").join(APP_DIR_NAME);
    }
    PathBuf::from(".").join(".config").join(APP_DIR_NAME)
}

fn default_data_dir_from_env(
    xdg_data_home: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
    userprofile: Option<std::ffi::OsString>,
) -> PathBuf {
    if let Some(path) = xdg_data_home.filter(|path| !path.is_empty()) {
        return PathBuf::from(path).join(APP_DIR_NAME);
    }
    if let Some(path) = home.filter(|path| !path.is_empty()) {
        return PathBuf::from(path)
            .join(".local")
            .join("share")
            .join(APP_DIR_NAME);
    }
    if let Some(path) = userprofile.filter(|path| !path.is_empty()) {
        return PathBuf::from(path)
            .join(".local")
            .join("share")
            .join(APP_DIR_NAME);
    }
    PathBuf::from(".")
        .join(".local")
        .join("share")
        .join(APP_DIR_NAME)
}

fn expected_server_binary(source_dir: &Path) -> PathBuf {
    source_dir
        .join("build")
        .join("bin")
        .join(binary_name("llama-server"))
}

fn legacy_server_binary(source_dir: &Path) -> PathBuf {
    source_dir
        .parent()
        .unwrap_or(source_dir)
        .join("bin")
        .join(binary_name("llama-server"))
}

fn default_state_dir_from_env(
    xdg_state_home: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
    userprofile: Option<std::ffi::OsString>,
) -> PathBuf {
    if let Some(path) = xdg_state_home.filter(|path| !path.is_empty()) {
        return PathBuf::from(path).join(APP_DIR_NAME);
    }
    if let Some(path) = home.filter(|path| !path.is_empty()) {
        return PathBuf::from(path)
            .join(".local")
            .join("state")
            .join(APP_DIR_NAME);
    }
    if let Some(path) = userprofile.filter(|path| !path.is_empty()) {
        return PathBuf::from(path)
            .join(".local")
            .join("state")
            .join(APP_DIR_NAME);
    }
    PathBuf::from(".")
        .join(".local")
        .join("state")
        .join(APP_DIR_NAME)
}

fn format_url_host(host: &str) -> String {
    if host.starts_with('[') || !host.contains(':') {
        host.to_string()
    } else {
        format!("[{host}]")
    }
}

fn binary_name(base: &str) -> String {
    if cfg!(windows) {
        format!("{base}.exe")
    } else {
        base.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn state_path_respects_xdg_config_home() {
        assert_eq!(
            state_path_from_env(Some("/tmp/config".into()), Some("/home/user".into()), None),
            PathBuf::from("/tmp/config/lmml/state.toml")
        );
    }

    #[test]
    fn state_path_falls_back_to_home_config() {
        assert_eq!(
            state_path_from_env(None, Some("/home/user".into()), None),
            PathBuf::from("/home/user/.config/lmml/state.toml")
        );
    }

    #[test]
    fn log_path_respects_xdg_data_home() {
        let data_dir =
            default_data_dir_from_env(Some("/tmp/data".into()), Some("/home/user".into()), None);
        assert_eq!(
            data_dir.join(LOG_FILE_NAME),
            PathBuf::from("/tmp/data/lmml/lmml.log")
        );
    }

    #[test]
    fn runtime_log_dir_respects_xdg_state_home() {
        let state_dir =
            default_state_dir_from_env(Some("/tmp/state".into()), Some("/home/user".into()), None);
        assert_eq!(
            state_dir.join("runtime"),
            PathBuf::from("/tmp/state/lmml/runtime")
        );
    }

    #[test]
    fn defaults_match_plan_schema() {
        let state = AppState::default();
        assert_eq!(state.server.port, 8080);
        assert_eq!(state.server.host, "127.0.0.1");
        assert_eq!(state.server.ctx_size, 4096);
        assert_eq!(state.server.n_gpu_layers, -1);
        assert_eq!(state.server.batch_size, 512);
        assert_eq!(state.server.ubatch_size, 512);
        assert_eq!(state.server.threads, 8);
        assert!(state.server.flash_attn);
        assert_eq!(state.build.track_mode, TrackMode::Main);
        assert_eq!(state.runtime.opencode.port, 1200);
        assert_eq!(state.runtime.opencode.ctx_size, 65_536);
        assert_eq!(state.runtime.opencode.parallel, 4);
        assert_eq!(state.runtime.opencode_fast.port, 1200);
        assert_eq!(state.runtime.opencode_fast.ctx_size, 32_768);
        assert_eq!(state.runtime.opencode_fast.parallel, 2);
        assert_eq!(state.runtime.opencode.gpu_layers, -1);
        assert_eq!(state.runtime.state.opencode.status, RuntimeStatus::Stopped);
        assert!(state.model.profiles.is_empty());
        assert!(state.model.active_profile.is_empty());
    }

    #[test]
    fn model_runtime_profiles_match_exact_path_or_file_name() {
        let profile = ModelRuntimeProfile {
            name: "nemotron-native".to_string(),
            model: PathBuf::from("Nemotron3-Nano-4B-Q8_K_P.gguf"),
            server: ServerConfig {
                chat_template: String::new(),
                jinja: true,
                ..ServerConfig::default()
            },
        };
        let model_state = ModelState {
            profiles: vec![profile],
            ..ModelState::default()
        };

        assert_eq!(
            model_state
                .runtime_profile_for_path(Path::new(
                    "/home/angelo/.local/share/lmml/models/Nemotron3-Nano-4B-Q8_K_P.gguf"
                ))
                .map(|profile| profile.name.as_str()),
            Some("nemotron-native")
        );
        assert!(model_state
            .runtime_profile_for_path(Path::new("/models/Qwen3.5-4B-Q6_K.gguf"))
            .is_none());
    }

    #[test]
    fn builtin_qwen_profiles_are_added_without_overwriting_custom_profiles() {
        let mut model_state = ModelState {
            profiles: vec![ModelRuntimeProfile {
                name: "custom".to_string(),
                model: PathBuf::from("custom.gguf"),
                server: ServerConfig::default(),
            }],
            ..ModelState::default()
        };

        model_state.ensure_builtin_profiles();
        model_state.ensure_builtin_profiles();

        let qwen_profiles =
            model_state.runtime_profiles_for_path(Path::new("/models/Qwen3.5-4B-Q8_0.gguf"));
        assert_eq!(qwen_profiles.len(), 15);
        assert_eq!(model_state.active_profile, "orion-qwen-q8-deep");
        assert_eq!(model_state.profiles.len(), 39);
    }

    #[test]
    fn qwen_profiles_cycle_through_matching_4b_profiles() {
        let mut model_state = ModelState::default();
        let model = Path::new("/models/Qwen3.5-4B-Q8_0.gguf");
        model_state.ensure_builtin_profiles();

        assert_eq!(
            model_state
                .runtime_profile_for_path(model)
                .map(|profile| profile.name.as_str()),
            Some("orion-qwen-q8-deep")
        );
        assert_eq!(
            model_state
                .cycle_runtime_profile_for_path(model)
                .map(|profile| profile.name.as_str()),
            Some("orion-qwen-q8-balanced")
        );
        let extra_args = &model_state
            .runtime_profile_for_path(model)
            .expect("balanced profile")
            .server
            .extra_args;
        assert_eq!(extra_args[0], "--parallel");
        assert_eq!(extra_args[1], "2");
        assert_eq!(extra_args[2], "--slot-save-path");
        assert!(extra_args[3].ends_with("lmml/llama-slots"));
        assert_eq!(
            &extra_args[4..],
            ["-ctk", "q8_0", "-ctv", "q8_0", "--cache-ram", "4096"]
        );
        assert_eq!(
            model_state
                .cycle_runtime_profile_for_path(model)
                .map(|profile| profile.name.as_str()),
            Some("orion-qwen-q8-kvu-fanout4")
        );
        let extra_args = &model_state
            .runtime_profile_for_path(model)
            .expect("orion kv-unified fanout4 profile")
            .server
            .extra_args;
        assert_eq!(&extra_args[0..2], ["--parallel", "4"]);
        assert_eq!(&extra_args[4..8], ["-ctk", "q4_0", "-ctv", "q4_0"]);
        assert!(extra_args.iter().any(|arg| arg == "--kv-unified"));
        assert_eq!(
            model_state
                .cycle_runtime_profile_for_path(model)
                .map(|profile| profile.name.as_str()),
            Some("orion-qwen-q8-kvu-fanout6")
        );
        assert_eq!(
            model_state
                .cycle_runtime_profile_for_path(model)
                .map(|profile| profile.name.as_str()),
            Some("orion-qwen-q8-kvu-fanout8")
        );
        assert_eq!(
            model_state
                .cycle_runtime_profile_for_path(model)
                .map(|profile| profile.name.as_str()),
            Some("5060ti-qwen4b-fanout4")
        );
        assert_eq!(
            model_state
                .cycle_runtime_profile_for_path(model)
                .map(|profile| profile.name.as_str()),
            Some("5060ti-qwen4b-dual")
        );
        assert_eq!(
            model_state
                .cycle_runtime_profile_for_path(model)
                .map(|profile| profile.name.as_str()),
            Some("5060ti-qwen4b-kvu-fanout4")
        );
        assert_eq!(
            model_state
                .cycle_runtime_profile_for_path(model)
                .map(|profile| profile.name.as_str()),
            Some("5060ti-qwen4b-kvu-fanout6")
        );
        assert_eq!(
            model_state
                .cycle_runtime_profile_for_path(model)
                .map(|profile| profile.name.as_str()),
            Some("5060ti-qwen4b-kvu-fanout8")
        );
        assert_eq!(
            model_state
                .cycle_runtime_profile_for_path(model)
                .map(|profile| profile.name.as_str()),
            Some("5070ti-qwen4b-fanout4")
        );
        assert_eq!(
            model_state
                .cycle_runtime_profile_for_path(model)
                .map(|profile| profile.name.as_str()),
            Some("5070ti-qwen4b-dual")
        );
        assert_eq!(
            model_state
                .cycle_runtime_profile_for_path(model)
                .map(|profile| profile.name.as_str()),
            Some("5070ti-qwen4b-kvu-fanout4")
        );
        assert_eq!(
            model_state
                .cycle_runtime_profile_for_path(model)
                .map(|profile| profile.name.as_str()),
            Some("5070ti-qwen4b-kvu-fanout6")
        );
        assert_eq!(
            model_state
                .cycle_runtime_profile_for_path(model)
                .map(|profile| profile.name.as_str()),
            Some("5070ti-qwen4b-kvu-fanout8")
        );
        assert_eq!(
            model_state
                .cycle_runtime_profile_for_path(model)
                .map(|profile| profile.name.as_str()),
            Some("orion-qwen-q8-deep")
        );
    }

    #[test]
    fn qwen9b_fleet_profiles_are_available() {
        let mut model_state = ModelState::default();
        let model = Path::new("/models/Qwen3.5-9B-Q8_0.gguf");
        model_state.ensure_builtin_profiles();

        let profiles = model_state.runtime_profiles_for_path(model);
        let names: Vec<&str> = profiles
            .iter()
            .map(|profile| profile.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec![
                "m6000-qwen9b-deep",
                "m6000-qwen9b-fanout1",
                "m6000-qwen9b-fanout2",
                "m6000-qwen9b-fanout3",
                "m6000-qwen9b-fanout4",
                "m6000-qwen9b-fanout6",
                "m6000-qwen9b-mtp-deep",
                "m6000-qwen9b-mtp-vision",
                "m6000-qwen9b-kvu-fanout4",
                "m6000-qwen9b-kvu-fanout6",
                "m6000-qwen9b-kvu-fanout8",
                "5060ti-qwen9b-deep",
                "5060ti-qwen9b-balanced2",
                "5060ti-qwen9b-kvu-fanout4",
                "5060ti-qwen9b-kvu-fanout6",
                "5060ti-qwen9b-kvu-fanout8",
                "5070ti-qwen9b-deep",
                "5070ti-qwen9b-balanced2",
                "5070ti-qwen9b-kvu-fanout4",
                "5070ti-qwen9b-kvu-fanout6",
                "5070ti-qwen9b-kvu-fanout8"
            ]
        );
        assert_eq!(profiles[1].server.ctx_size, 262_144);
        assert_eq!(&profiles[1].server.extra_args[0..2], ["--parallel", "2"]);
        assert!(!profiles[1].server.flash_attn);
        assert_eq!(&profiles[2].server.extra_args[0..2], ["--parallel", "2"]);
        assert!(!profiles[2].server.flash_attn);
        assert_eq!(&profiles[3].server.extra_args[0..2], ["--parallel", "3"]);
        assert!(!profiles[3].server.flash_attn);
        assert_eq!(&profiles[4].server.extra_args[0..2], ["--parallel", "4"]);
        assert!(!profiles[4].server.flash_attn);
        assert_eq!(profiles[5].server.ubatch_size, 96);
        assert_eq!(
            &profiles[5].server.extra_args[8..10],
            ["--cache-ram", "8192"]
        );
        assert_eq!(
            &profiles[6].server.extra_args[10..12],
            ["--spec-type", "draft-mtp"]
        );
        assert_eq!(
            &profiles[6].server.extra_args[12..],
            ["--temp", "0.6", "--top-p", "0.95", "--top-k", "20", "--min-p", "0"]
        );
        assert_eq!(
            &profiles[7].server.extra_args[10..12],
            ["--spec-type", "draft-mtp"]
        );
        assert_eq!(profiles[7].server.extra_args[20], "--mmproj");
        assert!(
            profiles[7].server.extra_args[21].ends_with("lmml/models/mmproj-Qwen3.5-9B-BF16.gguf")
        );
        assert_eq!(profiles[8].server.ctx_size, 86_016);
        assert_eq!(&profiles[8].server.extra_args[0..2], ["--parallel", "4"]);
        assert_eq!(
            &profiles[8].server.extra_args[4..8],
            ["-ctk", "q4_0", "-ctv", "q4_0"]
        );
        assert!(profiles[8]
            .server
            .extra_args
            .iter()
            .any(|arg| arg == "--kv-unified"));
        assert_eq!(profiles[9].server.ctx_size, 86_016);
        assert_eq!(&profiles[9].server.extra_args[0..2], ["--parallel", "6"]);
        assert_eq!(profiles[10].server.ctx_size, 86_016);
        assert_eq!(&profiles[10].server.extra_args[0..2], ["--parallel", "8"]);
        assert_eq!(profiles[11].server.ctx_size, 196_608);
        assert_eq!(&profiles[12].server.extra_args[0..2], ["--parallel", "2"]);
        assert_eq!(profiles[13].server.ctx_size, 73_728);
        assert_eq!(&profiles[14].server.extra_args[0..2], ["--parallel", "6"]);
        assert_eq!(&profiles[17].server.extra_args[0..2], ["--parallel", "2"]);
        assert_eq!(profiles[18].server.ctx_size, 73_728);
        assert_eq!(&profiles[20].server.extra_args[0..2], ["--parallel", "8"]);
    }

    #[test]
    fn gemma4_mtp_profile_loads_main_and_draft_models() {
        let mut model_state = ModelState::default();
        let model = Path::new("/models/Gemma4-12B-QAT-Q4_K_M.gguf");
        model_state.ensure_builtin_profiles();

        let profile = model_state
            .runtime_profile_for_path(model)
            .expect("Gemma4 MTP profile");

        assert_eq!(profile.name, "gemma4-12b-mtp-q4km");
        assert_eq!(profile.server.ctx_size, 73_728);
        assert_eq!(profile.server.n_gpu_layers, 99);
        assert!(profile.server.flash_attn);
        assert!(profile.server.jinja);
        assert_eq!(
            profile.server.extra_args,
            vec![
                "--parallel",
                "1",
                "--slot-save-path",
                profile.server.extra_args[3].as_str(),
                "-md",
                profile.server.extra_args[5].as_str(),
                "--spec-type",
                "draft-mtp",
                "--temp",
                "0.6",
                "--top-k",
                "64",
                "--top-p",
                "0.9",
                "--min-p",
                "0.05",
                "--repeat-penalty",
                "1.1",
            ]
        );
        assert!(profile.server.extra_args[3].ends_with("lmml/llama-slots"));
        assert!(profile.server.extra_args[5].ends_with("lmml/models/mtp-gemma-4-12B-it.gguf"));
    }

    #[test]
    fn bc250_qwen9b_vulkan_profile_is_available() {
        let mut model_state = ModelState::default();
        let model = Path::new("/models/Qwen3.5-9B-Q4_K_M.gguf");
        model_state.ensure_builtin_profiles();

        let profile = model_state
            .runtime_profile_for_path(model)
            .expect("BC-250 Qwen profile");

        assert_eq!(profile.name, "bc250-qwen9b-q4km-vulkan");
        assert_eq!(profile.server.host, "0.0.0.0");
        assert_eq!(profile.server.port, 8080);
        assert_eq!(profile.server.ctx_size, 4096);
        assert_eq!(profile.server.n_gpu_layers, 99);
        assert_eq!(profile.server.threads, 6);
        assert!(profile.server.flash_attn);
        assert_eq!(
            profile.server.extra_args,
            vec![
                "--parallel",
                "1",
                "--slot-save-path",
                profile.server.extra_args[3].as_str(),
                "--cache-ram",
                "1024",
            ]
        );
        assert!(profile.server.extra_args[3].ends_with("lmml/llama-slots"));
    }

    #[test]
    fn runtime_profiles_are_addressable_by_stable_name() {
        let mut runtime = RuntimeConfig::default();

        assert_eq!(
            RuntimeConfig::profile_names(),
            ["opencode", "opencode-fast"]
        );
        assert_eq!(
            runtime
                .profile("opencode")
                .map(RuntimeProfile::api_base_url),
            Some("http://127.0.0.1:1200/v1".to_string())
        );
        assert_eq!(
            runtime
                .profile("opencode-fast")
                .map(RuntimeProfile::api_base_url),
            Some("http://127.0.0.1:1200/v1".to_string())
        );
        assert!(runtime.profile("missing").is_none());

        runtime
            .profile_mut("opencode-fast")
            .expect("fast profile")
            .model = PathBuf::from("/models/fast.gguf");
        assert_eq!(
            runtime
                .profile("opencode-fast")
                .expect("fast profile")
                .model_name(),
            "fast.gguf"
        );
    }

    #[test]
    fn runtime_schema_round_trips_toml() {
        let state = AppState::default();
        let encoded = toml::to_string_pretty(&state).expect("serialize");
        let decoded: AppState = toml::from_str(&encoded).expect("deserialize");

        assert_eq!(decoded.runtime, state.runtime);
        assert!(encoded.contains("[runtime.opencode]"));
        assert!(encoded.contains("[runtime.opencode-fast]"));
        assert!(encoded.contains("[runtime.state.opencode]"));
    }

    #[test]
    fn round_trips_state_toml() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let path = tempdir.path().join("lmml").join("state.toml");
        let state = sample_state();

        state.save_to_path(&path).expect("save state");
        let loaded = AppState::load_from_path(&path).expect("load state");

        assert_eq!(loaded, state);
    }

    #[test]
    fn default_build_binary_points_to_llama_cpp_build() {
        let state = AppState::default();

        assert_eq!(
            state.build.binary,
            state
                .build
                .source_dir
                .join("build")
                .join("bin")
                .join(binary_name("llama-server"))
        );
    }

    #[test]
    fn load_repairs_legacy_build_binary_default() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let path = tempdir.path().join("lmml").join("state.toml");
        let mut state = sample_state();
        state.build.source_dir = tempdir.path().join("data").join("lmml").join("llama.cpp");
        state.build.binary = tempdir
            .path()
            .join("data")
            .join("lmml")
            .join("bin")
            .join(binary_name("llama-server"));
        state.save_to_path(&path).expect("save legacy state");

        let loaded = AppState::load_from_path(&path).expect("load migrated state");

        assert_eq!(
            loaded.build.binary,
            expected_server_binary(&state.build.source_dir)
        );
    }

    #[test]
    fn load_preserves_custom_build_binary() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let path = tempdir.path().join("lmml").join("state.toml");
        let mut state = sample_state();
        state.build.binary = tempdir.path().join("custom").join("llama-server");
        state.save_to_path(&path).expect("save custom state");

        let loaded = AppState::load_from_path(&path).expect("load state");

        assert_eq!(loaded.build.binary, state.build.binary);
    }

    #[test]
    fn load_creates_default_when_missing() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let path = tempdir.path().join("lmml").join("state.toml");

        let loaded = AppState::load_from_path(&path).expect("load default");

        assert_eq!(loaded, AppState::default());
        assert!(path.exists());
    }

    #[test]
    fn read_only_load_returns_default_without_creating_file() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let path = tempdir.path().join("lmml").join("state.toml");

        let loaded = AppState::load_existing_or_default_from_path(&path).expect("load default");

        assert_eq!(loaded, AppState::default());
        assert!(!path.exists());
    }

    #[test]
    fn runtime_profile_base_url_brackets_ipv6_literals() {
        let profile = RuntimeProfile {
            host: "::1".to_string(),
            port: 1200,
            ..RuntimeProfile::default()
        };

        assert_eq!(profile.api_base_url(), "http://[::1]:1200/v1");
    }

    #[test]
    fn reset_path_replaces_existing_state() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let path = tempdir.path().join("lmml").join("state.toml");
        sample_state().save_to_path(&path).expect("save sample");

        AppState::reset_path(&path).expect("reset");
        let loaded = AppState::load_from_path(&path).expect("load reset");

        assert_eq!(loaded, AppState::default());
    }

    #[test]
    fn parse_errors_are_typed() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let path = tempdir.path().join("state.toml");
        fs::write(&path, "not = [valid").expect("write invalid toml");

        assert!(matches!(
            AppState::load_from_path(&path),
            Err(StateError::Parse { .. })
        ));
    }

    fn sample_state() -> AppState {
        AppState {
            build: BuildState {
                source_dir: PathBuf::from("/data/lmml/llama.cpp"),
                binary: PathBuf::from("/data/lmml/custom/llama-server"),
                commit: "abc1234".to_string(),
                cmake_hash: "e3b0c44298fc".to_string(),
                backend: "Cuda".to_string(),
                archs: vec!["sm_75".to_string(), "sm_86".to_string()],
                sccache_used: true,
                last_built: "2026-06-01T00:00:00Z".to_string(),
                track_mode: TrackMode::Tag,
            },
            model: ModelState {
                last_used: PathBuf::from("/models/mistral.gguf"),
                models_dir: PathBuf::from("/models"),
                aliases: vec![PathBuf::from("/external")],
                profiles: vec![ModelRuntimeProfile {
                    name: "mistral".to_string(),
                    model: PathBuf::from("/models/mistral.gguf"),
                    server: ServerConfig {
                        port: 8081,
                        ..ServerConfig::default()
                    },
                }],
                active_profile: "mistral".to_string(),
            },
            server: ServerConfig {
                port: 8081,
                host: "0.0.0.0".to_string(),
                ctx_size: 8192,
                n_gpu_layers: 35,
                batch_size: 256,
                ubatch_size: 128,
                threads: 12,
                flash_attn: false,
                mlock: true,
                api_key: "secret".to_string(),
                jinja: true,
                chat_template: "chatml".to_string(),
                extra_args: vec!["--verbose".to_string()],
            },
            runtime: RuntimeConfig {
                opencode: RuntimeProfile {
                    host: "127.0.0.1".to_string(),
                    port: 1200,
                    model: PathBuf::from("/models/full.gguf"),
                    ctx_size: 65_536,
                    gpu_layers: -1,
                    batch_size: 512,
                    threads: 8,
                    parallel: 4,
                    extra_args: vec!["--metrics".to_string()],
                    autostart: false,
                },
                opencode_fast: RuntimeProfile {
                    host: "127.0.0.1".to_string(),
                    port: 1200,
                    model: PathBuf::from("/models/fast.gguf"),
                    ctx_size: 32_768,
                    gpu_layers: -1,
                    batch_size: 512,
                    threads: 8,
                    parallel: 2,
                    extra_args: Vec::new(),
                    autostart: false,
                },
                state: RuntimeState {
                    opencode: RuntimeProfileState {
                        status: RuntimeStatus::Ready,
                        pid: Some(1234),
                        host: "127.0.0.1".to_string(),
                        port: 1200,
                        model: PathBuf::from("/models/full.gguf"),
                        log_path: PathBuf::from("/state/lmml/runtime/opencode.log"),
                        started_at: "2026-06-01T00:00:00Z".to_string(),
                        last_health_at: "2026-06-01T00:00:05Z".to_string(),
                        last_health: "ok".to_string(),
                        failure_count: 0,
                    },
                    opencode_fast: RuntimeProfileState {
                        status: RuntimeStatus::Stopped,
                        ..RuntimeProfileState::default()
                    },
                },
            },
            system_profile: Some(SystemProfile {
                cuda_toolkit: Some("12.4".to_string()),
                gpu_names: vec!["NVIDIA GeForce RTX 3090".to_string()],
                gpu_archs: vec!["sm_86".to_string()],
                rocm_available: false,
                rocm_targets: Vec::new(),
                vram_mb: vec![24576],
                sccache: true,
            }),
        }
    }
}
