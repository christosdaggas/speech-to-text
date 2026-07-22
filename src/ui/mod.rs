// Speech to Text - UI Module
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! User interface components.

mod controls;
mod header;
mod help_page;
pub mod history_page;
mod main_window;
mod mini_panel;
pub mod result_state;
mod sidebar;
mod status_bar;
mod transcript_view;
mod welcome_wizard;

pub mod settings;
pub mod widgets;

pub use controls::{ControlAction, Controls};
pub use header::HeaderControls;
pub use help_page::HelpPage;
pub use history_page::HistoryPage;
pub use main_window::MainWindow;
pub use mini_panel::{MiniPanel, MiniPanelAction};
pub use status_bar::StatusBar;
pub use transcript_view::TranscriptView;
pub use welcome_wizard::WelcomeWizard;
