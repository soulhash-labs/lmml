//! Artifact catalog inspection for scriptable lifecycle clients.

use std::path::Path;

pub(super) fn list(lineage_id: Option<&str>, data_root: &Path, json: bool) -> i32 {
    let root = data_root.join("lmml/models/artifacts");
    let mut artifacts = match lmml_substrate::load_artifact_manifests(&root) {
        Ok(artifacts) => artifacts,
        Err(error) => {
            eprintln!("model artifacts failed: {error}");
            return 1;
        }
    };
    if let Some(lineage_id) = lineage_id {
        artifacts.retain(|manifest| manifest.artifact.model_lineage_id == lineage_id);
    }
    if json {
        match serde_json::to_string_pretty(&artifacts) {
            Ok(payload) => println!("{payload}"),
            Err(error) => {
                eprintln!("could not serialize artifact catalog: {error}");
                return 1;
            }
        }
    } else if artifacts.is_empty() {
        println!("no registered artifacts");
    } else {
        for manifest in artifacts {
            let quantization = manifest
                .artifact
                .quantization
                .map_or_else(|| "canonical".to_string(), |value| format!("{value:?}"));
            let status = if manifest.admission.is_some() {
                "admitted"
            } else {
                "source"
            };
            println!(
                "{}\t{}\t{}\t{}\t{}",
                manifest.artifact.artifact_id,
                manifest.artifact.model_lineage_id,
                quantization,
                status,
                manifest.artifact_path.display()
            );
        }
    }
    0
}
