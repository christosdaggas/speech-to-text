// Speech to Text - Controls Panel
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Record / Pause / Stop controls and action buttons (Copy, Clear, Save).

use gtk4::prelude::*;
use crate::i18n::gettext;
use gtk4::glib;
use gtk4 as gtk;
use libadwaita as adw;
use adw::subclass::prelude::*;
use std::cell::RefCell;

/// Signals emitted by the controls panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlAction {
    Record,
    Pause,
    Resume,
    Stop,
    Cancel,
    Copy,
    Clear,
    Save,
    OpenFile,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Controls {
        pub record_btn: RefCell<Option<gtk::Button>>,
        pub pause_btn: RefCell<Option<gtk::Button>>,
        pub stop_btn: RefCell<Option<gtk::Button>>,
        pub cancel_btn: RefCell<Option<gtk::Button>>,
        pub open_file_btn: RefCell<Option<gtk::Button>>,
        pub copy_btn: RefCell<Option<gtk::Button>>,
        pub clear_btn: RefCell<Option<gtk::Button>>,
        pub save_btn: RefCell<Option<gtk::Button>>,
        pub translate_toggle: RefCell<Option<gtk::ToggleButton>>,
        pub ai_toggle: RefCell<Option<gtk::ToggleButton>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Controls {
        const NAME: &'static str = "SttControls";
        type Type = super::Controls;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for Controls {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_ui();
        }
    }

    impl WidgetImpl for Controls {}
    impl BoxImpl for Controls {}
}

glib::wrapper! {
    pub struct Controls(ObjectSubclass<imp::Controls>)
        @extends gtk::Widget, gtk::Box;
}

impl Controls {
    pub fn new() -> Self {
        // NOTE: the `spacing` set here (a builder property) is the one that
        // actually sticks — it is applied AFTER `constructed()`, so a later
        // `set_spacing()` in `setup_ui` would be overridden. Set it here.
        glib::Object::builder()
            .property("orientation", gtk::Orientation::Horizontal)
            .property("spacing", 18)
            .build()
    }

    fn setup_ui(&self) {
        let imp = self.imp();

        self.add_css_class("controls-panel");
        self.set_margin_start(16);
        self.set_margin_end(16);
        self.set_margin_top(4);
        self.set_margin_bottom(12);
        // Centred row of buttons. Within-group spacing comes from the box
        // `spacing` set in `new()` (18px); groups get an extra margin gap below.
        self.set_halign(gtk::Align::Center);
        self.set_spacing(18);

        /// Build a uniform icon + label button.
        fn labelled(icon: &str, label: &str) -> gtk::Button {
            let btn = gtk::Button::new();
            let content = adw::ButtonContent::new();
            content.set_icon_name(icon);
            content.set_label(label);
            btn.set_child(Some(&content));
            btn
        }

        // ── Open file (alternative to recording) ──
        let open_file_btn = labelled("document-open-symbolic", &gettext("Open File"));
        open_file_btn.set_tooltip_text(Some(
            gettext("Open an audio file (WAV, MP3, FLAC, OGG, Opus, M4A) and transcribe it").as_str()
        ));
        self.append(&open_file_btn);

        // ── Transport (Record · Pause · Stop · Cancel) ──
        // Only Record is accent (blue) and only Cancel is destructive (red);
        // Pause/Stop stay neutral.
        let record_btn = labelled("media-record-symbolic", &gettext("Record"));
        record_btn.add_css_class("suggested-action");
        record_btn.set_tooltip_text(Some(gettext("Start recording (Ctrl+R)").as_str()));

        let pause_btn = labelled("media-playback-pause-symbolic", &gettext("Pause"));
        pause_btn.set_tooltip_text(Some(gettext("Pause recording").as_str()));
        pause_btn.set_sensitive(false);

        let stop_btn = labelled("media-playback-stop-symbolic", &gettext("Stop"));
        stop_btn.set_tooltip_text(Some(gettext("Stop recording and transcribe").as_str()));
        stop_btn.set_sensitive(false);

        let cancel_btn = labelled("process-stop-symbolic", &gettext("Cancel"));
        cancel_btn.add_css_class("destructive-action");
        cancel_btn.set_tooltip_text(Some(gettext("Cancel recording and discard").as_str()));
        cancel_btn.set_sensitive(false);

        self.append(&record_btn);
        self.append(&pause_btn);
        self.append(&stop_btn);
        self.append(&cancel_btn);

        // ── Modes (Translate · Improve with AI), as toggles ──
        let translate_toggle = gtk::ToggleButton::new();
        let translate_content = adw::ButtonContent::new();
        translate_content.set_icon_name("preferences-desktop-locale-symbolic");
        translate_content.set_label(gettext("Translate").as_str());
        translate_toggle.set_child(Some(&translate_content));
        translate_toggle.set_tooltip_text(Some(gettext("Translate to English").as_str()));
        translate_toggle.add_css_class("translate-toggle"); // vivid bg when active
        translate_toggle.set_margin_start(16); // extra gap between groups (adds to box spacing)
        self.append(&translate_toggle);

        // Improve with AI: when active, the NEXT transcriptions are auto-improved
        // with the active LLM preset. Hidden until the LLM integration is enabled.
        let ai_toggle = gtk::ToggleButton::new();
        let ai_content = adw::ButtonContent::new();
        ai_content.set_icon_name("com.chrisdaggas.speech-to-text-ai");
        ai_content.set_label(gettext("Improve with AI").as_str());
        ai_toggle.set_child(Some(&ai_content));
        ai_toggle.set_tooltip_text(Some(gettext("Improve the next transcriptions with the LLM (active preset)").as_str()));
        ai_toggle.add_css_class("ai-toggle"); // vivid bg when active
        ai_toggle.set_visible(false);
        self.append(&ai_toggle);

        // ── Actions (Copy · Clear · Save) — icon-only ──
        let copy_btn = gtk::Button::from_icon_name("edit-copy-symbolic");
        copy_btn.set_tooltip_text(Some(gettext("Copy transcript to clipboard").as_str()));
        copy_btn.set_margin_start(16); // extra gap between groups (adds to box spacing)

        let clear_btn = gtk::Button::from_icon_name("edit-clear-all-symbolic");
        clear_btn.set_tooltip_text(Some(gettext("Clear transcript").as_str()));

        let save_btn = gtk::Button::from_icon_name("document-save-symbolic");
        save_btn.set_tooltip_text(Some(gettext("Save transcript to file").as_str()));

        self.append(&copy_btn);
        self.append(&clear_btn);
        self.append(&save_btn);

        // Store references
        *imp.record_btn.borrow_mut() = Some(record_btn);
        *imp.pause_btn.borrow_mut() = Some(pause_btn);
        *imp.stop_btn.borrow_mut() = Some(stop_btn);
        *imp.cancel_btn.borrow_mut() = Some(cancel_btn);
        *imp.open_file_btn.borrow_mut() = Some(open_file_btn);
        *imp.copy_btn.borrow_mut() = Some(copy_btn);
        *imp.clear_btn.borrow_mut() = Some(clear_btn);
        *imp.save_btn.borrow_mut() = Some(save_btn);
        *imp.translate_toggle.borrow_mut() = Some(translate_toggle);
        *imp.ai_toggle.borrow_mut() = Some(ai_toggle);
    }

    /// Whether "Improve with AI" is armed.
    pub fn is_ai_active(&self) -> bool {
        self.imp().ai_toggle.borrow().as_ref().map(|t| t.is_active()).unwrap_or(false)
    }

    /// Set the "Improve with AI" toggle state.
    pub fn set_ai_active(&self, active: bool) {
        if let Some(t) = self.imp().ai_toggle.borrow().as_ref() {
            t.set_active(active);
        }
    }

    /// Connect a callback for when the "Improve with AI" toggle changes.
    pub fn connect_ai_toggled<F: Fn(bool) + 'static>(&self, callback: F) {
        if let Some(t) = self.imp().ai_toggle.borrow().as_ref() {
            t.connect_toggled(move |t| callback(t.is_active()));
        }
    }

    /// Show/hide the "Improve with AI" button.
    pub fn set_ai_visible(&self, visible: bool) {
        if let Some(t) = self.imp().ai_toggle.borrow().as_ref() {
            t.set_visible(visible);
        }
    }

    /// Connect an action callback. Call this once from the parent.
    pub fn connect_action<F: Fn(ControlAction) + Clone + 'static>(&self, callback: F) {
        let imp = self.imp();

        if let Some(btn) = imp.record_btn.borrow().as_ref() {
            let cb = callback.clone();
            btn.connect_clicked(move |_| cb(ControlAction::Record));
        }
        if let Some(btn) = imp.pause_btn.borrow().as_ref() {
            let cb = callback.clone();
            btn.connect_clicked(move |_| cb(ControlAction::Pause));
        }
        if let Some(btn) = imp.stop_btn.borrow().as_ref() {
            let cb = callback.clone();
            btn.connect_clicked(move |_| cb(ControlAction::Stop));
        }
        if let Some(btn) = imp.cancel_btn.borrow().as_ref() {
            let cb = callback.clone();
            btn.connect_clicked(move |_| cb(ControlAction::Cancel));
        }
        if let Some(btn) = imp.open_file_btn.borrow().as_ref() {
            let cb = callback.clone();
            btn.connect_clicked(move |_| cb(ControlAction::OpenFile));
        }
        if let Some(btn) = imp.copy_btn.borrow().as_ref() {
            let cb = callback.clone();
            btn.connect_clicked(move |_| cb(ControlAction::Copy));
        }
        if let Some(btn) = imp.clear_btn.borrow().as_ref() {
            let cb = callback.clone();
            btn.connect_clicked(move |_| cb(ControlAction::Clear));
        }
        if let Some(btn) = imp.save_btn.borrow().as_ref() {
            let cb = callback.clone();
            btn.connect_clicked(move |_| cb(ControlAction::Save));
        }
    }

    /// Set recording state — enables/disables appropriate buttons.
    pub fn set_recording_state(&self, recording: bool) {
        let imp = self.imp();
        if let Some(btn) = imp.record_btn.borrow().as_ref() {
            btn.set_sensitive(!recording);
        }
        if let Some(btn) = imp.pause_btn.borrow().as_ref() {
            btn.set_sensitive(recording);
        }
        if let Some(btn) = imp.stop_btn.borrow().as_ref() {
            btn.set_sensitive(recording);
        }
        if let Some(btn) = imp.cancel_btn.borrow().as_ref() {
            btn.set_sensitive(recording);
        }
    }

    /// Set paused state — toggle the pause/resume icon + label on the button's
    /// `ButtonContent` child.
    pub fn set_paused_state(&self, paused: bool) {
        if let Some(btn) = self.imp().pause_btn.borrow().as_ref() {
            if let Some(content) = btn.child().and_downcast::<adw::ButtonContent>() {
                if paused {
                    content.set_icon_name("media-playback-start-symbolic");
                    content.set_label(gettext("Resume").as_str());
                    btn.set_tooltip_text(Some(gettext("Resume recording").as_str()));
                } else {
                    content.set_icon_name("media-playback-pause-symbolic");
                    content.set_label(gettext("Pause").as_str());
                    btn.set_tooltip_text(Some(gettext("Pause recording").as_str()));
                }
            }
        }
    }

    /// Reset all buttons to initial state.
    pub fn reset(&self) {
        self.set_recording_state(false);
        self.set_paused_state(false);
    }

    /// Get whether the translate toggle is active.
    pub fn is_translate_active(&self) -> bool {
        self.imp()
            .translate_toggle
            .borrow()
            .as_ref()
            .map(|t| t.is_active())
            .unwrap_or(false)
    }

    /// Set the translate toggle state.
    pub fn set_translate_active(&self, active: bool) {
        if let Some(toggle) = self.imp().translate_toggle.borrow().as_ref() {
            toggle.set_active(active);
        }
    }

    /// Connect a callback for when translate toggle changes.
    pub fn connect_translate_changed<F: Fn(bool) + 'static>(&self, callback: F) {
        if let Some(toggle) = self.imp().translate_toggle.borrow().as_ref() {
            toggle.connect_active_notify(move |t| {
                callback(t.is_active());
            });
        }
    }

    /// Enable/disable the translate toggle based on backend capabilities. It
    /// stays visible (greyed-out when disabled) rather than disappearing.
    pub fn set_translate_enabled(&self, enabled: bool) {
        if let Some(toggle) = self.imp().translate_toggle.borrow().as_ref() {
            toggle.set_visible(true);
            toggle.set_sensitive(enabled);
            if !enabled {
                toggle.set_active(false);
            }
        }
    }
}
