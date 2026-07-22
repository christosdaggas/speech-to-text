// Speech to Text - Controls Panel
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Hero-centric record controls.
//!
//! A large circular record button (mic ⇢ red stop square when live) is the
//! centrepiece; secondary actions (Open File, Pause, Cancel, Copy, Clear,
//! Save) and the Translate / Improve-with-AI toggles sit in a calm row
//! beneath it. The public API is unchanged from the legacy row layout, so the
//! recording state machine in `MainWindow` keeps working untouched.

use gtk4::prelude::*;
use crate::i18n::gettext;
use gtk4::glib;
use gtk4 as gtk;
use libadwaita as adw;
use adw::subclass::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::config::AppConfig;

/// Signals emitted by the controls panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlAction {
    Record,
    Pause,
    Resume,
    Stop,
    Cancel,
    Copy,
    Clear,
    Save,
    OpenFile,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Controls {
        pub record_btn: RefCell<Option<gtk::Button>>,
        pub pause_btn: RefCell<Option<gtk::Button>>,
        // stop_btn is no longer rendered (the hero doubles as Record ⇢ Stop),
        // but the field is retained so any guarded `if let Some(..)` access in
        // the parent stays a no-op rather than a compile error.
        pub stop_btn: RefCell<Option<gtk::Button>>,
        pub cancel_btn: RefCell<Option<gtk::Button>>,
        pub open_file_btn: RefCell<Option<gtk::Button>>,
        pub copy_btn: RefCell<Option<gtk::Button>>,
        pub clear_btn: RefCell<Option<gtk::Button>>,
        pub save_btn: RefCell<Option<gtk::Button>>,
        pub translate_toggle: RefCell<Option<gtk::ToggleButton>>,
        pub ai_toggle: RefCell<Option<gtk::ToggleButton>>,
        pub hero_state: RefCell<Option<gtk::Box>>,
        pub hero_state_label: RefCell<Option<gtk::Label>>,
        pub hero_title: RefCell<Option<gtk::Label>>,
        pub hero_subtitle: RefCell<Option<gtk::Label>>,
        pub hero_hint: RefCell<Option<gtk::Label>>,
        pub hero_record_label: RefCell<Option<gtk::Label>>,
        pub rec_tools: RefCell<Option<gtk::Box>>,
        pub mode_buttons: RefCell<Vec<gtk::ToggleButton>>,
        /// Mirrors the live recording flag so the hero's click handler can
        /// dispatch Record vs Stop without a second source of truth.
        pub recording_state: Cell<bool>,
        /// Mirrors whether the current recording is paused so the single pause
        /// button can dispatch Pause and Resume correctly.
        pub paused_state: Cell<bool>,
        /// Action dispatcher populated by `connect_action`; the hero reads it
        /// lazily on click.
        pub action_cb: RefCell<Option<Rc<dyn Fn(ControlAction)>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Controls {
        const NAME: &'static str = "SttControls";
        type Type = super::Controls;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for Controls {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_ui();
        }
    }

    impl WidgetImpl for Controls {}
    impl BoxImpl for Controls {}
}

glib::wrapper! {
    pub struct Controls(ObjectSubclass<imp::Controls>)
        @extends gtk::Widget, gtk::Box;
}

impl Controls {
    pub fn new() -> Self {
        // Vertical stack: hero zone on top, secondary controls below.
        glib::Object::builder()
            .property("orientation", gtk::Orientation::Vertical)
            .property("spacing", 0)
            .build()
    }

    fn setup_ui(&self) {
        let imp = self.imp();

        self.add_css_class("controls-panel");

        // Capture stage: the record orb sits *beside* the text block (state
        // pill, title, subtitle, hint) rather than on top of it, so the hero
        // zone stays short and the whole window is less tall.
        let hero_zone = gtk::Box::new(gtk::Orientation::Vertical, 0);
        hero_zone.add_css_class("hero-zone");
        hero_zone.set_halign(gtk::Align::Fill);
        hero_zone.set_hexpand(true);

        // Horizontal group: orb on the left, text column on the right, centred
        // as a unit within the zone.
        let hero_row = gtk::Box::new(gtk::Orientation::Horizontal, 24);
        hero_row.add_css_class("hero-row");
        hero_row.set_halign(gtk::Align::Center);
        hero_row.set_valign(gtk::Align::Center);

        // Big circular record button. Click dispatch is wired in
        // `connect_action` via the shared `action_cb`; here we only attach the
        // lazy dispatcher that reads the current recording flag.
        let hero_btn = gtk::Button::new();
        hero_btn.add_css_class("hero-record");
        hero_btn.add_css_class("circular");
        hero_btn.add_css_class("suggested-action");
        hero_btn.set_tooltip_text(Some(gettext("Start recording (Ctrl+R)").as_str()));
        hero_btn.set_size_request(72, 72);
        hero_btn.set_halign(gtk::Align::Center);
        hero_btn.set_valign(gtk::Align::Center);
        hero_btn.set_hexpand(false);
        hero_btn.set_vexpand(false);

        let hero_icon = gtk::Image::from_icon_name("audio-input-microphone-symbolic");
        hero_icon.set_pixel_size(28);
        hero_btn.set_child(Some(&hero_icon));
        hero_row.append(&hero_btn);

        // Left-aligned text column next to the orb.
        let hero_text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        hero_text.add_css_class("hero-text");
        hero_text.set_halign(gtk::Align::Start);
        hero_text.set_valign(gtk::Align::Center);

        // State pill: "Ready" / "Recording"
        let hero_state = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        hero_state.add_css_class("hero-state");
        hero_state.add_css_class("ready");
        hero_state.set_halign(gtk::Align::Start);
        let hero_state_dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        hero_state_dot.add_css_class("hero-state-dot");
        hero_state_dot.set_size_request(7, 7);
        hero_state_dot.set_valign(gtk::Align::Center);
        let hero_state_label = gtk::Label::new(Some(&gettext("Ready")));
        hero_state_label.add_css_class("hero-state-label");
        hero_state.append(&hero_state_dot);
        hero_state.append(&hero_state_label);
        hero_text.append(&hero_state);

        // Title
        let hero_title = gtk::Label::new(Some(gettext("Start a new recording").as_str()));
        hero_title.add_css_class("hero-title");
        hero_title.set_halign(gtk::Align::Start);
        hero_title.set_justify(gtk::Justification::Left);
        hero_text.append(&hero_title);

        // Subtitle
        let hero_subtitle = gtk::Label::new(Some(&gettext(
            "Capture a thought, draft a message, or transcribe an audio file with local speech recognition.",
        )));
        hero_subtitle.add_css_class("hero-subtitle");
        hero_subtitle.set_wrap(true);
        hero_subtitle.set_justify(gtk::Justification::Left);
        hero_subtitle.set_halign(gtk::Align::Start);
        hero_subtitle.set_xalign(0.0);
        hero_subtitle.set_max_width_chars(46);
        hero_text.append(&hero_subtitle);

        // Keyboard hint
        let hero_hint = gtk::Label::new(Some(
            gettext("Ctrl+R to start · Esc to cancel").as_str(),
        ));
        hero_hint.add_css_class("hero-hint");
        hero_hint.set_halign(gtk::Align::Start);
        hero_text.append(&hero_hint);

        // Pause / Cancel — visible only while recording.
        fn stage_tool(icon: &str, label: &str, tooltip: &str) -> gtk::Button {
            let btn = gtk::Button::new();
            let content = adw::ButtonContent::new();
            content.set_icon_name(icon);
            content.set_label(label);
            btn.set_child(Some(&content));
            btn.add_css_class("rec-tool");
            btn.set_tooltip_text(Some(tooltip));
            btn
        }

        // Kept always-visible (faded out while idle) so the hero zone's height
        // never changes when recording starts — otherwise the window resizes.
        let rec_tools = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        rec_tools.add_css_class("rec-tools");
        rec_tools.set_halign(gtk::Align::Start);
        rec_tools.set_opacity(0.0);
        rec_tools.set_can_target(false);

        let pause_btn = stage_tool(
            "media-playback-pause-symbolic",
            &gettext("Pause"),
            &gettext("Pause recording"),
        );
        pause_btn.set_sensitive(false);
        rec_tools.append(&pause_btn);

        let cancel_btn = stage_tool(
            "process-stop-symbolic",
            &gettext("Cancel"),
            &gettext("Cancel recording and discard"),
        );
        cancel_btn.add_css_class("destructive-action");
        cancel_btn.set_sensitive(false);
        rec_tools.append(&cancel_btn);
        hero_text.append(&rec_tools);

        // Lazy click dispatch: Record when idle, Stop when live.
        let self_weak = self.downgrade();
        hero_btn.connect_clicked(move |_| {
            let Some(view) = self_weak.upgrade() else { return };
            let imp = view.imp();
            let cb_opt = imp.action_cb.borrow().clone();
            if let Some(cb) = cb_opt {
                if imp.recording_state.get() {
                    cb(ControlAction::Stop);
                } else {
                    cb(ControlAction::Record);
                }
            }
        });

        hero_row.append(&hero_text);
        hero_zone.append(&hero_row);
        self.append(&hero_zone);

        // Copy / Clear / Save stay header-less utility widgets for the existing
        // callback API; the transcript-card footer dispatches the same actions.
        fn icon_btn(icon: &str, tooltip: &str) -> gtk::Button {
            let btn = gtk::Button::from_icon_name(icon);
            btn.set_tooltip_text(Some(tooltip));
            btn
        }

        // Built with the same icon+label content box as the Translate / Enhance
        // toggles below, so all three action buttons read as one uniform set.
        let open_file_btn = gtk::Button::new();
        let open_file_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        open_file_content.add_css_class("tool-toggle-content");
        let open_file_icon = gtk::Image::from_icon_name("document-open-symbolic");
        open_file_icon.set_pixel_size(14);
        open_file_content.append(&open_file_icon);
        open_file_content.append(&gtk::Label::new(Some(&gettext("Open file"))));
        open_file_btn.set_child(Some(&open_file_content));
        open_file_btn.add_css_class("hero-toggle");
        open_file_btn.add_css_class("open-file-btn");
        open_file_btn.set_tooltip_text(Some(
            gettext("Open an audio file (WAV, MP3, FLAC, OGG, Opus, M4A) and transcribe it").as_str(),
        ));

        let copy_btn = icon_btn("edit-copy-symbolic", &gettext("Copy transcript to clipboard"));
        let clear_btn = icon_btn("edit-clear-all-symbolic", &gettext("Clear transcript"));
        let save_btn = icon_btn("document-save-symbolic", &gettext("Save transcript to file"));

        // Mode row: mode switcher group + the action-button group, spaced apart.
        let mode_row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        mode_row.add_css_class("hero-mode-row");
        mode_row.set_halign(gtk::Align::Center);

        let mode_switcher = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        mode_switcher.add_css_class("mode-switcher");
        const MODES: [(&str, &str); 5] = [
            ("plain", "Plain"),
            ("message", "Message"),
            ("email", "Email"),
            ("note", "Note"),
            ("code_prompt", "Code"),
        ];
        let active_mode = AppConfig::load().dictation_mode;
        let mut mode_buttons = Vec::with_capacity(MODES.len());
        let mut first_mode: Option<gtk::ToggleButton> = None;
        for (id, label) in MODES {
            let button = gtk::ToggleButton::with_label(&gettext(label));
            button.add_css_class("mode-chip");
            if let Some(first) = first_mode.as_ref() {
                button.set_group(Some(first));
            } else {
                first_mode = Some(button.clone());
            }
            button.set_active(active_mode == id);
            button.connect_toggled(move |button| {
                if button.is_active() {
                    let mut config = AppConfig::load();
                    config.dictation_mode = id.to_string();
                    config.save();
                }
            });
            mode_switcher.append(&button);
            mode_buttons.push(button);
        }
        mode_row.append(&mode_switcher);

        // The three action buttons (Translate / Enhance / Open file) share one
        // row with uniform spacing so they read as a matching set.
        let actions_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions_box.add_css_class("hero-actions");

        fn tool_toggle(icon_name: &str, label: &str) -> gtk::ToggleButton {
            let toggle = gtk::ToggleButton::new();
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            content.add_css_class("tool-toggle-content");
            let icon = gtk::Image::from_icon_name(icon_name);
            icon.set_pixel_size(14);
            content.append(&icon);
            content.append(&gtk::Label::new(Some(label)));
            toggle.set_child(Some(&content));
            toggle
        }

        let translate_toggle = tool_toggle(
            "preferences-desktop-locale-symbolic",
            gettext("Translate").as_str(),
        );
        translate_toggle.set_tooltip_text(Some(gettext("Translate to English").as_str()));
        translate_toggle.add_css_class("translate-toggle");
        translate_toggle.add_css_class("hero-toggle");
        actions_box.append(&translate_toggle);

        // Improve with AI: when active, the NEXT transcriptions are auto-improved
        // with the active LLM preset. Hidden until the LLM integration is enabled.
        let ai_toggle = tool_toggle(
            "com.chrisdaggas.speech-to-text-ai",
            gettext("Enhance").as_str(),
        );
        ai_toggle.set_tooltip_text(Some(
            gettext("Improve the next transcriptions with the LLM (active preset)").as_str(),
        ));
        ai_toggle.add_css_class("ai-toggle");
        ai_toggle.add_css_class("hero-toggle");
        ai_toggle.set_visible(false);
        actions_box.append(&ai_toggle);

        actions_box.append(&open_file_btn);
        mode_row.append(&actions_box);

        self.append(&mode_row);

        // Store references
        *imp.record_btn.borrow_mut() = Some(hero_btn);
        *imp.pause_btn.borrow_mut() = Some(pause_btn);
        *imp.cancel_btn.borrow_mut() = Some(cancel_btn);
        *imp.open_file_btn.borrow_mut() = Some(open_file_btn);
        *imp.copy_btn.borrow_mut() = Some(copy_btn);
        *imp.clear_btn.borrow_mut() = Some(clear_btn);
        *imp.save_btn.borrow_mut() = Some(save_btn);
        *imp.translate_toggle.borrow_mut() = Some(translate_toggle);
        *imp.ai_toggle.borrow_mut() = Some(ai_toggle);
        *imp.hero_state.borrow_mut() = Some(hero_state);
        *imp.hero_state_label.borrow_mut() = Some(hero_state_label);
        *imp.hero_title.borrow_mut() = Some(hero_title);
        *imp.hero_subtitle.borrow_mut() = Some(hero_subtitle);
        *imp.hero_hint.borrow_mut() = Some(hero_hint);
        *imp.rec_tools.borrow_mut() = Some(rec_tools);
        *imp.mode_buttons.borrow_mut() = mode_buttons;
    }

    /// Whether "Improve with AI" is armed.
    pub fn is_ai_active(&self) -> bool {
        self.imp().ai_toggle.borrow().as_ref().map(|t| t.is_active()).unwrap_or(false)
    }

    /// Set the "Improve with AI" toggle state.
    pub fn set_ai_active(&self, active: bool) {
        if let Some(t) = self.imp().ai_toggle.borrow().as_ref() {
            t.set_active(active);
        }
    }

    /// Connect a callback for when the "Improve with AI" toggle changes.
    pub fn connect_ai_toggled<F: Fn(bool) + 'static>(&self, callback: F) {
        if let Some(t) = self.imp().ai_toggle.borrow().as_ref() {
            t.connect_toggled(move |t| callback(t.is_active()));
        }
    }

    /// Show/hide the "Improve with AI" button.
    pub fn set_ai_visible(&self, visible: bool) {
        if let Some(t) = self.imp().ai_toggle.borrow().as_ref() {
            t.set_visible(visible);
        }
    }

    /// Connect an action callback. The hero button dispatches Record ⇢ Stop
    /// internally based on the current recording flag; all other actions are
    /// wired directly to their buttons.
    pub fn connect_action<F: Fn(ControlAction) + Clone + 'static>(&self, callback: F) {
        let imp = self.imp();

        // Store the callback so the hero's lazy dispatcher can use it.
        let cb: Rc<dyn Fn(ControlAction)> = Rc::new(callback.clone());
        *imp.action_cb.borrow_mut() = Some(cb);

        if let Some(btn) = imp.pause_btn.borrow().as_ref() {
            let controls = self.downgrade();
            btn.connect_clicked(move |_| {
                let Some(controls) = controls.upgrade() else { return };
                let action = if controls.imp().paused_state.get() {
                    ControlAction::Resume
                } else {
                    ControlAction::Pause
                };
                let callback = controls.imp().action_cb.borrow().clone();
                if let Some(cb) = callback {
                    cb(action);
                }
            });
        }
        if let Some(btn) = imp.cancel_btn.borrow().as_ref() {
            let cb = callback.clone();
            btn.connect_clicked(move |_| cb(ControlAction::Cancel));
        }
        if let Some(btn) = imp.open_file_btn.borrow().as_ref() {
            let cb = callback.clone();
            btn.connect_clicked(move |_| cb(ControlAction::OpenFile));
        }
        if let Some(btn) = imp.copy_btn.borrow().as_ref() {
            let cb = callback.clone();
            btn.connect_clicked(move |_| cb(ControlAction::Copy));
        }
        if let Some(btn) = imp.clear_btn.borrow().as_ref() {
            let cb = callback.clone();
            btn.connect_clicked(move |_| cb(ControlAction::Clear));
        }
        if let Some(btn) = imp.save_btn.borrow().as_ref() {
            let cb = callback;
            btn.connect_clicked(move |_| cb(ControlAction::Save));
        }
    }

    /// Set recording state.
    ///
    /// - flips the hero between the blue mic (idle) and the red stop square
    ///   (live), with a soft pulsing glow while recording;
    /// - enables Pause/Cancel.
    pub fn set_recording_state(&self, recording: bool) {
        let imp = self.imp();
        imp.recording_state.set(recording);

        if let Some(state) = imp.hero_state.borrow().as_ref() {
            state.remove_css_class("ready");
            state.remove_css_class("recording");
            state.add_css_class(if recording { "recording" } else { "ready" });
        }
        if let Some(label) = imp.hero_state_label.borrow().as_ref() {
            let text = if recording {
                gettext("Recording")
            } else {
                gettext("Ready")
            };
            label.set_text(&text);
        }
        if let Some(label) = imp.hero_title.borrow().as_ref() {
            let text = if recording {
                gettext("Listening now")
            } else {
                gettext("Start a new recording")
            };
            label.set_text(&text);
        }
        if let Some(label) = imp.hero_subtitle.borrow().as_ref() {
            let text = if recording {
                gettext("Your words appear below as you speak. Use the button again when you are done.")
            } else {
                gettext("Capture a thought, draft a message, or transcribe an audio file with local speech recognition.")
            };
            label.set_text(&text);
        }
        if let Some(label) = imp.hero_hint.borrow().as_ref() {
            let text = if recording {
                gettext("Recording in progress · Esc to stop")
            } else {
                gettext("Ctrl+R to start · Esc to cancel")
            };
            label.set_text(&text);
        }
        if let Some(label) = imp.hero_record_label.borrow().as_ref() {
            label.set_visible(false);
        }

        if let Some(btn) = imp.record_btn.borrow().as_ref() {
            if let Some(img) = btn.child().and_downcast::<gtk::Image>() {
                img.set_icon_name(if recording {
                    Some("media-playback-stop-symbolic")
                } else {
                    Some("audio-input-microphone-symbolic")
                });
            }
            btn.remove_css_class("suggested-action");
            btn.remove_css_class("hero-recording");
            if recording {
                btn.add_css_class("hero-recording");
                btn.set_tooltip_text(Some(gettext("Stop recording and transcribe").as_str()));
            } else {
                btn.add_css_class("suggested-action");
                btn.set_tooltip_text(Some(gettext("Start recording (Ctrl+R)").as_str()));
            }
        }

        if let Some(btn) = imp.pause_btn.borrow().as_ref() {
            btn.set_sensitive(recording);
        }
        if let Some(btn) = imp.cancel_btn.borrow().as_ref() {
            btn.set_sensitive(recording);
        }
        if let Some(tools) = imp.rec_tools.borrow().as_ref() {
            // Fade instead of show/hide: the row always occupies layout space,
            // keeping the window height stable across idle ⇄ recording.
            tools.set_opacity(if recording { 1.0 } else { 0.0 });
            tools.set_can_target(recording);
        }
        if let Some(btn) = imp.open_file_btn.borrow().as_ref() {
            btn.set_sensitive(!recording);
        }
        // Legacy stop_btn is intentionally absent; nothing to toggle.
    }

    /// Set paused state — toggle the pause/resume icon, label + tooltip.
    pub fn set_paused_state(&self, paused: bool) {
        self.imp().paused_state.set(paused);
        if let Some(btn) = self.imp().pause_btn.borrow().as_ref() {
            let Some(content) = btn.child().and_downcast::<adw::ButtonContent>() else {
                return;
            };
            if paused {
                content.set_icon_name("media-playback-start-symbolic");
                content.set_label(&gettext("Resume"));
                btn.set_tooltip_text(Some(gettext("Resume recording").as_str()));
            } else {
                content.set_icon_name("media-playback-pause-symbolic");
                content.set_label(&gettext("Pause"));
                btn.set_tooltip_text(Some(gettext("Pause recording").as_str()));
            }
        }
    }

    /// Reset all buttons to initial state.
    pub fn reset(&self) {
        self.set_recording_state(false);
        self.set_paused_state(false);
    }

    /// Get whether the translate toggle is active.
    pub fn is_translate_active(&self) -> bool {
        self.imp()
            .translate_toggle
            .borrow()
            .as_ref()
            .map(|t| t.is_active())
            .unwrap_or(false)
    }

    /// Set the translate toggle state.
    pub fn set_translate_active(&self, active: bool) {
        if let Some(toggle) = self.imp().translate_toggle.borrow().as_ref() {
            toggle.set_active(active);
        }
    }

    /// Connect a callback for when translate toggle changes.
    pub fn connect_translate_changed<F: Fn(bool) + 'static>(&self, callback: F) {
        if let Some(toggle) = self.imp().translate_toggle.borrow().as_ref() {
            toggle.connect_active_notify(move |t| {
                callback(t.is_active());
            });
        }
    }

    /// Enable/disable the translate toggle based on backend capabilities. It
    /// stays visible (greyed-out when disabled) rather than disappearing.
    pub fn set_translate_enabled(&self, enabled: bool) {
        if let Some(toggle) = self.imp().translate_toggle.borrow().as_ref() {
            toggle.set_visible(true);
            toggle.set_sensitive(enabled);
            if !enabled {
                toggle.set_active(false);
            }
        }
    }
}
