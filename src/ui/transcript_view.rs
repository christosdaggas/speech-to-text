// Speech to Text - Transcript View
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Main transcription panel with message bubbles, live text, and confidence indicator.

use gtk4::prelude::*;
use gtk4::glib;
use gtk4 as gtk;
use libadwaita as adw;
use adw::subclass::prelude::*;
use std::cell::{Cell, RefCell};

use crate::i18n::gettext;

/// Number of cells in the transcribing decode-sweep (mirrors the mini panel).
const N_SEGS: usize = 24;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct TranscriptView {
        pub bubble_list: RefCell<Option<gtk::Box>>,
        pub scrolled_window: RefCell<Option<gtk::ScrolledWindow>>,
        pub placeholder: RefCell<Option<gtk::Label>>,
        pub messages: RefCell<Vec<String>>,
        // Multi-message model: one selectable bubble per dictation. Indices are
        // stable (we only append or clear-all).
        pub message_bubbles: RefCell<Vec<gtk::Box>>,
        pub message_labels: RefCell<Vec<gtk::Label>>,
        pub selected_idx: Cell<isize>,
        pub message_selected_cb: RefCell<Option<Box<dyn Fn(usize)>>>,
        /// Transient "live preview" bubble shown while recording (not a message).
        pub live_preview: RefCell<Option<gtk::Box>>,
        pub live_preview_label: RefCell<Option<gtk::Label>>,
        pub confidence_bar: RefCell<Option<gtk::LevelBar>>,
        pub confidence_label: RefCell<Option<gtk::Label>>,
        pub timer_label: RefCell<Option<gtk::Label>>,
        pub stats_label: RefCell<Option<gtk::Label>>,
        pub is_recording: Cell<bool>,
        // Transform actions (dropdown) + raw/polished variant selector (under the transcript)
        pub controls_row: RefCell<Option<gtk::Box>>,
        pub actions_btn: RefCell<Option<gtk::MenuButton>>,
        pub actions_list: RefCell<Option<gtk::Box>>,
        pub chip_buttons: RefCell<Vec<gtk::Button>>,
        pub chip_callback: RefCell<Option<Box<dyn Fn(usize)>>>,
        pub variant_dropdown: RefCell<Option<gtk::DropDown>>,
        pub variant_callback: RefCell<Option<Box<dyn Fn(usize)>>>,
        pub variant_syncing: Cell<bool>,
        pub voice_edit_btn: RefCell<Option<gtk::Button>>,
        pub voice_edit_callback: RefCell<Option<Box<dyn Fn()>>>,
        // Summary & chapters (LLM-generated for long file transcripts)
        pub summary_expander: RefCell<Option<gtk::Expander>>,
        pub summary_label: RefCell<Option<gtk::Label>>,
        pub chapters_box: RefCell<Option<gtk::Box>>,
        pub waveform_area: RefCell<Option<gtk::DrawingArea>>,
        pub waveform_data: RefCell<Vec<f32>>,
        pub drop_callback: RefCell<Option<Box<dyn Fn(std::path::PathBuf)>>>,
        // Transcribing decode-sweep (shown in place of the waveform while decoding)
        pub seg_box: RefCell<Option<gtk::Box>>,
        pub seg_cells: RefCell<Vec<gtk::Box>>,
        pub seg_pos: Cell<usize>,
        pub decoding: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TranscriptView {
        const NAME: &'static str = "SttTranscriptView";
        type Type = super::TranscriptView;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for TranscriptView {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_ui();
        }
    }

    impl WidgetImpl for TranscriptView {}
    impl BoxImpl for TranscriptView {}
}

glib::wrapper! {
    pub struct TranscriptView(ObjectSubclass<imp::TranscriptView>)
        @extends gtk::Widget, gtk::Box;
}

impl TranscriptView {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("orientation", gtk::Orientation::Vertical)
            .property("spacing", 0)
            // Fill the right column's full width when the window is resized/maximized.
            .property("hexpand", true)
            .build()
    }

    fn setup_ui(&self) {
        let imp = self.imp();
        self.add_css_class("transcript-view");

        // === Info bar: Timer + Language ===
        let info_bar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        info_bar.set_margin_start(16);
        info_bar.set_margin_end(16);
        info_bar.set_margin_top(8);
        info_bar.set_margin_bottom(4);

        // Recording timer
        let timer_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let rec_icon = gtk::Image::from_icon_name("media-record-symbolic");
        rec_icon.set_pixel_size(12);
        rec_icon.add_css_class("recording-indicator");
        timer_box.append(&rec_icon);

        let timer_label = gtk::Label::new(Some("00:00"));
        timer_label.add_css_class("monospace");
        timer_label.add_css_class("caption");
        timer_box.append(&timer_label);
        info_bar.append(&timer_box);

        // Spacer (keeps the timer left-aligned). The language indicator now
        // lives in the bottom status bar.
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        info_bar.append(&spacer);

        // Session stats (word count · WPM), right-aligned. Empty until a result.
        let stats_label = gtk::Label::new(None);
        stats_label.add_css_class("caption");
        stats_label.add_css_class("dim-label");
        stats_label.set_xalign(1.0);
        info_bar.append(&stats_label);

        self.append(&info_bar);

        // === Message bubbles area ===
        let scrolled = gtk::ScrolledWindow::new();
        // Size to content (capped) instead of greedily expanding, so the controls
        // row sits directly under the message block instead of being pushed to the
        // window bottom. A vexpanding spacer (added below) keeps the waveform at the
        // bottom; messages beyond the cap scroll.
        scrolled.set_vexpand(false);
        scrolled.set_propagate_natural_height(true);
        scrolled.set_max_content_height(500);
        scrolled.set_hscrollbar_policy(gtk::PolicyType::Never);
        scrolled.set_margin_start(16);
        scrolled.set_margin_end(16);
        scrolled.set_margin_top(8);
        scrolled.set_margin_bottom(8);

        let bubble_list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        bubble_list.set_valign(gtk::Align::Start);
        bubble_list.set_margin_end(12);

        // Placeholder label
        let placeholder = gtk::Label::new(Some("Press Record to start transcribing…"));
        placeholder.add_css_class("dim-label");
        placeholder.set_vexpand(true);
        placeholder.set_valign(gtk::Align::Center);
        placeholder.set_halign(gtk::Align::Center);
        bubble_list.append(&placeholder);

        scrolled.set_child(Some(&bubble_list));
        self.append(&scrolled);

        // === Transform actions + raw/polished selector (under the transcript) ===
        // The row is always present; its children (Actions menu / selector /
        // voice-edit) manage their own visibility. (Do NOT toggle the row via
        // is_visible()-based logic: is_visible() considers ancestors and
        // deadlocks here.) The variant selector sits on the left; the Actions
        // dropdown and Voice edit are grouped together on the right.
        let controls_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        controls_row.set_margin_start(16);
        controls_row.set_margin_end(16);
        controls_row.set_margin_bottom(4);

        // Raw ↔ Polished selector (shown only once a variant exists).
        let variant_dropdown = gtk::DropDown::from_strings(&[]);
        variant_dropdown.set_valign(gtk::Align::Center);
        variant_dropdown.set_visible(false);
        variant_dropdown.set_tooltip_text(Some(&gettext("Switch between the raw transcript and AI versions")));
        controls_row.append(&variant_dropdown);

        let view = self.clone();
        variant_dropdown.connect_selected_notify(move |dd| {
            if view.imp().variant_syncing.get() {
                return;
            }
            let idx = dd.selected() as usize;
            if let Some(cb) = view.imp().variant_callback.borrow().as_ref() {
                cb(idx);
            }
        });

        // Spacer pushes the Actions dropdown + Voice edit to the right edge.
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        controls_row.append(&spacer);

        // Actions: every transform preset collapsed into one dropdown so the row
        // stays calm instead of a wall of chips. Items are (re)built by
        // `set_chip_presets`; clicking one fires `chip_callback(index)`.
        let actions_btn = gtk::MenuButton::new();
        actions_btn.add_css_class("pill");
        actions_btn.add_css_class("transform-action"); // 12px rounded-rect (not a full pill)
        actions_btn.set_valign(gtk::Align::Center);
        actions_btn.set_visible(false);
        actions_btn.set_tooltip_text(Some(&gettext("Transform this text with AI")));
        let actions_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let actions_icon = gtk::Image::from_icon_name("com.chrisdaggas.speech-to-text-ai");
        actions_icon.set_pixel_size(16);
        let actions_label = gtk::Label::new(Some(&gettext("Actions")));
        let actions_caret = gtk::Image::from_icon_name("pan-down-symbolic");
        actions_caret.set_pixel_size(12);
        actions_content.append(&actions_icon);
        actions_content.append(&actions_label);
        actions_content.append(&actions_caret);
        actions_btn.set_child(Some(&actions_content));

        let actions_popover = gtk::Popover::new();
        actions_popover.add_css_class("menu");
        actions_popover.set_has_arrow(false);          // clean rectangular menu
        actions_popover.set_position(gtk::PositionType::Top); // open upward, over the transcript
        let actions_list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        actions_popover.set_child(Some(&actions_list));
        actions_btn.set_popover(Some(&actions_popover));
        controls_row.append(&actions_btn);

        // Voice edit: speak an instruction to transform the selected message.
        let voice_edit_btn = gtk::Button::new();
        let ve_content = adw::ButtonContent::new();
        ve_content.set_icon_name("document-edit-symbolic");
        ve_content.set_label(&gettext("Voice edit"));
        voice_edit_btn.set_child(Some(&ve_content));
        voice_edit_btn.add_css_class("pill");
        voice_edit_btn.add_css_class("suggested-action");
        voice_edit_btn.add_css_class("transform-action"); // 12px rounded-rect (not a full pill)
        voice_edit_btn.set_valign(gtk::Align::Center);
        voice_edit_btn.set_visible(false);
        voice_edit_btn.set_tooltip_text(Some(&gettext("Speak an instruction to change the selected message")));
        controls_row.append(&voice_edit_btn);
        let view = self.clone();
        voice_edit_btn.connect_clicked(move |_| {
            if let Some(cb) = view.imp().voice_edit_callback.borrow().as_ref() {
                cb();
            }
        });
        *imp.voice_edit_btn.borrow_mut() = Some(voice_edit_btn);

        self.append(&controls_row);

        // === Summary & chapters (LLM, long transcripts; hidden until generated) ===
        let summary_expander = gtk::Expander::new(Some(&gettext("Summary & chapters")));
        summary_expander.set_margin_start(16);
        summary_expander.set_margin_end(16);
        summary_expander.set_margin_bottom(4);
        summary_expander.set_visible(false);
        let summary_body = gtk::Box::new(gtk::Orientation::Vertical, 6);
        summary_body.set_margin_top(6);
        let summary_label = gtk::Label::new(None);
        summary_label.set_wrap(true);
        summary_label.set_xalign(0.0);
        summary_label.set_selectable(true);
        summary_body.append(&summary_label);
        let chapters_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
        summary_body.append(&chapters_box);
        let summary_scroller = gtk::ScrolledWindow::builder()
            .max_content_height(160)
            .propagate_natural_height(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&summary_body)
            .build();
        summary_expander.set_child(Some(&summary_scroller));
        self.append(&summary_expander);

        // === Bottom panel (waveform + confidence) ===
        let bottom_panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
        bottom_panel.add_css_class("bottom-panel");

        // (No divider line above the waveform — the message area flows straight
        // into the visualizer without a border.)

        // === Waveform visualizer (fixed height) ===
        let waveform_area = gtk::DrawingArea::new();
        waveform_area.set_height_request(48);
        waveform_area.set_margin_start(16);
        waveform_area.set_margin_end(16);
        waveform_area.set_margin_bottom(4);
        waveform_area.add_css_class("waveform");
        waveform_area.set_visible(true);

        let view_weak = self.downgrade();
        waveform_area.set_draw_func(move |_area, cr, width, height| {
            let Some(view) = view_weak.upgrade() else { return };
            let data = view.imp().waveform_data.borrow();

            // Draw waveform bars
            let n_bars = 64usize;
            let bar_width = width as f64 / n_bars as f64;
            let mid_y = height as f64 / 2.0;

            // Use GNOME accent color from CSS (color: @accent_bg_color on .waveform)
            let color = _area.color();
            cr.set_source_rgba(
                color.red() as f64,
                color.green() as f64,
                color.blue() as f64,
                0.8,
            );

            if data.is_empty() {
                // Draw flat line
                cr.set_line_width(1.0);
                cr.move_to(0.0, mid_y);
                cr.line_to(width as f64, mid_y);
                let _ = cr.stroke();
                return;
            }

            for i in 0..n_bars {
                let idx = i * data.len() / n_bars;
                let amplitude = (data.get(idx).copied().unwrap_or(0.0) * 5.0).min(1.0);
                let bar_height = amplitude as f64 * (height as f64 - 4.0);
                let x = i as f64 * bar_width + 1.0;
                let y = mid_y - bar_height / 2.0;
                // Rounded rectangle approximation
                cr.rectangle(x, y, (bar_width - 2.0).max(1.0), bar_height.max(2.0));
            }
            let _ = cr.fill();
        });

        bottom_panel.append(&waveform_area);

        // Transcribing decode-sweep (hidden until transcription starts) — same
        // fixed height (48) as the waveform, so swapping them never shifts layout.
        let seg_box = gtk::Box::new(gtk::Orientation::Horizontal, 3);
        seg_box.set_margin_start(16);
        seg_box.set_margin_end(16);
        seg_box.set_margin_bottom(4);
        seg_box.set_height_request(48);
        seg_box.set_visible(false);
        let mut seg_cells = Vec::with_capacity(N_SEGS);
        for _ in 0..N_SEGS {
            let c = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            c.add_css_class("mp-seg");
            c.set_hexpand(true);
            c.set_valign(gtk::Align::Center);
            seg_box.append(&c);
            seg_cells.push(c);
        }
        bottom_panel.append(&seg_box);

        // === Confidence bar ===
        let confidence_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        confidence_box.set_margin_start(16);
        confidence_box.set_margin_end(16);
        confidence_box.set_margin_top(4);
        confidence_box.set_margin_bottom(12);

        let conf_icon = gtk::Image::from_icon_name("dialog-information-symbolic");
        conf_icon.set_pixel_size(14);
        confidence_box.append(&conf_icon);

        let conf_text = gtk::Label::new(Some("Confidence:"));
        conf_text.add_css_class("caption");
        conf_text.add_css_class("dim-label");
        confidence_box.append(&conf_text);

        let confidence_bar = gtk::LevelBar::new();
        confidence_bar.set_min_value(0.0);
        confidence_bar.set_max_value(1.0);
        confidence_bar.set_value(0.0);
        confidence_bar.set_hexpand(true);
        confidence_bar.add_css_class("confidence-bar");
        confidence_box.append(&confidence_bar);

        let confidence_label = gtk::Label::new(Some("—"));
        confidence_label.add_css_class("caption");
        confidence_label.add_css_class("monospace");
        confidence_label.set_width_chars(5);
        confidence_label.set_xalign(1.0);
        confidence_box.append(&confidence_label);

        bottom_panel.append(&confidence_box);

        // Flexible gap: pushes the waveform/confidence panel to the window bottom
        // while the messages + controls stay attached together near the top.
        let bottom_spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        bottom_spacer.set_vexpand(true);
        self.append(&bottom_spacer);

        self.append(&bottom_panel);

        // Store references
        *imp.bubble_list.borrow_mut() = Some(bubble_list);
        *imp.scrolled_window.borrow_mut() = Some(scrolled);
        *imp.placeholder.borrow_mut() = Some(placeholder);
        *imp.confidence_bar.borrow_mut() = Some(confidence_bar);
        *imp.confidence_label.borrow_mut() = Some(confidence_label);
        *imp.timer_label.borrow_mut() = Some(timer_label);
        *imp.stats_label.borrow_mut() = Some(stats_label);
        *imp.waveform_area.borrow_mut() = Some(waveform_area);
        *imp.seg_box.borrow_mut() = Some(seg_box);
        *imp.seg_cells.borrow_mut() = seg_cells;
        *imp.controls_row.borrow_mut() = Some(controls_row);
        *imp.actions_btn.borrow_mut() = Some(actions_btn);
        *imp.actions_list.borrow_mut() = Some(actions_list);
        *imp.variant_dropdown.borrow_mut() = Some(variant_dropdown);
        *imp.summary_expander.borrow_mut() = Some(summary_expander);
        *imp.summary_label.borrow_mut() = Some(summary_label);
        *imp.chapters_box.borrow_mut() = Some(chapters_box);

        // Drag-and-drop for audio files
        let drop_target = gtk::DropTarget::new(gtk::gio::File::static_type(), gtk::gdk::DragAction::COPY);
        let view_weak = self.downgrade();
        drop_target.connect_drop(move |_, value, _x, _y| {
            let Some(view) = view_weak.upgrade() else { return false };
            let Ok(file) = value.get::<gtk::gio::File>() else { return false };
            let Some(path) = file.path() else { return false };
            let cb = view.imp().drop_callback.borrow();
            if let Some(ref callback) = *cb {
                callback(path);
                return true;
            }
            false
        });
        self.add_controller(drop_target);
    }

    /// Create a selectable message bubble. Returns (bubble container, text label).
    /// Clicking the bubble selects that message (emits its index).
    fn create_bubble(&self, text: &str, index: usize) -> (gtk::Box, gtk::Label) {
        let bubble = gtk::Box::new(gtk::Orientation::Vertical, 0);
        bubble.add_css_class("message-bubble");

        let overlay = gtk::Overlay::new();

        let label = gtk::Label::new(Some(text));
        label.set_wrap(true);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        label.set_xalign(0.0);
        label.set_selectable(true);
        label.add_css_class("bubble-text");
        label.set_margin_end(32); // room for the copy button overlay
        overlay.set_child(Some(&label));

        // Copy button reads the label's CURRENT text (so it follows variants).
        let copy_btn = gtk::Button::from_icon_name("edit-copy-symbolic");
        copy_btn.add_css_class("flat");
        copy_btn.add_css_class("circular");
        copy_btn.add_css_class("bubble-copy-btn");
        copy_btn.set_tooltip_text(Some("Copy this message"));
        copy_btn.set_halign(gtk::Align::End);
        copy_btn.set_valign(gtk::Align::Start);
        let label_weak = label.downgrade();
        copy_btn.connect_clicked(move |btn| {
            if let (Some(display), Some(lbl)) = (gtk::gdk::Display::default(), label_weak.upgrade()) {
                display.clipboard().set_text(&lbl.text());
            }
            btn.set_icon_name("object-select-symbolic");
            let btn_weak = btn.downgrade();
            glib::timeout_add_local_once(std::time::Duration::from_millis(800), move || {
                if let Some(b) = btn_weak.upgrade() {
                    b.set_icon_name("edit-copy-symbolic");
                }
            });
        });
        overlay.add_overlay(&copy_btn);
        bubble.append(&overlay);

        // Click to select this message (so chips / Improve act on it).
        let gesture = gtk::GestureClick::new();
        let view_weak = self.downgrade();
        gesture.connect_released(move |_, _, _, _| {
            if let Some(view) = view_weak.upgrade() {
                if let Some(cb) = view.imp().message_selected_cb.borrow().as_ref() {
                    cb(index);
                }
            }
        });
        bubble.add_controller(gesture);

        (bubble, label)
    }

    /// Append a new message bubble and return its (stable) index. Auto-selects it.
    pub fn add_message(&self, text: &str) -> usize {
        let imp = self.imp();
        if let Some(placeholder) = imp.placeholder.borrow().as_ref() {
            placeholder.set_visible(false);
        }
        let index = imp.message_labels.borrow().len();
        let (bubble, label) = self.create_bubble(text, index);
        if let Some(list) = imp.bubble_list.borrow().as_ref() {
            list.append(&bubble);
        }
        imp.message_bubbles.borrow_mut().push(bubble);
        imp.message_labels.borrow_mut().push(label);
        imp.messages.borrow_mut().push(text.to_string());
        self.set_selected_message(index);
        self.scroll_to_bottom();
        index
    }

    /// Replace the text of message `idx` (e.g. after an AI variant).
    pub fn update_message(&self, idx: usize, text: &str) {
        let imp = self.imp();
        if let Some(label) = imp.message_labels.borrow().get(idx) {
            label.set_text(text);
        }
        if let Some(m) = imp.messages.borrow_mut().get_mut(idx) {
            *m = text.to_string();
        }
    }

    /// Highlight message `idx` as selected (and unhighlight the rest).
    pub fn set_selected_message(&self, idx: usize) {
        let imp = self.imp();
        imp.selected_idx.set(idx as isize);
        for (i, b) in imp.message_bubbles.borrow().iter().enumerate() {
            if i == idx {
                b.add_css_class("selected");
            } else {
                b.remove_css_class("selected");
            }
        }
    }

    /// Register the handler invoked with a message index when its bubble is clicked.
    pub fn connect_message_selected<F: Fn(usize) + 'static>(&self, f: F) {
        *self.imp().message_selected_cb.borrow_mut() = Some(Box::new(f));
    }

    /// Number of messages currently shown.
    pub fn message_count(&self) -> usize {
        self.imp().message_labels.borrow().len()
    }

    /// Show transient "live preview" text while recording (a dim bubble at the
    /// end; NOT a real message). Empty text removes it.
    pub fn set_live_preview(&self, text: &str) {
        let imp = self.imp();
        if text.trim().is_empty() {
            self.clear_live_preview();
            return;
        }
        if let Some(placeholder) = imp.placeholder.borrow().as_ref() {
            placeholder.set_visible(false);
        }
        // Reuse the existing preview label if present, else build one.
        if let Some(lbl) = imp.live_preview_label.borrow().as_ref() {
            lbl.set_text(text);
            self.scroll_to_bottom();
            return;
        }
        let bubble = gtk::Box::new(gtk::Orientation::Vertical, 0);
        bubble.add_css_class("message-bubble");
        bubble.add_css_class("live-preview");
        let label = gtk::Label::new(Some(text));
        label.set_wrap(true);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        label.set_xalign(0.0);
        label.add_css_class("bubble-text");
        label.add_css_class("dim-label");
        bubble.append(&label);
        if let Some(list) = imp.bubble_list.borrow().as_ref() {
            list.append(&bubble);
        }
        *imp.live_preview.borrow_mut() = Some(bubble);
        *imp.live_preview_label.borrow_mut() = Some(label);
        self.scroll_to_bottom();
    }

    /// Remove the transient live-preview bubble.
    pub fn clear_live_preview(&self) {
        let imp = self.imp();
        if let (Some(list), Some(bubble)) =
            (imp.bubble_list.borrow().as_ref(), imp.live_preview.borrow().as_ref())
        {
            list.remove(bubble);
        }
        *imp.live_preview.borrow_mut() = None;
        *imp.live_preview_label.borrow_mut() = None;
    }

    fn scroll_to_bottom(&self) {
        if let Some(sw) = self.imp().scrolled_window.borrow().as_ref() {
            let adj = sw.vadjustment();
            glib::idle_add_local_once(move || {
                adj.set_value(adj.upper() - adj.page_size());
            });
        }
    }

    /// Append text as a new message (legacy alias for `add_message`).
    pub fn append_text(&self, text: &str) {
        self.add_message(text);
    }

    /// Replace everything with a single message (legacy helper).
    pub fn set_text(&self, text: &str) {
        self.clear();
        if !text.is_empty() {
            self.add_message(text);
        }
    }

    /// Clear all message bubbles + the live preview and reset the controls.
    pub fn clear(&self) {
        let imp = self.imp();
        self.clear_live_preview();
        imp.messages.borrow_mut().clear();
        imp.message_bubbles.borrow_mut().clear();
        imp.message_labels.borrow_mut().clear();
        imp.selected_idx.set(-1);

        if let Some(list) = imp.bubble_list.borrow().as_ref() {
            // Remove all children except the placeholder
            while let Some(child) = list.last_child() {
                if let Some(placeholder) = imp.placeholder.borrow().as_ref() {
                    if &child == placeholder.upcast_ref::<gtk::Widget>() {
                        break;
                    }
                }
                list.remove(&child);
            }
        }

        if let Some(placeholder) = imp.placeholder.borrow().as_ref() {
            placeholder.set_visible(true);
        }

        self.set_confidence(0.0);
        self.set_stats(0, None);
        self.hide_result_controls();
        self.clear_summary();
    }

    /// Update confidence indicator (0.0 - 1.0).
    pub fn set_confidence(&self, confidence: f64) {
        let imp = self.imp();
        if let Some(bar) = imp.confidence_bar.borrow().as_ref() {
            bar.set_value(confidence);
        }
        if let Some(label) = imp.confidence_label.borrow().as_ref() {
            if confidence > 0.0 {
                label.set_text(&format!("{:.0}%", confidence * 100.0));
            } else {
                label.set_text("—");
            }
        }
    }

    /// Update the recording timer display.
    pub fn set_timer(&self, seconds: u64) {
        if let Some(label) = self.imp().timer_label.borrow().as_ref() {
            let mins = seconds / 60;
            let secs = seconds % 60;
            label.set_text(&format!("{:02}:{:02}", mins, secs));
        }
    }

    /// Update the session-stats label ("128 words · 96 wpm"). `words == 0` clears it.
    pub fn set_stats(&self, words: usize, wpm: Option<u32>) {
        if let Some(l) = self.imp().stats_label.borrow().as_ref() {
            if words == 0 {
                l.set_text("");
                return;
            }
            let word_label = if words == 1 { gettext("word") } else { gettext("words") };
            let base = format!("{} {}", words, word_label);
            match wpm {
                Some(v) => l.set_text(&format!("{} · {} wpm", base, v)),
                None => l.set_text(&base),
            }
        }
    }

    /// Rebuild the Actions-dropdown items from preset names (one row per preset).
    pub fn set_chip_presets(&self, names: &[String]) {
        let imp = self.imp();
        let Some(list) = imp.actions_list.borrow().clone() else { return };
        let popover = imp.actions_btn.borrow().as_ref().and_then(|b| b.popover());
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
        let mut buttons = Vec::with_capacity(names.len());
        for (i, name) in names.iter().enumerate() {
            let btn = gtk::Button::with_label(name);
            btn.add_css_class("flat");
            btn.add_css_class("menu-item");
            btn.set_halign(gtk::Align::Fill);
            if let Some(label) = btn.child().and_downcast::<gtk::Label>() {
                label.set_xalign(0.0);
            }
            let view = self.clone();
            let pop_weak = popover.as_ref().map(|p| p.downgrade());
            btn.connect_clicked(move |_| {
                // Dismiss the menu, then run the transform on the selected message.
                if let Some(p) = pop_weak.as_ref().and_then(|p| p.upgrade()) {
                    p.popdown();
                }
                if let Some(cb) = view.imp().chip_callback.borrow().as_ref() {
                    cb(i);
                }
            });
            list.append(&btn);
            buttons.push(btn);
        }
        *imp.chip_buttons.borrow_mut() = buttons;
    }

    /// Register the handler invoked with the chip (preset) index when clicked.
    pub fn connect_chip_activated<F: Fn(usize) + 'static>(&self, f: F) {
        *self.imp().chip_callback.borrow_mut() = Some(Box::new(f));
    }

    /// Enable/disable the Actions dropdown (disabled while an AI request is in flight).
    pub fn set_chips_sensitive(&self, on: bool) {
        if let Some(b) = self.imp().actions_btn.borrow().as_ref() {
            b.set_sensitive(on);
        }
        for b in self.imp().chip_buttons.borrow().iter() {
            b.set_sensitive(on);
        }
    }

    /// Show/hide the Actions dropdown (e.g. only when the LLM integration is enabled).
    pub fn set_chips_visible(&self, on: bool) {
        if let Some(b) = self.imp().actions_btn.borrow().as_ref() {
            b.set_visible(on);
        }
    }

    /// Rebuild the raw/polished variant selector. Hidden when there's only one
    /// entry (i.e. just the raw transcript, no AI variants yet).
    pub fn set_variant_selector(&self, labels: &[String], active: usize) {
        let imp = self.imp();
        let Some(dd) = imp.variant_dropdown.borrow().clone() else { return };
        imp.variant_syncing.set(true);
        let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
        dd.set_model(Some(&gtk::StringList::new(&refs)));
        dd.set_selected(active as u32);
        dd.set_visible(labels.len() > 1);
        imp.variant_syncing.set(false);
    }

    /// Register the handler invoked with the selected variant index (0 = raw).
    pub fn connect_variant_changed<F: Fn(usize) + 'static>(&self, f: F) {
        *self.imp().variant_callback.borrow_mut() = Some(Box::new(f));
    }

    /// Register the handler invoked when the Voice-edit button is clicked.
    pub fn connect_voice_edit<F: Fn() + 'static>(&self, f: F) {
        *self.imp().voice_edit_callback.borrow_mut() = Some(Box::new(f));
    }

    /// Show/hide the Voice-edit button (only when the LLM is enabled + a result).
    pub fn set_voice_edit_visible(&self, on: bool) {
        if let Some(b) = self.imp().voice_edit_btn.borrow().as_ref() {
            b.set_visible(on);
        }
    }

    /// Toggle the Voice-edit button between idle and "recording the instruction".
    pub fn set_voice_edit_recording(&self, recording: bool) {
        if let Some(b) = self.imp().voice_edit_btn.borrow().as_ref() {
            if let Some(c) = b.child().and_downcast::<adw::ButtonContent>() {
                if recording {
                    c.set_icon_name("media-playback-stop-symbolic");
                    c.set_label(&gettext("Stop edit"));
                    b.remove_css_class("suggested-action");
                    b.add_css_class("destructive-action");
                } else {
                    c.set_icon_name("document-edit-symbolic");
                    c.set_label(&gettext("Voice edit"));
                    b.remove_css_class("destructive-action");
                    b.add_css_class("suggested-action");
                }
            }
        }
    }


    /// Show the "Summarizing…" placeholder (or hide it when `on` is false).
    pub fn set_summary_loading(&self, on: bool) {
        let imp = self.imp();
        if let Some(e) = imp.summary_expander.borrow().as_ref() {
            e.set_visible(on);
            e.set_expanded(on);
        }
        if let Some(l) = imp.summary_label.borrow().as_ref() {
            let placeholder = if on { gettext("Summarizing…") } else { String::new() };
            l.set_text(&placeholder);
        }
        if on {
            if let Some(cb) = imp.chapters_box.borrow().as_ref() {
                while let Some(child) = cb.first_child() {
                    cb.remove(&child);
                }
            }
        }
    }

    /// Populate the summary text and the chapter list.
    pub fn set_summary(&self, summary: &str, chapters: &[(String, String)]) {
        let imp = self.imp();
        if let Some(e) = imp.summary_expander.borrow().as_ref() {
            e.set_visible(true);
            e.set_expanded(true);
        }
        if let Some(l) = imp.summary_label.borrow().as_ref() {
            l.set_text(summary);
        }
        if let Some(cb) = imp.chapters_box.borrow().as_ref() {
            while let Some(child) = cb.first_child() {
                cb.remove(&child);
            }
            for (ts, title) in chapters {
                let row = gtk::Label::new(Some(&format!("{}  —  {}", ts, title)));
                row.set_xalign(0.0);
                row.set_wrap(true);
                row.set_selectable(true);
                row.add_css_class("caption");
                cb.append(&row);
            }
        }
    }

    /// Hide and clear the summary/chapters section.
    pub fn clear_summary(&self) {
        let imp = self.imp();
        if let Some(e) = imp.summary_expander.borrow().as_ref() {
            e.set_visible(false);
        }
        if let Some(l) = imp.summary_label.borrow().as_ref() {
            l.set_text("");
        }
        if let Some(cb) = imp.chapters_box.borrow().as_ref() {
            while let Some(child) = cb.first_child() {
                cb.remove(&child);
            }
        }
    }

    /// Hide the chips / selector / voice-edit children (while recording / when no
    /// result). The row container itself stays present (its children are hidden).
    pub fn hide_result_controls(&self) {
        let imp = self.imp();
        if let Some(d) = imp.variant_dropdown.borrow().as_ref() {
            d.set_visible(false);
        }
        if let Some(b) = imp.actions_btn.borrow().as_ref() {
            b.set_visible(false);
        }
        if let Some(b) = imp.voice_edit_btn.borrow().as_ref() {
            b.set_visible(false);
        }
    }

    /// Set the recording state.
    pub fn set_recording(&self, recording: bool) {
        self.imp().is_recording.set(recording);
        if !recording {
            self.imp().waveform_data.borrow_mut().clear();
            if let Some(area) = self.imp().waveform_area.borrow().as_ref() {
                area.queue_draw();
            }
        }
    }

    /// Update waveform amplitude data and trigger a redraw.
    pub fn update_waveform(&self, amplitudes: Vec<f32>) {
        *self.imp().waveform_data.borrow_mut() = amplitudes;
        if let Some(area) = self.imp().waveform_area.borrow().as_ref() {
            area.queue_draw();
        }
    }

    /// Start the transcribing decode-sweep animation (shown in place of the
    /// recording waveform), mirroring the mini panel's sweep for consistency.
    pub fn start_transcribing_anim(&self) {
        let imp = self.imp();
        if let Some(w) = imp.waveform_area.borrow().as_ref() {
            w.set_visible(false);
        }
        if let Some(s) = imp.seg_box.borrow().as_ref() {
            s.set_visible(true);
        }
        imp.seg_pos.set(0);
        if imp.decoding.get() {
            return;
        }
        imp.decoding.set(true);

        let view_weak = self.downgrade();
        glib::timeout_add_local(std::time::Duration::from_millis(90), move || {
            let Some(view) = view_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let imp = view.imp();
            if !imp.decoding.get() {
                return glib::ControlFlow::Break;
            }
            let cells = imp.seg_cells.borrow();
            let n = cells.len();
            if n == 0 {
                return glib::ControlFlow::Break;
            }
            let pos = imp.seg_pos.get();
            let win = 6usize; // moving lit window width
            for (i, c) in cells.iter().enumerate() {
                let lit = (i + n - (pos % n)) % n < win;
                if lit {
                    c.add_css_class("on");
                } else {
                    c.remove_css_class("on");
                }
            }
            imp.seg_pos.set(pos + 1);
            glib::ControlFlow::Continue
        });
    }

    /// Show a *determinate* decode progress (0–100) by lighting the segmented
    /// bar proportionally instead of the indeterminate sweep. Used by live
    /// streaming, where whisper reports real progress.
    pub fn set_decode_progress(&self, pct: i32) {
        let imp = self.imp();
        imp.decoding.set(false); // stop any running sweep loop
        if let Some(w) = imp.waveform_area.borrow().as_ref() {
            w.set_visible(false);
        }
        if let Some(s) = imp.seg_box.borrow().as_ref() {
            s.set_visible(true);
        }
        let cells = imp.seg_cells.borrow();
        let n = cells.len();
        if n == 0 {
            return;
        }
        let lit = ((pct.clamp(0, 100) as f64 / 100.0) * n as f64).round() as usize;
        for (i, c) in cells.iter().enumerate() {
            if i < lit {
                c.add_css_class("on");
            } else {
                c.remove_css_class("on");
            }
        }
    }

    /// Stop the transcribing animation and restore the waveform.
    pub fn stop_transcribing_anim(&self) {
        let imp = self.imp();
        imp.decoding.set(false);
        if let Some(s) = imp.seg_box.borrow().as_ref() {
            s.set_visible(false);
        }
        if let Some(w) = imp.waveform_area.borrow().as_ref() {
            w.set_visible(true);
        }
        for c in imp.seg_cells.borrow().iter() {
            c.remove_css_class("on");
        }
    }

    /// Get the full text content (all messages joined by newlines).
    pub fn get_text(&self) -> String {
        let messages = self.imp().messages.borrow();
        if messages.is_empty() {
            String::new()
        } else {
            messages.join("\n\n")
        }
    }

    /// Connect a callback for when an audio file is dropped onto the view.
    pub fn connect_file_dropped<F: Fn(std::path::PathBuf) + 'static>(&self, callback: F) {
        *self.imp().drop_callback.borrow_mut() = Some(Box::new(callback));
    }
}
