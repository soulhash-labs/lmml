//! Successor training, merge, regression, and admission gates.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

#[derive(Debug, Deserialize)]
struct MergeEvidence {
    schema_version: u32,
    candidate_path: PathBuf,
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
    let training = match read_and_parse(
        options.training_manifest,
        lmml_substrate::parse_training_run_manifest_json,
    ) {
        Ok(training) => training,
        Err(error) => return fail("successor merge", error),
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
    if base_source == candidate_path {
        return fail(
            "successor merge",
            "candidate directory must not overwrite the canonical parent".to_string(),
        );
    }
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
    let evidence_candidate = match evidence.candidate_path.canonicalize() {
        Ok(path) => path,
        Err(error) => return fail("successor merge", error.to_string()),
    };
    if evidence_candidate != candidate_path {
        return fail(
            "successor merge",
            "merge evidence names a different candidate directory".to_string(),
        );
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
        adapter_artifact_id: training.adapter.artifact_id.clone(),
        candidate_path,
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
    if let Err(error) = lmml_substrate::validate_training_run_against_base(&training, &base)
        .and_then(|()| lmml_substrate::validate_candidate_against_training(&candidate, &training))
        .and_then(|()| {
            lmml_substrate::validate_successor_structure(&base, &candidate.candidate_manifest)
        })
        .and_then(|()| {
            lmml_substrate::verify_safetensors(
                &candidate.candidate_path,
                &candidate.candidate_manifest,
            )
        })
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
    let candidate = match read_and_parse(
        candidate_manifest,
        lmml_substrate::parse_successor_candidate_manifest_json,
    ) {
        Ok(candidate) => candidate,
        Err(error) => return fail("successor admission", error),
    };
    let baseline = match read_and_parse(
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
    if baseline.model_lineage_id != candidate.parent_lineage_id {
        return fail(
            "successor admission",
            "baseline does not belong to the candidate parent".to_string(),
        );
    }
    let baseline_cases: std::collections::BTreeSet<&str> = baseline
        .cases
        .iter()
        .map(|case| case.case_id.as_str())
        .collect();
    let regression_cases: std::collections::BTreeSet<&str> = successor
        .regression
        .iter()
        .map(|case| case.case_id.as_str())
        .collect();
    if baseline_cases != regression_cases {
        return fail(
            "successor admission",
            "regression cases must exactly match the frozen parent baseline".to_string(),
        );
    }
    if let Err(error) = lmml_substrate::validate_successor_admission(&successor, &candidate) {
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

fn read_and_parse<T>(
    path: &Path,
    parse: impl FnOnce(&str) -> Result<T, lmml_substrate::SubstrateError>,
) -> Result<T, String> {
    let payload =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    parse(&payload).map_err(|error| format!("{}: {error}", path.display()))
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
