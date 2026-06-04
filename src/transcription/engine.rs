// Speech to Text - Transcription Engine
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Shared transcription result types and Whisper implementation.

use std::path::Path;
use tracing::info;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::error::{AppError, AppResult};

// ---------------------------------------------------------------------------
// Shared result types
// ---------------------------------------------------------------------------

/// A single transcription segment with optional timing and confidence.
#[derive(Debug, Clone)]
pub struct TranscriptionSegment {
    /// Start time in milliseconds (None if the backend doesn't provide timestamps).
    pub start_ms: Option<i64>,
    /// End time in milliseconds (None if the backend doesn't provide timestamps).
    pub end_ms: Option<i64>,
    /// Transcribed text for this segment.
    pub text: String,
    /// Confidence probability (0.0 - 1.0), None if unavailable.
    pub confidence: Option<f32>,
}

/// Complete transcription result.
#[derive(Debug, Clone)]
pub struct TranscriptionResult {
    /// All segments.
    pub segments: Vec<TranscriptionSegment>,
    /// Full concatenated text.
    pub text: String,
    /// Average confidence across all segments (None if unavailable).
    pub average_confidence: Option<f32>,
    /// Detected language (if auto-detect was used).
    pub detected_language: Option<String>,
}

// ---------------------------------------------------------------------------
// Whisper backend
// ---------------------------------------------------------------------------

/// Wrapper around WhisperContext that handles model loading and transcription.
pub struct TranscriptionEngine {
    ctx: WhisperContext,
    model_id: String,
}

impl TranscriptionEngine {
    /// Load a Whisper model from the given path.
    pub fn load_model(model_path: &Path, model_id: &str) -> AppResult<Self> {
        Self::load_model_with_gpu(model_path, model_id, false)
    }

    /// Load a Whisper model with optional GPU acceleration.
    pub fn load_model_with_gpu(model_path: &Path, model_id: &str, use_gpu: bool) -> AppResult<Self> {
        info!("Loading Whisper model from {:?} (GPU: {})", model_path, use_gpu);

        if !model_path.exists() {
            return Err(AppError::ModelNotFound(
                format!("Model file not found: {:?}", model_path)
            ));
        }

        let mut params = WhisperContextParameters::default();
        params.use_gpu(use_gpu);

        let ctx = WhisperContext::new_with_params(
            model_path.to_str().ok_or_else(|| {
                AppError::ModelLoadFailed("Invalid model path encoding".into())
            })?,
            params,
        ).map_err(|e| AppError::ModelLoadFailed(format!("Failed to load model: {}", e)))?;

        info!("Whisper model '{}' loaded successfully (GPU: {})", model_id, use_gpu);

        Ok(Self {
            ctx,
            model_id: model_id.to_string(),
        })
    }

    /// Transcribe audio data.
    ///
    /// `audio` must be mono 16kHz f32 PCM.
    /// `language` is an ISO 639-1 code (e.g., "en", "el") or None for auto-detect.
    /// `n_threads` is the number of CPU threads to use.
    /// `translate` if true, translates the output to English.
    /// `beam_size` controls beam search width (1 = greedy).
    /// `temperature` controls sampling randomness (0.0 = deterministic).
    /// `initial_prompt` optional prompt with domain-specific vocabulary.
    pub fn transcribe(
        &self,
        audio: &[f32],
        language: Option<&str>,
        n_threads: u32,
        translate: bool,
        beam_size: u32,
        temperature: f32,
        initial_prompt: Option<&str>,
    ) -> AppResult<TranscriptionResult> {
        if audio.is_empty() {
            return Ok(TranscriptionResult {
                segments: Vec::new(),
                text: String::new(),
                average_confidence: None,
                detected_language: None,
            });
        }

        info!(
            "Transcribing {:.1}s of audio with model '{}' ({} threads)",
            audio.len() as f32 / 16000.0,
            self.model_id,
            n_threads
        );

        let beam_size = beam_size.min(8); // whisper.cpp supports max 8 decoders
        let mut params = if beam_size > 1 {
            FullParams::new(SamplingStrategy::BeamSearch { beam_size: beam_size as i32, patience: -1.0 })
        } else {
            FullParams::new(SamplingStrategy::Greedy { best_of: 1 })
        };

        params.set_n_threads(n_threads as i32);
        params.set_translate(translate);
        params.set_no_context(true);
        params.set_no_timestamps(false);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_token_timestamps(true);
        params.set_suppress_blank(true);
        params.set_suppress_non_speech_tokens(true);
        params.set_temperature(temperature);
        params.set_temperature_inc(0.0);
        params.set_entropy_thold(2.2);
        params.set_logprob_thold(-0.8);

        if let Some(lang) = language {
            params.set_language(Some(lang));
        } else {
            // Auto-detect the language AND transcribe. Note: `set_detect_language(true)`
            // makes whisper.cpp detect the language and then STOP without
            // transcribing (0 segments), so we must pass the special "auto"
            // language instead. The detected language is still read back below via
            // `full_lang_id_from_state()`.
            params.set_language(Some("auto"));
        }

        if let Some(prompt) = initial_prompt {
            params.set_initial_prompt(prompt);
        }

        // Run transcription
        let mut state = self.ctx.create_state()
            .map_err(|e| AppError::Transcription(format!("Failed to create state: {}", e)))?;

        state.full(params, audio)
            .map_err(|e| AppError::Transcription(format!("Transcription failed: {}", e)))?;

        // When auto-detecting, surface the language Whisper picked (ISO 639-1 code).
        let detected_language = if language.is_none() {
            state.full_lang_id_from_state().ok()
                .and_then(whisper_rs::get_lang_str)
                .map(|s| s.to_string())
        } else {
            None
        };

        // Collect segments
        let num_segments = state.full_n_segments()
            .map_err(|e| AppError::Transcription(format!("Failed to get segments: {}", e)))?;

        let mut segments = Vec::with_capacity(num_segments as usize);
        let mut full_text = String::new();
        let mut total_confidence = 0.0f32;

        for i in 0..num_segments {
            let start_ms = state.full_get_segment_t0(i)
                .map_err(|e| AppError::Transcription(format!("Failed to get segment start: {}", e)))?
                as i64 * 10; // whisper timestamps are in centiseconds
            let end_ms = state.full_get_segment_t1(i)
                .map_err(|e| AppError::Transcription(format!("Failed to get segment end: {}", e)))?
                as i64 * 10;
            let text = state.full_get_segment_text(i)
                .map_err(|e| AppError::Transcription(format!("Failed to get segment text: {}", e)))?;

            // Calculate average token probability for this segment
            let n_tokens = state.full_n_tokens(i)
                .unwrap_or(0);
            let confidence = if n_tokens > 0 {
                let mut sum = 0.0f32;
                for t in 0..n_tokens {
                    sum += state.full_get_token_prob(i, t).unwrap_or(0.0);
                }
                sum / n_tokens as f32
            } else {
                0.0
            };

            full_text.push_str(&text);
            total_confidence += confidence;

            segments.push(TranscriptionSegment {
                start_ms: Some(start_ms),
                end_ms: Some(end_ms),
                text,
                confidence: Some(confidence),
            });
        }

        let average_confidence = if segments.is_empty() {
            None
        } else {
            Some(total_confidence / segments.len() as f32)
        };

        info!(
            "Transcription complete: {} segments, avg confidence {:.1}%",
            segments.len(),
            average_confidence.unwrap_or(0.0) * 100.0
        );

        Ok(TranscriptionResult {
            segments,
            text: full_text.trim().to_string(),
            average_confidence,
            detected_language,
        })
    }

    /// Get the loaded model ID.
    pub fn model_id(&self) -> &str {
        &self.model_id
    }
}