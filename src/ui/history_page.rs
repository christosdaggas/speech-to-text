// Speech to Text - History Page
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Transcription history page with search and management.

use crate::i18n::gettext;
use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk4 as gtk;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::HashMap;

/// A single history entry.
///
/// Newer optional fields use `#[serde(default)]` so history files written by
/// older builds (which lacked them) still deserialize cleanly.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: String,
    pub title: String,
    pub text: String,
    pub language: String,
    pub duration_secs: u64,
    pub timestamp: String,
    pub model: String,
    /// Word count of the raw transcript (for the session-stats display).
    #[serde(default)]
    pub word_count: Option<u32>,
    /// The AI-polished version, when the user produced one ("Improve"/chips).
    #[serde(default)]
    pub polished_text: Option<String>,
    /// LLM summary of long file transcripts, when generated.
    #[serde(default)]
    pub summary: Option<String>,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct HistoryPage {
        pub list_box: RefCell<Option<gtk::ListBox>>,
        pub search_entry: RefCell<Option<gtk::SearchEntry>>,
        pub empty_status: RefCell<Option<adw::StatusPage>>,
        pub entries: RefCell<Vec<HistoryEntry>>,
        /// Map of entry id → its visible row (so titles can be updated in place).
        pub rows: RefCell<HashMap<String, adw::ActionRow>>,
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
        self.add_css_class("history-page");

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
        search_entry.set_placeholder_text(Some(gettext("Search transcriptions…").as_str()));
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
                page.confirm_clear_all();
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
        placeholder.set_description(Some(
            gettext("Your transcription history will appear here").as_str(),
        ));
        list_box.set_placeholder(Some(placeholder.upcast_ref::<gtk::Widget>()));

        scrolled.set_child(Some(&list_box));
        self.append(&scrolled);

        // Search filtering
        let page_weak = self.downgrade();
        let placeholder_ref = placeholder.clone();
        search_entry.connect_search_changed(move |entry| {
            let query = entry.text().to_string().to_lowercase();
            let Some(page) = page_weak.upgrade() else {
                return;
            };
            for history_entry in page.imp().entries.borrow().iter() {
                if let Some(row) = page.imp().rows.borrow().get(&history_entry.id) {
                    let searchable = format!(
                        "{}\n{}\n{}\n{}",
                        history_entry.title,
                        history_entry.text,
                        history_entry.polished_text.as_deref().unwrap_or_default(),
                        history_entry.summary.as_deref().unwrap_or_default(),
                    )
                    .to_lowercase();
                    row.set_visible(query.is_empty() || searchable.contains(&query));
                }
            }
            let placeholder_title = if query.is_empty() {
                gettext("No Transcriptions Yet")
            } else {
                gettext("No matching transcriptions")
            };
            placeholder_ref.set_title(&placeholder_title);
        });

        let page_weak = self.downgrade();
        list_box.connect_row_activated(move |_, activated| {
            let Some(page) = page_weak.upgrade() else {
                return;
            };
            let id = page
                .imp()
                .rows
                .borrow()
                .iter()
                .find_map(|(id, row)| (row == activated).then(|| id.clone()));
            if let Some(id) = id {
                page.show_entry(&id);
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

    /// Ask for confirmation before clearing all history (it cannot be undone).
    pub fn confirm_clear_all(&self) {
        let dialog = adw::AlertDialog::new(
            Some(gettext("Clear all history?").as_str()),
            Some(
                gettext(
                    "This permanently deletes every saved transcription. This cannot be undone.",
                )
                .as_str(),
            ),
        );
        dialog.add_response("cancel", gettext("Cancel").as_str());
        dialog.add_response("clear", gettext("Clear All").as_str());
        dialog.set_response_appearance("clear", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");

        let page = self.clone();
        dialog.choose(self, gtk::gio::Cancellable::NONE, move |resp| {
            if resp.as_str() == "clear" {
                page.clear_all();
            }
        });
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
        let mut subtitle = format!(
            "{} • {} • {}",
            entry.timestamp,
            entry.language,
            format_duration(entry.duration_secs)
        );
        // Word count (and words-per-minute when the clip is long enough).
        if let Some(words) = entry.word_count {
            subtitle.push_str(&format!(" • {} {}", words, gettext("words")));
            if let Some(wpm) =
                crate::ui::result_state::wpm(words as usize, entry.duration_secs as f32)
            {
                subtitle.push_str(&format!(" · {} wpm", wpm));
            }
        }
        let row = adw::ActionRow::builder()
            .title(&entry.title)
            .subtitle(&subtitle)
            .activatable(true)
            .build();
        row.set_use_markup(false);

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
        self.imp().rows.borrow_mut().insert(entry.id.clone(), row);
    }

    fn show_entry(&self, id: &str) {
        let Some(entry) = self
            .imp()
            .entries
            .borrow()
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
        else {
            return;
        };

        let detail = gtk::Window::builder()
            .title(&entry.title)
            .default_width(640)
            .default_height(480)
            .modal(true)
            .build();
        if let Some(parent) = self.root().and_downcast::<gtk::Window>() {
            detail.set_transient_for(Some(&parent));
        }
        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_margin_top(16);
        scrolled.set_margin_bottom(16);
        scrolled.set_margin_start(16);
        scrolled.set_margin_end(16);
        let text = gtk::TextView::new();
        text.set_editable(false);
        text.set_cursor_visible(false);
        text.set_wrap_mode(gtk::WrapMode::WordChar);
        text.buffer().set_text(&entry.text);
        scrolled.set_child(Some(&text));
        detail.set_child(Some(&scrolled));
        detail.present();
    }

    /// Update the title of an existing entry (in memory, on disk, and in the UI
    /// row if visible). Used by the LLM auto-title feature.
    pub fn update_entry_title(&self, id: &str, title: &str) {
        let imp = self.imp();
        let mut changed = false;
        if let Some(e) = imp.entries.borrow_mut().iter_mut().find(|e| e.id == id) {
            e.title = title.to_string();
            changed = true;
        }
        if let Some(row) = imp.rows.borrow().get(id) {
            row.set_title(title);
        }
        if changed {
            self.save_history();
        }
    }

    /// Persist history entries to disk as JSON.
    fn save_history(&self) {
        let _guard = HISTORY_FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let entries = self.imp().entries.borrow();
        let path = crate::config::AppConfig::history_dir().join("history.json");
        match serde_json::to_string_pretty(&*entries) {
            // Transcripts are personal data: write privately (0600 in a 0700 dir)
            // and atomically so other local users can't read them and a crash
            // can't corrupt the file.
            Ok(json) => {
                if let Err(e) = crate::fsio::write_private(&path, json.as_bytes()) {
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
                let backup =
                    path.with_extension(format!("json.corrupt-{}", chrono::Utc::now().timestamp()));
                if let Err(rename_error) = std::fs::rename(&path, &backup) {
                    tracing::warn!("Failed to preserve corrupt history: {}", rename_error);
                } else {
                    tracing::warn!("Preserved corrupt history at {:?}", backup);
                }
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

/// Serializes all history-file read-modify-write sequences so a background
/// append (global dictation while the window is closed) and an auto-title update
/// can't clobber each other's changes. Atomic writes prevent *corruption*; this
/// prevents a *lost update*.
static HISTORY_FILE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Append a single entry to the on-disk history file.
///
/// Use this ONLY when no [`HistoryPage`] is loaded in memory (e.g. a global
/// dictation completed while the main window is closed). When the main window
/// is open, route through [`HistoryPage::add_entry`] instead, which keeps the
/// in-memory list and disk in sync — otherwise a later `save_history()` would
/// overwrite a directly-appended entry.
pub fn append_entry_to_disk(entry: &HistoryEntry) {
    let _guard = HISTORY_FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = crate::config::AppConfig::history_dir().join("history.json");
    let mut entries: Vec<HistoryEntry> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_default();
    entries.push(entry.clone());
    match serde_json::to_string_pretty(&entries) {
        Ok(json) => {
            if let Err(e) = crate::fsio::write_private(&path, json.as_bytes()) {
                tracing::warn!("Failed to write history: {}", e);
            }
        }
        Err(e) => tracing::warn!("Failed to serialize history: {}", e),
    }
}

/// Update the title of an entry directly on disk (used by LLM auto-title when
/// the main window is closed). No-op if the entry id isn't found.
pub fn update_title_on_disk(id: &str, title: &str) {
    let _guard = HISTORY_FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = crate::config::AppConfig::history_dir().join("history.json");
    let mut entries: Vec<HistoryEntry> = match std::fs::read_to_string(&path)
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
    {
        Some(e) => e,
        None => return,
    };
    let mut changed = false;
    for entry in entries.iter_mut() {
        if entry.id == id {
            entry.title = title.to_string();
            changed = true;
            break;
        }
    }
    if !changed {
        return;
    }
    if let Ok(json) = serde_json::to_string_pretty(&entries) {
        if let Err(e) = crate::fsio::write_private(&path, json.as_bytes()) {
            tracing::warn!("Failed to write history: {}", e);
        }
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
