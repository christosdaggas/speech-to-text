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
use std::sync::Arc;
use tracing::info;

use crate::config::AppConfig;
use crate::ui::MainWindow;
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

            let window = if let Some(window) = application.active_window() {
                window
            } else {
                let config = self.config.borrow().clone()
                    .unwrap_or_else(|| Arc::new(AppConfig::default()));
                let window = MainWindow::new(&*application, config);
                window.upcast()
            };

            window.present();
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

        self.add_action_entries([action_quit, action_about, action_whats_new]);
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
                "<p>Version 1.0.0 - July 2026</p>\
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
                "<p>Version 1.0.0 - July 2026</p>\
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
