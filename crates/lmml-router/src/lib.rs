//! LAN router and load balancer for LMML node APIs.
//!
//! `lmml-router` accepts OpenAI-compatible, Anthropic-compatible, and
//! LMML-native inference requests, discovers the configured LMML worker nodes,
//! then proxies each request to the best currently routable node.
//!
//! The router does not start or stop workers. Each upstream should run
//! `lmml-node`, which in turn proxies to the local `llama-server`.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::{HeaderMap, Request as AxumRequest, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use lmml_api::{
    ApiErrorBody, BackendKind, ErrorResponse, HealthResponse, InferRequest, LoadResponse,
    ModelDescriptor, NodeCapabilities, NodeRole, NodeStatus, PrivacyTier, API_VERSION,
    HEADER_NODE_ID, HEADER_REQUEST_ID, LAN_DISCOVERY_DEFAULT_TTL_MS,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use thiserror::Error;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

mod discovery;

pub use discovery::{parse_lan_advertisement, LanAdvertisementParseError};

/// Default HTTP port for the LAN LMML router.
pub const DEFAULT_ROUTER_PORT: u16 = 8100;

/// Default timeout for proxied generation requests in milliseconds.
pub const DEFAULT_PROXY_TIMEOUT_MS: u64 = 7_200_000;

/// Default per-node discovery timeout in milliseconds.
pub const DEFAULT_DISCOVERY_TIMEOUT_MS: u64 = 1_500;

/// Maximum accepted router request body size.
pub const MAX_ROUTER_BODY_BYTES: usize = 1024 * 1024;

/// Static upstream node configuration for [`RouterConfig`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamNodeConfig {
    /// Router-local name for this upstream.
    pub name: String,
    /// Base URL for an `lmml-node` API, for example `http://bc250:8101`.
    pub base_url: String,
    /// Optional bearer token used when calling protected upstream routes.
    pub api_key: Option<String>,
    /// True when this upstream came from LAN discovery rather than static config.
    pub discovered: bool,
}

impl UpstreamNodeConfig {
    /// Create an upstream configuration with a normalized base URL.
    pub fn new(name: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            base_url: normalize_base_url(&base_url.into()),
            api_key: None,
            discovered: false,
        }
    }
}

/// Runtime configuration for an LMML LAN router.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouterConfig {
    /// HTTP listen host.
    pub host: String,
    /// HTTP listen port.
    pub port: u16,
    /// Stable router identifier advertised to clients.
    pub router_id: String,
    /// Human-readable router name advertised to clients.
    pub router_name: String,
    /// Optional public URL clients should use.
    pub public_url: Option<String>,
    /// Static upstream LMML nodes.
    pub upstreams: Vec<UpstreamNodeConfig>,
    /// Listen for LAN node advertisements and merge them with static upstreams.
    pub discover_lan: bool,
    /// IPv4 multicast endpoint used for LAN node advertisements.
    pub lan_discovery_addr: SocketAddr,
    /// Expiry window for discovered nodes in milliseconds.
    pub discovered_node_ttl_ms: u64,
    /// Bearer token used when probing discovered upstream nodes.
    pub discovered_upstream_api_key: Option<String>,
    /// Timeout for proxied inference requests in milliseconds.
    pub proxy_timeout_ms: u64,
    /// Per-node timeout for health/capability/load discovery in milliseconds.
    pub discovery_timeout_ms: u64,
    /// Optional bearer token required for protected router routes.
    pub api_key: Option<String>,
    /// Explicit development escape hatch for non-local unauthenticated binds.
    pub allow_unsafe_lan_without_auth: bool,
    /// Free-form router tags.
    pub tags: Vec<String>,
}

impl RouterConfig {
    /// Validate security-sensitive router settings before binding a socket.
    pub fn validate(&self) -> Result<(), RouterConfigError> {
        if self.upstreams.is_empty() && !self.discover_lan {
            return Err(RouterConfigError::NoUpstreams);
        }
        if self.discover_lan && discovered_upstream_api_key(self).is_none() {
            return Err(RouterConfigError::ApiKeyRequiredForLanDiscovery);
        }
        if !is_local_bind(&self.host)
            && api_key(self).is_none()
            && !self.allow_unsafe_lan_without_auth
        {
            return Err(RouterConfigError::ApiKeyRequiredForLanBind {
                host: self.host.clone(),
            });
        }
        Ok(())
    }

    /// Return the socket address used by the HTTP server.
    pub fn socket_addr(&self) -> Result<SocketAddr, RouterConfigError> {
        socket_addr_string(&self.host, self.port)
            .parse()
            .map_err(|_| RouterConfigError::InvalidSocketAddress {
                host: self.host.clone(),
                port: self.port,
            })
    }
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: DEFAULT_ROUTER_PORT,
            router_id: default_router_id(),
            router_name: default_router_name(),
            public_url: None,
            upstreams: Vec::new(),
            discover_lan: false,
            lan_discovery_addr: discovery::default_lan_discovery_addr(),
            discovered_node_ttl_ms: LAN_DISCOVERY_DEFAULT_TTL_MS,
            discovered_upstream_api_key: None,
            proxy_timeout_ms: DEFAULT_PROXY_TIMEOUT_MS,
            discovery_timeout_ms: DEFAULT_DISCOVERY_TIMEOUT_MS,
            api_key: None,
            allow_unsafe_lan_without_auth: false,
            tags: vec!["lmml".to_string(), "router".to_string()],
        }
    }
}

/// Security, parsing, or address validation error for [`RouterConfig`].
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum RouterConfigError {
    /// At least one upstream node is required.
    #[error("at least one upstream LMML node is required unless LAN discovery is enabled")]
    NoUpstreams,
    /// LAN discovery requires a default token for authenticated discovered probes.
    #[error("default upstream key required when enabling LAN discovery; use --upstream-key default=<token>")]
    ApiKeyRequiredForLanDiscovery,
    /// LAN-visible router APIs must be authenticated unless explicitly unsafe.
    #[error("API key required when binding lmml-router to non-local host {host}")]
    ApiKeyRequiredForLanBind {
        /// Host that would expose the router beyond localhost.
        host: String,
    },
    /// Host and port could not be parsed as a socket address.
    #[error("invalid router socket address {host}:{port}")]
    InvalidSocketAddress {
        /// Configured host.
        host: String,
        /// Configured port.
        port: u16,
    },
    /// An `--upstream` argument could not be parsed.
    #[error("invalid upstream spec {spec}; expected name=url or url")]
    InvalidUpstreamSpec {
        /// Raw upstream argument.
        spec: String,
    },
    /// An `--upstream-key` argument could not be parsed.
    #[error("invalid upstream key spec {spec}; expected name=token")]
    InvalidUpstreamKeySpec {
        /// Raw upstream key argument.
        spec: String,
    },
    /// An upstream key referenced an unknown upstream name.
    #[error("upstream key references unknown upstream {name}")]
    UnknownUpstreamKey {
        /// Unknown upstream name.
        name: String,
    },
}

/// Shared application state for the router HTTP service.
#[derive(Debug, Clone)]
pub struct RouterAppState {
    config: Arc<RouterConfig>,
    client: reqwest::Client,
    started_at: Instant,
    counters: Arc<Mutex<RouterCounters>>,
    discovered: discovery::DiscoveredNodeTable,
}

impl RouterAppState {
    /// Create router state from validated configuration.
    pub fn new(config: RouterConfig) -> Result<Self, RouterConfigError> {
        config.validate()?;
        Ok(Self {
            config: Arc::new(config),
            client: reqwest::Client::new(),
            started_at: Instant::now(),
            counters: Arc::new(Mutex::new(RouterCounters::default())),
            discovered: discovery::DiscoveredNodeTable::default(),
        })
    }

    fn effective_upstreams(&self) -> Vec<UpstreamNodeConfig> {
        let mut upstreams = self.config.upstreams.clone();
        let Some(api_key) = discovered_upstream_api_key(&self.config) else {
            return upstreams;
        };
        let static_names = upstreams
            .iter()
            .map(|upstream| upstream.name.clone())
            .collect::<BTreeSet<_>>();
        for upstream in
            self.discovered
                .upstreams(Instant::now(), self.discovered_node_ttl(), api_key)
        {
            if !static_names.contains(&upstream.name) {
                upstreams.push(upstream);
            }
        }
        upstreams
    }

    fn discovered_node_ttl(&self) -> Duration {
        Duration::from_millis(self.config.discovered_node_ttl_ms)
    }

    fn active_discovered_len(&self) -> usize {
        self.discovered
            .active_len(Instant::now(), self.discovered_node_ttl())
    }

    fn record_discovered_advertisement(
        &self,
        advertisement: lmml_api::LanNodeAdvertisement,
        now: Instant,
    ) -> bool {
        if discovered_upstream_api_key(&self.config).is_none() {
            return false;
        }
        self.discovered.record(advertisement, now)
    }
}

#[derive(Debug, Default)]
struct RouterCounters {
    running_by_upstream: BTreeMap<String, u32>,
    completed_requests: u64,
    failed_requests: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct RouterCounterSnapshot {
    running_requests: u32,
    completed_requests: u64,
    failed_requests: u64,
}

/// Create the LMML router HTTP service.
pub fn router(state: RouterAppState) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/capabilities", get(capabilities))
        .route("/v1/load", get(load))
        .route("/v1/models", get(models))
        .route("/v1/infer", post(infer))
        .route("/v1/messages", post(anthropic_messages))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/embeddings", post(embeddings))
        .with_state(state)
}

/// Parse an upstream spec accepted by `lmml-router --upstream`.
///
/// Supported forms are `name=http://host:8101` and `http://host:8101`.
pub fn parse_upstream_spec(spec: &str) -> Result<UpstreamNodeConfig, RouterConfigError> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Err(RouterConfigError::InvalidUpstreamSpec {
            spec: spec.to_string(),
        });
    }
    if let Some((name, url)) = trimmed.split_once('=') {
        let name = name.trim();
        let url = url.trim();
        if name.is_empty() || !looks_like_http_url(url) {
            return Err(RouterConfigError::InvalidUpstreamSpec {
                spec: spec.to_string(),
            });
        }
        return Ok(UpstreamNodeConfig::new(name, url));
    }
    if !looks_like_http_url(trimmed) {
        return Err(RouterConfigError::InvalidUpstreamSpec {
            spec: spec.to_string(),
        });
    }
    Ok(UpstreamNodeConfig::new(
        inferred_upstream_name(trimmed),
        trimmed,
    ))
}

/// Apply `name=token` upstream API key specs to parsed upstreams.
///
/// A `default=token` spec returns the token used for authenticated probes
/// against nodes found through LAN discovery.
pub fn apply_upstream_key_specs(
    upstreams: &mut [UpstreamNodeConfig],
    specs: &[String],
) -> Result<Option<String>, RouterConfigError> {
    let mut default_key = None;
    for spec in specs {
        let Some((name, key)) = spec.split_once('=') else {
            return Err(RouterConfigError::InvalidUpstreamKeySpec { spec: spec.clone() });
        };
        let name = name.trim();
        let key = key.trim();
        if name.is_empty() || key.is_empty() {
            return Err(RouterConfigError::InvalidUpstreamKeySpec { spec: spec.clone() });
        }
        if name == "default" {
            default_key = Some(key.to_string());
            continue;
        }
        let Some(upstream) = upstreams.iter_mut().find(|upstream| upstream.name == name) else {
            return Err(RouterConfigError::UnknownUpstreamKey {
                name: name.to_string(),
            });
        };
        upstream.api_key = Some(key.to_string());
    }
    Ok(default_key)
}

/// Listen for UDP multicast LAN advertisements and update router discovery state.
pub async fn run_lan_discovery_listener(state: RouterAppState) -> io::Result<()> {
    let socket = discovery::bind_lan_discovery_socket(state.config.lan_discovery_addr).await?;
    let mut buffer = vec![0_u8; discovery::max_advertisement_bytes()];
    loop {
        let (len, peer) = socket.recv_from(&mut buffer).await?;
        match parse_lan_advertisement(&buffer[..len]) {
            Ok(advertisement) => {
                let node_id = advertisement.node_id.clone();
                if state.record_discovered_advertisement(advertisement, Instant::now()) {
                    tracing::debug!(node_id, peer = %peer, "recorded LMML LAN advertisement");
                }
            }
            Err(error) => {
                tracing::trace!(error = %error, peer = %peer, "ignored LAN advertisement");
            }
        }
    }
}

async fn health(State(state): State<RouterAppState>) -> Json<HealthResponse> {
    let probes = discover_upstreams(&state).await;
    let ready = probes.iter().filter(|probe| probe.is_ready()).count();
    let total = probes.len();
    Json(HealthResponse {
        api_version: API_VERSION.to_string(),
        node_id: state.config.router_id.clone(),
        node_name: state.config.router_name.clone(),
        status: if ready > 0 {
            NodeStatus::Ready
        } else {
            NodeStatus::Degraded
        },
        time_utc: utc_now_rfc3339(),
        uptime_s: state.started_at.elapsed().as_secs(),
        llama_healthy: ready > 0,
        active_model: None,
        message: Some(format!(
            "lmml-router active; {ready}/{total} upstream nodes ready"
        )),
    })
}

async fn capabilities(
    State(state): State<RouterAppState>,
    headers: HeaderMap,
) -> Result<Json<NodeCapabilities>, ApiFailure> {
    authorize(&state.config, &headers)?;
    let probes = discover_upstreams(&state).await;
    Ok(Json(aggregate_capabilities(&state, &probes)))
}

async fn load(
    State(state): State<RouterAppState>,
    headers: HeaderMap,
) -> Result<Json<LoadResponse>, ApiFailure> {
    authorize(&state.config, &headers)?;
    let probes = discover_upstreams(&state).await;
    Ok(Json(aggregate_load(&state, &probes)))
}

async fn models(
    State(state): State<RouterAppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ModelDescriptor>>, ApiFailure> {
    authorize(&state.config, &headers)?;
    let probes = discover_upstreams(&state).await;
    Ok(Json(aggregate_models(&probes)))
}

async fn infer(
    State(state): State<RouterAppState>,
    request: AxumRequest<axum::body::Body>,
) -> Result<Response, ApiFailure> {
    proxy_json_route(&state, request, "/v1/infer").await
}

async fn chat_completions(
    State(state): State<RouterAppState>,
    request: AxumRequest<axum::body::Body>,
) -> Result<Response, ApiFailure> {
    proxy_json_route(&state, request, "/v1/chat/completions").await
}

async fn anthropic_messages(
    State(state): State<RouterAppState>,
    request: AxumRequest<axum::body::Body>,
) -> Result<Response, ApiFailure> {
    proxy_json_route(&state, request, "/v1/messages").await
}

async fn embeddings(
    State(state): State<RouterAppState>,
    request: AxumRequest<axum::body::Body>,
) -> Result<Response, ApiFailure> {
    proxy_json_route(&state, request, "/v1/embeddings").await
}

#[derive(Debug, Clone)]
struct UpstreamProbe {
    config: UpstreamNodeConfig,
    health: Option<HealthResponse>,
    capabilities: Option<NodeCapabilities>,
    load: Option<LoadResponse>,
}

impl UpstreamProbe {
    fn is_ready(&self) -> bool {
        let health_ready = self
            .health
            .as_ref()
            .map(|health| health.status == NodeStatus::Ready && health.llama_healthy)
            .unwrap_or(false);
        let auth_verified = !self.config.discovered
            || self
                .capabilities
                .as_ref()
                .map(|capabilities| capabilities.auth_required)
                .unwrap_or(false);
        health_ready && auth_verified
    }

    fn running_requests(&self) -> u32 {
        self.load
            .as_ref()
            .map(|load| load.running_requests)
            .unwrap_or(u32::MAX)
    }
}

#[derive(Debug, Clone)]
struct AuthorizedBody {
    request_id: Option<String>,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ApiFailure {
    status: StatusCode,
    code: &'static str,
    message: String,
    request_id: Option<String>,
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
        let mut response = (
            self.status,
            Json(ErrorResponse {
                error: ApiErrorBody {
                    code: self.code.to_string(),
                    message: self.message,
                    request_id: self.request_id.clone(),
                    details: None,
                },
            }),
        )
            .into_response();
        attach_request_id_headers(&mut response, self.request_id.as_deref());
        response
    }
}

async fn proxy_json_route(
    state: &RouterAppState,
    request: AxumRequest<axum::body::Body>,
    path: &'static str,
) -> Result<Response, ApiFailure> {
    let body = read_authorized_body(&state.config, request).await?;
    let value = serde_json::from_slice::<Value>(&body.bytes).map_err(|error| {
        ApiFailure::new(
            StatusCode::BAD_REQUEST,
            "invalid_json",
            format!("invalid router request JSON: {error}"),
            body.request_id.clone(),
        )
    })?;
    if path == "/v1/infer" {
        let request = serde_json::from_value::<InferRequest>(value.clone()).map_err(|error| {
            ApiFailure::new(
                StatusCode::BAD_REQUEST,
                "invalid_infer_request",
                format!("invalid LMML infer request JSON: {error}"),
                body.request_id.clone(),
            )
        })?;
        if request.prompt.trim().is_empty() {
            return Err(ApiFailure::new(
                StatusCode::BAD_REQUEST,
                "empty_prompt",
                "prompt must not be empty",
                body.request_id.clone().or(request.request_id),
            ));
        }
    }

    let requested_model = requested_model(path, &value);
    let upstream =
        select_upstream(state, path, requested_model.as_deref(), &body.request_id).await?;
    call_upstream(state, &upstream, path, body).await
}

async fn select_upstream(
    state: &RouterAppState,
    path: &str,
    model: Option<&str>,
    request_id: &Option<String>,
) -> Result<UpstreamNodeConfig, ApiFailure> {
    let probes = discover_upstreams(state).await;
    probes
        .into_iter()
        .filter(|probe| probe.is_ready())
        .filter(|probe| {
            probe
                .capabilities
                .as_ref()
                .map(|capabilities| route_supported(capabilities, path))
                .unwrap_or(false)
        })
        .filter(|probe| {
            probe
                .capabilities
                .as_ref()
                .map(|capabilities| model_supported(capabilities, model))
                .unwrap_or(false)
        })
        .min_by_key(|probe| {
            u64::from(probe.running_requests())
                + u64::from(upstream_running_requests(state, &probe.config.name))
        })
        .map(|probe| probe.config)
        .ok_or_else(|| {
            ApiFailure::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "no_routable_upstream",
                "no ready LMML upstream supports this route and model",
                request_id.clone(),
            )
        })
}

async fn call_upstream(
    state: &RouterAppState,
    upstream: &UpstreamNodeConfig,
    path: &str,
    body: AuthorizedBody,
) -> Result<Response, ApiFailure> {
    increment_router_running(state, &upstream.name);
    let result = call_upstream_inner(state, upstream, path, body).await;
    finish_router_request(state, &upstream.name, result.is_ok());
    result
}

async fn call_upstream_inner(
    state: &RouterAppState,
    upstream: &UpstreamNodeConfig,
    path: &str,
    body: AuthorizedBody,
) -> Result<Response, ApiFailure> {
    let timeout = Duration::from_millis(state.config.proxy_timeout_ms);
    let url = upstream_url(&upstream.base_url, path);
    let upstream_body = body.bytes.clone();
    let request_id = body.request_id.clone();
    let upstream_key = upstream.api_key.clone();
    let upstream_response = tokio::time::timeout(timeout, async {
        let mut request = state
            .client
            .post(url)
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(upstream_body);
        if let Some(api_key) = upstream_key.as_deref() {
            request = request.bearer_auth(api_key);
        }
        if let Some(request_id) = request_id.as_deref() {
            request = request.header(HEADER_REQUEST_ID, request_id);
        }
        let response = request.send().await?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let upstream_node_id = response
            .headers()
            .get(HEADER_NODE_ID)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let bytes = response.bytes().await?;
        Ok::<_, reqwest::Error>((status, content_type, upstream_node_id, bytes.to_vec()))
    })
    .await
    .map_err(|_| {
        ApiFailure::new(
            StatusCode::GATEWAY_TIMEOUT,
            "upstream_timeout",
            format!("LMML upstream {} timed out", upstream.name),
            body.request_id.clone(),
        )
    })?
    .map_err(|error| {
        ApiFailure::new(
            StatusCode::BAD_GATEWAY,
            "upstream_request_failed",
            format!("failed to call LMML upstream {}: {error}", upstream.name),
            body.request_id.clone(),
        )
    })?;

    let (status, content_type, upstream_node_id, bytes) = upstream_response;
    if !(200..300).contains(&status) {
        return Err(ApiFailure::new(
            StatusCode::BAD_GATEWAY,
            "upstream_error",
            format!("LMML upstream {} returned HTTP {status}", upstream.name),
            body.request_id,
        ));
    }

    let status = StatusCode::from_u16(status).map_err(|error| {
        ApiFailure::new(
            StatusCode::BAD_GATEWAY,
            "upstream_invalid_status",
            format!(
                "LMML upstream {} returned invalid HTTP status: {error}",
                upstream.name
            ),
            body.request_id.clone(),
        )
    })?;
    let mut builder = Response::builder().status(status);
    if let Some(content_type) = content_type {
        builder = builder.header(axum::http::header::CONTENT_TYPE, content_type);
    }
    if let Some(request_id) = body.request_id.as_deref() {
        builder = builder.header(HEADER_REQUEST_ID, request_id);
    }
    if let Some(upstream_node_id) = upstream_node_id {
        builder = builder.header(HEADER_NODE_ID, upstream_node_id);
    }
    builder
        .body(axum::body::Body::from(bytes))
        .map_err(|error| {
            ApiFailure::new(
                StatusCode::BAD_GATEWAY,
                "router_response_failed",
                format!("failed to build router response: {error}"),
                body.request_id,
            )
        })
}

async fn discover_upstreams(state: &RouterAppState) -> Vec<UpstreamProbe> {
    let upstreams = state.effective_upstreams();
    let mut tasks = tokio::task::JoinSet::new();
    for (index, upstream) in upstreams.into_iter().enumerate() {
        let state = state.clone();
        tasks.spawn(async move { (index, fetch_upstream_probe(&state, &upstream).await) });
    }

    let mut indexed = Vec::new();
    while let Some(result) = tasks.join_next().await {
        if let Ok(probe) = result {
            indexed.push(probe);
        }
    }
    indexed.sort_by_key(|(index, _)| *index);
    indexed.into_iter().map(|(_, probe)| probe).collect()
}

async fn fetch_upstream_probe(
    state: &RouterAppState,
    upstream: &UpstreamNodeConfig,
) -> UpstreamProbe {
    let timeout = Duration::from_millis(state.config.discovery_timeout_ms);
    let health = get_upstream_json::<HealthResponse>(state, upstream, "/v1/health", timeout);
    let capabilities =
        get_upstream_json::<NodeCapabilities>(state, upstream, "/v1/capabilities", timeout);
    let load = get_upstream_json::<LoadResponse>(state, upstream, "/v1/load", timeout);
    let (health, capabilities, load) = tokio::join!(health, capabilities, load);
    UpstreamProbe {
        config: upstream.clone(),
        health: health.ok(),
        capabilities: capabilities.ok(),
        load: load.ok(),
    }
}

async fn get_upstream_json<T>(
    state: &RouterAppState,
    upstream: &UpstreamNodeConfig,
    path: &str,
    timeout: Duration,
) -> Result<T, reqwest::Error>
where
    T: DeserializeOwned,
{
    let mut request = state.client.get(upstream_url(&upstream.base_url, path));
    if let Some(api_key) = upstream.api_key.as_deref() {
        request = request.bearer_auth(api_key);
    }
    request.timeout(timeout).send().await?.json::<T>().await
}

fn aggregate_capabilities(state: &RouterAppState, probes: &[UpstreamProbe]) -> NodeCapabilities {
    let config = state.config.as_ref();
    let mut gpus = Vec::new();
    let mut models = Vec::new();
    let mut model_ids = BTreeSet::new();
    let mut max_context_tokens = None;
    let mut supports_infer = false;
    let mut supports_chat_completions = false;
    let mut supports_anthropic_messages = false;
    let mut supports_embeddings = false;
    let mut backends = BTreeSet::new();

    for capabilities in probes
        .iter()
        .filter(|probe| probe.is_ready())
        .filter_map(|probe| probe.capabilities.as_ref())
    {
        gpus.extend(capabilities.gpus.clone());
        for model in &capabilities.models {
            if model_ids.insert(model.id.clone()) {
                models.push(model.clone());
            }
        }
        max_context_tokens = max_context_tokens.max(capabilities.max_context_tokens);
        supports_infer |= capabilities.supports_infer;
        supports_chat_completions |= capabilities.supports_chat_completions;
        supports_anthropic_messages |= capabilities.supports_anthropic_messages;
        supports_embeddings |= capabilities.supports_embeddings;
        backends.insert(format!("{:?}", capabilities.backend));
    }

    let ready = probes.iter().filter(|probe| probe.is_ready()).count();
    let mut extra = BTreeMap::new();
    extra.insert("upstream_count".to_string(), json!(probes.len()));
    extra.insert(
        "static_upstream_count".to_string(),
        json!(config.upstreams.len()),
    );
    extra.insert(
        "discovered_upstream_count".to_string(),
        json!(state.active_discovered_len()),
    );
    extra.insert("ready_upstream_count".to_string(), json!(ready));
    extra.insert(
        "upstreams".to_string(),
        json!(probes
            .iter()
            .map(upstream_summary)
            .collect::<Vec<BTreeMap<String, Value>>>()),
    );

    NodeCapabilities {
        api_version: API_VERSION.to_string(),
        lmml_version: env!("CARGO_PKG_VERSION").to_string(),
        node_id: config.router_id.clone(),
        node_name: config.router_name.clone(),
        public_url: config.public_url.clone(),
        roles: vec![NodeRole::Router],
        tags: config.tags.clone(),
        privacy: privacy_tier(config),
        backend: aggregate_backend(&backends),
        gpus,
        models,
        max_context_tokens,
        supports_infer,
        supports_chat_completions,
        supports_anthropic_messages,
        supports_embeddings,
        supports_server_control: false,
        auth_required: api_key(config).is_some(),
        llama_cpp_commit: None,
        agentq: None,
        extra,
    }
}

fn aggregate_models(probes: &[UpstreamProbe]) -> Vec<ModelDescriptor> {
    let mut models = Vec::new();
    let mut model_ids = BTreeSet::new();
    for model in probes
        .iter()
        .filter(|probe| probe.is_ready())
        .filter_map(|probe| probe.capabilities.as_ref())
        .flat_map(|capabilities| capabilities.models.iter())
    {
        if model_ids.insert(model.id.clone()) {
            models.push(model.clone());
        }
    }
    models
}

fn aggregate_load(state: &RouterAppState, probes: &[UpstreamProbe]) -> LoadResponse {
    let config = &state.config;
    let counters = router_counter_snapshot(state);
    let mut memory_total_mb = 0;
    let mut memory_used_mb = 0;
    let mut running_requests = counters.running_requests;
    let mut completed_requests = counters.completed_requests;
    let mut failed_requests = counters.failed_requests;
    let mut tokens_in_total = 0;
    let mut tokens_out_total = 0;
    let mut gpus = Vec::new();

    for load in probes
        .iter()
        .filter(|probe| probe.is_ready())
        .filter_map(|probe| probe.load.as_ref())
    {
        memory_total_mb += load.memory_total_mb;
        memory_used_mb += load.memory_used_mb;
        running_requests += load.running_requests;
        completed_requests += load.completed_requests;
        failed_requests += load.failed_requests;
        tokens_in_total += load.tokens_in_total;
        tokens_out_total += load.tokens_out_total;
        gpus.extend(load.gpus.clone());
    }

    LoadResponse {
        node_id: config.router_id.clone(),
        status: if probes.iter().any(UpstreamProbe::is_ready) {
            NodeStatus::Ready
        } else {
            NodeStatus::Degraded
        },
        cpu_usage_pct: 0.0,
        memory_total_mb,
        memory_used_mb,
        gpus,
        running_requests,
        completed_requests,
        failed_requests,
        tokens_in_total,
        tokens_out_total,
    }
}

fn upstream_summary(probe: &UpstreamProbe) -> BTreeMap<String, Value> {
    let mut summary = BTreeMap::new();
    summary.insert("name".to_string(), json!(probe.config.name));
    summary.insert("url".to_string(), json!(probe.config.base_url));
    summary.insert("discovered".to_string(), json!(probe.config.discovered));
    summary.insert(
        "status".to_string(),
        json!(probe
            .health
            .as_ref()
            .map(|health| format!("{:?}", health.status).to_lowercase())
            .unwrap_or_else(|| "unreachable".to_string())),
    );
    if let Some(capabilities) = probe.capabilities.as_ref() {
        summary.insert("node_id".to_string(), json!(capabilities.node_id));
        summary.insert(
            "backend".to_string(),
            json!(format!("{:?}", capabilities.backend)),
        );
    }
    summary
}

fn route_supported(capabilities: &NodeCapabilities, path: &str) -> bool {
    match path {
        "/v1/infer" => capabilities.supports_infer,
        "/v1/chat/completions" => capabilities.supports_chat_completions,
        "/v1/messages" => capabilities.supports_anthropic_messages,
        "/v1/embeddings" => capabilities.supports_embeddings,
        _ => false,
    }
}

fn model_supported(capabilities: &NodeCapabilities, model: Option<&str>) -> bool {
    let Some(model) = model else {
        return true;
    };
    if capabilities.models.is_empty() {
        return true;
    }
    let requested = model.to_lowercase();
    capabilities.models.iter().any(|candidate| {
        candidate.id.eq_ignore_ascii_case(model)
            || candidate.name.eq_ignore_ascii_case(model)
            || candidate
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(model))
            || candidate.id.to_lowercase().contains(&requested)
            || candidate.name.to_lowercase().contains(&requested)
    })
}

fn requested_model(path: &str, value: &Value) -> Option<String> {
    match path {
        "/v1/infer" | "/v1/chat/completions" | "/v1/messages" | "/v1/embeddings" => value
            .get("model")
            .and_then(Value::as_str)
            .filter(|model| !model.trim().is_empty())
            .map(str::to_string),
        _ => None,
    }
}

async fn read_authorized_body(
    config: &RouterConfig,
    request: AxumRequest<axum::body::Body>,
) -> Result<AuthorizedBody, ApiFailure> {
    let request_id = request_id_from_headers(request.headers());
    authorize(config, request.headers())?;
    let bytes = axum::body::to_bytes(request.into_body(), MAX_ROUTER_BODY_BYTES)
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

fn authorize(config: &RouterConfig, headers: &HeaderMap) -> Result<(), ApiFailure> {
    let Some(expected) = api_key(config) else {
        return Ok(());
    };
    let bearer = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let bearer_token = bearer.strip_prefix("Bearer ").unwrap_or_default();
    let api_key_header = headers
        .get("x-api-key")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if constant_time_eq(bearer_token.as_bytes(), expected.as_bytes())
        || constant_time_eq(api_key_header.as_bytes(), expected.as_bytes())
    {
        return Ok(());
    }
    Err(ApiFailure::new(
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        "missing or invalid bearer token",
        None,
    ))
}

fn request_id_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get(HEADER_REQUEST_ID)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}

fn increment_router_running(state: &RouterAppState, upstream: &str) {
    if let Ok(mut counters) = state.counters.lock() {
        *counters
            .running_by_upstream
            .entry(upstream.to_string())
            .or_default() += 1;
    }
}

fn finish_router_request(state: &RouterAppState, upstream: &str, success: bool) {
    if let Ok(mut counters) = state.counters.lock() {
        if let Some(running) = counters.running_by_upstream.get_mut(upstream) {
            *running = running.saturating_sub(1);
        }
        if success {
            counters.completed_requests += 1;
        } else {
            counters.failed_requests += 1;
        }
    }
}

fn upstream_running_requests(state: &RouterAppState, upstream: &str) -> u32 {
    state
        .counters
        .lock()
        .ok()
        .and_then(|counters| counters.running_by_upstream.get(upstream).copied())
        .unwrap_or(0)
}

fn router_counter_snapshot(state: &RouterAppState) -> RouterCounterSnapshot {
    let Ok(counters) = state.counters.lock() else {
        return RouterCounterSnapshot::default();
    };
    RouterCounterSnapshot {
        running_requests: counters.running_by_upstream.values().copied().sum(),
        completed_requests: counters.completed_requests,
        failed_requests: counters.failed_requests,
    }
}

fn attach_request_id_headers(response: &mut Response, request_id: Option<&str>) {
    let Some(request_id) = request_id else {
        return;
    };
    let Ok(value) = axum::http::HeaderValue::from_str(request_id) else {
        return;
    };
    response.headers_mut().insert(HEADER_REQUEST_ID, value);
}

fn aggregate_backend(backends: &BTreeSet<String>) -> BackendKind {
    if backends.len() == 1 {
        match backends.iter().next().map(String::as_str) {
            Some("Cuda") => BackendKind::Cuda,
            Some("Metal") => BackendKind::Metal,
            Some("Hip") => BackendKind::Hip,
            Some("Vulkan") => BackendKind::Vulkan,
            Some("CpuAvx2") => BackendKind::CpuAvx2,
            Some("CpuAvx") => BackendKind::CpuAvx,
            Some("CpuFallback") => BackendKind::CpuFallback,
            Some("Unknown") | None => BackendKind::Unknown,
            Some(_) => BackendKind::Unknown,
        }
    } else {
        BackendKind::Unknown
    }
}

fn privacy_tier(config: &RouterConfig) -> PrivacyTier {
    if is_local_bind(&config.host) {
        PrivacyTier::LocalhostOnly
    } else {
        PrivacyTier::LanOnly
    }
}

fn api_key(config: &RouterConfig) -> Option<&str> {
    config
        .api_key
        .as_deref()
        .filter(|api_key| !api_key.trim().is_empty())
}

fn discovered_upstream_api_key(config: &RouterConfig) -> Option<&str> {
    config
        .discovered_upstream_api_key
        .as_deref()
        .filter(|api_key| !api_key.trim().is_empty())
}

fn is_local_bind(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "::1" | "localhost")
}

fn socket_addr_string(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else if host == "localhost" {
        format!("127.0.0.1:{port}")
    } else {
        format!("{host}:{port}")
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |diff, (left, right)| diff | (left ^ right))
        == 0
}

fn default_router_id() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|host| !host.trim().is_empty())
        .map(|host| format!("lmml-router-{host}"))
        .unwrap_or_else(|| "lmml-router-local".to_string())
}

fn default_router_name() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|host| !host.trim().is_empty())
        .map(|host| format!("{host} LMML router"))
        .unwrap_or_else(|| "LMML router".to_string())
}

fn looks_like_http_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

fn normalize_base_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

fn upstream_url(base_url: &str, path: &str) -> String {
    format!(
        "{}/{}",
        normalize_base_url(base_url),
        path.trim_start_matches('/')
    )
}

fn utc_now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn inferred_upstream_name(url: &str) -> String {
    url.trim()
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/')
        .replace([':', '/', '.'], "-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use axum::routing::post;
    use lmml_api::{
        GpuDescriptor, LanNodeAdvertisement, ModelDescriptor, LAN_DISCOVERY_MAGIC,
        LAN_DISCOVERY_VERSION,
    };
    use pretty_assertions::assert_eq;
    use tower::ServiceExt;

    #[test]
    fn parse_upstream_supports_named_and_inferred_specs() {
        assert_eq!(
            parse_upstream_spec("bc250=http://192.168.50.176:8101").expect("named upstream"),
            UpstreamNodeConfig::new("bc250", "http://192.168.50.176:8101")
        );
        assert_eq!(
            parse_upstream_spec("http://192.168.50.178:8101")
                .expect("inferred upstream")
                .name,
            "192-168-50-178-8101"
        );
    }

    #[test]
    fn upstream_key_specs_support_default_key_for_discovery() {
        let mut upstreams = vec![UpstreamNodeConfig::new("static", "http://127.0.0.1:8101")];

        let default_key = apply_upstream_key_specs(
            &mut upstreams,
            &[
                "static=worker-key".to_string(),
                "default=lan-key".to_string(),
            ],
        )
        .expect("apply keys");

        assert_eq!(upstreams[0].api_key.as_deref(), Some("worker-key"));
        assert_eq!(default_key.as_deref(), Some("lan-key"));
    }

    #[test]
    fn lan_bind_requires_auth_by_default() {
        let config = RouterConfig {
            host: "0.0.0.0".to_string(),
            upstreams: vec![UpstreamNodeConfig::new("node", "http://127.0.0.1:8101")],
            ..RouterConfig::default()
        };

        assert_eq!(
            config.validate(),
            Err(RouterConfigError::ApiKeyRequiredForLanBind {
                host: "0.0.0.0".to_string()
            })
        );
    }

    #[test]
    fn config_requires_at_least_one_upstream() {
        assert_eq!(
            RouterConfig::default().validate(),
            Err(RouterConfigError::NoUpstreams)
        );
    }

    #[test]
    fn lan_discovery_requires_default_upstream_key() {
        let config = RouterConfig {
            discover_lan: true,
            ..RouterConfig::default()
        };

        assert_eq!(
            config.validate(),
            Err(RouterConfigError::ApiKeyRequiredForLanDiscovery)
        );
    }

    #[test]
    fn discovered_upstreams_merge_with_static_and_static_wins() {
        let state = RouterAppState::new(RouterConfig {
            discover_lan: true,
            discovered_upstream_api_key: Some("lan-key".to_string()),
            upstreams: vec![UpstreamNodeConfig::new("node-a", "http://static:8101")],
            ..RouterConfig::default()
        })
        .expect("router state");
        let now = Instant::now();

        assert!(state.record_discovered_advertisement(
            test_advertisement("node-a", "http://discovered-a:8101"),
            now
        ));
        assert!(state.record_discovered_advertisement(
            test_advertisement("node-b", "http://discovered-b:8101"),
            now
        ));

        let upstreams = state.effective_upstreams();

        assert_eq!(upstreams.len(), 2);
        assert_eq!(upstreams[0].name, "node-a");
        assert_eq!(upstreams[0].base_url, "http://static:8101");
        assert_eq!(upstreams[1].name, "node-b");
        assert_eq!(upstreams[1].base_url, "http://discovered-b:8101");
        assert_eq!(upstreams[1].api_key.as_deref(), Some("lan-key"));
        assert!(upstreams[1].discovered);
    }

    #[tokio::test]
    async fn unauthenticated_invalid_json_is_rejected_before_parsing() {
        let app = test_router(RouterConfig {
            api_key: Some("secret".to_string()),
            upstreams: vec![UpstreamNodeConfig::new("node", "http://127.0.0.1:1")],
            ..RouterConfig::default()
        });

        let response = app
            .oneshot(raw_request("/v1/infer", "{", None))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn authenticated_invalid_json_returns_structured_error() {
        let app = test_router(RouterConfig {
            api_key: Some("secret".to_string()),
            upstreams: vec![UpstreamNodeConfig::new("node", "http://127.0.0.1:1")],
            ..RouterConfig::default()
        });

        let response = app
            .oneshot(raw_request("/v1/infer", "{", Some("secret")))
            .await
            .expect("response");
        let status = response.status();
        let body = json_body(response).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "invalid_json");
    }

    #[tokio::test]
    async fn infer_routes_to_requested_model_node() {
        let workstation = spawn_upstream(mock_node(
            "workstation",
            "qwen9b",
            BackendKind::Cuda,
            2,
            "/v1/infer",
        ))
        .await;
        let bc250 = spawn_upstream(mock_node(
            "bc250",
            "Qwen3.5-9B-Q4_K_M.gguf",
            BackendKind::Vulkan,
            0,
            "/v1/infer",
        ))
        .await;
        let app = test_router(RouterConfig {
            api_key: Some("secret".to_string()),
            upstreams: vec![
                UpstreamNodeConfig::new("workstation", workstation),
                UpstreamNodeConfig::new("bc250", bc250),
            ],
            ..RouterConfig::default()
        });
        let body = json!({
            "model": "Qwen3.5-9B-Q4_K_M.gguf",
            "task_type": "general",
            "prompt": "hello",
            "metadata": {}
        });

        let response = app
            .oneshot(json_request("/v1/infer", &body, Some("secret")))
            .await
            .expect("response");
        let body = json_body(response).await;

        assert_eq!(body["node_id"], "bc250");
    }

    #[tokio::test]
    async fn chat_completions_choose_least_loaded_ready_node() {
        let busy = spawn_upstream(mock_node(
            "busy",
            "shared-model",
            BackendKind::Cuda,
            9,
            "/v1/chat/completions",
        ))
        .await;
        let idle = spawn_upstream(mock_node(
            "idle",
            "shared-model",
            BackendKind::Vulkan,
            1,
            "/v1/chat/completions",
        ))
        .await;
        let app = test_router(RouterConfig {
            api_key: Some("secret".to_string()),
            upstreams: vec![
                UpstreamNodeConfig::new("busy", busy),
                UpstreamNodeConfig::new("idle", idle),
            ],
            ..RouterConfig::default()
        });

        let response = app
            .oneshot(json_request(
                "/v1/chat/completions",
                &json!({ "model": "shared-model", "messages": [] }),
                Some("secret"),
            ))
            .await
            .expect("response");
        let body = json_body(response).await;

        assert_eq!(body["choices"][0]["message"]["content"], "idle");
    }

    #[tokio::test]
    async fn no_routable_upstream_returns_service_unavailable() {
        let app = test_router(RouterConfig {
            api_key: Some("secret".to_string()),
            upstreams: vec![UpstreamNodeConfig::new("node", "http://127.0.0.1:1")],
            discovery_timeout_ms: 25,
            ..RouterConfig::default()
        });

        let response = app
            .oneshot(json_request(
                "/v1/chat/completions",
                &json!({ "model": "missing", "messages": [] }),
                Some("secret"),
            ))
            .await
            .expect("response");
        let status = response.status();
        let body = json_body(response).await;

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["error"]["code"], "no_routable_upstream");
    }

    #[tokio::test]
    async fn capabilities_aggregate_ready_upstreams() {
        let cuda =
            spawn_upstream(mock_node("cuda", "qwen", BackendKind::Cuda, 0, "/v1/infer")).await;
        let vulkan = spawn_upstream(mock_node(
            "vulkan",
            "bc250-qwen",
            BackendKind::Vulkan,
            0,
            "/v1/infer",
        ))
        .await;
        let degraded = spawn_upstream(mock_node_with_health(
            "degraded",
            "unroutable-model",
            BackendKind::Vulkan,
            0,
            "/v1/infer",
            NodeStatus::Degraded,
            false,
        ))
        .await;
        let app = test_router(RouterConfig {
            api_key: Some("secret".to_string()),
            upstreams: vec![
                UpstreamNodeConfig::new("cuda", cuda),
                UpstreamNodeConfig::new("vulkan", vulkan),
                UpstreamNodeConfig::new("degraded", degraded),
            ],
            ..RouterConfig::default()
        });

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/capabilities")
                    .header("Authorization", "Bearer secret")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let body = json_body(response).await;

        assert_eq!(body["roles"][0], "router");
        assert_eq!(body["models"].as_array().expect("models").len(), 2);
        assert_eq!(body["extra"]["ready_upstream_count"], 2);
        assert_eq!(body["extra"]["upstream_count"], 3);
    }

    #[tokio::test]
    async fn discovered_upstream_requires_authenticated_capabilities() {
        let upstream = spawn_upstream(mock_node(
            "node-a",
            "qwen",
            BackendKind::Cuda,
            0,
            "/v1/infer",
        ))
        .await;
        let state = RouterAppState::new(RouterConfig {
            discover_lan: true,
            discovered_upstream_api_key: Some("worker-key".to_string()),
            ..RouterConfig::default()
        })
        .expect("router state");
        assert!(state.record_discovered_advertisement(
            test_advertisement("node-a", &upstream),
            Instant::now()
        ));
        let app = router(state);

        let response = app
            .oneshot(get_request("/v1/capabilities", None))
            .await
            .expect("response");
        let body = json_body(response).await;

        assert_eq!(body["models"].as_array().expect("models").len(), 0);
        assert_eq!(body["extra"]["upstream_count"], 1);
        assert_eq!(body["extra"]["ready_upstream_count"], 0);
    }

    #[tokio::test]
    async fn models_requires_auth() {
        let app = test_router(RouterConfig {
            api_key: Some("secret".to_string()),
            upstreams: vec![UpstreamNodeConfig::new("node", "http://127.0.0.1:1")],
            ..RouterConfig::default()
        });

        let response = app
            .oneshot(get_request("/v1/models", None))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn models_aggregate_ready_upstreams_only() {
        let ready = spawn_upstream(mock_node(
            "ready",
            "ready-model",
            BackendKind::Cuda,
            0,
            "/v1/infer",
        ))
        .await;
        let degraded = spawn_upstream(mock_node_with_health(
            "degraded",
            "degraded-model",
            BackendKind::Vulkan,
            0,
            "/v1/infer",
            NodeStatus::Degraded,
            false,
        ))
        .await;
        let app = test_router(RouterConfig {
            api_key: Some("secret".to_string()),
            upstreams: vec![
                UpstreamNodeConfig::new("ready", ready),
                UpstreamNodeConfig::new("degraded", degraded),
            ],
            ..RouterConfig::default()
        });

        let response = app
            .oneshot(get_request("/v1/models", Some("secret")))
            .await
            .expect("response");
        let body = json_body(response).await;

        assert_eq!(
            body.as_array().expect("models"),
            &[json!({
                "id": "ready-model",
                "name": "ready-model",
                "path": null,
                "architecture": null,
                "quantization": null,
                "context_length": 4096,
                "size_bytes": null,
                "loaded": true,
                "aliases": []
            })]
        );
    }

    #[tokio::test]
    async fn load_includes_router_completed_proxy_requests() {
        let upstream =
            spawn_upstream(mock_node("node", "qwen", BackendKind::Cuda, 0, "/v1/infer")).await;
        let app = test_router(RouterConfig {
            api_key: Some("secret".to_string()),
            upstreams: vec![UpstreamNodeConfig::new("node", upstream)],
            ..RouterConfig::default()
        });
        let body = json!({
            "model": "qwen",
            "task_type": "general",
            "prompt": "hello",
            "metadata": {}
        });

        let response = app
            .clone()
            .oneshot(json_request("/v1/infer", &body, Some("secret")))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/load")
                    .header("Authorization", "Bearer secret")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let body = json_body(response).await;

        assert_eq!(body["running_requests"], 0);
        assert_eq!(body["completed_requests"], 1);
        assert_eq!(body["failed_requests"], 0);
    }

    #[tokio::test]
    async fn load_ignores_degraded_upstream_counters() {
        let ready = spawn_upstream(mock_node(
            "ready",
            "qwen",
            BackendKind::Cuda,
            1,
            "/v1/infer",
        ))
        .await;
        let degraded = spawn_upstream(mock_node_with_health(
            "degraded",
            "qwen",
            BackendKind::Vulkan,
            99,
            "/v1/infer",
            NodeStatus::Degraded,
            false,
        ))
        .await;
        let app = test_router(RouterConfig {
            api_key: Some("secret".to_string()),
            upstreams: vec![
                UpstreamNodeConfig::new("ready", ready),
                UpstreamNodeConfig::new("degraded", degraded),
            ],
            ..RouterConfig::default()
        });

        let response = app
            .oneshot(get_request("/v1/load", Some("secret")))
            .await
            .expect("response");
        let body = json_body(response).await;

        assert_eq!(body["running_requests"], 1);
    }

    fn test_router(config: RouterConfig) -> Router {
        router(RouterAppState::new(config).expect("router state"))
    }

    fn get_request(path: &str, api_key: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().method("GET").uri(path);
        if let Some(api_key) = api_key {
            builder = builder.header("Authorization", format!("Bearer {api_key}"));
        }
        builder.body(Body::empty()).expect("request")
    }

    fn raw_request(path: &str, body: &str, api_key: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder()
            .method("POST")
            .uri(path)
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .header(HEADER_REQUEST_ID, "req-1");
        if let Some(api_key) = api_key {
            builder = builder.header("Authorization", format!("Bearer {api_key}"));
        }
        builder.body(Body::from(body.to_string())).expect("request")
    }

    fn json_request(path: &str, body: &Value, api_key: Option<&str>) -> Request<Body> {
        raw_request(path, &body.to_string(), api_key)
    }

    async fn json_body(response: Response) -> Value {
        let bytes = to_bytes(response.into_body(), MAX_ROUTER_BODY_BYTES)
            .await
            .expect("body bytes");
        serde_json::from_slice(&bytes).expect("json body")
    }

    fn mock_node(
        node_id: &'static str,
        model: &'static str,
        backend: BackendKind,
        running_requests: u32,
        completion_path: &'static str,
    ) -> axum::Router {
        mock_node_with_health(
            node_id,
            model,
            backend,
            running_requests,
            completion_path,
            NodeStatus::Ready,
            true,
        )
    }

    fn mock_node_with_health(
        node_id: &'static str,
        model: &'static str,
        backend: BackendKind,
        running_requests: u32,
        completion_path: &'static str,
        health_status: NodeStatus,
        llama_healthy: bool,
    ) -> axum::Router {
        let health_node = node_id;
        let capabilities_node = node_id;
        let load_node = node_id;
        let completion_node = node_id;
        let health_status_for_health = health_status.clone();
        let health_status_for_load = health_status;
        axum::Router::new()
            .route(
                "/v1/health",
                get(move || {
                    let status = health_status_for_health.clone();
                    async move {
                        Json(HealthResponse {
                            api_version: API_VERSION.to_string(),
                            node_id: health_node.to_string(),
                            node_name: health_node.to_string(),
                            status,
                            time_utc: String::new(),
                            uptime_s: 1,
                            llama_healthy,
                            active_model: None,
                            message: None,
                        })
                    }
                }),
            )
            .route(
                "/v1/capabilities",
                get(move || {
                    let backend = backend.clone();
                    async move {
                        Json(NodeCapabilities {
                            api_version: API_VERSION.to_string(),
                            lmml_version: "test".to_string(),
                            node_id: capabilities_node.to_string(),
                            node_name: capabilities_node.to_string(),
                            public_url: None,
                            roles: vec![NodeRole::LanWorker],
                            tags: vec!["test".to_string()],
                            privacy: PrivacyTier::LanOnly,
                            backend,
                            gpus: vec![GpuDescriptor {
                                name: format!("{capabilities_node} gpu"),
                                backend: BackendKind::Unknown,
                                arch: None,
                                vram_total_mb: 1024,
                                vram_free_mb: Some(512),
                            }],
                            models: vec![ModelDescriptor {
                                id: model.to_string(),
                                name: model.to_string(),
                                path: None,
                                architecture: None,
                                quantization: None,
                                context_length: Some(4096),
                                size_bytes: None,
                                loaded: true,
                                aliases: Vec::new(),
                            }],
                            max_context_tokens: Some(4096),
                            supports_infer: true,
                            supports_chat_completions: true,
                            supports_anthropic_messages: true,
                            supports_embeddings: true,
                            supports_server_control: false,
                            auth_required: false,
                            llama_cpp_commit: None,
                            agentq: None,
                            extra: BTreeMap::new(),
                        })
                    }
                }),
            )
            .route(
                "/v1/load",
                get(move || {
                    let status = health_status_for_load.clone();
                    async move {
                        Json(LoadResponse {
                            node_id: load_node.to_string(),
                            status,
                            cpu_usage_pct: 0.0,
                            memory_total_mb: 4096,
                            memory_used_mb: 1024,
                            gpus: Vec::new(),
                            running_requests,
                            completed_requests: 0,
                            failed_requests: 0,
                            tokens_in_total: 0,
                            tokens_out_total: 0,
                        })
                    }
                }),
            )
            .route(
                completion_path,
                post(move || async move {
                    Json(if completion_path == "/v1/infer" {
                        json!({
                            "request_id": "req-1",
                            "node_id": completion_node,
                            "model": model,
                            "output": completion_node,
                            "raw": null,
                            "latency_ms": 1,
                            "tokens_in": null,
                            "tokens_out": null,
                            "finish_reason": "stop"
                        })
                    } else {
                        json!({
                            "choices": [{
                                "message": {
                                    "role": "assistant",
                                    "content": completion_node
                                }
                            }]
                        })
                    })
                }),
            )
    }

    fn test_advertisement(node_id: &str, public_url: &str) -> LanNodeAdvertisement {
        LanNodeAdvertisement {
            magic: LAN_DISCOVERY_MAGIC.to_string(),
            version: LAN_DISCOVERY_VERSION,
            api_version: API_VERSION.to_string(),
            node_id: node_id.to_string(),
            node_name: node_id.to_string(),
            public_url: public_url.to_string(),
            backend: BackendKind::Cuda,
            gpus: Vec::new(),
            models: Vec::new(),
            auth_required: true,
            roles: vec![NodeRole::LanWorker],
            tags: vec!["lmml".to_string()],
            last_seen_utc: "2026-07-20T00:00:00Z".to_string(),
        }
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
