use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use super::*;
use crate::{
    import_safetensors, import_successor_safetensors, inspect_safetensors_file, sha256_data,
    sha256_file, validate_adapter_safetensors, ArtifactAdmission, ArtifactIdentity, BaselineCase,
    Hash256, MergeDelta, QuantizationKind, RegressionResult, TrainingAuthorizationManifest,
    SCHEMA_VERSION,
};

const TIMESTAMP: &str = "2026-08-25T00:00:00Z";

fn hash(character: char) -> Hash256 {
    Hash256::parse(character.to_string().repeat(64)).expect("test hash")
}

fn write_tensor(path: &Path, name: &str, dtype: &str, shape: &[u64], value: &[u8]) {
    let dimensions = shape
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let header = format!(
        "{{\"{name}\":{{\"dtype\":\"{dtype}\",\"shape\":[{dimensions}],\"data_offsets\":[0,{}]}}}}",
        value.len()
    );
    let mut bytes = (header.len() as u64).to_le_bytes().to_vec();
    bytes.extend(header.as_bytes());
    bytes.extend(value);
    fs::write(path, bytes).expect("write Safetensors");
}

fn checkpoint(lineage_id: &str, parent: Option<&str>) -> (tempfile::TempDir, SubstrateManifest) {
    let directory = tempfile::tempdir().expect("checkpoint");
    fs::write(
        directory.path().join("config.json"),
        r#"{"model_type":"qwen3_5","architectures":["Qwen3_5ForConditionalGeneration"]}"#,
    )
    .expect("config");
    fs::write(directory.path().join("tokenizer.json"), "tokenizer").expect("tokenizer");
    fs::write(
        directory.path().join("model.safetensors.index.json"),
        r#"{"weight_map":{"layer.weight":"model-00001-of-00001.safetensors"}}"#,
    )
    .expect("index");
    write_tensor(
        &directory.path().join("model-00001-of-00001.safetensors"),
        "layer.weight",
        "U8",
        &[4],
        &[1, 2, 3, 4],
    );
    let manifest = match parent {
        Some(parent) => import_successor_safetensors(directory.path(), lineage_id, parent),
        None => import_safetensors(directory.path(), lineage_id),
    }
    .expect("import checkpoint");
    (directory, manifest)
}

fn adapter(directory: &Path, value: f32) -> (std::path::PathBuf, Hash256) {
    let path = directory.join("adapter_model.safetensors");
    write_tensor(
        &path,
        "base_model.model.layers.0.self_attn.q_proj.lora_A.weight",
        "F32",
        &[1],
        &value.to_le_bytes(),
    );
    let digest = sha256_file(&path).expect("adapter hash");
    (path, digest)
}

fn training_run(
    base: &SubstrateManifest,
    adapter_path: &Path,
    adapter_hash: Hash256,
) -> TrainingRunManifest {
    let authorization = training_authorization(base);
    TrainingRunManifest {
        schema_version: SCHEMA_VERSION,
        training_run_id: "train-1".into(),
        authorization_id: authorization.authorization_id.clone(),
        authorization_hash: training_authorization_hash(&authorization)
            .expect("authorization hash"),
        base_lineage_id: base.model.lineage_id.clone(),
        base_manifest_hash: base.model.canonical_manifest_hash.clone(),
        config_hash: base.config_hash.clone(),
        tokenizer_hash: base.tokenizer_hash.clone(),
        dataset_hash: hash('d'),
        seed: 42,
        training_config: BTreeMap::new(),
        approved_allowlist: vec!["model.layers.*".into()],
        trainable_parameters: vec!["model.layers.0.self_attn.q_proj.lora_A".into()],
        final_loss: 1.0,
        gradient_norms: vec![0.5],
        adapter: ArtifactIdentity {
            artifact_id: "adapter-1".into(),
            model_lineage_id: base.model.lineage_id.clone(),
            representation: ModelRepresentation::Safetensors,
            quantization: None,
            artifact_hash: adapter_hash,
        },
        adapter_path: adapter_path.to_path_buf(),
        adapter_tensors: inspect_safetensors_file(adapter_path).expect("adapter tensors"),
        created_at: TIMESTAMP.into(),
    }
}

fn training_authorization(base: &SubstrateManifest) -> TrainingAuthorizationManifest {
    TrainingAuthorizationManifest {
        schema_version: SCHEMA_VERSION,
        authorization_id: "authorize-1".into(),
        base_lineage_id: base.model.lineage_id.clone(),
        base_manifest_hash: base.model.canonical_manifest_hash.clone(),
        config_hash: base.config_hash.clone(),
        tokenizer_hash: base.tokenizer_hash.clone(),
        dataset_hash: hash('d'),
        seed: 42,
        training_config: BTreeMap::new(),
        approved_allowlist: vec!["model.layers.*".into()],
        trainable_parameters: vec!["model.layers.0.self_attn.q_proj.lora_A".into()],
        created_at: TIMESTAMP.into(),
    }
}

fn admitted_artifact(path: &Path) -> ArtifactManifest {
    let artifact_hash = sha256_file(path).expect("artifact hash");
    ArtifactManifest {
        schema_version: SCHEMA_VERSION,
        artifact: ArtifactIdentity {
            artifact_id: "qwen38-q8".into(),
            model_lineage_id: "qwen38-27b".into(),
            representation: ModelRepresentation::Gguf,
            quantization: Some(QuantizationKind::Q8_0),
            artifact_hash: artifact_hash.clone(),
        },
        parent_artifact: Some("qwen38-27b-safetensors".into()),
        canonical_model: "qwen38-27b".into(),
        tool: "llama.cpp".into(),
        tool_version: "test".into(),
        command_or_parameters: vec!["Q8_0".into()],
        source_hashes: vec![hash('a')],
        output_hash: artifact_hash,
        artifact_path: path.to_path_buf(),
        admission: Some(ArtifactAdmission {
            backend: "llama.cpp".into(),
            backend_version: "test".into(),
            checked_at: TIMESTAMP.into(),
        }),
        created_at: TIMESTAMP.into(),
    }
}

#[test]
fn unexpected_trainable_parameter_is_rejected() {
    let (_directory, base) = checkpoint("qwen38-27b", None);
    let scratch = tempfile::tempdir().expect("scratch");
    let (adapter_path, adapter_hash) = adapter(scratch.path(), 1.0);
    let mut run = training_run(&base, &adapter_path, adapter_hash);
    run.trainable_parameters = vec!["model.embed_tokens.weight".into()];

    assert!(matches!(
        validate_training_run_against_base(&run, &base),
        Err(SubstrateError::UnexpectedTrainableParameter(_))
    ));
}

#[test]
fn completed_training_must_match_preoptimization_authorization() {
    let (_directory, base) = checkpoint("qwen38-27b", None);
    let scratch = tempfile::tempdir().expect("scratch");
    let (adapter_path, adapter_hash) = adapter(scratch.path(), 1.0);
    let mut run = training_run(&base, &adapter_path, adapter_hash);
    let authorization = training_authorization(&base);
    run.seed += 1;

    assert!(matches!(
        validate_training_run_against_authorization(&run, &authorization),
        Err(SubstrateError::InvalidLifecycleManifest(_))
    ));
}

#[test]
fn non_finite_adapter_is_rejected() {
    let (_directory, base) = checkpoint("qwen38-27b", None);
    let scratch = tempfile::tempdir().expect("scratch");
    let (adapter_path, adapter_hash) = adapter(scratch.path(), f32::NAN);
    let run = training_run(&base, &adapter_path, adapter_hash);

    assert!(matches!(
        validate_training_run_against_base(&run, &base),
        Err(SubstrateError::NonFiniteTrainingState(_))
    ));
}

#[test]
fn full_base_tensor_cannot_masquerade_as_adapter() {
    let scratch = tempfile::tempdir().expect("scratch");
    let path = scratch.path().join("adapter_model.safetensors");
    write_tensor(
        &path,
        "model.layers.0.self_attn.q_proj.weight",
        "F32",
        &[1],
        &1.0_f32.to_le_bytes(),
    );
    let tensors = inspect_safetensors_file(&path).expect("tensor inventory");

    assert!(matches!(
        validate_adapter_safetensors(&path, &tensors),
        Err(SubstrateError::InvalidLifecycleManifest(_))
    ));
}

#[test]
fn no_op_merge_is_rejected() {
    let (_parent_directory, _parent) = checkpoint("qwen38-27b", None);
    let (candidate_directory, candidate_manifest) =
        checkpoint("qwen38-successor-1", Some("qwen38-27b"));
    let candidate = SuccessorCandidateManifest {
        schema_version: SCHEMA_VERSION,
        candidate_id: "candidate-1".into(),
        parent_lineage_id: "qwen38-27b".into(),
        training_run_id: "train-1".into(),
        adapter_artifact_id: "adapter-1".into(),
        candidate_path: candidate_directory.path().to_path_buf(),
        requested_dtype: "bfloat16".into(),
        effective_load_dtype: "bfloat16".into(),
        merge_dtype: "bfloat16".into(),
        output_dtype: "bfloat16".into(),
        candidate_manifest,
        delta: MergeDelta {
            changed_tensor_count: 0,
            unchanged_tensor_count: 1,
            max_absolute_delta: 0.0,
            aggregate_norm_delta: 0.0,
        },
        equivalence_max_absolute_delta: 0.0,
        equivalence_tolerance: 1e-4,
        created_at: TIMESTAMP.into(),
    };

    assert!(matches!(
        parse_successor_candidate_manifest_json(
            &serde_json::to_string(&candidate).expect("serialize")
        ),
        Err(SubstrateError::NoOpMerge)
    ));
}

#[test]
fn successor_requires_distinct_explicit_parent() {
    let successor = SuccessorManifest {
        schema_version: SCHEMA_VERSION,
        successor_lineage_id: "qwen38-27b".into(),
        parent_lineage_id: "qwen38-27b".into(),
        candidate_id: "candidate-1".into(),
        training_run_id: "train-1".into(),
        successor_hash: hash('a'),
        regression: vec![RegressionResult {
            case_id: "anchor-1".into(),
            parent_metric: 1.0,
            candidate_metric: 1.0,
            delta: 0.0,
            maximum_degradation: 0.1,
            passed: true,
        }],
        admitted_at: TIMESTAMP.into(),
    };

    assert!(matches!(
        parse_successor_manifest_json(&serde_json::to_string(&successor).expect("serialize")),
        Err(SubstrateError::InvalidLifecycleManifest(_))
    ));
}

#[test]
fn runtime_lease_uses_exact_admitted_artifact_hash() {
    let directory = tempfile::tempdir().expect("artifact");
    let artifact_path = directory.path().join("model.gguf");
    fs::write(&artifact_path, b"GGUF-runtime-test").expect("GGUF");
    let artifact = admitted_artifact(&artifact_path);
    let runtime = RuntimeManifest {
        runtime_id: "runtime-1".into(),
        pid: 1234,
        model_lineage_id: "qwen38-27b".into(),
        artifact_id: artifact.artifact.artifact_id.clone(),
        artifact_hash: artifact.artifact.artifact_hash.clone(),
        representation: ModelRepresentation::Gguf,
        quantization: Some(QuantizationKind::Q8_0),
        backend: "llama.cpp".into(),
        backend_version: "test".into(),
        endpoint: "http://127.0.0.1:1200".into(),
        context_size: 16_384,
        capabilities: vec![RuntimeCapability::TextGeneration],
        created_at: TIMESTAMP.into(),
    };
    let request = ModelRequest {
        model_lineage_id: "qwen38-27b".into(),
        purpose: "baseline".into(),
        representation_preference: vec![ModelRepresentation::Gguf],
        required_capabilities: vec![RuntimeCapability::TextGeneration],
    };

    let lease = issue_model_lease(
        &request,
        std::slice::from_ref(&artifact),
        &[runtime],
        "lease-1",
    )
    .expect("lease");
    assert_eq!(lease.artifact_hash, artifact.artifact.artifact_hash);
    assert_eq!(lease.artifact_id, artifact.artifact.artifact_id);
}

#[test]
fn llama_cpp_runtime_cannot_claim_eleven_taps() {
    let runtime = RuntimeManifest {
        runtime_id: "runtime-1".into(),
        pid: 1234,
        model_lineage_id: "qwen38-27b".into(),
        artifact_id: "qwen38-q8".into(),
        artifact_hash: hash('a'),
        representation: ModelRepresentation::Gguf,
        quantization: Some(QuantizationKind::Q8_0),
        backend: "llama.cpp".into(),
        backend_version: "test".into(),
        endpoint: "http://127.0.0.1:1200".into(),
        context_size: 16_384,
        capabilities: vec![RuntimeCapability::ElevenTapObservation],
        created_at: TIMESTAMP.into(),
    };

    assert!(matches!(
        parse_runtime_manifest_json(&serde_json::to_string(&runtime).expect("serialize")),
        Err(SubstrateError::RuntimeCapabilityUnsupported(_))
    ));
}

#[test]
fn baseline_persistence_is_append_only_and_hash_checked() {
    let directory = tempfile::tempdir().expect("baseline");
    let output = "Rome".to_string();
    let baseline = BaselineManifest {
        schema_version: SCHEMA_VERSION,
        baseline_id: "pristine-1".into(),
        model_lineage_id: "qwen38-27b".into(),
        artifact_id: "qwen38-q8".into(),
        artifact_hash: hash('a'),
        backend: "llama.cpp".into(),
        backend_version: "test".into(),
        parameters: vec!["--temp".into(), "0".into()],
        cases: vec![BaselineCase {
            case_id: "capital".into(),
            prompt: "Capital of Italy?".into(),
            output_hash: sha256_data(output.as_bytes()),
            output,
        }],
        created_at: TIMESTAMP.into(),
    };
    let path = directory.path().join("manifest.json");
    store_baseline_manifest(&path, &baseline).expect("store baseline");
    store_baseline_manifest(&path, &baseline).expect("idempotent baseline");

    let mut corrupted = baseline;
    corrupted.cases[0].output.push('!');
    assert!(matches!(
        store_baseline_manifest(directory.path().join("bad.json"), &corrupted),
        Err(SubstrateError::InvalidLifecycleManifest(_))
    ));
}
