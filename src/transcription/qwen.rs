// Speech to Text - Qwen3-ASR CLI Sidecar
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! CLI sidecar adapter for Qwen3-ASR (local, offline).
//!
//! Mirrors the Cohere backend: downloads the pre-built `asr` runtime (bundles
//! libtorch) from the `second-state/qwen3_asr_rs` GitHub releases, downloads the
//! **ungated** Qwen3-ASR-0.6B weights from HuggingFace (no token needed), and
//! shells out to the `asr` CLI for transcription. Qwen3-ASR auto-detects the
//! language when none is given.

use crate::config::AppConfig;
use crate::error::{AppError, AppResult};
use super::engine::{TranscriptionResult, TranscriptionSegment};
use futures::StreamExt;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

/// Pre-built CPU runtime (bundles the `asr`/`asr-server` binaries + libtorch).
const RUNTIME_URL: &str =
    "https://github.com/second-state/qwen3_asr_rs/releases/download/v0.2.0/asr-linux-x86_64.zip";

/// Available model sizes: 0.6B (small/fast) and 1.7B (full/most accurate).
pub const QWEN_SIZES: [&str; 2] = ["0.6B", "1.7B"];

/// The configured active model size (defaults to 0.6B).
pub fn active_size() -> String {
    let s = AppConfig::load().qwen_model_size;
    if QWEN_SIZES.contains(&s.as_str()) { s } else { "0.6B".to_string() }
}

/// HuggingFace repo id for a given size (ungated).
fn model_repo(size: &str) -> String {
    format!("Qwen/Qwen3-ASR-{}", size)
}

/// Directory for the Qwen runtime binary and bundled libtorch.
pub fn qwen_runtime_dir() -> PathBuf {
    AppConfig::data_dir().join("qwen-runtime")
}

/// Per-size model directory (so both sizes can coexist on disk).
pub fn model_dir_for(size: &str) -> PathBuf {
    AppConfig::data_dir().join(format!("qwen-model-{}", size))
}

/// Directory for the currently-selected model size.
pub fn qwen_model_dir() -> PathBuf {
    model_dir_for(&active_size())
}

/// Locate the `asr` binary anywhere under the runtime dir (the release zip may
/// nest it inside a top-level folder).
fn qwen_asr_binary() -> Option<PathBuf> {
    find_file_recursive(&qwen_runtime_dir(), "asr")
}

/// Returns `true` if the `asr` binary is present.
pub fn is_runtime_installed() -> bool {
    qwen_asr_binary().is_some()
}

/// Returns `true` if a given model size is fully downloaded (a sentinel file is
/// written only after every repo file has been fetched — robust for the sharded
/// 1.7B model).
pub fn is_model_downloaded_size(size: &str) -> bool {
    model_dir_for(size).join(".download_complete").is_file()
}

/// Returns `true` if the active model size is downloaded.
pub fn is_model_downloaded() -> bool {
    is_model_downloaded_size(&active_size())
}

/// Returns `true` if Qwen3-ASR is fully ready to transcribe with the active size.
pub fn qwen_ready() -> bool {
    is_runtime_installed() && is_model_downloaded()
}

/// Delete the downloaded Qwen runtime from disk.
pub fn delete_runtime() -> AppResult<()> {
    let dir = qwen_runtime_dir();
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    Ok(())
}

/// Delete a downloaded Qwen model size from disk.
pub fn delete_model(size: &str) -> AppResult<()> {
    let dir = model_dir_for(size);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    Ok(())
}

/// Download and extract the Qwen runtime (bundles libtorch).
#[tracing::instrument(name = "download.qwen_runtime", skip_all)]
pub async fn download_runtime(progress: impl Fn(u64, u64)) -> AppResult<()> {
    let dir = qwen_runtime_dir();
    std::fs::create_dir_all(&dir)?;

    let zip_path = dir.join("runtime.zip");

    let client = super::download_client();
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
        "qwen3_asr_rs",
        "v0.2.0",
        "asr-linux-x86_64.zip",
    )
    .await;
    super::verify::verify_pinned_file(
        &zip_path,
        "qwen-runtime-v0.2.0-linux-x86_64",
        published_sha.as_deref(),
    )?;

    // Run the blocking extraction off the async runtime.
    {
        let zip_c = zip_path.clone();
        let dir_c = dir.clone();
        tokio::task::spawn_blocking(move || super::archive::safe_extract_zip(&zip_c, &dir_c))
            .await
            .map_err(|e| AppError::ModelDownloadFailed(format!("extraction task failed: {e}")))??;
    }
    let _ = std::fs::remove_file(&zip_path);

    // Make the CLI binaries executable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for name in ["asr", "asr-server"] {
            if let Some(bin) = find_file_recursive(&dir, name) {
                let _ = std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755));
            }
        }
    }

    if !is_runtime_installed() {
        return Err(AppError::ModelDownloadFailed(
            "Runtime extracted but the 'asr' binary was not found. \
             The archive layout may have changed."
                .into(),
        ));
    }

    Ok(())
}

/// Download a Qwen3-ASR model size from HuggingFace (ungated, no token). Queries
/// the repo file list via the HF API, so it handles both the single-file 0.6B
/// model and the sharded 1.7B model. A `.download_complete` sentinel is written
/// only once every file has been fetched.
#[tracing::instrument(name = "download.qwen_model", skip_all, fields(size = %size))]
pub async fn download_model(size: &str, progress: impl Fn(u64, u64)) -> AppResult<()> {
    let dir = model_dir_for(size);
    std::fs::create_dir_all(&dir)?;
    let _ = std::fs::remove_file(dir.join(".download_complete"));

    let repo = model_repo(size);
    let client = super::download_client();

    // List the repo files via the HF tree API, which also exposes the per-file
    // LFS sha256 (`lfs.oid`) used to verify the (large) weight downloads.
    let api = format!("https://huggingface.co/api/models/{}/tree/main?recursive=true", repo);
    let tree: serde_json::Value = client
        .get(&api)
        .send()
        .await
        .map_err(|e| AppError::ModelDownloadFailed(format!("Model listing failed: {}", e)))?
        .error_for_status()
        .map_err(|e| AppError::ModelDownloadFailed(format!("Model listing failed: {}", e)))?
        .json()
        .await
        .map_err(|e| AppError::ModelDownloadFailed(format!("Model listing parse failed: {}", e)))?;

    let entries = tree
        .as_array()
        .ok_or_else(|| AppError::ModelDownloadFailed("Unexpected model listing format.".into()))?;

    // Plan the downloads: (relative path, expected sha256 if LFS). Validate
    // every remote filename for type and path-safety BEFORE any I/O.
    let mut planned: Vec<(String, Option<String>)> = Vec::new();
    for e in entries {
        if e["type"].as_str() != Some("file") {
            continue;
        }
        let Some(path) = e["path"].as_str() else { continue };
        if path == ".gitattributes" || path == "README.md" {
            continue;
        }
        if !is_allowed_model_file(path) {
            return Err(AppError::ModelDownloadFailed(format!(
                "Refusing unexpected model file from remote listing: {path}"
            )));
        }
        if super::safe_path::safe_join(&dir, path).is_none() {
            return Err(AppError::ModelDownloadFailed(format!(
                "Refusing unsafe model filename from remote listing: {path}"
            )));
        }
        let sha = e["lfs"]["oid"].as_str().and_then(super::verify::normalize_hf_oid);
        if matches!(Path::new(path).extension().and_then(|e| e.to_str()), Some("safetensors") | Some("bin"))
            && sha.is_none()
        {
            return Err(AppError::ModelDownloadFailed(format!(
                "No trusted SHA-256 was published for model weight file: {path}"
            )));
        }
        planned.push((path.to_string(), sha));
    }

    if planned.is_empty() {
        return Err(AppError::ModelDownloadFailed(
            "Model file listing was empty.".into(),
        ));
    }

    for (path, expected) in &planned {
        let dest = super::safe_path::safe_join(&dir, path).ok_or_else(|| {
            AppError::ModelDownloadFailed(format!("Refusing unsafe model filename: {path}"))
        })?;
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if dest.is_file() {
            // Resume: keep an existing weight only if it matches its hash.
            // Small non-LFS metadata files are validated by their allowlisted
            // names and parsed by the sidecar.
            match expected {
                Some(sha)
                    if super::verify::sha256_file(&dest)
                        .map(|h| h.eq_ignore_ascii_case(sha))
                        .unwrap_or(false) =>
                {
                    continue;
                }
                Some(_) => {
                    let _ = std::fs::remove_file(&dest);
                }
                None => continue,
            }
        }
        let url = format!("https://huggingface.co/{}/resolve/main/{}", repo, path);
        // Report progress for the (large) weight shards.
        if path.ends_with(".safetensors") {
            download_file(&client, &url, &dest, &progress).await?;
        } else {
            download_file(&client, &url, &dest, &|_, _| {}).await?;
        }
        // Verify integrity against the provider-declared sha256 (LFS files).
        if let Some(sha) = expected {
            super::verify::verify_file(&dest, sha)?;
        }
    }

    // The model repos omit tokenizer.json; copy the matching one bundled in the
    // runtime. Best-effort here (the runtime may not be installed yet) — the
    // transcribe path also heals this just-in-time.
    let _ = ensure_tokenizer(size);

    // Mark complete.
    std::fs::write(dir.join(".download_complete"), b"ok")?;
    Ok(())
}

/// Allow only known Qwen model artifact file types from the remote listing.
fn is_allowed_model_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [".json", ".txt", ".safetensors", ".model", ".bin"]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

/// Copy the pre-built `tokenizer-<size>.json` from the runtime bundle into the
/// model directory as `tokenizer.json` (the HuggingFace repos don't ship it).
fn ensure_tokenizer(size: &str) -> AppResult<()> {
    let dest = model_dir_for(size).join("tokenizer.json");
    if dest.is_file() {
        return Ok(());
    }
    let name = format!("tokenizer-{}.json", size);
    match find_file_recursive(&qwen_runtime_dir(), &name) {
        Some(src) => {
            std::fs::copy(&src, &dest)?;
            Ok(())
        }
        None => Err(AppError::Transcription(format!(
            "Bundled tokenizer '{}' not found — re-download the Qwen runtime.",
            name
        ))),
    }
}

/// Transcribe audio via the Qwen3-ASR CLI binary.
///
/// `audio` is mono 16 kHz f32 PCM. This is a **blocking** call. When `language`
/// is `None` (or unmapped), Qwen3-ASR auto-detects the language.
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

    if !qwen_ready() {
        return Err(AppError::Transcription(
            "Qwen3-ASR is not set up. Download the runtime and model in Settings → Model."
                .into(),
        ));
    }

    // Ensure the model dir has tokenizer.json (copied from the runtime bundle).
    ensure_tokenizer(&active_size())?;

    let binary = qwen_asr_binary().ok_or_else(|| {
        AppError::Transcription("Qwen 'asr' binary not found.".into())
    })?;
    let root = binary.parent().unwrap_or(&qwen_runtime_dir()).to_path_buf();

    let wav_data = super::encode_wav_16bit(audio, 16000);
    // Exclusive, random-named, 0600 temp file (RAII cleanup); kept alive until
    // the sidecar exits.
    let mut tmp = tempfile::Builder::new()
        .prefix("stt-qwen-")
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

    // CLI: `asr <model_dir> <wav> [language-name]`
    let mut cmd = std::process::Command::new(&binary);
    cmd.arg(qwen_model_dir());
    cmd.arg(&temp_wav);
    if let Some(name) = language.and_then(qwen_language_name) {
        cmd.arg(name);
    }

    // libtorch shared libraries live under the runtime dir (and its libtorch/lib).
    let mut ld_dirs = vec![
        root.join("libtorch").join("lib"),
        root.join("libtorch"),
        root.clone(),
    ];
    if let Ok(existing) = std::env::var("LD_LIBRARY_PATH") {
        ld_dirs.push(PathBuf::from(existing));
    }
    let ld_path = ld_dirs
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(":");
    cmd.env("LD_LIBRARY_PATH", &ld_path);

    let output = super::run_command_with_timeout(
        &mut cmd,
        std::time::Duration::from_secs(10 * 60),
    )?;

    drop(tmp); // RAII removes the temp WAV

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Transcription(format!(
            "Qwen3-ASR transcription failed: {}",
            stderr.trim()
        )));
    }

    // Output looks like: "Language: English / Text: hello world".
    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let text = match raw.find("Text:") {
        Some(idx) => raw[idx + "Text:".len()..].trim().to_string(),
        None => raw.clone(),
    };

    Ok(TranscriptionResult {
        segments: vec![TranscriptionSegment {
            start_ms: None,
            end_ms: None,
            text: text.clone(),
            confidence: None,
        }],
        text,
        average_confidence: None,
        // Detected-language display uses ISO codes elsewhere; the CLI emits a
        // language *name*, so we leave this None (status shows plain "Auto-detect").
        detected_language: None,
    })
}

/// Map an ISO 639-1 language code to the lowercase language name the Qwen CLI
/// expects. Unmapped codes return `None` ⇒ auto-detect.
fn qwen_language_name(code: &str) -> Option<&'static str> {
    Some(match code {
        "zh" => "chinese",
        "en" => "english",
        "ar" => "arabic",
        "de" => "german",
        "fr" => "french",
        "es" => "spanish",
        "pt" => "portuguese",
        "id" => "indonesian",
        "it" => "italian",
        "ko" => "korean",
        "ru" => "russian",
        "th" => "thai",
        "vi" => "vietnamese",
        "ja" => "japanese",
        "tr" => "turkish",
        "hi" => "hindi",
        "ms" => "malay",
        "nl" => "dutch",
        "sv" => "swedish",
        "da" => "danish",
        "fi" => "finnish",
        "pl" => "polish",
        "cs" => "czech",
        "fa" => "persian",
        "el" => "greek",
        "ro" => "romanian",
        "hu" => "hungarian",
        "mk" => "macedonian",
        _ => return None,
    })
}

// ── Private helpers ──────────────────────────────────────────────────────────

async fn download_file(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    progress: &impl Fn(u64, u64),
) -> AppResult<()> {
    let partial = PathBuf::from(format!("{}.partial", dest.display()));

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| AppError::ModelDownloadFailed(format!("Download failed: {}", e)))?;

    if !resp.status().is_success() {
        return Err(AppError::ModelDownloadFailed(format!(
            "Download returned HTTP {}",
            resp.status()
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

    tokio::fs::rename(&partial, &dest).await?;
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
