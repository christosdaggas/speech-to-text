// Speech to Text - Microphone Settings Page
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Microphone selection and audio device configuration page.

use gtk4::prelude::*;
use adw::prelude::*;
use gtk4::glib;
use gtk4 as gtk;
use libadwaita as adw;
use adw::subclass::prelude::*;
use std::cell::RefCell;

use crate::audio::capture::{list_input_devices, AudioDevice};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct MicrophonePage {
        pub device_group: RefCell<Option<adw::PreferencesGroup>>,
        pub device_rows: RefCell<Vec<adw::ActionRow>>,
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
        device_group.set_title("Input Device");
        device_group.set_description(Some("Select the microphone to use for recording"));

        // Refresh button in header
        let refresh_btn = gtk::Button::from_icon_name("view-refresh-symbolic");
        refresh_btn.set_tooltip_text(Some("Refresh device list"));
        refresh_btn.add_css_class("flat");
        device_group.set_header_suffix(Some(&refresh_btn));

        self.add(&device_group);
        *imp.device_group.borrow_mut() = Some(device_group);

        // Audio settings group
        let audio_group = adw::PreferencesGroup::new();
        audio_group.set_title("Audio Settings");

        let sample_rate_row = adw::ActionRow::builder()
            .title("Sample Rate")
            .subtitle("Audio will be resampled to 16kHz for Whisper")
            .build();
        let rate_label = gtk::Label::new(Some("16,000 Hz"));
        rate_label.add_css_class("dim-label");
        sample_rate_row.add_suffix(&rate_label);
        audio_group.add(&sample_rate_row);

        let channels_row = adw::ActionRow::builder()
            .title("Channels")
            .subtitle("Stereo input is automatically converted to mono")
            .build();
        let ch_label = gtk::Label::new(Some("Mono"));
        ch_label.add_css_class("dim-label");
        channels_row.add_suffix(&ch_label);
        audio_group.add(&channels_row);

        self.add(&audio_group);

        // Test group
        let test_group = adw::PreferencesGroup::new();
        test_group.set_title("Microphone Test");

        let test_row = adw::ActionRow::builder()
            .title("Test Microphone")
            .subtitle("Record a short sample to verify your microphone works")
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
            btn.set_label("Listening…");
            let btn_weak = btn.downgrade();
            let row_weak = test_row_ref.downgrade();

            // Record 2 seconds of audio to test
            let (sender, receiver) = async_channel::bounded::<Result<String, String>>(1);
            std::thread::spawn(move || {
                let mut cap = crate::audio::AudioCapture::new();
                let result = cap.start_recording(None);
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
                        btn.set_label("Test");
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

            let mut new_rows = Vec::new();
            for device in devices {
                let row = adw::ActionRow::builder()
                    .title(device.name.as_str())
                    .subtitle(if device.is_default { "Default device" } else { "" })
                    .activatable(true)
                    .build();

                if device.is_default {
                    let check = gtk::Image::from_icon_name("object-select-symbolic");
                    check.add_css_class("accent");
                    row.add_suffix(&check);
                }

                group.add(&row);
                new_rows.push(row);
            }

            if devices.is_empty() {
                let row = adw::ActionRow::builder()
                    .title("No audio input devices found")
                    .subtitle("Check that a microphone is connected")
                    .build();
                let icon = gtk::Image::from_icon_name("dialog-warning-symbolic");
                row.add_prefix(&icon);
                group.add(&row);
                new_rows.push(row);
            }

            *imp.device_rows.borrow_mut() = new_rows;
        }
    }
}
