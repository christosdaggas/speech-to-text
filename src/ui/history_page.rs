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
use std::sync::atomic::{AtomicU64, Ordering};

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
        /// Precomputed lowercase "searchable text" per entry id, so the search
        /// box doesn't re-allocate and case-fold the whole transcript corpus
        /// on every query change.
        pub search_cache: RefCell<HashMap<String, String>>,
        /// Bumped when the list is cleared so an in-flight chunked populate
        /// from a stale load stops adding rows.
        pub populate_generation: std::cell::Cell<u64>,
        /// Whether the initial disk load has merged into `entries`. Saves
        /// before that point are deferred (see `save_history`).
        pub loaded: std::cell::Cell<bool>,
        /// A save was requested while the initial load was still in flight.
        pub save_pending: std::cell::Cell<bool>,
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
        page.load_history_async();
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

        // Search filtering over the precomputed lowercase cache — re-building
        // and case-folding the whole transcript corpus per query change made
        // typing in the search box stutter once the history grew.
        let page_weak = self.downgrade();
        let placeholder_ref = placeholder.clone();
        search_entry.connect_search_changed(move |entry| {
            let query = entry.text().to_string().to_lowercase();
            let Some(page) = page_weak.upgrade() else {
                return;
            };
            let rows = page.imp().rows.borrow();
            if query.is_empty() {
                for row in rows.values() {
                    row.set_visible(true);
                }
            } else {
                let cache = page.imp().search_cache.borrow();
                for (id, row) in rows.iter() {
                    let matches = cache.get(id).map(|s| s.contains(&query)).unwrap_or(true);
                    row.set_visible(matches);
                }
            }
            drop(rows);
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
        // Stop any in-flight chunked populate from re-adding rows.
        imp.populate_generation
            .set(imp.populate_generation.get().wrapping_add(1));
        if let Some(list_box) = imp.list_box.borrow().as_ref() {
            while let Some(row) = list_box.row_at_index(0) {
                list_box.remove(&row);
            }
        }
        imp.entries.borrow_mut().clear();
        imp.rows.borrow_mut().clear();
        imp.search_cache.borrow_mut().clear();
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
                page.imp().rows.borrow_mut().remove(&entry_id);
                page.imp().search_cache.borrow_mut().remove(&entry_id);
                page.save_history();
            }
        });

        row.add_suffix(&delete_btn);

        list_box.prepend(&row);
        self.imp().rows.borrow_mut().insert(entry.id.clone(), row);
        self.imp()
            .search_cache
            .borrow_mut()
            .insert(entry.id.clone(), searchable_blob(entry));
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
            imp.search_cache
                .borrow_mut()
                .insert(id.to_string(), searchable_blob(e));
        }
        if let Some(row) = imp.rows.borrow().get(id) {
            row.set_title(title);
        }
        if changed {
            self.save_history();
        }
    }

    /// Persist history entries to disk as JSON. The in-memory list is
    /// snapshotted on the GTK thread; serialization and the fsync'd atomic
    /// write happen on a worker so a growing history can never stall the UI
    /// right between transcription completion and result delivery.
    fn save_history(&self) {
        let imp = self.imp();
        // Until the initial disk load has merged, the in-memory list holds
        // only post-construction entries — writing it now would TRUNCATE the
        // user's entire prior history on disk. Defer; finish_load releases it.
        if !imp.loaded.get() {
            imp.save_pending.set(true);
            return;
        }
        save_snapshot_async(imp.entries.borrow().clone());
    }

    /// Load history from disk on a worker thread, then populate the list in
    /// small idle-callback chunks — parsing and building thousands of rows
    /// synchronously in `new()` delayed the first window paint by the size of
    /// the user's lifetime history.
    fn load_history_async(&self) {
        let (tx, rx) = async_channel::bounded::<Vec<HistoryEntry>>(1);
        // Capture the generation BEFORE spawning the reader: a "Clear All"
        // confirmed while the file is still being read must cancel the merge,
        // or every "permanently deleted" entry would be resurrected.
        let generation = self.imp().populate_generation.get();
        std::thread::spawn(move || {
            let _ = tx.send_blocking(read_history_from_disk());
        });
        let page_weak = self.downgrade();
        glib::spawn_future_local(async move {
            let entries = rx.recv().await.unwrap_or_default();
            let Some(page) = page_weak.upgrade() else {
                return;
            };
            page.finish_load(entries, generation);
        });
    }

    /// Merge the loaded entries into memory (unless a clear cancelled the
    /// load), start the chunked row populate, and release any save deferred
    /// while the load was in flight.
    fn finish_load(&self, entries: Vec<HistoryEntry>, generation: u64) {
        let imp = self.imp();
        let cancelled = imp.populate_generation.get() != generation;
        if !cancelled && !entries.is_empty() {
            // Memory first, so search/save/add_entry see the full list at
            // once: loaded history, then anything added since construction.
            {
                let mut current = imp.entries.borrow_mut();
                let newer = std::mem::take(&mut *current);
                *current = entries;
                current.extend(newer);
            }
            self.populate_in_chunks(generation);
        }
        imp.loaded.set(true);
        if imp.save_pending.take() {
            self.save_history();
        }
    }

    /// Insert rows for the current entries in batches per idle tick, keeping
    /// the main loop responsive. New dictations arriving mid-populate go
    /// through [`Self::add_entry`] (appended after the snapshot), so the
    /// snapshot walk can't duplicate them; `populate_generation` aborts the
    /// walk when the list is cleared underneath it.
    fn populate_in_chunks(&self, generation: u64) {
        const CHUNK: usize = 50;
        let imp = self.imp();
        let snapshot: Vec<HistoryEntry> = imp.entries.borrow().clone();
        let mut next = 0usize;
        let page_weak = self.downgrade();
        glib::idle_add_local(move || {
            let Some(page) = page_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if page.imp().populate_generation.get() != generation {
                return glib::ControlFlow::Break;
            }
            let list_box = match page.imp().list_box.borrow().clone() {
                Some(l) => l,
                None => return glib::ControlFlow::Break,
            };
            let end = (next + CHUNK).min(snapshot.len());
            for entry in &snapshot[next..end] {
                // add_entry may have rendered this id already (dictation that
                // completed while chunks were still loading).
                if !page.imp().rows.borrow().contains_key(&entry.id) {
                    page.add_entry_row(&list_box, entry);
                }
            }
            next = end;
            if next >= snapshot.len() {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }
}

/// Lowercase haystack used by the search box.
fn searchable_blob(entry: &HistoryEntry) -> String {
    format!(
        "{}\n{}\n{}\n{}",
        entry.title,
        entry.text,
        entry.polished_text.as_deref().unwrap_or_default(),
        entry.summary.as_deref().unwrap_or_default(),
    )
    .to_lowercase()
}

/// Read and parse the history file (preserving a corrupt file aside instead of
/// silently losing it). Runs on worker threads.
fn read_history_from_disk() -> Vec<HistoryEntry> {
    // Order the initial read after any in-flight background append (a headless
    // dictation finishing right as the first window opens): reading the file
    // mid read-modify-write would drop the appended entry from memory, and the
    // next page save would then erase it from disk too.
    let _guard = HISTORY_FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = crate::config::AppConfig::history_dir().join("history.json");
    if !path.exists() {
        return Vec::new();
    }
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Failed to read history: {}", e);
            return Vec::new();
        }
    };
    match serde_json::from_str(&contents) {
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
            Vec::new()
        }
    }
}

/// Monotonic sequencing for page-snapshot saves: two background writes can
/// finish out of order, and an older snapshot must never overwrite a newer one.
static SAVE_SEQ: AtomicU64 = AtomicU64::new(0);
static SAVED_SEQ: AtomicU64 = AtomicU64::new(0);

fn save_snapshot_async(entries: Vec<HistoryEntry>) {
    let seq = SAVE_SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    history_write_started();
    std::thread::spawn(move || {
        {
            let _guard = HISTORY_FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            if SAVED_SEQ.load(Ordering::SeqCst) < seq {
                let path = crate::config::AppConfig::history_dir().join("history.json");
                match serde_json::to_string_pretty(&entries) {
                    // Transcripts are personal data: write privately (0600 in a
                    // 0700 dir) and atomically so other local users can't read
                    // them and a crash can't corrupt the file.
                    Ok(json) => {
                        if let Err(e) = crate::fsio::write_private(&path, json.as_bytes()) {
                            tracing::warn!("Failed to write history: {}", e);
                        } else {
                            SAVED_SEQ.store(seq, Ordering::SeqCst);
                        }
                    }
                    Err(e) => tracing::warn!("Failed to serialize history: {}", e),
                }
            }
        }
        history_write_finished();
    });
}

/// Count of in-flight background history writes, so shutdown can wait for
/// them: detached threads die with the process, and a dictation saved right
/// before quitting must still reach the disk.
static PENDING_HISTORY_WRITES: AtomicU64 = AtomicU64::new(0);

fn history_write_started() {
    PENDING_HISTORY_WRITES.fetch_add(1, Ordering::SeqCst);
}

fn history_write_finished() {
    PENDING_HISTORY_WRITES.fetch_sub(1, Ordering::SeqCst);
}

/// Block (bounded by `timeout`) until all background history writes finished.
/// Called from application shutdown on the main thread — the UI is going away,
/// so a short blocking wait is acceptable.
pub fn wait_for_pending_writes(timeout: std::time::Duration) {
    let deadline = std::time::Instant::now() + timeout;
    while PENDING_HISTORY_WRITES.load(Ordering::SeqCst) > 0 {
        if std::time::Instant::now() >= deadline {
            tracing::warn!("Timed out waiting for background history writes");
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
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
    let entry = entry.clone();
    history_write_started();
    std::thread::spawn(move || {
        {
            let _guard = HISTORY_FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let path = crate::config::AppConfig::history_dir().join("history.json");
            let mut entries: Vec<HistoryEntry> = std::fs::read_to_string(&path)
                .ok()
                .and_then(|c| serde_json::from_str(&c).ok())
                .unwrap_or_default();
            entries.push(entry);
            match serde_json::to_string_pretty(&entries) {
                Ok(json) => {
                    if let Err(e) = crate::fsio::write_private(&path, json.as_bytes()) {
                        tracing::warn!("Failed to write history: {}", e);
                    }
                }
                Err(e) => tracing::warn!("Failed to serialize history: {}", e),
            }
        }
        history_write_finished();
    });
}

/// Update the title of an entry directly on disk (used by LLM auto-title when
/// the main window is closed). No-op if the entry id isn't found.
pub fn update_title_on_disk(id: &str, title: &str) {
    let id = id.to_string();
    let title = title.to_string();
    history_write_started();
    std::thread::spawn(move || {
        {
            let _guard = HISTORY_FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let path = crate::config::AppConfig::history_dir().join("history.json");
            let entries: Option<Vec<HistoryEntry>> = std::fs::read_to_string(&path)
                .ok()
                .and_then(|c| serde_json::from_str(&c).ok());
            if let Some(mut entries) = entries {
                let mut changed = false;
                for entry in entries.iter_mut() {
                    if entry.id == id {
                        entry.title = title.clone();
                        changed = true;
                        break;
                    }
                }
                if changed {
                    if let Ok(json) = serde_json::to_string_pretty(&entries) {
                        if let Err(e) = crate::fsio::write_private(&path, json.as_bytes()) {
                            tracing::warn!("Failed to write history: {}", e);
                        }
                    }
                }
            }
        }
        history_write_finished();
    });
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
