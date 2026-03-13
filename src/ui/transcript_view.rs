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

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct TranscriptView {
        pub bubble_list: RefCell<Option<gtk::Box>>,
        pub scrolled_window: RefCell<Option<gtk::ScrolledWindow>>,
        pub placeholder: RefCell<Option<gtk::Label>>,
        pub messages: RefCell<Vec<String>>,
        pub confidence_bar: RefCell<Option<gtk::LevelBar>>,
        pub confidence_label: RefCell<Option<gtk::Label>>,
        pub timer_label: RefCell<Option<gtk::Label>>,
        pub language_label: RefCell<Option<gtk::Label>>,
        pub is_recording: Cell<bool>,
        pub waveform_area: RefCell<Option<gtk::DrawingArea>>,
        pub waveform_data: RefCell<Vec<f32>>,
        pub drop_callback: RefCell<Option<Box<dyn Fn(std::path::PathBuf)>>>,
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

        // Spacer
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        info_bar.append(&spacer);

        // Language detection
        let lang_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let lang_icon = gtk::Image::from_icon_name("preferences-desktop-locale-symbolic");
        lang_icon.set_pixel_size(12);
        lang_box.append(&lang_icon);

        let language_label = gtk::Label::new(Some("Auto-detect"));
        language_label.add_css_class("caption");
        language_label.add_css_class("dim-label");
        lang_box.append(&language_label);
        info_bar.append(&lang_box);

        self.append(&info_bar);

        // === Message bubbles area ===
        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_vexpand(true);
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

        // === Bottom panel (waveform + confidence) ===
        let bottom_panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
        bottom_panel.add_css_class("bottom-panel");

        // Separator above waveform / confidence
        let conf_separator = gtk::Separator::new(gtk::Orientation::Horizontal);
        conf_separator.set_margin_start(16);
        conf_separator.set_margin_end(16);
        conf_separator.set_margin_bottom(4);
        bottom_panel.append(&conf_separator);

        // === Waveform visualizer ===
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

        self.append(&bottom_panel);

        // Store references
        *imp.bubble_list.borrow_mut() = Some(bubble_list);
        *imp.scrolled_window.borrow_mut() = Some(scrolled);
        *imp.placeholder.borrow_mut() = Some(placeholder);
        *imp.confidence_bar.borrow_mut() = Some(confidence_bar);
        *imp.confidence_label.borrow_mut() = Some(confidence_label);
        *imp.timer_label.borrow_mut() = Some(timer_label);
        *imp.language_label.borrow_mut() = Some(language_label);
        *imp.waveform_area.borrow_mut() = Some(waveform_area);

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

    /// Create a message bubble widget for the given text.
    fn create_bubble(&self, text: &str) -> gtk::Box {
        let bubble = gtk::Box::new(gtk::Orientation::Vertical, 0);
        bubble.add_css_class("message-bubble");

        let overlay = gtk::Overlay::new();

        // Message text
        let label = gtk::Label::new(Some(text));
        label.set_wrap(true);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        label.set_xalign(0.0);
        label.set_selectable(true);
        label.add_css_class("bubble-text");
        label.set_margin_end(32); // Leave space for the copy button overlay
        overlay.set_child(Some(&label));

        // Copy button overlaid at top-right corner
        let copy_btn = gtk::Button::from_icon_name("edit-copy-symbolic");
        copy_btn.add_css_class("flat");
        copy_btn.add_css_class("circular");
        copy_btn.add_css_class("bubble-copy-btn");
        copy_btn.set_tooltip_text(Some("Copy this message"));
        copy_btn.set_halign(gtk::Align::End);
        copy_btn.set_valign(gtk::Align::Start);

        let text_for_copy = text.to_string();
        copy_btn.connect_clicked(move |btn| {
            if let Some(display) = gtk::gdk::Display::default() {
                display.clipboard().set_text(&text_for_copy);
            }
            // Brief visual feedback
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

        bubble
    }

    /// Append text as a new message bubble.
    pub fn append_text(&self, text: &str) {
        let imp = self.imp();

        // Hide placeholder on first message
        if let Some(placeholder) = imp.placeholder.borrow().as_ref() {
            placeholder.set_visible(false);
        }

        // Store the message text
        imp.messages.borrow_mut().push(text.to_string());

        // Create and add the bubble widget
        if let Some(list) = imp.bubble_list.borrow().as_ref() {
            let bubble = self.create_bubble(text);
            list.append(&bubble);
        }

        // Auto-scroll to bottom
        if let Some(sw) = imp.scrolled_window.borrow().as_ref() {
            let adj = sw.vadjustment();
            glib::idle_add_local_once(move || {
                adj.set_value(adj.upper() - adj.page_size());
            });
        }
    }

    /// Set the full transcript text, replacing everything.
    pub fn set_text(&self, text: &str) {
        self.clear();
        if !text.is_empty() {
            self.append_text(text);
        }
    }

    /// Clear all message bubbles.
    pub fn clear(&self) {
        let imp = self.imp();
        imp.messages.borrow_mut().clear();

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

        // Show placeholder again
        if let Some(placeholder) = imp.placeholder.borrow().as_ref() {
            placeholder.set_visible(true);
        }

        self.set_confidence(0.0);
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

    /// Update the detected language display.
    pub fn set_language(&self, language: &str) {
        if let Some(label) = self.imp().language_label.borrow().as_ref() {
            label.set_text(language);
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
