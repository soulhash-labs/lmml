//! Successor training, merge, regression, and admission gates.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

#[derive(Debug, Deserialize)]
struct MergeEvidence {
    schema_version: u32,
    base: PathBuf,
    adapter: PathBuf,
    candidate_path: PathBuf,
    merge_tool: String,
    merge_tool_version: String,
    requested_dtype: String,
    effective_load_dtype: String,
    merge_dtype: String,
    output_dtype: String,
    delta: lmml_substrate::MergeDelta,
    equivalence_max_absolute_delta: f64,
    equivalence_tolerance: f64,
}

pub(super) struct CandidateOptions<'a> {
    pub base_manifest: &'a Path,
    pub base_source: &'a Path,
    pub training_manifest: &'a Path,
    pub authorization_manifest: Option<&'a Path>,
    pub candidate_path: &'a Path,
    pub successor_lineage_id: &'a str,
    pub candidate_id: &'a str,
    pub report: &'a Path,
    pub output: Option<&'a Path>,
    pub json: bool,
}

pub(super) fn inspect_adapter(path: &Path, json: bool) -> i32 {
    let tensors = match lmml_substrate::inspect_safetensors_file(path) {
        Ok(tensors) => tensors,
        Err(error) => return fail("adapter inspection", error.to_string()),
    };
    if let Err(error) = lmml_substrate::validate_adapter_safetensors(path, &tensors) {
        return fail("adapter inspection", error.to_string());
    }
    let digest = match lmml_substrate::sha256_file(path) {
        Ok(digest) => digest,
        Err(error) => return fail("adapter inspection", error.to_string()),
    };
    if json {
        println!(
            "{}",
            serde_json::json!({
                "path": path,
                "sha256": digest,
                "tensors": tensors,
            })
        );
    } else {
        println!("adapter: {}", path.display());
        println!("sha256: {digest}");
        println!("tensors: {}", tensors.len());
    }
    0
}

pub(super) fn authorize_training(
    base_manifest: &Path,
    authorization: &Path,
    output: Option<&Path>,
    data_root: &Path,
    json: bool,
) -> i32 {
    let base = match super::read_substrate_manifest(base_manifest) {
        Ok(base) => base,
        Err(error) => return fail("successor training authorization", error),
    };
    let authorization = match read_and_parse(
        authorization,
        lmml_substrate::parse_training_authorization_manifest_json,
    ) {
        Ok(authorization) => authorization,
        Err(error) => return fail("successor training authorization", error),
    };
    if let Err(error) =
        lmml_substrate::validate_training_authorization_against_base(&authorization, &base)
    {
        return fail("successor training authorization", error.to_string());
    }
    let output = output.map(PathBuf::from).unwrap_or_else(|| {
        data_root
            .join("lmml/models/artifacts/adapters/authorizations")
            .join(&authorization.authorization_id)
            .join("authorization_manifest.json")
    });
    if let Err(error) =
        lmml_substrate::store_training_authorization_manifest(&output, &authorization)
    {
        return fail("successor training authorization", error.to_string());
    }
    emit(&authorization, &output, json, "training authorization")
}

pub(super) fn register_training(
    base_manifest: &Path,
    authorization_manifest: &Path,
    report: &Path,
    output: Option<&Path>,
    data_root: &Path,
    json: bool,
) -> i32 {
    let base = match super::read_substrate_manifest(base_manifest) {
        Ok(base) => base,
        Err(error) => return fail("successor training", error),
    };
    let authorization = match read_and_parse(
        authorization_manifest,
        lmml_substrate::parse_training_authorization_manifest_json,
    ) {
        Ok(authorization) => authorization,
        Err(error) => return fail("successor training", error),
    };
    let training = match read_and_parse(report, lmml_substrate::parse_training_run_manifest_json) {
        Ok(training) => training,
        Err(error) => return fail("successor training", error),
    };
    if let Err(error) =
        lmml_substrate::validate_training_authorization_against_base(&authorization, &base)
            .and_then(|()| {
                lmml_substrate::validate_training_run_against_authorization(
                    &training,
                    &authorization,
                )
            })
            .and_then(|()| lmml_substrate::validate_training_run_against_base(&training, &base))
    {
        return fail("successor training gate", error.to_string());
    }
    let output = output.map(PathBuf::from).unwrap_or_else(|| {
        data_root
            .join("lmml/models/artifacts/adapters")
            .join(&training.training_run_id)
            .join("training_manifest.json")
    });
    if let Err(error) = lmml_substrate::store_training_run_manifest(&output, &training) {
        return fail("successor training registration", error.to_string());
    }
    emit(&training, &output, json, "training run")
}

pub(super) fn register_candidate(options: CandidateOptions<'_>, data_root: &Path) -> i32 {
    let base = match super::read_substrate_manifest(options.base_manifest) {
        Ok(base) => base,
        Err(error) => return fail("successor merge", error),
    };
    let training_manifest_path = match options.training_manifest.canonicalize() {
        Ok(path) => path,
        Err(error) => return fail("successor merge", error.to_string()),
    };
    let (training, training_manifest_hash) = match read_and_parse_hashed(
        &training_manifest_path,
        lmml_substrate::parse_training_run_manifest_json,
    ) {
        Ok(training) => training,
        Err(error) => return fail("successor merge", error),
    };
    let authorization_path = options
        .authorization_manifest
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            data_root
                .join("lmml/models/artifacts/adapters/authorizations")
                .join(&training.authorization_id)
                .join("authorization_manifest.json")
        });
    let authorization = match read_and_parse(
        &authorization_path,
        lmml_substrate::parse_training_authorization_manifest_json,
    ) {
        Ok(authorization) => authorization,
        Err(error) => return fail("successor merge authorization", error),
    };
    if let Err(error) = lmml_substrate::verify_safetensors(options.base_source, &base) {
        return fail("successor merge base identity", error.to_string());
    }
    let base_source = match options.base_source.canonicalize() {
        Ok(path) => path,
        Err(error) => return fail("successor merge", error.to_string()),
    };
    let candidate_path = match options.candidate_path.canonicalize() {
        Ok(path) => path,
        Err(error) => return fail("successor merge", error.to_string()),
    };
    let report_path = match options.report.canonicalize() {
        Ok(path) => path,
        Err(error) => return fail("successor merge", error.to_string()),
    };
    let evidence: MergeEvidence = match read_json(options.report) {
        Ok(evidence) => evidence,
        Err(error) => return fail("successor merge", error),
    };
    if evidence.schema_version != 1 {
        return fail(
            "successor merge",
            format!(
                "unsupported merge evidence schema {}",
                evidence.schema_version
            ),
        );
    }
    let training_adapter = match training.adapter_path.canonicalize() {
        Ok(path) => path,
        Err(error) => return fail("successor merge", error.to_string()),
    };
    if let Err(error) = validate_successor_paths(
        &base_source,
        &training_adapter,
        &candidate_path,
        &report_path,
    ) {
        return fail("successor merge", error);
    }
    if let Err(error) =
        validate_merge_evidence_paths(&evidence, &base_source, &training_adapter, &candidate_path)
    {
        return fail("successor merge", error);
    }
    let canonical_candidate = match lmml_substrate::import_successor_safetensors(
        &candidate_path,
        options.successor_lineage_id,
        &base.model.lineage_id,
    ) {
        Ok(candidate) => candidate,
        Err(error) => return fail("successor candidate import", error.to_string()),
    };
    let created_at = match OffsetDateTime::now_utc().format(&Rfc3339) {
        Ok(timestamp) => timestamp,
        Err(error) => return fail("successor merge", error.to_string()),
    };
    let candidate = lmml_substrate::SuccessorCandidateManifest {
        schema_version: lmml_substrate::SCHEMA_VERSION,
        candidate_id: options.candidate_id.to_string(),
        parent_lineage_id: base.model.lineage_id.clone(),
        training_run_id: training.training_run_id.clone(),
        training_manifest_path,
        training_manifest_hash,
        adapter_artifact_id: training.adapter.artifact_id.clone(),
        candidate_path,
        merge_tool: evidence.merge_tool,
        merge_tool_version: evidence.merge_tool_version,
        requested_dtype: evidence.requested_dtype,
        effective_load_dtype: evidence.effective_load_dtype,
        merge_dtype: evidence.merge_dtype,
        output_dtype: evidence.output_dtype,
        candidate_manifest: canonical_candidate,
        delta: evidence.delta,
        equivalence_max_absolute_delta: evidence.equivalence_max_absolute_delta,
        equivalence_tolerance: evidence.equivalence_tolerance,
        created_at,
    };
    if let Err(error) =
        lmml_substrate::validate_training_authorization_against_base(&authorization, &base)
            .and_then(|()| {
                lmml_substrate::validate_training_run_against_authorization(
                    &training,
                    &authorization,
                )
            })
            .and_then(|()| lmml_substrate::validate_training_run_against_base(&training, &base))
            .and_then(|()| {
                lmml_substrate::validate_candidate_against_training(&candidate, &training)
            })
            .and_then(|()| {
                lmml_substrate::validate_successor_structure(&base, &candidate.candidate_manifest)
            })
            .and_then(|()| verify_successor_candidate_payload(&candidate))
    {
        return fail("successor merge gate", error.to_string());
    }
    let output = options.output.map(PathBuf::from).unwrap_or_else(|| {
        data_root
            .join("lmml/models/artifacts/successors/candidates")
            .join(&candidate.candidate_id)
            .join("candidate_manifest.json")
    });
    if let Err(error) = lmml_substrate::store_successor_candidate_manifest(&output, &candidate) {
        return fail("successor candidate registration", error.to_string());
    }
    emit(&candidate, &output, options.json, "successor candidate")
}

pub(super) fn admit(
    candidate_manifest: &Path,
    baseline_manifest: &Path,
    report: &Path,
    output: Option<&Path>,
    data_root: &Path,
    json: bool,
) -> i32 {
    let (candidate, candidate_manifest_hash) = match read_and_parse_hashed(
        candidate_manifest,
        lmml_substrate::parse_successor_candidate_manifest_json,
    ) {
        Ok(candidate) => candidate,
        Err(error) => return fail("successor admission", error),
    };
    let (baseline, baseline_hash) = match read_and_parse_hashed(
        baseline_manifest,
        lmml_substrate::parse_baseline_manifest_json,
    ) {
        Ok(baseline) => baseline,
        Err(error) => return fail("successor admission", error),
    };
    let successor = match read_and_parse(report, lmml_substrate::parse_successor_manifest_json) {
        Ok(successor) => successor,
        Err(error) => return fail("successor admission", error),
    };
    let (training, training_manifest_hash) = match read_and_parse_hashed(
        &candidate.training_manifest_path,
        lmml_substrate::parse_training_run_manifest_json,
    ) {
        Ok(training) => training,
        Err(error) => return fail("successor admission", error),
    };
    if let Err(error) =
        lmml_substrate::validate_successor_baseline(&successor, &baseline, &baseline_hash)
            .and_then(|()| lmml_substrate::validate_successor_admission(&successor, &candidate))
            .and_then(|()| {
                lmml_substrate::validate_candidate_against_training(&candidate, &training)
            })
            .and_then(|()| {
                lmml_substrate::validate_successor_provenance(
                    &successor,
                    &candidate,
                    &candidate_manifest_hash,
                    &training,
                    &training_manifest_hash,
                )
            })
            .and_then(|()| verify_successor_candidate_payload(&candidate))
    {
        return fail("successor admission gate", error.to_string());
    }
    let output = output.map(PathBuf::from).unwrap_or_else(|| {
        data_root
            .join("lmml/models/successors")
            .join(&successor.successor_lineage_id)
            .join("successor_manifest.json")
    });
    let canonical_path = data_root
        .join("lmml/models/manifests")
        .join(format!("{}.json", successor.successor_lineage_id));
    if let Err(error) =
        lmml_substrate::store_substrate_manifest(&canonical_path, &candidate.candidate_manifest)
    {
        return fail("successor canonical registration", error.to_string());
    }
    if let Err(error) = lmml_substrate::store_successor_manifest(&output, &successor) {
        return fail("successor admission registration", error.to_string());
    }
    emit(&successor, &output, json, "admitted successor")
}

fn verify_successor_candidate_payload(
    candidate: &lmml_substrate::SuccessorCandidateManifest,
) -> Result<(), lmml_substrate::SubstrateError> {
    lmml_substrate::verify_safetensors(&candidate.candidate_path, &candidate.candidate_manifest)?;
    lmml_substrate::validate_finite_checkpoint(
        &candidate.candidate_path,
        &candidate.candidate_manifest,
    )
}

fn read_and_parse<T>(
    path: &Path,
    parse: impl FnOnce(&str) -> Result<T, lmml_substrate::SubstrateError>,
) -> Result<T, String> {
    let payload =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    parse(&payload).map_err(|error| format!("{}: {error}", path.display()))
}

fn read_and_parse_hashed<T>(
    path: &Path,
    parse: impl FnOnce(&str) -> Result<T, lmml_substrate::SubstrateError>,
) -> Result<(T, lmml_substrate::Hash256), String> {
    let payload = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let hash = lmml_substrate::sha256_data(&payload);
    let payload =
        std::str::from_utf8(&payload).map_err(|error| format!("{}: {error}", path.display()))?;
    let value = parse(payload).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok((value, hash))
}

fn validate_merge_evidence_paths(
    evidence: &MergeEvidence,
    base_source: &Path,
    training_adapter: &Path,
    candidate_path: &Path,
) -> Result<(), String> {
    let evidence_candidate = evidence
        .candidate_path
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if evidence_candidate != candidate_path {
        return Err("merge evidence names a different candidate directory".to_string());
    }
    let evidence_base = evidence
        .base
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if evidence_base != base_source {
        return Err("merge evidence names a different canonical base".to_string());
    }
    let evidence_adapter = evidence
        .adapter
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if evidence_adapter != training_adapter
        && training_adapter.parent() != Some(evidence_adapter.as_path())
    {
        return Err("merge evidence names a different adapter".to_string());
    }
    Ok(())
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

fn validate_successor_paths(
    base: &Path,
    adapter: &Path,
    candidate: &Path,
    report: &Path,
) -> Result<(), String> {
    if paths_overlap(base, candidate) {
        return Err("candidate directory must remain outside the canonical parent".to_string());
    }
    if paths_overlap(adapter, candidate) {
        return Err("candidate directory must remain outside the adapter".to_string());
    }
    if report.starts_with(base) || report.starts_with(adapter) || report.starts_with(candidate) {
        return Err(
            "merge report must remain outside the base, adapter, and candidate".to_string(),
        );
    }
    Ok(())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let payload =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    serde_json::from_str(&payload).map_err(|error| format!("{}: {error}", path.display()))
}

fn emit(value: &impl serde::Serialize, output: &Path, json: bool, label: &str) -> i32 {
    if json {
        match serde_json::to_string_pretty(value) {
            Ok(payload) => println!("{payload}"),
            Err(error) => return fail(label, error.to_string()),
        }
    } else {
        println!("{label} registered\nmanifest: {}", output.display());
    }
    0
}

fn fail(operation: &str, error: String) -> i32 {
    eprintln!("{operation} failed: {error}");
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_evidence_is_bound_to_base_adapter_and_candidate_paths() {
        let directory = tempfile::tempdir().expect("tempdir");
        let base = directory.path().join("base");
        let adapter = directory.path().join("adapter");
        let candidate = directory.path().join("candidate");
        std::fs::create_dir_all(&base).expect("base");
        std::fs::create_dir_all(&adapter).expect("adapter");
        std::fs::create_dir_all(&candidate).expect("candidate");
        let adapter_file = adapter.join("adapter_model.safetensors");
        std::fs::write(&adapter_file, b"adapter").expect("adapter payload");
        let evidence = MergeEvidence {
            schema_version: 1,
            base: base.canonicalize().expect("canonical base"),
            adapter: adapter.canonicalize().expect("canonical adapter"),
            candidate_path: candidate.canonicalize().expect("canonical candidate"),
            merge_tool: "scripts/merge_lora.py".into(),
            merge_tool_version: "sha256:test".into(),
            requested_dtype: "bfloat16".into(),
            effective_load_dtype: "bfloat16".into(),
            merge_dtype: "bfloat16".into(),
            output_dtype: "bfloat16".into(),
            delta: lmml_substrate::MergeDelta {
                changed_tensor_count: 1,
                unchanged_tensor_count: 0,
                max_absolute_delta: 1.0,
                aggregate_norm_delta: 1.0,
            },
            equivalence_max_absolute_delta: 0.0,
            equivalence_tolerance: 1e-4,
        };

        assert!(validate_merge_evidence_paths(
            &evidence,
            &base.canonicalize().expect("canonical base"),
            &adapter_file.canonicalize().expect("canonical adapter"),
            &candidate.canonicalize().expect("canonical candidate"),
        )
        .is_ok());

        let other = directory.path().join("other");
        std::fs::create_dir(&other).expect("other");
        assert!(validate_merge_evidence_paths(
            &evidence,
            &other.canonicalize().expect("canonical other"),
            &adapter_file.canonicalize().expect("canonical adapter"),
            &candidate.canonicalize().expect("canonical candidate"),
        )
        .is_err());
    }

    #[test]
    fn successor_candidate_must_not_nest_with_canonical_base() {
        let base = Path::new("/models/qwen38-base");
        assert!(paths_overlap(base, base));
        assert!(paths_overlap(
            base,
            Path::new("/models/qwen38-base/candidate")
        ));
        assert!(paths_overlap(Path::new("/models"), base));
        assert!(!paths_overlap(base, Path::new("/models/qwen38-successor")));
    }

    #[test]
    fn successor_report_and_candidate_remain_outside_inputs() {
        let base = Path::new("/models/qwen38-base");
        let adapter = Path::new("/models/adapters/a1");
        let candidate = Path::new("/models/successors/m1");
        let report = Path::new("/models/reports/m1.json");
        validate_successor_paths(base, adapter, candidate, report).expect("separate paths");

        for invalid_report in [
            base.join("report.json"),
            adapter.join("report.json"),
            candidate.join("report.json"),
        ] {
            assert!(validate_successor_paths(base, adapter, candidate, &invalid_report).is_err());
        }
        assert!(
            validate_successor_paths(base, adapter, &adapter.join("candidate"), report).is_err()
        );
    }

    #[test]
    fn successor_admission_rechecks_merged_checkpoint_bytes() {
        let directory = tempfile::tempdir().expect("tempdir");
        let candidate_path = directory.path().join("candidate");
        std::fs::create_dir(&candidate_path).expect("candidate");
        write_safetensors_fixture(&candidate_path);
        let candidate_manifest = lmml_substrate::import_successor_safetensors(
            &candidate_path,
            "qwen38-successor-1",
            "qwen38-27b",
        )
        .expect("candidate manifest");
        let candidate = lmml_substrate::SuccessorCandidateManifest {
            schema_version: lmml_substrate::SCHEMA_VERSION,
            candidate_id: "candidate-1".into(),
            parent_lineage_id: "qwen38-27b".into(),
            training_run_id: "train-1".into(),
            training_manifest_path: directory.path().join("training_manifest.json"),
            training_manifest_hash: lmml_substrate::Hash256::parse("e".repeat(64))
                .expect("training hash"),
            adapter_artifact_id: "adapter-1".into(),
            candidate_path: candidate_path.clone(),
            merge_tool: "scripts/merge_lora.py".into(),
            merge_tool_version: "sha256:test".into(),
            requested_dtype: "bfloat16".into(),
            effective_load_dtype: "bfloat16".into(),
            merge_dtype: "bfloat16".into(),
            output_dtype: "bfloat16".into(),
            candidate_manifest,
            delta: lmml_substrate::MergeDelta {
                changed_tensor_count: 1,
                unchanged_tensor_count: 0,
                max_absolute_delta: 1.0,
                aggregate_norm_delta: 1.0,
            },
            equivalence_max_absolute_delta: 0.0,
            equivalence_tolerance: 1e-4,
            created_at: "2026-09-23T00:00:00Z".into(),
        };
        verify_successor_candidate_payload(&candidate).expect("unchanged candidate");

        let shard = candidate_path.join("model-00001-of-00001.safetensors");
        let mut bytes = std::fs::read(&shard).expect("shard");
        *bytes.last_mut().expect("tensor byte") ^= 0xff;
        std::fs::write(shard, bytes).expect("mutated shard");

        assert!(verify_successor_candidate_payload(&candidate).is_err());
    }

    #[test]
    fn successor_admission_rejects_changed_training_record() {
        let directory = tempfile::tempdir().expect("tempdir");
        let data_root = directory.path().join("data");
        let candidate_path = directory.path().join("candidate");
        std::fs::create_dir(&candidate_path).expect("candidate");
        write_safetensors_fixture(&candidate_path);
        let checkpoint = lmml_substrate::import_successor_safetensors(
            &candidate_path,
            "qwen38-successor-1",
            "qwen38-27b",
        )
        .expect("candidate checkpoint");
        let hash = |value: char| {
            lmml_substrate::Hash256::parse(value.to_string().repeat(64)).expect("hash")
        };
        let training = lmml_substrate::TrainingRunManifest {
            schema_version: lmml_substrate::SCHEMA_VERSION,
            training_run_id: "train-1".into(),
            authorization_id: "authorize-1".into(),
            authorization_hash: hash('a'),
            base_lineage_id: "qwen38-27b".into(),
            base_manifest_hash: hash('b'),
            config_hash: hash('c'),
            tokenizer_hash: hash('d'),
            dataset_hash: hash('e'),
            seed: 42,
            training_config: std::collections::BTreeMap::new(),
            approved_allowlist: vec!["model.layers.*".into()],
            trainable_parameters: vec!["model.layers.0.self_attn.q_proj.lora_A".into()],
            final_loss: 1.0,
            gradient_norms: vec![0.5],
            adapter: lmml_substrate::ArtifactIdentity {
                artifact_id: "adapter-1".into(),
                model_lineage_id: "qwen38-27b".into(),
                representation: lmml_substrate::ModelRepresentation::Safetensors,
                quantization: None,
                artifact_hash: hash('f'),
            },
            adapter_path: directory.path().join("adapter.safetensors"),
            adapter_tensors: vec![checkpoint.tensors[0].clone()],
            created_at: "2026-09-23T00:00:00Z".into(),
        };
        let training_path = directory.path().join("training.json");
        lmml_substrate::store_training_run_manifest(&training_path, &training)
            .expect("training record");
        let training_hash = lmml_substrate::sha256_file(&training_path).expect("training hash");
        let candidate = lmml_substrate::SuccessorCandidateManifest {
            schema_version: lmml_substrate::SCHEMA_VERSION,
            candidate_id: "candidate-1".into(),
            parent_lineage_id: "qwen38-27b".into(),
            training_run_id: training.training_run_id.clone(),
            training_manifest_path: training_path.clone(),
            training_manifest_hash: training_hash.clone(),
            adapter_artifact_id: training.adapter.artifact_id.clone(),
            candidate_path: candidate_path.clone(),
            merge_tool: "scripts/merge_lora.py".into(),
            merge_tool_version: "sha256:test".into(),
            requested_dtype: "bfloat16".into(),
            effective_load_dtype: "bfloat16".into(),
            merge_dtype: "bfloat16".into(),
            output_dtype: "bfloat16".into(),
            candidate_manifest: checkpoint,
            delta: lmml_substrate::MergeDelta {
                changed_tensor_count: 1,
                unchanged_tensor_count: 0,
                max_absolute_delta: 1.0,
                aggregate_norm_delta: 1.0,
            },
            equivalence_max_absolute_delta: 0.0,
            equivalence_tolerance: 1e-4,
            created_at: "2026-09-23T00:00:00Z".into(),
        };
        let candidate_record = directory.path().join("candidate.json");
        lmml_substrate::store_successor_candidate_manifest(&candidate_record, &candidate)
            .expect("candidate record");
        let candidate_hash =
            lmml_substrate::sha256_file(&candidate_record).expect("candidate hash");
        let baseline_output = "baseline".to_string();
        let baseline = lmml_substrate::BaselineManifest {
            schema_version: lmml_substrate::SCHEMA_VERSION,
            baseline_id: "baseline-1".into(),
            model_lineage_id: "qwen38-27b".into(),
            artifact_id: "qwen38-q8".into(),
            artifact_hash: hash('1'),
            backend: "llama.cpp".into(),
            backend_version: "test".into(),
            parameters: vec!["--temp".into(), "0".into()],
            cases: vec![lmml_substrate::BaselineCase {
                case_id: "anchor-1".into(),
                prompt: "anchor prompt".into(),
                output_hash: lmml_substrate::sha256_data(baseline_output.as_bytes()),
                output: baseline_output,
            }],
            created_at: "2026-09-23T00:00:00Z".into(),
        };
        let baseline_path = directory.path().join("baseline.json");
        lmml_substrate::store_baseline_manifest(&baseline_path, &baseline)
            .expect("baseline record");
        let baseline_hash = lmml_substrate::sha256_file(&baseline_path).expect("baseline hash");
        let successor_hash = candidate
            .candidate_manifest
            .model
            .canonical_manifest_hash
            .clone();
        let successor = lmml_substrate::SuccessorManifest {
            schema_version: lmml_substrate::SCHEMA_VERSION,
            successor_lineage_id: "qwen38-successor-1".into(),
            parent_lineage_id: "qwen38-27b".into(),
            candidate_id: candidate.candidate_id.clone(),
            training_run_id: training.training_run_id.clone(),
            training_manifest_hash: training_hash,
            candidate_manifest_hash: candidate_hash,
            adapter_artifact_id: training.adapter.artifact_id.clone(),
            dataset_hash: training.dataset_hash.clone(),
            seed: training.seed,
            training_config: training.training_config.clone(),
            approved_allowlist: training.approved_allowlist.clone(),
            trainable_parameters: training.trainable_parameters.clone(),
            requested_dtype: candidate.requested_dtype.clone(),
            effective_load_dtype: candidate.effective_load_dtype.clone(),
            merge_dtype: candidate.merge_dtype.clone(),
            output_dtype: candidate.output_dtype.clone(),
            merge_tool: candidate.merge_tool.clone(),
            merge_tool_version: candidate.merge_tool_version.clone(),
            tensor_manifest_hash: successor_hash.clone(),
            merge_delta: candidate.delta.clone(),
            equivalence_max_absolute_delta: candidate.equivalence_max_absolute_delta,
            equivalence_tolerance: candidate.equivalence_tolerance,
            parent_baseline_id: baseline.baseline_id.clone(),
            parent_baseline_hash: baseline_hash,
            successor_hash,
            regression: vec![lmml_substrate::RegressionResult {
                case_id: "anchor-1".into(),
                parent_metric: 1.0,
                candidate_metric: 1.0,
                delta: 0.0,
                maximum_degradation: 0.1,
                passed: true,
            }],
            admitted_at: "2026-09-23T00:00:00Z".into(),
        };
        let report = directory.path().join("successor.json");
        lmml_substrate::store_successor_manifest(&report, &successor).expect("successor report");
        let mut payload = std::fs::read(&training_path).expect("training bytes");
        payload.push(b'\n');
        std::fs::write(&training_path, payload).expect("changed training record");

        assert_eq!(
            admit(
                &candidate_record,
                &baseline_path,
                &report,
                None,
                &data_root,
                false,
            ),
            1
        );
        assert!(!data_root
            .join("lmml/models/successors/qwen38-successor-1/successor_manifest.json")
            .exists());
    }

    fn write_safetensors_fixture(root: &Path) {
        std::fs::write(
            root.join("config.json"),
            r#"{"model_type":"qwen3_5","architectures":["Qwen3_5ForConditionalGeneration"]}"#,
        )
        .expect("config");
        std::fs::write(root.join("tokenizer.json"), "tokenizer").expect("tokenizer");
        std::fs::write(
            root.join("model.safetensors.index.json"),
            r#"{"weight_map":{"layer.weight":"model-00001-of-00001.safetensors"}}"#,
        )
        .expect("index");
        let header = r#"{"layer.weight":{"dtype":"U8","shape":[4],"data_offsets":[0,4]}}"#;
        let mut shard = (header.len() as u64).to_le_bytes().to_vec();
        shard.extend(header.as_bytes());
        shard.extend([1, 2, 3, 4]);
        std::fs::write(root.join("model-00001-of-00001.safetensors"), shard).expect("shard");
    }
}
