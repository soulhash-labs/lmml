use crate::models::DownloadEvent;
use std::path::Path;
use tokio::sync::mpsc;

/// Download a model from HuggingFace.
/// `model_id` can be:
///   - "hf://user/model" or "hf://user/model:quant"
///   - "user/model" or "user/model:quant"
///   - a full URL
///
/// Downloads to a `.part` file first, then renames on completion.
/// If a `.part` file already exists, attempts resume via `Range` header.
pub async fn download_model(
    model_id: &str,
    dest_dir: &Path,
    tx: mpsc::Sender<DownloadEvent>,
) -> Result<(), String> {
    let (download_url, filename) = parse_model_source(model_id)?;
    let final_path = dest_dir.join(&filename);

    // Check if a .part file exists for resume
    let part_path = dest_dir.join(format!("{filename}.part"));
    let existing_size = std::fs::metadata(&part_path)
        .ok()
        .map(|m| m.len())
        .unwrap_or(0);

    let client = reqwest::Client::new();
    let mut req = client.get(&download_url);

    if existing_size > 0 {
        req = req.header("Range", format!("bytes={existing_size}-"));
    }

    let response = req
        .send()
        .await
        .map_err(|e| format!("Failed to start download — check your internet connection.\n{e}"))?;

    let total = if existing_size > 0 {
        // Content-Range: bytes <start>-<end>/<total>
        response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.split('/').next_back())
            .and_then(|s| s.parse::<u64>().ok())
            .or_else(|| response.content_length().map(|l| l + existing_size))
            .unwrap_or(0)
    } else {
        response.content_length().unwrap_or(0)
    };

    let _ = tx
        .send(DownloadEvent::Progress {
            bytes: existing_size,
            total,
            speed: 0.0,
            eta_secs: 0.0,
        })
        .await;

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&part_path)
        .await
        .map_err(|e| format!("Failed to create file at {}\n{e}", part_path.display()))?;

    let mut stream = response.bytes_stream();
    let mut downloaded: u64 = existing_size;
    let start = std::time::Instant::now();

    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Download interrupted: {e}"))?;
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .map_err(|e| format!("Failed to write to disk: {e}"))?;

        downloaded += chunk.len() as u64;
        let elapsed = start.elapsed().as_secs_f64();
        let speed = if elapsed > 0.0 {
            downloaded as f64 / elapsed
        } else {
            0.0
        };
        let remaining = total.saturating_sub(downloaded);
        let eta_secs = if speed > 0.0 {
            remaining as f64 / speed
        } else {
            0.0
        };

        let _ = tx
            .send(DownloadEvent::Progress {
                bytes: downloaded,
                total,
                speed,
                eta_secs,
            })
            .await;
    }

    // Verify total if known
    if total > 0 && downloaded < total {
        return Err(format!(
            "Download incomplete: got {downloaded} of {total} bytes"
        ));
    }

    // Rename .part to final filename
    std::fs::rename(&part_path, &final_path)
        .map_err(|e| format!("Failed to finalize download: {e}"))?;

    let _ = tx.send(DownloadEvent::Complete(Ok(()))).await;
    Ok(())
}

/// Parse a model source string into a download URL and filename.
/// Supports: hf://user/model, hf://user/model:quant, user/model, direct URLs.
pub fn parse_model_source(input: &str) -> Result<(String, String), String> {
    let stripped = input.strip_prefix("hf://").unwrap_or(input);

    // Direct URL (only after stripping hf://, in case someone uses hf:// for HuggingFace URLs)
    if (stripped.starts_with("http://") || stripped.starts_with("https://"))
        && !stripped.starts_with("https://huggingface.co/")
    {
        let filename = stripped
            .split('/')
            .next_back()
            .unwrap_or("model.gguf")
            .to_string();
        return Ok((stripped.to_string(), filename));
    }

    // HuggingFace model ID: "user/model" or "user/model:quant"
    if let Some((user_model, quant)) = stripped.split_once(':') {
        let quant_filter = quant.to_lowercase();
        let url = format!("https://huggingface.co/{user_model}/resolve/main/{quant_filter}");
        let filename = format!("{user_model}-{quant_filter}.gguf").replace('/', "--");
        Ok((url, filename))
    } else if stripped.contains('/') {
        let api_url = format!("https://huggingface.co/api/models/{stripped}");
        Ok((
            api_url,
            format!(
                "{}.gguf",
                stripped.split('/').next_back().unwrap_or("model")
            ),
        ))
    } else {
        Err(format!(
            "Invalid model source '{input}'. Use 'hf://user/model', 'user/model:quant', or a direct URL."
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hf_prefix() {
        let (url, _) = parse_model_source("hf://ggml-org/gemma-3-1b-it-GGUF:Q4_K_M").unwrap();
        assert!(url.contains("huggingface.co/ggml-org/gemma-3-1b-it-GGUF/resolve/main/q4_k_m"));
    }

    #[test]
    fn test_parse_plain_model_id() {
        let (url, _) = parse_model_source("ggml-org/gemma-3-1b-it-GGUF:Q4_K_M").unwrap();
        assert!(url.contains("resolve/main/q4_k_m"));
    }

    #[test]
    fn test_parse_direct_url() {
        let (url, name) = parse_model_source("https://example.com/model.gguf").unwrap();
        assert_eq!(url, "https://example.com/model.gguf");
        assert_eq!(name, "model.gguf");
    }

    #[test]
    fn test_parse_invalid() {
        assert!(parse_model_source("just-a-name").is_err());
    }
}
