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
use std::sync::{Arc, Mutex};

use crate::application::Application;
use crate::audio::AudioCapture;
use crate::config::AppConfig;
use crate::transcription::{ModelCatalog, TranscriptionEngine};
use crate::ui::{
    Controls, HeaderControls, HelpPage, HistoryPage, StatusBar, TranscriptView,
    WelcomeWizard, ControlAction,
};
use crate::ui::settings::{MicrophonePage, ModelPage, LanguagePage, PerformancePage, language_code_to_name};
use crate::ui::widgets::GpuStatusPanel;
use crate::transcription::postprocess;

/// Navigation items for the sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavItem {
    Transcription,
    History,
    Microphone,
    Model,
    Language,
    Performance,
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
            Self::Help => "help-about-symbolic",
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            Self::Transcription => "Transcription",
            Self::History => "History",
            Self::Microphone => "Microphone",
            Self::Model => "Model",
            Self::Language => "Language",
            Self::Performance => "Performance",
            Self::Help => "Help",
        }
    }

    /// Whether this is a settings page (shown under "Settings" header).
    pub fn is_settings(&self) -> bool {
        matches!(self, Self::Microphone | Self::Model | Self::Language | Self::Performance)
    }

    pub fn all() -> &'static [NavItem] {
        &[
            Self::Transcription,
            Self::History,
            Self::Microphone,
            Self::Model,
            Self::Language,
            Self::Performance,
            Self::Help,
        ]
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct MainWindow {
        pub toast_overlay: RefCell<Option<adw::ToastOverlay>>,
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
        pub syncing_dropdown: Cell<bool>,

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

        // App state
        pub config: RefCell<Option<Arc<AppConfig>>>,
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
    pub fn new(app: &Application, config: Arc<AppConfig>) -> Self {
        let window: Self = glib::Object::builder()
            .property("application", app)
            .build();

        window.set_default_size(1100, 750);
        window.set_title(Some(crate::APP_NAME));

        // Store config and initialize shared state
        let imp = window.imp();
        *imp.config.borrow_mut() = Some(config.clone());
        *imp.audio_capture.borrow_mut() = Some(Arc::new(Mutex::new(AudioCapture::new())));
        *imp.engine.borrow_mut() = Some(Arc::new(Mutex::new(None)));
        *imp.model_catalog.borrow_mut() = Some(Arc::new(ModelCatalog::new()));

        // Show welcome wizard on first run
        if config.first_run {
            let wizard = WelcomeWizard::new(&window);
            wizard.present();
        } else {
            // Try to load the selected model in the background
            window.load_selected_model();
        }

        window
    }

    fn setup_ui(&self) {
        let imp = self.imp();

        // Main horizontal layout: sidebar | separator | content
        let main_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);

        // === SIDEBAR ===
        let sidebar_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sidebar_box.set_width_request(220);
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
        content_box.set_hexpand(true);

        // Header with mic/model selectors
        let header_controls = HeaderControls::new();
        content_box.append(&header_controls);

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

        // Wire model dropdown selection change
        self.setup_model_dropdown();

        // Wire language changes to header
        self.setup_language_display();

        // Check for updates at startup
        self.check_for_updates();
    }

    /// Toggle sidebar between expanded (250px) and collapsed (50px).
    fn toggle_sidebar(&self) {
        let imp = self.imp();

        let is_collapsed = imp.sidebar_collapsed.get();
        let new_collapsed = !is_collapsed;
        imp.sidebar_collapsed.set(new_collapsed);

        if let Some(sidebar_box) = imp.sidebar_box.borrow().as_ref() {
            if new_collapsed {
                sidebar_box.set_width_request(50);
                sidebar_box.add_css_class("sidebar-collapsed");
            } else {
                sidebar_box.set_width_request(220);
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
    fn transcribe_file(&self, path: std::path::PathBuf) {
        let imp = self.imp();

        let engine_arc = match imp.engine.borrow().clone() {
            Some(e) => e,
            None => {
                self.show_toast("No model loaded — download and select a model first");
                return;
            }
        };

        if let Some(sb) = imp.status_bar.borrow().as_ref() {
            sb.set_recording_status("Decoding file…");
        }

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
            .unwrap_or(false);

        let language_code = imp.language_page.borrow()
            .as_ref()
            .and_then(|p| p.selected_language_code());

        let initial_prompt = imp.performance_page.borrow()
            .as_ref()
            .and_then(|p| p.get_initial_prompt());

        let (sender, receiver) = async_channel::bounded::<Result<(String, f32, Vec<(i64, i64, String)>), String>>(1);

        std::thread::spawn(move || {
            let audio_data = match crate::audio::file_decoder::decode_audio_file(&path) {
                Ok(data) => data,
                Err(e) => {
                    let _ = sender.send_blocking(Err(format!("Failed to decode file: {}", e)));
                    return;
                }
            };

            let result = match engine_arc.lock() {
                Ok(guard) => {
                    if let Some(engine) = guard.as_ref() {
                        match engine.transcribe(&audio_data, language_code.as_deref(), n_threads, translate, beam_size, temperature, initial_prompt.as_deref()) {
                            Ok(result) => {
                                let segments: Vec<(i64, i64, String)> = result.segments.iter()
                                    .map(|s| (s.start_ms, s.end_ms, s.text.clone()))
                                    .collect();
                                Ok((result.text, result.average_confidence, segments))
                            }
                            Err(e) => Err(format!("Transcription failed: {}", e)),
                        }
                    } else {
                        Err("No model loaded".to_string())
                    }
                }
                Err(e) => Err(format!("Lock error: {}", e)),
            };
            let _ = sender.send_blocking(result);
        });

        let window = self.clone();
        glib::spawn_future_local(async move {
            if let Ok(result) = receiver.recv().await {
                match result {
                    Ok((text, confidence, segments)) => {
                        let cleaned = postprocess::cleanup_transcript(&text);
                        if let Some(tv) = window.imp().transcript_view.borrow().as_ref() {
                            tv.append_text(&cleaned);
                            tv.set_confidence(confidence as f64);
                        }
                        if let Some(sb) = window.imp().status_bar.borrow().as_ref() {
                            sb.set_recording_status("Ready");
                        }
                        *window.imp().last_segments.borrow_mut() = segments;
                        if !cleaned.is_empty() {
                            if let Some(display) = gtk::gdk::Display::default() {
                                display.clipboard().set_text(&cleaned);
                            }
                        }
                        window.show_toast("File transcription complete");
                    }
                    Err(msg) => {
                        window.show_toast(&msg);
                        if let Some(sb) = window.imp().status_bar.borrow().as_ref() {
                            sb.set_recording_status("Error");
                        }
                    }
                }
            }
        });
    }

    /// Wire model dropdown changes: when user selects a model, load it.
    fn setup_model_dropdown(&self) {        let imp = self.imp();
        let header = match imp.header_controls.borrow().as_ref() {
            Some(h) => h.clone(),
            None => return,
        };

        // Sync dropdown with config
        if let Some(config) = imp.config.borrow().as_ref() {
            let base_id = ModelCatalog::base_model_id(&config.selected_model);
            let index = match base_id {
                "tiny" => 0,
                "base" => 1,
                "small" => 2,
                "medium" => 3,
                "large-v3" => 4,
                _ => 1,
            };
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

            // Update config with base model ID
            if let Some(config) = window.imp().config.borrow().as_ref() {
                let mut new_config = (**config).clone();
                new_config.selected_model = model_id.clone();
                new_config.save();
            }

            // Resolve the best available variant (quantized or full)
            let config = AppConfig::load();
            let resolved = ModelCatalog::resolve_model(&model_id, config.use_quantized);

            // Check if this model is downloaded
            if ModelCatalog::is_downloaded(&resolved) {
                window.load_model_by_id(&resolved);
            } else {
                // Model not downloaded: show red indicator
                if let Some(header) = window.imp().header_controls.borrow().as_ref() {
                    header.set_model_status(false, &model_id);
                }
                if let Some(sb) = window.imp().status_bar.borrow().as_ref() {
                    sb.set_model_name(&format!("{} (not downloaded)", model_id));
                }
                window.show_toast(&format!("Model '{}' is not downloaded. Go to Model settings to download it.", model_id));
            }
        });
    }

    /// Wire language settings changes to update the header language display.
    fn setup_language_display(&self) {
        let imp = self.imp();

        let header = match imp.header_controls.borrow().as_ref() {
            Some(h) => h.clone(),
            None => return,
        };

        let language_page = match imp.language_page.borrow().as_ref() {
            Some(p) => p.clone(),
            None => return,
        };

        // Set initial language display from config
        if let Some(config) = imp.config.borrow().as_ref() {
            if config.auto_detect_language {
                header.set_language_display("Auto-detect");
            } else if let Some(ref lang) = config.language {
                header.set_language_display(&language_code_to_name(lang));
            }
        }

        // Connect language page changes
        let header_ref = header.clone();
        language_page.connect_language_changed(move |lang_name| {
            header_ref.set_language_display(&lang_name);
        });

        // Sync translate toggle between controls and language page
        let controls = match imp.controls.borrow().as_ref() {
            Some(c) => c.clone(),
            None => return,
        };

        // When controls translate toggle changes, sync to language page
        let lang_page_ref = language_page.clone();
        controls.connect_translate_changed(move |active| {
            lang_page_ref.set_translate_enabled(active);
        });
    }

    fn on_record(&self) {
        let imp = self.imp();
        let audio = match imp.audio_capture.borrow().clone() {
            Some(a) => a,
            None => return,
        };

        // Set up waveform channel for UI visualization
        let (waveform_tx, waveform_rx) = async_channel::bounded::<Vec<f32>>(32);
        match audio.lock() {
            Ok(mut cap) => {
                cap.set_waveform_sender(waveform_tx);
            }
            Err(e) => {
                self.show_toast(&format!("Lock error: {}", e));
                return;
            }
        }

        // AudioCapture contains a cpal::Stream which is !Send,
        // but start_recording is fast (sets up the stream), so run on main thread.
        let result = match audio.lock() {
            Ok(mut cap) => cap.start_recording(None),
            Err(e) => {
                self.show_toast(&format!("Lock error: {}", e));
                return;
            }
        };

        match result {
            Ok(()) => {
                if let Some(controls) = imp.controls.borrow().as_ref() {
                    controls.set_recording_state(true);
                }
                if let Some(tv) = imp.transcript_view.borrow().as_ref() {
                    tv.set_recording(true);
                }
                if let Some(sb) = imp.status_bar.borrow().as_ref() {
                    sb.set_recording_status("Recording…");
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

                tracing::info!("Recording started");
            }
            Err(e) => {
                self.show_toast(&format!("Failed to start recording: {}", e));
            }
        }
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
            sb.set_recording_status("Paused");
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
            sb.set_recording_status("Recording…");
        }
    }

    fn on_stop(&self) {
        let imp = self.imp();

        // Stop recording and get audio data
        let audio_capture = match imp.audio_capture.borrow().clone() {
            Some(a) => a,
            None => return,
        };

        let audio_data = match audio_capture.lock() {
            Ok(mut cap) => match cap.stop_recording() {
                Ok(data) => data,
                Err(e) => {
                    self.show_toast(&format!("Error stopping recording: {}", e));
                    return;
                }
            },
            Err(_) => return,
        };

        // Update UI
        if let Some(controls) = imp.controls.borrow().as_ref() {
            controls.set_recording_state(false);
            controls.set_paused_state(false);
        }
        if let Some(tv) = imp.transcript_view.borrow().as_ref() {
            tv.set_recording(false);
        }

        if audio_data.is_empty() {
            self.show_toast("No audio recorded");
            if let Some(sb) = imp.status_bar.borrow().as_ref() {
                sb.set_recording_status("Ready");
            }
            return;
        }

        // Transcribe
        let engine_arc = match imp.engine.borrow().clone() {
            Some(e) => e,
            None => {
                self.show_toast("No model loaded — download and select a model first");
                if let Some(sb) = imp.status_bar.borrow().as_ref() {
                    sb.set_recording_status("No model loaded");
                }
                return;
            }
        };

        if let Some(sb) = imp.status_bar.borrow().as_ref() {
            sb.set_recording_status("Transcribing…");
        }

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

        let language_code = imp.language_page.borrow()
            .as_ref()
            .and_then(|p| p.selected_language_code());

        let initial_prompt = imp.performance_page.borrow()
            .as_ref()
            .and_then(|p| p.get_initial_prompt());

        let (sender, receiver) = async_channel::bounded::<Result<(String, f32, Vec<(i64, i64, String)>), String>>(1);

        std::thread::spawn(move || {
            let result = match engine_arc.lock() {
                Ok(guard) => {
                    if let Some(engine) = guard.as_ref() {
                        match engine.transcribe(&audio_data, language_code.as_deref(), n_threads, translate, beam_size, temperature, initial_prompt.as_deref()) {
                            Ok(result) => {
                                let segments: Vec<(i64, i64, String)> = result.segments.iter()
                                    .map(|s| (s.start_ms, s.end_ms, s.text.clone()))
                                    .collect();
                                Ok((result.text, result.average_confidence, segments))
                            }
                            Err(e) => Err(format!("Transcription failed: {}", e)),
                        }
                    } else {
                        Err("No model loaded".to_string())
                    }
                }
                Err(e) => Err(format!("Lock error: {}", e)),
            };
            let _ = sender.send_blocking(result);
        });

        let window = self.clone();
        glib::spawn_future_local(async move {
            if let Ok(result) = receiver.recv().await {
                match result {
                    Ok((text, confidence, segments)) => {
                        let cleaned = postprocess::cleanup_transcript(&text);
                        if let Some(tv) = window.imp().transcript_view.borrow().as_ref() {
                            tv.append_text(&cleaned);
                            tv.set_confidence(confidence as f64);
                        }
                        if let Some(sb) = window.imp().status_bar.borrow().as_ref() {
                            sb.set_recording_status("Ready");
                        }
                        // Store segments for SRT export
                        *window.imp().last_segments.borrow_mut() = segments;
                        // Auto-copy to clipboard
                        if !cleaned.is_empty() {
                            if let Some(display) = gtk::gdk::Display::default() {
                                display.clipboard().set_text(&cleaned);
                            }
                        }
                        // Add to history
                        if !cleaned.is_empty() {
                            if let Some(hp) = window.imp().history_page.borrow().as_ref() {
                                let lang_name = window.imp().language_page.borrow()
                                    .as_ref()
                                    .map(|p| p.selected_language_name())
                                    .unwrap_or_else(|| "Auto-detect".to_string());
                                let model_name = window.imp().header_controls.borrow()
                                    .as_ref()
                                    .map(|h| h.selected_model_id())
                                    .unwrap_or_else(|| "unknown".to_string());
                                let title = if cleaned.len() > 60 {
                                    format!("{}…", &cleaned[..60])
                                } else {
                                    cleaned.clone()
                                };
                                let entry = crate::ui::history_page::HistoryEntry {
                                    id: uuid::Uuid::new_v4().to_string(),
                                    title,
                                    text: cleaned.clone(),
                                    language: lang_name,
                                    duration_secs: 0,
                                    timestamp: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
                                    model: model_name,
                                };
                                hp.add_entry(entry);
                            }
                        }
                        tracing::info!("Transcription complete ({:.0}% confidence), copied to clipboard", confidence * 100.0);
                    }
                    Err(msg) => {
                        window.show_toast(&msg);
                        if let Some(sb) = window.imp().status_bar.borrow().as_ref() {
                            sb.set_recording_status("Error");
                        }
                    }
                }
            }
        });
    }

    fn on_cancel(&self) {
        let imp = self.imp();

        // Stop recording and discard the audio data
        let audio_capture = match imp.audio_capture.borrow().clone() {
            Some(a) => a,
            None => return,
        };

        if let Ok(mut cap) = audio_capture.lock() {
            let _ = cap.stop_recording(); // Discard return value
        }

        // Reset UI
        if let Some(controls) = imp.controls.borrow().as_ref() {
            controls.set_recording_state(false);
            controls.set_paused_state(false);
        }
        if let Some(tv) = imp.transcript_view.borrow().as_ref() {
            tv.set_recording(false);
        }
        if let Some(sb) = imp.status_bar.borrow().as_ref() {
            sb.set_recording_status("Ready");
        }

        self.show_toast("Recording cancelled");
        tracing::info!("Recording cancelled by user");
    }

    fn on_copy(&self) {
        let imp = self.imp();
        if let Some(tv) = imp.transcript_view.borrow().as_ref() {
            let text = tv.get_text();
            if !text.is_empty() {
                if let Some(display) = gtk::gdk::Display::default() {
                    display.clipboard().set_text(&text);
                    self.show_toast("Copied to clipboard");
                }
            }
        }
    }

    fn on_clear(&self) {
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
            self.show_toast("Nothing to save");
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
                        Ok(()) => window.show_toast("Transcript saved"),
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
        row.set_tooltip_text(Some(nav_item.title()));

        let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        hbox.set_margin_top(8);
        hbox.set_margin_bottom(8);
        hbox.set_margin_start(12);
        hbox.set_margin_end(12);
        hbox.add_css_class("nav-row-box");

        let icon = gtk::Image::from_icon_name(nav_item.icon_name());
        icon.set_pixel_size(20);
        hbox.append(&icon);

        let label = gtk::Label::new(Some(nav_item.title()));
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
            NavItem::Help => "help",
        };

        if let Some(stack) = imp.content_stack.borrow().as_ref() {
            stack.set_visible_child_name(page_name);
        }

        imp.current_nav.set(Some(nav_item));
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
                        "help" => NavItem::Help,
                        _ => return,
                    };
                    window.navigate_to(nav_item);
                }
            })
            .build();

        self.add_action_entries([action_quit, action_toggle_sidebar, action_navigate]);
    }

    /// Show a toast notification.
    pub fn show_toast(&self, message: &str) {
        if let Some(overlay) = self.imp().toast_overlay.borrow().as_ref() {
            let toast = adw::Toast::new(message);
            toast.set_timeout(3);
            overlay.add_toast(toast);
        }
    }

    /// Try to load the currently selected model in a background thread.
    fn load_selected_model(&self) {
        let imp = self.imp();
        let config = match imp.config.borrow().clone() {
            Some(c) => c,
            None => return,
        };
        let model_id = ModelCatalog::resolve_model(&config.selected_model, config.use_quantized);
        self.load_model_by_id(&model_id);
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

        if let Some(sb) = imp.status_bar.borrow().as_ref() {
            sb.set_recording_status("Loading model…");
        }

        // Use async_channel to send result back to the main thread
        let (sender, receiver) = async_channel::bounded::<Result<String, String>>(1);

        let model_id_owned = model_id.to_string();
        std::thread::spawn(move || {
            match TranscriptionEngine::load_model_with_gpu(&model_path, &model_id_owned, use_gpu) {
                Ok(engine) => {
                    if let Ok(mut guard) = engine_arc.lock() {
                        *guard = Some(engine);
                    }
                    let _ = sender.send_blocking(Ok(model_id_owned));
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
                    Ok(mid) => window.on_model_loaded(&mid),
                    Err(msg) => window.show_toast(&msg),
                }
            }
        });
    }

    /// Called when a model finishes loading.
    fn on_model_loaded(&self, model_id: &str) {
        tracing::info!("Model '{}' loaded and ready", model_id);

        if let Some(header) = self.imp().header_controls.borrow().as_ref() {
            header.set_model_status(true, model_id);

            // Sync the dropdown to reflect the base model size
            let base_id = ModelCatalog::base_model_id(model_id);
            let index = match base_id {
                "tiny" => 0u32,
                "base" => 1,
                "small" => 2,
                "medium" => 3,
                "large-v3" => 4,
                _ => 1,
            };
            self.imp().syncing_dropdown.set(true);
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
            status_bar.set_recording_status("Ready");
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
