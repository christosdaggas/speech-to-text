// Speech to Text - Recording Controller
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Shared, UI-agnostic recording + transcription controller.
//!
//! Owns the single [`AudioCapture`] (one cpal stream) and the single Whisper
//! [`TranscriptionEngine`], so the main window, the mini panel, and the global
//! dictation shortcut all drive the *same* capture and engine instead of
//! fighting over duplicate instances. Capture start/stop must run on the glib
//! main thread (the cpal `Stream` is `!Send`); only transcription is offloaded
//! to a worker thread, exactly as the main window already did.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use std::sync::atomic::AtomicBool;

use crate::audio::AudioCapture;
use crate::audio::capture::RecordingState;
use crate::error::AppResult;
use crate::transcription::engine::{SegmentEvent, TranscribeHooks, TranscriptionResult};
use crate::transcription::{ModelCatalog, TranscriptionEngine, postprocess};

/// Channels + abort handle for a streaming transcription (Whisper only). The UI
/// drains `segments`/`progress` for live display and awaits `outcome` for the
/// final, authoritative result.
pub struct StreamingTranscription {
    pub outcome: async_channel::Receiver<Result<DictationOutcome, String>>,
    pub segments: async_channel::Receiver<SegmentEvent>,
    pub progress: async_channel::Receiver<i32>,
    pub abort: std::sync::Arc<AtomicBool>,
}

/// Which UI currently owns the recording session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingOwner {
    None,
    Main,
    Mini,
    /// A nested short capture for the "Voice edit" feature (spoken instruction).
    VoiceEdit,
}

/// Output formatting mode for a dictation. v1 ships `Plain`; the richer modes
/// are accepted and stored but currently behave identically to `Plain` until
/// LLM-backed rewriting lands in a later phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DictationMode {
    #[default]
    Plain,
    Message,
    Email,
    Note,
    CodePrompt,
}

impl DictationMode {
    pub fn from_config_str(s: &str) -> Self {
        match s {
            "message" => Self::Message,
            "email" => Self::Email,
            "note" => Self::Note,
            "code_prompt" => Self::CodePrompt,
            _ => Self::Plain,
        }
    }

    pub fn as_config_str(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Message => "message",
            Self::Email => "email",
            Self::Note => "note",
            Self::CodePrompt => "code_prompt",
        }
    }
}

/// Apply mode-specific formatting to already-sanitized transcript text.
///
/// Deterministic (no external LLM): each mode reshapes the cleaned transcript
/// for a different destination.
pub fn apply_mode(text: &str, mode: DictationMode) -> String {
    let text = text.trim();
    if text.is_empty() {
        return String::new();
    }
    match mode {
        // Clean prose, exactly as sanitized.
        DictationMode::Plain => text.to_string(),
        // Chat-ready: one paragraph, guaranteed terminal punctuation.
        DictationMode::Message => ensure_terminal_punctuation(&collapse_whitespace(text)),
        // Greeting + body + sign-off scaffold.
        DictationMode::Email => format_email(text),
        // Bulleted list, one bullet per sentence.
        DictationMode::Note => format_note(text),
        // Concise instruction: drop speech fillers, single paragraph, punctuated.
        DictationMode::CodePrompt => {
            ensure_terminal_punctuation(&capitalize_first(&strip_fillers(&collapse_whitespace(text))))
        }
    }
}

/// Collapse runs of whitespace (incl. newlines) to single spaces.
fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Ensure the text ends with sentence-terminating punctuation.
fn ensure_terminal_punctuation(text: &str) -> String {
    let t = text.trim_end();
    if t.is_empty() {
        return String::new();
    }
    match t.chars().last() {
        Some('.') | Some('!') | Some('?') | Some(':') | Some(';') | Some('…') => t.to_string(),
        _ => format!("{t}."),
    }
}

/// Capitalize the first alphabetic character.
fn capitalize_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Split into sentences, keeping terminating punctuation.
fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        current.push(ch);
        if matches!(ch, '.' | '!' | '?' | '…') {
            let s = current.trim().to_string();
            if !s.is_empty() {
                sentences.push(s);
            }
            current.clear();
        }
    }
    let rest = current.trim();
    if !rest.is_empty() {
        sentences.push(rest.to_string());
    }
    sentences
}

/// Format as a bulleted note, one bullet per sentence.
fn format_note(text: &str) -> String {
    let sentences = split_sentences(&collapse_whitespace(text));
    if sentences.is_empty() {
        return String::new();
    }
    sentences.iter().map(|s| format!("• {s}")).collect::<Vec<_>>().join("\n")
}

/// Wrap the body in a simple email scaffold.
fn format_email(text: &str) -> String {
    let body = collapse_whitespace(text);
    format!("Hi,\n\n{body}\n\nBest regards,")
}

/// Single-word speech fillers removed by Code Prompt mode.
const FILLERS: &[&str] = &["um", "umm", "uh", "uhh", "er", "erm", "ah", "hmm", "mhm"];

/// Drop standalone filler words (case-insensitive), preserving everything else.
fn strip_fillers(text: &str) -> String {
    let kept: Vec<&str> = text
        .split_whitespace()
        .filter(|w| {
            let core = w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
            !FILLERS.contains(&core.as_str())
        })
        .collect();
    collapse_whitespace(&kept.join(" "))
}

/// A snapshot of the inference parameters for one dictation run. Deliberately
/// widget-free: the main window builds this from its settings widgets, while
/// the global-shortcut path builds it from `AppConfig` (the main window may be
/// closed).
#[derive(Debug, Clone)]
pub struct DictationParams {
    pub backend: String,
    pub language_code: Option<String>,
    pub n_threads: u32,
    pub beam_size: u32,
    pub temperature: f32,
    pub translate: bool,
    pub initial_prompt: Option<String>,
    pub selected_microphone: Option<String>,
    pub mode: DictationMode,
    /// Personal-dictionary replacement rules applied after sanitization.
    pub replacements: Vec<crate::config::DictReplacement>,
}

impl DictationParams {
    /// Build a parameter snapshot from configuration alone (no widgets). Used by
    /// the global dictation path, which may run with the main window closed.
    pub fn from_config(config: &crate::config::AppConfig) -> Self {
        let language_code = if config.backend == "cohere" {
            // Cohere's CLI has no auto-detect (it defaults to English), so always
            // pass the user's configured language; otherwise non-English speech
            // is mis-transcribed into gibberish.
            Some(config.language.clone().unwrap_or_else(|| "en".to_string()))
        } else if config.auto_detect_language {
            None
        } else {
            config.language.clone()
        };
        Self {
            backend: config.backend.clone(),
            language_code,
            n_threads: config.effective_threads(),
            beam_size: config.beam_size,
            temperature: config.temperature,
            translate: config.translate_to_english,
            initial_prompt: config.effective_initial_prompt(),
            selected_microphone: config.selected_microphone.clone(),
            mode: DictationMode::from_config_str(&config.dictation_mode),
            replacements: if config.dictionary_enabled {
                config.dictionary_replacements.clone()
            } else {
                Vec::new()
            },
        }
    }
}

/// The result of a completed transcription, already sanitized + mode-formatted.
#[derive(Debug, Clone)]
pub struct DictationOutcome {
    /// Raw text straight from the engine (before sanitize/mode).
    pub raw_text: String,
    /// Sanitized + mode-formatted text — what should be shown/pasted/copied.
    pub cleaned_text: String,
    pub confidence: f32,
    /// (start_ms, end_ms, text) segments for SRT export.
    pub segments: Vec<(i64, i64, String)>,
    /// Language Whisper picked, when auto-detect was used.
    pub detected_language: Option<String>,
    /// Recording/clip length in seconds (mic time, or decoded length for files).
    /// Used for words-per-minute stats and history.
    pub duration_secs: f32,
}

/// Run the transcription core synchronously. Intended to be called from a
/// worker thread (it blocks on the engine). Handles both the Whisper and
/// Cohere backends and applies sanitize + mode formatting.
pub fn run_transcription(
    engine: &Arc<Mutex<Option<TranscriptionEngine>>>,
    audio: &[f32],
    params: &DictationParams,
    duration_secs: f32,
) -> Result<DictationOutcome, String> {
    run_transcription_hooked(engine, audio, params, duration_secs, &TranscribeHooks::default())
}

/// Like [`run_transcription`] but with live hooks (segment/progress/abort).
/// Only the Whisper backend honors the hooks; the CLI sidecars run batch.
pub fn run_transcription_hooked(
    engine: &Arc<Mutex<Option<TranscriptionEngine>>>,
    audio: &[f32],
    params: &DictationParams,
    duration_secs: f32,
    hooks: &TranscribeHooks,
) -> Result<DictationOutcome, String> {
    if params.backend == "cohere" {
        if !crate::transcription::cohere::cohere_ready() {
            return Err("Cohere is not set up. Go to Settings → Model to download the runtime and model.".to_string());
        }
        match crate::transcription::cohere::transcribe_via_cli(audio, params.language_code.as_deref()) {
            Ok(r) => Ok(build_outcome(r, params, duration_secs)),
            Err(e) => Err(format!("{}", e)),
        }
    } else if params.backend == "qwen" {
        if !crate::transcription::qwen::qwen_ready() {
            return Err("Qwen3-ASR is not set up. Go to Settings → Model to download the runtime and model.".to_string());
        }
        match crate::transcription::qwen::transcribe_via_cli(audio, params.language_code.as_deref()) {
            Ok(r) => Ok(build_outcome(r, params, duration_secs)),
            Err(e) => Err(format!("{}", e)),
        }
    } else {
        match engine.lock() {
            Ok(guard) => {
                if let Some(eng) = guard.as_ref() {
                    match eng.transcribe_with_hooks(
                        audio,
                        params.language_code.as_deref(),
                        params.n_threads,
                        params.translate,
                        params.beam_size,
                        params.temperature,
                        params.initial_prompt.as_deref(),
                        hooks,
                    ) {
                        Ok(r) => Ok(build_outcome(r, params, duration_secs)),
                        Err(e) => Err(format!("Transcription failed: {}", e)),
                    }
                } else {
                    Err("No model loaded".to_string())
                }
            }
            Err(e) => Err(format!("Lock error: {}", e)),
        }
    }
}

fn build_outcome(result: TranscriptionResult, params: &DictationParams, duration_secs: f32) -> DictationOutcome {
    let confidence = result.average_confidence.unwrap_or(0.0);
    let segments: Vec<(i64, i64, String)> = result.segments.iter()
        .map(|s| (s.start_ms.unwrap_or(0), s.end_ms.unwrap_or(0), s.text.clone()))
        .collect();
    let sanitized = postprocess::sanitize_transcript(&result.text, Some(confidence));
    // Apply personal-dictionary "heard → correct" replacements on the raw ASR
    // text before mode formatting (LLM variants are left untouched).
    let corrected = if params.replacements.is_empty() {
        sanitized
    } else {
        postprocess::apply_dictionary_replacements(&sanitized, &params.replacements)
    };
    let cleaned_text = apply_mode(&corrected, params.mode);
    DictationOutcome {
        raw_text: result.text,
        cleaned_text,
        confidence,
        segments,
        detected_language: result.detected_language,
        duration_secs,
    }
}

/// Shared recording + transcription controller, owned by the `Application` and
/// `Rc`-shared with each UI. Lives entirely on the glib main thread.
pub struct RecordingController {
    audio: Arc<Mutex<AudioCapture>>,
    engine: Arc<Mutex<Option<TranscriptionEngine>>>,
    model_catalog: Arc<ModelCatalog>,
    owner: Cell<RecordingOwner>,
}

impl RecordingController {
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            audio: Arc::new(Mutex::new(AudioCapture::new())),
            engine: Arc::new(Mutex::new(None)),
            model_catalog: Arc::new(ModelCatalog::new()),
            owner: Cell::new(RecordingOwner::None),
        })
    }

    /// The shared audio capture handle (same instance every caller sees).
    pub fn audio_arc(&self) -> Arc<Mutex<AudioCapture>> {
        self.audio.clone()
    }

    /// The shared Whisper engine slot.
    pub fn engine_arc(&self) -> Arc<Mutex<Option<TranscriptionEngine>>> {
        self.engine.clone()
    }

    /// The shared model catalog.
    pub fn model_catalog_arc(&self) -> Arc<ModelCatalog> {
        self.model_catalog.clone()
    }

    // --- recording ownership -------------------------------------------------

    pub fn owner(&self) -> RecordingOwner {
        self.owner.get()
    }

    pub fn is_recording(&self) -> bool {
        self.owner.get() != RecordingOwner::None
    }

    /// Acquire the recording slot for `owner`. Prevents two cpal streams at once
    /// (main window vs. global shortcut). Self-healing: the real source of truth
    /// is whether audio is actually capturing, so if nothing is recording the
    /// slot is free even when a previous owner wasn't released — otherwise a
    /// missed `release()` (or a panic) would wedge recording forever.
    pub fn try_acquire(&self, owner: RecordingOwner) -> bool {
        let busy = self.state() != RecordingState::Idle
            && self.owner.get() != RecordingOwner::None;
        if busy {
            return self.owner.get() == owner;
        }
        self.owner.set(owner);
        true
    }

    pub fn release(&self) {
        self.owner.set(RecordingOwner::None);
    }

    // --- capture (main thread only: cpal Stream is !Send) --------------------

    /// Start capturing from `device` (or the default), wiring `waveform_tx` for
    /// UI visualization.
    pub fn start(
        &self,
        device: Option<&str>,
        waveform_tx: async_channel::Sender<Vec<f32>>,
    ) -> AppResult<()> {
        let mut cap = self.audio.lock().unwrap_or_else(|e| e.into_inner());
        cap.set_waveform_sender(waveform_tx);
        cap.start_recording(device)
    }

    /// Stop capturing and return the recorded mono 16kHz audio.
    pub fn stop(&self) -> AppResult<Vec<f32>> {
        let mut cap = self.audio.lock().unwrap_or_else(|e| e.into_inner());
        cap.stop_recording()
    }

    /// Stop capturing and discard the audio.
    pub fn cancel(&self) {
        let mut cap = self.audio.lock().unwrap_or_else(|e| e.into_inner());
        let _ = cap.stop_recording();
    }

    pub fn pause(&self) {
        if let Ok(mut cap) = self.audio.lock() {
            cap.pause();
        }
    }

    pub fn resume(&self) {
        if let Ok(mut cap) = self.audio.lock() {
            cap.resume();
        }
    }

    pub fn state(&self) -> RecordingState {
        self.audio.lock()
            .map(|c| c.state())
            .unwrap_or(RecordingState::Idle)
    }

    pub fn recording_duration_secs(&self) -> f32 {
        self.audio.lock()
            .map(|c| c.recording_duration_secs())
            .unwrap_or(0.0)
    }

    /// Non-destructive snapshot of the audio captured so far (mono 16 kHz), for
    /// live transcription while recording continues.
    pub fn live_snapshot(&self) -> Vec<f32> {
        self.audio.lock()
            .map(|c| c.snapshot_mono_16khz())
            .unwrap_or_default()
    }

    // --- transcription -------------------------------------------------------

    /// Spawn transcription on a worker thread; the returned receiver yields the
    /// outcome once, to be awaited via `glib::spawn_future_local`.
    pub fn transcribe_async(
        &self,
        audio: Vec<f32>,
        params: DictationParams,
        duration_secs: f32,
    ) -> async_channel::Receiver<Result<DictationOutcome, String>> {
        let (sender, receiver) = async_channel::bounded(1);
        let engine = self.engine.clone();
        std::thread::spawn(move || {
            let result = run_transcription(&engine, &audio, &params, duration_secs);
            let _ = sender.send_blocking(result);
        });
        receiver
    }

    /// Like [`transcribe_async`] but with live streaming: the returned handles
    /// carry incremental segments + real progress (Whisper only) and an abort
    /// flag. The UI drains `segments`/`progress` while awaiting `outcome`.
    pub fn transcribe_async_streaming(
        &self,
        audio: Vec<f32>,
        params: DictationParams,
        duration_secs: f32,
    ) -> StreamingTranscription {
        let (out_tx, out_rx) = async_channel::bounded(1);
        let (seg_tx, seg_rx) = async_channel::unbounded::<SegmentEvent>();
        let (prog_tx, prog_rx) = async_channel::unbounded::<i32>();
        let abort = std::sync::Arc::new(AtomicBool::new(false));
        let engine = self.engine.clone();
        let hooks = TranscribeHooks {
            segment_tx: Some(seg_tx),
            progress_tx: Some(prog_tx),
            abort: Some(abort.clone()),
        };
        std::thread::spawn(move || {
            let result = run_transcription_hooked(&engine, &audio, &params, duration_secs, &hooks);
            let _ = out_tx.send_blocking(result);
        });
        StreamingTranscription { outcome: out_rx, segments: seg_rx, progress: prog_rx, abort }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_round_trips_through_config_str() {
        for s in ["plain", "message", "email", "note", "code_prompt"] {
            let mode = DictationMode::from_config_str(s);
            assert_eq!(mode.as_config_str(), s);
        }
    }

    #[test]
    fn unknown_mode_falls_back_to_plain() {
        assert_eq!(DictationMode::from_config_str("nonsense"), DictationMode::Plain);
    }

    #[test]
    fn plain_mode_is_identity() {
        let sanitized = postprocess::sanitize_transcript("hello world. this is a test", Some(0.9));
        assert_eq!(apply_mode(&sanitized, DictationMode::Plain), sanitized);
    }

    #[test]
    fn message_mode_adds_terminal_punctuation_and_single_line() {
        assert_eq!(apply_mode("Hello world", DictationMode::Message), "Hello world.");
        assert_eq!(apply_mode("Hi there.", DictationMode::Message), "Hi there.");
        assert_eq!(apply_mode("line one\nline two", DictationMode::Message), "line one line two.");
    }

    #[test]
    fn email_mode_wraps_with_greeting_and_signoff() {
        let out = apply_mode("Please review the attached document.", DictationMode::Email);
        assert!(out.starts_with("Hi,\n\n"));
        assert!(out.contains("Please review the attached document."));
        assert!(out.trim_end().ends_with("Best regards,"));
    }

    #[test]
    fn note_mode_bullets_each_sentence() {
        let out = apply_mode("Buy milk. Call the bank. Finish the report.", DictationMode::Note);
        assert_eq!(out, "• Buy milk.\n• Call the bank.\n• Finish the report.");
    }

    #[test]
    fn code_prompt_mode_strips_fillers() {
        let out = apply_mode("Um, add a uh retry loop", DictationMode::CodePrompt);
        assert_eq!(out, "Add a retry loop.");
    }

    #[test]
    fn empty_input_stays_empty_for_all_modes() {
        for m in [DictationMode::Plain, DictationMode::Message, DictationMode::Email, DictationMode::Note, DictationMode::CodePrompt] {
            assert_eq!(apply_mode("   ", m), "");
        }
    }
}
