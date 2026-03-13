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
    pub auto_detect_language: bool,

    /// Default directory for saving transcriptions.
    pub save_directory: Option<String>,

    /// Number of threads for whisper inference (0 = auto).
    pub n_threads: u32,

    /// UI theme preference ("system", "light", "dark").
    pub theme: Option<String>,

    /// Custom directory for storing models (None = default XDG data dir).
    pub model_directory: Option<String>,

    /// Initial prompt / custom vocabulary for Whisper (helps with domain-specific terms).
    pub initial_prompt: Option<String>,

    /// Whether to prefer quantized (q5) model variants over full models.
    pub use_quantized: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            first_run: true,
            selected_model: "base".to_string(),
            selected_microphone: None,
            use_gpu: false,
            cpu_fallback: true,
            language: Some("en".to_string()),
            auto_detect_language: false,
            save_directory: None,
            n_threads: 0,
            theme: None,
            model_directory: None,
            initial_prompt: None,
            use_quantized: false,
        }
    }
}

impl AppConfig {
    /// Load configuration from disk, or create defaults.
    pub fn load() -> Self {
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

    /// Save configuration to disk.
    pub fn save(&self) {
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
