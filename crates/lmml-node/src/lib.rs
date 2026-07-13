//! Headless LMML node HTTP API.
//!
//! This crate exposes node-state endpoints and the canonical LMML-native
//! inference proxy for LAN schedulers and future LMML cluster views. It proxies
//! inference to an already-running local `llama-server`; server lifecycle
//! control remains a separate milestone.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::{HeaderMap, Request as AxumRequest, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use lmml_api::{
    AgentQDescriptor, ApiErrorBody, BackendKind, ErrorResponse, GpuDescriptor, HealthResponse,
    InferRequest, InferResponse, LoadResponse, ModelDescriptor, NodeCapabilities, NodeRole,
    NodeStatus, PrivacyTier, ServerAction, ServerControlRequest, ServerControlResponse,
    API_VERSION, HEADER_REQUEST_ID,
};
use lmml_detect::{BuildBackend, GpuInfo, SystemProfile};
use lmml_models::{ModelEntry, ModelRegistry};
use lmml_state::ModelState;
use serde_json::{json, Value};
use thiserror::Error;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// Default HTTP port for the headless LMML node API.
pub const DEFAULT_NODE_PORT: u16 = 8101;

/// Default local llama-server base URL used by the inference proxy.
pub const DEFAULT_LLAMA_BASE_URL: &str = "http://127.0.0.1:1200";

/// Default inference proxy timeout in milliseconds.
pub const DEFAULT_INFER_TIMEOUT_MS: u64 = 7_200_000;

/// Maximum accepted LMML node proxy request body size.
pub const MAX_PROXY_BODY_BYTES: usize = 1024 * 1024;

const HEALTH_CHECK_TIMEOUT_MS: u64 = 500;

/// Runtime configuration for an LMML node API server.
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
    /// Local llama-server base URL used by `POST /v1/infer`.
    pub llama_base_url: String,
    /// Timeout for proxied inference requests in milliseconds.
    pub infer_timeout_ms: u64,
    /// Enable explicit managed-server lifecycle control routes.
    pub enable_server_control: bool,
    /// Optional bearer token required for protected node routes.
    pub api_key: Option<String>,
    /// Explicit development escape hatch for non-local unauthenticated binds.
    pub allow_unsafe_lan_without_auth: bool,
    /// Free-form node routing tags.
    pub tags: Vec<String>,
    /// Roles advertised by this node.
    pub roles: Vec<NodeRole>,
    /// Optional AgentQ bridge advertisement. Routes are not enabled in Phase 2B.
    pub agentq: Option<AgentQDescriptor>,
}

impl NodeConfig {
    /// Validate security-sensitive node settings before binding a socket.
    pub fn validate(&self) -> Result<(), NodeConfigError> {
        if self.enable_server_control && api_key(self).is_none() {
            return Err(NodeConfigError::ApiKeyRequiredForServerControl);
        }
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
            llama_base_url: DEFAULT_LLAMA_BASE_URL.to_string(),
            infer_timeout_ms: DEFAULT_INFER_TIMEOUT_MS,
            enable_server_control: false,
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
    /// Server control requires bearer authentication even on localhost.
    #[error("API key required when enabling lmml-node server control")]
    ApiKeyRequiredForServerControl,
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
    pub fn health(&self, llama_healthy: bool) -> HealthResponse {
        let status = if llama_healthy {
            NodeStatus::Ready
        } else {
            NodeStatus::Degraded
        };
        let message = if llama_healthy {
            "node API active; llama-server proxy is healthy"
        } else {
            "node API active; llama-server proxy health is unavailable"
        };
        HealthResponse {
            api_version: API_VERSION.to_string(),
            node_id: self.config.node_id.clone(),
            node_name: self.config.node_name.clone(),
            status,
            time_utc: utc_now_rfc3339(),
            uptime_s: self.started_at.elapsed().as_secs(),
            llama_healthy,
            active_model: None,
            message: Some(message.to_string()),
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
            supports_infer: true,
            supports_chat_completions: true,
            supports_embeddings: true,
            supports_server_control: self.config.enable_server_control,
            auth_required: api_key(&self.config).is_some(),
            llama_cpp_commit: None,
            agentq: self.config.agentq.clone(),
            extra: BTreeMap::new(),
        }
    }

    /// Build the load DTO for the current snapshot.
    pub fn load(&self, llama_healthy: bool) -> LoadResponse {
        LoadResponse {
            node_id: self.config.node_id.clone(),
            status: if llama_healthy {
                NodeStatus::Ready
            } else {
                NodeStatus::Degraded
            },
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
    client: reqwest::Client,
}

impl NodeAppState {
    /// Create router state from an immutable snapshot.
    pub fn new(snapshot: NodeSnapshot) -> Self {
        Self {
            snapshot: Arc::new(snapshot),
            client: reqwest::Client::new(),
        }
    }
}

/// Create the LMML node HTTP router.
pub fn router(state: NodeAppState) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/capabilities", get(capabilities))
        .route("/v1/load", get(load))
        .route("/v1/models", get(models))
        .route("/v1/infer", post(infer))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/embeddings", post(embeddings))
        .route("/v1/server/control", post(server_control))
        .with_state(state)
}

async fn health(State(state): State<NodeAppState>) -> Json<HealthResponse> {
    let llama_healthy = upstream_llama_healthy(&state).await;
    Json(state.snapshot.health(llama_healthy))
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
    let llama_healthy = upstream_llama_healthy(&state).await;
    Ok(Json(state.snapshot.load(llama_healthy)))
}

async fn models(
    State(state): State<NodeAppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ModelDescriptor>>, ApiFailure> {
    authorize(&state.snapshot.config, &headers)?;
    Ok(Json(state.snapshot.models()))
}

async fn infer(
    State(state): State<NodeAppState>,
    request: AxumRequest<axum::body::Body>,
) -> Result<Json<InferResponse>, ApiFailure> {
    let body = read_authorized_body(&state.snapshot.config, request).await?;
    let request = serde_json::from_slice::<InferRequest>(&body.bytes).map_err(|error| {
        ApiFailure::new(
            StatusCode::BAD_REQUEST,
            "invalid_json",
            format!("invalid inference request JSON: {error}"),
            body.request_id,
        )
    })?;
    let response = proxy_infer(&state, request).await?;
    Ok(Json(response))
}

async fn chat_completions(
    State(state): State<NodeAppState>,
    request: AxumRequest<axum::body::Body>,
) -> Result<Response, ApiFailure> {
    proxy_json_passthrough(&state, request, "/v1/chat/completions").await
}

async fn embeddings(
    State(state): State<NodeAppState>,
    request: AxumRequest<axum::body::Body>,
) -> Result<Response, ApiFailure> {
    proxy_json_passthrough(&state, request, "/v1/embeddings").await
}

async fn server_control(
    State(state): State<NodeAppState>,
    request: AxumRequest<axum::body::Body>,
) -> Result<Json<ServerControlResponse>, ApiFailure> {
    let request_id = request_id_from_headers(request.headers());
    authorize(&state.snapshot.config, request.headers())?;
    if !state.snapshot.config.enable_server_control {
        return Err(ApiFailure::new(
            StatusCode::NOT_FOUND,
            "server_control_disabled",
            "server control is disabled for this lmml-node",
            request_id,
        ));
    }

    let body = read_authorized_body(&state.snapshot.config, request).await?;
    let request = serde_json::from_slice::<ServerControlRequest>(&body.bytes).map_err(|error| {
        ApiFailure::new(
            StatusCode::BAD_REQUEST,
            "invalid_server_control_request",
            format!("invalid server control request JSON: {error}"),
            body.request_id,
        )
    })?;
    let response = handle_server_control(&state, request).await?;
    Ok(Json(response))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ApiFailure {
    status: StatusCode,
    code: &'static str,
    message: String,
    request_id: Option<String>,
}

struct AuthorizedBody {
    request_id: Option<String>,
    bytes: Vec<u8>,
}

impl ApiFailure {
    fn new(
        status: StatusCode,
        code: &'static str,
        message: impl Into<String>,
        request_id: Option<String>,
    ) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            request_id,
        }
    }
}

impl IntoResponse for ApiFailure {
    fn into_response(self) -> Response {
        let body = ErrorResponse {
            error: ApiErrorBody {
                code: self.code.to_string(),
                message: self.message,
                request_id: self.request_id,
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
    Err(ApiFailure::new(
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        "missing or invalid bearer token",
        None,
    ))
}

async fn read_authorized_body(
    config: &NodeConfig,
    request: AxumRequest<axum::body::Body>,
) -> Result<AuthorizedBody, ApiFailure> {
    let request_id = request_id_from_headers(request.headers());
    authorize(config, request.headers())?;
    let bytes = axum::body::to_bytes(request.into_body(), MAX_PROXY_BODY_BYTES)
        .await
        .map_err(|error| {
            ApiFailure::new(
                StatusCode::BAD_REQUEST,
                "invalid_body",
                format!("failed to read request body: {error}"),
                request_id.clone(),
            )
        })?;
    Ok(AuthorizedBody {
        request_id,
        bytes: bytes.to_vec(),
    })
}

async fn proxy_json_passthrough(
    state: &NodeAppState,
    request: AxumRequest<axum::body::Body>,
    upstream_path: &str,
) -> Result<Response, ApiFailure> {
    let body = read_authorized_body(&state.snapshot.config, request).await?;
    ensure_json_body(&body)?;
    let url = upstream_url(&state.snapshot.config.llama_base_url, upstream_path);
    let timeout = Duration::from_millis(state.snapshot.config.infer_timeout_ms);
    let upstream_body = body.bytes.clone();
    let upstream_request_id = body.request_id.clone();
    let upstream = tokio::time::timeout(timeout, async {
        let mut request = state
            .client
            .post(url)
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(upstream_body);
        if let Some(request_id) = upstream_request_id.as_deref() {
            request = request.header(HEADER_REQUEST_ID, request_id);
        }
        let response = request.send().await?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let bytes = response.bytes().await?;
        Ok::<_, reqwest::Error>((status, content_type, bytes.to_vec()))
    })
    .await
    .map_err(|_| {
        ApiFailure::new(
            StatusCode::GATEWAY_TIMEOUT,
            "upstream_timeout",
            "llama-server compatibility request timed out",
            body.request_id.clone(),
        )
    })?
    .map_err(|error| {
        ApiFailure::new(
            StatusCode::BAD_GATEWAY,
            "upstream_request_failed",
            format!("failed to call llama-server: {error}"),
            body.request_id.clone(),
        )
    })?;

    let (status, content_type, bytes) = upstream;
    if !(200..300).contains(&status) {
        return Err(ApiFailure::new(
            StatusCode::BAD_GATEWAY,
            "upstream_error",
            format!("llama-server returned HTTP {status}"),
            body.request_id,
        ));
    }

    let status = StatusCode::from_u16(status).map_err(|error| {
        ApiFailure::new(
            StatusCode::BAD_GATEWAY,
            "upstream_invalid_status",
            format!("llama-server returned invalid HTTP status: {error}"),
            body.request_id.clone(),
        )
    })?;
    let mut builder = Response::builder().status(status);
    if let Some(content_type) = content_type {
        builder = builder.header(axum::http::header::CONTENT_TYPE, content_type);
    }
    builder
        .body(axum::body::Body::from(bytes))
        .map_err(|error| {
            ApiFailure::new(
                StatusCode::BAD_GATEWAY,
                "passthrough_response_failed",
                format!("failed to build passthrough response: {error}"),
                body.request_id,
            )
        })
}

fn ensure_json_body(body: &AuthorizedBody) -> Result<(), ApiFailure> {
    serde_json::from_slice::<Value>(&body.bytes)
        .map(|_| ())
        .map_err(|error| {
            ApiFailure::new(
                StatusCode::BAD_REQUEST,
                "invalid_json",
                format!("invalid compatibility request JSON: {error}"),
                body.request_id.clone(),
            )
        })
}

async fn handle_server_control(
    state: &NodeAppState,
    request: ServerControlRequest,
) -> Result<ServerControlResponse, ApiFailure> {
    match request.action {
        ServerAction::Status => {
            let llama_healthy = upstream_llama_healthy(state).await;
            Ok(ServerControlResponse {
                node_id: state.snapshot.config.node_id.clone(),
                status: if llama_healthy {
                    NodeStatus::Ready
                } else {
                    NodeStatus::Degraded
                },
                message: if llama_healthy {
                    "llama-server is reachable through the configured proxy URL".to_string()
                } else {
                    "llama-server is not reachable through the configured proxy URL".to_string()
                },
            })
        }
        ServerAction::Start | ServerAction::Stop | ServerAction::Restart => Err(ApiFailure::new(
            StatusCode::NOT_IMPLEMENTED,
            "server_manager_unavailable",
            "server lifecycle actions require a managed lmml-server handle",
            None,
        )),
    }
}

async fn proxy_infer(
    state: &NodeAppState,
    request: InferRequest,
) -> Result<InferResponse, ApiFailure> {
    let request_id = normalized_request_id(request.request_id.as_deref());
    if request.prompt.trim().is_empty() {
        return Err(ApiFailure::new(
            StatusCode::BAD_REQUEST,
            "empty_prompt",
            "prompt must not be empty",
            Some(request_id),
        ));
    }

    let url = upstream_chat_completions_url(&state.snapshot.config.llama_base_url);
    let upstream_request = build_upstream_chat_request(&request);
    let timeout = Duration::from_millis(state.snapshot.config.infer_timeout_ms);
    let started = Instant::now();
    let upstream = tokio::time::timeout(timeout, async {
        let response = state
            .client
            .post(url)
            .header(HEADER_REQUEST_ID, request_id.as_str())
            .json(&upstream_request)
            .send()
            .await?;
        let status = response.status().as_u16();
        let body = response.text().await?;
        Ok::<_, reqwest::Error>((status, body))
    })
    .await
    .map_err(|_| {
        ApiFailure::new(
            StatusCode::GATEWAY_TIMEOUT,
            "upstream_timeout",
            "llama-server inference request timed out",
            Some(request_id.clone()),
        )
    })?
    .map_err(|error| {
        ApiFailure::new(
            StatusCode::BAD_GATEWAY,
            "upstream_request_failed",
            format!("failed to call llama-server: {error}"),
            Some(request_id.clone()),
        )
    })?;

    let (status, body) = upstream;
    if !(200..300).contains(&status) {
        return Err(ApiFailure::new(
            StatusCode::BAD_GATEWAY,
            "upstream_error",
            format!("llama-server returned HTTP {status}"),
            Some(request_id),
        ));
    }

    let value = serde_json::from_str::<Value>(&body).map_err(|error| {
        ApiFailure::new(
            StatusCode::BAD_GATEWAY,
            "upstream_invalid_json",
            format!("llama-server returned invalid JSON: {error}"),
            Some(request_id.clone()),
        )
    })?;

    map_upstream_chat_response(
        request_id,
        state.snapshot.config.node_id.clone(),
        request.model.as_deref(),
        value,
        started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
    )
}

async fn upstream_llama_healthy(state: &NodeAppState) -> bool {
    let timeout = Duration::from_millis(HEALTH_CHECK_TIMEOUT_MS);
    for path in ["/v1/health", "/health"] {
        let url = upstream_url(&state.snapshot.config.llama_base_url, path);
        let Ok(result) = tokio::time::timeout(timeout, state.client.get(url).send()).await else {
            continue;
        };
        let Ok(response) = result else {
            continue;
        };
        if response.status().is_success() {
            return true;
        }
    }
    false
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

fn request_id_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get(HEADER_REQUEST_ID)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
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

fn normalized_request_id(request_id: Option<&str>) -> String {
    request_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(next_request_id)
}

fn next_request_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};

    static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);
    let count = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("lmml-{nanos}-{count}")
}

fn upstream_chat_completions_url(base_url: &str) -> String {
    upstream_url(base_url, "/v1/chat/completions")
}

fn upstream_url(base_url: &str, path: &str) -> String {
    let base_url = base_url.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    if base_url.ends_with("/v1") && path.starts_with("v1/") {
        format!("{base_url}/{}", path.trim_start_matches("v1/"))
    } else {
        format!("{base_url}/{path}")
    }
}

fn build_upstream_chat_request(request: &InferRequest) -> Value {
    let mut messages = Vec::new();
    if let Some(system) = request
        .system
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        messages.push(json!({
            "role": "system",
            "content": system,
        }));
    }
    messages.push(json!({
        "role": "user",
        "content": request.prompt,
    }));

    let mut body = json!({
        "messages": messages,
        "stream": false,
    });
    let Some(object) = body.as_object_mut() else {
        return body;
    };

    if let Some(model) = request
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        object.insert("model".to_string(), json!(model));
    }
    if let Some(max_tokens) = request.max_tokens {
        object.insert("max_tokens".to_string(), json!(max_tokens));
    }
    if let Some(temperature) = request.temperature {
        object.insert("temperature".to_string(), json!(temperature));
    }
    if let Some(top_p) = request.top_p {
        object.insert("top_p".to_string(), json!(top_p));
    }
    if let Some(seed) = request.seed {
        object.insert("seed".to_string(), json!(seed));
    }
    if let Some(stop) = request.stop.as_ref().filter(|stop| !stop.is_empty()) {
        object.insert("stop".to_string(), json!(stop));
    }
    if let Some(format) = request
        .response_format
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let format_type = if format == "json" {
            "json_object"
        } else {
            format
        };
        object.insert(
            "response_format".to_string(),
            json!({
                "type": format_type,
            }),
        );
    }

    body
}

fn map_upstream_chat_response(
    request_id: String,
    node_id: String,
    requested_model: Option<&str>,
    value: Value,
    latency_ms: u64,
) -> Result<InferResponse, ApiFailure> {
    let output = value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/choices/0/text").and_then(Value::as_str))
        .ok_or_else(|| {
            ApiFailure::new(
                StatusCode::BAD_GATEWAY,
                "upstream_invalid_response",
                "llama-server response did not include generated text",
                Some(request_id.clone()),
            )
        })?
        .to_string();
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| requested_model.map(str::to_string));
    let tokens_in = value
        .pointer("/usage/prompt_tokens")
        .and_then(Value::as_u64);
    let tokens_out = value
        .pointer("/usage/completion_tokens")
        .and_then(Value::as_u64);
    let finish_reason = value
        .pointer("/choices/0/finish_reason")
        .and_then(Value::as_str)
        .map(str::to_string);

    Ok(InferResponse {
        request_id,
        node_id,
        model,
        output,
        raw: Some(value),
        latency_ms,
        tokens_in,
        tokens_out,
        finish_reason,
    })
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
    use axum::routing::post;
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
    fn server_control_requires_auth_even_on_localhost() {
        let config = NodeConfig {
            enable_server_control: true,
            ..NodeConfig::default()
        };

        assert_eq!(
            config.validate(),
            Err(NodeConfigError::ApiKeyRequiredForServerControl)
        );
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

    #[test]
    fn infer_request_maps_to_upstream_chat_request() {
        let request = InferRequest {
            request_id: Some("req-1".to_string()),
            model: Some("qwen".to_string()),
            system: Some("You are concise.".to_string()),
            prompt: "Say ok.".to_string(),
            max_tokens: Some(64),
            temperature: Some(0.4),
            top_p: Some(0.9),
            seed: Some(42),
            stop: Some(vec!["STOP".to_string()]),
            response_format: Some("json".to_string()),
            ..InferRequest::default()
        };

        let value = build_upstream_chat_request(&request);

        assert_eq!(value["model"], "qwen");
        assert_eq!(value["messages"][0]["role"], "system");
        assert_eq!(value["messages"][0]["content"], "You are concise.");
        assert_eq!(value["messages"][1]["role"], "user");
        assert_eq!(value["messages"][1]["content"], "Say ok.");
        assert_eq!(value["max_tokens"], 64);
        assert!((value["temperature"].as_f64().expect("temperature") - 0.4).abs() < 0.000_001);
        assert!((value["top_p"].as_f64().expect("top_p") - 0.9).abs() < 0.000_001);
        assert_eq!(value["seed"], 42);
        assert_eq!(value["stop"][0], "STOP");
        assert_eq!(value["stream"], false);
        assert_eq!(value["response_format"]["type"], "json_object");
    }

    #[test]
    fn upstream_chat_response_maps_to_infer_response() {
        let value = json!({
            "model": "qwen",
            "choices": [
                {
                    "message": { "role": "assistant", "content": "ok" },
                    "finish_reason": "stop"
                }
            ],
            "usage": {
                "prompt_tokens": 12,
                "completion_tokens": 2
            }
        });

        let response = map_upstream_chat_response(
            "req-1".to_string(),
            "node-a".to_string(),
            Some("fallback-model"),
            value,
            7,
        )
        .expect("mapped response");

        assert_eq!(response.request_id, "req-1");
        assert_eq!(response.node_id, "node-a");
        assert_eq!(response.model.as_deref(), Some("qwen"));
        assert_eq!(response.output, "ok");
        assert_eq!(response.tokens_in, Some(12));
        assert_eq!(response.tokens_out, Some(2));
        assert_eq!(response.finish_reason.as_deref(), Some("stop"));
        assert_eq!(response.latency_ms, 7);
    }

    #[test]
    fn capabilities_advertise_compatibility_routes() {
        let capabilities = test_snapshot(NodeConfig::default()).capabilities();

        assert!(capabilities.supports_infer);
        assert!(capabilities.supports_chat_completions);
        assert!(capabilities.supports_embeddings);
        assert!(!capabilities.supports_server_control);
    }

    #[test]
    fn capabilities_tie_server_control_to_config_flag() {
        let capabilities = test_snapshot(NodeConfig {
            enable_server_control: true,
            api_key: Some("secret".to_string()),
            ..NodeConfig::default()
        })
        .capabilities();

        assert!(capabilities.supports_server_control);
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
    async fn infer_requires_auth() {
        let app = router(NodeAppState::new(test_snapshot(NodeConfig {
            api_key: Some("secret".to_string()),
            ..NodeConfig::default()
        })));
        let response = app
            .oneshot(json_request("/v1/infer", &InferRequest::new("hello"), None))
            .await
            .expect("infer response");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn infer_rejects_empty_prompt_with_structured_error() {
        let app = router(NodeAppState::new(test_snapshot(NodeConfig {
            api_key: Some("secret".to_string()),
            ..NodeConfig::default()
        })));
        let request = InferRequest {
            request_id: Some("req-empty".to_string()),
            prompt: "   ".to_string(),
            ..InferRequest::default()
        };
        let response = app
            .oneshot(json_request("/v1/infer", &request, Some("secret")))
            .await
            .expect("infer response");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_text(response).await;
        let error: ErrorResponse = serde_json::from_str(&body).expect("error json");
        assert_eq!(error.error.code, "empty_prompt");
        assert_eq!(error.error.request_id.as_deref(), Some("req-empty"));
    }

    #[tokio::test]
    async fn infer_invalid_json_without_auth_returns_unauthorized() {
        let app = router(NodeAppState::new(test_snapshot(NodeConfig {
            api_key: Some("secret".to_string()),
            ..NodeConfig::default()
        })));
        let response = app
            .oneshot(raw_json_request("/v1/infer", "{", None))
            .await
            .expect("infer response");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn infer_invalid_json_with_auth_returns_structured_error() {
        let app = router(NodeAppState::new(test_snapshot(NodeConfig {
            api_key: Some("secret".to_string()),
            ..NodeConfig::default()
        })));
        let response = app
            .oneshot(raw_json_request("/v1/infer", "{", Some("secret")))
            .await
            .expect("infer response");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_text(response).await;
        let error: ErrorResponse = serde_json::from_str(&body).expect("error json");
        assert_eq!(error.error.code, "invalid_json");
    }

    #[tokio::test]
    async fn health_reports_ready_when_upstream_health_succeeds() {
        let upstream = spawn_upstream(axum::Router::new().route(
            "/v1/health",
            get(|| async { Json(json!({ "status": "ok" })) }),
        ))
        .await;
        let app = router(NodeAppState::new(test_snapshot(NodeConfig {
            llama_base_url: upstream,
            ..NodeConfig::default()
        })));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/health")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("health response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        let health: HealthResponse = serde_json::from_str(&body).expect("health json");
        assert_eq!(health.status, NodeStatus::Ready);
        assert!(health.llama_healthy);
    }

    #[tokio::test]
    async fn load_reports_ready_when_upstream_health_succeeds() {
        let upstream = spawn_upstream(
            axum::Router::new().route("/health", get(|| async { Json(json!({ "status": "ok" })) })),
        )
        .await;
        let app = router(NodeAppState::new(test_snapshot(NodeConfig {
            llama_base_url: upstream,
            ..NodeConfig::default()
        })));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/load")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("load response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        let load: LoadResponse = serde_json::from_str(&body).expect("load json");
        assert_eq!(load.status, NodeStatus::Ready);
    }

    #[tokio::test]
    async fn infer_preserves_request_id_and_maps_upstream_response() {
        let upstream = spawn_upstream(axum::Router::new().route(
            "/v1/chat/completions",
            post(|| async {
                Json(json!({
                    "model": "qwen",
                    "choices": [
                        {
                            "message": { "role": "assistant", "content": "proxied ok" },
                            "finish_reason": "stop"
                        }
                    ],
                    "usage": {
                        "prompt_tokens": 3,
                        "completion_tokens": 2
                    }
                }))
            }),
        ))
        .await;
        let app = router(NodeAppState::new(test_snapshot(NodeConfig {
            api_key: Some("secret".to_string()),
            llama_base_url: upstream,
            ..NodeConfig::default()
        })));
        let request = InferRequest {
            request_id: Some("req-proxy".to_string()),
            prompt: "hello".to_string(),
            ..InferRequest::default()
        };
        let response = app
            .oneshot(json_request("/v1/infer", &request, Some("secret")))
            .await
            .expect("infer response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        let infer: InferResponse = serde_json::from_str(&body).expect("infer json");
        assert_eq!(infer.request_id, "req-proxy");
        assert_eq!(infer.output, "proxied ok");
        assert_eq!(infer.tokens_in, Some(3));
        assert_eq!(infer.tokens_out, Some(2));
    }

    #[tokio::test]
    async fn infer_assigns_request_id_when_absent() {
        let upstream = spawn_upstream(axum::Router::new().route(
            "/v1/chat/completions",
            post(|| async {
                Json(json!({
                    "choices": [
                        {
                            "message": { "role": "assistant", "content": "assigned" },
                            "finish_reason": "stop"
                        }
                    ]
                }))
            }),
        ))
        .await;
        let app = router(NodeAppState::new(test_snapshot(NodeConfig {
            api_key: Some("secret".to_string()),
            llama_base_url: upstream,
            ..NodeConfig::default()
        })));

        let response = app
            .oneshot(json_request(
                "/v1/infer",
                &InferRequest::new("hello"),
                Some("secret"),
            ))
            .await
            .expect("infer response");
        let body = response_text(response).await;
        let infer: InferResponse = serde_json::from_str(&body).expect("infer json");

        assert!(infer.request_id.starts_with("lmml-"));
    }

    #[tokio::test]
    async fn infer_maps_upstream_error() {
        let upstream = spawn_upstream(axum::Router::new().route(
            "/v1/chat/completions",
            post(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "boom") }),
        ))
        .await;
        let app = router(NodeAppState::new(test_snapshot(NodeConfig {
            api_key: Some("secret".to_string()),
            llama_base_url: upstream,
            ..NodeConfig::default()
        })));
        let response = app
            .oneshot(json_request(
                "/v1/infer",
                &InferRequest::new("hello"),
                Some("secret"),
            ))
            .await
            .expect("infer response");

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = response_text(response).await;
        let error: ErrorResponse = serde_json::from_str(&body).expect("error json");
        assert_eq!(error.error.code, "upstream_error");
        assert!(error.error.request_id.is_some());
    }

    #[tokio::test]
    async fn infer_maps_upstream_timeout() {
        let upstream = spawn_upstream(axum::Router::new().route(
            "/v1/chat/completions",
            post(|| async {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Json(json!({
                    "choices": [
                        {
                            "message": { "role": "assistant", "content": "late" },
                            "finish_reason": "stop"
                        }
                    ]
                }))
            }),
        ))
        .await;
        let app = router(NodeAppState::new(test_snapshot(NodeConfig {
            api_key: Some("secret".to_string()),
            llama_base_url: upstream,
            infer_timeout_ms: 1,
            ..NodeConfig::default()
        })));
        let response = app
            .oneshot(json_request(
                "/v1/infer",
                &InferRequest::new("hello"),
                Some("secret"),
            ))
            .await
            .expect("infer response");

        assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
        let body = response_text(response).await;
        let error: ErrorResponse = serde_json::from_str(&body).expect("error json");
        assert_eq!(error.error.code, "upstream_timeout");
        assert!(error.error.request_id.is_some());
    }

    #[tokio::test]
    async fn chat_completions_requires_auth_before_body_read() {
        let app = router(NodeAppState::new(test_snapshot(NodeConfig {
            api_key: Some("secret".to_string()),
            ..NodeConfig::default()
        })));
        let response = app
            .oneshot(raw_json_request("/v1/chat/completions", "{", None))
            .await
            .expect("chat response");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn embeddings_invalid_json_with_auth_returns_structured_error() {
        let app = router(NodeAppState::new(test_snapshot(NodeConfig {
            api_key: Some("secret".to_string()),
            ..NodeConfig::default()
        })));
        let response = app
            .oneshot(raw_json_request("/v1/embeddings", "{", Some("secret")))
            .await
            .expect("embeddings response");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_text(response).await;
        let error: ErrorResponse = serde_json::from_str(&body).expect("error json");
        assert_eq!(error.error.code, "invalid_json");
    }

    #[tokio::test]
    async fn chat_completions_preserves_upstream_success_status_and_body() {
        let upstream = spawn_upstream(axum::Router::new().route(
            "/v1/chat/completions",
            post(|body: String| async move {
                (
                    StatusCode::ACCEPTED,
                    [(axum::http::header::CONTENT_TYPE, "application/json")],
                    body,
                )
            }),
        ))
        .await;
        let app = router(NodeAppState::new(test_snapshot(NodeConfig {
            api_key: Some("secret".to_string()),
            llama_base_url: upstream,
            ..NodeConfig::default()
        })));
        let request_body = r#"{"messages":[{"role":"user","content":"hello"}]}"#;
        let response = app
            .oneshot(raw_json_request(
                "/v1/chat/completions",
                request_body,
                Some("secret"),
            ))
            .await
            .expect("chat response");

        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let body = response_text(response).await;
        let echoed: Value = serde_json::from_str(&body).expect("echoed json");
        assert_eq!(echoed["messages"][0]["content"], "hello");
    }

    #[tokio::test]
    async fn embeddings_preserves_upstream_success_body() {
        let upstream = spawn_upstream(axum::Router::new().route(
            "/v1/embeddings",
            post(|| async {
                Json(json!({
                    "object": "list",
                    "data": [
                        {
                            "object": "embedding",
                            "embedding": [0.1, 0.2],
                            "index": 0
                        }
                    ]
                }))
            }),
        ))
        .await;
        let app = router(NodeAppState::new(test_snapshot(NodeConfig {
            api_key: Some("secret".to_string()),
            llama_base_url: upstream,
            ..NodeConfig::default()
        })));
        let response = app
            .oneshot(raw_json_request(
                "/v1/embeddings",
                r#"{"input":["hello"]}"#,
                Some("secret"),
            ))
            .await
            .expect("embeddings response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        let value: Value = serde_json::from_str(&body).expect("embedding json");
        assert_eq!(value["data"][0]["embedding"][0], 0.1);
    }

    #[tokio::test]
    async fn chat_completions_maps_upstream_error() {
        let upstream = spawn_upstream(axum::Router::new().route(
            "/v1/chat/completions",
            post(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "boom") }),
        ))
        .await;
        let app = router(NodeAppState::new(test_snapshot(NodeConfig {
            api_key: Some("secret".to_string()),
            llama_base_url: upstream,
            ..NodeConfig::default()
        })));
        let response = app
            .oneshot(raw_json_request(
                "/v1/chat/completions",
                r#"{"messages":[]}"#,
                Some("secret"),
            ))
            .await
            .expect("chat response");

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = response_text(response).await;
        let error: ErrorResponse = serde_json::from_str(&body).expect("error json");
        assert_eq!(error.error.code, "upstream_error");
    }

    #[tokio::test]
    async fn embeddings_maps_upstream_timeout() {
        let upstream = spawn_upstream(axum::Router::new().route(
            "/v1/embeddings",
            post(|| async {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Json(json!({ "data": [] }))
            }),
        ))
        .await;
        let app = router(NodeAppState::new(test_snapshot(NodeConfig {
            api_key: Some("secret".to_string()),
            llama_base_url: upstream,
            infer_timeout_ms: 1,
            ..NodeConfig::default()
        })));
        let response = app
            .oneshot(raw_json_request(
                "/v1/embeddings",
                r#"{"input":["hello"]}"#,
                Some("secret"),
            ))
            .await
            .expect("embeddings response");

        assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
        let body = response_text(response).await;
        let error: ErrorResponse = serde_json::from_str(&body).expect("error json");
        assert_eq!(error.error.code, "upstream_timeout");
    }

    #[tokio::test]
    async fn server_control_requires_auth_before_body_read() {
        let app = router(NodeAppState::new(test_snapshot(NodeConfig {
            api_key: Some("secret".to_string()),
            enable_server_control: true,
            ..NodeConfig::default()
        })));
        let response = app
            .oneshot(raw_json_request("/v1/server/control", "{", None))
            .await
            .expect("server control response");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn server_control_disabled_by_default_before_body_parse() {
        let app = router(NodeAppState::new(test_snapshot(NodeConfig {
            api_key: Some("secret".to_string()),
            ..NodeConfig::default()
        })));
        let response = app
            .oneshot(raw_json_request("/v1/server/control", "{", Some("secret")))
            .await
            .expect("server control response");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = response_text(response).await;
        let error: ErrorResponse = serde_json::from_str(&body).expect("error json");
        assert_eq!(error.error.code, "server_control_disabled");
    }

    #[tokio::test]
    async fn server_control_rejects_invalid_action() {
        let app = router(NodeAppState::new(test_snapshot(NodeConfig {
            api_key: Some("secret".to_string()),
            enable_server_control: true,
            ..NodeConfig::default()
        })));
        let response = app
            .oneshot(raw_json_request(
                "/v1/server/control",
                r#"{"action":"explode"}"#,
                Some("secret"),
            ))
            .await
            .expect("server control response");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_text(response).await;
        let error: ErrorResponse = serde_json::from_str(&body).expect("error json");
        assert_eq!(error.error.code, "invalid_server_control_request");
    }

    #[tokio::test]
    async fn server_control_status_reports_upstream_health() {
        let upstream = spawn_upstream(axum::Router::new().route(
            "/v1/health",
            get(|| async { Json(json!({ "status": "ok" })) }),
        ))
        .await;
        let app = router(NodeAppState::new(test_snapshot(NodeConfig {
            node_id: "node-a".to_string(),
            api_key: Some("secret".to_string()),
            enable_server_control: true,
            llama_base_url: upstream,
            ..NodeConfig::default()
        })));
        let response = app
            .oneshot(raw_json_request(
                "/v1/server/control",
                r#"{"action":"status"}"#,
                Some("secret"),
            ))
            .await
            .expect("server control response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        let control: ServerControlResponse =
            serde_json::from_str(&body).expect("server control json");
        assert_eq!(control.node_id, "node-a");
        assert_eq!(control.status, NodeStatus::Ready);
        assert!(control.message.contains("reachable"));
    }

    #[tokio::test]
    async fn server_control_lifecycle_actions_report_unavailable_manager() {
        let app = router(NodeAppState::new(test_snapshot(NodeConfig {
            api_key: Some("secret".to_string()),
            enable_server_control: true,
            ..NodeConfig::default()
        })));
        let response = app
            .oneshot(raw_json_request(
                "/v1/server/control",
                r#"{"action":"start"}"#,
                Some("secret"),
            ))
            .await
            .expect("server control response");

        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
        let body = response_text(response).await;
        let error: ErrorResponse = serde_json::from_str(&body).expect("error json");
        assert_eq!(error.error.code, "server_manager_unavailable");
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

    fn json_request(uri: &str, body: &InferRequest, bearer: Option<&str>) -> Request<Body> {
        let body = serde_json::to_string(body).expect("serialize request body");
        raw_json_request(uri, &body, bearer)
    }

    fn raw_json_request(uri: &str, body: &str, bearer: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder()
            .method("POST")
            .uri(uri)
            .header(axum::http::header::CONTENT_TYPE, "application/json");
        if let Some(bearer) = bearer {
            builder = builder.header(
                axum::http::header::AUTHORIZATION,
                format!("Bearer {bearer}"),
            );
        }
        builder.body(Body::from(body.to_string())).expect("request")
    }

    async fn spawn_upstream(router: axum::Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind upstream");
        let addr = listener.local_addr().expect("upstream addr");
        tokio::spawn(async move {
            axum::serve(listener, router).await.expect("serve upstream");
        });
        format!("http://{addr}")
    }
}
