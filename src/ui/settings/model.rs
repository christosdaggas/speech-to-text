// Speech to Text - Model Settings Page
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Model selection, download management, and model info page.

use crate::i18n::gettext;
use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk4 as gtk;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::RefCell;

use crate::application::tokio_runtime;
use crate::config::AppConfig;
use crate::transcription::{download_model, ModelCatalog};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ModelPage {
        pub engine_combo: RefCell<Option<adw::ComboRow>>,
        pub model_group: RefCell<Option<adw::PreferencesGroup>>,
        pub progress_bar: RefCell<Option<gtk::ProgressBar>>,
        pub download_status: RefCell<Option<gtk::Label>>,
        pub buttons: RefCell<Vec<(String, gtk::Button)>>,
        pub cohere_group: RefCell<Option<adw::PreferencesGroup>>,
        /// Cancel flag for the currently active model download, if any.
        pub current_cancel: RefCell<Option<std::sync::Arc<std::sync::atomic::AtomicBool>>>,
        pub cancel_btn: RefCell<Option<gtk::Button>>,
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

    /// Set the engine combo from a backend id ("whisper" / "cohere" / "qwen").
    pub fn set_engine(&self, backend: &str) {
        if let Some(combo) = self.imp().engine_combo.borrow().as_ref() {
            combo.set_selected(Self::backend_to_index(backend));
        }
    }

    /// Connect a callback fired when the engine selection changes
    /// ("whisper" / "cohere" / "qwen").
    pub fn connect_engine_changed<F: Fn(String) + 'static>(&self, callback: F) {
        if let Some(combo) = self.imp().engine_combo.borrow().as_ref() {
            combo.connect_selected_notify(move |c| {
                callback(Self::index_to_backend(c.selected()).to_string());
            });
        }
    }

    /// Engine combo order: 0 = Whisper, 1 = Cohere, 2 = Qwen3-ASR.
    fn backend_to_index(backend: &str) -> u32 {
        match backend {
            "cohere" => 1,
            "qwen" => 2,
            _ => 0,
        }
    }

    fn index_to_backend(index: u32) -> &'static str {
        match index {
            1 => "cohere",
            2 => "qwen",
            _ => "whisper",
        }
    }

    fn setup_ui(&self) {
        let imp = self.imp();
        let catalog = ModelCatalog::new();

        // Engine selector — choose which transcription engine is active. Persisted
        // to config.backend, so it applies everywhere including the mini panel.
        let engine_group = adw::PreferencesGroup::new();
        engine_group.set_title(gettext("Engine").as_str());
        engine_group.set_description(Some(
            gettext(
                "Which transcription engine to use. Applies everywhere, including the mini panel.",
            )
            .as_str(),
        ));
        let engine_list = gtk::StringList::new(&["Whisper", "Cohere Transcribe", "Qwen3-ASR"]);
        let engine_combo = adw::ComboRow::builder()
            .title(gettext("Default Engine").as_str())
            .subtitle(gettext("Whisper, Cohere Transcribe and Qwen3-ASR all run locally.").as_str())
            .model(&engine_list)
            .build();
        engine_combo.set_selected(Self::backend_to_index(&AppConfig::load().backend));
        engine_group.add(&engine_combo);
        self.add(&engine_group);
        *imp.engine_combo.borrow_mut() = Some(engine_combo);

        // Full models group
        let full_group = adw::PreferencesGroup::new();
        full_group.set_title(gettext("Full Models").as_str());
        full_group.set_description(Some(
            gettext("Original precision (f16). Largest size, maximum accuracy.").as_str(),
        ));

        // Quantized models group
        let quantized_group = adw::PreferencesGroup::new();
        quantized_group.set_title(gettext("Quantized Models").as_str());
        quantized_group.set_description(Some(
            gettext("5-bit quantized (Q5). Much smaller with near-identical accuracy.").as_str(),
        ));

        let mut buttons = Vec::new();

        for model_info in catalog.models() {
            let target_group = if model_info.quantized {
                &quantized_group
            } else {
                &full_group
            };

            let row = adw::ActionRow::builder()
                .title(&model_info.display_name)
                .subtitle(&format!(
                    "{} — {}",
                    model_info.size_display, model_info.description
                ))
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
                action_btn.set_label(gettext("Downloaded").as_str());
                action_btn.set_sensitive(false);
                action_btn.add_css_class("success");
            } else {
                action_btn.set_label(gettext("Download").as_str());
            }

            // Delete button — visible only while the model is on disk.
            let delete_btn = gtk::Button::from_icon_name("user-trash-symbolic");
            delete_btn.set_valign(gtk::Align::Center);
            delete_btn.add_css_class("flat");
            delete_btn.add_css_class("circular");
            delete_btn.set_tooltip_text(Some(gettext("Delete this model from disk").as_str()));
            delete_btn.set_visible(ModelCatalog::is_downloaded(&model_info.id));

            // Wire download
            let model_info_clone = model_info.clone();
            let page_weak = self.downgrade();
            let btn_clone = action_btn.clone();
            let delete_w_for_dl = delete_btn.downgrade();

            action_btn.connect_clicked(move |_| {
                btn_clone.set_sensitive(false);
                btn_clone.set_label(gettext("Downloading…").as_str());

                let info = model_info_clone.clone();
                let page_w = page_weak.clone();
                let btn_w = btn_clone.downgrade();
                let delete_w = delete_w_for_dl.clone();
                let model_id_for_load = info.id.clone();

                // Cancel flag — shared with the Cancel button in the progress group.
                let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                if let Some(page) = page_w.upgrade() {
                    *page.imp().current_cancel.borrow_mut() = Some(cancel.clone());
                    if let Some(cb) = page.imp().cancel_btn.borrow().as_ref() {
                        cb.set_visible(true);
                        cb.set_sensitive(true);
                    }
                }
                let cancel_for_dl = cancel.clone();

                // Channel for progress
                let (progress_tx, progress_rx) = async_channel::bounded::<(u64, u64)>(64);
                let (done_tx, done_rx) = async_channel::bounded::<Result<(), String>>(1);

                // Spawn download on tokio runtime
                let rt = tokio_runtime();
                rt.spawn(async move {
                    match download_model(&info, cancel_for_dl, move |downloaded, total| {
                        let _ = progress_tx.send_blocking((downloaded, total));
                    })
                    .await
                    {
                        Ok(_) => {
                            let _ = done_tx.send_blocking(Ok(()));
                        }
                        Err(e) => {
                            let _ = done_tx.send_blocking(Err(e.to_string()));
                        }
                    }
                });

                // Progress polling on GTK main thread
                let page_w2 = page_w.clone();
                glib::spawn_future_local(async move {
                    while let Ok((downloaded, total)) = progress_rx.recv().await {
                        if let Some(page) = page_w2.upgrade() {
                            let frac = if total > 0 {
                                downloaded as f64 / total as f64
                            } else {
                                0.0
                            };
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
                        // Clear the shared cancel state and hide the Cancel button.
                        if let Some(page) = page_w.upgrade() {
                            *page.imp().current_cancel.borrow_mut() = None;
                            if let Some(cb) = page.imp().cancel_btn.borrow().as_ref() {
                                cb.set_visible(false);
                            }
                        }
                        match result {
                            Ok(()) => {
                                if let Some(btn) = btn_w.upgrade() {
                                    btn.set_label(gettext("Downloaded").as_str());
                                    btn.remove_css_class("pill");
                                    btn.add_css_class("success");
                                    btn.add_css_class("pill");
                                }
                                if let Some(del) = delete_w.upgrade() {
                                    del.set_visible(true);
                                }
                                if let Some(page) = page_w.upgrade() {
                                    page.set_download_progress(1.0, "Download complete!");

                                    // Save config: store the exact downloaded model ID
                                    let mut config = AppConfig::load();
                                    config.selected_model = model_id_for_load.clone();
                                    config.first_run = false;
                                    config.save();

                                    // Find MainWindow and load the exact downloaded model
                                    if let Some(window) = page.ancestor(gtk::Window::static_type())
                                    {
                                        if let Some(window) = window.downcast_ref::<gtk::Window>() {
                                            if let Some(main_window) =
                                                window.downcast_ref::<crate::ui::MainWindow>()
                                            {
                                                main_window.load_model_by_id(&model_id_for_load);
                                            }
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                let cancelled = e.contains("cancelled");
                                if let Some(btn) = btn_w.upgrade() {
                                    btn.set_label(if cancelled { "Download" } else { "Retry" });
                                    btn.set_sensitive(true);
                                }
                                if let Some(page) = page_w.upgrade() {
                                    if cancelled {
                                        page.set_download_progress(0.0, "Download cancelled");
                                    } else {
                                        page.set_download_progress(0.0, &format!("Error: {}", e));
                                    }
                                }
                            }
                        }
                    }
                });
            });

            // Wire delete (with confirmation)
            {
                let model_id = model_info.id.clone();
                let model_name = model_info.display_name.clone();
                let action_weak = action_btn.downgrade();
                let delete_weak = delete_btn.downgrade();
                let page_weak_del = self.downgrade();
                delete_btn.connect_clicked(move |btn| {
                    let dialog = adw::AlertDialog::new(
                        Some("Delete model?"),
                        Some(&format!(
                            "This removes the {} model from disk. You can re-download it anytime.",
                            model_name
                        )),
                    );
                    dialog.add_response("cancel", "Cancel");
                    dialog.add_response("delete", "Delete");
                    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
                    dialog.set_default_response(Some("cancel"));
                    dialog.set_close_response("cancel");

                    let model_id = model_id.clone();
                    let action_weak = action_weak.clone();
                    let delete_weak = delete_weak.clone();
                    let page_weak_del = page_weak_del.clone();
                    dialog.choose(btn, gtk::gio::Cancellable::NONE, move |resp| {
                        if resp.as_str() != "delete" {
                            return;
                        }
                        match ModelCatalog::delete_model(&model_id) {
                            Ok(()) => {
                                if let Some(a) = action_weak.upgrade() {
                                    a.set_label(gettext("Download").as_str());
                                    a.set_sensitive(true);
                                    a.remove_css_class("success");
                                }
                                if let Some(d) = delete_weak.upgrade() {
                                    d.set_visible(false);
                                }
                                // Refresh the main window's dropdown / active model.
                                if let Some(page) = page_weak_del.upgrade() {
                                    if let Some(main_window) = page
                                        .ancestor(gtk::Window::static_type())
                                        .and_then(|w| w.downcast::<crate::ui::MainWindow>().ok())
                                    {
                                        main_window.handle_model_deleted(&model_id);
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!("Failed to delete model {}: {}", model_id, e);
                            }
                        }
                    });
                });
            }

            row.add_suffix(&delete_btn);
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
        progress_group.set_title(gettext("Download Progress").as_str());

        let progress_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        progress_box.set_margin_top(8);
        progress_box.set_margin_bottom(8);
        progress_box.set_margin_start(12);
        progress_box.set_margin_end(12);

        let progress_bar = gtk::ProgressBar::new();
        progress_bar.set_show_text(true);
        progress_bar.set_fraction(0.0);
        progress_bar.set_text(Some(gettext("No download in progress").as_str()));
        progress_box.append(&progress_bar);

        let download_status = gtk::Label::new(Some(gettext("").as_str()));
        download_status.add_css_class("caption");
        download_status.add_css_class("dim-label");
        download_status.set_xalign(0.0);
        progress_box.append(&download_status);

        // Cancel button — appears only while a download is running.
        let cancel_btn = gtk::Button::with_label("Cancel");
        cancel_btn.set_halign(gtk::Align::End);
        cancel_btn.set_margin_top(4);
        cancel_btn.add_css_class("pill");
        cancel_btn.add_css_class("destructive-action");
        cancel_btn.set_visible(false);
        {
            let page_weak_cancel = self.downgrade();
            cancel_btn.connect_clicked(move |btn| {
                btn.set_sensitive(false);
                if let Some(page) = page_weak_cancel.upgrade() {
                    if let Some(flag) = page.imp().current_cancel.borrow().as_ref() {
                        flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            });
        }
        progress_box.append(&cancel_btn);

        // Wrap in a preferences row
        let progress_row = adw::ActionRow::new();
        progress_row.set_child(Some(&progress_box));
        progress_group.add(&progress_row);

        self.add(&progress_group);

        *imp.progress_bar.borrow_mut() = Some(progress_bar);
        *imp.download_status.borrow_mut() = Some(download_status);
        *imp.cancel_btn.borrow_mut() = Some(cancel_btn);

        // Storage info group
        let storage_group = adw::PreferencesGroup::new();
        storage_group.set_title(gettext("Storage").as_str());
        storage_group.set_description(Some(
            gettext("Choose where Whisper models are stored.").as_str(),
        ));

        let models_dir = crate::config::AppConfig::models_dir();
        let storage_row = adw::ActionRow::builder()
            .title(gettext("Models Directory").as_str())
            .subtitle(&*models_dir.to_string_lossy())
            .build();

        let open_btn = gtk::Button::from_icon_name("folder-open-symbolic");
        open_btn.set_tooltip_text(Some(gettext("Open in file manager").as_str()));
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
        change_btn.set_tooltip_text(Some(gettext("Change models directory").as_str()));
        change_btn.set_valign(gtk::Align::Center);
        change_btn.add_css_class("flat");
        let storage_row_weak = storage_row.downgrade();
        change_btn.connect_clicked(move |btn| {
            let dialog = gtk::FileDialog::builder()
                .title(gettext("Select Models Directory").as_str())
                .build();

            let storage_row_w = storage_row_weak.clone();
            if let Some(window) = btn.root().and_then(|r| r.downcast::<gtk::Window>().ok()) {
                let window2 = window.clone();
                dialog.select_folder(Some(&window), gtk::gio::Cancellable::NONE, move |result| {
                    let Ok(folder) = result else { return };
                    let Some(path) = folder.path() else { return };
                    let path_str = path.to_string_lossy().to_string();

                    // Warn before trusting a custom location: model files there
                    // are loaded by the app, so it should be a private directory
                    // the user controls (not shared/world-writable).
                    let confirm = adw::AlertDialog::new(
                        Some(gettext("Use this folder for models?").as_str()),
                        Some(&gettext(
                            "Model files will be stored in and loaded from:\n{path}\n\nChoose a private location you control — models read from a shared or world-writable directory could be replaced by another user.",
                        ).replace("{path}", &path_str)),
                    );
                    confirm.add_response("cancel", gettext("Cancel").as_str());
                    confirm.add_response("use", gettext("Use Folder").as_str());
                    confirm.set_response_appearance("use", adw::ResponseAppearance::Suggested);
                    confirm.set_default_response(Some("cancel"));
                    confirm.set_close_response("cancel");

                    let storage_row_w = storage_row_w.clone();
                    confirm.choose(&window2, gtk::gio::Cancellable::NONE, move |resp| {
                        if resp.as_str() != "use" {
                            return;
                        }
                        let mut config = crate::config::AppConfig::load();
                        config.model_directory = Some(path_str.clone());
                        config.save();
                        if let Some(row) = storage_row_w.upgrade() {
                            row.set_subtitle(&path_str);
                        }
                    });
                });
            }
        });
        storage_row.add_suffix(&change_btn);

        let reset_btn = gtk::Button::from_icon_name("edit-undo-symbolic");
        reset_btn.set_tooltip_text(Some(gettext("Reset to default location").as_str()));
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

        // Cohere Transcribe section
        let cohere_group = adw::PreferencesGroup::new();
        cohere_group.set_title(gettext("Cohere Transcribe").as_str());
        cohere_group.set_description(Some(
            "High-accuracy multilingual speech-to-text (14 languages). \
             Requires a free HuggingFace account to download the model.",
        ));

        // Token entry — masked (PasswordEntryRow) with a built-in reveal toggle.
        let token_entry = adw::PasswordEntryRow::builder()
            .title(gettext("HuggingFace Token").as_str())
            .show_apply_button(true)
            .build();

        // Pre-fill the token from the system keyring (async).
        {
            let entry_weak = token_entry.downgrade();
            let (tx, rx) = async_channel::bounded::<Option<String>>(1);
            tokio_runtime().spawn(async move {
                let _ = tx.send(crate::secrets::load_hf_token().await).await;
            });
            glib::spawn_future_local(async move {
                if let Ok(Some(token)) = rx.recv().await {
                    if let Some(entry) = entry_weak.upgrade() {
                        entry.set_text(&token);
                    }
                }
            });
        }

        // Persist the token to the keyring (never to plaintext config), and tell
        // the user if the keyring rejected it instead of failing silently.
        token_entry.connect_apply(|entry| {
            let text = entry.text().to_string();
            let (tx, rx) = async_channel::bounded::<Result<(), String>>(1);
            tokio_runtime().spawn(async move {
                let res = if text.is_empty() {
                    crate::secrets::delete_hf_token().await
                } else {
                    crate::secrets::store_hf_token(&text).await
                };
                let _ = tx.send(res.map_err(|e| e.to_string())).await;
            });
            let entry_weak = entry.downgrade();
            glib::spawn_future_local(async move {
                if let Ok(Err(err)) = rx.recv().await {
                    tracing::warn!("Could not store HuggingFace token in keyring: {}", crate::error::redact_secrets(&err));
                    if let Some(entry) = entry_weak.upgrade() {
                        let dialog = adw::AlertDialog::new(
                            Some(gettext("Keyring Error").as_str()),
                            Some(&format!(
                                "{}\n\n{}",
                                gettext("Couldn't save the HuggingFace token to the system keyring, so it was not stored. Check that a keyring service (GNOME Keyring / KWallet) is running and unlocked."),
                                crate::error::redact_secrets(&err)
                            )),
                        );
                        dialog.add_response("ok", gettext("OK").as_str());
                        dialog.set_default_response(Some("ok"));
                        dialog.set_close_response("ok");
                        dialog.present(Some(&entry));
                    }
                }
            });
        });

        cohere_group.add(&token_entry);

        // Token instructions row
        let token_info_row = adw::ActionRow::builder()
            .title(gettext("Get a Token").as_str())
            .subtitle(
                "1. Create a free account at huggingface.co\n\
                 2. Visit the model page and click \"Agree and access repository\"\n\
                 3. Go to Settings → Access Tokens and create a token with read permission",
            )
            .build();

        let get_token_btn = gtk::Button::with_label("Open HuggingFace");
        get_token_btn.set_valign(gtk::Align::Center);
        get_token_btn.add_css_class("pill");
        get_token_btn.connect_clicked(|btn| {
            let uri = "https://huggingface.co/settings/tokens";
            if let Some(root) = btn.root().and_then(|r| r.downcast::<gtk::Window>().ok()) {
                let launcher = gtk::UriLauncher::new(uri);
                launcher.launch(Some(&root), gtk::gio::Cancellable::NONE, |_| {});
            }
        });
        token_info_row.add_suffix(&get_token_btn);
        cohere_group.add(&token_info_row);

        // Runtime download row — laid out exactly like the Whisper model rows:
        // a dim size label, a trash button (shown only when installed), and the
        // Download/Downloaded pill. No "Installed" status label.
        let runtime_row = adw::ActionRow::builder()
            .title(gettext("Runtime").as_str())
            .subtitle(&format!("~123 MB — {}", gettext("Transcription engine")))
            .build();

        let runtime_size = gtk::Label::new(Some("~123 MB"));
        runtime_size.add_css_class("dim-label");
        runtime_size.add_css_class("caption");
        runtime_row.add_suffix(&runtime_size);

        let runtime_delete = gtk::Button::from_icon_name("user-trash-symbolic");
        runtime_delete.set_valign(gtk::Align::Center);
        runtime_delete.add_css_class("flat");
        runtime_delete.add_css_class("circular");
        runtime_delete.set_tooltip_text(Some(gettext("Delete the runtime from disk").as_str()));
        runtime_delete.set_visible(crate::transcription::cohere::is_runtime_installed());

        let runtime_btn = gtk::Button::new();
        runtime_btn.set_valign(gtk::Align::Center);
        runtime_btn.add_css_class("pill");
        if crate::transcription::cohere::is_runtime_installed() {
            runtime_btn.set_label(gettext("Downloaded").as_str());
            runtime_btn.set_sensitive(false);
            runtime_btn.add_css_class("success");
        } else {
            runtime_btn.set_label(gettext("Download").as_str());
        }

        // Download
        {
            let page_weak = self.downgrade();
            let btn_clone = runtime_btn.clone();
            let delete_w_for_dl = runtime_delete.downgrade();
            runtime_btn.connect_clicked(move |_| {
                btn_clone.set_sensitive(false);
                btn_clone.set_label(gettext("Downloading…").as_str());

                let page_w = page_weak.clone();
                let btn_w = btn_clone.downgrade();
                let delete_w = delete_w_for_dl.clone();

                let (progress_tx, progress_rx) = async_channel::bounded::<(u64, u64)>(64);
                let (done_tx, done_rx) = async_channel::bounded::<Result<(), String>>(1);

                let rt = crate::application::tokio_runtime();
                rt.spawn(async move {
                    match crate::transcription::cohere::download_runtime(move |dl, total| {
                        let _ = progress_tx.send_blocking((dl, total));
                    })
                    .await
                    {
                        Ok(_) => {
                            let _ = done_tx.send_blocking(Ok(()));
                        }
                        Err(e) => {
                            let _ = done_tx.send_blocking(Err(e.to_string()));
                        }
                    }
                });

                let page_w2 = page_w.clone();
                glib::spawn_future_local(async move {
                    while let Ok((downloaded, total)) = progress_rx.recv().await {
                        if let Some(page) = page_w2.upgrade() {
                            let frac = if total > 0 {
                                downloaded as f64 / total as f64
                            } else {
                                0.0
                            };
                            let mb_down = downloaded as f64 / 1_000_000.0;
                            let mb_total = total as f64 / 1_000_000.0;
                            page.set_download_progress(
                                frac,
                                &format!("Runtime: {:.0} / {:.0} MB", mb_down, mb_total),
                            );
                        }
                    }
                });

                glib::spawn_future_local(async move {
                    if let Ok(result) = done_rx.recv().await {
                        match result {
                            Ok(()) => {
                                if let Some(btn) = btn_w.upgrade() {
                                    btn.set_label(gettext("Downloaded").as_str());
                                    btn.set_sensitive(false);
                                    btn.add_css_class("success");
                                }
                                if let Some(del) = delete_w.upgrade() {
                                    del.set_visible(true);
                                }
                                if let Some(page) = page_w.upgrade() {
                                    page.set_download_progress(1.0, "Runtime download complete!");
                                }
                            }
                            Err(e) => {
                                if let Some(btn) = btn_w.upgrade() {
                                    btn.set_label(gettext("Retry").as_str());
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
        }

        // Delete (with confirmation) — mirrors the Whisper rows.
        {
            let action_weak = runtime_btn.downgrade();
            let delete_weak = runtime_delete.downgrade();
            runtime_delete.connect_clicked(move |btn| {
                let dialog = adw::AlertDialog::new(
                    Some("Delete runtime?"),
                    Some("This removes the Cohere runtime (~123 MB) from disk. You can re-download it anytime."),
                );
                dialog.add_response("cancel", "Cancel");
                dialog.add_response("delete", "Delete");
                dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
                dialog.set_default_response(Some("cancel"));
                dialog.set_close_response("cancel");

                let action_weak = action_weak.clone();
                let delete_weak = delete_weak.clone();
                dialog.choose(btn, gtk::gio::Cancellable::NONE, move |resp| {
                    if resp.as_str() != "delete" {
                        return;
                    }
                    match crate::transcription::cohere::delete_runtime() {
                        Ok(()) => {
                            if let Some(a) = action_weak.upgrade() {
                                a.set_label(gettext("Download").as_str());
                                a.set_sensitive(true);
                                a.remove_css_class("success");
                            }
                            if let Some(d) = delete_weak.upgrade() {
                                d.set_visible(false);
                            }
                        }
                        Err(e) => tracing::warn!("Failed to delete Cohere runtime: {}", e),
                    }
                });
            });
        }

        runtime_row.add_suffix(&runtime_delete);
        runtime_row.add_suffix(&runtime_btn);
        cohere_group.add(&runtime_row);

        // Model download row — same layout as the Whisper rows (size label, trash
        // button shown only when downloaded, Download/Downloaded pill).
        let model_row = adw::ActionRow::builder()
            .title(gettext("Model Weights").as_str())
            .subtitle(&format!("~4.1 GB — {}", gettext("Cohere Transcribe model")))
            .build();

        let model_size = gtk::Label::new(Some("~4.1 GB"));
        model_size.add_css_class("dim-label");
        model_size.add_css_class("caption");
        model_row.add_suffix(&model_size);

        let model_delete = gtk::Button::from_icon_name("user-trash-symbolic");
        model_delete.set_valign(gtk::Align::Center);
        model_delete.add_css_class("flat");
        model_delete.add_css_class("circular");
        model_delete.set_tooltip_text(Some(gettext("Delete the model weights from disk").as_str()));
        model_delete.set_visible(crate::transcription::cohere::is_model_downloaded());

        let model_btn = gtk::Button::new();
        model_btn.set_valign(gtk::Align::Center);
        model_btn.add_css_class("pill");
        if crate::transcription::cohere::is_model_downloaded() {
            model_btn.set_label(gettext("Downloaded").as_str());
            model_btn.set_sensitive(false);
            model_btn.add_css_class("success");
        } else {
            model_btn.set_label(gettext("Download").as_str());
        }

        // Download
        {
            let page_weak = self.downgrade();
            let btn_clone = model_btn.clone();
            let delete_w_for_dl = model_delete.downgrade();
            let token_entry_weak = token_entry.downgrade();
            model_btn.connect_clicked(move |_| {
                let token = token_entry_weak
                    .upgrade()
                    .map(|e| e.text().to_string())
                    .unwrap_or_default();

                if token.is_empty() {
                    if let Some(page) = page_weak.upgrade() {
                        page.set_download_progress(
                            0.0,
                            "Please enter your HuggingFace token first.",
                        );
                    }
                    return;
                }

                if !crate::transcription::cohere::is_runtime_installed() {
                    if let Some(page) = page_weak.upgrade() {
                        page.set_download_progress(0.0, "Please download the runtime first.");
                    }
                    return;
                }

                btn_clone.set_sensitive(false);
                btn_clone.set_label(gettext("Downloading…").as_str());

                let page_w = page_weak.clone();
                let btn_w = btn_clone.downgrade();
                let delete_w = delete_w_for_dl.clone();

                let (progress_tx, progress_rx) = async_channel::bounded::<(u64, u64)>(64);
                let (done_tx, done_rx) = async_channel::bounded::<Result<(), String>>(1);

                let rt = crate::application::tokio_runtime();
                rt.spawn(async move {
                    // Persist the token securely in the keyring (not plaintext config).
                    if let Err(e) = crate::secrets::store_hf_token(&token).await {
                        tracing::warn!(
                            "Could not store HuggingFace token in keyring: {}",
                            crate::error::redact_secrets(&e.to_string())
                        );
                    }
                    match crate::transcription::cohere::download_model(&token, move |dl, total| {
                        let _ = progress_tx.send_blocking((dl, total));
                    })
                    .await
                    {
                        Ok(_) => {
                            let _ = done_tx.send_blocking(Ok(()));
                        }
                        Err(e) => {
                            let _ = done_tx.send_blocking(Err(e.to_string()));
                        }
                    }
                });

                let page_w2 = page_w.clone();
                glib::spawn_future_local(async move {
                    while let Ok((downloaded, total)) = progress_rx.recv().await {
                        if let Some(page) = page_w2.upgrade() {
                            let frac = if total > 0 {
                                downloaded as f64 / total as f64
                            } else {
                                0.0
                            };
                            let gb_down = downloaded as f64 / 1_000_000_000.0;
                            let gb_total = total as f64 / 1_000_000_000.0;
                            page.set_download_progress(
                                frac,
                                &format!("Model: {:.2} / {:.2} GB", gb_down, gb_total),
                            );
                        }
                    }
                });

                glib::spawn_future_local(async move {
                    if let Ok(result) = done_rx.recv().await {
                        match result {
                            Ok(()) => {
                                if let Some(btn) = btn_w.upgrade() {
                                    btn.set_label(gettext("Downloaded").as_str());
                                    btn.set_sensitive(false);
                                    btn.add_css_class("success");
                                }
                                if let Some(del) = delete_w.upgrade() {
                                    del.set_visible(true);
                                }
                                if let Some(page) = page_w.upgrade() {
                                    page.set_download_progress(1.0, "Model download complete!");
                                }
                            }
                            Err(e) => {
                                if let Some(btn) = btn_w.upgrade() {
                                    btn.set_label(gettext("Retry").as_str());
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
        }

        // Delete (with confirmation) — mirrors the Whisper rows.
        {
            let action_weak = model_btn.downgrade();
            let delete_weak = model_delete.downgrade();
            model_delete.connect_clicked(move |btn| {
                let dialog = adw::AlertDialog::new(
                    Some("Delete model weights?"),
                    Some("This removes the Cohere Transcribe model (~4.1 GB) from disk. You can re-download it anytime."),
                );
                dialog.add_response("cancel", "Cancel");
                dialog.add_response("delete", "Delete");
                dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
                dialog.set_default_response(Some("cancel"));
                dialog.set_close_response("cancel");

                let action_weak = action_weak.clone();
                let delete_weak = delete_weak.clone();
                dialog.choose(btn, gtk::gio::Cancellable::NONE, move |resp| {
                    if resp.as_str() != "delete" {
                        return;
                    }
                    match crate::transcription::cohere::delete_model() {
                        Ok(()) => {
                            if let Some(a) = action_weak.upgrade() {
                                a.set_label(gettext("Download").as_str());
                                a.set_sensitive(true);
                                a.remove_css_class("success");
                            }
                            if let Some(d) = delete_weak.upgrade() {
                                d.set_visible(false);
                            }
                        }
                        Err(e) => tracing::warn!("Failed to delete Cohere model: {}", e),
                    }
                });
            });
        }

        model_row.add_suffix(&model_delete);
        model_row.add_suffix(&model_btn);
        cohere_group.add(&model_row);

        self.add(&cohere_group);
        *imp.cohere_group.borrow_mut() = Some(cohere_group);

        // ── Qwen3-ASR section (local, ungated — no token needed) ────────────
        let qwen_group = adw::PreferencesGroup::new();
        qwen_group.set_title(gettext("Qwen3-ASR").as_str());
        qwen_group.set_description(Some(
            "Local multilingual speech-to-text (30 languages incl. Greek). \
             Runs offline; no account or token required.",
        ));

        // Runtime row
        let q_runtime_row = adw::ActionRow::builder()
            .title(gettext("Runtime").as_str())
            .subtitle(&format!("~500 MB — {}", gettext("Engine + libtorch")))
            .build();
        let q_runtime_size = gtk::Label::new(Some("~500 MB"));
        q_runtime_size.add_css_class("dim-label");
        q_runtime_size.add_css_class("caption");
        q_runtime_row.add_suffix(&q_runtime_size);

        let q_runtime_delete = gtk::Button::from_icon_name("user-trash-symbolic");
        q_runtime_delete.set_valign(gtk::Align::Center);
        q_runtime_delete.add_css_class("flat");
        q_runtime_delete.add_css_class("circular");
        q_runtime_delete.set_tooltip_text(Some(gettext("Delete the runtime from disk").as_str()));
        q_runtime_delete.set_visible(crate::transcription::qwen::is_runtime_installed());

        let q_runtime_btn = gtk::Button::new();
        q_runtime_btn.set_valign(gtk::Align::Center);
        q_runtime_btn.add_css_class("pill");
        if crate::transcription::qwen::is_runtime_installed() {
            q_runtime_btn.set_label(gettext("Downloaded").as_str());
            q_runtime_btn.set_sensitive(false);
            q_runtime_btn.add_css_class("success");
        } else {
            q_runtime_btn.set_label(gettext("Download").as_str());
        }
        {
            let page_weak = self.downgrade();
            let btn_clone = q_runtime_btn.clone();
            let delete_w_for_dl = q_runtime_delete.downgrade();
            q_runtime_btn.connect_clicked(move |_| {
                btn_clone.set_sensitive(false);
                btn_clone.set_label(gettext("Downloading…").as_str());
                let page_w = page_weak.clone();
                let btn_w = btn_clone.downgrade();
                let delete_w = delete_w_for_dl.clone();
                let (progress_tx, progress_rx) = async_channel::bounded::<(u64, u64)>(64);
                let (done_tx, done_rx) = async_channel::bounded::<Result<(), String>>(1);
                let rt = crate::application::tokio_runtime();
                rt.spawn(async move {
                    match crate::transcription::qwen::download_runtime(move |dl, total| {
                        let _ = progress_tx.send_blocking((dl, total));
                    })
                    .await
                    {
                        Ok(_) => {
                            let _ = done_tx.send_blocking(Ok(()));
                        }
                        Err(e) => {
                            let _ = done_tx.send_blocking(Err(e.to_string()));
                        }
                    }
                });
                let page_w2 = page_w.clone();
                glib::spawn_future_local(async move {
                    while let Ok((downloaded, total)) = progress_rx.recv().await {
                        if let Some(page) = page_w2.upgrade() {
                            let frac = if total > 0 {
                                downloaded as f64 / total as f64
                            } else {
                                0.0
                            };
                            page.set_download_progress(
                                frac,
                                &format!(
                                    "Runtime: {:.0} / {:.0} MB",
                                    downloaded as f64 / 1_000_000.0,
                                    total as f64 / 1_000_000.0
                                ),
                            );
                        }
                    }
                });
                glib::spawn_future_local(async move {
                    if let Ok(result) = done_rx.recv().await {
                        match result {
                            Ok(()) => {
                                if let Some(btn) = btn_w.upgrade() {
                                    btn.set_label(gettext("Downloaded").as_str());
                                    btn.set_sensitive(false);
                                    btn.add_css_class("success");
                                }
                                if let Some(del) = delete_w.upgrade() {
                                    del.set_visible(true);
                                }
                                if let Some(page) = page_w.upgrade() {
                                    page.set_download_progress(1.0, "Runtime download complete!");
                                }
                            }
                            Err(e) => {
                                if let Some(btn) = btn_w.upgrade() {
                                    btn.set_label(gettext("Retry").as_str());
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
        }
        {
            let action_weak = q_runtime_btn.downgrade();
            let delete_weak = q_runtime_delete.downgrade();
            q_runtime_delete.connect_clicked(move |btn| {
                let dialog = adw::AlertDialog::new(Some("Delete runtime?"), Some("This removes the Qwen3-ASR runtime from disk. You can re-download it anytime."));
                dialog.add_response("cancel", "Cancel");
                dialog.add_response("delete", "Delete");
                dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
                dialog.set_default_response(Some("cancel"));
                dialog.set_close_response("cancel");
                let action_weak = action_weak.clone();
                let delete_weak = delete_weak.clone();
                dialog.choose(btn, gtk::gio::Cancellable::NONE, move |resp| {
                    if resp.as_str() != "delete" { return; }
                    match crate::transcription::qwen::delete_runtime() {
                        Ok(()) => {
                            if let Some(a) = action_weak.upgrade() { a.set_label(gettext("Download").as_str()); a.set_sensitive(true); a.remove_css_class("success"); }
                            if let Some(d) = delete_weak.upgrade() { d.set_visible(false); }
                        }
                        Err(e) => tracing::warn!("Failed to delete Qwen runtime: {}", e),
                    }
                });
            });
        }
        q_runtime_row.add_suffix(&q_runtime_delete);
        q_runtime_row.add_suffix(&q_runtime_btn);
        qwen_group.add(&q_runtime_row);

        // One downloadable row per size (both can coexist on disk). The active
        // size is chosen from the top-bar model dropdown, like Whisper.
        let q_row_small = self.build_qwen_model_row("0.6B", "~0.9 GB");
        qwen_group.add(&q_row_small);
        let q_row_full = self.build_qwen_model_row("1.7B", "~2.4 GB");
        qwen_group.add(&q_row_full);

        self.add(&qwen_group);
    }

    /// Build a downloadable Qwen3-ASR model row for a specific size. Download and
    /// delete operate on that size; progress is reported on the shared bar.
    fn build_qwen_model_row(&self, size: &'static str, approx: &'static str) -> adw::ActionRow {
        let row = adw::ActionRow::builder()
            .title(&format!("{} — {}", gettext("Model Weights"), size))
            .subtitle(&format!("{} — Qwen3-ASR-{} model", approx, size))
            .build();

        let size_label = gtk::Label::new(Some(approx));
        size_label.add_css_class("dim-label");
        size_label.add_css_class("caption");
        row.add_suffix(&size_label);

        let delete_btn = gtk::Button::from_icon_name("user-trash-symbolic");
        delete_btn.set_valign(gtk::Align::Center);
        delete_btn.add_css_class("flat");
        delete_btn.add_css_class("circular");
        delete_btn.set_tooltip_text(Some(gettext("Delete the model weights from disk").as_str()));
        delete_btn.set_visible(crate::transcription::qwen::is_model_downloaded_size(size));

        let action_btn = gtk::Button::new();
        action_btn.set_valign(gtk::Align::Center);
        action_btn.add_css_class("pill");
        if crate::transcription::qwen::is_model_downloaded_size(size) {
            action_btn.set_label(gettext("Downloaded").as_str());
            action_btn.set_sensitive(false);
            action_btn.add_css_class("success");
        } else {
            action_btn.set_label(gettext("Download").as_str());
        }

        // Download
        {
            let page_weak = self.downgrade();
            let btn_clone = action_btn.clone();
            let delete_w_for_dl = delete_btn.downgrade();
            action_btn.connect_clicked(move |_| {
                btn_clone.set_sensitive(false);
                btn_clone.set_label(gettext("Downloading…").as_str());
                let page_w = page_weak.clone();
                let btn_w = btn_clone.downgrade();
                let delete_w = delete_w_for_dl.clone();
                let (progress_tx, progress_rx) = async_channel::bounded::<(u64, u64)>(64);
                let (done_tx, done_rx) = async_channel::bounded::<Result<(), String>>(1);
                let rt = crate::application::tokio_runtime();
                rt.spawn(async move {
                    match crate::transcription::qwen::download_model(size, move |dl, total| {
                        let _ = progress_tx.send_blocking((dl, total));
                    })
                    .await
                    {
                        Ok(_) => {
                            let _ = done_tx.send_blocking(Ok(()));
                        }
                        Err(e) => {
                            let _ = done_tx.send_blocking(Err(e.to_string()));
                        }
                    }
                });
                let page_w2 = page_w.clone();
                glib::spawn_future_local(async move {
                    while let Ok((downloaded, total)) = progress_rx.recv().await {
                        if let Some(page) = page_w2.upgrade() {
                            let frac = if total > 0 {
                                downloaded as f64 / total as f64
                            } else {
                                0.0
                            };
                            page.set_download_progress(
                                frac,
                                &format!(
                                    "Model {}: {:.2} / {:.2} GB",
                                    size,
                                    downloaded as f64 / 1_000_000_000.0,
                                    total as f64 / 1_000_000_000.0
                                ),
                            );
                        }
                    }
                });
                glib::spawn_future_local(async move {
                    if let Ok(result) = done_rx.recv().await {
                        match result {
                            Ok(()) => {
                                if let Some(btn) = btn_w.upgrade() {
                                    btn.set_label(gettext("Downloaded").as_str());
                                    btn.set_sensitive(false);
                                    btn.add_css_class("success");
                                }
                                if let Some(del) = delete_w.upgrade() {
                                    del.set_visible(true);
                                }
                                if let Some(page) = page_w.upgrade() {
                                    page.set_download_progress(1.0, "Model download complete!");
                                }
                            }
                            Err(e) => {
                                if let Some(btn) = btn_w.upgrade() {
                                    btn.set_label(gettext("Retry").as_str());
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
        }

        // Delete (with confirmation)
        {
            let action_weak = action_btn.downgrade();
            let delete_weak = delete_btn.downgrade();
            delete_btn.connect_clicked(move |btn| {
                let dialog = adw::AlertDialog::new(
                    Some("Delete model weights?"),
                    Some(&format!("This removes the Qwen3-ASR {} model from disk. You can re-download it anytime.", size)),
                );
                dialog.add_response("cancel", "Cancel");
                dialog.add_response("delete", "Delete");
                dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
                dialog.set_default_response(Some("cancel"));
                dialog.set_close_response("cancel");
                let action_weak = action_weak.clone();
                let delete_weak = delete_weak.clone();
                dialog.choose(btn, gtk::gio::Cancellable::NONE, move |resp| {
                    if resp.as_str() != "delete" { return; }
                    match crate::transcription::qwen::delete_model(size) {
                        Ok(()) => {
                            if let Some(a) = action_weak.upgrade() { a.set_label(gettext("Download").as_str()); a.set_sensitive(true); a.remove_css_class("success"); }
                            if let Some(d) = delete_weak.upgrade() { d.set_visible(false); }
                        }
                        Err(e) => tracing::warn!("Failed to delete Qwen model: {}", e),
                    }
                });
            });
        }

        row.add_suffix(&delete_btn);
        row.add_suffix(&action_btn);
        row
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
            bar.set_text(Some(gettext("No download in progress").as_str()));
        }
        if let Some(label) = imp.download_status.borrow().as_ref() {
            label.set_text("");
        }
    }
}
