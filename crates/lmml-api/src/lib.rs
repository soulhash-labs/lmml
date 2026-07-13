//! Shared HTTP API contracts for lmml nodes.
//!
//! This crate contains dependency-light serde DTOs used by the future
//! headless `lmml-node` binary, LMML TUI cluster views, and external clients
//! such as AgentQ. It intentionally avoids server, process, and AgentQ packet
//! logic so the contracts can remain stable across runtime implementations.
//!
//! # Example
//!
//! ```rust
//! use lmml_api::{InferRequest, TaskType};
//!
//! let request = InferRequest::new("Explain the current node status.");
//! assert_eq!(request.task_type, TaskType::General);
//! ```

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Current version string for the LMML node HTTP API.
pub const API_VERSION: &str = "lmml-node-api/v1";

/// Header used by clients to pass an idempotent request identifier.
pub const HEADER_REQUEST_ID: &str = "x-lmml-request-id";

/// Header used by nodes to identify the worker handling a response.
pub const HEADER_NODE_ID: &str = "x-lmml-node-id";

/// Hardware or runtime backend used by a node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    /// NVIDIA CUDA backend.
    Cuda,
    /// Apple Metal backend.
    Metal,
    /// AMD ROCm/HIP backend.
    Hip,
    /// Vulkan backend, often useful as a broad GPU fallback.
    Vulkan,
    /// CPU backend with AVX2 acceleration.
    CpuAvx2,
    /// CPU backend with AVX acceleration.
    CpuAvx,
    /// Portable CPU fallback backend.
    CpuFallback,
    /// Backend is not known or has not been detected.
    Unknown,
}

/// Operational role advertised by an LMML node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeRole {
    /// Operator workstation with the primary local GPU.
    WorkstationMainGpu,
    /// General LAN worker.
    LanWorker,
    /// Batch evaluator for offline or background work.
    BatchEvaluator,
    /// Embedding-specialized worker.
    EmbeddingWorker,
    /// Critic or review worker.
    CriticWorker,
    /// Router or coordinator node.
    Router,
    /// Node that also hosts the LMML TUI.
    TuiHost,
    /// Role is not known.
    Unknown,
}

/// Privacy tier declared by a node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyTier {
    /// Node should only receive LAN-local traffic.
    LanOnly,
    /// Node should only receive localhost traffic.
    LocalhostOnly,
    /// Node may call or receive internet-routed work.
    InternetAllowed,
}

/// Runtime health state for a node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    /// Node and local model runtime are ready.
    Ready,
    /// Node is reachable but one or more dependencies are degraded.
    Degraded,
    /// Node is starting a local dependency.
    Starting,
    /// Node is intentionally stopped.
    Stopped,
    /// Node is in an error state.
    Error,
}

/// GPU information advertised by a node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuDescriptor {
    /// Human-readable GPU name.
    pub name: String,
    /// Backend used for this GPU.
    pub backend: BackendKind,
    /// Optional architecture string, such as `sm_86` or `gfx1030`.
    pub arch: Option<String>,
    /// Total VRAM in MiB.
    pub vram_total_mb: u64,
    /// Free VRAM in MiB when the node can report it.
    pub vram_free_mb: Option<u64>,
}

/// Model metadata advertised by a node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDescriptor {
    /// Stable model identifier used in requests.
    pub id: String,
    /// Human-readable model name.
    pub name: String,
    /// Optional local path. Public LAN responses may omit this.
    pub path: Option<String>,
    /// Model architecture, such as `llama` or `qwen`.
    pub architecture: Option<String>,
    /// Quantization string, such as `Q4_K_M`.
    pub quantization: Option<String>,
    /// Context length in tokens.
    pub context_length: Option<u32>,
    /// Model size in bytes.
    pub size_bytes: Option<u64>,
    /// Whether the model is currently loaded by the local runtime.
    pub loaded: bool,
    /// Alternate names accepted by this node.
    pub aliases: Vec<String>,
}

/// AgentQ-related capabilities advertised by an LMML node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentQDescriptor {
    /// Whether AgentQ bridge behavior is enabled.
    pub enabled: bool,
    /// AgentQ bridge API version.
    pub api_version: String,
    /// AgentQ node name advertised by this LMML worker.
    pub node_name: String,
    /// AgentQ node kind, kept as a string to avoid coupling to AgentQ crates.
    pub node_kind: String,
    /// Optional CROWN cell role.
    pub crown_cell: Option<String>,
    /// Packet overhead in bytes when binary AgentQ packet routes are enabled.
    pub packet_overhead_bytes: Option<usize>,
    /// AgentQ-related endpoints exposed by the node.
    pub endpoints: Vec<String>,
    /// Tags useful for AgentQ routing.
    pub tags: Vec<String>,
}

/// Static and semi-static capabilities advertised by an LMML node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeCapabilities {
    /// LMML node HTTP API version.
    pub api_version: String,
    /// LMML binary or crate version.
    pub lmml_version: String,
    /// Node identifier.
    pub node_id: String,
    /// Human-readable node name.
    pub node_name: String,
    /// Public URL clients should use, if advertised.
    pub public_url: Option<String>,
    /// Roles this node can serve.
    pub roles: Vec<NodeRole>,
    /// Free-form tags used for routing.
    pub tags: Vec<String>,
    /// Node privacy tier.
    pub privacy: PrivacyTier,
    /// Primary backend.
    pub backend: BackendKind,
    /// GPU descriptors.
    pub gpus: Vec<GpuDescriptor>,
    /// Models known to this node.
    pub models: Vec<ModelDescriptor>,
    /// Maximum context length among known models.
    pub max_context_tokens: Option<u32>,
    /// True when `/v1/infer` is supported.
    pub supports_infer: bool,
    /// True when `/v1/chat/completions` proxy compatibility is supported.
    pub supports_chat_completions: bool,
    /// True when `/v1/embeddings` proxy compatibility is supported.
    pub supports_embeddings: bool,
    /// True when server lifecycle control is enabled.
    pub supports_server_control: bool,
    /// Whether API authentication is required for protected routes.
    pub auth_required: bool,
    /// llama.cpp commit used to build the local runtime, when known.
    pub llama_cpp_commit: Option<String>,
    /// Optional AgentQ bridge descriptor.
    pub agentq: Option<AgentQDescriptor>,
    /// Extension map for non-breaking additions.
    pub extra: BTreeMap<String, Value>,
}

/// Health response returned by `GET /v1/health`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HealthResponse {
    /// LMML node HTTP API version.
    pub api_version: String,
    /// Node identifier.
    pub node_id: String,
    /// Human-readable node name.
    pub node_name: String,
    /// Current node status.
    pub status: NodeStatus,
    /// Current UTC time as an RFC 3339 string.
    pub time_utc: String,
    /// Node process uptime in seconds.
    pub uptime_s: u64,
    /// Whether the local llama.cpp server is healthy.
    pub llama_healthy: bool,
    /// Active model, when known.
    pub active_model: Option<ModelDescriptor>,
    /// Optional human-readable status message.
    pub message: Option<String>,
}

/// Load and request counters returned by `GET /v1/load`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadResponse {
    /// Node identifier.
    pub node_id: String,
    /// Current node status.
    pub status: NodeStatus,
    /// CPU utilization percentage.
    pub cpu_usage_pct: f32,
    /// Total system memory in MiB.
    pub memory_total_mb: u64,
    /// Used system memory in MiB.
    pub memory_used_mb: u64,
    /// Current GPU descriptors.
    pub gpus: Vec<GpuDescriptor>,
    /// Number of currently running requests.
    pub running_requests: u32,
    /// Number of completed requests since process start.
    pub completed_requests: u64,
    /// Number of failed requests since process start.
    pub failed_requests: u64,
    /// Total input tokens processed since process start.
    pub tokens_in_total: u64,
    /// Total output tokens produced since process start.
    pub tokens_out_total: u64,
}

/// Inference task type used for scheduling and prompt shaping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskType {
    /// General inference.
    General,
    /// Planning work.
    Planning,
    /// Coding work.
    Coding,
    /// Critique or review work.
    Critique,
    /// Browser or navigation work.
    Browser,
    /// Research work.
    Research,
    /// Embedding generation.
    Embedding,
    /// Reranking work.
    Rerank,
    /// Summarization work.
    Summarization,
    /// JSON-constrained generation.
    Json,
    /// Batch evaluation.
    BatchEval,
}

/// Stable LMML-native inference request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InferRequest {
    /// Optional request ID supplied by the caller.
    pub request_id: Option<String>,
    /// Optional higher-level task ID.
    pub task_id: Option<String>,
    /// Optional agent or caller ID.
    pub agent_id: Option<String>,
    /// Scheduling or prompt-shaping task type.
    pub task_type: TaskType,
    /// Optional requested model ID.
    pub model: Option<String>,
    /// Optional system prompt.
    pub system: Option<String>,
    /// User prompt.
    pub prompt: String,
    /// Maximum output tokens.
    pub max_tokens: Option<u32>,
    /// Sampling temperature.
    pub temperature: Option<f32>,
    /// Nucleus sampling probability.
    pub top_p: Option<f32>,
    /// Optional deterministic seed.
    pub seed: Option<i64>,
    /// Optional stop strings.
    pub stop: Option<Vec<String>>,
    /// Optional response format hint, such as `json`.
    pub response_format: Option<String>,
    /// Extension metadata for routers and agents.
    pub metadata: BTreeMap<String, Value>,
}

impl InferRequest {
    /// Create a general inference request with conservative defaults.
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            ..Self::default()
        }
    }
}

impl Default for InferRequest {
    fn default() -> Self {
        Self {
            request_id: None,
            task_id: None,
            agent_id: None,
            task_type: TaskType::General,
            model: None,
            system: None,
            prompt: String::new(),
            max_tokens: Some(1024),
            temperature: Some(0.2),
            top_p: Some(0.95),
            seed: None,
            stop: None,
            response_format: None,
            metadata: BTreeMap::new(),
        }
    }
}

/// Stable LMML-native inference response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InferResponse {
    /// Request ID from the caller or assigned by the node.
    pub request_id: String,
    /// Node that handled the request.
    pub node_id: String,
    /// Model used for generation.
    pub model: Option<String>,
    /// Generated text.
    pub output: String,
    /// Raw upstream response, when proxying preserves it.
    pub raw: Option<Value>,
    /// End-to-end request latency in milliseconds.
    pub latency_ms: u64,
    /// Input token count when reported by the upstream runtime.
    pub tokens_in: Option<u64>,
    /// Output token count when reported by the upstream runtime.
    pub tokens_out: Option<u64>,
    /// Finish reason when reported by the upstream runtime.
    pub finish_reason: Option<String>,
}

/// Embedding request accepted by LMML nodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingRequest {
    /// Optional request ID supplied by the caller.
    pub request_id: Option<String>,
    /// Input strings to embed.
    pub input: Vec<String>,
    /// Optional requested model ID.
    pub model: Option<String>,
    /// Optional encoding format hint.
    pub encoding_format: Option<String>,
}

/// Embedding response returned by LMML nodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingResponse {
    /// Request ID from the caller or assigned by the node.
    pub request_id: String,
    /// Node that handled the request.
    pub node_id: String,
    /// Raw upstream embedding payload.
    pub raw: Value,
    /// End-to-end request latency in milliseconds.
    pub latency_ms: u64,
}

/// Server lifecycle action requested from a node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerAction {
    /// Start the managed local server.
    Start,
    /// Stop the managed local server.
    Stop,
    /// Restart the managed local server.
    Restart,
    /// Return current managed server status.
    Status,
}

/// Request body for `POST /v1/server/control`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerControlRequest {
    /// Requested lifecycle action.
    pub action: ServerAction,
}

/// Response body for `POST /v1/server/control`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerControlResponse {
    /// Node that handled the control request.
    pub node_id: String,
    /// Node status after the action.
    pub status: NodeStatus,
    /// Human-readable action result.
    pub message: String,
}

/// Top-level error response returned by LMML node APIs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorResponse {
    /// Structured error body.
    pub error: ApiErrorBody,
}

/// Structured error body returned by LMML node APIs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiErrorBody {
    /// Stable machine-readable error code.
    pub code: String,
    /// Human-readable error message.
    pub message: String,
    /// Request ID related to the failure, when known.
    pub request_id: Option<String>,
    /// Optional additional structured details.
    pub details: Option<BTreeMap<String, String>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn infer_request_round_trips_with_request_id() {
        let mut request = InferRequest::new("say ok");
        request.request_id = Some("req-1".to_string());
        request.task_type = TaskType::Critique;
        request
            .metadata
            .insert("privacy".to_string(), json!("lan_only"));

        let text = serde_json::to_string(&request).expect("serialize infer request");
        let decoded: InferRequest = serde_json::from_str(&text).expect("deserialize infer request");

        assert_eq!(decoded, request);
        assert_eq!(decoded.request_id.as_deref(), Some("req-1"));
    }

    #[test]
    fn capabilities_include_version_and_auth_fields() {
        let capabilities = NodeCapabilities {
            api_version: API_VERSION.to_string(),
            lmml_version: "0.1.0".to_string(),
            node_id: "node-a".to_string(),
            node_name: "Node A".to_string(),
            public_url: Some("http://127.0.0.1:8101".to_string()),
            roles: vec![NodeRole::LanWorker],
            tags: vec!["lmml".to_string()],
            privacy: PrivacyTier::LocalhostOnly,
            backend: BackendKind::CpuFallback,
            gpus: Vec::new(),
            models: Vec::new(),
            max_context_tokens: None,
            supports_infer: true,
            supports_chat_completions: false,
            supports_embeddings: false,
            supports_server_control: false,
            auth_required: true,
            llama_cpp_commit: Some("abc123".to_string()),
            agentq: None,
            extra: BTreeMap::new(),
        };

        let value = serde_json::to_value(&capabilities).expect("serialize capabilities");

        assert_eq!(value["api_version"], API_VERSION);
        assert_eq!(value["lmml_version"], "0.1.0");
        assert_eq!(value["llama_cpp_commit"], "abc123");
        assert_eq!(value["auth_required"], true);
    }

    #[test]
    fn server_control_request_uses_snake_case_action() {
        let request = ServerControlRequest {
            action: ServerAction::Restart,
        };

        let value = serde_json::to_value(request).expect("serialize control request");

        assert_eq!(value, json!({ "action": "restart" }));
    }

    #[test]
    fn error_response_round_trips() {
        let response = ErrorResponse {
            error: ApiErrorBody {
                code: "bad_request".to_string(),
                message: "prompt is empty".to_string(),
                request_id: Some("req-2".to_string()),
                details: None,
            },
        };

        let text = serde_json::to_string(&response).expect("serialize error response");
        let decoded: ErrorResponse =
            serde_json::from_str(&text).expect("deserialize error response");

        assert_eq!(decoded, response);
    }
}
