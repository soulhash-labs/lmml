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
    /// Cached system profile summary from the detection crate.
    pub system_profile: Option<SystemProfile>,
}

impl AppState {
    /// Load state from the default XDG path, creating defaults if missing.
    pub fn load() -> Result<Self, StateError> {
        Self::load_from_path(Self::path())
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
        toml::from_str(&content).map_err(|source| StateError::Parse {
            path: path.to_path_buf(),
            source,
        })
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
        Self {
            source_dir: data_dir.join("llama.cpp"),
            binary: data_dir.join("bin").join(binary_name("llama-server")),
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

/// Cached system profile summary used for fast TUI startup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemProfile {
    /// CUDA toolkit version, if detected.
    pub cuda_toolkit: Option<String>,
    /// Detected GPU names.
    pub gpu_names: Vec<String>,
    /// Detected GPU CUDA architectures.
    pub gpu_archs: Vec<String>,
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
    fn load_creates_default_when_missing() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let path = tempdir.path().join("lmml").join("state.toml");

        let loaded = AppState::load_from_path(&path).expect("load default");

        assert_eq!(loaded, AppState::default());
        assert!(path.exists());
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
                binary: PathBuf::from("/data/lmml/bin/llama-server"),
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
            system_profile: Some(SystemProfile {
                cuda_toolkit: Some("12.4".to_string()),
                gpu_names: vec!["NVIDIA GeForce RTX 3090".to_string()],
                gpu_archs: vec!["sm_86".to_string()],
                vram_mb: vec![24576],
                sccache: true,
            }),
        }
    }
}
