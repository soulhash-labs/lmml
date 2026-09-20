//! Transactional OpenCode and Oh My OpenAgent route synchronization.
//!
//! This module keeps the single-server model identifier aligned across both
//! user-owned JSON files without replacing unrelated providers, plugins,
//! instructions, snapshot controls, or compaction policy.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use lmml_state::AppState;
use serde_json::{Map, Value};

use super::{
    create_backup, desired_opencode_patch, diff_values, merge_opencode_config, opencode_model_name,
    opencode_primary_profile, read_json_or_empty, temp_path, validate_config_shape, ConfigurePlan,
    RoutingOptions, RuntimeCliError,
};

/// Dry-run result for synchronized OpenCode and Oh My OpenAgent routing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCodeHandoffPlan {
    /// OpenCode configuration plan.
    pub opencode: ConfigurePlan,
    /// Oh My OpenAgent config path.
    pub openagent_path: PathBuf,
    /// Structural changes planned for Oh My OpenAgent.
    pub openagent_diff: Vec<String>,
}

/// Result of applying a synchronized two-file routing transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCodeHandoffApply {
    /// Updated OpenCode config path.
    pub opencode_path: PathBuf,
    /// OpenCode backup path.
    pub opencode_backup: PathBuf,
    /// Updated Oh My OpenAgent config path.
    pub openagent_path: PathBuf,
    /// Oh My OpenAgent backup path.
    pub openagent_backup: PathBuf,
    /// OpenCode structural changes.
    pub opencode_diff: Vec<String>,
    /// Oh My OpenAgent structural changes.
    pub openagent_diff: Vec<String>,
}

/// Return the default Oh My OpenAgent config path from XDG/HOME.
pub fn default_openagent_config_path() -> PathBuf {
    default_openagent_config_path_from_env(
        env::var_os("XDG_CONFIG_HOME"),
        env::var_os("HOME"),
        env::var_os("USERPROFILE"),
    )
}

/// Build a dry-run plan for the single-server OpenCode handoff.
pub fn plan_opencode_handoff(
    state: &AppState,
    opencode_path: impl AsRef<Path>,
    openagent_path: impl AsRef<Path>,
    force: bool,
) -> Result<OpenCodeHandoffPlan, RuntimeCliError> {
    let opencode =
        super::plan_opencode_configure(state, opencode_path, RoutingOptions::default(), force)?;
    let openagent_path = openagent_path.as_ref();
    let current = read_openagent(openagent_path)?;
    let next = patch_openagent(current.clone(), &managed_model_route(state)?)?;
    Ok(OpenCodeHandoffPlan {
        opencode,
        openagent_path: openagent_path.to_path_buf(),
        openagent_diff: diff_values("", &current, &next),
    })
}

/// Apply OpenCode and Oh My OpenAgent routing as one compensating transaction.
pub fn apply_opencode_handoff(
    state: &AppState,
    opencode_path: impl AsRef<Path>,
    openagent_path: impl AsRef<Path>,
    force: bool,
) -> Result<OpenCodeHandoffApply, RuntimeCliError> {
    let opencode_path = opencode_path.as_ref();
    let openagent_path = openagent_path.as_ref();
    let plan = plan_opencode_handoff(state, opencode_path, openagent_path, force)?;
    if plan.opencode.has_provider_conflicts && !force {
        return Err(RuntimeCliError::Conflict);
    }

    let current_opencode = read_json_or_empty(opencode_path)?;
    validate_config_shape(&current_opencode, force)?;
    let next_opencode = merge_opencode_config(
        current_opencode,
        desired_opencode_patch(state, RoutingOptions::default())?,
    )?;
    let current_openagent = read_openagent(openagent_path)?;
    let next_openagent = patch_openagent(current_openagent, &managed_model_route(state)?)?;

    ensure_parent(opencode_path)?;
    ensure_parent(openagent_path)?;
    let opencode_backup = backup_file(opencode_path)?;
    let openagent_backup = backup_file(openagent_path)?;
    let opencode_temp = stage_json(opencode_path, &next_opencode)?;
    let openagent_temp = stage_json(openagent_path, &next_openagent)?;

    fs::rename(&opencode_temp, opencode_path).map_err(|error| RuntimeCliError::Rename {
        source: opencode_temp,
        dest: opencode_path.to_path_buf(),
        error,
    })?;
    if let Err(error) = fs::rename(&openagent_temp, openagent_path) {
        let failure = RuntimeCliError::Rename {
            source: openagent_temp,
            dest: openagent_path.to_path_buf(),
            error,
        };
        if let Err(rollback) = fs::copy(&opencode_backup, opencode_path) {
            return Err(RuntimeCliError::TransactionRollback {
                failure: failure.to_string(),
                rollback: rollback.to_string(),
            });
        }
        return Err(failure);
    }

    Ok(OpenCodeHandoffApply {
        opencode_path: opencode_path.to_path_buf(),
        opencode_backup,
        openagent_path: openagent_path.to_path_buf(),
        openagent_backup,
        opencode_diff: plan.opencode.diff,
        openagent_diff: plan.openagent_diff,
    })
}

fn managed_model_route(state: &AppState) -> Result<String, RuntimeCliError> {
    let profile = opencode_primary_profile(state)?;
    Ok(format!(
        "llamacpp/{}",
        opencode_model_name(&profile, "opencode")?
    ))
}

fn read_openagent(path: &Path) -> Result<Value, RuntimeCliError> {
    if !path.is_file() {
        return Err(RuntimeCliError::MissingOpenAgentConfig {
            path: path.to_path_buf(),
        });
    }
    let value = read_json_or_empty(path)?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(RuntimeCliError::UnexpectedJsonShape {
            path: "Oh My OpenAgent root".to_string(),
            expected: "object",
        })
    }
}

fn patch_openagent(mut value: Value, route: &str) -> Result<Value, RuntimeCliError> {
    let root = value
        .as_object_mut()
        .ok_or_else(|| RuntimeCliError::UnexpectedJsonShape {
            path: "Oh My OpenAgent root".to_string(),
            expected: "object",
        })?;
    for section_name in ["agents", "categories"] {
        let Some(section) = root.get_mut(section_name) else {
            continue;
        };
        let section =
            section
                .as_object_mut()
                .ok_or_else(|| RuntimeCliError::UnexpectedJsonShape {
                    path: section_name.to_string(),
                    expected: "object",
                })?;
        for entry in section.values_mut() {
            let Some(entry) = entry.as_object_mut() else {
                continue;
            };
            entry.insert("model".to_string(), Value::String(route.to_string()));
            entry.remove("fallback_models");
        }
    }

    let manager = object_entry(root, "llamaModelManager")?;
    let sync = object_entry(manager, "openagentSync")?;
    for key in ["model", "fastModel", "fullModel", "fullFallbackModel"] {
        sync.insert(key.to_string(), Value::String(route.to_string()));
    }
    Ok(value)
}

fn object_entry<'a>(
    object: &'a mut Map<String, Value>,
    key: &str,
) -> Result<&'a mut Map<String, Value>, RuntimeCliError> {
    object
        .entry(key.to_string())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| RuntimeCliError::UnexpectedJsonShape {
            path: key.to_string(),
            expected: "object",
        })
}

fn ensure_parent(path: &Path) -> Result<(), RuntimeCliError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| RuntimeCliError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

fn backup_file(path: &Path) -> Result<PathBuf, RuntimeCliError> {
    let backup = create_backup(path)?;
    fs::copy(path, &backup).map_err(|error| RuntimeCliError::Backup {
        source: path.to_path_buf(),
        dest: backup.clone(),
        error,
    })?;
    Ok(backup)
}

fn stage_json(path: &Path, value: &Value) -> Result<PathBuf, RuntimeCliError> {
    let temp = temp_path(path);
    let content = serde_json::to_string_pretty(value).map_err(RuntimeCliError::SerializeJson)?;
    fs::write(&temp, format!("{content}\n")).map_err(|source| RuntimeCliError::Write {
        path: temp.clone(),
        source,
    })?;
    let staged = fs::read_to_string(&temp).map_err(|source| RuntimeCliError::Read {
        path: temp.clone(),
        source,
    })?;
    serde_json::from_str::<Value>(&staged).map_err(|source| RuntimeCliError::ParseJson {
        path: temp.clone(),
        source,
    })?;
    Ok(temp)
}

fn default_openagent_config_path_from_env(
    xdg_config_home: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
    userprofile: Option<std::ffi::OsString>,
) -> PathBuf {
    if let Some(path) = xdg_config_home.filter(|path| !path.is_empty()) {
        return PathBuf::from(path)
            .join("opencode")
            .join("oh-my-openagent.json");
    }
    if let Some(path) = home.filter(|path| !path.is_empty()) {
        return PathBuf::from(path).join(".config/opencode/oh-my-openagent.json");
    }
    if let Some(path) = userprofile.filter(|path| !path.is_empty()) {
        return PathBuf::from(path).join(".config/opencode/oh-my-openagent.json");
    }
    PathBuf::from(".config/opencode/oh-my-openagent.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn handoff_updates_routes_and_preserves_user_owned_fields() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let opencode = tempdir.path().join("opencode.json");
        let openagent = tempdir.path().join("oh-my-openagent.json");
        fs::write(
            &opencode,
            r#"{"snapshot":false,"plugin":["keep"],"compaction":{"reserved":65536},"provider":{"other":{"name":"keep"}}}"#,
        )
        .expect("opencode fixture");
        fs::write(
            &openagent,
            r#"{"agents":{"worker":{"model":"old/model","prompt":"keep","fallback_models":["old/model"]}},"categories":{"review":{"model":"old/model"}},"unrelated":{"keep":true},"llamaModelManager":{"openagentSync":{"custom":"keep"}}}"#,
        )
        .expect("openagent fixture");
        let mut state = AppState::default();
        state.runtime.opencode.model = PathBuf::from("/models/current.gguf");

        let applied =
            apply_opencode_handoff(&state, &opencode, &openagent, false).expect("apply handoff");
        let opencode_json: Value =
            serde_json::from_str(&fs::read_to_string(&opencode).expect("opencode"))
                .expect("opencode json");
        let openagent_json: Value =
            serde_json::from_str(&fs::read_to_string(&openagent).expect("openagent"))
                .expect("openagent json");

        assert_eq!(opencode_json["snapshot"], false);
        assert_eq!(opencode_json["plugin"][0], "keep");
        assert_eq!(opencode_json["compaction"]["reserved"], 65536);
        assert_eq!(opencode_json["provider"]["other"]["name"], "keep");
        assert_eq!(opencode_json["model"], "llamacpp/current.gguf");
        assert_eq!(
            openagent_json["agents"]["worker"]["model"],
            "llamacpp/current.gguf"
        );
        assert!(openagent_json["agents"]["worker"]
            .get("fallback_models")
            .is_none());
        assert_eq!(openagent_json["agents"]["worker"]["prompt"], "keep");
        assert_eq!(openagent_json["unrelated"]["keep"], true);
        assert_eq!(
            openagent_json["llamaModelManager"]["openagentSync"]["fastModel"],
            "llamacpp/current.gguf"
        );
        assert_eq!(
            openagent_json["llamaModelManager"]["openagentSync"]["custom"],
            "keep"
        );
        assert!(applied.opencode_backup.is_file());
        assert!(applied.openagent_backup.is_file());
    }

    #[test]
    fn missing_openagent_config_is_rejected_before_opencode_changes() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let opencode = tempdir.path().join("opencode.json");
        let openagent = tempdir.path().join("missing.json");
        fs::write(&opencode, r#"{"snapshot":false}"#).expect("opencode fixture");
        let before = fs::read_to_string(&opencode).expect("before");
        let mut state = AppState::default();
        state.runtime.opencode.model = PathBuf::from("/models/current.gguf");

        assert!(matches!(
            apply_opencode_handoff(&state, &opencode, &openagent, false),
            Err(RuntimeCliError::MissingOpenAgentConfig { .. })
        ));
        assert_eq!(fs::read_to_string(opencode).expect("after"), before);
    }
}
