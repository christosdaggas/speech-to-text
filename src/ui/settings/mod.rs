// Speech to Text - Settings Module
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Settings pages for the sidebar navigation.

use gtk4 as gtk;
use gtk4::prelude::*;
use libadwaita as adw;

pub mod microphone;
pub mod model;
pub mod language;
pub mod performance;
pub mod dictation;
pub mod dictionary;
pub mod llm;
pub mod api;

pub use microphone::MicrophonePage;
pub use model::ModelPage;
pub use language::LanguagePage;
pub use language::language_code_to_name;
pub use performance::PerformancePage;
pub use dictation::DictationPage;
pub use dictionary::DictionaryPage;
pub use llm::LlmPage;
pub use api::ApiPage;

/// Effectively-unlimited clamp width: large enough that no monitor caps the
/// content, small enough to avoid arithmetic overflow inside libadwaita's
/// allocation maths.
const UNLIMITED_CLAMP: i32 = 1_000_000;

/// libadwaita wraps every `AdwPreferencesPage` in an internal `AdwClampScrollable`
/// that caps its content at 600px and centres it in a narrow column. The rest of
/// this app lays content out full-width, so walk the page's widget tree and lift
/// the cap on every clamp it finds — the preference groups then fill the whole
/// available width of the content area.
pub fn fill_preferences_width(page: &impl IsA<gtk::Widget>) {
    fn widen(widget: &gtk::Widget) {
        if let Some(clamp) = widget.downcast_ref::<adw::ClampScrollable>() {
            clamp.set_maximum_size(UNLIMITED_CLAMP);
            clamp.set_tightening_threshold(UNLIMITED_CLAMP);
        } else if let Some(clamp) = widget.downcast_ref::<adw::Clamp>() {
            clamp.set_maximum_size(UNLIMITED_CLAMP);
            clamp.set_tightening_threshold(UNLIMITED_CLAMP);
        }
        let mut child = widget.first_child();
        while let Some(c) = child {
            widen(&c);
            child = c.next_sibling();
        }
    }
    widen(page.upcast_ref::<gtk::Widget>());
}
