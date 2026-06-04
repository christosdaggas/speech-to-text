// Speech to Text - Main Entry Point
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Speech to Text - A GTK4/Libadwaita offline speech-to-text application.
//!
//! Uses Whisper (via whisper.cpp) for local transcription with GPU acceleration support.

use gtk4::prelude::*;
use gtk4::glib;

mod application;
mod audio;
mod config;
mod error;
mod i18n;
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

    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    tracing::info!("Starting {} v{}", APP_NAME, VERSION);

    // Initialize translations
    let locale_dir = option_env!("LOCALE_DIR").unwrap_or("/usr/share/locale");
    gettextrs::setlocale(gettextrs::LocaleCategory::LcAll, "");
    gettextrs::bindtextdomain("speech-to-text", locale_dir).ok();
    gettextrs::textdomain("speech-to-text").ok();

    // Create and run the application
    let app = Application::new();
    app.run()
}
