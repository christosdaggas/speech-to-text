// Speech to Text - Cohere Transcribe CLI Sidecar
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! CLI sidecar adapter for Cohere Transcribe.
//!
//! Downloads the pre-built `transcribe` binary (~123 MB, self-contained with
//! libtorch + vocab.json) from GitHub releases, downloads the 4.1 GB model
//! weights from HuggingFace (gated, requires token), and shells out to the
//! CLI for transcription. No server, no manual steps.

use crate::config::AppConfig;
use crate::error::{AppError, AppResult};
use super::engine::{TranscriptionResult, TranscriptionSegment};
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

/// Download and extract the Cohere runtime (~123 MB).
pub async fn download_runtime(progress: impl Fn(u64, u64)) -> AppResult<()> {
    let dir = cohere_runtime_dir();
    std::fs::create_dir_all(&dir)?;

    let zip_path = dir.join("runtime.zip");

    let client = reqwest::Client::new();
    let resp = client
        .get(RUNTIME_URL)
        .send()
        .await
        .map_err(|e| AppError::ModelDownloadFailed(format!("Runtime download failed: {}", e)))?;

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
        let chunk = chunk
            .map_err(|e| AppError::ModelDownloadFailed(format!("Download error: {}", e)))?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        progress(downloaded, total);
    }
    file.flush().await?;
    drop(file);

    extract_zip(&zip_path, &dir)?;

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
pub async fn download_model(hf_token: &str, progress: impl Fn(u64, u64)) -> AppResult<()> {
    let dir = cohere_model_dir();
    std::fs::create_dir_all(&dir)?;

    let client = reqwest::Client::new();

    // model.safetensors (~4.1 GB) — reports progress, skip if already exists
    let safetensors = dir.join("model.safetensors");
    if !safetensors.is_file() {
        download_hf_file(
            &client,
            MODEL_URL,
            hf_token,
            &safetensors,
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
        &|_, _| {},
    )
    .await?;

    // tokenizer_config.json (~48 KB) — required by CLI
    download_hf_file(
        &client,
        TOKENIZER_CONFIG_URL,
        hf_token,
        &dir.join("tokenizer_config.json"),
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
pub fn transcribe_via_cli(
    audio: &[f32],
    language: Option<&str>,
) -> AppResult<TranscriptionResult> {
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

    let wav_data = encode_wav_16bit(audio, 16000);
    let temp_wav = std::env::temp_dir().join(format!("stt-cohere-{}.wav", std::process::id()));
    std::fs::write(&temp_wav, &wav_data)?;

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

    let output = cmd
        .output()
        .map_err(|e| AppError::Transcription(format!("Failed to run transcribe binary: {}", e)))?;

    let _ = std::fs::remove_file(&temp_wav);

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
        let chunk = chunk
            .map_err(|e| AppError::ModelDownloadFailed(format!("Download error: {}", e)))?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
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

    tokio::fs::rename(&partial, &dest).await?;

    Ok(())
}

fn extract_zip(zip_path: &Path, dest: &Path) -> AppResult<()> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| AppError::ModelDownloadFailed(format!("Failed to open zip: {}", e)))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| AppError::ModelDownloadFailed(format!("Zip read error: {}", e)))?;

        let name = entry.name().replace('\\', "/");
        if name.contains("..") || name.starts_with('/') {
            continue;
        }

        let out_path = dest.join(&name);
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out_file = std::fs::File::create(&out_path)?;
            std::io::copy(&mut entry, &mut out_file)?;
        }
    }

    // If the binary ended up in a subdirectory, flatten it
    if !dest.join("transcribe").exists() {
        if let Ok(entries) = std::fs::read_dir(dest) {
            for entry in entries.flatten() {
                if entry.file_type().map_or(false, |t| t.is_dir())
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
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.file_name().map(|n| n == name).unwrap_or(false) {
            return Some(path);
        }
        if path.is_dir() {
            if let Some(found) = find_file_recursive(&path, name) {
                return Some(found);
            }
        }
    }
    None
}

/// Encode f32 PCM samples as a 16-bit WAV file in memory.
fn encode_wav_16bit(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let num_samples = samples.len();
    let data_size = (num_samples * 2) as u32;
    let file_size = 36 + data_size;

    let mut buf = Vec::with_capacity(file_size as usize + 8);

    // RIFF header
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&file_size.to_le_bytes());
    buf.extend_from_slice(b"WAVE");

    // fmt chunk
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
    buf.extend_from_slice(&1u16.to_le_bytes()); // mono
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    buf.extend_from_slice(&2u16.to_le_bytes()); // block align
    buf.extend_from_slice(&16u16.to_le_bytes()); // bits per sample

    // data chunk
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_size.to_le_bytes());

    for &sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let val = (clamped * 32767.0) as i16;
        buf.extend_from_slice(&val.to_le_bytes());
    }

    buf
}
