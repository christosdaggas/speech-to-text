// Speech to Text - Application
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Main Application.
//!
//! GObject subclass for the Adwaita Application.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk4 as gtk;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use tracing::info;

use crate::audio::capture::RecordingState;
use crate::config::AppConfig;
use crate::i18n::gettext;
use crate::recording::{DictationOutcome, DictationParams, RecordingController, RecordingOwner};
use crate::ui::{MainWindow, MiniPanel, MiniPanelAction};
use crate::{APP_ID, APP_NAME, VERSION};

/// Set once at startup: true only when the process was launched with `--hidden`
/// (used by the autostart entry). A manual launch leaves this false so the main
/// window is always shown.
pub static LAUNCH_HIDDEN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// Global Tokio runtime for async operations (model downloads, etc.).
pub static TOKIO_RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();

/// Get or initialize the global Tokio runtime.
pub fn tokio_runtime() -> &'static tokio::runtime::Runtime {
    TOKIO_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("Failed to create Tokio runtime")
    })
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Application {
        pub config: RefCell<Option<Arc<AppConfig>>>,
        /// Shared recording + transcription controller (one cpal stream + one
        /// engine) used by the main window, the mini panel, and the global
        /// dictation shortcut.
        pub controller: RefCell<Option<Rc<RecordingController>>>,
        /// The single floating mini panel instance (created lazily, hidden when
        /// not in use).
        pub mini_panel: RefCell<Option<MiniPanel>>,
        /// Text of the most recent global dictation, for the panel's Copy/Paste.
        pub last_text: RefCell<String>,
        /// The current global-dictation result (raw transcript + AI variants) for
        /// the panel's transform chips and raw/polished selector.
        pub last_result_state: RefCell<Option<crate::ui::result_state::ResultState>>,
        /// Target text being edited by an in-progress Voice Edit (the spoken
        /// instruction is captured, then applied to this text).
        pub voice_edit_target: RefCell<Option<String>>,
        /// Whether the first `activate` has happened (so re-launch always shows
        /// the window even when `start_hidden` is set).
        pub started: std::cell::Cell<bool>,
        /// Keeps the application alive in the background (no window needed).
        /// Dropping this guard releases the hold, so it lives for the app's life.
        pub hold_guard: RefCell<Option<gio::ApplicationHoldGuard>>,
        /// The running local HTTP API server, when enabled. Dropping the handle
        /// stops the server and closes the port.
        pub api_server: RefCell<Option<crate::api::ApiServerHandle>>,
        /// Invalidates asynchronous API starts after disable/restart requests.
        pub api_start_generation: std::cell::Cell<u64>,
        /// Invalidates stale global-dictation, LLM, and auto-paste callbacks.
        pub dictation_generation: std::cell::Cell<u64>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Application {
        const NAME: &'static str = "SpeechToTextApplication";
        type Type = super::Application;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for Application {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.setup_actions();
            obj.set_accels_for_action("app.quit", &["<primary>q"]);
            obj.set_accels_for_action("win.record-toggle", &["<primary>r"]);
            obj.set_accels_for_action("win.cancel-recording", &["Escape"]);
        }
    }

    impl ApplicationImpl for Application {
        fn activate(&self) {
            let application = self.obj();

            // Autostart needs only tray, shortcuts and the optional API. Avoid
            // constructing every GTK page and loading a multi-GB model until the
            // user explicitly opens the application.
            let launch_hidden = *crate::application::LAUNCH_HIDDEN.get().unwrap_or(&false);
            if !self.started.get() && launch_hidden && application.main_window().is_none() {
                self.started.set(true);
                return;
            }

            // Find the existing main window (it may be hidden in the tray) or
            // create it. Don't rely on active_window(): a hidden window — or the
            // mini panel being the active one — would make it return the wrong
            // thing and spawn a duplicate.
            let window = application.main_window().unwrap_or_else(|| {
                let config = self
                    .config
                    .borrow()
                    .clone()
                    .unwrap_or_else(|| Arc::new(AppConfig::load()));
                *self.config.borrow_mut() = Some(config.clone());
                MainWindow::new(&application, config)
            });

            // Start hidden ONLY when launched with `--hidden` (autostart at
            // login). A manual launch always shows the window, and any later
            // activation (re-launch, tray "Open") does too.
            if self.started.get() || !launch_hidden {
                window.present();
            }
            self.started.set(true);

            // Diagnostic (inert unless STT_DEBUG_WIDTH is set): log the real
            // allocated window width. Kept because the window's width comes from
            // content sizing, not set_default_size — verify, never assume.
            if std::env::var("STT_DEBUG_WIDTH").is_ok() {
                let w = window.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(2500), move || {
                    eprintln!("STT_WIN_WIDTH={}", w.width());
                });
            }
        }

        fn startup(&self) {
            self.parent_startup();

            info!("{} {} starting up", APP_NAME, VERSION);

            // Initialize Libadwaita
            adw::init().expect("Failed to initialize Libadwaita");

            // Set up icon search paths for development
            if let Some(display) = gtk::gdk::Display::default() {
                let icon_theme = gtk::IconTheme::for_display(&display);

                if let Ok(exe_path) = std::env::current_exe() {
                    if let Some(exe_dir) = exe_path.parent() {
                        let dev_icons = exe_dir.join("../../data/icons");
                        if dev_icons.exists() {
                            if let Some(path_str) = dev_icons
                                .canonicalize()
                                .ok()
                                .and_then(|p| p.to_str().map(String::from))
                            {
                                icon_theme.add_search_path(&path_str);
                            }
                        }
                    }
                }
                icon_theme.add_search_path("data/icons");
            }

            gtk::Window::set_default_icon_name(crate::APP_ID);

            // Load configuration
            let config = Arc::new(AppConfig::load());
            *self.config.borrow_mut() = Some(config.clone());

            // Create the shared recording controller once, before any window.
            if self.controller.borrow().is_none() {
                *self.controller.borrow_mut() = Some(RecordingController::new());
            }

            // Preload the selected Whisper model in the background so dictation
            // from the mini panel / tray / global shortcut works immediately
            // when the app is autostarted hidden and no window — the usual
            // model loader — is ever constructed. A normal launch builds
            // MainWindow, whose load_selected_model() handles it instead.
            let launch_hidden = *crate::application::LAUNCH_HIDDEN.get().unwrap_or(&false);
            if launch_hidden && !config.first_run && config.backend == "whisper" {
                if let Some(controller) = self.controller.borrow().as_ref() {
                    let engine = controller.engine_arc();
                    let cfg = (*config).clone();
                    std::thread::Builder::new()
                        .name("model-preload".into())
                        .spawn(move || {
                            if let Err(e) = crate::recording::ensure_engine_loaded(&engine, &cfg) {
                                tracing::warn!("Startup model preload skipped: {e}");
                            }
                        })
                        .ok();
                }
            }

            // Start the local HTTP API server if the user enabled it.
            self.obj().start_api_server();

            // Register the global dictation shortcut via the portal. Best-effort:
            // failures are logged and the app keeps working with in-app controls.
            if config.mini_panel_enabled {
                use crate::portal::shortcuts::ShortcutKind;
                let (tx, rx) = async_channel::bounded::<ShortcutKind>(4);
                let trigger = config.global_shortcut.clone();
                // The transform-selection shortcut is opt-in (Settings → LLM).
                let transform_trigger = if config.llm_enabled && config.llm_selection_enabled {
                    Some(config.llm_selection_shortcut.clone())
                } else {
                    None
                };
                crate::application::tokio_runtime().spawn(
                    crate::portal::shortcuts::run_global_shortcuts(trigger, transform_trigger, tx),
                );

                let app_weak = self.obj().downgrade();
                glib::spawn_future_local(async move {
                    while let Ok(kind) = rx.recv().await {
                        let Some(app) = app_weak.upgrade() else { break };
                        match kind {
                            ShortcutKind::Dictation => {
                                app.activate_action("start-global-dictation", None)
                            }
                            ShortcutKind::TransformSelection => {
                                app.activate_action("transform-selection", None)
                            }
                        }
                    }
                });
            }

            // Keep the app alive in the background (no window required) so the
            // tray icon and global shortcut keep working after the main window
            // is closed. Quit explicitly via Ctrl+Q or the tray "Quit" item.
            // The guard must be retained — dropping it releases the hold.
            *self.hold_guard.borrow_mut() = Some(self.obj().hold());

            // System tray icon (best-effort; needs a StatusNotifier host).
            let tray_rx = crate::tray::spawn_tray();
            let app_weak = self.obj().downgrade();
            glib::spawn_future_local(async move {
                while let Ok(action) = tray_rx.recv().await {
                    let Some(app) = app_weak.upgrade() else { break };
                    app.on_tray_action(action);
                }
            });

            // Apply saved theme
            if let Some(ref theme) = config.theme {
                crate::ui::widgets::ThemePopover::apply_theme(theme);
            }

            // Load CSS stylesheet
            let obj = self.obj();
            obj.load_css();
        }
    }

    impl GtkApplicationImpl for Application {}
    impl AdwApplicationImpl for Application {}
}

glib::wrapper! {
    pub struct Application(ObjectSubclass<imp::Application>)
        @extends gio::Application, gtk::Application, adw::Application,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl Application {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("application-id", APP_ID)
            .property("flags", gio::ApplicationFlags::FLAGS_NONE)
            .build()
    }

    /// The shared recording controller, creating it on first use if startup
    /// hasn't run yet (it normally has).
    pub fn controller(&self) -> Rc<RecordingController> {
        if let Some(controller) = self.imp().controller.borrow().as_ref() {
            return controller.clone();
        }
        let controller = RecordingController::new();
        *self.imp().controller.borrow_mut() = Some(controller.clone());
        controller
    }

    // ===================================================================
    // Local HTTP API server
    // ===================================================================

    /// Whether the local API server is currently running.
    pub fn api_server_running(&self) -> bool {
        self.imp().api_server.borrow().is_some()
    }

    /// Start the local API server per the saved config. No-op if it's already
    /// running or disabled. When token auth is on, the bearer token is loaded
    /// from the keyring (created on first use) off the GTK thread, then the
    /// listener is bound back on the main thread and the handle is stored.
    pub fn start_api_server(&self) {
        if self.api_server_running() {
            return;
        }
        let config = AppConfig::load();
        if !config.api_server_enabled {
            return;
        }
        let generation = self.imp().api_start_generation.get().wrapping_add(1);
        self.imp().api_start_generation.set(generation);
        let controller = self.controller();
        let engine = controller.engine_arc();
        let catalog = controller.model_catalog_arc();
        let port = config.api_server_port;

        if !config.api_token_enabled {
            self.finish_start_api_server(engine, catalog, port, None, generation);
            return;
        }

        let (tx, rx) = async_channel::bounded::<Result<String, String>>(1);
        crate::application::tokio_runtime().spawn(async move {
            let token = match crate::secrets::load_api_token().await {
                Some(t) if !t.is_empty() => Ok(t),
                _ => {
                    let t = crate::api::generate_token();
                    crate::secrets::store_api_token(&t)
                        .await
                        .map(|_| t)
                        .map_err(|e| crate::error::redact_secrets(&e.to_string()))
                }
            };
            let _ = tx.send(token).await;
        });
        let app_weak = self.downgrade();
        glib::spawn_future_local(async move {
            let Ok(token) = rx.recv().await else { return };
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            let token = match token {
                Ok(token) => token,
                Err(error) => {
                    tracing::warn!("Could not store API token; server was not started: {error}");
                    return;
                }
            };
            let controller = app.controller();
            app.finish_start_api_server(
                controller.engine_arc(),
                controller.model_catalog_arc(),
                port,
                Some(token),
                generation,
            );
        });
    }

    fn finish_start_api_server(
        &self,
        engine: Arc<std::sync::Mutex<Option<crate::transcription::TranscriptionEngine>>>,
        catalog: Arc<crate::transcription::ModelCatalog>,
        port: u16,
        token: Option<String>,
        generation: u64,
    ) {
        let config = AppConfig::load();
        if self.api_server_running()
            || self.imp().api_start_generation.get() != generation
            || !config.api_server_enabled
            || config.api_server_port != port
        {
            return;
        }
        match crate::api::start(engine, catalog, port, token) {
            Ok(handle) => *self.imp().api_server.borrow_mut() = Some(handle),
            Err(e) => tracing::warn!("Could not start API server: {e}"),
        }
    }

    /// Stop the local API server if running (closes the port).
    pub fn stop_api_server(&self) {
        self.imp()
            .api_start_generation
            .set(self.imp().api_start_generation.get().wrapping_add(1));
        if let Some(handle) = self.imp().api_server.borrow_mut().take() {
            handle.stop();
        }
    }

    /// Restart the API server (used after a port change while enabled).
    pub fn restart_api_server(&self) {
        self.stop_api_server();
        self.start_api_server();
    }

    // ===================================================================
    // Global dictation (mini panel)
    // ===================================================================

    /// The current configuration. Reads the (cache-backed) saved config so the
    /// global dictation path always reflects the latest settings — translate,
    /// model, language, mic, mode — not just the values from startup.
    fn config_snapshot(&self) -> AppConfig {
        AppConfig::load()
    }

    /// Find the main window among the application's windows, if open.
    fn main_window(&self) -> Option<MainWindow> {
        self.windows()
            .into_iter()
            .find_map(|w| w.downcast::<MainWindow>().ok())
    }

    /// Show the main window (creating it if it doesn't exist yet).
    fn present_main_window(&self) {
        let window = self.main_window().unwrap_or_else(|| {
            let config = self
                .imp()
                .config
                .borrow()
                .clone()
                .unwrap_or_else(|| Arc::new(AppConfig::load()));
            MainWindow::new(self, config)
        });
        window.present();
    }

    fn on_tray_action(&self, action: crate::tray::TrayAction) {
        use crate::tray::TrayAction;
        match action {
            TrayAction::Dictate => self.toggle_global_dictation(),
            TrayAction::TransformSelection => self.transform_selection(),
            TrayAction::Open => self.present_main_window(),
            TrayAction::Quit => self.quit(),
        }
    }

    /// The mini panel, created (and its actions wired) on first use.
    fn mini_panel(&self) -> MiniPanel {
        if let Some(panel) = self.imp().mini_panel.borrow().as_ref() {
            return panel.clone();
        }
        let panel = MiniPanel::new(self);
        // Anchor the panel to the main window as a transient child. GTK4 has no
        // skip-taskbar API, but window managers don't give transient children a
        // separate taskbar/dock entry — so the app shows a single icon instead of
        // two (main window + panel) while both are open. The main window object
        // lives for the whole app lifetime (hide_on_close), so this stays valid
        // even when it's hidden in the tray.
        if let Some(main) = self.main_window() {
            panel.set_transient_for(Some(&main));
        }
        let app_weak = self.downgrade();
        panel.connect_action(move |action| {
            if let Some(app) = app_weak.upgrade() {
                app.on_mini_panel_action(action);
            }
        });
        panel.set_keep_on_top(self.config_snapshot().mini_panel_always_on_top);
        *self.imp().mini_panel.borrow_mut() = Some(panel.clone());
        panel
    }

    fn on_mini_panel_action(&self, action: MiniPanelAction) {
        match action {
            // Stop/Cancel are owner-aware: a Voice-edit capture is stopped/cancelled
            // by its own path, not the global-dictation path.
            MiniPanelAction::Stop => {
                if self.controller().owner() == RecordingOwner::VoiceEdit {
                    self.stop_voice_edit();
                } else {
                    self.stop_global_dictation();
                }
            }
            MiniPanelAction::Cancel => {
                if self.controller().owner() == RecordingOwner::VoiceEdit {
                    self.cancel_voice_edit();
                } else {
                    self.cancel_global_dictation();
                }
            }
            // "New": start a fresh recording reusing the already-open panel.
            MiniPanelAction::Again => self.start_global_dictation(),
            MiniPanelAction::Paste => self.paste_preview_text(),
            MiniPanelAction::Copy => self.copy_preview_text(),
            MiniPanelAction::Close => self.close_mini_panel(),
            MiniPanelAction::Chip(idx) => self.on_panel_chip(idx),
            MiniPanelAction::Variant(idx) => self.on_panel_variant(idx),
            MiniPanelAction::VoiceEdit => self.start_voice_edit(),
        }
    }

    /// Toggle global dictation: start when idle, stop when the mini panel is
    /// already recording, ignore while the main window is recording.
    fn toggle_global_dictation(&self) {
        match self.controller().owner() {
            RecordingOwner::Mini => self.stop_global_dictation(),
            RecordingOwner::Main => {
                info!("Global shortcut ignored: main window is recording");
            }
            RecordingOwner::VoiceEdit => {
                info!("Global shortcut ignored: a voice edit is in progress");
            }
            RecordingOwner::None => self.start_global_dictation(),
        }
    }

    fn start_global_dictation(&self) {
        let controller = self.controller();
        if !controller.try_acquire(RecordingOwner::Mini) {
            return;
        }
        let generation = self.imp().dictation_generation.get().wrapping_add(1);
        self.imp().dictation_generation.set(generation);

        let config = self.config_snapshot();
        let lang_label = panel_lang_label(&config);
        let panel = self.mini_panel();
        // Re-apply each run so toggling the setting takes effect without restart.
        panel.set_keep_on_top(config.mini_panel_always_on_top);
        // Show the LLM indicator only when auto-improve will actually run on this
        // dictation (integration enabled AND auto-apply on) — not merely when an
        // LLM connection is configured.
        panel.set_llm_active(config.llm_enabled && config.llm_auto_apply);
        panel.show_recording(&lang_label);
        panel.present();

        let (waveform_tx, waveform_rx) = async_channel::bounded::<Vec<f32>>(32);
        match controller.start(config.selected_microphone.as_deref(), waveform_tx) {
            Ok(()) => {
                // Feed the waveform to the panel.
                let panel_weak = panel.downgrade();
                glib::spawn_future_local(async move {
                    while let Ok(amps) = waveform_rx.recv().await {
                        let Some(p) = panel_weak.upgrade() else { break };
                        p.update_waveform(amps);
                    }
                });

                // Tick the timer until recording stops.
                let app_weak = self.downgrade();
                let panel_weak = panel.downgrade();
                // 100ms tick so the timer can show centiseconds.
                glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                    let (Some(app), Some(panel)) = (app_weak.upgrade(), panel_weak.upgrade())
                    else {
                        return glib::ControlFlow::Break;
                    };
                    let controller = app.controller();
                    if controller.owner() != RecordingOwner::Mini
                        || controller.state() == RecordingState::Idle
                    {
                        return glib::ControlFlow::Break;
                    }
                    panel.set_timer(controller.recording_duration_secs() as f64);
                    glib::ControlFlow::Continue
                });

                info!("Global dictation started");
            }
            Err(e) => {
                controller.release();
                panel.show_error(&format!("Couldn't start recording: {e}"));
            }
        }
    }

    fn stop_global_dictation(&self) {
        let controller = self.controller();
        if controller.owner() != RecordingOwner::Mini {
            return;
        }

        // Capture mic duration before stop() drains the buffer (for WPM stats).
        let duration_secs = controller.recording_duration_secs();
        let generation = self.imp().dictation_generation.get();

        let audio = match controller.stop_snapshot() {
            Ok(a) => a,
            Err(e) => {
                controller.release();
                self.mini_panel()
                    .show_error(&format!("Error stopping recording: {e}"));
                return;
            }
        };
        controller.release();

        let panel = self.mini_panel();
        let config = self.config_snapshot();
        panel.show_transcribing(&panel_model_label(&config), &panel_lang_label(&config));
        if config.backend == "cohere" && !crate::transcription::cohere::cohere_ready() {
            panel.show_error(&crate::i18n::gettext(
                "Cohere is not set up. Go to Settings → Model to download the runtime and model.",
            ));
            return;
        }
        if config.backend == "qwen" && !crate::transcription::qwen::qwen_ready() {
            panel.show_error(&crate::i18n::gettext(
                "Qwen3-ASR is not set up. Go to Settings → Model to download the runtime and model.",
            ));
            return;
        }

        let params = DictationParams::from_config(&config);

        // The pop-up always uses a clean batch decode (no in-decode hooks).
        // Whisper.cpp callbacks under Vulkan + GTK always-on-top compositing
        // trip -6 here, and live-segment preview adds little UX value for the
        // pop-up's short dictations. The live_transcription setting applies to
        // the main window's live loop, not this path.
        let receiver = controller.transcribe_snapshot_async(audio, params, duration_secs);

        let app_weak = self.downgrade();
        glib::spawn_future_local(async move {
            let result = receiver.recv().await;
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            if app.imp().dictation_generation.get() != generation {
                return;
            }
            // If the user already started a new dictation while this one was
            // transcribing, drop this (stale) result so it doesn't overwrite the
            // live recording UI.
            if app.controller().owner() == RecordingOwner::Mini {
                return;
            }
            match result {
                Ok(Ok(outcome)) => app.finish_global_dictation(outcome, generation),
                Ok(Err(msg)) => app.mini_panel().show_error(&msg),
                Err(_) => {}
            }
        });
    }

    fn finish_global_dictation(&self, outcome: DictationOutcome, generation: u64) {
        let panel = self.mini_panel();
        let cleaned = outcome.cleaned_text.clone();
        if cleaned.is_empty() {
            panel.show_error(&crate::i18n::gettext(
                "No clear speech detected — try again",
            ));
            return;
        }

        // History always keeps the raw transcript (auto-improve adds a variant; it
        // doesn't replace what's recorded).
        self.record_global_history(&cleaned, &outcome);

        // Build the current result (raw + stats + segments) for the chips and the
        // raw/polished selector.
        let state = crate::ui::result_state::ResultState::new(
            cleaned.clone(),
            outcome.duration_secs,
            outcome.detected_language.clone(),
            outcome.segments.clone(),
        );
        *self.imp().last_result_state.borrow_mut() = Some(state);

        let config = self.config_snapshot();
        if config.llm_enabled && config.llm_auto_apply && !config.llm_presets.is_empty() {
            // Improve the transcript with the active preset before delivering it.
            let idx = config.llm_active_preset.min(config.llm_presets.len() - 1);
            let preset = config.llm_presets[idx].clone();
            let llm_cfg = resolve_llm_cfg(&config, &preset);
            panel.show_improving();
            let rx = crate::llm::improve_async(llm_cfg, preset.system_prompt(), cleaned.clone());
            let app_weak = self.downgrade();
            let label = preset.name.clone();
            glib::spawn_future_local(async move {
                let res = rx.recv().await;
                let Some(app) = app_weak.upgrade() else {
                    return;
                };
                if app.imp().dictation_generation.get() != generation {
                    return;
                }
                // Drop stale results if a new dictation has started meanwhile.
                if app.controller().owner() == RecordingOwner::Mini {
                    return;
                }
                // On success, add the improved text as the active variant; on any
                // error fall back to the raw transcript (active stays 0).
                if let Ok(Ok(improved)) = &res {
                    if !improved.trim().is_empty() {
                        if let Some(st) = app.imp().last_result_state.borrow_mut().as_mut() {
                            st.push_variant(label, improved.trim().to_string());
                        }
                    }
                }
                app.deliver_active_result();
            });
        } else {
            self.deliver_active_result();
        }
        info!(
            "Global dictation complete ({:.0}% confidence)",
            outcome.confidence * 100.0
        );
    }

    /// Deliver the current result state's active text (clipboard + auto-paste or
    /// result view), honoring the auto-paste setting. Used for the initial result.
    fn deliver_active_result(&self) {
        let text = self
            .imp()
            .last_result_state
            .borrow()
            .as_ref()
            .map(|s| s.active_text().to_string())
            .unwrap_or_default();
        self.deliver_global_result(text);
    }

    /// Re-show the active result in the panel WITHOUT auto-pasting (used after a
    /// chip or raw/polished switch, when the user is interacting with the panel).
    fn show_active_result(&self) {
        let text = self
            .imp()
            .last_result_state
            .borrow()
            .as_ref()
            .map(|s| s.active_text().to_string())
            .unwrap_or_default();
        *self.imp().last_text.borrow_mut() = text.clone();
        let panel = self.mini_panel();
        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(&text);
            display.flush();
        }
        panel.show_result(&text, true);
        self.render_panel_result_extras();
    }

    /// Populate the panel's transform chips, stats line, and raw/polished selector
    /// from the current result state + LLM config.
    fn render_panel_result_extras(&self) {
        let panel = self.mini_panel();
        let cfg = self.config_snapshot();
        let names: Vec<String> = cfg.llm_presets.iter().map(|p| p.name.clone()).collect();
        panel.set_chip_presets(&names);
        panel.set_chips_visible(cfg.llm_enabled);
        panel.set_chips_sensitive(true);
        panel.set_voice_edit_visible(cfg.llm_enabled);
        let state = self.imp().last_result_state.borrow();
        if let Some(st) = state.as_ref() {
            panel.set_result_stats(st.stats.words, st.stats.wpm);
            let labels = st.selector_labels(&crate::i18n::gettext("Raw"));
            panel.set_variant_selector(&labels, st.active);
        }
    }

    /// Handle a transform chip in the panel: run preset `idx` on the active text
    /// and add the result as a new active variant (no auto-paste).
    fn on_panel_chip(&self, idx: usize) {
        let source = self
            .imp()
            .last_result_state
            .borrow()
            .as_ref()
            .map(|s| s.active_text().trim().to_string())
            .unwrap_or_default();
        if source.is_empty() {
            return;
        }
        let config = self.config_snapshot();
        let Some(preset) = config.llm_presets.get(idx).cloned() else {
            return;
        };
        let llm_cfg = resolve_llm_cfg(&config, &preset);
        self.mini_panel().set_chips_sensitive(false);
        let rx = crate::llm::improve_async(llm_cfg, preset.system_prompt(), source);
        let app_weak = self.downgrade();
        let label = preset.name.clone();
        glib::spawn_future_local(async move {
            let res = rx.recv().await;
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            app.mini_panel().set_chips_sensitive(true);
            if let Ok(Ok(improved)) = &res {
                if !improved.trim().is_empty() {
                    if let Some(st) = app.imp().last_result_state.borrow_mut().as_mut() {
                        st.push_variant(label, improved.trim().to_string());
                    }
                    app.show_active_result();
                }
            }
        });
    }

    /// Handle the panel's raw/polished selector change.
    fn on_panel_variant(&self, idx: usize) {
        if let Some(st) = self.imp().last_result_state.borrow_mut().as_mut() {
            st.set_active(idx);
        }
        self.show_active_result();
    }

    /// Begin a Voice Edit: capture a short spoken instruction to transform the
    /// current result's active text. Reuses the single recording controller under
    /// a dedicated `VoiceEdit` owner so it can't collide with global dictation.
    fn start_voice_edit(&self) {
        let target = self
            .imp()
            .last_result_state
            .borrow()
            .as_ref()
            .map(|s| s.active_text().trim().to_string())
            .unwrap_or_default();
        let panel = self.mini_panel();
        if target.is_empty() {
            panel.show_error(&crate::i18n::gettext("No text to edit."));
            return;
        }
        let config = self.config_snapshot();
        if !config.llm_enabled {
            panel.show_error(&crate::i18n::gettext(
                "Enable the LLM in Settings → LLM to use Voice edit.",
            ));
            return;
        }
        let controller = self.controller();
        if !controller.try_acquire(RecordingOwner::VoiceEdit) {
            return; // something else is recording
        }
        *self.imp().voice_edit_target.borrow_mut() = Some(target);

        panel.show_recording(&crate::i18n::gettext("Speak your edit"));
        panel.present();

        let (waveform_tx, waveform_rx) = async_channel::bounded::<Vec<f32>>(32);
        match controller.start(config.selected_microphone.as_deref(), waveform_tx) {
            Ok(()) => {
                let panel_weak = panel.downgrade();
                glib::spawn_future_local(async move {
                    while let Ok(amps) = waveform_rx.recv().await {
                        let Some(p) = panel_weak.upgrade() else { break };
                        p.update_waveform(amps);
                    }
                });
                let app_weak = self.downgrade();
                let panel_weak = panel.downgrade();
                glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                    let (Some(app), Some(panel)) = (app_weak.upgrade(), panel_weak.upgrade())
                    else {
                        return glib::ControlFlow::Break;
                    };
                    let controller = app.controller();
                    if controller.owner() != RecordingOwner::VoiceEdit
                        || controller.state() == RecordingState::Idle
                    {
                        return glib::ControlFlow::Break;
                    }
                    panel.set_timer(controller.recording_duration_secs() as f64);
                    glib::ControlFlow::Continue
                });
            }
            Err(e) => {
                controller.release();
                *self.imp().voice_edit_target.borrow_mut() = None;
                panel.show_error(&format!("Couldn't start recording: {e}"));
            }
        }
    }

    /// Stop the Voice-edit capture and transcribe the spoken instruction.
    fn stop_voice_edit(&self) {
        let controller = self.controller();
        if controller.owner() != RecordingOwner::VoiceEdit {
            return;
        }
        let duration_secs = controller.recording_duration_secs();
        let audio = match controller.stop() {
            Ok(a) => a,
            Err(e) => {
                controller.release();
                self.mini_panel()
                    .show_error(&format!("Error stopping recording: {e}"));
                return;
            }
        };
        controller.release();

        let panel = self.mini_panel();
        if audio.is_empty() {
            panel.show_error(&crate::i18n::gettext("Didn't catch an instruction."));
            self.show_active_result();
            return;
        }
        let config = self.config_snapshot();
        panel.show_transcribing(&panel_model_label(&config), &panel_lang_label(&config));

        let params = DictationParams::from_config(&config);
        let receiver = controller.transcribe_async(audio, params, duration_secs);
        let app_weak = self.downgrade();
        glib::spawn_future_local(async move {
            let result = receiver.recv().await;
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            match result {
                Ok(Ok(outcome)) => app.run_voice_edit_llm(outcome.cleaned_text),
                Ok(Err(msg)) => app.mini_panel().show_error(&msg),
                Err(_) => {}
            }
        });
    }

    /// Cancel a Voice-edit capture and restore the previous result view.
    fn cancel_voice_edit(&self) {
        let controller = self.controller();
        if controller.owner() == RecordingOwner::VoiceEdit {
            controller.cancel();
            controller.release();
        }
        *self.imp().voice_edit_target.borrow_mut() = None;
        self.show_active_result();
    }

    /// Apply the spoken instruction to the target text via the LLM and add the
    /// result as a new "Voice edit" variant.
    fn run_voice_edit_llm(&self, instruction: String) {
        let instruction = instruction.trim().to_string();
        let target = self
            .imp()
            .voice_edit_target
            .borrow()
            .clone()
            .unwrap_or_default();
        *self.imp().voice_edit_target.borrow_mut() = None;
        let panel = self.mini_panel();
        if instruction.is_empty() {
            panel.show_error(&crate::i18n::gettext("Didn't catch an instruction."));
            self.show_active_result();
            return;
        }
        if target.is_empty() {
            self.show_active_result();
            return;
        }
        let config = self.config_snapshot();
        let llm_cfg = crate::llm::LlmConfig {
            api_url: config.llm_api_url.clone(),
            api_key: None,
            model: config.llm_model.clone(),
            temperature: config.llm_temperature,
        };
        let system = "You are editing the user's text. Apply the user's spoken instruction to \
                      the TARGET TEXT and reply with ONLY the edited text, preserving the original \
                      language. Do not add explanations or quotes.";
        let user = format!("Apply this instruction: {instruction}\n\nText:\n{target}");
        panel.show_improving();
        let rx = crate::llm::improve_async(llm_cfg, system.to_string(), user);
        let app_weak = self.downgrade();
        glib::spawn_future_local(async move {
            let res = rx.recv().await;
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            match res {
                Ok(Ok(edited)) if !edited.trim().is_empty() => {
                    if let Some(st) = app.imp().last_result_state.borrow_mut().as_mut() {
                        st.push_variant(
                            crate::i18n::gettext("Voice edit"),
                            edited.trim().to_string(),
                        );
                    }
                    app.show_active_result();
                }
                Ok(Ok(_)) => {
                    app.mini_panel()
                        .show_error(&crate::i18n::gettext("AI returned an empty result"));
                }
                Ok(Err(e)) => app.mini_panel().show_error(&e),
                Err(_) => {}
            }
        });
    }

    /// Put the final text on the clipboard and either auto-paste it (re-showing
    /// the panel afterwards) or show it in the result state.
    fn deliver_global_result(&self, text: String) {
        *self.imp().last_text.borrow_mut() = text.clone();

        let panel = self.mini_panel();
        if self.config_snapshot().auto_paste {
            // Auto-paste path: the clipboard MUST be set while the panel surface
            // holds keyboard focus. On Wayland, Mutter rejects a set_selection from
            // an unfocused surface, so setting it here — when the user may have
            // clicked into their editor mid-recording — silently keeps the
            // *previous* clipboard and pastes stale text. The reshow task
            // re-acquires focus, sets the clipboard, hides, then pastes.
            let generation = self.imp().dictation_generation.get();
            self.spawn_autopaste_then_reshow(text, generation);
        } else {
            // Non-auto-paste: the panel stays visible and focused (its preceding
            // transcribing/improving state was presented), so the set succeeds and
            // the manual Copy/Paste buttons can read from it.
            if let Some(display) = gtk::gdk::Display::default() {
                display.clipboard().set_text(&text);
                display.flush();
            }
            panel.show_result(&text, true);
            self.render_panel_result_extras();
        }
    }

    fn record_global_history(&self, text: &str, outcome: &DictationOutcome) {
        let config = self.config_snapshot();
        let lang_name = if config.auto_detect_language {
            outcome
                .detected_language
                .as_deref()
                .map(|c| {
                    format!(
                        "Auto-detect ({})",
                        crate::ui::settings::language_code_to_name(c)
                    )
                })
                .unwrap_or_else(|| "Auto-detect".to_string())
        } else {
            config
                .language
                .as_deref()
                .map(crate::ui::settings::language_code_to_name)
                .unwrap_or_else(|| "Auto-detect".to_string())
        };
        let model = if config.backend == "cohere" {
            "cohere-transcribe".to_string()
        } else {
            config.selected_model.clone()
        };
        let entry = crate::ui::history_page::HistoryEntry {
            id: uuid::Uuid::new_v4().to_string(),
            title: ellipsize_chars(text, 60),
            text: text.to_string(),
            language: lang_name,
            duration_secs: outcome.duration_secs.round() as u64,
            timestamp: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
            model,
            word_count: Some(crate::ui::result_state::word_count(text) as u32),
            ..Default::default()
        };

        // Route through the live HistoryPage when the main window is open
        // (keeps memory + disk in sync); otherwise append to disk directly.
        let entry_id = entry.id.clone();
        if let Some(win) = self.main_window() {
            win.add_history_entry(entry);
        } else {
            crate::ui::history_page::append_entry_to_disk(&entry);
        }

        // LLM auto-title (best effort; updates the entry once it returns).
        self.auto_title(entry_id, text.to_string());
    }

    /// When the LLM is enabled, generate a short (≤6 word) title for a saved
    /// transcript and update the history entry once it comes back.
    pub fn auto_title(&self, id: String, raw_text: String) {
        let config = self.config_snapshot();
        // Only contact the LLM automatically when auto-improve is enabled — with
        // it off, no automatic requests are sent (titling included).
        if !config.llm_enabled || !config.llm_auto_apply || raw_text.trim().is_empty() {
            return;
        }
        let llm_cfg = crate::llm::LlmConfig {
            api_url: config.llm_api_url.clone(),
            api_key: None,
            model: config.llm_model.clone(),
            temperature: 0.2,
        };
        let prompt = "Give a concise title of at most 6 words for the following text. \
                      Reply with only the title — no quotes, no trailing punctuation."
            .to_string();
        let rx = crate::llm::improve_async(llm_cfg, prompt, raw_text);
        let app_weak = self.downgrade();
        glib::spawn_future_local(async move {
            if let Ok(Ok(title)) = rx.recv().await {
                let title = title.trim().trim_matches('"').trim().to_string();
                if title.is_empty() {
                    return;
                }
                let title = ellipsize_chars(&title, 60);
                let Some(app) = app_weak.upgrade() else {
                    return;
                };
                if let Some(win) = app.main_window() {
                    win.history_update_title(&id, &title);
                } else {
                    crate::ui::history_page::update_title_on_disk(&id, &title);
                }
            }
        });
    }

    fn cancel_global_dictation(&self) {
        self.imp()
            .dictation_generation
            .set(self.imp().dictation_generation.get().wrapping_add(1));
        let controller = self.controller();
        if controller.owner() == RecordingOwner::Mini {
            controller.cancel();
            controller.release();
        }
        self.close_mini_panel();
    }

    /// System-wide "Transform selection with AI": read the PRIMARY selection
    /// (falling back to the clipboard), run the active preset, put the result on
    /// the clipboard and paste it back into the focused app.
    ///
    /// On Wayland we can't read an arbitrary app's live selection API, so this
    /// uses the PRIMARY selection / clipboard text the user already highlighted
    /// or copied.
    fn transform_selection(&self) {
        let config = self.config_snapshot();
        if !config.llm_enabled || config.llm_presets.is_empty() {
            self.mini_panel().show_error(&crate::i18n::gettext(
                "Enable the LLM in Settings → LLM to use Transform selection.",
            ));
            self.mini_panel().present();
            return;
        }
        let idx = config.llm_active_preset.min(config.llm_presets.len() - 1);
        let preset = config.llm_presets[idx].clone();
        let llm_cfg = resolve_llm_cfg(&config, &preset);

        let Some(display) = gtk::gdk::Display::default() else {
            return;
        };
        let primary = display.primary_clipboard();
        let clipboard = display.clipboard();

        let app_weak = self.downgrade();
        glib::spawn_future_local(async move {
            // Read PRIMARY first (highlighted text), then fall back to clipboard.
            let mut text = primary
                .read_text_future()
                .await
                .ok()
                .flatten()
                .map(|g| g.to_string())
                .unwrap_or_default();
            if text.trim().is_empty() {
                text = clipboard
                    .read_text_future()
                    .await
                    .ok()
                    .flatten()
                    .map(|g| g.to_string())
                    .unwrap_or_default();
            }
            let text = text.trim().to_string();
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            if text.is_empty() {
                let panel = app.mini_panel();
                panel.show_error(&crate::i18n::gettext(
                    "No selected or copied text found to transform.",
                ));
                panel.present();
                return;
            }

            // Show progress on the mini panel.
            let panel = app.mini_panel();
            panel.set_llm_active(true);
            panel.show_improving();
            panel.present();

            let rx = crate::llm::improve_async(llm_cfg, preset.system_prompt(), text);
            let res = rx.recv().await;
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            match res {
                Ok(Ok(improved)) if !improved.trim().is_empty() => {
                    app.deliver_global_result(improved.trim().to_string());
                }
                Ok(Err(e)) => app.mini_panel().show_error(&e),
                _ => app
                    .mini_panel()
                    .show_error(&crate::i18n::gettext("AI transform failed.")),
            }
        });
    }

    fn paste_preview_text(&self) {
        // The clipboard already holds the text; hide to return focus, then paste.
        self.close_mini_panel();
        self.spawn_autopaste();
    }

    fn copy_preview_text(&self) {
        // Copy exactly what's shown in the result panel; fall back to last_text.
        let panel = self.mini_panel();
        let mut text = panel.transcript_text();
        if text.trim().is_empty() {
            text = self.imp().last_text.borrow().clone();
        }
        if text.is_empty() {
            return;
        }
        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(&text);
        }
        // Give visible feedback that the copy happened.
        panel.show_copied_badge();
    }

    fn close_mini_panel(&self) {
        if let Some(panel) = self.imp().mini_panel.borrow().as_ref() {
            panel.set_visible(false);
        }
    }

    /// Wait until the freshly-set clipboard content is actually live, then a
    /// short focus-settle delay, so a synthesized Ctrl+V reads the *current*
    /// transcript and not the previously-owned clipboard content (Wayland sets
    /// the selection asynchronously).
    async fn await_clipboard_ready() {
        if let Some(display) = gtk::gdk::Display::default() {
            // Round-trip read forces GTK to process the pending set_selection.
            let _ = display.clipboard().read_text_future().await;
        }
        // Give the compositor time to return focus to the target app after the
        // panel unmapped before we inject the paste keystroke.
        glib::timeout_future(std::time::Duration::from_millis(250)).await;
    }

    /// Hide the panel, then auto-paste the (already-set) clipboard into the
    /// now-focused app on the Tokio runtime.
    fn spawn_autopaste(&self) {
        glib::spawn_future_local(async {
            Self::await_clipboard_ready().await;
            crate::application::tokio_runtime().spawn(async {
                let _ = crate::portal::paste::try_autopaste().await;
            });
        });
    }

    /// Hide the panel so keyboard focus returns to the target app, deliver the
    /// transcript into it, then re-present the panel in the result state so the
    /// user can immediately dictate again — the "dictate → paste → stay open →
    /// repeat" loop.
    ///
    /// Primary path: the Clipboard portal owns the system selection
    /// focus-independently (see
    /// [`crate::portal::paste::paste_text_via_remote_desktop`]), so the *current*
    /// transcript is pasted even when the panel never held focus — including when
    /// the user clicked into another window mid-transcription. Fallback (compositor
    /// without the Clipboard portal): set the GTK clipboard while the panel is
    /// focused, then inject Ctrl+V.
    fn spawn_autopaste_then_reshow(&self, text: String, generation: u64) {
        let app_weak = self.downgrade();
        glib::spawn_future_local(async move {
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            if app.imp().dictation_generation.get() != generation {
                return;
            }
            let panel = app.mini_panel();

            // Hide so the compositor hands keyboard focus back to the previously
            // focused app — the injected Ctrl+V must land there, not on the panel.
            // We deliberately do NOT try to (re)focus the panel: the portal sets
            // the clipboard without focus, which is what makes the
            // click-into-another-window case work.
            panel.set_visible(false);
            glib::timeout_future(std::time::Duration::from_millis(300)).await;
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            if app.imp().dictation_generation.get() != generation {
                return;
            }

            // Primary: Clipboard portal + Ctrl+V on one RemoteDesktop session.
            let (tx, rx) = async_channel::bounded::<bool>(1);
            let portal_text = text.clone();
            crate::application::tokio_runtime().spawn(async move {
                let ok = crate::portal::paste::paste_text_via_remote_desktop(portal_text).await;
                let _ = tx.send(ok).await;
            });
            let pasted = rx.recv().await.unwrap_or(false);

            // Fallback when the compositor has no Clipboard portal: set the GTK
            // clipboard while the panel holds focus, then inject Ctrl+V.
            if !pasted {
                if let Some(app) = app_weak.upgrade() {
                    let panel = app.mini_panel();
                    panel.set_visible(true);
                    panel.present();
                    Self::wait_for_panel_active(&panel, 600).await;
                    if let Some(display) = gtk::gdk::Display::default() {
                        display.clipboard().set_text(&text);
                        display.flush();
                        let _ = display.clipboard().read_text_future().await;
                    }
                    panel.set_visible(false);
                    glib::timeout_future(std::time::Duration::from_millis(300)).await;
                    let (done_tx, done_rx) = async_channel::bounded::<()>(1);
                    crate::application::tokio_runtime().spawn(async move {
                        let _ = crate::portal::paste::try_autopaste().await;
                        let _ = done_tx.send(()).await;
                    });
                    let _ = done_rx.recv().await;
                }
            }

            // Re-present the panel showing the transcript, unless the user has
            // already started a new recording in the meantime.
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            if app.imp().dictation_generation.get() != generation
                || app.controller().owner() == RecordingOwner::Mini
            {
                return;
            }
            let panel = app.mini_panel();
            panel.set_visible(true);
            panel.present();
            panel.show_result(&text, true);
            app.render_panel_result_extras();
        });
    }

    /// Wait until the mini panel surface is active (has keyboard focus), up to
    /// `timeout_ms`. Signal-driven via `notify::is-active` (present() grants focus
    /// on the compositor's own clock, so polling would race it) with a timeout
    /// fallback so the paste flow is never blocked indefinitely.
    async fn wait_for_panel_active(panel: &MiniPanel, timeout_ms: u64) {
        if panel.is_active() {
            return;
        }
        let (tx, rx) = async_channel::bounded::<()>(1);
        let handler = panel.connect_is_active_notify({
            let tx = tx.clone();
            move |p| {
                if p.is_active() {
                    let _ = tx.try_send(());
                }
            }
        });
        glib::timeout_add_local_once(std::time::Duration::from_millis(timeout_ms), move || {
            let _ = tx.try_send(());
        });
        let _ = rx.recv().await;
        panel.disconnect(handler);
    }

    fn load_css(&self) {
        let display = match gtk::gdk::Display::default() {
            Some(d) => d,
            None => return,
        };

        let provider = gtk::CssProvider::new();
        let css = include_str!("../data/resources/style.css");
        provider.load_from_string(css);

        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        // Reload CSS on theme changes
        let style_manager = adw::StyleManager::default();
        let provider_weak = provider.downgrade();

        let pw = provider_weak.clone();
        style_manager.connect_color_scheme_notify(move |_| {
            if let Some(p) = pw.upgrade() {
                Self::reload_css_provider(&p);
            }
        });

        let pw = provider_weak.clone();
        style_manager.connect_dark_notify(move |_| {
            if let Some(p) = pw.upgrade() {
                Self::reload_css_provider(&p);
            }
        });

        let pw = provider_weak;
        style_manager.connect_high_contrast_notify(move |_| {
            if let Some(p) = pw.upgrade() {
                Self::reload_css_provider(&p);
            }
        });
    }

    fn reload_css_provider(provider: &gtk::CssProvider) {
        let css = include_str!("../data/resources/style.css");
        provider.load_from_string(css);
    }

    fn setup_actions(&self) {
        let action_quit = gio::ActionEntry::builder("quit")
            .activate(|app: &Self, _, _| {
                app.quit();
            })
            .build();

        let action_about = gio::ActionEntry::builder("about")
            .activate(|app: &Self, _, _| {
                app.show_about();
            })
            .build();

        let action_whats_new = gio::ActionEntry::builder("whats-new")
            .activate(|app: &Self, _, _| {
                app.show_whats_new();
            })
            .build();

        // Global dictation: toggles the mini panel recording on/off.
        let action_dictation = gio::ActionEntry::builder("start-global-dictation")
            .activate(|app: &Self, _, _| {
                app.toggle_global_dictation();
            })
            .build();

        // Transform selection/clipboard with the active AI preset.
        let action_transform = gio::ActionEntry::builder("transform-selection")
            .activate(|app: &Self, _, _| {
                app.transform_selection();
            })
            .build();

        self.add_action_entries([
            action_quit,
            action_about,
            action_whats_new,
            action_dictation,
            action_transform,
        ]);
    }

    fn show_about(&self) {
        let window = self.active_window();

        let about = adw::AboutDialog::builder()
            .application_name(APP_NAME)
            .application_icon(APP_ID)
            .developer_name("Christos A. Daggas")
            .version(VERSION)
            .copyright("© 2026 Christos A. Daggas")
            .license_type(gtk::License::MitX11)
            .website("https://chrisdaggas.com")
            .issue_url("https://github.com/christosdaggas/speech-to-text/issues")
            .developers(vec!["Christos A. Daggas"])
            .comments("Offline speech-to-text transcription using Whisper")
            .release_notes(
                "<p>Version 1.5.0</p>\
                <ul>\
                    <li>Verified runtime and model downloads with resumable transfers.</li>\
                    <li>Faster inference, bounded background work, and smoother live previews.</li>\
                    <li>More reliable recording, pause, resume, language selection, and cancellation.</li>\
                    <li>Privacy-first AI consent, explicit automation controls, and safer credentials.</li>\
                    <li>Expanded history, file transcription recovery, and full-text search.</li>\
                    <li>A refined workspace with improved navigation, Help, Settings, and light-theme styling.</li>\
                </ul>\
                <p>Version 1.4.0</p>\
                <ul>\
                    <li>New: an Open File button in the controls row transcribes an existing audio file from disk (WAV, MP3, FLAC, OGG, Opus, or M4A) — results, stats, segments, SRT export, and the Actions/Voice-edit menu all work as they do for a live recording.</li>\
                    <li>Fixed: the mini panel no longer fails mid-session with “Generic whisper error, code -6” on GPUs that use Vulkan, especially with larger models or wider beam search. The mini panel now always uses a clean batch decode.</li>\
                    <li>Fixed: borderline audio (whispered, noisy, or short clips) no longer breaks a whole transcription. Whisper’s built-in temperature retry is re-enabled, so a difficult segment is degraded gracefully instead of throwing an error.</li>\
                    <li>Changed: “Show text live while transcribing” applies only to the main window now; the mini panel is always a clean batch decode. The Settings label reflects this.</li>\
                    <li>Changed: the beam_size setting is honoured everywhere — the main window’s live preview no longer hard-codes greedy decoding. It still has a self-protection that pauses the preview if your hardware can’t keep up.</li>\
                    <li>Changed: the mini panel’s “Improve with AI” chips are consolidated into a single “Actions” dropdown next to Voice edit, matching the main window.</li>\
                    <li>Changed: Settings pages now fill the full content width instead of being clamped to a narrow centred column.</li>\
                </ul>\
                <p>Version 1.3.0</p>\
                <ul>\
                    <li>Security and distribution hardening: verified downloads, keyring-only secrets, private/atomic config+history, LLM HTTPS enforcement + consent, resource limits, error/log redaction</li>\
                    <li>Auto-paste off by default for new installs; update check is now a setting; clear-all history asks for confirmation</li>\
                    <li>Fixed: the mini panel pasted the previous transcript when you clicked into another window mid-recording — clipboard is now set while the panel holds focus (Wayland requires this)</li>\
                    <li>Fixed: the mini-panel AI icon now appears only when auto-improve will actually run</li>\
                </ul>\
                <p>Version 1.2.0</p>\
                <ul>\
                    <li>Mini Panel: dictate into any app with a global shortcut — it transcribes, pastes into the focused app, and stays open so you can dictate again</li>\
                    <li>System tray icon and background mode: run minimized; start dictation, open, or quit from the tray</li>\
                    <li>Dictation modes: Plain, Message, Email, Note, and Code Prompt formatting</li>\
                    <li>Whisper Large v3 Turbo models (full and quantized)</li>\
                    <li>Engine selector moved to Settings → Model (“Default Engine”): choose Whisper or Cohere Transcribe</li>\
                    <li>Translate to English now also applies to the mini panel</li>\
                    <li>Fixed: auto-detect language no longer produces empty transcriptions</li>\
                    <li>Fixed: Cohere Transcribe now uses your selected language</li>\
                    <li>Fixed: recording no longer gets stuck repeating old text</li>\
                </ul>\
                <p>Version 1.1.0</p>\
                <ul>\
                    <li>Multi-backend transcription engine support</li>\
                    <li>Fixed icon display in welcome wizard</li>\
                    <li>Stability and reliability improvements</li>\
                </ul>\
                <p>Version 1.0.0 - July 2026</p>\
                <ul>\
                    <li>GPU acceleration enabled by default</li>\
                    <li>GNOME accent color support for waveform animation</li>\
                    <li>Improved UI consistency with sidebar-matching theme</li>\
                    <li>Offline transcription using Whisper (whisper.cpp)</li>\
                    <li>Multiple Whisper model sizes (Tiny to Large v3)</li>\
                    <li>Real-time confidence scoring</li>\
                    <li>Transcription history with search</li>\
                    <li>Audio device selection</li>\
                    <li>Pause/resume recording</li>\
                    <li>Save transcripts to file</li>\
                    <li>Auto-detect language</li>\
                    <li>Theme switching (System, Light, Dark)</li>\
                    <li>Custom model storage location</li>\
                    <li>Automatic update checking from GitHub</li>\
                </ul>"
            )
            .build();

        about.present(window.as_ref());
    }

    fn show_whats_new(&self) {
        let dialog = adw::Window::builder()
            .application(self)
            .title(gettext("What's New"))
            .default_width(680)
            .default_height(720)
            .modal(true)
            .build();

        if let Some(window) = self.active_window() {
            dialog.set_transient_for(Some(&window));
        }

        let toolbar = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        let title = adw::WindowTitle::builder()
            .title(gettext("What's New"))
            .subtitle(gettext("Version 1.5.0"))
            .build();
        header.set_title_widget(Some(&title));
        toolbar.add_top_bar(&header);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 24);
        content.set_margin_top(32);
        content.set_margin_bottom(32);
        content.set_margin_start(24);
        content.set_margin_end(24);

        let hero_icon = gtk::Image::from_icon_name(APP_ID);
        hero_icon.set_pixel_size(80);
        content.append(&hero_icon);

        let heading = gtk::Label::new(Some(&gettext("Speech to Text 1.5")));
        heading.add_css_class("title-1");
        heading.set_wrap(true);
        heading.set_justify(gtk::Justification::Center);
        content.append(&heading);

        let intro = gtk::Label::new(Some(&gettext(
            "A faster, safer, and more polished release for everyday dictation and transcription.",
        )));
        intro.add_css_class("dim-label");
        intro.set_wrap(true);
        intro.set_justify(gtk::Justification::Center);
        intro.set_max_width_chars(60);
        intro.set_halign(gtk::Align::Center);
        content.append(&intro);

        Self::append_whats_new_group(
            &content,
            &gettext("Faster and smoother"),
            &[
                gettext("Faster startup when launching hidden, without loading the interface or a model."),
                gettext("Bounded inference work and non-blocking audio capture keep recording responsive."),
                gettext("Live previews process only the latest audio tail and discard stale results."),
                gettext("Interrupted model downloads can resume instead of starting over."),
            ],
        );
        Self::append_whats_new_group(
            &content,
            &gettext("Safer by default"),
            &[
                gettext("Runtime, model, and sidecar downloads are verified before use."),
                gettext("The local API now enforces validation, request limits, timeouts, and safer access controls."),
                gettext("AI features use endpoint-specific consent, bounded responses, and explicit automation choices."),
                gettext("Credentials stay out of plaintext settings and release signing now fails closed."),
            ],
        );
        Self::append_whats_new_group(
            &content,
            &gettext("More reliable"),
            &[
                gettext("Pause, resume, shortcuts, language selection, onboarding, and cancellation behave consistently."),
                gettext("Outdated transcription and AI callbacks can no longer overwrite newer work."),
                gettext("Corrupt settings and history files are backed up before recovery."),
                gettext("File transcriptions are preserved in History and can be searched in full."),
            ],
        );
        Self::append_whats_new_group(
            &content,
            &gettext("Refined experience"),
            &[
                gettext("The workspace, Settings, History, Help, and model selector share a clearer visual language."),
                gettext("Current Session shows either the live preview or the latest completed transcription."),
                gettext("Navigation, keyboard behavior, light-theme cards, and compact layouts have been polished."),
                gettext("History detail views make complete transcripts easier to review and manage."),
            ],
        );

        let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
        separator.set_margin_top(8);
        separator.set_margin_bottom(8);
        content.append(&separator);

        let history_heading = gtk::Label::new(Some(&gettext("Previous releases")));
        history_heading.add_css_class("title-2");
        history_heading.set_halign(gtk::Align::Start);
        content.append(&history_heading);

        Self::append_whats_new_group(
            &content,
            "Version 1.4.0",
            &[
                gettext("Added an Open File button for transcribing WAV, MP3, FLAC, OGG, Opus, and M4A files with the same results and tools as live recordings."),
                gettext("Fixed mini-panel failures on Vulkan GPUs by always using a clean batch decode."),
                gettext("Re-enabled Whisper temperature retry so difficult audio degrades gracefully instead of failing the entire transcription."),
                gettext("Limited live transcription previews to the main window so the mini panel remains a clean batch decode."),
                gettext("Applied the configured beam size consistently to live previews."),
                gettext("Consolidated the mini panel's AI tools into an Actions menu."),
                gettext("Allowed Settings pages to use the full available content width."),
            ],
        );
        Self::append_whats_new_group(
            &content,
            "Version 1.3.0",
            &[
                gettext("Hardened downloads, secret storage, configuration, History, AI connections, resource limits, and log redaction."),
                gettext("Disabled auto-paste by default for new installs, made update checks configurable, and added confirmation before clearing History."),
                gettext("Fixed the mini panel pasting the previous transcript when focus changed during recording."),
                gettext("Made the mini-panel AI indicator appear only when automatic improvement will run."),
            ],
        );
        Self::append_whats_new_group(
            &content,
            "Version 1.2.0",
            &[
                gettext("Added the Mini Panel for dictating into any application with a global shortcut."),
                gettext("Added a system tray icon and background mode."),
                gettext("Added Plain, Message, Email, Note, and Code Prompt dictation modes."),
                gettext("Added full and quantized Whisper Large v3 Turbo models."),
                gettext("Moved the transcription engine selector to Model settings."),
                gettext("Applied Translate to English to mini-panel transcriptions."),
                gettext("Fixed empty transcriptions when automatically detecting the language."),
                gettext("Fixed Cohere Transcribe ignoring the selected language."),
                gettext("Fixed recording sessions repeating old text."),
            ],
        );
        Self::append_whats_new_group(
            &content,
            "Version 1.1.0",
            &[
                gettext("Added multiple transcription backend support."),
                gettext("Fixed icon display in the welcome wizard."),
                gettext("Improved stability and reliability."),
            ],
        );
        Self::append_whats_new_group(
            &content,
            "Version 1.0.0",
            &[
                gettext("Enabled GPU acceleration by default."),
                gettext("Added GNOME accent-color support for waveform animation."),
                gettext("Improved visual consistency with the sidebar theme."),
                gettext("Added offline transcription using Whisper."),
                gettext("Added Whisper model sizes from Tiny through Large v3."),
                gettext("Added real-time confidence scoring."),
                gettext("Added searchable transcription History."),
                gettext("Added audio-device selection."),
                gettext("Added pause and resume recording."),
                gettext("Added transcript file export."),
                gettext("Added automatic language detection."),
                gettext("Added System, Light, and Dark themes."),
                gettext("Added a configurable model storage location."),
                gettext("Added automatic update checks from GitHub."),
            ],
        );

        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        let clamp = adw::Clamp::new();
        clamp.set_maximum_size(720);
        clamp.set_tightening_threshold(560);
        clamp.set_child(Some(&content));
        scrolled.set_child(Some(&clamp));
        toolbar.set_content(Some(&scrolled));
        dialog.set_content(Some(&toolbar));
        dialog.present();
    }

    fn append_whats_new_group(container: &gtk::Box, title: &str, items: &[String]) {
        let group = adw::PreferencesGroup::builder().title(title).build();
        for item in items {
            let row = adw::ActionRow::builder().title(item).title_lines(0).build();
            let icon = gtk::Image::from_icon_name("object-select-symbolic");
            icon.add_css_class("success");
            row.add_prefix(&icon);
            group.add(&row);
        }
        container.append(&group);
    }
}

/// Short language label for the mini panel meta ("Auto" or the configured code).
fn panel_lang_label(config: &AppConfig) -> String {
    if config.auto_detect_language {
        crate::i18n::gettext("Auto")
    } else {
        config
            .language
            .clone()
            .unwrap_or_else(|| crate::i18n::gettext("Auto"))
    }
}

/// Build the LLM connection config for a preset, applying its per-preset
/// model/temperature overrides over the global connection settings.
///
/// Canonical resolver, shared with the main window (see
/// `MainWindow::resolve_llm_config_for_preset`).
pub(crate) fn resolve_llm_cfg(
    config: &AppConfig,
    preset: &crate::config::LlmPreset,
) -> crate::llm::LlmConfig {
    crate::llm::LlmConfig {
        api_url: config.llm_api_url.clone(),
        api_key: None, // loaded from the keyring inside improve_async
        model: preset
            .model
            .clone()
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| config.llm_model.clone()),
        temperature: preset.temperature.unwrap_or(config.llm_temperature),
    }
}

/// Model label for the mini panel meta (the active engine/model).
fn panel_model_label(config: &AppConfig) -> String {
    match config.backend.as_str() {
        "cohere" => "Cohere Transcribe".to_string(),
        "qwen" => "Qwen3-ASR".to_string(),
        _ => config.selected_model.clone(),
    }
}

/// Truncate `text` to at most `max_chars` characters, appending an ellipsis.
fn ellipsize_chars(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{}…", truncated)
    } else {
        truncated
    }
}
