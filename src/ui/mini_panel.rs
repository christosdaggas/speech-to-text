// Speech to Text - Mini Panel
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Compact floating dictation panel.
//!
//! A second, independent `adw::ApplicationWindow` used by the global dictation
//! shortcut. It shows a live waveform + timer while recording, then a transcript
//! preview with Paste / Copy / Close once transcription completes.
//!
//! Note on GNOME/Wayland: a normal client cannot force always-on-top, position
//! itself near the cursor, or use layer-shell (Mutter implements none of these),
//! so this is a compact window the compositor places. The dictation flow works
//! regardless; it simply isn't a forced topmost overlay.

use gtk4::prelude::*;
use gtk4::glib;
use gtk4 as gtk;
use libadwaita as adw;
use adw::prelude::*;
use adw::subclass::prelude::*;
use std::cell::{Cell, RefCell};

use crate::application::Application;
use crate::i18n::gettext;

/// Actions emitted by the mini panel's buttons / close request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MiniPanelAction {
    /// Stop recording and transcribe.
    Stop,
    /// Abort recording and discard.
    Cancel,
    /// Paste the previewed transcript into the focused app.
    Paste,
    /// Copy the previewed transcript to the clipboard.
    Copy,
    /// Start a fresh dictation without closing the panel.
    Again,
    /// Dismiss the panel after a result.
    Close,
}

/// Visual state of the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum PanelState {
    #[default]
    Recording,
    Transcribing,
    Result,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct MiniPanel {
        pub(super) state: Cell<PanelState>,
        pub waveform_area: RefCell<Option<gtk::DrawingArea>>,
        pub waveform_data: RefCell<Vec<f32>>,
        pub record_dot: RefCell<Option<gtk::Image>>,
        pub spinner: RefCell<Option<gtk::Spinner>>,
        pub timer_label: RefCell<Option<gtk::Label>>,
        pub state_label: RefCell<Option<gtk::Label>>,
        pub mode_chip: RefCell<Option<gtk::Label>>,
        pub preview_label: RefCell<Option<gtk::Label>>,
        pub copied_box: RefCell<Option<gtk::Box>>,
        // Buttons
        pub stop_btn: RefCell<Option<gtk::Button>>,
        pub cancel_btn: RefCell<Option<gtk::Button>>,
        pub again_btn: RefCell<Option<gtk::Button>>,
        pub paste_btn: RefCell<Option<gtk::Button>>,
        pub copy_btn: RefCell<Option<gtk::Button>>,
        pub close_btn: RefCell<Option<gtk::Button>>,
        pub action_callback: RefCell<Option<Box<dyn Fn(MiniPanelAction)>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MiniPanel {
        const NAME: &'static str = "SttMiniPanel";
        type Type = super::MiniPanel;
        type ParentType = adw::ApplicationWindow;
    }

    impl ObjectImpl for MiniPanel {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_ui();
        }
    }

    impl WidgetImpl for MiniPanel {}
    impl WindowImpl for MiniPanel {}
    impl ApplicationWindowImpl for MiniPanel {}
    impl AdwApplicationWindowImpl for MiniPanel {}
}

glib::wrapper! {
    pub struct MiniPanel(ObjectSubclass<imp::MiniPanel>)
        @extends gtk::Widget, gtk::Window, gtk::ApplicationWindow, adw::ApplicationWindow,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native, gtk::Root;
}

impl MiniPanel {
    const WIDTH: i32 = 380;
    const HEIGHT: i32 = 210;

    pub fn new(app: &Application) -> Self {
        let panel: Self = glib::Object::builder()
            .property("application", app)
            .build();
        panel.set_default_size(Self::WIDTH, Self::HEIGHT);
        panel.set_resizable(false);
        panel.set_title(Some(&gettext("Dictation")));
        panel.set_hide_on_close(true);
        panel
    }

    fn setup_ui(&self) {
        let imp = self.imp();
        self.add_css_class("mini-panel");

        let toolbar = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        header.add_css_class("flat");
        toolbar.add_top_bar(&header);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        body.set_margin_start(16);
        body.set_margin_end(16);
        body.set_margin_top(8);
        body.set_margin_bottom(16);

        // === Status line: dot/spinner + timer + state + mode chip ===
        let status_line = gtk::Box::new(gtk::Orientation::Horizontal, 10);

        let record_dot = gtk::Image::from_icon_name("media-record-symbolic");
        record_dot.set_pixel_size(14);
        record_dot.add_css_class("recording-indicator");
        record_dot.add_css_class("recording-pulse");
        status_line.append(&record_dot);

        let spinner = gtk::Spinner::new();
        spinner.set_visible(false);
        status_line.append(&spinner);

        let timer_label = gtk::Label::new(Some("00:00"));
        timer_label.add_css_class("monospace");
        timer_label.add_css_class("title-4");
        status_line.append(&timer_label);

        let state_label = gtk::Label::new(Some(&gettext("Listening…")));
        state_label.add_css_class("dim-label");
        state_label.set_halign(gtk::Align::Start);
        state_label.set_hexpand(true);
        status_line.append(&state_label);

        let mode_chip = gtk::Label::new(Some("Plain"));
        mode_chip.add_css_class("caption");
        mode_chip.add_css_class("mini-panel-chip");
        status_line.append(&mode_chip);

        body.append(&status_line);

        // === Waveform ===
        let waveform_area = gtk::DrawingArea::new();
        waveform_area.set_height_request(46);
        waveform_area.add_css_class("waveform");
        let panel_weak = self.downgrade();
        waveform_area.set_draw_func(move |area, cr, width, height| {
            let Some(panel) = panel_weak.upgrade() else { return };
            let data = panel.imp().waveform_data.borrow();

            let n_bars = 64usize;
            let bar_width = width as f64 / n_bars as f64;
            let mid_y = height as f64 / 2.0;

            let color = area.color();
            cr.set_source_rgba(
                color.red() as f64,
                color.green() as f64,
                color.blue() as f64,
                0.8,
            );

            if data.is_empty() {
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
                cr.rectangle(x, y, (bar_width - 2.0).max(1.0), bar_height.max(2.0));
            }
            let _ = cr.fill();
        });
        body.append(&waveform_area);

        // === Transcript preview (hidden until result) ===
        let preview_label = gtk::Label::new(None);
        preview_label.set_wrap(true);
        preview_label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        preview_label.set_xalign(0.0);
        preview_label.set_selectable(true);
        preview_label.add_css_class("mini-panel-preview");
        preview_label.set_visible(false);
        body.append(&preview_label);

        // "Copied to clipboard" confirmation (hidden until result)
        let copied_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        copied_box.set_visible(false);
        let copied_icon = gtk::Image::from_icon_name("object-select-symbolic");
        copied_icon.set_pixel_size(14);
        copied_icon.add_css_class("success");
        copied_box.append(&copied_icon);
        let copied_label = gtk::Label::new(Some(&gettext("Copied to clipboard")));
        copied_label.add_css_class("caption");
        copied_label.add_css_class("success");
        copied_box.append(&copied_label);
        body.append(&copied_box);

        // === Button row ===
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.set_homogeneous(true);
        actions.set_margin_top(2);

        let stop_btn = gtk::Button::with_label(&gettext("Stop & Transcribe"));
        stop_btn.add_css_class("suggested-action");
        stop_btn.add_css_class("pill");
        actions.append(&stop_btn);

        let cancel_btn = gtk::Button::with_label(&gettext("Cancel"));
        cancel_btn.add_css_class("destructive-action");
        cancel_btn.add_css_class("pill");
        actions.append(&cancel_btn);

        // Result state: start a new dictation without closing the panel.
        let again_btn = gtk::Button::with_label(&gettext("New"));
        again_btn.add_css_class("suggested-action");
        again_btn.add_css_class("pill");
        again_btn.set_visible(false);
        actions.append(&again_btn);

        let paste_btn = gtk::Button::with_label(&gettext("Paste"));
        paste_btn.add_css_class("suggested-action");
        paste_btn.add_css_class("pill");
        paste_btn.set_visible(false);
        actions.append(&paste_btn);

        let copy_btn = gtk::Button::with_label(&gettext("Copy"));
        copy_btn.add_css_class("pill");
        copy_btn.set_visible(false);
        actions.append(&copy_btn);

        let close_btn = gtk::Button::with_label(&gettext("Close"));
        close_btn.add_css_class("pill");
        close_btn.set_visible(false);
        actions.append(&close_btn);

        body.append(&actions);

        toolbar.set_content(Some(&body));
        self.set_content(Some(&toolbar));

        // Store references
        *imp.waveform_area.borrow_mut() = Some(waveform_area);
        *imp.record_dot.borrow_mut() = Some(record_dot);
        *imp.spinner.borrow_mut() = Some(spinner);
        *imp.timer_label.borrow_mut() = Some(timer_label);
        *imp.state_label.borrow_mut() = Some(state_label);
        *imp.mode_chip.borrow_mut() = Some(mode_chip);
        *imp.preview_label.borrow_mut() = Some(preview_label);
        *imp.copied_box.borrow_mut() = Some(copied_box);
        *imp.stop_btn.borrow_mut() = Some(stop_btn.clone());
        *imp.cancel_btn.borrow_mut() = Some(cancel_btn.clone());
        *imp.again_btn.borrow_mut() = Some(again_btn.clone());
        *imp.paste_btn.borrow_mut() = Some(paste_btn.clone());
        *imp.copy_btn.borrow_mut() = Some(copy_btn.clone());
        *imp.close_btn.borrow_mut() = Some(close_btn.clone());

        // Wire buttons to the action callback.
        self.wire_button(&stop_btn, MiniPanelAction::Stop);
        self.wire_button(&cancel_btn, MiniPanelAction::Cancel);
        self.wire_button(&again_btn, MiniPanelAction::Again);
        self.wire_button(&paste_btn, MiniPanelAction::Paste);
        self.wire_button(&copy_btn, MiniPanelAction::Copy);
        self.wire_button(&close_btn, MiniPanelAction::Close);

        // Closing the window (titlebar X) maps to Cancel while recording, else
        // Close. The window only hides (hide_on_close), so it can be reused.
        let panel_weak = self.downgrade();
        self.connect_close_request(move |_| {
            if let Some(panel) = panel_weak.upgrade() {
                let action = match panel.imp().state.get() {
                    PanelState::Result => MiniPanelAction::Close,
                    _ => MiniPanelAction::Cancel,
                };
                panel.emit_action(action);
            }
            glib::Propagation::Proceed
        });
    }

    fn wire_button(&self, button: &gtk::Button, action: MiniPanelAction) {
        let panel_weak = self.downgrade();
        button.connect_clicked(move |_| {
            if let Some(panel) = panel_weak.upgrade() {
                panel.emit_action(action);
            }
        });
    }

    fn emit_action(&self, action: MiniPanelAction) {
        if let Some(cb) = self.imp().action_callback.borrow().as_ref() {
            cb(action);
        }
    }

    /// Register a callback for panel actions (Stop / Cancel / Paste / Copy / Close).
    pub fn connect_action<F: Fn(MiniPanelAction) + 'static>(&self, f: F) {
        *self.imp().action_callback.borrow_mut() = Some(Box::new(f));
    }

    /// Enter the recording state and reset the panel. `mode_label` is the human
    /// label for the active dictation mode (e.g. "Plain").
    pub fn show_recording(&self, mode_label: &str) {
        let imp = self.imp();
        imp.state.set(PanelState::Recording);
        imp.waveform_data.borrow_mut().clear();

        if let Some(chip) = imp.mode_chip.borrow().as_ref() {
            chip.set_text(mode_label);
        }
        if let Some(dot) = imp.record_dot.borrow().as_ref() {
            dot.set_visible(true);
        }
        if let Some(spinner) = imp.spinner.borrow().as_ref() {
            spinner.set_visible(false);
            spinner.stop();
        }
        if let Some(label) = imp.timer_label.borrow().as_ref() {
            label.set_text("00:00");
        }
        if let Some(label) = imp.state_label.borrow().as_ref() {
            label.remove_css_class("success");
            label.add_css_class("dim-label");
            label.set_text(&gettext("Listening…"));
        }
        if let Some(area) = imp.waveform_area.borrow().as_ref() {
            area.set_visible(true);
            area.queue_draw();
        }
        if let Some(preview) = imp.preview_label.borrow().as_ref() {
            preview.set_visible(false);
            preview.set_text("");
        }
        if let Some(copied) = imp.copied_box.borrow().as_ref() {
            copied.set_visible(false);
        }
        // Always re-enable Stop: a previous transcription disabled it, and that
        // state would otherwise persist into a new recording.
        if let Some(btn) = imp.stop_btn.borrow().as_ref() {
            btn.set_sensitive(true);
        }
        self.set_recording_buttons(true);
    }

    /// Enter the transcribing state (spinner, disabled Stop).
    pub fn show_transcribing(&self) {
        let imp = self.imp();
        imp.state.set(PanelState::Transcribing);
        if let Some(dot) = imp.record_dot.borrow().as_ref() {
            dot.set_visible(false);
        }
        if let Some(spinner) = imp.spinner.borrow().as_ref() {
            spinner.set_visible(true);
            spinner.start();
        }
        if let Some(label) = imp.state_label.borrow().as_ref() {
            label.set_text(&gettext("Transcribing…"));
        }
        if let Some(btn) = imp.stop_btn.borrow().as_ref() {
            btn.set_sensitive(false);
        }
    }

    /// Show the transcription result. `copied` controls the "Copied to clipboard"
    /// confirmation line.
    pub fn show_result(&self, text: &str, copied: bool) {
        let imp = self.imp();
        imp.state.set(PanelState::Result);

        if let Some(dot) = imp.record_dot.borrow().as_ref() {
            dot.set_visible(false);
        }
        if let Some(spinner) = imp.spinner.borrow().as_ref() {
            spinner.set_visible(false);
            spinner.stop();
        }
        if let Some(label) = imp.timer_label.borrow().as_ref() {
            label.set_text("");
        }
        if let Some(label) = imp.state_label.borrow().as_ref() {
            label.remove_css_class("dim-label");
            label.add_css_class("success");
            label.set_text(&gettext("Transcript ready"));
        }
        if let Some(area) = imp.waveform_area.borrow().as_ref() {
            area.set_visible(false);
        }
        if let Some(preview) = imp.preview_label.borrow().as_ref() {
            preview.set_text(text);
            preview.set_visible(true);
        }
        if let Some(copied_box) = imp.copied_box.borrow().as_ref() {
            copied_box.set_visible(copied);
        }
        if let Some(btn) = imp.stop_btn.borrow().as_ref() {
            btn.set_sensitive(true);
        }
        self.set_recording_buttons(false);
    }

    /// Show an error message in place of a result.
    pub fn show_error(&self, message: &str) {
        let imp = self.imp();
        imp.state.set(PanelState::Result);
        if let Some(spinner) = imp.spinner.borrow().as_ref() {
            spinner.set_visible(false);
            spinner.stop();
        }
        if let Some(dot) = imp.record_dot.borrow().as_ref() {
            dot.set_visible(false);
        }
        if let Some(label) = imp.state_label.borrow().as_ref() {
            label.remove_css_class("success");
            label.add_css_class("dim-label");
            label.set_text(&gettext("Couldn't transcribe"));
        }
        if let Some(area) = imp.waveform_area.borrow().as_ref() {
            area.set_visible(false);
        }
        if let Some(preview) = imp.preview_label.borrow().as_ref() {
            preview.set_text(message);
            preview.set_visible(true);
        }
        if let Some(copied_box) = imp.copied_box.borrow().as_ref() {
            copied_box.set_visible(false);
        }
        if let Some(btn) = imp.stop_btn.borrow().as_ref() {
            btn.set_sensitive(true);
        }
        // On error offer retry (New) + Close; nothing to copy/paste.
        if let Some(b) = imp.stop_btn.borrow().as_ref() { b.set_visible(false); }
        if let Some(b) = imp.cancel_btn.borrow().as_ref() { b.set_visible(false); }
        if let Some(b) = imp.again_btn.borrow().as_ref() { b.set_visible(true); }
        if let Some(b) = imp.paste_btn.borrow().as_ref() { b.set_visible(false); }
        if let Some(b) = imp.copy_btn.borrow().as_ref() { b.set_visible(false); }
        if let Some(b) = imp.close_btn.borrow().as_ref() { b.set_visible(true); }
    }

    /// Toggle between the recording button set (Stop/Cancel) and the result set
    /// (Copy/Close). The Paste button is hidden by default: the transcript is
    /// always on the clipboard, and automatic typing (Remote Desktop) only runs
    /// when the user enables it — in which case the panel is hidden instead of
    /// showing this result view.
    fn set_recording_buttons(&self, recording: bool) {
        let imp = self.imp();
        if let Some(b) = imp.stop_btn.borrow().as_ref() { b.set_visible(recording); }
        if let Some(b) = imp.cancel_btn.borrow().as_ref() { b.set_visible(recording); }
        if let Some(b) = imp.again_btn.borrow().as_ref() { b.set_visible(!recording); }
        if let Some(b) = imp.paste_btn.borrow().as_ref() { b.set_visible(false); }
        if let Some(b) = imp.copy_btn.borrow().as_ref() { b.set_visible(!recording); }
        if let Some(b) = imp.close_btn.borrow().as_ref() { b.set_visible(!recording); }
    }

    /// Update the timer (seconds).
    pub fn set_timer(&self, seconds: u64) {
        if let Some(label) = self.imp().timer_label.borrow().as_ref() {
            let mins = seconds / 60;
            let secs = seconds % 60;
            label.set_text(&format!("{:02}:{:02}", mins, secs));
        }
    }

    /// Update the live waveform and redraw.
    pub fn update_waveform(&self, amplitudes: Vec<f32>) {
        *self.imp().waveform_data.borrow_mut() = amplitudes;
        if let Some(area) = self.imp().waveform_area.borrow().as_ref() {
            area.queue_draw();
        }
    }
}
