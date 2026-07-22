// Speech to Text - Model Row Widget
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Custom list row for model selection with download status.

use adw::subclass::prelude::*;
use gtk4 as gtk;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::{Cell, RefCell};

/// Model download state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModelState {
    #[default]
    NotDownloaded,
    Downloading,
    Downloaded,
    Selected,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ModelRow {
        pub name_label: RefCell<Option<gtk::Label>>,
        pub size_label: RefCell<Option<gtk::Label>>,
        pub desc_label: RefCell<Option<gtk::Label>>,
        pub action_btn: RefCell<Option<gtk::Button>>,
        pub progress_bar: RefCell<Option<gtk::ProgressBar>>,
        pub state: Cell<ModelState>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ModelRow {
        const NAME: &'static str = "SttModelRow";
        type Type = super::ModelRow;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for ModelRow {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_ui();
        }
    }

    impl WidgetImpl for ModelRow {}
    impl BoxImpl for ModelRow {}
}

glib::wrapper! {
    pub struct ModelRow(ObjectSubclass<imp::ModelRow>)
        @extends gtk::Widget, gtk::Box;
}

impl ModelRow {
    pub fn new(name: &str, size: &str, description: &str) -> Self {
        let row: Self = glib::Object::builder()
            .property("orientation", gtk::Orientation::Horizontal)
            .property("spacing", 12)
            .build();
        row.set_data(name, size, description);
        row
    }

    fn setup_ui(&self) {
        let imp = self.imp();

        self.set_margin_start(8);
        self.set_margin_end(8);
        self.set_margin_top(6);
        self.set_margin_bottom(6);

        // Info column
        let info_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
        info_box.set_hexpand(true);

        let name_label = gtk::Label::new(None);
        name_label.add_css_class("heading");
        name_label.set_xalign(0.0);
        info_box.append(&name_label);

        let desc_label = gtk::Label::new(None);
        desc_label.add_css_class("caption");
        desc_label.add_css_class("dim-label");
        desc_label.set_xalign(0.0);
        desc_label.set_wrap(true);
        info_box.append(&desc_label);

        let progress_bar = gtk::ProgressBar::new();
        progress_bar.set_visible(false);
        progress_bar.set_show_text(true);
        info_box.append(&progress_bar);

        self.append(&info_box);

        // Size label
        let size_label = gtk::Label::new(None);
        size_label.add_css_class("caption");
        size_label.add_css_class("dim-label");
        size_label.set_valign(gtk::Align::Center);
        self.append(&size_label);

        // Action button
        let action_btn = gtk::Button::with_label("Download");
        action_btn.add_css_class("pill");
        action_btn.set_valign(gtk::Align::Center);
        self.append(&action_btn);

        *imp.name_label.borrow_mut() = Some(name_label);
        *imp.size_label.borrow_mut() = Some(size_label);
        *imp.desc_label.borrow_mut() = Some(desc_label);
        *imp.action_btn.borrow_mut() = Some(action_btn);
        *imp.progress_bar.borrow_mut() = Some(progress_bar);
    }

    fn set_data(&self, name: &str, size: &str, description: &str) {
        let imp = self.imp();
        if let Some(l) = imp.name_label.borrow().as_ref() {
            l.set_text(name);
        }
        if let Some(l) = imp.size_label.borrow().as_ref() {
            l.set_text(size);
        }
        if let Some(l) = imp.desc_label.borrow().as_ref() {
            l.set_text(description);
        }
    }

    /// Update the model state.
    pub fn set_state(&self, state: ModelState) {
        let imp = self.imp();
        imp.state.set(state);

        if let Some(btn) = imp.action_btn.borrow().as_ref() {
            btn.remove_css_class("suggested-action");
            btn.remove_css_class("destructive-action");

            match state {
                ModelState::NotDownloaded => {
                    btn.set_label("Download");
                    btn.set_sensitive(true);
                }
                ModelState::Downloading => {
                    btn.set_label("Downloading…");
                    btn.set_sensitive(false);
                }
                ModelState::Downloaded => {
                    btn.set_label("Select");
                    btn.set_sensitive(true);
                }
                ModelState::Selected => {
                    btn.set_label("Selected");
                    btn.set_sensitive(false);
                    btn.add_css_class("suggested-action");
                }
            }
        }

        if let Some(bar) = imp.progress_bar.borrow().as_ref() {
            bar.set_visible(state == ModelState::Downloading);
        }
    }

    /// Set download progress (0.0 - 1.0).
    pub fn set_progress(&self, fraction: f64) {
        if let Some(bar) = self.imp().progress_bar.borrow().as_ref() {
            bar.set_fraction(fraction);
            bar.set_text(Some(&format!("{:.0}%", fraction * 100.0)));
        }
    }

    /// Get the action button for connecting signals.
    pub fn action_button(&self) -> Option<gtk::Button> {
        self.imp().action_btn.borrow().clone()
    }
}
