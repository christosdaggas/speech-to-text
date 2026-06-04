// Speech to Text - Microphone Settings Page
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Microphone selection and audio device configuration page.

use gtk4::prelude::*;
use crate::i18n::gettext;
use adw::prelude::*;
use gtk4::glib;
use gtk4 as gtk;
use libadwaita as adw;
use adw::subclass::prelude::*;
use std::cell::RefCell;

use crate::audio::capture::{list_input_devices, AudioDevice};
use crate::config::AppConfig;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct MicrophonePage {
        pub device_group: RefCell<Option<adw::PreferencesGroup>>,
        pub device_rows: RefCell<Vec<adw::ActionRow>>,
        /// Per-device selection checkmarks, keyed by device name, so the active
        /// device can be re-highlighted when the selection changes.
        pub device_checks: RefCell<Vec<(gtk::Image, String)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MicrophonePage {
        const NAME: &'static str = "SttMicrophonePage";
        type Type = super::MicrophonePage;
        type ParentType = adw::PreferencesPage;
    }

    impl ObjectImpl for MicrophonePage {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_ui();
        }
    }

    impl WidgetImpl for MicrophonePage {}
    impl adw::subclass::prelude::PreferencesPageImpl for MicrophonePage {}
}

glib::wrapper! {
    pub struct MicrophonePage(ObjectSubclass<imp::MicrophonePage>)
        @extends gtk::Widget, adw::PreferencesPage;
}

impl MicrophonePage {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("title", "Microphone")
            .property("icon-name", "audio-input-microphone-symbolic")
            .build()
    }

    fn setup_ui(&self) {
        let imp = self.imp();

        // Device selection group
        let device_group = adw::PreferencesGroup::new();
        device_group.set_title(gettext("Input Device").as_str());
        device_group.set_description(Some(gettext("Select the microphone to use for recording").as_str()));

        // Refresh button in header
        let refresh_btn = gtk::Button::from_icon_name("view-refresh-symbolic");
        refresh_btn.set_tooltip_text(Some(gettext("Refresh device list").as_str()));
        refresh_btn.add_css_class("flat");
        device_group.set_header_suffix(Some(&refresh_btn));

        self.add(&device_group);
        *imp.device_group.borrow_mut() = Some(device_group);

        // Audio settings group
        let audio_group = adw::PreferencesGroup::new();
        audio_group.set_title(gettext("Audio Settings").as_str());

        let sample_rate_row = adw::ActionRow::builder()
            .title(gettext("Sample Rate").as_str())
            .subtitle(gettext("Audio will be resampled to 16kHz for Whisper").as_str())
            .build();
        let rate_label = gtk::Label::new(Some(gettext("16,000 Hz").as_str()));
        rate_label.add_css_class("dim-label");
        sample_rate_row.add_suffix(&rate_label);
        audio_group.add(&sample_rate_row);

        let channels_row = adw::ActionRow::builder()
            .title(gettext("Channels").as_str())
            .subtitle(gettext("Stereo input is automatically converted to mono").as_str())
            .build();
        let ch_label = gtk::Label::new(Some(gettext("Mono").as_str()));
        ch_label.add_css_class("dim-label");
        channels_row.add_suffix(&ch_label);
        audio_group.add(&channels_row);

        self.add(&audio_group);

        // Test group
        let test_group = adw::PreferencesGroup::new();
        test_group.set_title(gettext("Microphone Test").as_str());

        let test_row = adw::ActionRow::builder()
            .title(gettext("Test Microphone").as_str())
            .subtitle(gettext("Record a short sample to verify your microphone works").as_str())
            .build();
        let test_btn = gtk::Button::with_label("Test");
        test_btn.set_valign(gtk::Align::Center);
        test_btn.add_css_class("pill");
        test_row.add_suffix(&test_btn);
        test_row.set_activatable_widget(Some(&test_btn));
        test_group.add(&test_row);

        self.add(&test_group);

        // Connect mic test
        let test_row_ref = test_row.clone();
        test_btn.connect_clicked(move |btn| {
            btn.set_sensitive(false);
            btn.set_label(gettext("Listening…").as_str());
            let btn_weak = btn.downgrade();
            let row_weak = test_row_ref.downgrade();

            // Record 2 seconds of audio to test, using the selected device.
            let selected_device = AppConfig::load().selected_microphone;
            let (sender, receiver) = async_channel::bounded::<Result<String, String>>(1);
            std::thread::spawn(move || {
                let mut cap = crate::audio::AudioCapture::new();
                let result = cap.start_recording(selected_device.as_deref());
                if let Err(e) = result {
                    let _ = sender.send_blocking(Err(format!("Failed: {}", e)));
                    return;
                }
                std::thread::sleep(std::time::Duration::from_secs(2));
                match cap.stop_recording() {
                    Ok(samples) => {
                        if samples.is_empty() {
                            let _ = sender.send_blocking(Ok("No audio detected".to_string()));
                        } else {
                            let peak = samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
                            let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
                            let _ = sender.send_blocking(Ok(format!(
                                "OK — Peak: {:.1}%, RMS: {:.1}%",
                                peak * 100.0,
                                rms * 100.0
                            )));
                        }
                    }
                    Err(e) => {
                        let _ = sender.send_blocking(Err(e.to_string()));
                    }
                }
            });

            glib::spawn_future_local(async move {
                if let Ok(result) = receiver.recv().await {
                    if let Some(btn) = btn_weak.upgrade() {
                        btn.set_sensitive(true);
                        btn.set_label(gettext("Test").as_str());
                    }
                    if let Some(row) = row_weak.upgrade() {
                        match result {
                            Ok(msg) => row.set_subtitle(&msg),
                            Err(msg) => row.set_subtitle(&msg),
                        }
                    }
                }
            });
        });

        // Connect refresh
        let page_weak = self.downgrade();
        refresh_btn.connect_clicked(move |_| {
            if let Some(page) = page_weak.upgrade() {
                page.refresh_devices();
            }
        });

        // Auto-enumerate devices on creation
        self.refresh_devices();
    }

    /// Refresh the device list.
    pub fn refresh_devices(&self) {
        let page = self.clone();
        let (sender, receiver) = async_channel::bounded::<Vec<AudioDevice>>(1);

        std::thread::spawn(move || {
            let devices = list_input_devices().unwrap_or_default();
            let _ = sender.send_blocking(devices);
        });

        glib::spawn_future_local(async move {
            if let Ok(devices) = receiver.recv().await {
                page.populate_device_list(&devices);
            }
        });
    }

    /// Populate the device list from enumerated audio devices.
    fn populate_device_list(&self, devices: &[AudioDevice]) {
        let imp = self.imp();
        if let Some(group) = imp.device_group.borrow().as_ref() {
            // Remove old rows
            for row in imp.device_rows.borrow().iter() {
                group.remove(row);
            }

            // The active device: the user's saved choice, or the default device.
            let saved = AppConfig::load().selected_microphone;
            let effective = saved.clone().or_else(|| {
                devices.iter().find(|d| d.is_default).map(|d| d.name.clone())
            });

            let mut new_rows = Vec::new();
            let mut new_checks = Vec::new();
            for device in devices {
                let subtitle = if device.is_default { "Default device" } else { "" };
                let row = adw::ActionRow::builder()
                    .title(device.name.as_str())
                    .subtitle(subtitle)
                    .activatable(true)
                    .build();

                // Every row carries a checkmark; only the active device shows it.
                let check = gtk::Image::from_icon_name("object-select-symbolic");
                check.add_css_class("accent");
                check.set_visible(effective.as_deref() == Some(device.name.as_str()));
                row.add_suffix(&check);

                // Clicking a row selects that device.
                let page_weak = self.downgrade();
                let device_name = device.name.clone();
                row.connect_activated(move |_| {
                    if let Some(page) = page_weak.upgrade() {
                        page.set_selected_device(&device_name);
                    }
                });

                group.add(&row);
                new_rows.push(row);
                new_checks.push((check, device.name.clone()));
            }

            if devices.is_empty() {
                let row = adw::ActionRow::builder()
                    .title(gettext("No audio input devices found").as_str())
                    .subtitle(gettext("Check that a microphone is connected").as_str())
                    .build();
                let icon = gtk::Image::from_icon_name("dialog-warning-symbolic");
                row.add_prefix(&icon);
                group.add(&row);
                new_rows.push(row);
            }

            *imp.device_rows.borrow_mut() = new_rows;
            *imp.device_checks.borrow_mut() = new_checks;
        }
    }

    /// Persist the chosen input device and move the selection checkmark to it.
    fn set_selected_device(&self, device_name: &str) {
        let mut config = AppConfig::load();
        config.selected_microphone = Some(device_name.to_string());
        config.save();

        for (check, name) in self.imp().device_checks.borrow().iter() {
            check.set_visible(name == device_name);
        }
    }
}
