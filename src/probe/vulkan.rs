//! Vulkan SDK detection.
//!
//! Checks if vulkaninfo is available and runs it to verify Vulkan support.

pub async fn detect() -> bool {
    // Prefer pkg-config check first
    if cfg!(target_os = "linux") {
        let pkg_config = tokio::process::Command::new("pkg-config")
            .args(["--exists", "vulkan"])
            .output()
            .await;
        if let Ok(output) = pkg_config {
            if output.status.success() {
                return true;
            }
        }
    }

    // Fallback: check if vulkaninfo exists and runs
    let vulkaninfo = tokio::process::Command::new("vulkaninfo").output().await;

    match vulkaninfo {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}
