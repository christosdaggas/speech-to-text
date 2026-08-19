// Speech to Text - Voice Activity Detection
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Silero VAD (via whisper.cpp) — finds where speech actually is.
//!
//! Used as a cheap pre-pass before Whisper decoding: recordings that contain
//! no speech at all are skipped outright (no decode, no hallucinated text),
//! and leading/trailing silence is trimmed so the decoder only sees the
//! speech span. Interior silences are left alone — cutting them would shift
//! every later segment timestamp, and the decoder crosses them cheaply.
//!
//! whisper.cpp's own `whisper_full_params.vad` is NOT used: that flag is
//! consumed only by the `whisper_full()` convenience wrapper, and this app
//! (like whisper-rs) drives `whisper_full_with_state()`, which bypasses it —
//! verified experimentally against whisper.cpp 1.8.3.
//!
//! The Silero model (~865 KB, `ggml-silero-v5.1.2.bin`) ships with the app in
//! `data/vad/`; packaged builds install it under `/usr/share/speech-to-text`.

use std::path::PathBuf;
use tracing::{debug, warn};
use whisper_rs::{WhisperVadContext, WhisperVadContextParams, WhisperVadParams};

/// Audio sample rate the whole pipeline runs at.
const SAMPLE_RATE: usize = 16_000;

/// Extra samples kept on each side of the detected speech span, so a soft
/// onset or trailing consonant is never clipped (Silero already pads each
/// segment a little; this pads the overall span once more).
const EDGE_PAD_SAMPLES: usize = SAMPLE_RATE / 2; // 500 ms

/// The bundled VAD model, or None when it isn't installed. Debug builds read
/// it from the source tree (same pattern as the translation catalogues);
/// release builds from the packaged data directory.
pub fn model_path() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(dir) = option_env!("VAD_DIR") {
        candidates.push(PathBuf::from(dir).join("ggml-silero-v5.1.2.bin"));
    }
    candidates.push(PathBuf::from(
        "/usr/share/speech-to-text/ggml-silero-v5.1.2.bin",
    ));
    candidates.into_iter().find(|p| p.exists())
}

/// The sample range `[start, end)` that contains all detected speech, padded,
/// or `None` when the audio contains no speech at all.
///
/// Any VAD failure (missing model, load error) degrades to "treat everything
/// as speech" — transcription must never be lost to a broken pre-pass.
pub fn speech_span(audio: &[f32]) -> SpeechSpan {
    let Some(model) = model_path() else {
        return SpeechSpan::Unavailable;
    };
    let mut ctx_params = WhisperVadContextParams::default();
    ctx_params.set_n_threads(4);
    // The VAD net is tiny; CPU keeps the GPU (and its VRAM) for the ASR model.
    ctx_params.set_use_gpu(false);

    let mut ctx = match WhisperVadContext::new(model.to_string_lossy().as_ref(), ctx_params) {
        Ok(c) => c,
        Err(e) => {
            warn!("VAD model failed to load, skipping VAD: {e}");
            return SpeechSpan::Unavailable;
        }
    };

    let segments = match ctx.segments_from_samples(WhisperVadParams::new(), audio) {
        Ok(s) => s,
        Err(e) => {
            warn!("VAD run failed, skipping VAD: {e}");
            return SpeechSpan::Unavailable;
        }
    };

    let mut first_cs = f32::MAX;
    let mut last_cs = 0.0f32;
    let mut count = 0usize;
    for seg in segments {
        first_cs = first_cs.min(seg.start);
        last_cs = last_cs.max(seg.end);
        count += 1;
    }
    if count == 0 {
        debug!("VAD: no speech detected in {} samples", audio.len());
        return SpeechSpan::NoSpeech;
    }

    // Segment timestamps are in centiseconds.
    let to_sample = |cs: f32| ((cs / 100.0) * SAMPLE_RATE as f32) as usize;
    let start = to_sample(first_cs).saturating_sub(EDGE_PAD_SAMPLES);
    let end = (to_sample(last_cs) + EDGE_PAD_SAMPLES).min(audio.len());
    debug!(
        "VAD: {} speech segments, span {:.2}s–{:.2}s of {:.2}s",
        count,
        start as f32 / SAMPLE_RATE as f32,
        end as f32 / SAMPLE_RATE as f32,
        audio.len() as f32 / SAMPLE_RATE as f32,
    );
    SpeechSpan::Speech { start, end }
}

/// Outcome of the VAD pre-pass.
pub enum SpeechSpan {
    /// Speech found: decode `audio[start..end]` and add the start offset back
    /// onto every segment timestamp.
    Speech { start: usize, end: usize },
    /// The whole clip is silence/noise — skip decoding entirely.
    NoSpeech,
    /// VAD could not run (model missing or failed) — decode everything.
    Unavailable,
}
