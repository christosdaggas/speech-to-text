// Speech to Text - Application
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Main Application.
//!
//! GObject subclass for the Adwaita Application.

use gtk4::prelude::*;
use gtk4::gio;
use gtk4::glib;
use gtk4 as gtk;
use libadwaita as adw;
use adw::prelude::*;
use adw::subclass::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use tracing::info;

use crate::audio::capture::RecordingState;
use crate::config::AppConfig;
use crate::recording::{
    DictationMode, DictationOutcome, DictationParams, RecordingController, RecordingOwner,
};
use crate::ui::{MainWindow, MiniPanel, MiniPanelAction};
use crate::{APP_ID, APP_NAME, VERSION};

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
        /// Whether the first `activate` has happened (so re-launch always shows
        /// the window even when `start_hidden` is set).
        pub started: std::cell::Cell<bool>,
        /// Keeps the application alive in the background (no window needed).
        /// Dropping this guard releases the hold, so it lives for the app's life.
        pub hold_guard: RefCell<Option<gio::ApplicationHoldGuard>>,
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
        }
    }

    impl ApplicationImpl for Application {
        fn activate(&self) {
            let application = self.obj();

            // Find the existing main window (it may be hidden in the tray) or
            // create it. Don't rely on active_window(): a hidden window — or the
            // mini panel being the active one — would make it return the wrong
            // thing and spawn a duplicate.
            let window = application.main_window().unwrap_or_else(|| {
                let config = self.config.borrow().clone().unwrap_or_else(|| Arc::new(AppConfig::load()));
                *self.config.borrow_mut() = Some(config.clone());
                MainWindow::new(&application, config)
            });

            // Honor "start hidden" only on the very first activation; any later
            // activation (re-launch, tray "Open") always shows the window.
            let start_hidden = self.config.borrow()
                .as_ref()
                .map(|c| c.start_hidden)
                .unwrap_or(false);
            if self.started.get() || !start_hidden {
                window.present();
            }
            self.started.set(true);
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
                            if let Some(path_str) = dev_icons.canonicalize().ok()
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

            // Register the global dictation shortcut via the portal. Best-effort:
            // failures are logged and the app keeps working with in-app controls.
            if config.mini_panel_enabled {
                let (tx, rx) = async_channel::bounded::<()>(4);
                let trigger = config.global_shortcut.clone();
                crate::application::tokio_runtime()
                    .spawn(crate::portal::shortcuts::run_global_shortcuts(trigger, tx));

                let app_weak = self.obj().downgrade();
                glib::spawn_future_local(async move {
                    while rx.recv().await.is_ok() {
                        let Some(app) = app_weak.upgrade() else { break };
                        app.activate_action("start-global-dictation", None);
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
        self.windows().into_iter().find_map(|w| w.downcast::<MainWindow>().ok())
    }

    /// Show the main window (creating it if it doesn't exist yet).
    fn present_main_window(&self) {
        let window = self.main_window().unwrap_or_else(|| {
            let config = self.imp().config.borrow().clone().unwrap_or_else(|| Arc::new(AppConfig::load()));
            MainWindow::new(self, config)
        });
        window.present();
    }

    fn on_tray_action(&self, action: crate::tray::TrayAction) {
        use crate::tray::TrayAction;
        match action {
            TrayAction::Dictate => self.toggle_global_dictation(),
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
        let app_weak = self.downgrade();
        panel.connect_action(move |action| {
            if let Some(app) = app_weak.upgrade() {
                app.on_mini_panel_action(action);
            }
        });
        *self.imp().mini_panel.borrow_mut() = Some(panel.clone());
        panel
    }

    fn on_mini_panel_action(&self, action: MiniPanelAction) {
        match action {
            MiniPanelAction::Stop => self.stop_global_dictation(),
            MiniPanelAction::Cancel => self.cancel_global_dictation(),
            // "New": start a fresh recording reusing the already-open panel.
            MiniPanelAction::Again => self.start_global_dictation(),
            MiniPanelAction::Paste => self.paste_preview_text(),
            MiniPanelAction::Copy => self.copy_preview_text(),
            MiniPanelAction::Close => self.close_mini_panel(),
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
            RecordingOwner::None => self.start_global_dictation(),
        }
    }

    fn start_global_dictation(&self) {
        let controller = self.controller();
        if !controller.try_acquire(RecordingOwner::Mini) {
            return;
        }

        let config = self.config_snapshot();
        let mode = DictationMode::from_config_str(&config.dictation_mode);
        let panel = self.mini_panel();
        panel.show_recording(&mode_display_label(mode));
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
                glib::timeout_add_seconds_local(1, move || {
                    let (Some(app), Some(panel)) = (app_weak.upgrade(), panel_weak.upgrade()) else {
                        return glib::ControlFlow::Break;
                    };
                    let controller = app.controller();
                    if controller.owner() != RecordingOwner::Mini
                        || controller.state() == RecordingState::Idle
                    {
                        return glib::ControlFlow::Break;
                    }
                    panel.set_timer(controller.recording_duration_secs() as u64);
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

        let audio = match controller.stop() {
            Ok(a) => a,
            Err(e) => {
                controller.release();
                self.mini_panel().show_error(&format!("Error stopping recording: {e}"));
                return;
            }
        };
        controller.release();

        let panel = self.mini_panel();
        if audio.is_empty() {
            panel.show_error(&crate::i18n::gettext("No clear speech detected — try again"));
            return;
        }
        panel.show_transcribing();

        let config = self.config_snapshot();
        if config.backend == "cohere" && !crate::transcription::cohere::cohere_ready() {
            panel.show_error(&crate::i18n::gettext(
                "Cohere is not set up. Go to Settings → Model to download the runtime and model.",
            ));
            return;
        }

        let params = DictationParams::from_config(&config);
        let receiver = controller.transcribe_async(audio, params);

        let app_weak = self.downgrade();
        glib::spawn_future_local(async move {
            let result = receiver.recv().await;
            let Some(app) = app_weak.upgrade() else { return };
            // If the user already started a new dictation while this one was
            // transcribing, drop this (stale) result so it doesn't overwrite the
            // live recording UI.
            if app.controller().owner() == RecordingOwner::Mini {
                return;
            }
            match result {
                Ok(Ok(outcome)) => app.finish_global_dictation(outcome),
                Ok(Err(msg)) => app.mini_panel().show_error(&msg),
                Err(_) => {}
            }
        });
    }

    fn finish_global_dictation(&self, outcome: DictationOutcome) {
        let panel = self.mini_panel();
        let cleaned = outcome.cleaned_text.clone();
        if cleaned.is_empty() {
            panel.show_error(&crate::i18n::gettext("No clear speech detected — try again"));
            return;
        }

        // Clipboard is always set — the reliable fallback.
        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(&cleaned);
        }
        *self.imp().last_text.borrow_mut() = cleaned.clone();
        self.record_global_history(&cleaned, &outcome);

        if self.config_snapshot().auto_paste {
            // Hide so focus returns to the previous app, paste into it, then
            // re-show the panel so the user can immediately dictate again ("New").
            panel.set_visible(false);
            self.spawn_autopaste_then_reshow(cleaned.clone());
        } else {
            panel.show_result(&cleaned, true);
        }
        info!(
            "Global dictation complete ({:.0}% confidence)",
            outcome.confidence * 100.0
        );
    }

    fn record_global_history(&self, text: &str, outcome: &DictationOutcome) {
        let config = self.config_snapshot();
        let lang_name = if config.auto_detect_language {
            outcome.detected_language.as_deref()
                .map(|c| format!("Auto-detect ({})", crate::ui::settings::language_code_to_name(c)))
                .unwrap_or_else(|| "Auto-detect".to_string())
        } else {
            config.language.as_deref()
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
            duration_secs: 0,
            timestamp: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
            model,
        };

        // Route through the live HistoryPage when the main window is open
        // (keeps memory + disk in sync); otherwise append to disk directly.
        if let Some(win) = self.main_window() {
            win.add_history_entry(entry);
        } else {
            crate::ui::history_page::append_entry_to_disk(&entry);
        }
    }

    fn cancel_global_dictation(&self) {
        let controller = self.controller();
        if controller.owner() == RecordingOwner::Mini {
            controller.cancel();
            controller.release();
        }
        self.close_mini_panel();
    }

    fn paste_preview_text(&self) {
        // The clipboard already holds the text; hide to return focus, then paste.
        self.close_mini_panel();
        self.spawn_autopaste();
    }

    fn copy_preview_text(&self) {
        let text = self.imp().last_text.borrow().clone();
        if !text.is_empty() {
            if let Some(display) = gtk::gdk::Display::default() {
                display.clipboard().set_text(&text);
            }
        }
    }

    fn close_mini_panel(&self) {
        if let Some(panel) = self.imp().mini_panel.borrow().as_ref() {
            panel.set_visible(false);
        }
    }

    /// Hide the panel, then (after a short delay so focus returns to the target
    /// app) attempt a best-effort auto-paste on the Tokio runtime.
    fn spawn_autopaste(&self) {
        glib::timeout_add_local_once(std::time::Duration::from_millis(120), || {
            crate::application::tokio_runtime().spawn(async {
                let _ = crate::portal::paste::try_autopaste().await;
            });
        });
    }

    /// Hide the panel, paste into the now-focused app, then re-present the panel
    /// in the result state so the user can immediately dictate again — the
    /// "dictate → paste → stay open → repeat" loop.
    fn spawn_autopaste_then_reshow(&self, text: String) {
        let (done_tx, done_rx) = async_channel::bounded::<()>(1);
        // Short delay so focus returns to the target window before we paste.
        glib::timeout_add_local_once(std::time::Duration::from_millis(120), move || {
            crate::application::tokio_runtime().spawn(async move {
                let _ = crate::portal::paste::try_autopaste().await;
                let _ = done_tx.send(()).await;
            });
        });
        // Once the paste finishes, bring the panel back showing the transcript.
        let app_weak = self.downgrade();
        glib::spawn_future_local(async move {
            let _ = done_rx.recv().await;
            let Some(app) = app_weak.upgrade() else { return };
            // Don't clobber a new recording the user may have started meanwhile.
            if app.controller().owner() == RecordingOwner::Mini {
                return;
            }
            let panel = app.mini_panel();
            panel.present();
            panel.show_result(&text, true);
        });
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

        self.add_action_entries([action_quit, action_about, action_whats_new, action_dictation]);
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
                "<p>Version 1.2.0</p>\
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
        let window = self.active_window();

        let dialog = adw::AboutDialog::builder()
            .application_name(format!("What's New in {}", APP_NAME))
            .application_icon(APP_ID)
            .version(VERSION)
            .release_notes(
                "<p>Version 1.2.0</p>\
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

        dialog.present(window.as_ref());
    }
}

/// Human-readable label for a dictation mode chip.
fn mode_display_label(mode: DictationMode) -> String {
    use crate::i18n::gettext;
    match mode {
        DictationMode::Plain => gettext("Plain"),
        DictationMode::Message => gettext("Message"),
        DictationMode::Email => gettext("Email"),
        DictationMode::Note => gettext("Note"),
        DictationMode::CodePrompt => gettext("Code Prompt"),
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
