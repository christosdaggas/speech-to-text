// Speech to Text - Error Types
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Application error types.

use thiserror::Error;

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

pub type AppResult<T> = Result<T, AppError>;
