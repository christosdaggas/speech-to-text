// Speech to Text - Status Bar
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Bottom status bar showing recording state, model info, GPU/CPU mode, and offline badge.

use crate::i18n::gettext;
use adw::subclass::prelude::*;
use gtk4 as gtk;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::RefCell;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct StatusBar {
        pub recording_label: RefCell<Option<gtk::Label>>,
        pub recording_icon: RefCell<Option<gtk::Image>>,
        pub model_label: RefCell<Option<gtk::Label>>,
        pub language_label: RefCell<Option<gtk::Label>>,
        pub compute_label: RefCell<Option<gtk::Label>>,
        pub version_label: RefCell<Option<gtk::Label>>,
        pub update_box: RefCell<Option<gtk::Box>>,
        pub update_label: RefCell<Option<gtk::Label>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for StatusBar {
        const NAME: &'static str = "SttStatusBar";
        type Type = super::StatusBar;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for StatusBar {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_ui();
        }
    }

    impl WidgetImpl for StatusBar {}
    impl BoxImpl for StatusBar {}
}

glib::wrapper! {
    pub struct StatusBar(ObjectSubclass<imp::StatusBar>)
        @extends gtk::Widget, gtk::Box;
}

impl StatusBar {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("orientation", gtk::Orientation::Horizontal)
            .property("spacing", 0)
            .build()
    }

    fn setup_ui(&self) {
        let imp = self.imp();

        self.add_css_class("status-bar");

        // === Recording indicator ===
        let rec_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);

        let rec_icon = gtk::Image::from_icon_name("media-record-symbolic");
        rec_icon.set_pixel_size(10);
        rec_icon.add_css_class("dim-label");
        rec_box.append(&rec_icon);

        let rec_label = gtk::Label::new(Some(gettext("Idle").as_str()));
        rec_label.add_css_class("caption");
        rec_label.add_css_class("dim-label");
        rec_box.append(&rec_label);

        self.append(&rec_box);

        // Separator
        let sep1 = gtk::Separator::new(gtk::Orientation::Vertical);
        sep1.set_margin_start(12);
        sep1.set_margin_end(12);
        self.append(&sep1);

        // (Model indicator removed from the status bar.)

        // === Language (Auto-detect or the chosen language) ===
        let lang_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);

        let lang_icon = gtk::Image::from_icon_name("preferences-desktop-locale-symbolic");
        lang_icon.set_pixel_size(10);
        lang_box.append(&lang_icon);

        let language_label = gtk::Label::new(Some(gettext("Auto-detect").as_str()));
        language_label.add_css_class("caption");
        language_label.add_css_class("dim-label");
        lang_box.append(&language_label);

        self.append(&lang_box);

        // Separator
        let sep_lang = gtk::Separator::new(gtk::Orientation::Vertical);
        sep_lang.set_margin_start(12);
        sep_lang.set_margin_end(12);
        self.append(&sep_lang);

        // === Compute mode ===
        let compute_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);

        let cpu_icon = gtk::Image::from_icon_name("computer-symbolic");
        cpu_icon.set_pixel_size(10);
        compute_box.append(&cpu_icon);

        let compute_label = gtk::Label::new(Some(gettext("CPU").as_str()));
        compute_label.add_css_class("caption");
        compute_label.add_css_class("dim-label");
        compute_box.append(&compute_label);

        self.append(&compute_box);

        // Spacer
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        self.append(&spacer);

        // Separator before version info
        let sep3 = gtk::Separator::new(gtk::Orientation::Vertical);
        sep3.set_margin_start(12);
        sep3.set_margin_end(12);
        self.append(&sep3);

        // === Update indicator (hidden by default) ===
        let update_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        update_box.set_visible(false);

        let update_icon = gtk::Image::from_icon_name("software-update-available-symbolic");
        update_icon.set_pixel_size(10);
        update_icon.add_css_class("error");
        update_box.append(&update_icon);

        let update_label = gtk::Label::new(Some(gettext("Update available").as_str()));
        update_label.add_css_class("caption");
        update_label.add_css_class("error");
        update_box.append(&update_label);

        self.append(&update_box);

        // === Version label ===
        let version_label =
            gtk::Label::new(Some(&format!("Version {}", env!("CARGO_PKG_VERSION"))));
        version_label.add_css_class("caption");
        version_label.add_css_class("dim-label");
        version_label.set_margin_start(4);
        self.append(&version_label);

        // Store references
        *imp.recording_label.borrow_mut() = Some(rec_label);
        *imp.recording_icon.borrow_mut() = Some(rec_icon);
        *imp.language_label.borrow_mut() = Some(language_label);
        *imp.compute_label.borrow_mut() = Some(compute_label);
        *imp.version_label.borrow_mut() = Some(version_label);
        *imp.update_box.borrow_mut() = Some(update_box);
        *imp.update_label.borrow_mut() = Some(update_label);
    }

    /// Update recording status.
    pub fn set_recording_status(&self, status: &str) {
        let imp = self.imp();
        if let Some(label) = imp.recording_label.borrow().as_ref() {
            label.set_text(status);
        }
        if let Some(icon) = imp.recording_icon.borrow().as_ref() {
            icon.remove_css_class("recording-pulse");
            icon.remove_css_class("dim-label");
            icon.remove_css_class("error");
            icon.remove_css_class("success");
            icon.remove_css_class("warning");

            match status {
                s if s.starts_with("Recording") => {
                    icon.add_css_class("success");
                    icon.add_css_class("recording-pulse");
                }
                s if s.starts_with("Transcribing") => {
                    icon.add_css_class("accent");
                    icon.add_css_class("recording-pulse");
                }
                "Paused" => {
                    icon.add_css_class("warning");
                }
                _ => {
                    icon.add_css_class("dim-label");
                }
            }
        }
    }

    /// Update model name in the status bar.
    pub fn set_model_name(&self, name: &str) {
        if let Some(label) = self.imp().model_label.borrow().as_ref() {
            label.set_text(name);
        }
    }

    /// Update the language display (e.g. "Auto-detect" or "Greek").
    pub fn set_language(&self, language: &str) {
        if let Some(label) = self.imp().language_label.borrow().as_ref() {
            label.set_text(language);
        }
    }

    /// Update compute mode display.
    pub fn set_compute_mode(&self, mode: &str) {
        if let Some(label) = self.imp().compute_label.borrow().as_ref() {
            label.set_text(mode);
        }
    }

    /// Show update available indicator with version string.
    pub fn show_update_available(&self, version: &str) {
        let imp = self.imp();
        if let Some(update_box) = imp.update_box.borrow().as_ref() {
            update_box.set_visible(true);
        }
        if let Some(update_label) = imp.update_label.borrow().as_ref() {
            update_label.set_text(&format!("v{} available", version));
        }
    }
}
