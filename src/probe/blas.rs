use super::BlasProbe;

const OPENBLAS_PC: &str = "openblas";
const MKL_PC: &str = "mkl-static-lp64";

pub async fn detect() -> BlasProbe {
    // Try OpenBLAS first
    if pkg_config_exists(OPENBLAS_PC) {
        let version = pkg_config_version(OPENBLAS_PC).unwrap_or_default();
        return BlasProbe::Found {
            library: "OpenBLAS".into(),
            version,
        };
    }

    // Try Intel MKL
    if pkg_config_exists(MKL_PC) {
        let version = pkg_config_version(MKL_PC).unwrap_or_default();
        return BlasProbe::Found {
            library: "Intel MKL".into(),
            version,
        };
    }

    BlasProbe::NotFound
}

fn pkg_config_exists(package: &str) -> bool {
    std::process::Command::new("pkg-config")
        .arg("--exists")
        .arg(package)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn pkg_config_version(package: &str) -> Option<String> {
    let output = std::process::Command::new("pkg-config")
        .arg("--modversion")
        .arg(package)
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}
