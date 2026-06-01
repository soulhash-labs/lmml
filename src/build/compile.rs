use crate::build::BuildEvent;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// Build llama.cpp with the given cmake flags.
/// Sends progress events on `tx`.
/// If `cancel_flag` is set to true, the build is aborted mid-process.
pub async fn run_build(
    llama_cpp_path: &Path,
    flags: &[String],
    jobs: u32,
    tx: mpsc::Sender<BuildEvent>,
    cancel_flag: Option<Arc<AtomicBool>>,
) -> Result<(), String> {
    let build_dir = llama_cpp_path.join("build");

    // Phase 1: cmake configure
    let _ = tx
        .send(BuildEvent::Line("Configuring with cmake...".into()))
        .await;

    let mut cmake_args = vec!["-B".to_string(), build_dir.to_string_lossy().to_string()];
    cmake_args.extend_from_slice(flags);

    if cancel_flag
        .as_ref()
        .is_some_and(|f| f.load(Ordering::Relaxed))
    {
        return Err("Build cancelled".into());
    }
    run_command("cmake", &cmake_args, &tx, cancel_flag.as_deref()).await?;
    let _ = tx
        .send(BuildEvent::Line("✓ cmake configuration complete".into()))
        .await;

    // Phase 2: cmake build
    let effective_jobs = if jobs == 0 {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    } else {
        jobs as usize
    };

    let _ = tx
        .send(BuildEvent::Line(format!(
            "Building with {effective_jobs} jobs..."
        )))
        .await;

    let build_args = vec![
        "--build".to_string(),
        build_dir.to_string_lossy().to_string(),
        "--config".to_string(),
        "Release".to_string(),
        "-j".to_string(),
        effective_jobs.to_string(),
    ];

    if cancel_flag
        .as_ref()
        .is_some_and(|f| f.load(Ordering::Relaxed))
    {
        return Err("Build cancelled".into());
    }
    run_command("cmake", &build_args, &tx, cancel_flag.as_deref()).await?;

    // Phase 3: verify
    let binary = llama_cpp_path.join("build").join("bin").join("llama-cli");
    let verify_output = tokio::process::Command::new(&binary)
        .arg("--version")
        .output()
        .await;

    match verify_output {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let _ = tx
                .send(BuildEvent::Line(format!("✓ Build verified: {version}")))
                .await;
            link_binaries(llama_cpp_path, &tx).await?;
            let _ = tx.send(BuildEvent::Complete(Ok(()))).await;
            Ok(())
        }
        _ => {
            let msg = format!(
                "Build completed but {} failed to run — binary may be corrupted or incompatible.",
                binary.display()
            );
            let _ = tx.send(BuildEvent::Complete(Err(msg.clone()))).await;
            Err(msg)
        }
    }
}

async fn link_binaries(llama_cpp_path: &Path, tx: &mpsc::Sender<BuildEvent>) -> Result<(), String> {
    let Some(parent) = llama_cpp_path.parent() else {
        return Ok(());
    };
    let link_dir = parent.join("bin");
    tokio::fs::create_dir_all(&link_dir)
        .await
        .map_err(|e| format!("Failed to create {}: {e}", link_dir.display()))?;

    for name in ["llama-cli", "llama-server"] {
        let target = llama_cpp_path.join("build").join("bin").join(name);
        if !target.exists() {
            continue;
        }
        let link = link_dir.join(name);
        if link.exists() || link.symlink_metadata().is_ok() {
            tokio::fs::remove_file(&link)
                .await
                .map_err(|e| format!("Failed to replace {}: {e}", link.display()))?;
        }
        create_link_or_copy(&target, &link).await?;
        let _ = tx
            .send(BuildEvent::Line(format!(
                "✓ Linked {} -> {}",
                link.display(),
                target.display()
            )))
            .await;
    }

    Ok(())
}

#[cfg(unix)]
async fn create_link_or_copy(target: &Path, link: &Path) -> Result<(), String> {
    let target = target.to_path_buf();
    let link = link.to_path_buf();
    let link_for_error = link.clone();
    tokio::task::spawn_blocking(move || std::os::unix::fs::symlink(&target, &link))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("Failed to symlink {}: {e}", link_for_error.display()))
}

#[cfg(not(unix))]
async fn create_link_or_copy(target: &Path, link: &Path) -> Result<(), String> {
    tokio::fs::copy(target, link)
        .await
        .map(|_| ())
        .map_err(|e| format!("Failed to copy {}: {e}", link.display()))
}

/// Run a command and stream its output line by line.
/// If a cancel flag is provided and flips true mid-run, the child process is killed.
async fn run_command(
    program: &str,
    args: &[String],
    tx: &mpsc::Sender<BuildEvent>,
    cancel_flag: Option<&AtomicBool>,
) -> Result<(), String> {
    let mut child = tokio::process::Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to start {program} — is it installed?\n{e}"))?;

    let pid = child.id().unwrap_or(0);

    // Take pipes before moving child into shared handle
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    // Wrap child in Arc<Mutex> so it can be shared between wait and cancel tasks
    let child = Arc::new(Mutex::new(child));

    let tx_out = tx.clone();
    let read_stdout = tokio::spawn(async move {
        use tokio::io::AsyncBufReadExt;
        let reader = tokio::io::BufReader::new(stdout);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let truncated = if line.len() > 200 {
                format!("{}...", &line[..197])
            } else {
                line.clone()
            };
            let _ = tx_out.send(BuildEvent::Line(truncated)).await;
            if let Some((cur, tot)) = parse_progress(&line) {
                let _ = tx_out
                    .send(BuildEvent::Progress {
                        current: cur,
                        total: tot,
                    })
                    .await;
            }
        }
    });

    let tx_err = tx.clone();
    let read_stderr = tokio::spawn(async move {
        use tokio::io::AsyncBufReadExt;
        let reader = tokio::io::BufReader::new(stderr);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let truncated = if line.len() > 200 {
                format!("{}...", &line[..197])
            } else {
                line.clone()
            };
            let _ = tx_err.send(BuildEvent::Line(truncated)).await;
        }
    });

    // Wait for the child to exit, polling the cancel flag every 500ms
    let status = if let Some(flag) = cancel_flag {
        loop {
            let mut guard = child.lock().await;
            tokio::select! {
                result = guard.wait() => {
                    drop(guard);
                    if let Ok(status) = result {
                        break Some(status);
                    }
                    break None;
                }
                _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {
                    drop(guard);
                    if flag.load(Ordering::Relaxed) {
                        let mut guard = child.lock().await;
                        let _ = guard.kill().await;
                        let _ = guard.wait().await;
                        break None;
                    }
                }
            }
        }
    } else {
        child.lock().await.wait().await.ok()
    };

    let _ = read_stdout.await;
    let _ = read_stderr.await;

    match status {
        Some(exit) if exit.success() => Ok(()),
        Some(_) => Err(format!("{program} (pid {pid}) failed")),
        None => {
            let _ = tx
                .send(BuildEvent::Line("Build cancelled by user".into()))
                .await;
            Err("Build cancelled".into())
        }
    }
}

/// Parse cmake progress like `[ 27/342]` or `[342/342]` from a line.
fn parse_progress(line: &str) -> Option<(u32, u32)> {
    let line = line.trim();
    if !line.starts_with('[') {
        return None;
    }
    let end = line.find(']')?;
    let inner = &line[1..end].trim();
    let slash = inner.find('/')?;
    let current = inner[..slash].trim().parse().ok()?;
    let total = inner[slash + 1..].trim().parse().ok()?;
    Some((current, total))
}
