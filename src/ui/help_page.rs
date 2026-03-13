// Speech to Text - Help Page
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Help Page - Application documentation and guidance.

use gtk4 as gtk;
use gtk4::prelude::*;
use gtk4::glib;
use gtk4::subclass::prelude::*;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct HelpPage {}

    #[glib::object_subclass]
    impl ObjectSubclass for HelpPage {
        const NAME: &'static str = "SttHelpPage";
        type Type = super::HelpPage;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for HelpPage {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_ui();
        }
    }

    impl WidgetImpl for HelpPage {}
    impl BoxImpl for HelpPage {}
}

glib::wrapper! {
    pub struct HelpPage(ObjectSubclass<imp::HelpPage>)
        @extends gtk::Widget, gtk::Box;
}

impl HelpPage {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("orientation", gtk::Orientation::Vertical)
            .property("spacing", 0)
            .build()
    }

    fn setup_ui(&self) {
        // Page header
        let header_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        header_box.set_margin_start(24);
        header_box.set_margin_end(24);
        header_box.set_margin_top(24);
        header_box.set_margin_bottom(12);

        let title = gtk::Label::new(Some("Help"));
        title.add_css_class("title-1");
        title.set_halign(gtk::Align::Start);
        header_box.append(&title);

        let subtitle = gtk::Label::new(Some("Learn how to use Speech to Text"));
        subtitle.add_css_class("dim-label");
        subtitle.set_halign(gtk::Align::Start);
        header_box.append(&subtitle);

        self.append(&header_box);

        // Scrollable content
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_hexpand(true);
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);

        let content_box = gtk::Box::new(gtk::Orientation::Vertical, 24);
        content_box.set_margin_start(24);
        content_box.set_margin_end(24);
        content_box.set_margin_top(12);
        content_box.set_margin_bottom(24);

        content_box.append(&self.create_section(
            "About Speech to Text",
            "Speech to Text is an offline speech transcription application for Linux. \
             It uses OpenAI's Whisper model to convert spoken audio into text, \
             running entirely on your machine — no internet connection required after \
             downloading the model. Your audio data never leaves your device."
        ));

        content_box.append(&self.create_section(
            "Transcription",
            "The Transcription page is your main workspace. Press Record to start \
             capturing audio from your microphone, then press Stop to transcribe. \
             The transcribed text appears in message bubbles that you can copy individually. \
             Use Pause to temporarily halt recording without losing progress. \
             The confidence bar at the bottom shows how certain the model is about \
             the transcription accuracy."
        ));

        content_box.append(&self.create_section(
            "History",
            "The History page keeps a record of all your past transcriptions. \
             Each entry shows the text, language, duration, and model used. \
             Use the search bar to find specific transcriptions. \
             You can copy or delete individual entries."
        ));

        content_box.append(&self.create_section(
            "Translation",
            "Whisper can translate speech from any supported language into English. \
             Enable translation using the Translate toggle on the transcription page \
             or in Language settings. Note that Whisper only supports translation \
             to English — other target languages are not available."
        ));

        content_box.append(&self.create_section(
            "Language Settings",
            "By default, Whisper auto-detects the spoken language. \
             You can override this by disabling auto-detect and selecting a specific \
             language from the dropdown. This can improve accuracy when you know \
             which language will be spoken. Whisper supports 99 languages."
        ));

        content_box.append(&self.create_section(
            "Model Selection",
            "Whisper comes in several sizes, from Tiny (~75 MB) to Large v3 (~3 GB). \
             Larger models provide better accuracy but require more memory and take \
             longer to transcribe. Start with Tiny or Base for testing, then move to \
             Small or Medium for production use. Download models from the Model \
             settings page."
        ));

        content_box.append(&self.create_section(
            "Performance",
            "The Performance page lets you configure GPU acceleration and CPU thread \
             count. Enable GPU if you have a compatible NVIDIA (CUDA) or AMD graphics \
             card for faster transcription. Adjust worker threads based on your CPU — \
             more threads mean faster processing but higher CPU usage. \
             The beam size setting controls search accuracy vs speed."
        ));

        content_box.append(&self.create_section(
            "Tips",
            "• Use a quiet environment for best transcription accuracy.\n\
             • Speak clearly and at a moderate pace.\n\
             • For long recordings, the Medium or Large model gives best results.\n\
             • Enable GPU acceleration if available for significantly faster processing.\n\
             • Use the Tiny model for quick tests and the Large model for important work.\n\
             • Check the confidence bar — low confidence may indicate background noise."
        ));

        scroll.set_child(Some(&content_box));
        self.append(&scroll);
    }

    fn create_section(&self, title: &str, description: &str) -> gtk::Box {
        let section = gtk::Box::new(gtk::Orientation::Vertical, 8);

        let title_label = gtk::Label::new(Some(title));
        title_label.add_css_class("title-3");
        title_label.set_halign(gtk::Align::Start);
        section.append(&title_label);

        let desc_label = gtk::Label::new(Some(description));
        desc_label.set_wrap(true);
        desc_label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        desc_label.set_xalign(0.0);
        desc_label.set_halign(gtk::Align::Start);
        desc_label.add_css_class("body");
        section.append(&desc_label);

        section
    }
}
