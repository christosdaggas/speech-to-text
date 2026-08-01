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
    "sk-",
    "sk_",
    "hf_",
    "ghp_",
    "gho_",
    "github_pat_",
    "xoxb-",
    "xoxp-",
    "AKIA",
    "AIza",
];

fn looks_secret(token: &str) -> bool {
    if token.len() >= 12 && SECRET_PREFIXES.iter().any(|p| token.starts_with(p)) {
        return true;
    }
    // Prefix-less keys: this app's own API bearer token is 64 plain hex chars,
    // and many providers issue unprefixed keys. Redact any long run that looks
    // like key material — alphanumeric/-/_ only, with both letters and digits.
    // Deliberately NO '.', '/', '+', '=': file paths and dotted filenames
    // (model-00001-of-00002.safetensors, ~/.local/share/...) are the dominant
    // false-positive class, and diagnostic messages must keep them readable.
    token.len() >= 32
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        && token.chars().any(|c| c.is_ascii_digit())
        && token.chars().any(|c| c.is_ascii_alphabetic())
}

/// Remove sensitive substrings from a message before it is shown to the user or
/// written to a log: API keys / bearer tokens, and the user's home-directory
/// path (which leaks the username). Best-effort and dependency-free.
pub fn redact_secrets(input: &str) -> String {
    // Collapse the user's home directory to "~".
    let s = match dirs::home_dir().and_then(|h| h.to_str().map(str::to_string)) {
        Some(home) if !home.is_empty() => input.replace(home.as_str(), "~"),
        _ => input.to_string(),
    };

    // Redact bearer tokens and key-like words, token by token, splitting on
    // ANY whitespace (upstream error bodies are often multi-line — a secret
    // after a newline must not escape the tokenizer) while preserving the
    // original separators.
    let mut out = String::with_capacity(s.len());
    let mut prev_was_bearer = false;
    let mut rest = s.as_str();
    while !rest.is_empty() {
        let non_ws = rest
            .find(|c: char| !c.is_whitespace())
            .unwrap_or(rest.len());
        out.push_str(&rest[..non_ws]);
        rest = &rest[non_ws..];
        if rest.is_empty() {
            break;
        }
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let word = &rest[..end];
        rest = &rest[end..];
        let core = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_');
        let redact = !core.is_empty() && (prev_was_bearer || looks_secret(core));
        prev_was_bearer = core.eq_ignore_ascii_case("bearer");
        if redact {
            out.push_str(&word.replace(core, "[REDACTED]"));
        } else {
            out.push_str(word);
        }
    }
    out
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
    fn redacts_unprefixed_hex_tokens() {
        // The app's own API bearer token: 64 plain hex chars, no known prefix.
        let tok = "9f3a1c0b7d2e485196a0b3c4d5e6f7089f3a1c0b7d2e485196a0b3c4d5e6f708";
        let r = redact_secrets(&format!("401 unauthorized for token {tok}"));
        assert!(!r.contains(tok), "unprefixed token leaked: {r}");
        assert!(r.contains("[REDACTED]"));
    }

    #[test]
    fn redacts_secrets_after_newlines_and_tabs() {
        let r = redact_secrets("upstream said:\nBearer\tsk-ABCDEF0123456789\ndone");
        assert!(!r.contains("sk-ABCDEF0123456789"), "leaked: {r}");
        // Original separators survive redaction.
        assert!(r.contains('\n'));
    }

    #[test]
    fn leaves_ordinary_text_alone() {
        let msg = "Failed to load model ggml-large-v3-q5_1.bin from disk";
        assert_eq!(redact_secrets(msg), msg);
    }

    #[test]
    fn leaves_paths_and_dotted_filenames_alone() {
        // Long path-like and dotted tokens must survive: integrity and
        // model-missing errors are undiagnosable without them.
        let msg = "Model file not found: \"/opt/stt/models/ggml-large-v3-q5_1.bin\"";
        assert_eq!(redact_secrets(msg), msg);
        let msg2 = "Refusing unexpected model file: model-00001-of-00002.safetensors";
        assert_eq!(redact_secrets(msg2), msg2);
    }

    #[test]
    fn leaves_ordinary_text_intact() {
        let s = "Model not found: ggml-base.en at https://example.com/v1";
        assert_eq!(redact_secrets(s), s);
    }
}
