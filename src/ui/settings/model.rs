// Speech to Text - Model Settings Page
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Model selection, download management, and model info page.

use gtk4::prelude::*;
use adw::prelude::*;
use gtk4::glib;
use gtk4 as gtk;
use libadwaita as adw;
use adw::subclass::prelude::*;
use std::cell::RefCell;

use crate::application::tokio_runtime;
use crate::config::AppConfig;
use crate::transcription::{ModelCatalog, download_model};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ModelPage {
        pub model_group: RefCell<Option<adw::PreferencesGroup>>,
        pub progress_bar: RefCell<Option<gtk::ProgressBar>>,
        pub download_status: RefCell<Option<gtk::Label>>,
        pub buttons: RefCell<Vec<(String, gtk::Button)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ModelPage {
        const NAME: &'static str = "SttModelPage";
        type Type = super::ModelPage;
        type ParentType = adw::PreferencesPage;
    }

    impl ObjectImpl for ModelPage {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_ui();
        }
    }

    impl WidgetImpl for ModelPage {}
    impl adw::subclass::prelude::PreferencesPageImpl for ModelPage {}
}

glib::wrapper! {
    pub struct ModelPage(ObjectSubclass<imp::ModelPage>)
        @extends gtk::Widget, adw::PreferencesPage;
}

impl ModelPage {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("title", "Model")
            .property("icon-name", "system-software-install-symbolic")
            .build()
    }

    fn setup_ui(&self) {
        let imp = self.imp();
        let catalog = ModelCatalog::new();

        // Quantization preference group
        let pref_group = adw::PreferencesGroup::new();
        pref_group.set_title("Model Format");
        pref_group.set_description(Some("Quantized models are 57–66% smaller with minimal quality loss."));

        let quantized_switch = adw::SwitchRow::builder()
            .title("Prefer Quantized Models")
            .subtitle("Use compressed (Q5) variants when available")
            .build();

        let config = AppConfig::load();
        quantized_switch.set_active(config.use_quantized);

        quantized_switch.connect_active_notify(|switch| {
            let mut config = AppConfig::load();
            config.use_quantized = switch.is_active();
            config.save();
        });

        pref_group.add(&quantized_switch);
        self.add(&pref_group);

        // Full models group
        let full_group = adw::PreferencesGroup::new();
        full_group.set_title("Full Models");
        full_group.set_description(Some("Original precision (f16). Largest size, maximum accuracy."));

        // Quantized models group
        let quantized_group = adw::PreferencesGroup::new();
        quantized_group.set_title("Quantized Models");
        quantized_group.set_description(Some("5-bit quantized (Q5). Much smaller with near-identical accuracy."));

        let mut buttons = Vec::new();

        for model_info in catalog.models() {
            let target_group = if model_info.quantized { &quantized_group } else { &full_group };

            let row = adw::ActionRow::builder()
                .title(&model_info.display_name)
                .subtitle(&format!("{} — {}", model_info.size_display, model_info.description))
                .activatable(true)
                .build();

            let size_label = gtk::Label::new(Some(&model_info.size_display));
            size_label.add_css_class("dim-label");
            size_label.add_css_class("caption");
            row.add_suffix(&size_label);

            let action_btn = gtk::Button::new();
            action_btn.set_valign(gtk::Align::Center);
            action_btn.add_css_class("pill");

            if ModelCatalog::is_downloaded(&model_info.id) {
                action_btn.set_label("Downloaded");
                action_btn.set_sensitive(false);
                action_btn.add_css_class("success");
            } else {
                action_btn.set_label("Download");
            }

            // Wire download
            let model_info_clone = model_info.clone();
            let page_weak = self.downgrade();
            let btn_clone = action_btn.clone();
            let is_quantized = model_info.quantized;

            action_btn.connect_clicked(move |_| {
                btn_clone.set_sensitive(false);
                btn_clone.set_label("Downloading…");

                let info = model_info_clone.clone();
                let page_w = page_weak.clone();
                let btn_w = btn_clone.downgrade();
                let model_id_for_load = info.id.clone();
                let is_q = is_quantized;

                // Channel for progress
                let (progress_tx, progress_rx) = async_channel::bounded::<(u64, u64)>(64);
                let (done_tx, done_rx) = async_channel::bounded::<Result<(), String>>(1);

                // Spawn download on tokio runtime
                let rt = tokio_runtime();
                rt.spawn(async move {
                    match download_model(&info, move |downloaded, total| {
                        let _ = progress_tx.send_blocking((downloaded, total));
                    }).await {
                        Ok(_) => { let _ = done_tx.send_blocking(Ok(())); }
                        Err(e) => { let _ = done_tx.send_blocking(Err(e.to_string())); }
                    }
                });

                // Progress polling on GTK main thread
                let page_w2 = page_w.clone();
                glib::spawn_future_local(async move {
                    while let Ok((downloaded, total)) = progress_rx.recv().await {
                        if let Some(page) = page_w2.upgrade() {
                            let frac = if total > 0 { downloaded as f64 / total as f64 } else { 0.0 };
                            let mb_down = downloaded as f64 / 1_000_000.0;
                            let mb_total = total as f64 / 1_000_000.0;
                            page.set_download_progress(
                                frac,
                                &format!("{:.0} / {:.0} MB", mb_down, mb_total),
                            );
                        }
                    }
                });

                // Completion handler
                glib::spawn_future_local(async move {
                    if let Ok(result) = done_rx.recv().await {
                        match result {
                            Ok(()) => {
                                if let Some(btn) = btn_w.upgrade() {
                                    btn.set_label("Downloaded");
                                    btn.remove_css_class("pill");
                                    btn.add_css_class("success");
                                    btn.add_css_class("pill");
                                }
                                if let Some(page) = page_w.upgrade() {
                                    page.set_download_progress(1.0, "Download complete!");

                                    // Save config: store base model ID + quantization preference
                                    let mut config = AppConfig::load();
                                    let base_id = crate::transcription::ModelCatalog::base_model_id(&model_id_for_load).to_string();
                                    config.selected_model = base_id;
                                    config.use_quantized = is_q;
                                    config.first_run = false;
                                    config.save();

                                    // Find MainWindow and load the exact downloaded model
                                    if let Some(window) = page.ancestor(gtk::Window::static_type()) {
                                        if let Some(window) = window.downcast_ref::<gtk::Window>() {
                                            if let Some(main_window) = window.downcast_ref::<crate::ui::MainWindow>() {
                                                main_window.load_model_by_id(&model_id_for_load);
                                            }
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                if let Some(btn) = btn_w.upgrade() {
                                    btn.set_label("Retry");
                                    btn.set_sensitive(true);
                                }
                                if let Some(page) = page_w.upgrade() {
                                    page.set_download_progress(0.0, &format!("Error: {}", e));
                                }
                            }
                        }
                    }
                });
            });

            row.add_suffix(&action_btn);
            buttons.push((model_info.id.clone(), action_btn));
            target_group.add(&row);
        }

        self.add(&full_group);
        self.add(&quantized_group);
        *imp.model_group.borrow_mut() = Some(full_group);
        *imp.buttons.borrow_mut() = buttons;

        // Download progress group
        let progress_group = adw::PreferencesGroup::new();
        progress_group.set_title("Download Progress");

        let progress_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        progress_box.set_margin_top(8);
        progress_box.set_margin_bottom(8);
        progress_box.set_margin_start(12);
        progress_box.set_margin_end(12);

        let progress_bar = gtk::ProgressBar::new();
        progress_bar.set_show_text(true);
        progress_bar.set_fraction(0.0);
        progress_bar.set_text(Some("No download in progress"));
        progress_box.append(&progress_bar);

        let download_status = gtk::Label::new(Some(""));
        download_status.add_css_class("caption");
        download_status.add_css_class("dim-label");
        download_status.set_xalign(0.0);
        progress_box.append(&download_status);

        // Wrap in a preferences row
        let progress_row = adw::ActionRow::new();
        progress_row.set_child(Some(&progress_box));
        progress_group.add(&progress_row);

        self.add(&progress_group);

        *imp.progress_bar.borrow_mut() = Some(progress_bar);
        *imp.download_status.borrow_mut() = Some(download_status);

        // Storage info group
        let storage_group = adw::PreferencesGroup::new();
        storage_group.set_title("Storage");
        storage_group.set_description(Some("Choose where Whisper models are stored."));

        let models_dir = crate::config::AppConfig::models_dir();
        let storage_row = adw::ActionRow::builder()
            .title("Models Directory")
            .subtitle(&*models_dir.to_string_lossy())
            .build();

        let open_btn = gtk::Button::from_icon_name("folder-open-symbolic");
        open_btn.set_tooltip_text(Some("Open in file manager"));
        open_btn.set_valign(gtk::Align::Center);
        open_btn.add_css_class("flat");
        let dir_path = models_dir.clone();
        open_btn.connect_clicked(move |btn| {
            let file = gtk::gio::File::for_path(&dir_path);
            let launcher = gtk::FileLauncher::new(Some(&file));
            let window = btn.root().and_then(|r| r.downcast::<gtk::Window>().ok());
            launcher.launch(window.as_ref(), gtk::gio::Cancellable::NONE, |_| {});
        });
        storage_row.add_suffix(&open_btn);

        let change_btn = gtk::Button::from_icon_name("document-edit-symbolic");
        change_btn.set_tooltip_text(Some("Change models directory"));
        change_btn.set_valign(gtk::Align::Center);
        change_btn.add_css_class("flat");
        let storage_row_weak = storage_row.downgrade();
        change_btn.connect_clicked(move |btn| {
            let dialog = gtk::FileDialog::builder()
                .title("Select Models Directory")
                .build();

            let btn_weak = btn.downgrade();
            let storage_row_w = storage_row_weak.clone();
            if let Some(window) = btn.root().and_then(|r| r.downcast::<gtk::Window>().ok()) {
                dialog.select_folder(Some(&window), gtk::gio::Cancellable::NONE, move |result| {
                    if let Ok(folder) = result {
                        if let Some(path) = folder.path() {
                            let path_str = path.to_string_lossy().to_string();
                            let mut config = crate::config::AppConfig::load();
                            config.model_directory = Some(path_str.clone());
                            config.save();
                            if let Some(row) = storage_row_w.upgrade() {
                                row.set_subtitle(&path_str);
                            }
                        }
                    }
                    let _ = btn_weak;
                });
            }
        });
        storage_row.add_suffix(&change_btn);

        let reset_btn = gtk::Button::from_icon_name("edit-undo-symbolic");
        reset_btn.set_tooltip_text(Some("Reset to default location"));
        reset_btn.set_valign(gtk::Align::Center);
        reset_btn.add_css_class("flat");
        let storage_row_weak2 = storage_row.downgrade();
        reset_btn.connect_clicked(move |_| {
            let mut config = crate::config::AppConfig::load();
            config.model_directory = None;
            config.save();
            if let Some(row) = storage_row_weak2.upgrade() {
                let default_dir = crate::config::AppConfig::default_models_dir();
                row.set_subtitle(&*default_dir.to_string_lossy());
            }
        });
        storage_row.add_suffix(&reset_btn);

        storage_group.add(&storage_row);
        self.add(&storage_group);
    }

    /// Update download progress (0.0 - 1.0).
    pub fn set_download_progress(&self, fraction: f64, status_text: &str) {
        let imp = self.imp();
        if let Some(bar) = imp.progress_bar.borrow().as_ref() {
            bar.set_fraction(fraction);
            bar.set_text(Some(&format!("{:.0}%", fraction * 100.0)));
        }
        if let Some(label) = imp.download_status.borrow().as_ref() {
            label.set_text(status_text);
        }
    }

    /// Clear download progress.
    pub fn clear_progress(&self) {
        let imp = self.imp();
        if let Some(bar) = imp.progress_bar.borrow().as_ref() {
            bar.set_fraction(0.0);
            bar.set_text(Some("No download in progress"));
        }
        if let Some(label) = imp.download_status.borrow().as_ref() {
            label.set_text("");
        }
    }
}
