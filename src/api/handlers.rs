// Speech to Text - API endpoint handlers
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Endpoint bodies for the local API: `/v1/transcribe`, `/v1/translate`,
//! `/v1/health`, `/v1/models`. Cross-cutting concerns (auth, CORS, routing)
//! live in [`super::server`].

use std::collections::HashMap;
use std::io::Write;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::{header, Request, Response, StatusCode};
use serde::{Deserialize, Serialize};

use super::server::{json_error, json_ok};
use super::{
    Job, Resp, ServerState, BODY_FRAME_TIMEOUT_SECS, MAX_API_UPLOAD_BYTES, REQUEST_TIMEOUT_SECS,
};
use crate::config::AppConfig;
use crate::recording::{DictationMode, DictationParams};

// ── Response payloads ────────────────────────────────────────────────────────

#[derive(Serialize)]
struct SegmentJson {
    start_ms: i64,
    end_ms: i64,
    text: String,
}

#[derive(Serialize)]
struct TranscribeResponse {
    /// Cleaned, mode-formatted transcript.
    text: String,
    /// Raw engine output before sanitize/mode formatting.
    raw_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detected_language: Option<String>,
    confidence: f32,
    duration_secs: f32,
    /// Present only when `translate_to` was requested (LLM translation).
    #[serde(skip_serializing_if = "Option::is_none")]
    translated_text: Option<String>,
    /// Per-segment timestamps; omitted when `segments=false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    segments: Option<Vec<SegmentJson>>,
}

// ── GET /v1/health ───────────────────────────────────────────────────────────

pub(super) fn health() -> Resp {
    let config = AppConfig::load();
    #[derive(Serialize)]
    struct Health {
        status: &'static str,
        version: &'static str,
        backend: String,
        model: String,
    }
    json_ok(&Health {
        status: "ok",
        version: crate::VERSION,
        backend: config.backend.clone(),
        model: config.selected_model.clone(),
    })
}

// ── GET /v1/models ───────────────────────────────────────────────────────────

pub(super) fn models(state: &ServerState) -> Resp {
    let config = AppConfig::load();
    #[derive(Serialize)]
    struct ModelJson {
        id: String,
        downloaded: bool,
        selected: bool,
    }
    #[derive(Serialize)]
    struct ModelsResponse {
        models: Vec<ModelJson>,
        selected: String,
    }
    let models: Vec<ModelJson> = state
        .catalog
        .downloaded_models()
        .into_iter()
        .map(|id| ModelJson {
            selected: id == config.selected_model,
            downloaded: true,
            id,
        })
        .collect();
    json_ok(&ModelsResponse {
        models,
        selected: config.selected_model,
    })
}

// ── POST /v1/transcribe ──────────────────────────────────────────────────────

pub(super) async fn transcribe(req: Request<Incoming>, state: &ServerState) -> Resp {
    let _upload_permit = match state.uploads.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => return too_busy(),
    };
    let query = req.uri().query().unwrap_or("").to_string();
    let content_type = req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // Reject early on a declared oversized Content-Length (don't trust it; the
    // streaming read below enforces the cap regardless).
    if let Some(len) = req
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<usize>().ok())
    {
        if len > MAX_API_UPLOAD_BYTES {
            return json_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "too_large",
                "Upload exceeds the size limit",
            );
        }
    }

    let body = match read_capped(req, MAX_API_UPLOAD_BYTES).await {
        Ok(b) => b,
        Err(ReadErr::TooLarge) => {
            return json_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "too_large",
                "Upload exceeds the size limit",
            )
        }
        Err(ReadErr::Timeout) => {
            return json_error(
                StatusCode::REQUEST_TIMEOUT,
                "read_timeout",
                "Upload stalled",
            )
        }
        Err(ReadErr::Io(e)) => return json_error(StatusCode::BAD_REQUEST, "read_failed", &e),
    };

    // Audio bytes come from a multipart `file`/`audio` field (browsers) or the
    // raw body (native clients). Multipart text fields also feed params.
    let (audio_bytes, mut fields) = if content_type.starts_with("multipart/form-data") {
        match extract_multipart(&content_type, body).await {
            Ok(v) => v,
            Err(e) => return json_error(StatusCode::BAD_REQUEST, "bad_multipart", &e),
        }
    } else {
        (body, HashMap::new())
    };

    if audio_bytes.is_empty() {
        return json_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "no_audio",
            "No audio data in request",
        );
    }

    // Query string overrides multipart form fields for the same key.
    for (k, v) in parse_query(&query) {
        fields.insert(k, v);
    }

    let config = AppConfig::load();
    let mut params = DictationParams::from_config(&config);
    if let Err(message) = apply_overrides(&mut params, &fields) {
        return json_error(StatusCode::UNPROCESSABLE_ENTITY, "bad_parameter", &message);
    }

    let translate_to = fields
        .get("translate_to")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if let Some(target) = translate_to.as_deref() {
        if let Err(message) = validate_text_parameter("translate_to", target, 64) {
            return json_error(StatusCode::UNPROCESSABLE_ENTITY, "bad_parameter", &message);
        }
    }
    let include_segments = fields.get("segments").map(|v| v != "false").unwrap_or(true);

    let temp = match tokio::task::spawn_blocking(move || write_temp(&audio_bytes)).await {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, "temp_failed", &e),
        Err(e) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "temp_failed",
                &format!("Temporary-file task failed: {e}"),
            )
        }
    };

    // Hand off to the inference worker; bounded queue ⇒ 429 when full.
    let (reply_tx, reply_rx) = async_channel::bounded(1);
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let job = Job {
        audio: temp,
        params,
        reply: reply_tx,
        cancelled: cancelled.clone(),
    };
    if state.jobs.try_send(job).is_err() {
        return too_busy();
    }

    let outcome = match tokio::time::timeout(
        std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS),
        reply_rx.recv(),
    )
    .await
    {
        Ok(Ok(Ok(o))) => o,
        Ok(Ok(Err(msg))) => return transcription_error(&msg),
        Ok(Err(_)) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "worker_gone",
                "Worker unavailable",
            )
        }
        Err(_) => {
            cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
            return json_error(
                StatusCode::GATEWAY_TIMEOUT,
                "timeout",
                "Transcription timed out",
            );
        }
    };

    // Optional LLM translation of the transcript to an arbitrary language.
    let mut translated_text = None;
    if let Some(lang) = translate_to {
        match llm_translate(&config, &outcome.cleaned_text, &lang).await {
            Ok(t) => translated_text = Some(t),
            Err(e) => return json_error(StatusCode::BAD_GATEWAY, "translate_failed", &e),
        }
    }

    let segments = include_segments.then(|| {
        outcome
            .segments
            .iter()
            .map(|(start_ms, end_ms, text)| SegmentJson {
                start_ms: *start_ms,
                end_ms: *end_ms,
                text: text.clone(),
            })
            .collect()
    });

    json_ok(&TranscribeResponse {
        text: outcome.cleaned_text,
        raw_text: outcome.raw_text,
        detected_language: outcome.detected_language,
        confidence: outcome.confidence,
        duration_secs: outcome.duration_secs,
        translated_text,
        segments,
    })
}

// ── POST /v1/translate (text-only) ───────────────────────────────────────────

pub(super) async fn translate(req: Request<Incoming>) -> Resp {
    #[derive(Deserialize)]
    struct TranslateReq {
        text: String,
        target_language: String,
    }
    let body = match read_capped(req, 4 * 1024 * 1024).await {
        Ok(b) => b,
        Err(ReadErr::TooLarge) => {
            return json_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "too_large",
                "Body exceeds the size limit",
            )
        }
        Err(ReadErr::Timeout) => {
            return json_error(
                StatusCode::REQUEST_TIMEOUT,
                "read_timeout",
                "Request body stalled",
            )
        }
        Err(ReadErr::Io(e)) => return json_error(StatusCode::BAD_REQUEST, "read_failed", &e),
    };
    let parsed: TranslateReq = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => return json_error(StatusCode::UNPROCESSABLE_ENTITY, "bad_json", &e.to_string()),
    };
    if let Err(message) = validate_text_parameter("text", &parsed.text, 1024 * 1024) {
        return json_error(StatusCode::UNPROCESSABLE_ENTITY, "bad_parameter", &message);
    }
    if let Err(message) = validate_text_parameter("target_language", &parsed.target_language, 64) {
        return json_error(StatusCode::UNPROCESSABLE_ENTITY, "bad_parameter", &message);
    }
    let config = AppConfig::load();
    match llm_translate(&config, &parsed.text, &parsed.target_language).await {
        Ok(t) => json_ok(&serde_json::json!({ "translated_text": t })),
        Err(e) => json_error(StatusCode::BAD_GATEWAY, "translate_failed", &e),
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

enum ReadErr {
    TooLarge,
    Timeout,
    Io(String),
}

/// Read the whole request body into memory, aborting if it exceeds `cap`.
async fn read_capped(req: Request<Incoming>, cap: usize) -> Result<Vec<u8>, ReadErr> {
    let mut body = req.into_body();
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let next = tokio::time::timeout(
            std::time::Duration::from_secs(BODY_FRAME_TIMEOUT_SECS),
            body.frame(),
        )
        .await
        .map_err(|_| ReadErr::Timeout)?;
        let Some(next) = next else { break };
        let frame = next.map_err(|e| ReadErr::Io(e.to_string()))?;
        if let Ok(data) = frame.into_data() {
            if buf.len() + data.len() > cap {
                return Err(ReadErr::TooLarge);
            }
            buf.extend_from_slice(&data);
        }
    }
    Ok(buf)
}

/// Write bytes to a private (0600) temp file and return its path (unlinked on
/// drop of the returned `TempPath`).
fn write_temp(bytes: &[u8]) -> Result<tempfile::TempPath, String> {
    let mut tf = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    tf.write_all(bytes).map_err(|e| e.to_string())?;
    tf.flush().map_err(|e| e.to_string())?;
    Ok(tf.into_temp_path())
}

/// Parse a URL query string into a map (percent-decoded).
fn parse_query(query: &str) -> HashMap<String, String> {
    url::form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect()
}

/// Pull the audio bytes (the `file`/`audio` part or any part with a filename)
/// and the remaining text fields out of a multipart body.
async fn extract_multipart(
    content_type: &str,
    body: Vec<u8>,
) -> Result<(Vec<u8>, HashMap<String, String>), String> {
    let boundary = multer::parse_boundary(content_type).map_err(|e| e.to_string())?;
    let stream = futures::stream::once(async move { Ok::<_, std::io::Error>(Bytes::from(body)) });
    let mut multipart = multer::Multipart::new(stream, boundary);

    let mut audio: Vec<u8> = Vec::new();
    let mut fields: HashMap<String, String> = HashMap::new();
    while let Some(field) = multipart.next_field().await.map_err(|e| e.to_string())? {
        let name = field.name().map(|s| s.to_string());
        let is_file =
            field.file_name().is_some() || matches!(name.as_deref(), Some("file") | Some("audio"));
        if is_file {
            let data = field.bytes().await.map_err(|e| e.to_string())?;
            audio = data.to_vec();
        } else if let Some(name) = name {
            let value = field.text().await.map_err(|e| e.to_string())?;
            fields.insert(name, value);
        }
    }
    Ok((audio, fields))
}

/// Apply request overrides onto config-derived params. The API contract: the
/// Whisper `translate` flag defaults OFF (it does NOT inherit the GUI's
/// "Translate to English" toggle); an explicit `language` overrides auto-detect.
fn apply_overrides(
    params: &mut DictationParams,
    fields: &HashMap<String, String>,
) -> Result<(), String> {
    params.translate = matches!(
        fields.get("translate").map(|s| s.as_str()),
        Some("true") | Some("1")
    );
    if let Some(lang) = fields.get("language") {
        let lang = lang.trim();
        if !lang.is_empty() && !lang.eq_ignore_ascii_case("auto") {
            let valid = (2..=16).contains(&lang.len())
                && lang.bytes().all(|b| b.is_ascii_alphabetic() || b == b'-');
            if !valid {
                return Err("language must be a 2-16 character language code".into());
            }
        }
        params.language_code = if lang.is_empty() || lang.eq_ignore_ascii_case("auto") {
            None
        } else {
            Some(lang.to_string())
        };
    }
    if let Some(beam) = fields.get("beam_size").and_then(|s| s.parse::<u32>().ok()) {
        if !(1..=8).contains(&beam) {
            return Err("beam_size must be between 1 and 8".into());
        }
        params.beam_size = beam;
    } else if fields.contains_key("beam_size") {
        return Err("beam_size must be an integer between 1 and 8".into());
    }
    if let Some(temp) = fields
        .get("temperature")
        .and_then(|s| s.parse::<f32>().ok())
    {
        if !temp.is_finite() || !(0.0..=1.0).contains(&temp) {
            return Err("temperature must be a finite number between 0 and 1".into());
        }
        params.temperature = temp;
    } else if fields.contains_key("temperature") {
        return Err("temperature must be a finite number between 0 and 1".into());
    }
    if let Some(prompt) = fields.get("initial_prompt") {
        validate_text_parameter("initial_prompt", prompt, 8 * 1024)?;
        if !prompt.trim().is_empty() {
            params.initial_prompt = Some(prompt.clone());
        }
    }
    if let Some(mode) = fields.get("mode") {
        if !matches!(
            mode.as_str(),
            "plain" | "message" | "email" | "note" | "code_prompt"
        ) {
            return Err("mode must be plain, message, email, note, or code_prompt".into());
        }
        params.mode = DictationMode::from_config_str(mode);
    }
    Ok(())
}

fn validate_text_parameter(name: &str, value: &str, max_bytes: usize) -> Result<(), String> {
    if value.contains('\0') {
        return Err(format!("{name} must not contain NUL characters"));
    }
    if value.len() > max_bytes {
        return Err(format!("{name} exceeds the {max_bytes}-byte limit"));
    }
    Ok(())
}

/// Translate `text` into `target_lang` via the configured LLM (the same path
/// the GUI's "Translate" preset uses). Requires LLM enabled in settings.
async fn llm_translate(
    config: &AppConfig,
    text: &str,
    target_lang: &str,
) -> Result<String, String> {
    if !config.llm_enabled {
        return Err("LLM is not enabled in settings".to_string());
    }
    let preset = crate::config::LlmPreset {
        name: "Translate".into(),
        prompt: String::new(),
        model: None,
        temperature: None,
        translate_to: Some(target_lang.to_string()),
    };
    let mut cfg = crate::application::resolve_llm_cfg(config, &preset);
    if cfg.api_key.is_none() {
        cfg.api_key = crate::secrets::load_llm_api_key().await;
    }
    crate::llm::chat(&cfg, &preset.system_prompt(), text)
        .await
        .map_err(|e| e.user_message())
}

fn transcription_error(msg: &str) -> Resp {
    let status = if msg.contains("not downloaded") {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    json_error(status, "transcription_failed", msg)
}

fn too_busy() -> Resp {
    let body =
        serde_json::json!({ "error": { "code": "busy", "message": "Server is busy; retry shortly" } })
            .to_string();
    Response::builder()
        .status(StatusCode::TOO_MANY_REQUESTS)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::RETRY_AFTER, "2")
        .body(Full::new(Bytes::from(body)))
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::from_static(b"{}"))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> DictationParams {
        DictationParams::from_config(&AppConfig::default())
    }

    #[test]
    fn rejects_nul_and_oversized_prompt_overrides() {
        let mut request_params = params();
        let fields = HashMap::from([("initial_prompt".to_string(), "bad\0prompt".to_string())]);
        assert!(apply_overrides(&mut request_params, &fields).is_err());

        let mut request_params = params();
        let fields = HashMap::from([("initial_prompt".to_string(), "x".repeat(8193))]);
        assert!(apply_overrides(&mut request_params, &fields).is_err());
    }

    #[test]
    fn validates_numeric_and_language_overrides() {
        for (key, value) in [
            ("temperature", "NaN"),
            ("temperature", "1.1"),
            ("beam_size", "0"),
            ("beam_size", "9"),
            ("language", "el\0"),
            ("language", "not a language"),
        ] {
            let mut params = params();
            let fields = HashMap::from([(key.to_string(), value.to_string())]);
            assert!(
                apply_overrides(&mut params, &fields).is_err(),
                "{key}={value}"
            );
        }

        let mut params = params();
        let fields = HashMap::from([
            ("temperature".to_string(), "0.2".to_string()),
            ("beam_size".to_string(), "4".to_string()),
            ("language".to_string(), "el".to_string()),
        ]);
        assert!(apply_overrides(&mut params, &fields).is_ok());
        assert_eq!(params.temperature, 0.2);
        assert_eq!(params.beam_size, 4);
        assert_eq!(params.language_code.as_deref(), Some("el"));
    }
}
