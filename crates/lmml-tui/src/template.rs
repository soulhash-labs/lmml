//! Verification of the exact llama-server prompt renderer.
//!
//! The verifier talks to the already-running llama-server instance through its
//! native `/apply-template` and `/tokenize` endpoints. This keeps production
//! rendering authoritative and prevents a Python-only renderer from becoming
//! an approval oracle.

use std::path::Path;
use std::time::Duration;

use lmml_substrate::{sha256_data, sha256_file, Hash256};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const MAX_FIXTURES: usize = 128;
const MAX_PROMPT_BYTES: usize = 1024 * 1024;

/// One deterministic conversation and its expected production-renderer output.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RendererFixture {
    /// Stable fixture identifier.
    pub case_id: String,
    /// Messages passed to llama-server's template renderer.
    pub messages: Value,
    /// SHA-256 of the exact rendered prompt bytes.
    pub rendered_prompt_hash: Hash256,
    /// Exact token IDs returned by the production tokenizer.
    pub token_ids: Vec<i32>,
}

/// Machine-readable evidence produced by renderer verification.
#[derive(Debug, Serialize)]
pub(crate) struct RendererVerificationReport {
    /// Renderer implementation used for the check.
    pub renderer_id: &'static str,
    /// Server endpoint used for rendering.
    pub endpoint: String,
    /// Hash of the exact llama-server executable supplied by the operator.
    pub executable_hash: Hash256,
    /// Hash of the exact model supplied by the operator.
    pub model_hash: Hash256,
    /// Number of verified fixtures.
    pub fixture_count: usize,
    /// Whether every rendered byte string and token sequence matched.
    pub passed: bool,
}

/// Verify prompt bytes and token IDs against the exact running llama-server.
pub(crate) async fn verify(
    endpoint: &str,
    binary: &Path,
    model: &Path,
    fixtures_path: &Path,
    api_key: Option<&str>,
    json_output: bool,
) -> i32 {
    match verify_inner(endpoint, binary, model, fixtures_path, api_key).await {
        Ok(report) => {
            if json_output {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report).expect("report serialization")
                );
            } else {
                println!("renderer: {}", report.renderer_id);
                println!("endpoint: {}", report.endpoint);
                println!("fixtures: {}", report.fixture_count);
                println!("status: passed");
            }
            0
        }
        Err(error) => {
            if json_output {
                println!("{}", json!({"status": "failed", "error": error}));
            } else {
                eprintln!("renderer verification failed: {error}");
            }
            1
        }
    }
}

async fn verify_inner(
    endpoint: &str,
    binary: &Path,
    model: &Path,
    fixtures_path: &Path,
    api_key: Option<&str>,
) -> Result<RendererVerificationReport, String> {
    let fixtures: Vec<RendererFixture> = serde_json::from_slice(
        &std::fs::read(fixtures_path).map_err(|error| format!("read fixtures: {error}"))?,
    )
    .map_err(|error| format!("parse fixtures: {error}"))?;
    if fixtures.is_empty() || fixtures.len() > MAX_FIXTURES {
        return Err(format!(
            "fixture count must be between 1 and {MAX_FIXTURES}"
        ));
    }
    for fixture in &fixtures {
        if fixture.case_id.trim().is_empty() {
            return Err("fixture case_id must not be empty".into());
        }
        if !fixture.messages.is_array() {
            return Err(format!(
                "fixture {} messages must be an array",
                fixture.case_id
            ));
        }
    }
    let executable_hash = sha256_file(binary).map_err(|error| format!("hash binary: {error}"))?;
    let model_hash = sha256_file(model).map_err(|error| format!("hash model: {error}"))?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|error| format!("create renderer client: {error}"))?;
    let endpoint = endpoint.trim_end_matches('/').to_string();
    let mut health_request = client.get(format!("{endpoint}/health"));
    if let Some(api_key) = api_key.filter(|value| !value.is_empty()) {
        health_request = health_request.bearer_auth(api_key);
    }
    let health = health_request
        .send()
        .await
        .map_err(|error| format!("renderer health request: {error}"))?;
    if !health.status().is_success() {
        return Err(format!("renderer is not ready: {}", health.status()));
    }
    let mut models_request = client.get(format!("{endpoint}/v1/models"));
    if let Some(api_key) = api_key.filter(|value| !value.is_empty()) {
        models_request = models_request.bearer_auth(api_key);
    }
    let models: Value = models_request
        .send()
        .await
        .map_err(|error| format!("renderer model identity request: {error}"))?
        .json()
        .await
        .map_err(|error| format!("renderer model identity response: {error}"))?;
    let served_model = models
        .get("data")
        .and_then(Value::as_array)
        .and_then(|models| models.first())
        .and_then(|model| model.get("id"))
        .and_then(Value::as_str)
        .ok_or_else(|| "renderer model identity response has no data[0].id".to_string())?;
    let expected_model = model.to_string_lossy();
    if served_model != expected_model {
        return Err(format!(
            "renderer loaded model {served_model}, expected {expected_model}"
        ));
    }
    for fixture in &fixtures {
        let apply_url = format!("{endpoint}/apply-template");
        let mut request = client
            .post(apply_url)
            .json(&json!({"messages": &fixture.messages}));
        if let Some(api_key) = api_key.filter(|value| !value.is_empty()) {
            request = request.bearer_auth(api_key);
        }
        let response = tokio::time::timeout(Duration::from_secs(60), request.send())
            .await
            .map_err(|_| format!("fixture {} renderer timed out", fixture.case_id))?
            .map_err(|error| format!("fixture {} renderer request: {error}", fixture.case_id))?;
        if !response.status().is_success() {
            return Err(format!(
                "fixture {} renderer returned {}",
                fixture.case_id,
                response.status()
            ));
        }
        let payload: Value = response
            .json()
            .await
            .map_err(|error| format!("fixture {} renderer response: {error}", fixture.case_id))?;
        let prompt = payload
            .get("prompt")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("fixture {} response has no prompt", fixture.case_id))?;
        if prompt.len() > MAX_PROMPT_BYTES {
            return Err(format!(
                "fixture {} prompt exceeds size limit",
                fixture.case_id
            ));
        }
        if sha256_data(prompt.as_bytes()) != fixture.rendered_prompt_hash {
            return Err(format!("fixture {} rendered bytes differ", fixture.case_id));
        }

        let mut request = client.post(format!("{endpoint}/tokenize")).json(&json!({
            "content": prompt,
            "add_special": false,
            "parse_special": true
        }));
        if let Some(api_key) = api_key.filter(|value| !value.is_empty()) {
            request = request.bearer_auth(api_key);
        }
        let response = tokio::time::timeout(Duration::from_secs(60), request.send())
            .await
            .map_err(|_| format!("fixture {} tokenizer timed out", fixture.case_id))?
            .map_err(|error| format!("fixture {} tokenizer request: {error}", fixture.case_id))?;
        if !response.status().is_success() {
            return Err(format!(
                "fixture {} tokenizer returned {}",
                fixture.case_id,
                response.status()
            ));
        }
        let payload: Value = response
            .json()
            .await
            .map_err(|error| format!("fixture {} tokenizer response: {error}", fixture.case_id))?;
        let tokens = payload
            .get("tokens")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("fixture {} response has no tokens", fixture.case_id))?;
        let actual: Vec<i32> = tokens
            .iter()
            .map(|token| {
                token
                    .as_i64()
                    .and_then(|value| i32::try_from(value).ok())
                    .ok_or_else(|| format!("fixture {} returned invalid token", fixture.case_id))
            })
            .collect::<Result<_, _>>()?;
        if actual != fixture.token_ids {
            return Err(format!("fixture {} token IDs differ", fixture.case_id));
        }
    }
    Ok(RendererVerificationReport {
        renderer_id: "llama.cpp/server-native",
        endpoint,
        executable_hash,
        model_hash,
        fixture_count: fixtures.len(),
        passed: true,
    })
}
