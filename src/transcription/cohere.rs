// Speech to Text - Cohere Transcribe CLI Sidecar
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! CLI sidecar adapter for Cohere Transcribe.
//!
//! Downloads the pre-built `transcribe` binary (~123 MB, self-contained with
//! libtorch + vocab.json) from GitHub releases, downloads the 4.1 GB model
//! weights from HuggingFace (gated, requires token), and shells out to the
//! CLI for transcription. No server, no manual steps.

use super::engine::{TranscriptionResult, TranscriptionSegment};
use crate::config::AppConfig;
use crate::error::{AppError, AppResult};
use futures::StreamExt;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

const RUNTIME_URL: &str =
    "https://github.com/second-state/cohere_transcribe_rs/releases/download/v0.1.1/transcribe-linux-x86_64.zip";
const MODEL_URL: &str =
    "https://huggingface.co/CohereLabs/cohere-transcribe-03-2026/resolve/main/model.safetensors";
const CONFIG_URL: &str =
    "https://huggingface.co/CohereLabs/cohere-transcribe-03-2026/resolve/main/config.json";
const TOKENIZER_CONFIG_URL: &str =
    "https://huggingface.co/CohereLabs/cohere-transcribe-03-2026/resolve/main/tokenizer_config.json";

/// Directory for the Cohere runtime binary and libraries.
pub fn cohere_runtime_dir() -> PathBuf {
    AppConfig::data_dir().join("cohere-runtime")
}

/// Directory for Cohere model weights.
pub fn cohere_model_dir() -> PathBuf {
    AppConfig::data_dir().join("cohere-model")
}

/// Returns `true` if the `transcribe` binary is present.
pub fn is_runtime_installed() -> bool {
    cohere_runtime_dir().join("transcribe").is_file()
}

/// Returns `true` if all required model files are present.
pub fn is_model_downloaded() -> bool {
    let dir = cohere_model_dir();
    dir.join("model.safetensors").is_file()
        && dir.join("config.json").is_file()
        && dir.join("vocab.json").is_file()
        && dir.join("tokenizer_config.json").is_file()
}

/// Returns `true` if Cohere is fully ready to transcribe.
pub fn cohere_ready() -> bool {
    is_runtime_installed() && is_model_downloaded()
}

/// Delete the downloaded Cohere runtime (binary + libraries) from disk.
pub fn delete_runtime() -> AppResult<()> {
    let dir = cohere_runtime_dir();
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    Ok(())
}

/// Delete the downloaded Cohere model weights from disk.
pub fn delete_model() -> AppResult<()> {
    let dir = cohere_model_dir();
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    Ok(())
}

/// Download and extract the Cohere runtime (~123 MB).
#[tracing::instrument(name = "download.cohere_runtime", skip_all)]
pub async fn download_runtime(progress: impl Fn(u64, u64)) -> AppResult<()> {
    let dir = cohere_runtime_dir();
    std::fs::create_dir_all(&dir)?;

    let zip_path = dir.join("runtime.zip");

    let client = super::download_client();
    let resp =
        client.get(RUNTIME_URL).send().await.map_err(|e| {
            AppError::ModelDownloadFailed(format!("Runtime download failed: {}", e))
        })?;

    if !resp.status().is_success() {
        return Err(AppError::ModelDownloadFailed(format!(
            "Runtime download returned HTTP {}",
            resp.status()
        )));
    }

    let total = resp.content_length().unwrap_or(0);
    let mut downloaded: u64 = 0;
    let mut file = tokio::fs::File::create(&zip_path).await?;
    let mut stream = resp.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|e| AppError::ModelDownloadFailed(format!("Download error: {}", e)))?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        if downloaded > crate::limits::MAX_DOWNLOAD_BYTES {
            drop(file);
            let _ = std::fs::remove_file(&zip_path);
            return Err(AppError::ModelDownloadFailed(
                "Runtime download exceeded the maximum allowed size.".into(),
            ));
        }
        progress(downloaded, total);
    }
    file.flush().await?;
    drop(file);

    // Verify the runtime archive against GitHub's published digest before we
    // extract and execute anything from it. Fail closed on mismatch.
    let published_sha = super::verify::github_asset_sha256(
        &client,
        "second-state",
        "cohere_transcribe_rs",
        "v0.1.1",
        "transcribe-linux-x86_64.zip",
    )
    .await;
    super::verify::verify_pinned_file(
        &zip_path,
        "cohere-runtime-v0.1.1-linux-x86_64",
        published_sha.as_deref(),
    )?;

    // Extraction is CPU/IO-bound and uses blocking std::fs — run it off the
    // async runtime so it can't stall other tasks.
    {
        let zip_c = zip_path.clone();
        let dir_c = dir.clone();
        tokio::task::spawn_blocking(move || extract_zip(&zip_c, &dir_c))
            .await
            .map_err(|e| AppError::ModelDownloadFailed(format!("extraction task failed: {e}")))??;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let binary = dir.join("transcribe");
        if binary.exists() {
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755))?;
        }
    }

    let _ = std::fs::remove_file(&zip_path);

    if !is_runtime_installed() {
        return Err(AppError::ModelDownloadFailed(
            "Runtime extracted but 'transcribe' binary not found. \
             The archive layout may have changed."
                .into(),
        ));
    }

    Ok(())
}

/// Download Cohere model weights from HuggingFace (~4.1 GB, requires token).
#[tracing::instrument(name = "download.cohere_model", skip_all)]
pub async fn download_model(hf_token: &str, progress: impl Fn(u64, u64)) -> AppResult<()> {
    let dir = cohere_model_dir();
    std::fs::create_dir_all(&dir)?;

    let client = super::download_client();

    // The gated model API exposes the LFS SHA-256 when called with the user's
    // token. Refuse the download if that trust metadata is unavailable.
    let model_sha = super::verify::hf_lfs_sha256_with_token(
        &client,
        "CohereLabs/cohere-transcribe-03-2026",
        "model.safetensors",
        Some(hf_token),
    )
    .await
    .ok_or_else(|| {
        AppError::ModelDownloadFailed(
            "Could not obtain the trusted SHA-256 for the Cohere model; download refused.".into(),
        )
    })?;

    // model.safetensors (~4.1 GB) — reports progress, verifies existing files.
    let safetensors = dir.join("model.safetensors");
    let model_valid = safetensors.is_file()
        && super::verify::sha256_file(&safetensors)
            .map(|actual| actual.eq_ignore_ascii_case(&model_sha))
            .unwrap_or(false);
    if !model_valid {
        let _ = std::fs::remove_file(&safetensors);
        download_hf_file(
            &client,
            MODEL_URL,
            hf_token,
            &safetensors,
            Some(&model_sha),
            &progress,
        )
        .await?;
    }

    // config.json (~4 KB) — negligible
    download_hf_file(
        &client,
        CONFIG_URL,
        hf_token,
        &dir.join("config.json"),
        None,
        &|_, _| {},
    )
    .await?;

    // tokenizer_config.json (~48 KB) — required by CLI
    download_hf_file(
        &client,
        TOKENIZER_CONFIG_URL,
        hf_token,
        &dir.join("tokenizer_config.json"),
        None,
        &|_, _| {},
    )
    .await?;

    // Copy vocab.json from runtime dir → model dir
    let vocab_dst = dir.join("vocab.json");
    if !vocab_dst.exists() {
        if let Some(vocab_src) = find_file_recursive(&cohere_runtime_dir(), "vocab.json") {
            std::fs::copy(&vocab_src, &vocab_dst)?;
        }
    }

    if !is_model_downloaded() {
        return Err(AppError::ModelDownloadFailed(
            "Model files incomplete. Make sure the runtime is installed first \
             (vocab.json is needed from the runtime)."
                .into(),
        ));
    }

    Ok(())
}

/// Transcribe audio via the Cohere CLI binary.
///
/// `audio` is mono 16 kHz f32 PCM. This is a **blocking** call.
pub fn transcribe_via_cli(audio: &[f32], language: Option<&str>) -> AppResult<TranscriptionResult> {
    if audio.is_empty() {
        return Ok(TranscriptionResult {
            segments: Vec::new(),
            text: String::new(),
            average_confidence: None,
            detected_language: None,
        });
    }

    if !cohere_ready() {
        return Err(AppError::Transcription(
            "Cohere Transcribe is not set up. Download the runtime and model in Settings → Model."
                .into(),
        ));
    }

    let wav_data = super::encode_wav_16bit(audio, 16000);
    // Exclusive, random-named, 0600 temp file (RAII cleanup); kept alive until
    // the sidecar exits.
    let mut tmp = tempfile::Builder::new()
        .prefix("stt-cohere-")
        .suffix(".wav")
        .tempfile()
        .map_err(|e| AppError::Transcription(format!("Failed to create temp file: {e}")))?;
    {
        use std::io::Write;
        tmp.write_all(&wav_data)
            .and_then(|_| tmp.flush())
            .map_err(|e| AppError::Transcription(format!("Failed to write temp audio: {e}")))?;
    }
    let temp_wav = tmp.path().to_path_buf();

    let binary = cohere_runtime_dir().join("transcribe");
    let model_dir = cohere_model_dir();

    let mut cmd = std::process::Command::new(&binary);
    cmd.arg("--model-dir").arg(&model_dir);
    if let Some(lang) = language {
        cmd.arg("--language").arg(lang);
    }
    cmd.arg(&temp_wav);

    // Add runtime dir to LD_LIBRARY_PATH for libtorch shared libraries
    let rt_dir = cohere_runtime_dir();
    let ld_path = match std::env::var("LD_LIBRARY_PATH") {
        Ok(existing) => format!("{}:{}", rt_dir.display(), existing),
        Err(_) => rt_dir.display().to_string(),
    };
    cmd.env("LD_LIBRARY_PATH", &ld_path);

    let output =
        super::run_command_with_timeout(&mut cmd, std::time::Duration::from_secs(10 * 60))?;

    drop(tmp); // RAII removes the temp WAV

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Transcription(format!(
            "Cohere transcription failed: {}",
            stderr.trim()
        )));
    }

    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let lang_out = language.map(|s| s.to_string());

    Ok(TranscriptionResult {
        segments: vec![TranscriptionSegment {
            start_ms: None,
            end_ms: None,
            text: text.clone(),
            confidence: None,
        }],
        text,
        average_confidence: None,
        detected_language: lang_out,
    })
}

// ── Private helpers ──────────────────────────────────────────────────────────

async fn download_hf_file(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    dest: &Path,
    expected_sha256: Option<&str>,
    progress: &impl Fn(u64, u64),
) -> AppResult<()> {
    let partial = PathBuf::from(format!("{}.partial", dest.display()));

    let resp = client
        .get(url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| AppError::ModelDownloadFailed(format!("Download failed: {}", e)))?;

    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(AppError::ModelDownloadFailed(
            "HuggingFace token is invalid or you haven't accepted the model license. \
             Visit https://huggingface.co/CohereLabs/cohere-transcribe-03-2026 and click \
             \"Agree and access repository\", then try again."
                .into(),
        ));
    }
    if !status.is_success() {
        return Err(AppError::ModelDownloadFailed(format!(
            "HuggingFace download returned HTTP {}",
            status
        )));
    }

    let total = resp.content_length().unwrap_or(0);
    let mut downloaded: u64 = 0;
    let mut file = tokio::fs::File::create(&partial).await?;
    let mut stream = resp.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|e| AppError::ModelDownloadFailed(format!("Download error: {}", e)))?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        if downloaded > crate::limits::MAX_DOWNLOAD_BYTES {
            drop(file);
            let _ = tokio::fs::remove_file(&partial).await;
            return Err(AppError::ModelDownloadFailed(
                "Download exceeded the maximum allowed size.".into(),
            ));
        }
        progress(downloaded, total);
    }
    file.flush().await?;
    drop(file);

    // Validate JSON files — HuggingFace can return 200 OK with an HTML error
    // body for gated models when auth fails silently.
    if dest.extension().and_then(|e| e.to_str()) == Some("json") {
        let bytes = tokio::fs::read(&partial).await?;
        if serde_json::from_slice::<serde_json::Value>(&bytes).is_err() {
            let _ = tokio::fs::remove_file(&partial).await;
            let snippet = String::from_utf8_lossy(&bytes[..bytes.len().min(200)]);
            return Err(AppError::ModelDownloadFailed(format!(
                "Downloaded file is not valid JSON (possible auth error). \
                 Content starts with: {}",
                snippet
            )));
        }
    }

    if let Some(expected) = expected_sha256 {
        super::verify::verify_file(&partial, expected)?;
    }

    tokio::fs::rename(&partial, &dest).await?;

    Ok(())
}

fn extract_zip(zip_path: &Path, dest: &Path) -> AppResult<()> {
    // Hardened extraction (path-traversal/zip-bomb/symlink safe).
    super::archive::safe_extract_zip(zip_path, dest)?;

    // If the binary ended up in a subdirectory, flatten it
    if !dest.join("transcribe").exists() {
        if let Ok(entries) = std::fs::read_dir(dest) {
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|t| t.is_dir())
                    && entry.path().join("transcribe").exists()
                {
                    for sub in std::fs::read_dir(entry.path())?.flatten() {
                        let target = dest.join(sub.file_name());
                        std::fs::rename(sub.path(), &target)?;
                    }
                    let _ = std::fs::remove_dir(entry.path());
                    break;
                }
            }
        }
    }

    Ok(())
}

fn find_file_recursive(dir: &Path, name: &str) -> Option<PathBuf> {
    find_file_recursive_inner(dir, name, 0)
}

fn find_file_recursive_inner(dir: &Path, name: &str, depth: usize) -> Option<PathBuf> {
    if depth > 16 {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let file_type = entry.file_type().ok()?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_file() && path.file_name().map(|n| n == name).unwrap_or(false) {
            return Some(path);
        }
        if file_type.is_dir() {
            if let Some(found) = find_file_recursive_inner(&path, name, depth + 1) {
                return Some(found);
            }
        }
    }
    None
}
