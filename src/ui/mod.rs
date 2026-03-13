// Speech to Text - UI Module
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! User interface components.

mod main_window;
mod sidebar;
mod header;
mod transcript_view;
mod controls;
mod history_page;
mod help_page;
mod welcome_wizard;
mod status_bar;

pub mod settings;
pub mod widgets;

pub use main_window::MainWindow;
pub use header::HeaderControls;
pub use transcript_view::TranscriptView;
pub use controls::{Controls, ControlAction};
pub use history_page::HistoryPage;
pub use help_page::HelpPage;
pub use welcome_wizard::WelcomeWizard;
pub use status_bar::StatusBar;
