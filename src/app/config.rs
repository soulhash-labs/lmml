//! Configuration persistence.
//!
//! Reads/writes `~/.lmml/config.toml` and `~/.lmml/state.toml`.
//! Created with defaults on first launch.

use color_eyre::{eyre::Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::SystemTime;

pub const CONFIG_VERSION: u32 = 1;

/// Returns the lmml config directory (overridable via `LMML_CONFIG_DIR`).
pub fn config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("LMML_CONFIG_DIR") {
        PathBuf::from(dir)
    } else {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".lmml")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_dir_env_override() {
        // Temporarily set LMML_CONFIG_DIR
        let original = std::env::var("LMML_CONFIG_DIR").ok();
        std::env::set_var("LMML_CONFIG_DIR", "/tmp/lmml-test");
        let dir = config_dir();
        assert_eq!(dir, PathBuf::from("/tmp/lmml-test"));
        // Restore
        match original {
            Some(v) => std::env::set_var("LMML_CONFIG_DIR", v),
            None => std::env::remove_var("LMML_CONFIG_DIR"),
        }
    }

    #[test]
    fn test_config_dir_default_home() {
        let original_config = std::env::var("LMML_CONFIG_DIR").ok();
        std::env::remove_var("LMML_CONFIG_DIR");

        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", "/home/testuser");

        let dir = config_dir();
        assert_eq!(dir, PathBuf::from("/home/testuser/.lmml"));

        // Restore
        match original_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match original_config {
            Some(v) => std::env::set_var("LMML_CONFIG_DIR", v),
            None => std::env::remove_var("LMML_CONFIG_DIR"),
        }
    }
}

fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

fn state_path() -> PathBuf {
    config_dir().join("state.toml")
}

fn models_path() -> PathBuf {
    config_dir().join("models")
}

fn build_path() -> PathBuf {
    config_dir().join("build")
}

fn llama_cpp_path() -> PathBuf {
    build_path().join("llama.cpp")
}

/// User preferences, persisted in `~/.lmml/config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub general: GeneralConfig,
    pub build: BuildConfig,
    pub server: ServerConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    pub model_dirs: Vec<String>,
    pub default_model: String,
    pub theme: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildConfig {
    pub llama_cpp_path: String,
    pub extra_cmake_flags: Vec<String>,
    pub jobs: u32,
    pub backend: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub port: u16,
    pub context_size: u32,
    pub gpu_layers: u32,
    pub threads: u32,
    pub batch_size: u32,
    pub extra_args: Vec<String>,
}

impl Config {
    /// Load config from disk, or return defaults if file doesn't exist.
    pub fn load_or_default() -> Self {
        load_config().unwrap_or_default()
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            version: CONFIG_VERSION,
            general: GeneralConfig {
                model_dirs: vec![models_path().to_string_lossy().to_string()],
                default_model: String::new(),
                theme: "auto".to_string(),
            },
            build: BuildConfig {
                llama_cpp_path: llama_cpp_path().to_string_lossy().to_string(),
                extra_cmake_flags: Vec::new(),
                jobs: 0,
                backend: "auto".to_string(),
            },
            server: ServerConfig {
                port: 8080,
                context_size: 8192,
                gpu_layers: 99,
                threads: 0,
                batch_size: 512,
                extra_args: Vec::new(),
            },
        }
    }
}

/// Auto-managed session state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppStateToml {
    pub last_session: LastSession,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LastSession {
    pub last_model: String,
    pub server_was_running: bool,
    pub build_state: String,
    pub build_commit: String,
}

impl Default for AppStateToml {
    fn default() -> Self {
        AppStateToml {
            last_session: LastSession {
                last_model: String::new(),
                server_was_running: false,
                build_state: "not-started".to_string(),
                build_commit: String::new(),
            },
        }
    }
}

/// Load config from disk, or create defaults if missing.
pub fn load_config() -> Result<Config> {
    let path = config_path();
    if path.exists() {
        let content = std::fs::read_to_string(&path)
            .wrap_err_with(|| format!("Failed to read config at {}", path.display()))?;
        let mut config: Config = toml::from_str(&content)
            .wrap_err_with(|| format!("Failed to parse config at {}", path.display()))?;
        // Schema migration
        let current_version = config.version;
        if current_version < CONFIG_VERSION {
            let backup = config_path().with_extension("toml.bak");
            if !backup.exists() {
                std::fs::copy(&path, &backup)
                    .wrap_err_with(|| format!("Failed to backup config to {}", backup.display()))?;
            }
            migrate_config(&mut config, current_version);
            config.version = CONFIG_VERSION;
            save_config(&config)?;
        }
        Ok(config)
    } else {
        let config = Config::default();
        save_config(&config)?;
        Ok(config)
    }
}

/// Apply schema migrations for versions < CONFIG_VERSION.
fn migrate_config(config: &mut Config, from_version: u32) {
    if from_version < 1 {
        // v0 -> v1: no structural changes yet, but ensure all fields present
        if config.general.theme.is_empty() {
            config.general.theme = "auto".to_string();
        }
        if config.build.llama_cpp_path.is_empty() {
            config.build.llama_cpp_path = llama_cpp_path().to_string_lossy().to_string();
        }
        if config.build.backend.is_empty() {
            config.build.backend = "auto".to_string();
        }
        if config.server.port == 0 {
            config.server.port = 8080;
        }
        if config.server.context_size == 0 {
            config.server.context_size = 8192;
        }
        if config.server.gpu_layers == 0 {
            config.server.gpu_layers = 99;
        }
    }
}

/// Metadata cache for downloaded models, persisted in `~/.lmml/models.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelsCache {
    pub models: Vec<CachedModel>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedModel {
    pub path: String,
    pub name: String,
    pub quantization: String,
    pub size_bytes: u64,
    pub file_modified: String,
}

fn models_cache_path() -> PathBuf {
    config_dir().join("models.toml")
}

fn metrics_path() -> PathBuf {
    config_dir().join("metrics.toml")
}

pub fn load_models_cache() -> Result<ModelsCache> {
    let path = models_cache_path();
    if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        Ok(toml::from_str(&content)?)
    } else {
        Ok(ModelsCache::default())
    }
}

pub fn save_models_cache(cache: &ModelsCache) -> Result<()> {
    let content = toml::to_string_pretty(cache)?;
    std::fs::write(models_cache_path(), content)?;
    Ok(())
}

/// Persisted server performance samples.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MetricsHistory {
    pub samples: Vec<MetricSample>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricSample {
    pub timestamp: String,
    pub latency_ms: f64,
    pub tok_s: f64,
    pub active_slots: Option<u64>,
    pub kv_cache_used: Option<u64>,
    pub kv_cache_total: Option<u64>,
}

pub fn load_metrics_history() -> Result<MetricsHistory> {
    let path = metrics_path();
    if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        Ok(toml::from_str(&content)?)
    } else {
        Ok(MetricsHistory::default())
    }
}

pub fn append_metric_sample(metrics: &crate::server::ServerMetrics) -> Result<()> {
    let mut history = load_metrics_history()?;
    history.samples.push(MetricSample {
        timestamp: chrono::Utc::now().to_rfc3339(),
        latency_ms: metrics.latency_ms,
        tok_s: metrics.tok_s,
        active_slots: metrics.active_slots,
        kv_cache_used: metrics.kv_cache_used,
        kv_cache_total: metrics.kv_cache_total,
    });

    const MAX_SAMPLES: usize = 1_000;
    if history.samples.len() > MAX_SAMPLES {
        let remove_count = history.samples.len() - MAX_SAMPLES;
        history.samples.drain(0..remove_count);
    }

    let content = toml::to_string_pretty(&history)?;
    std::fs::write(metrics_path(), content)?;
    Ok(())
}

/// Check if the config file has been modified since `last_checked`.
/// Returns `Ok(true)` if a reload is needed.
pub fn check_config_reload(last_modified: &mut SystemTime) -> bool {
    if let Ok(metadata) = std::fs::metadata(config_path()) {
        if let Ok(modified) = metadata.modified() {
            if modified > *last_modified {
                *last_modified = modified;
                return true;
            }
        }
    }
    false
}

/// Save config to disk.
pub fn save_config(config: &Config) -> Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)
        .wrap_err_with(|| format!("Failed to create config directory at {}", dir.display()))?;

    let content = toml::to_string_pretty(config).wrap_err("Failed to serialize config")?;
    std::fs::write(config_path(), content)
        .wrap_err_with(|| format!("Failed to write config to {}", config_path().display()))?;
    Ok(())
}

/// Load state from disk, or return defaults if missing.
pub fn load_state() -> Result<AppStateToml> {
    let path = state_path();
    if path.exists() {
        let content = std::fs::read_to_string(&path)
            .wrap_err_with(|| format!("Failed to read state at {}", path.display()))?;
        let state: AppStateToml = toml::from_str(&content)
            .wrap_err_with(|| format!("Failed to parse state at {}", path.display()))?;
        Ok(state)
    } else {
        Ok(AppStateToml::default())
    }
}

/// Save state to disk.
pub fn save_state(state: &AppStateToml) -> Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)
        .wrap_err_with(|| format!("Failed to create config directory at {}", dir.display()))?;

    let content = toml::to_string_pretty(state).wrap_err("Failed to serialize state")?;
    std::fs::write(state_path(), content)
        .wrap_err_with(|| format!("Failed to write state to {}", state_path().display()))?;
    Ok(())
}

/// Ensure the config directory and default subdirectories exist.
pub fn ensure_dirs() -> Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)
        .wrap_err_with(|| format!("Failed to create config directory at {}", dir.display()))?;
    std::fs::create_dir_all(models_path()).wrap_err_with(|| {
        format!(
            "Failed to create models directory at {}",
            models_path().display()
        )
    })?;
    std::fs::create_dir_all(build_path()).wrap_err_with(|| {
        format!(
            "Failed to create build directory at {}",
            build_path().display()
        )
    })?;
    Ok(())
}

pub fn models_dir() -> PathBuf {
    models_path()
}

pub fn build_dir() -> PathBuf {
    build_path()
}

pub fn llama_cpp_dir() -> PathBuf {
    llama_cpp_path()
}
