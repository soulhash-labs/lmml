//! Fail-closed request gateway for approved LMML runtimes.
//!
//! The gateway is the only component intended to expose an approved backend
//! to clients. It validates request data against the substrate-owned prompt
//! schema, rejects template controls, checks admission for every request, and
//! forwards only to a configured private backend URL.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use lmml_api::{
    sign_gateway_attestation, unix_now, GatewayAttestationClaims, HEADER_GATEWAY_ATTESTATION,
};
use lmml_substrate::{
    compute_admission_status, is_receipt_revoked, load_approval_receipt, load_approval_trust_root,
    load_revocation_state, validate_prompt_request, AdmissionState, AdmissionStatus, Hash256,
    RequestPromptSchema,
};
use reqwest::Client;
use serde_json::Value;
use thiserror::Error;
use tokio::sync::oneshot;
use tokio::sync::RwLock;

/// Maximum request body size before policy validation.
pub const DEFAULT_MAX_BODY_BYTES: usize = 1024 * 1024;

/// Runtime configuration for one gateway instance.
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    /// Public gateway bind address.
    pub listen: SocketAddr,
    /// Private backend base URL, never advertised to clients.
    pub backend_url: String,
    /// Request timeout applied to upstream calls.
    pub request_timeout: Duration,
    /// Maximum JSON body accepted by the gateway.
    pub max_body_bytes: usize,
    /// Optional bearer token required by client routes.
    pub api_key: Option<String>,
    /// Bearer token used only for the private backend transport.
    pub backend_api_key: Option<String>,
    /// Stable identity included in signed health attestations.
    pub gateway_id: Option<String>,
    /// Deployment identity included in signed health attestations.
    pub deployment_hash: Option<String>,
    /// Approval receipt identity included in signed health attestations.
    pub receipt_id: Option<String>,
    /// Approval receipt sequence included in signed health attestations.
    pub receipt_sequence: u64,
    /// Revocation epoch included in signed health attestations.
    pub revocation_epoch: u64,
    /// Operator-provisioned Ed25519 PKCS#8 key for health attestations.
    pub attestation_private_key: Option<Vec<u8>>,
    /// Lifetime of a health attestation.
    pub attestation_ttl: Duration,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            listen: SocketAddr::from(([127, 0, 0, 1], 8080)),
            backend_url: "http://127.0.0.1:1200".into(),
            request_timeout: Duration::from_secs(600),
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
            api_key: None,
            backend_api_key: None,
            gateway_id: None,
            deployment_hash: None,
            receipt_id: None,
            receipt_sequence: 0,
            revocation_epoch: 0,
            attestation_private_key: None,
            attestation_ttl: Duration::from_secs(30),
        }
    }
}

/// Mutable admission state shared by every request and active stream.
#[derive(Debug, Clone)]
pub struct GatewayAdmission {
    status: Arc<RwLock<AdmissionStatus>>,
    store: Arc<RwLock<Option<Arc<ApprovalStore>>>>,
    active: Arc<RwLock<bool>>,
}

/// Signed approval state that the gateway revalidates for every request.
#[derive(Debug, Clone)]
pub struct ApprovalStore {
    /// Signed receipt path.
    pub receipt_path: std::path::PathBuf,
    /// Pinned trust-root path.
    pub trust_root_path: std::path::PathBuf,
    /// Optional signed revocation state path.
    pub revocation_state_path: Option<std::path::PathBuf>,
    /// Expected prompt configuration identity.
    pub prompt_hash: Hash256,
    /// Expected runtime configuration identity.
    pub runtime_hash: Hash256,
    /// Expected deployment identity.
    pub deployment_hash: Hash256,
}

impl ApprovalStore {
    fn read_status(&self) -> Result<AdmissionStatus, String> {
        let trust_root =
            load_approval_trust_root(&self.trust_root_path).map_err(|error| error.to_string())?;
        let receipt = load_approval_receipt(&self.receipt_path, &trust_root)
            .map_err(|error| error.to_string())?;
        let revoked = match &self.revocation_state_path {
            Some(path) => {
                let state =
                    load_revocation_state(path, &trust_root).map_err(|error| error.to_string())?;
                is_receipt_revoked(&state, &receipt.receipt_id)
            }
            None => false,
        };
        Ok(compute_admission_status(
            Some(&receipt),
            &self.prompt_hash,
            &self.runtime_hash,
            &self.deployment_hash,
            revoked,
        ))
    }
}

impl GatewayAdmission {
    /// Create an admission state from a computed substrate status.
    pub fn new(status: AdmissionStatus) -> Self {
        Self {
            status: Arc::new(RwLock::new(status)),
            store: Arc::new(RwLock::new(None)),
            active: Arc::new(RwLock::new(true)),
        }
    }

    /// Create admission state backed by signed receipt and revocation files.
    pub fn with_store(status: AdmissionStatus, store: ApprovalStore) -> Self {
        Self {
            status: Arc::new(RwLock::new(status)),
            store: Arc::new(RwLock::new(Some(Arc::new(store)))),
            active: Arc::new(RwLock::new(true)),
        }
    }

    /// Create a store-backed admission that remains blocked until activation.
    pub fn pending_with_store(status: AdmissionStatus, store: ApprovalStore) -> Self {
        Self {
            status: Arc::new(RwLock::new(status)),
            store: Arc::new(RwLock::new(Some(Arc::new(store)))),
            active: Arc::new(RwLock::new(false)),
        }
    }

    /// Return the current computed state.
    pub async fn status(&self) -> AdmissionStatus {
        self.refresh().await;
        let status = self.status.read().await.clone();
        if !*self.active.read().await && status.state == AdmissionState::Approved {
            return AdmissionStatus {
                state: AdmissionState::Blocked,
                reason: "post-load verification pending".into(),
            };
        }
        status
    }

    /// Atomically expose the gateway after all post-load checks have passed.
    pub async fn activate_if_approved(&self) -> Result<(), String> {
        self.refresh().await;
        let status = self.status.read().await.clone();
        if status.state != AdmissionState::Approved {
            return Err(format!(
                "runtime admission is {:?}: {}",
                status.state, status.reason
            ));
        }
        *self.active.write().await = true;
        Ok(())
    }

    async fn refresh(&self) {
        let Some(store) = self.store.read().await.clone() else {
            return;
        };
        let refreshed = tokio::task::spawn_blocking(move || store.read_status()).await;
        let status = match refreshed {
            Ok(Ok(status)) => status,
            Ok(Err(reason)) => AdmissionStatus {
                state: AdmissionState::Blocked,
                reason: format!("approval store verification failed: {reason}"),
            },
            Err(error) => AdmissionStatus {
                state: AdmissionState::Blocked,
                reason: format!("approval store verification task failed: {error}"),
            },
        };
        *self.status.write().await = status;
    }

    /// Revoke the current deployment and stop new/active streamed work.
    pub async fn revoke(&self, reason: impl Into<String>) {
        *self.store.write().await = None;
        *self.status.write().await = AdmissionStatus {
            state: AdmissionState::Revoked,
            reason: reason.into(),
        };
        *self.active.write().await = false;
    }
}

/// Gateway application state.
#[derive(Clone)]
pub struct GatewayState {
    config: Arc<GatewayConfig>,
    schema: Arc<RequestPromptSchema>,
    admission: GatewayAdmission,
    client: Client,
}

impl GatewayState {
    /// Construct a gateway state after validating its basic private-backend contract.
    pub fn new(
        config: GatewayConfig,
        schema: RequestPromptSchema,
        admission: GatewayAdmission,
    ) -> Result<Self, GatewayError> {
        if config.backend_url.trim().is_empty() {
            return Err(GatewayError::InvalidConfiguration(
                "backend_url is empty".into(),
            ));
        }
        let backend = url::Url::parse(&config.backend_url).map_err(|error| {
            GatewayError::InvalidConfiguration(format!("backend_url is invalid: {error}"))
        })?;
        let private_host = matches!(backend.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
        if backend.scheme() != "http"
            || !private_host
            || backend.username() != ""
            || backend.password().is_some()
            || backend.query().is_some()
            || backend.fragment().is_some()
        {
            return Err(GatewayError::InvalidConfiguration(
                "backend_url must be an unauthenticated loopback HTTP URL".into(),
            ));
        }
        if config.max_body_bytes == 0 {
            return Err(GatewayError::InvalidConfiguration(
                "max_body_bytes must be positive".into(),
            ));
        }
        if config.backend_api_key.as_deref().is_some_and(str::is_empty) {
            return Err(GatewayError::InvalidConfiguration(
                "backend_api_key must not be empty".into(),
            ));
        }
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(GatewayError::HttpClient)?;
        Ok(Self {
            config: Arc::new(config),
            schema: Arc::new(schema),
            admission,
            client,
        })
    }

    /// Return the admission controller for lifecycle revocation.
    pub fn admission(&self) -> GatewayAdmission {
        self.admission.clone()
    }

    /// Build the HTTP router. The backend is never mounted as a public route.
    pub fn router(&self) -> Router {
        Router::new()
            .route("/health", get(health))
            .route("/v1/models", get(models))
            .route("/v1/chat/completions", post(chat_completions))
            .with_state(self.clone())
    }

    /// Serve the gateway until the supplied shutdown signal is received.
    pub async fn serve(
        self,
        listener: tokio::net::TcpListener,
        shutdown: oneshot::Receiver<()>,
    ) -> Result<(), std::io::Error> {
        axum::serve(listener, self.router())
            .with_graceful_shutdown(async {
                let _ = shutdown.await;
            })
            .await
    }
}

/// Gateway construction and transport errors.
#[derive(Debug, Error)]
pub enum GatewayError {
    /// Configuration would expose an unusable or ambiguous gateway.
    #[error("invalid gateway configuration: {0}")]
    InvalidConfiguration(String),
    /// HTTP client construction failed.
    #[error("gateway HTTP client construction failed: {0}")]
    HttpClient(#[source] reqwest::Error),
}

async fn health(State(state): State<GatewayState>) -> Response {
    let status = state.admission.status().await;
    if status.state == AdmissionState::Approved {
        if let Some(attestation) = state.attestation_header() {
            (
                StatusCode::OK,
                [
                    ("x-lmml-gateway", "approved"),
                    (HEADER_GATEWAY_ATTESTATION, attestation.as_str()),
                ],
                Json(serde_json::json!({"status": "ok", "gateway": "lmml"})),
            )
                .into_response()
        } else if approval_enforcement_enabled() {
            error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "approved gateway has no signed attestation configuration",
            )
        } else {
            (
                StatusCode::OK,
                [("x-lmml-gateway", "approved")],
                Json(serde_json::json!({"status": "ok", "gateway": "lmml"})),
            )
                .into_response()
        }
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"status": "unavailable", "reason": status.reason})),
        )
            .into_response()
    }
}

fn approval_enforcement_enabled() -> bool {
    matches!(
        std::env::var("LMML_APPROVAL_ENFORCEMENT").ok().as_deref(),
        Some("1" | "true" | "yes")
    )
}

impl GatewayState {
    fn attestation_header(&self) -> Option<String> {
        let config = &self.config;
        let key = config.attestation_private_key.as_deref()?;
        let gateway_id = config.gateway_id.clone()?;
        let deployment_hash = config.deployment_hash.clone()?;
        let receipt_id = config.receipt_id.clone()?;
        let now = unix_now();
        sign_gateway_attestation(
            GatewayAttestationClaims {
                schema_version: 1,
                gateway_id,
                deployment_hash,
                receipt_id,
                receipt_sequence: config.receipt_sequence,
                revocation_epoch: config.revocation_epoch,
                issued_at_unix: now,
                expires_at_unix: now + config.attestation_ttl.as_secs() as i64,
            },
            key,
        )
        .ok()
    }
}

async fn models(State(state): State<GatewayState>, headers: HeaderMap) -> Response {
    if let Err(response) = authorize(&state, &headers).await {
        return response;
    }
    let status = state.admission.status().await;
    if status.state != AdmissionState::Approved {
        return admission_response(status);
    }
    proxy_without_body(&state, "/v1/models").await
}

async fn chat_completions(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    request: Request<Body>,
) -> Response {
    if let Err(response) = authorize(&state, &headers).await {
        return response;
    }
    let status = state.admission.status().await;
    if status.state != AdmissionState::Approved {
        return admission_response(status);
    }
    let body = match axum::body::to_bytes(request.into_body(), state.config.max_body_bytes).await {
        Ok(body) => body,
        Err(error) => {
            return error_response(StatusCode::PAYLOAD_TOO_LARGE, error.to_string());
        }
    };
    let value: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(error) => return error_response(StatusCode::BAD_REQUEST, error.to_string()),
    };
    if let Err(error) = validate_prompt_request(&state.schema, &value) {
        return error_response(StatusCode::BAD_REQUEST, error.to_string());
    }
    let url = format!(
        "{}/v1/chat/completions",
        state.config.backend_url.trim_end_matches('/')
    );
    let upstream = match tokio::time::timeout(
        state.config.request_timeout,
        state
            .client
            .post(url)
            .header(header::CONTENT_TYPE, "application/json")
            .headers(backend_headers(&state))
            .body(body)
            .send(),
    )
    .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => return error_response(StatusCode::BAD_GATEWAY, error.to_string()),
        Err(_) => return error_response(StatusCode::GATEWAY_TIMEOUT, "backend request timed out"),
    };
    forward_response(&state, upstream).await
}

async fn proxy_without_body(state: &GatewayState, path: &str) -> Response {
    let url = format!("{}{}", state.config.backend_url.trim_end_matches('/'), path);
    match tokio::time::timeout(
        state.config.request_timeout,
        state.client.get(url).headers(backend_headers(state)).send(),
    )
    .await
    {
        Ok(Ok(response)) => forward_response(state, response).await,
        Ok(Err(error)) => error_response(StatusCode::BAD_GATEWAY, error.to_string()),
        Err(_) => error_response(StatusCode::GATEWAY_TIMEOUT, "backend request timed out"),
    }
}

fn backend_headers(state: &GatewayState) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Some(api_key) = state.config.backend_api_key.as_deref() {
        if let Ok(value) = format!("Bearer {api_key}").parse() {
            headers.insert(header::AUTHORIZATION, value);
        }
    }
    headers
}

async fn forward_response(state: &GatewayState, response: reqwest::Response) -> Response {
    let status =
        StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .cloned();
    let upstream = response;
    let admission = state.admission.clone();
    let timeout = state.config.request_timeout;
    let stream = futures_util::stream::unfold(
        (upstream, admission, Instant::now() + timeout),
        |(mut response, admission, deadline)| async move {
            loop {
                if Instant::now() >= deadline
                    || admission.status().await.state != AdmissionState::Approved
                {
                    return None;
                }
                match tokio::time::timeout(Duration::from_millis(250), response.chunk()).await {
                    Ok(Ok(Some(bytes))) => {
                        if admission.status().await.state != AdmissionState::Approved {
                            return None;
                        }
                        return Some((
                            Ok::<_, std::io::Error>(bytes),
                            (response, admission, deadline),
                        ));
                    }
                    Ok(Ok(None)) => return None,
                    Ok(Err(error)) => {
                        return Some((
                            Err(std::io::Error::other(error.to_string())),
                            (response, admission, deadline),
                        ));
                    }
                    Err(_) => continue,
                }
            }
        },
    );
    let mut builder = Response::builder().status(status);
    if let Some(content_type) = content_type {
        builder = builder.header(header::CONTENT_TYPE, content_type);
    }
    builder
        .body(Body::from_stream(stream.map(|chunk| {
            chunk.map_err(|error| std::io::Error::other(error.to_string()))
        })))
        .unwrap_or_else(|error| {
            error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
        })
}

async fn authorize(state: &GatewayState, headers: &HeaderMap) -> Result<(), Response> {
    if let Some(expected) = state.config.api_key.as_deref() {
        let supplied = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));
        if supplied != Some(expected) {
            return Err(error_response(
                StatusCode::UNAUTHORIZED,
                "missing or invalid bearer token",
            ));
        }
    }
    Ok(())
}

fn admission_response(status: AdmissionStatus) -> Response {
    error_response(
        StatusCode::SERVICE_UNAVAILABLE,
        format!("runtime admission is {:?}: {}", status.state, status.reason),
    )
}

fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(serde_json::json!({"error": message.into()}))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn schema() -> RequestPromptSchema {
        RequestPromptSchema {
            schema_version: 1,
            schema_id: "chat-v1".into(),
            allowed_roles: vec!["user".into(), "assistant".into()],
            allowed_content_types: vec!["text".into()],
            max_tools: 0,
            max_nesting: 8,
            max_request_bytes: 4096,
            max_context_tokens: 1024,
            reasoning_controls: false,
            assistant_prefill: false,
            permitted_template_kwargs: Default::default(),
        }
    }

    fn gateway() -> GatewayState {
        GatewayState::new(
            GatewayConfig::default(),
            schema(),
            GatewayAdmission::new(AdmissionStatus {
                state: AdmissionState::Approved,
                reason: "test".into(),
            }),
        )
        .expect("gateway")
    }

    #[tokio::test]
    async fn unapproved_requests_fail_closed_before_backend_access() {
        let state = GatewayState::new(
            GatewayConfig::default(),
            schema(),
            GatewayAdmission::new(AdmissionStatus {
                state: AdmissionState::Invalidated,
                reason: "changed".into(),
            }),
        )
        .expect("gateway");
        let response = state
            .router()
            .oneshot(
                Request::post("/v1/chat/completions")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"messages":[{"role":"user","content":"hi"}]}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn gateway_rejects_direct_template_override() {
        let state = gateway();
        let response = state
            .router()
            .oneshot(
                Request::post("/v1/chat/completions")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"messages":[{"role":"user","content":"hi"}],"chat_template":"unsafe"}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn revocation_blocks_new_requests() {
        let state = gateway();
        state.admission().revoke("operator revoked").await;
        let response = state
            .router()
            .oneshot(
                Request::post("/v1/chat/completions")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"messages":[{"role":"user","content":"hi"}]}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn approval_store_failure_fails_closed() {
        let admission = GatewayAdmission::with_store(
            AdmissionStatus {
                state: AdmissionState::Approved,
                reason: "test".into(),
            },
            ApprovalStore {
                receipt_path: "/missing/receipt.json".into(),
                trust_root_path: "/missing/trust-root.json".into(),
                revocation_state_path: None,
                prompt_hash: Hash256::parse("00".repeat(32)).expect("hash"),
                runtime_hash: Hash256::parse("00".repeat(32)).expect("hash"),
                deployment_hash: Hash256::parse("00".repeat(32)).expect("hash"),
            },
        );
        assert_eq!(admission.status().await.state, AdmissionState::Blocked);
    }

    #[tokio::test]
    async fn pending_admission_stays_blocked_until_activation() {
        let admission = GatewayAdmission::pending_with_store(
            AdmissionStatus {
                state: AdmissionState::Approved,
                reason: "test".into(),
            },
            ApprovalStore {
                receipt_path: "/missing/receipt.json".into(),
                trust_root_path: "/missing/trust-root.json".into(),
                revocation_state_path: None,
                prompt_hash: Hash256::parse("00".repeat(32)).expect("hash"),
                runtime_hash: Hash256::parse("00".repeat(32)).expect("hash"),
                deployment_hash: Hash256::parse("00".repeat(32)).expect("hash"),
            },
        );
        assert_eq!(admission.status().await.state, AdmissionState::Blocked);
    }

    #[test]
    fn gateway_rejects_non_loopback_backend() {
        let result = GatewayState::new(
            GatewayConfig {
                backend_url: "http://192.0.2.1:1200".into(),
                ..GatewayConfig::default()
            },
            schema(),
            GatewayAdmission::new(AdmissionStatus {
                state: AdmissionState::Approved,
                reason: "test".into(),
            }),
        );
        let error = match result {
            Ok(_) => panic!("remote backend must be rejected"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("loopback"));
    }
}
