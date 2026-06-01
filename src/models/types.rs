use serde::{Deserialize, Serialize};

/// Metadata parsed from a .gguf model file.
#[derive(Debug, Clone)]
pub struct ModelMetadata {
    pub filename: String,
    pub path: String,
    pub size_bytes: u64,
    pub quantization: String,
    pub param_count: String,
    pub model_type: String,
    pub is_loaded: bool,
    pub is_favorite: bool,
    pub last_used: String,
}

/// Quantization types commonly used in GGUF models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Quantization {
    Q2K,
    Q3KSmall,
    Q3KMedium,
    Q3KLarge,
    Q4KSmall,
    Q4KMedium,
    Q4KLarge,
    Q5KSmall,
    Q5KMedium,
    Q5KLarge,
    Q6K,
    Q8_0,
    F16,
}

impl std::fmt::Display for Quantization {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Quantization::Q2K => write!(f, "Q2_K"),
            Quantization::Q3KSmall => write!(f, "Q3_K_S"),
            Quantization::Q3KMedium => write!(f, "Q3_K_M"),
            Quantization::Q3KLarge => write!(f, "Q3_K_L"),
            Quantization::Q4KSmall => write!(f, "Q4_K_S"),
            Quantization::Q4KMedium => write!(f, "Q4_K_M"),
            Quantization::Q4KLarge => write!(f, "Q4_K_L"),
            Quantization::Q5KSmall => write!(f, "Q5_K_S"),
            Quantization::Q5KMedium => write!(f, "Q5_K_M"),
            Quantization::Q5KLarge => write!(f, "Q5_K_L"),
            Quantization::Q6K => write!(f, "Q6_K"),
            Quantization::Q8_0 => write!(f, "Q8_0"),
            Quantization::F16 => write!(f, "F16"),
        }
    }
}

impl Quantization {
    /// Parse a quantization from a GGUF filename.
    pub fn from_filename(name: &str) -> Option<Quantization> {
        let upper = name.to_uppercase();
        if upper.contains("Q2_K") {
            return Some(Quantization::Q2K);
        }
        if upper.contains("Q3_K_S") {
            return Some(Quantization::Q3KSmall);
        }
        if upper.contains("Q3_K_M") {
            return Some(Quantization::Q3KMedium);
        }
        if upper.contains("Q3_K_L") {
            return Some(Quantization::Q3KLarge);
        }
        if upper.contains("Q4_K_S") {
            return Some(Quantization::Q4KSmall);
        }
        if upper.contains("Q4_K_M") {
            return Some(Quantization::Q4KMedium);
        }
        if upper.contains("Q4_K_L") {
            return Some(Quantization::Q4KLarge);
        }
        if upper.contains("Q5_K_S") {
            return Some(Quantization::Q5KSmall);
        }
        if upper.contains("Q5_K_M") {
            return Some(Quantization::Q5KMedium);
        }
        if upper.contains("Q5_K_L") {
            return Some(Quantization::Q5KLarge);
        }
        if upper.contains("Q6_K") {
            return Some(Quantization::Q6K);
        }
        if upper.contains("Q8_0") {
            return Some(Quantization::Q8_0);
        }
        if upper.contains("F16") || upper.contains("FP16") {
            return Some(Quantization::F16);
        }
        None
    }
}

/// Extract a human-readable model name from a GGUF filename.
pub fn extract_model_name(filename: &str) -> String {
    let name = filename.strip_suffix(".gguf").unwrap_or(filename);
    // Strip common quantization suffixes for cleaner display
    let quants = [
        "-Q2_K", "-Q3_K_S", "-Q3_K_M", "-Q3_K_L", "-Q4_K_S", "-Q4_K_M", "-Q4_K_L", "-Q5_K_S",
        "-Q5_K_M", "-Q5_K_L", "-Q6_K", "-Q8_0", "-F16", "-FP16",
    ];
    for q in &quants {
        if let Some(base) = name.strip_suffix(q) {
            return base.to_string();
        }
    }
    name.to_string()
}

/// Format bytes into a human-readable string (e.g., "4.92 GB").
pub fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{size} {}", UNITS[unit])
    } else {
        format!("{size:.2} {}", UNITS[unit])
    }
}

/// Extract approximate parameter count from model name.
pub fn extract_param_count(name: &str) -> String {
    let patterns = [
        ("8x7B", "8x7B"),
        ("8x22B", "8x22B"),
        ("70B", "70B"),
        ("34B", "34B"),
        ("27B", "27B"),
        ("13B", "13B"),
        ("8B", "8B"),
        ("7B", "7B"),
        ("3B", "3B"),
        ("2B", "2B"),
        ("1B", "1B"),
    ];
    for (pattern, label) in &patterns {
        if name.contains(pattern) {
            return label.to_string();
        }
    }
    "?".to_string()
}

/// Extract architecture type from model name.
pub fn extract_model_type(name: &str) -> String {
    let lower = name.to_lowercase();
    let archs = [
        "llama",
        "mistral",
        "mixtral",
        "phi",
        "gemma",
        "qwen",
        "deepseek",
        "falcon",
        "starcoder",
        "codellama",
        "dbrx",
        "command-r",
        "yi",
        "orca",
        "neural-chat",
        "solar",
    ];
    for arch in &archs {
        if lower.contains(arch) {
            return arch.to_string();
        }
    }
    "unknown".to_string()
}
