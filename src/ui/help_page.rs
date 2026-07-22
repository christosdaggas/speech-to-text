// Speech to Text - Help Page
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Help page — a real, illustrated guide: what each engine/model is, why pick
//! one over another, and how every feature works. Diagrams are drawn natively
//! (Cairo + GTK widgets) so they adapt to the GNOME light/dark theme.

use gtk4 as gtk;
use crate::i18n::gettext;
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
        self.add_css_class("help-page");

        // Page header
        let header_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        header_box.set_margin_start(24);
        header_box.set_margin_end(24);
        header_box.set_margin_top(24);
        header_box.set_margin_bottom(12);

        let title = gtk::Label::new(Some(gettext("Help").as_str()));
        title.add_css_class("title-1");
        title.set_halign(gtk::Align::Start);
        header_box.append(&title);

        let subtitle = gtk::Label::new(Some(
            gettext("Everything you need to get great transcriptions — and choose the right settings.").as_str(),
        ));
        subtitle.add_css_class("dim-label");
        subtitle.set_halign(gtk::Align::Start);
        subtitle.set_wrap(true);
        subtitle.set_xalign(0.0);
        header_box.append(&subtitle);

        self.append(&header_box);

        // Scrollable content
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_hexpand(true);
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 22);
        content.set_margin_start(24);
        content.set_margin_end(24);
        content.set_margin_top(12);
        content.set_margin_bottom(32);
        // Keep the column readable on very wide windows.
        content.set_halign(gtk::Align::Fill);

        // ── Quick start ───────────────────────────────────────────────
        content.append(&self.heading(&gettext("Quick start")));
        content.append(&self.steps_card(&[
            gettext("Open Settings → Model and download a Whisper model (start with Base)."),
            gettext("On the Transcription page press Record, speak, then press Stop."),
            gettext("Your text appears and is copied to the clipboard automatically."),
            gettext("(Optional) Turn on Improve with AI to clean up grammar & punctuation."),
        ]));

        // ── Recording ─────────────────────────────────────────────────
        content.append(&self.heading(&gettext("Recording")));
        content.append(&self.body(&gettext(
            "Press Record, speak, and press Stop to transcribe. Use Pause to hold without losing \
             the take, or Cancel to discard it. You can also drag an audio file (WAV, MP3, FLAC, \
             OGG, M4A) onto the transcript to transcribe it. The confidence bar at the bottom shows \
             how sure the model is.",
        )));
        content.append(&self.flow_diagram(&[
            gettext("⏺ Record"),
            gettext("Speak"),
            gettext("⏹ Stop"),
            gettext("Transcribe"),
            gettext("📋 Clipboard"),
        ]));

        // ── Engines & models ──────────────────────────────────────────
        content.append(&self.heading(&gettext("Engines & models — which to choose")));
        content.append(&self.body(&gettext(
            "Three local engines, all offline after download. Whisper is the default and most flexible.",
        )));
        let cards = gtk::FlowBox::new();
        cards.set_selection_mode(gtk::SelectionMode::None);
        cards.set_max_children_per_line(2);
        cards.set_min_children_per_line(1);
        cards.set_column_spacing(12);
        cards.set_row_spacing(12);
        cards.set_homogeneous(true);
        for (t, d) in [
            (gettext("Whisper"), gettext("OpenAI's model via whisper.cpp. 99 languages, auto-detect, translate-to-English, timestamps, GPU support. Best all-round choice.")),
            (gettext("Qwen3-ASR"), gettext("Alibaba's model. 30+ languages including Greek, auto-detect. Two sizes (0.6B / 1.7B). Good accuracy, CPU-only, no timestamps.")),
            (gettext("Cohere"), gettext("Local Cohere runtime. Solid quality, but the language must be set (no auto-detect) and there are no per-segment timestamps.")),
            (gettext("Quantized (Q5)"), gettext("Every Whisper size has a Q5 variant — about 60% smaller with near-identical accuracy. Pick these if disk or RAM is tight.")),
        ] {
            cards.insert(&self.feature_card(&t, &d), -1);
        }
        content.append(&cards);

        content.append(&self.subheading(&gettext("Whisper model sizes")));
        content.append(&self.legend_row());
        content.append(&self.model_chart());
        content.append(&self.body(&gettext(
            "Bigger = more accurate but slower and heavier. Large v3 Turbo is the sweet spot: near \
             Large-v3 accuracy, several times faster.",
        )));
        content.append(&self.table(
            &gettext("If you want…"),
            &gettext("Choose"),
            &[
                (gettext("Quick tests / very low resources"), gettext("Tiny or Base (or their Q5)")),
                (gettext("A good everyday balance"), gettext("Small or Large v3 Turbo")),
                (gettext("Best possible accuracy, have the RAM/GPU"), gettext("Large v3")),
                (gettext("Greek / non-English everyday dictation"), gettext("Whisper Small/Turbo or Qwen3-ASR")),
            ],
        ));

        // ── Languages ─────────────────────────────────────────────────
        content.append(&self.heading(&gettext("Languages & translation")));
        content.append(&self.body(&gettext(
            "Whisper auto-detects the language by default. If you always speak one language, set it \
             explicitly in Language settings — it's faster and avoids mis-detection on short clips. \
             Translate converts speech to English (Whisper only; English is the only target).",
        )));

        // ── Improve with AI ───────────────────────────────────────────
        content.append(&self.heading(&gettext("Improve with AI")));
        content.append(&self.body(&gettext(
            "Optionally send the transcript to an LLM to clean it up, summarize, translate, or reshape \
             it. Works with any OpenAI-compatible server (LM Studio, Ollama, vLLM, OpenAI) — set it up \
             in LLM settings.",
        )));
        content.append(&self.bullets(&[
            gettext("Chips under the transcript run a preset in one tap — Clean up, Key points, Formal, Short, Long, Translate…"),
            gettext("The Raw ⇄ Improved switch keeps the original — AI versions are added, never destroyed."),
            gettext("Auto-improve runs your active preset on every dictation automatically."),
        ]));
        content.append(&self.callout(
            "warning",
            &gettext("Privacy"),
            &gettext("With the LLM on, transcript text leaves your device for the endpoint you configured (a local server stays on your machine; a cloud one does not). You're asked to consent the first time."),
        ));

        // ── Dictionary ────────────────────────────────────────────────
        content.append(&self.heading(&gettext("Personal dictionary")));
        content.append(&self.body(&gettext(
            "Teach the app your names, jargon and acronyms (Dictionary settings). Terms bias Whisper \
             toward those words; replacements fix consistent spellings (e.g. heard \"cube\" → \"Kube\"). \
             Everything stays on your device.",
        )));

        // ── Voice edit ────────────────────────────────────────────────
        content.append(&self.heading(&gettext("Voice edit")));
        content.append(&self.body(&gettext(
            "On a result, press the voice-edit button and just say what to change — \"make it shorter\", \
             \"turn into bullets\", \"reply politely\". The app transcribes your instruction and applies \
             it to the text.",
        )));

        // ── Live text ─────────────────────────────────────────────────
        content.append(&self.heading(&gettext("Live text & progress")));
        content.append(&self.body(&gettext(
            "Turn on Live transcription (Dictation settings, Whisper only) to watch the text appear \
             while you speak, with a real progress bar instead of a spinner. The final text is always \
             the full-accuracy result produced when you press Stop.",
        )));

        // ── Mini panel ────────────────────────────────────────────────
        content.append(&self.heading(&gettext("Mini panel & global dictation")));
        content.append(&self.body(&gettext(
            "A global shortcut opens a compact floating panel so you can dictate into any app: it \
             records, transcribes, optionally polishes, and pastes the finished text where your cursor \
             is. Enable it and set the shortcut in Dictation settings.",
        )));

        // ── Performance ───────────────────────────────────────────────
        content.append(&self.heading(&gettext("Performance")));
        content.append(&self.bullets(&[
            gettext("GPU acceleration (NVIDIA/AMD) speeds up Whisper a lot — enable it if you have a supported card."),
            gettext("Threads: more CPU threads = faster, higher load. The default adapts to your CPU."),
            gettext("Beam size: higher = a little more accurate but slower; 1 = fastest (greedy)."),
        ]));
        content.append(&self.callout(
            "success",
            &gettext("Tip"),
            &gettext("The GPU card in the sidebar shows your card and live VRAM use — handy to check there's room before a large model loads."),
        ));

        // ── Privacy ───────────────────────────────────────────────────
        content.append(&self.heading(&gettext("Privacy")));
        content.append(&self.body(&gettext(
            "Transcription is 100% local. Audio never leaves your device. The only network use is \
             opt-in: the LLM features (your configured endpoint) and an optional startup update check \
             (GitHub). API keys are stored in the system keyring, never in plain text.",
        )));

        // ── Troubleshooting ───────────────────────────────────────────
        content.append(&self.heading(&gettext("Troubleshooting")));
        content.append(&self.table(
            &gettext("Symptom"),
            &gettext("Try"),
            &[
                (gettext("\"No clear speech detected\""), gettext("Speak closer to the mic; pick the right input in Microphone settings; record in a quieter environment.")),
                (gettext("Nothing happens on Stop"), gettext("Check a model is downloaded (Model settings) and the engine is set up. Watch the notification at the top of the window for the exact error.")),
                (gettext("Wrong language / gibberish"), gettext("Set the language explicitly, or switch engine. For Cohere the language must be set (no auto-detect).")),
                (gettext("Slow transcription"), gettext("Use a smaller / Turbo / Q5 model, enable GPU, or raise the thread count.")),
            ],
        ));

        scroll.set_child(Some(&content));
        self.append(&scroll);
    }

    // ── Building blocks ───────────────────────────────────────────────

    fn heading(&self, text: &str) -> gtk::Label {
        let l = gtk::Label::new(Some(text));
        l.add_css_class("title-3");
        l.set_halign(gtk::Align::Start);
        l.set_margin_top(8);
        l
    }

    fn subheading(&self, text: &str) -> gtk::Label {
        let l = gtk::Label::new(Some(text));
        l.add_css_class("heading");
        l.set_halign(gtk::Align::Start);
        l
    }

    fn body(&self, text: &str) -> gtk::Label {
        let l = gtk::Label::new(Some(text));
        l.set_wrap(true);
        l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        l.set_xalign(0.0);
        l.set_halign(gtk::Align::Start);
        l.add_css_class("body");
        l
    }

    /// A numbered-steps card.
    fn steps_card(&self, steps: &[String]) -> gtk::Frame {
        let frame = gtk::Frame::new(None);
        frame.add_css_class("card");
        let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
        b.set_margin_start(16);
        b.set_margin_end(16);
        b.set_margin_top(12);
        b.set_margin_bottom(12);
        for (i, s) in steps.iter().enumerate() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            let num = gtk::Label::new(Some(&format!("{}", i + 1)));
            num.add_css_class("caption-heading");
            num.add_css_class("accent");
            num.set_valign(gtk::Align::Start);
            num.set_width_chars(2);
            let txt = self.body(s);
            txt.set_hexpand(true);
            row.append(&num);
            row.append(&txt);
            b.append(&row);
        }
        frame.set_child(Some(&b));
        frame
    }

    /// A small titled feature card (for the engine comparison).
    fn feature_card(&self, title: &str, desc: &str) -> gtk::Frame {
        let frame = gtk::Frame::new(None);
        frame.add_css_class("card");
        frame.set_hexpand(true);
        let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
        b.set_margin_start(14);
        b.set_margin_end(14);
        b.set_margin_top(10);
        b.set_margin_bottom(10);
        let t = gtk::Label::new(Some(title));
        t.add_css_class("heading");
        t.set_halign(gtk::Align::Start);
        b.append(&t);
        let d = self.body(desc);
        d.add_css_class("dim-label");
        b.append(&d);
        frame.set_child(Some(&b));
        frame
    }

    /// A bulleted list.
    fn bullets(&self, items: &[String]) -> gtk::Box {
        let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
        for it in items {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let dot = gtk::Label::new(Some("•"));
            dot.add_css_class("accent");
            dot.set_valign(gtk::Align::Start);
            let txt = self.body(it);
            txt.set_hexpand(true);
            row.append(&dot);
            row.append(&txt);
            b.append(&row);
        }
        b
    }

    /// A colored callout box (kind = "success" | "warning").
    fn callout(&self, kind: &str, title: &str, text: &str) -> gtk::Frame {
        let frame = gtk::Frame::new(None);
        frame.add_css_class("card");
        let b = gtk::Box::new(gtk::Orientation::Vertical, 2);
        b.set_margin_start(14);
        b.set_margin_end(14);
        b.set_margin_top(10);
        b.set_margin_bottom(10);
        let t = gtk::Label::new(Some(title));
        t.add_css_class("caption-heading");
        t.add_css_class(kind);
        t.set_halign(gtk::Align::Start);
        b.append(&t);
        b.append(&self.body(text));
        frame.set_child(Some(&b));
        frame
    }

    /// A two-column reference table (header + rows), drawn with a Grid + separators.
    fn table(&self, h1: &str, h2: &str, rows: &[(String, String)]) -> gtk::Frame {
        let frame = gtk::Frame::new(None);
        frame.add_css_class("card");
        let grid = gtk::Grid::new();
        grid.set_column_spacing(16);
        grid.set_row_spacing(8);
        grid.set_margin_start(14);
        grid.set_margin_end(14);
        grid.set_margin_top(12);
        grid.set_margin_bottom(12);
        grid.set_column_homogeneous(false);

        let th1 = gtk::Label::new(Some(h1));
        th1.add_css_class("caption-heading");
        th1.add_css_class("dim-label");
        th1.set_xalign(0.0);
        let th2 = gtk::Label::new(Some(h2));
        th2.add_css_class("caption-heading");
        th2.add_css_class("dim-label");
        th2.set_xalign(0.0);
        grid.attach(&th1, 0, 0, 1, 1);
        grid.attach(&th2, 1, 0, 1, 1);

        let mut r = 1;
        for (a, b) in rows {
            let sep = gtk::Separator::new(gtk::Orientation::Horizontal);
            grid.attach(&sep, 0, r, 2, 1);
            r += 1;
            let la = self.body(a);
            la.set_valign(gtk::Align::Start);
            la.set_width_chars(26);
            la.set_max_width_chars(30);
            let lb = self.body(b);
            lb.set_hexpand(true);
            grid.attach(&la, 0, r, 1, 1);
            grid.attach(&lb, 1, r, 1, 1);
            r += 1;
        }
        frame.set_child(Some(&grid));
        frame
    }

    /// A horizontal flow diagram of pills joined by arrows.
    fn flow_diagram(&self, steps: &[String]) -> gtk::ScrolledWindow {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.set_halign(gtk::Align::Start);
        row.set_margin_top(4);
        row.set_margin_bottom(4);
        for (i, s) in steps.iter().enumerate() {
            if i > 0 {
                let arrow = gtk::Label::new(Some("→"));
                arrow.add_css_class("dim-label");
                row.append(&arrow);
            }
            let pill = gtk::Frame::new(None);
            pill.add_css_class("card");
            let l = gtk::Label::new(Some(s));
            l.add_css_class("caption-heading");
            l.set_margin_start(12);
            l.set_margin_end(12);
            l.set_margin_top(6);
            l.set_margin_bottom(6);
            pill.set_child(Some(&l));
            row.append(&pill);
        }
        let sc = gtk::ScrolledWindow::new();
        sc.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
        sc.set_child(Some(&row));
        sc
    }

    /// Legend for the model chart.
    fn legend_row(&self) -> gtk::Box {
        let b = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        let mk = |text: &str, r: f64, g: f64, bl: f64| {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let swatch = gtk::DrawingArea::new();
            swatch.set_content_width(14);
            swatch.set_content_height(10);
            swatch.set_valign(gtk::Align::Center);
            swatch.set_draw_func(move |_a, cr, w, h| {
                cr.set_source_rgb(r, g, bl);
                cr.rectangle(0.0, 0.0, w as f64, h as f64);
                let _ = cr.fill();
            });
            let l = gtk::Label::new(Some(text));
            l.add_css_class("caption");
            l.add_css_class("dim-label");
            row.append(&swatch);
            row.append(&l);
            row
        };
        b.append(&mk(&gettext("Accuracy"), 0.21, 0.52, 0.89));
        b.append(&mk(&gettext("Speed"), 0.18, 0.76, 0.49));
        b
    }

    /// The Whisper model comparison chart (accuracy + speed bars), drawn with
    /// Cairo so text uses the theme foreground colour while the series keep fixed
    /// blue/green that read on light and dark.
    fn model_chart(&self) -> gtk::DrawingArea {
        let area = gtk::DrawingArea::new();
        area.set_content_height(196);
        area.set_hexpand(true);
        area.set_draw_func(move |a, cr, w, h| {
            let fg = a.color();
            // (name, accuracy 0-1, speed 0-1, size)
            let rows: [(&str, f64, f64, &str); 6] = [
                ("Tiny", 0.18, 0.96, "~75 MB"),
                ("Base", 0.34, 0.80, "~142 MB"),
                ("Small", 0.55, 0.58, "~466 MB"),
                ("Medium", 0.76, 0.38, "~1.5 GB"),
                ("Large v3", 0.96, 0.22, "~3 GB"),
                ("Large v3 Turbo", 0.90, 0.66, "~1.6 GB"),
            ];
            let n = rows.len() as f64;
            let left = 116.0_f64;
            let right = w as f64 - 86.0;
            let track = (right - left).max(40.0);
            let row_h = h as f64 / n;
            cr.select_font_face("Sans", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
            cr.set_font_size(12.5);

            for (i, (name, acc, spd, size)) in rows.iter().enumerate() {
                let cy = i as f64 * row_h + row_h / 2.0;
                // Name (theme fg)
                cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.95);
                cr.move_to(8.0, cy + 4.0);
                let _ = cr.show_text(name);
                // Track
                cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.10);
                cr.rectangle(left, cy - 9.0, track, 12.0);
                let _ = cr.fill();
                // Accuracy bar (blue)
                cr.set_source_rgb(0.21, 0.52, 0.89);
                cr.rectangle(left, cy - 9.0, track * acc, 12.0);
                let _ = cr.fill();
                // Speed bar (green, thinner, below)
                cr.set_source_rgb(0.18, 0.76, 0.49);
                cr.rectangle(left, cy + 5.0, track * spd, 6.0);
                let _ = cr.fill();
                // Size (dim fg)
                cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.6);
                cr.move_to(right + 10.0, cy + 4.0);
                let _ = cr.show_text(size);
            }
        });
        area
    }
}
