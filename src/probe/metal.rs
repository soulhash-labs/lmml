//! macOS Metal detection.
//!
//! Metal is only available on macOS. Returns true only on macOS.

pub async fn detect() -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }

    // Metal is built into macOS — check macOS version >= 10.15 (Catalina)
    // which is where Metal 2+ is standard
    if let Ok(output) = tokio::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .await
    {
        if output.status.success() {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            // Simple version check: macOS 10.15+ or 11+ means Metal
            if version.starts_with("10.") {
                if let Some(minor) = version.split('.').nth(1) {
                    if let Ok(v) = minor.parse::<u32>() {
                        return v >= 15;
                    }
                }
            }
            if version.starts_with("11.")
                || version.starts_with("12.")
                || version.starts_with("13.")
                || version.starts_with("14.")
                || version.starts_with("15.")
            {
                return true;
            }
        }
    }

    // On macOS, Metal is always available on modern versions
    true
}
