// Speech to Text - Transcription Module
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Transcription types, Whisper implementation, model management, and post-processing.

pub mod archive;
pub mod cohere;
pub mod engine;
pub mod model;
pub mod postprocess;
pub mod qwen;
pub mod safe_path;
pub mod summary;
pub mod verify;

pub use engine::TranscriptionEngine;
pub use model::{ModelCatalog, download_model};

/// Shared HTTP client for model/runtime downloads. It sets a connect timeout
/// and an idle read timeout so a stalled server can't hang a download forever,
/// but deliberately has **no** overall request timeout — model archives are
/// large and legitimately slow. Falls back to a default client if the builder
/// fails (should never happen with rustls compiled in).
pub fn download_client() -> reqwest::Client {
    use std::time::Duration;
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(120))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Encode mono f32 PCM samples as a 16-bit WAV file in memory. Shared by the
/// CLI-sidecar backends (Cohere, Qwen3-ASR) that take a WAV file path.
pub(crate) fn encode_wav_16bit(samples: &[f32], sample_rate: u32) -> Vec<u8> {
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
