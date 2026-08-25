//! Versioned substrate, artifact, runtime, and observation contracts.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use thiserror::Error;

/// Version of the persisted substrate and artifact schemas.
pub const SCHEMA_VERSION: u32 = 2;

/// Lowercase SHA-256 digest encoded as hexadecimal.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hash256(pub(crate) String);

impl Hash256 {
    /// Parse and validate a lowercase hexadecimal SHA-256 digest.
    pub fn parse(value: impl Into<String>) -> Result<Self, SubstrateError> {
        let value = value.into();
        if value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            Ok(Self(value))
        } else {
            Err(SubstrateError::InvalidHash(value))
        }
    }

    /// Return the digest as lowercase hexadecimal text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Hash256 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for Hash256 {
    type Err = SubstrateError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl Serialize for Hash256 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Hash256 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

/// Conceptual model lineage identity, independent of any deployment file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelIdentity {
    /// Stable lineage identifier chosen by the importer/operator.
    pub lineage_id: String,
    /// Human-readable model name.
    pub model_name: String,
    /// Parent lineage for successor checkpoints.
    pub parent: Option<String>,
    /// Digest of the canonical substrate manifest with this field zeroed.
    pub canonical_manifest_hash: Hash256,
}

/// Deployment representation of an artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelRepresentation {
    /// Hugging Face/Safetensors checkpoint representation.
    Safetensors,
    /// llama.cpp GGUF deployment representation.
    Gguf,
}

/// Supported quantization labels for derived artifacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(non_camel_case_types)]
pub enum QuantizationKind {
    /// F16 artifact.
    F16,
    /// BF16 artifact.
    Bf16,
    /// Q8_0 artifact.
    Q8_0,
    /// Q6_K artifact.
    Q6_K,
    /// Q4_K_M artifact.
    Q4_K_M,
}

/// Identity of a derived or canonical artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactIdentity {
    /// Stable artifact identifier.
    pub artifact_id: String,
    /// Parent conceptual model lineage.
    pub model_lineage_id: String,
    /// File/container representation.
    pub representation: ModelRepresentation,
    /// Optional quantization applied to the representation.
    pub quantization: Option<QuantizationKind>,
    /// SHA-256 of the artifact payload or canonical artifact manifest.
    pub artifact_hash: Hash256,
}

/// Architecture facts copied from the actual checkpoint configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchitectureFingerprint {
    /// `model_type` from the checkpoint configuration.
    pub model_type: Option<String>,
    /// Architecture names declared by the checkpoint.
    pub architectures: Vec<String>,
    /// Full configuration object retained as structured evidence.
    pub fields: BTreeMap<String, Value>,
}

/// One tensor header discovered in a Safetensors shard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TensorDescriptor {
    /// Tensor name.
    pub name: String,
    /// Relative shard path.
    pub shard: String,
    /// Safetensors dtype label.
    pub dtype: String,
    /// Tensor dimensions in file order.
    pub shape: Vec<u64>,
    /// Product of `shape` dimensions.
    pub parameter_count: u64,
}

/// Hash and tensor inventory for one shard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShardManifest {
    /// Relative path beneath the canonical root.
    pub path: String,
    /// File size in bytes.
    pub size_bytes: u64,
    /// SHA-256 of the complete shard.
    pub sha256: Hash256,
    /// Tensor names found in the header.
    pub tensor_names: Vec<String>,
}

/// Hash and size for a non-weight checkpoint asset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuxiliaryFileManifest {
    /// Relative path beneath the canonical root.
    pub path: String,
    /// File size in bytes.
    pub size_bytes: u64,
    /// SHA-256 of the complete file.
    pub sha256: Hash256,
}

/// Bounded progress emitted after each canonical weight shard is hashed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportProgress {
    /// Shard that has just completed hashing.
    pub current_shard: String,
    /// Number of fully hashed shards.
    pub shards_completed: usize,
    /// Total number of canonical shards.
    pub shards_total: usize,
    /// Fully hashed shard bytes.
    pub bytes_hashed: u64,
    /// Total bytes across canonical weight shards.
    pub total_bytes: u64,
}

/// Deterministic manifest for a canonical Safetensors substrate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubstrateManifest {
    /// Schema version for forward compatibility.
    pub schema_version: u32,
    /// Conceptual model identity.
    pub model: ModelIdentity,
    /// Architecture facts from `config.json`.
    pub architecture: ArchitectureFingerprint,
    /// SHA-256 of `config.json`.
    pub config_hash: Hash256,
    /// SHA-256 over framed tokenizer-related file records.
    pub tokenizer_hash: Hash256,
    /// SHA-256 of `model.safetensors.index.json`, when present.
    pub index_hash: Option<Hash256>,
    /// Tokenizer, generation, processor, and model-code assets.
    pub auxiliary_files: Vec<AuxiliaryFileManifest>,
    /// Number of tensor descriptors.
    pub tensor_count: u64,
    /// Sum of tensor element counts.
    pub parameter_count: u64,
    /// Ordered shard records.
    pub shards: Vec<ShardManifest>,
    /// Ordered tensor records.
    pub tensors: Vec<TensorDescriptor>,
}

/// Append-only manifest record for a derived artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactManifest {
    /// Schema version for forward compatibility.
    pub schema_version: u32,
    /// Artifact identity.
    pub artifact: ArtifactIdentity,
    /// Immediate parent artifact, if this is derived.
    pub parent_artifact: Option<String>,
    /// Canonical model lineage identifier.
    pub canonical_model: String,
    /// Tool name that produced the artifact.
    pub tool: String,
    /// Tool version or commit.
    pub tool_version: String,
    /// Exact command or structured parameters.
    pub command_or_parameters: Vec<String>,
    /// Input hashes used to produce this artifact.
    pub source_hashes: Vec<Hash256>,
    /// Output artifact hash.
    pub output_hash: Hash256,
    /// Absolute path to the immutable artifact payload.
    pub artifact_path: PathBuf,
    /// Backend admission evidence for runnable deployment artifacts.
    pub admission: Option<ArtifactAdmission>,
    /// RFC3339 creation timestamp.
    pub created_at: String,
}

/// Evidence that a deployment backend opened and checked an artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactAdmission {
    /// Backend used for admission, such as `llama.cpp`.
    pub backend: String,
    /// Exact backend version or source revision.
    pub backend_version: String,
    /// RFC3339 timestamp for the completed admission check.
    pub checked_at: String,
}

/// Capability that a runtime may advertise to a caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeCapability {
    /// Text generation through the deployment runtime.
    TextGeneration,
    /// Logit access when the selected integration exposes it.
    Logits,
    /// Embedding generation when the selected integration exposes it.
    Embeddings,
    /// Arbitrary hidden-state observation.
    HiddenStateObservation,
    /// The eleven research taps required by a future instrumented provider.
    ElevenTapObservation,
}

/// Request for a model capability without exposing a filesystem path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelRequest {
    /// Conceptual lineage requested by the caller.
    pub model_lineage_id: String,
    /// Human-readable workload purpose.
    pub purpose: String,
    /// Ordered representation preferences.
    pub representation_preference: Vec<ModelRepresentation>,
    /// Capabilities that the selected runtime must provide.
    pub required_capabilities: Vec<RuntimeCapability>,
}

/// The exact artifact and runtime granted for a model request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelLease {
    /// Stable lease identifier.
    pub lease_id: String,
    /// Conceptual model lineage.
    pub model_lineage_id: String,
    /// Exact artifact selected by LMML.
    pub artifact_id: String,
    /// Runtime instance selected by LMML.
    pub runtime_id: String,
    /// OpenAI-compatible or local runtime endpoint.
    pub endpoint: String,
    /// Manifest hash used for selection.
    pub manifest_hash: Hash256,
    /// Artifact payload hash used for selection.
    pub artifact_hash: Hash256,
    /// Capabilities actually granted.
    pub capabilities: Vec<RuntimeCapability>,
}

/// Runtime facts recorded when an exact artifact is launched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeManifest {
    /// Stable runtime identifier.
    pub runtime_id: String,
    /// Local process ID attested during runtime registration.
    pub pid: u32,
    /// Conceptual model lineage.
    pub model_lineage_id: String,
    /// Exact artifact identity.
    pub artifact_id: String,
    /// SHA-256 of the executed artifact.
    pub artifact_hash: Hash256,
    /// Representation executed by the backend.
    pub representation: ModelRepresentation,
    /// Quantization, if applicable.
    pub quantization: Option<QuantizationKind>,
    /// Backend name, such as llama.cpp.
    pub backend: String,
    /// Backend version or commit.
    pub backend_version: String,
    /// OpenAI-compatible or provider-specific runtime endpoint.
    pub endpoint: String,
    /// Context size selected for the runtime.
    pub context_size: usize,
    /// Capabilities actually exposed by this runtime.
    pub capabilities: Vec<RuntimeCapability>,
    /// RFC3339 registration timestamp.
    pub created_at: String,
}

/// Persisted deterministic inference case used by a pristine baseline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaselineCase {
    /// Stable case identifier.
    pub case_id: String,
    /// Exact prompt submitted to the backend.
    pub prompt: String,
    /// SHA-256 of the captured backend output.
    pub output_hash: Hash256,
    /// Captured output for later human and regression review.
    pub output: String,
}

/// Immutable pristine inference baseline tied to one exact artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaselineManifest {
    /// Version of the persisted lifecycle schema.
    pub schema_version: u32,
    /// Stable baseline identifier.
    pub baseline_id: String,
    /// Conceptual model lineage.
    pub model_lineage_id: String,
    /// Exact artifact used by the baseline.
    pub artifact_id: String,
    /// SHA-256 of the artifact bytes.
    pub artifact_hash: Hash256,
    /// Backend and version used for execution.
    pub backend: String,
    /// Backend version or source revision.
    pub backend_version: String,
    /// Deterministic command parameters shared by all cases.
    pub parameters: Vec<String>,
    /// Ordered baseline cases and their outputs.
    pub cases: Vec<BaselineCase>,
    /// RFC3339 creation timestamp.
    pub created_at: String,
}

/// Immutable authorization issued before successor optimization starts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainingAuthorizationManifest {
    /// Version of the persisted lifecycle schema.
    pub schema_version: u32,
    /// Stable authorization identifier.
    pub authorization_id: String,
    /// Canonical parent lineage selected for training.
    pub base_lineage_id: String,
    /// Canonical parent manifest hash.
    pub base_manifest_hash: Hash256,
    /// Canonical configuration hash.
    pub config_hash: Hash256,
    /// Canonical tokenizer hash.
    pub tokenizer_hash: Hash256,
    /// SHA-256 identity of the training dataset.
    pub dataset_hash: Hash256,
    /// Reproducible training seed.
    pub seed: u64,
    /// Training configuration approved before optimization.
    pub training_config: BTreeMap<String, Value>,
    /// Approved trainable parameter names or prefixes.
    pub approved_allowlist: Vec<String>,
    /// Parameters enumerated trainable before optimization.
    pub trainable_parameters: Vec<String>,
    /// RFC3339 authorization timestamp.
    pub created_at: String,
}

/// Immutable record of a completed successor training run and its safety gates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainingRunManifest {
    /// Version of the persisted lifecycle schema.
    pub schema_version: u32,
    /// Stable training-run identifier.
    pub training_run_id: String,
    /// Pre-optimization authorization consumed by this run.
    pub authorization_id: String,
    /// SHA-256 of the serialized authorization manifest.
    pub authorization_hash: Hash256,
    /// Canonical parent lineage selected for training.
    pub base_lineage_id: String,
    /// Canonical parent manifest hash.
    pub base_manifest_hash: Hash256,
    /// Canonical configuration hash.
    pub config_hash: Hash256,
    /// Canonical tokenizer hash.
    pub tokenizer_hash: Hash256,
    /// SHA-256 identity of the training dataset.
    pub dataset_hash: Hash256,
    /// Reproducible training seed.
    pub seed: u64,
    /// Training configuration retained as structured evidence.
    pub training_config: BTreeMap<String, Value>,
    /// Approved trainable parameter names or prefixes.
    pub approved_allowlist: Vec<String>,
    /// Parameters reported trainable before optimization.
    pub trainable_parameters: Vec<String>,
    /// Final finite loss reported by the trainer.
    pub final_loss: f64,
    /// Finite gradient norms captured during training.
    pub gradient_norms: Vec<f64>,
    /// Validated adapter artifact identity.
    pub adapter: ArtifactIdentity,
    /// Path to the immutable adapter Safetensors payload.
    pub adapter_path: PathBuf,
    /// Exact adapter tensor inventory expected in the Safetensors payload.
    pub adapter_tensors: Vec<TensorDescriptor>,
    /// RFC3339 completion timestamp.
    pub created_at: String,
}

/// Numerical evidence that a candidate merge changed intended weights.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MergeDelta {
    /// Number of tensors with a non-trivial change.
    pub changed_tensor_count: u64,
    /// Number of tensors unchanged at the configured tolerance.
    pub unchanged_tensor_count: u64,
    /// Largest absolute element delta observed.
    pub max_absolute_delta: f64,
    /// Aggregate L2 norm of all observed deltas.
    pub aggregate_norm_delta: f64,
}

/// Candidate-only successor merge record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SuccessorCandidateManifest {
    /// Version of the persisted lifecycle schema.
    pub schema_version: u32,
    /// Stable candidate identifier.
    pub candidate_id: String,
    /// Explicit parent conceptual lineage.
    pub parent_lineage_id: String,
    /// Training run that produced the adapter.
    pub training_run_id: String,
    /// Adapter artifact used by the merge.
    pub adapter_artifact_id: String,
    /// New candidate Safetensors directory.
    pub candidate_path: PathBuf,
    /// Requested model loading dtype.
    pub requested_dtype: String,
    /// Effective model loading dtype.
    pub effective_load_dtype: String,
    /// Dtype used while merging adapter weights.
    pub merge_dtype: String,
    /// Dtype written to the successor checkpoint.
    pub output_dtype: String,
    /// Canonical manifest generated from the merged candidate.
    pub candidate_manifest: SubstrateManifest,
    /// Proof that the merge was not a no-op.
    pub delta: MergeDelta,
    /// Maximum absolute logit difference between live-adapter and merged runs.
    pub equivalence_max_absolute_delta: f64,
    /// Configured equivalence tolerance.
    pub equivalence_tolerance: f64,
    /// RFC3339 completion timestamp.
    pub created_at: String,
}

/// Regression result for one admitted successor baseline case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegressionResult {
    /// Baseline case identifier.
    pub case_id: String,
    /// Parent metric value.
    pub parent_metric: f64,
    /// Candidate metric value.
    pub candidate_metric: f64,
    /// Candidate minus parent.
    pub delta: f64,
    /// Maximum permitted degradation.
    pub maximum_degradation: f64,
    /// Whether the configured bound passed.
    pub passed: bool,
}

/// Immutable admission record for a successor model lineage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SuccessorManifest {
    /// Version of the persisted lifecycle schema.
    pub schema_version: u32,
    /// Newly admitted conceptual model lineage.
    pub successor_lineage_id: String,
    /// Explicit parent conceptual lineage.
    pub parent_lineage_id: String,
    /// Candidate merge admitted by this record.
    pub candidate_id: String,
    /// Training run that produced the candidate.
    pub training_run_id: String,
    /// Canonical successor manifest hash.
    pub successor_hash: Hash256,
    /// Regression evidence against the pristine parent baseline.
    pub regression: Vec<RegressionResult>,
    /// RFC3339 admission timestamp.
    pub admitted_at: String,
}

/// A request for intermediate activations in a research runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivationObservationRequest {
    /// Input token IDs or an opaque provider-owned input reference.
    pub input_ids: Vec<u32>,
    /// Requested layer/tap identifiers from the installed provider.
    pub taps: Vec<String>,
}

/// Provider response for an activation observation request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActivationObservationResponse {
    /// Provider-defined tap outputs. LMML does not assign semantic meaning.
    pub values: BTreeMap<String, Vec<f32>>,
}

/// Explicit failure returned when an installed runtime lacks research taps.
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
pub enum ActivationObservationError {
    /// The selected runtime does not expose the requested capability.
    #[error("activation observation is unsupported by this runtime")]
    Unsupported,
    /// Provider rejected the request for a concrete reason.
    #[error("activation observation provider rejected the request: {0}")]
    Provider(String),
}

/// Optional provider for real hidden-state/tap observations.
pub trait ActivationObservationProvider {
    /// Return capabilities actually implemented by this provider.
    fn capabilities(&self) -> &[RuntimeCapability];

    /// Observe activations or return an explicit capability error.
    fn observe(
        &self,
        request: ActivationObservationRequest,
    ) -> Result<ActivationObservationResponse, ActivationObservationError>;
}

/// Standard llama.cpp provider, which deliberately exposes no hidden-state taps.
#[derive(Debug, Default, Clone, Copy)]
pub struct LlamaCppActivationProvider;

impl ActivationObservationProvider for LlamaCppActivationProvider {
    fn capabilities(&self) -> &[RuntimeCapability] {
        &[RuntimeCapability::TextGeneration]
    }

    fn observe(
        &self,
        _request: ActivationObservationRequest,
    ) -> Result<ActivationObservationResponse, ActivationObservationError> {
        Err(ActivationObservationError::Unsupported)
    }
}

/// Import or verification failures for canonical substrate data.
#[derive(Debug, Error)]
pub enum SubstrateError {
    /// Canonical root does not exist or is not a directory.
    #[error("canonical substrate root is missing: {0}")]
    MissingRoot(PathBuf),
    /// Required tokenizer files are absent.
    #[error("canonical substrate has no tokenizer artifacts: {0}")]
    MissingTokenizer(PathBuf),
    /// A digest is not a valid SHA-256 hexadecimal value.
    #[error("invalid SHA-256 digest: {0}")]
    InvalidHash(String),
    /// A persisted manifest uses an unsupported schema version.
    #[error("unsupported substrate schema version {actual}; expected {expected}")]
    UnsupportedSchemaVersion { expected: u32, actual: u32 },
    /// A lineage or artifact identifier cannot be used as a managed filename.
    #[error("invalid managed identifier: {0}")]
    InvalidIdentifier(String),
    /// Filesystem operation failed.
    #[error("substrate IO failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// JSON document failed to parse.
    #[error("invalid JSON in {path}: {source}")]
    InvalidJson {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    /// Checkpoint config JSON is not a structured object.
    #[error("checkpoint config must be a JSON object: {0}")]
    InvalidConfig(PathBuf),
    /// Safetensors index is malformed.
    #[error("invalid Safetensors index: {0}")]
    InvalidIndex(PathBuf),
    /// One or more shards referenced by the index are absent.
    #[error("Safetensors index references missing shards: {0:?}")]
    MissingShards(Vec<String>),
    /// Referenced shard set differs from directory contents.
    #[error("mixed or stale Safetensors shard set; referenced={referenced:?}, actual={actual:?}")]
    MixedShardSet {
        referenced: Vec<String>,
        actual: Vec<String>,
    },
    /// Index maps a tensor to a different shard than its header.
    #[error("Safetensors index mismatch for tensor {tensor} in shard {shard}")]
    IndexMismatch { tensor: String, shard: String },
    /// Tensor names in the index and shard headers differ.
    #[error(
        "Safetensors index tensor set differs from shard headers; indexed={indexed:?}, discovered={discovered:?}"
    )]
    IndexTensorSetMismatch {
        indexed: Vec<String>,
        discovered: Vec<String>,
    },
    /// A tensor name occurs in more than one shard.
    #[error("duplicate tensor name across Safetensors shards: {0}")]
    DuplicateTensor(String),
    /// Tensor dimensions overflow parameter count.
    #[error("parameter count overflow for tensor {0}")]
    ParameterOverflow(String),
    /// Safetensors header is too large.
    #[error("Safetensors header is too large: {0}")]
    HeaderTooLarge(PathBuf),
    /// Safetensors shard contains no tensor headers.
    #[error("Safetensors shard contains no tensors: {0}")]
    NoTensors(PathBuf),
    /// Individual tensor metadata is malformed.
    #[error("invalid tensor header for {tensor} in {path}: {source}")]
    InvalidTensorHeader {
        path: PathBuf,
        tensor: String,
        #[source]
        source: serde_json::Error,
    },
    /// Individual tensor metadata has the wrong Safetensors shape.
    #[error("invalid tensor metadata for {tensor} in {path}")]
    InvalidTensorMetadata { path: PathBuf, tensor: String },
    /// Deployment output is not a GGUF container.
    #[error("invalid GGUF artifact: {0}")]
    InvalidGguf(PathBuf),
    /// Tensor offsets do not exactly cover a valid shard payload.
    #[error("invalid or truncated Safetensors payload layout: {0}")]
    InvalidTensorLayout(PathBuf),
    /// Tensor dtype cannot be structurally validated.
    #[error("unsupported Safetensors dtype {dtype} for tensor {tensor}")]
    UnsupportedSafetensorsDtype { tensor: String, dtype: String },
    /// Manifest does not match the canonical files.
    #[error("substrate manifest does not match the canonical files")]
    ManifestMismatch,
    /// Manifest fields contradict their contained records.
    #[error("invalid substrate manifest: {0}")]
    InvalidSubstrateManifest(String),
    /// An immutable lineage ID already has a different canonical manifest.
    #[error("canonical manifest already exists with different contents: {0}")]
    ManifestConflict(PathBuf),
    /// Manifest serialization failed.
    #[error("failed to serialize substrate manifest: {0}")]
    Serialize(#[source] serde_json::Error),
    /// Persisted manifest JSON is malformed or violates the schema.
    #[error("invalid substrate manifest JSON: {0}")]
    ManifestJson(#[source] serde_json::Error),
    /// Persisted manifest omits a numeric schema version.
    #[error("substrate manifest is missing a numeric schema_version")]
    MissingSchemaVersion,
    /// An artifact ID already has a different append-only record.
    #[error("artifact manifest already exists with different contents: {0}")]
    ArtifactConflict(PathBuf),
    /// Artifact identity fields contradict each other.
    #[error("invalid artifact manifest: {0}")]
    InvalidArtifactManifest(String),
    /// A requested runtime capability has no eligible admitted backend.
    #[error("runtime capability unsupported: {0}")]
    RuntimeCapabilityUnsupported(String),
    /// The requested conceptual lineage does not match the selected base.
    #[error("base lineage mismatch: expected {expected}, found {actual}")]
    BaseLineageMismatch { expected: String, actual: String },
    /// A trainer exposed a parameter outside its approved allowlist.
    #[error("unexpected trainable parameter: {0}")]
    UnexpectedTrainableParameter(String),
    /// Training or adapter state contains NaN or infinity.
    #[error("non-finite training state: {0}")]
    NonFiniteTrainingState(String),
    /// A merged candidate contains no meaningful intended tensor change.
    #[error("successor merge is a no-op")]
    NoOpMerge,
    /// Live-adapter and merged-candidate logits exceed the configured tolerance.
    #[error("adapter/merged equivalence failed: delta {delta} exceeds tolerance {tolerance}")]
    EquivalenceFailure { delta: f64, tolerance: f64 },
    /// Candidate regression exceeds an admission threshold.
    #[error("successor regression gate failed for case {0}")]
    RegressionGateFailure(String),
    /// A successor has no immutable admission record.
    #[error("successor lineage is not admitted: {0}")]
    SuccessorNotAdmitted(String),
    /// A lifecycle manifest violates its versioned contract.
    #[error("invalid lifecycle manifest: {0}")]
    InvalidLifecycleManifest(String),
    /// A lifecycle record conflicts with an existing append-only record.
    #[error("lifecycle manifest already exists with different contents: {0}")]
    LifecycleConflict(PathBuf),
}
