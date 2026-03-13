// Speech to Text - Header Controls
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Header bar with microphone selector, model selector, and status indicator.

use gtk4::prelude::*;
use gtk4::glib;
use gtk4 as gtk;
use libadwaita as adw;
use adw::subclass::prelude::*;
use std::cell::RefCell;

use crate::ui::widgets::ThemePopover;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct HeaderControls {
        pub mic_dropdown: RefCell<Option<gtk::DropDown>>,
        pub model_dropdown: RefCell<Option<gtk::DropDown>>,
        pub status_label: RefCell<Option<gtk::Label>>,
        pub status_icon: RefCell<Option<gtk::Image>>,
        pub header_bar: RefCell<Option<adw::HeaderBar>>,
        pub theme_popover: RefCell<Option<ThemePopover>>,
        pub language_label: RefCell<Option<gtk::Label>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for HeaderControls {
        const NAME: &'static str = "SttHeaderControls";
        type Type = super::HeaderControls;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for HeaderControls {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_ui();
        }
    }

    impl WidgetImpl for HeaderControls {}
    impl BoxImpl for HeaderControls {}
}

glib::wrapper! {
    pub struct HeaderControls(ObjectSubclass<imp::HeaderControls>)
        @extends gtk::Widget, gtk::Box;
}

impl HeaderControls {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("orientation", gtk::Orientation::Vertical)
            .build()
    }

    fn setup_ui(&self) {
        let imp = self.imp();

        let header_bar = adw::HeaderBar::new();
        header_bar.set_show_start_title_buttons(false);

        // === Left side: Microphone selector ===
        let mic_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        mic_box.set_margin_start(4);

        let mic_label = gtk::Label::new(Some("Microphone:"));
        mic_label.add_css_class("dim-label");
        mic_box.append(&mic_label);

        let mic_icon = gtk::Image::from_icon_name("audio-input-microphone-symbolic");
        mic_icon.set_pixel_size(16);

        let mic_model = gtk::StringList::new(&["Built-in Audio"]);
        let mic_dropdown = gtk::DropDown::new(Some(mic_model), gtk::Expression::NONE);
        mic_dropdown.set_tooltip_text(Some("Select microphone"));
        mic_box.append(&mic_dropdown);

        header_bar.pack_start(&mic_box);

        // === Center: Model selector + status ===
        let center_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        center_box.set_halign(gtk::Align::Center);

        let model_label = gtk::Label::new(Some("Model:"));
        model_label.add_css_class("dim-label");
        center_box.append(&model_label);

        let model_icon = gtk::Image::from_icon_name("system-software-install-symbolic");
        model_icon.set_pixel_size(16);

        let model_list = gtk::StringList::new(&["Whisper Tiny", "Whisper Base", "Whisper Small", "Whisper Medium", "Whisper Large"]);
        let model_dropdown = gtk::DropDown::new(Some(model_list), gtk::Expression::NONE);
        model_dropdown.set_tooltip_text(Some("Select Whisper model"));
        center_box.append(&model_dropdown);

        // Model status indicator
        let status_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        status_box.set_margin_start(8);

        // Theme-aware status dot: uses @error_color / @success_color
        // via the widget's resolved style context instead of hardcoded RGB.
        let status_icon = gtk::Image::from_icon_name("media-record-symbolic");
        status_icon.set_pixel_size(14);
        status_icon.set_valign(gtk::Align::Center);
        status_icon.add_css_class("status-dot");
        status_icon.add_css_class("error");
        status_box.append(&status_icon);

        let status_label = gtk::Label::new(Some("No Model"));
        status_label.add_css_class("caption");
        status_box.append(&status_label);

        center_box.append(&status_box);

        header_bar.set_title_widget(Some(&center_box));

        // === Right side: Language label + Hamburger menu ===
        let lang_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        lang_box.set_margin_end(4);

        let lang_icon = gtk::Image::from_icon_name("preferences-desktop-locale-symbolic");
        lang_icon.set_pixel_size(16);
        lang_box.append(&lang_icon);

        let language_label = gtk::Label::new(Some("Auto-detect"));
        language_label.add_css_class("caption");
        language_label.add_css_class("dim-label");
        lang_box.append(&language_label);

        // Pack menu button first (rightmost, next to window buttons),
        // then language box to its left
        let menu_button = gtk::MenuButton::new();
        menu_button.set_icon_name("open-menu-symbolic");
        menu_button.set_tooltip_text(Some("Main menu"));
        let theme_popover = ThemePopover::new();
        menu_button.set_popover(Some(&theme_popover));
        header_bar.pack_end(&menu_button);

        header_bar.pack_end(&lang_box);

        self.append(&header_bar);

        // Store references
        *imp.mic_dropdown.borrow_mut() = Some(mic_dropdown);
        *imp.model_dropdown.borrow_mut() = Some(model_dropdown);
        *imp.status_label.borrow_mut() = Some(status_label);
        *imp.status_icon.borrow_mut() = Some(status_icon);
        *imp.header_bar.borrow_mut() = Some(header_bar);
        *imp.theme_popover.borrow_mut() = Some(theme_popover);
        *imp.language_label.borrow_mut() = Some(language_label);
    }

    /// Update the microphone list.
    pub fn set_microphones(&self, devices: &[String]) {
        if let Some(dropdown) = self.imp().mic_dropdown.borrow().as_ref() {
            let model = gtk::StringList::new(
                &devices.iter().map(|s| s.as_str()).collect::<Vec<_>>()
            );
            dropdown.set_model(Some(&model));
        }
    }

    /// Update the model status indicator.
    pub fn set_model_status(&self, loaded: bool, _model_name: &str) {
        let imp = self.imp();

        if let Some(label) = imp.status_label.borrow().as_ref() {
            if loaded {
                label.set_text("Model Loaded");
            } else {
                label.set_text("No Model");
            }
        }

        if let Some(icon) = imp.status_icon.borrow().as_ref() {
            // Use semantic CSS classes instead of hardcoded colors.
            icon.remove_css_class("error");
            icon.remove_css_class("success");
            if loaded {
                icon.add_css_class("success");
            } else {
                icon.add_css_class("error");
            }
        }
    }

    /// Select a model by index in the dropdown.
    pub fn set_selected_model(&self, index: u32) {
        if let Some(dropdown) = self.imp().model_dropdown.borrow().as_ref() {
            dropdown.set_selected(index);
        }
    }

    /// Get the currently selected model ID based on dropdown index.
    pub fn selected_model_id(&self) -> String {
        let index = self.imp().model_dropdown.borrow()
            .as_ref()
            .map(|d| d.selected())
            .unwrap_or(0);
        match index {
            0 => "tiny".to_string(),
            1 => "base".to_string(),
            2 => "small".to_string(),
            3 => "medium".to_string(),
            4 => "large-v3".to_string(),
            _ => "base".to_string(),
        }
    }

    /// Connect a callback for when the model dropdown selection changes.
    pub fn connect_model_changed<F: Fn(String) + 'static>(&self, callback: F) {
        if let Some(dropdown) = self.imp().model_dropdown.borrow().as_ref() {
            dropdown.connect_selected_notify(move |dd| {
                let model_id = match dd.selected() {
                    0 => "tiny",
                    1 => "base",
                    2 => "small",
                    3 => "medium",
                    4 => "large-v3",
                    _ => "base",
                };
                callback(model_id.to_string());
            });
        }
    }

    /// Update the language display in the header.
    pub fn set_language_display(&self, language: &str) {
        if let Some(label) = self.imp().language_label.borrow().as_ref() {
            label.set_text(language);
        }
    }
}
