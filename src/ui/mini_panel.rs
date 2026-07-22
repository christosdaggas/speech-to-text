// Speech to Text - Mini Panel
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Compact floating dictation panel (the "instrument" redesign).
//!
//! A second, independent `adw::ApplicationWindow` used by the global dictation
//! shortcut. A slim Adwaita headerbar (state + minimize/close) sits on top; the
//! body is a `gtk::Stack` of three same-height pages:
//!   • Recording   — big tabular timer (with centiseconds), a colourful live
//!                   waveform, an LED level meter, Stop/Cancel.
//!   • Transcribing — "Decoding…" + elapsed, an indeterminate accent-coloured
//!                   segmented sweep, Working…/Cancel.
//!   • Result      — the transcript (3 lines, ellipsized), a "Copied" badge,
//!                   New/Copy/Paste.
//!
//! Colours follow the GNOME theme: the segments/spinner use `@accent_bg_color`,
//! the buttons use `.suggested-action` / `.destructive-action` / `@success`.
//!
//! Note on GNOME/Wayland: a normal client cannot force always-on-top, position
//! itself near the cursor, or use layer-shell (Mutter implements none of these),
//! so this is a compact window the compositor places.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk4 as gtk;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::{Cell, RefCell};

use crate::application::Application;
use crate::i18n::gettext;

/// Actions emitted by the mini panel's buttons / close request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MiniPanelAction {
    Stop,
    Cancel,
    Paste,
    Copy,
    Again,
    Close,
    /// A transform chip was clicked (carries the preset index).
    Chip(usize),
    /// The raw/polished selector changed (carries the variant index, 0 = raw).
    Variant(usize),
    /// The "Voice edit" button was clicked.
    VoiceEdit,
}

/// Visual state of the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum PanelState {
    #[default]
    Recording,
    Transcribing,
    Result,
}

/// Number of LED cells in the recording level meter.
const N_LEDS: usize = 7;
/// Number of cells in the decode (transcribing) segmented bar.
const N_SEGS: usize = 22;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct MiniPanel {
        pub(super) state: Cell<PanelState>,
        /// Best-effort "keep above" (no-op on GNOME/Mutter Wayland).
        pub(super) keep_on_top: Cell<bool>,
        pub action_callback: RefCell<Option<Box<dyn Fn(MiniPanelAction)>>>,

        // Header
        pub hdr_dot: RefCell<Option<gtk::Box>>,
        pub hdr_spinner: RefCell<Option<gtk::Spinner>>,
        pub hdr_label: RefCell<Option<gtk::Label>>,
        /// LLM-connection indicator shown in the recording body (under "Auto"),
        /// visible only when the LLM integration is enabled.
        pub llm_indicator: RefCell<Option<gtk::Image>>,

        // Stack of the three pages
        pub stack: RefCell<Option<gtk::Stack>>,

        // Recording widgets
        pub rec_meta_r: RefCell<Option<gtk::Label>>,
        pub timer_label: RefCell<Option<gtk::Label>>,
        pub cs_label: RefCell<Option<gtk::Label>>,
        pub waveform_area: RefCell<Option<gtk::DrawingArea>>,
        pub waveform_data: RefCell<Vec<f32>>,
        pub leds: RefCell<Vec<gtk::Box>>,

        // Transcribing widgets
        pub tr_meta_l: RefCell<Option<gtk::Label>>,
        pub tr_meta_r: RefCell<Option<gtk::Label>>,
        pub tr_elapsed: RefCell<Option<gtk::Label>>,
        pub tr_partial: RefCell<Option<gtk::Label>>,
        pub seg_cells: RefCell<Vec<gtk::Box>>,
        pub seg_pos: Cell<usize>,
        pub tr_ticks: Cell<u32>,
        pub decoding: Cell<bool>,

        // Result widgets
        pub transcript_label: RefCell<Option<gtk::Label>>,
        pub copied_badge: RefCell<Option<gtk::Label>>,
        pub result_stats: RefCell<Option<gtk::Label>>,
        pub actions_btn: RefCell<Option<gtk::MenuButton>>,
        pub actions_list: RefCell<Option<gtk::Box>>,
        pub chip_buttons: RefCell<Vec<gtk::Button>>,
        pub variant_dropdown: RefCell<Option<gtk::DropDown>>,
        pub variant_syncing: Cell<bool>,
        pub voice_edit_btn: RefCell<Option<gtk::Button>>,
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
    const WIDTH: i32 = 452;

    pub fn new(app: &Application) -> Self {
        let panel: Self = glib::Object::builder().property("application", app).build();
        panel.set_default_size(Self::WIDTH, -1);
        panel.set_resizable(false);
        panel.set_title(Some(&gettext("Dictation")));
        panel.set_hide_on_close(true);
        panel
    }

    fn setup_ui(&self) {
        let imp = self.imp();
        self.add_css_class("mini-panel");

        let toolbar = adw::ToolbarView::new();

        // ── Header bar: state (start) + minimize/close (end) ───────────────
        let header = adw::HeaderBar::new();
        // (No .flat — keep the default headerbar background so the top bar is
        // visually distinct from the body.)
        header.set_decoration_layout(Some(":minimize,close"));
        header.set_title_widget(Some(&gtk::Label::new(None))); // clear centre title

        let state_box = gtk::Box::new(gtk::Orientation::Horizontal, 9);
        let hdr_dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        hdr_dot.add_css_class("mp-dot");
        hdr_dot.add_css_class("rec");
        hdr_dot.set_size_request(10, 10);
        hdr_dot.set_valign(gtk::Align::Center); // keep it a round dot, not stretched vertically
        hdr_dot.set_halign(gtk::Align::Center);
        state_box.append(&hdr_dot);
        let hdr_spinner = gtk::Spinner::new();
        hdr_spinner.set_visible(false);
        state_box.append(&hdr_spinner);
        let hdr_label = gtk::Label::new(Some(&gettext("Recording")));
        hdr_label.add_css_class("mp-state");
        state_box.append(&hdr_label);
        header.pack_start(&state_box);

        toolbar.add_top_bar(&header);

        // ── Body: a Stack of three equal-height pages ──────────────────────
        let stack = gtk::Stack::new();
        stack.set_hhomogeneous(true); // constant width
        stack.set_vhomogeneous(true); // constant height across states (content is centered, buttons pinned bottom)

        stack.add_named(&self.build_recording_page(), Some("recording"));
        stack.add_named(&self.build_transcribing_page(), Some("transcribing"));
        stack.add_named(&self.build_result_page(), Some("result"));

        toolbar.set_content(Some(&stack));
        self.set_content(Some(&toolbar));

        *imp.hdr_dot.borrow_mut() = Some(hdr_dot);
        *imp.hdr_spinner.borrow_mut() = Some(hdr_spinner);
        *imp.hdr_label.borrow_mut() = Some(hdr_label);
        *imp.stack.borrow_mut() = Some(stack);

        // Close (titlebar X) → Cancel while recording, else Close.
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

        // Best-effort keep-on-top: re-raise when it loses focus while visible.
        self.connect_is_active_notify(move |panel| {
            if !panel.imp().keep_on_top.get() || panel.is_active() || !panel.is_visible() {
                return;
            }
            panel.present();
        });
    }

    /// Common page scaffold: a vertical box with standard margins.
    fn new_page() -> gtk::Box {
        let page = gtk::Box::new(gtk::Orientation::Vertical, 10);
        page.set_margin_start(16);
        page.set_margin_end(16);
        page.set_margin_top(10);
        page.set_margin_bottom(12);
        page
    }

    /// A dim "meta" row with a left and right label (left grows).
    fn meta_row(left: &str, right: &str) -> (gtk::Box, gtk::Label, gtk::Label) {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let l = gtk::Label::new(Some(left));
        l.add_css_class("mp-meta");
        l.set_xalign(0.0);
        l.set_hexpand(true);
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let r = gtk::Label::new(Some(right));
        r.add_css_class("mp-meta");
        r.set_xalign(1.0);
        row.append(&l);
        row.append(&r);
        (row, l, r)
    }

    /// Centred content area that fills the (equal) page height; its natural-size
    /// content is vertically centred, so short states don't leave a top gap.
    fn page_body() -> gtk::Box {
        let b = gtk::Box::new(gtk::Orientation::Vertical, 10);
        b.set_vexpand(true);
        b.set_valign(gtk::Align::Center);
        b
    }

    /// Bottom button row (equal-width buttons).
    fn actions_row() -> gtk::Box {
        let a = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        a.set_homogeneous(true);
        a.set_margin_top(4);
        a
    }

    fn build_recording_page(&self) -> gtk::Box {
        let imp = self.imp();
        let page = Self::new_page();
        let body = Self::page_body();

        // meta: format (left) + language (right)
        let (meta, _ml, mr) = Self::meta_row("16 kHz · Mono", "Auto");
        body.append(&meta);
        *imp.rec_meta_r.borrow_mut() = Some(mr);

        // LLM-connection indicator, right-aligned directly under the language
        // ("Auto"). Only shown when the LLM integration is enabled; the whole row
        // is hidden otherwise so it leaves no gap.
        let ai_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        ai_row.set_halign(gtk::Align::End);
        ai_row.set_visible(false);
        let llm_indicator = gtk::Image::from_icon_name("com.chrisdaggas.speech-to-text-ai");
        llm_indicator.set_pixel_size(18);
        llm_indicator.set_tooltip_text(Some(gettext("LLM connection is enabled").as_str()));
        ai_row.append(&llm_indicator);
        body.append(&ai_row);
        *imp.llm_indicator.borrow_mut() = Some(llm_indicator);

        // Big timer + centiseconds (kept on the same baseline).
        let timer_row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        timer_row.set_valign(gtk::Align::Baseline);
        let timer = gtk::Label::new(Some("00:00"));
        timer.add_css_class("mp-timer");
        timer.set_xalign(0.0);
        let cs = gtk::Label::new(Some(".00"));
        cs.add_css_class("mp-cs");
        cs.set_valign(gtk::Align::Baseline);
        timer_row.append(&timer);
        timer_row.append(&cs);
        body.append(&timer_row);
        *imp.timer_label.borrow_mut() = Some(timer);
        *imp.cs_label.borrow_mut() = Some(cs);

        // Colourful live waveform.
        let wave = gtk::DrawingArea::new();
        wave.set_height_request(48);
        wave.set_hexpand(true);
        wave.add_css_class("mp-wave");
        let panel_weak = self.downgrade();
        wave.set_draw_func(move |_area, cr, width, height| {
            let Some(panel) = panel_weak.upgrade() else {
                return;
            };
            draw_waveform(&panel, cr, width, height);
        });
        body.append(&wave);
        *imp.waveform_area.borrow_mut() = Some(wave);

        // LED level meter + label.
        let lvl_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let leds_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        leds_box.set_valign(gtk::Align::Center);
        let mut leds = Vec::with_capacity(N_LEDS);
        for _ in 0..N_LEDS {
            let led = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            led.add_css_class("mp-led");
            led.set_valign(gtk::Align::Center);
            led.set_size_request(9, 9);
            leds_box.append(&led);
            leds.push(led);
        }
        let lvl_label = gtk::Label::new(Some(&gettext("Level")));
        lvl_label.add_css_class("mp-meta");
        lvl_row.append(&leds_box);
        lvl_row.append(&lvl_label);
        body.append(&lvl_row);
        *imp.leds.borrow_mut() = leds;

        page.append(&body);

        // Actions: Stop (destructive) + Cancel.
        let actions = Self::actions_row();
        let stop = Self::icon_button("media-playback-stop-symbolic", &gettext("Stop"));
        stop.add_css_class("destructive-action");
        let cancel = Self::icon_button("window-close-symbolic", &gettext("Cancel"));
        actions.append(&stop);
        actions.append(&cancel);
        page.append(&actions);
        self.wire_button(&stop, MiniPanelAction::Stop);
        self.wire_button(&cancel, MiniPanelAction::Cancel);

        page
    }

    fn build_transcribing_page(&self) -> gtk::Box {
        let imp = self.imp();
        let page = Self::new_page();
        let body = Self::page_body();

        let (meta, ml, mr) = Self::meta_row("Decoding", "Auto");
        body.append(&meta);
        *imp.tr_meta_l.borrow_mut() = Some(ml);
        *imp.tr_meta_r.borrow_mut() = Some(mr);

        // Title + elapsed.
        let title_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        title_row.set_valign(gtk::Align::Baseline);
        let title = gtk::Label::new(Some(&gettext("Decoding…")));
        title.add_css_class("mp-title");
        title.set_xalign(0.0);
        title.set_hexpand(true);
        let elapsed = gtk::Label::new(Some("0.0 s"));
        elapsed.add_css_class("mp-meta");
        elapsed.set_xalign(1.0);
        title_row.append(&title);
        title_row.append(&elapsed);
        body.append(&title_row);
        *imp.tr_elapsed.borrow_mut() = Some(elapsed);

        // Indeterminate segmented sweep.
        let seg = gtk::Box::new(gtk::Orientation::Horizontal, 3);
        seg.set_hexpand(true);
        let mut cells = Vec::with_capacity(N_SEGS);
        for _ in 0..N_SEGS {
            let c = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            c.add_css_class("mp-seg");
            c.set_hexpand(true);
            seg.append(&c);
            cells.push(c);
        }
        body.append(&seg);
        *imp.seg_cells.borrow_mut() = cells;

        // A quiet status line to balance the layout.
        let (status, _sl, _sr) = Self::meta_row(&gettext("Transcribing your dictation…"), "");
        body.append(&status);

        // Live preview text (filled while decoding when live transcription is on).
        let partial = gtk::Label::new(None);
        partial.add_css_class("mp-meta");
        partial.set_wrap(true);
        partial.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        partial.set_lines(2);
        partial.set_ellipsize(gtk::pango::EllipsizeMode::End);
        partial.set_xalign(0.0);
        partial.set_visible(false);
        body.append(&partial);
        *imp.tr_partial.borrow_mut() = Some(partial);

        page.append(&body);

        // Actions: Working… (disabled) + Cancel.
        let actions = Self::actions_row();
        let working = Self::icon_button("content-loading-symbolic", &gettext("Working…"));
        working.set_sensitive(false);
        let cancel = Self::icon_button("window-close-symbolic", &gettext("Cancel"));
        actions.append(&working);
        actions.append(&cancel);
        page.append(&actions);
        self.wire_button(&cancel, MiniPanelAction::Cancel);

        page
    }

    fn build_result_page(&self) -> gtk::Box {
        let imp = self.imp();
        let page = Self::new_page();
        let body = Self::page_body();

        // Transcript (up to 3 lines, ellipsized) inside a bordered card so the
        // result page doesn't look empty.
        let transcript = gtk::Label::new(None);
        transcript.add_css_class("mp-transcript");
        transcript.set_wrap(true);
        transcript.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        transcript.set_lines(3);
        transcript.set_ellipsize(gtk::pango::EllipsizeMode::End);
        transcript.set_xalign(0.0);
        transcript.set_yalign(0.0);
        transcript.set_justify(gtk::Justification::Left);
        transcript.set_selectable(true);
        transcript.set_hexpand(true);
        transcript.set_vexpand(true);

        let transcript_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        transcript_box.add_css_class("mp-transcript-box");
        transcript_box.set_hexpand(true);
        transcript_box.append(&transcript);
        body.append(&transcript_box);
        *imp.transcript_label.borrow_mut() = Some(transcript);

        // Transform actions collapsed into one "Actions" dropdown (shown only when
        // the LLM is enabled), grouped with Voice edit on the right — mirrors the
        // main window. The raw/polished selector sits on the left.
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);

        let variant_dropdown = gtk::DropDown::from_strings(&[]);
        variant_dropdown.set_valign(gtk::Align::Center);
        variant_dropdown.set_visible(false);
        controls.append(&variant_dropdown);

        let controls_spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        controls_spacer.set_hexpand(true);
        controls.append(&controls_spacer);

        // Actions dropdown (items rebuilt by `set_chip_presets`).
        let actions_btn = gtk::MenuButton::new();
        actions_btn.add_css_class("pill");
        actions_btn.add_css_class("transform-action");
        actions_btn.set_valign(gtk::Align::Center);
        actions_btn.set_visible(false);
        actions_btn.set_tooltip_text(Some(&gettext("Transform this text with AI")));
        let actions_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let actions_icon = gtk::Image::from_icon_name("com.chrisdaggas.speech-to-text-ai");
        actions_icon.set_pixel_size(15);
        let actions_label = gtk::Label::new(Some(&gettext("Actions")));
        let actions_caret = gtk::Image::from_icon_name("pan-down-symbolic");
        actions_caret.set_pixel_size(12);
        actions_content.append(&actions_icon);
        actions_content.append(&actions_label);
        actions_content.append(&actions_caret);
        actions_btn.set_child(Some(&actions_content));
        let actions_popover = gtk::Popover::new();
        actions_popover.add_css_class("menu");
        actions_popover.set_has_arrow(false);
        actions_popover.set_position(gtk::PositionType::Top);
        let actions_list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        actions_popover.set_child(Some(&actions_list));
        actions_btn.set_popover(Some(&actions_popover));
        controls.append(&actions_btn);
        *imp.actions_btn.borrow_mut() = Some(actions_btn);
        *imp.actions_list.borrow_mut() = Some(actions_list);

        // Voice edit (labelled, accent) — speak an instruction to change the text.
        let voice_edit_btn = gtk::Button::new();
        let ve_content = adw::ButtonContent::new();
        ve_content.set_icon_name("document-edit-symbolic");
        ve_content.set_label(&gettext("Voice edit"));
        voice_edit_btn.set_child(Some(&ve_content));
        voice_edit_btn.add_css_class("pill");
        voice_edit_btn.add_css_class("suggested-action");
        voice_edit_btn.add_css_class("transform-action");
        voice_edit_btn.set_valign(gtk::Align::Center);
        voice_edit_btn.set_visible(false);
        voice_edit_btn.set_tooltip_text(Some(&gettext(
            "Voice edit: speak an instruction to change this text",
        )));
        controls.append(&voice_edit_btn);
        *imp.voice_edit_btn.borrow_mut() = Some(voice_edit_btn.clone());
        self.wire_button(&voice_edit_btn, MiniPanelAction::VoiceEdit);

        body.append(&controls);

        // Meta row: stats (left, grows) + "Copied ✓" badge (right).
        let meta = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let stats = gtk::Label::new(None);
        stats.add_css_class("mp-meta");
        stats.set_xalign(0.0);
        stats.set_hexpand(true);
        meta.append(&stats);
        let badge = gtk::Label::new(Some(&gettext("Copied ✓")));
        badge.add_css_class("mp-badge");
        badge.set_visible(false);
        meta.append(&badge);
        body.append(&meta);
        *imp.copied_badge.borrow_mut() = Some(badge);
        *imp.result_stats.borrow_mut() = Some(stats);
        *imp.variant_dropdown.borrow_mut() = Some(variant_dropdown.clone());

        let panel_weak = self.downgrade();
        variant_dropdown.connect_selected_notify(move |dd| {
            let Some(panel) = panel_weak.upgrade() else {
                return;
            };
            if panel.imp().variant_syncing.get() {
                return;
            }
            panel.emit_action(MiniPanelAction::Variant(dd.selected() as usize));
        });

        page.append(&body);

        // Actions: New (suggested) · Copy (success) · Paste (destructive).
        let actions = Self::actions_row();
        let new_btn = Self::icon_button("list-add-symbolic", &gettext("New"));
        new_btn.add_css_class("suggested-action");
        let copy_btn = Self::icon_button("edit-copy-symbolic", &gettext("Copy"));
        copy_btn.add_css_class("mp-copy");
        let paste_btn = Self::icon_button("edit-paste-symbolic", &gettext("Paste"));
        paste_btn.add_css_class("destructive-action");
        actions.append(&new_btn);
        actions.append(&copy_btn);
        actions.append(&paste_btn);
        page.append(&actions);
        self.wire_button(&new_btn, MiniPanelAction::Again);
        self.wire_button(&copy_btn, MiniPanelAction::Copy);
        self.wire_button(&paste_btn, MiniPanelAction::Paste);

        page
    }

    /// Build a flat-ish button with a symbolic icon before the label.
    fn icon_button(icon: &str, label: &str) -> gtk::Button {
        let b = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        b.set_halign(gtk::Align::Center);
        let img = gtk::Image::from_icon_name(icon);
        img.set_pixel_size(15);
        let lbl = gtk::Label::new(Some(label));
        b.append(&img);
        b.append(&lbl);
        let btn = gtk::Button::new();
        btn.set_child(Some(&b));
        btn.add_css_class("mp-btn");
        btn
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

    pub fn connect_action<F: Fn(MiniPanelAction) + 'static>(&self, f: F) {
        *self.imp().action_callback.borrow_mut() = Some(Box::new(f));
    }

    /// Enable/disable the best-effort keep-on-top behavior.
    pub fn set_keep_on_top(&self, enabled: bool) {
        self.imp().keep_on_top.set(enabled);
    }

    /// Show/hide the LLM-connection indicator (and its row) in the recording body.
    pub fn set_llm_active(&self, active: bool) {
        if let Some(img) = self.imp().llm_indicator.borrow().as_ref() {
            img.set_visible(active);
            if let Some(row) = img.parent() {
                row.set_visible(active);
            }
        }
    }

    /// The text currently shown in the result transcript (full, not ellipsized).
    pub fn transcript_text(&self) -> String {
        self.imp()
            .transcript_label
            .borrow()
            .as_ref()
            .map(|l| l.text().to_string())
            .unwrap_or_default()
    }

    /// Reveal the "Copied ✓" badge (feedback for the Copy button).
    pub fn show_copied_badge(&self) {
        if let Some(b) = self.imp().copied_badge.borrow().as_ref() {
            b.set_visible(true);
        }
    }

    /// Update the result stats line ("128 words · 96 wpm"). 0 words clears it.
    pub fn set_result_stats(&self, words: usize, wpm: Option<u32>) {
        if let Some(l) = self.imp().result_stats.borrow().as_ref() {
            if words == 0 {
                l.set_text("");
                return;
            }
            let word_label = if words == 1 {
                gettext("word")
            } else {
                gettext("words")
            };
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
        let Some(list) = imp.actions_list.borrow().clone() else {
            return;
        };
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
            let panel_weak = self.downgrade();
            let pop_weak = popover.as_ref().map(|p| p.downgrade());
            btn.connect_clicked(move |_| {
                if let Some(p) = pop_weak.as_ref().and_then(|p| p.upgrade()) {
                    p.popdown();
                }
                if let Some(p) = panel_weak.upgrade() {
                    p.emit_action(MiniPanelAction::Chip(i));
                }
            });
            list.append(&btn);
            buttons.push(btn);
        }
        *imp.chip_buttons.borrow_mut() = buttons;
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

    /// Show/hide the Actions dropdown (only when the LLM integration is enabled).
    pub fn set_chips_visible(&self, on: bool) {
        if let Some(b) = self.imp().actions_btn.borrow().as_ref() {
            b.set_visible(on);
        }
    }

    /// Rebuild the raw/polished variant selector (hidden with one entry only).
    pub fn set_variant_selector(&self, labels: &[String], active: usize) {
        let imp = self.imp();
        let Some(dd) = imp.variant_dropdown.borrow().clone() else {
            return;
        };
        imp.variant_syncing.set(true);
        let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
        dd.set_model(Some(&gtk::StringList::new(&refs)));
        dd.set_selected(active as u32);
        dd.set_visible(labels.len() > 1);
        imp.variant_syncing.set(false);
    }

    /// Show/hide the Voice-edit button (only when the LLM is enabled).
    pub fn set_voice_edit_visible(&self, on: bool) {
        if let Some(b) = self.imp().voice_edit_btn.borrow().as_ref() {
            b.set_visible(on);
        }
    }

    /// Hide the chips/selector and clear the stats line (used by non-result states).
    fn reset_result_extras(&self) {
        self.set_chips_visible(false);
        if let Some(dd) = self.imp().variant_dropdown.borrow().as_ref() {
            dd.set_visible(false);
        }
        self.set_voice_edit_visible(false);
        self.set_result_stats(0, None);
    }

    // ── State transitions ──────────────────────────────────────────────────

    fn set_header(&self, label: &str, dot_class: Option<&str>, spinning: bool) {
        let imp = self.imp();
        if let Some(l) = imp.hdr_label.borrow().as_ref() {
            l.set_text(label);
        }
        if let Some(dot) = imp.hdr_dot.borrow().as_ref() {
            dot.remove_css_class("rec");
            dot.remove_css_class("ok");
            match dot_class {
                Some(c) => {
                    dot.add_css_class(c);
                    dot.set_visible(true);
                }
                None => dot.set_visible(false),
            }
        }
        if let Some(sp) = imp.hdr_spinner.borrow().as_ref() {
            sp.set_visible(spinning);
            if spinning {
                sp.start();
            } else {
                sp.stop();
            }
        }
    }

    /// Enter the recording state. `lang_label` is shown in the meta (e.g. "el" / "Auto").
    pub fn show_recording(&self, lang_label: &str) {
        let imp = self.imp();
        self.stop_decode_anim();
        self.reset_result_extras();
        imp.state.set(PanelState::Recording);
        imp.waveform_data.borrow_mut().clear();
        self.set_header(&gettext("Recording"), Some("rec"), false);
        if let Some(r) = imp.rec_meta_r.borrow().as_ref() {
            r.set_text(lang_label);
        }
        if let Some(t) = imp.timer_label.borrow().as_ref() {
            t.set_text("00:00");
        }
        if let Some(c) = imp.cs_label.borrow().as_ref() {
            c.set_text(".00");
        }
        self.set_level(0.0);
        if let Some(a) = imp.waveform_area.borrow().as_ref() {
            a.queue_draw();
        }
        if let Some(s) = imp.stack.borrow().as_ref() {
            s.set_visible_child_name("recording");
        }
    }

    /// Enter the transcribing state with an indeterminate decode sweep.
    pub fn show_transcribing(&self, model_label: &str, lang_label: &str) {
        let imp = self.imp();
        imp.state.set(PanelState::Transcribing);
        self.set_header(&gettext("Transcribing"), None, true);
        if let Some(l) = imp.tr_meta_l.borrow().as_ref() {
            l.set_text(&format!("{} · {}", gettext("Decode"), model_label));
        }
        if let Some(r) = imp.tr_meta_r.borrow().as_ref() {
            r.set_text(lang_label);
        }
        if let Some(e) = imp.tr_elapsed.borrow().as_ref() {
            e.set_text("0.0 s");
        }
        self.set_partial_text("");
        imp.seg_pos.set(0);
        imp.tr_ticks.set(0);
        if let Some(s) = imp.stack.borrow().as_ref() {
            s.set_visible_child_name("transcribing");
        }
        self.start_decode_anim();
    }

    /// Enter the "improving with AI" state (reuses the decode-sweep visual).
    pub fn show_improving(&self) {
        let imp = self.imp();
        imp.state.set(PanelState::Transcribing);
        self.set_header(&gettext("Improving with AI"), None, true);
        if let Some(l) = imp.tr_meta_l.borrow().as_ref() {
            l.set_text(&gettext("Enhancing transcript…"));
        }
        if let Some(r) = imp.tr_meta_r.borrow().as_ref() {
            r.set_text("");
        }
        if let Some(e) = imp.tr_elapsed.borrow().as_ref() {
            e.set_text("0.0 s");
        }
        imp.seg_pos.set(0);
        imp.tr_ticks.set(0);
        if let Some(s) = imp.stack.borrow().as_ref() {
            s.set_visible_child_name("transcribing");
        }
        self.start_decode_anim();
    }

    /// Show the transcription result. `copied` toggles the "Copied" badge.
    pub fn show_result(&self, text: &str, copied: bool) {
        let imp = self.imp();
        self.stop_decode_anim();
        imp.state.set(PanelState::Result);
        self.set_header(&gettext("Transcript ready"), Some("ok"), false);
        if let Some(t) = imp.transcript_label.borrow().as_ref() {
            t.set_text(text);
        }
        if let Some(b) = imp.copied_badge.borrow().as_ref() {
            b.set_visible(copied);
        }
        if let Some(s) = imp.stack.borrow().as_ref() {
            s.set_visible_child_name("result");
        }
    }

    /// Show an error message in place of a result.
    pub fn show_error(&self, message: &str) {
        // Strip any secret/home-path that leaked into the message before display.
        let message = crate::error::redact_secrets(message);
        let message = message.as_str();
        let imp = self.imp();
        self.stop_decode_anim();
        self.reset_result_extras();
        imp.state.set(PanelState::Result);
        self.set_header(&gettext("Couldn't transcribe"), Some("rec"), false);
        if let Some(t) = imp.transcript_label.borrow().as_ref() {
            t.set_text(message);
        }
        if let Some(b) = imp.copied_badge.borrow().as_ref() {
            b.set_visible(false);
        }
        if let Some(s) = imp.stack.borrow().as_ref() {
            s.set_visible_child_name("result");
        }
    }

    /// Update the timer (fractional seconds) — formats "MM:SS" + ".cc".
    pub fn set_timer(&self, secs: f64) {
        let imp = self.imp();
        let secs = secs.max(0.0);
        let mins = (secs as u64) / 60;
        let s = (secs as u64) % 60;
        let cs = ((secs.fract()) * 100.0) as u64;
        if let Some(t) = imp.timer_label.borrow().as_ref() {
            t.set_text(&format!("{:02}:{:02}", mins, s));
        }
        if let Some(c) = imp.cs_label.borrow().as_ref() {
            c.set_text(&format!(".{:02}", cs.min(99)));
        }
    }

    /// Update the live waveform + LED level meter from new amplitudes.
    pub fn update_waveform(&self, amplitudes: Vec<f32>) {
        let peak = amplitudes.iter().fold(0.0f32, |m, &a| m.max(a.abs()));
        *self.imp().waveform_data.borrow_mut() = amplitudes;
        if let Some(area) = self.imp().waveform_area.borrow().as_ref() {
            area.queue_draw();
        }
        self.set_level(peak);
    }

    /// Light the LED meter according to `peak` (0.0 – 1.0-ish).
    fn set_level(&self, peak: f32) {
        let lit = ((peak * 6.0).min(1.0) * N_LEDS as f32).round() as usize;
        let leds = self.imp().leds.borrow();
        for (i, led) in leds.iter().enumerate() {
            led.remove_css_class("on");
            led.remove_css_class("g");
            led.remove_css_class("y");
            led.remove_css_class("r");
            if i < lit {
                led.add_css_class("on");
                let cls = if i >= N_LEDS - 1 {
                    "r"
                } else if i >= N_LEDS - 3 {
                    "y"
                } else {
                    "g"
                };
                led.add_css_class(cls);
            }
        }
    }

    // ── Decode (transcribing) indeterminate animation ──────────────────────

    fn start_decode_anim(&self) {
        let imp = self.imp();
        if imp.decoding.get() {
            return;
        }
        imp.decoding.set(true);
        let panel_weak = self.downgrade();
        glib::timeout_add_local(std::time::Duration::from_millis(90), move || {
            let Some(panel) = panel_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let imp = panel.imp();
            if !imp.decoding.get() || imp.state.get() != PanelState::Transcribing {
                return glib::ControlFlow::Break;
            }
            // Moving lit window of width ~6 over the segment cells.
            let pos = imp.seg_pos.get();
            let win = 6usize;
            let cells = imp.seg_cells.borrow();
            for (i, c) in cells.iter().enumerate() {
                let lit = (i + N_SEGS - (pos % N_SEGS)) % N_SEGS < win;
                if lit {
                    c.add_css_class("on");
                } else {
                    c.remove_css_class("on");
                }
            }
            imp.seg_pos.set(pos + 1);
            // Elapsed time (tick = 90ms).
            let ticks = imp.tr_ticks.get() + 1;
            imp.tr_ticks.set(ticks);
            if ticks % 4 == 0 {
                if let Some(e) = imp.tr_elapsed.borrow().as_ref() {
                    e.set_text(&format!("{:.1} s", ticks as f64 * 0.09));
                }
            }
            glib::ControlFlow::Continue
        });
    }

    fn stop_decode_anim(&self) {
        self.imp().decoding.set(false);
    }

    /// Show live preview text on the transcribing page (live transcription only).
    pub fn set_partial_text(&self, text: &str) {
        if let Some(l) = self.imp().tr_partial.borrow().as_ref() {
            l.set_visible(!text.is_empty());
            l.set_text(text);
        }
    }

    /// Convert the indeterminate sweep into a determinate fill (0–100).
    pub fn set_decode_progress(&self, pct: i32) {
        let imp = self.imp();
        imp.decoding.set(false); // stop the sweep loop
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
}

/// Draw the colourful live waveform (mirrored bars, per-bar hue spectrum).
fn draw_waveform(panel: &MiniPanel, cr: &gtk::cairo::Context, width: i32, height: i32) {
    let data = panel.imp().waveform_data.borrow();
    let w = width as f64;
    let h = height as f64;
    let mid = h / 2.0;
    let n_bars = 56usize;
    let bar_w = (w / n_bars as f64).max(1.0);

    if data.is_empty() {
        // Flat idle line in the first spectrum colour.
        let (r, g, b) = hsl_to_rgb(200.0, 0.7, 0.6);
        cr.set_source_rgba(r, g, b, 0.6);
        cr.set_line_width(1.5);
        cr.move_to(0.0, mid);
        cr.line_to(w, mid);
        let _ = cr.stroke();
        return;
    }

    for i in 0..n_bars {
        let idx = i * data.len() / n_bars;
        let amp = (data.get(idx).copied().unwrap_or(0.0).abs() * 5.0).min(1.0) as f64;
        let bar_h = (amp * (h - 4.0)).max(2.0);
        let x = i as f64 * bar_w + 1.0;
        let y = mid - bar_h / 2.0;
        let hue = 185.0 + (i as f64 / (n_bars - 1) as f64) * 150.0; // teal → magenta
        let (r, g, b) = hsl_to_rgb(hue, 0.85, 0.63);
        cr.set_source_rgb(r, g, b);
        cr.rectangle(x, y, (bar_w - 1.5).max(1.0), bar_h);
        let _ = cr.fill();
    }
}

/// Minimal HSL→RGB (h in degrees, s/l in 0..1) → (r,g,b) in 0..1.
fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (f64, f64, f64) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = (h % 360.0) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match hp as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    (r1 + m, g1 + m, b1 + m)
}
