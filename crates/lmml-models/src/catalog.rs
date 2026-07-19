//! Advisory local-LLM model family catalog.
//!
//! LMML runtime profiles are intentionally exact and hardware-specific. This
//! catalog is broader: it recognizes known Qwen, Gemma, and Hermes family
//! variants so the TUI and installer can provide practical guidance without
//! creating unvalidated runtime profiles for every model size.

use std::fmt;

/// Supported upstream model family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmFamily {
    /// Alibaba Qwen3.5 open-weight model family.
    Qwen35,
    /// Alibaba Qwen3.6 open-weight model family.
    Qwen36,
    /// Google Gemma 4 open model family.
    Gemma4,
    /// Nous Research Hermes 4 model family.
    Hermes4,
}

impl fmt::Display for LlmFamily {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LlmFamily::Qwen35 => formatter.write_str("Qwen3.5"),
            LlmFamily::Qwen36 => formatter.write_str("Qwen3.6"),
            LlmFamily::Gemma4 => formatter.write_str("Gemma 4"),
            LlmFamily::Hermes4 => formatter.write_str("Hermes 4"),
        }
    }
}

/// Broad architecture class for model sizing guidance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmArchitecture {
    /// Dense decoder model.
    Dense,
    /// Dense model with efficient/effective parameter naming.
    EffectiveDense {
        /// Approximate loaded parameter count, including embeddings.
        loaded_parameters: &'static str,
    },
    /// Sparse mixture-of-experts model.
    MixtureOfExperts {
        /// Approximate active parameter count per generated token.
        active_parameters: &'static str,
    },
}

impl fmt::Display for LlmArchitecture {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LlmArchitecture::Dense => formatter.write_str("dense"),
            LlmArchitecture::EffectiveDense { loaded_parameters } => {
                write!(formatter, "effective dense ({loaded_parameters} loaded)")
            }
            LlmArchitecture::MixtureOfExperts { active_parameters } => {
                write!(formatter, "MoE ({active_parameters} active)")
            }
        }
    }
}

/// Input/output modality supported by a known model variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmModality {
    /// Text input and text output.
    Text,
    /// Image input support.
    Vision,
    /// Video input support.
    Video,
    /// Audio input support.
    Audio,
}

impl fmt::Display for LlmModality {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LlmModality::Text => formatter.write_str("text"),
            LlmModality::Vision => formatter.write_str("vision"),
            LlmModality::Video => formatter.write_str("video"),
            LlmModality::Audio => formatter.write_str("audio"),
        }
    }
}

/// Source category for catalog entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmCatalogSource {
    /// Official upstream model-family documentation or repository.
    Official,
    /// Community GGUF packaging exists, but the family/variant is official.
    CommunityGguf,
}

/// Known model-family variant and local-AI sizing guidance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownModelVariant {
    /// Upstream family.
    pub family: LlmFamily,
    /// Display name used in docs and TUI details.
    pub name: &'static str,
    /// Public parameter label, such as `9B` or `35B-A3B`.
    pub parameter_label: &'static str,
    /// Native or documented context window in tokens.
    pub context_tokens: u32,
    /// Architecture class.
    pub architecture: LlmArchitecture,
    /// Supported modalities relevant to local serving.
    pub modalities: &'static [LlmModality],
    /// Practical local-AI guidance for the variant.
    pub local_guidance: &'static str,
    /// Short architecture-specific note shown in the TUI.
    pub implementation_note: &'static str,
    /// Runtime, prompt-template, quantization, or sidecar notes.
    pub serving_notes: &'static [&'static str],
    /// Filename or repository substrings used for matching local GGUFs.
    pub filename_needles: &'static [&'static str],
    /// Source status for the catalog entry.
    pub source: LlmCatalogSource,
}

impl KnownModelVariant {
    /// Return modalities as a comma-separated string for display.
    pub fn modalities_label(&self) -> String {
        self.modalities
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

const TEXT: &[LlmModality] = &[LlmModality::Text];
const TEXT_VISION_VIDEO: &[LlmModality] =
    &[LlmModality::Text, LlmModality::Vision, LlmModality::Video];
const GEMMA_ALL_SMALL: &[LlmModality] = &[
    LlmModality::Text,
    LlmModality::Vision,
    LlmModality::Video,
    LlmModality::Audio,
];
const GEMMA_TEXT_VISION_VIDEO: &[LlmModality] =
    &[LlmModality::Text, LlmModality::Vision, LlmModality::Video];
const QWEN35_NOTES: &[&str] = &[
    "use embedded GGUF chat template unless a model-specific override is validated",
    "thinking mode should keep at least 128k context when available",
    "LMML adds --reasoning-format none at launch to preserve raw thinking tags",
    "LMML adds q8_0 KV cache when Qwen context exceeds 32k and no KV type is already set",
];
const QWEN36_NOTES: &[&str] = &[
    "use embedded GGUF chat template; Qwen3.6 official docs list llama.cpp GGUF support",
    "current official open Qwen3.6 variants are 27B dense and 35B-A3B MoE",
    "LMML adds --reasoning-format none at launch to preserve raw thinking tags",
    "LMML adds q8_0 KV cache when Qwen context exceeds 32k and no KV type is already set",
];
const GEMMA4_COMMON_NOTES: &[&str] = &[
    "native roles: system, user, assistant",
    "thinking is toggled with the <|think|> token in the system prompt",
    "prefer official Google QAT q4_0 GGUF checkpoints for near-lossless 4-bit execution",
    "all Gemma 4 sizes ship MTP draft support; use a matching drafter GGUF for speculative decoding",
];
const HERMES4_COMMON_NOTES: &[&str] = &[
    "use ChatML formatting with system, user, and assistant role blocks",
    "hybrid reasoning may emit <think>...</think> segments",
    "14B sampling baseline: temperature=0.6 top_p=0.95 top_k=20",
    "validate Hermes tool-call tag parsing before enabling agent tool execution",
];

const KNOWN_MODEL_VARIANTS: &[KnownModelVariant] = &[
    KnownModelVariant {
        family: LlmFamily::Qwen35,
        name: "Qwen3.5-0.8B",
        parameter_label: "0.8B",
        context_tokens: 262_144,
        architecture: LlmArchitecture::Dense,
        modalities: TEXT_VISION_VIDEO,
        local_guidance: "tiny Qwen3.5 development, task-specific tuning, or CPU fallback",
        implementation_note: "small dense hybrid Qwen3.5 multimodal variant",
        serving_notes: QWEN35_NOTES,
        filename_needles: &["qwen3.5-0.8b", "qwen35-0.8b"],
        source: LlmCatalogSource::CommunityGguf,
    },
    KnownModelVariant {
        family: LlmFamily::Qwen35,
        name: "Qwen3.5-2B",
        parameter_label: "2B",
        context_tokens: 262_144,
        architecture: LlmArchitecture::Dense,
        modalities: TEXT_VISION_VIDEO,
        local_guidance: "small agent fast path on 8GB+ GPUs",
        implementation_note: "small dense hybrid Qwen3.5 multimodal variant",
        serving_notes: QWEN35_NOTES,
        filename_needles: &["qwen3.5-2b", "qwen35-2b"],
        source: LlmCatalogSource::CommunityGguf,
    },
    KnownModelVariant {
        family: LlmFamily::Qwen35,
        name: "Qwen3.5-4B",
        parameter_label: "4B",
        context_tokens: 262_144,
        architecture: LlmArchitecture::Dense,
        modalities: TEXT_VISION_VIDEO,
        local_guidance: "validated LMML sweet spot for 11GB-16GB long-context coding",
        implementation_note: "dense Qwen3.5 multimodal variant with long-context thinking support",
        serving_notes: QWEN35_NOTES,
        filename_needles: &["qwen3.5-4b", "qwen35-4b"],
        source: LlmCatalogSource::CommunityGguf,
    },
    KnownModelVariant {
        family: LlmFamily::Qwen35,
        name: "Qwen3.5-9B",
        parameter_label: "9B",
        context_tokens: 262_144,
        architecture: LlmArchitecture::Dense,
        modalities: TEXT_VISION_VIDEO,
        local_guidance: "validated 24GB workstation coding target; careful on 16GB",
        implementation_note: "dense Qwen3.5 multimodal variant with long-context thinking support",
        serving_notes: QWEN35_NOTES,
        filename_needles: &["qwen3.5-9b", "qwen35-9b"],
        source: LlmCatalogSource::CommunityGguf,
    },
    KnownModelVariant {
        family: LlmFamily::Qwen35,
        name: "Qwen3.5-27B",
        parameter_label: "27B",
        context_tokens: 262_144,
        architecture: LlmArchitecture::Dense,
        modalities: TEXT_VISION_VIDEO,
        local_guidance: "24GB+ Q4 target; prefer 32GB+ for long context",
        implementation_note: "medium dense Qwen3.5 coding/reasoning model",
        serving_notes: QWEN35_NOTES,
        filename_needles: &["qwen3.5-27b", "qwen35-27b"],
        source: LlmCatalogSource::CommunityGguf,
    },
    KnownModelVariant {
        family: LlmFamily::Qwen35,
        name: "Qwen3.5-35B-A3B",
        parameter_label: "35B-A3B",
        context_tokens: 262_144,
        architecture: LlmArchitecture::MixtureOfExperts {
            active_parameters: "3B",
        },
        modalities: TEXT_VISION_VIDEO,
        local_guidance: "MoE coding/reasoning target; 24GB Q4 minimum, 32GB+ preferred",
        implementation_note: "sparse Qwen3.5 MoE with 3B active parameters",
        serving_notes: QWEN35_NOTES,
        filename_needles: &["qwen3.5-35b-a3b", "qwen35-35b-a3b"],
        source: LlmCatalogSource::CommunityGguf,
    },
    KnownModelVariant {
        family: LlmFamily::Qwen35,
        name: "Qwen3.5-122B-A10B",
        parameter_label: "122B-A10B",
        context_tokens: 262_144,
        architecture: LlmArchitecture::MixtureOfExperts {
            active_parameters: "10B",
        },
        modalities: TEXT_VISION_VIDEO,
        local_guidance: "multi-GPU or server-class local inference target",
        implementation_note: "large sparse Qwen3.5 MoE with 10B active parameters",
        serving_notes: QWEN35_NOTES,
        filename_needles: &["qwen3.5-122b-a10b", "qwen35-122b-a10b"],
        source: LlmCatalogSource::CommunityGguf,
    },
    KnownModelVariant {
        family: LlmFamily::Qwen35,
        name: "Qwen3.5-397B-A17B",
        parameter_label: "397B-A17B",
        context_tokens: 262_144,
        architecture: LlmArchitecture::MixtureOfExperts {
            active_parameters: "17B",
        },
        modalities: TEXT_VISION_VIDEO,
        local_guidance: "datacenter-scale or serious multi-GPU only",
        implementation_note: "frontier-scale sparse Qwen3.5 MoE with 17B active parameters",
        serving_notes: QWEN35_NOTES,
        filename_needles: &["qwen3.5-397b-a17b", "qwen35-397b-a17b"],
        source: LlmCatalogSource::CommunityGguf,
    },
    KnownModelVariant {
        family: LlmFamily::Qwen36,
        name: "Qwen3.6-27B",
        parameter_label: "27B",
        context_tokens: 262_144,
        architecture: LlmArchitecture::Dense,
        modalities: TEXT_VISION_VIDEO,
        local_guidance: "current Qwen3.6 dense local flagship; 24GB Q4 minimum",
        implementation_note: "Qwen3.6 dense open model focused on coding and repository reasoning",
        serving_notes: QWEN36_NOTES,
        filename_needles: &["qwen3.6-27b", "qwen36-27b"],
        source: LlmCatalogSource::Official,
    },
    KnownModelVariant {
        family: LlmFamily::Qwen36,
        name: "Qwen3.6-35B-A3B",
        parameter_label: "35B-A3B",
        context_tokens: 262_144,
        architecture: LlmArchitecture::MixtureOfExperts {
            active_parameters: "3B",
        },
        modalities: TEXT_VISION_VIDEO,
        local_guidance: "current Qwen3.6 agentic-coding MoE; 24GB Q4 minimum, 32GB+ preferred",
        implementation_note: "Qwen3.6 sparse MoE open model focused on agentic coding",
        serving_notes: QWEN36_NOTES,
        filename_needles: &["qwen3.6-35b-a3b", "qwen36-35b-a3b"],
        source: LlmCatalogSource::Official,
    },
    KnownModelVariant {
        family: LlmFamily::Gemma4,
        name: "Gemma 4 E2B",
        parameter_label: "E2B",
        context_tokens: 128_000,
        architecture: LlmArchitecture::EffectiveDense {
            loaded_parameters: "5.1B",
        },
        modalities: GEMMA_ALL_SMALL,
        local_guidance: "edge/mobile-class Gemma 4; fastest local experimentation",
        implementation_note:
            "effective-size Gemma 4 model using Per-Layer Embeddings for edge hardware",
        serving_notes: GEMMA4_COMMON_NOTES,
        filename_needles: &["gemma-4-e2b", "gemma4-e2b", "gemma-4-e2b_q4_0"],
        source: LlmCatalogSource::Official,
    },
    KnownModelVariant {
        family: LlmFamily::Gemma4,
        name: "Gemma 4 E4B",
        parameter_label: "E4B",
        context_tokens: 128_000,
        architecture: LlmArchitecture::EffectiveDense {
            loaded_parameters: "8B",
        },
        modalities: GEMMA_ALL_SMALL,
        local_guidance: "small Gemma 4 agent profile for 8GB-12GB systems",
        implementation_note:
            "effective-size Gemma 4 model using Per-Layer Embeddings for edge hardware",
        serving_notes: GEMMA4_COMMON_NOTES,
        filename_needles: &["gemma-4-e4b", "gemma4-e4b", "gemma-4-e4b_q4_0"],
        source: LlmCatalogSource::Official,
    },
    KnownModelVariant {
        family: LlmFamily::Gemma4,
        name: "Gemma 4 12B",
        parameter_label: "12B",
        context_tokens: 256_000,
        architecture: LlmArchitecture::Dense,
        modalities: GEMMA_ALL_SMALL,
        local_guidance: "LMML MTP profile target for 16GB GPUs with Q4/QAT GGUF",
        implementation_note:
            "encoder-free unified multimodal model with direct vision/audio projections",
        serving_notes: GEMMA4_COMMON_NOTES,
        filename_needles: &[
            "gemma-4-12b",
            "gemma4-12b",
            "gemma-4-12b-it-qat-q4_0",
            "gemma4-12b-qat-q4_k_m",
        ],
        source: LlmCatalogSource::Official,
    },
    KnownModelVariant {
        family: LlmFamily::Gemma4,
        name: "Gemma 4 26B-A4B",
        parameter_label: "26B-A4B",
        context_tokens: 256_000,
        architecture: LlmArchitecture::MixtureOfExperts {
            active_parameters: "4B",
        },
        modalities: GEMMA_TEXT_VISION_VIDEO,
        local_guidance: "MoE Gemma 4 target; 16GB Q4 is possible, 24GB+ safer",
        implementation_note: "efficient Gemma 4 MoE that activates about 4B parameters per token",
        serving_notes: GEMMA4_COMMON_NOTES,
        filename_needles: &["gemma-4-26b-a4b", "gemma4-26b-a4b", "gemma-4-26b_q4_0"],
        source: LlmCatalogSource::Official,
    },
    KnownModelVariant {
        family: LlmFamily::Gemma4,
        name: "Gemma 4 31B",
        parameter_label: "31B",
        context_tokens: 256_000,
        architecture: LlmArchitecture::Dense,
        modalities: GEMMA_TEXT_VISION_VIDEO,
        local_guidance: "largest dense Gemma 4 local target; 24GB Q4 minimum, 32GB+ preferred",
        implementation_note: "heaviest dense Gemma 4 model for server-grade local developer setups",
        serving_notes: GEMMA4_COMMON_NOTES,
        filename_needles: &["gemma-4-31b", "gemma4-31b"],
        source: LlmCatalogSource::Official,
    },
    KnownModelVariant {
        family: LlmFamily::Hermes4,
        name: "Hermes 4 14B",
        parameter_label: "14B",
        context_tokens: 40_960,
        architecture: LlmArchitecture::Dense,
        modalities: TEXT,
        local_guidance:
            "Qwen-based Hermes coding/reasoning target; 16GB Q4 possible, 24GB preferred",
        implementation_note: "hybrid-mode Hermes 4 model based on Qwen 3 14B",
        serving_notes: HERMES4_COMMON_NOTES,
        filename_needles: &["hermes-4-14b", "hermes4-14b", "nousresearch-hermes-4-14b"],
        source: LlmCatalogSource::Official,
    },
    KnownModelVariant {
        family: LlmFamily::Hermes4,
        name: "Hermes 4.3 36B",
        parameter_label: "36B",
        context_tokens: 524_288,
        architecture: LlmArchitecture::Dense,
        modalities: TEXT,
        local_guidance: "large Hermes local/server target; 32GB+ Q4 or multi-GPU recommended",
        implementation_note: "Hermes 4.3 model based on Seed 36B",
        serving_notes: HERMES4_COMMON_NOTES,
        filename_needles: &[
            "hermes-4.3-36b",
            "hermes4.3-36b",
            "hermes-43-36b",
            "hermes43-36b",
        ],
        source: LlmCatalogSource::Official,
    },
    KnownModelVariant {
        family: LlmFamily::Hermes4,
        name: "Hermes 4 70B",
        parameter_label: "70B",
        context_tokens: 131_072,
        architecture: LlmArchitecture::Dense,
        modalities: TEXT,
        local_guidance: "server or multi-GPU Hermes target; 48GB+ or heavy quantization expected",
        implementation_note: "large Hermes 4 model based on Llama 3.1 70B",
        serving_notes: HERMES4_COMMON_NOTES,
        filename_needles: &["hermes-4-70b", "hermes4-70b"],
        source: LlmCatalogSource::Official,
    },
    KnownModelVariant {
        family: LlmFamily::Hermes4,
        name: "Hermes 4 405B FP8",
        parameter_label: "405B FP8",
        context_tokens: 131_072,
        architecture: LlmArchitecture::Dense,
        modalities: TEXT,
        local_guidance: "datacenter-class Hermes target; not a normal single-workstation profile",
        implementation_note: "frontier-scale Hermes 4 model based on Llama 3.1 405B",
        serving_notes: HERMES4_COMMON_NOTES,
        filename_needles: &["hermes-4-405b", "hermes4-405b", "hermes-4-405b-fp8"],
        source: LlmCatalogSource::Official,
    },
];

/// Return all known model variants in display order.
pub fn known_model_variants() -> &'static [KnownModelVariant] {
    KNOWN_MODEL_VARIANTS
}

/// Return all known variants for a family.
pub fn known_family_variants(family: LlmFamily) -> Vec<&'static KnownModelVariant> {
    KNOWN_MODEL_VARIANTS
        .iter()
        .filter(|variant| variant.family == family)
        .collect()
}

/// Match a filename, display name, or repository id to a known model variant.
pub fn match_known_model_name(name: &str) -> Option<&'static KnownModelVariant> {
    let normalized_name = normalize_match_text(name);
    KNOWN_MODEL_VARIANTS.iter().find(|variant| {
        variant
            .filename_needles
            .iter()
            .any(|needle| normalized_name.contains(&normalize_match_text(needle)))
    })
}

fn normalize_match_text(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        known_family_variants, match_known_model_name, LlmArchitecture, LlmFamily, LlmModality,
        TEXT,
    };

    #[test]
    fn matches_qwen36_official_variants() {
        let variant = match_known_model_name("Qwen3.6-35B-A3B-Q4_K_M.gguf").expect("match qwen");

        assert_eq!(variant.family, LlmFamily::Qwen36);
        assert_eq!(variant.parameter_label, "35B-A3B");
        assert_eq!(
            variant.architecture,
            LlmArchitecture::MixtureOfExperts {
                active_parameters: "3B"
            }
        );
    }

    #[test]
    fn matches_qwen35_small_variants() {
        let variant =
            match_known_model_name("unsloth-Qwen3.5-0.8B-Q8_0.gguf").expect("match qwen small");

        assert_eq!(variant.family, LlmFamily::Qwen35);
        assert_eq!(variant.parameter_label, "0.8B");
        assert!(variant.modalities.contains(&LlmModality::Vision));
    }

    #[test]
    fn matches_gemma4_moe_variant() {
        let variant =
            match_known_model_name("gemma-4-26B-A4B-it-qat-Q4_0.gguf").expect("match gemma");

        assert_eq!(variant.family, LlmFamily::Gemma4);
        assert!(variant
            .serving_notes
            .iter()
            .any(|note| note.contains("<|think|>")));
        assert_eq!(
            variant.architecture,
            LlmArchitecture::MixtureOfExperts {
                active_parameters: "4B"
            }
        );
    }

    #[test]
    fn returns_only_qwen36_official_variants() {
        let variants = known_family_variants(LlmFamily::Qwen36);

        assert_eq!(variants.len(), 2);
        assert!(variants.iter().any(|variant| variant.name == "Qwen3.6-27B"));
        assert!(variants
            .iter()
            .any(|variant| variant.name == "Qwen3.6-35B-A3B"));
    }

    #[test]
    fn matches_hermes4_variants() {
        let variant = match_known_model_name("NousResearch-Hermes-4.3-36B-Q4_K_M.gguf")
            .expect("match hermes");

        assert_eq!(variant.family, LlmFamily::Hermes4);
        assert_eq!(variant.parameter_label, "36B");
        assert_eq!(variant.context_tokens, 524_288);
        assert_eq!(variant.modalities, TEXT);
        assert!(variant
            .serving_notes
            .iter()
            .any(|note| note.contains("ChatML")));
    }
}
