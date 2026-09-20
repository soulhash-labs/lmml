//! Runtime-flavor verification after a llama.cpp build.
//!
//! The checks in this module bind the resulting executable to the selected
//! trusted source flavor and reject mixed shared-library installations before
//! the build is recorded as usable.

use std::path::{Path, PathBuf};

use crate::{BuildConfig, BuildError};

pub(super) async fn verify_runtime(config: &BuildConfig, server: &Path) -> Result<(), BuildError> {
    verify_flavor_source(config).await?;
    verify_linux_dependencies(config, server).await
}

async fn verify_flavor_source(config: &BuildConfig) -> Result<(), BuildError> {
    if config.flavor != lmml_compat::LlamaRuntimeFlavor::Prism {
        return Ok(());
    }

    let header = config.source_dir.join("ggml/include/ggml.h");
    let source = tokio::fs::read_to_string(&header).await.map_err(|error| {
        BuildError::Verification(format!(
            "failed to inspect Prism tensor registry {}: {error}",
            header.display()
        ))
    })?;
    let missing = ["GGML_TYPE_PQ2_0", "GGML_TYPE_PTQ1_0"]
        .into_iter()
        .filter(|marker| !source.contains(marker))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(BuildError::Verification(format!(
            "Prism source at {} lacks required tensor types: {}",
            config.source_dir.display(),
            missing.join(", ")
        )))
    }
}

#[cfg(target_os = "linux")]
async fn verify_linux_dependencies(config: &BuildConfig, server: &Path) -> Result<(), BuildError> {
    let output = tokio::process::Command::new("ldd")
        .arg(server)
        .output()
        .await
        .map_err(|error| {
            BuildError::Verification(format!(
                "failed to inspect shared libraries for {}: {error}",
                server.display()
            ))
        })?;
    if !output.status.success() {
        return Err(BuildError::Verification(format!(
            "ldd failed for {}",
            server.display()
        )));
    }

    let build_root = canonical_path(&config.source_dir.join("build"))?;
    let dependencies = String::from_utf8_lossy(&output.stdout);
    for line in dependencies.lines().filter(|line| {
        ["libggml", "libllama", "libmtmd"]
            .iter()
            .any(|library| line.contains(library))
    }) {
        if line.contains("not found") {
            return Err(BuildError::Verification(format!(
                "runtime dependency is unresolved: {}",
                line.trim()
            )));
        }
        let Some(path) = dependency_path(line) else {
            continue;
        };
        let resolved = canonical_path(&path)?;
        if !resolved.starts_with(&build_root) {
            return Err(BuildError::Verification(format!(
                "{} runtime would load {} outside its isolated build {}",
                config.flavor,
                resolved.display(),
                build_root.display()
            )));
        }
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
async fn verify_linux_dependencies(
    _config: &BuildConfig,
    _server: &Path,
) -> Result<(), BuildError> {
    Ok(())
}

#[cfg(target_os = "linux")]
fn dependency_path(line: &str) -> Option<PathBuf> {
    let candidate = line
        .split_once("=>")
        .map(|(_, right)| right.trim())
        .unwrap_or_else(|| line.trim())
        .split_whitespace()
        .next()?;
    candidate.starts_with('/').then(|| PathBuf::from(candidate))
}

#[cfg(target_os = "linux")]
fn canonical_path(path: &Path) -> Result<PathBuf, BuildError> {
    path.canonicalize().map_err(|error| {
        BuildError::Verification(format!(
            "failed to resolve runtime path {}: {error}",
            path.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    #[test]
    fn parses_linked_dependency_paths() {
        assert_eq!(
            super::dependency_path("libllama.so => /tmp/prism/build/lib/libllama.so (0x01)"),
            Some(std::path::PathBuf::from("/tmp/prism/build/lib/libllama.so"))
        );
        assert_eq!(super::dependency_path("libggml.so => not found"), None);
    }
}
