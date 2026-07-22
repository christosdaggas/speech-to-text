// Speech to Text - Theme Popover
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Theme selector popup with System/Light/Dark options, About, and Quit.

use crate::i18n::gettext;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk4 as gtk;
use gtk4::glib;
use libadwaita as adw;
use std::cell::RefCell;

use crate::config::AppConfig;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ThemePopover {
        pub theme: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ThemePopover {
        const NAME: &'static str = "SttThemePopover";
        type Type = super::ThemePopover;
        type ParentType = gtk::Popover;
    }

    impl ObjectImpl for ThemePopover {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            // Scoped CSS class so style.css targets only this popover,
            // not every popover in the application.
            obj.add_css_class("stt-theme-menu");
            obj.setup_ui();
        }
    }

    impl WidgetImpl for ThemePopover {}
    impl PopoverImpl for ThemePopover {}
}

glib::wrapper! {
    pub struct ThemePopover(ObjectSubclass<imp::ThemePopover>)
        @extends gtk::Widget, gtk::Popover;
}

impl ThemePopover {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    /// Load saved theme and apply it.
    pub fn load_saved_theme(&self) {
        let config = AppConfig::load();
        let theme = config.theme.clone().unwrap_or_else(|| "system".to_string());
        Self::apply_theme(&theme);
        *self.imp().theme.borrow_mut() = theme;
    }

    fn setup_ui(&self) {
        let main_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(0)
            .width_request(280)
            .build();

        // Theme selector section
        let theme_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(18)
            .halign(gtk::Align::Center)
            .margin_top(18)
            .margin_bottom(18)
            .build();

        let default_btn = gtk::ToggleButton::new();
        let light_btn = gtk::ToggleButton::new();
        let dark_btn = gtk::ToggleButton::new();

        fn create_theme_content(css_class: &str, is_selected: bool) -> gtk::Overlay {
            let overlay = gtk::Overlay::new();
            let icon = gtk::Box::builder()
                .width_request(44)
                .height_request(44)
                .build();
            icon.add_css_class("theme-selector");
            icon.add_css_class(css_class);
            overlay.set_child(Some(&icon));

            if is_selected {
                let check = gtk::Image::from_icon_name("object-select-symbolic");
                check.add_css_class("theme-check");
                check.set_halign(gtk::Align::Center);
                check.set_valign(gtk::Align::Center);
                overlay.add_overlay(&check);
            }
            overlay
        }

        default_btn.set_child(Some(&create_theme_content("theme-default", false)));
        default_btn.set_tooltip_text(Some(gettext("System").as_str()));
        default_btn.add_css_class("flat");
        default_btn.add_css_class("circular");
        default_btn.add_css_class("theme-button");

        light_btn.set_child(Some(&create_theme_content("theme-light", false)));
        light_btn.set_tooltip_text(Some(gettext("Light").as_str()));
        light_btn.add_css_class("flat");
        light_btn.add_css_class("circular");
        light_btn.add_css_class("theme-button");

        dark_btn.set_child(Some(&create_theme_content("theme-dark", false)));
        dark_btn.set_tooltip_text(Some(gettext("Dark").as_str()));
        dark_btn.add_css_class("flat");
        dark_btn.add_css_class("circular");
        dark_btn.add_css_class("theme-button");

        light_btn.set_group(Some(&default_btn));
        dark_btn.set_group(Some(&default_btn));

        // Set initial state
        let style_manager = adw::StyleManager::default();
        match style_manager.color_scheme() {
            adw::ColorScheme::ForceLight => {
                light_btn.set_active(true);
                light_btn.set_child(Some(&create_theme_content("theme-light", true)));
            }
            adw::ColorScheme::ForceDark => {
                dark_btn.set_active(true);
                dark_btn.set_child(Some(&create_theme_content("theme-dark", true)));
            }
            _ => {
                default_btn.set_active(true);
                default_btn.set_child(Some(&create_theme_content("theme-default", true)));
            }
        }

        // Connect toggle signals
        let light_btn_clone = light_btn.clone();
        let dark_btn_clone = dark_btn.clone();
        default_btn.connect_toggled(move |btn| {
            if btn.is_active() {
                adw::StyleManager::default().set_color_scheme(adw::ColorScheme::Default);
                Self::save_theme("system");
                btn.set_child(Some(&create_theme_content("theme-default", true)));
                light_btn_clone.set_child(Some(&create_theme_content("theme-light", false)));
                dark_btn_clone.set_child(Some(&create_theme_content("theme-dark", false)));
            }
        });

        let default_btn_clone = default_btn.clone();
        let dark_btn_clone2 = dark_btn.clone();
        light_btn.connect_toggled(move |btn| {
            if btn.is_active() {
                adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceLight);
                Self::save_theme("light");
                btn.set_child(Some(&create_theme_content("theme-light", true)));
                default_btn_clone.set_child(Some(&create_theme_content("theme-default", false)));
                dark_btn_clone2.set_child(Some(&create_theme_content("theme-dark", false)));
            }
        });

        let default_btn_clone2 = default_btn.clone();
        let light_btn_clone2 = light_btn.clone();
        dark_btn.connect_toggled(move |btn| {
            if btn.is_active() {
                adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
                Self::save_theme("dark");
                btn.set_child(Some(&create_theme_content("theme-dark", true)));
                default_btn_clone2.set_child(Some(&create_theme_content("theme-default", false)));
                light_btn_clone2.set_child(Some(&create_theme_content("theme-light", false)));
            }
        });

        theme_box.append(&default_btn);
        theme_box.append(&light_btn);
        theme_box.append(&dark_btn);
        main_box.append(&theme_box);

        // Separator
        let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
        separator.set_margin_start(12);
        separator.set_margin_end(12);
        main_box.append(&separator);

        // Menu items
        let menu_list = gtk::Box::new(gtk::Orientation::Vertical, 2);
        menu_list.set_margin_top(6);
        menu_list.set_margin_bottom(6);
        menu_list.set_margin_start(6);
        menu_list.set_margin_end(6);

        // What's New button
        let whats_new_btn = gtk::Button::new();
        let whats_new_box = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        whats_new_box.set_margin_start(6);
        whats_new_box.set_margin_end(6);
        whats_new_box.set_margin_top(8);
        whats_new_box.set_margin_bottom(8);
        let whats_new_icon = gtk::Image::from_icon_name("dialog-information-symbolic");
        let whats_new_label = gtk::Label::new(Some(gettext("What's New").as_str()));
        whats_new_label.set_halign(gtk::Align::Start);
        whats_new_label.set_hexpand(true);
        whats_new_box.append(&whats_new_icon);
        whats_new_box.append(&whats_new_label);
        whats_new_btn.set_child(Some(&whats_new_box));
        whats_new_btn.add_css_class("flat");
        whats_new_btn.add_css_class("menu-item");
        whats_new_btn.set_action_name(Some("app.whats-new"));
        menu_list.append(&whats_new_btn);

        // About button
        let about_btn = gtk::Button::new();
        let about_box = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        about_box.set_margin_start(6);
        about_box.set_margin_end(6);
        about_box.set_margin_top(8);
        about_box.set_margin_bottom(8);
        let about_icon = gtk::Image::from_icon_name("help-about-symbolic");
        let about_label = gtk::Label::new(Some(gettext("About Speech to Text").as_str()));
        about_label.set_halign(gtk::Align::Start);
        about_label.set_hexpand(true);
        about_box.append(&about_icon);
        about_box.append(&about_label);
        about_btn.set_child(Some(&about_box));
        about_btn.add_css_class("flat");
        about_btn.add_css_class("menu-item");
        about_btn.set_action_name(Some("app.about"));
        menu_list.append(&about_btn);

        // Quit button
        let quit_btn = gtk::Button::new();
        let quit_box = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        quit_box.set_margin_start(6);
        quit_box.set_margin_end(6);
        quit_box.set_margin_top(8);
        quit_box.set_margin_bottom(8);
        let quit_icon = gtk::Image::from_icon_name("application-exit-symbolic");
        let quit_label = gtk::Label::new(Some(gettext("Quit").as_str()));
        quit_label.set_halign(gtk::Align::Start);
        quit_label.set_hexpand(true);
        quit_box.append(&quit_icon);
        quit_box.append(&quit_label);
        quit_btn.set_child(Some(&quit_box));
        quit_btn.add_css_class("flat");
        quit_btn.add_css_class("menu-item");
        quit_btn.set_action_name(Some("app.quit"));
        menu_list.append(&quit_btn);

        main_box.append(&menu_list);
        self.set_child(Some(&main_box));
    }

    fn save_theme(theme: &str) {
        let mut config = AppConfig::load();
        config.theme = Some(theme.to_string());
        config.save();
    }

    pub fn apply_theme(theme: &str) {
        let style_manager = adw::StyleManager::default();
        match theme {
            "light" => style_manager.set_color_scheme(adw::ColorScheme::ForceLight),
            "dark" => style_manager.set_color_scheme(adw::ColorScheme::ForceDark),
            _ => style_manager.set_color_scheme(adw::ColorScheme::Default),
        }
    }
}

impl Default for ThemePopover {
    fn default() -> Self {
        Self::new()
    }
}
