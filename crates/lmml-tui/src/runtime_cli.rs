//! Headless runtime-profile commands for coding harness integration.

use std::env;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use lmml_state::{AppState, RuntimeConfig, RuntimeProfile, RuntimeProfileState, RuntimeStatus};
use serde_json::{json, Map, Value};
use thiserror::Error;

/// Result of planning or applying an OpenCode configuration update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurePlan {
    /// Path to the OpenCode config file.
    pub path: PathBuf,
    /// Human-readable structural diff lines.
    pub diff: Vec<String>,
    /// Routing decisions for top-level OpenCode model fields.
    pub routing: RoutingPlan,
    /// Whether conflicting existing lmml-owned keys were found.
    pub has_conflicts: bool,
    /// Whether provider entries conflict and require `--force`.
    pub has_provider_conflicts: bool,
    /// Whether top-level routing will replace an existing value.
    pub has_routing_conflicts: bool,
}

/// Result of applying an OpenCode configuration update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigureApply {
    /// Path written.
    pub path: PathBuf,
    /// Backup file created before writing.
    pub backup_path: PathBuf,
    /// Human-readable structural diff lines.
    pub diff: Vec<String>,
}

/// Source used when deciding whether to patch top-level OpenCode routing keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingSource {
    /// Preserve an existing key and do not create it when missing.
    Existing,
    /// Set the key to lmml's managed local provider/model.
    Lmml,
    /// Do not touch the key.
    None,
}

/// Routing choices for OpenCode's top-level model keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoutingOptions {
    /// Routing choice for `model`.
    pub model: RoutingSource,
    /// Routing choice for `small_model`.
    pub small_model: RoutingSource,
}

impl Default for RoutingOptions {
    fn default() -> Self {
        Self {
            model: RoutingSource::Lmml,
            small_model: RoutingSource::Lmml,
        }
    }
}

/// Rendered routing plan for dry-run output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutingPlan {
    /// Top-level `model` plan.
    pub model: RoutingDecision,
    /// Top-level `small_model` plan.
    pub small_model: RoutingDecision,
}

/// Routing decision for one top-level OpenCode key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutingDecision {
    /// OpenCode top-level key.
    pub key: &'static str,
    /// Requested source behavior.
    pub source: RoutingSource,
    /// Existing value in the config, if present.
    pub existing: Option<String>,
    /// lmml value, if applicable.
    pub lmml: String,
    /// Whether the existing and lmml values conflict for this source.
    pub conflict: bool,
}

/// Render runtime profile status as a stable table.
pub fn render_status(state: &AppState) -> String {
    let mut lines = vec![format!(
        "{:<14} {:<10} {:<7} {:<28} {}",
        "profile", "status", "pid", "url", "model"
    )];
    for name in RuntimeConfig::profile_names() {
        let Some(profile) = state.runtime.profile(name) else {
            continue;
        };
        let Some(runtime) = state.runtime.state.profile(name) else {
            continue;
        };
        lines.push(render_status_row(name, profile, runtime));
    }
    lines.join("\n")
}

/// Render OpenCode provider config as ready-to-paste JSON.
pub fn render_opencode_config(state: &AppState) -> Result<String, RuntimeCliError> {
    let value = desired_opencode_config(state)?;
    serde_json::to_string_pretty(&value).map_err(RuntimeCliError::SerializeJson)
}

/// Return warning lines for incomplete OpenCode runtime profiles.
pub fn opencode_config_warnings(state: &AppState) -> Vec<String> {
    RuntimeConfig::profile_names()
        .into_iter()
        .filter_map(|name| {
            let profile = state.runtime.profile(name)?;
            profile.model.as_os_str().is_empty().then(|| {
                format!(
                    "warning: runtime profile `{name}` has no model configured; start will fail until a model is selected"
                )
            })
        })
        .collect()
}

/// Return the default OpenCode config path from XDG/HOME.
pub fn default_opencode_config_path() -> PathBuf {
    default_opencode_config_path_from_env(
        env::var_os("XDG_CONFIG_HOME"),
        env::var_os("HOME"),
        env::var_os("USERPROFILE"),
    )
}

/// Build a dry-run plan for updating an OpenCode config.
pub fn plan_opencode_configure(
    state: &AppState,
    path: impl AsRef<Path>,
    routing: RoutingOptions,
    force: bool,
) -> Result<ConfigurePlan, RuntimeCliError> {
    let path = path.as_ref();
    let current = read_json_or_empty(path)?;
    let desired = desired_opencode_patch(state, routing)?;
    let routing_plan = build_routing_plan(&current, state, routing)?;
    validate_config_shape(&current, force)?;
    let diff = diff_values(
        "",
        &current,
        &merge_opencode_config(current.clone(), desired.clone())?,
    );
    let has_provider_conflicts = provider_conflicts(&current, &desired);
    let has_routing_conflicts = top_level_conflicts(&current, &desired);
    let has_conflicts = has_provider_conflicts || has_routing_conflicts;
    Ok(ConfigurePlan {
        path: path.to_path_buf(),
        diff,
        routing: routing_plan,
        has_conflicts,
        has_provider_conflicts,
        has_routing_conflicts,
    })
}

/// Apply an OpenCode config update after creating a backup.
pub fn apply_opencode_configure(
    state: &AppState,
    path: impl AsRef<Path>,
    routing: RoutingOptions,
    force: bool,
) -> Result<ConfigureApply, RuntimeCliError> {
    let path = path.as_ref();
    let current = read_json_or_empty(path)?;
    let desired = desired_opencode_patch(state, routing)?;
    validate_config_shape(&current, force)?;
    let has_provider_conflicts = provider_conflicts(&current, &desired);
    if has_provider_conflicts && !force {
        return Err(RuntimeCliError::Conflict);
    }
    let next = merge_opencode_config(current.clone(), desired)?;
    let diff = diff_values("", &current, &next);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| RuntimeCliError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let backup_path = create_backup(path)?;
    let temp_path = temp_path(path);
    if path.exists() {
        fs::copy(path, &backup_path).map_err(|source| RuntimeCliError::Backup {
            source: path.to_path_buf(),
            dest: backup_path.clone(),
            error: source,
        })?;
    } else {
        fs::write(&backup_path, "{}\n").map_err(|source| RuntimeCliError::Write {
            path: backup_path.clone(),
            source,
        })?;
    }

    let content = serde_json::to_string_pretty(&next).map_err(RuntimeCliError::SerializeJson)?;
    fs::write(&temp_path, format!("{content}\n")).map_err(|source| RuntimeCliError::Write {
        path: temp_path.clone(),
        source,
    })?;
    let written = fs::read_to_string(&temp_path).map_err(|source| RuntimeCliError::Read {
        path: temp_path.clone(),
        source,
    })?;
    serde_json::from_str::<Value>(&written).map_err(|source| RuntimeCliError::ParseJson {
        path: temp_path.clone(),
        source,
    })?;
    fs::rename(&temp_path, path).map_err(|source| RuntimeCliError::Rename {
        source: temp_path,
        dest: path.to_path_buf(),
        error: source,
    })?;

    Ok(ConfigureApply {
        path: path.to_path_buf(),
        backup_path,
        diff,
    })
}

/// Restore an OpenCode config from a JSON backup.
pub fn rollback_opencode_config(
    backup_path: impl AsRef<Path>,
    target_path: impl AsRef<Path>,
) -> Result<(), RuntimeCliError> {
    let backup_path = backup_path.as_ref();
    let target_path = target_path.as_ref();
    let content = fs::read_to_string(backup_path).map_err(|source| RuntimeCliError::Read {
        path: backup_path.to_path_buf(),
        source,
    })?;
    serde_json::from_str::<Value>(&content).map_err(|source| RuntimeCliError::ParseJson {
        path: backup_path.to_path_buf(),
        source,
    })?;
    fs::write(target_path, content).map_err(|source| RuntimeCliError::Write {
        path: target_path.to_path_buf(),
        source,
    })
}

fn render_status_row(
    name: &str,
    profile: &RuntimeProfile,
    runtime: &RuntimeProfileState,
) -> String {
    let pid = runtime
        .pid
        .map(|pid| pid.to_string())
        .unwrap_or_else(|| "-".to_string());
    let model_name = profile.model_name();
    let model = if model_name.is_empty() {
        "-".to_string()
    } else {
        model_name
    };
    format!(
        "{name:<14} {:<10} {pid:<7} {:<28} {model}",
        status_label(runtime.status),
        profile.api_base_url()
    )
}

fn status_label(status: RuntimeStatus) -> &'static str {
    match status {
        RuntimeStatus::Stopped => "stopped",
        RuntimeStatus::Starting => "starting",
        RuntimeStatus::Ready => "ready",
        RuntimeStatus::Unhealthy => "unhealthy",
        RuntimeStatus::Failed => "failed",
        RuntimeStatus::Stopping => "stopping",
    }
}

fn desired_opencode_config(state: &AppState) -> Result<Value, RuntimeCliError> {
    let full = state
        .runtime
        .profile("opencode")
        .ok_or(RuntimeCliError::UnknownProfile)?;
    let fast = state
        .runtime
        .profile("opencode-fast")
        .ok_or(RuntimeCliError::UnknownProfile)?;
    let full_model = opencode_model_name(full, "opencode");
    let fast_model = opencode_model_name(fast, "opencode-fast");
    Ok(json!({
        "provider": desired_provider_object(full, fast),
        "model": format!("llamacpp/{full_model}"),
        "small_model": format!("llamacpp_fast/{fast_model}"),
        "compaction": {
            "auto": true,
            "prune": true,
            "reserved": 32768
        }
    }))
}

fn desired_opencode_patch(
    state: &AppState,
    routing: RoutingOptions,
) -> Result<Value, RuntimeCliError> {
    let full = state
        .runtime
        .profile("opencode")
        .ok_or(RuntimeCliError::UnknownProfile)?;
    let fast = state
        .runtime
        .profile("opencode-fast")
        .ok_or(RuntimeCliError::UnknownProfile)?;
    let full_model = opencode_model_name(full, "opencode");
    let fast_model = opencode_model_name(fast, "opencode-fast");
    let mut patch = Map::new();
    patch.insert("provider".to_string(), desired_provider_object(full, fast));
    if routing.model == RoutingSource::Lmml {
        patch.insert(
            "model".to_string(),
            Value::String(format!("llamacpp/{full_model}")),
        );
    }
    if routing.small_model == RoutingSource::Lmml {
        patch.insert(
            "small_model".to_string(),
            Value::String(format!("llamacpp_fast/{fast_model}")),
        );
    }
    Ok(Value::Object(patch))
}

fn desired_provider_object(full: &RuntimeProfile, fast: &RuntimeProfile) -> Value {
    let full_model = opencode_model_name(full, "opencode");
    let fast_model = opencode_model_name(fast, "opencode-fast");
    json!({
        "llamacpp": {
            "npm": "@ai-sdk/openai-compatible",
            "name": "lmml llama.cpp",
            "options": {
                "baseURL": full.api_base_url(),
                "timeout": 7200000,
                "chunkTimeout": 300000
            },
            "models": {
                full_model.clone(): {
                    "name": format!("{full_model} (lmml full)")
                }
            }
        },
        "llamacpp_fast": {
            "npm": "@ai-sdk/openai-compatible",
            "name": "lmml llama.cpp fast",
            "options": {
                "baseURL": fast.api_base_url(),
                "timeout": 7200000,
                "chunkTimeout": 300000
            },
            "models": {
                fast_model.clone(): {
                    "name": format!("{fast_model} (lmml fast)")
                }
            }
        }
    })
}

fn opencode_model_name(profile: &RuntimeProfile, fallback: &str) -> String {
    let model_name = profile.model_name();
    if model_name.is_empty() {
        format!("{fallback}-model-unset.gguf")
    } else {
        model_name
    }
}

fn read_json_or_empty(path: &Path) -> Result<Value, RuntimeCliError> {
    if !path.exists() {
        return Ok(Value::Object(Map::new()));
    }
    let content = fs::read_to_string(path).map_err(|source| RuntimeCliError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&content).map_err(|source| RuntimeCliError::ParseJson {
        path: path.to_path_buf(),
        source,
    })
}

fn merge_opencode_config(mut current: Value, patch: Value) -> Result<Value, RuntimeCliError> {
    let current_object = object_mut(&mut current, "root")?;
    let patch_object = object_ref(&patch, "patch")?;
    for (key, value) in patch_object {
        if key == "provider" {
            let provider = current_object
                .entry("provider".to_string())
                .or_insert_with(|| Value::Object(Map::new()));
            let provider_object = object_mut(provider, "provider")?;
            let desired_providers = object_ref(value, "provider patch")?;
            for (provider_key, provider_value) in desired_providers {
                provider_object.insert(provider_key.clone(), provider_value.clone());
            }
        } else {
            current_object.insert(key.clone(), value.clone());
        }
    }
    Ok(current)
}

fn validate_config_shape(value: &Value, force: bool) -> Result<(), RuntimeCliError> {
    if !value.is_object() {
        return if force {
            Ok(())
        } else {
            Err(RuntimeCliError::UnexpectedJsonShape {
                path: "root".to_string(),
                expected: "object",
            })
        };
    }
    if let Some(provider) = value.get("provider") {
        if !provider.is_object() {
            return if force {
                Ok(())
            } else {
                Err(RuntimeCliError::UnexpectedJsonShape {
                    path: "provider".to_string(),
                    expected: "object",
                })
            };
        }
    }
    Ok(())
}

fn object_ref<'a>(value: &'a Value, path: &str) -> Result<&'a Map<String, Value>, RuntimeCliError> {
    value
        .as_object()
        .ok_or_else(|| RuntimeCliError::UnexpectedJsonShape {
            path: path.to_string(),
            expected: "object",
        })
}

fn object_mut<'a>(
    value: &'a mut Value,
    path: &str,
) -> Result<&'a mut Map<String, Value>, RuntimeCliError> {
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    value
        .as_object_mut()
        .ok_or_else(|| RuntimeCliError::UnexpectedJsonShape {
            path: path.to_string(),
            expected: "object",
        })
}

fn build_routing_plan(
    current: &Value,
    state: &AppState,
    routing: RoutingOptions,
) -> Result<RoutingPlan, RuntimeCliError> {
    let full = state
        .runtime
        .profile("opencode")
        .ok_or(RuntimeCliError::UnknownProfile)?;
    let fast = state
        .runtime
        .profile("opencode-fast")
        .ok_or(RuntimeCliError::UnknownProfile)?;
    let model_lmml = format!("llamacpp/{}", opencode_model_name(full, "opencode"));
    let small_model_lmml = format!(
        "llamacpp_fast/{}",
        opencode_model_name(fast, "opencode-fast")
    );
    Ok(RoutingPlan {
        model: routing_decision(current, "model", routing.model, model_lmml),
        small_model: routing_decision(
            current,
            "small_model",
            routing.small_model,
            small_model_lmml,
        ),
    })
}

fn routing_decision(
    current: &Value,
    key: &'static str,
    source: RoutingSource,
    lmml: String,
) -> RoutingDecision {
    let existing = current
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let conflict = source == RoutingSource::Lmml
        && existing.as_ref().is_some_and(|existing| existing != &lmml);
    RoutingDecision {
        key,
        source,
        existing,
        lmml,
        conflict,
    }
}

fn top_level_conflicts(current: &Value, desired: &Value) -> bool {
    ["model", "small_model"].into_iter().any(|key| {
        current
            .get(key)
            .zip(desired.get(key))
            .is_some_and(|(current, desired)| current != desired)
    })
}

fn provider_conflicts(current: &Value, desired: &Value) -> bool {
    let Some(current_provider) = current.get("provider").and_then(Value::as_object) else {
        return false;
    };
    let Some(desired_provider) = desired.get("provider").and_then(Value::as_object) else {
        return false;
    };
    ["llamacpp", "llamacpp_fast"].into_iter().any(|key| {
        current_provider
            .get(key)
            .zip(desired_provider.get(key))
            .is_some_and(|(current, desired)| current != desired)
    })
}

fn diff_values(path: &str, before: &Value, after: &Value) -> Vec<String> {
    if before == after {
        return Vec::new();
    }
    match (before, after) {
        (Value::Object(before), Value::Object(after)) => {
            let mut keys = before.keys().chain(after.keys()).collect::<Vec<_>>();
            keys.sort();
            keys.dedup();
            let mut lines = Vec::new();
            for key in keys {
                let child_path = if path.is_empty() {
                    key.to_string()
                } else {
                    format!("{path}.{key}")
                };
                match (before.get(key), after.get(key)) {
                    (None, Some(value)) => {
                        lines.push(format!("+ {child_path}: {}", compact(value)))
                    }
                    (Some(value), None) => {
                        lines.push(format!("- {child_path}: {}", compact(value)))
                    }
                    (Some(left), Some(right)) => {
                        lines.extend(diff_values(&child_path, left, right))
                    }
                    (None, None) => {}
                }
            }
            lines
        }
        _ => vec![format!(
            "~ {path}: {} -> {}",
            compact(before),
            compact(after)
        )],
    }
}

fn compact(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "<json>".to_string())
}

fn create_backup(path: &Path) -> Result<PathBuf, RuntimeCliError> {
    for attempt in 0..1000 {
        let backup_path = backup_path(path, attempt);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&backup_path)
        {
            Ok(_) => return Ok(backup_path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(RuntimeCliError::Write {
                    path: backup_path,
                    source,
                });
            }
        }
    }
    Err(RuntimeCliError::BackupNameExhausted {
        path: path.to_path_buf(),
    })
}

fn backup_path(path: &Path, attempt: u32) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("opencode.json");
    path.with_file_name(format!("{file_name}.bak-{suffix}-{pid}-{attempt}"))
}

fn temp_path(path: &Path) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("opencode.json");
    path.with_file_name(format!("{file_name}.tmp-{suffix}-{pid}"))
}

fn default_opencode_config_path_from_env(
    xdg_config_home: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
    userprofile: Option<std::ffi::OsString>,
) -> PathBuf {
    if let Some(path) = xdg_config_home.filter(|path| !path.is_empty()) {
        return PathBuf::from(path).join("opencode").join("opencode.json");
    }
    if let Some(path) = home.filter(|path| !path.is_empty()) {
        return PathBuf::from(path)
            .join(".config")
            .join("opencode")
            .join("opencode.json");
    }
    if let Some(path) = userprofile.filter(|path| !path.is_empty()) {
        return PathBuf::from(path)
            .join(".config")
            .join("opencode")
            .join("opencode.json");
    }
    PathBuf::from(".")
        .join(".config")
        .join("opencode")
        .join("opencode.json")
}

/// Errors returned by runtime-profile CLI helpers.
#[derive(Debug, Error)]
pub enum RuntimeCliError {
    /// Runtime profile was not known.
    #[error("unknown runtime profile")]
    UnknownProfile,
    /// Existing lmml-owned OpenCode provider differs from the desired value.
    #[error("OpenCode config contains conflicting lmml provider entries; review --dry-run output and rerun with --force if intended")]
    Conflict,
    /// JSON had an unsupported shape for safe structural patching.
    #[error("unexpected OpenCode JSON shape at {path}: expected {expected}")]
    UnexpectedJsonShape {
        /// JSON path.
        path: String,
        /// Expected JSON type.
        expected: &'static str,
    },
    /// Could not allocate a collision-free backup name.
    #[error("failed to allocate backup path for {path}")]
    BackupNameExhausted {
        /// Target config path.
        path: PathBuf,
    },
    /// Could not create a parent directory.
    #[error("failed to create directory {path}: {source}")]
    CreateDir {
        /// Directory path.
        path: PathBuf,
        /// Source IO error.
        #[source]
        source: std::io::Error,
    },
    /// Could not read a file.
    #[error("failed to read {path}: {source}")]
    Read {
        /// File path.
        path: PathBuf,
        /// Source IO error.
        #[source]
        source: std::io::Error,
    },
    /// Could not parse JSON.
    #[error("failed to parse JSON at {path}: {source}")]
    ParseJson {
        /// File path.
        path: PathBuf,
        /// JSON parser error.
        #[source]
        source: serde_json::Error,
    },
    /// Could not serialize JSON.
    #[error("failed to serialize JSON: {0}")]
    SerializeJson(#[source] serde_json::Error),
    /// Could not create a backup.
    #[error("failed to back up {source} to {dest}: {error}")]
    Backup {
        /// Original file.
        source: PathBuf,
        /// Backup file.
        dest: PathBuf,
        /// Source IO error.
        #[source]
        error: std::io::Error,
    },
    /// Could not write a file.
    #[error("failed to write {path}: {source}")]
    Write {
        /// File path.
        path: PathBuf,
        /// Source IO error.
        #[source]
        source: std::io::Error,
    },
    /// Could not rename a file.
    #[error("failed to rename {source} to {dest}: {error}")]
    Rename {
        /// Temporary file path.
        source: PathBuf,
        /// Destination path.
        dest: PathBuf,
        /// Source IO error.
        #[source]
        error: std::io::Error,
    },
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn status_table_renders_builtin_profiles() {
        let mut state = AppState::default();
        state.runtime.opencode.model = PathBuf::from("/models/full.gguf");
        state.runtime.state.opencode.status = RuntimeStatus::Ready;
        state.runtime.state.opencode.pid = Some(1234);

        let table = render_status(&state);

        assert!(table.contains("opencode"));
        assert!(table.contains("ready"));
        assert!(table.contains("1234"));
        assert!(table.contains("http://127.0.0.1:4010/v1"));
        assert!(table.contains("opencode-fast"));
    }

    #[test]
    fn opencode_config_matches_managed_profiles() {
        let mut state = AppState::default();
        state.runtime.opencode.model = PathBuf::from("/models/full.gguf");
        state.runtime.opencode_fast.model = PathBuf::from("/models/fast.gguf");

        let rendered = render_opencode_config(&state).expect("render config");
        let json: Value = serde_json::from_str(&rendered).expect("valid json");

        assert_eq!(
            json["provider"]["llamacpp"]["options"]["baseURL"],
            "http://127.0.0.1:4010/v1"
        );
        assert_eq!(
            json["provider"]["llamacpp_fast"]["options"]["baseURL"],
            "http://127.0.0.1:4011/v1"
        );
        assert_eq!(json["model"], "llamacpp/full.gguf");
        assert_eq!(json["small_model"], "llamacpp_fast/fast.gguf");
    }

    #[test]
    fn dry_run_preserves_unrelated_opencode_keys() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let config_path = tempdir.path().join("opencode.json");
        fs::write(
            &config_path,
            r#"{"plugin":["oh-my-openagent@latest"],"provider":{"openai":{"npm":"x"}}}"#,
        )
        .expect("write config");

        let state = AppState::default();
        let plan = plan_opencode_configure(&state, &config_path, RoutingOptions::default(), false)
            .expect("plan");

        assert!(!plan.has_conflicts);
        assert!(plan
            .diff
            .iter()
            .any(|line| line.contains("provider.llamacpp")));
        let current = read_json_or_empty(&config_path).expect("read current");
        assert_eq!(current["plugin"][0], "oh-my-openagent@latest");
        assert_eq!(current["provider"]["openai"]["npm"], "x");
    }

    #[test]
    fn configure_detects_conflicting_lmml_provider() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let config_path = tempdir.path().join("opencode.json");
        fs::write(
            &config_path,
            r#"{"provider":{"llamacpp":{"name":"custom existing"}}}"#,
        )
        .expect("write config");

        let state = AppState::default();
        let plan = plan_opencode_configure(&state, &config_path, RoutingOptions::default(), false)
            .expect("plan");

        assert!(plan.has_conflicts);
        assert!(plan.has_provider_conflicts);
        assert!(!plan.has_routing_conflicts);
        assert!(matches!(
            apply_opencode_configure(&state, &config_path, RoutingOptions::default(), false),
            Err(RuntimeCliError::Conflict)
        ));
    }

    #[test]
    fn default_local_first_replaces_cloud_routing_without_force() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let config_path = tempdir.path().join("opencode.json");
        fs::write(
            &config_path,
            r#"{"model":"openai/gpt-4o","small_model":"openai/gpt-4o-mini"}"#,
        )
        .expect("write config");

        let mut state = AppState::default();
        state.runtime.opencode.model = PathBuf::from("/models/full.gguf");
        state.runtime.opencode_fast.model = PathBuf::from("/models/fast.gguf");
        let plan = plan_opencode_configure(&state, &config_path, RoutingOptions::default(), false)
            .expect("plan");

        assert!(plan.has_conflicts);
        assert!(!plan.has_provider_conflicts);
        assert!(plan.has_routing_conflicts);
        apply_opencode_configure(&state, &config_path, RoutingOptions::default(), false)
            .expect("apply config");
        let updated = read_json_or_empty(&config_path).expect("read updated");

        assert_eq!(updated["model"], "llamacpp/full.gguf");
        assert_eq!(updated["small_model"], "llamacpp_fast/fast.gguf");
    }

    #[test]
    fn configure_existing_routing_preserves_cloud_models() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let config_path = tempdir.path().join("opencode.json");
        fs::write(
            &config_path,
            r#"{"model":"anthropic/claude-sonnet-4-5","small_model":"openai/gpt-4.1-mini"}"#,
        )
        .expect("write config");

        let state = AppState::default();
        let routing = RoutingOptions {
            model: RoutingSource::Existing,
            small_model: RoutingSource::Existing,
        };
        let plan = plan_opencode_configure(&state, &config_path, routing, false).expect("plan");
        let applied =
            apply_opencode_configure(&state, &config_path, routing, false).expect("apply config");
        let updated = read_json_or_empty(&config_path).expect("read updated");

        assert!(!plan.has_conflicts);
        assert!(applied.backup_path.exists());
        assert_eq!(updated["model"], "anthropic/claude-sonnet-4-5");
        assert_eq!(updated["small_model"], "openai/gpt-4.1-mini");
        assert_eq!(
            updated["provider"]["llamacpp"]["options"]["baseURL"],
            "http://127.0.0.1:4010/v1"
        );
    }

    #[test]
    fn configure_mixed_routing_updates_only_requested_model_key() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let config_path = tempdir.path().join("opencode.json");
        fs::write(
            &config_path,
            r#"{"model":"anthropic/claude-sonnet-4-5","small_model":"openai/gpt-4.1-mini"}"#,
        )
        .expect("write config");

        let mut state = AppState::default();
        state.runtime.opencode_fast.model = PathBuf::from("/models/fast.gguf");
        let routing = RoutingOptions {
            model: RoutingSource::Existing,
            small_model: RoutingSource::Lmml,
        };
        let plan = plan_opencode_configure(&state, &config_path, routing, false).expect("plan");

        assert!(plan.has_conflicts);
        assert!(plan.has_routing_conflicts);
        apply_opencode_configure(&state, &config_path, routing, false).expect("apply config");
        let updated = read_json_or_empty(&config_path).expect("read updated");

        assert_eq!(updated["model"], "anthropic/claude-sonnet-4-5");
        assert_eq!(updated["small_model"], "llamacpp_fast/fast.gguf");
    }

    #[test]
    fn configure_default_local_first_writes_both_routing_keys() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let config_path = tempdir.path().join("opencode.json");
        fs::write(&config_path, r#"{"plugin":["keep"]}"#).expect("write config");

        let mut state = AppState::default();
        state.runtime.opencode.model = PathBuf::from("/models/full.gguf");
        state.runtime.opencode_fast.model = PathBuf::from("/models/fast.gguf");
        apply_opencode_configure(&state, &config_path, RoutingOptions::default(), false)
            .expect("apply config");
        let updated = read_json_or_empty(&config_path).expect("read updated");

        assert_eq!(updated["model"], "llamacpp/full.gguf");
        assert_eq!(updated["small_model"], "llamacpp_fast/fast.gguf");
    }

    #[test]
    fn configure_rejects_non_object_root_without_force() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let config_path = tempdir.path().join("opencode.json");
        fs::write(&config_path, r#"[]"#).expect("write config");

        let state = AppState::default();
        let error = plan_opencode_configure(&state, &config_path, RoutingOptions::default(), false)
            .expect_err("shape error");

        assert!(matches!(
            error,
            RuntimeCliError::UnexpectedJsonShape { ref path, .. } if path == "root"
        ));
    }

    #[test]
    fn configure_rejects_non_object_provider_without_force() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let config_path = tempdir.path().join("opencode.json");
        fs::write(&config_path, r#"{"provider":[]}"#).expect("write config");

        let state = AppState::default();
        let error = plan_opencode_configure(&state, &config_path, RoutingOptions::default(), false)
            .expect_err("shape error");

        assert!(matches!(
            error,
            RuntimeCliError::UnexpectedJsonShape { ref path, .. } if path == "provider"
        ));
    }

    #[test]
    fn configure_apply_writes_backup_and_preserves_unrelated_keys() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let config_path = tempdir.path().join("opencode.json");
        fs::write(&config_path, r#"{"plugin":["keep"]}"#).expect("write config");

        let state = AppState::default();
        let applied =
            apply_opencode_configure(&state, &config_path, RoutingOptions::default(), false)
                .expect("apply config");
        let updated = read_json_or_empty(&config_path).expect("read updated");

        assert!(applied.backup_path.exists());
        assert_eq!(updated["plugin"][0], "keep");
        assert_eq!(
            updated["provider"]["llamacpp"]["options"]["baseURL"],
            "http://127.0.0.1:4010/v1"
        );
    }

    #[test]
    fn configure_apply_creates_distinct_backups() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let config_path = tempdir.path().join("opencode.json");
        fs::write(&config_path, r#"{"plugin":["keep"]}"#).expect("write config");

        let state = AppState::default();
        let first =
            apply_opencode_configure(&state, &config_path, RoutingOptions::default(), false)
                .expect("first apply");
        let second =
            apply_opencode_configure(&state, &config_path, RoutingOptions::default(), true)
                .expect("second apply");

        assert_ne!(first.backup_path, second.backup_path);
        assert!(first.backup_path.exists());
        assert!(second.backup_path.exists());
    }

    #[test]
    fn rollback_restores_backup_content() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let config_path = tempdir.path().join("opencode.json");
        let backup_path = tempdir.path().join("opencode.json.bak");
        fs::write(&config_path, r#"{"provider":{"llamacpp":{"name":"new"}}}"#)
            .expect("write config");
        fs::write(&backup_path, r#"{"provider":{"openai":{"name":"old"}}}"#).expect("write backup");

        rollback_opencode_config(&backup_path, &config_path).expect("rollback");
        let restored = read_json_or_empty(&config_path).expect("read restored");

        assert_eq!(restored["provider"]["openai"]["name"], "old");
        assert!(restored["provider"].get("llamacpp").is_none());
    }

    #[test]
    fn default_opencode_path_respects_xdg_config_home() {
        assert_eq!(
            default_opencode_config_path_from_env(
                Some("/tmp/config".into()),
                Some("/home/user".into()),
                None
            ),
            PathBuf::from("/tmp/config/opencode/opencode.json")
        );
    }
}
