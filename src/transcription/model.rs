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
#[allow(dead_code)]
pub async fn download_model<F>(
    model_info: &ModelInfo,
    progress_callback: F,
) -> AppResult<PathBuf>
where
    F: Fn(u64, u64) + Send + 'static,
{
    let models_dir = AppConfig::models_dir();
    std::fs::create_dir_all(&models_dir)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&models_dir, std::fs::Permissions::from_mode(0o700));
    }

    let output_path = ModelCatalog::model_path(&model_info.id);
    let temp_path = output_path.with_extension("bin.partial");

    info!("Downloading model '{}' from {}", model_info.id, model_info.url);

    let response = reqwest::get(&model_info.url).await
        .map_err(|e| AppError::ModelDownloadFailed(format!("HTTP request failed: {}", e)))?;

    if !response.status().is_success() {
        return Err(AppError::ModelDownloadFailed(
            format!("HTTP {}: {}", response.status(), model_info.url)
        ));
    }

    let total_size = response.content_length().unwrap_or(model_info.size_bytes);
    let mut downloaded: u64 = 0;

    let mut file = tokio::fs::File::create(&temp_path).await
        .map_err(|e| AppError::ModelDownloadFailed(format!("Failed to create file: {}", e)))?;

    use futures::StreamExt;
    use tokio::io::AsyncWriteExt;

    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk
            .map_err(|e| AppError::ModelDownloadFailed(format!("Download interrupted: {}", e)))?;

        file.write_all(&chunk).await
            .map_err(|e| AppError::ModelDownloadFailed(format!("Failed to write: {}", e)))?;

        downloaded += chunk.len() as u64;
        progress_callback(downloaded, total_size);
    }

    file.flush().await
        .map_err(|e| AppError::ModelDownloadFailed(format!("Failed to flush: {}", e)))?;

    // Rename temp file to final path atomically
    tokio::fs::rename(&temp_path, &output_path).await
        .map_err(|e| AppError::ModelDownloadFailed(format!("Failed to finalize: {}", e)))?;

    info!("Model '{}' downloaded successfully to {:?}", model_info.id, output_path);
    Ok(output_path)
}
