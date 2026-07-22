// Speech to Text - Language Settings Page
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Language selection and auto-detection settings.

use gtk4::prelude::*;
use crate::i18n::gettext;
use adw::prelude::*;
use gtk4::glib;
use gtk4 as gtk;
use libadwaita as adw;
use adw::subclass::prelude::*;
use std::cell::RefCell;

use crate::config::AppConfig;

/// Language codes in the same order as the manual-selection combo rows.
const LANG_CODES: [&str; 20] = [
    "en", "el", "es", "fr", "de", "it", "pt", "ru", "zh", "ja",
    "ko", "ar", "hi", "nl", "pl", "sv", "tr", "uk", "vi", "th",
];

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct LanguagePage {
        pub auto_detect_switch: RefCell<Option<adw::SwitchRow>>,
        pub language_combo: RefCell<Option<adw::ComboRow>>,
        pub translate_switch: RefCell<Option<gtk::Switch>>,
        pub target_label: RefCell<Option<adw::ActionRow>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LanguagePage {
        const NAME: &'static str = "SttLanguagePage";
        type Type = super::LanguagePage;
        type ParentType = adw::PreferencesPage;
    }

    impl ObjectImpl for LanguagePage {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_ui();
        }
    }

    impl WidgetImpl for LanguagePage {}
    impl adw::subclass::prelude::PreferencesPageImpl for LanguagePage {}
}

glib::wrapper! {
    pub struct LanguagePage(ObjectSubclass<imp::LanguagePage>)
        @extends gtk::Widget, adw::PreferencesPage;
}

impl LanguagePage {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("title", "Language")
            .property("icon-name", "preferences-desktop-locale-symbolic")
            .build()
    }

    fn setup_ui(&self) {
        let imp = self.imp();

        // Detection group
        let detect_group = adw::PreferencesGroup::new();
        detect_group.set_title(gettext("Language Detection").as_str());
        detect_group.set_description(Some(gettext("Whisper can automatically detect the spoken language").as_str()));

        let auto_detect = adw::SwitchRow::builder()
            .title(gettext("Auto-detect Language").as_str())
            .subtitle(gettext("Let Whisper determine the language automatically").as_str())
            .active(true)
            .build();

        detect_group.add(&auto_detect);
        self.add(&detect_group);

        // Manual selection group
        let manual_group = adw::PreferencesGroup::new();
        manual_group.set_title(gettext("Manual Selection").as_str());
        manual_group.set_description(Some(gettext("Force a specific language for transcription").as_str()));

        let languages = gtk::StringList::new(&[
            "English", "Greek", "Spanish", "French", "German",
            "Italian", "Portuguese", "Russian", "Chinese", "Japanese",
            "Korean", "Arabic", "Hindi", "Dutch", "Polish",
            "Swedish", "Turkish", "Ukrainian", "Vietnamese", "Thai",
        ]);

        let lang_combo = adw::ComboRow::builder()
            .title(gettext("Language").as_str())
            .subtitle(gettext("Used when auto-detect is disabled").as_str())
            .model(&languages)
            .build();
        lang_combo.set_sensitive(false); // Disabled when auto-detect is on

        manual_group.add(&lang_combo);
        self.add(&manual_group);

        // Toggle sensitivity based on auto-detect
        let combo_ref = lang_combo.clone();
        auto_detect.connect_active_notify(move |switch| {
            combo_ref.set_sensitive(!switch.is_active());
        });

        // Translation group
        let translate_group = adw::PreferencesGroup::new();
        translate_group.set_title(gettext("Translation").as_str());
        translate_group.set_description(Some(
            "Whisper can translate speech from any supported language into English. \
             This is a built-in capability of the Whisper model — English is the \
             only supported target language."
        ));

        let translate_row = adw::ActionRow::builder()
            .title(gettext("Enable Translation").as_str())
            .subtitle(gettext("Translate spoken audio to English").as_str())
            .build();

        let translate_switch = gtk::Switch::new();
        translate_switch.set_valign(gtk::Align::Center);
        translate_row.add_suffix(&translate_switch);
        translate_row.set_activatable_widget(Some(&translate_switch));
        translate_group.add(&translate_row);

        // Fixed target language row (informational, not editable)
        let target_row = adw::ActionRow::builder()
            .title(gettext("Target Language").as_str())
            .subtitle(gettext("Only English is supported by Whisper").as_str())
            .sensitive(false)
            .build();

        let target_label = gtk::Label::new(Some(gettext("English").as_str()));
        target_label.add_css_class("dim-label");
        target_label.set_valign(gtk::Align::Center);
        target_row.add_suffix(&target_label);
        translate_group.add(&target_row);

        self.add(&translate_group);

        // Info group
        let info_group = adw::PreferencesGroup::new();
        info_group.set_title(gettext("Supported Languages").as_str());
        info_group.set_description(Some(
            "Whisper supports 99 languages. The most common ones are listed above. \
             For the full list, see the Whisper documentation."
        ));
        self.add(&info_group);

        *imp.auto_detect_switch.borrow_mut() = Some(auto_detect);
        *imp.language_combo.borrow_mut() = Some(lang_combo);
        *imp.translate_switch.borrow_mut() = Some(translate_switch);
        *imp.target_label.borrow_mut() = Some(target_row);

        // Restore the saved manual language, then persist combo changes. This is
        // the language used by the Cohere backend (which has no auto-detect) and
        // by the global dictation path.
        self.load_from_config();
        self.connect_persistence();
    }

    fn load_from_config(&self) {
        let config = AppConfig::load();
        if let Some(switch) = self.imp().auto_detect_switch.borrow().as_ref() {
            switch.set_active(config.auto_detect_language);
        }
        if let Some(combo) = self.imp().language_combo.borrow().as_ref() {
            if let Some(ref code) = config.language {
                if let Some(idx) = LANG_CODES.iter().position(|c| c == code) {
                    combo.set_selected(idx as u32);
                }
            }
        }
    }

    fn connect_persistence(&self) {
        if let Some(switch) = self.imp().auto_detect_switch.borrow().as_ref() {
            switch.connect_active_notify(|switch| {
                let mut config = AppConfig::load();
                config.auto_detect_language = switch.is_active();
                config.save();
            });
        }
        if let Some(combo) = self.imp().language_combo.borrow().as_ref() {
            combo.connect_selected_notify(|combo| {
                let idx = (combo.selected() as usize).min(LANG_CODES.len() - 1);
                let mut c = AppConfig::load();
                c.language = Some(LANG_CODES[idx].to_string());
                c.save();
            });
        }
    }

    /// Get whether auto-detect is enabled.
    pub fn is_auto_detect(&self) -> bool {
        self.imp()
            .auto_detect_switch
            .borrow()
            .as_ref()
            .map(|s| s.is_active())
            .unwrap_or(true)
    }

    /// Get whether translate-to-English is enabled.
    pub fn is_translate_enabled(&self) -> bool {
        self.imp()
            .translate_switch
            .borrow()
            .as_ref()
            .map(|s| s.is_active())
            .unwrap_or(false)
    }

    /// Set auto-detect state.
    pub fn set_auto_detect(&self, enabled: bool) {
        if let Some(switch) = self.imp().auto_detect_switch.borrow().as_ref() {
            switch.set_active(enabled);
        }
    }

    /// Set translate state.
    pub fn set_translate_enabled(&self, enabled: bool) {
        if let Some(switch) = self.imp().translate_switch.borrow().as_ref() {
            switch.set_active(enabled);
        }
    }

    /// Connect a callback for when the translate switch changes.
    pub fn connect_translate_changed<F: Fn(bool) + 'static>(&self, callback: F) {
        if let Some(switch) = self.imp().translate_switch.borrow().as_ref() {
            switch.connect_active_notify(move |s| callback(s.is_active()));
        }
    }

    /// Get the target language name for translation (always English for Whisper).
    pub fn selected_target_language_name(&self) -> String {
        "English".to_string()
    }

    /// Get the target language code for translation (always "en" for Whisper).
    pub fn selected_target_language_code(&self) -> String {
        "en".to_string()
    }

    /// Get the display name of the currently selected language.
    pub fn selected_language_name(&self) -> String {
        if self.is_auto_detect() {
            return "Auto-detect".to_string();
        }
        let languages = [
            "English", "Greek", "Spanish", "French", "German",
            "Italian", "Portuguese", "Russian", "Chinese", "Japanese",
            "Korean", "Arabic", "Hindi", "Dutch", "Polish",
            "Swedish", "Turkish", "Ukrainian", "Vietnamese", "Thai",
        ];
        let index = self.imp().language_combo.borrow()
            .as_ref()
            .map(|c| c.selected() as usize)
            .unwrap_or(0);
        languages.get(index).unwrap_or(&"Auto-detect").to_string()
    }

    /// Get the selected language code for whisper (e.g. "en", "el") or None for auto-detect.
    pub fn selected_language_code(&self) -> Option<String> {
        if self.is_auto_detect() {
            return None;
        }
        let codes = [
            "en", "el", "es", "fr", "de",
            "it", "pt", "ru", "zh", "ja",
            "ko", "ar", "hi", "nl", "pl",
            "sv", "tr", "uk", "vi", "th",
        ];
        let index = self.imp().language_combo.borrow()
            .as_ref()
            .map(|c| c.selected() as usize)
            .unwrap_or(0);
        codes.get(index).map(|s| s.to_string())
    }


    /// Connect a callback for when language selection changes.
    pub fn connect_language_changed<F: Fn(String) + Clone + 'static>(&self, callback: F) {
        let imp = self.imp();

        if let Some(switch) = imp.auto_detect_switch.borrow().as_ref() {
            let page = self.clone();
            let cb = callback.clone();
            switch.connect_active_notify(move |_| {
                cb(page.selected_language_name());
            });
        }

        if let Some(combo) = imp.language_combo.borrow().as_ref() {
            let page = self.clone();
            let cb = callback;
            combo.connect_selected_notify(move |_| {
                cb(page.selected_language_name());
            });
        }
    }

    /// Enable/disable auto-detect based on backend capabilities.
    pub fn set_auto_detect_available(&self, available: bool) {
        if let Some(switch) = self.imp().auto_detect_switch.borrow().as_ref() {
            switch.set_sensitive(available);
            if !available {
                switch.set_active(false);
            }
        }
    }

    /// Enable/disable translation based on backend capabilities.
    pub fn set_translation_available(&self, available: bool) {
        if let Some(switch) = self.imp().translate_switch.borrow().as_ref() {
            switch.set_sensitive(available);
            if !available {
                switch.set_active(false);
            }
        }
        if let Some(row) = self.imp().target_label.borrow().as_ref() {
            row.set_sensitive(available);
        }
    }
}

/// Convert a language code to display name.
pub fn language_code_to_name(code: &str) -> String {
    match code {
        "en" => "English",
        "el" => "Greek",
        "es" => "Spanish",
        "fr" => "French",
        "de" => "German",
        "it" => "Italian",
        "pt" => "Portuguese",
        "ru" => "Russian",
        "zh" => "Chinese",
        "ja" => "Japanese",
        "ko" => "Korean",
        "ar" => "Arabic",
        "hi" => "Hindi",
        "nl" => "Dutch",
        "pl" => "Polish",
        "sv" => "Swedish",
        "tr" => "Turkish",
        "uk" => "Ukrainian",
        "vi" => "Vietnamese",
        "th" => "Thai",
        _ => code,
    }.to_string()
}
