// Speech to Text - Main Window
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Main application window with sidebar navigation and content area.

use gtk4::prelude::*;
use gtk4::{gio, glib};
use gtk4 as gtk;
use libadwaita as adw;
use adw::prelude::*;
use adw::subclass::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use crate::application::Application;
use crate::audio::AudioCapture;
use crate::config::{AppConfig, LlmPreset};
use crate::recording::{DictationParams, DictationMode, RecordingController, RecordingOwner};
use crate::transcription::{ModelCatalog, TranscriptionEngine};
use crate::ui::{
    Controls, HeaderControls, HelpPage, HistoryPage, StatusBar, TranscriptView,
    WelcomeWizard, ControlAction,
};
use crate::ui::settings::{MicrophonePage, ModelPage, LanguagePage, PerformancePage, DictationPage, DictionaryPage, LlmPage, language_code_to_name};
use crate::ui::widgets::GpuStatusPanel;
use crate::transcription::postprocess;
use crate::i18n::gettext;

/// Whisper inference parameters gathered from the settings widgets, shared by
/// the live-recording and file-drop transcription paths.
struct WhisperRunParams {
    n_threads: u32,
    beam_size: u32,
    temperature: f32,
    translate: bool,
    initial_prompt: Option<String>,
}

/// Navigation items for the sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavItem {
    Transcription,
    History,
    Microphone,
    Model,
    Language,
    Performance,
    Dictation,
    Dictionary,
    Llm,
    Help,
}

impl NavItem {
    pub fn icon_name(&self) -> &'static str {
        match self {
            Self::Transcription => "audio-input-microphone-symbolic",
            Self::History => "document-open-recent-symbolic",
            Self::Microphone => "audio-card-symbolic",
            Self::Model => "system-software-install-symbolic",
            Self::Language => "preferences-desktop-locale-symbolic",
            Self::Performance => "preferences-system-symbolic",
            Self::Dictation => "input-keyboard-symbolic",
            Self::Dictionary => "accessories-dictionary-symbolic",
            Self::Llm => "network-transmit-receive-symbolic",
            Self::Help => "help-about-symbolic",
        }
    }

    pub fn title(&self) -> String {
        match self {
            Self::Transcription => gettext("Transcription"),
            Self::History => gettext("History"),
            Self::Microphone => gettext("Microphone"),
            Self::Model => gettext("Model"),
            Self::Language => gettext("Language"),
            Self::Performance => gettext("Performance"),
            Self::Dictation => gettext("Dictation"),
            Self::Dictionary => gettext("Dictionary"),
            Self::Llm => gettext("LLM"),
            Self::Help => gettext("Help"),
        }
    }

    /// Whether this is a settings page (shown under "Settings" header).
    pub fn is_settings(&self) -> bool {
        matches!(self, Self::Microphone | Self::Model | Self::Language | Self::Performance | Self::Dictation | Self::Dictionary | Self::Llm)
    }

    pub fn all() -> &'static [NavItem] {
        &[
            Self::Transcription,
            Self::History,
            Self::Microphone,
            Self::Model,
            Self::Language,
            Self::Performance,
            Self::Dictation,
            Self::Dictionary,
            Self::Llm,
            Self::Help,
        ]
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct MainWindow {
        pub toast_overlay: RefCell<Option<adw::ToastOverlay>>,
        /// Top-of-content notification bar (themed, in-window) — replaces the
        /// bottom OSD toasts so messages are easy to see and match the theme.
        pub notification_banner: RefCell<Option<adw::Banner>>,
        pub sidebar_box: RefCell<Option<gtk::Box>>,
        pub sidebar_list: RefCell<Option<gtk::ListBox>>,
        pub content_stack: RefCell<Option<gtk::Stack>>,
        pub header_controls: RefCell<Option<HeaderControls>>,
        pub current_nav: Cell<Option<NavItem>>,
        pub nav_labels: RefCell<Vec<gtk::Label>>,
        pub nav_boxes: RefCell<Vec<gtk::Box>>,
        pub sidebar_collapsed: Cell<bool>,
        pub sidebar_toggle_btn: RefCell<Option<gtk::Button>>,
        pub sidebar_title_box: RefCell<Option<gtk::Box>>,
        pub settings_header_label: RefCell<Option<gtk::Label>>,
        pub info_box: RefCell<Option<gtk::Box>>,
        pub gpu_panel: RefCell<Option<GpuStatusPanel>>,
        pub update_banner: RefCell<Option<gtk::Box>>,
        pub syncing_backend: Cell<bool>,
        pub syncing_dropdown: Cell<bool>,
        pub syncing_translate: Cell<bool>,
        pub syncing_ai: Cell<bool>,
        /// Guards the live (while-speaking) decode so ticks don't pile up.
        pub live_decoding: Cell<bool>,
        /// True while recording a Voice-edit instruction in the main window.
        pub voice_editing: Cell<bool>,
        /// Set when a live decode was too slow or failed; disables the live loop
        /// for the rest of this recording so it can't flood a slow/VRAM-tight GPU.
        pub live_too_slow: Cell<bool>,

        // Content pages
        pub transcript_view: RefCell<Option<TranscriptView>>,
        pub controls: RefCell<Option<Controls>>,
        pub status_bar: RefCell<Option<StatusBar>>,
        pub history_page: RefCell<Option<HistoryPage>>,
        pub help_page: RefCell<Option<HelpPage>>,
        pub microphone_page: RefCell<Option<MicrophonePage>>,
        pub model_page: RefCell<Option<ModelPage>>,
        pub language_page: RefCell<Option<LanguagePage>>,
        pub performance_page: RefCell<Option<PerformancePage>>,
        pub dictation_page: RefCell<Option<DictationPage>>,
        pub dictionary_page: RefCell<Option<DictionaryPage>>,
        pub llm_page: RefCell<Option<LlmPage>>,
        /// One result per dictation (raw transcript + AI variants). The transcript
        /// view shows one selectable bubble per entry; `selected_message` is the
        /// one the chips / Improve / Voice edit act on. Append-only + clear-all,
        /// so indices stay in sync with the transcript view's bubbles.
        pub messages: RefCell<Vec<crate::ui::result_state::ResultState>>,
        pub selected_message: Cell<isize>,

        // App state
        pub config: RefCell<Option<Arc<AppConfig>>>,
        /// Shared recording + transcription controller (owned by the Application).
        pub controller: RefCell<Option<Rc<RecordingController>>>,
        pub audio_capture: RefCell<Option<Arc<Mutex<AudioCapture>>>>,
        pub engine: RefCell<Option<Arc<Mutex<Option<TranscriptionEngine>>>>>,
        pub model_catalog: RefCell<Option<Arc<ModelCatalog>>>,
        /// Last transcription segments for SRT export: (start_ms, end_ms, text).
        pub last_segments: RefCell<Vec<(i64, i64, String)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MainWindow {
        const NAME: &'static str = "SpeechToTextMainWindow";
        type Type = super::MainWindow;
        type ParentType = adw::ApplicationWindow;
    }

    impl ObjectImpl for MainWindow {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.setup_ui();
            obj.setup_actions();
        }
    }

    impl WidgetImpl for MainWindow {}
    impl WindowImpl for MainWindow {}
    impl ApplicationWindowImpl for MainWindow {}
    impl AdwApplicationWindowImpl for MainWindow {}
}

glib::wrapper! {
    pub struct MainWindow(ObjectSubclass<imp::MainWindow>)
        @extends gtk::Widget, gtk::Window, gtk::ApplicationWindow, adw::ApplicationWindow,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl MainWindow {
    // Widened so the transcript area on the right is wider while the sidebar
    // stays at SIDEBAR_EXPANDED_WIDTH. Content = WINDOW_WIDTH - 280: at 968 the
    // content is ~688px (two +15% steps over the original 800/520 layout).
    const WINDOW_WIDTH: i32 = 1280;
    const WINDOW_HEIGHT: i32 = 750;
    /// Minimum width of the right column (transcript area). The window is now
    /// resizable, so `set_default_size` drives the initial 1100px width and this
    /// is only the floor below which the right column won't shrink. Kept modest
    /// so the user can also drag the window smaller; the sidebar stays fixed.
    const CONTENT_MIN_WIDTH: i32 = 560;
    const SIDEBAR_EXPANDED_WIDTH: i32 = 280;
    const SIDEBAR_COLLAPSED_WIDTH: i32 = 50;

    pub fn new(app: &Application, config: Arc<AppConfig>) -> Self {
        let window: Self = glib::Object::builder()
            .property("application", app)
            .build();

        window.set_default_size(Self::WINDOW_WIDTH, Self::WINDOW_HEIGHT);
        // Resizable + maximizable. The sidebar keeps a fixed width (hexpand=false
        // + width_request), and only the content_box has hexpand=true, so all the
        // extra width on resize/maximize goes to the right side.
        window.set_resizable(true);
        window.set_title(Some(crate::APP_NAME));

        // Store config and borrow the shared recording state from the
        // Application's controller so the main window, the mini panel, and the
        // global dictation shortcut all drive the same cpal stream + engine.
        let imp = window.imp();
        *imp.config.borrow_mut() = Some(config.clone());
        let controller = app.controller();
        *imp.audio_capture.borrow_mut() = Some(controller.audio_arc());
        *imp.engine.borrow_mut() = Some(controller.engine_arc());
        *imp.model_catalog.borrow_mut() = Some(controller.model_catalog_arc());
        *imp.controller.borrow_mut() = Some(controller);

        // One-time migration: move any plaintext HF token from config into the
        // system keyring, then clear it from the (world-readable-ish) config file.
        if let Some(token) = config.cohere_hf_token.clone().filter(|t| !t.trim().is_empty()) {
            crate::application::tokio_runtime().spawn(async move {
                if crate::secrets::store_hf_token(&token).await.is_ok() {
                    let mut c = AppConfig::load();
                    c.cohere_hf_token = None;
                    c.save();
                    tracing::info!("Migrated HuggingFace token from config to keyring");
                }
            });
        }

        window.sync_ui_from_config();

        // Closing the window hides it; the app keeps running in the background
        // (tray icon + global shortcut stay active). Quit via Ctrl+Q or the
        // tray "Quit" item. Re-open by launching the app again or via the tray.
        window.connect_close_request(|win| {
            win.set_visible(false);
            glib::Propagation::Stop
        });

        // Show welcome wizard on first run
        if config.first_run {
            let wizard = WelcomeWizard::new(&window);
            wizard.present();
        } else {
            // Try to load the selected model in the background
            window.load_selected_model();
        }

        // Diagnostic (inert unless STT_DEBUG_WIDTH is set): log the real ALLOCATED
        // widths after layout settles, so the sidebar width can be verified
        // (width_request is only a minimum — content can force it wider).
        if std::env::var("STT_DEBUG_WIDTH").is_ok() {
            let w = window.downgrade();
            glib::timeout_add_local_once(std::time::Duration::from_millis(2500), move || {
                if let Some(win) = w.upgrade() {
                    let imp = win.imp();
                    let sidebar = imp.sidebar_box.borrow().as_ref().map(|b| b.width()).unwrap_or(-1);
                    let collapsed = imp.sidebar_collapsed.get();
                    eprintln!(
                        "STT_SIDEBAR_WIDTH={} STT_SIDEBAR_REQUEST={} STT_WIN_WIDTH={} collapsed={}",
                        sidebar,
                        if collapsed { Self::SIDEBAR_COLLAPSED_WIDTH } else { Self::SIDEBAR_EXPANDED_WIDTH },
                        win.width(),
                        collapsed,
                    );
                }
            });
        }

        window
    }

    fn setup_ui(&self) {
        let imp = self.imp();

        // Main horizontal layout: sidebar | separator | content
        let main_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);

        // === SIDEBAR ===
        let sidebar_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sidebar_box.set_width_request(Self::SIDEBAR_EXPANDED_WIDTH);
        sidebar_box.set_hexpand(false);
        sidebar_box.add_css_class("sidebar-box");

        // Sidebar header
        let sidebar_header = adw::HeaderBar::new();
        sidebar_header.set_show_end_title_buttons(false);
        sidebar_header.set_show_start_title_buttons(false);

        // Sidebar collapse button
        let sidebar_toggle_btn = gtk::Button::builder()
            .icon_name("sidebar-show-symbolic")
            .tooltip_text("Collapse sidebar")
            .build();
        sidebar_toggle_btn.add_css_class("flat");
        sidebar_toggle_btn.set_action_name(Some("win.toggle-sidebar"));
        sidebar_header.pack_end(&sidebar_toggle_btn);

        let sidebar_icon = gtk::Image::from_icon_name(crate::APP_ID);
        sidebar_icon.set_pixel_size(20);
        sidebar_icon.set_margin_end(8);

        let title_label = gtk::Label::new(Some(crate::APP_NAME));
        title_label.add_css_class("title");

        let title_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        title_box.set_halign(gtk::Align::Center);
        title_box.append(&sidebar_icon);
        title_box.append(&title_label);

        sidebar_header.set_title_widget(Some(&title_box));
        sidebar_box.append(&sidebar_header);

        // Navigation list
        let sidebar_list = gtk::ListBox::new();
        sidebar_list.set_selection_mode(gtk::SelectionMode::Single);
        sidebar_list.add_css_class("navigation-sidebar");

        let mut nav_labels = Vec::new();
        let mut nav_boxes = Vec::new();

        // Add nav items with section headers
        let mut added_settings_header = false;
        let settings_header_label_opt: Option<gtk::Label> = None;
        for nav_item in NavItem::all() {
            if nav_item.is_settings() && !added_settings_header {
                // Add a small spacer before settings items (no label)
                let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
                spacer.set_margin_top(8);
                sidebar_box.append(&spacer);
                added_settings_header = true;
            }

            let (row, label, hbox) = self.create_nav_row(*nav_item);
            sidebar_list.append(&row);
            nav_labels.push(label);
            nav_boxes.push(hbox);
        }

        let sidebar_scroll = gtk::ScrolledWindow::new();
        sidebar_scroll.set_vexpand(true);
        sidebar_scroll.set_child(Some(&sidebar_list));
        sidebar_box.append(&sidebar_scroll);

        // GPU Status panel at bottom of sidebar
        let gpu_panel = GpuStatusPanel::new();
        sidebar_box.append(&gpu_panel);

        // Version and author info at the bottom of sidebar
        let info_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
        info_box.set_margin_start(12);
        info_box.set_margin_end(12);
        info_box.set_margin_top(8);
        info_box.set_margin_bottom(8);

        // Update banner — hidden until version check completes
        let update_banner = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        update_banner.add_css_class("update-banner");
        update_banner.set_visible(false);
        update_banner.set_halign(gtk::Align::Start);

        let update_icon = gtk::Image::from_icon_name("software-update-available-symbolic");
        update_icon.set_pixel_size(14);
        update_banner.append(&update_icon);

        let update_label = gtk::Label::new(Some("New version available"));
        update_label.add_css_class("update-banner-label");
        update_banner.append(&update_label);

        info_box.append(&update_banner);

        sidebar_box.append(&info_box);

        // Separator
        let separator = gtk::Separator::new(gtk::Orientation::Vertical);

        // === CONTENT AREA ===
        let content_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        // hexpand=true is what makes the right column absorb all extra width when
        // the window is resized/maximized, while the sidebar (hexpand=false) keeps
        // its fixed width. CONTENT_MIN_WIDTH is just the floor for this column.
        content_box.set_hexpand(true);
        content_box.set_width_request(Self::CONTENT_MIN_WIDTH);

        // Header with mic/model selectors
        let header_controls = HeaderControls::new();
        content_box.append(&header_controls);

        // In-app notification bar (top of content, themed). Its single action
        // button is used only for the "Undo" after an AI improvement.
        let notification_banner = adw::Banner::new("");
        notification_banner.set_revealed(false);
        content_box.append(&notification_banner);

        // Content stack for pages
        let content_stack = gtk::Stack::new();
        content_stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        content_stack.set_transition_duration(200);
        content_stack.set_vexpand(true);
        content_stack.set_hexpand(true);

        // Transcription page (main view)
        let transcription_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        transcription_page.set_vexpand(true);

        let transcript_view = TranscriptView::new();
        transcription_page.append(&transcript_view);

        // Full-width container so the sidebar background spans the entire width
        let controls_area = gtk::Box::new(gtk::Orientation::Vertical, 0);
        controls_area.add_css_class("controls-area");
        let controls = Controls::new();
        controls_area.append(&controls);
        transcription_page.append(&controls_area);

        content_stack.add_named(&transcription_page, Some("transcription"));

        // History page
        let history_page = HistoryPage::new();
        content_stack.add_named(&history_page, Some("history"));

        // Settings pages
        let microphone_page = MicrophonePage::new();
        content_stack.add_named(&microphone_page, Some("microphone"));

        let model_page = ModelPage::new();
        content_stack.add_named(&model_page, Some("model"));

        let language_page = LanguagePage::new();
        content_stack.add_named(&language_page, Some("language"));

        let performance_page = PerformancePage::new();
        content_stack.add_named(&performance_page, Some("performance"));

        let dictation_page = DictationPage::new();
        content_stack.add_named(&dictation_page, Some("dictation"));

        let dictionary_page = DictionaryPage::new();
        content_stack.add_named(&dictionary_page, Some("dictionary"));

        let llm_page = LlmPage::new();
        content_stack.add_named(&llm_page, Some("llm"));

        // Help page
        let help_page = HelpPage::new();
        content_stack.add_named(&help_page, Some("help"));

        content_box.append(&content_stack);

        // Status bar
        let status_bar = StatusBar::new();
        content_box.append(&status_bar);

        // Assemble main layout
        main_box.append(&sidebar_box);
        main_box.append(&separator);
        main_box.append(&content_box);

        // Toast overlay for notifications
        let toast_overlay = adw::ToastOverlay::new();
        toast_overlay.set_child(Some(&main_box));

        self.set_content(Some(&toast_overlay));

        // Store references
        *imp.toast_overlay.borrow_mut() = Some(toast_overlay);
        *imp.notification_banner.borrow_mut() = Some(notification_banner.clone());

        // The banner's action button is only ever the "Undo" after an AI improve
        // (switch the result back to the raw transcript).
        {
            let window = self.clone();
            notification_banner.connect_button_clicked(move |b| {
                b.set_revealed(false);
                window.select_variant(0);
            });
        }
        *imp.sidebar_box.borrow_mut() = Some(sidebar_box);
        *imp.sidebar_list.borrow_mut() = Some(sidebar_list.clone());
        *imp.content_stack.borrow_mut() = Some(content_stack);
        *imp.header_controls.borrow_mut() = Some(header_controls);
        *imp.transcript_view.borrow_mut() = Some(transcript_view);
        *imp.controls.borrow_mut() = Some(controls);
        *imp.status_bar.borrow_mut() = Some(status_bar);
        *imp.history_page.borrow_mut() = Some(history_page);
        *imp.help_page.borrow_mut() = Some(help_page);
        *imp.microphone_page.borrow_mut() = Some(microphone_page);
        *imp.model_page.borrow_mut() = Some(model_page);
        *imp.language_page.borrow_mut() = Some(language_page);
        *imp.performance_page.borrow_mut() = Some(performance_page);
        *imp.dictation_page.borrow_mut() = Some(dictation_page);
        *imp.dictionary_page.borrow_mut() = Some(dictionary_page);
        *imp.llm_page.borrow_mut() = Some(llm_page);
        *imp.gpu_panel.borrow_mut() = Some(gpu_panel);
        *imp.nav_labels.borrow_mut() = nav_labels;
        *imp.nav_boxes.borrow_mut() = nav_boxes;
        *imp.sidebar_toggle_btn.borrow_mut() = Some(sidebar_toggle_btn);
        *imp.sidebar_title_box.borrow_mut() = Some(title_box);
        *imp.settings_header_label.borrow_mut() = settings_header_label_opt;
        *imp.info_box.borrow_mut() = Some(info_box);
        *imp.update_banner.borrow_mut() = Some(update_banner);
        imp.sidebar_collapsed.set(false);

        // Connect navigation
        let window = self.clone();
        sidebar_list.connect_row_selected(move |_, row| {
            if let Some(row) = row {
                let index = row.index() as usize;
                if let Some(nav_item) = NavItem::all().get(index) {
                    window.navigate_to(*nav_item);
                }
            }
        });

        // Select first item
        if let Some(first_row) = sidebar_list.row_at_index(0) {
            sidebar_list.select_row(Some(&first_row));
        }

        // Wire recording controls
        self.setup_controls();

        // Wire file drag-and-drop
        self.setup_file_drop();

        // Wire backend selector
        self.setup_engine_selector();

        // Wire model dropdown selection change
        self.setup_model_dropdown();

        // Wire language changes to header
        self.setup_language_display();

        // Wire the "Improve with AI" toggle
        self.setup_ai_toggle();

        // Wire transform chips + raw/polished variant selector
        self.setup_transcript_extras();

        // Check for updates at startup
        self.check_for_updates();
    }

    /// Wire the "Improve with AI" toggle: arming it turns on auto-improve so the
    /// next transcriptions are improved with the active preset (like Translate).
    /// Kept in sync with the "Auto-improve after dictation" setting via config.
    fn setup_ai_toggle(&self) {
        let Some(controls) = self.imp().controls.borrow().as_ref().cloned() else {
            return;
        };
        let window = self.clone();
        controls.connect_ai_toggled(move |active| {
            if window.imp().syncing_ai.get() {
                return;
            }
            let mut c = AppConfig::load();
            c.llm_auto_apply = active;
            c.save();
        });
        self.refresh_ai_button();
    }

    /// Show/hide the "Improve with AI" toggle and sync its state with the current
    /// `llm_enabled` / `llm_auto_apply` settings.
    fn refresh_ai_button(&self) {
        if let Some(controls) = self.imp().controls.borrow().as_ref() {
            let cfg = AppConfig::load();
            controls.set_ai_visible(cfg.llm_enabled);
            self.imp().syncing_ai.set(true);
            controls.set_ai_active(cfg.llm_auto_apply);
            self.imp().syncing_ai.set(false);
        }
        // Keep the transform chips in sync with the (possibly edited) preset list.
        self.refresh_chip_presets();
    }

    /// Wire chip / raw-polished selector / message-selection callbacks and
    /// populate the chip presets from config.
    fn setup_transcript_extras(&self) {
        let Some(tv) = self.imp().transcript_view.borrow().as_ref().cloned() else {
            return;
        };
        let window = self.clone();
        tv.connect_chip_activated(move |idx| window.on_chip(idx));
        let window = self.clone();
        tv.connect_variant_changed(move |idx| window.select_variant(idx));
        let window = self.clone();
        tv.connect_message_selected(move |idx| window.select_message(idx));
        let window = self.clone();
        tv.connect_voice_edit(move || window.on_voice_edit());
        self.imp().selected_message.set(-1);
        self.refresh_chip_presets();
    }

    /// The currently-selected message index, if any.
    fn selected_idx(&self) -> Option<usize> {
        let s = self.imp().selected_message.get();
        if s < 0 { None } else { Some(s as usize) }
    }

    /// Refresh the chip preset labels + visibility from the current LLM config.
    fn refresh_chip_presets(&self) {
        let Some(tv) = self.imp().transcript_view.borrow().as_ref().cloned() else {
            return;
        };
        let cfg = AppConfig::load();
        let names: Vec<String> = cfg.llm_presets.iter().map(|p| p.name.clone()).collect();
        tv.set_chip_presets(&names);
        let has_result = !self.imp().messages.borrow().is_empty();
        tv.set_chips_visible(cfg.llm_enabled && has_result);
    }

    /// Append a finished transcription as a new selectable message and render it.
    fn add_result_message(&self, state: crate::ui::result_state::ResultState, confidence: f64) {
        let Some(tv) = self.imp().transcript_view.borrow().as_ref().cloned() else {
            return;
        };
        tv.clear_live_preview();
        let text = state.active_text().to_string();
        let (words, wpm) = (state.stats.words, state.stats.wpm);
        self.imp().messages.borrow_mut().push(state);
        let idx = tv.add_message(&text); // append + auto-select
        self.imp().selected_message.set(idx as isize);
        tv.set_confidence(confidence);
        tv.set_stats(words, wpm);
        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(&text);
        }
        self.refresh_result_controls();
    }

    /// Rebuild the variant selector + stats + chip state from the SELECTED message.
    fn refresh_result_controls(&self) {
        let Some(tv) = self.imp().transcript_view.borrow().as_ref().cloned() else {
            return;
        };
        let cfg = AppConfig::load();
        let Some(sel) = self.selected_idx() else {
            tv.hide_result_controls();
            return;
        };
        let messages = self.imp().messages.borrow();
        if let Some(st) = messages.get(sel) {
            let labels = st.selector_labels(&gettext("Raw"));
            tv.set_variant_selector(&labels, st.active);
            tv.set_stats(st.stats.words, st.stats.wpm);
            tv.set_chips_visible(cfg.llm_enabled);
            tv.set_chips_sensitive(true);
            tv.set_voice_edit_visible(cfg.llm_enabled);
        }
    }

    /// Select a message (clicking its bubble): highlight it and point the
    /// chips / selector / stats at it.
    fn select_message(&self, idx: usize) {
        if idx >= self.imp().messages.borrow().len() {
            return;
        }
        self.imp().selected_message.set(idx as isize);
        if let Some(tv) = self.imp().transcript_view.borrow().as_ref() {
            tv.set_selected_message(idx);
        }
        self.refresh_result_controls();
    }

    /// Switch the active variant of the SELECTED message (from the dropdown).
    fn select_variant(&self, vidx: usize) {
        let Some(sel) = self.selected_idx() else { return };
        let active_text = {
            let mut messages = self.imp().messages.borrow_mut();
            let Some(st) = messages.get_mut(sel) else { return };
            st.set_active(vidx);
            st.active_text().to_string()
        };
        if let Some(tv) = self.imp().transcript_view.borrow().as_ref() {
            tv.update_message(sel, &active_text);
        }
        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(&active_text);
        }
    }

    /// Handle a transform-chip click: run preset `idx` on the SELECTED message.
    fn on_chip(&self, chip_idx: usize) {
        let Some(sel) = self.selected_idx() else { return };
        let cfg = AppConfig::load();
        let Some(preset) = cfg.llm_presets.get(chip_idx).cloned() else {
            return;
        };
        self.run_preset_on_message(sel, preset, false);
    }

    /// Run `preset` on message `msg_idx`'s active text and add the result as a new
    /// (auto-activated) variant on THAT message only — other messages are untouched.
    fn run_preset_on_message(&self, msg_idx: usize, preset: LlmPreset, show_undo: bool) {
        let source = {
            let messages = self.imp().messages.borrow();
            match messages.get(msg_idx) {
                Some(st) => st.active_text().trim().to_string(),
                None => return,
            }
        };
        if source.is_empty() {
            return;
        }
        let llm_cfg = Self::resolve_llm_config_for_preset(&preset);
        if let Some(tv) = self.imp().transcript_view.borrow().as_ref() {
            tv.set_chips_sensitive(false);
        }
        if let Some(sb) = self.imp().status_bar.borrow().as_ref() {
            sb.set_recording_status(&gettext("Improving with AI…"));
        }
        let rx = crate::llm::improve_async(llm_cfg, preset.system_prompt(), source);
        let window = self.clone();
        let label = preset.name.clone();
        glib::spawn_future_local(async move {
            let res = rx.recv().await;
            if let Some(tv) = window.imp().transcript_view.borrow().as_ref() {
                tv.set_chips_sensitive(true);
            }
            if let Some(sb) = window.imp().status_bar.borrow().as_ref() {
                sb.set_recording_status(&gettext("Ready"));
            }
            match res {
                Ok(Ok(improved)) => {
                    let improved = improved.trim().to_string();
                    if improved.is_empty() {
                        window.show_toast(&gettext("AI returned an empty result"));
                        return;
                    }
                    let active_text = {
                        let mut messages = window.imp().messages.borrow_mut();
                        let Some(st) = messages.get_mut(msg_idx) else { return };
                        st.push_variant(label, improved);
                        st.active_text().to_string()
                    };
                    if let Some(tv) = window.imp().transcript_view.borrow().as_ref() {
                        tv.update_message(msg_idx, &active_text);
                    }
                    // If this is the selected message, refresh its controls.
                    if window.selected_idx() == Some(msg_idx) {
                        window.refresh_result_controls();
                        if let Some(display) = gtk::gdk::Display::default() {
                            display.clipboard().set_text(&active_text);
                        }
                    }
                    if show_undo {
                        window.show_improve_undo_toast();
                    }
                }
                Ok(Err(e)) => window.show_toast(&format!("{}: {}", gettext("AI error"), e)),
                Err(_) => {}
            }
        });
    }

    /// Build the LLM connection config for a preset. Delegates to the canonical
    /// resolver in `application` so the override logic lives in one place.
    fn resolve_llm_config_for_preset(preset: &LlmPreset) -> crate::llm::LlmConfig {
        crate::application::resolve_llm_cfg(&AppConfig::load(), preset)
    }

    /// Auto-improve: run the active preset on the most-recent message.
    fn improve_with_preset(&self, preset_idx: usize, _source_text: String) {
        let cfg = AppConfig::load();
        let Some(preset) = cfg.llm_presets.get(preset_idx).cloned() else {
            return;
        };
        let last = match self.imp().messages.borrow().len() {
            0 => return,
            n => n - 1,
        };
        self.run_preset_on_message(last, preset, true);
    }

    /// Voice edit (main window): toggle recording a spoken instruction for the
    /// selected message.
    fn on_voice_edit(&self) {
        if self.imp().voice_editing.get() {
            self.stop_voice_edit_main();
        } else {
            self.start_voice_edit_main();
        }
    }

    fn start_voice_edit_main(&self) {
        let Some(sel) = self.selected_idx() else {
            self.show_toast(&gettext("Select a message first."));
            return;
        };
        let has_text = self
            .imp()
            .messages
            .borrow()
            .get(sel)
            .map(|s| !s.active_text().trim().is_empty())
            .unwrap_or(false);
        if !has_text {
            self.show_toast(&gettext("No text to edit."));
            return;
        }
        let cfg = AppConfig::load();
        if !cfg.llm_enabled {
            self.show_toast(&gettext("Enable the LLM in Settings → LLM to use Voice edit."));
            return;
        }
        let Some(controller) = self.controller() else { return };
        if !controller.try_acquire(RecordingOwner::VoiceEdit) {
            self.show_toast(&gettext("Busy — finish the current recording first."));
            return;
        }
        let (waveform_tx, _waveform_rx) = async_channel::bounded::<Vec<f32>>(32);
        match controller.start(cfg.selected_microphone.as_deref(), waveform_tx) {
            Ok(()) => {
                self.imp().voice_editing.set(true);
                if let Some(tv) = self.imp().transcript_view.borrow().as_ref() {
                    tv.set_voice_edit_recording(true);
                }
                if let Some(sb) = self.imp().status_bar.borrow().as_ref() {
                    sb.set_recording_status(&gettext("Listening for your edit… click \"Stop edit\" when done"));
                }
            }
            Err(e) => {
                controller.release();
                self.show_toast(&format!("Couldn't start recording: {e}"));
            }
        }
    }

    fn stop_voice_edit_main(&self) {
        self.imp().voice_editing.set(false);
        if let Some(tv) = self.imp().transcript_view.borrow().as_ref() {
            tv.set_voice_edit_recording(false);
        }
        let Some(controller) = self.controller() else { return };
        if controller.owner() != RecordingOwner::VoiceEdit {
            return;
        }
        let dur = controller.recording_duration_secs();
        let audio = match controller.stop() {
            Ok(a) => a,
            Err(e) => {
                controller.release();
                self.show_toast(&format!("Error stopping recording: {e}"));
                return;
            }
        };
        controller.release();
        if audio.is_empty() {
            self.show_toast(&gettext("Didn't catch an instruction."));
            if let Some(sb) = self.imp().status_bar.borrow().as_ref() {
                sb.set_recording_status(&gettext("Ready"));
            }
            return;
        }
        let Some(sel) = self.selected_idx() else { return };
        let target = self
            .imp()
            .messages
            .borrow()
            .get(sel)
            .map(|s| s.active_text().to_string())
            .unwrap_or_default();
        if target.trim().is_empty() {
            return;
        }
        if let Some(sb) = self.imp().status_bar.borrow().as_ref() {
            sb.set_recording_status(&gettext("Applying your edit…"));
        }
        // Transcribe the spoken instruction (normal short dictation).
        let params = self.build_dictation_params(self.active_backend());
        let receiver = controller.transcribe_async(audio, params, dur);
        let window = self.clone();
        glib::spawn_future_local(async move {
            let instruction = match receiver.recv().await {
                Ok(Ok(o)) => o.cleaned_text.trim().to_string(),
                _ => String::new(),
            };
            if instruction.is_empty() {
                if let Some(sb) = window.imp().status_bar.borrow().as_ref() {
                    sb.set_recording_status(&gettext("Ready"));
                }
                window.show_toast(&gettext("Didn't catch an instruction."));
                return;
            }
            // Apply the instruction to the target text via the LLM.
            let cfg = AppConfig::load();
            let llm_cfg = crate::llm::LlmConfig {
                api_url: cfg.llm_api_url.clone(),
                api_key: None,
                model: cfg.llm_model.clone(),
                temperature: cfg.llm_temperature,
            };
            let system = "You are editing the user's text. Apply the user's spoken instruction to \
                          the TARGET TEXT and reply with ONLY the edited text, preserving the original \
                          language. Do not add explanations or quotes.";
            let user = format!("Apply this instruction: {instruction}\n\nText:\n{target}");
            let rx = crate::llm::improve_async(llm_cfg, system.to_string(), user);
            let res = rx.recv().await;
            if let Some(sb) = window.imp().status_bar.borrow().as_ref() {
                sb.set_recording_status(&gettext("Ready"));
            }
            match res {
                Ok(Ok(edited)) if !edited.trim().is_empty() => {
                    let active_text = {
                        let mut messages = window.imp().messages.borrow_mut();
                        let Some(st) = messages.get_mut(sel) else { return };
                        st.push_variant(gettext("Voice edit"), edited.trim().to_string());
                        st.active_text().to_string()
                    };
                    if let Some(tv) = window.imp().transcript_view.borrow().as_ref() {
                        tv.update_message(sel, &active_text);
                    }
                    if window.selected_idx() == Some(sel) {
                        window.refresh_result_controls();
                        if let Some(display) = gtk::gdk::Display::default() {
                            display.clipboard().set_text(&active_text);
                        }
                    }
                }
                Ok(Ok(_)) => window.show_toast(&gettext("AI returned an empty result")),
                Ok(Err(e)) => window.show_toast(&format!("{}: {}", gettext("AI error"), e)),
                Err(_) => {}
            }
        });
    }

    /// Confirm an AI improvement in the top banner with an Undo button that
    /// switches back to the raw transcript (non-destructive — the AI variant
    /// stays available in the raw/polished selector).
    fn show_improve_undo_toast(&self) {
        self.show_banner(&gettext("Improved with AI"), true);
    }

    /// Show an in-app notification in the top banner (themed, not a bottom OSD
    /// toast). When `with_undo` is set, the banner shows an "Undo" button wired
    /// to revert the last AI improvement. Auto-hides after a few seconds.
    fn show_banner(&self, message: &str, with_undo: bool) {
        let Some(banner) = self.imp().notification_banner.borrow().clone() else {
            return;
        };
        banner.set_title(&crate::error::redact_secrets(message));
        let undo_label = gettext("Undo");
        banner.set_button_label(if with_undo { Some(undo_label.as_str()) } else { None });
        banner.set_revealed(true);
        // Auto-hide; only hide if the banner still shows THIS message (so a newer
        // notification isn't dismissed early).
        let banner_weak = banner.downgrade();
        let shown = crate::error::redact_secrets(message);
        let secs = if with_undo { 6 } else { 4 };
        glib::timeout_add_seconds_local_once(secs, move || {
            if let Some(b) = banner_weak.upgrade() {
                if b.title().as_str() == shown {
                    b.set_revealed(false);
                }
            }
        });
    }

    /// If auto-improve is enabled, run the active preset over `text`.
    fn maybe_auto_improve(&self, text: &str) {
        let cfg = AppConfig::load();
        if cfg.llm_enabled && cfg.llm_auto_apply && !cfg.llm_presets.is_empty() {
            let idx = cfg.llm_active_preset.min(cfg.llm_presets.len() - 1);
            self.improve_with_preset(idx, text.to_string());
        }
    }

    /// For long transcripts (files / long dictations), generate an LLM summary +
    /// chapters and show them in the transcript view's "Summary & chapters"
    /// section. Self-gates on the LLM being enabled and a length threshold.
    fn maybe_generate_summary(&self, text: &str, duration_secs: f32) {
        let cfg = AppConfig::load();
        if !cfg.llm_enabled {
            return;
        }
        let words = crate::ui::result_state::word_count(text);
        if words < crate::limits::SUMMARY_MIN_WORDS && duration_secs < crate::limits::SUMMARY_MIN_SECS {
            return;
        }
        let Some(tv) = self.imp().transcript_view.borrow().as_ref().cloned() else {
            return;
        };
        let llm_cfg = crate::llm::LlmConfig {
            api_url: cfg.llm_api_url.clone(),
            api_key: None,
            model: cfg.llm_model.clone(),
            temperature: cfg.llm_temperature,
        };
        tv.set_summary_loading(true);

        let rx_sum = crate::llm::improve_async(
            llm_cfg.clone(),
            crate::transcription::summary::SUMMARY_SYSTEM_PROMPT.to_string(),
            text.to_string(),
        );
        let segments = self.imp().last_segments.borrow().clone();
        let chap_input = crate::transcription::summary::build_chaptered_input(&segments);
        let rx_chap = if chap_input.trim().is_empty() {
            None
        } else {
            Some(crate::llm::improve_async(
                llm_cfg,
                crate::transcription::summary::CHAPTERS_SYSTEM_PROMPT.to_string(),
                chap_input,
            ))
        };

        let window = self.clone();
        glib::spawn_future_local(async move {
            let summary_text = match rx_sum.recv().await {
                Ok(Ok(s)) => s.trim().to_string(),
                _ => String::new(),
            };
            let chapters = if let Some(rx) = rx_chap {
                match rx.recv().await {
                    Ok(Ok(r)) => crate::transcription::summary::parse_chapters(&r),
                    _ => Vec::new(),
                }
            } else {
                Vec::new()
            };
            if let Some(tv) = window.imp().transcript_view.borrow().as_ref() {
                if summary_text.is_empty() && chapters.is_empty() {
                    tv.clear_summary();
                } else {
                    tv.set_summary(&summary_text, &chapters);
                }
            }
        });
    }

    fn current_config_snapshot(&self) -> Option<AppConfig> {
        self.imp().config.borrow()
            .as_ref()
            .map(|config| (**config).clone())
    }

    /// The shared recording controller (cloned `Rc`).
    fn controller(&self) -> Option<Rc<RecordingController>> {
        self.imp().controller.borrow().clone()
    }

    /// Build a widget-free parameter snapshot for a dictation run, combining the
    /// Whisper inference settings, the selected language, microphone, and mode.
    fn build_dictation_params(&self, backend: String) -> DictationParams {
        let imp = self.imp();
        let whisper = self.gather_whisper_params();
        let config = self.current_config_snapshot();
        let mut language_code = imp.language_page.borrow()
            .as_ref()
            .and_then(|p| p.selected_language_code());
        // Cohere can't auto-detect — it needs an explicit language or it assumes
        // English. Fall back to the configured language (then English).
        if backend == "cohere" && language_code.is_none() {
            language_code = config.as_ref()
                .and_then(|c| c.language.clone())
                .or_else(|| Some("en".to_string()));
        }
        let selected_microphone = config.as_ref().and_then(|c| c.selected_microphone.clone());
        let mode = config.as_ref()
            .map(|c| DictationMode::from_config_str(&c.dictation_mode))
            .unwrap_or_default();
        // Merge the user's initial prompt with the personal-dictionary terms, and
        // collect the replacement rules (both via the shared config helper, so the
        // widget path matches the global-dictation path).
        let initial_prompt = config.as_ref()
            .and_then(|c| c.effective_initial_prompt())
            .or(whisper.initial_prompt);
        let replacements = config.as_ref()
            .filter(|c| c.dictionary_enabled)
            .map(|c| c.dictionary_replacements.clone())
            .unwrap_or_default();
        DictationParams {
            backend,
            language_code,
            n_threads: whisper.n_threads,
            beam_size: whisper.beam_size,
            temperature: whisper.temperature,
            translate: whisper.translate,
            initial_prompt,
            selected_microphone,
            mode,
            replacements,
        }
    }

    fn active_backend(&self) -> String {
        // The engine now lives in config (set via the Model tab's "Default Engine").
        self.current_config_snapshot()
            .map(|config| config.backend)
            .unwrap_or_else(|| "whisper".to_string())
    }

    /// Update the language display in the bottom status bar (Auto-detect / chosen).
    fn set_language_display(&self, text: &str) {
        if let Some(sb) = self.imp().status_bar.borrow().as_ref() {
            sb.set_language(text);
        }
    }

    fn ellipsize_chars(text: &str, max_chars: usize) -> String {
        let mut chars = text.chars();
        let truncated: String = chars.by_ref().take(max_chars).collect();
        if chars.next().is_some() {
            format!("{}…", truncated)
        } else {
            truncated
        }
    }

    fn replace_config(&self, config: AppConfig) {
        *self.imp().config.borrow_mut() = Some(Arc::new(config));
    }

    fn sync_ui_from_config(&self) {
        let Some(config) = self.current_config_snapshot() else {
            return;
        };

        let downloaded = Self::downloaded_model_entries();

        // Reflect the engine in the Model tab's "Default Engine" combo (guarded
        // so it doesn't re-fire the change handler).
        if let Some(model_page) = self.imp().model_page.borrow().as_ref() {
            self.imp().syncing_backend.set(true);
            model_page.set_engine(&config.backend);
            self.imp().syncing_backend.set(false);
        }

        if let Some(header) = self.imp().header_controls.borrow().as_ref() {
            // Guard the whole rebuild+restore. `update_models_for_backend` calls
            // `set_selected(0)` internally, which fires the model-changed handler
            // synchronously; without the guard it would clobber
            // `config.selected_model` with the first entry in the list (e.g.
            // "tiny") instead of the model the user actually chose.
            self.imp().syncing_dropdown.set(true);
            header.update_models_for_backend(&config.backend, &downloaded);

            if config.auto_detect_language {
                self.set_language_display("Auto-detect");
            } else if let Some(language) = config.language.as_deref() {
                self.set_language_display(&language_code_to_name(language));
            }

            if config.backend == "cohere" {
                header.set_selected_model(0);
                header.set_model_status(crate::transcription::cohere::cohere_ready(), "cohere-transcribe");
            } else if config.backend == "qwen" {
                // If the active size isn't downloaded but the other is, switch the
                // active size to a downloaded one (so the dropdown + status agree).
                let downloaded: Vec<String> = ["0.6B", "1.7B"]
                    .into_iter()
                    .filter(|s| crate::transcription::qwen::is_model_downloaded_size(s))
                    .map(|s| s.to_string())
                    .collect();
                let active = if downloaded.iter().any(|s| s == &config.qwen_model_size) {
                    config.qwen_model_size.clone()
                } else if let Some(first) = downloaded.first() {
                    let mut corrected = config.clone();
                    corrected.qwen_model_size = first.clone();
                    corrected.save();
                    self.replace_config(corrected);
                    first.clone()
                } else {
                    config.qwen_model_size.clone()
                };
                header.set_selected_model_by_id(&active);
                header.set_model_status(crate::transcription::qwen::qwen_ready(), "qwen3-asr");
            } else {
                let index = downloaded.iter()
                    .position(|(id, _)| id == &config.selected_model)
                    .unwrap_or(0) as u32;
                header.set_selected_model(index);
                header.set_model_status(false, "");
            }
            self.imp().syncing_dropdown.set(false);
        }

        self.apply_backend_capabilities(&config.backend);

        if let Some(status_bar) = self.imp().status_bar.borrow().as_ref() {
            if config.backend == "cohere" {
                status_bar.set_model_name(&gettext("Cohere Transcribe"));
                status_bar.set_recording_status(if crate::transcription::cohere::cohere_ready() {
                    "Ready"
                } else {
                    "Setup Required"
                });
            } else if config.backend == "qwen" {
                status_bar.set_model_name(&gettext("Qwen3-ASR"));
                status_bar.set_recording_status(if crate::transcription::qwen::qwen_ready() {
                    "Ready"
                } else {
                    "Setup Required"
                });
            } else if ModelCatalog::is_downloaded(&config.selected_model) {
                status_bar.set_model_name(&config.selected_model);
                status_bar.set_recording_status(&gettext("Loading model…"));
            } else {
                status_bar.set_model_name(&gettext("No model selected"));
                status_bar.set_recording_status(&gettext("No Model"));
            }
        }
    }

    /// Toggle sidebar between expanded and collapsed widths.
    fn toggle_sidebar(&self) {
        let imp = self.imp();

        let is_collapsed = imp.sidebar_collapsed.get();
        let new_collapsed = !is_collapsed;
        imp.sidebar_collapsed.set(new_collapsed);

        if let Some(sidebar_box) = imp.sidebar_box.borrow().as_ref() {
            if new_collapsed {
                sidebar_box.set_width_request(Self::SIDEBAR_COLLAPSED_WIDTH);
                sidebar_box.add_css_class("sidebar-collapsed");
            } else {
                sidebar_box.set_width_request(Self::SIDEBAR_EXPANDED_WIDTH);
                sidebar_box.remove_css_class("sidebar-collapsed");
            }
        }

        // Hide/show sidebar title
        if let Some(title_box) = imp.sidebar_title_box.borrow().as_ref() {
            title_box.set_visible(!new_collapsed);
        }

        // Hide/show settings header label
        if let Some(label) = imp.settings_header_label.borrow().as_ref() {
            label.set_visible(!new_collapsed);
        }

        // Hide/show navigation labels and adjust box alignment
        for label in imp.nav_labels.borrow().iter() {
            label.set_visible(!new_collapsed);
        }

        for hbox in imp.nav_boxes.borrow().iter() {
            if new_collapsed {
                hbox.set_margin_start(0);
                hbox.set_margin_end(0);
                hbox.set_spacing(0);
                hbox.set_halign(gtk::Align::Center);
            } else {
                hbox.set_margin_start(12);
                hbox.set_margin_end(12);
                hbox.set_spacing(12);
                hbox.set_halign(gtk::Align::Fill);
            }
        }

        // Hide/show info box at bottom
        if let Some(info_box) = imp.info_box.borrow().as_ref() {
            info_box.set_visible(!new_collapsed);
        }

        // Hide/show GPU panel
        if let Some(gpu_panel) = imp.gpu_panel.borrow().as_ref() {
            gpu_panel.set_visible(!new_collapsed);
        }

        // Update toggle button
        if let Some(btn) = imp.sidebar_toggle_btn.borrow().as_ref() {
            if new_collapsed {
                btn.set_tooltip_text(Some("Expand sidebar"));
                btn.set_icon_name("sidebar-show-right-symbolic");
            } else {
                btn.set_tooltip_text(Some("Collapse sidebar"));
                btn.set_icon_name("sidebar-show-symbolic");
            }
        }
    }

    /// Connect record / pause / stop / copy / clear / save buttons to real actions.
    fn setup_controls(&self) {
        let imp = self.imp();
        let controls = match imp.controls.borrow().as_ref() {
            Some(c) => c.clone(),
            None => return,
        };
        let window = self.clone();

        controls.connect_action(move |action| {
            match action {
                ControlAction::Record => window.on_record(),
                ControlAction::Pause => window.on_pause(),
                ControlAction::Resume => window.on_resume(),
                ControlAction::Stop => window.on_stop(),
                ControlAction::Cancel => window.on_cancel(),
                ControlAction::Copy => window.on_copy(),
                ControlAction::Clear => window.on_clear(),
                ControlAction::Save => window.on_save(),
            }
        });
    }

    /// Wire drag-and-drop of audio files onto the transcript view.
    fn setup_file_drop(&self) {
        let imp = self.imp();
        let tv = match imp.transcript_view.borrow().as_ref() {
            Some(tv) => tv.clone(),
            None => return,
        };

        let window = self.clone();
        tv.connect_file_dropped(move |path| {
            window.transcribe_file(path);
        });
    }

    /// Transcribe an audio file dropped onto the view.
    /// Gather the current Whisper inference parameters from the settings widgets.
    /// Shared by the live-recording and file-drop transcription paths.
    fn gather_whisper_params(&self) -> WhisperRunParams {
        let imp = self.imp();
        let n_threads = imp.performance_page.borrow()
            .as_ref()
            .map(|p| p.get_thread_count())
            .unwrap_or(num_cpus::get().min(8) as u32);
        let beam_size = imp.performance_page.borrow()
            .as_ref()
            .map(|p| p.get_beam_size())
            .unwrap_or(5);
        let temperature = imp.performance_page.borrow()
            .as_ref()
            .map(|p| p.get_temperature())
            .unwrap_or(0.0);
        let translate = imp.controls.borrow()
            .as_ref()
            .map(|c| c.is_translate_active())
            .unwrap_or_else(|| {
                imp.language_page.borrow()
                    .as_ref()
                    .map(|p| p.is_translate_enabled())
                    .unwrap_or(false)
            });
        let initial_prompt = imp.performance_page.borrow()
            .as_ref()
            .and_then(|p| p.get_initial_prompt());
        WhisperRunParams { n_threads, beam_size, temperature, translate, initial_prompt }
    }

    fn transcribe_file(&self, path: std::path::PathBuf) {
        let imp = self.imp();

        if let Some(sb) = imp.status_bar.borrow().as_ref() {
            sb.set_recording_status(&gettext("Decoding file…"));
        }

        let backend = self.active_backend();

        // Backend readiness up front (matches the live path's UX).
        if backend == "cohere" && !crate::transcription::cohere::cohere_ready() {
            self.show_toast(&gettext("Cohere is not set up. Go to Settings → Model to download the runtime and model."));
            return;
        }
        if backend == "qwen" && !crate::transcription::qwen::qwen_ready() {
            self.show_toast(&gettext("Qwen3-ASR is not set up. Go to Settings → Model to download the runtime and model."));
            return;
        }

        let Some(controller) = self.controller() else {
            return;
        };
        let engine = controller.engine_arc();
        let params = self.build_dictation_params(backend);

        // Show the decode-sweep animation while the file is transcribed.
        if let Some(tv) = imp.transcript_view.borrow().as_ref() {
            tv.start_transcribing_anim();
        }

        let (sender, receiver) = async_channel::bounded::<Result<crate::recording::DictationOutcome, String>>(1);

        // Decode the file and run the shared transcription core on a worker thread.
        std::thread::spawn(move || {
            let audio_data = match crate::audio::file_decoder::decode_audio_file(&path) {
                Ok(data) => data,
                Err(e) => {
                    let _ = sender.send_blocking(Err(format!("Failed to decode file: {}", e)));
                    return;
                }
            };
            // No mic time for a file: derive duration from the decoded mono-16k length.
            let duration_secs = audio_data.len() as f32 / 16_000.0;
            let result = crate::recording::run_transcription(&engine, &audio_data, &params, duration_secs);
            let _ = sender.send_blocking(result);
        });

        let window = self.clone();
        glib::spawn_future_local(async move {
            if let Ok(result) = receiver.recv().await {
                if let Some(tv) = window.imp().transcript_view.borrow().as_ref() {
                    tv.stop_transcribing_anim();
                }
                match result {
                    Ok(outcome) => {
                        let duration_secs = outcome.duration_secs;
                        let confidence = outcome.confidence;
                        let detected = outcome.detected_language.clone();
                        let segments = outcome.segments;
                        let cleaned = outcome.cleaned_text;
                        if cleaned.is_empty() {
                            *window.imp().last_segments.borrow_mut() = Vec::new();
                            if let Some(sb) = window.imp().status_bar.borrow().as_ref() {
                                sb.set_recording_status(&gettext("Ready"));
                            }
                            window.show_toast(&gettext("No clear speech detected in the audio"));
                            return;
                        }
                        if let Some(sb) = window.imp().status_bar.borrow().as_ref() {
                            sb.set_recording_status(&gettext("Ready"));
                        }
                        *window.imp().last_segments.borrow_mut() = segments.clone();
                        // Surface the auto-detected language (only set when auto-detect was used).
                        if let Some(code) = detected.as_deref() {
                            window.set_language_display(&format!("Auto-detect ({})", language_code_to_name(code)));
                        }
                        // Render as the current result (text + confidence + stats +
                        // clipboard + chips/selector).
                        let state = crate::ui::result_state::ResultState::new(
                            cleaned.clone(), duration_secs, detected, segments,
                        );
                        window.add_result_message(state, confidence as f64);
                        window.show_toast(&gettext("File transcription complete"));
                        window.maybe_generate_summary(&cleaned, duration_secs);
                        window.maybe_auto_improve(&cleaned);
                    }
                    Err(msg) => {
                        window.show_toast(&msg);
                        if let Some(sb) = window.imp().status_bar.borrow().as_ref() {
                            sb.set_recording_status(&gettext("Error"));
                        }
                    }
                }
            }
        });
    }

    /// Wire the "Default Engine" combo in the Model tab: switching it updates the
    /// header model dropdown, status bar, capability gating, and model loading.
    fn setup_engine_selector(&self) {
        let imp = self.imp();

        // Initial sync of the header model list + capabilities from config.
        // Guarded so the internal `set_selected(0)` never persists over the
        // saved model (the model-changed handler may already be connected on
        // later calls / reorderings).
        if let Some(config) = imp.config.borrow().as_ref() {
            let backend = config.backend.clone();
            if let Some(header) = imp.header_controls.borrow().as_ref() {
                let downloaded = Self::downloaded_model_entries();
                imp.syncing_dropdown.set(true);
                header.update_models_for_backend(&backend, &downloaded);
                imp.syncing_dropdown.set(false);
            }
            self.apply_backend_capabilities(&backend);
        }

        let model_page = match imp.model_page.borrow().as_ref() {
            Some(p) => p.clone(),
            None => return,
        };

        let window = self.clone();
        model_page.connect_engine_changed(move |backend| {
            if window.imp().syncing_backend.get() {
                return;
            }

            if let Some(mut new_config) = window.current_config_snapshot() {
                new_config.backend = backend.clone();
                new_config.save();
                window.replace_config(new_config);
            }

            if backend != "whisper" {
                // Non-Whisper backends (Cohere, Qwen3-ASR) don't use the loaded
                // Whisper engine — drop it to free memory.
                if let Some(engine_arc) = window.imp().engine.borrow().as_ref().cloned() {
                    if let Ok(mut guard) = engine_arc.lock() {
                        *guard = None;
                    }
                }
                tracing::info!("Switched transcription engine to {}", backend);
            } else {
                tracing::info!("Switched transcription engine to Whisper");
            }

            window.sync_ui_from_config();

            if backend == "whisper" {
                if let Some(config) = window.current_config_snapshot() {
                    window.load_model_by_id(&config.selected_model);
                }
            }
        });
    }

    /// Apply UI capability constraints based on the selected backend.
    fn apply_backend_capabilities(&self, backend: &str) {
        let imp = self.imp();
        let is_whisper = backend == "whisper";
        // Qwen3-ASR auto-detects the language; Cohere does not.
        let supports_auto_detect = backend == "whisper" || backend == "qwen";

        // Controls: translate toggle (Whisper-only) — greyed out, not hidden.
        if let Some(controls) = imp.controls.borrow().as_ref() {
            controls.set_translate_enabled(is_whisper);
        }

        // Language page: auto-detect and translation
        if let Some(page) = imp.language_page.borrow().as_ref() {
            page.set_auto_detect_available(supports_auto_detect);
            page.set_translation_available(is_whisper);
        }

        // Performance page: Whisper-specific settings
        if let Some(page) = imp.performance_page.borrow().as_ref() {
            page.set_whisper_settings_available(is_whisper);
        }
    }

    /// Get downloaded Whisper models as (id, display_name) pairs for the dropdown.
    fn downloaded_model_entries() -> Vec<(String, String)> {
        let catalog = ModelCatalog::new();
        let mut entries = Vec::new();
        // Check every model in the catalog individually
        for model in catalog.models() {
            if ModelCatalog::is_downloaded(&model.id) {
                entries.push((model.id.clone(), model.display_name.clone()));
            }
        }
        entries
    }

    /// Wire model dropdown changes: when user selects a model, load it.
    fn setup_model_dropdown(&self) {
        let imp = self.imp();
        let header = match imp.header_controls.borrow().as_ref() {
            Some(h) => h.clone(),
            None => return,
        };

        // Sync dropdown with config — find the current model's index in the downloaded list
        if let Some(config) = imp.config.borrow().as_ref() {
            let downloaded = Self::downloaded_model_entries();
            let index = downloaded.iter()
                .position(|(id, _)| id == &config.selected_model)
                .unwrap_or(0) as u32;
            imp.syncing_dropdown.set(true);
            header.set_selected_model(index);
            imp.syncing_dropdown.set(false);
        }

        // Connect dropdown selection change
        let window = self.clone();
        header.connect_model_changed(move |model_id| {
            // Skip if this change was triggered programmatically
            if window.imp().syncing_dropdown.get() {
                return;
            }

            let backend = window
                .current_config_snapshot()
                .map(|c| c.backend)
                .unwrap_or_else(|| "whisper".to_string());

            // For Qwen the dropdown selects the model SIZE ("0.6B"/"1.7B").
            if backend == "qwen" {
                if let Some(mut cfg) = window.current_config_snapshot() {
                    cfg.qwen_model_size = model_id.clone();
                    cfg.save();
                    window.replace_config(cfg);
                }
                // Refresh the ready/status indicator for the newly active size.
                window.sync_ui_from_config();
                return;
            }

            // Cohere has no selectable model.
            if backend != "whisper" {
                return;
            }

            // Whisper: the model_id is the resolved downloaded model ID.
            if let Some(mut new_config) = window.current_config_snapshot() {
                new_config.selected_model = model_id.clone();
                new_config.save();
                window.replace_config(new_config);
            }
            window.load_model_by_id(&model_id);
        });
    }

    /// Wire language settings changes to update the header language display.
    fn setup_language_display(&self) {
        let imp = self.imp();

        let language_page = match imp.language_page.borrow().as_ref() {
            Some(p) => p.clone(),
            None => return,
        };

        // Set initial language display from config
        if let Some(config) = imp.config.borrow().as_ref() {
            if config.auto_detect_language {
                self.set_language_display("Auto-detect");
            } else if let Some(ref lang) = config.language {
                self.set_language_display(&language_code_to_name(lang));
            }
        }

        // Connect language page changes
        let window = self.clone();
        language_page.connect_language_changed(move |lang_name| {
            window.set_language_display(&lang_name);
        });

        // Sync translate toggle between controls and language page
        let controls = match imp.controls.borrow().as_ref() {
            Some(c) => c.clone(),
            None => return,
        };

        // Translate is shared by the main window AND the mini panel / global
        // dictation, so persist it to config and keep both toggles in sync. A
        // guard prevents the two `set_*` calls from ping-ponging.
        let window = self.clone();
        let lang_page_ref = language_page.clone();
        controls.connect_translate_changed(move |active| {
            if window.imp().syncing_translate.get() {
                return;
            }
            window.imp().syncing_translate.set(true);
            lang_page_ref.set_translate_enabled(active);
            window.imp().syncing_translate.set(false);
            let mut c = AppConfig::load();
            c.translate_to_english = active;
            c.save();
        });

        let window = self.clone();
        let controls_ref = controls.clone();
        language_page.connect_translate_changed(move |active| {
            if window.imp().syncing_translate.get() {
                return;
            }
            window.imp().syncing_translate.set(true);
            controls_ref.set_translate_active(active);
            window.imp().syncing_translate.set(false);
            let mut c = AppConfig::load();
            c.translate_to_english = active;
            c.save();
        });

        // Restore the saved translate state into both toggles (guarded so it
        // doesn't re-trigger a save).
        if let Some(config) = imp.config.borrow().as_ref() {
            imp.syncing_translate.set(true);
            controls.set_translate_active(config.translate_to_english);
            language_page.set_translate_enabled(config.translate_to_english);
            imp.syncing_translate.set(false);
        }
    }

    fn on_record(&self) {
        let imp = self.imp();
        let Some(controller) = self.controller() else {
            return;
        };

        // Don't start if another owner (e.g. global dictation) is recording.
        if !controller.try_acquire(RecordingOwner::Main) {
            return;
        }

        // Set up waveform channel for UI visualization.
        let (waveform_tx, waveform_rx) = async_channel::bounded::<Vec<f32>>(32);

        // AudioCapture contains a cpal::Stream which is !Send, but start is fast
        // (sets up the stream), so it runs on the main thread via the controller.
        let selected_device = self.current_config_snapshot()
            .and_then(|config| config.selected_microphone);
        let result = controller.start(selected_device.as_deref(), waveform_tx);

        match result {
            Ok(()) => {
                if let Some(controls) = imp.controls.borrow().as_ref() {
                    controls.set_recording_state(true);
                }
                if let Some(tv) = imp.transcript_view.borrow().as_ref() {
                    tv.set_recording(true);
                    // Hide the previous result's chips/selector while recording.
                    tv.hide_result_controls();
                }
                if let Some(sb) = imp.status_bar.borrow().as_ref() {
                    sb.set_recording_status(&gettext("Recording…"));
                }
                self.start_recording_timer();

                // Feed waveform data to transcript view
                let window = self.downgrade();
                glib::spawn_future_local(async move {
                    while let Ok(amplitudes) = waveform_rx.recv().await {
                        let Some(win) = window.upgrade() else { break };
                        let tv = win.imp().transcript_view.borrow().clone();
                        if let Some(tv) = tv.as_ref() {
                            tv.update_waveform(amplitudes);
                        }
                    }
                });

                // Live (while-speaking) transcription: periodically re-decode the
                // audio captured so far and show it as tentative text. Whisper
                // only; the authoritative result is produced on Stop.
                if AppConfig::load().live_transcription && self.active_backend() == "whisper" {
                    self.imp().live_decoding.set(false);
                    self.imp().live_too_slow.set(false);
                    self.start_live_loop();
                }

                tracing::info!("Recording started");
            }
            Err(e) => {
                controller.release();
                self.show_toast(&format!("Failed to start recording: {}", e));
            }
        }
    }

    /// Periodic live decode of the audio captured so far, shown as tentative text
    /// while recording continues. Greedy (beam=1) + plain formatting for low
    /// latency; never overlaps itself; stops when recording ends. The final,
    /// authoritative result is produced by `on_stop`.
    fn start_live_loop(&self) {
        let window = self.downgrade();
        glib::timeout_add_local(std::time::Duration::from_millis(2000), move || {
            let Some(win) = window.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let Some(controller) = win.controller() else {
                return glib::ControlFlow::Break;
            };
            // Stop once recording ends (or another owner took over).
            if controller.owner() != RecordingOwner::Main
                || controller.state() == crate::audio::capture::RecordingState::Idle
            {
                return glib::ControlFlow::Break;
            }
            // Self-protect: if a previous live decode was too slow or failed, stop
            // the loop so we never flood a slow / VRAM-tight GPU (e.g. large-v3).
            // The final on-stop decode still produces the full transcript.
            if win.imp().live_too_slow.get() {
                return glib::ControlFlow::Break;
            }
            // Skip while a previous live decode is still running.
            if win.imp().live_decoding.get() {
                return glib::ControlFlow::Continue;
            }
            let mut snapshot = controller.live_snapshot();
            // Need at least ~1s of conditioned audio to bother.
            if snapshot.len() < 16_000 {
                return glib::ControlFlow::Continue;
            }
            // Bound cost: decode only the most recent ~12s window. The final
            // on-stop pass still decodes the whole take at full quality.
            const LIVE_WINDOW: usize = 12 * 16_000;
            if snapshot.len() > LIVE_WINDOW {
                snapshot = snapshot.split_off(snapshot.len() - LIVE_WINDOW);
            }
            win.imp().live_decoding.set(true);
            let mut params = DictationParams::from_config(&AppConfig::load());
            params.beam_size = 1; // greedy: fast enough to keep up
            params.mode = DictationMode::Plain; // show raw-ish text live
            let dur = controller.recording_duration_secs();
            let started = std::time::Instant::now();
            let rx = controller.transcribe_async(snapshot, params, dur);
            let win2 = win.downgrade();
            glib::spawn_future_local(async move {
                let res = rx.recv().await;
                let Some(win) = win2.upgrade() else { return };
                win.imp().live_decoding.set(false);
                match res {
                    Ok(Ok(outcome)) => {
                        // If even a 12s greedy decode is slow, this hardware/model
                        // can't keep up live — disable the loop for this recording.
                        if started.elapsed().as_secs_f32() > 3.5 {
                            win.imp().live_too_slow.set(true);
                        }
                        let still_recording = win
                            .controller()
                            .map(|c| c.owner() == RecordingOwner::Main)
                            .unwrap_or(false);
                        if !still_recording {
                            return;
                        }
                        let text = outcome.cleaned_text;
                        if !text.trim().is_empty() {
                            if let Some(tv) = win.imp().transcript_view.borrow().as_ref() {
                                tv.set_live_preview(&text);
                            }
                        }
                    }
                    // Any failure (e.g. "failed to encode" on a VRAM-tight GPU):
                    // stop the live loop silently so it can't repeat.
                    _ => win.imp().live_too_slow.set(true),
                }
            });
            glib::ControlFlow::Continue
        });
    }

    fn on_pause(&self) {
        let imp = self.imp();
        if let Some(audio) = imp.audio_capture.borrow().as_ref() {
            if let Ok(mut cap) = audio.lock() {
                cap.pause();
            }
        }
        if let Some(controls) = imp.controls.borrow().as_ref() {
            controls.set_paused_state(true);
        }
        if let Some(sb) = imp.status_bar.borrow().as_ref() {
            sb.set_recording_status(&gettext("Paused"));
        }
    }

    fn on_resume(&self) {
        let imp = self.imp();
        if let Some(audio) = imp.audio_capture.borrow().as_ref() {
            if let Ok(mut cap) = audio.lock() {
                cap.resume();
            }
        }
        if let Some(controls) = imp.controls.borrow().as_ref() {
            controls.set_paused_state(false);
        }
        if let Some(sb) = imp.status_bar.borrow().as_ref() {
            sb.set_recording_status(&gettext("Recording…"));
        }
    }

    fn on_stop(&self) {
        let imp = self.imp();
        let Some(controller) = self.controller() else {
            return;
        };

        // Capture the mic duration BEFORE stopping (stop drains the buffer, and
        // silence-trimming makes the conditioned sample count shorter than the
        // real recording time). Needed for words-per-minute stats.
        let duration_secs = controller.recording_duration_secs();

        // Stop recording and get audio data.
        let audio_data = match controller.stop() {
            Ok(data) => data,
            Err(e) => {
                controller.release();
                self.show_toast(&format!("Error stopping recording: {}", e));
                return;
            }
        };
        controller.release();

        // Update UI
        if let Some(controls) = imp.controls.borrow().as_ref() {
            controls.set_recording_state(false);
            controls.set_paused_state(false);
        }
        if let Some(tv) = imp.transcript_view.borrow().as_ref() {
            tv.set_recording(false);
        }

        if audio_data.is_empty() {
            self.show_toast(&gettext("No clear speech detected — try speaking closer to the microphone"));
            if let Some(sb) = imp.status_bar.borrow().as_ref() {
                sb.set_recording_status(&gettext("Ready"));
            }
            return;
        }

        // Determine backend
        let backend = self.active_backend();

        if let Some(sb) = imp.status_bar.borrow().as_ref() {
            sb.set_recording_status(match backend.as_str() {
                "cohere" => "Transcribing with Cohere…",
                "qwen" => "Transcribing with Qwen3-ASR…",
                _ => "Transcribing with Whisper…",
            });
        }

        // Backend readiness is checked up front so we can show the dedicated
        // "Setup Required" status (the controller also guards internally).
        if backend == "cohere" && !crate::transcription::cohere::cohere_ready() {
            self.show_toast(&gettext("Cohere is not set up. Go to Settings → Model to download the runtime and model."));
            if let Some(sb) = imp.status_bar.borrow().as_ref() {
                sb.set_recording_status(&gettext("Setup Required"));
            }
            return;
        }
        if backend == "qwen" && !crate::transcription::qwen::qwen_ready() {
            self.show_toast(&gettext("Qwen3-ASR is not set up. Go to Settings → Model to download the runtime and model."));
            if let Some(sb) = imp.status_bar.borrow().as_ref() {
                sb.set_recording_status(&gettext("Setup Required"));
            }
            return;
        }

        tracing::info!("Using {} backend for live transcription", backend);
        // Show the decode-sweep animation while transcription runs.
        if let Some(tv) = imp.transcript_view.borrow().as_ref() {
            tv.start_transcribing_anim();
        }
        // Final, authoritative decode. (When live transcription is on, the live
        // preview already ran *during* recording via the live loop; here we always
        // use the clean batch path so the result + chips/toggle/stats render
        // reliably — the previous on-stop streaming path could hide the chips with
        // a late segment update.)
        let params = self.build_dictation_params(backend);
        let receiver = controller.transcribe_async(audio_data, params, duration_secs);
        let window = self.clone();
        glib::spawn_future_local(async move {
            if let Ok(result) = receiver.recv().await {
                window.apply_transcription_result(result);
            }
        });
    }

    /// Finalize a live transcription result: render it, save to history, and run
    /// the summary / auto-improve passes. Shared by the streaming and batch paths.
    fn apply_transcription_result(&self, result: Result<crate::recording::DictationOutcome, String>) {
        if let Some(tv) = self.imp().transcript_view.borrow().as_ref() {
            tv.stop_transcribing_anim();
        }
        match result {
            Ok(outcome) => {
                let duration_secs = outcome.duration_secs;
                let confidence = outcome.confidence;
                let detected = outcome.detected_language.clone();
                let segments = outcome.segments;
                let cleaned = outcome.cleaned_text;
                if cleaned.is_empty() {
                    *self.imp().last_segments.borrow_mut() = Vec::new();
                    if let Some(sb) = self.imp().status_bar.borrow().as_ref() {
                        sb.set_recording_status(&gettext("Ready"));
                    }
                    self.show_toast(&gettext("No clear speech detected — try again in a quieter environment"));
                    return;
                }
                if let Some(sb) = self.imp().status_bar.borrow().as_ref() {
                    sb.set_recording_status(&gettext("Ready"));
                }
                // Store segments for SRT export
                *self.imp().last_segments.borrow_mut() = segments.clone();
                // Surface the auto-detected language (only set when auto-detect was used).
                if let Some(code) = detected.as_deref() {
                    self.set_language_display(&format!("Auto-detect ({})", language_code_to_name(code)));
                }
                // Render as the current result (text + confidence + stats +
                // clipboard + chips/selector).
                let state = crate::ui::result_state::ResultState::new(
                    cleaned.clone(), duration_secs, detected, segments,
                );
                self.add_result_message(state, confidence as f64);
                // Add to history
                if let Some(hp) = self.imp().history_page.borrow().as_ref() {
                    let lang_name = self.imp().language_page.borrow()
                        .as_ref()
                        .map(|p| p.selected_language_name())
                        .unwrap_or_else(|| "Auto-detect".to_string());
                    let model_name = self.imp().header_controls.borrow()
                        .as_ref()
                        .map(|h| h.selected_model_id())
                        .unwrap_or_else(|| "unknown".to_string());
                    let title = Self::ellipsize_chars(&cleaned, 60);
                    let entry_id = uuid::Uuid::new_v4().to_string();
                    let entry = crate::ui::history_page::HistoryEntry {
                        id: entry_id.clone(),
                        title,
                        text: cleaned.clone(),
                        language: lang_name,
                        duration_secs: duration_secs.round() as u64,
                        timestamp: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
                        model: model_name,
                        word_count: Some(crate::ui::result_state::word_count(&cleaned) as u32),
                        ..Default::default()
                    };
                    hp.add_entry(entry);
                    // LLM auto-title (best effort) via the Application.
                    if let Some(app) = self
                        .application()
                        .and_downcast::<crate::application::Application>()
                    {
                        app.auto_title(entry_id, cleaned.clone());
                    }
                }
                tracing::info!("Transcription complete ({:.0}% confidence), copied to clipboard", confidence * 100.0);
                // Summarize long dictations; auto-improve with the active preset
                // (adds a variant; the raw transcript stays available + in History).
                self.maybe_generate_summary(&cleaned, duration_secs);
                self.maybe_auto_improve(&cleaned);
            }
            Err(msg) => {
                self.show_toast(&msg);
                if let Some(sb) = self.imp().status_bar.borrow().as_ref() {
                    sb.set_recording_status(&gettext("Error"));
                }
            }
        }
    }

    fn on_cancel(&self) {
        let imp = self.imp();
        let Some(controller) = self.controller() else {
            return;
        };

        // Stop recording and discard the audio data.
        controller.cancel();
        controller.release();

        // Reset UI
        if let Some(controls) = imp.controls.borrow().as_ref() {
            controls.set_recording_state(false);
            controls.set_paused_state(false);
        }
        if let Some(tv) = imp.transcript_view.borrow().as_ref() {
            tv.set_recording(false);
        }
        if let Some(sb) = imp.status_bar.borrow().as_ref() {
            sb.set_recording_status(&gettext("Ready"));
        }

        self.show_toast(&gettext("Recording cancelled"));
        tracing::info!("Recording cancelled by user");
    }

    fn on_copy(&self) {
        let imp = self.imp();
        if let Some(tv) = imp.transcript_view.borrow().as_ref() {
            let text = tv.get_text();
            if !text.is_empty() {
                if let Some(display) = gtk::gdk::Display::default() {
                    display.clipboard().set_text(&text);
                    self.show_toast(&gettext("Copied to clipboard"));
                }
            }
        }
    }

    fn on_clear(&self) {
        self.imp().messages.borrow_mut().clear();
        self.imp().selected_message.set(-1);
        if let Some(tv) = self.imp().transcript_view.borrow().as_ref() {
            tv.clear();
        }
    }

    fn on_save(&self) {
        let imp = self.imp();
        let text = match imp.transcript_view.borrow().as_ref() {
            Some(tv) => tv.get_text(),
            None => return,
        };
        if text.is_empty() {
            self.show_toast(&gettext("Nothing to save"));
            return;
        }

        let segments = imp.last_segments.borrow().clone();

        let txt_filter = gtk::FileFilter::new();
        txt_filter.set_name(Some("Text file (.txt)"));
        txt_filter.add_pattern("*.txt");

        let srt_filter = gtk::FileFilter::new();
        srt_filter.set_name(Some("SRT subtitles (.srt)"));
        srt_filter.add_pattern("*.srt");

        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&txt_filter);
        filters.append(&srt_filter);

        let dialog = gtk::FileDialog::builder()
            .title("Save Transcript")
            .initial_name("transcript.txt")
            .filters(&filters)
            .build();

        let window = self.clone();
        dialog.save(Some(&window.clone()), gtk::gio::Cancellable::NONE, move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    let content = if path.extension().is_some_and(|e| e == "srt") && !segments.is_empty() {
                        let seg_refs: Vec<(i64, i64, &str)> = segments.iter()
                            .map(|(s, e, t)| (*s, *e, t.as_str()))
                            .collect();
                        postprocess::format_as_srt(&seg_refs)
                    } else {
                        text.clone()
                    };
                    match std::fs::write(&path, &content) {
                        Ok(()) => window.show_toast(&gettext("Transcript saved")),
                        Err(e) => window.show_toast(&format!("Failed to save: {}", e)),
                    }
                }
            }
        });
    }

    /// Start a recording timer that updates the transcript view every second.
    fn start_recording_timer(&self) {
        let window = self.downgrade();
        glib::timeout_add_seconds_local(1, move || {
            let Some(window) = window.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let imp = window.imp();
            let audio = match imp.audio_capture.borrow().as_ref() {
                Some(a) => a.clone(),
                None => return glib::ControlFlow::Break,
            };
            match audio.lock() {
                Ok(cap) => {
                    if cap.state() == crate::audio::capture::RecordingState::Idle {
                        return glib::ControlFlow::Break;
                    }
                    let secs = cap.recording_duration_secs() as u64;
                    if let Some(tv) = imp.transcript_view.borrow().as_ref() {
                        tv.set_timer(secs);
                    }
                }
                Err(_) => return glib::ControlFlow::Break,
            }
            glib::ControlFlow::Continue
        });
    }

    fn create_nav_row(&self, nav_item: NavItem) -> (gtk::ListBoxRow, gtk::Label, gtk::Box) {
        let row = gtk::ListBoxRow::new();
        row.set_selectable(true);
        row.set_tooltip_text(Some(&nav_item.title()));

        let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        hbox.set_margin_top(8);
        hbox.set_margin_bottom(8);
        hbox.set_margin_start(12);
        hbox.set_margin_end(12);
        hbox.add_css_class("nav-row-box");

        let icon = gtk::Image::from_icon_name(nav_item.icon_name());
        icon.set_pixel_size(20);
        hbox.append(&icon);

        let label = gtk::Label::new(Some(&nav_item.title()));
        label.set_halign(gtk::Align::Start);
        label.set_hexpand(true);
        label.add_css_class("nav-label");
        hbox.append(&label);

        row.set_child(Some(&hbox));
        (row, label, hbox)
    }

    /// Navigate to a page.
    pub fn navigate_to(&self, nav_item: NavItem) {
        let imp = self.imp();

        let page_name = match nav_item {
            NavItem::Transcription => "transcription",
            NavItem::History => "history",
            NavItem::Microphone => "microphone",
            NavItem::Model => "model",
            NavItem::Language => "language",
            NavItem::Performance => "performance",
            NavItem::Dictation => "dictation",
            NavItem::Dictionary => "dictionary",
            NavItem::Llm => "llm",
            NavItem::Help => "help",
        };

        if let Some(stack) = imp.content_stack.borrow().as_ref() {
            stack.set_visible_child_name(page_name);
        }

        imp.current_nav.set(Some(nav_item));

        // The LLM enable switch may have changed; keep the AI button in sync.
        self.refresh_ai_button();

        // Returning to Transcription with a non-Whisper backend: re-sync so the
        // model dropdown reflects any size just downloaded in Settings (Qwen).
        if matches!(nav_item, NavItem::Transcription)
            && self
                .current_config_snapshot()
                .map(|c| c.backend != "whisper")
                .unwrap_or(false)
        {
            self.sync_ui_from_config();
        }
    }

    fn setup_actions(&self) {
        // Quit
        let action_quit = gio::ActionEntry::builder("quit")
            .activate(|window: &Self, _, _| {
                window.close();
            })
            .build();

        // Toggle sidebar
        let action_toggle_sidebar = gio::ActionEntry::builder("toggle-sidebar")
            .activate(|window: &Self, _, _| {
                window.toggle_sidebar();
            })
            .build();

        // Navigate
        let action_navigate = gio::ActionEntry::builder("navigate")
            .parameter_type(Some(&String::static_variant_type()))
            .activate(|window: &Self, _, parameter| {
                if let Some(name) = parameter.and_then(|p| p.get::<String>()) {
                    let nav_item = match name.as_str() {
                        "transcription" => NavItem::Transcription,
                        "history" => NavItem::History,
                        "microphone" => NavItem::Microphone,
                        "model" => NavItem::Model,
                        "language" => NavItem::Language,
                        "performance" => NavItem::Performance,
                        "dictation" => NavItem::Dictation,
                        "dictionary" => NavItem::Dictionary,
                        "llm" => NavItem::Llm,
                        "help" => NavItem::Help,
                        _ => return,
                    };
                    window.navigate_to(nav_item);
                }
            })
            .build();

        self.add_action_entries([action_quit, action_toggle_sidebar, action_navigate]);
    }

    /// Add a history entry through the live HistoryPage (keeps the in-memory
    /// list and disk in sync). Used by the global dictation path when the main
    /// window is open.
    pub fn add_history_entry(&self, entry: crate::ui::history_page::HistoryEntry) {
        if let Some(hp) = self.imp().history_page.borrow().as_ref() {
            hp.add_entry(entry);
        }
    }

    /// Update a history entry's title in place (used by LLM auto-title).
    pub fn history_update_title(&self, id: &str, title: &str) {
        if let Some(hp) = self.imp().history_page.borrow().as_ref() {
            hp.update_entry_title(id, title);
        }
    }

    /// Show a toast notification. Messages are redacted (no secrets/home paths)
    /// since some callers pass through error text.
    pub fn show_toast(&self, message: &str) {
        // Routed through the top in-app banner instead of a bottom OSD toast.
        self.show_banner(message, false);
    }

    /// Try to load the currently selected model in a background thread.
    fn load_selected_model(&self) {
        let config = match self.current_config_snapshot() {
            Some(config) => config,
            None => return,
        };

        if config.backend != "whisper" {
            tracing::info!("{} backend is active; skipping Whisper model preload", config.backend);
            return;
        }

        // `selected_model` is normally the exact downloaded ID, but older configs
        // (and pre-fix wizard runs) may store a base ID like "tiny" while only the
        // quantized variant "tiny-q5_1" is on disk. Resolve to an actually-present
        // model and persist the correction so the dropdown and engine stay in sync.
        let selected = config.selected_model.clone();
        if ModelCatalog::is_downloaded(&selected) {
            self.load_model_by_id(&selected);
            return;
        }

        let base = ModelCatalog::base_model_id(&selected).to_string();
        let resolved = ModelCatalog::resolve_model(&base, config.use_quantized);
        if ModelCatalog::is_downloaded(&resolved) {
            tracing::info!("Selected model '{}' missing; resolved to downloaded '{}'", selected, resolved);
            let mut corrected = config.clone();
            corrected.selected_model = resolved.clone();
            corrected.save();
            self.replace_config(corrected);
            self.sync_ui_from_config();
            self.load_model_by_id(&resolved);
        } else {
            tracing::warn!("Selected model '{}' not downloaded and no variant available", selected);
        }
    }

    /// React to a model being deleted from disk: refresh the dropdown and, if the
    /// active model was the one removed, switch to another downloaded model (or
    /// clear the engine if none remain).
    pub fn handle_model_deleted(&self, deleted_id: &str) {
        let was_active = self.current_config_snapshot()
            .map(|c| c.selected_model == deleted_id)
            .unwrap_or(false);

        if was_active {
            let downloaded = Self::downloaded_model_entries();
            if let Some((next_id, _)) = downloaded.first().cloned() {
                if let Some(mut config) = self.current_config_snapshot() {
                    config.selected_model = next_id.clone();
                    config.save();
                    self.replace_config(config);
                }
                self.load_model_by_id(&next_id);
            } else {
                // No models left — drop the loaded engine.
                if let Some(engine_arc) = self.imp().engine.borrow().as_ref() {
                    if let Ok(mut guard) = engine_arc.lock() {
                        *guard = None;
                    }
                }
                if let Some(sb) = self.imp().status_bar.borrow().as_ref() {
                    sb.set_model_name(&gettext("No model selected"));
                    sb.set_recording_status(&gettext("No Model"));
                }
            }
        }

        // Always refresh the dropdown to reflect the new downloaded set.
        self.sync_ui_from_config();
        self.show_toast(&format!("Deleted model: {}", deleted_id));
    }

    /// Load a specific model by ID in a background thread.
    /// After loading, updates header and status bar.
    pub fn load_model_by_id(&self, model_id: &str) {
        let imp = self.imp();

        if !ModelCatalog::is_downloaded(model_id) {
            tracing::warn!("Model '{}' not downloaded", model_id);
            return;
        }

        let model_path = ModelCatalog::model_path(model_id);
        let engine_arc = match imp.engine.borrow().clone() {
            Some(e) => e,
            None => return,
        };

        // Check GPU setting from performance page or config
        let use_gpu = imp.performance_page.borrow()
            .as_ref()
            .map(|p| p.get_gpu_enabled())
            .unwrap_or_else(|| self.is_gpu_mode());

        // Whether to retry on CPU if a GPU load fails.
        let cpu_fallback = imp.performance_page.borrow()
            .as_ref()
            .map(|p| p.get_cpu_fallback())
            .unwrap_or_else(|| self.current_config_snapshot().map(|c| c.cpu_fallback).unwrap_or(true));

        if let Some(sb) = imp.status_bar.borrow().as_ref() {
            // Show the selected model immediately so the status bar matches the
            // header dropdown (not just after a slow/failed load finishes).
            sb.set_model_name(model_id);
            sb.set_recording_status(&gettext("Loading model…"));
        }

        // Result carries the loaded model ID plus whether we fell back to CPU.
        let (sender, receiver) = async_channel::bounded::<Result<(String, bool), String>>(1);

        let model_id_owned = model_id.to_string();
        std::thread::spawn(move || {
            let store = |engine| {
                if let Ok(mut guard) = engine_arc.lock() {
                    *guard = Some(engine);
                }
            };

            match TranscriptionEngine::load_model_with_gpu(&model_path, &model_id_owned, use_gpu) {
                Ok(engine) => {
                    store(engine);
                    let _ = sender.send_blocking(Ok((model_id_owned, false)));
                }
                Err(e) if use_gpu && cpu_fallback => {
                    tracing::warn!("GPU model load failed ({}); retrying on CPU", e);
                    match TranscriptionEngine::load_model_with_gpu(&model_path, &model_id_owned, false) {
                        Ok(engine) => {
                            store(engine);
                            let _ = sender.send_blocking(Ok((model_id_owned, true)));
                        }
                        Err(e2) => {
                            let _ = sender.send_blocking(Err(format!(
                                "Failed to load model on GPU and CPU: {}", e2
                            )));
                        }
                    }
                }
                Err(e) => {
                    let _ = sender.send_blocking(Err(format!("Failed to load model: {}", e)));
                }
            }
        });

        let window = self.clone();
        glib::spawn_future_local(async move {
            while let Ok(result) = receiver.recv().await {
                match result {
                    Ok((mid, downgraded)) => {
                        if downgraded {
                            window.show_toast(&gettext("GPU unavailable — loaded model on CPU"));
                        }
                        window.on_model_loaded(&mid);
                    }
                    Err(msg) => window.show_toast(&msg),
                }
            }
        });
    }

    /// Called when a model finishes loading.
    fn on_model_loaded(&self, model_id: &str) {
        let backend = self.current_config_snapshot()
            .map(|config| config.backend)
            .unwrap_or_else(|| "whisper".to_string());

        if backend != "whisper" {
            tracing::info!(
                "Whisper model '{}' finished loading, but {} is the active backend; leaving its UI state unchanged",
                model_id, backend
            );
            return;
        }

        tracing::info!("Model '{}' loaded and ready", model_id);

        if let Some(header) = self.imp().header_controls.borrow().as_ref() {
            header.set_model_status(true, model_id);

            // Refresh the dropdown list (a new model may have just been
            // downloaded). Guard the rebuild so `update_models_for_backend`'s
            // internal `set_selected(0)` doesn't fire the change handler and
            // overwrite the freshly-loaded model with the first list entry.
            let downloaded = Self::downloaded_model_entries();
            self.imp().syncing_dropdown.set(true);
            header.update_models_for_backend(&backend, &downloaded);

            // Find the model's index in the refreshed list
            let index = downloaded.iter()
                .position(|(id, _)| id == model_id)
                .unwrap_or(0) as u32;
            header.set_selected_model(index);
            self.imp().syncing_dropdown.set(false);
        }

        let use_gpu = self.imp().performance_page.borrow()
            .as_ref()
            .map(|p| p.get_gpu_enabled())
            .unwrap_or_else(|| self.is_gpu_mode());

        // Set model name and initial compute mode immediately
        if let Some(status_bar) = self.imp().status_bar.borrow().as_ref() {
            status_bar.set_model_name(model_id);
            status_bar.set_compute_mode(if use_gpu { "GPU" } else { "CPU" });
            status_bar.set_recording_status(&gettext("Ready"));
        }

        // Detect GPU info asynchronously to avoid blocking the UI thread
        let (sender, receiver) = async_channel::bounded::<bool>(1);
        std::thread::spawn(move || {
            let has_gpu = crate::ui::widgets::gpu_status::detect_gpu_info().is_some();
            let _ = sender.send_blocking(has_gpu);
        });

        let window = self.clone();
        glib::spawn_future_local(async move {
            if let Ok(has_gpu) = receiver.recv().await {
                if let Some(status_bar) = window.imp().status_bar.borrow().as_ref() {
                    let use_gpu = window.imp().performance_page.borrow()
                        .as_ref()
                        .map(|p| p.get_gpu_enabled())
                        .unwrap_or_else(|| window.is_gpu_mode());
                    let compute_mode = if has_gpu || use_gpu { "GPU" } else { "CPU" };
                    status_bar.set_compute_mode(compute_mode);
                }
            }
        });
    }

    fn is_gpu_mode(&self) -> bool {
        self.imp().config.borrow()
            .as_ref()
            .map(|c| c.use_gpu)
            .unwrap_or(false)
    }

    /// Check GitHub for a newer release and show the update banner if found.
    fn check_for_updates(&self) {
        // Respect the user's choice: the startup update check is the only
        // unprompted network request the app makes, so it can be disabled.
        if !AppConfig::load().update_check_enabled {
            return;
        }

        let (sender, receiver) = async_channel::bounded::<Option<crate::version_check::UpdateInfo>>(1);

        // Spawn the async HTTP check on the Tokio runtime
        let current_version = crate::VERSION.to_string();
        crate::application::tokio_runtime().spawn(async move {
            let result = crate::version_check::check_for_update(&current_version).await;
            let _ = sender.send(result).await;
        });

        // Receive the result on the GTK main thread
        let window = self.clone();
        glib::spawn_future_local(async move {
            if let Ok(Some(info)) = receiver.recv().await {
                window.show_update_available(&info);
            }
        });
    }

    /// Make the update banner visible with the new version info.
    fn show_update_available(&self, info: &crate::version_check::UpdateInfo) {
        let imp = self.imp();
        if let Some(banner) = imp.update_banner.borrow().as_ref() {
            // Update the label text
            if let Some(child) = banner.last_child() {
                if let Ok(label) = child.downcast::<gtk::Label>() {
                    label.set_label(&format!("v{} available", info.latest_version));
                }
            }
            banner.set_visible(true);
        }
        // Also update the status bar
        if let Some(sb) = imp.status_bar.borrow().as_ref() {
            sb.show_update_available(&info.latest_version);
        }
    }
}
