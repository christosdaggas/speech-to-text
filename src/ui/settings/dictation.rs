// Speech to Text - Dictation Settings Page
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Mini panel, global dictation shortcut, auto-paste, and dictation mode.

use gtk4::prelude::*;
use crate::i18n::gettext;
use adw::prelude::*;
use gtk4::glib;
use gtk4 as gtk;
use libadwaita as adw;
use adw::subclass::prelude::*;
use std::cell::RefCell;

use crate::config::AppConfig;

/// Config string for each dictation mode, in ComboRow order.
const MODE_IDS: [&str; 5] = ["plain", "message", "email", "note", "code_prompt"];

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct DictationPage {
        pub mini_panel_switch: RefCell<Option<adw::SwitchRow>>,
        pub start_hidden_switch: RefCell<Option<adw::SwitchRow>>,
        pub shortcut_entry: RefCell<Option<adw::EntryRow>>,
        pub auto_paste_switch: RefCell<Option<adw::SwitchRow>>,
        pub paste_helper_row: RefCell<Option<adw::ActionRow>>,
        pub mode_row: RefCell<Option<adw::ComboRow>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DictationPage {
        const NAME: &'static str = "SttDictationPage";
        type Type = super::DictationPage;
        type ParentType = adw::PreferencesPage;
    }

    impl ObjectImpl for DictationPage {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_ui();
        }
    }

    impl WidgetImpl for DictationPage {}
    impl adw::subclass::prelude::PreferencesPageImpl for DictationPage {}
}

glib::wrapper! {
    pub struct DictationPage(ObjectSubclass<imp::DictationPage>)
        @extends gtk::Widget, adw::PreferencesPage;
}

impl DictationPage {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("title", "Dictation")
            .property("icon-name", "input-keyboard-symbolic")
            .build()
    }

    fn setup_ui(&self) {
        let imp = self.imp();

        // === Mini panel & global shortcut ===
        let panel_group = adw::PreferencesGroup::new();
        // Note: PreferencesGroup titles are parsed as Pango markup — avoid a bare '&'.
        panel_group.set_title(gettext("Mini Panel and Global Dictation").as_str());
        panel_group.set_description(Some(&gettext(
            "Dictate into any app from a compact floating panel, opened by a global shortcut.",
        )));

        let mini_panel_switch = adw::SwitchRow::builder()
            .title(gettext("Enable Mini Panel").as_str())
            .subtitle(gettext("Register the global dictation shortcut (restart to apply changes)").as_str())
            .active(true)
            .build();
        panel_group.add(&mini_panel_switch);

        let start_hidden_switch = adw::SwitchRow::builder()
            .title(gettext("Start Hidden in Tray").as_str())
            .subtitle(gettext("Launch with no window — only the tray icon and shortcut (ideal for autostart)").as_str())
            .active(false)
            .build();
        panel_group.add(&start_hidden_switch);

        let shortcut_entry = adw::EntryRow::builder()
            .title(gettext("Preferred Shortcut").as_str())
            .build();
        // The desktop owns the real binding; this is only a suggestion.
        let hint = gtk::Label::new(Some(gettext("Set in GNOME Settings").as_str()));
        hint.add_css_class("dim-label");
        hint.add_css_class("caption");
        shortcut_entry.add_suffix(&hint);
        panel_group.add(&shortcut_entry);

        // Row that opens the desktop keyboard settings where the actual binding lives.
        let configure_row = adw::ActionRow::builder()
            .title(gettext("Configure System Shortcut").as_str())
            .subtitle(gettext("On GNOME, the actual key is set in Settings → Keyboard").as_str())
            .activatable(true)
            .build();
        let open_icon = gtk::Image::from_icon_name("go-next-symbolic");
        configure_row.add_suffix(&open_icon);
        configure_row.connect_activated(|_| {
            // Best-effort: launch GNOME keyboard settings.
            let _ = std::process::Command::new("gnome-control-center")
                .arg("keyboard")
                .spawn();
        });
        panel_group.add(&configure_row);

        self.add(&panel_group);

        // === Auto-paste ===
        let paste_group = adw::PreferencesGroup::new();
        paste_group.set_title(gettext("Auto-paste").as_str());
        paste_group.set_description(Some(&gettext(
            "After dictation the text is always copied to the clipboard. Auto-paste additionally types it into the focused app when possible.",
        )));

        let auto_paste_switch = adw::SwitchRow::builder()
            .title(gettext("Type into the focused app").as_str())
            .subtitle(gettext("Auto-types the transcript via Remote Desktop (asks permission). Off = clipboard only, press Ctrl+V yourself.").as_str())
            .active(false)
            .build();
        paste_group.add(&auto_paste_switch);

        let paste_helper_row = adw::ActionRow::builder()
            .title(gettext("Paste Method").as_str())
            .subtitle(paste_helper_description())
            .build();
        paste_group.add(&paste_helper_row);

        self.add(&paste_group);

        // === Dictation mode ===
        let mode_group = adw::PreferencesGroup::new();
        mode_group.set_title(gettext("Dictation Mode").as_str());
        mode_group.set_description(Some(&gettext(
            "How the transcript is formatted: Plain (clean text), Message (one line), Email (greeting + sign-off), Note (bullets), or Code Prompt (filler-free instruction).",
        )));

        let modes = gtk::StringList::new(&[
            gettext("Plain").as_str(),
            gettext("Message").as_str(),
            gettext("Email").as_str(),
            gettext("Note").as_str(),
            gettext("Code Prompt").as_str(),
        ]);
        let mode_row = adw::ComboRow::builder()
            .title(gettext("Mode").as_str())
            .model(&modes)
            .build();
        mode_group.add(&mode_row);

        self.add(&mode_group);

        // Store references
        *imp.mini_panel_switch.borrow_mut() = Some(mini_panel_switch);
        *imp.start_hidden_switch.borrow_mut() = Some(start_hidden_switch);
        *imp.shortcut_entry.borrow_mut() = Some(shortcut_entry);
        *imp.auto_paste_switch.borrow_mut() = Some(auto_paste_switch);
        *imp.paste_helper_row.borrow_mut() = Some(paste_helper_row);
        *imp.mode_row.borrow_mut() = Some(mode_row);

        // Restore saved values, THEN wire persistence so restoring doesn't save.
        self.load_from_config();
        self.connect_persistence();
    }

    fn load_from_config(&self) {
        let config = AppConfig::load();
        let imp = self.imp();
        if let Some(s) = imp.mini_panel_switch.borrow().as_ref() {
            s.set_active(config.mini_panel_enabled);
        }
        if let Some(s) = imp.start_hidden_switch.borrow().as_ref() {
            s.set_active(config.start_hidden);
        }
        if let Some(e) = imp.shortcut_entry.borrow().as_ref() {
            e.set_text(&config.global_shortcut);
        }
        if let Some(s) = imp.auto_paste_switch.borrow().as_ref() {
            s.set_active(config.auto_paste);
        }
        if let Some(r) = imp.mode_row.borrow().as_ref() {
            let idx = MODE_IDS.iter().position(|m| *m == config.dictation_mode).unwrap_or(0);
            r.set_selected(idx as u32);
        }
    }

    fn connect_persistence(&self) {
        let imp = self.imp();
        if let Some(s) = imp.mini_panel_switch.borrow().as_ref() {
            s.connect_active_notify(|s| {
                let mut c = AppConfig::load();
                c.mini_panel_enabled = s.is_active();
                c.save();
            });
        }
        if let Some(s) = imp.start_hidden_switch.borrow().as_ref() {
            s.connect_active_notify(|s| {
                let mut c = AppConfig::load();
                c.start_hidden = s.is_active();
                c.save();
            });
        }
        if let Some(e) = imp.shortcut_entry.borrow().as_ref() {
            e.connect_changed(|e| {
                let text = e.text().to_string();
                if text.trim().is_empty() {
                    return;
                }
                let mut c = AppConfig::load();
                c.global_shortcut = text;
                c.save();
            });
        }
        if let Some(s) = imp.auto_paste_switch.borrow().as_ref() {
            s.connect_active_notify(|s| {
                let mut c = AppConfig::load();
                c.auto_paste = s.is_active();
                c.save();
            });
        }
        if let Some(r) = imp.mode_row.borrow().as_ref() {
            r.connect_selected_notify(|r| {
                let idx = (r.selected() as usize).min(MODE_IDS.len() - 1);
                let mut c = AppConfig::load();
                c.dictation_mode = MODE_IDS[idx].to_string();
                c.save();
            });
        }
    }
}

/// A user-facing description of the active auto-paste method.
fn paste_helper_description() -> String {
    use crate::portal::paste::PasteHelper;
    match crate::portal::paste::detect_paste_helper() {
        PasteHelper::RemoteDesktopPortal => {
            gettext("Desktop portal (asks for permission once)")
        }
        PasteHelper::Ydotool => gettext("ydotool"),
        PasteHelper::None => gettext("Clipboard only — press Ctrl+V to paste"),
    }
}
