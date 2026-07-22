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
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::{Arc, Mutex};

use crate::audio::buffer::RawAudioSnapshot;
use crate::audio::capture::RecordingState;
use crate::audio::AudioCapture;
use crate::error::AppResult;
use crate::transcription::engine::TranscriptionResult;
use crate::transcription::{postprocess, ModelCatalog, TranscriptionEngine};

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
        DictationMode::CodePrompt => ensure_terminal_punctuation(&capitalize_first(
            &strip_fillers(&collapse_whitespace(text)),
        )),
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
    sentences
        .iter()
        .map(|s| format!("• {s}"))
        .collect::<Vec<_>>()
        .join("\n")
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
            let core = w
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase();
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
    if params.backend == "cohere" {
        if !crate::transcription::cohere::cohere_ready() {
            return Err(
                "Cohere is not set up. Go to Settings → Model to download the runtime and model."
                    .to_string(),
            );
        }
        match crate::transcription::cohere::transcribe_via_cli(
            audio,
            params.language_code.as_deref(),
        ) {
            Ok(r) => Ok(build_outcome(r, params, duration_secs)),
            Err(e) => Err(format!("{}", e)),
        }
    } else if params.backend == "qwen" {
        if !crate::transcription::qwen::qwen_ready() {
            return Err("Qwen3-ASR is not set up. Go to Settings → Model to download the runtime and model.".to_string());
        }
        match crate::transcription::qwen::transcribe_via_cli(audio, params.language_code.as_deref())
        {
            Ok(r) => Ok(build_outcome(r, params, duration_secs)),
            Err(e) => Err(format!("{}", e)),
        }
    } else {
        let mut guard = match engine.lock() {
            Ok(g) => g,
            Err(e) => return Err(format!("Lock error: {}", e)),
        };

        // First attempt on the loaded engine (GPU if it was loaded with it).
        // Capture what we'd need for a CPU reload, then drop the borrow so we can
        // swap the engine in the shared slot below.
        let (first, gpu_reload) = {
            let Some(eng) = guard.as_ref() else {
                return Err("No model loaded".to_string());
            };
            let reload = eng
                .uses_gpu()
                .then(|| (eng.model_path().to_path_buf(), eng.model_id().to_string()));
            let res = eng.transcribe(
                audio,
                params.language_code.as_deref(),
                params.n_threads,
                params.translate,
                params.beam_size,
                params.temperature,
                params.initial_prompt.as_deref(),
            );
            (res, reload)
        };

        match first {
            Ok(r) => Ok(build_outcome(r, params, duration_secs)),
            Err(e) => {
                // A GPU encode can fail intermittently on some drivers (whisper
                // returns "failed to encode" / -6), especially under GPU-memory
                // pressure. Reload the model on CPU once, swap it into the shared
                // slot so later transcriptions skip the doomed GPU path, and retry
                // — the user gets a transcript instead of an error. Non-GPU
                // failures are real, so surface them.
                let Some((model_path, model_id)) = gpu_reload else {
                    return Err(format!("Transcription failed: {}", e));
                };
                tracing::warn!(
                    "GPU transcription failed ({e}); reloading '{model_id}' on CPU and retrying"
                );
                match TranscriptionEngine::load_model_with_gpu(&model_path, &model_id, false) {
                    Ok(cpu_eng) => {
                        let retry = cpu_eng.transcribe(
                            audio,
                            params.language_code.as_deref(),
                            params.n_threads,
                            params.translate,
                            params.beam_size,
                            params.temperature,
                            params.initial_prompt.as_deref(),
                        );
                        *guard = Some(cpu_eng);
                        retry
                            .map(|r| build_outcome(r, params, duration_secs))
                            .map_err(|e2| format!("Transcription failed: {}", e2))
                    }
                    Err(e2) => Err(format!("Transcription failed (GPU then CPU): {}", e2)),
                }
            }
        }
    }
}

/// Ensure a Whisper engine is loaded in the shared slot, loading the configured
/// model on first use. No-op if an engine is already loaded — so the API server
/// and the GUI share the one instance (whoever loads first wins). Honors
/// `config.use_gpu` with a CPU fallback when `cpu_fallback` is set, mirroring the
/// main window's loader. Blocking: call from a worker thread, not the UI thread.
///
/// Only relevant for the Whisper backend; Cohere/Qwen run via their CLIs and
/// need no preloaded engine.
pub fn ensure_engine_loaded(
    engine: &Arc<Mutex<Option<TranscriptionEngine>>>,
    config: &crate::config::AppConfig,
) -> Result<(), String> {
    if engine
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .is_some_and(|loaded| loaded.model_id() == config.selected_model)
    {
        return Ok(());
    }

    let model_id = config.selected_model.clone();
    if !ModelCatalog::is_downloaded(&model_id) {
        return Err(format!(
            "Model '{}' is not downloaded. Open the app and download it in Settings → Model.",
            model_id
        ));
    }
    let model_path = ModelCatalog::model_path(&model_id);

    let loaded =
        match TranscriptionEngine::load_model_with_gpu(&model_path, &model_id, config.use_gpu) {
            Ok(e) => e,
            Err(e) if config.use_gpu && config.cpu_fallback => {
                tracing::warn!("API: GPU model load failed ({e}); retrying on CPU");
                TranscriptionEngine::load_model_with_gpu(&model_path, &model_id, false)
                    .map_err(|e2| format!("Failed to load model on GPU and CPU: {e2}"))?
            }
            Err(e) => return Err(format!("Failed to load model: {e}")),
        };

    // Re-check under the lock: another thread may have loaded it meanwhile.
    let mut guard = engine.lock().unwrap_or_else(|e| e.into_inner());
    if guard
        .as_ref()
        .is_none_or(|engine| engine.model_id() != config.selected_model)
    {
        *guard = Some(loaded);
    }
    Ok(())
}

fn build_outcome(
    result: TranscriptionResult,
    params: &DictationParams,
    duration_secs: f32,
) -> DictationOutcome {
    let confidence = result.average_confidence.unwrap_or(0.0);
    let segments: Vec<(i64, i64, String)> = result
        .segments
        .iter()
        .map(|s| {
            (
                s.start_ms.unwrap_or(0),
                s.end_ms.unwrap_or(0),
                s.text.clone(),
            )
        })
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
    inference_tx: SyncSender<InferenceJob>,
}

enum InferenceAudio {
    Prepared(Vec<f32>),
    Raw(RawAudioSnapshot),
}

struct InferenceJob {
    audio: InferenceAudio,
    params: DictationParams,
    duration_secs: f32,
    reply: async_channel::Sender<Result<DictationOutcome, String>>,
}

impl RecordingController {
    pub fn new() -> Rc<Self> {
        let engine = Arc::new(Mutex::new(None));
        let (inference_tx, inference_rx) = std::sync::mpsc::sync_channel::<InferenceJob>(2);
        let worker_engine = engine.clone();
        std::thread::Builder::new()
            .name("gui-transcribe".into())
            .spawn(move || {
                while let Ok(job) = inference_rx.recv() {
                    let audio = match job.audio {
                        InferenceAudio::Prepared(audio) => audio,
                        InferenceAudio::Raw(snapshot) => snapshot.condition(),
                    };
                    let result = if audio.is_empty() {
                        Err(
                            "No clear speech detected — try speaking closer to the microphone"
                                .into(),
                        )
                    } else {
                        run_transcription(&worker_engine, &audio, &job.params, job.duration_secs)
                    };
                    let _ = job.reply.send_blocking(result);
                }
            })
            .expect("failed to start transcription worker");

        // `AudioCapture` holds a cpal `Stream`, which is `!Send`/`!Sync`, so this
        // `Arc` can never really cross a thread — clippy is right that it buys
        // nothing over an `Rc`. It stays an `Arc` anyway because the handle is
        // handed out by `audio_arc()` and stored as `Arc<Mutex<AudioCapture>>`
        // by the main window; the shared type has to match on both sides.
        // Capture itself is main-thread-only (see the module docs), so the
        // atomic refcount is simply unused, not unsound.
        #[allow(clippy::arc_with_non_send_sync)]
        let audio = Arc::new(Mutex::new(AudioCapture::new()));

        Rc::new(Self {
            audio,
            engine,
            model_catalog: Arc::new(ModelCatalog::new()),
            owner: Cell::new(RecordingOwner::None),
            inference_tx,
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
        let busy = self.state() != RecordingState::Idle && self.owner.get() != RecordingOwner::None;
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

    /// Stop capture and detach raw audio without conditioning it on the caller's
    /// thread. Use with [`Self::transcribe_snapshot_async`].
    pub fn stop_snapshot(&self) -> AppResult<RawAudioSnapshot> {
        let mut cap = self.audio.lock().unwrap_or_else(|e| e.into_inner());
        cap.stop_recording_snapshot()
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
        self.audio
            .lock()
            .map(|c| c.state())
            .unwrap_or(RecordingState::Idle)
    }

    pub fn recording_duration_secs(&self) -> f32 {
        self.audio
            .lock()
            .map(|c| c.recording_duration_secs())
            .unwrap_or(0.0)
    }

    /// Non-destructive snapshot of the audio captured so far (mono 16 kHz), for
    /// live transcription while recording continues.
    pub fn live_snapshot(&self, max_samples: usize) -> Vec<f32> {
        self.audio
            .lock()
            .map(|c| c.snapshot_mono_16khz(max_samples))
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
        self.enqueue_inference(InferenceJob {
            audio: InferenceAudio::Prepared(audio),
            params,
            duration_secs,
            reply: sender,
        });
        receiver
    }

    /// Condition detached raw audio and transcribe it entirely on a worker.
    pub fn transcribe_snapshot_async(
        &self,
        snapshot: RawAudioSnapshot,
        params: DictationParams,
        duration_secs: f32,
    ) -> async_channel::Receiver<Result<DictationOutcome, String>> {
        let (sender, receiver) = async_channel::bounded(1);
        self.enqueue_inference(InferenceJob {
            audio: InferenceAudio::Raw(snapshot),
            params,
            duration_secs,
            reply: sender,
        });
        receiver
    }

    fn enqueue_inference(&self, job: InferenceJob) {
        if let Err(error) = self.inference_tx.try_send(job) {
            let (job, message) = match error {
                TrySendError::Full(job) => (job, "Transcription queue is full; retry shortly."),
                TrySendError::Disconnected(job) => (job, "Transcription worker is unavailable."),
            };
            let _ = job.reply.try_send(Err(message.into()));
        }
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
        assert_eq!(
            DictationMode::from_config_str("nonsense"),
            DictationMode::Plain
        );
    }

    #[test]
    fn plain_mode_is_identity() {
        let sanitized = postprocess::sanitize_transcript("hello world. this is a test", Some(0.9));
        assert_eq!(apply_mode(&sanitized, DictationMode::Plain), sanitized);
    }

    #[test]
    fn message_mode_adds_terminal_punctuation_and_single_line() {
        assert_eq!(
            apply_mode("Hello world", DictationMode::Message),
            "Hello world."
        );
        assert_eq!(apply_mode("Hi there.", DictationMode::Message), "Hi there.");
        assert_eq!(
            apply_mode("line one\nline two", DictationMode::Message),
            "line one line two."
        );
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
        let out = apply_mode(
            "Buy milk. Call the bank. Finish the report.",
            DictationMode::Note,
        );
        assert_eq!(out, "• Buy milk.\n• Call the bank.\n• Finish the report.");
    }

    #[test]
    fn code_prompt_mode_strips_fillers() {
        let out = apply_mode("Um, add a uh retry loop", DictationMode::CodePrompt);
        assert_eq!(out, "Add a retry loop.");
    }

    #[test]
    fn empty_input_stays_empty_for_all_modes() {
        for m in [
            DictationMode::Plain,
            DictationMode::Message,
            DictationMode::Email,
            DictationMode::Note,
            DictationMode::CodePrompt,
        ] {
            assert_eq!(apply_mode("   ", m), "");
        }
    }
}
