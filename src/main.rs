// Speech to Text - Main Entry Point
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Speech to Text - A GTK4/Libadwaita offline speech-to-text application.
//!
//! Uses Whisper (via whisper.cpp) for local transcription with GPU acceleration support.

use gtk4::prelude::*;
use gtk4::glib;

mod api;
mod application;
mod audio;
mod config;
mod error;
mod fsio;
mod i18n;
mod limits;
mod llm;
mod portal;
mod recording;
mod secrets;
mod transcription;
mod tray;
mod ui;
mod version_check;

use application::Application;

/// Application ID for GNOME/Freedesktop
pub const APP_ID: &str = "com.chrisdaggas.speech-to-text";
/// Human-readable application name
pub const APP_NAME: &str = "Speech to Text";
/// Application version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> glib::ExitCode {
    // Initialize the global Tokio runtime for async tasks
    let runtime = application::tokio_runtime();
    let _guard = runtime.enter();

    // Set the program name (critical for Wayland window matching)
    glib::set_prgname(Some(APP_ID));
    glib::set_application_name(APP_NAME);

    // Initialize logging. Default: our crate at INFO, dependencies at WARN, so
    // routine logs stay readable and no third-party crate dumps request/response
    // detail. Transcript text and secrets are never logged at any level; set
    // RUST_LOG (e.g. `RUST_LOG=debug`) for verbose diagnostics when troubleshooting.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,speech_to_text=info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
    tracing::info!("Starting {} v{}", APP_NAME, VERSION);

    // Initialize translations
    let locale_dir = option_env!("LOCALE_DIR").unwrap_or("/usr/share/locale");
    gettextrs::setlocale(gettextrs::LocaleCategory::LcAll, "");
    gettextrs::bindtextdomain("speech-to-text", locale_dir).ok();
    gettextrs::textdomain("speech-to-text").ok();

    // The autostart-only `--hidden` flag must NOT reach GApplication (FLAGS_NONE
    // would reject the unknown option), so detect it here and hand `run` an argv
    // without it. A manual launch (no flag) always shows the main window.
    let args: Vec<String> = std::env::args().collect();
    let hidden = args.iter().any(|a| a == "--hidden");
    let _ = application::LAUNCH_HIDDEN.set(hidden);
    let argv: Vec<String> = args.into_iter().filter(|a| a != "--hidden").collect();

    // Create and run the application
    let app = Application::new();
    app.run_with_args(&argv)
}
