use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

struct RecordingAdmission {
    calls: AtomicUsize,
}

impl GgufAdmissionProvider for RecordingAdmission {
    async fn admit(&self, artifact: &Path, server: &Path) -> Result<(), String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if !server.is_file() {
            return Err("test server missing".into());
        }
        lmml_substrate::validate_gguf(artifact).map_err(|error| error.to_string())?;
        let metadata = lmml_models::parse_gguf_metadata(artifact)
            .await
            .map_err(|error| error.to_string())?;
        if metadata.tensor_count != 1 || metadata.architecture.as_deref() != Some("llama") {
            return Err("test admission received incomplete GGUF metadata".into());
        }
        Ok(())
    }
}

#[test]
fn publication_refuses_existing_destination() {
    let dir = tempfile::tempdir().expect("tempdir");
    let source = dir.path().join("source.gguf");
    let destination = dir.path().join("destination.gguf");
    std::fs::write(&source, b"source").expect("source");
    std::fs::write(&destination, b"existing").expect("destination");

    let error = publish_no_clobber(&source, &destination).expect_err("conflict");
    assert!(error.contains("already exists"));
    assert_eq!(
        std::fs::read(&destination).expect("read destination"),
        b"existing"
    );
}

#[test]
fn source_anchor_reuse_rejects_invalid_timestamp() {
    let dir = tempfile::tempdir().expect("tempdir");
    let digest = lmml_substrate::Hash256::parse("a".repeat(64)).expect("hash");
    let expected = artifact_manifest(
        "qwen38-27b-safetensors",
        "qwen38-27b",
        lmml_substrate::ModelRepresentation::Safetensors,
        None,
        digest.clone(),
        None,
        "lmml-canonical",
        "2",
        vec![digest],
        vec!["canonical_manifest".into()],
        dir.path(),
        None,
    )
    .expect("manifest");
    std::fs::create_dir_all(dir.path()).expect("artifact root");
    let mut corrupted = expected.clone();
    corrupted.created_at = "not-rfc3339".into();
    std::fs::write(
        dir.path().join("qwen38-27b-safetensors.json"),
        serde_json::to_vec_pretty(&corrupted).expect("serialize"),
    )
    .expect("corrupt anchor");

    assert!(matches!(
        ensure_source_artifact(dir.path(), &expected),
        Err(lmml_substrate::SubstrateError::InvalidArtifactManifest(_))
    ));
}

#[tokio::test]
async fn streamed_process_reports_success_and_failure_tail() {
    let success = run_streamed(
        "sh",
        &["-c".into(), "printf 'converted\\n'".into()],
        "[test]",
    )
    .await
    .expect("success process");
    assert!(success.success);
    assert_eq!(success.tail, vec!["converted"]);

    let failure = run_streamed(
        "sh",
        &["-c".into(), "printf 'failed\\n' >&2; exit 7".into()],
        "[test]",
    )
    .await
    .expect("failure process");
    assert!(!failure.success);
    assert_eq!(failure.tail, vec!["failed"]);
}

#[cfg(unix)]
#[tokio::test]
async fn quantizer_preflight_rejects_unadvertised_quantization() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("tempdir");
    let quantizer = dir.path().join("llama-quantize");
    std::fs::write(&quantizer, "#!/bin/sh\necho 'Q8_0 F16'\n").expect("quantizer");
    let mut permissions = std::fs::metadata(&quantizer)
        .expect("metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&quantizer, permissions).expect("permissions");

    assert!(quantizer_supports(&quantizer, QuantizationArg::Q8_0).await);
    assert!(!quantizer_supports(&quantizer, QuantizationArg::Q6K).await);
}

#[tokio::test]
async fn quantized_output_is_validated_published_and_registered_from_temp_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let intermediate = dir.path().join("converted.gguf");
    let output_temp = dir.path().join(".derived.gguf.tmp");
    let output = dir.path().join("derived-q8.gguf");
    let artifact_root = dir.path().join("artifacts");
    let mut valid_header = b"GGUF".to_vec();
    valid_header.extend(3_u32.to_le_bytes());
    valid_header.extend([0_u8; 16]);
    std::fs::write(&intermediate, valid_header).expect("intermediate");

    let quantizer_args = build_quantizer_args(&intermediate, &output_temp, QuantizationArg::Q8_0);
    assert_eq!(quantizer_args[1], output_temp.to_string_lossy());
    assert_ne!(quantizer_args[1], output.to_string_lossy());
    let fake_quantizer_args = vec![
        "-c".into(),
        "cp \"$1\" \"$2\"".into(),
        "fake-quantizer".into(),
        quantizer_args[0].clone(),
        quantizer_args[1].clone(),
        quantizer_args[2].clone(),
    ];
    let result = run_streamed("sh", &fake_quantizer_args, "[fake quantizer]")
        .await
        .expect("fake quantizer");
    assert!(result.success);

    lmml_substrate::validate_gguf(&output_temp).expect("temporary GGUF validates");
    let hash = lmml_substrate::sha256_file(&output_temp).expect("hash");
    publish_no_clobber(&output_temp, &output).expect("publish");
    assert!(output.is_file());
    assert!(!output_temp.exists());

    let manifest = artifact_manifest(
        "qwen38-q8-test",
        "qwen38-27b",
        lmml_substrate::ModelRepresentation::Gguf,
        Some(lmml_substrate::QuantizationKind::Q8_0),
        hash.clone(),
        None,
        "fake-quantizer",
        "test",
        vec![hash],
        quantizer_args,
        &output,
        Some(lmml_substrate::ArtifactAdmission {
            backend: "llama.cpp".into(),
            backend_version: "test".into(),
            checked_at: "2026-08-25T00:00:00Z".into(),
        }),
    )
    .expect("artifact manifest");
    let registered =
        lmml_substrate::append_artifact_manifest(&artifact_root, &manifest).expect("register");
    assert!(registered.is_file());
}

#[test]
fn successor_derivation_requires_immutable_admission() {
    let dir = tempfile::tempdir().expect("tempdir");
    let source = dir.path().join("successor");
    let data_root = dir.path().join("data");
    std::fs::create_dir_all(&source).expect("source");
    write_safetensors_fixture(&source);
    let successor =
        lmml_substrate::import_successor_safetensors(&source, "qwen38-successor-1", "qwen38-27b")
            .expect("successor import");

    assert!(matches!(
        require_admitted_successor(&successor, &data_root),
        Err(lmml_substrate::SubstrateError::SuccessorNotAdmitted(_))
    ));

    let admission = lmml_substrate::SuccessorManifest {
        schema_version: lmml_substrate::SCHEMA_VERSION,
        successor_lineage_id: "qwen38-successor-1".into(),
        parent_lineage_id: "qwen38-27b".into(),
        candidate_id: "candidate-1".into(),
        training_run_id: "train-1".into(),
        successor_hash: successor.model.canonical_manifest_hash.clone(),
        regression: vec![lmml_substrate::RegressionResult {
            case_id: "anchor-1".into(),
            parent_metric: 1.0,
            candidate_metric: 1.0,
            delta: 0.0,
            maximum_degradation: 0.1,
            passed: true,
        }],
        admitted_at: "2026-08-25T00:00:00Z".into(),
    };
    let path = data_root.join("lmml/models/successors/qwen38-successor-1/successor_manifest.json");
    lmml_substrate::store_successor_manifest(&path, &admission).expect("store admission");
    require_admitted_successor(&successor, &data_root).expect("admitted successor");
}

#[cfg(unix)]
#[tokio::test]
async fn complete_quantized_derive_publishes_registers_and_rolls_back_conflicts() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("tempdir");
    let source = dir.path().join("source");
    let tools = dir.path().join("tools");
    let data_root = dir.path().join("data");
    std::fs::create_dir_all(&source).expect("source");
    std::fs::create_dir_all(&tools).expect("tools");
    write_safetensors_fixture(&source);
    let substrate =
        lmml_substrate::import_safetensors(&source, "qwen38-27b").expect("import substrate");
    let manifest_path = dir.path().join("manifest.json");
    std::fs::write(
        &manifest_path,
        lmml_substrate::manifest_json(&substrate).expect("manifest JSON"),
    )
    .expect("manifest");
    let gguf_fixture = dir.path().join("fixture.gguf");
    std::fs::write(&gguf_fixture, fixture_gguf()).expect("GGUF fixture");

    let converter = tools.join("convert_hf_to_gguf.py");
    std::fs::write(
        &converter,
        format!("#!/bin/sh\ncp '{}' \"$3\"\n", gguf_fixture.display()),
    )
    .expect("converter");
    let quantizer = tools.join("llama-quantize");
    std::fs::write(
        &quantizer,
        "#!/bin/sh\nif [ \"${1:-}\" = --version ]; then echo fake-quantizer-1; exit 0; fi\nif [ \"${1:-}\" = --help ]; then echo Q8_0; exit 0; fi\ncp \"$1\" \"$2\"\n",
    )
    .expect("quantizer");
    let server = tools.join("llama-server");
    std::fs::write(&server, "#!/bin/sh\necho fake-server-1\n").expect("server");
    for executable in [&quantizer, &server] {
        let mut permissions = std::fs::metadata(executable)
            .expect("metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(executable, permissions).expect("permissions");
    }

    let admission = RecordingAdmission {
        calls: AtomicUsize::new(0),
    };
    let output = dir.path().join("qwen38-q8.gguf");
    let result = derive_model_with(
        &manifest_path,
        &source,
        &output,
        "qwen38-q8-test",
        QuantizationArg::Q8_0,
        Some(&converter),
        Some(&quantizer),
        Some(&server),
        "/bin/sh",
        None,
        None,
        false,
        &data_root,
        &admission,
    )
    .await;
    assert_eq!(result, 0);
    assert!(output.is_file());
    assert!(data_root
        .join("lmml/models/artifacts/qwen38-27b-safetensors.json")
        .is_file());
    assert!(data_root
        .join("lmml/models/artifacts/qwen38-q8-test.json")
        .is_file());

    let conflicting_output = dir.path().join("qwen38-q8-conflict.gguf");
    let conflict = derive_model_with(
        &manifest_path,
        &source,
        &conflicting_output,
        "qwen38-q8-test",
        QuantizationArg::Q8_0,
        Some(&converter),
        Some(&quantizer),
        Some(&server),
        "/bin/sh",
        None,
        None,
        false,
        &data_root,
        &admission,
    )
    .await;
    assert_eq!(conflict, 1);
    assert!(!conflicting_output.exists());
    assert_eq!(admission.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn rocm_converter_command_mounts_source_and_output() {
    let dir = tempfile::tempdir().expect("tempdir");
    let converter_root = dir.path().join("llama.cpp");
    let source = dir.path().join("source");
    let output = dir.path().join("output.gguf");
    std::fs::create_dir_all(&converter_root).expect("converter root");
    std::fs::create_dir_all(&source).expect("source");
    let converter = converter_root.join("convert_hf_to_gguf.py");
    std::fs::write(&converter, b"#!/usr/bin/env python3\n").expect("converter");
    let command = build_converter_command(
        "python3",
        &converter,
        &source,
        &output,
        QuantizationArg::F16,
        Some(Path::new("/usr/bin/docker")),
        Some("rocm/pytorch:test"),
    )
    .expect("container command");
    assert_eq!(command.0, PathBuf::from("/usr/bin/docker"));
    assert!(!command.1.iter().any(|arg| arg.starts_with("--device=")));
    assert!(command.1.iter().any(|arg| arg.contains("/lmml/source:ro")));
    assert!(command.1.iter().any(|arg| arg.contains("/lmml/output")));
    assert!(command
        .1
        .iter()
        .any(|arg| arg == "/lmml/output/output.gguf"));
}

#[test]
fn rocm_preflight_imports_and_reports_transformers() {
    assert!(container::CONVERTER_PREFLIGHT
        .contains("import sys, torch, numpy, safetensors, transformers, gguf"));
    assert!(container::CONVERTER_PREFLIGHT.contains("';transformers='"));
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

fn fixture_gguf() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"GGUF");
    bytes.extend_from_slice(&3_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u64.to_le_bytes());
    bytes.extend_from_slice(&1_u64.to_le_bytes());
    write_gguf_string(&mut bytes, "general.architecture");
    bytes.extend_from_slice(&8_u32.to_le_bytes());
    write_gguf_string(&mut bytes, "llama");
    write_gguf_string(&mut bytes, "blk.0.weight");
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u64.to_le_bytes());
    bytes.extend_from_slice(&8_u32.to_le_bytes());
    bytes.extend_from_slice(&0_u64.to_le_bytes());
    bytes
}

fn write_gguf_string(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
    bytes.extend_from_slice(value.as_bytes());
}
