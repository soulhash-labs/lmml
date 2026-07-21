//! Detect tab rendering.

use lmml_detect::{
    gpu_catalog, CmakeInfo, CompilerInfo, CudaCompatibility, GitInfo, GpuInfo, MissingPrerequisite,
    RocmSupport, SystemProfile, VulkanSupport,
};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::Frame;

use crate::app::App;

/// Render the hardware detection tab.
pub fn render(area: Rect, app: &App, frame: &mut Frame) {
    let left = system_lines(app);
    let right = if app.detect_log.is_empty() {
        vec![
            Line::from("Press d to scan hardware and prerequisites."),
            Line::from("Raw probe output and install hints will appear here."),
        ]
    } else {
        app.detect_log.iter().cloned().map(Line::from).collect()
    };
    super::render_two_pane(
        area,
        super::pane("System", left),
        super::pane("Probe Log", right),
        frame,
    );
}

fn system_lines(app: &App) -> Vec<Line<'static>> {
    let Some(profile) = &app.detect_profile else {
        if let Some(cached) = &app.state.system_profile {
            let mut lines = vec![
                Line::from("Cached detection summary"),
                Line::from(format!("GPUs: {}", cached.gpu_names.len())),
                Line::from(format!("sccache: {}", cached.sccache)),
            ];
            if !cached.gpu_archs.is_empty() {
                lines.push(Line::from(format!(
                    "CUDA archs: {}",
                    cached.gpu_archs.join(", ")
                )));
            }
            if cached.rocm_available {
                lines.push(Line::from(format!(
                    "ROCm targets: {}",
                    cached.rocm_targets.join(", ")
                )));
            }
            lines.push(Line::from(""));
            lines.push(Line::from("Press d to refresh full probe details."));
            return lines;
        }
        return vec![
            Line::from("No system scan yet."),
            Line::from("Press d to detect hardware and build prerequisites."),
        ];
    };

    let mut lines = vec![
        compiler_line(profile.compiler.as_ref()),
        cmake_line(profile.cmake.as_ref()),
        git_line(profile.git.as_ref()),
        cuda_line(&profile.cuda),
    ];
    if let Some(line) = rocm_line(&profile.rocm) {
        lines.push(line);
    }
    if let Some(line) = vulkan_line(&profile.vulkan) {
        lines.push(line);
    }
    lines.extend(gpu_lines(&profile.gpus));
    lines.push(badge_line(
        if profile.sccache.is_some() {
            Badge::Ok
        } else {
            Badge::Warn
        },
        match &profile.sccache {
            Some(path) => format!("sccache active: {}", path.display()),
            None => "sccache not found; repeat builds will be slower".to_string(),
        },
    ));
    lines.push(badge_line(
        Badge::Ok,
        format!(
            "RAM: {} GB available / {} GB total",
            profile.memory.available_mb / 1024,
            profile.memory.total_mb / 1024
        ),
    ));
    let disk_gb = profile.disk.available_bytes / 1024 / 1024 / 1024;
    lines.push(badge_line(
        if profile.disk.require(4 * 1024 * 1024 * 1024).is_ok() {
            Badge::Ok
        } else {
            Badge::Error
        },
        format!("disk: {disk_gb} GB free at {}", profile.disk.path.display()),
    ));
    lines.push(badge_line(
        Badge::Ok,
        format!("recommended backend: {:?}", profile.recommended_backend()),
    ));
    let catalog_lines = ai_catalog_lines(profile);
    if !catalog_lines.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            "AI accelerator catalog",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
        lines.extend(catalog_lines);
    }

    let missing = profile.missing_prerequisites();
    if !missing.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            "Missing prerequisites",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ));
        lines.extend(missing.iter().map(missing_line));
    }

    let warnings = profile.warnings();
    if !warnings.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            "Warnings",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
        lines.extend(
            warnings
                .into_iter()
                .map(|warning| badge_line(Badge::Warn, warning.message)),
        );
    }

    lines
}

fn compiler_line(compiler: Option<&CompilerInfo>) -> Line<'static> {
    match compiler {
        Some(compiler) if compiler.cpp17_ok => badge_line(
            Badge::Ok,
            format!(
                "compiler: {} ({})",
                compiler.path.display(),
                compiler.version
            ),
        ),
        Some(compiler) => badge_line(
            Badge::Error,
            format!(
                "compiler lacks C++17: {} ({})",
                compiler.path.display(),
                compiler
                    .cpp17_error
                    .clone()
                    .unwrap_or_else(|| "probe failed".to_string())
            ),
        ),
        None => badge_line(Badge::Error, "compiler not found".to_string()),
    }
}

fn cmake_line(cmake: Option<&CmakeInfo>) -> Line<'static> {
    match cmake {
        Some(cmake) if cmake.meets_minimum => {
            badge_line(Badge::Ok, format!("cmake {}", cmake.version))
        }
        Some(cmake) => badge_line(
            Badge::Error,
            format!("cmake {} detected; need >= 3.21", cmake.version),
        ),
        None => badge_line(Badge::Error, "cmake not found".to_string()),
    }
}

fn git_line(git: Option<&GitInfo>) -> Line<'static> {
    match git {
        Some(git) if git.meets_minimum => badge_line(Badge::Ok, format!("git {}", git.version)),
        Some(git) => badge_line(
            Badge::Error,
            format!("git {} detected; need >= 2.28", git.version),
        ),
        None => badge_line(Badge::Error, "git not found".to_string()),
    }
}

fn cuda_line(cuda: &CudaCompatibility) -> Line<'static> {
    match cuda {
        CudaCompatibility::Compatible { archs } => {
            badge_line(Badge::Ok, format!("CUDA compatible: {}", archs.join(";")))
        }
        CudaCompatibility::ToolkitTooOld {
            gpu_arch,
            minimum_toolkit,
            found_toolkit,
        } => badge_line(
            Badge::Warn,
            format!("{gpu_arch} requires CUDA >= {minimum_toolkit}; found {found_toolkit}"),
        ),
        CudaCompatibility::ToolkitTooNew {
            gpu_arch,
            maximum_toolkit,
            found_toolkit,
        } => badge_line(
            Badge::Warn,
            format!(
                "{gpu_arch} is not supported by CUDA {found_toolkit}; use CUDA {maximum_toolkit}"
            ),
        ),
        CudaCompatibility::NoGpu => {
            badge_line(Badge::Warn, "nvcc found, no CUDA GPUs detected".to_string())
        }
        CudaCompatibility::NvccMissing => badge_line(
            Badge::Warn,
            "nvcc not found; CUDA backend unavailable".to_string(),
        ),
    }
}

fn vulkan_line(vulkan: &VulkanSupport) -> Option<Line<'static>> {
    if vulkan.available {
        let devices = if vulkan.devices.is_empty() {
            "available".to_string()
        } else {
            vulkan.devices.join(", ")
        };
        Some(badge_line(Badge::Ok, format!("Vulkan: {devices}")))
    } else {
        None
    }
}

fn rocm_line(rocm: &RocmSupport) -> Option<Line<'static>> {
    if rocm.available {
        let targets = if rocm.targets.is_empty() {
            "targets auto".to_string()
        } else {
            rocm.targets.join(";")
        };
        return Some(badge_line(Badge::Ok, format!("ROCm/HIP: {targets}")));
    }
    if rocm.hipconfig_path.is_some() {
        return Some(badge_line(
            Badge::Warn,
            "ROCm/HIP tooling found; no gfx target detected".to_string(),
        ));
    }
    None
}

fn gpu_lines(gpus: &[GpuInfo]) -> Vec<Line<'static>> {
    if gpus.is_empty() {
        return vec![badge_line(Badge::Warn, "GPU: none detected".to_string())];
    }
    gpus.iter()
        .map(|gpu| {
            let arch = gpu.arch.unwrap_or("unknown");
            badge_line(
                Badge::Ok,
                format!(
                    "{} · {} · {} GB VRAM",
                    gpu.name,
                    arch,
                    gpu.memory_total_mb / 1024
                ),
            )
        })
        .collect()
}

fn ai_catalog_lines(profile: &SystemProfile) -> Vec<Line<'static>> {
    gpu_catalog::matches_from_system_profile(profile)
        .into_iter()
        .map(|matched| badge_line(Badge::Ok, matched.gpu.summary()))
        .collect()
}

fn missing_line(missing: &MissingPrerequisite) -> Line<'static> {
    badge_line(
        Badge::Error,
        format!("{}: {}", missing.name, missing.install),
    )
}

#[derive(Debug, Clone, Copy)]
enum Badge {
    Ok,
    Warn,
    Error,
}

fn badge_line(status: Badge, text: String) -> Line<'static> {
    let (label, color) = match status {
        Badge::Ok => ("OK", Color::Green),
        Badge::Warn => ("WARN", Color::Yellow),
        Badge::Error => ("MISS", Color::Red),
    };
    Line::from(vec![
        Span::styled(
            format!("[{label}] "),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::raw(text),
    ])
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use lmml_detect::{
        BuildBackend, CmakeInfo, CompilerInfo, CpuFeatures, CudaVersion, DiskInfo, GitInfo,
        MemInfo, MetalSupport, SystemProfile, VulkanSupport,
    };

    use super::*;

    #[test]
    fn system_lines_show_badges_archs_and_install_hints() {
        let mut app = App::default();
        app.detect_profile = Some(SystemProfile {
            compiler: Some(CompilerInfo {
                path: PathBuf::from("/usr/bin/c++"),
                version: "g++ 13".to_string(),
                cpp17_ok: true,
                cpp17_error: None,
            }),
            cmake: Some(CmakeInfo {
                path: PathBuf::from("/usr/bin/cmake"),
                version: "3.28.0".to_string(),
                meets_minimum: true,
            }),
            git: Some(GitInfo {
                path: PathBuf::from("/usr/bin/git"),
                version: "2.45.0".to_string(),
                meets_minimum: true,
            }),
            cuda: CudaCompatibility::Compatible {
                archs: vec!["sm_86"],
            },
            rocm: lmml_detect::RocmSupport::default(),
            gpus: vec![GpuInfo {
                name: "RTX 3090".to_string(),
                memory_total_mb: 24576,
                compute_cap: "8.6".to_string(),
                arch: Some("sm_86"),
            }],
            gpu_probe_error: None,
            nvidia_devices: lmml_detect::NvidiaDeviceNodes::default(),
            sccache: None,
            metal: MetalSupport {
                available: false,
                displays: Vec::new(),
            },
            vulkan: VulkanSupport {
                available: false,
                devices: Vec::new(),
            },
            cpu: CpuFeatures {
                model: "CPU".to_string(),
                cores: 8,
                threads: 16,
                avx: true,
                avx2: true,
                avx512: false,
                neon: false,
                features: vec!["AVX2".to_string()],
            },
            memory: MemInfo {
                total_mb: 65536,
                available_mb: 32768,
            },
            disk: DiskInfo {
                available_bytes: 8 * 1024 * 1024 * 1024,
                path: PathBuf::from("/tmp"),
            },
        });

        let text = flatten_lines(system_lines(&app));
        assert!(text.contains("[OK] compiler"));
        assert!(text.contains("CUDA compatible: sm_86"));
        assert!(text.contains("RTX 3090"));
        assert!(text.contains("great-value 24GB prosumer card"));
        assert!(text.contains("[WARN] sccache not found"));
        assert!(text.contains(&format!(
            "{:?}",
            BuildBackend::Cuda {
                archs: vec!["sm_86"]
            }
        )));
    }

    #[test]
    fn system_lines_show_missing_prerequisites() {
        let mut app = App::default();
        app.detect_profile = Some(SystemProfile {
            compiler: None,
            cmake: None,
            git: None,
            cuda: CudaCompatibility::NvccMissing,
            rocm: lmml_detect::RocmSupport::default(),
            gpus: Vec::new(),
            gpu_probe_error: None,
            nvidia_devices: lmml_detect::NvidiaDeviceNodes::default(),
            sccache: None,
            metal: MetalSupport {
                available: false,
                displays: Vec::new(),
            },
            vulkan: VulkanSupport {
                available: false,
                devices: Vec::new(),
            },
            cpu: CpuFeatures {
                model: "CPU".to_string(),
                cores: 4,
                threads: 8,
                avx: false,
                avx2: false,
                avx512: false,
                neon: false,
                features: vec!["generic".to_string()],
            },
            memory: MemInfo {
                total_mb: 8192,
                available_mb: 4096,
            },
            disk: DiskInfo {
                available_bytes: 1024,
                path: PathBuf::from("/tmp"),
            },
        });

        let text = flatten_lines(system_lines(&app));
        assert!(text.contains("compiler not found"));
        assert!(text.contains("sudo apt install build-essential"));
        assert!(text.contains("sudo apt install cmake"));
        assert!(text.contains("sudo apt install git"));
        assert!(text.contains("4 GB free disk"));
    }

    #[test]
    fn cuda_toolkit_too_new_warning_is_rendered() {
        let line = cuda_line(&CudaCompatibility::ToolkitTooNew {
            gpu_arch: "sm_61",
            maximum_toolkit: "12.x",
            found_toolkit: CudaVersion::new(13, 1).raw,
        });
        assert!(flatten_lines(vec![line])
            .contains("sm_61 is not supported by CUDA 13.1; use CUDA 12.x"));
    }

    #[test]
    fn cuda_toolkit_warning_is_rendered() {
        let line = cuda_line(&CudaCompatibility::ToolkitTooOld {
            gpu_arch: "sm_89",
            minimum_toolkit: "11.8",
            found_toolkit: CudaVersion::new(11, 0).raw,
        });
        assert!(flatten_lines(vec![line]).contains("sm_89 requires CUDA >= 11.8"));
    }

    fn flatten_lines(lines: Vec<Line<'static>>) -> String {
        lines
            .into_iter()
            .flat_map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.into_owned())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
            .join("")
    }
}
