//! Live llama-server property inspection.
//!
//! This module reads `/props` after readiness so LMML can bind external
//! harness metadata to the context capacity the running server exposes.

use std::time::Duration;

use serde_json::Value;

use super::{base_url, ServerError, HEALTH_CHECK_TIMEOUT_SECS};

/// Context and parallelism reported by a live llama-server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerProperties {
    /// Maximum context available to one request slot.
    pub context_per_slot: u32,
    /// Number of parallel request slots.
    pub total_slots: u32,
}

/// Query a llama-server's live context properties.
pub async fn query_server_properties(
    host: &str,
    port: u16,
    api_key: Option<&str>,
) -> Result<ServerProperties, ServerError> {
    query_server_properties_endpoint(&base_url(host, port), api_key).await
}

/// Verify that a live llama-server provides the required per-slot context.
pub async fn verify_context_capacity(
    host: &str,
    port: u16,
    expected_context_per_slot: u32,
    api_key: Option<&str>,
) -> Result<ServerProperties, ServerError> {
    verify_context_capacity_endpoint(&base_url(host, port), expected_context_per_slot, api_key)
        .await
}

/// Verify context capacity for an operator-supplied llama-server endpoint.
pub async fn verify_context_capacity_endpoint(
    endpoint: &str,
    expected_context_per_slot: u32,
    api_key: Option<&str>,
) -> Result<ServerProperties, ServerError> {
    let properties = query_server_properties_endpoint(endpoint, api_key).await?;
    if properties.context_per_slot < expected_context_per_slot {
        return Err(ServerError::ContextCapacityMismatch {
            expected: expected_context_per_slot,
            actual: properties.context_per_slot,
            slots: properties.total_slots,
        });
    }
    Ok(properties)
}

async fn query_server_properties_endpoint(
    endpoint: &str,
    api_key: Option<&str>,
) -> Result<ServerProperties, ServerError> {
    let url = properties_url(endpoint);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(HEALTH_CHECK_TIMEOUT_SECS))
        .build()
        .map_err(ServerError::HttpClient)?;
    let mut request = client.get(&url);
    if let Some(api_key) = api_key.filter(|value| !value.is_empty()) {
        request = request.bearer_auth(api_key);
    }
    let response = request
        .send()
        .await
        .map_err(|source| ServerError::PropertiesRequest {
            url: url.clone(),
            source,
        })?;
    if !response.status().is_success() {
        return Err(ServerError::PropertiesStatus {
            url,
            status: response.status().as_u16(),
        });
    }
    let value =
        response
            .json::<Value>()
            .await
            .map_err(|source| ServerError::PropertiesRequest {
                url: url.clone(),
                source,
            })?;
    parse_server_properties(&url, &value)
}

fn properties_url(endpoint: &str) -> String {
    let endpoint = endpoint.trim_end_matches('/');
    let root = endpoint.strip_suffix("/v1").unwrap_or(endpoint);
    format!("{root}/props")
}

fn parse_server_properties(url: &str, value: &Value) -> Result<ServerProperties, ServerError> {
    let context_per_slot = value
        .pointer("/default_generation_settings/n_ctx")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| ServerError::PropertiesShape {
            url: url.to_string(),
            field: "default_generation_settings.n_ctx",
        })?;
    let total_slots = value
        .get("total_slots")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| ServerError::PropertiesShape {
            url: url.to_string(),
            field: "total_slots",
        })?;
    Ok(ServerProperties {
        context_per_slot,
        total_slots,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn properties_url_accepts_root_or_v1_endpoint() {
        assert_eq!(
            properties_url("http://127.0.0.1:8080"),
            "http://127.0.0.1:8080/props"
        );
        assert_eq!(
            properties_url("http://127.0.0.1:8080/v1/"),
            "http://127.0.0.1:8080/props"
        );
    }

    #[test]
    fn parses_context_per_slot_and_parallel_slots() {
        let value = serde_json::json!({
            "default_generation_settings": {"n_ctx": 131072},
            "total_slots": 2
        });

        assert_eq!(
            parse_server_properties("http://localhost/props", &value).expect("properties"),
            ServerProperties {
                context_per_slot: 131_072,
                total_slots: 2,
            }
        );
    }

    #[test]
    fn rejects_properties_without_context() {
        let error = parse_server_properties(
            "http://localhost/props",
            &serde_json::json!({"total_slots": 1}),
        )
        .expect_err("missing context");

        assert!(matches!(
            error,
            ServerError::PropertiesShape {
                field: "default_generation_settings.n_ctx",
                ..
            }
        ));
    }
}
