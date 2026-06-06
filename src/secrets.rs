// Speech to Text - Secure secret storage
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Secure storage for sensitive values (the HuggingFace token, the LLM API key)
//! via the freedesktop Secret Service (GNOME Keyring / KWallet) using `oo7`.
//!
//! All functions are async and degrade gracefully: if no Secret Service is
//! available, the `load_*` helpers return `None` and the store/delete helpers
//! return an error the caller can surface or ignore.

use crate::error::{AppError, AppResult};

/// Store (or replace) a named secret in the Secret Service. `secret_type` is the
/// keyring `type` attribute that distinguishes our items (e.g. "hf_token").
async fn store_secret(secret_type: &str, secret: &str) -> AppResult<()> {
    let attrs = [("application", "speech-to-text"), ("type", secret_type)];
    let label = format!("Speech to Text — {secret_type}");
    let keyring = oo7::Keyring::new()
        .await
        .map_err(|e| AppError::Config(format!("Keyring unavailable: {e}")))?;
    keyring
        .create_item(&label, &attrs, secret.to_string(), true)
        .await
        .map_err(|e| AppError::Config(format!("Failed to store secret: {e}")))?;
    Ok(())
}

/// Load a named secret from the Secret Service, if present.
async fn load_secret(secret_type: &str) -> Option<String> {
    let attrs = [("application", "speech-to-text"), ("type", secret_type)];
    let keyring = oo7::Keyring::new().await.ok()?;
    let items = keyring.search_items(&attrs).await.ok()?;
    let item = items.first()?;
    let secret = item.secret().await.ok()?;
    String::from_utf8(secret.as_bytes().to_vec()).ok()
}

/// Remove a named secret.
async fn delete_secret(secret_type: &str) -> AppResult<()> {
    let attrs = [("application", "speech-to-text"), ("type", secret_type)];
    let keyring = oo7::Keyring::new()
        .await
        .map_err(|e| AppError::Config(format!("Keyring unavailable: {e}")))?;
    keyring
        .delete(&attrs)
        .await
        .map_err(|e| AppError::Config(format!("Failed to delete secret: {e}")))?;
    Ok(())
}

// ── HuggingFace token (Cohere model download) ───────────────────────────────

/// Store (or replace) the HuggingFace token in the Secret Service.
pub async fn store_hf_token(token: &str) -> AppResult<()> {
    store_secret("hf_token", token).await
}

/// Load the HuggingFace token from the Secret Service, if present.
pub async fn load_hf_token() -> Option<String> {
    load_secret("hf_token").await
}

/// Remove the stored HuggingFace token.
pub async fn delete_hf_token() -> AppResult<()> {
    delete_secret("hf_token").await
}

// ── LLM API key ─────────────────────────────────────────────────────────────

/// Store (or replace) the LLM API key in the Secret Service.
pub async fn store_llm_api_key(key: &str) -> AppResult<()> {
    store_secret("llm_api_key", key).await
}

/// Load the LLM API key from the Secret Service, if present.
pub async fn load_llm_api_key() -> Option<String> {
    load_secret("llm_api_key").await
}

/// Remove the stored LLM API key.
pub async fn delete_llm_api_key() -> AppResult<()> {
    delete_secret("llm_api_key").await
}
