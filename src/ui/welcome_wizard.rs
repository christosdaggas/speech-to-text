// Speech to Text - Welcome Wizard
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! First-run welcome wizard for model download and initial setup.

use gtk4::prelude::*;
use crate::i18n::gettext;
use adw::prelude::*;
use gtk4::glib;
use gtk4 as gtk;
use libadwaita as adw;
use adw::subclass::prelude::*;
use std::cell::{Cell, RefCell};

use crate::application::tokio_runtime;
use crate::config::AppConfig;
use crate::transcription::{ModelCatalog, download_model};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct WelcomeWizard {
        pub carousel: RefCell<Option<adw::Carousel>>,
        pub progress_bar: RefCell<Option<gtk::ProgressBar>>,
        pub status_label: RefCell<Option<gtk::Label>>,
        pub download_btn: RefCell<Option<gtk::Button>>,
        pub model_dropdown: RefCell<Option<gtk::DropDown>>,
        pub next_btn: RefCell<Option<gtk::Button>>,
        pub finish_btn: RefCell<Option<gtk::Button>>,
        pub completed: Cell<bool>,
        pub downloaded_model_id: RefCell<Option<String>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for WelcomeWizard {
        const NAME: &'static str = "SttWelcomeWizard";
        type Type = super::WelcomeWizard;
        type ParentType = adw::Window;
    }

    impl ObjectImpl for WelcomeWizard {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_ui();
        }
    }

    impl WidgetImpl for WelcomeWizard {}
    impl WindowImpl for WelcomeWizard {}
    impl adw::subclass::prelude::AdwWindowImpl for WelcomeWizard {}
}

glib::wrapper! {
    pub struct WelcomeWizard(ObjectSubclass<imp::WelcomeWizard>)
        @extends gtk::Widget, gtk::Window, adw::Window;
}

impl WelcomeWizard {
    pub fn new(parent: &impl IsA<gtk::Window>) -> Self {
        let wizard: Self = glib::Object::builder()
            .property("title", "Welcome to Speech to Text")
            .property("default-width", 600)
            .property("default-height", 500)
            .property("modal", true)
            .property("transient-for", parent)
            .build();
        wizard
    }

    fn setup_ui(&self) {
        let imp = self.imp();

        let main_box = gtk::Box::new(gtk::Orientation::Vertical, 0);

        // Header bar
        let header = adw::HeaderBar::new();
        header.set_show_end_title_buttons(false);
        header.set_show_start_title_buttons(false);
        main_box.append(&header);

        // Carousel for wizard pages
        let carousel = adw::Carousel::new();
        carousel.set_vexpand(true);
        carousel.set_allow_mouse_drag(false);
        carousel.set_allow_scroll_wheel(false);
        carousel.set_allow_long_swipes(false);

        // === Page 1: Welcome ===
        let welcome_page = self.build_welcome_page();
        carousel.append(&welcome_page);

        // === Page 2: Model selection + download ===
        let model_page = self.build_model_page();
        carousel.append(&model_page);

        // === Page 3: Finish ===
        let finish_page = self.build_finish_page();
        carousel.append(&finish_page);

        main_box.append(&carousel);

        // Carousel dots indicator
        let dots = adw::CarouselIndicatorDots::new();
        dots.set_carousel(Some(&carousel));
        main_box.append(&dots);

        // Navigation buttons
        let nav_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        nav_box.set_margin_start(24);
        nav_box.set_margin_end(24);
        nav_box.set_margin_bottom(16);
        nav_box.set_margin_top(8);
        nav_box.set_halign(gtk::Align::End);

        let next_btn = gtk::Button::with_label("Next");
        next_btn.add_css_class("suggested-action");
        next_btn.add_css_class("pill");
        nav_box.append(&next_btn);

        let finish_btn = gtk::Button::with_label("Get Started");
        finish_btn.add_css_class("suggested-action");
        finish_btn.add_css_class("pill");
        finish_btn.set_visible(false);
        finish_btn.set_sensitive(false);
        nav_box.append(&finish_btn);

        main_box.append(&nav_box);

        self.set_content(Some(&main_box));

        // Navigation logic
        let carousel_ref = carousel.clone();
        let finish_btn_ref = finish_btn.clone();
        let next_btn_ref = next_btn.clone();
        let wizard_weak_nav = self.downgrade();
        next_btn.connect_clicked(move |_| {
            let pos = carousel_ref.position() as u32;
            let n_pages = carousel_ref.n_pages();
            if pos + 1 < n_pages {
                let next_page = carousel_ref.nth_page(pos + 1);
                carousel_ref.scroll_to(&next_page, true);
            }
            if pos + 2 >= n_pages {
                next_btn_ref.set_visible(false);
                finish_btn_ref.set_visible(true);
            }
            // Auto-start download when landing on model page (page index 1)
            if pos == 0 {
                if let Some(wizard) = wizard_weak_nav.upgrade() {
                    if let Some(btn) = wizard.imp().download_btn.borrow().as_ref() {
                        btn.emit_clicked();
                    }
                }
            }
        });

        let wizard_weak = self.downgrade();
        finish_btn.connect_clicked(move |_| {
            if let Some(wizard) = wizard_weak.upgrade() {
                let Some(model_id) = wizard.imp().downloaded_model_id.borrow().clone() else {
                    return;
                };
                if !ModelCatalog::is_downloaded(&model_id) {
                    return;
                }
                wizard.imp().completed.set(true);

                // Save config: mark first_run = false and set selected model.
                // Store the EXACT downloaded model ID (e.g. "tiny-q5_1") so the
                // engine reloads the same variant on the next launch.
                let mut config = AppConfig::load();
                config.first_run = false;
                if let Some(ref model_id) = *wizard.imp().downloaded_model_id.borrow() {
                    // Track whether the downloaded model is a quantized variant so
                    // resolve_model() can recover the right one if the ID ever drifts.
                    config.use_quantized = ModelCatalog::new()
                        .get(model_id)
                        .map(|m| m.quantized)
                        .unwrap_or(false);
                    config.selected_model = model_id.clone();
                }
                config.save();

                // Tell the main window to load the downloaded model directly
                let load_id = wizard.imp().downloaded_model_id.borrow().clone()
                    .unwrap_or_else(|| config.selected_model.clone());
                if let Some(parent) = wizard.transient_for() {
                    if let Some(main_window) = parent.downcast_ref::<super::MainWindow>() {
                        main_window.load_model_by_id(&load_id);
                    }
                }

                wizard.close();
            }
        });

        *imp.carousel.borrow_mut() = Some(carousel);
        *imp.next_btn.borrow_mut() = Some(next_btn);
        *imp.finish_btn.borrow_mut() = Some(finish_btn);
    }

    fn build_welcome_page(&self) -> gtk::Box {
        let page = gtk::Box::new(gtk::Orientation::Vertical, 16);
        page.set_valign(gtk::Align::Center);
        page.set_margin_start(48);
        page.set_margin_end(48);

        let icon = Self::icon_with_fallback(crate::APP_ID, "audio-input-microphone-symbolic");
        icon.set_pixel_size(96);
        icon.add_css_class("accent");
        page.append(&icon);

        let title = gtk::Label::new(Some(gettext("Welcome to Speech to Text").as_str()));
        title.add_css_class("title-1");
        page.append(&title);

        let subtitle = gtk::Label::new(Some(
            "Convert speech to text locally on your machine.\n\
             No internet connection required after initial setup."
        ));
        subtitle.set_justify(gtk::Justification::Center);
        subtitle.add_css_class("body");
        subtitle.set_wrap(true);
        page.append(&subtitle);

        let features = gtk::Box::new(gtk::Orientation::Vertical, 8);
        features.set_margin_top(24);
        features.set_halign(gtk::Align::Center);

        for (icon_name, text) in [
            ("network-offline-symbolic", "100% offline — your audio never leaves your device"),
            ("preferences-desktop-locale-symbolic", "Supports 99 languages with auto-detection"),
            ("media-record-symbolic", "Real-time recording with live waveform"),
        ] {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let icon = gtk::Image::from_icon_name(icon_name);
            icon.set_pixel_size(16);
            icon.add_css_class("accent");
            row.append(&icon);
            let label = gtk::Label::new(Some(text));
            label.add_css_class("body");
            row.append(&label);
            features.append(&row);
        }

        page.append(&features);
        page
    }

    fn build_model_page(&self) -> gtk::Box {
        let imp = self.imp();
        let page = gtk::Box::new(gtk::Orientation::Vertical, 16);
        page.set_valign(gtk::Align::Center);
        page.set_margin_start(48);
        page.set_margin_end(48);

        let title = gtk::Label::new(Some(gettext("Download a Whisper Model").as_str()));
        title.add_css_class("title-2");
        page.append(&title);

        let desc = gtk::Label::new(Some(
            "A language model is needed for transcription.\n\
             Quantized (Q5) models are smaller with near-identical quality."
        ));
        desc.set_justify(gtk::Justification::Center);
        desc.set_wrap(true);
        desc.add_css_class("body");
        page.append(&desc);

        // Model selector — build from catalog
        let catalog = ModelCatalog::new();
        let labels: Vec<String> = catalog.models().iter().map(|m| {
            if m.quantized {
                format!("{} ({}) — Quantized", m.display_name, m.size_display)
            } else {
                format!("{} ({}) — Full", m.display_name, m.size_display)
            }
        }).collect();
        let label_refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
        let model_list = gtk::StringList::new(&label_refs);
        let model_dropdown = gtk::DropDown::new(Some(model_list), gtk::Expression::NONE);
        // Default to first quantized model (index 5 = Tiny Q5) for smaller initial download
        model_dropdown.set_selected(5);
        model_dropdown.set_margin_top(8);
        page.append(&model_dropdown);

        // Download button
        let download_btn = gtk::Button::with_label("Download Model");
        download_btn.add_css_class("suggested-action");
        download_btn.add_css_class("pill");
        download_btn.set_margin_top(12);
        download_btn.set_halign(gtk::Align::Center);
        page.append(&download_btn);

        // Progress bar
        let progress_bar = gtk::ProgressBar::new();
        progress_bar.set_show_text(true);
        progress_bar.set_margin_top(8);
        progress_bar.set_visible(false);
        page.append(&progress_bar);

        let status_label = gtk::Label::new(None);
        status_label.add_css_class("caption");
        status_label.add_css_class("dim-label");
        page.append(&status_label);

        // Download click handler — actually downloads the model
        let dropdown_ref = model_dropdown.clone();
        let progress_ref = progress_bar.clone();
        let btn_ref = download_btn.clone();
        let status_ref = status_label.clone();
        let wizard_weak = self.downgrade();

        download_btn.connect_clicked(move |_| {
            let selected = dropdown_ref.selected();
            let catalog = ModelCatalog::new();
            let models = catalog.models();
            let model_info = match models.get(selected as usize) {
                Some(m) => (*m).clone(),
                None => return,
            };

            // Check if already downloaded
            if ModelCatalog::is_downloaded(&model_info.id) {
                status_ref.set_text("Model already downloaded!");
                if let Some(wizard) = wizard_weak.upgrade() {
                    *wizard.imp().downloaded_model_id.borrow_mut() = Some(model_info.id.clone());
                    wizard.download_complete();
                }
                return;
            }

            let downloaded_id = model_info.id.clone();

            btn_ref.set_sensitive(false);
            btn_ref.set_label(gettext("Downloading…").as_str());
            progress_ref.set_visible(true);
            progress_ref.set_fraction(0.0);
            status_ref.set_text("Starting download…");

            let (progress_tx, progress_rx) = async_channel::bounded::<(u64, u64)>(64);
            let (done_tx, done_rx) = async_channel::bounded::<Result<(), String>>(1);

            let rt = tokio_runtime();
            rt.spawn(async move {
                let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                match download_model(&model_info, cancel, move |downloaded, total| {
                    let _ = progress_tx.send_blocking((downloaded, total));
                }).await {
                    Ok(_) => { let _ = done_tx.send_blocking(Ok(())); }
                    Err(e) => { let _ = done_tx.send_blocking(Err(e.to_string())); }
                }
            });

            // Progress updates
            let progress_bar_clone = progress_ref.clone();
            let status_clone = status_ref.clone();
            glib::spawn_future_local(async move {
                while let Ok((downloaded, total)) = progress_rx.recv().await {
                    let frac = if total > 0 { downloaded as f64 / total as f64 } else { 0.0 };
                    let mb_down = downloaded as f64 / 1_000_000.0;
                    let mb_total = total as f64 / 1_000_000.0;
                    progress_bar_clone.set_fraction(frac);
                    progress_bar_clone.set_text(Some(&format!("Downloading… {:.0}%", frac * 100.0)));
                    status_clone.set_text(&format!("{:.0} / {:.0} MB", mb_down, mb_total));
                }
            });

            // Completion
            let wizard_w = wizard_weak.clone();
            let btn_clone = btn_ref.clone();
            let status_clone2 = status_ref.clone();
            let downloaded_id_clone = downloaded_id.clone();
            glib::spawn_future_local(async move {
                if let Ok(result) = done_rx.recv().await {
                    match result {
                        Ok(()) => {
                            if let Some(wizard) = wizard_w.upgrade() {
                                *wizard.imp().downloaded_model_id.borrow_mut() = Some(downloaded_id_clone);
                                wizard.download_complete();
                            }
                        }
                        Err(e) => {
                            btn_clone.set_label(gettext("Retry").as_str());
                            btn_clone.set_sensitive(true);
                            status_clone2.set_text(&format!("Error: {}", e));
                        }
                    }
                }
            });
        });

        *imp.progress_bar.borrow_mut() = Some(progress_bar);
        *imp.status_label.borrow_mut() = Some(status_label);
        *imp.download_btn.borrow_mut() = Some(download_btn);
        *imp.model_dropdown.borrow_mut() = Some(model_dropdown);

        page
    }

    fn build_finish_page(&self) -> gtk::Box {
        let page = gtk::Box::new(gtk::Orientation::Vertical, 16);
        page.set_valign(gtk::Align::Center);
        page.set_margin_start(48);
        page.set_margin_end(48);

        let icon = Self::icon_with_fallback("object-select-symbolic", "emblem-ok-symbolic");
        icon.set_pixel_size(64);
        icon.add_css_class("success");
        page.append(&icon);

        let title = gtk::Label::new(Some(gettext("You're All Set!").as_str()));
        title.add_css_class("title-1");
        page.append(&title);

        let desc = gtk::Label::new(Some(
            "Everything is configured. Press Record to start\n\
             transcribing speech to text."
        ));
        desc.set_justify(gtk::Justification::Center);
        desc.set_wrap(true);
        desc.add_css_class("body");
        page.append(&desc);

        page
    }

    /// Update download progress from the model downloader.
    pub fn set_download_progress(&self, fraction: f64) {
        if let Some(bar) = self.imp().progress_bar.borrow().as_ref() {
            bar.set_fraction(fraction);
            bar.set_text(Some(&format!("Downloading… {:.0}%", fraction * 100.0)));
        }
    }

    /// Mark download as complete.
    pub fn download_complete(&self) {
        if let Some(bar) = self.imp().progress_bar.borrow().as_ref() {
            bar.set_fraction(1.0);
            bar.set_text(Some(gettext("Download complete!").as_str()));
        }
        if let Some(btn) = self.imp().download_btn.borrow().as_ref() {
            btn.set_label(gettext("Downloaded ✓").as_str());
            btn.set_sensitive(false);
        }
        // Auto-advance to finish page
        if let Some(carousel) = self.imp().carousel.borrow().as_ref() {
            let last = carousel.nth_page(carousel.n_pages() - 1);
            carousel.scroll_to(&last, true);
        }
        if let Some(next) = self.imp().next_btn.borrow().as_ref() {
            next.set_visible(false);
        }
        if let Some(finish) = self.imp().finish_btn.borrow().as_ref() {
            finish.set_visible(true);
            finish.set_sensitive(true);
        }
    }

    /// Check if the wizard was completed (not just closed).
    pub fn is_completed(&self) -> bool {
        self.imp().completed.get()
    }

    /// Create an image from an icon name, falling back if the primary icon is unavailable.
    fn icon_with_fallback(name: &str, fallback: &str) -> gtk::Image {
        if let Some(display) = gtk::gdk::Display::default() {
            let theme = gtk::IconTheme::for_display(&display);
            if theme.has_icon(name) {
                return gtk::Image::from_icon_name(name);
            }
        }
        gtk::Image::from_icon_name(fallback)
    }
}
