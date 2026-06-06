// Speech to Text - Error Types
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Application error types.

use thiserror::Error;

/// Token prefixes that indicate an API key / access token we must never show or
/// log. Conservative on purpose: we only redact words that both start with one
/// of these and are long enough to plausibly be a secret, so ordinary text
/// (model names, URLs) is left intact.
const SECRET_PREFIXES: &[&str] = &[
    "sk-", "sk_", "hf_", "ghp_", "gho_", "github_pat_", "xoxb-", "xoxp-", "AKIA", "AIza",
];

fn looks_secret(token: &str) -> bool {
    token.len() >= 12 && SECRET_PREFIXES.iter().any(|p| token.starts_with(p))
}

/// Remove sensitive substrings from a message before it is shown to the user or
/// written to a log: API keys / bearer tokens, and the user's home-directory
/// path (which leaks the username). Best-effort and dependency-free.
pub fn redact_secrets(input: &str) -> String {
    // Collapse the user's home directory to "~".
    let mut s = match dirs::home_dir().and_then(|h| h.to_str().map(str::to_string)) {
        Some(home) if !home.is_empty() => input.replace(home.as_str(), "~"),
        _ => input.to_string(),
    };

    // Redact bearer tokens and key-like words, token by token.
    let mut prev_was_bearer = false;
    let words: Vec<String> = s
        .split(' ')
        .map(|word| {
            let core =
                word.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_');
            let redact = !core.is_empty() && (prev_was_bearer || looks_secret(core));
            prev_was_bearer = core.eq_ignore_ascii_case("bearer");
            if redact {
                word.replace(core, "[REDACTED]")
            } else {
                word.to_string()
            }
        })
        .collect();
    s = words.join(" ");
    s
}

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Audio error: {0}")]
    Audio(String),

    #[error("No audio input devices found")]
    NoAudioDevices,

    #[error("Microphone not available: {0}")]
    MicrophoneUnavailable(String),

    #[error("Transcription error: {0}")]
    Transcription(String),

    #[error("Model not found: {0}")]
    ModelNotFound(String),

    #[error("Model loading failed: {0}")]
    ModelLoadFailed(String),

    #[error("Model download failed: {0}")]
    ModelDownloadFailed(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

impl AppError {
    /// A redacted, user-safe rendering of this error (no secrets or home paths).
    /// Use this for toasts/dialogs and anywhere an error reaches the user.
    pub fn user_message(&self) -> String {
        redact_secrets(&self.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_bearer_and_keys() {
        let r = redact_secrets("Authorization: Bearer sk-ABCDEF0123456789 failed");
        assert!(!r.contains("sk-ABCDEF0123456789"), "key leaked: {r}");
        assert!(r.contains("[REDACTED]"));
    }

    #[test]
    fn redacts_hf_token_anywhere() {
        let r = redact_secrets("token hf_abcdefABCDEF0123 rejected");
        assert!(!r.contains("hf_abcdefABCDEF0123"));
    }

    #[test]
    fn leaves_ordinary_text_intact() {
        let s = "Model not found: ggml-base.en at https://example.com/v1";
        assert_eq!(redact_secrets(s), s);
    }
}
