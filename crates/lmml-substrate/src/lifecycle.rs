//! Validation and append-only persistence for model lifecycle records.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::storage::{persist_immutable, validate_schema_version};
use crate::{
    parse_artifact_manifest_json, validate_identifier, ArtifactManifest, BaselineManifest,
    ModelLease, ModelRepresentation, ModelRequest, RuntimeCapability, RuntimeManifest,
    SubstrateError, SubstrateManifest, SuccessorCandidateManifest, SuccessorManifest,
    TrainingAuthorizationManifest, TrainingRunManifest,
};

/// Parse and validate a pristine baseline manifest.
pub fn parse_baseline_manifest_json(payload: &str) -> Result<BaselineManifest, SubstrateError> {
    parse_validated(payload, validate_baseline_manifest)
}

/// Persist a pristine baseline without replacing an existing record.
pub fn store_baseline_manifest(
    path: impl AsRef<Path>,
    manifest: &BaselineManifest,
) -> Result<PathBuf, SubstrateError> {
    validate_baseline_manifest(manifest)?;
    store_json(path.as_ref(), manifest)
}

/// Parse and validate a pre-optimization training authorization.
pub fn parse_training_authorization_manifest_json(
    payload: &str,
) -> Result<TrainingAuthorizationManifest, SubstrateError> {
    parse_validated(payload, validate_training_authorization_manifest)
}

/// Persist a pre-optimization training authorization without replacement.
pub fn store_training_authorization_manifest(
    path: impl AsRef<Path>,
    manifest: &TrainingAuthorizationManifest,
) -> Result<PathBuf, SubstrateError> {
    validate_training_authorization_manifest(manifest)?;
    store_json(path.as_ref(), manifest)
}

/// Compute the stable SHA-256 identity of a validated training authorization.
pub fn training_authorization_hash(
    manifest: &TrainingAuthorizationManifest,
) -> Result<crate::Hash256, SubstrateError> {
    validate_training_authorization_manifest(manifest)?;
    let payload = serde_json::to_vec(manifest).map_err(SubstrateError::Serialize)?;
    Ok(crate::sha256_data(&payload))
}

/// Parse and validate a successor training-run manifest.
pub fn parse_training_run_manifest_json(
    payload: &str,
) -> Result<TrainingRunManifest, SubstrateError> {
    parse_validated(payload, validate_training_run_manifest)
}

/// Persist a validated successor training-run record.
pub fn store_training_run_manifest(
    path: impl AsRef<Path>,
    manifest: &TrainingRunManifest,
) -> Result<PathBuf, SubstrateError> {
    validate_training_run_manifest(manifest)?;
    store_json(path.as_ref(), manifest)
}

/// Parse and validate a candidate-only successor merge record.
pub fn parse_successor_candidate_manifest_json(
    payload: &str,
) -> Result<SuccessorCandidateManifest, SubstrateError> {
    parse_validated(payload, validate_successor_candidate_manifest)
}

/// Persist a candidate-only successor merge record.
pub fn store_successor_candidate_manifest(
    path: impl AsRef<Path>,
    manifest: &SuccessorCandidateManifest,
) -> Result<PathBuf, SubstrateError> {
    validate_successor_candidate_manifest(manifest)?;
    store_json(path.as_ref(), manifest)
}

/// Parse and validate an admitted successor record.
pub fn parse_successor_manifest_json(payload: &str) -> Result<SuccessorManifest, SubstrateError> {
    parse_validated(payload, validate_successor_manifest)
}

/// Persist an admitted successor without replacing its lineage record.
pub fn store_successor_manifest(
    path: impl AsRef<Path>,
    manifest: &SuccessorManifest,
) -> Result<PathBuf, SubstrateError> {
    validate_successor_manifest(manifest)?;
    store_json(path.as_ref(), manifest)
}

/// Parse and validate an artifact-bound runtime manifest.
pub fn parse_runtime_manifest_json(payload: &str) -> Result<RuntimeManifest, SubstrateError> {
    parse_validated(payload, validate_runtime_manifest)
}

/// Persist a runtime manifest without replacing an existing runtime identity.
pub fn store_runtime_manifest(
    path: impl AsRef<Path>,
    manifest: &RuntimeManifest,
) -> Result<PathBuf, SubstrateError> {
    validate_runtime_manifest(manifest)?;
    store_json(path.as_ref(), manifest)
}

/// Parse and validate an issued model lease.
pub fn parse_model_lease_json(payload: &str) -> Result<ModelLease, SubstrateError> {
    parse_validated(payload, validate_model_lease)
}

/// Persist an issued lease without replacing an existing lease identity.
pub fn store_model_lease(
    path: impl AsRef<Path>,
    lease: &ModelLease,
) -> Result<PathBuf, SubstrateError> {
    validate_model_lease(lease)?;
    store_json(path.as_ref(), lease)
}

/// Read all validated artifact records in deterministic artifact-ID order.
pub fn load_artifact_manifests(
    root: impl AsRef<Path>,
) -> Result<Vec<ArtifactManifest>, SubstrateError> {
    load_json_directory(root.as_ref(), parse_artifact_manifest_json, |manifest| {
        manifest.artifact.artifact_id.as_str()
    })
}

/// Read all validated runtime records in deterministic runtime-ID order.
pub fn load_runtime_manifests(
    root: impl AsRef<Path>,
) -> Result<Vec<RuntimeManifest>, SubstrateError> {
    load_json_directory(root.as_ref(), parse_runtime_manifest_json, |manifest| {
        manifest.runtime_id.as_str()
    })
}

/// Select an admitted artifact-bound runtime and issue a capability lease.
pub fn issue_model_lease(
    request: &ModelRequest,
    artifacts: &[ArtifactManifest],
    runtimes: &[RuntimeManifest],
    lease_id: &str,
) -> Result<ModelLease, SubstrateError> {
    validate_identifier(&request.model_lineage_id)?;
    validate_identifier(lease_id)?;
    if request.purpose.trim().is_empty() {
        return Err(SubstrateError::InvalidLifecycleManifest(
            "runtime request requires a purpose".to_string(),
        ));
    }
    if request.required_capabilities.is_empty() {
        return Err(SubstrateError::InvalidLifecycleManifest(
            "runtime request requires at least one capability".to_string(),
        ));
    }
    if request.representation_preference.is_empty() {
        return Err(SubstrateError::InvalidLifecycleManifest(
            "runtime request requires a representation preference".to_string(),
        ));
    }
    for artifact in artifacts {
        crate::storage::validate_artifact_manifest(artifact)?;
    }
    for runtime in runtimes {
        validate_runtime_manifest(runtime)?;
    }
    let mut eligible: Vec<(&RuntimeManifest, &ArtifactManifest)> = runtimes
        .iter()
        .filter_map(|runtime| {
            artifacts
                .iter()
                .find(|artifact| artifact.artifact.artifact_id == runtime.artifact_id)
                .map(|artifact| (runtime, artifact))
        })
        .filter(|(runtime, artifact)| {
            runtime.model_lineage_id == request.model_lineage_id
                && runtime.model_lineage_id == artifact.artifact.model_lineage_id
                && runtime.artifact_hash == artifact.artifact.artifact_hash
                && runtime.representation == artifact.artifact.representation
                && runtime.quantization == artifact.artifact.quantization
                && artifact.admission.is_some()
                && request
                    .representation_preference
                    .contains(&runtime.representation)
                && request
                    .required_capabilities
                    .iter()
                    .all(|capability| runtime.capabilities.contains(capability))
        })
        .collect();
    eligible.sort_by(|left, right| left.0.runtime_id.cmp(&right.0.runtime_id));
    let Some((runtime, artifact)) = eligible.first() else {
        return Err(SubstrateError::RuntimeCapabilityUnsupported(format!(
            "lineage {} with {:?}",
            request.model_lineage_id, request.required_capabilities
        )));
    };
    let lease = ModelLease {
        lease_id: lease_id.to_string(),
        model_lineage_id: request.model_lineage_id.clone(),
        artifact_id: runtime.artifact_id.clone(),
        runtime_id: runtime.runtime_id.clone(),
        endpoint: runtime.endpoint.clone(),
        manifest_hash: artifact.source_hashes[0].clone(),
        artifact_hash: artifact.artifact.artifact_hash.clone(),
        capabilities: request.required_capabilities.clone(),
    };
    validate_model_lease(&lease)?;
    Ok(lease)
}

/// Validate base identity, trainable allowlists, finite state, and adapter hash.
pub fn validate_training_run_against_base(
    run: &TrainingRunManifest,
    base: &SubstrateManifest,
) -> Result<(), SubstrateError> {
    validate_training_run_manifest(run)?;
    if run.base_lineage_id != base.model.lineage_id {
        return Err(SubstrateError::BaseLineageMismatch {
            expected: base.model.lineage_id.clone(),
            actual: run.base_lineage_id.clone(),
        });
    }
    if run.base_manifest_hash != base.model.canonical_manifest_hash
        || run.config_hash != base.config_hash
        || run.tokenizer_hash != base.tokenizer_hash
    {
        return Err(SubstrateError::ManifestMismatch);
    }
    let adapter_hash = crate::sha256_file(&run.adapter_path)?;
    if adapter_hash != run.adapter.artifact_hash {
        return Err(SubstrateError::ManifestMismatch);
    }
    crate::validate_adapter_safetensors(&run.adapter_path, &run.adapter_tensors)?;
    Ok(())
}

/// Validate a pre-optimization authorization against its canonical base.
pub fn validate_training_authorization_against_base(
    authorization: &TrainingAuthorizationManifest,
    base: &SubstrateManifest,
) -> Result<(), SubstrateError> {
    validate_training_authorization_manifest(authorization)?;
    if authorization.base_lineage_id != base.model.lineage_id {
        return Err(SubstrateError::BaseLineageMismatch {
            expected: base.model.lineage_id.clone(),
            actual: authorization.base_lineage_id.clone(),
        });
    }
    if authorization.base_manifest_hash != base.model.canonical_manifest_hash
        || authorization.config_hash != base.config_hash
        || authorization.tokenizer_hash != base.tokenizer_hash
    {
        return Err(SubstrateError::ManifestMismatch);
    }
    Ok(())
}

/// Require a completed run to reproduce its pre-optimization authorization.
pub fn validate_training_run_against_authorization(
    run: &TrainingRunManifest,
    authorization: &TrainingAuthorizationManifest,
) -> Result<(), SubstrateError> {
    validate_training_run_manifest(run)?;
    validate_training_authorization_manifest(authorization)?;
    let expected_hash = training_authorization_hash(authorization)?;
    if run.authorization_id != authorization.authorization_id
        || run.authorization_hash != expected_hash
        || run.base_lineage_id != authorization.base_lineage_id
        || run.base_manifest_hash != authorization.base_manifest_hash
        || run.config_hash != authorization.config_hash
        || run.tokenizer_hash != authorization.tokenizer_hash
        || run.dataset_hash != authorization.dataset_hash
        || run.seed != authorization.seed
        || run.training_config != authorization.training_config
        || run.approved_allowlist != authorization.approved_allowlist
        || run.trainable_parameters != authorization.trainable_parameters
    {
        return Err(SubstrateError::InvalidLifecycleManifest(
            "training run does not match its pre-optimization authorization".to_string(),
        ));
    }
    let authorization_time = parse_timestamp(&authorization.created_at)?;
    let run_time = parse_timestamp(&run.created_at)?;
    if run_time < authorization_time {
        return Err(SubstrateError::InvalidLifecycleManifest(
            "training run predates its pre-optimization authorization".to_string(),
        ));
    }
    Ok(())
}

/// Validate a candidate merge against its explicit parent and training run.
pub fn validate_candidate_against_training(
    candidate: &SuccessorCandidateManifest,
    training: &TrainingRunManifest,
) -> Result<(), SubstrateError> {
    validate_successor_candidate_manifest(candidate)?;
    if candidate.parent_lineage_id != training.base_lineage_id
        || candidate.training_run_id != training.training_run_id
        || candidate.adapter_artifact_id != training.adapter.artifact_id
    {
        return Err(SubstrateError::InvalidLifecycleManifest(
            "candidate parent, training run, or adapter identity mismatch".to_string(),
        ));
    }
    if parse_timestamp(&candidate.created_at)? < parse_timestamp(&training.created_at)? {
        return Err(SubstrateError::InvalidLifecycleManifest(
            "successor candidate predates its training run".to_string(),
        ));
    }
    Ok(())
}

/// Require a normal LoRA merge to preserve architecture and tensor shapes.
pub fn validate_successor_structure(
    parent: &SubstrateManifest,
    candidate: &SubstrateManifest,
) -> Result<(), SubstrateError> {
    if candidate.model.parent.as_deref() != Some(parent.model.lineage_id.as_str()) {
        return Err(SubstrateError::BaseLineageMismatch {
            expected: parent.model.lineage_id.clone(),
            actual: candidate.model.parent.clone().unwrap_or_default(),
        });
    }
    if candidate.architecture.model_type != parent.architecture.model_type
        || candidate.architecture.architectures != parent.architecture.architectures
        || candidate.config_hash != parent.config_hash
        || candidate.tokenizer_hash != parent.tokenizer_hash
        || candidate.tensor_count != parent.tensor_count
        || candidate.parameter_count != parent.parameter_count
    {
        return Err(SubstrateError::InvalidLifecycleManifest(
            "successor architecture differs from its parent".to_string(),
        ));
    }
    let parent_tensors = parent
        .tensors
        .iter()
        .map(|tensor| (&tensor.name, &tensor.dtype, &tensor.shape));
    let candidate_tensors = candidate
        .tensors
        .iter()
        .map(|tensor| (&tensor.name, &tensor.dtype, &tensor.shape));
    if !parent_tensors.eq(candidate_tensors) {
        return Err(SubstrateError::InvalidLifecycleManifest(
            "successor tensor names, dtypes, or shapes differ from its parent".to_string(),
        ));
    }
    Ok(())
}

/// Validate successor admission against its candidate and regression gates.
pub fn validate_successor_admission(
    successor: &SuccessorManifest,
    candidate: &SuccessorCandidateManifest,
) -> Result<(), SubstrateError> {
    validate_successor_manifest(successor)?;
    if successor.parent_lineage_id != candidate.parent_lineage_id
        || successor.successor_lineage_id != candidate.candidate_manifest.model.lineage_id
        || successor.candidate_id != candidate.candidate_id
        || successor.training_run_id != candidate.training_run_id
        || successor.successor_hash != candidate.candidate_manifest.model.canonical_manifest_hash
    {
        return Err(SubstrateError::InvalidLifecycleManifest(
            "successor admission does not match its candidate".to_string(),
        ));
    }
    if parse_timestamp(&successor.admitted_at)? < parse_timestamp(&candidate.created_at)? {
        return Err(SubstrateError::InvalidLifecycleManifest(
            "successor admission predates its merge candidate".to_string(),
        ));
    }
    Ok(())
}

/// Bind successor regression evidence to one exact frozen parent baseline.
pub fn validate_successor_baseline(
    successor: &SuccessorManifest,
    baseline: &BaselineManifest,
    baseline_hash: &crate::Hash256,
) -> Result<(), SubstrateError> {
    validate_successor_manifest(successor)?;
    validate_baseline_manifest(baseline)?;
    if successor.parent_lineage_id != baseline.model_lineage_id
        || successor.parent_baseline_id != baseline.baseline_id
        || &successor.parent_baseline_hash != baseline_hash
    {
        return Err(SubstrateError::InvalidLifecycleManifest(
            "successor regression is not bound to the exact frozen parent baseline".to_string(),
        ));
    }
    let baseline_cases = baseline
        .cases
        .iter()
        .map(|case| case.case_id.as_str())
        .collect::<BTreeSet<_>>();
    let regression_cases = successor
        .regression
        .iter()
        .map(|case| case.case_id.as_str())
        .collect::<BTreeSet<_>>();
    if baseline_cases != regression_cases {
        return Err(SubstrateError::InvalidLifecycleManifest(
            "regression cases must exactly match the frozen parent baseline".to_string(),
        ));
    }
    Ok(())
}

fn validate_baseline_manifest(manifest: &BaselineManifest) -> Result<(), SubstrateError> {
    validate_schema_version(manifest.schema_version)?;
    validate_identifier(&manifest.baseline_id)?;
    validate_identifier(&manifest.model_lineage_id)?;
    validate_identifier(&manifest.artifact_id)?;
    validate_timestamp(&manifest.created_at)?;
    if manifest.backend.trim().is_empty()
        || manifest.backend_version.trim().is_empty()
        || manifest.cases.is_empty()
    {
        return Err(SubstrateError::InvalidLifecycleManifest(
            "baseline requires a backend, version, and cases".to_string(),
        ));
    }
    let mut case_ids = BTreeSet::new();
    for case in &manifest.cases {
        validate_identifier(&case.case_id)?;
        if case.prompt.is_empty() || !case_ids.insert(&case.case_id) {
            return Err(SubstrateError::InvalidLifecycleManifest(
                "baseline case IDs must be unique and prompts non-empty".to_string(),
            ));
        }
        if crate::sha256_bytes(case.output.as_bytes()) != case.output_hash {
            return Err(SubstrateError::InvalidLifecycleManifest(format!(
                "baseline output hash mismatch for {}",
                case.case_id
            )));
        }
    }
    Ok(())
}

fn validate_training_run_manifest(run: &TrainingRunManifest) -> Result<(), SubstrateError> {
    validate_schema_version(run.schema_version)?;
    validate_identifier(&run.training_run_id)?;
    validate_identifier(&run.authorization_id)?;
    validate_identifier(&run.base_lineage_id)?;
    validate_identifier(&run.adapter.artifact_id)?;
    validate_timestamp(&run.created_at)?;
    if run.adapter.representation != ModelRepresentation::Safetensors
        || run.adapter.quantization.is_some()
        || run.adapter.model_lineage_id != run.base_lineage_id
    {
        return Err(SubstrateError::InvalidLifecycleManifest(
            "training adapter identity is inconsistent with its base".to_string(),
        ));
    }
    if run.adapter_tensors.is_empty() {
        return Err(SubstrateError::InvalidLifecycleManifest(
            "training run requires an adapter tensor inventory".to_string(),
        ));
    }
    validate_trainable_inventory(&run.approved_allowlist, &run.trainable_parameters)?;
    if !run.final_loss.is_finite() {
        return Err(SubstrateError::NonFiniteTrainingState("loss".to_string()));
    }
    if run.gradient_norms.is_empty() {
        return Err(SubstrateError::InvalidLifecycleManifest(
            "training run requires gradient-norm evidence".to_string(),
        ));
    }
    if run.gradient_norms.iter().any(|norm| !norm.is_finite()) {
        return Err(SubstrateError::NonFiniteTrainingState(
            "gradient norm".to_string(),
        ));
    }
    Ok(())
}

fn validate_training_authorization_manifest(
    authorization: &TrainingAuthorizationManifest,
) -> Result<(), SubstrateError> {
    validate_schema_version(authorization.schema_version)?;
    validate_identifier(&authorization.authorization_id)?;
    validate_identifier(&authorization.base_lineage_id)?;
    validate_timestamp(&authorization.created_at)?;
    validate_trainable_inventory(
        &authorization.approved_allowlist,
        &authorization.trainable_parameters,
    )
}

fn validate_trainable_inventory(
    approved_allowlist: &[String],
    trainable_parameters: &[String],
) -> Result<(), SubstrateError> {
    if approved_allowlist.is_empty() || trainable_parameters.is_empty() {
        return Err(SubstrateError::InvalidLifecycleManifest(
            "training run requires allowlist and trainable inventory".to_string(),
        ));
    }
    let mut allowlist_entries = BTreeSet::new();
    for allowed in approved_allowlist {
        let prefix = allowed.strip_suffix('*').unwrap_or(allowed);
        if prefix.is_empty() || prefix.contains('*') || !allowlist_entries.insert(allowed) {
            return Err(SubstrateError::InvalidLifecycleManifest(
                "training allowlist entries must be non-empty, unique, and use only a trailing wildcard"
                    .to_string(),
            ));
        }
    }
    let mut parameters = BTreeSet::new();
    for parameter in trainable_parameters {
        if parameter.is_empty() || !parameters.insert(parameter) {
            return Err(SubstrateError::InvalidLifecycleManifest(
                "trainable parameter names must be non-empty and unique".to_string(),
            ));
        }
        if !is_lora_parameter_name(parameter) {
            return Err(SubstrateError::UnexpectedTrainableParameter(
                parameter.clone(),
            ));
        }
        if !approved_allowlist
            .iter()
            .any(|allowed| matches_allowlist(parameter, allowed))
        {
            return Err(SubstrateError::UnexpectedTrainableParameter(
                parameter.clone(),
            ));
        }
    }
    Ok(())
}

fn validate_successor_candidate_manifest(
    candidate: &SuccessorCandidateManifest,
) -> Result<(), SubstrateError> {
    validate_schema_version(candidate.schema_version)?;
    validate_identifier(&candidate.candidate_id)?;
    validate_identifier(&candidate.parent_lineage_id)?;
    validate_identifier(&candidate.training_run_id)?;
    validate_identifier(&candidate.adapter_artifact_id)?;
    validate_timestamp(&candidate.created_at)?;
    crate::storage::validate_substrate_manifest(&candidate.candidate_manifest)?;
    if candidate.candidate_manifest.model.parent.as_deref()
        != Some(candidate.parent_lineage_id.as_str())
    {
        return Err(SubstrateError::InvalidLifecycleManifest(
            "candidate canonical manifest requires its explicit parent".to_string(),
        ));
    }
    if candidate.delta.changed_tensor_count == 0
        || candidate.delta.max_absolute_delta <= 0.0
        || candidate.delta.aggregate_norm_delta <= 0.0
    {
        return Err(SubstrateError::NoOpMerge);
    }
    let finite = [
        candidate.delta.max_absolute_delta,
        candidate.delta.aggregate_norm_delta,
        candidate.equivalence_max_absolute_delta,
        candidate.equivalence_tolerance,
    ]
    .iter()
    .all(|value| value.is_finite());
    if !finite {
        return Err(SubstrateError::NonFiniteTrainingState(
            "merge or equivalence metric".to_string(),
        ));
    }
    if candidate.equivalence_max_absolute_delta < 0.0
        || candidate.equivalence_tolerance < 0.0
        || candidate.equivalence_max_absolute_delta > candidate.equivalence_tolerance
    {
        return Err(SubstrateError::EquivalenceFailure {
            delta: candidate.equivalence_max_absolute_delta,
            tolerance: candidate.equivalence_tolerance,
        });
    }
    if candidate.requested_dtype.is_empty()
        || candidate.effective_load_dtype.is_empty()
        || candidate.merge_dtype.is_empty()
        || candidate.output_dtype.is_empty()
    {
        return Err(SubstrateError::InvalidLifecycleManifest(
            "candidate merge requires explicit dtype provenance".to_string(),
        ));
    }
    Ok(())
}

fn validate_successor_manifest(manifest: &SuccessorManifest) -> Result<(), SubstrateError> {
    validate_schema_version(manifest.schema_version)?;
    validate_identifier(&manifest.successor_lineage_id)?;
    validate_identifier(&manifest.parent_lineage_id)?;
    validate_identifier(&manifest.candidate_id)?;
    validate_identifier(&manifest.training_run_id)?;
    validate_identifier(&manifest.parent_baseline_id)?;
    validate_timestamp(&manifest.admitted_at)?;
    if manifest.successor_lineage_id == manifest.parent_lineage_id {
        return Err(SubstrateError::InvalidLifecycleManifest(
            "successor requires a distinct explicit parent".to_string(),
        ));
    }
    if manifest.regression.is_empty() {
        return Err(SubstrateError::InvalidLifecycleManifest(
            "successor admission requires regression evidence".to_string(),
        ));
    }
    let mut case_ids = BTreeSet::new();
    for result in &manifest.regression {
        validate_identifier(&result.case_id)?;
        if !case_ids.insert(&result.case_id) {
            return Err(SubstrateError::InvalidLifecycleManifest(
                "successor regression case IDs must be unique".to_string(),
            ));
        }
        if !result.parent_metric.is_finite()
            || !result.candidate_metric.is_finite()
            || !result.delta.is_finite()
            || !result.maximum_degradation.is_finite()
        {
            return Err(SubstrateError::NonFiniteTrainingState(format!(
                "regression case {}",
                result.case_id
            )));
        }
        let expected_delta = result.candidate_metric - result.parent_metric;
        let delta_tolerance =
            f64::EPSILON * expected_delta.abs().max(result.delta.abs()).max(1.0) * 8.0;
        if result.maximum_degradation < 0.0
            || (result.delta - expected_delta).abs() > delta_tolerance
        {
            return Err(SubstrateError::InvalidLifecycleManifest(format!(
                "regression delta is inconsistent for case {}",
                result.case_id
            )));
        }
        let expected_pass = result.delta <= result.maximum_degradation;
        if result.passed != expected_pass || !expected_pass {
            return Err(SubstrateError::RegressionGateFailure(
                result.case_id.clone(),
            ));
        }
    }
    Ok(())
}

fn validate_runtime_manifest(manifest: &RuntimeManifest) -> Result<(), SubstrateError> {
    validate_identifier(&manifest.runtime_id)?;
    validate_identifier(&manifest.model_lineage_id)?;
    validate_identifier(&manifest.artifact_id)?;
    validate_timestamp(&manifest.created_at)?;
    if manifest.backend.trim().is_empty()
        || manifest.backend_version.trim().is_empty()
        || manifest.endpoint.trim().is_empty()
        || manifest.context_size == 0
        || manifest.pid == 0
        || manifest.capabilities.is_empty()
    {
        return Err(SubstrateError::InvalidLifecycleManifest(
            "runtime manifest is incomplete".to_string(),
        ));
    }
    if manifest.backend == "llama.cpp"
        && manifest.capabilities.iter().any(|capability| {
            matches!(
                capability,
                RuntimeCapability::HiddenStateObservation | RuntimeCapability::ElevenTapObservation
            )
        })
    {
        return Err(SubstrateError::RuntimeCapabilityUnsupported(
            "llama.cpp does not expose arbitrary hidden states or eleven taps".to_string(),
        ));
    }
    Ok(())
}

fn validate_model_lease(lease: &ModelLease) -> Result<(), SubstrateError> {
    validate_identifier(&lease.lease_id)?;
    validate_identifier(&lease.model_lineage_id)?;
    validate_identifier(&lease.artifact_id)?;
    validate_identifier(&lease.runtime_id)?;
    if lease.endpoint.trim().is_empty() || lease.capabilities.is_empty() {
        return Err(SubstrateError::InvalidLifecycleManifest(
            "model lease requires endpoint and capabilities".to_string(),
        ));
    }
    Ok(())
}

fn matches_allowlist(parameter: &str, allowed: &str) -> bool {
    allowed
        .strip_suffix('*')
        .map_or(parameter == allowed, |prefix| parameter.starts_with(prefix))
}

fn is_lora_parameter_name(parameter: &str) -> bool {
    [
        ".lora_A",
        ".lora_B",
        ".lora_embedding_A",
        ".lora_embedding_B",
    ]
    .iter()
    .any(|marker| parameter.contains(marker))
}

fn parse_timestamp(timestamp: &str) -> Result<OffsetDateTime, SubstrateError> {
    OffsetDateTime::parse(timestamp, &Rfc3339).map_err(|_| {
        SubstrateError::InvalidLifecycleManifest("timestamp must be RFC3339".to_string())
    })
}

fn validate_timestamp(timestamp: &str) -> Result<(), SubstrateError> {
    parse_timestamp(timestamp).map(|_| ())
}

fn parse_validated<T: DeserializeOwned>(
    payload: &str,
    validate: impl FnOnce(&T) -> Result<(), SubstrateError>,
) -> Result<T, SubstrateError> {
    let value = serde_json::from_str(payload).map_err(SubstrateError::ManifestJson)?;
    validate(&value)?;
    Ok(value)
}

fn store_json<T: Serialize>(path: &Path, value: &T) -> Result<PathBuf, SubstrateError> {
    let payload = serde_json::to_vec_pretty(value).map_err(SubstrateError::Serialize)?;
    persist_immutable(path, &payload, SubstrateError::LifecycleConflict)?;
    Ok(path.to_path_buf())
}

fn load_json_directory<T>(
    root: &Path,
    parse: impl Fn(&str) -> Result<T, SubstrateError>,
    key: impl for<'a> Fn(&'a T) -> &'a str,
) -> Result<Vec<T>, SubstrateError> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(SubstrateError::Io {
                path: root.to_path_buf(),
                source,
            });
        }
    };
    let mut manifests = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| SubstrateError::Io {
            path: root.to_path_buf(),
            source,
        })?;
        if entry.path().extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let payload = fs::read_to_string(entry.path()).map_err(|source| SubstrateError::Io {
            path: entry.path(),
            source,
        })?;
        manifests.push(parse(&payload)?);
    }
    manifests.sort_by(|left, right| key(left).cmp(key(right)));
    Ok(manifests)
}

#[cfg(test)]
mod tests;
