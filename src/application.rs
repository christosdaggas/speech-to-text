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
        /// While-recording chunked-decode state for the current global
        /// dictation (whisper backend only; `None` when not dictating or when
        /// the backend decodes via a sidecar).
        pub chunked: RefCell<Option<super::ChunkedDictation>>,
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
        fn shutdown(&self) {
            // Stop any warm sidecar server (it also dies with us via
            // PDEATHSIG, but an orderly kill releases its memory immediately).
            crate::transcription::sidecar_server::shutdown_all();
            // History writes run on detached worker threads that die with the
            // process; give a dictation saved right before quitting a bounded
            // moment to reach the disk.
            crate::ui::history_page::wait_for_pending_writes(std::time::Duration::from_secs(3));
            self.parent_shutdown();
        }

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

                // Pre-warm the persistent RemoteDesktop session (grant already
                // held → never prompts) so the first auto-paste of the run
                // doesn't pay the portal handshake.
                if config.auto_paste {
                    crate::portal::paste::warm_up();
                }

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

/// While-recording chunked-decode state for one global dictation. Long
/// dictations are decoded in pause-aligned chunks as they are spoken, so the
/// post-stop wait covers only the remaining tail instead of the whole take.
/// Dictations shorter than the minimum chunk length never chunk and follow the
/// classic single-decode path unchanged.
pub struct ChunkedDictation {
    /// The dictation this state belongs to (guards every async continuation).
    generation: u64,
    /// Outcomes of successfully decoded chunks, in take order. Their
    /// `raw_text` is joined and polished once at finalize.
    results: Vec<crate::recording::DictationOutcome>,
    /// A chunk decode is in flight (at most one at a time).
    pending: bool,
    /// Chunking stopped for this take (a chunk decode failed); the unconsumed
    /// audio simply goes to the final decode.
    disabled: bool,
    /// Language pinned from the first decoded chunk so auto-detect can't
    /// switch languages between chunks of one take.
    language: Option<String>,
}

impl ChunkedDictation {
    fn new(generation: u64) -> Self {
        Self {
            generation,
            results: Vec::new(),
            pending: false,
            disabled: false,
            language: None,
        }
    }
}

/// Minimum chunk length: below this the take is decoded in one piece, so
/// short dictations behave exactly as before chunking existed.
const CHUNK_MIN_SECS: f32 = 20.0;
/// Force a cut at the quietest point once this much audio has accumulated,
/// so a non-stop talker still gets bounded chunks.
const CHUNK_MAX_SECS: f32 = 40.0;

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
        engine: crate::recording::SharedEngine,
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
        // Deliberately NOT transient-for the main window: when a focused window
        // is unmapped, Mutter hands keyboard focus to its mapped transient
        // parent in preference to the previously focused window. A transient
        // panel would therefore send the post-hide focus — and the injected
        // Ctrl+V with it — to our own main window whenever it is open in the
        // background, instead of back to the user's editor. That was the
        // "never pastes while the main window is open" bug. The cost is a
        // second taskbar entry while both windows are open; correctness wins.
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
        // Chunked while-recording decode: long dictations are decoded in
        // pause-aligned chunks as they are spoken, so the post-stop wait stays
        // roughly constant (the tail) instead of growing with the take.
        // Whisper only — the sidecar backends reload weights per invocation.
        *self.imp().chunked.borrow_mut() = if config.backend == "whisper" {
            Some(ChunkedDictation::new(generation))
        } else {
            None
        };
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
                let mut chunk_tick = 0u32;
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
                    // Dead stream (mic unplugged): auto-stop and transcribe
                    // what was captured instead of recording silence forever.
                    if let Some(err) = controller.take_stream_error() {
                        tracing::warn!("Audio stream died mid-dictation: {err}");
                        app.stop_global_dictation();
                        return glib::ControlFlow::Break;
                    }
                    panel.set_timer(controller.recording_duration_secs() as f64);
                    // Chunked decode check once a second (the peek scans audio
                    // for a pause — too heavy for every 100ms tick).
                    chunk_tick += 1;
                    if chunk_tick.is_multiple_of(10) {
                        app.maybe_chunk_decode();
                    }
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

        let mut params = DictationParams::from_config(&config);

        // Chunked-decode handover — BEFORE any early return below: the state
        // must never stay live past stop (a late chunk continuation would find
        // it, push a result, and commit a stale cut into a future capture).
        // TAKE it out entirely and move the committed chunk outcomes into the
        // tail continuation BY VALUE: at stop the committed results are final
        // (a still-in-flight chunk's late result finds no state and is
        // discarded; its audio was never committed, so the tail snapshot above
        // includes it), and owning them means a quick next dictation or a
        // Cancel can no longer destroy this take's already-decoded text.
        let chunked_results: Option<Vec<DictationOutcome>> = {
            let mut chunked = self.imp().chunked.borrow_mut();
            match chunked.take() {
                Some(st) if st.generation == generation && !st.results.is_empty() => {
                    // Pin the chunk-detected language for the tail too.
                    if params.language_code.is_none() {
                        params.language_code = st.language.clone();
                    }
                    Some(st.results)
                }
                _ => None, // nothing chunk-decoded: plain single-decode path
            }
        };

        // The backend can have changed mid-take (Settings are live). If it now
        // points at an uninstalled sidecar, delivery is abandoned — but any
        // already-decoded chunk text is real speech and must reach History.
        if (config.backend == "cohere" && !crate::transcription::cohere::cohere_ready())
            || (config.backend == "qwen" && !crate::transcription::qwen::qwen_ready())
        {
            if let Some(chunks) = chunked_results {
                self.salvage_chunked_to_history(chunks, duration_secs);
            }
            let msg = if config.backend == "cohere" {
                crate::i18n::gettext(
                    "Cohere is not set up. Go to Settings → Model to download the runtime and model.",
                )
            } else {
                crate::i18n::gettext(
                    "Qwen3-ASR is not set up. Go to Settings → Model to download the runtime and model.",
                )
            };
            panel.show_error(&msg);
            return;
        }
        // The tail's own stats cover only the remaining audio; the full take
        // duration goes to the combined outcome (correct WPM/history stats and
        // duration-weighted confidence).
        let tail_duration = if chunked_results.is_some() {
            audio.duration_secs()
        } else {
            duration_secs
        };

        // The pop-up always uses a clean batch decode (no in-decode hooks).
        // Whisper.cpp callbacks under Vulkan + GTK always-on-top compositing
        // trip -6 here, and live-segment preview adds little UX value for the
        // pop-up's short dictations. The live_transcription setting applies to
        // the main window's live loop, not this path.
        let receiver = controller.transcribe_snapshot_async(audio, params, tail_duration);

        let app_weak = self.downgrade();
        glib::spawn_future_local(async move {
            let result = receiver.recv().await;
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            if let Some(chunks) = chunked_results {
                // Combine the owned chunk outcomes with the tail. A failed
                // tail must not discard the chunks — they carry most of the
                // take.
                let tail = match result {
                    Ok(Ok(outcome)) => Some(outcome),
                    Ok(Err(msg)) => {
                        tracing::warn!("Tail decode failed ({msg}); delivering chunked text only");
                        None
                    }
                    Err(_) => {
                        tracing::warn!("Tail decode reply lost; delivering chunked text only");
                        None
                    }
                };
                let config = app.config_snapshot();
                let params = DictationParams::from_config(&config);
                let outcome = crate::recording::combine_chunked_outcomes(
                    chunks,
                    tail,
                    &params,
                    duration_secs,
                );
                let stale = app.imp().dictation_generation.get() != generation
                    || app.controller().owner() == RecordingOwner::Mini;
                if stale {
                    if !outcome.cleaned_text.is_empty() {
                        info!("Stale chunked dictation finished — saving to history only");
                        app.record_global_history(&outcome.cleaned_text, &outcome);
                    }
                    return;
                }
                app.finish_global_dictation(outcome, generation);
                return;
            }
            // Stale check: the user already started a new dictation while this
            // one was transcribing. The finished transcript must still reach
            // History — silently discarding completed speech is data loss —
            // but the panel UI and clipboard belong to the newer dictation.
            let stale = app.imp().dictation_generation.get() != generation
                || app.controller().owner() == RecordingOwner::Mini;
            match result {
                Ok(Ok(outcome)) => {
                    if stale {
                        if !outcome.cleaned_text.is_empty() {
                            info!("Stale dictation finished — saving transcript to history only");
                            app.record_global_history(&outcome.cleaned_text, &outcome);
                        }
                        return;
                    }
                    app.finish_global_dictation(outcome, generation);
                }
                Ok(Err(msg)) => {
                    if !stale {
                        app.mini_panel().show_error(&msg);
                    }
                }
                Err(_) => {
                    // The inference worker died without replying. Surface it —
                    // leaving the panel on "Transcribing" forever with no
                    // message wedges the whole session from the user's view.
                    if !stale {
                        app.mini_panel().show_error(&crate::i18n::gettext(
                            "Transcription failed unexpectedly — please try again.",
                        ));
                    }
                }
            }
        });
    }

    /// Save already-decoded chunk text to History when delivery is being
    /// abandoned (e.g. the backend was switched to an uninstalled sidecar
    /// mid-take) — decoded speech must never silently vanish.
    fn salvage_chunked_to_history(&self, chunks: Vec<DictationOutcome>, total_duration: f32) {
        if chunks.is_empty() {
            return;
        }
        let config = self.config_snapshot();
        let params = DictationParams::from_config(&config);
        let outcome =
            crate::recording::combine_chunked_outcomes(chunks, None, &params, total_duration);
        if !outcome.cleaned_text.is_empty() {
            info!("Salvaging chunk-decoded text to history");
            self.record_global_history(&outcome.cleaned_text, &outcome);
        }
    }

    /// Attempt one while-recording chunk decode: if the take has accumulated
    /// enough audio since the last committed cut AND contains a real speech
    /// pause, decode that region on the inference worker while recording
    /// continues. Called ~1/s from the dictation timer; at most one chunk
    /// decode runs at a time.
    fn maybe_chunk_decode(&self) {
        let generation = self.imp().dictation_generation.get();
        {
            let chunked = self.imp().chunked.borrow();
            let Some(st) = chunked.as_ref() else { return };
            if st.generation != generation || st.pending || st.disabled {
                return;
            }
        }
        let controller = self.controller();
        if controller.owner() != RecordingOwner::Mini {
            return;
        }
        let Some((snapshot, cut)) = controller.peek_stable_chunk(CHUNK_MIN_SECS, CHUNK_MAX_SECS)
        else {
            return;
        };
        let chunk_secs = snapshot.duration_secs();
        let config = self.config_snapshot();
        let mut params = DictationParams::from_config(&config);
        if params.language_code.is_none() {
            // Keep auto-detect consistent across the take: later chunks reuse
            // the language the first chunk detected.
            params.language_code = self
                .imp()
                .chunked
                .borrow()
                .as_ref()
                .and_then(|st| st.language.clone());
        }
        if let Some(st) = self.imp().chunked.borrow_mut().as_mut() {
            st.pending = true;
        }
        info!("Chunk decode: {chunk_secs:.1}s of audio while recording continues");
        let receiver = controller.transcribe_snapshot_async(snapshot, params, chunk_secs);
        let app_weak = self.downgrade();
        glib::spawn_future_local(async move {
            let result = receiver.recv().await;
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            let commit = {
                let mut chunked = app.imp().chunked.borrow_mut();
                // Stop already took the state (this late result's audio went,
                // uncommitted, into the tail decode — dropping it here is what
                // prevents the text from appearing twice), or a new dictation
                // replaced it: both mean this result is obsolete.
                let Some(st) = chunked.as_mut() else { return };
                if st.generation != generation {
                    return;
                }
                st.pending = false;
                match result {
                    Ok(Ok(outcome)) => {
                        if st.language.is_none() {
                            st.language = outcome.detected_language.clone();
                        }
                        st.results.push(outcome);
                        true
                    }
                    Ok(Err(msg)) if msg.starts_with("No clear speech") => {
                        // A pause-only region: commit it (nothing to say)
                        // and keep chunking.
                        true
                    }
                    _ => {
                        // Real decode failure: stop chunking; the audio was
                        // never committed, so the final decode covers it.
                        tracing::warn!("Chunk decode failed; falling back to single decode");
                        st.disabled = true;
                        false
                    }
                }
            };
            if commit {
                app.controller().commit_chunk(cut);
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
            // Improve the transcript with the active preset before delivering it,
            // but never let the LLM hold an already-finished transcript hostage:
            // the chat client allows up to 300s (cold local model loads), which
            // is far too long for the interactive dictate→paste loop. If the
            // reply doesn't arrive in time, deliver the raw transcript now and
            // apply the improvement as a variant when it eventually lands.
            const AUTO_APPLY_DELIVERY_TIMEOUT: std::time::Duration =
                std::time::Duration::from_secs(12);
            let idx = config.llm_active_preset.min(config.llm_presets.len() - 1);
            let preset = config.llm_presets[idx].clone();
            let llm_cfg = resolve_llm_cfg(&config, &preset);
            panel.show_improving();
            let rx = crate::llm::improve_async(llm_cfg, preset.system_prompt(), cleaned.clone());
            let app_weak = self.downgrade();
            let label = preset.name.clone();
            glib::spawn_future_local(async move {
                let apply_variant = |app: &Application, res: &Result<String, String>| {
                    // On success, add the improved text as the active variant; on
                    // any error the raw transcript stays active.
                    if let Ok(improved) = res {
                        if !improved.trim().is_empty() {
                            if let Some(st) = app.imp().last_result_state.borrow_mut().as_mut() {
                                st.push_variant(label.clone(), improved.trim().to_string());
                            }
                        }
                    }
                };
                let recv = rx.recv();
                futures::pin_mut!(recv);
                let timeout = glib::timeout_future(AUTO_APPLY_DELIVERY_TIMEOUT);
                futures::pin_mut!(timeout);
                match futures::future::select(recv, timeout).await {
                    futures::future::Either::Left((res, _)) => {
                        let Some(app) = app_weak.upgrade() else {
                            return;
                        };
                        if app.imp().dictation_generation.get() != generation
                            || app.controller().owner() == RecordingOwner::Mini
                        {
                            // Stale: a new dictation started meanwhile.
                            return;
                        }
                        if let Ok(res) = res {
                            apply_variant(&app, &res);
                        }
                        app.deliver_active_result();
                    }
                    futures::future::Either::Right((_, recv)) => {
                        // Timed out: deliver the raw transcript immediately…
                        {
                            let Some(app) = app_weak.upgrade() else {
                                return;
                            };
                            if app.imp().dictation_generation.get() != generation
                                || app.controller().owner() == RecordingOwner::Mini
                            {
                                return;
                            }
                            tracing::info!(
                                "LLM auto-apply exceeded {}s — delivering raw transcript",
                                AUTO_APPLY_DELIVERY_TIMEOUT.as_secs()
                            );
                            app.deliver_active_result();
                        }
                        // …and surface the improvement as a variant when (if) it
                        // arrives, without re-pasting. Deliberately no clipboard
                        // write here: this can land minutes later, and silently
                        // replacing whatever the user has copied since (or lying
                        // with a "Copied" badge while the portal still serves
                        // the raw transcript) is worse than showing the variant.
                        let Ok(res) = recv.await else { return };
                        let Some(app) = app_weak.upgrade() else {
                            return;
                        };
                        if app.imp().dictation_generation.get() != generation
                            || app.controller().owner() != RecordingOwner::None
                        {
                            return;
                        }
                        apply_variant(&app, &res);
                        app.refresh_active_result_view();
                    }
                }
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
    /// chip or raw/polished switch, when the user is interacting with the panel —
    /// the panel holds focus then, so the clipboard set actually works).
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

    /// Update the panel view to the active result WITHOUT touching the
    /// clipboard and without claiming "Copied". For asynchronous late arrivals
    /// (e.g. a slow LLM improvement) where the panel is typically unfocused: a
    /// GTK clipboard set would silently no-op on Wayland — or, worse, actually
    /// replace something the user has copied since.
    fn refresh_active_result_view(&self) {
        let text = self
            .imp()
            .last_result_state
            .borrow()
            .as_ref()
            .map(|s| s.active_text().to_string())
            .unwrap_or_default();
        *self.imp().last_text.borrow_mut() = text.clone();
        let panel = self.mini_panel();
        panel.show_result(&text, false);
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
        let generation = self.imp().dictation_generation.get();
        let rx = crate::llm::improve_async(llm_cfg, preset.system_prompt(), source);
        let app_weak = self.downgrade();
        let label = preset.name.clone();
        glib::spawn_future_local(async move {
            let res = rx.recv().await;
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            app.mini_panel().set_chips_sensitive(true);
            // Drop stale results: the user started a new dictation while the
            // LLM was working — replacing the live Recording view (and the
            // result state a newer dictation now owns) with the old result
            // would also expose "Again", which restarts capture and silently
            // discards the in-progress recording.
            if app.imp().dictation_generation.get() != generation
                || app.controller().owner() != RecordingOwner::None
            {
                return;
            }
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
                    // Dead stream: stop and transcribe what was captured.
                    if let Some(err) = controller.take_stream_error() {
                        tracing::warn!("Audio stream died mid voice edit: {err}");
                        app.stop_voice_edit();
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
        let generation = self.imp().dictation_generation.get();
        let duration_secs = controller.recording_duration_secs();
        // Detach raw audio only; the expensive conditioning (mono + sinc
        // resample of the whole capture) runs on the inference worker, not the
        // GTK thread.
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
        if audio.raw_len() == 0 {
            panel.show_error(&crate::i18n::gettext("Didn't catch an instruction."));
            self.show_active_result();
            return;
        }
        let config = self.config_snapshot();
        panel.show_transcribing(&panel_model_label(&config), &panel_lang_label(&config));

        let params = DictationParams::from_config(&config);
        let receiver = controller.transcribe_snapshot_async(audio, params, duration_secs);
        let app_weak = self.downgrade();
        glib::spawn_future_local(async move {
            let result = receiver.recv().await;
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            // Drop stale results: the user cancelled the edit or started a new
            // recording while the instruction was transcribing.
            if app.imp().dictation_generation.get() != generation
                || app.controller().owner() != RecordingOwner::None
            {
                return;
            }
            match result {
                Ok(Ok(outcome)) => app.run_voice_edit_llm(outcome.cleaned_text),
                Ok(Err(msg)) => app.mini_panel().show_error(&msg),
                Err(_) => app.mini_panel().show_error(&crate::i18n::gettext(
                    "Transcription failed unexpectedly — please try again.",
                )),
            }
        });
    }

    /// Cancel a Voice-edit capture and restore the previous result view.
    fn cancel_voice_edit(&self) {
        // Invalidate any in-flight voice-edit transcription/LLM callbacks so a
        // cancelled edit can't resurface seconds later and silently replace
        // the clipboard/result state.
        self.imp()
            .dictation_generation
            .set(self.imp().dictation_generation.get().wrapping_add(1));
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
        let generation = self.imp().dictation_generation.get();
        let rx = crate::llm::improve_async(llm_cfg, system.to_string(), user);
        let app_weak = self.downgrade();
        glib::spawn_future_local(async move {
            let res = rx.recv().await;
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            // Drop stale results: the user cancelled or started a new recording
            // while the LLM was working — applying the edit now would overwrite
            // newer state (and the clipboard) with old data.
            if app.imp().dictation_generation.get() != generation
                || app.controller().owner() != RecordingOwner::None
            {
                return;
            }
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

        if self.config_snapshot().auto_paste {
            // Auto-paste path: hide the panel so focus returns to the target
            // app, deliver via the persistent portal session (focus-
            // independent selection + injected Ctrl+V, honest outcome), then
            // re-show the panel with the result.
            let generation = self.imp().dictation_generation.get();
            self.spawn_autopaste_then_reshow(text, generation);
        } else {
            // Non-auto-paste: put the transcript on the clipboard and show it.
            // Prefer the focus-independent portal selection — a plain set_text
            // from an unfocused surface silently no-ops on Wayland (the user
            // may have clicked into another window mid-transcription). Fall
            // back to the GTK clipboard, and only claim "Copied" when the
            // panel actually holds focus, so the badge never lies.
            let generation = self.imp().dictation_generation.get();
            let app_weak = self.downgrade();
            glib::spawn_future_local(async move {
                let (tx, rx) = async_channel::bounded::<bool>(1);
                let portal_text = text.clone();
                crate::application::tokio_runtime().spawn(async move {
                    let ok = crate::portal::paste::set_clipboard_via_portal(portal_text).await;
                    let _ = tx.send(ok).await;
                });
                // Bounded wait: the serialized portal actor can stall behind a
                // session handshake or an unanswered consent dialog, and the
                // result view must still appear.
                let mut copied = Self::await_bool_with_timeout(rx, 5_000).await;
                let Some(app) = app_weak.upgrade() else {
                    return;
                };
                // The await opened a window for a new dictation to start; a
                // stale result must not repaint the live Recording view (from
                // PanelState::Result the titlebar X merely hides the panel,
                // which would leave the microphone running headless).
                if app.imp().dictation_generation.get() != generation
                    || app.controller().owner() != RecordingOwner::None
                {
                    return;
                }
                let panel = app.mini_panel();
                if !copied {
                    if let Some(display) = gtk::gdk::Display::default() {
                        display.clipboard().set_text(&text);
                        display.flush();
                        let _ = display.clipboard().read_text_future().await;
                    }
                    copied = panel.is_active();
                }
                panel.show_result(&text, copied);
                app.render_panel_result_extras();
            });
        }
    }

    /// Await a `bool` reply from the portal actor with a timeout, treating
    /// expiry (or a dropped channel) as `false`.
    async fn await_bool_with_timeout(rx: async_channel::Receiver<bool>, timeout_ms: u64) -> bool {
        let recv = rx.recv();
        futures::pin_mut!(recv);
        let timeout = glib::timeout_future(std::time::Duration::from_millis(timeout_ms));
        futures::pin_mut!(timeout);
        match futures::future::select(recv, timeout).await {
            futures::future::Either::Left((res, _)) => res.unwrap_or(false),
            futures::future::Either::Right(_) => {
                tracing::warn!("Portal clipboard call timed out");
                false
            }
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
        // Drop any chunked-decode state; in-flight chunk results are already
        // invalidated by the generation bump above.
        *self.imp().chunked.borrow_mut() = None;
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
        // Never hijack an active capture: this flow repaints the panel and
        // ultimately hides it for the paste — doing that mid-recording left a
        // live microphone running with no visible UI to stop it.
        if self.controller().owner() != RecordingOwner::None {
            info!("Transform selection ignored: a recording is in progress");
            return;
        }
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
        // Refresh the clipboard with the SHOWN transcript before pasting: the
        // honest-copied delivery paths can display a result that never reached
        // the clipboard (copied=false), and injecting Ctrl+V then would paste
        // stale content. The user just clicked the panel, so it holds focus
        // and the GTK clipboard set actually works here.
        let panel = self.mini_panel();
        let mut text = panel.transcript_text();
        if text.trim().is_empty() {
            text = self.imp().last_text.borrow().clone();
        }
        if !text.is_empty() {
            if let Some(display) = gtk::gdk::Display::default() {
                display.clipboard().set_text(&text);
                display.flush();
            }
        }
        // Hide to return focus to the target app, then paste.
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

    /// Wait until the freshly-set clipboard content is actually live and the
    /// compositor has moved keyboard focus off the (just-hidden) panel, so a
    /// synthesized Ctrl+V reads the *current* transcript in the right window.
    /// Event-driven (focus-loss notify) instead of a fixed sleep: a fixed delay
    /// races the compositor's asynchronous focus hand-back under load.
    async fn await_clipboard_ready(panel: &MiniPanel) {
        if let Some(display) = gtk::gdk::Display::default() {
            // Round-trip read forces GTK to process the pending set_selection.
            let _ = display.clipboard().read_text_future().await;
        }
        Self::wait_for_panel_inactive(panel, 400).await;
        // Short settle for the compositor to focus the target app.
        glib::timeout_future(std::time::Duration::from_millis(80)).await;
    }

    /// Hide the panel, then auto-paste the (already-set) clipboard into the
    /// now-focused app on the Tokio runtime.
    fn spawn_autopaste(&self) {
        let panel = self.mini_panel();
        glib::spawn_future_local(async move {
            Self::await_clipboard_ready(&panel).await;
            crate::application::tokio_runtime().spawn(async {
                let _ = crate::portal::paste::try_autopaste().await;
            });
        });
    }

    /// Hide the panel so keyboard focus returns to the target app, deliver the
    /// transcript into it, then re-show the panel in the result state so the
    /// user can immediately dictate again — the "dictate → paste → stay open →
    /// repeat" loop.
    ///
    /// Primary path: the persistent portal session owns the system selection
    /// focus-independently (see
    /// [`crate::portal::paste::deliver_text_via_portal`]), so the *current*
    /// transcript is pasted even when the panel never held focus — including
    /// when the user clicked into another window mid-transcription. The
    /// delivery outcome is honest: the fallback (GTK clipboard + Ctrl+V) only
    /// runs when the portal was truly unavailable, and it only injects the
    /// keystroke when the panel actually obtained focus for the clipboard set —
    /// otherwise Ctrl+V would paste the *previous* clipboard content.
    fn spawn_autopaste_then_reshow(&self, text: String, generation: u64) {
        use crate::portal::paste::DeliveryOutcome;
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
            // focused app — the injected Ctrl+V must land there, not on the
            // panel. Wait for the focus to actually leave (event-driven) rather
            // than a fixed sleep; if the panel never held focus this returns
            // immediately.
            panel.set_visible(false);
            Self::wait_for_panel_inactive(&panel, 400).await;
            glib::timeout_future(std::time::Duration::from_millis(80)).await;
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            if app.imp().dictation_generation.get() != generation {
                return;
            }

            // Primary: persistent portal session (selection + Ctrl+V). Bounded
            // wait: the serialized actor can stall behind a session handshake
            // or an unanswered (expired-grant) consent dialog, and the panel
            // must never stay hidden with the transcript unshown.
            let (tx, rx) = async_channel::bounded::<DeliveryOutcome>(1);
            let portal_text = text.clone();
            crate::application::tokio_runtime().spawn(async move {
                let out = crate::portal::paste::deliver_text_via_portal(portal_text).await;
                let _ = tx.send(out).await;
            });
            let (outcome, timed_out) = {
                let recv = rx.recv();
                futures::pin_mut!(recv);
                let timeout = glib::timeout_future(std::time::Duration::from_secs(8));
                futures::pin_mut!(timeout);
                match futures::future::select(recv, timeout).await {
                    futures::future::Either::Left((res, _)) => {
                        (res.unwrap_or(DeliveryOutcome::Unavailable), false)
                    }
                    futures::future::Either::Right(_) => {
                        tracing::warn!("Portal delivery timed out; showing the result instead");
                        (DeliveryOutcome::Unavailable, true)
                    }
                }
            };

            // The portal round-trip can take a while (expired-grant consent
            // dialog); re-check that this delivery is still current before
            // touching the panel or the clipboard.
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            if app.imp().dictation_generation.get() != generation
                || app.controller().owner() != RecordingOwner::None
            {
                return;
            }

            // Pasted/NotConsumed: the portal session owns the selection with the
            // current transcript and keeps serving it, so the clipboard is good
            // either way; only an actually-consumed paste counts as pasted.
            let mut copied = matches!(
                outcome,
                DeliveryOutcome::Pasted | DeliveryOutcome::NotConsumed
            );

            // On timeout, skip the fallback injection entirely: the actor may
            // still be mid-delivery (e.g. a consent dialog answered late), and
            // a second keystroke on top of its eventual Ctrl+V would paste
            // twice. The result view below keeps the transcript reachable.
            if outcome == DeliveryOutcome::Unavailable && !timed_out {
                // Fallback (no portal / no grant): set the GTK clipboard while
                // the panel holds focus, then inject Ctrl+V — but ONLY if focus
                // was actually obtained. present() from a background app can be
                // denied by focus-stealing prevention, and set_text from an
                // unfocused surface silently no-ops on Wayland; injecting then
                // would paste stale content.
                let panel = app.mini_panel();
                panel.set_visible(true);
                panel.present();
                Self::wait_for_panel_active(&panel, 600).await;
                if panel.is_active() {
                    if let Some(display) = gtk::gdk::Display::default() {
                        display.clipboard().set_text(&text);
                        display.flush();
                        let _ = display.clipboard().read_text_future().await;
                    }
                    copied = true;
                    panel.set_visible(false);
                    Self::wait_for_panel_inactive(&panel, 400).await;
                    glib::timeout_future(std::time::Duration::from_millis(80)).await;
                    // Re-check once more before injecting the keystroke.
                    let Some(app) = app_weak.upgrade() else {
                        return;
                    };
                    if app.imp().dictation_generation.get() != generation
                        || app.controller().owner() != RecordingOwner::None
                    {
                        return;
                    }
                    let (done_tx, done_rx) = async_channel::bounded::<()>(1);
                    crate::application::tokio_runtime().spawn(async move {
                        let _ = crate::portal::paste::try_autopaste().await;
                        let _ = done_tx.send(()).await;
                    });
                    let _ = done_rx.recv().await;
                }
                // If focus was never obtained the transcript is still shown in
                // the result panel below (with an honest not-copied badge), so
                // nothing is silently lost.
            }

            // Re-show the panel with the transcript, unless the user has
            // already started a new recording in the meantime. No present():
            // an activation request would steal focus back from the app the
            // text was just pasted into. Display the CURRENT active text — a
            // late LLM improvement may have landed while the delivery ran, and
            // showing the captured raw text would contradict the variant
            // selector rendered by render_panel_result_extras.
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            if app.imp().dictation_generation.get() != generation
                || app.controller().owner() == RecordingOwner::Mini
            {
                return;
            }
            let display_text = app
                .imp()
                .last_result_state
                .borrow()
                .as_ref()
                .map(|s| s.active_text().to_string())
                .unwrap_or_else(|| text.clone());
            let panel = app.mini_panel();
            panel.set_visible(true);
            panel.show_result(&display_text, copied);
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

    /// Wait until the mini panel surface is NOT active (keyboard focus has
    /// left it), up to `timeout_ms`. Used after hiding the panel: the
    /// compositor hands focus back asynchronously, and injecting Ctrl+V before
    /// that completes pastes into the wrong (or no) surface. Returns
    /// immediately when the panel never held focus.
    async fn wait_for_panel_inactive(panel: &MiniPanel, timeout_ms: u64) {
        if !panel.is_active() {
            return;
        }
        let (tx, rx) = async_channel::bounded::<()>(1);
        let handler = panel.connect_is_active_notify({
            let tx = tx.clone();
            move |p| {
                if !p.is_active() {
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

        // The full MIT license text, shown verbatim in the Legal section
        // (mirrors the repository's LICENSE file).
        const MIT_LICENSE: &str = "MIT License\n\n\
            Copyright (c) 2026 Christos A. Daggas\n\n\
            Permission is hereby granted, free of charge, to any person obtaining a copy \
            of this software and associated documentation files (the \"Software\"), to deal \
            in the Software without restriction, including without limitation the rights \
            to use, copy, modify, merge, publish, distribute, sublicense, and/or sell \
            copies of the Software, and to permit persons to whom the Software is \
            furnished to do so, subject to the following conditions:\n\n\
            The above copyright notice and this permission notice shall be included in all \
            copies or substantial portions of the Software.\n\n\
            THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR \
            IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, \
            FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE \
            AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER \
            LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, \
            OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE \
            SOFTWARE.";

        // No release notes here on purpose — What's New has its own item in
        // the main menu. The website link lives on the developer row in
        // Credits rather than in Details.
        let about = adw::AboutDialog::builder()
            .application_name(APP_NAME)
            .application_icon(APP_ID)
            .developer_name("Christos A. Daggas")
            .version(VERSION)
            .copyright("© 2026 Christos A. Daggas")
            .license(MIT_LICENSE)
            .issue_url("https://github.com/christosdaggas/speech-to-text/issues")
            .developers(vec!["Christos A. Daggas https://chrisdaggas.com"])
            .comments(
                "Speech to Text turns your voice into text, entirely on your device. \
                 Dictate into any application with a global shortcut, transcribe audio \
                 files, and shape the result with dictation modes, translation, and \
                 optional AI polish. Whisper, Qwen3-ASR, and Cohere Transcribe all run \
                 locally — nothing you say leaves your computer.",
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
            .subtitle(gettext("Version 1.6.0"))
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

        let heading = gtk::Label::new(Some(&gettext("Speech to Text 1.6")));
        heading.add_css_class("title-1");
        heading.set_wrap(true);
        heading.set_justify(gtk::Justification::Center);
        content.append(&heading);

        let intro = gtk::Label::new(Some(&gettext(
            "Reliable paste, instant delivery, and heavy lifting while you speak.",
        )));
        intro.add_css_class("dim-label");
        intro.set_wrap(true);
        intro.set_justify(gtk::Justification::Center);
        intro.set_max_width_chars(60);
        intro.set_halign(gtk::Align::Center);
        content.append(&intro);

        // Current release: one continuous card, no section headings (an empty
        // group title renders no header), matching the release cards below.
        Self::append_whats_new_group(
            &content,
            "",
            &[
                gettext("Auto-paste now works when the main window is open in the background — keyboard focus returns to your editor, not to the app."),
                gettext("A persistent desktop portal session removes about a second of overhead from every paste."),
                gettext("The paste permission is requested once, from Settings — never in the middle of a dictation."),
                gettext("The Copied badge appears only when the text really is on the clipboard."),
                gettext("Long dictations are transcribed in the background while you speak, so the wait after Stop stays short."),
                gettext("Qwen3-ASR and Cohere keep their models loaded between dictations instead of reloading gigabytes every time."),
                gettext("Recording starts instantly — the microphone device is cached between takes."),
                gettext("AI improvement no longer delays delivery: the raw transcript arrives first and the polished variant follows."),
                gettext("A finished transcription is always saved to History, even if you have already started the next one."),
                gettext("The global shortcut re-registers itself if the desktop portal restarts."),
                gettext("A disconnected microphone stops the recording and transcribes what was captured."),
                gettext("Transcription errors are shown instead of leaving the panel stuck on Transcribing."),
                gettext("Provider metadata downloads are size-capped, and redirects can no longer downgrade to plaintext."),
                gettext("Secret redaction now catches unprefixed tokens, across line breaks."),
                gettext("The health endpoint exposes less, and language parameters are strictly validated."),
                gettext("History and settings are written off the UI thread with safe ordering."),
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
            "Version 1.5.0",
            &[
                gettext("Faster hidden startup, bounded inference work, and non-blocking audio capture."),
                gettext("Verified, resumable runtime and model downloads; keyring-only credentials."),
                gettext("Endpoint-specific AI consent, request limits, and safer access controls for the local API."),
                gettext("Expanded History with full-text search and file-transcription recovery."),
                gettext("A refined workspace: Settings, History, Help, and the model selector share a clearer visual language."),
            ],
        );
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
