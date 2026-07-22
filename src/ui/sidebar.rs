// Speech to Text - Sidebar
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Sidebar navigation component (managed by MainWindow).
//! This module provides helper types for sidebar construction.

use gtk4 as gtk;
use gtk4::prelude::*;

/// Create a section header label for the sidebar.
#[allow(dead_code)]
pub fn create_section_header(title: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(title));
    label.set_halign(gtk::Align::Start);
    label.set_margin_start(12);
    label.set_margin_top(12);
    label.set_margin_bottom(4);
    label.add_css_class("dim-label");
    label.add_css_class("caption");
    label.add_css_class("sidebar-section-header");
    label
}

/// Sidebar is built directly in MainWindow::setup_ui.
/// This module exists for any extracted sidebar helpers.
#[allow(dead_code)]
pub struct Sidebar;

#[allow(dead_code)]
impl Sidebar {
    pub fn new() -> Self {
        Self
    }
}
