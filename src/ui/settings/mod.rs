// Speech to Text - Settings Module
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Settings pages for the sidebar navigation.

use gtk4 as gtk;
use gtk4::prelude::*;
use libadwaita as adw;

pub mod api;
pub mod dictation;
pub mod dictionary;
pub mod language;
pub mod llm;
pub mod microphone;
pub mod model;
pub mod performance;

pub use api::ApiPage;
pub use dictation::DictationPage;
pub use dictionary::DictionaryPage;
pub use language::language_code_to_name;
pub use language::LanguagePage;
pub use llm::LlmPage;
pub use microphone::MicrophonePage;
pub use model::ModelPage;
pub use performance::PerformancePage;

/// Effectively-unlimited clamp width: large enough that no monitor caps the
/// content, small enough to avoid arithmetic overflow inside libadwaita's
/// allocation maths.
const UNLIMITED_CLAMP: i32 = 1_000_000;

/// Coalesces config saves from text inputs: runs the last scheduled action
/// once, `delay` after the LAST `schedule()` call. A per-keystroke
/// `AppConfig::save()` costs a full serialize + fsync on the GTK thread
/// (~10 fsyncs/second while typing), which made these entries visibly lag.
/// Call `flush()` when the widget unmaps so a pending save can't be lost.
#[derive(Clone)]
pub struct SaveDebouncer {
    inner: std::rc::Rc<std::cell::RefCell<DebouncerState>>,
    delay: std::time::Duration,
}

#[derive(Default)]
struct DebouncerState {
    source: Option<gtk::glib::SourceId>,
    action: Option<Box<dyn FnOnce()>>,
}

impl SaveDebouncer {
    pub fn new(delay_ms: u64) -> Self {
        Self {
            inner: std::rc::Rc::new(std::cell::RefCell::new(DebouncerState::default())),
            delay: std::time::Duration::from_millis(delay_ms),
        }
    }

    /// Replace any pending action with `action` and restart the delay.
    pub fn schedule(&self, action: Box<dyn FnOnce() + 'static>) {
        let mut st = self.inner.borrow_mut();
        if let Some(id) = st.source.take() {
            id.remove();
        }
        st.action = Some(action);
        let inner = self.inner.clone();
        st.source = Some(gtk::glib::timeout_add_local_once(self.delay, move || {
            let action = {
                let mut st = inner.borrow_mut();
                st.source = None;
                st.action.take()
            };
            if let Some(action) = action {
                action();
            }
        }));
    }

    /// Run any pending action immediately.
    pub fn flush(&self) {
        let action = {
            let mut st = self.inner.borrow_mut();
            if let Some(id) = st.source.take() {
                id.remove();
            }
            st.action.take()
        };
        if let Some(action) = action {
            action();
        }
    }
}

/// libadwaita wraps every `AdwPreferencesPage` in an internal `AdwClampScrollable`
/// that caps its content at 600px and centres it in a narrow column. The rest of
/// this app lays content out full-width, so walk the page's widget tree and lift
/// the cap on every clamp it finds — the preference groups then fill the whole
/// available width of the content area.
pub fn fill_preferences_width(page: &impl IsA<gtk::Widget>) {
    page.add_css_class("settings-page");

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
