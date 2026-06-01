//! OS and architecture detection.
//!
//! Returns the running OS type (Linux / macOS / Windows), architecture,
//! and kernel version string.

use super::OsInfo;

pub fn detect() -> OsInfo {
    let os_type = std::env::consts::OS.to_string();
    let arch = std::env::consts::ARCH.to_string();
    let kernel = detect_kernel_version(&os_type);

    OsInfo {
        os_type,
        arch,
        kernel,
    }
}

fn detect_kernel_version(os: &str) -> String {
    let output = match os {
        "linux" => std::process::Command::new("uname").arg("-r").output(),
        "macos" => std::process::Command::new("sw_vers")
            .arg("-productVersion")
            .output(),
        "windows" => std::process::Command::new("cmd")
            .args(["/c", "ver"])
            .output(),
        _ => return "unknown".to_string(),
    };

    match output {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
        _ => "unknown".to_string(),
    }
}
