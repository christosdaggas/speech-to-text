// Speech to Text - Personal Dictionary Settings Page
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Personal dictionary: vocabulary terms (fed to Whisper's initial prompt) and
//! "heard → correct" replacement rules (applied to the transcript). Local only —
//! nothing here is sent anywhere.

use gtk4::prelude::*;
use crate::i18n::gettext;
use adw::prelude::*;
use gtk4::glib;
use gtk4 as gtk;
use libadwaita as adw;
use adw::subclass::prelude::*;
use std::cell::{Cell, RefCell};

use crate::config::{AppConfig, DictReplacement};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct DictionaryPage {
        pub enable_switch: RefCell<Option<adw::SwitchRow>>,
        pub terms_view: RefCell<Option<gtk::TextView>>,
        pub repl_group: RefCell<Option<adw::PreferencesGroup>>,
        pub repl_rows: RefCell<Vec<adw::ExpanderRow>>,
        pub loading: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DictionaryPage {
        const NAME: &'static str = "SttDictionaryPage";
        type Type = super::DictionaryPage;
        type ParentType = adw::PreferencesPage;
    }

    impl ObjectImpl for DictionaryPage {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_ui();
        }
    }

    impl WidgetImpl for DictionaryPage {}
    impl adw::subclass::prelude::PreferencesPageImpl for DictionaryPage {}
}

glib::wrapper! {
    pub struct DictionaryPage(ObjectSubclass<imp::DictionaryPage>)
        @extends gtk::Widget, adw::PreferencesPage;
}

impl Default for DictionaryPage {
    fn default() -> Self {
        Self::new()
    }
}

impl DictionaryPage {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("title", "Dictionary")
            .property("icon-name", "accessories-dictionary-symbolic")
            .build()
    }

    fn setup_ui(&self) {
        let imp = self.imp();

        // === Master enable ===
        let enable_group = adw::PreferencesGroup::new();
        enable_group.set_title(gettext("Personal Dictionary").as_str());
        enable_group.set_description(Some(&gettext(
            "Improve recognition of names, jargon and acronyms, and fix consistent spellings. Everything here stays on your device.",
        )));
        let enable_switch = adw::SwitchRow::builder()
            .title(gettext("Enable personal dictionary").as_str())
            .subtitle(gettext("Use the terms and replacements below").as_str())
            .active(true)
            .build();
        enable_group.add(&enable_switch);
        self.add(&enable_group);

        // === Vocabulary terms ===
        let terms_group = adw::PreferencesGroup::new();
        terms_group.set_title(gettext("Vocabulary").as_str());
        terms_group.set_description(Some(&gettext(
            "Words to bias recognition toward — names, product names, technical terms. One term per line.",
        )));
        let terms_view = gtk::TextView::new();
        terms_view.set_wrap_mode(gtk::WrapMode::Word);
        terms_view.set_top_margin(8);
        terms_view.set_bottom_margin(8);
        terms_view.set_left_margin(8);
        terms_view.set_right_margin(8);
        let scroller = gtk::ScrolledWindow::builder()
            .min_content_height(96)
            .max_content_height(200)
            .vexpand(false)
            .child(&terms_view)
            .build();
        let frame = gtk::Frame::new(None);
        frame.set_child(Some(&scroller));
        terms_group.add(&frame);
        self.add(&terms_group);

        // === Replacements ===
        let repl_group = adw::PreferencesGroup::new();
        repl_group.set_title(gettext("Replacements").as_str());
        repl_group.set_description(Some(&gettext(
            "Replace what was heard with the correct text (e.g. a name spelled consistently).",
        )));
        let add_btn = gtk::Button::from_icon_name("list-add-symbolic");
        add_btn.set_tooltip_text(Some(&gettext("Add replacement")));
        add_btn.add_css_class("flat");
        add_btn.set_valign(gtk::Align::Center);
        repl_group.set_header_suffix(Some(&add_btn));
        self.add(&repl_group);

        // Store references.
        *imp.enable_switch.borrow_mut() = Some(enable_switch.clone());
        *imp.terms_view.borrow_mut() = Some(terms_view.clone());
        *imp.repl_group.borrow_mut() = Some(repl_group);

        // Wire handlers.
        let page = self.clone();
        enable_switch.connect_active_notify(move |s| {
            if page.imp().loading.get() {
                return;
            }
            let mut c = AppConfig::load();
            c.dictionary_enabled = s.is_active();
            c.save();
        });

        let page = self.clone();
        terms_view.buffer().connect_changed(move |buf| {
            if page.imp().loading.get() {
                return;
            }
            let text = buf.text(&buf.start_iter(), &buf.end_iter(), false).to_string();
            let terms: Vec<String> = text
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect();
            let mut c = AppConfig::load();
            c.dictionary_terms = terms;
            c.save();
        });

        let page = self.clone();
        add_btn.connect_clicked(move |_| {
            let mut c = AppConfig::load();
            c.dictionary_replacements.push(DictReplacement::default());
            c.save();
            page.rebuild_replacements();
        });

        self.load_from_config();

        // Re-sync when the page is shown (settings may change elsewhere).
        let page = self.clone();
        self.connect_map(move |_| page.load_from_config());
    }

    fn load_from_config(&self) {
        let imp = self.imp();
        let cfg = AppConfig::load();
        imp.loading.set(true);
        if let Some(s) = imp.enable_switch.borrow().as_ref() {
            s.set_active(cfg.dictionary_enabled);
        }
        if let Some(v) = imp.terms_view.borrow().as_ref() {
            v.buffer().set_text(&cfg.dictionary_terms.join("\n"));
        }
        imp.loading.set(false);
        self.rebuild_replacements();
    }

    /// Mutate replacement rule `idx` and persist.
    fn update_rule(&self, idx: usize, f: impl FnOnce(&mut DictReplacement)) {
        if self.imp().loading.get() {
            return;
        }
        let mut c = AppConfig::load();
        if let Some(r) = c.dictionary_replacements.get_mut(idx) {
            f(r);
            c.save();
        }
    }

    /// Rebuild the replacement rows from config.
    fn rebuild_replacements(&self) {
        let imp = self.imp();
        let Some(group) = imp.repl_group.borrow().clone() else {
            return;
        };
        // Remove previously-added rows.
        for row in imp.repl_rows.borrow_mut().drain(..) {
            group.remove(&row);
        }

        let cfg = AppConfig::load();
        let mut rows = Vec::new();
        for (idx, rule) in cfg.dictionary_replacements.iter().enumerate() {
            let title = if rule.from.trim().is_empty() && rule.to.trim().is_empty() {
                gettext("New replacement")
            } else {
                format!("{} → {}", rule.from, rule.to)
            };
            let exp = adw::ExpanderRow::builder().title(&title).build();

            let from_row = adw::EntryRow::builder()
                .title(gettext("Heard").as_str())
                .text(&rule.from)
                .show_apply_button(true)
                .build();
            let to_row = adw::EntryRow::builder()
                .title(gettext("Replace with").as_str())
                .text(&rule.to)
                .show_apply_button(true)
                .build();
            let whole_word = adw::SwitchRow::builder()
                .title(gettext("Whole word only").as_str())
                .active(rule.whole_word)
                .build();
            let case_sensitive = adw::SwitchRow::builder()
                .title(gettext("Case sensitive").as_str())
                .active(rule.case_sensitive)
                .build();

            let remove_row = adw::ActionRow::builder()
                .title(gettext("Remove this replacement").as_str())
                .build();
            let remove_btn = gtk::Button::from_icon_name("user-trash-symbolic");
            remove_btn.add_css_class("flat");
            remove_btn.add_css_class("destructive-action");
            remove_btn.set_valign(gtk::Align::Center);
            remove_row.add_suffix(&remove_btn);

            exp.add_row(&from_row);
            exp.add_row(&to_row);
            exp.add_row(&whole_word);
            exp.add_row(&case_sensitive);
            exp.add_row(&remove_row);

            // Wire edits.
            let page = self.clone();
            let exp_w = exp.downgrade();
            from_row.connect_apply(move |e| {
                let v = e.text().to_string();
                page.update_rule(idx, |r| r.from = v.clone());
                if let Some(exp) = exp_w.upgrade() {
                    page.refresh_row_title(&exp, idx);
                }
            });
            let page = self.clone();
            let exp_w = exp.downgrade();
            to_row.connect_apply(move |e| {
                let v = e.text().to_string();
                page.update_rule(idx, |r| r.to = v.clone());
                if let Some(exp) = exp_w.upgrade() {
                    page.refresh_row_title(&exp, idx);
                }
            });
            let page = self.clone();
            whole_word.connect_active_notify(move |s| {
                let v = s.is_active();
                page.update_rule(idx, |r| r.whole_word = v);
            });
            let page = self.clone();
            case_sensitive.connect_active_notify(move |s| {
                let v = s.is_active();
                page.update_rule(idx, |r| r.case_sensitive = v);
            });
            let page = self.clone();
            remove_btn.connect_clicked(move |_| {
                if page.imp().loading.get() {
                    return;
                }
                let mut c = AppConfig::load();
                if idx < c.dictionary_replacements.len() {
                    c.dictionary_replacements.remove(idx);
                    c.save();
                }
                page.rebuild_replacements();
            });

            group.add(&exp);
            rows.push(exp);
        }
        *imp.repl_rows.borrow_mut() = rows;
    }

    /// Update an expander row's title to reflect the current rule.
    fn refresh_row_title(&self, exp: &adw::ExpanderRow, idx: usize) {
        let cfg = AppConfig::load();
        if let Some(rule) = cfg.dictionary_replacements.get(idx) {
            let title = if rule.from.trim().is_empty() && rule.to.trim().is_empty() {
                gettext("New replacement")
            } else {
                format!("{} → {}", rule.from, rule.to)
            };
            exp.set_title(&title);
        }
    }
}
