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

/// Write or remove the user autostart entry that launches the app hidden in the
/// tray at login (`Exec=… --hidden`). Only this autostart path starts without a
/// window; launching the app by hand always shows the main window.
fn set_autostart_hidden(enabled: bool) {
    let dir = glib::user_config_dir().join("autostart");
    let path = dir.join(format!("{}.desktop", crate::APP_ID));
    if enabled {
        let exec = std::env::current_exe()
            .ok()
            .and_then(|p| p.to_str().map(String::from))
            .unwrap_or_else(|| "speech-to-text".to_string());
        let _ = std::fs::create_dir_all(&dir);
        let content = format!(
            "[Desktop Entry]\nType=Application\nName={}\nExec={} --hidden\nIcon={}\nX-GNOME-Autostart-enabled=true\n",
            crate::APP_NAME, exec, crate::APP_ID,
        );
        let _ = std::fs::write(&path, content);
    } else {
        let _ = std::fs::remove_file(&path);
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct DictationPage {
        pub mini_panel_switch: RefCell<Option<adw::SwitchRow>>,
        pub always_on_top_switch: RefCell<Option<adw::SwitchRow>>,
        pub start_hidden_switch: RefCell<Option<adw::SwitchRow>>,
        pub shortcut_entry: RefCell<Option<adw::EntryRow>>,
        pub auto_paste_switch: RefCell<Option<adw::SwitchRow>>,
        pub paste_helper_row: RefCell<Option<adw::ActionRow>>,
        pub mode_row: RefCell<Option<adw::ComboRow>>,
        pub update_check_switch: RefCell<Option<adw::SwitchRow>>,
        pub live_transcription_switch: RefCell<Option<adw::SwitchRow>>,
        /// Guards programmatic toggles so consent dialogs don't re-fire.
        pub loading: std::cell::Cell<bool>,
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

        let always_on_top_switch = adw::SwitchRow::builder()
            .title(gettext("Keep Mini Panel on Top").as_str())
            .subtitle(gettext("Best-effort: re-raises the panel when it loses focus. On GNOME/Wayland use the panel's titlebar menu → \"Always on Top\" for a reliable result.").as_str())
            .active(false)
            .build();
        panel_group.add(&always_on_top_switch);

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

        // Let the user revoke the Remote Desktop input-injection permission that
        // auto-paste persists (deletes the stored restore token).
        let revoke_row = adw::ActionRow::builder()
            .title(gettext("Revoke Paste Permission").as_str())
            .subtitle(gettext(
                "Forget the granted Remote Desktop permission. You'll be asked again the next time you auto-paste.",
            ).as_str())
            .build();
        let revoke_btn = gtk::Button::builder()
            .label(gettext("Revoke").as_str())
            .valign(gtk::Align::Center)
            .build();
        revoke_btn.add_css_class("flat");
        {
            let row_weak = revoke_row.downgrade();
            revoke_btn.connect_clicked(move |_| {
                let ok = crate::portal::paste::revoke_restore_token();
                if let Some(row) = row_weak.upgrade() {
                    row.set_subtitle(&if ok {
                        gettext("Permission revoked — you'll be asked again next time you auto-paste.")
                    } else {
                        gettext("Could not revoke the permission; see the logs for details.")
                    });
                }
            });
        }
        revoke_row.add_suffix(&revoke_btn);
        paste_group.add(&revoke_row);

        self.add(&paste_group);

        // === Privacy / updates ===
        let updates_group = adw::PreferencesGroup::new();
        updates_group.set_title(gettext("Privacy").as_str());
        updates_group.set_description(Some(&gettext(
            "Transcription runs locally. The only automatic network request is an optional check for a newer release at startup.",
        )));

        let update_check_switch = adw::SwitchRow::builder()
            .title(gettext("Check for updates on startup").as_str())
            .subtitle(gettext("Contacts GitHub to see if a newer version is available.").as_str())
            .active(true)
            .build();
        updates_group.add(&update_check_switch);

        self.add(&updates_group);

        // === Live transcription ===
        let live_group = adw::PreferencesGroup::new();
        live_group.set_title(gettext("Live Transcription").as_str());
        live_group.set_description(Some(&gettext(
            "Show tentative text in the main window while you are still speaking (Whisper only). Does not apply to the mini panel — that always uses a clean batch decode. The final result is always the full-accuracy decode.",
        )));
        let live_transcription_switch = adw::SwitchRow::builder()
            .title(gettext("Show text live while transcribing (main window only)").as_str())
            .active(false)
            .build();
        live_group.add(&live_transcription_switch);
        self.add(&live_group);

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
        *imp.always_on_top_switch.borrow_mut() = Some(always_on_top_switch);
        *imp.start_hidden_switch.borrow_mut() = Some(start_hidden_switch);
        *imp.shortcut_entry.borrow_mut() = Some(shortcut_entry);
        *imp.auto_paste_switch.borrow_mut() = Some(auto_paste_switch);
        *imp.paste_helper_row.borrow_mut() = Some(paste_helper_row);
        *imp.mode_row.borrow_mut() = Some(mode_row);
        *imp.update_check_switch.borrow_mut() = Some(update_check_switch);
        *imp.live_transcription_switch.borrow_mut() = Some(live_transcription_switch);

        // Restore saved values, THEN wire persistence so restoring doesn't save.
        self.load_from_config();
        self.connect_persistence();
    }

    fn load_from_config(&self) {
        let config = AppConfig::load();
        let imp = self.imp();
        imp.loading.set(true);
        if let Some(s) = imp.mini_panel_switch.borrow().as_ref() {
            s.set_active(config.mini_panel_enabled);
        }
        if let Some(s) = imp.always_on_top_switch.borrow().as_ref() {
            s.set_active(config.mini_panel_always_on_top);
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
        if let Some(s) = imp.update_check_switch.borrow().as_ref() {
            s.set_active(config.update_check_enabled);
        }
        if let Some(s) = imp.live_transcription_switch.borrow().as_ref() {
            s.set_active(config.live_transcription);
        }
        imp.loading.set(false);
    }

    /// Consent before enabling auto-paste (typing into other apps).
    fn confirm_auto_paste(&self, switch: &adw::SwitchRow) {
        let dialog = adw::AlertDialog::new(
            Some(gettext("Enable auto-paste?").as_str()),
            Some(gettext(
                "After dictation, the app will type the transcript into whichever window is focused, \
                 using the Remote Desktop portal — your desktop will ask for permission the first time. \
                 The transcript is always copied to the clipboard regardless of this setting.",
            ).as_str()),
        );
        dialog.add_response("cancel", gettext("Cancel").as_str());
        dialog.add_response("enable", gettext("Enable").as_str());
        dialog.set_response_appearance("enable", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("enable"));
        dialog.set_close_response("cancel");

        let page = self.clone();
        let switch = switch.clone();
        dialog.choose(self, gtk::gio::Cancellable::NONE, move |resp| {
            if resp.as_str() == "enable" {
                let mut c = AppConfig::load();
                c.auto_paste = true;
                c.save();
            } else {
                // Revert the toggle without re-triggering the consent handler.
                page.imp().loading.set(true);
                switch.set_active(false);
                page.imp().loading.set(false);
            }
        });
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
        if let Some(s) = imp.always_on_top_switch.borrow().as_ref() {
            s.connect_active_notify(|s| {
                let mut c = AppConfig::load();
                c.mini_panel_always_on_top = s.is_active();
                c.save();
            });
        }
        if let Some(s) = imp.start_hidden_switch.borrow().as_ref() {
            s.connect_active_notify(|s| {
                let active = s.is_active();
                let mut c = AppConfig::load();
                c.start_hidden = active;
                c.save();
                // Create/remove the autostart entry that launches the app
                // hidden at login. Manual launches still show the window.
                set_autostart_hidden(active);
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
            let page = self.clone();
            s.connect_active_notify(move |s| {
                if page.imp().loading.get() {
                    return;
                }
                if s.is_active() {
                    // Ask for consent before enabling typing into other apps.
                    page.confirm_auto_paste(s);
                } else {
                    let mut c = AppConfig::load();
                    c.auto_paste = false;
                    c.save();
                }
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
        if let Some(s) = imp.update_check_switch.borrow().as_ref() {
            s.connect_active_notify(|s| {
                let mut c = AppConfig::load();
                c.update_check_enabled = s.is_active();
                c.save();
            });
        }
        if let Some(s) = imp.live_transcription_switch.borrow().as_ref() {
            let page = self.clone();
            s.connect_active_notify(move |s| {
                if page.imp().loading.get() {
                    return;
                }
                let mut c = AppConfig::load();
                c.live_transcription = s.is_active();
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
