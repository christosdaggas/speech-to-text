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

    /// HuggingFace token for downloading the Cohere Transcribe model (gated).
    #[serde(default)]
    pub cohere_hf_token: Option<String>,

    /// Whether the floating mini panel + global dictation shortcut are enabled.
    #[serde(default = "default_true")]
    pub mini_panel_enabled: bool,

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
            global_shortcut: default_global_shortcut(),
            auto_paste: true,
            dictation_mode: default_dictation_mode(),
            start_hidden: false,
            translate_to_english: false,
        }
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
                        Ok(config) => {
                            info!("Loaded configuration from {:?}", config_path);
                            return config;
                        }
                        Err(e) => {
                            warn!("Failed to parse config file: {}, using defaults", e);
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

        if let Some(parent) = config_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                warn!("Failed to create config directory: {}", e);
                return;
            }

            // Set directory permissions (Unix only)
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
            }
        }

        match serde_json::to_string_pretty(self) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&config_path, &json) {
                    warn!("Failed to write config file: {}", e);
                    return;
                }

                // Set file permissions (Unix only)
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600));
                }

                info!("Saved configuration to {:?}", config_path);
            }
            Err(e) => {
                warn!("Failed to serialize config: {}", e);
            }
        }
    }

    /// Path to the configuration file.
    pub fn config_file_path() -> PathBuf {
        Self::config_dir().join("config.json")
    }

    /// Configuration directory (~/.config/speech-to-text/).
    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("/tmp"))
                    .join(".config")
            })
            .join("speech-to-text")
    }

    /// Data directory for models (~/.local/share/speech-to-text/).
    pub fn data_dir() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("/tmp"))
                    .join(".local/share")
            })
            .join("speech-to-text")
    }

    /// Directory where Whisper models are stored.
    /// Uses custom model_directory if set, otherwise default XDG data path.
    /// Accepts an optional config reference to avoid re-reading disk.
    pub fn models_dir_with_config(config: Option<&AppConfig>) -> PathBuf {
        let custom = config
            .and_then(|c| c.model_directory.as_ref())
            .or_else(|| {
                // Fallback: read from disk only if no config provided
                None
            });
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
        self.inner.read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn update<F>(&self, f: F)
    where
        F: FnOnce(&mut AppConfig),
    {
        let mut config = self.inner.write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&mut config);
        config.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_round_trip_preserves_performance_fields() {
        let mut config = AppConfig::default();
        config.use_gpu = true;
        config.n_threads = 6;
        config.beam_size = 3;
        config.temperature = 0.4;
        config.selected_microphone = Some("USB Mic".into());
        config.selected_model = "small-q5_1".into();

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
        assert_eq!(config.global_shortcut, "<Ctrl><Alt>space");
        assert!(config.auto_paste); // type-into-app on by default
        assert_eq!(config.dictation_mode, "plain");
        assert!(!config.start_hidden);
        assert!(!config.translate_to_english);
    }

    #[test]
    fn serde_round_trip_preserves_mini_panel_fields() {
        let mut config = AppConfig::default();
        config.mini_panel_enabled = false;
        config.global_shortcut = "<Super>d".into();
        config.auto_paste = false;
        config.dictation_mode = "email".into();

        let json = serde_json::to_string(&config).expect("serialize");
        let restored: AppConfig = serde_json::from_str(&json).expect("deserialize");

        assert!(!restored.mini_panel_enabled);
        assert_eq!(restored.global_shortcut, "<Super>d");
        assert!(!restored.auto_paste);
        assert_eq!(restored.dictation_mode, "email");
    }
}
