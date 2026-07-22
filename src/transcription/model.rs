// Speech to Text - Model Management
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Whisper model catalog, download, and path management.

use std::collections::HashMap;
use std::path::PathBuf;
use tracing::info;

use crate::config::AppConfig;
use crate::error::{AppError, AppResult};

/// Information about a Whisper model.
#[derive(Debug, Clone)]
pub struct ModelInfo {
    /// Model display name (e.g., "Tiny", "Base").
    pub display_name: String,
    /// Model identifier (e.g., "tiny", "base").
    pub id: String,
    /// Download URL.
    pub url: String,
    /// Expected file size in bytes.
    pub size_bytes: u64,
    /// Human-readable file size.
    pub size_display: String,
    /// Brief description of quality/speed tradeoff.
    pub description: String,
    /// Whether this is a quantized model variant.
    pub quantized: bool,
}

/// Download/availability status of a model.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelStatus {
    /// Model is available on disk.
    Downloaded,
    /// Model is currently being downloaded (progress 0.0 - 1.0).
    Downloading(f64),
    /// Model is not yet downloaded.
    NotDownloaded,
    /// Download or validation error.
    Error(String),
}

/// Manages the catalog of available Whisper models.
pub struct ModelCatalog {
    models: HashMap<String, ModelInfo>,
    order: Vec<String>,
}

impl ModelCatalog {
    pub fn new() -> Self {
        let mut catalog = Self {
            models: HashMap::new(),
            order: Vec::new(),
        };

        // HuggingFace whisper.cpp ggml models
        let base_url = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

        // Full (f16) models
        catalog.add_model(ModelInfo {
            display_name: "Tiny".into(),
            id: "tiny".into(),
            url: format!("{}/ggml-tiny.bin", base_url),
            size_bytes: 75_000_000,
            size_display: "~75 MB".into(),
            description: "Fastest, lowest accuracy. Good for testing.".into(),
            quantized: false,
        });

        catalog.add_model(ModelInfo {
            display_name: "Base".into(),
            id: "base".into(),
            url: format!("{}/ggml-base.bin", base_url),
            size_bytes: 142_000_000,
            size_display: "~142 MB".into(),
            description: "Good balance of speed and accuracy. Recommended.".into(),
            quantized: false,
        });

        catalog.add_model(ModelInfo {
            display_name: "Small".into(),
            id: "small".into(),
            url: format!("{}/ggml-small.bin", base_url),
            size_bytes: 466_000_000,
            size_display: "~466 MB".into(),
            description: "Better accuracy, moderate speed.".into(),
            quantized: false,
        });

        catalog.add_model(ModelInfo {
            display_name: "Medium".into(),
            id: "medium".into(),
            url: format!("{}/ggml-medium.bin", base_url),
            size_bytes: 1_500_000_000,
            size_display: "~1.5 GB".into(),
            description: "High accuracy, slower transcription.".into(),
            quantized: false,
        });

        catalog.add_model(ModelInfo {
            display_name: "Large v3".into(),
            id: "large-v3".into(),
            url: format!("{}/ggml-large-v3.bin", base_url),
            size_bytes: 3_000_000_000,
            size_display: "~3 GB".into(),
            description: "Best accuracy. Requires significant resources.".into(),
            quantized: false,
        });

        catalog.add_model(ModelInfo {
            display_name: "Large v3 Turbo".into(),
            id: "large-v3-turbo".into(),
            url: format!("{}/ggml-large-v3-turbo.bin", base_url),
            size_bytes: 1_624_000_000,
            size_display: "~1.6 GB".into(),
            description: "Near Large v3 accuracy, several times faster. Great quality/speed pick.".into(),
            quantized: false,
        });

        // Quantized (q5) models — significantly smaller with minimal quality loss
        catalog.add_model(ModelInfo {
            display_name: "Tiny Q5".into(),
            id: "tiny-q5_1".into(),
            url: format!("{}/ggml-tiny-q5_1.bin", base_url),
            size_bytes: 32_000_000,
            size_display: "~32 MB".into(),
            description: "Quantized tiny. ~57% smaller, near-identical accuracy.".into(),
            quantized: true,
        });

        catalog.add_model(ModelInfo {
            display_name: "Base Q5".into(),
            id: "base-q5_1".into(),
            url: format!("{}/ggml-base-q5_1.bin", base_url),
            size_bytes: 58_000_000,
            size_display: "~58 MB".into(),
            description: "Quantized base. ~59% smaller, near-identical accuracy.".into(),
            quantized: true,
        });

        catalog.add_model(ModelInfo {
            display_name: "Small Q5".into(),
            id: "small-q5_1".into(),
            url: format!("{}/ggml-small-q5_1.bin", base_url),
            size_bytes: 182_000_000,
            size_display: "~182 MB".into(),
            description: "Quantized small. ~61% smaller, near-identical accuracy.".into(),
            quantized: true,
        });

        catalog.add_model(ModelInfo {
            display_name: "Medium Q5".into(),
            id: "medium-q5_0".into(),
            url: format!("{}/ggml-medium-q5_0.bin", base_url),
            size_bytes: 515_000_000,
            size_display: "~515 MB".into(),
            description: "Quantized medium. ~66% smaller, near-identical accuracy.".into(),
            quantized: true,
        });

        catalog.add_model(ModelInfo {
            display_name: "Large v3 Q5".into(),
            id: "large-v3-q5_0".into(),
            url: format!("{}/ggml-large-v3-q5_0.bin", base_url),
            size_bytes: 1_080_000_000,
            size_display: "~1.1 GB".into(),
            description: "Quantized large. ~64% smaller, near-identical accuracy.".into(),
            quantized: true,
        });

        catalog.add_model(ModelInfo {
            display_name: "Large v3 Turbo Q5".into(),
            id: "large-v3-turbo-q5_0".into(),
            url: format!("{}/ggml-large-v3-turbo-q5_0.bin", base_url),
            size_bytes: 574_000_000,
            size_display: "~574 MB".into(),
            description: "Quantized Turbo. ~65% smaller, near-identical accuracy. Fast and light.".into(),
            quantized: true,
        });

        catalog
    }

    fn add_model(&mut self, info: ModelInfo) {
        self.order.push(info.id.clone());
        self.models.insert(info.id.clone(), info);
    }

    /// Get all models in display order.
    pub fn models(&self) -> Vec<&ModelInfo> {
        self.order.iter()
            .filter_map(|id| self.models.get(id))
            .collect()
    }

    /// Get a specific model by ID.
    pub fn get(&self, model_id: &str) -> Option<&ModelInfo> {
        self.models.get(model_id)
    }

    /// Get the local file path for a model.
    pub fn model_path(model_id: &str) -> PathBuf {
        AppConfig::models_dir().join(format!("ggml-{}.bin", model_id))
    }

    /// Check if a model is downloaded and ready to use.
    pub fn is_downloaded(model_id: &str) -> bool {
        let path = Self::model_path(model_id);
        path.exists() && path.metadata().map(|m| m.len() > 0).unwrap_or(false)
    }

    /// Get the status of a model.
    pub fn status(model_id: &str) -> ModelStatus {
        if Self::is_downloaded(model_id) {
            ModelStatus::Downloaded
        } else {
            ModelStatus::NotDownloaded
        }
    }

    /// Delete a downloaded model.
    pub fn delete_model(model_id: &str) -> AppResult<()> {
        let path = Self::model_path(model_id);
        if path.exists() {
            std::fs::remove_file(&path)?;
            info!("Deleted model file: {:?}", path);
        }
        Ok(())
    }

    /// Get all downloaded model IDs.
    pub fn downloaded_models(&self) -> Vec<String> {
        self.order.iter()
            .filter(|id| Self::is_downloaded(id))
            .cloned()
            .collect()
    }

    /// Get only full (non-quantized) models in display order.
    pub fn full_models(&self) -> Vec<&ModelInfo> {
        self.order.iter()
            .filter_map(|id| self.models.get(id))
            .filter(|m| !m.quantized)
            .collect()
    }

    /// Get only quantized models in display order.
    pub fn quantized_models(&self) -> Vec<&ModelInfo> {
        self.order.iter()
            .filter_map(|id| self.models.get(id))
            .filter(|m| m.quantized)
            .collect()
    }

    /// Get the quantized variant ID for a base model.
    pub fn quantized_variant(base_id: &str) -> Option<String> {
        match base_id {
            "tiny" => Some("tiny-q5_1".into()),
            "base" => Some("base-q5_1".into()),
            "small" => Some("small-q5_1".into()),
            "medium" => Some("medium-q5_0".into()),
            "large-v3" => Some("large-v3-q5_0".into()),
            "large-v3-turbo" => Some("large-v3-turbo-q5_0".into()),
            _ => None,
        }
    }

    /// Get the base (full) model ID from any model ID (strips quantization suffix).
    pub fn base_model_id(model_id: &str) -> &str {
        // Quantized IDs contain "-q5_" or "-q8_" etc.
        if let Some((base, _)) = model_id.rsplit_once("-q") {
            // Verify we actually split on a quantization suffix (digit after 'q')
            base
        } else {
            model_id
        }
    }

    /// Compute the effective model ID given a base model and quantization preference.
    pub fn effective_model_id(base_id: &str, use_quantized: bool) -> String {
        if use_quantized {
            Self::quantized_variant(base_id).unwrap_or_else(|| base_id.to_string())
        } else {
            base_id.to_string()
        }
    }

    /// Resolve the best available model: tries preferred variant first, falls back to the other.
    pub fn resolve_model(base_id: &str, prefer_quantized: bool) -> String {
        let preferred = Self::effective_model_id(base_id, prefer_quantized);
        if Self::is_downloaded(&preferred) {
            return preferred;
        }
        // Fallback to opposite variant
        let fallback = Self::effective_model_id(base_id, !prefer_quantized);
        if Self::is_downloaded(&fallback) {
            return fallback;
        }
        // Neither downloaded, return preferred (load will show appropriate error)
        preferred
    }
}

/// Download a model asynchronously with progress reporting.
///
/// `progress_callback` receives (bytes_downloaded, total_bytes) on each chunk.
/// `cancel` aborts the download when set to `true`; the partial file is retained
/// and resumed on the next attempt.
/// The completed file is verified against the server's `Content-Length` to guard
/// against silent truncation or an HTML error page being saved as a model.
#[tracing::instrument(name = "download.whisper_model", skip_all, fields(model = %model_info.id))]
pub async fn download_model<F>(
    model_info: &ModelInfo,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    progress_callback: F,
) -> AppResult<PathBuf>
where
    F: Fn(u64, u64) + Send + 'static,
{
    use std::sync::atomic::Ordering;

    let models_dir = AppConfig::models_dir();
    std::fs::create_dir_all(&models_dir)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&models_dir, std::fs::Permissions::from_mode(0o700));
    }

    let output_path = ModelCatalog::model_path(&model_info.id);
    let temp_path = output_path.with_extension("bin.partial");

    let client = super::download_client();
    let filename = format!("ggml-{}.bin", model_info.id);
    let expected_sha = crate::transcription::verify::hf_lfs_sha256(
        &client,
        "ggerganov/whisper.cpp",
        &filename,
    )
    .await
    .ok_or_else(|| {
        AppError::ModelDownloadFailed(format!(
            "Could not obtain the trusted SHA-256 for '{filename}'; download refused."
        ))
    })?;

    let partial_matches = if temp_path.is_file() {
        let path = temp_path.clone();
        let expected = expected_sha.clone();
        tokio::task::spawn_blocking(move || {
            crate::transcription::verify::sha256_file(&path)
                .map(|actual| actual.eq_ignore_ascii_case(&expected))
                .unwrap_or(false)
        })
        .await
        .unwrap_or(false)
    } else {
        false
    };
    if partial_matches {
        tokio::fs::rename(&temp_path, &output_path).await.map_err(|e| {
            AppError::ModelDownloadFailed(format!("Failed to finalize resumed download: {e}"))
        })?;
        return Ok(output_path);
    }

    info!("Downloading model '{}' from {}", model_info.id, model_info.url);

    let existing = tokio::fs::metadata(&temp_path)
        .await
        .map(|metadata| metadata.len())
        .unwrap_or(0)
        .min(crate::limits::MAX_DOWNLOAD_BYTES);
    let mut request = client.get(&model_info.url);
    if existing > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={existing}-"));
    }
    let response = request.send().await
        .map_err(|e| AppError::ModelDownloadFailed(format!("HTTP request failed: {}", e)))?;

    if !response.status().is_success() {
        return Err(AppError::ModelDownloadFailed(
            format!("HTTP {}: {}", response.status(), model_info.url)
        ));
    }

    let resuming = existing > 0 && response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    let start = if resuming { existing } else { 0 };
    let content_len = response.content_length();
    let total_size = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.rsplit_once('/'))
        .and_then(|(_, total)| total.parse::<u64>().ok())
        .or_else(|| content_len.map(|remaining| start.saturating_add(remaining)))
        .unwrap_or(model_info.size_bytes);
    let mut downloaded = start;

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resuming)
        .truncate(!resuming)
        .open(&temp_path)
        .await
        .map_err(|e| AppError::ModelDownloadFailed(format!("Failed to create file: {}", e)))?;

    use futures::StreamExt;
    use tokio::io::AsyncWriteExt;

    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        if cancel.load(Ordering::Relaxed) {
            drop(file);
            return Err(AppError::ModelDownloadFailed("Download cancelled".into()));
        }

        let chunk = chunk
            .map_err(|e| AppError::ModelDownloadFailed(format!("Download interrupted: {}", e)))?;

        file.write_all(&chunk).await
            .map_err(|e| AppError::ModelDownloadFailed(format!("Failed to write: {}", e)))?;

        downloaded += chunk.len() as u64;
        if downloaded > crate::limits::MAX_DOWNLOAD_BYTES {
            drop(file);
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(AppError::ModelDownloadFailed(
                "Download exceeded the maximum allowed size.".into(),
            ));
        }
        progress_callback(downloaded, total_size);
    }

    file.flush().await
        .map_err(|e| AppError::ModelDownloadFailed(format!("Failed to flush: {}", e)))?;
    drop(file);

    // Integrity check before publishing the file.
    let valid = match content_len {
        Some(expected) => downloaded == start.saturating_add(expected),
        // No Content-Length: at least reject obviously-truncated downloads.
        None => downloaded >= model_info.size_bytes / 2,
    };
    if !valid {
        let _ = tokio::fs::remove_file(&temp_path).await;
        return Err(AppError::ModelDownloadFailed(format!(
            "Incomplete download for '{}': got {} bytes (expected ~{})",
            model_info.id, downloaded, total_size
        )));
    }

    // Verify before publishing the model to the final path.
    let verify_path = temp_path.clone();
    let verify_sha = expected_sha.clone();
    tokio::task::spawn_blocking(move || {
        crate::transcription::verify::verify_file(&verify_path, &verify_sha)
    })
    .await
    .map_err(|e| AppError::ModelDownloadFailed(format!("Integrity task failed: {e}")))??;

    // Rename temp file to final path atomically
    tokio::fs::rename(&temp_path, &output_path).await
        .map_err(|e| AppError::ModelDownloadFailed(format!("Failed to finalize: {}", e)))?;

    info!("Model '{}' downloaded successfully to {:?} ({} bytes)", model_info.id, output_path, downloaded);
    Ok(output_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_model_id_strips_quantization_suffix() {
        assert_eq!(ModelCatalog::base_model_id("tiny"), "tiny");
        assert_eq!(ModelCatalog::base_model_id("tiny-q5_1"), "tiny");
        assert_eq!(ModelCatalog::base_model_id("base-q5_1"), "base");
        assert_eq!(ModelCatalog::base_model_id("medium-q5_0"), "medium");
        assert_eq!(ModelCatalog::base_model_id("large-v3"), "large-v3");
        assert_eq!(ModelCatalog::base_model_id("large-v3-q5_0"), "large-v3");
        assert_eq!(ModelCatalog::base_model_id("large-v3-turbo"), "large-v3-turbo");
        assert_eq!(ModelCatalog::base_model_id("large-v3-turbo-q5_0"), "large-v3-turbo");
    }

    #[test]
    fn quantized_variant_maps_each_base_model() {
        assert_eq!(ModelCatalog::quantized_variant("tiny").as_deref(), Some("tiny-q5_1"));
        assert_eq!(ModelCatalog::quantized_variant("base").as_deref(), Some("base-q5_1"));
        assert_eq!(ModelCatalog::quantized_variant("small").as_deref(), Some("small-q5_1"));
        assert_eq!(ModelCatalog::quantized_variant("medium").as_deref(), Some("medium-q5_0"));
        assert_eq!(ModelCatalog::quantized_variant("large-v3").as_deref(), Some("large-v3-q5_0"));
        assert_eq!(ModelCatalog::quantized_variant("large-v3-turbo").as_deref(), Some("large-v3-turbo-q5_0"));
        assert_eq!(ModelCatalog::quantized_variant("nonexistent"), None);
    }

    #[test]
    fn effective_model_id_honors_quantization_preference() {
        assert_eq!(ModelCatalog::effective_model_id("base", false), "base");
        assert_eq!(ModelCatalog::effective_model_id("base", true), "base-q5_1");
        assert_eq!(ModelCatalog::effective_model_id("large-v3", true), "large-v3-q5_0");
        // A model with no quantized variant falls back to the base id.
        assert_eq!(ModelCatalog::effective_model_id("nonexistent", true), "nonexistent");
    }

    #[test]
    fn base_and_effective_round_trip() {
        // The B2 invariant: resolving a quantized id back to base and forward
        // again with use_quantized=true returns the same quantized id.
        for full in ["tiny-q5_1", "base-q5_1", "medium-q5_0", "large-v3-q5_0"] {
            let base = ModelCatalog::base_model_id(full);
            assert_eq!(ModelCatalog::effective_model_id(base, true), full);
        }
    }

    #[test]
    fn catalog_contains_expected_models() {
        let catalog = ModelCatalog::new();
        assert!(catalog.get("base").is_some());
        assert!(catalog.get("large-v3-q5_0").is_some());
        assert!(catalog.get("large-v3-turbo").is_some());
        assert!(catalog.get("large-v3-turbo-q5_0").is_some());
        // Full and quantized partition the catalog.
        assert_eq!(
            catalog.full_models().len() + catalog.quantized_models().len(),
            catalog.models().len()
        );
    }
}
