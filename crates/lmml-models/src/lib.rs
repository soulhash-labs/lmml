//! GGUF model registry and metadata management for lmml.
//!
//! This crate scans local model directories, parses GGUF headers, estimates
//! whether models fit in detected GPU VRAM, and provides registry operations for
//! aliases and deletion.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use lmml_detect::GpuInfo;
use reqwest::header::{CONTENT_LENGTH, RANGE};
use serde::Deserialize;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

/// A local GGUF model entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelEntry {
    /// Model file path.
    pub path: PathBuf,
    /// Display name from GGUF metadata or filename.
    pub name: String,
    /// File size in bytes.
    pub size_bytes: u64,
    /// Quantization string, such as `Q4_K_M`.
    pub quant: String,
    /// Context length from GGUF metadata.
    pub context_length: Option<u32>,
    /// Architecture from GGUF metadata.
    pub architecture: Option<String>,
    /// True when the model came from an alias/external path.
    pub aliased: bool,
}

impl ModelEntry {
    /// Estimate whether this model fits in detected GPU VRAM.
    pub fn vram_fit(&self, gpus: &[GpuInfo]) -> VramFit {
        let Some(total_vram_mb) = gpus.iter().map(|gpu| gpu.memory_total_mb).max() else {
            return VramFit::CpuOnly;
        };
        let model_mb = bytes_to_mib(self.size_bytes);
        let overhead_mb = 768;
        let estimated_mb = model_mb.saturating_add(overhead_mb);
        if estimated_mb <= total_vram_mb {
            return VramFit::Full {
                vram_used_mb: estimated_mb,
                vram_free_mb: total_vram_mb - estimated_mb,
            };
        }

        if total_vram_mb > overhead_mb {
            let usable_mb = total_vram_mb - overhead_mb;
            let recommended_ngl = ((usable_mb as f64 / model_mb.max(1) as f64) * 80.0)
                .floor()
                .clamp(1.0, 80.0) as i32;
            return VramFit::Partial {
                recommended_ngl,
                cpu_layers: 80 - recommended_ngl,
            };
        }

        VramFit::TooLarge {
            model_mb,
            vram_mb: total_vram_mb,
        }
    }

    /// Returns the recommended `-ngl` for this model on the detected GPUs.
    pub fn recommended_ngl(&self, gpus: &[GpuInfo]) -> i32 {
        match self.vram_fit(gpus) {
            VramFit::Full { .. } => -1,
            VramFit::Partial {
                recommended_ngl, ..
            } => recommended_ngl,
            VramFit::TooLarge { .. } | VramFit::CpuOnly => 0,
        }
    }
}

/// VRAM fit estimate for a model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VramFit {
    /// Entire model fits in VRAM.
    Full {
        /// Estimated VRAM used in MiB.
        vram_used_mb: u64,
        /// Remaining VRAM in MiB.
        vram_free_mb: u64,
    },
    /// Partial GPU offload is recommended.
    Partial {
        /// Recommended `-ngl` value.
        recommended_ngl: i32,
        /// Approximate layers remaining on CPU.
        cpu_layers: i32,
    },
    /// Model is too large for meaningful GPU offload.
    TooLarge {
        /// Model size in MiB.
        model_mb: u64,
        /// GPU VRAM in MiB.
        vram_mb: u64,
    },
    /// No GPU available.
    CpuOnly,
}

/// Local model registry configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRegistry {
    /// Primary models directory.
    pub models_dir: PathBuf,
    /// External model files or directories.
    pub aliases: Vec<PathBuf>,
}

impl ModelRegistry {
    /// Scan the registry for GGUF files.
    pub async fn scan(&self) -> Vec<ModelEntry> {
        let mut models = Vec::new();
        scan_path(&self.models_dir, false, &mut models).await;
        for alias in &self.aliases {
            scan_path(alias, true, &mut models).await;
        }
        models.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.path.cmp(&b.path)));
        models.dedup_by(|a, b| a.path == b.path);
        models
    }

    /// Add an external alias path to this in-memory registry.
    pub fn add_alias(&mut self, path: PathBuf) -> Result<(), RegistryError> {
        if !path.exists() {
            return Err(RegistryError::MissingPath { path });
        }
        if !self.aliases.iter().any(|alias| alias == &path) {
            self.aliases.push(path);
        }
        Ok(())
    }

    /// Download a model URL into `models_dir`, resuming a partial file when present.
    pub async fn download(
        &self,
        url: &str,
        on_progress: impl Fn(DownloadProgress) + Send + 'static,
    ) -> Result<ModelEntry, DownloadError> {
        self.download_with_client(url, reqwest::Client::new(), on_progress)
            .await
    }

    async fn download_with_client(
        &self,
        url: &str,
        client: reqwest::Client,
        on_progress: impl Fn(DownloadProgress) + Send + 'static,
    ) -> Result<ModelEntry, DownloadError> {
        tokio::fs::create_dir_all(&self.models_dir)
            .await
            .map_err(DownloadError::Io)?;
        let filename = filename_from_url(url)?;
        let final_path = self.models_dir.join(&filename);
        let part_path = self.models_dir.join(format!("{filename}.part"));
        let mut resumed_from = tokio::fs::metadata(&part_path)
            .await
            .map(|metadata| metadata.len())
            .unwrap_or(0);

        let mut request = client.get(url);
        if let Some(range) = range_header(resumed_from) {
            request = request.header(RANGE, range);
        }
        let mut response = request.send().await.map_err(DownloadError::Http)?;
        if !response.status().is_success()
            && response.status() != reqwest::StatusCode::PARTIAL_CONTENT
        {
            return Err(DownloadError::Status(response.status().as_u16()));
        }
        let append = resumed_from > 0 && response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
        if resumed_from > 0 && !append {
            resumed_from = 0;
        }

        let total_bytes = response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .map(|length| length + resumed_from);
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(&part_path)
            .await
            .map_err(DownloadError::Io)?;
        let mut bytes_received = resumed_from;

        while let Some(chunk) = response.chunk().await.map_err(DownloadError::Http)? {
            file.write_all(&chunk).await.map_err(DownloadError::Io)?;
            bytes_received += chunk.len() as u64;
            on_progress(DownloadProgress {
                bytes_received,
                total_bytes,
                resumed_from,
            });
        }
        file.flush().await.map_err(DownloadError::Io)?;
        drop(file);
        tokio::fs::rename(&part_path, &final_path)
            .await
            .map_err(DownloadError::Io)?;
        parse_model_file(&final_path, false)
            .await
            .ok_or_else(|| DownloadError::InvalidDownloadedFile(final_path))
    }

    /// Delete a model file.
    pub fn delete(&self, entry: &ModelEntry) -> Result<(), RegistryError> {
        if entry.path.exists() {
            std::fs::remove_file(&entry.path).map_err(|source| RegistryError::DeleteFailed {
                path: entry.path.clone(),
                source,
            })?;
        }
        Ok(())
    }
}

/// Download progress event used by the Milestone 9 downloader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadProgress {
    /// Bytes received so far.
    pub bytes_received: u64,
    /// Total expected bytes.
    pub total_bytes: Option<u64>,
    /// Byte offset resumed from.
    pub resumed_from: u64,
}

/// Hugging Face model search query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HfSearchQuery {
    /// Search keywords.
    pub keywords: String,
    /// Optional architecture filter, such as `llama` or `mistral`.
    pub architecture: Option<String>,
    /// Optional quantization tier filter.
    pub quant_filter: Option<QuantTier>,
    /// Maximum result count.
    pub max_results: usize,
}

/// Quantization tier filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuantTier {
    /// Q4 quantized models.
    Q4,
    /// Q5 quantized models.
    Q5,
    /// Q6 quantized models.
    Q6,
    /// Q8 quantized models.
    Q8,
    /// FP16 models.
    F16,
    /// FP32 models.
    F32,
}

impl QuantTier {
    fn needle(self) -> &'static str {
        match self {
            QuantTier::Q4 => "Q4",
            QuantTier::Q5 => "Q5",
            QuantTier::Q6 => "Q6",
            QuantTier::Q8 => "Q8",
            QuantTier::F16 => "F16",
            QuantTier::F32 => "F32",
        }
    }
}

/// Hugging Face GGUF file result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HfModelResult {
    /// Hugging Face repository id.
    pub repo_id: String,
    /// GGUF filename.
    pub filename: String,
    /// File size in bytes, if reported.
    pub size_bytes: u64,
    /// Repository download count.
    pub downloads: u64,
    /// Direct resolve URL for the file.
    pub url: String,
}

/// Search Hugging Face for GGUF model files.
pub async fn search_huggingface(query: HfSearchQuery) -> Result<Vec<HfModelResult>, HfError> {
    let client = reqwest::Client::new();
    search_huggingface_with_client(query, client).await
}

async fn search_huggingface_with_client(
    query: HfSearchQuery,
    client: reqwest::Client,
) -> Result<Vec<HfModelResult>, HfError> {
    let response = client
        .get("https://huggingface.co/api/models")
        .query(&[
            ("search", query.keywords.as_str()),
            ("filter", "gguf"),
            ("full", "true"),
        ])
        .send()
        .await
        .map_err(HfError::Http)?;
    if !response.status().is_success() {
        return Err(HfError::Status(response.status().as_u16()));
    }
    let body = response.text().await.map_err(HfError::Http)?;
    parse_hf_search_response(&query, &body)
}

/// Parse Hugging Face model API JSON into GGUF file results.
pub fn parse_hf_search_response(
    query: &HfSearchQuery,
    body: &str,
) -> Result<Vec<HfModelResult>, HfError> {
    let repos: Vec<HfRepo> = serde_json::from_str(body).map_err(HfError::Json)?;
    let mut results = Vec::new();
    for repo in repos {
        let repo_id = repo.id.or(repo.model_id).unwrap_or_default();
        if repo_id.is_empty() {
            continue;
        }
        if let Some(architecture) = &query.architecture {
            let haystack = format!("{} {}", repo_id, repo.tags.join(" ")).to_lowercase();
            if !haystack.contains(&architecture.to_lowercase()) {
                continue;
            }
        }
        for sibling in repo.siblings {
            let Some(filename) = sibling.rfilename else {
                continue;
            };
            if !filename.to_lowercase().ends_with(".gguf") {
                continue;
            }
            if let Some(quant) = query.quant_filter {
                if !filename.to_uppercase().contains(quant.needle()) {
                    continue;
                }
            }
            results.push(HfModelResult {
                repo_id: repo_id.clone(),
                filename: filename.clone(),
                size_bytes: sibling.size.unwrap_or(0),
                downloads: repo.downloads.unwrap_or(0),
                url: hf_resolve_url(&repo_id, &filename),
            });
            if results.len() >= query.max_results {
                return Ok(results);
            }
        }
    }
    Ok(results)
}

/// Hugging Face API error.
#[derive(Debug, Error)]
pub enum HfError {
    /// HTTP request failed.
    #[error("Hugging Face request failed: {0}")]
    Http(#[source] reqwest::Error),
    /// API returned a non-success status.
    #[error("Hugging Face returned HTTP status {0}")]
    Status(u16),
    /// JSON response did not match expected shape.
    #[error("failed to parse Hugging Face response: {0}")]
    Json(#[source] serde_json::Error),
}

/// Registry operation error.
#[derive(Debug, Error)]
pub enum RegistryError {
    /// Alias path does not exist.
    #[error("model alias path does not exist: {path}")]
    MissingPath {
        /// Missing path.
        path: PathBuf,
    },
    /// Model deletion failed.
    #[error("failed to delete model {path}: {source}")]
    DeleteFailed {
        /// Model path.
        path: PathBuf,
        /// IO source error.
        #[source]
        source: std::io::Error,
    },
}

/// Download error.
#[derive(Debug, Error)]
pub enum DownloadError {
    /// HTTP request failed.
    #[error("download request failed: {0}")]
    Http(#[source] reqwest::Error),
    /// Server returned a non-success status.
    #[error("download returned HTTP status {0}")]
    Status(u16),
    /// Filesystem IO failed.
    #[error("download IO failed: {0}")]
    Io(#[source] std::io::Error),
    /// URL did not contain a valid filename.
    #[error("download URL does not contain a valid filename: {0}")]
    InvalidFilename(String),
    /// Downloaded file could not be parsed as a model.
    #[error("downloaded file is not a valid model: {0}")]
    InvalidDownloadedFile(PathBuf),
}

#[derive(Debug, Deserialize)]
struct HfRepo {
    id: Option<String>,
    #[serde(rename = "modelId")]
    model_id: Option<String>,
    siblings: Vec<HfSibling>,
    downloads: Option<u64>,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct HfSibling {
    rfilename: Option<String>,
    size: Option<u64>,
}

/// Parsed GGUF metadata header.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GgufMetadata {
    /// GGUF version.
    pub version: u32,
    /// Tensor count in the file.
    pub tensor_count: u64,
    /// Model display name.
    pub name: Option<String>,
    /// Architecture string.
    pub architecture: Option<String>,
    /// Context length.
    pub context_length: Option<u32>,
    /// Embedding length.
    pub embedding_length: Option<u32>,
    /// Transformer block count.
    pub block_count: Option<u32>,
    /// Quantization inferred from tensor types.
    pub quant: Option<String>,
}

/// Parse a GGUF file into a [`ModelEntry`].
pub async fn parse_model_file(path: impl AsRef<Path>, aliased: bool) -> Option<ModelEntry> {
    let path = path.as_ref();
    let metadata = tokio::fs::metadata(path).await.ok()?;
    if !metadata.is_file() {
        return None;
    }
    if path.extension().is_none_or(|ext| ext != "gguf") {
        return None;
    }

    let gguf = parse_gguf_metadata(path).await.ok();
    let filename = path.file_name()?.to_string_lossy();
    let architecture = gguf
        .as_ref()
        .and_then(|metadata| metadata.architecture.clone());
    let quant = gguf
        .as_ref()
        .and_then(|metadata| metadata.quant.clone())
        .or_else(|| quant_from_filename(&filename))
        .unwrap_or_else(|| "unknown".to_string());
    let name = gguf
        .as_ref()
        .and_then(|metadata| metadata.name.clone())
        .unwrap_or_else(|| model_name_from_filename(&filename));

    Some(ModelEntry {
        path: path.to_path_buf(),
        name,
        size_bytes: metadata.len(),
        quant,
        context_length: gguf.as_ref().and_then(|metadata| metadata.context_length),
        architecture,
        aliased,
    })
}

/// Parse the GGUF binary metadata header.
pub async fn parse_gguf_metadata(path: impl AsRef<Path>) -> Result<GgufMetadata, GgufError> {
    let mut file = tokio::fs::File::open(path.as_ref())
        .await
        .map_err(GgufError::Io)?;
    parse_gguf_reader(&mut file).await
}

async fn parse_gguf_reader<R>(reader: &mut R) -> Result<GgufMetadata, GgufError>
where
    R: AsyncRead + Unpin,
{
    let mut magic = [0_u8; 4];
    reader.read_exact(&mut magic).await.map_err(GgufError::Io)?;
    if &magic != b"GGUF" {
        return Err(GgufError::InvalidMagic);
    }

    let version = read_u32(reader).await?;
    let tensor_count = read_u64(reader).await?;
    let metadata_count = read_u64(reader).await?;
    let mut metadata = GgufMetadata {
        version,
        tensor_count,
        ..GgufMetadata::default()
    };

    for _ in 0..metadata_count {
        let key = read_string(reader).await?;
        let value_type = read_u32(reader).await?;
        let value = read_value(reader, value_type).await?;
        apply_metadata_value(&mut metadata, &key, value);
    }

    let mut tensor_types = BTreeSet::new();
    for _ in 0..tensor_count {
        let _name = read_string(reader).await?;
        let dimensions = read_u32(reader).await?;
        for _ in 0..dimensions {
            let _dimension = read_u64(reader).await?;
        }
        let tensor_type = read_u32(reader).await?;
        let _offset = read_u64(reader).await?;
        if let Some(quant) = tensor_type_to_quant(tensor_type) {
            tensor_types.insert(quant.to_string());
        }
    }
    metadata.quant = quant_from_tensor_types(&tensor_types);
    Ok(metadata)
}

/// GGUF parse error.
#[derive(Debug, Error)]
pub enum GgufError {
    /// IO error while reading.
    #[error("failed to read GGUF data: {0}")]
    Io(#[source] std::io::Error),
    /// Magic bytes did not equal `GGUF`.
    #[error("invalid GGUF magic")]
    InvalidMagic,
    /// Unsupported metadata value type.
    #[error("unsupported GGUF metadata value type {0}")]
    UnsupportedValueType(u32),
}

#[derive(Debug, Clone, PartialEq)]
enum GgufValue {
    U8(u8),
    I8(i8),
    U16(u16),
    I16(i16),
    U32(u32),
    I32(i32),
    U64(u64),
    I64(i64),
    F32(f32),
    F64(f64),
    Bool(bool),
    String(String),
    Array(Vec<GgufValue>),
}

async fn scan_path(path: &Path, aliased: bool, models: &mut Vec<ModelEntry>) {
    if path.is_file() {
        if let Some(entry) = parse_model_file(path, aliased).await {
            models.push(entry);
        }
        return;
    }
    let mut entries = match tokio::fs::read_dir(path).await {
        Ok(entries) => entries,
        Err(_) => return,
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with('.'))
        {
            continue;
        }
        if path.is_dir() {
            scan_path_boxed(path, aliased, models).await;
        } else if let Some(entry) = parse_model_file(path, aliased).await {
            models.push(entry);
        }
    }
}

fn scan_path_boxed<'a>(
    path: PathBuf,
    aliased: bool,
    models: &'a mut Vec<ModelEntry>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
    Box::pin(async move {
        scan_path(&path, aliased, models).await;
    })
}

fn apply_metadata_value(metadata: &mut GgufMetadata, key: &str, value: GgufValue) {
    match (key, value) {
        ("general.name", GgufValue::String(value)) => metadata.name = Some(value),
        ("general.architecture", GgufValue::String(value)) => metadata.architecture = Some(value),
        (key, value) if key.ends_with(".context_length") => {
            metadata.context_length = value_to_u32(value);
        }
        (key, value) if key.ends_with(".embedding_length") => {
            metadata.embedding_length = value_to_u32(value);
        }
        (key, value) if key.ends_with(".block_count") => metadata.block_count = value_to_u32(value),
        _ => {}
    }
}

fn value_to_u32(value: GgufValue) -> Option<u32> {
    match value {
        GgufValue::U8(value) => Some(value.into()),
        GgufValue::I8(value) => value.try_into().ok(),
        GgufValue::U16(value) => Some(value.into()),
        GgufValue::I16(value) => value.try_into().ok(),
        GgufValue::U32(value) => Some(value),
        GgufValue::I32(value) => value.try_into().ok(),
        GgufValue::U64(value) => value.try_into().ok(),
        GgufValue::I64(value) => value.try_into().ok(),
        GgufValue::F32(_)
        | GgufValue::F64(_)
        | GgufValue::Bool(_)
        | GgufValue::String(_)
        | GgufValue::Array(_) => None,
    }
}

async fn read_value<R>(reader: &mut R, value_type: u32) -> Result<GgufValue, GgufError>
where
    R: AsyncRead + Unpin,
{
    match value_type {
        0 => Ok(GgufValue::U8(read_u8(reader).await?)),
        1 => Ok(GgufValue::I8(read_i8(reader).await?)),
        2 => Ok(GgufValue::U16(read_u16(reader).await?)),
        3 => Ok(GgufValue::I16(read_i16(reader).await?)),
        4 => Ok(GgufValue::U32(read_u32(reader).await?)),
        5 => Ok(GgufValue::I32(read_i32(reader).await?)),
        6 => Ok(GgufValue::F32(read_f32(reader).await?)),
        7 => Ok(GgufValue::Bool(read_u8(reader).await? != 0)),
        8 => Ok(GgufValue::String(read_string(reader).await?)),
        9 => read_array(reader).await,
        10 => Ok(GgufValue::U64(read_u64(reader).await?)),
        11 => Ok(GgufValue::I64(read_i64(reader).await?)),
        12 => Ok(GgufValue::F64(read_f64(reader).await?)),
        other => Err(GgufError::UnsupportedValueType(other)),
    }
}

async fn read_array<R>(reader: &mut R) -> Result<GgufValue, GgufError>
where
    R: AsyncRead + Unpin,
{
    let inner_type = read_u32(reader).await?;
    let len = read_u64(reader).await?;
    let mut values = Vec::new();
    for _ in 0..len {
        values.push(Box::pin(read_value(reader, inner_type)).await?);
    }
    Ok(GgufValue::Array(values))
}

async fn read_string<R>(reader: &mut R) -> Result<String, GgufError>
where
    R: AsyncRead + Unpin,
{
    let len = read_u64(reader).await?;
    let mut bytes = vec![0_u8; len as usize];
    reader.read_exact(&mut bytes).await.map_err(GgufError::Io)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

async fn read_u8<R: AsyncRead + Unpin>(reader: &mut R) -> Result<u8, GgufError> {
    let mut buf = [0_u8; 1];
    reader.read_exact(&mut buf).await.map_err(GgufError::Io)?;
    Ok(buf[0])
}

async fn read_i8<R: AsyncRead + Unpin>(reader: &mut R) -> Result<i8, GgufError> {
    Ok(read_u8(reader).await? as i8)
}

async fn read_u16<R: AsyncRead + Unpin>(reader: &mut R) -> Result<u16, GgufError> {
    let mut buf = [0_u8; 2];
    reader.read_exact(&mut buf).await.map_err(GgufError::Io)?;
    Ok(u16::from_le_bytes(buf))
}

async fn read_i16<R: AsyncRead + Unpin>(reader: &mut R) -> Result<i16, GgufError> {
    let mut buf = [0_u8; 2];
    reader.read_exact(&mut buf).await.map_err(GgufError::Io)?;
    Ok(i16::from_le_bytes(buf))
}

async fn read_u32<R: AsyncRead + Unpin>(reader: &mut R) -> Result<u32, GgufError> {
    let mut buf = [0_u8; 4];
    reader.read_exact(&mut buf).await.map_err(GgufError::Io)?;
    Ok(u32::from_le_bytes(buf))
}

async fn read_i32<R: AsyncRead + Unpin>(reader: &mut R) -> Result<i32, GgufError> {
    let mut buf = [0_u8; 4];
    reader.read_exact(&mut buf).await.map_err(GgufError::Io)?;
    Ok(i32::from_le_bytes(buf))
}

async fn read_u64<R: AsyncRead + Unpin>(reader: &mut R) -> Result<u64, GgufError> {
    let mut buf = [0_u8; 8];
    reader.read_exact(&mut buf).await.map_err(GgufError::Io)?;
    Ok(u64::from_le_bytes(buf))
}

async fn read_i64<R: AsyncRead + Unpin>(reader: &mut R) -> Result<i64, GgufError> {
    let mut buf = [0_u8; 8];
    reader.read_exact(&mut buf).await.map_err(GgufError::Io)?;
    Ok(i64::from_le_bytes(buf))
}

async fn read_f32<R: AsyncRead + Unpin>(reader: &mut R) -> Result<f32, GgufError> {
    let mut buf = [0_u8; 4];
    reader.read_exact(&mut buf).await.map_err(GgufError::Io)?;
    Ok(f32::from_le_bytes(buf))
}

async fn read_f64<R: AsyncRead + Unpin>(reader: &mut R) -> Result<f64, GgufError> {
    let mut buf = [0_u8; 8];
    reader.read_exact(&mut buf).await.map_err(GgufError::Io)?;
    Ok(f64::from_le_bytes(buf))
}

fn quant_from_filename(filename: &str) -> Option<String> {
    let upper = filename.to_uppercase();
    [
        "Q2_K", "Q3_K_S", "Q3_K_M", "Q3_K_L", "Q4_K_S", "Q4_K_M", "Q4_K_L", "Q5_K_S", "Q5_K_M",
        "Q5_K_L", "Q6_K", "Q8_0", "F16", "F32",
    ]
    .iter()
    .find(|quant| upper.contains(**quant))
    .map(|quant| (*quant).to_string())
}

fn quant_from_tensor_types(types: &BTreeSet<String>) -> Option<String> {
    if types.is_empty() {
        None
    } else if types.len() == 1 {
        types.iter().next().cloned()
    } else {
        Some(types.iter().cloned().collect::<Vec<_>>().join("+"))
    }
}

fn tensor_type_to_quant(tensor_type: u32) -> Option<&'static str> {
    match tensor_type {
        0 => Some("F32"),
        1 => Some("F16"),
        2 => Some("Q4_0"),
        3 => Some("Q4_1"),
        6 => Some("Q5_0"),
        7 => Some("Q5_1"),
        8 => Some("Q8_0"),
        10 => Some("Q2_K"),
        11 => Some("Q3_K"),
        12 => Some("Q4_K"),
        13 => Some("Q5_K"),
        14 => Some("Q6_K"),
        15 => Some("Q8_K"),
        _ => None,
    }
}

fn model_name_from_filename(filename: &str) -> String {
    let name = filename.strip_suffix(".gguf").unwrap_or(filename);
    if let Some(quant) = quant_from_filename(name) {
        name.trim_end_matches(&format!("-{quant}"))
            .trim_end_matches(&format!(".{quant}"))
            .to_string()
    } else {
        name.to_string()
    }
}

fn filename_from_url(url: &str) -> Result<String, DownloadError> {
    let without_query = url.split('?').next().unwrap_or(url);
    let filename = without_query
        .rsplit('/')
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| DownloadError::InvalidFilename(url.to_string()))?;
    Ok(filename.to_string())
}

fn range_header(resumed_from: u64) -> Option<String> {
    (resumed_from > 0).then(|| format!("bytes={resumed_from}-"))
}

fn hf_resolve_url(repo_id: &str, filename: &str) -> String {
    let mut url = reqwest::Url::parse("https://huggingface.co/").expect("static HF base URL");
    {
        let mut segments = url.path_segments_mut().expect("static HF base URL path");
        for segment in repo_id.split('/') {
            segments.push(segment);
        }
        segments.push("resolve");
        segments.push("main");
        for segment in filename.split('/') {
            segments.push(segment);
        }
    }
    url.to_string()
}

fn bytes_to_mib(bytes: u64) -> u64 {
    bytes.div_ceil(1024 * 1024)
}

/// Format bytes as a compact human-readable size.
pub fn format_size(bytes: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    if bytes as f64 >= GIB {
        format!("{:.1} GB", bytes as f64 / GIB)
    } else {
        format!("{:.1} MB", bytes as f64 / MIB)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[tokio::test]
    async fn parses_gguf_metadata_and_tensor_quant() {
        let bytes = fixture_gguf();
        let mut reader = std::io::Cursor::new(bytes);
        let metadata = parse_gguf_reader(&mut reader).await.expect("parse gguf");
        assert_eq!(
            metadata,
            GgufMetadata {
                version: 3,
                tensor_count: 1,
                name: Some("Mistral Test".to_string()),
                architecture: Some("llama".to_string()),
                context_length: Some(4096),
                embedding_length: Some(4096),
                block_count: Some(32),
                quant: Some("Q4_K".to_string()),
            }
        );
    }

    #[tokio::test]
    async fn scans_registry_and_marks_aliases() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let models = tempdir.path().join("models");
        let alias = tempdir.path().join("external");
        std::fs::create_dir_all(&models).expect("models dir");
        std::fs::create_dir_all(&alias).expect("alias dir");
        write_file(&models.join("mistral-7b-Q4_K_M.gguf"), &fixture_gguf());
        write_file(&alias.join("llama-7b-Q8_0.gguf"), &fixture_gguf());

        let registry = ModelRegistry {
            models_dir: models,
            aliases: vec![alias],
        };
        let entries = registry.scan().await;
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|entry| entry.aliased));
        assert!(entries
            .iter()
            .all(|entry| entry.context_length == Some(4096)));
    }

    #[test]
    fn vram_fit_covers_full_partial_cpu() {
        let model = ModelEntry {
            path: PathBuf::from("model.gguf"),
            name: "model".to_string(),
            size_bytes: 4 * 1024 * 1024 * 1024,
            quant: "Q4_K_M".to_string(),
            context_length: None,
            architecture: None,
            aliased: false,
        };
        let gpu = GpuInfo {
            name: "GPU".to_string(),
            memory_total_mb: 8192,
            compute_cap: "8.6".to_string(),
            arch: Some("sm_86"),
        };
        assert!(matches!(model.vram_fit(&[gpu]), VramFit::Full { .. }));

        let small_gpu = GpuInfo {
            name: "GPU".to_string(),
            memory_total_mb: 2048,
            compute_cap: "8.6".to_string(),
            arch: Some("sm_86"),
        };
        assert!(matches!(
            model.vram_fit(&[small_gpu]),
            VramFit::Partial { .. }
        ));
        assert_eq!(model.vram_fit(&[]), VramFit::CpuOnly);
    }

    #[test]
    fn add_alias_rejects_missing_path_and_dedups() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let alias = tempdir.path().join("alias");
        std::fs::create_dir_all(&alias).expect("alias dir");
        let mut registry = ModelRegistry {
            models_dir: tempdir.path().join("models"),
            aliases: Vec::new(),
        };
        registry.add_alias(alias.clone()).expect("add alias");
        registry.add_alias(alias).expect("dedup alias");
        assert_eq!(registry.aliases.len(), 1);
        assert!(matches!(
            registry.add_alias(tempdir.path().join("missing")),
            Err(RegistryError::MissingPath { .. })
        ));
    }

    #[tokio::test]
    async fn parse_model_falls_back_to_filename_quant() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let path = tempdir.path().join("phi-3-Q8_0.gguf");
        std::fs::write(&path, b"GGUFbad").expect("write invalid file");
        let entry = parse_model_file(&path, false).await.expect("entry");
        assert_eq!(entry.quant, "Q8_0");
        assert_eq!(entry.name, "phi-3");
    }

    #[test]
    fn parses_hf_search_response_with_filters() {
        let body = r#"
        [
          {
            "id": "org/Mistral-GGUF",
            "downloads": 42,
            "tags": ["mistral", "gguf"],
            "siblings": [
              {"rfilename": "nested/mistral 7b Q4_K_M.gguf", "size": 1234},
              {"rfilename": "mistral-7b-Q8_0.gguf", "size": 9999},
              {"rfilename": "README.md", "size": 12}
            ]
          }
        ]
        "#;
        let query = HfSearchQuery {
            keywords: "mistral".to_string(),
            architecture: Some("mistral".to_string()),
            quant_filter: Some(QuantTier::Q4),
            max_results: 10,
        };
        let results = parse_hf_search_response(&query, body).expect("parse hf");
        assert_eq!(
            results,
            vec![HfModelResult {
                repo_id: "org/Mistral-GGUF".to_string(),
                filename: "nested/mistral 7b Q4_K_M.gguf".to_string(),
                size_bytes: 1234,
                downloads: 42,
                url:
                    "https://huggingface.co/org/Mistral-GGUF/resolve/main/nested/mistral%207b%20Q4_K_M.gguf"
                        .to_string(),
            }]
        );
    }

    #[test]
    fn resume_download_uses_range_header_and_url_filename() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let models_dir = tempdir.path().join("models");
        std::fs::create_dir_all(&models_dir).expect("models dir");
        let part_path = models_dir.join("model-Q4_K_M.gguf.part");
        let fixture = fixture_gguf();
        std::fs::write(&part_path, &fixture[..12]).expect("partial");

        let resumed_from = std::fs::metadata(&part_path).expect("metadata").len();
        assert_eq!(resumed_from, 12);
        assert_eq!(range_header(resumed_from).as_deref(), Some("bytes=12-"));
        assert_eq!(range_header(0), None);
        assert_eq!(
            filename_from_url(
                "https://huggingface.co/org/repo/resolve/main/model-Q4_K_M.gguf?download=1"
            )
            .expect("filename"),
            "model-Q4_K_M.gguf"
        );
    }

    fn fixture_gguf() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"GGUF");
        bytes.extend_from_slice(&3_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u64.to_le_bytes());
        bytes.extend_from_slice(&5_u64.to_le_bytes());
        write_kv_string(&mut bytes, "general.name", "Mistral Test");
        write_kv_string(&mut bytes, "general.architecture", "llama");
        write_kv_u32(&mut bytes, "llama.context_length", 4096);
        write_kv_u32(&mut bytes, "llama.embedding_length", 4096);
        write_kv_u32(&mut bytes, "llama.block_count", 32);
        write_string(&mut bytes, "blk.0.attn_q.weight");
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        bytes.extend_from_slice(&4096_u64.to_le_bytes());
        bytes.extend_from_slice(&4096_u64.to_le_bytes());
        bytes.extend_from_slice(&12_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u64.to_le_bytes());
        bytes
    }

    fn write_file(path: &Path, bytes: &[u8]) {
        let mut file = std::fs::File::create(path).expect("create file");
        file.write_all(bytes).expect("write file");
    }

    fn write_kv_string(bytes: &mut Vec<u8>, key: &str, value: &str) {
        write_string(bytes, key);
        bytes.extend_from_slice(&8_u32.to_le_bytes());
        write_string(bytes, value);
    }

    fn write_kv_u32(bytes: &mut Vec<u8>, key: &str, value: u32) {
        write_string(bytes, key);
        bytes.extend_from_slice(&4_u32.to_le_bytes());
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn write_string(bytes: &mut Vec<u8>, value: &str) {
        bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }
}
