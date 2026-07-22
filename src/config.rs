// Speech to Text - Configuration
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Application configuration with XDG-compliant paths.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::RwLock;
use tracing::{info, warn};

/// Application configuration persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Whether this is the first time the app has been run.
    pub first_run: bool,

    /// Active transcription backend ("whisper" or "cohere").
    #[serde(default = "default_backend")]
    pub backend: String,

    /// Selected Whisper model name (e.g., "base", "small", "medium").
    pub selected_model: String,

    /// Selected microphone device name (None = system default).
    pub selected_microphone: Option<String>,

    /// Whether to use GPU acceleration (when compiled with GPU feature).
    pub use_gpu: bool,

    /// Whether to fall back to CPU if GPU fails.
    pub cpu_fallback: bool,

    /// Transcription language (None = auto-detect).
    pub language: Option<String>,

    /// Whether to auto-detect language.
    #[serde(default = "default_true")]
    pub auto_detect_language: bool,

    /// Default directory for saving transcriptions.
    pub save_directory: Option<String>,

    /// Number of threads for whisper inference (0 = auto).
    pub n_threads: u32,

    /// Beam search width for whisper inference (1 = greedy).
    #[serde(default = "default_beam_size")]
    pub beam_size: u32,

    /// Sampling temperature for whisper inference (0.0 = deterministic).
    #[serde(default)]
    pub temperature: f32,

    /// UI theme preference ("system", "light", "dark").
    pub theme: Option<String>,

    /// Custom directory for storing models (None = default XDG data dir).
    pub model_directory: Option<String>,

    /// Initial prompt / custom vocabulary for Whisper (helps with domain-specific terms).
    pub initial_prompt: Option<String>,

    /// Whether to prefer quantized (q5) model variants over full models.
    pub use_quantized: bool,

    /// Legacy plaintext HuggingFace token. Kept only so old configs can be read
    /// and migrated into the system keyring on startup; `skip_serializing` means
    /// it is never written back to disk in plaintext again. New code stores the
    /// token via [`crate::secrets`].
    #[serde(default, skip_serializing)]
    pub cohere_hf_token: Option<String>,

    /// Whether the floating mini panel + global dictation shortcut are enabled.
    #[serde(default = "default_true")]
    pub mini_panel_enabled: bool,

    /// Best-effort "keep the mini panel above other windows". On GNOME/Mutter
    /// Wayland no client can force always-on-top, so this only re-raises the
    /// panel when it loses focus; it has full effect on wlroots compositors.
    #[serde(default)]
    pub mini_panel_always_on_top: bool,

    /// Preferred global shortcut trigger. This is only a *suggestion*: on
    /// GNOME/Wayland the desktop owns the real binding via the GlobalShortcuts
    /// portal, so the user confirms/changes it in Settings → Keyboard.
    #[serde(default = "default_global_shortcut")]
    pub global_shortcut: String,

    /// Whether to type the transcript into the focused app after dictation
    /// (via the RemoteDesktop portal). The clipboard is ALWAYS set regardless;
    /// this only controls the best-effort automated typing, which shows a
    /// one-time permission prompt on GNOME. On by default so the mini panel
    /// pastes into the app you're working in.
    #[serde(default = "default_true")]
    pub auto_paste: bool,

    /// Dictation output mode: "plain", "message", "email", "note", "code_prompt".
    #[serde(default = "default_dictation_mode")]
    pub dictation_mode: String,

    /// Start the app hidden (no main window), living in the system tray with the
    /// global shortcut active.
    #[serde(default)]
    pub start_hidden: bool,

    /// Translate the transcript to English (Whisper's built-in translate task).
    /// Applies to both the main window and the mini panel / global dictation.
    #[serde(default)]
    pub translate_to_english: bool,

    // ── LLM integration ──────────────────────────────────────────────
    /// Whether the LLM "Improve with AI" integration is enabled.
    #[serde(default)]
    pub llm_enabled: bool,

    /// Set once the user has acknowledged the privacy consent for sending
    /// transcript text to the configured LLM endpoint. Prevents re-prompting.
    #[serde(default)]
    pub llm_consent_given: bool,

    /// Normalized scheme/host/port for which consent was granted. A changed
    /// endpoint invalidates consent even when the legacy boolean remains true.
    #[serde(default)]
    pub llm_consent_endpoint: Option<String>,

    /// OpenAI-compatible base URL (e.g. http://localhost:1234/v1).
    #[serde(default = "default_llm_url")]
    pub llm_api_url: String,

    /// Selected model id (from the server's /models list, or typed manually).
    #[serde(default)]
    pub llm_model: String,

    /// Default sampling temperature for LLM requests.
    #[serde(default = "default_llm_temperature")]
    pub llm_temperature: f32,

    /// Auto-improve the transcript after every dictation (uses the active preset).
    #[serde(default)]
    pub llm_auto_apply: bool,

    /// Automatically summarize long transcripts. Separate from auto-improve so
    /// enabling an LLM connection alone never triggers background data sends.
    #[serde(default)]
    pub llm_auto_summary: bool,

    /// Index of the active preset in `llm_presets`.
    #[serde(default)]
    pub llm_active_preset: usize,

    /// Editable prompt presets.
    #[serde(default = "default_llm_presets")]
    pub llm_presets: Vec<LlmPreset>,

    /// Preferred global shortcut for "transform selection/clipboard with AI".
    #[serde(default = "default_selection_shortcut")]
    pub llm_selection_shortcut: String,

    /// Whether the transform-selection global shortcut is registered.
    #[serde(default)]
    pub llm_selection_enabled: bool,

    /// Selected Qwen3-ASR model size ("0.6B" = small/fast, "1.7B" = full).
    #[serde(default = "default_qwen_model_size")]
    pub qwen_model_size: String,

    /// Whether to check GitHub for a newer release at startup. This is the only
    /// automatic network request the app makes on its own (it leaks IP/timing/
    /// version to GitHub), so it is exposed as a setting. Defaults to on to
    /// preserve existing behaviour; users can disable it in Settings.
    #[serde(default = "default_true")]
    pub update_check_enabled: bool,

    /// Personal-dictionary vocabulary hints (names, jargon, acronyms). Fed into
    /// Whisper's initial prompt to bias recognition toward these terms.
    #[serde(default)]
    pub dictionary_terms: Vec<String>,

    /// Personal-dictionary "heard → correct" replacement pairs, applied to the
    /// transcript after recognition (e.g. consistent spelling of names).
    #[serde(default)]
    pub dictionary_replacements: Vec<DictReplacement>,

    /// Master switch for the personal dictionary (terms + replacements).
    #[serde(default = "default_true")]
    pub dictionary_enabled: bool,

    /// Show transcription text live as it decodes (Whisper only). Off by default;
    /// the final result is always the full-context decode.
    #[serde(default)]
    pub live_transcription: bool,

    // ── Local HTTP API server ────────────────────────────────────────
    /// Whether the local HTTP API server is enabled. Off by default; when on,
    /// the server binds 127.0.0.1 only so other local apps can POST audio for
    /// transcription/translation.
    #[serde(default)]
    pub api_server_enabled: bool,

    /// TCP port for the local API server (bound on 127.0.0.1 only).
    #[serde(default = "default_api_port")]
    pub api_server_port: u16,

    /// Require an `Authorization: Bearer <token>` on API requests. The token
    /// itself lives in the system keyring, never in this file. On by default.
    #[serde(default = "default_true")]
    pub api_token_enabled: bool,
}

/// One personal-dictionary replacement rule: replace `from` with `to`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DictReplacement {
    pub from: String,
    pub to: String,
    /// Only replace whole words (token must be bounded by non-alphanumerics).
    #[serde(default)]
    pub whole_word: bool,
    /// Match case-sensitively (default: case-insensitive).
    #[serde(default)]
    pub case_sensitive: bool,
}

/// An editable LLM prompt preset (with optional per-preset overrides).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmPreset {
    pub name: String,
    pub prompt: String,
    /// Per-preset model override (None = use the global model).
    #[serde(default)]
    pub model: Option<String>,
    /// Per-preset temperature override (None = use the global temperature).
    #[serde(default)]
    pub temperature: Option<f32>,
    /// When `Some(lang)`, this is a translate preset; the prompt is generated.
    #[serde(default)]
    pub translate_to: Option<String>,
}

impl LlmPreset {
    /// The effective system prompt. For translate presets the editable `prompt`
    /// is used as a template with `{lang}` substituted from `translate_to`
    /// (falling back to a built-in template when the prompt is empty).
    pub fn system_prompt(&self) -> String {
        if let Some(lang) = self.translate_to.as_deref() {
            let base = if self.prompt.trim().is_empty() {
                "Translate the following text to {lang}. Reply with only the translation, no notes."
            } else {
                self.prompt.as_str()
            };
            base.replace("{lang}", lang)
        } else {
            self.prompt.clone()
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            first_run: true,
            backend: "whisper".to_string(),
            selected_model: "base".to_string(),
            selected_microphone: None,
            use_gpu: false,
            cpu_fallback: true,
            language: Some("en".to_string()),
            auto_detect_language: true,
            save_directory: None,
            n_threads: 0,
            beam_size: 5,
            temperature: 0.0,
            theme: None,
            model_directory: None,
            initial_prompt: None,
            use_quantized: false,
            cohere_hf_token: None,
            mini_panel_enabled: true,
            mini_panel_always_on_top: false,
            global_shortcut: default_global_shortcut(),
            // New installs default to clipboard-only. Auto-typing into the focused
            // app needs the RemoteDesktop portal permission, so it is opt-in (with
            // a consent prompt on enable). Existing users keep their saved setting:
            // the field is already present in their config, so this default does
            // not override it, and configs predating the field keep auto-paste on
            // via `default_true` (their prior behaviour).
            auto_paste: false,
            dictation_mode: default_dictation_mode(),
            start_hidden: false,
            translate_to_english: false,
            llm_enabled: false,
            llm_consent_given: false,
            llm_consent_endpoint: None,
            llm_api_url: default_llm_url(),
            llm_model: String::new(),
            llm_temperature: default_llm_temperature(),
            llm_auto_apply: false,
            llm_auto_summary: false,
            llm_active_preset: 0,
            llm_presets: default_llm_presets(),
            llm_selection_shortcut: default_selection_shortcut(),
            llm_selection_enabled: false,
            qwen_model_size: default_qwen_model_size(),
            update_check_enabled: true,
            dictionary_terms: Vec::new(),
            dictionary_replacements: Vec::new(),
            dictionary_enabled: true,
            live_transcription: false,
            api_server_enabled: false,
            api_server_port: default_api_port(),
            api_token_enabled: true,
        }
    }
}

impl AppConfig {
    /// The Whisper initial prompt to use, merging the user's free-text
    /// `initial_prompt` with the personal-dictionary terms (when enabled). The
    /// result is capped to keep within Whisper's ~224 prompt-token budget
    /// (roughly 800 characters); extra terms are dropped.
    pub fn effective_initial_prompt(&self) -> Option<String> {
        // ~3.5 chars/token × 224 tokens, with headroom for the user's own prompt.
        const MAX_PROMPT_CHARS: usize = 800;
        let mut parts: Vec<String> = Vec::new();
        if let Some(p) = self.initial_prompt.as_ref() {
            let p = p.trim();
            if !p.is_empty() {
                parts.push(p.to_string());
            }
        }
        if self.dictionary_enabled {
            let terms: Vec<&str> = self
                .dictionary_terms
                .iter()
                .map(|t| t.trim())
                .filter(|t| !t.is_empty())
                .collect();
            if !terms.is_empty() {
                parts.push(terms.join(", "));
            }
        }
        if parts.is_empty() {
            return None;
        }
        let mut combined = parts.join(" ");
        if combined.chars().count() > MAX_PROMPT_CHARS {
            combined = combined.chars().take(MAX_PROMPT_CHARS).collect();
            tracing::warn!(
                "Initial prompt + dictionary terms exceeded the prompt budget; truncated."
            );
        }
        Some(combined)
    }
}

/// Default backend for serde deserialization of old configs.
fn default_backend() -> String {
    "whisper".to_string()
}

/// Default to `true` for serde deserialization of old configs.
fn default_true() -> bool {
    true
}

/// Default beam size for serde deserialization of old configs.
fn default_beam_size() -> u32 {
    5
}

/// Default preferred global shortcut for serde deserialization of old configs.
fn default_global_shortcut() -> String {
    "<Ctrl><Alt>space".to_string()
}

/// Default dictation mode for serde deserialization of old configs.
fn default_dictation_mode() -> String {
    "plain".to_string()
}

/// Default OpenAI-compatible base URL (LM Studio's default).
fn default_llm_url() -> String {
    "http://localhost:1234/v1".to_string()
}

/// Default LLM sampling temperature.
fn default_llm_temperature() -> f32 {
    0.3
}

/// Default global shortcut for transform-selection.
fn default_selection_shortcut() -> String {
    "<Ctrl><Alt>i".to_string()
}

/// Default Qwen3-ASR model size (the small/fast 0.6B model).
fn default_qwen_model_size() -> String {
    "0.6B".to_string()
}

/// Default port for the local API server.
fn default_api_port() -> u16 {
    8756
}

/// Built-in prompt presets shipped with the app.
fn default_llm_presets() -> Vec<LlmPreset> {
    let p = |name: &str, prompt: &str| LlmPreset {
        name: name.to_string(),
        prompt: prompt.to_string(),
        model: None,
        temperature: None,
        translate_to: None,
    };
    vec![
        p("Clean up", "You are cleaning up a raw speech-to-text transcript that may contain mis-heard words, wrong homophones, missing or wrong punctuation, and run-on sentences. Fix grammar, punctuation, capitalization and spacing, and where a word or phrase clearly does not fit, infer what the speaker most likely meant and correct it. Preserve the original language, meaning and tone. Reply with only the corrected text."),
        p("Key points", "Rewrite the following into a short list of key points, in the same language. Reply with only the bullet points."),
        p("Formal", "Rewrite the following in a clear, formal tone, preserving the original language and meaning. Reply with only the rewritten text."),
        p("Short", "Rewrite the following to be as short as possible while keeping the essential meaning, in the same language. Reply with only the shortened text."),
        p("Long", "Expand the following into a more detailed, well-developed version, preserving the original language and intent without inventing facts. Reply with only the expanded text."),
        p("Professional email", "Rewrite the following as a clear, polite, professional email. Keep the original language. Reply with only the email body."),
        p("Summary (bullets)", "Summarize the following into concise bullet points, in the same language. Reply with only the summary."),
        LlmPreset { name: "Translate".into(), prompt: "Translate the following text to {lang}. Reply with only the translation, no notes.".into(), model: None, temperature: None, translate_to: Some("English".into()) },
        p("Code prompt", "Turn the following into a concise, well-structured coding instruction. Remove filler words. Reply with only the instruction."),
    ]
}

/// Process-wide cache so the many `AppConfig::load()` call sites don't re-read
/// and re-parse the JSON from disk on every access (startup did this ~50 times).
/// `save()` keeps the cache in sync, so reads always reflect the latest state.
static CONFIG_CACHE: RwLock<Option<AppConfig>> = RwLock::new(None);

impl AppConfig {
    /// Load configuration, returning the cached copy if available.
    pub fn load() -> Self {
        if let Some(cached) = CONFIG_CACHE
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            return cached;
        }

        let config = Self::load_from_disk();
        *CONFIG_CACHE.write().unwrap_or_else(|e| e.into_inner()) = Some(config.clone());
        config
    }

    /// Read and parse the configuration from disk, falling back to defaults.
    fn load_from_disk() -> Self {
        let config_path = Self::config_file_path();

        if config_path.exists() {
            match std::fs::read_to_string(&config_path) {
                Ok(contents) => {
                    match serde_json::from_str::<AppConfig>(&contents) {
                        Ok(mut config) => {
                            info!("Loaded configuration from {:?}", config_path);
                            // One-time migration of the built-in prompt presets to
                            // their newer defaults (only if still unedited).
                            if config.migrate_builtin_presets() {
                                if let Ok(json) = serde_json::to_string_pretty(&config) {
                                    let _ =
                                        crate::fsio::write_private(&config_path, json.as_bytes());
                                }
                            }
                            return config;
                        }
                        Err(e) => {
                            warn!("Failed to parse config file: {}, using defaults", e);
                            let backup = config_path.with_extension(format!(
                                "json.corrupt-{}",
                                chrono::Utc::now().timestamp()
                            ));
                            if let Err(rename_error) = std::fs::rename(&config_path, &backup) {
                                warn!("Failed to preserve corrupt config: {}", rename_error);
                            } else {
                                warn!("Preserved corrupt config at {:?}", backup);
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to read config file: {}, using defaults", e);
                }
            }
        }

        let config = Self::default();
        config.save();
        config
    }

    /// Save configuration to disk and refresh the in-memory cache.
    pub fn save(&self) {
        // Keep the cache consistent with the latest intended state.
        *CONFIG_CACHE.write().unwrap_or_else(|e| e.into_inner()) = Some(self.clone());
        let config_path = Self::config_file_path();

        match serde_json::to_string_pretty(self) {
            Ok(json) => {
                // Atomic, private (0600) write into a 0700 directory — the config
                // can hold an endpoint URL / model name and should not be readable
                // by other local users, and a crash mid-write must not corrupt it.
                if let Err(e) = crate::fsio::write_private(&config_path, json.as_bytes()) {
                    warn!("Failed to write config file: {}", e);
                    return;
                }
                info!("Saved configuration to {:?}", config_path);
            }
            Err(e) => {
                warn!("Failed to serialize config: {}", e);
            }
        }
    }

    /// Update the built-in "Clean up" and "Translate" presets to their newer
    /// default prompts, but only when the user hasn't edited them (the stored
    /// prompt still matches the old default / is empty for Translate). Returns
    /// `true` if anything changed. This brings existing configs in line with the
    /// improved defaults without clobbering user customizations.
    fn migrate_builtin_presets(&mut self) -> bool {
        const OLD_CLEANUP: &str = "Clean up the following transcribed text: fix grammar, punctuation and capitalization. Keep the meaning and the original language. Reply with only the corrected text.";
        let defaults = default_llm_presets();
        let new_cleanup = defaults
            .iter()
            .find(|p| p.name == "Clean up")
            .map(|p| p.prompt.clone());
        let new_translate = defaults
            .iter()
            .find(|p| p.translate_to.is_some())
            .map(|p| p.prompt.clone());

        let mut changed = false;
        for p in self.llm_presets.iter_mut() {
            // "Clean up": replace the old stock prompt with the improved one.
            if p.name == "Clean up" && p.prompt == OLD_CLEANUP {
                if let Some(np) = new_cleanup.as_ref() {
                    p.prompt = np.clone();
                    changed = true;
                }
            }
            // Translate presets that have no editable prompt yet: seed the
            // editable "{lang}" template so it shows in the settings UI.
            if p.translate_to.is_some() && p.prompt.trim().is_empty() {
                if let Some(np) = new_translate.as_ref() {
                    p.prompt = np.clone();
                    changed = true;
                }
            }
        }

        // Append the newer one-tap "chip" presets (Key points / Formal / Short /
        // Long) to configs created before they existed, so the transform chips
        // include them. Appended (not inserted) to preserve existing indices.
        for name in ["Key points", "Formal", "Short", "Long"] {
            if !self.llm_presets.iter().any(|p| p.name == name) {
                if let Some(def) = defaults.iter().find(|p| p.name == name) {
                    self.llm_presets.push(def.clone());
                    changed = true;
                }
            }
        }
        changed
    }

    /// Path to the configuration file.
    pub fn config_file_path() -> PathBuf {
        Self::config_dir().join("config.json")
    }

    /// Configuration directory (~/.config/speech-to-text/).
    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join(".config")))
            .unwrap_or_else(Self::private_fallback_base)
            .join("speech-to-text")
    }

    /// Data directory for models (~/.local/share/speech-to-text/).
    pub fn data_dir() -> PathBuf {
        dirs::data_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join(".local/share")))
            .unwrap_or_else(Self::private_fallback_base)
            .join("speech-to-text")
    }

    /// Last-resort base directory when no XDG/HOME directory can be determined.
    /// Never `/tmp`: that is world-writable and predictable, so another local
    /// user could pre-create our paths and read transcripts/secrets. Prefer the
    /// per-user `$XDG_RUNTIME_DIR` (already mode 0700); otherwise a private
    /// directory under the current working directory. `fsio::write_private`
    /// further enforces 0700 on whatever directory we end up using.
    fn private_fallback_base() -> PathBuf {
        if let Some(rt) = std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
            return PathBuf::from(rt);
        }
        PathBuf::from(".speech-to-text")
    }

    /// Directory where Whisper models are stored.
    /// Uses custom model_directory if set, otherwise default XDG data path.
    /// Accepts an optional config reference to avoid re-reading disk.
    pub fn models_dir_with_config(config: Option<&AppConfig>) -> PathBuf {
        // No disk fallback: callers that care pass the config in.
        let custom = config.and_then(|c| c.model_directory.as_ref());
        if let Some(dir) = custom {
            PathBuf::from(dir)
        } else {
            Self::default_models_dir()
        }
    }

    /// Directory where Whisper models are stored.
    /// This reads the current config from disk — prefer `models_dir_with_config`
    /// when you already have a config reference.
    pub fn models_dir() -> PathBuf {
        let config = Self::load();
        if let Some(ref custom) = config.model_directory {
            PathBuf::from(custom)
        } else {
            Self::default_models_dir()
        }
    }

    /// Default directory for models (XDG data dir).
    pub fn default_models_dir() -> PathBuf {
        Self::data_dir().join("models")
    }

    /// Directory where transcription history is stored.
    pub fn history_dir() -> PathBuf {
        Self::data_dir().join("history")
    }

    /// Get the effective number of threads for whisper inference.
    pub fn effective_threads(&self) -> u32 {
        if self.n_threads == 0 {
            num_cpus::get() as u32
        } else {
            self.n_threads
        }
    }
}

/// Thread-safe configuration wrapper that auto-saves on mutation.
#[allow(dead_code)]
pub struct ConfigStore {
    inner: RwLock<AppConfig>,
}

#[allow(dead_code)]
impl ConfigStore {
    pub fn new(config: AppConfig) -> Self {
        Self {
            inner: RwLock::new(config),
        }
    }

    pub fn read(&self) -> AppConfig {
        self.inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn update<F>(&self, f: F)
    where
        F: FnOnce(&mut AppConfig),
    {
        let mut config = self
            .inner
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&mut config);
        config.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictionary_fields_round_trip_and_legacy_loads() {
        // Old config without the dictionary fields must still deserialize.
        let mut value = serde_json::to_value(AppConfig::default()).unwrap();
        value.as_object_mut().unwrap().remove("dictionary_terms");
        value
            .as_object_mut()
            .unwrap()
            .remove("dictionary_replacements");
        value.as_object_mut().unwrap().remove("dictionary_enabled");
        let raw = serde_json::to_string(&value).unwrap();
        let cfg: AppConfig = serde_json::from_str(&raw).expect("legacy deserialize");
        assert!(cfg.dictionary_enabled, "missing field defaults to enabled");
        assert!(cfg.dictionary_terms.is_empty());

        // Round-trip with content.
        let cfg = AppConfig {
            dictionary_terms: vec!["Kubernetes".into(), "Daggas".into()],
            dictionary_replacements: vec![DictReplacement {
                from: "cube".into(),
                to: "Kube".into(),
                whole_word: true,
                case_sensitive: false,
            }],
            ..AppConfig::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: AppConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.dictionary_terms.len(), 2);
        assert_eq!(back.dictionary_replacements[0].to, "Kube");
        assert!(back.dictionary_replacements[0].whole_word);
    }

    #[test]
    fn api_server_fields_round_trip_and_legacy_loads() {
        // Old config without the API fields must still deserialize to safe
        // defaults: server OFF, token required, default port.
        let mut value = serde_json::to_value(AppConfig::default()).unwrap();
        let obj = value.as_object_mut().unwrap();
        obj.remove("api_server_enabled");
        obj.remove("api_server_port");
        obj.remove("api_token_enabled");
        let raw = serde_json::to_string(&value).unwrap();
        let cfg: AppConfig = serde_json::from_str(&raw).expect("legacy deserialize");
        assert!(!cfg.api_server_enabled, "server is off by default");
        assert!(cfg.api_token_enabled, "token is required by default");
        assert_eq!(cfg.api_server_port, 8756);

        // Round-trip with content.
        let cfg = AppConfig {
            api_server_enabled: true,
            api_server_port: 9100,
            api_token_enabled: false,
            ..AppConfig::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: AppConfig = serde_json::from_str(&json).unwrap();
        assert!(back.api_server_enabled);
        assert_eq!(back.api_server_port, 9100);
        assert!(!back.api_token_enabled);
    }

    #[test]
    fn effective_initial_prompt_merges_and_caps() {
        // `mut` is kept: the dictionary is toggled off further down to re-check the merge.
        let mut cfg = AppConfig {
            initial_prompt: Some("My meeting notes.".into()),
            dictionary_terms: vec!["Kubernetes".into(), "Daggas".into()],
            ..AppConfig::default()
        };
        let p = cfg.effective_initial_prompt().unwrap();
        assert!(p.contains("My meeting notes."));
        assert!(p.contains("Kubernetes") && p.contains("Daggas"));

        // Disabling the dictionary drops the terms.
        cfg.dictionary_enabled = false;
        let p = cfg.effective_initial_prompt().unwrap();
        assert!(!p.contains("Kubernetes"));

        // No prompt + no terms → None.
        let empty = AppConfig {
            initial_prompt: None,
            dictionary_terms: vec![],
            ..AppConfig::default()
        };
        assert!(empty.effective_initial_prompt().is_none());

        // Cap is enforced.
        let cfg = AppConfig {
            initial_prompt: None,
            dictionary_terms: vec!["x".repeat(2000)],
            ..AppConfig::default()
        };
        assert!(cfg.effective_initial_prompt().unwrap().chars().count() <= 800);
    }

    #[test]
    fn legacy_token_is_read_but_never_serialized() {
        // An old config with a plaintext token must still parse (so we can
        // migrate it into the keyring) but must never be written back out.
        let mut value = serde_json::to_value(AppConfig::default()).unwrap();
        value["cohere_hf_token"] = serde_json::Value::String("hf_secretvalue".into());
        let raw = serde_json::to_string(&value).unwrap();

        let cfg: AppConfig = serde_json::from_str(&raw).expect("deserialize legacy");
        assert_eq!(cfg.cohere_hf_token.as_deref(), Some("hf_secretvalue"));

        let json = serde_json::to_string(&cfg).expect("serialize");
        assert!(
            !json.contains("cohere_hf_token") && !json.contains("hf_secretvalue"),
            "legacy plaintext token was re-serialized: {json}"
        );
    }

    #[test]
    fn new_install_defaults_are_privacy_preserving() {
        let cfg = AppConfig::default();
        assert!(!cfg.auto_paste, "auto-paste should be off for new installs");
        assert!(!cfg.llm_enabled);
        assert!(!cfg.llm_consent_given);
        assert!(cfg.llm_consent_endpoint.is_none());
        assert!(!cfg.llm_auto_summary);
        assert!(
            cfg.update_check_enabled,
            "update check defaults on (documented)"
        );
    }

    #[test]
    fn serde_round_trip_preserves_performance_fields() {
        let config = AppConfig {
            use_gpu: true,
            n_threads: 6,
            beam_size: 3,
            temperature: 0.4,
            selected_microphone: Some("USB Mic".into()),
            selected_model: "small-q5_1".into(),
            ..AppConfig::default()
        };

        let json = serde_json::to_string(&config).expect("serialize");
        let restored: AppConfig = serde_json::from_str(&json).expect("deserialize");

        assert!(restored.use_gpu);
        assert_eq!(restored.n_threads, 6);
        assert_eq!(restored.beam_size, 3);
        assert!((restored.temperature - 0.4).abs() < f32::EPSILON);
        assert_eq!(restored.selected_microphone.as_deref(), Some("USB Mic"));
        assert_eq!(restored.selected_model, "small-q5_1");
    }

    #[test]
    fn old_config_without_new_fields_gets_defaults() {
        // A config written before beam_size/temperature existed must still load.
        let legacy = r#"{
            "first_run": false,
            "selected_model": "base",
            "selected_microphone": null,
            "use_gpu": false,
            "cpu_fallback": true,
            "language": "en",
            "save_directory": null,
            "n_threads": 0,
            "theme": null,
            "model_directory": null,
            "initial_prompt": null,
            "use_quantized": false
        }"#;
        let config: AppConfig = serde_json::from_str(legacy).expect("legacy config should load");
        assert_eq!(config.beam_size, 5); // default_beam_size
        assert_eq!(config.temperature, 0.0);
        assert_eq!(config.backend, "whisper"); // default_backend
        assert!(config.auto_detect_language); // default_true
                                              // Mini panel fields must also default cleanly for pre-existing configs.
        assert!(config.mini_panel_enabled);
        assert!(!config.mini_panel_always_on_top);
        assert_eq!(config.global_shortcut, "<Ctrl><Alt>space");
        assert!(config.auto_paste); // type-into-app on by default
        assert_eq!(config.dictation_mode, "plain");
        assert!(!config.start_hidden);
        assert!(!config.translate_to_english);
        // LLM fields default cleanly for pre-existing configs.
        assert!(!config.llm_enabled);
        assert_eq!(config.llm_api_url, "http://localhost:1234/v1");
        assert_eq!(config.llm_model, "");
        assert!((config.llm_temperature - 0.3).abs() < f32::EPSILON);
        assert!(!config.llm_auto_apply);
        assert_eq!(config.llm_active_preset, 0);
        assert!(!config.llm_presets.is_empty()); // built-in presets
        assert!(!config.llm_selection_enabled);
    }

    #[test]
    fn serde_round_trip_preserves_mini_panel_fields() {
        let config = AppConfig {
            mini_panel_enabled: false,
            mini_panel_always_on_top: true,
            global_shortcut: "<Super>d".into(),
            auto_paste: false,
            dictation_mode: "email".into(),
            ..AppConfig::default()
        };

        let json = serde_json::to_string(&config).expect("serialize");
        let restored: AppConfig = serde_json::from_str(&json).expect("deserialize");

        assert!(!restored.mini_panel_enabled);
        assert!(restored.mini_panel_always_on_top);
        assert_eq!(restored.global_shortcut, "<Super>d");
        assert!(!restored.auto_paste);
        assert_eq!(restored.dictation_mode, "email");
    }

    #[test]
    fn serde_round_trip_preserves_llm_fields() {
        let config = AppConfig {
            llm_enabled: true,
            llm_api_url: "http://localhost:11434/v1".into(),
            llm_model: "llama3.1:8b".into(),
            llm_temperature: 0.7,
            llm_auto_apply: true,
            llm_active_preset: 2,
            llm_selection_enabled: true,
            llm_presets: vec![crate::config::LlmPreset {
                name: "T".into(),
                prompt: "Translate the following text to {lang}.".into(),
                model: Some("m".into()),
                temperature: Some(0.1),
                translate_to: Some("French".into()),
            }],
            ..AppConfig::default()
        };

        let json = serde_json::to_string(&config).expect("serialize");
        let restored: AppConfig = serde_json::from_str(&json).expect("deserialize");

        assert!(restored.llm_enabled);
        assert_eq!(restored.llm_api_url, "http://localhost:11434/v1");
        assert_eq!(restored.llm_model, "llama3.1:8b");
        assert!((restored.llm_temperature - 0.7).abs() < f32::EPSILON);
        assert!(restored.llm_auto_apply);
        assert_eq!(restored.llm_active_preset, 2);
        assert!(restored.llm_selection_enabled);
        assert_eq!(restored.llm_presets.len(), 1);
        assert_eq!(
            restored.llm_presets[0].translate_to.as_deref(),
            Some("French")
        );
        assert_eq!(restored.llm_presets[0].model.as_deref(), Some("m"));
        // Translate preset substitutes {lang} from translate_to into the prompt.
        assert!(restored.llm_presets[0].system_prompt().contains("French"));
        assert!(!restored.llm_presets[0].system_prompt().contains("{lang}"));
    }

    #[test]
    fn migration_updates_old_builtin_presets_only() {
        // `mut` is kept: migrate_builtin_presets() rewrites the presets in place below.
        let mut c = AppConfig {
            llm_presets: vec![
                // Old stock "Clean up" prompt.
                LlmPreset {
                    name: "Clean up".into(),
                    prompt: "Clean up the following transcribed text: fix grammar, punctuation and capitalization. Keep the meaning and the original language. Reply with only the corrected text.".into(),
                    model: None, temperature: None, translate_to: None,
                },
                // Old Translate preset with an empty (locked) prompt.
                LlmPreset {
                    name: "Translate".into(),
                    prompt: String::new(),
                    model: None, temperature: None, translate_to: Some("English".into()),
                },
                // A user-customized preset — must be left untouched.
                LlmPreset {
                    name: "Clean up".into(),
                    prompt: "my own edited prompt".into(),
                    model: None, temperature: None, translate_to: None,
                },
            ],
            ..AppConfig::default()
        };

        assert!(c.migrate_builtin_presets());
        assert!(c.llm_presets[0].prompt.contains("speech-to-text")); // improved Clean up
        assert!(c.llm_presets[1].prompt.contains("{lang}")); // translate template seeded
        assert_eq!(c.llm_presets[2].prompt, "my own edited prompt"); // untouched
                                                                     // Idempotent: a second run changes nothing.
        assert!(!c.migrate_builtin_presets());
    }
}
