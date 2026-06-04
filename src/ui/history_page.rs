// Speech to Text - History Page
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Transcription history page with search and management.

use gtk4::prelude::*;
use crate::i18n::gettext;
use adw::prelude::*;
use gtk4::glib;
use gtk4 as gtk;
use libadwaita as adw;
use adw::subclass::prelude::*;
use std::cell::RefCell;
use serde::{Deserialize, Serialize};

/// A single history entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: String,
    pub title: String,
    pub text: String,
    pub language: String,
    pub duration_secs: u64,
    pub timestamp: String,
    pub model: String,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct HistoryPage {
        pub list_box: RefCell<Option<gtk::ListBox>>,
        pub search_entry: RefCell<Option<gtk::SearchEntry>>,
        pub empty_status: RefCell<Option<adw::StatusPage>>,
        pub entries: RefCell<Vec<HistoryEntry>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for HistoryPage {
        const NAME: &'static str = "SttHistoryPage";
        type Type = super::HistoryPage;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for HistoryPage {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_ui();
        }
    }

    impl WidgetImpl for HistoryPage {}
    impl BoxImpl for HistoryPage {}
}

glib::wrapper! {
    pub struct HistoryPage(ObjectSubclass<imp::HistoryPage>)
        @extends gtk::Widget, gtk::Box;
}

impl HistoryPage {
    pub fn new() -> Self {
        let page: Self = glib::Object::builder()
            .property("orientation", gtk::Orientation::Vertical)
            .property("spacing", 0)
            .build();
        page.load_history();
        page
    }

    fn setup_ui(&self) {
        let imp = self.imp();

        // Header area with search
        let header_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        header_box.set_margin_start(16);
        header_box.set_margin_end(16);
        header_box.set_margin_top(16);
        header_box.set_margin_bottom(8);

        let title = gtk::Label::new(Some(gettext("Transcription History").as_str()));
        title.add_css_class("title-3");
        title.set_hexpand(true);
        title.set_xalign(0.0);
        header_box.append(&title);

        let search_entry = gtk::SearchEntry::new();
        search_entry.set_placeholder_text(Some("Search transcriptions…"));
        search_entry.set_hexpand(false);
        search_entry.set_width_chars(25);
        header_box.append(&search_entry);

        // Clear all button
        let clear_all_btn = gtk::Button::from_icon_name("edit-clear-all-symbolic");
        clear_all_btn.set_tooltip_text(Some(gettext("Clear all history").as_str()));
        clear_all_btn.add_css_class("flat");
        let page_weak = self.downgrade();
        clear_all_btn.connect_clicked(move |_| {
            if let Some(page) = page_weak.upgrade() {
                page.clear_all();
            }
        });
        header_box.append(&clear_all_btn);

        self.append(&header_box);

        // Scrolled list
        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_vexpand(true);

        let list_box = gtk::ListBox::new();
        list_box.set_selection_mode(gtk::SelectionMode::None);
        list_box.add_css_class("boxed-list");
        list_box.set_margin_start(16);
        list_box.set_margin_end(16);
        list_box.set_margin_bottom(16);

        // Empty state placeholder
        let placeholder = adw::StatusPage::new();
        placeholder.set_icon_name(Some("document-open-recent-symbolic"));
        placeholder.set_title(gettext("No Transcriptions Yet").as_str());
        placeholder.set_description(Some(gettext("Your transcription history will appear here").as_str()));
        list_box.set_placeholder(Some(placeholder.upcast_ref::<gtk::Widget>()));

        scrolled.set_child(Some(&list_box));
        self.append(&scrolled);

        // Search filtering
        let list_ref = list_box.clone();
        search_entry.connect_search_changed(move |entry| {
            let query = entry.text().to_string().to_lowercase();
            let mut idx = 0;
            while let Some(row) = list_ref.row_at_index(idx) {
                if query.is_empty() {
                    row.set_visible(true);
                } else {
                    // Filter based on the row title
                    let visible = row
                        .child()
                        .and_then(|w| w.downcast::<adw::ActionRow>().ok())
                        .map(|r| {
                            r.title().to_string().to_lowercase().contains(&query)
                                || r.subtitle().map(|s| s.to_string().to_lowercase().contains(&query)).unwrap_or(false)
                        })
                        .unwrap_or(true);
                    row.set_visible(visible);
                }
                idx += 1;
            }
        });

        *imp.list_box.borrow_mut() = Some(list_box);
        *imp.search_entry.borrow_mut() = Some(search_entry);
    }

    /// Add a history entry to the list.
    pub fn add_entry(&self, entry: HistoryEntry) {
        let imp = self.imp();

        if let Some(list_box) = imp.list_box.borrow().as_ref() {
            self.add_entry_row(list_box, &entry);
        }

        imp.entries.borrow_mut().push(entry);
        self.save_history();
    }

    /// Clear all history entries.
    pub fn clear_all(&self) {
        let imp = self.imp();
        if let Some(list_box) = imp.list_box.borrow().as_ref() {
            while let Some(row) = list_box.row_at_index(0) {
                list_box.remove(&row);
            }
        }
        imp.entries.borrow_mut().clear();
        self.save_history();
    }

    /// Add a UI row for an entry (used by both add_entry and load_history).
    fn add_entry_row(&self, list_box: &gtk::ListBox, entry: &HistoryEntry) {
        let row = adw::ActionRow::builder()
            .title(&entry.title)
            .subtitle(&format!(
                "{} • {} • {}",
                entry.timestamp, entry.language, format_duration(entry.duration_secs)
            ))
            .activatable(true)
            .build();

        // Model badge
        let model_badge = gtk::Label::new(Some(&entry.model));
        model_badge.add_css_class("caption");
        model_badge.add_css_class("dim-label");
        row.add_suffix(&model_badge);

        // Copy button
        let copy_btn = gtk::Button::from_icon_name("edit-copy-symbolic");
        copy_btn.set_tooltip_text(Some(gettext("Copy to clipboard").as_str()));
        copy_btn.set_valign(gtk::Align::Center);
        copy_btn.add_css_class("flat");
        let text = entry.text.clone();
        copy_btn.connect_clicked(move |btn| {
            if let Some(display) = btn.display().into() {
                let clipboard = gtk::gdk::Display::clipboard(&display);
                clipboard.set_text(&text);
            }
        });
        row.add_suffix(&copy_btn);

        // Delete button
        let delete_btn = gtk::Button::from_icon_name("user-trash-symbolic");
        delete_btn.set_tooltip_text(Some(gettext("Delete").as_str()));
        delete_btn.set_valign(gtk::Align::Center);
        delete_btn.add_css_class("flat");

        let list_box_ref = list_box.clone();
        let entry_id = entry.id.clone();
        let page_weak = self.downgrade();
        delete_btn.connect_clicked(move |btn| {
            if let Some(row) = btn.ancestor(gtk::ListBoxRow::static_type()) {
                list_box_ref.remove(&row);
            }
            if let Some(page) = page_weak.upgrade() {
                page.imp().entries.borrow_mut().retain(|e| e.id != entry_id);
                page.save_history();
            }
        });

        row.add_suffix(&delete_btn);

        list_box.prepend(&row);
    }

    /// Persist history entries to disk as JSON.
    fn save_history(&self) {
        let entries = self.imp().entries.borrow();
        let history_dir = crate::config::AppConfig::history_dir();
        if let Err(e) = std::fs::create_dir_all(&history_dir) {
            tracing::warn!("Failed to create history dir: {}", e);
            return;
        }
        let path = history_dir.join("history.json");
        match serde_json::to_string_pretty(&*entries) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, &json) {
                    tracing::warn!("Failed to write history: {}", e);
                }
            }
            Err(e) => tracing::warn!("Failed to serialize history: {}", e),
        }
    }

    /// Load history from disk.
    fn load_history(&self) {
        let path = crate::config::AppConfig::history_dir().join("history.json");
        if !path.exists() {
            return;
        }
        let contents = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Failed to read history: {}", e);
                return;
            }
        };
        let entries: Vec<HistoryEntry> = match serde_json::from_str(&contents) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Failed to parse history: {}", e);
                return;
            }
        };

        let imp = self.imp();
        if let Some(list_box) = imp.list_box.borrow().as_ref() {
            for entry in &entries {
                self.add_entry_row(list_box, entry);
            }
        }
        *imp.entries.borrow_mut() = entries;
    }
}

/// Append a single entry to the on-disk history file.
///
/// Use this ONLY when no [`HistoryPage`] is loaded in memory (e.g. a global
/// dictation completed while the main window is closed). When the main window
/// is open, route through [`HistoryPage::add_entry`] instead, which keeps the
/// in-memory list and disk in sync — otherwise a later `save_history()` would
/// overwrite a directly-appended entry.
pub fn append_entry_to_disk(entry: &HistoryEntry) {
    let history_dir = crate::config::AppConfig::history_dir();
    if let Err(e) = std::fs::create_dir_all(&history_dir) {
        tracing::warn!("Failed to create history dir: {}", e);
        return;
    }
    let path = history_dir.join("history.json");
    let mut entries: Vec<HistoryEntry> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_default();
    entries.push(entry.clone());
    match serde_json::to_string_pretty(&entries) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, &json) {
                tracing::warn!("Failed to write history: {}", e);
            }
        }
        Err(e) => tracing::warn!("Failed to serialize history: {}", e),
    }
}

fn format_duration(secs: u64) -> String {
    let mins = secs / 60;
    let s = secs % 60;
    if mins > 0 {
        format!("{}m {}s", mins, s)
    } else {
        format!("{}s", s)
    }
}
