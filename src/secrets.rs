// Speech to Text - Secure secret storage
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Secure storage for sensitive values (the HuggingFace token) via the
//! freedesktop Secret Service (GNOME Keyring / KWallet) using `oo7`.
//!
//! All functions are async and degrade gracefully: if no Secret Service is
//! available, `load_hf_token` returns `None` and the store/delete helpers
//! return an error the caller can surface or ignore.

use crate::error::{AppError, AppResult};

/// Attributes uniquely identifying our token item in the keyring.
const ATTRS: [(&str, &str); 2] = [("application", "speech-to-text"), ("type", "hf_token")];
const LABEL: &str = "Speech to Text — HuggingFace token";

/// Store (or replace) the HuggingFace token in the Secret Service.
pub async fn store_hf_token(token: &str) -> AppResult<()> {
    let keyring = oo7::Keyring::new()
        .await
        .map_err(|e| AppError::Config(format!("Keyring unavailable: {e}")))?;
    keyring
        .create_item(LABEL, &ATTRS, token.to_string(), true)
        .await
        .map_err(|e| AppError::Config(format!("Failed to store token: {e}")))?;
    Ok(())
}

/// Load the HuggingFace token from the Secret Service, if present.
pub async fn load_hf_token() -> Option<String> {
    let keyring = oo7::Keyring::new().await.ok()?;
    let items = keyring.search_items(&ATTRS).await.ok()?;
    let item = items.first()?;
    let secret = item.secret().await.ok()?;
    String::from_utf8(secret.as_bytes().to_vec()).ok()
}

/// Remove the stored HuggingFace token.
pub async fn delete_hf_token() -> AppResult<()> {
    let keyring = oo7::Keyring::new()
        .await
        .map_err(|e| AppError::Config(format!("Keyring unavailable: {e}")))?;
    keyring
        .delete(&ATTRS)
        .await
        .map_err(|e| AppError::Config(format!("Failed to delete token: {e}")))?;
    Ok(())
}
