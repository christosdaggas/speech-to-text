// Speech to Text - LLM client
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Minimal OpenAI-compatible chat client for local/remote LLMs (LM Studio,
//! Ollama, vLLM, OpenAI, …). All of them expose `POST {base}/chat/completions`
//! and `GET {base}/models`, so a single client covers them. The Bearer key is
//! optional (local servers usually don't need it).
//!
//! Calls run on the global Tokio runtime (reqwest needs a reactor) and are
//! bridged back to the GTK main loop via `async_channel` — never inside the
//! blocking transcription worker.

use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::time::Duration;
use url::Url;

use crate::application::tokio_runtime;
use crate::error::{AppError, AppResult};
use crate::secrets;

/// Resolved connection settings for one request.
#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub api_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub temperature: f32,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
    temperature: f32,
    stream: bool,
}

#[derive(Deserialize)]
struct ChatResponse {
    #[serde(default)]
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: RespMessage,
}

#[derive(Deserialize)]
struct RespMessage {
    #[serde(default)]
    content: String,
}

#[derive(Deserialize)]
struct ModelsResponse {
    #[serde(default)]
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
}

/// Join a base URL and a path, tolerating a trailing slash on the base.
fn endpoint(base: &str, path: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), path)
}

/// Whether a plain (non-TLS) `http://` endpoint is acceptable for this host.
/// Only loopback and private-LAN hosts qualify (LM Studio / Ollama / a home
/// server); transcripts to any public host must go over HTTPS so they aren't
/// sent in cleartext.
fn host_allows_plaintext(host: &str) -> bool {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
            IpAddr::V6(v6) => {
                let s = v6.segments();
                // ::1 loopback, fc00::/7 unique-local, or fe80::/10 link-local.
                v6.is_loopback() || (s[0] & 0xfe00) == 0xfc00 || (s[0] & 0xffc0) == 0xfe80
            }
        };
    }
    // Hostnames: localhost and single-label / known-local suffixes are LAN/mDNS.
    let h = host.to_ascii_lowercase();
    h == "localhost"
        || h.ends_with(".localhost")
        || h.ends_with(".local")
        || h.ends_with(".lan")
        || h.ends_with(".home")
        || h.ends_with(".internal")
        || !h.contains('.')
}

/// Validate an LLM endpoint URL. Returns the parsed URL on success. Rejects
/// non-http(s) schemes and refuses plain `http://` to public hosts (cleartext
/// transcript exfiltration risk). `http://` stays allowed for loopback/LAN so
/// local servers like LM Studio keep working.
pub fn validate_endpoint(api_url: &str) -> AppResult<Url> {
    let url = Url::parse(api_url.trim())
        .map_err(|_| AppError::Transcription(format!("LLM API URL is not a valid URL: {api_url}")))?;
    match url.scheme() {
        "https" => {}
        "http" => {
            let host = url
                .host_str()
                .ok_or_else(|| AppError::Transcription("LLM API URL has no host".into()))?;
            if !host_allows_plaintext(host) {
                return Err(AppError::Transcription(format!(
                    "Refusing to send transcripts over plain HTTP to a public host ({host}). \
                     Use https:// for remote endpoints; http:// is allowed only for localhost \
                     or your local network."
                )));
            }
        }
        other => {
            return Err(AppError::Transcription(format!(
                "Unsupported LLM API URL scheme '{other}'. Use http (local) or https."
            )));
        }
    }
    Ok(url)
}

/// The host part of an LLM endpoint URL, for display in consent/privacy UI.
pub fn endpoint_host(api_url: &str) -> Option<String> {
    Url::parse(api_url.trim())
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
}

fn http_client(timeout_secs: u64) -> AppResult<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()?)
}

/// Send a chat-completion request and return the assistant's reply text.
///
/// `skip_all` on the span is deliberate: it gives an `llm.chat` operation
/// context for logs/diagnostics without ever recording the prompt, the user's
/// transcript text, or the API key as span fields.
#[tracing::instrument(name = "llm.chat", skip_all, fields(model = %cfg.model))]
pub async fn chat(cfg: &LlmConfig, system_prompt: &str, user_text: &str) -> AppResult<String> {
    if cfg.api_url.trim().is_empty() {
        return Err(AppError::Transcription("LLM API URL is empty".into()));
    }
    if cfg.model.trim().is_empty() {
        return Err(AppError::Transcription("No LLM model selected".into()));
    }
    // Block cleartext transcript exfiltration to public hosts before sending.
    validate_endpoint(&cfg.api_url)?;

    let body = ChatRequest {
        model: &cfg.model,
        messages: vec![
            Message { role: "system", content: system_prompt },
            Message { role: "user", content: user_text },
        ],
        temperature: cfg.temperature,
        stream: false,
    };

    // Generous timeout: a local server may need to load the model into memory on
    // the very first request, which can take well over a minute for large models.
    let mut req = http_client(300)?.post(endpoint(&cfg.api_url, "chat/completions")).json(&body);
    if let Some(key) = cfg.api_key.as_deref() {
        if !key.is_empty() {
            req = req.header("Authorization", format!("Bearer {key}"));
        }
    }

    let resp = req.send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let txt = resp.text().await.unwrap_or_default();
        let snippet: String = txt.chars().take(200).collect();
        // The upstream body can echo back the request (including any key) — redact.
        let snippet = crate::error::redact_secrets(&snippet);
        return Err(AppError::Transcription(format!("LLM HTTP {status}: {snippet}")));
    }

    // Reject implausibly large responses (DoS guard) before buffering the body.
    if let Some(len) = resp.content_length() {
        if len > crate::limits::MAX_LLM_RESPONSE_BYTES {
            return Err(AppError::Transcription(
                "LLM response is too large.".into(),
            ));
        }
    }

    let parsed: ChatResponse = resp.json().await?;
    let content = parsed
        .choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .ok_or_else(|| AppError::Transcription("LLM returned no choices".into()))?;
    Ok(content.trim().to_string())
}

/// List the model ids exposed by the server (`GET {base}/models`).
pub async fn list_models(cfg: &LlmConfig) -> AppResult<Vec<String>> {
    if cfg.api_url.trim().is_empty() {
        return Err(AppError::Transcription("LLM API URL is empty".into()));
    }
    validate_endpoint(&cfg.api_url)?;
    let mut req = http_client(15)?.get(endpoint(&cfg.api_url, "models"));
    if let Some(key) = cfg.api_key.as_deref() {
        if !key.is_empty() {
            req = req.header("Authorization", format!("Bearer {key}"));
        }
    }
    let resp = req.send().await?;
    if !resp.status().is_success() {
        return Err(AppError::Transcription(format!("LLM HTTP {}", resp.status())));
    }
    let parsed: ModelsResponse = resp.json().await?;
    Ok(parsed.data.into_iter().map(|m| m.id).collect())
}

// ── Async helpers (spawn on Tokio, bridge back via async_channel) ───────────

/// Load the keyring API key into `cfg` if not already set.
async fn fill_key(cfg: &mut LlmConfig) {
    if cfg.api_key.is_none() {
        cfg.api_key = secrets::load_llm_api_key().await;
    }
}

/// Run a chat request off the GTK thread; the receiver yields the reply once.
pub fn improve_async(
    mut cfg: LlmConfig,
    system_prompt: String,
    text: String,
) -> async_channel::Receiver<Result<String, String>> {
    let (tx, rx) = async_channel::bounded(1);
    tokio_runtime().spawn(async move {
        fill_key(&mut cfg).await;
        let res = chat(&cfg, &system_prompt, &text).await.map_err(|e| e.to_string());
        let _ = tx.send(res).await;
    });
    rx
}

/// Fetch the model list off the GTK thread.
pub fn list_models_async(mut cfg: LlmConfig) -> async_channel::Receiver<Result<Vec<String>, String>> {
    let (tx, rx) = async_channel::bounded(1);
    tokio_runtime().spawn(async move {
        fill_key(&mut cfg).await;
        let res = list_models(&cfg).await.map_err(|e| e.to_string());
        let _ = tx.send(res).await;
    });
    rx
}

/// Test the connection: list models, then a 1-line chat probe; returns a status.
pub fn probe_async(mut cfg: LlmConfig) -> async_channel::Receiver<Result<String, String>> {
    let (tx, rx) = async_channel::bounded(1);
    tokio_runtime().spawn(async move {
        fill_key(&mut cfg).await;
        let result = async {
            let models = list_models(&cfg).await?;
            // Use the configured model, or the first available, for the probe.
            if cfg.model.trim().is_empty() {
                if let Some(first) = models.first() {
                    cfg.model = first.clone();
                }
            }
            let reply = chat(&cfg, "You are a connection test.", "Reply with the single word: OK").await?;
            let reply: String = reply.chars().take(40).collect();
            Ok::<String, AppError>(format!("Connected — {} model(s); reply: \"{}\"", models.len(), reply))
        }
        .await
        .map_err(|e| e.to_string());
        let _ = tx.send(result).await;
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_handles_trailing_slash() {
        assert_eq!(endpoint("http://localhost:1234/v1", "chat/completions"), "http://localhost:1234/v1/chat/completions");
        assert_eq!(endpoint("http://localhost:1234/v1/", "models"), "http://localhost:1234/v1/models");
    }

    #[test]
    fn serializes_chat_request() {
        let body = ChatRequest {
            model: "m",
            messages: vec![
                Message { role: "system", content: "sys" },
                Message { role: "user", content: "hi" },
            ],
            temperature: 0.3,
            stream: false,
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["model"], "m");
        assert_eq!(json["stream"], false);
        assert_eq!(json["messages"][0]["role"], "system");
        assert_eq!(json["messages"][1]["content"], "hi");
    }

    #[test]
    fn parses_chat_response() {
        let raw = r#"{"choices":[{"message":{"role":"assistant","content":"  hello  "}}]}"#;
        let parsed: ChatResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.choices[0].message.content.trim(), "hello");
    }

    #[test]
    fn parses_models_response() {
        let raw = r#"{"object":"list","data":[{"id":"a","object":"model"},{"id":"b"}]}"#;
        let parsed: ModelsResponse = serde_json::from_str(raw).unwrap();
        let ids: Vec<String> = parsed.data.into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[test]
    fn https_to_any_host_is_allowed() {
        assert!(validate_endpoint("https://api.openai.com/v1").is_ok());
        assert!(validate_endpoint("https://example.com:8443/v1").is_ok());
    }

    #[test]
    fn http_allowed_for_loopback_and_lan() {
        for url in [
            "http://localhost:1234/v1",
            "http://127.0.0.1:1234/v1",
            "http://[::1]:1234/v1",
            "http://192.168.1.50:11434/v1",
            "http://10.0.0.5/v1",
            "http://172.16.3.4/v1",
            "http://my-server:1234/v1", // single-label LAN name
            "http://nas.local/v1",
        ] {
            assert!(validate_endpoint(url).is_ok(), "should allow http for {url}");
        }
    }

    #[test]
    fn http_to_public_host_is_rejected() {
        for url in [
            "http://api.openai.com/v1",
            "http://example.com/v1",
            "http://8.8.8.8/v1",
        ] {
            assert!(validate_endpoint(url).is_err(), "should reject http for {url}");
        }
    }

    #[test]
    fn invalid_url_and_scheme_are_rejected() {
        assert!(validate_endpoint("not a url").is_err());
        assert!(validate_endpoint("ftp://example.com/v1").is_err());
        assert!(validate_endpoint("file:///etc/passwd").is_err());
    }
}
