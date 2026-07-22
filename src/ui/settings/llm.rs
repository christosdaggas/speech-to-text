// Speech to Text - LLM Settings Page
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Configure the connection to a local/cloud OpenAI-compatible LLM (LM Studio,
//! Ollama, vLLM, OpenAI …) and the editable "Improve with AI" prompt presets.
//!
//! The API key is stored in the system keyring (never in the plaintext config).
//! Model discovery (`GET /models`) populates a dropdown; a free-text entry is
//! always available as a fallback when a server doesn't expose `/models`.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk4 as gtk;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::application::tokio_runtime;
use crate::config::{AppConfig, LlmPreset};
use crate::i18n::gettext;
use crate::llm::LlmConfig;

/// Provider quick-fill presets: (label, default base URL).
const PROVIDERS: [(&str, &str); 5] = [
    ("LM Studio", "http://localhost:1234/v1"),
    ("Ollama", "http://localhost:11434/v1"),
    ("vLLM", "http://localhost:8000/v1"),
    ("OpenAI", "https://api.openai.com/v1"),
    ("Custom", ""),
];

/// Target languages offered for translate presets.
const TRANSLATE_LANGS: [&str; 11] = [
    "English",
    "Greek",
    "Spanish",
    "French",
    "German",
    "Italian",
    "Portuguese",
    "Russian",
    "Chinese",
    "Japanese",
    "Arabic",
];

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct LlmPage {
        // Connection
        pub enable_switch: RefCell<Option<adw::SwitchRow>>,
        pub provider_combo: RefCell<Option<adw::ComboRow>>,
        pub url_entry: RefCell<Option<adw::EntryRow>>,
        pub key_entry: RefCell<Option<adw::PasswordEntryRow>>,
        pub model_combo: RefCell<Option<adw::ComboRow>>,
        pub model_entry: RefCell<Option<adw::EntryRow>>,
        pub temp_spin: RefCell<Option<adw::SpinRow>>,
        pub test_status: RefCell<Option<gtk::Label>>,

        // Presets
        pub preset_combo: RefCell<Option<adw::ComboRow>>,
        pub name_entry: RefCell<Option<adw::EntryRow>>,
        pub prompt_view: RefCell<Option<gtk::TextView>>,
        pub preset_model_combo: RefCell<Option<adw::ComboRow>>,
        pub temp_override_switch: RefCell<Option<adw::SwitchRow>>,
        pub preset_temp_spin: RefCell<Option<adw::SpinRow>>,
        pub translate_combo: RefCell<Option<adw::ComboRow>>,
        pub auto_switch: RefCell<Option<adw::SwitchRow>>,
        pub auto_summary_switch: RefCell<Option<adw::SwitchRow>>,

        // System-wide transform
        pub selection_switch: RefCell<Option<adw::SwitchRow>>,
        pub selection_shortcut_entry: RefCell<Option<adw::EntryRow>>,

        // Shared state
        pub models: Rc<RefCell<Vec<String>>>,
        pub loading: Rc<Cell<bool>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LlmPage {
        const NAME: &'static str = "SttLlmPage";
        type Type = super::LlmPage;
        type ParentType = adw::PreferencesPage;
    }

    impl ObjectImpl for LlmPage {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_ui();
        }
    }

    impl WidgetImpl for LlmPage {}
    impl adw::subclass::prelude::PreferencesPageImpl for LlmPage {}
}

glib::wrapper! {
    pub struct LlmPage(ObjectSubclass<imp::LlmPage>)
        @extends gtk::Widget, adw::PreferencesPage;
}

impl Default for LlmPage {
    fn default() -> Self {
        Self::new()
    }
}

impl LlmPage {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("title", "LLM")
            .property("icon-name", "network-transmit-receive-symbolic")
            .build()
    }

    fn setup_ui(&self) {
        let imp = self.imp();

        // ── Connection ──────────────────────────────────────────────
        let conn_group = adw::PreferencesGroup::new();
        conn_group.set_title(gettext("LLM Connection").as_str());
        conn_group.set_description(Some(&gettext(
            "Send transcripts to a local or cloud LLM to clean up, rewrite, summarize or translate them. Works with any OpenAI-compatible server (LM Studio, Ollama, vLLM, OpenAI).",
        )));

        let enable_switch = adw::SwitchRow::builder()
            .title(gettext("Enable LLM").as_str())
            .subtitle(
                gettext("Show the \"Improve with AI\" action and the auto-improve option").as_str(),
            )
            .build();
        conn_group.add(&enable_switch);

        let provider_names: Vec<&str> = PROVIDERS.iter().map(|(n, _)| *n).collect();
        let provider_combo = adw::ComboRow::builder()
            .title(gettext("Provider preset").as_str())
            .subtitle(gettext("Fills the API URL with a common default").as_str())
            .model(&gtk::StringList::new(&provider_names))
            .build();
        conn_group.add(&provider_combo);

        let url_entry = adw::EntryRow::builder()
            .title(gettext("API URL").as_str())
            .show_apply_button(true)
            .build();
        conn_group.add(&url_entry);

        // PasswordEntryRow masks the key and provides a built-in reveal toggle,
        // so the secret isn't shoulder-surfable in the settings UI.
        let key_entry = adw::PasswordEntryRow::builder()
            .title(gettext("API Key (optional)").as_str())
            .show_apply_button(true)
            .build();
        let key_hint = gtk::Label::new(Some(gettext("Stored in keyring").as_str()));
        key_hint.add_css_class("dim-label");
        key_hint.add_css_class("caption");
        key_entry.add_suffix(&key_hint);
        conn_group.add(&key_entry);

        // Discovered-model dropdown + refresh.
        let model_combo = adw::ComboRow::builder()
            .title(gettext("Model").as_str())
            .subtitle(gettext("Discovered from the server").as_str())
            .model(&gtk::StringList::new(&[]))
            .build();
        let refresh_btn = gtk::Button::from_icon_name("view-refresh-symbolic");
        refresh_btn.set_tooltip_text(Some(&gettext("Refresh model list")));
        refresh_btn.add_css_class("flat");
        refresh_btn.set_valign(gtk::Align::Center);
        model_combo.add_suffix(&refresh_btn);
        conn_group.add(&model_combo);

        // Manual fallback (servers without /models, or to override).
        let model_entry = adw::EntryRow::builder()
            .title(gettext("Model name (manual)").as_str())
            .show_apply_button(true)
            .build();
        conn_group.add(&model_entry);

        let temp_adj = gtk::Adjustment::new(0.3, 0.0, 1.0, 0.05, 0.1, 0.0);
        let temp_spin = adw::SpinRow::new(Some(&temp_adj), 0.05, 2);
        temp_spin.set_title(&gettext("Temperature"));
        temp_spin.set_subtitle(&gettext("Lower = more deterministic"));
        conn_group.add(&temp_spin);

        // Test connection row.
        let test_row = adw::ActionRow::builder()
            .title(gettext("Test connection").as_str())
            .build();
        let test_status = gtk::Label::new(None);
        test_status.add_css_class("dim-label");
        test_status.add_css_class("caption");
        test_status.set_wrap(true);
        test_status.set_xalign(1.0);
        test_status.set_max_width_chars(40);
        let test_btn = gtk::Button::with_label(&gettext("Test"));
        test_btn.add_css_class("flat");
        test_btn.set_valign(gtk::Align::Center);
        test_row.add_suffix(&test_status);
        test_row.add_suffix(&test_btn);
        conn_group.add(&test_row);

        self.add(&conn_group);

        // ── Prompt presets ──────────────────────────────────────────
        let preset_group = adw::PreferencesGroup::new();
        preset_group.set_title(gettext("Prompt Presets").as_str());
        preset_group.set_description(Some(&gettext(
            "Reusable instructions for the LLM. The active preset is used by \"Improve with AI\", the mini panel and the transform-selection shortcut.",
        )));

        let preset_combo = adw::ComboRow::builder()
            .title(gettext("Active preset").as_str())
            .model(&gtk::StringList::new(&[]))
            .build();
        preset_group.add(&preset_combo);

        let name_entry = adw::EntryRow::builder()
            .title(gettext("Name").as_str())
            .show_apply_button(true)
            .build();
        preset_group.add(&name_entry);

        // Preset action buttons (Add / Duplicate / Delete).
        let btn_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        btn_box.set_margin_top(6);
        btn_box.set_margin_bottom(6);
        btn_box.set_halign(gtk::Align::End);
        let add_btn = gtk::Button::with_label(&gettext("Add"));
        let dup_btn = gtk::Button::with_label(&gettext("Duplicate"));
        let del_btn = gtk::Button::with_label(&gettext("Delete"));
        del_btn.add_css_class("destructive-action");
        btn_box.append(&add_btn);
        btn_box.append(&dup_btn);
        btn_box.append(&del_btn);
        preset_group.add(&btn_box);

        self.add(&preset_group);

        // Prompt editor (multi-line; adw rows are single-line).
        let prompt_group = adw::PreferencesGroup::new();
        prompt_group.set_title(gettext("Prompt").as_str());
        prompt_group.set_description(Some(&gettext(
            "The system instruction sent with the transcript. For translate presets, the {lang} placeholder is filled from the \"Translate to\" picker.",
        )));
        let prompt_view = gtk::TextView::new();
        prompt_view.set_wrap_mode(gtk::WrapMode::Word);
        prompt_view.set_top_margin(8);
        prompt_view.set_bottom_margin(8);
        prompt_view.set_left_margin(8);
        prompt_view.set_right_margin(8);
        let scroller = gtk::ScrolledWindow::builder()
            .min_content_height(110)
            .max_content_height(220)
            .vexpand(false)
            .child(&prompt_view)
            .build();
        let frame = gtk::Frame::new(None);
        frame.set_child(Some(&scroller));
        prompt_group.add(&frame);
        self.add(&prompt_group);

        // ── Per-preset overrides & behaviour ────────────────────────
        let over_group = adw::PreferencesGroup::new();
        over_group.set_title(gettext("Preset Options").as_str());

        let preset_model_combo = adw::ComboRow::builder()
            .title(gettext("Model (this preset)").as_str())
            .subtitle(gettext("Overrides the connection model").as_str())
            .model(&gtk::StringList::new(&[gettext("Default").as_str()]))
            .build();
        over_group.add(&preset_model_combo);

        let temp_override_switch = adw::SwitchRow::builder()
            .title(gettext("Override temperature").as_str())
            .build();
        over_group.add(&temp_override_switch);

        let ptemp_adj = gtk::Adjustment::new(0.3, 0.0, 1.0, 0.05, 0.1, 0.0);
        let preset_temp_spin = adw::SpinRow::new(Some(&ptemp_adj), 0.05, 2);
        preset_temp_spin.set_title(&gettext("Temperature (this preset)"));
        preset_temp_spin.set_sensitive(false);
        over_group.add(&preset_temp_spin);

        let mut tr_items: Vec<String> = vec![gettext("— not a translate preset —")];
        tr_items.extend(TRANSLATE_LANGS.iter().map(|s| s.to_string()));
        let tr_refs: Vec<&str> = tr_items.iter().map(|s| s.as_str()).collect();
        let translate_combo = adw::ComboRow::builder()
            .title(gettext("Translate to").as_str())
            .subtitle(gettext("Makes this preset translate the transcript").as_str())
            .model(&gtk::StringList::new(&tr_refs))
            .build();
        over_group.add(&translate_combo);

        let auto_switch = adw::SwitchRow::builder()
            .title(gettext("Auto-improve after dictation").as_str())
            .subtitle(
                gettext("Runs the active preset automatically on every dictation (off by default)")
                    .as_str(),
            )
            .build();
        over_group.add(&auto_switch);

        let auto_summary_switch = adw::SwitchRow::builder()
            .title(gettext("Automatically summarize long transcripts").as_str())
            .subtitle(gettext("Sends long transcripts to the configured endpoint after transcription (off by default)").as_str())
            .build();
        over_group.add(&auto_summary_switch);

        self.add(&over_group);

        // ── System-wide transform ───────────────────────────────────
        let sys_group = adw::PreferencesGroup::new();
        sys_group.set_title(gettext("System-wide Transform").as_str());
        sys_group.set_description(Some(&gettext(
            "Transform highlighted or copied text in any app with the active preset, via a global shortcut and the tray menu. The result is pasted back.",
        )));

        let selection_switch = adw::SwitchRow::builder()
            .title(gettext("Enable transform-selection shortcut").as_str())
            .subtitle(gettext("Register a global shortcut (restart to apply)").as_str())
            .build();
        sys_group.add(&selection_switch);

        let selection_shortcut_entry = adw::EntryRow::builder()
            .title(gettext("Shortcut").as_str())
            .show_apply_button(true)
            .build();
        let sc_hint = gtk::Label::new(Some(gettext("e.g. <Ctrl><Alt>i").as_str()));
        sc_hint.add_css_class("dim-label");
        sc_hint.add_css_class("caption");
        selection_shortcut_entry.add_suffix(&sc_hint);
        sys_group.add(&selection_shortcut_entry);

        self.add(&sys_group);

        // Store references.
        *imp.enable_switch.borrow_mut() = Some(enable_switch);
        *imp.provider_combo.borrow_mut() = Some(provider_combo);
        *imp.url_entry.borrow_mut() = Some(url_entry);
        *imp.key_entry.borrow_mut() = Some(key_entry);
        *imp.model_combo.borrow_mut() = Some(model_combo);
        *imp.model_entry.borrow_mut() = Some(model_entry);
        *imp.temp_spin.borrow_mut() = Some(temp_spin);
        *imp.test_status.borrow_mut() = Some(test_status);
        *imp.preset_combo.borrow_mut() = Some(preset_combo);
        *imp.name_entry.borrow_mut() = Some(name_entry);
        *imp.prompt_view.borrow_mut() = Some(prompt_view);
        *imp.preset_model_combo.borrow_mut() = Some(preset_model_combo);
        *imp.temp_override_switch.borrow_mut() = Some(temp_override_switch);
        *imp.preset_temp_spin.borrow_mut() = Some(preset_temp_spin);
        *imp.translate_combo.borrow_mut() = Some(translate_combo);
        *imp.auto_switch.borrow_mut() = Some(auto_switch);
        *imp.auto_summary_switch.borrow_mut() = Some(auto_summary_switch);
        *imp.selection_switch.borrow_mut() = Some(selection_switch);
        *imp.selection_shortcut_entry.borrow_mut() = Some(selection_shortcut_entry);

        // Wire button handlers (need the page handle).
        let page = self.clone();
        add_btn.connect_clicked(move |_| page.add_preset());
        let page = self.clone();
        dup_btn.connect_clicked(move |_| page.duplicate_preset());
        let page = self.clone();
        del_btn.connect_clicked(move |_| page.delete_preset());
        let page = self.clone();
        refresh_btn.connect_clicked(move |_| page.refresh_models());
        let page = self.clone();
        test_btn.connect_clicked(move |_| page.test_connection());

        // Restore saved values, THEN wire persistence so restoring doesn't save.
        self.load_from_config();
        self.connect_persistence();

        // Re-sync the switches each time the page is shown, so changes made
        // elsewhere (e.g. the toolbar "Improve with AI" toggle) are reflected.
        let page = self.clone();
        self.connect_map(move |_| page.sync_state());
    }

    /// Lightweight re-sync of the enable / auto-improve / selection switches from
    /// config (no model refetch). Called when the page becomes visible.
    fn sync_state(&self) {
        let imp = self.imp();
        let cfg = AppConfig::load();
        imp.loading.set(true);
        if let Some(s) = imp.enable_switch.borrow().as_ref() {
            s.set_active(cfg.llm_enabled);
        }
        if let Some(s) = imp.auto_switch.borrow().as_ref() {
            s.set_active(cfg.llm_auto_apply);
        }
        if let Some(s) = imp.auto_summary_switch.borrow().as_ref() {
            s.set_active(cfg.llm_auto_summary);
        }
        if let Some(s) = imp.selection_switch.borrow().as_ref() {
            s.set_active(cfg.llm_selection_enabled);
        }
        imp.loading.set(false);
    }

    // ── Helpers ─────────────────────────────────────────────────────

    fn current_llm_config(&self) -> LlmConfig {
        let c = AppConfig::load();
        LlmConfig {
            api_url: c.llm_api_url,
            api_key: None, // loaded from keyring inside the async helpers
            model: c.llm_model,
            temperature: c.llm_temperature,
        }
    }

    fn current_preset_index(&self) -> usize {
        self.imp()
            .preset_combo
            .borrow()
            .as_ref()
            .map(|c| c.selected() as usize)
            .unwrap_or(0)
    }

    /// Read the selected string of a ComboRow backed by a StringList.
    fn combo_text(c: &adw::ComboRow) -> Option<String> {
        c.selected_item()
            .and_downcast::<gtk::StringObject>()
            .map(|s| s.string().to_string())
    }

    /// Mutate the active preset and persist.
    fn update_active_preset(&self, f: impl FnOnce(&mut LlmPreset)) {
        if self.imp().loading.get() {
            return;
        }
        let idx = self.current_preset_index();
        let mut c = AppConfig::load();
        if let Some(p) = c.llm_presets.get_mut(idx) {
            f(p);
            c.save();
        }
    }

    // ── Config <-> UI ───────────────────────────────────────────────

    fn load_from_config(&self) {
        let imp = self.imp();
        let cfg = AppConfig::load();

        imp.loading.set(true);
        if let Some(s) = imp.enable_switch.borrow().as_ref() {
            s.set_active(cfg.llm_enabled);
        }
        if let Some(e) = imp.url_entry.borrow().as_ref() {
            e.set_text(&cfg.llm_api_url);
        }
        if let Some(e) = imp.model_entry.borrow().as_ref() {
            e.set_text(&cfg.llm_model);
        }
        if let Some(s) = imp.temp_spin.borrow().as_ref() {
            s.set_value(cfg.llm_temperature as f64);
        }
        if let Some(s) = imp.auto_switch.borrow().as_ref() {
            s.set_active(cfg.llm_auto_apply);
        }
        if let Some(s) = imp.auto_summary_switch.borrow().as_ref() {
            s.set_active(cfg.llm_auto_summary);
        }
        if let Some(s) = imp.selection_switch.borrow().as_ref() {
            s.set_active(cfg.llm_selection_enabled);
        }
        if let Some(e) = imp.selection_shortcut_entry.borrow().as_ref() {
            e.set_text(&cfg.llm_selection_shortcut);
        }
        // Provider combo: match the saved URL to a known provider, else "Custom".
        if let Some(c) = imp.provider_combo.borrow().as_ref() {
            let idx = PROVIDERS
                .iter()
                .position(|(_, url)| !url.is_empty() && *url == cfg.llm_api_url)
                .unwrap_or(PROVIDERS.len() - 1);
            c.set_selected(idx as u32);
        }
        imp.loading.set(false);

        // Pre-fill the API key from the keyring (async).
        if let Some(entry) = imp.key_entry.borrow().as_ref() {
            let entry = entry.clone();
            let (tx, rx) = async_channel::bounded::<Option<String>>(1);
            tokio_runtime().spawn(async move {
                let _ = tx.send(crate::secrets::load_llm_api_key().await).await;
            });
            glib::spawn_future_local(async move {
                if let Ok(Some(key)) = rx.recv().await {
                    entry.set_text(&key);
                }
            });
        }

        // Build the preset combo and load the active preset into the editor.
        let active = cfg
            .llm_active_preset
            .min(cfg.llm_presets.len().saturating_sub(1));
        self.rebuild_preset_combo(active);

        // Discover models if the integration is enabled and a URL is set.
        if cfg.llm_enabled && !cfg.llm_api_url.trim().is_empty() {
            self.refresh_models();
        }
    }

    /// Show a simple informational/warning dialog with an OK button.
    fn warn_dialog(&self, title: &str, body: &str) {
        let dialog = adw::AlertDialog::new(Some(title), Some(body));
        dialog.add_response("ok", gettext("OK").as_str());
        dialog.set_default_response(Some("ok"));
        dialog.set_close_response("ok");
        dialog.present(Some(self));
    }

    fn save_endpoint(&self, url: &str) {
        let mut config = AppConfig::load();
        let old_scope = crate::llm::consent_scope(&config.llm_api_url);
        let new_scope = crate::llm::consent_scope(url);
        let consent_invalidated =
            old_scope != new_scope && (config.llm_enabled || config.llm_consent_given);

        config.llm_api_url = url.to_string();
        if consent_invalidated {
            config.llm_enabled = false;
            config.llm_consent_given = false;
            config.llm_consent_endpoint = None;
        }
        config.save();

        if consent_invalidated {
            self.imp().loading.set(true);
            if let Some(enable) = self.imp().enable_switch.borrow().as_ref() {
                enable.set_active(false);
            }
            self.imp().loading.set(false);
            self.warn_dialog(
                gettext("LLM consent required").as_str(),
                gettext("The LLM destination changed. Review the new endpoint and enable the LLM again to consent.").as_str(),
            );
        }
    }

    /// First-time privacy consent before enabling "Improve with AI", naming the
    /// host transcript text would be sent to.
    fn confirm_llm_enable(&self, switch: &adw::SwitchRow) {
        let cfg = AppConfig::load();
        let Some(consent_scope) = crate::llm::consent_scope(&cfg.llm_api_url) else {
            self.imp().loading.set(true);
            switch.set_active(false);
            self.imp().loading.set(false);
            self.warn_dialog(
                gettext("Endpoint not allowed").as_str(),
                gettext("Enter a valid, allowed LLM endpoint before enabling the integration.")
                    .as_str(),
            );
            return;
        };
        let host = crate::llm::endpoint_host(&cfg.llm_api_url)
            .unwrap_or_else(|| gettext("the configured endpoint"));
        let body = gettext(
            "\"Improve with AI\" will send your transcript text to {host}. If that is a remote or \
             cloud service, your text leaves your device. Only enable this for an endpoint you trust.",
        )
        .replace("{host}", &host);

        let dialog = adw::AlertDialog::new(
            Some(gettext("Send transcripts to an LLM?").as_str()),
            Some(&body),
        );
        dialog.add_response("cancel", gettext("Cancel").as_str());
        dialog.add_response("enable", gettext("Enable").as_str());
        dialog.set_response_appearance("enable", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");

        let page = self.clone();
        let switch = switch.clone();
        dialog.choose(self, gtk::gio::Cancellable::NONE, move |resp| {
            if resp.as_str() == "enable" {
                let mut c = AppConfig::load();
                c.llm_enabled = true;
                c.llm_consent_given = true;
                c.llm_consent_endpoint = Some(consent_scope.clone());
                c.save();
                if !c.llm_api_url.trim().is_empty() {
                    page.refresh_models();
                }
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

        if let Some(s) = imp.enable_switch.borrow().as_ref() {
            let page = self.clone();
            s.connect_active_notify(move |s| {
                if page.imp().loading.get() {
                    return;
                }
                if s.is_active() {
                    let mut c = AppConfig::load();
                    let consent_matches = crate::llm::consent_scope(&c.llm_api_url).as_ref()
                        == c.llm_consent_endpoint.as_ref();
                    if c.llm_consent_given && consent_matches {
                        c.llm_enabled = true;
                        c.save();
                        if !c.llm_api_url.trim().is_empty() {
                            page.refresh_models();
                        }
                    } else {
                        // First time: show a privacy consent dialog naming the host.
                        page.confirm_llm_enable(s);
                    }
                } else {
                    let mut c = AppConfig::load();
                    c.llm_enabled = false;
                    c.save();
                }
            });
        }

        if let Some(c) = imp.provider_combo.borrow().as_ref() {
            let page = self.clone();
            c.connect_selected_notify(move |c| {
                if page.imp().loading.get() {
                    return;
                }
                let idx = (c.selected() as usize).min(PROVIDERS.len() - 1);
                let (_, url) = PROVIDERS[idx];
                if url.is_empty() {
                    return; // "Custom" leaves the URL untouched
                }
                if let Some(e) = page.imp().url_entry.borrow().as_ref() {
                    e.set_text(url);
                }
                page.save_endpoint(url);
                page.refresh_models();
            });
        }

        if let Some(e) = imp.url_entry.borrow().as_ref() {
            let page = self.clone();
            e.connect_apply(move |e| {
                let url = e.text().to_string();
                // Give immediate feedback if the URL would be rejected at send time.
                if let Err(err) = crate::llm::validate_endpoint(&url) {
                    page.warn_dialog(
                        gettext("Endpoint not allowed").as_str(),
                        &err.user_message(),
                    );
                    return;
                }
                page.save_endpoint(&url);
                page.refresh_models();
            });
        }

        if let Some(e) = imp.key_entry.borrow().as_ref() {
            let page = self.clone();
            e.connect_apply(move |e| {
                let text = e.text().to_string();
                let (tx, rx) = async_channel::bounded::<Result<(), String>>(1);
                tokio_runtime().spawn(async move {
                    let res = if text.is_empty() {
                        crate::secrets::delete_llm_api_key().await
                    } else {
                        crate::secrets::store_llm_api_key(&text).await
                    };
                    let _ = tx.send(res.map_err(|e| e.to_string())).await;
                });
                let page2 = page.clone();
                glib::spawn_future_local(async move {
                    if let Ok(Err(err)) = rx.recv().await {
                        tracing::warn!("Could not store LLM API key in keyring: {}", crate::error::redact_secrets(&err));
                        let body = format!(
                            "{}\n\n{}",
                            gettext("Couldn't save the API key to the system keyring, so it was not stored. Check that a keyring service (GNOME Keyring / KWallet) is running and unlocked."),
                            crate::error::redact_secrets(&err)
                        );
                        page2.warn_dialog(gettext("Keyring Error").as_str(), &body);
                    }
                });
                page.refresh_models();
            });
        }

        if let Some(c) = imp.model_combo.borrow().as_ref() {
            let page = self.clone();
            c.connect_selected_notify(move |c| {
                if page.imp().loading.get() {
                    return;
                }
                if let Some(model) = Self::combo_text(c) {
                    let mut cfg = AppConfig::load();
                    cfg.llm_model = model.clone();
                    cfg.save();
                    if let Some(e) = page.imp().model_entry.borrow().as_ref() {
                        page.imp().loading.set(true);
                        e.set_text(&model);
                        page.imp().loading.set(false);
                    }
                }
            });
        }

        if let Some(e) = imp.model_entry.borrow().as_ref() {
            let page = self.clone();
            e.connect_apply(move |e| {
                let model = e.text().to_string();
                let mut c = AppConfig::load();
                c.llm_model = model;
                c.save();
                // Reflect into the dropdown (adds it if missing).
                let models = page.imp().models.borrow().clone();
                page.set_model_list(models);
            });
        }

        if let Some(s) = imp.temp_spin.borrow().as_ref() {
            let page = self.clone();
            s.adjustment().connect_value_changed(move |adj| {
                if page.imp().loading.get() {
                    return;
                }
                let mut c = AppConfig::load();
                c.llm_temperature = adj.value() as f32;
                c.save();
            });
        }

        // Preset combo = active preset selector + editor source.
        if let Some(c) = imp.preset_combo.borrow().as_ref() {
            let page = self.clone();
            c.connect_selected_notify(move |c| {
                if page.imp().loading.get() {
                    return;
                }
                let idx = c.selected() as usize;
                let mut cfg = AppConfig::load();
                cfg.llm_active_preset = idx;
                cfg.save();
                page.load_preset_into_editor(idx);
            });
        }

        if let Some(e) = imp.name_entry.borrow().as_ref() {
            let page = self.clone();
            e.connect_apply(move |e| {
                let name = e.text().to_string();
                if name.trim().is_empty() {
                    return;
                }
                page.update_active_preset(|p| p.name = name.clone());
                let idx = page.current_preset_index();
                page.rebuild_preset_combo(idx);
            });
        }

        if let Some(v) = imp.prompt_view.borrow().as_ref() {
            let page = self.clone();
            v.buffer().connect_changed(move |buf| {
                if page.imp().loading.get() {
                    return;
                }
                let text = buf
                    .text(&buf.start_iter(), &buf.end_iter(), false)
                    .to_string();
                page.update_active_preset(|p| p.prompt = text.clone());
            });
        }

        if let Some(c) = imp.preset_model_combo.borrow().as_ref() {
            let page = self.clone();
            c.connect_selected_notify(move |c| {
                if page.imp().loading.get() {
                    return;
                }
                let model = if c.selected() == 0 {
                    None
                } else {
                    Self::combo_text(c)
                };
                page.update_active_preset(|p| p.model = model.clone());
            });
        }

        if let Some(s) = imp.temp_override_switch.borrow().as_ref() {
            let page = self.clone();
            s.connect_active_notify(move |s| {
                let active = s.is_active();
                if let Some(sp) = page.imp().preset_temp_spin.borrow().as_ref() {
                    sp.set_sensitive(active);
                }
                if page.imp().loading.get() {
                    return;
                }
                let value = page
                    .imp()
                    .preset_temp_spin
                    .borrow()
                    .as_ref()
                    .map(|sp| sp.value() as f32)
                    .unwrap_or(0.3);
                page.update_active_preset(|p| {
                    p.temperature = if active { Some(value) } else { None }
                });
            });
        }

        if let Some(s) = imp.preset_temp_spin.borrow().as_ref() {
            let page = self.clone();
            s.adjustment().connect_value_changed(move |adj| {
                if page.imp().loading.get() {
                    return;
                }
                let on = page
                    .imp()
                    .temp_override_switch
                    .borrow()
                    .as_ref()
                    .map(|s| s.is_active())
                    .unwrap_or(false);
                if !on {
                    return;
                }
                let value = adj.value() as f32;
                page.update_active_preset(|p| p.temperature = Some(value));
            });
        }

        if let Some(c) = imp.translate_combo.borrow().as_ref() {
            let page = self.clone();
            c.connect_selected_notify(move |c| {
                if page.imp().loading.get() {
                    return;
                }
                let sel = c.selected() as usize;
                let lang = if sel == 0 {
                    None
                } else {
                    TRANSLATE_LANGS.get(sel - 1).map(|s| s.to_string())
                };
                // The prompt stays editable for translate presets too — the
                // {lang} placeholder is substituted at request time.
                page.update_active_preset(|p| p.translate_to = lang.clone());
            });
        }

        if let Some(s) = imp.auto_switch.borrow().as_ref() {
            let page = self.clone();
            s.connect_active_notify(move |s| {
                if page.imp().loading.get() {
                    return;
                }
                let mut c = AppConfig::load();
                c.llm_auto_apply = s.is_active();
                c.save();
            });
        }

        if let Some(s) = imp.auto_summary_switch.borrow().as_ref() {
            let page = self.clone();
            s.connect_active_notify(move |s| {
                if page.imp().loading.get() {
                    return;
                }
                let mut config = AppConfig::load();
                config.llm_auto_summary = s.is_active();
                config.save();
            });
        }

        if let Some(s) = imp.selection_switch.borrow().as_ref() {
            let page = self.clone();
            s.connect_active_notify(move |s| {
                if page.imp().loading.get() {
                    return;
                }
                let mut c = AppConfig::load();
                c.llm_selection_enabled = s.is_active();
                c.save();
            });
        }

        if let Some(e) = imp.selection_shortcut_entry.borrow().as_ref() {
            e.connect_apply(|e| {
                let text = e.text().to_string();
                if text.trim().is_empty() {
                    return;
                }
                let mut c = AppConfig::load();
                c.llm_selection_shortcut = text;
                c.save();
            });
        }
    }

    // ── Model discovery ─────────────────────────────────────────────

    fn refresh_models(&self) {
        if let Some(l) = self.imp().test_status.borrow().as_ref() {
            l.set_text(&gettext("Fetching models…"));
        }
        let cfg = self.current_llm_config();
        let rx = crate::llm::list_models_async(cfg);
        let page = self.clone();
        glib::spawn_future_local(async move {
            if let Ok(res) = rx.recv().await {
                match res {
                    Ok(models) if !models.is_empty() => {
                        let n = models.len();
                        page.set_model_list(models);
                        if let Some(l) = page.imp().test_status.borrow().as_ref() {
                            l.set_text(&format!("{} {}", n, gettext("models")));
                        }
                    }
                    Ok(_) => {
                        if let Some(l) = page.imp().test_status.borrow().as_ref() {
                            l.set_text(&gettext("No models — type one manually"));
                        }
                    }
                    Err(e) => {
                        if let Some(l) = page.imp().test_status.borrow().as_ref() {
                            l.set_text(&e);
                        }
                    }
                }
            }
        });
    }

    /// Rebuild both model dropdowns from `models`, preserving the saved/used model.
    fn set_model_list(&self, models: Vec<String>) {
        let imp = self.imp();
        let saved = AppConfig::load().llm_model;
        *imp.models.borrow_mut() = models.clone();

        imp.loading.set(true);
        if let Some(c) = imp.model_combo.borrow().as_ref() {
            let mut list = models.clone();
            if !saved.is_empty() && !list.iter().any(|m| m == &saved) {
                list.insert(0, saved.clone());
            }
            let refs: Vec<&str> = list.iter().map(|s| s.as_str()).collect();
            c.set_model(Some(&gtk::StringList::new(&refs)));
            if let Some(pos) = list.iter().position(|m| m == &saved) {
                c.set_selected(pos as u32);
            }
        }
        if let Some(c) = imp.preset_model_combo.borrow().as_ref() {
            let mut list = vec![gettext("Default")];
            list.extend(models.iter().cloned());
            let refs: Vec<&str> = list.iter().map(|s| s.as_str()).collect();
            c.set_model(Some(&gtk::StringList::new(&refs)));
            let idx = self.current_preset_index();
            let sel = AppConfig::load()
                .llm_presets
                .get(idx)
                .and_then(|p| p.model.clone());
            let pos = match sel {
                Some(m) => list.iter().position(|x| x == &m).unwrap_or(0),
                None => 0,
            };
            c.set_selected(pos as u32);
        }
        imp.loading.set(false);
    }

    // ── Preset editor ───────────────────────────────────────────────

    fn rebuild_preset_combo(&self, select: usize) {
        let imp = self.imp();
        let cfg = AppConfig::load();
        let names: Vec<String> = cfg.llm_presets.iter().map(|p| p.name.clone()).collect();
        let sel = select.min(names.len().saturating_sub(1));

        imp.loading.set(true);
        if let Some(c) = imp.preset_combo.borrow().as_ref() {
            let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
            c.set_model(Some(&gtk::StringList::new(&refs)));
            c.set_selected(sel as u32);
        }
        imp.loading.set(false);

        self.load_preset_into_editor(sel);
    }

    fn load_preset_into_editor(&self, idx: usize) {
        let imp = self.imp();
        let cfg = AppConfig::load();
        let Some(p) = cfg.llm_presets.get(idx) else {
            return;
        };

        imp.loading.set(true);
        if let Some(e) = imp.name_entry.borrow().as_ref() {
            e.set_text(&p.name);
        }
        if let Some(v) = imp.prompt_view.borrow().as_ref() {
            v.buffer().set_text(&p.prompt);
        }
        if let Some(c) = imp.translate_combo.borrow().as_ref() {
            let pos = match p.translate_to.as_deref() {
                Some(lang) => TRANSLATE_LANGS
                    .iter()
                    .position(|l| *l == lang)
                    .map(|i| i + 1)
                    .unwrap_or(0),
                None => 0,
            };
            c.set_selected(pos as u32);
        }
        if let Some(c) = imp.preset_model_combo.borrow().as_ref() {
            let mut list = vec![gettext("Default")];
            list.extend(imp.models.borrow().iter().cloned());
            let pos = match p.model.as_deref() {
                Some(m) => list.iter().position(|x| x == m).unwrap_or(0),
                None => 0,
            };
            c.set_selected(pos as u32);
        }
        let has_temp = p.temperature.is_some();
        if let Some(s) = imp.temp_override_switch.borrow().as_ref() {
            s.set_active(has_temp);
        }
        if let Some(sp) = imp.preset_temp_spin.borrow().as_ref() {
            sp.set_value(p.temperature.unwrap_or(cfg.llm_temperature) as f64);
            sp.set_sensitive(has_temp);
        }
        imp.loading.set(false);
    }

    fn add_preset(&self) {
        let mut c = AppConfig::load();
        c.llm_presets.push(LlmPreset {
            name: gettext("New preset"),
            prompt: String::new(),
            model: None,
            temperature: None,
            translate_to: None,
        });
        let new_idx = c.llm_presets.len() - 1;
        c.llm_active_preset = new_idx;
        c.save();
        self.rebuild_preset_combo(new_idx);
    }

    fn duplicate_preset(&self) {
        let idx = self.current_preset_index();
        let mut c = AppConfig::load();
        if let Some(p) = c.llm_presets.get(idx).cloned() {
            let mut copy = p;
            copy.name = format!("{} {}", copy.name, gettext("(copy)"));
            let new_idx = idx + 1;
            c.llm_presets.insert(new_idx, copy);
            c.llm_active_preset = new_idx;
            c.save();
            self.rebuild_preset_combo(new_idx);
        }
    }

    fn delete_preset(&self) {
        let idx = self.current_preset_index();
        let mut c = AppConfig::load();
        if c.llm_presets.len() <= 1 {
            return; // keep at least one preset
        }
        c.llm_presets.remove(idx);
        let new_idx = idx.min(c.llm_presets.len() - 1);
        c.llm_active_preset = new_idx;
        c.save();
        self.rebuild_preset_combo(new_idx);
    }

    // ── Test connection ─────────────────────────────────────────────

    fn test_connection(&self) {
        if let Some(l) = self.imp().test_status.borrow().as_ref() {
            l.set_text(&gettext("Testing…"));
        }
        let cfg = self.current_llm_config();
        let rx = crate::llm::probe_async(cfg);
        let page = self.clone();
        glib::spawn_future_local(async move {
            if let Ok(res) = rx.recv().await {
                if let Some(l) = page.imp().test_status.borrow().as_ref() {
                    match res {
                        Ok(msg) => l.set_text(&msg),
                        Err(e) => l.set_text(&e),
                    }
                }
            }
        });
    }
}
