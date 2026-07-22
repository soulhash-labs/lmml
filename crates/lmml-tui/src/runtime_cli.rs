//! Headless runtime-profile commands for coding harness integration.

use std::env;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

/// Result of starting a managed runtime profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStartResult {
    /// Profile name.
    pub profile: String,
    /// Process ID.
    pub pid: u32,
    /// Ready URL.
    pub url: String,
    /// Log file path.
    pub log_path: PathBuf,
}

/// Result of stopping a managed runtime profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStopResult {
    /// Profile name.
    pub profile: String,
    /// Process ID that was stopped, if any.
    pub pid: Option<u32>,
    /// Human-readable stop status.
    pub message: String,
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

/// Render a Codex profile config that points Codex at lmml's Responses endpoint.
pub fn render_codex_config(state: &AppState) -> Result<String, RuntimeCliError> {
    let profile = state
        .runtime
        .profile("opencode")
        .ok_or(RuntimeCliError::UnknownProfile)?;
    let model = codex_model_name(profile);
    let base_url = profile.api_base_url();
    Ok(format!(
        concat!(
            "# ~/.codex/lmml.config.toml\n",
            "# Run with: codex --profile lmml\n",
            "model_provider = \"lmml\"\n",
            "model = {model}\n",
            "\n",
            "[model_providers.lmml]\n",
            "name = \"lmml\"\n",
            "base_url = {base_url}\n",
            "wire_api = \"responses\"\n"
        ),
        model = toml_string(&model),
        base_url = toml_string(&base_url),
    ))
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

/// Reconcile in-memory runtime status with currently alive PIDs.
pub fn reconcile_runtime_state(state: &mut AppState) {
    for name in RuntimeConfig::profile_names() {
        let Some(runtime) = state.runtime.state.profile_mut(name) else {
            continue;
        };
        let Some(pid) = runtime.pid else {
            continue;
        };
        if !pid_is_alive(pid) {
            runtime.status = RuntimeStatus::Stopped;
            runtime.pid = None;
            runtime.last_health = "pid not running".to_string();
        }
    }
}

/// Return whether a runtime profile name is built in and safe for path use.
pub fn is_known_profile(profile_name: &str) -> bool {
    RuntimeConfig::profile_names()
        .into_iter()
        .any(|name| name == profile_name)
}

/// Start a detached llama-server process for a runtime profile.
pub async fn start_profile(
    state: &mut AppState,
    profile_name: &str,
    startup_timeout: Duration,
) -> Result<RuntimeStartResult, RuntimeCliError> {
    let profile = state
        .runtime
        .profile(profile_name)
        .ok_or_else(|| RuntimeCliError::UnknownProfileName(profile_name.to_string()))?
        .clone();
    prevent_double_start(state, profile_name)?;
    validate_start_profile(profile_name, &profile, state)?;
    lmml_server::check_port_free(&profile.host, profile.port)
        .await
        .map_err(|source| RuntimeCliError::PortUnavailable {
            host: profile.host.clone(),
            port: profile.port,
            source,
        })?;

    let caps = lmml_compat::LlamaBinaryCapabilities::probe(&state.build.binary)
        .await
        .map_err(RuntimeCliError::Compat)?;
    let config = profile_to_server_config(&profile);
    let argv = lmml_compat::build_argv(&config, &caps);
    let log_path = runtime_log_path(profile_name);
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).map_err(|source| RuntimeCliError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|source| RuntimeCliError::Write {
            path: log_path.clone(),
            source,
        })?;
    let log_err = log.try_clone().map_err(|source| RuntimeCliError::Write {
        path: log_path.clone(),
        source,
    })?;

    let mut command = tokio::process::Command::new(&state.build.binary);
    command
        .args(&argv)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));
    detach_command(&mut command);
    let child = command.spawn().map_err(|source| RuntimeCliError::Spawn {
        binary: state.build.binary.clone(),
        source,
    })?;
    let pid = child.id().ok_or(RuntimeCliError::MissingPid)?;
    update_runtime_starting(state, profile_name, &profile, pid, &log_path)?;
    state.save()?;

    match lmml_server::wait_for_ready(&profile.host, profile.port, startup_timeout).await {
        Ok(url) => {
            update_runtime_ready(state, profile_name, &url)?;
            state.save()?;
            Ok(RuntimeStartResult {
                profile: profile_name.to_string(),
                pid,
                url,
                log_path,
            })
        }
        Err(error) => {
            let kill_result = terminate_pid(pid).await;
            if let Err(kill_error) = kill_result {
                let startup = error.to_string();
                let cleanup = kill_error.to_string();
                update_runtime_failed(
                    state,
                    profile_name,
                    format!("{startup}; additionally failed to terminate process group: {cleanup}"),
                )?;
                state.save()?;
                return Err(RuntimeCliError::StartupCleanup {
                    profile: profile_name.to_string(),
                    startup,
                    cleanup,
                });
            }
            update_runtime_failed(state, profile_name, error.to_string())?;
            state.save()?;
            Err(RuntimeCliError::Startup {
                profile: profile_name.to_string(),
                source: error,
            })
        }
    }
}

/// Stop a detached runtime profile process recorded in state.
pub async fn stop_profile(
    state: &mut AppState,
    profile_name: &str,
) -> Result<RuntimeStopResult, RuntimeCliError> {
    let binary = state.build.binary.clone();
    let runtime = state
        .runtime
        .state
        .profile_mut(profile_name)
        .ok_or_else(|| RuntimeCliError::UnknownProfileName(profile_name.to_string()))?;
    let Some(pid) = runtime.pid else {
        runtime.status = RuntimeStatus::Stopped;
        state.save()?;
        return Ok(RuntimeStopResult {
            profile: profile_name.to_string(),
            pid: None,
            message: "already stopped".to_string(),
        });
    };
    if !pid_is_alive(pid) {
        runtime.status = RuntimeStatus::Stopped;
        runtime.pid = None;
        runtime.last_health = "pid not running".to_string();
        state.save()?;
        return Ok(RuntimeStopResult {
            profile: profile_name.to_string(),
            pid: Some(pid),
            message: "stale pid cleared".to_string(),
        });
    }
    if !pid_looks_like_llama_server(pid, &binary) {
        runtime.status = RuntimeStatus::Unhealthy;
        runtime.last_health = "recorded pid does not look like llama-server".to_string();
        state.save()?;
        return Err(RuntimeCliError::PidMismatch {
            profile: profile_name.to_string(),
            pid,
        });
    }
    runtime.status = RuntimeStatus::Stopping;
    state.save()?;
    if let Err(error) = terminate_pid(pid).await {
        let runtime = state
            .runtime
            .state
            .profile_mut(profile_name)
            .ok_or_else(|| RuntimeCliError::UnknownProfileName(profile_name.to_string()))?;
        runtime.status = RuntimeStatus::Failed;
        runtime.last_health = error.to_string();
        runtime.failure_count = runtime.failure_count.saturating_add(1);
        state.save()?;
        return Err(error);
    }
    let runtime = state
        .runtime
        .state
        .profile_mut(profile_name)
        .ok_or_else(|| RuntimeCliError::UnknownProfileName(profile_name.to_string()))?;
    runtime.status = RuntimeStatus::Stopped;
    runtime.pid = None;
    runtime.last_health = "stopped".to_string();
    state.save()?;
    Ok(RuntimeStopResult {
        profile: profile_name.to_string(),
        pid: Some(pid),
        message: "stopped".to_string(),
    })
}

/// Return the log path for a profile.
pub fn runtime_log_path(profile_name: &str) -> PathBuf {
    AppState::runtime_log_dir().join(format!("{profile_name}.log"))
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

fn validate_start_profile(
    profile_name: &str,
    profile: &RuntimeProfile,
    state: &AppState,
) -> Result<(), RuntimeCliError> {
    if profile.model.as_os_str().is_empty() {
        return Err(RuntimeCliError::MissingModel {
            profile: profile_name.to_string(),
        });
    }
    if !profile.model.exists() {
        return Err(RuntimeCliError::MissingModelPath {
            profile: profile_name.to_string(),
            path: profile.model.clone(),
        });
    }
    if !state.build.binary.exists() {
        return Err(RuntimeCliError::MissingServerBinary {
            path: state.build.binary.clone(),
        });
    }
    Ok(())
}

fn prevent_double_start(state: &AppState, profile_name: &str) -> Result<(), RuntimeCliError> {
    let Some(runtime) = state.runtime.state.profile(profile_name) else {
        return Err(RuntimeCliError::UnknownProfileName(
            profile_name.to_string(),
        ));
    };
    let Some(pid) = runtime.pid else {
        return Ok(());
    };
    if !pid_is_alive(pid) {
        return Ok(());
    }
    if pid_looks_like_llama_server(pid, &state.build.binary) {
        return Err(RuntimeCliError::AlreadyRunning {
            profile: profile_name.to_string(),
            pid,
        });
    }
    Err(RuntimeCliError::PidMismatch {
        profile: profile_name.to_string(),
        pid,
    })
}

fn profile_to_server_config(profile: &RuntimeProfile) -> lmml_compat::ServerConfig {
    lmml_compat::ServerConfig {
        model: profile.model.clone(),
        port: profile.port,
        host: profile.host.clone(),
        ctx_size: profile.ctx_size,
        n_gpu_layers: profile.gpu_layers,
        batch_size: profile.batch_size,
        ubatch_size: profile.batch_size,
        threads: profile.threads,
        flash_attn: true,
        mlock: false,
        api_key: None,
        chat_template: None,
        jinja: false,
        extra_args: profile.extra_args.clone(),
    }
}

fn update_runtime_starting(
    state: &mut AppState,
    profile_name: &str,
    profile: &RuntimeProfile,
    pid: u32,
    log_path: &Path,
) -> Result<(), RuntimeCliError> {
    let runtime = state
        .runtime
        .state
        .profile_mut(profile_name)
        .ok_or_else(|| RuntimeCliError::UnknownProfileName(profile_name.to_string()))?;
    runtime.status = RuntimeStatus::Starting;
    runtime.pid = Some(pid);
    runtime.host = profile.host.clone();
    runtime.port = profile.port;
    runtime.model = profile.model.clone();
    runtime.log_path = log_path.to_path_buf();
    runtime.started_at = unix_timestamp_string();
    runtime.last_health_at = String::new();
    runtime.last_health = "starting".to_string();
    runtime.failure_count = 0;
    Ok(())
}

fn update_runtime_ready(
    state: &mut AppState,
    profile_name: &str,
    url: &str,
) -> Result<(), RuntimeCliError> {
    let runtime = state
        .runtime
        .state
        .profile_mut(profile_name)
        .ok_or_else(|| RuntimeCliError::UnknownProfileName(profile_name.to_string()))?;
    runtime.status = RuntimeStatus::Ready;
    runtime.last_health_at = unix_timestamp_string();
    runtime.last_health = format!("ready {url}");
    runtime.failure_count = 0;
    Ok(())
}

fn update_runtime_failed(
    state: &mut AppState,
    profile_name: &str,
    reason: String,
) -> Result<(), RuntimeCliError> {
    let runtime = state
        .runtime
        .state
        .profile_mut(profile_name)
        .ok_or_else(|| RuntimeCliError::UnknownProfileName(profile_name.to_string()))?;
    runtime.status = RuntimeStatus::Failed;
    runtime.last_health_at = unix_timestamp_string();
    runtime.last_health = reason;
    runtime.failure_count = runtime.failure_count.saturating_add(1);
    Ok(())
}

fn unix_timestamp_string() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

fn pid_is_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        if pid_is_zombie(pid) {
            return false;
        }
        // SAFETY: kill with signal 0 only checks whether the process exists.
        if unsafe { libc::kill(pid as i32, 0) == 0 } {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

#[cfg(target_os = "linux")]
fn pid_is_zombie(pid: u32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .is_some_and(|stat| {
            stat.rsplit_once(") ")
                .and_then(|(_, rest)| rest.split_whitespace().next())
                .is_some_and(|state| state == "Z")
        })
}

#[cfg(all(unix, not(target_os = "linux")))]
fn pid_is_zombie(pid: u32) -> bool {
    std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "stat="])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .trim_start()
                .starts_with('Z')
        })
        .unwrap_or(false)
}

fn pid_looks_like_llama_server(pid: u32, binary: &Path) -> bool {
    let expected = binary
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("llama-server");
    std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).contains(expected))
        .unwrap_or(false)
}

async fn terminate_pid(pid: u32) -> Result<(), RuntimeCliError> {
    #[cfg(unix)]
    {
        signal_process_group(pid, libc::SIGTERM)?;
        for _ in 0..50 {
            if !pid_is_alive(pid) {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        signal_process_group(pid, libc::SIGKILL)?;
        for _ in 0..50 {
            if !pid_is_alive(pid) {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err(RuntimeCliError::KillTimeout { pid })
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        Err(RuntimeCliError::UnsupportedPlatform)
    }
}

#[cfg(unix)]
fn signal_process_group(pid: u32, signal: i32) -> Result<(), RuntimeCliError> {
    let pgid = -(pid as i32);
    // SAFETY: pid came from lmml runtime state and start_profile puts it in a new session.
    if unsafe { libc::kill(pgid, signal) } == 0 {
        return Ok(());
    }
    let source = std::io::Error::last_os_error();
    if source.raw_os_error() == Some(libc::ESRCH) && !pid_is_alive(pid) {
        return Ok(());
    }
    Err(RuntimeCliError::Kill { pid, source })
}

fn detach_command(command: &mut tokio::process::Command) {
    #[cfg(unix)]
    {
        // SAFETY: pre_exec runs only async-signal-safe setsid before exec.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    #[cfg(not(unix))]
    {
        let _ = command;
    }
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

fn codex_model_name(profile: &RuntimeProfile) -> String {
    let model_name = profile.model_name();
    if model_name.is_empty() {
        "lmml-model-unset.gguf".to_string()
    } else {
        model_name
    }
}

fn toml_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
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
    /// Named runtime profile was not known.
    #[error("unknown runtime profile `{0}`")]
    UnknownProfileName(String),
    /// Runtime profile has no model configured.
    #[error("runtime profile `{profile}` has no model configured")]
    MissingModel {
        /// Profile name.
        profile: String,
    },
    /// Runtime profile model path does not exist.
    #[error("runtime profile `{profile}` model does not exist: {path}")]
    MissingModelPath {
        /// Profile name.
        profile: String,
        /// Missing model path.
        path: PathBuf,
    },
    /// Built llama-server binary is missing.
    #[error("llama-server binary does not exist: {path}")]
    MissingServerBinary {
        /// Missing binary path.
        path: PathBuf,
    },
    /// Runtime profile already has a live managed process.
    #[error("runtime profile `{profile}` is already running with pid {pid}; stop it first")]
    AlreadyRunning {
        /// Profile name.
        profile: String,
        /// Existing PID.
        pid: u32,
    },
    /// A configured runtime port is unavailable.
    #[error("runtime port {host}:{port} is unavailable: {source}")]
    PortUnavailable {
        /// Host.
        host: String,
        /// Port.
        port: u16,
        /// Source server error.
        #[source]
        source: lmml_server::ServerError,
    },
    /// llama-server compatibility probing failed.
    #[error("failed to probe llama-server compatibility: {0}")]
    Compat(#[source] lmml_compat::CompatError),
    /// Failed to spawn llama-server.
    #[error("failed to spawn llama-server at {binary}: {source}")]
    Spawn {
        /// Binary path.
        binary: PathBuf,
        /// Source IO error.
        #[source]
        source: std::io::Error,
    },
    /// Spawned process had no PID.
    #[error("spawned llama-server process did not report a pid")]
    MissingPid,
    /// Server startup failed.
    #[error("runtime profile `{profile}` failed to become ready: {source}")]
    Startup {
        /// Profile name.
        profile: String,
        /// Source server error.
        #[source]
        source: lmml_server::ServerError,
    },
    /// Startup failed and cleanup also failed.
    #[error("runtime profile `{profile}` failed to become ready ({startup}) and cleanup failed ({cleanup})")]
    StartupCleanup {
        /// Profile name.
        profile: String,
        /// Startup failure.
        startup: String,
        /// Cleanup failure.
        cleanup: String,
    },
    /// Persisted PID did not look like a llama-server process.
    #[error("runtime profile `{profile}` pid {pid} does not look like a managed llama-server")]
    PidMismatch {
        /// Profile name.
        profile: String,
        /// Recorded PID.
        pid: u32,
    },
    /// Killing a process failed.
    #[error("failed to signal pid {pid}: {source}")]
    Kill {
        /// PID.
        pid: u32,
        /// Source IO error.
        #[source]
        source: std::io::Error,
    },
    /// Process still appeared alive after SIGKILL wait.
    #[error("pid {pid} was still alive after SIGKILL")]
    KillTimeout {
        /// PID.
        pid: u32,
    },
    /// Platform does not support this process operation.
    #[error("runtime process operation is unsupported on this platform")]
    UnsupportedPlatform,
    /// State load/save failed.
    #[error("state error: {0}")]
    State(#[from] lmml_state::StateError),
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
        assert!(table.contains("http://127.0.0.1:1200/v1"));
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
            "http://127.0.0.1:1200/v1"
        );
        assert_eq!(
            json["provider"]["llamacpp_fast"]["options"]["baseURL"],
            "http://127.0.0.1:1200/v1"
        );
        assert_eq!(json["model"], "llamacpp/full.gguf");
        assert_eq!(json["small_model"], "llamacpp_fast/fast.gguf");
    }

    #[test]
    fn codex_config_targets_responses_endpoint() {
        let mut state = AppState::default();
        state.runtime.opencode.model = PathBuf::from("/models/full.gguf");

        let rendered = render_codex_config(&state).expect("render config");

        assert!(rendered.contains("model_provider = \"lmml\""));
        assert!(rendered.contains("model = \"full.gguf\""));
        assert!(rendered.contains("base_url = \"http://127.0.0.1:1200/v1\""));
        assert!(rendered.contains("wire_api = \"responses\""));
        assert!(rendered.contains("codex --profile lmml"));
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
            "http://127.0.0.1:1200/v1"
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
            "http://127.0.0.1:1200/v1"
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

    #[cfg(unix)]
    #[tokio::test]
    async fn terminate_pid_treats_missing_process_group_as_stopped() {
        let missing_pid = 999_999_999;

        terminate_pid(missing_pid)
            .await
            .expect("missing pid is already stopped");
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
