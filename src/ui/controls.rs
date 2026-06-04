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
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Controls {
        pub record_btn: RefCell<Option<gtk::Button>>,
        pub pause_btn: RefCell<Option<gtk::Button>>,
        pub stop_btn: RefCell<Option<gtk::Button>>,
        pub cancel_btn: RefCell<Option<gtk::Button>>,
        pub copy_btn: RefCell<Option<gtk::Button>>,
        pub clear_btn: RefCell<Option<gtk::Button>>,
        pub save_btn: RefCell<Option<gtk::Button>>,
        pub translate_toggle: RefCell<Option<gtk::ToggleButton>>,
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
        glib::Object::builder()
            .property("orientation", gtk::Orientation::Horizontal)
            .property("spacing", 0)
            .build()
    }

    fn setup_ui(&self) {
        let imp = self.imp();

        self.add_css_class("controls-panel");
        self.set_margin_start(16);
        self.set_margin_end(16);
        self.set_margin_bottom(12);
        self.set_halign(gtk::Align::Center);
        self.set_spacing(8);

        // === Recording controls (left group) ===
        let rec_group = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        rec_group.add_css_class("linked");

        // Record button
        let record_btn = gtk::Button::new();
        let rec_content = adw::ButtonContent::new();
        rec_content.set_icon_name("media-record-symbolic");
        rec_content.set_label(gettext("Record").as_str());
        record_btn.set_child(Some(&rec_content));
        record_btn.add_css_class("suggested-action");
        record_btn.set_tooltip_text(Some(gettext("Start recording (Ctrl+R)").as_str()));

        // Pause button
        let pause_btn = gtk::Button::from_icon_name("media-playback-pause-symbolic");
        pause_btn.set_tooltip_text(Some(gettext("Pause recording").as_str()));
        pause_btn.set_sensitive(false);

        // Stop button
        let stop_btn = gtk::Button::from_icon_name("media-playback-stop-symbolic");
        stop_btn.set_tooltip_text(Some(gettext("Stop recording and transcribe").as_str()));
        stop_btn.add_css_class("stop-action");
        stop_btn.set_sensitive(false);

        // Cancel button
        let cancel_btn = gtk::Button::from_icon_name("process-stop-symbolic");
        cancel_btn.set_tooltip_text(Some(gettext("Cancel recording and discard").as_str()));
        cancel_btn.add_css_class("destructive-action");
        cancel_btn.set_sensitive(false);

        rec_group.append(&record_btn);
        rec_group.append(&pause_btn);
        rec_group.append(&stop_btn);
        rec_group.append(&cancel_btn);
        self.append(&rec_group);

        // === Separator ===
        let sep = gtk::Separator::new(gtk::Orientation::Vertical);
        sep.set_margin_start(12);
        sep.set_margin_end(12);
        self.append(&sep);

        // === Translate toggle ===
        let translate_toggle = gtk::ToggleButton::new();
        let translate_content = adw::ButtonContent::new();
        translate_content.set_icon_name("preferences-desktop-locale-symbolic");
        translate_content.set_label(gettext("Translate").as_str());
        translate_toggle.set_child(Some(&translate_content));
        translate_toggle.set_tooltip_text(Some(gettext("Translate to English").as_str()));
        translate_toggle.add_css_class("flat");
        self.append(&translate_toggle);

        // === Separator ===
        let sep2 = gtk::Separator::new(gtk::Orientation::Vertical);
        sep2.set_margin_start(12);
        sep2.set_margin_end(12);
        self.append(&sep2);

        // === Action buttons (right group) ===
        let action_group = gtk::Box::new(gtk::Orientation::Horizontal, 4);

        // Copy button
        let copy_btn = gtk::Button::from_icon_name("edit-copy-symbolic");
        copy_btn.set_tooltip_text(Some(gettext("Copy transcript to clipboard").as_str()));
        copy_btn.add_css_class("flat");

        // Clear button
        let clear_btn = gtk::Button::from_icon_name("edit-clear-all-symbolic");
        clear_btn.set_tooltip_text(Some(gettext("Clear transcript").as_str()));
        clear_btn.add_css_class("flat");

        // Save button
        let save_btn = gtk::Button::new();
        let save_content = adw::ButtonContent::new();
        save_content.set_icon_name("document-save-symbolic");
        save_content.set_label(gettext("Save").as_str());
        save_btn.set_child(Some(&save_content));
        save_btn.add_css_class("flat");
        save_btn.set_tooltip_text(Some(gettext("Save transcript to file").as_str()));

        action_group.append(&copy_btn);
        action_group.append(&clear_btn);
        action_group.append(&save_btn);
        self.append(&action_group);

        // Store references
        *imp.record_btn.borrow_mut() = Some(record_btn);
        *imp.pause_btn.borrow_mut() = Some(pause_btn);
        *imp.stop_btn.borrow_mut() = Some(stop_btn);
        *imp.cancel_btn.borrow_mut() = Some(cancel_btn);
        *imp.copy_btn.borrow_mut() = Some(copy_btn);
        *imp.clear_btn.borrow_mut() = Some(clear_btn);
        *imp.save_btn.borrow_mut() = Some(save_btn);
        *imp.translate_toggle.borrow_mut() = Some(translate_toggle);
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

    /// Set paused state — toggle pause/resume icon.
    pub fn set_paused_state(&self, paused: bool) {
        if let Some(btn) = self.imp().pause_btn.borrow().as_ref() {
            if paused {
                btn.set_icon_name("media-playback-start-symbolic");
                btn.set_tooltip_text(Some(gettext("Resume recording").as_str()));
            } else {
                btn.set_icon_name("media-playback-pause-symbolic");
                btn.set_tooltip_text(Some(gettext("Pause recording").as_str()));
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

    /// Show or hide the translate toggle based on backend capabilities.
    pub fn set_translate_visible(&self, visible: bool) {
        if let Some(toggle) = self.imp().translate_toggle.borrow().as_ref() {
            toggle.set_visible(visible);
            if !visible {
                toggle.set_active(false);
            }
        }
    }
}
