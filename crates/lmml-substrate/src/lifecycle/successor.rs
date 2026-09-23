//! Cross-record validation for successor checkpoint admission.

use std::collections::BTreeSet;

use super::{
    parse_timestamp, validate_baseline_manifest, validate_successor_candidate_manifest,
    validate_successor_manifest, validate_training_run_manifest,
};
use crate::{
    BaselineManifest, SubstrateError, SubstrateManifest, SuccessorCandidateManifest,
    SuccessorManifest, TrainingRunManifest,
};

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

/// Bind an admitted successor to immutable training and merge provenance.
pub fn validate_successor_provenance(
    successor: &SuccessorManifest,
    candidate: &SuccessorCandidateManifest,
    candidate_manifest_hash: &crate::Hash256,
    training: &TrainingRunManifest,
    training_manifest_hash: &crate::Hash256,
) -> Result<(), SubstrateError> {
    validate_successor_manifest(successor)?;
    validate_successor_candidate_manifest(candidate)?;
    validate_training_run_manifest(training)?;
    if &successor.training_manifest_hash != training_manifest_hash
        || &candidate.training_manifest_hash != training_manifest_hash
        || &successor.candidate_manifest_hash != candidate_manifest_hash
        || successor.training_run_id != training.training_run_id
        || successor.training_run_id != candidate.training_run_id
        || successor.adapter_artifact_id != training.adapter.artifact_id
        || successor.adapter_artifact_id != candidate.adapter_artifact_id
        || successor.dataset_hash != training.dataset_hash
        || successor.seed != training.seed
        || successor.training_config != training.training_config
        || successor.approved_allowlist != training.approved_allowlist
        || successor.trainable_parameters != training.trainable_parameters
        || successor.requested_dtype != candidate.requested_dtype
        || successor.effective_load_dtype != candidate.effective_load_dtype
        || successor.merge_dtype != candidate.merge_dtype
        || successor.output_dtype != candidate.output_dtype
        || successor.merge_tool != candidate.merge_tool
        || successor.merge_tool_version != candidate.merge_tool_version
        || successor.tensor_manifest_hash
            != candidate.candidate_manifest.model.canonical_manifest_hash
        || successor.merge_delta != candidate.delta
        || successor.equivalence_max_absolute_delta != candidate.equivalence_max_absolute_delta
        || successor.equivalence_tolerance != candidate.equivalence_tolerance
    {
        return Err(SubstrateError::InvalidLifecycleManifest(
            "successor admission provenance does not match immutable training and merge records"
                .to_string(),
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
