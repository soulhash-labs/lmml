//! Signed prompt-configuration approval and deployment-integrity contracts.
//!
//! This module is Slice 0 of template approval. It defines deterministic
//! identities, a mandatory signed receipt, and a separately persisted
//! monotonic revocation anchor. It deliberately does not admit or launch a
//! runtime; launch and request enforcement are later slices.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::{sha256_data, sha256_file, Hash256, SubstrateError};

const APPROVAL_SCHEMA_VERSION: u32 = 1;
const APPROVAL_DOMAIN: &str = "LMML/APPROVAL-RECEIPT/v1";
const PROMPT_DOMAIN: &str = "LMML/PROMPT-CONFIG/v1";
const RUNTIME_DOMAIN: &str = "LMML/RUNTIME-CONFIG/v1";
const DEPLOYMENT_DOMAIN: &str = "LMML/DEPLOYMENT-MANIFEST/v1";
const REVOCATION_DOMAIN: &str = "LMML/REVOCATION-STATE/v1";
const TRUST_ROTATION_DOMAIN: &str = "LMML/TRUST-ROOT-ROTATION/v1";

/// Template bytes and their source/deployed relationship.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateIdentity {
    /// Contract schema version.
    pub schema_version: u32,
    /// Hash of the source tokenizer/template container.
    pub source_template_container_hash: Hash256,
    /// Exact source template bytes, when present.
    pub source_template_hash: Option<Hash256>,
    /// Selected named variant.
    pub selected_variant: String,
    /// Hash of the built-in template registry implementation, when used.
    pub builtin_registry_hash: Option<Hash256>,
    /// Template bytes embedded in the deployed GGUF, when present.
    pub gguf_embedded_template_hash: Option<Hash256>,
    /// External `--chat-template-file` bytes, when used.
    pub external_template_hash: Option<Hash256>,
    /// Exact bytes selected by the effective launch configuration.
    pub effective_template_hash: Hash256,
    /// Whether conversion preserved or transformed the source bytes.
    pub conversion_status: TemplateConversionStatus,
}

/// Hashes extracted from a source tokenizer/template package before review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateSourceInspection {
    /// Contract schema version.
    pub schema_version: u32,
    /// Hash of the tokenizer configuration container.
    pub source_template_container_hash: Hash256,
    /// Exact hashes for discovered named template variants.
    pub template_hashes: BTreeMap<String, Hash256>,
    /// Hash of an external `chat_template.jinja`, when present.
    pub external_template_hash: Option<Hash256>,
    /// Hash of the special-token map, when present.
    pub special_tokens_hash: Option<Hash256>,
}

/// Inspect source template assets without executing template code.
pub fn inspect_template_source(
    root: impl AsRef<Path>,
) -> Result<TemplateSourceInspection, SubstrateError> {
    let root = root.as_ref();
    let tokenizer_config = root.join("tokenizer_config.json");
    let config_bytes = fs::read(&tokenizer_config).map_err(|source| SubstrateError::Io {
        path: tokenizer_config.clone(),
        source,
    })?;
    let config: Value =
        serde_json::from_slice(&config_bytes).map_err(|source| SubstrateError::InvalidJson {
            path: tokenizer_config.clone(),
            source,
        })?;
    let container_hash = sha256_data(&config_bytes);
    let mut template_hashes = BTreeMap::new();
    if let Some(template) = config.get("chat_template") {
        match template {
            Value::String(value) => {
                template_hashes.insert("default".to_string(), sha256_data(value.as_bytes()));
            }
            Value::Object(variants) => {
                for (name, value) in variants {
                    let Value::String(value) = value else {
                        return Err(SubstrateError::InvalidApprovalContract(format!(
                            "chat_template variant {name} is not a string"
                        )));
                    };
                    template_hashes.insert(name.clone(), sha256_data(value.as_bytes()));
                }
            }
            _ => {
                return Err(SubstrateError::InvalidApprovalContract(
                    "chat_template must be a string or object".into(),
                ));
            }
        }
    }
    let external_path = root.join("chat_template.jinja");
    let external_template_hash = match fs::read(&external_path) {
        Ok(bytes) => Some(sha256_data(&bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(SubstrateError::Io {
                path: external_path,
                source,
            })
        }
    };
    let special_tokens_path = root.join("special_tokens_map.json");
    let special_tokens_hash = match fs::read(&special_tokens_path) {
        Ok(bytes) => Some(sha256_data(&bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(SubstrateError::Io {
                path: special_tokens_path,
                source,
            })
        }
    };
    Ok(TemplateSourceInspection {
        schema_version: APPROVAL_SCHEMA_VERSION,
        source_template_container_hash: container_hash,
        template_hashes,
        external_template_hash,
        special_tokens_hash,
    })
}

/// Conversion relationship between source and deployed template bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateConversionStatus {
    /// Source and deployed bytes are identical.
    Preserved,
    /// Conversion produced a deliberate normalized representation.
    Normalized,
    /// Conversion changed bytes and requires explicit review.
    Changed,
}

/// Versioned request fields permitted by an approved prompt configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestPromptSchema {
    /// Contract schema version.
    pub schema_version: u32,
    /// Stable schema identifier.
    pub schema_id: String,
    /// Allowed message roles.
    pub allowed_roles: Vec<String>,
    /// Allowed content kinds, such as `text` or `image`.
    pub allowed_content_types: Vec<String>,
    /// Maximum tool definitions accepted in one request.
    pub max_tools: u32,
    /// Maximum nesting depth for request data.
    pub max_nesting: u32,
    /// Maximum request payload bytes.
    pub max_request_bytes: u64,
    /// Maximum requested context tokens.
    pub max_context_tokens: u64,
    /// Whether reasoning controls may be supplied.
    pub reasoning_controls: bool,
    /// Whether assistant prefill/continuation is permitted.
    pub assistant_prefill: bool,
    /// Approved template keyword arguments and their allowed values/ranges.
    pub permitted_template_kwargs: BTreeMap<String, Value>,
}

/// The prompt-rendering configuration reviewed by an approval receipt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptConfiguration {
    /// Contract schema version.
    pub schema_version: u32,
    /// Model lineage covered by the review.
    pub model_lineage_id: String,
    /// Exact deployed artifact hash.
    pub artifact_hash: Hash256,
    /// Source tokenizer manifest identity.
    pub source_tokenizer_manifest_hash: Hash256,
    /// Deployed tokenizer metadata identity.
    pub deployed_tokenizer_metadata_hash: Hash256,
    /// Special-token map identity.
    pub special_token_map_hash: Hash256,
    /// Effective tokenizer identity used by the renderer.
    pub effective_tokenizer_hash: Hash256,
    /// Effective template identity.
    pub template: TemplateIdentity,
    /// Exact renderer implementation identifier.
    pub renderer_id: String,
    /// Renderer version and build identity.
    pub renderer_version: String,
    /// Renderer build hash.
    pub renderer_build_hash: Hash256,
    /// Renderer settings included in the identity.
    pub renderer_settings: BTreeMap<String, Value>,
    /// Request schema bound to the approval.
    pub request_schema: RequestPromptSchema,
    /// Multimodal processor identity, when required.
    pub multimodal_processor_hash: Option<Hash256>,
    /// Deployed mmproj identity, when required.
    pub mmproj_hash: Option<Hash256>,
}

/// A library identity in the declared dynamic-loader closure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryIdentity {
    /// Canonical resolved path.
    pub path: PathBuf,
    /// Exact library bytes.
    pub sha256: Hash256,
    /// Whether the library is required for startup.
    pub required: bool,
}

/// A runtime environment entry with its value bound without storing secrets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentIdentity {
    /// Environment variable name.
    pub name: String,
    /// Hash of the passed value, if non-secret.
    pub value_hash: Option<Hash256>,
    /// Whether only presence, not value, is permitted to be recorded.
    pub secret_presence_only: bool,
}

/// Exact operational runtime configuration bound to a prompt configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeConfiguration {
    /// Contract schema version.
    pub schema_version: u32,
    /// Prompt configuration identity.
    pub prompt_configuration_hash: Hash256,
    /// Runtime/backend identifier.
    pub backend: String,
    /// Backend version or source revision.
    pub backend_version: String,
    /// Executable identity.
    pub executable_hash: Hash256,
    /// Canonical argv, including resolved defaults.
    pub resolved_argv: Vec<String>,
    /// Explicitly allowed environment entries only.
    pub allowed_environment: Vec<EnvironmentIdentity>,
    /// Declared dynamic-loader closure.
    pub loaded_libraries: Vec<LibraryIdentity>,
    /// Context and scheduling settings.
    pub settings: BTreeMap<String, Value>,
}

/// Immutable files and identities used to construct one deployment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentManifest {
    /// Contract schema version.
    pub schema_version: u32,
    /// Stable deployment identifier.
    pub deployment_id: String,
    /// Model lineage and artifact identities.
    pub model_lineage_id: String,
    /// Deployed artifact hash.
    pub artifact_hash: Hash256,
    /// Prompt configuration identity.
    pub prompt_configuration_hash: Hash256,
    /// Runtime configuration identity.
    pub runtime_configuration_hash: Hash256,
    /// Content-addressed snapshot root.
    pub snapshot_hash: Hash256,
    /// Referenced immutable files and their hashes.
    pub files: Vec<LibraryIdentity>,
    /// RFC3339 creation time.
    pub created_at: String,
}

/// Immutable references and launch parameters for one approved deployment.
///
/// The bundle contains paths to already-published, signed inputs. It never
/// stores the signing private key or a backend bearer secret; the latter is
/// resolved from the explicitly named environment variable at launch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalBundle {
    /// Contract schema version.
    pub schema_version: u32,
    /// Stable bundle identifier.
    pub bundle_id: String,
    /// Prompt configuration JSON.
    pub prompt: PathBuf,
    /// Runtime configuration JSON.
    pub runtime: PathBuf,
    /// Deployment manifest JSON.
    pub deployment: PathBuf,
    /// Content-addressed deployment snapshot root.
    pub snapshot_root: PathBuf,
    /// Signed approval receipt JSON.
    pub receipt: PathBuf,
    /// Operator-pinned trust root JSON.
    pub trust_root: PathBuf,
    /// Optional signed revocation state JSON.
    pub revocation_state: Option<PathBuf>,
    /// Environment variable containing the private backend bearer token.
    pub backend_api_key_env: String,
    /// Public gateway port.
    pub public_port: u16,
    /// Private backend port.
    pub backend_port: u16,
    /// Context size bound to the launch request.
    pub context: u32,
    /// GPU layer setting bound to the launch request.
    pub gpu_layers: i32,
    /// Worker thread setting bound to the launch request.
    pub threads: usize,
}

/// Validate an approval bundle before any referenced file is opened.
pub fn validate_approval_bundle(bundle: &ApprovalBundle) -> Result<(), SubstrateError> {
    if bundle.schema_version != APPROVAL_SCHEMA_VERSION {
        return Err(SubstrateError::InvalidApprovalContract(
            "unsupported approval bundle schema".into(),
        ));
    }
    if bundle.bundle_id.trim().is_empty() {
        return Err(SubstrateError::InvalidApprovalContract(
            "approval bundle id is empty".into(),
        ));
    }
    if bundle.backend_api_key_env.trim().is_empty()
        || !bundle.backend_api_key_env.chars().all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        })
    {
        return Err(SubstrateError::InvalidApprovalContract(
            "backend API key environment name is invalid".into(),
        ));
    }
    if bundle.public_port == 0 || bundle.backend_port == 0 || bundle.context == 0 {
        return Err(SubstrateError::InvalidApprovalContract(
            "approval bundle ports and context must be non-zero".into(),
        ));
    }
    for path in [
        &bundle.prompt,
        &bundle.runtime,
        &bundle.deployment,
        &bundle.snapshot_root,
        &bundle.receipt,
        &bundle.trust_root,
    ] {
        if path.as_os_str().is_empty() {
            return Err(SubstrateError::InvalidApprovalContract(
                "approval bundle contains an empty path".into(),
            ));
        }
    }
    Ok(())
}

/// Load and validate a managed approval bundle.
pub fn load_approval_bundle(path: impl AsRef<Path>) -> Result<ApprovalBundle, SubstrateError> {
    let path = path.as_ref();
    let bytes = fs::read(path).map_err(|source| SubstrateError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let bundle: ApprovalBundle =
        serde_json::from_slice(&bytes).map_err(|source| SubstrateError::InvalidJson {
            path: path.to_path_buf(),
            source,
        })?;
    validate_approval_bundle(&bundle)?;
    let mut referenced_paths = vec![
        &bundle.prompt,
        &bundle.runtime,
        &bundle.deployment,
        &bundle.snapshot_root,
        &bundle.receipt,
        &bundle.trust_root,
    ];
    if let Some(path) = &bundle.revocation_state {
        referenced_paths.push(path);
    }
    for referenced in referenced_paths {
        let metadata = fs::symlink_metadata(referenced).map_err(|source| SubstrateError::Io {
            path: referenced.clone(),
            source,
        })?;
        if metadata.file_type().is_symlink() {
            return Err(SubstrateError::ApprovalRollback(format!(
                "approval bundle references a symlink: {}",
                referenced.display()
            )));
        }
    }
    Ok(bundle)
}

/// Persist a bundle once, refusing replacement of an existing bundle ID.
pub fn store_approval_bundle(
    path: impl AsRef<Path>,
    bundle: &ApprovalBundle,
) -> Result<PathBuf, SubstrateError> {
    validate_approval_bundle(bundle)?;
    let path = path.as_ref();
    let payload = canonical_json(bundle)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| SubstrateError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| {
            if source.kind() == std::io::ErrorKind::AlreadyExists {
                SubstrateError::ApprovalConflict(path.to_path_buf())
            } else {
                SubstrateError::Io {
                    path: path.to_path_buf(),
                    source,
                }
            }
        })?;
    file.write_all(&payload)
        .map_err(|source| SubstrateError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    file.sync_all().map_err(|source| SubstrateError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    make_read_only(path)?;
    Ok(path.to_path_buf())
}

/// One source file selected for an immutable deployment snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotInput {
    /// Source path controlled by the operator/importer.
    pub source: PathBuf,
    /// Relative destination beneath the managed snapshot root.
    pub relative_path: PathBuf,
    /// Whether the deployed process must have this file.
    pub required: bool,
}

/// File identity emitted after snapshot publication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotFile {
    /// Relative path beneath the content-addressed snapshot.
    pub relative_path: PathBuf,
    /// Exact copied bytes.
    pub sha256: Hash256,
    /// File size in bytes.
    pub size_bytes: u64,
    /// Whether the deployment requires this file.
    pub required: bool,
}

/// Publish immutable, content-addressed copies of deployment inputs.
pub fn publish_deployment_snapshot(
    root: impl AsRef<Path>,
    inputs: &[SnapshotInput],
) -> Result<(PathBuf, Hash256, Vec<SnapshotFile>), SubstrateError> {
    if inputs.is_empty() {
        return Err(SubstrateError::InvalidApprovalContract(
            "deployment snapshot has no inputs".into(),
        ));
    }
    let mut files = Vec::with_capacity(inputs.len());
    for input in inputs {
        validate_relative_path(&input.relative_path)?;
        let metadata = fs::metadata(&input.source).map_err(|source| SubstrateError::Io {
            path: input.source.clone(),
            source,
        })?;
        if !metadata.is_file() {
            return Err(SubstrateError::InvalidApprovalContract(format!(
                "snapshot source is not a regular file: {}",
                input.source.display()
            )));
        }
        files.push((input, sha256_file(&input.source)?, metadata.len()));
    }
    files.sort_by(|left, right| left.0.relative_path.cmp(&right.0.relative_path));
    if files
        .windows(2)
        .any(|pair| pair[0].0.relative_path == pair[1].0.relative_path)
    {
        return Err(SubstrateError::InvalidApprovalContract(
            "deployment snapshot contains duplicate paths".into(),
        ));
    }
    let identities: Vec<SnapshotFile> = files
        .iter()
        .map(|(input, hash, size_bytes)| SnapshotFile {
            relative_path: input.relative_path.clone(),
            sha256: hash.clone(),
            size_bytes: *size_bytes,
            required: input.required,
        })
        .collect();
    let snapshot_hash = domain_hash("LMML/DEPLOYMENT-SNAPSHOT/v1", &identities)?;
    let root = root.as_ref();
    fs::create_dir_all(root).map_err(|source| SubstrateError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    let snapshot_root = root.join(snapshot_hash.as_str());
    if snapshot_root.exists() {
        verify_snapshot(&snapshot_root, &identities)?;
        return Ok((snapshot_root, snapshot_hash, identities));
    }
    let temporary = root.join(format!(".{}.tmp-{}", snapshot_hash, std::process::id()));
    if temporary.exists() {
        fs::remove_dir_all(&temporary).map_err(|source| SubstrateError::Io {
            path: temporary.clone(),
            source,
        })?;
    }
    fs::create_dir_all(&temporary).map_err(|source| SubstrateError::Io {
        path: temporary.clone(),
        source,
    })?;
    for (input, expected_hash, expected_size) in files {
        let destination = temporary.join(&input.relative_path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|source| SubstrateError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        fs::copy(&input.source, &destination).map_err(|source| SubstrateError::Io {
            path: destination.clone(),
            source,
        })?;
        let copied_metadata = fs::metadata(&destination).map_err(|source| SubstrateError::Io {
            path: destination.clone(),
            source,
        })?;
        if copied_metadata.len() != expected_size || sha256_file(&destination)? != expected_hash {
            let _ = fs::remove_dir_all(&temporary);
            return Err(SubstrateError::ApprovalRollback(format!(
                "snapshot source changed while copying {}",
                input.source.display()
            )));
        }
        make_read_only(&destination)?;
    }
    make_read_only(&temporary)?;
    match fs::rename(&temporary, &snapshot_root) {
        Ok(()) => Ok((snapshot_root, snapshot_hash, identities)),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = fs::remove_dir_all(&temporary);
            verify_snapshot(&snapshot_root, &identities)?;
            Ok((snapshot_root, snapshot_hash, identities))
        }
        Err(source) => {
            let _ = fs::remove_dir_all(&temporary);
            Err(SubstrateError::Io {
                path: snapshot_root,
                source,
            })
        }
    }
}

/// Verify every file in a published deployment snapshot and its content hash.
pub fn verify_deployment_snapshot(
    snapshot_root: impl AsRef<Path>,
    manifest: &DeploymentManifest,
) -> Result<(), SubstrateError> {
    validate_deployment_manifest(manifest)?;
    let root = snapshot_root.as_ref();
    let mut files = Vec::with_capacity(manifest.files.len());
    for identity in &manifest.files {
        validate_relative_path(&identity.path)?;
        let path = root.join(&identity.path);
        let metadata = fs::symlink_metadata(&path).map_err(|source| SubstrateError::Io {
            path: path.clone(),
            source,
        })?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || sha256_file(&path)? != identity.sha256
        {
            return Err(SubstrateError::ApprovalRollback(format!(
                "deployment snapshot file failed identity verification: {}",
                identity.path.display()
            )));
        }
        files.push(SnapshotFile {
            relative_path: identity.path.clone(),
            sha256: identity.sha256.clone(),
            size_bytes: metadata.len(),
            required: identity.required,
        });
    }
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    if files
        .windows(2)
        .any(|pair| pair[0].relative_path == pair[1].relative_path)
    {
        return Err(SubstrateError::InvalidApprovalContract(
            "deployment manifest contains duplicate paths".into(),
        ));
    }
    let snapshot_hash = domain_hash("LMML/DEPLOYMENT-SNAPSHOT/v1", &files)?;
    if snapshot_hash != manifest.snapshot_hash {
        return Err(SubstrateError::ApprovalRollback(
            "deployment snapshot hash does not match manifest".into(),
        ));
    }
    Ok(())
}

/// Persisted decision recorded in a signed approval receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    /// Human/policy approval.
    Approved,
    /// Human/policy rejection.
    Rejected,
}

/// Wire values for persisted and computed approval lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionState {
    /// No inspection or approval record exists.
    Unknown,
    /// Source/deployed identities were extracted but not reviewed.
    Extracted,
    /// A reviewer must examine the exact configuration.
    NeedsReview,
    /// A signed persisted decision exists and current identities match.
    Approved,
    /// A signed persisted decision rejected the configuration.
    Rejected,
    /// Operational checks prevent launch or request admission.
    Blocked,
    /// A previously approved receipt was explicitly withdrawn.
    Revoked,
    /// A bound identity changed after approval.
    Invalidated,
    /// Runtime verification completed for the approved deployment.
    RuntimeVerified,
}

/// Explain a computed admission state without mutating persisted decisions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmissionStatus {
    /// Current computed state.
    pub state: AdmissionState,
    /// Stable machine-readable reason.
    pub reason: String,
}

/// Validate one JSON chat request against the approved prompt schema.
pub fn validate_prompt_request(
    schema: &RequestPromptSchema,
    request: &Value,
) -> Result<(), SubstrateError> {
    if schema.schema_version != APPROVAL_SCHEMA_VERSION {
        return Err(SubstrateError::RequestPolicyViolation(
            "unsupported request schema".into(),
        ));
    }
    let bytes = serde_json::to_vec(request).map_err(SubstrateError::Serialize)?;
    if bytes.len() as u64 > schema.max_request_bytes {
        return Err(SubstrateError::RequestPolicyViolation(
            "request exceeds the approved byte limit".into(),
        ));
    }
    if json_depth(request) > schema.max_nesting as usize {
        return Err(SubstrateError::RequestPolicyViolation(
            "request exceeds the approved nesting limit".into(),
        ));
    }
    let object = request.as_object().ok_or_else(|| {
        SubstrateError::RequestPolicyViolation("request must be a JSON object".into())
    })?;
    for key in object.keys() {
        if matches!(
            key.as_str(),
            "chat_template"
                | "chat_template_file"
                | "jinja"
                | "template"
                | "template_kwargs"
                | "chat_template_kwargs"
                | "grammar"
                | "reasoning_parser"
        ) {
            return Err(SubstrateError::RequestPolicyViolation(format!(
                "request-level template or parser override is forbidden: {key}"
            )));
        }
    }
    if let Some(messages) = object.get("messages") {
        validate_messages(schema, messages)?;
    } else if object.get("prompt").and_then(Value::as_str).is_none() {
        return Err(SubstrateError::RequestPolicyViolation(
            "request must contain messages or prompt".into(),
        ));
    }
    if let Some(tools) = object.get("tools") {
        let tools = tools.as_array().ok_or_else(|| {
            SubstrateError::RequestPolicyViolation("tools must be an array".into())
        })?;
        if tools.len() > schema.max_tools as usize {
            return Err(SubstrateError::RequestPolicyViolation(
                "request exceeds the approved tool count".into(),
            ));
        }
    }
    if let Some(max_tokens) = object.get("max_tokens").and_then(Value::as_u64) {
        if max_tokens > schema.max_context_tokens {
            return Err(SubstrateError::RequestPolicyViolation(
                "max_tokens exceeds the approved context budget".into(),
            ));
        }
    }
    for key in ["reasoning", "enable_thinking", "thinking"] {
        if object.contains_key(key) && !schema.reasoning_controls {
            return Err(SubstrateError::RequestPolicyViolation(format!(
                "reasoning control is not permitted: {key}"
            )));
        }
    }
    Ok(())
}

fn validate_messages(schema: &RequestPromptSchema, messages: &Value) -> Result<(), SubstrateError> {
    let messages = messages.as_array().ok_or_else(|| {
        SubstrateError::RequestPolicyViolation("messages must be an array".into())
    })?;
    for message in messages {
        let object = message.as_object().ok_or_else(|| {
            SubstrateError::RequestPolicyViolation("each message must be an object".into())
        })?;
        let role = object.get("role").and_then(Value::as_str).ok_or_else(|| {
            SubstrateError::RequestPolicyViolation("message role is required".into())
        })?;
        if !schema.allowed_roles.iter().any(|allowed| allowed == role) {
            return Err(SubstrateError::RequestPolicyViolation(format!(
                "message role is not permitted: {role}"
            )));
        }
        if let Some(content) = object.get("content") {
            let content_type = match content {
                Value::String(_) => "text",
                Value::Array(parts) => {
                    for part in parts {
                        let part_type =
                            part.get("type").and_then(Value::as_str).ok_or_else(|| {
                                SubstrateError::RequestPolicyViolation(
                                    "content part type is required".into(),
                                )
                            })?;
                        if !schema
                            .allowed_content_types
                            .iter()
                            .any(|allowed| allowed == part_type)
                        {
                            return Err(SubstrateError::RequestPolicyViolation(format!(
                                "content type is not permitted: {part_type}"
                            )));
                        }
                    }
                    continue;
                }
                _ => {
                    return Err(SubstrateError::RequestPolicyViolation(
                        "message content must be a string or array".into(),
                    ));
                }
            };
            if !schema
                .allowed_content_types
                .iter()
                .any(|allowed| allowed == content_type)
            {
                return Err(SubstrateError::RequestPolicyViolation(format!(
                    "content type is not permitted: {content_type}"
                )));
            }
        }
        if object.contains_key("prefix") && !schema.assistant_prefill {
            return Err(SubstrateError::RequestPolicyViolation(
                "assistant prefill is not permitted".into(),
            ));
        }
    }
    Ok(())
}

fn json_depth(value: &Value) -> usize {
    match value {
        Value::Array(values) => values.iter().map(json_depth).max().unwrap_or(0) + 1,
        Value::Object(values) => values.values().map(json_depth).max().unwrap_or(0) + 1,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => 1,
    }
}

/// Signed approval receipt. Unsigned receipts are never accepted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalReceipt {
    /// Receipt schema version.
    pub schema_version: u32,
    /// Stable receipt identifier.
    pub receipt_id: String,
    /// Decision represented by this receipt.
    pub decision: ApprovalDecision,
    /// Model lineage covered by the receipt.
    pub model_lineage_id: String,
    /// Bound artifact identity.
    pub artifact_hash: Hash256,
    /// Bound prompt configuration identity.
    pub prompt_configuration_hash: Hash256,
    /// Bound runtime configuration identity.
    pub runtime_configuration_hash: Hash256,
    /// Bound deployment identity.
    pub deployment_manifest_hash: Hash256,
    /// Policy version used for review.
    pub policy_version: String,
    /// Validation-suite identity and result set.
    pub validation_suite_id: String,
    /// Authorized reviewer identity.
    pub reviewer_id: String,
    /// Pinned signing key identity.
    pub signing_key_id: String,
    /// Monotonic receipt sequence.
    pub sequence: u64,
    /// RFC3339 issuance time.
    pub issued_at: String,
    /// Ed25519 signature over the domain-separated canonical payload.
    pub signature_hex: String,
}

/// A signed revocation event for a prior receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevocationEvent {
    /// Contract schema version.
    pub schema_version: u32,
    /// Monotonic event sequence.
    pub sequence: u64,
    /// Receipt being revoked.
    pub receipt_id: String,
    /// Reason for revocation.
    pub reason: String,
    /// RFC3339 revocation time.
    pub revoked_at: String,
}

/// Append-only revocation state, anchored separately from the event log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevocationState {
    /// Contract schema version.
    pub schema_version: u32,
    /// Monotonic store epoch.
    pub epoch: u64,
    /// Events in strictly increasing sequence order.
    pub events: Vec<RevocationEvent>,
    /// Hash of the canonical state payload.
    pub state_hash: Hash256,
    /// Ed25519 signature over the domain-separated state payload.
    pub signature_hex: String,
}

/// Operator-pinned public key used to verify approval receipts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalTrustRoot {
    /// Contract schema version.
    pub schema_version: u32,
    /// Stable key identifier.
    pub key_id: String,
    /// Ed25519 public key encoded as lowercase hexadecimal.
    pub public_key_hex: String,
    /// Monotonic key epoch used for explicit rotation.
    pub key_epoch: u64,
}

/// Signed migration from one operator trust root to a newer root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustRootRotation {
    /// Contract schema version.
    pub schema_version: u32,
    /// Previous pinned key identifier.
    pub old_key_id: String,
    /// Previous key epoch.
    pub old_key_epoch: u64,
    /// Replacement trust root.
    pub new_root: ApprovalTrustRoot,
    /// Monotonic migration sequence.
    pub sequence: u64,
    /// RFC3339 migration time.
    pub issued_at: String,
    /// Signature made by the old private key.
    pub signature_hex: String,
}

/// Sign a trust-root rotation with the currently pinned private key.
pub fn sign_trust_root_rotation(
    rotation: &mut TrustRootRotation,
    old_private_key: &[u8],
) -> Result<(), SubstrateError> {
    validate_trust_root_rotation(rotation)?;
    let pair = Ed25519KeyPair::from_pkcs8(old_private_key)
        .map_err(|_| SubstrateError::InvalidApprovalContract("invalid signing key".into()))?;
    rotation.signature_hex = hex_encode(pair.sign(&trust_rotation_payload(rotation)?).as_ref());
    Ok(())
}

/// Verify that a rotation is authorized by the currently pinned trust root.
pub fn verify_trust_root_rotation(
    rotation: &TrustRootRotation,
    old_root: &ApprovalTrustRoot,
) -> Result<(), SubstrateError> {
    validate_trust_root_rotation(rotation)?;
    validate_approval_trust_root(old_root)?;
    if rotation.old_key_id != old_root.key_id || rotation.old_key_epoch != old_root.key_epoch {
        return Err(SubstrateError::ApprovalSignatureInvalid);
    }
    if rotation.signature_hex.is_empty() {
        return Err(SubstrateError::UnsignedApproval);
    }
    let public_key = hex_decode(&old_root.public_key_hex)?;
    let signature = hex_decode(&rotation.signature_hex)?;
    UnparsedPublicKey::new(&ED25519, public_key)
        .verify(&trust_rotation_payload(rotation)?, &signature)
        .map_err(|_| SubstrateError::ApprovalSignatureInvalid)
}

/// Publish a verified trust-root rotation and replacement root atomically.
pub fn store_trust_root_rotation(
    root_path: impl AsRef<Path>,
    migration_path: impl AsRef<Path>,
    rotation: &TrustRootRotation,
    old_root: &ApprovalTrustRoot,
) -> Result<PathBuf, SubstrateError> {
    verify_trust_root_rotation(rotation, old_root)?;
    if rotation.new_root.key_epoch <= old_root.key_epoch {
        return Err(SubstrateError::ApprovalRollback(
            "trust-root rotation does not advance key epoch".into(),
        ));
    }
    let root_path = root_path.as_ref();
    let migration_path = migration_path.as_ref();
    for path in [root_path, migration_path] {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| SubstrateError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
            protect_directory(parent)?;
        }
    }
    let root_payload = canonical_json(&rotation.new_root)?;
    let migration_payload = canonical_json(rotation)?;
    persist_immutable(root_path, &root_payload, SubstrateError::ApprovalConflict)?;
    if let Err(error) = persist_immutable(
        migration_path,
        &migration_payload,
        SubstrateError::ApprovalConflict,
    ) {
        let _ignored = fs::remove_file(root_path);
        return Err(error);
    }
    protect_file(root_path)?;
    protect_file(migration_path)?;
    Ok(root_path.to_path_buf())
}

/// Validate an operator-pinned Ed25519 trust root.
pub fn validate_approval_trust_root(trust_root: &ApprovalTrustRoot) -> Result<(), SubstrateError> {
    if trust_root.schema_version != APPROVAL_SCHEMA_VERSION {
        return Err(SubstrateError::InvalidApprovalContract(
            "unsupported trust-root schema".into(),
        ));
    }
    require_text(&trust_root.key_id, "key_id")?;
    if trust_root.key_epoch == 0 {
        return Err(SubstrateError::InvalidApprovalContract(
            "trust-root key_epoch must be positive".into(),
        ));
    }
    if hex_decode(&trust_root.public_key_hex)?.len() != 32 {
        return Err(SubstrateError::InvalidApprovalContract(
            "Ed25519 public key must be 32 bytes".into(),
        ));
    }
    Ok(())
}

fn validate_trust_root_rotation(rotation: &TrustRootRotation) -> Result<(), SubstrateError> {
    if rotation.schema_version != APPROVAL_SCHEMA_VERSION {
        return Err(SubstrateError::InvalidApprovalContract(
            "unsupported trust-root rotation schema".into(),
        ));
    }
    require_text(&rotation.old_key_id, "old_key_id")?;
    if rotation.old_key_epoch == 0 || rotation.sequence == 0 {
        return Err(SubstrateError::InvalidApprovalContract(
            "trust-root rotation epoch and sequence must be positive".into(),
        ));
    }
    validate_approval_trust_root(&rotation.new_root)?;
    if time::OffsetDateTime::parse(
        &rotation.issued_at,
        &time::format_description::well_known::Rfc3339,
    )
    .is_err()
    {
        return Err(SubstrateError::InvalidApprovalContract(
            "trust-root rotation timestamp is not RFC3339".into(),
        ));
    }
    Ok(())
}

fn trust_rotation_payload(rotation: &TrustRootRotation) -> Result<Vec<u8>, SubstrateError> {
    let mut unsigned = rotation.clone();
    unsigned.signature_hex.clear();
    let mut payload = TRUST_ROTATION_DOMAIN.as_bytes().to_vec();
    payload.push(0);
    payload.extend(canonical_json(&unsigned)?);
    Ok(payload)
}

/// Persist the operator trust root without allowing silent replacement.
pub fn store_approval_trust_root(
    path: impl AsRef<Path>,
    trust_root: &ApprovalTrustRoot,
) -> Result<PathBuf, SubstrateError> {
    validate_approval_trust_root(trust_root)?;
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| SubstrateError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
        protect_directory(parent)?;
    }
    let payload = canonical_json(trust_root)?;
    persist_immutable(path, &payload, SubstrateError::ApprovalConflict)?;
    protect_file(path)?;
    Ok(path.to_path_buf())
}

/// Load and validate the immutable operator trust root.
pub fn load_approval_trust_root(
    path: impl AsRef<Path>,
) -> Result<ApprovalTrustRoot, SubstrateError> {
    let path = path.as_ref();
    let payload = fs::read(path).map_err(|source| SubstrateError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let trust_root: ApprovalTrustRoot =
        serde_json::from_slice(&payload).map_err(|source| SubstrateError::InvalidJson {
            path: path.to_path_buf(),
            source,
        })?;
    validate_approval_trust_root(&trust_root)?;
    Ok(trust_root)
}

/// Generate a portable Ed25519 signing key and its pinned trust root.
pub fn generate_approval_trust_root(
    key_id: impl Into<String>,
) -> Result<(ApprovalTrustRoot, Vec<u8>), SubstrateError> {
    let rng = SystemRandom::new();
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng)
        .map_err(|_| SubstrateError::InvalidApprovalContract("key generation failed".into()))?;
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref())
        .map_err(|_| SubstrateError::InvalidApprovalContract("generated key is invalid".into()))?;
    Ok((
        ApprovalTrustRoot {
            schema_version: APPROVAL_SCHEMA_VERSION,
            key_id: key_id.into(),
            public_key_hex: hex_encode(pair.public_key().as_ref()),
            key_epoch: 1,
        },
        pkcs8.as_ref().to_vec(),
    ))
}

/// Compute the deterministic identity of a prompt configuration.
pub fn prompt_configuration_hash(
    configuration: &PromptConfiguration,
) -> Result<Hash256, SubstrateError> {
    validate_prompt_configuration(configuration)?;
    domain_hash(PROMPT_DOMAIN, configuration)
}

/// Compute the deterministic identity of a runtime configuration.
pub fn runtime_configuration_hash(
    configuration: &RuntimeConfiguration,
) -> Result<Hash256, SubstrateError> {
    validate_runtime_configuration(configuration)?;
    domain_hash(RUNTIME_DOMAIN, configuration)
}

/// Compute the deterministic identity of a deployment manifest.
pub fn deployment_manifest_hash(manifest: &DeploymentManifest) -> Result<Hash256, SubstrateError> {
    validate_deployment_manifest(manifest)?;
    domain_hash(DEPLOYMENT_DOMAIN, manifest)
}

/// Compute the canonical hash for a revocation epoch and its events.
pub fn revocation_state_hash(
    epoch: u64,
    events: &[RevocationEvent],
) -> Result<Hash256, SubstrateError> {
    domain_hash(REVOCATION_DOMAIN, &RevocationPayload { epoch, events })
}

/// Sign an approved or rejected receipt using an operator-held PKCS#8 key.
pub fn sign_approval_receipt(
    receipt: &mut ApprovalReceipt,
    pkcs8_private_key: &[u8],
) -> Result<(), SubstrateError> {
    validate_receipt_fields(receipt)?;
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8_private_key)
        .map_err(|_| SubstrateError::InvalidApprovalContract("invalid signing key".into()))?;
    let payload = receipt_payload(receipt)?;
    receipt.signature_hex = hex_encode(pair.sign(&payload).as_ref());
    Ok(())
}

/// Verify a receipt against the operator-pinned public key.
pub fn verify_approval_receipt(
    receipt: &ApprovalReceipt,
    trust_root: &ApprovalTrustRoot,
) -> Result<(), SubstrateError> {
    validate_receipt_fields(receipt)?;
    validate_approval_trust_root(trust_root)?;
    if receipt.signature_hex.is_empty() {
        return Err(SubstrateError::UnsignedApproval);
    }
    if receipt.signing_key_id != trust_root.key_id {
        return Err(SubstrateError::ApprovalSignatureInvalid);
    }
    let public_key = hex_decode(&trust_root.public_key_hex)?;
    let signature = hex_decode(&receipt.signature_hex)?;
    UnparsedPublicKey::new(&ED25519, public_key)
        .verify(&receipt_payload(receipt)?, &signature)
        .map_err(|_| SubstrateError::ApprovalSignatureInvalid)
}

/// Compute current admission from a signed decision and exact bound identities.
pub fn compute_admission_status(
    receipt: Option<&ApprovalReceipt>,
    prompt_hash: &Hash256,
    runtime_hash: &Hash256,
    deployment_hash: &Hash256,
    revoked: bool,
) -> AdmissionStatus {
    let Some(receipt) = receipt else {
        return AdmissionStatus {
            state: AdmissionState::Unknown,
            reason: "no approval receipt".into(),
        };
    };
    if revoked {
        return AdmissionStatus {
            state: AdmissionState::Revoked,
            reason: "approval receipt was revoked".into(),
        };
    }
    if receipt.decision != ApprovalDecision::Approved {
        return AdmissionStatus {
            state: AdmissionState::Rejected,
            reason: "approval receipt rejected the configuration".into(),
        };
    }
    if &receipt.prompt_configuration_hash != prompt_hash {
        return AdmissionStatus {
            state: AdmissionState::Invalidated,
            reason: "prompt configuration identity changed".into(),
        };
    }
    if &receipt.runtime_configuration_hash != runtime_hash {
        return AdmissionStatus {
            state: AdmissionState::Invalidated,
            reason: "runtime configuration identity changed".into(),
        };
    }
    if &receipt.deployment_manifest_hash != deployment_hash {
        return AdmissionStatus {
            state: AdmissionState::Invalidated,
            reason: "deployment identity changed".into(),
        };
    }
    AdmissionStatus {
        state: AdmissionState::Approved,
        reason: "signed approval matches current identities".into(),
    }
}

/// Sign a revocation state using the operator-held PKCS#8 key.
pub fn sign_revocation_state(
    state: &mut RevocationState,
    pkcs8_private_key: &[u8],
) -> Result<(), SubstrateError> {
    validate_revocation_state(state)?;
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8_private_key)
        .map_err(|_| SubstrateError::InvalidApprovalContract("invalid signing key".into()))?;
    state.signature_hex = hex_encode(pair.sign(&revocation_payload(state)?).as_ref());
    Ok(())
}

/// Verify a revocation state against the operator-pinned public key.
pub fn verify_revocation_state(
    state: &RevocationState,
    trust_root: &ApprovalTrustRoot,
) -> Result<(), SubstrateError> {
    validate_revocation_state(state)?;
    validate_approval_trust_root(trust_root)?;
    if state.signature_hex.is_empty() {
        return Err(SubstrateError::UnsignedApproval);
    }
    let public_key = hex_decode(&trust_root.public_key_hex)?;
    let signature = hex_decode(&state.signature_hex)?;
    UnparsedPublicKey::new(&ED25519, public_key)
        .verify(&revocation_payload(state)?, &signature)
        .map_err(|_| SubstrateError::ApprovalSignatureInvalid)
}

/// Persist a signed receipt without allowing replacement or unsigned records.
pub fn store_approval_receipt(
    root: impl AsRef<Path>,
    anchor_path: impl AsRef<Path>,
    receipt: &ApprovalReceipt,
    trust_root: &ApprovalTrustRoot,
) -> Result<PathBuf, SubstrateError> {
    verify_approval_receipt(receipt, trust_root)?;
    if let Some((epoch, sequence)) = read_anchor(anchor_path.as_ref())? {
        if receipt.sequence < sequence {
            return Err(SubstrateError::ApprovalRollback(format!(
                "receipt sequence {} is older than anchor epoch {epoch} sequence {sequence}",
                receipt.sequence
            )));
        }
    }
    fs::create_dir_all(root.as_ref()).map_err(|source| SubstrateError::Io {
        path: root.as_ref().to_path_buf(),
        source,
    })?;
    let path = root.as_ref().join(format!("{}.json", receipt.receipt_id));
    let payload = canonical_json(receipt)?;
    persist_immutable(&path, &payload, SubstrateError::ApprovalConflict)?;
    let current_epoch = read_anchor(anchor_path.as_ref())?.map_or(0, |(epoch, _)| epoch);
    write_anchor(anchor_path.as_ref(), current_epoch, receipt.sequence)?;
    Ok(path)
}

/// Load and cryptographically verify one immutable approval receipt.
pub fn load_approval_receipt(
    path: impl AsRef<Path>,
    trust_root: &ApprovalTrustRoot,
) -> Result<ApprovalReceipt, SubstrateError> {
    let path = path.as_ref();
    let payload = fs::read(path).map_err(|source| SubstrateError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let receipt: ApprovalReceipt =
        serde_json::from_slice(&payload).map_err(|source| SubstrateError::InvalidJson {
            path: path.to_path_buf(),
            source,
        })?;
    verify_approval_receipt(&receipt, trust_root)?;
    Ok(receipt)
}

/// Load and cryptographically verify a persisted revocation state.
pub fn load_revocation_state(
    path: impl AsRef<Path>,
    trust_root: &ApprovalTrustRoot,
) -> Result<RevocationState, SubstrateError> {
    let path = path.as_ref();
    let payload = fs::read(path).map_err(|source| SubstrateError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let state: RevocationState =
        serde_json::from_slice(&payload).map_err(|source| SubstrateError::InvalidJson {
            path: path.to_path_buf(),
            source,
        })?;
    verify_revocation_state(&state, trust_root)?;
    Ok(state)
}

/// Report whether a signed revocation state names a receipt.
pub fn is_receipt_revoked(state: &RevocationState, receipt_id: &str) -> bool {
    state
        .events
        .iter()
        .any(|event| event.receipt_id == receipt_id)
}

/// Persist revocation state while rejecting rollback relative to a separate anchor.
pub fn store_revocation_state(
    root: impl AsRef<Path>,
    anchor_path: impl AsRef<Path>,
    state: &RevocationState,
    trust_root: &ApprovalTrustRoot,
) -> Result<PathBuf, SubstrateError> {
    verify_revocation_state(state, trust_root)?;
    let root = root.as_ref();
    let anchor_path = anchor_path.as_ref();
    let anchor = read_anchor(anchor_path)?;
    if let Some((epoch, sequence)) = anchor {
        let current = (
            state.epoch,
            state.events.last().map_or(0, |event| event.sequence),
        );
        if current < (epoch, sequence) {
            return Err(SubstrateError::ApprovalRollback(
                "revocation state is older than the protected anchor".into(),
            ));
        }
    }
    fs::create_dir_all(root).map_err(|source| SubstrateError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    let path = root.join(format!("epoch-{}.json", state.epoch));
    let payload = canonical_json(state)?;
    persist_immutable(&path, &payload, SubstrateError::RevocationConflict)?;
    let sequence = state.events.last().map_or(0, |event| event.sequence);
    write_anchor(anchor_path, state.epoch, sequence)?;
    Ok(path)
}

fn validate_prompt_configuration(
    configuration: &PromptConfiguration,
) -> Result<(), SubstrateError> {
    if configuration.schema_version != APPROVAL_SCHEMA_VERSION
        || configuration.template.schema_version != APPROVAL_SCHEMA_VERSION
        || configuration.request_schema.schema_version != APPROVAL_SCHEMA_VERSION
    {
        return Err(SubstrateError::InvalidApprovalContract(
            "unsupported prompt schema".into(),
        ));
    }
    require_text(&configuration.model_lineage_id, "model_lineage_id")?;
    require_text(&configuration.renderer_id, "renderer_id")?;
    require_text(&configuration.renderer_version, "renderer_version")?;
    require_text(&configuration.request_schema.schema_id, "schema_id")?;
    Ok(())
}

fn validate_runtime_configuration(
    configuration: &RuntimeConfiguration,
) -> Result<(), SubstrateError> {
    if configuration.schema_version != APPROVAL_SCHEMA_VERSION {
        return Err(SubstrateError::InvalidApprovalContract(
            "unsupported runtime schema".into(),
        ));
    }
    require_text(&configuration.backend, "backend")?;
    require_text(&configuration.backend_version, "backend_version")?;
    if configuration.resolved_argv.is_empty() {
        return Err(SubstrateError::InvalidApprovalContract(
            "resolved argv is empty".into(),
        ));
    }
    if configuration
        .allowed_environment
        .iter()
        .any(|entry| entry.name.trim().is_empty())
    {
        return Err(SubstrateError::InvalidApprovalContract(
            "environment name is empty".into(),
        ));
    }
    let mut environment_names = std::collections::BTreeSet::new();
    for entry in &configuration.allowed_environment {
        if !environment_names.insert(&entry.name) {
            return Err(SubstrateError::InvalidApprovalContract(format!(
                "environment name is duplicated: {}",
                entry.name
            )));
        }
        if !entry.secret_presence_only && entry.value_hash.is_none() {
            return Err(SubstrateError::InvalidApprovalContract(format!(
                "non-secret environment has no value hash: {}",
                entry.name
            )));
        }
    }
    Ok(())
}

fn validate_deployment_manifest(manifest: &DeploymentManifest) -> Result<(), SubstrateError> {
    if manifest.schema_version != APPROVAL_SCHEMA_VERSION {
        return Err(SubstrateError::InvalidApprovalContract(
            "unsupported deployment schema".into(),
        ));
    }
    require_text(&manifest.deployment_id, "deployment_id")?;
    require_text(&manifest.model_lineage_id, "model_lineage_id")?;
    if manifest.files.is_empty() {
        return Err(SubstrateError::InvalidApprovalContract(
            "deployment manifest has no files".into(),
        ));
    }
    validate_timestamp(&manifest.created_at)
}

fn validate_receipt_fields(receipt: &ApprovalReceipt) -> Result<(), SubstrateError> {
    if receipt.schema_version != APPROVAL_SCHEMA_VERSION {
        return Err(SubstrateError::InvalidApprovalContract(
            "unsupported receipt schema".into(),
        ));
    }
    for (value, name) in [
        (&receipt.receipt_id, "receipt_id"),
        (&receipt.model_lineage_id, "model_lineage_id"),
        (&receipt.policy_version, "policy_version"),
        (&receipt.validation_suite_id, "validation_suite_id"),
        (&receipt.reviewer_id, "reviewer_id"),
        (&receipt.signing_key_id, "signing_key_id"),
    ] {
        require_text(value, name)?;
    }
    validate_timestamp(&receipt.issued_at)
}

fn validate_revocation_state(state: &RevocationState) -> Result<(), SubstrateError> {
    if state.schema_version != APPROVAL_SCHEMA_VERSION {
        return Err(SubstrateError::InvalidApprovalContract(
            "unsupported revocation schema".into(),
        ));
    }
    let mut previous = 0;
    for event in &state.events {
        if event.schema_version != APPROVAL_SCHEMA_VERSION || event.sequence <= previous {
            return Err(SubstrateError::InvalidApprovalContract(
                "revocation sequence is not increasing".into(),
            ));
        }
        require_text(&event.receipt_id, "receipt_id")?;
        require_text(&event.reason, "reason")?;
        validate_timestamp(&event.revoked_at)?;
        previous = event.sequence;
    }
    let expected = domain_hash(
        REVOCATION_DOMAIN,
        &RevocationPayload {
            epoch: state.epoch,
            events: &state.events,
        },
    )?;
    if expected != state.state_hash {
        return Err(SubstrateError::ApprovalRollback(
            "revocation state hash mismatch".into(),
        ));
    }
    Ok(())
}

fn revocation_payload(state: &RevocationState) -> Result<Vec<u8>, SubstrateError> {
    let mut payload = REVOCATION_DOMAIN.as_bytes().to_vec();
    payload.push(0);
    payload.extend(canonical_json(&RevocationPayload {
        epoch: state.epoch,
        events: &state.events,
    })?);
    Ok(payload)
}

#[derive(Serialize)]
struct RevocationPayload<'a> {
    epoch: u64,
    events: &'a [RevocationEvent],
}

fn receipt_payload(receipt: &ApprovalReceipt) -> Result<Vec<u8>, SubstrateError> {
    let payload = ReceiptPayload {
        schema_version: receipt.schema_version,
        receipt_id: &receipt.receipt_id,
        decision: receipt.decision,
        model_lineage_id: &receipt.model_lineage_id,
        artifact_hash: &receipt.artifact_hash,
        prompt_configuration_hash: &receipt.prompt_configuration_hash,
        runtime_configuration_hash: &receipt.runtime_configuration_hash,
        deployment_manifest_hash: &receipt.deployment_manifest_hash,
        policy_version: &receipt.policy_version,
        validation_suite_id: &receipt.validation_suite_id,
        reviewer_id: &receipt.reviewer_id,
        signing_key_id: &receipt.signing_key_id,
        sequence: receipt.sequence,
        issued_at: &receipt.issued_at,
    };
    let mut bytes = APPROVAL_DOMAIN.as_bytes().to_vec();
    bytes.push(0);
    bytes.extend(canonical_json(&payload)?);
    Ok(bytes)
}

#[derive(Serialize)]
struct ReceiptPayload<'a> {
    schema_version: u32,
    receipt_id: &'a str,
    decision: ApprovalDecision,
    model_lineage_id: &'a str,
    artifact_hash: &'a Hash256,
    prompt_configuration_hash: &'a Hash256,
    runtime_configuration_hash: &'a Hash256,
    deployment_manifest_hash: &'a Hash256,
    policy_version: &'a str,
    validation_suite_id: &'a str,
    reviewer_id: &'a str,
    signing_key_id: &'a str,
    sequence: u64,
    issued_at: &'a str,
}

fn domain_hash<T: Serialize>(domain: &str, value: &T) -> Result<Hash256, SubstrateError> {
    let mut bytes = domain.as_bytes().to_vec();
    bytes.push(0);
    bytes.extend(canonical_json(value)?);
    Ok(sha256_data(&bytes))
}

fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, SubstrateError> {
    let value = serde_json::to_value(value).map_err(SubstrateError::Serialize)?;
    let value = canonical_value(value);
    serde_json::to_vec(&value).map_err(SubstrateError::Serialize)
}

fn canonical_value(value: Value) -> Value {
    match value {
        Value::Object(object) => {
            let sorted: BTreeMap<String, Value> = object
                .into_iter()
                .map(|(key, value)| (key, canonical_value(value)))
                .collect();
            let map: Map<String, Value> = sorted.into_iter().collect();
            Value::Object(map)
        }
        Value::Array(values) => Value::Array(values.into_iter().map(canonical_value).collect()),
        other => other,
    }
}

fn persist_immutable(
    path: &Path,
    payload: &[u8],
    conflict: impl Fn(PathBuf) -> SubstrateError,
) -> Result<(), SubstrateError> {
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => {
            file.write_all(payload)
                .map_err(|source| SubstrateError::Io {
                    path: path.to_path_buf(),
                    source,
                })?;
            file.sync_all().map_err(|source| SubstrateError::Io {
                path: path.to_path_buf(),
                source,
            })?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = fs::read(path).map_err(|source| SubstrateError::Io {
                path: path.to_path_buf(),
                source,
            })?;
            if existing == payload {
                Ok(())
            } else {
                Err(conflict(path.to_path_buf()))
            }
        }
        Err(source) => Err(SubstrateError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn protect_directory(path: &Path) -> Result<(), SubstrateError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|source| {
            SubstrateError::Io {
                path: path.to_path_buf(),
                source,
            }
        })?;
    }
    Ok(())
}

fn protect_file(path: &Path) -> Result<(), SubstrateError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|source| {
            SubstrateError::Io {
                path: path.to_path_buf(),
                source,
            }
        })?;
    }
    Ok(())
}

fn validate_relative_path(path: &Path) -> Result<(), SubstrateError> {
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
        || path.as_os_str().is_empty()
    {
        return Err(SubstrateError::InvalidApprovalContract(format!(
            "invalid snapshot relative path: {}",
            path.display()
        )));
    }
    Ok(())
}

fn verify_snapshot(root: &Path, expected: &[SnapshotFile]) -> Result<(), SubstrateError> {
    for file in expected {
        let path = root.join(&file.relative_path);
        let metadata = fs::symlink_metadata(&path).map_err(|source| SubstrateError::Io {
            path: path.clone(),
            source,
        })?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() != file.size_bytes
            || sha256_file(&path)? != file.sha256
        {
            return Err(SubstrateError::ApprovalRollback(format!(
                "published snapshot file failed identity verification: {}",
                file.relative_path.display()
            )));
        }
    }
    Ok(())
}

fn make_read_only(path: &Path) -> Result<(), SubstrateError> {
    let metadata = fs::metadata(path).map_err(|source| SubstrateError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut permissions = metadata.permissions();
    permissions.set_readonly(true);
    fs::set_permissions(path, permissions).map_err(|source| SubstrateError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn read_anchor(path: &Path) -> Result<Option<(u64, u64)>, SubstrateError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(SubstrateError::Io {
                path: path.to_path_buf(),
                source,
            })
        }
    };
    let mut fields = text.lines();
    let epoch = fields
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| SubstrateError::ApprovalRollback("invalid revocation anchor".into()))?;
    let sequence = fields
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| SubstrateError::ApprovalRollback("invalid revocation anchor".into()))?;
    Ok(Some((epoch, sequence)))
}

fn write_anchor(path: &Path, epoch: u64, sequence: u64) -> Result<(), SubstrateError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| SubstrateError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
        protect_directory(parent)?;
    }
    let anchor_payload = format!("{epoch}\n{sequence}\n");
    let anchor_tmp = path.with_extension("tmp");
    fs::write(&anchor_tmp, anchor_payload.as_bytes()).map_err(|source| SubstrateError::Io {
        path: anchor_tmp.clone(),
        source,
    })?;
    protect_file(&anchor_tmp)?;
    fs::rename(&anchor_tmp, path).map_err(|source| SubstrateError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn require_text(value: &str, name: &str) -> Result<(), SubstrateError> {
    if value.trim().is_empty() {
        Err(SubstrateError::InvalidApprovalContract(format!(
            "{name} is empty"
        )))
    } else {
        Ok(())
    }
}

fn validate_timestamp(value: &str) -> Result<(), SubstrateError> {
    OffsetDateTime::parse(value, &Rfc3339)
        .map(|_| ())
        .map_err(|_| {
            SubstrateError::InvalidApprovalContract(format!("invalid RFC3339 timestamp: {value}"))
        })
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hex_decode(value: &str) -> Result<Vec<u8>, SubstrateError> {
    if !value.len().is_multiple_of(2) {
        return Err(SubstrateError::InvalidApprovalContract(
            "hex value has odd length".into(),
        ));
    }
    (0..value.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&value[index..index + 2], 16)
                .map_err(|_| SubstrateError::InvalidApprovalContract("invalid hex value".into()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn hash(seed: u8) -> Hash256 {
        sha256_data(&[seed])
    }

    fn prompt() -> PromptConfiguration {
        PromptConfiguration {
            schema_version: 1,
            model_lineage_id: "model".into(),
            artifact_hash: hash(1),
            source_tokenizer_manifest_hash: hash(2),
            deployed_tokenizer_metadata_hash: hash(3),
            special_token_map_hash: hash(4),
            effective_tokenizer_hash: hash(5),
            template: TemplateIdentity {
                schema_version: 1,
                source_template_container_hash: hash(6),
                source_template_hash: Some(hash(7)),
                selected_variant: "default".into(),
                builtin_registry_hash: None,
                gguf_embedded_template_hash: Some(hash(8)),
                external_template_hash: None,
                effective_template_hash: hash(8),
                conversion_status: TemplateConversionStatus::Preserved,
            },
            renderer_id: "llama.cpp".into(),
            renderer_version: "test".into(),
            renderer_build_hash: hash(9),
            renderer_settings: BTreeMap::new(),
            request_schema: RequestPromptSchema {
                schema_version: 1,
                schema_id: "chat-v1".into(),
                allowed_roles: vec!["system".into(), "user".into(), "assistant".into()],
                allowed_content_types: vec!["text".into()],
                max_tools: 4,
                max_nesting: 8,
                max_request_bytes: 1_000_000,
                max_context_tokens: 4096,
                reasoning_controls: false,
                assistant_prefill: false,
                permitted_template_kwargs: BTreeMap::new(),
            },
            multimodal_processor_hash: None,
            mmproj_hash: None,
        }
    }

    #[test]
    fn prompt_hash_is_stable_for_map_order() {
        let mut first = prompt();
        first
            .renderer_settings
            .insert("b".into(), Value::Bool(true));
        first
            .renderer_settings
            .insert("a".into(), Value::Bool(false));
        let mut second = prompt();
        second
            .renderer_settings
            .insert("a".into(), Value::Bool(false));
        second
            .renderer_settings
            .insert("b".into(), Value::Bool(true));
        assert_eq!(
            prompt_configuration_hash(&first).expect("first hash"),
            prompt_configuration_hash(&second).expect("second hash")
        );
    }

    #[test]
    fn signed_receipt_requires_exact_trust_root() {
        let (root, key) = generate_approval_trust_root("operator-1").expect("key");
        let mut receipt = ApprovalReceipt {
            schema_version: 1,
            receipt_id: "receipt-1".into(),
            decision: ApprovalDecision::Approved,
            model_lineage_id: "model".into(),
            artifact_hash: hash(1),
            prompt_configuration_hash: hash(2),
            runtime_configuration_hash: hash(3),
            deployment_manifest_hash: hash(4),
            policy_version: "policy-1".into(),
            validation_suite_id: "suite-1".into(),
            reviewer_id: "operator".into(),
            signing_key_id: "operator-1".into(),
            sequence: 1,
            issued_at: "2026-01-01T00:00:00Z".into(),
            signature_hex: String::new(),
        };
        assert!(matches!(
            verify_approval_receipt(&receipt, &root),
            Err(SubstrateError::UnsignedApproval)
        ));
        sign_approval_receipt(&mut receipt, &key).expect("sign");
        verify_approval_receipt(&receipt, &root).expect("verify");
        receipt.reviewer_id = "tampered".into();
        assert!(matches!(
            verify_approval_receipt(&receipt, &root),
            Err(SubstrateError::ApprovalSignatureInvalid)
        ));
    }

    #[test]
    fn revocation_is_signed_and_anchor_rejects_rollback() {
        let (root, key) = generate_approval_trust_root("operator-1").expect("key");
        let event = RevocationEvent {
            schema_version: 1,
            sequence: 2,
            receipt_id: "receipt-1".into(),
            reason: "test".into(),
            revoked_at: "2026-01-01T00:00:00Z".into(),
        };
        let mut state = RevocationState {
            schema_version: 1,
            epoch: 3,
            state_hash: revocation_state_hash(3, std::slice::from_ref(&event)).expect("hash"),
            events: vec![event],
            signature_hex: String::new(),
        };
        assert!(matches!(
            verify_revocation_state(&state, &root),
            Err(SubstrateError::UnsignedApproval)
        ));
        sign_revocation_state(&mut state, &key).expect("sign");
        verify_revocation_state(&state, &root).expect("verify");

        let directory = tempfile::tempdir().expect("tempdir");
        let state_root = directory.path().join("revocations");
        let anchor = directory.path().join("revocation.anchor");
        store_revocation_state(&state_root, &anchor, &state, &root).expect("store");
        let mut older = state.clone();
        older.epoch = 2;
        older.state_hash = revocation_state_hash(2, &older.events).expect("older hash");
        sign_revocation_state(&mut older, &key).expect("sign older");
        assert!(matches!(
            store_revocation_state(&state_root, &anchor, &older, &root),
            Err(SubstrateError::ApprovalRollback(_))
        ));
    }

    #[test]
    fn trust_root_is_immutable_and_round_trips() {
        let (root, _) = generate_approval_trust_root("operator-1").expect("key");
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("trust-root.json");
        store_approval_trust_root(&path, &root).expect("store");
        assert_eq!(load_approval_trust_root(&path).expect("load"), root);

        let (replacement, _) = generate_approval_trust_root("operator-1").expect("key");
        assert!(matches!(
            store_approval_trust_root(&path, &replacement),
            Err(SubstrateError::ApprovalConflict(_))
        ));
    }

    #[test]
    fn trust_root_rotation_requires_old_key_and_advances_epoch() {
        let (old_root, old_key) = generate_approval_trust_root("operator-1").expect("old key");
        let (mut new_root, _) = generate_approval_trust_root("operator-2").expect("new key");
        new_root.key_epoch = 2;
        let mut rotation = TrustRootRotation {
            schema_version: 1,
            old_key_id: old_root.key_id.clone(),
            old_key_epoch: old_root.key_epoch,
            new_root: new_root.clone(),
            sequence: 1,
            issued_at: "2026-01-01T00:00:00Z".into(),
            signature_hex: String::new(),
        };
        sign_trust_root_rotation(&mut rotation, &old_key).expect("sign rotation");
        verify_trust_root_rotation(&rotation, &old_root).expect("verify rotation");
        let directory = tempfile::tempdir().expect("tempdir");
        let root_path = directory.path().join("new-root.json");
        let migration_path = directory.path().join("rotation.json");
        store_trust_root_rotation(&root_path, &migration_path, &rotation, &old_root)
            .expect("store rotation");
        assert_eq!(
            load_approval_trust_root(&root_path).expect("load"),
            new_root
        );

        let mut rollback = rotation.clone();
        rollback.new_root.key_epoch = 1;
        rollback.signature_hex.clear();
        sign_trust_root_rotation(&mut rollback, &old_key).expect("sign rollback");
        assert!(matches!(
            store_trust_root_rotation(
                directory.path().join("rollback.json"),
                directory.path().join("rollback-migration.json"),
                &rollback,
                &old_root,
            ),
            Err(SubstrateError::ApprovalRollback(_))
        ));
    }

    #[test]
    fn receipt_anchor_rejects_an_older_sequence_under_a_new_id() {
        let (root, key) = generate_approval_trust_root("operator-1").expect("key");
        let directory = tempfile::tempdir().expect("tempdir");
        let receipts = directory.path().join("receipts");
        let anchor = directory.path().join("approval.anchor");
        let mut newer = ApprovalReceipt {
            schema_version: 1,
            receipt_id: "newer".into(),
            decision: ApprovalDecision::Approved,
            model_lineage_id: "model".into(),
            artifact_hash: hash(1),
            prompt_configuration_hash: hash(2),
            runtime_configuration_hash: hash(3),
            deployment_manifest_hash: hash(4),
            policy_version: "policy-1".into(),
            validation_suite_id: "suite-1".into(),
            reviewer_id: "operator".into(),
            signing_key_id: "operator-1".into(),
            sequence: 9,
            issued_at: "2026-01-01T00:00:00Z".into(),
            signature_hex: String::new(),
        };
        sign_approval_receipt(&mut newer, &key).expect("sign newer");
        store_approval_receipt(&receipts, &anchor, &newer, &root).expect("store newer");

        let mut older = newer.clone();
        older.receipt_id = "older-different-id".into();
        older.sequence = 8;
        older.signature_hex.clear();
        sign_approval_receipt(&mut older, &key).expect("sign older");
        assert!(matches!(
            store_approval_receipt(&receipts, &anchor, &older, &root),
            Err(SubstrateError::ApprovalRollback(_))
        ));
    }

    #[test]
    fn deployment_snapshot_is_content_addressed_and_rejects_traversal() {
        let directory = tempfile::tempdir().expect("tempdir");
        let source = directory.path().join("server");
        fs::write(&source, b"server-bytes").expect("source");
        let snapshot_root = directory.path().join("snapshots");
        let inputs = vec![SnapshotInput {
            source: source.clone(),
            relative_path: PathBuf::from("bin/server"),
            required: true,
        }];
        let (published, snapshot_hash, files) =
            publish_deployment_snapshot(&snapshot_root, &inputs).expect("publish");
        assert_eq!(published, snapshot_root.join(snapshot_hash.as_str()));
        assert_eq!(files.len(), 1);
        assert_eq!(
            fs::read(published.join("bin/server")).expect("copy"),
            b"server-bytes"
        );
        let manifest = DeploymentManifest {
            schema_version: 1,
            deployment_id: "deployment-1".into(),
            model_lineage_id: "model".into(),
            artifact_hash: hash(1),
            prompt_configuration_hash: hash(2),
            runtime_configuration_hash: hash(3),
            snapshot_hash: snapshot_hash.clone(),
            files: vec![LibraryIdentity {
                path: PathBuf::from("bin/server"),
                sha256: files[0].sha256.clone(),
                required: true,
            }],
            created_at: "2026-01-01T00:00:00Z".into(),
        };
        assert!(verify_deployment_snapshot(&published, &manifest).is_ok());
        assert_eq!(
            publish_deployment_snapshot(&snapshot_root, &inputs)
                .expect("idempotent")
                .1,
            snapshot_hash
        );

        let traversal = [SnapshotInput {
            source,
            relative_path: PathBuf::from("../server"),
            required: true,
        }];
        assert!(matches!(
            publish_deployment_snapshot(&snapshot_root, &traversal),
            Err(SubstrateError::InvalidApprovalContract(_))
        ));
    }

    #[test]
    fn admission_state_invalidates_bound_identity_changes() {
        let receipt = ApprovalReceipt {
            schema_version: 1,
            receipt_id: "receipt-1".into(),
            decision: ApprovalDecision::Approved,
            model_lineage_id: "model".into(),
            artifact_hash: hash(1),
            prompt_configuration_hash: hash(2),
            runtime_configuration_hash: hash(3),
            deployment_manifest_hash: hash(4),
            policy_version: "policy-1".into(),
            validation_suite_id: "suite-1".into(),
            reviewer_id: "operator".into(),
            signing_key_id: "operator-1".into(),
            sequence: 1,
            issued_at: "2026-01-01T00:00:00Z".into(),
            signature_hex: "signed".into(),
        };
        assert_eq!(
            compute_admission_status(Some(&receipt), &hash(2), &hash(3), &hash(4), false).state,
            AdmissionState::Approved
        );
        assert_eq!(
            compute_admission_status(Some(&receipt), &hash(9), &hash(3), &hash(4), false).state,
            AdmissionState::Invalidated
        );
        assert_eq!(
            compute_admission_status(Some(&receipt), &hash(2), &hash(3), &hash(4), true).state,
            AdmissionState::Revoked
        );
    }

    #[test]
    fn request_policy_rejects_template_overrides_and_unsupported_inputs() {
        let schema = prompt().request_schema;
        let valid = serde_json::json!({
            "messages": [{"role": "user", "content": "hello"}],
            "max_tokens": 128
        });
        validate_prompt_request(&schema, &valid).expect("valid request");
        let override_request = serde_json::json!({
            "messages": [{"role": "user", "content": "hello"}],
            "chat_template": "unsafe"
        });
        assert!(matches!(
            validate_prompt_request(&schema, &override_request),
            Err(SubstrateError::RequestPolicyViolation(_))
        ));
        let role_request = serde_json::json!({
            "messages": [{"role": "tool", "content": "result"}]
        });
        assert!(matches!(
            validate_prompt_request(&schema, &role_request),
            Err(SubstrateError::RequestPolicyViolation(_))
        ));
    }

    #[test]
    fn source_inspection_hashes_named_templates_without_executing_them() {
        let directory = tempfile::tempdir().expect("tempdir");
        fs::write(
            directory.path().join("tokenizer_config.json"),
            br#"{"chat_template":{"default":"{{ messages }}","tools":"{{ tools }}"}}"#,
        )
        .expect("config");
        fs::write(directory.path().join("special_tokens_map.json"), b"{}").expect("tokens");
        let inspection = inspect_template_source(directory.path()).expect("inspection");
        assert_eq!(inspection.template_hashes.len(), 2);
        assert!(inspection.template_hashes.contains_key("tools"));
        assert!(inspection.special_tokens_hash.is_some());
    }

    #[test]
    fn approval_bundle_is_immutable_and_requires_explicit_secret_name() {
        let directory = tempfile::tempdir().expect("tempdir");
        let mut paths = Vec::new();
        for name in ["prompt", "runtime", "deployment", "receipt", "trust"] {
            let path = directory.path().join(format!("{name}.json"));
            fs::write(&path, b"{}").expect("fixture");
            paths.push(path);
        }
        let snapshot = directory.path().join("snapshot");
        fs::create_dir(&snapshot).expect("snapshot");
        let bundle = ApprovalBundle {
            schema_version: 1,
            bundle_id: "bundle-1".into(),
            prompt: paths[0].clone(),
            runtime: paths[1].clone(),
            deployment: paths[2].clone(),
            snapshot_root: snapshot,
            receipt: paths[3].clone(),
            trust_root: paths[4].clone(),
            revocation_state: None,
            backend_api_key_env: "LMML_BACKEND_API_KEY".into(),
            public_port: 8080,
            backend_port: 1200,
            context: 4096,
            gpu_layers: -1,
            threads: 0,
        };
        let bundle_path = directory.path().join("bundle.json");
        store_approval_bundle(&bundle_path, &bundle).expect("store");
        assert_eq!(load_approval_bundle(&bundle_path).expect("load"), bundle);
        assert!(matches!(
            store_approval_bundle(&bundle_path, &bundle),
            Err(SubstrateError::ApprovalConflict(_))
        ));
    }
}
