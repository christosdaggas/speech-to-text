// Speech to Text - Header Controls
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Header bar with microphone selector, model selector, and status indicator.

use gtk4::prelude::*;
use crate::i18n::gettext;
use gtk4::glib;
use gtk4 as gtk;
use gtk4::pango;
use libadwaita as adw;
use adw::subclass::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use crate::ui::widgets::ThemePopover;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct HeaderControls {
        pub mic_dropdown: RefCell<Option<gtk::DropDown>>,
        pub model_dropdown: RefCell<Option<gtk::DropDown>>,
        // `Rc` so the `connect_model_changed` closure shares the SAME list (a
        // bare `RefCell::clone()` would snapshot an independent — and stale/empty
        // — copy, so genuine user selections would never resolve to a model id).
        pub model_ids: Rc<RefCell<Vec<String>>>,
        pub status_label: RefCell<Option<gtk::Label>>,
        pub status_icon: RefCell<Option<gtk::Image>>,
        pub page_title_label: RefCell<Option<gtk::Label>>,
        pub header_bar: RefCell<Option<adw::HeaderBar>>,
        pub theme_popover: RefCell<Option<ThemePopover>>,
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
    const MIC_DROPDOWN_WIDTH: i32 = 120;
    const MODEL_DROPDOWN_WIDTH: i32 = 120;
    const STATUS_LABEL_CHARS: i32 = 10;

    pub fn new() -> Self {
        glib::Object::builder()
            .property("orientation", gtk::Orientation::Vertical)
            .build()
    }

    fn setup_ui(&self) {
        let imp = self.imp();

        let header_bar = adw::HeaderBar::new();
        header_bar.add_css_class("content-headerbar");
        header_bar.set_height_request(52);
        header_bar.set_show_start_title_buttons(false);
        // Prevent AdwHeaderBar from injecting the window title in the centre;
        // the page title is intentionally left-aligned like the mockup.
        header_bar.set_title_widget(Some(&gtk::Label::new(None)));

        let page_title = gtk::Label::new(Some(&gettext("Transcription")));
        page_title.add_css_class("header-page-title");
        page_title.set_xalign(0.0);
        header_bar.pack_start(&page_title);

        // Kept as the authoritative microphone selector for existing sync code;
        // microphone choice is presented in Settings instead of crowding this bar.
        let mic_model = gtk::StringList::new(&["Built-in Audio"]);
        let mic_dropdown = gtk::DropDown::new(Some(mic_model), gtk::Expression::NONE);
        Self::configure_fixed_width_dropdown(
            &mic_dropdown,
            Self::MIC_DROPDOWN_WIDTH,
            20,
        );

        let menu_button = gtk::MenuButton::new();
        menu_button.set_icon_name("view-more-symbolic");
        menu_button.set_tooltip_text(Some(gettext("Main menu").as_str()));
        let theme_popover = ThemePopover::new();
        menu_button.set_popover(Some(&theme_popover));
        header_bar.pack_end(&menu_button);

        let model_box = gtk::Box::new(gtk::Orientation::Horizontal, 7);
        model_box.add_css_class("header-model");
        let model_icon = gtk::Image::from_icon_name("emblem-system-symbolic");
        model_icon.set_pixel_size(15);
        model_box.append(&model_icon);

        let model_list = gtk::StringList::new(&["Tiny", "Base", "Small", "Medium", "Large"]);
        let model_dropdown = gtk::DropDown::new(Some(model_list), gtk::Expression::NONE);
        model_dropdown.add_css_class("header-model-dropdown");
        model_dropdown.set_tooltip_text(Some(gettext("Select model").as_str()));
        Self::configure_fixed_width_dropdown(
            &model_dropdown,
            150,
            18,
        );
        model_box.append(&model_dropdown);

        let status_icon = gtk::Image::from_icon_name("media-record-symbolic");
        status_icon.set_pixel_size(11);
        status_icon.add_css_class("status-dot");
        status_icon.add_css_class("error");
        model_box.append(&status_icon);
        header_bar.pack_end(&model_box);

        // Retained for the existing model-status update API. The visible status
        // is the coloured dot inside the model capsule.
        let status_label = gtk::Label::new(Some(&gettext("No Model")));
        status_label.set_visible(false);

        self.append(&header_bar);

        *imp.mic_dropdown.borrow_mut() = Some(mic_dropdown);
        *imp.model_dropdown.borrow_mut() = Some(model_dropdown);
        *imp.status_label.borrow_mut() = Some(status_label);
        *imp.status_icon.borrow_mut() = Some(status_icon);
        *imp.page_title_label.borrow_mut() = Some(page_title);
        *imp.header_bar.borrow_mut() = Some(header_bar);
        *imp.theme_popover.borrow_mut() = Some(theme_popover);
    }

    #[allow(dead_code)]
    fn setup_legacy_ui(&self) {
        let imp = self.imp();

        let header_bar = adw::HeaderBar::new();
        header_bar.set_show_start_title_buttons(false);

        // === Left side: Microphone selector ===
        let mic_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        mic_box.set_margin_start(4);

        let mic_icon = gtk::Image::from_icon_name("audio-input-microphone-symbolic");
        mic_icon.set_pixel_size(16);
        mic_icon.set_tooltip_text(Some(gettext("Microphone").as_str()));
        mic_box.append(&mic_icon);

        let mic_model = gtk::StringList::new(&["Built-in Audio"]);
        let mic_dropdown = gtk::DropDown::new(Some(mic_model), gtk::Expression::NONE);
        mic_dropdown.set_tooltip_text(Some(gettext("Select microphone").as_str()));
        Self::configure_fixed_width_dropdown(
            &mic_dropdown,
            Self::MIC_DROPDOWN_WIDTH,
            20,
        );
        mic_box.append(&mic_dropdown);

        header_bar.pack_start(&mic_box);

        // === Center: Model selector + status ===
        // (The engine/backend selector lives in Settings → Model → "Default Engine".)
        let center_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        center_box.set_halign(gtk::Align::Center);

        // Model selector
        let model_icon = gtk::Image::from_icon_name("system-software-install-symbolic");
        model_icon.set_pixel_size(16);
        model_icon.set_tooltip_text(Some(gettext("Model").as_str()));
        center_box.append(&model_icon);

        let model_list = gtk::StringList::new(&["Tiny", "Base", "Small", "Medium", "Large"]);
        let model_dropdown = gtk::DropDown::new(Some(model_list), gtk::Expression::NONE);
        model_dropdown.set_tooltip_text(Some(gettext("Select model").as_str()));
        Self::configure_fixed_width_dropdown(
            &model_dropdown,
            Self::MODEL_DROPDOWN_WIDTH,
            16,
        );
        center_box.append(&model_dropdown);

        // Model status indicator
        let status_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        status_box.set_margin_start(4);

        // Theme-aware status dot: uses @error_color / @success_color
        // via the widget's resolved style context instead of hardcoded RGB.
        let status_icon = gtk::Image::from_icon_name("media-record-symbolic");
        status_icon.set_pixel_size(14);
        status_icon.set_valign(gtk::Align::Center);
        status_icon.add_css_class("status-dot");
        status_icon.add_css_class("error");
        status_box.append(&status_icon);

        let status_label = gtk::Label::new(Some(gettext("No Model").as_str()));
        status_label.add_css_class("caption");
        status_label.set_width_chars(Self::STATUS_LABEL_CHARS);
        status_label.set_max_width_chars(Self::STATUS_LABEL_CHARS);
        status_label.set_ellipsize(pango::EllipsizeMode::End);
        status_label.set_xalign(0.0);
        status_box.append(&status_label);

        center_box.append(&status_box);

        header_bar.set_title_widget(Some(&center_box));

        // === Right side: Privacy badge + Hamburger menu ===
        // (The language indicator now lives in the bottom status bar.)
        //
        // Always-truthful badge: transcription is local regardless of the
        // optional network features (model downloads, AI, API).
        let menu_button = gtk::MenuButton::new();
        menu_button.set_icon_name("open-menu-symbolic");
        menu_button.set_tooltip_text(Some(gettext("Main menu").as_str()));
        let theme_popover = ThemePopover::new();
        menu_button.set_popover(Some(&theme_popover));
        header_bar.pack_end(&menu_button);

        let privacy_badge = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        privacy_badge.add_css_class("offline-badge");
        privacy_badge.set_tooltip_text(Some(
            gettext("Transcription is 100% local — audio never leaves your device").as_str(),
        ));
        let shield_icon = gtk::Image::from_icon_name("security-high-symbolic");
        shield_icon.set_pixel_size(13);
        let privacy_label = gtk::Label::new(Some(gettext("Local · Private").as_str()));
        privacy_badge.append(&shield_icon);
        privacy_badge.append(&privacy_label);
        // pack_end inserts right-to-left, so this lands just left of the menu.
        header_bar.pack_end(&privacy_badge);

        self.append(&header_bar);

        // Store references
        *imp.mic_dropdown.borrow_mut() = Some(mic_dropdown);
        *imp.model_dropdown.borrow_mut() = Some(model_dropdown);
        *imp.status_label.borrow_mut() = Some(status_label);
        *imp.status_icon.borrow_mut() = Some(status_icon);
        *imp.header_bar.borrow_mut() = Some(header_bar);
        *imp.theme_popover.borrow_mut() = Some(theme_popover);
    }

    pub fn set_page_title(&self, title: &str) {
        if let Some(label) = self.imp().page_title_label.borrow().as_ref() {
            label.set_text(title);
        }
    }

    fn configure_fixed_width_dropdown(dropdown: &gtk::DropDown, width: i32, max_chars: i32) {
        dropdown.set_width_request(width);
        dropdown.set_hexpand(false);
        dropdown.set_halign(gtk::Align::Start);
        let button_factory = Self::fixed_width_factory(max_chars);
        let list_factory = Self::fixed_width_factory(max_chars);
        dropdown.set_factory(Some(&button_factory));
        dropdown.set_list_factory(Some(&list_factory));
    }

    /// Create a SignalListItemFactory that renders items as fixed-width ellipsized labels.
    /// This prevents the DropDown from growing its natural size based on content.
    fn fixed_width_factory(max_chars: i32) -> gtk::SignalListItemFactory {
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(move |_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let label = gtk::Label::new(None);
            label.set_xalign(0.0);
            label.set_ellipsize(pango::EllipsizeMode::End);
            label.set_width_chars(max_chars);
            label.set_max_width_chars(max_chars);
            item.set_child(Some(&label));
        });
        factory.connect_bind(|_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let Some(string_object) = item.item().and_downcast::<gtk::StringObject>() else {
                return;
            };
            let Some(label) = item.child().and_downcast::<gtk::Label>() else {
                return;
            };
            label.set_text(&string_object.string());
        });
        factory
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

    /// Select the dropdown entry whose id matches `id` (falls back to the first
    /// entry if not present).
    pub fn set_selected_model_by_id(&self, id: &str) {
        let imp = self.imp();
        let idx = imp
            .model_ids
            .borrow()
            .iter()
            .position(|m| m == id)
            .unwrap_or(0) as u32;
        if let Some(dropdown) = imp.model_dropdown.borrow().as_ref() {
            dropdown.set_selected(idx);
        }
    }

    /// Update the model dropdown items based on the selected backend.
    /// For Whisper, only shows models that are actually downloaded.
    /// `downloaded` is a list of (model_id, display_name) for downloaded models.
    pub fn update_models_for_backend(&self, backend: &str, downloaded: &[(String, String)]) {
        let imp = self.imp();
        if let Some(dropdown) = imp.model_dropdown.borrow().as_ref() {
            if backend == "cohere" {
                let model_list = gtk::StringList::new(&["Cohere Transcribe"]);
                dropdown.set_model(Some(&model_list));
                dropdown.set_selected(0);
                dropdown.set_sensitive(false);
                *imp.model_ids.borrow_mut() = vec!["cohere-transcribe".to_string()];
            } else if backend == "qwen" {
                // List only the DOWNLOADED Qwen3-ASR sizes, like the Whisper
                // dropdown. ids are the size strings ("0.6B"/"1.7B").
                let mut names: Vec<&str> = Vec::new();
                let mut ids: Vec<String> = Vec::new();
                if crate::transcription::qwen::is_model_downloaded_size("0.6B") {
                    names.push("Qwen3 0.6B");
                    ids.push("0.6B".to_string());
                }
                if crate::transcription::qwen::is_model_downloaded_size("1.7B") {
                    names.push("Qwen3 1.7B");
                    ids.push("1.7B".to_string());
                }
                if names.is_empty() {
                    let model_list = gtk::StringList::new(&["No model downloaded"]);
                    dropdown.set_model(Some(&model_list));
                    dropdown.set_selected(0);
                    dropdown.set_sensitive(false);
                    *imp.model_ids.borrow_mut() = Vec::new();
                } else {
                    let model_list = gtk::StringList::new(&names);
                    dropdown.set_model(Some(&model_list));
                    dropdown.set_selected(0);
                    dropdown.set_sensitive(true);
                    *imp.model_ids.borrow_mut() = ids;
                }
            } else if downloaded.is_empty() {
                let model_list = gtk::StringList::new(&["No models downloaded"]);
                dropdown.set_model(Some(&model_list));
                dropdown.set_selected(0);
                dropdown.set_sensitive(false);
                *imp.model_ids.borrow_mut() = Vec::new();
            } else {
                let names: Vec<&str> = downloaded.iter().map(|(_, name)| name.as_str()).collect();
                let ids: Vec<String> = downloaded.iter().map(|(id, _)| id.clone()).collect();
                let model_list = gtk::StringList::new(&names);
                dropdown.set_model(Some(&model_list));
                dropdown.set_selected(0);
                dropdown.set_sensitive(true);
                *imp.model_ids.borrow_mut() = ids;
            }
        }
    }

    /// Get the currently selected model ID from the model dropdown. For Cohere,
    /// `update_models_for_backend` has set `model_ids = ["cohere-transcribe"]`.
    pub fn selected_model_id(&self) -> String {
        let imp = self.imp();
        let index = imp.model_dropdown.borrow()
            .as_ref()
            .map(|d| d.selected())
            .unwrap_or(0) as usize;
        let ids = imp.model_ids.borrow();
        ids.get(index).cloned().unwrap_or_else(|| "base".to_string())
    }

    /// Connect a callback for when the model dropdown selection changes.
    pub fn connect_model_changed<F: Fn(String) + 'static>(&self, callback: F) {
        let ids_ref = self.imp().model_ids.clone();
        if let Some(dropdown) = self.imp().model_dropdown.borrow().as_ref() {
            dropdown.connect_selected_notify(move |dd| {
                let index = dd.selected() as usize;
                let ids = ids_ref.borrow();
                if let Some(model_id) = ids.get(index) {
                    callback(model_id.clone());
                }
            });
        }
    }
}
