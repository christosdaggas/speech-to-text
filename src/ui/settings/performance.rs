// Speech to Text - Performance Settings Page
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! CPU threads, GPU acceleration, and performance tuning settings.

use crate::i18n::gettext;
use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk4 as gtk;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::RefCell;

use crate::config::AppConfig;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct PerformancePage {
        pub gpu_switch: RefCell<Option<adw::SwitchRow>>,
        pub cpu_fallback_switch: RefCell<Option<adw::SwitchRow>>,
        pub threads_spin: RefCell<Option<adw::SpinRow>>,
        pub beam_spin: RefCell<Option<adw::SpinRow>>,
        pub temperature_spin: RefCell<Option<adw::SpinRow>>,
        pub gpu_info_row: RefCell<Option<adw::ActionRow>>,
        pub gpu_status_icon: RefCell<Option<gtk::Image>>,
        pub initial_prompt_entry: RefCell<Option<adw::EntryRow>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PerformancePage {
        const NAME: &'static str = "SttPerformancePage";
        type Type = super::PerformancePage;
        type ParentType = adw::PreferencesPage;
    }

    impl ObjectImpl for PerformancePage {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_ui();
        }
    }

    impl WidgetImpl for PerformancePage {}
    impl adw::subclass::prelude::PreferencesPageImpl for PerformancePage {}
}

glib::wrapper! {
    pub struct PerformancePage(ObjectSubclass<imp::PerformancePage>)
        @extends gtk::Widget, adw::PreferencesPage;
}

impl PerformancePage {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("title", "Performance")
            .property("icon-name", "preferences-system-symbolic")
            .build()
    }

    fn setup_ui(&self) {
        let imp = self.imp();
        let max_threads = num_cpus::get() as f64;

        // GPU Acceleration group
        let gpu_group = adw::PreferencesGroup::new();
        gpu_group.set_title(gettext("GPU Acceleration").as_str());
        gpu_group.set_description(Some(
            "Use GPU for faster transcription when available. \
             Requires CUDA or OpenCL support.",
        ));

        let gpu_switch = adw::SwitchRow::builder()
            .title(gettext("Use GPU").as_str())
            .subtitle(gettext("Accelerate transcription with your graphics card").as_str())
            .active(true)
            .build();
        gpu_group.add(&gpu_switch);

        let cpu_fallback = adw::SwitchRow::builder()
            .title(gettext("CPU Fallback").as_str())
            .subtitle(gettext("Automatically fall back to CPU if GPU is unavailable").as_str())
            .active(true)
            .build();
        gpu_group.add(&cpu_fallback);

        // GPU info row
        let gpu_info = adw::ActionRow::builder()
            .title(gettext("GPU Status").as_str())
            .subtitle(gettext("Detecting…").as_str())
            .build();
        let gpu_status_icon = gtk::Image::from_icon_name("video-display-symbolic");
        gpu_status_icon.set_pixel_size(16);
        gpu_info.add_suffix(&gpu_status_icon);
        gpu_group.add(&gpu_info);

        self.add(&gpu_group);

        // CPU group
        let cpu_group = adw::PreferencesGroup::new();
        cpu_group.set_title(gettext("CPU Settings").as_str());
        cpu_group.set_description(Some(&format!(
            "This system has {} CPU cores available",
            num_cpus::get()
        )));

        let threads_adjustment = gtk::Adjustment::new(
            4.0,         // value
            1.0,         // lower
            max_threads, // upper
            1.0,         // step_increment
            4.0,         // page_increment
            0.0,         // page_size
        );

        let threads_spin = adw::SpinRow::builder()
            .title(gettext("Worker Threads").as_str())
            .subtitle(gettext("Number of CPU threads for transcription").as_str())
            .adjustment(&threads_adjustment)
            .build();
        cpu_group.add(&threads_spin);

        self.add(&cpu_group);

        // Memory group
        let mem_group = adw::PreferencesGroup::new();
        mem_group.set_title(gettext("Memory").as_str());

        let mem_row = adw::ActionRow::builder()
            .title(gettext("Estimated Memory Usage").as_str())
            .subtitle(gettext("Depends on the selected model size").as_str())
            .build();
        let mem_label = gtk::Label::new(Some(gettext("~200 MB").as_str()));
        mem_label.add_css_class("dim-label");
        mem_row.add_suffix(&mem_label);
        mem_group.add(&mem_row);

        let beam_adjustment = gtk::Adjustment::new(
            5.0, // value
            1.0, // lower
            8.0, // upper (whisper.cpp max decoders = 8)
            1.0, // step_increment
            1.0, // page_increment
            0.0, // page_size
        );

        let beam_spin = adw::SpinRow::builder()
            .title(gettext("Beam Size").as_str())
            .subtitle(gettext("Higher values improve accuracy but use more memory").as_str())
            .adjustment(&beam_adjustment)
            .build();
        mem_group.add(&beam_spin);

        // Temperature
        let temp_adjustment = gtk::Adjustment::new(
            0.0,  // value (default: 0.0 = greedy)
            0.0,  // lower
            1.0,  // upper
            0.05, // step_increment
            0.1,  // page_increment
            0.0,  // page_size
        );

        let temperature_spin = adw::SpinRow::builder()
            .title(gettext("Temperature").as_str())
            .subtitle(gettext("0 = deterministic, higher values add randomness").as_str())
            .adjustment(&temp_adjustment)
            .digits(2)
            .build();
        mem_group.add(&temperature_spin);

        // Initial prompt / custom vocabulary
        let prompt_group = adw::PreferencesGroup::new();
        prompt_group.set_title(gettext("Custom Vocabulary").as_str());
        prompt_group.set_description(Some(
            "Provide an initial prompt to guide Whisper. \
             Include domain-specific terms, proper nouns, or acronyms.",
        ));

        let initial_prompt_entry = adw::EntryRow::builder()
            .title(gettext("Initial Prompt").as_str())
            .build();
        prompt_group.add(&initial_prompt_entry);

        self.add(&prompt_group);

        self.add(&mem_group);

        // Store references
        *imp.gpu_switch.borrow_mut() = Some(gpu_switch);
        *imp.cpu_fallback_switch.borrow_mut() = Some(cpu_fallback);
        *imp.threads_spin.borrow_mut() = Some(threads_spin);
        *imp.beam_spin.borrow_mut() = Some(beam_spin);
        *imp.temperature_spin.borrow_mut() = Some(temperature_spin);
        *imp.gpu_info_row.borrow_mut() = Some(gpu_info.clone());
        *imp.gpu_status_icon.borrow_mut() = Some(gpu_status_icon.clone());
        *imp.initial_prompt_entry.borrow_mut() = Some(initial_prompt_entry);

        // Restore saved settings into the widgets, THEN wire persistence so the
        // restore itself doesn't trigger redundant saves.
        self.load_from_config();
        self.connect_persistence();

        // Detect GPU status asynchronously
        self.detect_gpu_status();
    }

    /// Initialize widget values from the persisted configuration.
    fn load_from_config(&self) {
        let config = AppConfig::load();
        let imp = self.imp();
        if let Some(s) = imp.gpu_switch.borrow().as_ref() {
            s.set_active(config.use_gpu);
        }
        if let Some(s) = imp.cpu_fallback_switch.borrow().as_ref() {
            s.set_active(config.cpu_fallback);
        }
        if let Some(s) = imp.threads_spin.borrow().as_ref() {
            // 0 means "auto" — show the effective core count in the spinner.
            let threads = if config.n_threads == 0 {
                num_cpus::get() as u32
            } else {
                config.n_threads
            };
            s.set_value(threads as f64);
        }
        if let Some(s) = imp.beam_spin.borrow().as_ref() {
            s.set_value(config.beam_size as f64);
        }
        if let Some(s) = imp.temperature_spin.borrow().as_ref() {
            s.set_value(config.temperature as f64);
        }
        if let Some(e) = imp.initial_prompt_entry.borrow().as_ref() {
            if let Some(ref prompt) = config.initial_prompt {
                e.set_text(prompt);
            }
        }
    }

    /// Wire each control to persist its value to the configuration on change.
    fn connect_persistence(&self) {
        let imp = self.imp();
        if let Some(s) = imp.gpu_switch.borrow().as_ref() {
            s.connect_active_notify(|s| {
                let mut c = AppConfig::load();
                c.use_gpu = s.is_active();
                c.save();
            });
        }
        if let Some(s) = imp.cpu_fallback_switch.borrow().as_ref() {
            s.connect_active_notify(|s| {
                let mut c = AppConfig::load();
                c.cpu_fallback = s.is_active();
                c.save();
            });
        }
        if let Some(s) = imp.threads_spin.borrow().as_ref() {
            s.connect_value_notify(|s| {
                let mut c = AppConfig::load();
                c.n_threads = s.value() as u32;
                c.save();
            });
        }
        if let Some(s) = imp.beam_spin.borrow().as_ref() {
            s.connect_value_notify(|s| {
                let mut c = AppConfig::load();
                c.beam_size = s.value() as u32;
                c.save();
            });
        }
        if let Some(s) = imp.temperature_spin.borrow().as_ref() {
            s.connect_value_notify(|s| {
                let mut c = AppConfig::load();
                c.temperature = s.value() as f32;
                c.save();
            });
        }
        if let Some(e) = imp.initial_prompt_entry.borrow().as_ref() {
            e.connect_changed(|e| {
                let mut c = AppConfig::load();
                let text = e.text().to_string();
                c.initial_prompt = if text.is_empty() { None } else { Some(text) };
                c.save();
            });
        }
    }

    fn detect_gpu_status(&self) {
        let (sender, receiver) = async_channel::bounded::<Option<String>>(1);

        std::thread::spawn(move || {
            let result =
                crate::ui::widgets::gpu_status::detect_gpu_info().map(|(name, _driver, _vram)| {
                    crate::ui::widgets::gpu_status::shorten_gpu_name_public(&name)
                });
            let _ = sender.send_blocking(result);
        });

        let page = self.clone();
        glib::spawn_future_local(async move {
            if let Ok(result) = receiver.recv().await {
                let imp = page.imp();
                if let Some(row) = imp.gpu_info_row.borrow().as_ref() {
                    if let Some(icon) = imp.gpu_status_icon.borrow().as_ref() {
                        match result {
                            Some(name) => {
                                row.set_subtitle(&name);
                                icon.set_icon_name(Some("object-select-symbolic"));
                                icon.remove_css_class("dim-label");
                                icon.add_css_class("success");
                            }
                            None => {
                                row.set_subtitle(gettext("No GPU detected — using CPU").as_str());
                                icon.set_icon_name(Some("dialog-warning-symbolic"));
                            }
                        }
                    }
                }
            }
        });
    }

    /// Get current settings.
    pub fn get_gpu_enabled(&self) -> bool {
        self.imp()
            .gpu_switch
            .borrow()
            .as_ref()
            .map(|s| s.is_active())
            .unwrap_or(false)
    }

    pub fn get_cpu_fallback(&self) -> bool {
        self.imp()
            .cpu_fallback_switch
            .borrow()
            .as_ref()
            .map(|s| s.is_active())
            .unwrap_or(true)
    }

    pub fn get_thread_count(&self) -> u32 {
        self.imp()
            .threads_spin
            .borrow()
            .as_ref()
            .map(|s| s.value() as u32)
            .unwrap_or(4)
    }

    pub fn get_beam_size(&self) -> u32 {
        self.imp()
            .beam_spin
            .borrow()
            .as_ref()
            .map(|s| s.value() as u32)
            .unwrap_or(5)
    }

    pub fn get_temperature(&self) -> f32 {
        self.imp()
            .temperature_spin
            .borrow()
            .as_ref()
            .map(|s| s.value() as f32)
            .unwrap_or(0.0)
    }

    pub fn get_initial_prompt(&self) -> Option<String> {
        self.imp()
            .initial_prompt_entry
            .borrow()
            .as_ref()
            .map(|e| e.text().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Enable/disable Whisper-specific settings based on backend capabilities.
    pub fn set_whisper_settings_available(&self, available: bool) {
        let imp = self.imp();
        if let Some(s) = imp.beam_spin.borrow().as_ref() {
            s.set_sensitive(available);
        }
        if let Some(s) = imp.temperature_spin.borrow().as_ref() {
            s.set_sensitive(available);
        }
        if let Some(s) = imp.initial_prompt_entry.borrow().as_ref() {
            s.set_sensitive(available);
        }
    }
}
