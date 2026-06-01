use crate::build::BuildEvent;
use std::path::Path;
use tokio::sync::mpsc;

/// Ensure the llama.cpp repository exists at `path`.
/// Clones if missing, pulls if already present.
/// Emits [`BuildEvent::CommitHash`] after the operation.
pub async fn ensure_repo(path: &Path, tx: mpsc::Sender<BuildEvent>) -> Result<(), String> {
    let result = if path.join("CMakeLists.txt").exists() {
        git_pull(path, &tx).await
    } else {
        git_clone(path, &tx).await
    };

    // Capture commit hash regardless of whether we cloned or pulled
    if let Some(hash) = get_commit_hash(path).await {
        let _ = tx.send(BuildEvent::CommitHash(hash)).await;
    }

    result
}

/// Capture the current HEAD commit hash.
async fn get_commit_hash(path: &Path) -> Option<String> {
    let output = tokio::process::Command::new("git")
        .args([
            "-C",
            &path.to_string_lossy(),
            "rev-parse",
            "--short",
            "HEAD",
        ])
        .output()
        .await
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

async fn git_clone(path: &Path, tx: &mpsc::Sender<BuildEvent>) -> Result<(), String> {
    let _ = tx
        .send(BuildEvent::Line("Cloning llama.cpp...".into()))
        .await;

    let mut child = tokio::process::Command::new("git")
        .arg("clone")
        .arg("--depth")
        .arg("1")
        .arg("https://github.com/ggml-org/llama.cpp.git")
        .arg(path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to start git clone — is git installed?\n{e}"))?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let reader = tokio::io::BufReader::new(stdout);
    let err_reader = tokio::io::BufReader::new(stderr);

    let tx_clone = tx.clone();
    let read_stdout = tokio::spawn(async move {
        use tokio::io::AsyncBufReadExt;
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let truncated = if line.len() > 200 {
                format!("{}...", &line[..197])
            } else {
                line
            };
            let _ = tx_clone.send(BuildEvent::Line(truncated)).await;
        }
    });

    let tx_clone2 = tx.clone();
    let read_stderr = tokio::spawn(async move {
        use tokio::io::AsyncBufReadExt;
        let mut lines = err_reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let truncated = if line.len() > 200 {
                format!("{}...", &line[..197])
            } else {
                line
            };
            let _ = tx_clone2.send(BuildEvent::Line(truncated)).await;
        }
    });

    let status = child
        .wait()
        .await
        .map_err(|e| format!("git clone process error: {e}"))?;
    let _ = read_stdout.await;
    let _ = read_stderr.await;

    if status.success() {
        let _ = tx
            .send(BuildEvent::Line("✓ llama.cpp cloned successfully".into()))
            .await;
        Ok(())
    } else {
        Err("git clone failed — check your internet connection and try again.".to_string())
    }
}

async fn git_pull(path: &Path, tx: &mpsc::Sender<BuildEvent>) -> Result<(), String> {
    let _ = tx
        .send(BuildEvent::Line("Updating llama.cpp...".into()))
        .await;

    let output = tokio::process::Command::new("git")
        .args(["-C", &path.to_string_lossy(), "pull", "--ff-only"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("Failed to run git pull: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let truncated = if line.len() > 200 {
            format!("{}...", &line[..197])
        } else {
            line.to_string()
        };
        let _ = tx.send(BuildEvent::Line(truncated)).await;
    }

    if output.status.success() {
        let _ = tx
            .send(BuildEvent::Line("✓ llama.cpp updated".into()))
            .await;
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("git pull failed:\n{stderr}"))
    }
}
