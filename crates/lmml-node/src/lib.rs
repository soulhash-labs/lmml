//! Headless read-only LMML node HTTP API.
//!
//! This crate exposes safe node-state endpoints for LAN schedulers and future
//! LMML cluster views. Phase 2A intentionally provides only read-only routes:
//! health, capabilities, load, and model inventory. Inference and server
//! lifecycle control are separate milestones so auth and overload behavior can
//! be hardened before write/control routes exist.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use lmml_api::{
    AgentQDescriptor, ApiErrorBody, BackendKind, ErrorResponse, GpuDescriptor, HealthResponse,
    LoadResponse, ModelDescriptor, NodeCapabilities, NodeRole, NodeStatus, PrivacyTier,
    API_VERSION,
};
use lmml_detect::{BuildBackend, GpuInfo, SystemProfile};
use lmml_models::{ModelEntry, ModelRegistry};
use lmml_state::ModelState;
use thiserror::Error;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// Default HTTP port for the headless LMML node API.
pub const DEFAULT_NODE_PORT: u16 = 8101;

/// Runtime configuration for a read-only LMML node API server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeConfig {
    /// HTTP listen host.
    pub host: String,
    /// HTTP listen port.
    pub port: u16,
    /// Stable node identifier advertised to clients.
    pub node_id: String,
    /// Human-readable node name advertised to clients.
    pub node_name: String,
    /// Optional public URL clients should use.
    pub public_url: Option<String>,
    /// Directories scanned for local GGUF models.
    pub model_dirs: Vec<PathBuf>,
    /// Optional bearer token required for protected read-only routes.
    pub api_key: Option<String>,
    /// Explicit development escape hatch for non-local unauthenticated binds.
    pub allow_unsafe_lan_without_auth: bool,
    /// Free-form node routing tags.
    pub tags: Vec<String>,
    /// Roles advertised by this node.
    pub roles: Vec<NodeRole>,
    /// Optional AgentQ bridge advertisement. Routes are not enabled in Phase 2A.
    pub agentq: Option<AgentQDescriptor>,
}

impl NodeConfig {
    /// Validate security-sensitive node settings before binding a socket.
    pub fn validate(&self) -> Result<(), NodeConfigError> {
        if !is_local_bind(&self.host)
            && api_key(self).is_none()
            && !self.allow_unsafe_lan_without_auth
        {
            return Err(NodeConfigError::ApiKeyRequiredForLanBind {
                host: self.host.clone(),
            });
        }
        Ok(())
    }

    /// Return the socket address used by the HTTP server.
    pub fn socket_addr(&self) -> Result<SocketAddr, NodeConfigError> {
        socket_addr_string(&self.host, self.port)
            .parse()
            .map_err(|_| NodeConfigError::InvalidSocketAddress {
                host: self.host.clone(),
                port: self.port,
            })
    }
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: DEFAULT_NODE_PORT,
            node_id: default_node_id(),
            node_name: default_node_name(),
            public_url: None,
            model_dirs: vec![default_model_dir()],
            api_key: None,
            allow_unsafe_lan_without_auth: false,
            tags: vec!["lmml".to_string()],
            roles: vec![NodeRole::LanWorker],
            agentq: None,
        }
    }
}

/// Security or address validation error for [`NodeConfig`].
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum NodeConfigError {
    /// LAN-visible node APIs must be authenticated unless explicitly unsafe.
    #[error("API key required when binding lmml-node to non-local host {host}")]
    ApiKeyRequiredForLanBind {
        /// Host that would expose the node beyond localhost.
        host: String,
    },
    /// Host and port could not be parsed as a socket address.
    #[error("invalid node socket address {host}:{port}")]
    InvalidSocketAddress {
        /// Configured host.
        host: String,
        /// Configured port.
        port: u16,
    },
}

/// Immutable read-only snapshot served by the node API.
#[derive(Debug, Clone)]
pub struct NodeSnapshot {
    /// Node configuration used to build the snapshot.
    pub config: NodeConfig,
    /// Detected host hardware and toolchain profile.
    pub system: SystemProfile,
    /// GGUF models known to the node.
    pub models: Vec<ModelEntry>,
    /// Process start time for uptime reporting.
    pub started_at: Instant,
}

impl NodeSnapshot {
    /// Create a snapshot by probing hardware and scanning configured model dirs.
    #[tracing::instrument(skip(config), fields(node_id = %config.node_id))]
    pub async fn detect(config: NodeConfig) -> Result<Self, NodeConfigError> {
        config.validate()?;
        let system = SystemProfile::detect().await;
        let models = scan_models(&config.model_dirs).await;
        Ok(Self {
            config,
            system,
            models,
            started_at: Instant::now(),
        })
    }

    /// Build the health DTO for the current snapshot.
    pub fn health(&self) -> HealthResponse {
        HealthResponse {
            api_version: API_VERSION.to_string(),
            node_id: self.config.node_id.clone(),
            node_name: self.config.node_name.clone(),
            status: NodeStatus::Degraded,
            time_utc: utc_now_rfc3339(),
            uptime_s: self.started_at.elapsed().as_secs(),
            llama_healthy: false,
            active_model: None,
            message: Some(
                "read-only node API active; llama-server is not managed in Phase 2A".to_string(),
            ),
        }
    }

    /// Build the capabilities DTO for the current snapshot.
    pub fn capabilities(&self) -> NodeCapabilities {
        let models = model_descriptors(&self.models, false);
        NodeCapabilities {
            api_version: API_VERSION.to_string(),
            lmml_version: env!("CARGO_PKG_VERSION").to_string(),
            node_id: self.config.node_id.clone(),
            node_name: self.config.node_name.clone(),
            public_url: self.config.public_url.clone(),
            roles: self.config.roles.clone(),
            tags: self.config.tags.clone(),
            privacy: privacy_tier(&self.config),
            backend: backend_kind(&self.system.recommended_backend()),
            gpus: gpu_descriptors(&self.system.gpus),
            max_context_tokens: models.iter().filter_map(|model| model.context_length).max(),
            models,
            supports_infer: false,
            supports_chat_completions: false,
            supports_embeddings: false,
            supports_server_control: false,
            auth_required: api_key(&self.config).is_some(),
            llama_cpp_commit: None,
            agentq: self.config.agentq.clone(),
            extra: BTreeMap::new(),
        }
    }

    /// Build the load DTO for the current snapshot.
    pub fn load(&self) -> LoadResponse {
        LoadResponse {
            node_id: self.config.node_id.clone(),
            status: NodeStatus::Degraded,
            cpu_usage_pct: 0.0,
            memory_total_mb: self.system.memory.total_mb,
            memory_used_mb: self
                .system
                .memory
                .total_mb
                .saturating_sub(self.system.memory.available_mb),
            gpus: gpu_descriptors(&self.system.gpus),
            running_requests: 0,
            completed_requests: 0,
            failed_requests: 0,
            tokens_in_total: 0,
            tokens_out_total: 0,
        }
    }

    /// Build model inventory DTOs with local paths included only for localhost.
    pub fn models(&self) -> Vec<ModelDescriptor> {
        model_descriptors(&self.models, is_local_bind(&self.config.host))
    }
}

/// Shared application state for the node HTTP router.
#[derive(Debug, Clone)]
pub struct NodeAppState {
    snapshot: Arc<NodeSnapshot>,
}

impl NodeAppState {
    /// Create router state from an immutable snapshot.
    pub fn new(snapshot: NodeSnapshot) -> Self {
        Self {
            snapshot: Arc::new(snapshot),
        }
    }
}

/// Create the read-only LMML node HTTP router.
pub fn router(state: NodeAppState) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/capabilities", get(capabilities))
        .route("/v1/load", get(load))
        .route("/v1/models", get(models))
        .with_state(state)
}

async fn health(State(state): State<NodeAppState>) -> Json<HealthResponse> {
    Json(state.snapshot.health())
}

async fn capabilities(
    State(state): State<NodeAppState>,
    headers: HeaderMap,
) -> Result<Json<NodeCapabilities>, ApiFailure> {
    authorize(&state.snapshot.config, &headers)?;
    Ok(Json(state.snapshot.capabilities()))
}

async fn load(
    State(state): State<NodeAppState>,
    headers: HeaderMap,
) -> Result<Json<LoadResponse>, ApiFailure> {
    authorize(&state.snapshot.config, &headers)?;
    Ok(Json(state.snapshot.load()))
}

async fn models(
    State(state): State<NodeAppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ModelDescriptor>>, ApiFailure> {
    authorize(&state.snapshot.config, &headers)?;
    Ok(Json(state.snapshot.models()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ApiFailure {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl IntoResponse for ApiFailure {
    fn into_response(self) -> Response {
        let body = ErrorResponse {
            error: ApiErrorBody {
                code: self.code.to_string(),
                message: self.message,
                request_id: None,
                details: None,
            },
        };
        (self.status, Json(body)).into_response()
    }
}

fn authorize(config: &NodeConfig, headers: &HeaderMap) -> Result<(), ApiFailure> {
    let Some(expected) = api_key(config) else {
        return Ok(());
    };
    let header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let actual = header.strip_prefix("Bearer ").unwrap_or_default();
    if constant_time_eq(actual.as_bytes(), expected.as_bytes()) {
        return Ok(());
    }
    Err(ApiFailure {
        status: StatusCode::UNAUTHORIZED,
        code: "unauthorized",
        message: "missing or invalid bearer token".to_string(),
    })
}

fn is_local_bind(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

fn api_key(config: &NodeConfig) -> Option<&str> {
    config
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
}

fn socket_addr_string(host: &str, port: u16) -> String {
    match host {
        "localhost" => format!("127.0.0.1:{port}"),
        "::1" => format!("[::1]:{port}"),
        host if host.contains(':') && !host.starts_with('[') => format!("[{host}]:{port}"),
        host => format!("{host}:{port}"),
    }
}

fn privacy_tier(config: &NodeConfig) -> PrivacyTier {
    if is_local_bind(&config.host) {
        PrivacyTier::LocalhostOnly
    } else {
        PrivacyTier::LanOnly
    }
}

fn backend_kind(backend: &BuildBackend) -> BackendKind {
    match backend {
        BuildBackend::Cuda { .. } => BackendKind::Cuda,
        BuildBackend::Metal => BackendKind::Metal,
        BuildBackend::Vulkan => BackendKind::Vulkan,
        BuildBackend::CpuAvx2 => BackendKind::CpuAvx2,
        BuildBackend::CpuAvx => BackendKind::CpuAvx,
        BuildBackend::CpuFallback => BackendKind::CpuFallback,
    }
}

fn gpu_descriptors(gpus: &[GpuInfo]) -> Vec<GpuDescriptor> {
    gpus.iter()
        .map(|gpu| GpuDescriptor {
            name: gpu.name.clone(),
            backend: BackendKind::Cuda,
            arch: gpu.arch.map(str::to_string),
            vram_total_mb: gpu.memory_total_mb,
            vram_free_mb: None,
        })
        .collect()
}

fn model_descriptors(models: &[ModelEntry], include_paths: bool) -> Vec<ModelDescriptor> {
    models
        .iter()
        .map(|model| ModelDescriptor {
            id: model_id(model),
            name: model.name.clone(),
            path: include_paths.then(|| model.path.to_string_lossy().into_owned()),
            architecture: model.architecture.clone(),
            quantization: Some(model.quant.clone()),
            context_length: model.context_length,
            size_bytes: Some(model.size_bytes),
            loaded: false,
            aliases: Vec::new(),
        })
        .collect()
}

fn model_id(model: &ModelEntry) -> String {
    let filename = model
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&model.name)
        .to_string();
    let stable_path = std::fs::canonicalize(&model.path).unwrap_or_else(|_| model.path.clone());
    let hash = fnv1a64(stable_path.to_string_lossy().as_bytes());
    format!("{filename}-{hash:016x}")
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    bytes.iter().fold(FNV_OFFSET_BASIS, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    })
}

async fn scan_models(model_dirs: &[PathBuf]) -> Vec<ModelEntry> {
    let Some((first, rest)) = model_dirs.split_first() else {
        return Vec::new();
    };
    let registry = ModelRegistry {
        models_dir: first.clone(),
        aliases: rest.to_vec(),
    };
    registry.scan().await
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let max_len = left.len().max(right.len());
    let mut diff = left.len() ^ right.len();
    for index in 0..max_len {
        let left_byte = left.get(index).copied().unwrap_or(0);
        let right_byte = right.get(index).copied().unwrap_or(0);
        diff |= usize::from(left_byte ^ right_byte);
    }
    diff == 0
}

fn default_node_id() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "lmml-node".to_string())
}

fn default_node_name() -> String {
    default_node_id()
}

fn default_model_dir() -> PathBuf {
    ModelState::default().models_dir
}

fn utc_now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::response::Response;
    use lmml_detect::{
        CpuFeatures, CudaCompatibility, DiskInfo, MemInfo, MetalSupport, NvidiaDeviceNodes,
        VulkanSupport,
    };
    use pretty_assertions::assert_eq;
    use tower::ServiceExt;

    #[test]
    fn default_config_uses_localhost_without_auth() {
        let config = NodeConfig::default();

        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, DEFAULT_NODE_PORT);
        assert_eq!(config.api_key, None);
        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn lan_bind_requires_auth_by_default() {
        let config = NodeConfig {
            host: "0.0.0.0".to_string(),
            api_key: Some(String::new()),
            ..NodeConfig::default()
        };

        assert_eq!(
            config.validate(),
            Err(NodeConfigError::ApiKeyRequiredForLanBind {
                host: "0.0.0.0".to_string()
            })
        );
    }

    #[test]
    fn lan_bind_allows_auth() {
        let config = NodeConfig {
            host: "0.0.0.0".to_string(),
            api_key: Some("secret".to_string()),
            ..NodeConfig::default()
        };

        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn constant_time_compare_checks_length_and_content() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"Secret"));
        assert!(!constant_time_eq(b"secret", b"secret2"));
    }

    #[test]
    fn socket_addr_supports_localhost_and_ipv6() {
        assert_eq!(socket_addr_string("localhost", 8101), "127.0.0.1:8101");
        assert_eq!(socket_addr_string("::1", 8101), "[::1]:8101");
    }

    #[tokio::test]
    async fn health_is_public_but_capabilities_require_auth() {
        let snapshot = test_snapshot(NodeConfig {
            api_key: Some("secret".to_string()),
            ..NodeConfig::default()
        });
        let app = router(NodeAppState::new(snapshot));

        let health = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/health")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("health response");
        assert_eq!(health.status(), StatusCode::OK);

        let denied = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/capabilities")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("capabilities response");
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);

        let allowed = app
            .oneshot(
                Request::builder()
                    .uri("/v1/capabilities")
                    .header(axum::http::header::AUTHORIZATION, "Bearer secret")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("capabilities response");
        assert_eq!(allowed.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn models_hide_paths_for_lan_binds() {
        let snapshot = test_snapshot(NodeConfig {
            host: "0.0.0.0".to_string(),
            api_key: Some("secret".to_string()),
            ..NodeConfig::default()
        });
        let app = router(NodeAppState::new(snapshot));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/models")
                    .header(axum::http::header::AUTHORIZATION, "Bearer secret")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("models response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        let models: Vec<ModelDescriptor> = serde_json::from_str(&body).expect("models json");

        assert_eq!(models.len(), 1);
        assert!(models[0].id.starts_with("qwen.gguf-"));
        assert_eq!(models[0].path, None);
    }

    #[test]
    fn duplicate_filenames_get_unique_stable_ids() {
        let models = vec![
            ModelEntry {
                path: PathBuf::from("/models/a/qwen.gguf"),
                name: "qwen.gguf".to_string(),
                size_bytes: 1024,
                quant: "Q8_0".to_string(),
                context_length: Some(262_144),
                architecture: Some("qwen".to_string()),
                aliased: false,
            },
            ModelEntry {
                path: PathBuf::from("/models/b/qwen.gguf"),
                name: "qwen.gguf".to_string(),
                size_bytes: 1024,
                quant: "Q8_0".to_string(),
                context_length: Some(262_144),
                architecture: Some("qwen".to_string()),
                aliased: false,
            },
        ];

        let descriptors = model_descriptors(&models, false);

        assert_eq!(descriptors[0].name, "qwen.gguf");
        assert_eq!(descriptors[1].name, "qwen.gguf");
        assert_ne!(descriptors[0].id, descriptors[1].id);
        assert_eq!(descriptors[0].path, None);
        assert_eq!(descriptors[1].path, None);
    }

    #[test]
    fn default_model_dir_matches_lmml_state() {
        assert_eq!(default_model_dir(), ModelState::default().models_dir);
    }

    fn test_snapshot(config: NodeConfig) -> NodeSnapshot {
        NodeSnapshot {
            config,
            system: test_system_profile(),
            models: vec![ModelEntry {
                path: PathBuf::from("/models/qwen.gguf"),
                name: "qwen.gguf".to_string(),
                size_bytes: 1024,
                quant: "Q8_0".to_string(),
                context_length: Some(262_144),
                architecture: Some("qwen".to_string()),
                aliased: false,
            }],
            started_at: Instant::now(),
        }
    }

    fn test_system_profile() -> SystemProfile {
        SystemProfile {
            compiler: None,
            cmake: None,
            git: None,
            cuda: CudaCompatibility::NoGpu,
            gpus: Vec::new(),
            gpu_probe_error: None,
            nvidia_devices: NvidiaDeviceNodes::default(),
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
                model: "test cpu".to_string(),
                cores: 1,
                threads: 1,
                avx: false,
                avx2: false,
                avx512: false,
                neon: false,
                features: Vec::new(),
            },
            memory: MemInfo {
                total_mb: 1024,
                available_mb: 512,
            },
            disk: DiskInfo {
                available_bytes: 1024,
                path: PathBuf::from("."),
            },
        }
    }

    async fn response_text(response: Response) -> String {
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        String::from_utf8(bytes.to_vec()).expect("utf8 body")
    }
}
