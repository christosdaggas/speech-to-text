// Speech to Text - GPU Status Panel
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! GPU status info card displayed at the bottom of the sidebar.

use gtk4::prelude::*;
use gtk4::glib;
use gtk4 as gtk;
use libadwaita as adw;
use adw::subclass::prelude::*;
use std::cell::RefCell;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct GpuStatusPanel {
        pub gpu_brand_label: RefCell<Option<gtk::Label>>,
        pub gpu_model_label: RefCell<Option<gtk::Label>>,
        pub gpu_status_label: RefCell<Option<gtk::Label>>,
        pub gpu_icon: RefCell<Option<gtk::Image>>,
        pub vram_label: RefCell<Option<gtk::Label>>,
        pub vram_bar: RefCell<Option<gtk::LevelBar>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GpuStatusPanel {
        const NAME: &'static str = "SttGpuStatusPanel";
        type Type = super::GpuStatusPanel;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for GpuStatusPanel {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_ui();
        }
    }

    impl WidgetImpl for GpuStatusPanel {}
    impl BoxImpl for GpuStatusPanel {}
}

glib::wrapper! {
    pub struct GpuStatusPanel(ObjectSubclass<imp::GpuStatusPanel>)
        @extends gtk::Widget, gtk::Box;
}

impl GpuStatusPanel {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("orientation", gtk::Orientation::Vertical)
            .property("spacing", 6)
            .build()
    }

    fn setup_ui(&self) {
        let imp = self.imp();

        self.add_css_class("gpu-status-panel");
        self.set_margin_start(8);
        self.set_margin_end(8);
        self.set_margin_bottom(8);

        // Card frame
        let frame = gtk::Frame::new(None);
        frame.add_css_class("card");

        let inner = gtk::Box::new(gtk::Orientation::Vertical, 4);
        inner.set_margin_start(12);
        inner.set_margin_end(12);
        inner.set_margin_top(8);
        inner.set_margin_bottom(8);

        // Header row
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);

        let gpu_icon = gtk::Image::from_icon_name("video-display-symbolic");
        gpu_icon.set_pixel_size(14);
        header.append(&gpu_icon);

        let title = gtk::Label::new(Some("GPU"));
        title.add_css_class("caption-heading");
        title.set_hexpand(true);
        title.set_xalign(0.0);
        header.append(&title);

        let gpu_status_label = gtk::Label::new(Some("N/A"));
        gpu_status_label.add_css_class("caption");
        gpu_status_label.add_css_class("dim-label");
        header.append(&gpu_status_label);

        inner.append(&header);

        // GPU brand
        let gpu_brand_label = gtk::Label::new(Some("Detecting…"));
        gpu_brand_label.add_css_class("caption");
        gpu_brand_label.add_css_class("caption-heading");
        gpu_brand_label.set_xalign(0.0);
        gpu_brand_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        // Cap the natural width so a long GPU string can't widen the whole
        // sidebar (ellipsize only bounds the *minimum*; this bounds the natural).
        gpu_brand_label.set_max_width_chars(24);
        inner.append(&gpu_brand_label);

        // GPU model
        let gpu_model_label = gtk::Label::new(Some(""));
        gpu_model_label.add_css_class("caption");
        gpu_model_label.set_xalign(0.0);
        gpu_model_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        gpu_model_label.set_max_width_chars(24);
        inner.append(&gpu_model_label);

        // VRAM bar
        let vram_box = gtk::Box::new(gtk::Orientation::Vertical, 2);

        let vram_header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let vram_text = gtk::Label::new(Some("VRAM"));
        vram_text.add_css_class("caption");
        vram_text.add_css_class("dim-label");
        vram_text.set_hexpand(true);
        vram_text.set_xalign(0.0);
        vram_header.append(&vram_text);

        let vram_label = gtk::Label::new(Some("0 / 0 GB"));
        vram_label.add_css_class("caption");
        vram_label.add_css_class("dim-label");
        vram_header.append(&vram_label);

        vram_box.append(&vram_header);

        let vram_bar = gtk::LevelBar::new();
        vram_bar.set_min_value(0.0);
        vram_bar.set_max_value(1.0);
        vram_bar.set_value(0.0);
        vram_bar.set_hexpand(true);
        vram_box.append(&vram_bar);

        inner.append(&vram_box);

        frame.set_child(Some(&inner));
        self.append(&frame);

        // Store references
        *imp.gpu_brand_label.borrow_mut() = Some(gpu_brand_label);
        *imp.gpu_model_label.borrow_mut() = Some(gpu_model_label);
        *imp.gpu_status_label.borrow_mut() = Some(gpu_status_label);
        *imp.gpu_icon.borrow_mut() = Some(gpu_icon);
        *imp.vram_label.borrow_mut() = Some(vram_label);
        *imp.vram_bar.borrow_mut() = Some(vram_bar);

        // Detect GPU at startup, then refresh VRAM usage periodically.
        self.detect_gpu();
        self.start_auto_refresh();
    }

    /// Periodically re-read VRAM usage so the bar reflects the *current* state
    /// (otherwise it's frozen at the startup reading — e.g. showing "full" long
    /// after another app released the memory). Cheap AMD sysfs read when present;
    /// otherwise a throttled nvidia-smi query.
    fn start_auto_refresh(&self) {
        let panel = self.downgrade();
        glib::timeout_add_seconds_local(4, move || {
            let Some(panel) = panel.upgrade() else {
                return glib::ControlFlow::Break;
            };
            // Only meaningful once a GPU with VRAM was detected.
            let (tx, rx) = async_channel::bounded::<Option<(f64, f64)>>(1);
            std::thread::spawn(move || {
                let _ = tx.send_blocking(read_vram_usage());
            });
            let panel2 = panel.clone();
            glib::spawn_future_local(async move {
                if let Ok(Some((used, total))) = rx.recv().await {
                    panel2.update_vram(used, total);
                }
            });
            glib::ControlFlow::Continue
        });
    }

    /// Update only the VRAM bar + label (leaves the GPU name/status intact).
    pub fn update_vram(&self, vram_used_gb: f64, vram_total_gb: f64) {
        let imp = self.imp();
        if let Some(bar) = imp.vram_bar.borrow().as_ref() {
            if vram_total_gb > 0.0 {
                bar.set_value((vram_used_gb / vram_total_gb).clamp(0.0, 1.0));
            }
        }
        if let Some(label) = imp.vram_label.borrow().as_ref() {
            if vram_total_gb > 0.0 {
                label.set_text(&format!("{:.1} / {:.1} GB", vram_used_gb, vram_total_gb));
            }
        }
    }

    /// Probe the system for GPU info.
    fn detect_gpu(&self) {
        let (sender, receiver) = async_channel::bounded::<Option<(String, String, f64, f64)>>(1);

        std::thread::spawn(move || {
            let result = detect_gpu_info();
            // Also try to read current VRAM usage for AMD
            let result = result.map(|(name, driver, vram_total)| {
                let vram_used = read_amd_vram_used().unwrap_or(0.0);
                (name, driver, vram_total, vram_used)
            });
            let _ = sender.send_blocking(result);
        });

        let panel = self.clone();
        glib::spawn_future_local(async move {
            if let Ok(result) = receiver.recv().await {
                match result {
                    Some((name, _driver, vram_total, vram_used)) => {
                        panel.set_gpu_info(&name, "Available", vram_used, vram_total);
                    }
                    None => {
                        panel.set_no_gpu();
                    }
                }
            }
        });
    }

    /// Update GPU info.
    pub fn set_gpu_info(&self, name: &str, status: &str, vram_used_gb: f64, vram_total_gb: f64) {
        let imp = self.imp();

        let shortened = shorten_gpu_name(name);
        let (brand, model) = split_gpu_brand_model(&shortened);

        if let Some(label) = imp.gpu_brand_label.borrow().as_ref() {
            label.set_text(brand);
        }
        if let Some(label) = imp.gpu_model_label.borrow().as_ref() {
            label.set_text(model);
        }
        if let Some(label) = imp.gpu_status_label.borrow().as_ref() {
            label.set_text(status);
            label.remove_css_class("success");
            label.remove_css_class("error");
            label.remove_css_class("dim-label");
            match status {
                "Available" | "Active" => label.add_css_class("success"),
                "Unavailable" | "Error" => label.add_css_class("error"),
                _ => label.add_css_class("dim-label"),
            }
        }
        if let Some(bar) = imp.vram_bar.borrow().as_ref() {
            if vram_total_gb > 0.0 {
                bar.set_value(vram_used_gb / vram_total_gb);
            }
        }
        if let Some(label) = imp.vram_label.borrow().as_ref() {
            label.set_text(&format!("{:.1} / {:.1} GB", vram_used_gb, vram_total_gb));
        }
    }

    /// Set status to "no GPU detected".
    pub fn set_no_gpu(&self) {
        let imp = self.imp();
        if let Some(label) = imp.gpu_brand_label.borrow().as_ref() {
            label.set_text("No GPU detected");
        }
        if let Some(label) = imp.gpu_model_label.borrow().as_ref() {
            label.set_text("");
        }
        if let Some(label) = imp.gpu_status_label.borrow().as_ref() {
            label.set_text("CPU only");
            label.remove_css_class("success");
            label.remove_css_class("error");
            label.add_css_class("dim-label");
        }
        if let Some(label) = imp.vram_label.borrow().as_ref() {
            label.set_text("N/A");
        }
    }
}

/// Detect GPU info by parsing lspci, sysfs, nvidia-smi.
/// Returns Some((name, driver_version, vram_gb)) or None.
pub fn detect_gpu_info() -> Option<(String, String, f64)> {
    // Try nvidia-smi first (most reliable for NVIDIA)
    if let Ok(output) = std::process::Command::new("nvidia-smi")
        .arg("--query-gpu=name,driver_version,memory.total")
        .arg("--format=csv,noheader,nounits")
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let parts: Vec<&str> = stdout.trim().split(", ").collect();
            if parts.len() >= 3 {
                let name = parts[0].to_string();
                let driver = parts[1].to_string();
                let vram_mb: f64 = parts[2].trim().parse().unwrap_or(0.0);
                return Some((name, driver, vram_mb / 1024.0));
            }
        }
    }

    // Try AMD via sysfs + lspci
    if let Some(result) = detect_amd_gpu() {
        return Some(result);
    }

    // Generic fallback: parse lspci for any discrete GPU
    if let Ok(output) = std::process::Command::new("lspci").output() {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let lower = line.to_lowercase();
                if lower.contains("vga") || lower.contains("3d controller") || lower.contains("display controller") {
                    if let Some(idx) = line.find(": ") {
                        let name = line[idx + 2..].trim().to_string();
                        return Some((name, "Unknown".into(), 0.0));
                    }
                }
            }
        }
    }

    None
}

/// Detect AMD GPU using sysfs and lspci.
fn detect_amd_gpu() -> Option<(String, String, f64)> {
    // Find a DRM card driven by amdgpu
    let drm_dir = std::path::Path::new("/sys/class/drm");
    if !drm_dir.exists() {
        return None;
    }

    for entry in std::fs::read_dir(drm_dir).ok()? {
        let entry = entry.ok()?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Match card0, card1, etc. (not card0-DP-1 etc.)
        if !name_str.starts_with("card") || name_str.contains('-') {
            continue;
        }

        let device_dir = entry.path().join("device");
        let uevent_path = device_dir.join("uevent");

        // Check if this card uses the amdgpu driver
        if let Ok(uevent) = std::fs::read_to_string(&uevent_path) {
            if !uevent.contains("DRIVER=amdgpu") {
                continue;
            }
        } else {
            continue;
        }

        // Read VRAM total (bytes)
        let vram_gb = std::fs::read_to_string(device_dir.join("mem_info_vram_total"))
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .map(|bytes| bytes as f64 / (1024.0 * 1024.0 * 1024.0))
            .unwrap_or(0.0);

        // Get GPU name from lspci
        let gpu_name = get_lspci_gpu_name("amd")
            .unwrap_or_else(|| "AMD GPU".to_string());

        return Some((gpu_name, "amdgpu".to_string(), vram_gb));
    }

    None
}

/// Extract GPU name from lspci output matching a vendor keyword.
fn get_lspci_gpu_name(vendor: &str) -> Option<String> {
    let output = std::process::Command::new("lspci").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let lower = line.to_lowercase();
        if (lower.contains("vga") || lower.contains("3d controller") || lower.contains("display controller"))
            && lower.contains(vendor)
        {
            if let Some(idx) = line.find(": ") {
                return Some(line[idx + 2..].trim().to_string());
            }
        }
    }
    None
}

/// Read current (used, total) VRAM in GB. Tries the cheap AMD sysfs path first
/// (no process spawn); falls back to an nvidia-smi query. None if unavailable.
fn read_vram_usage() -> Option<(f64, f64)> {
    if let (Some(used), Some(total)) = (read_amd_vram_used(), read_amd_vram_total()) {
        if total > 0.0 {
            return Some((used, total));
        }
    }
    let output = std::process::Command::new("nvidia-smi")
        .arg("--query-gpu=memory.used,memory.total")
        .arg("--format=csv,noheader,nounits")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.lines().next()?;
    let parts: Vec<&str> = line.split(',').map(|p| p.trim()).collect();
    if parts.len() < 2 {
        return None;
    }
    let used_mb: f64 = parts[0].parse().ok()?;
    let total_mb: f64 = parts[1].parse().ok()?;
    Some((used_mb / 1024.0, total_mb / 1024.0))
}

/// Read AMD total VRAM in GB from sysfs.
fn read_amd_vram_total() -> Option<f64> {
    let drm_dir = std::path::Path::new("/sys/class/drm");
    for entry in std::fs::read_dir(drm_dir).ok()? {
        let entry = entry.ok()?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with("card") || name_str.contains('-') {
            continue;
        }
        let device_dir = entry.path().join("device");
        if let Ok(s) = std::fs::read_to_string(device_dir.join("mem_info_vram_total")) {
            if let Ok(bytes) = s.trim().parse::<u64>() {
                return Some(bytes as f64 / (1024.0 * 1024.0 * 1024.0));
            }
        }
    }
    None
}

/// Read current AMD VRAM usage in GB from sysfs.
fn read_amd_vram_used() -> Option<f64> {
    let drm_dir = std::path::Path::new("/sys/class/drm");
    for entry in std::fs::read_dir(drm_dir).ok()? {
        let entry = entry.ok()?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with("card") || name_str.contains('-') {
            continue;
        }
        let device_dir = entry.path().join("device");
        if let Ok(s) = std::fs::read_to_string(device_dir.join("mem_info_vram_used")) {
            if let Ok(bytes) = s.trim().parse::<u64>() {
                return Some(bytes as f64 / (1024.0 * 1024.0 * 1024.0));
            }
        }
    }
    None
}

/// Split a shortened GPU name into (brand, model).
/// e.g. "AMD Radeon RX 7800 XT" → ("AMD", "Radeon RX 7800 XT")
/// e.g. "NVIDIA GeForce RTX 3060" → ("NVIDIA", "GeForce RTX 3060")
fn split_gpu_brand_model(name: &str) -> (&str, &str) {
    for prefix in &["NVIDIA ", "AMD ", "Intel "] {
        if name.starts_with(prefix) {
            return (prefix.trim(), &name[prefix.len()..]);
        }
    }
    (name, "")
}

/// Shorten a verbose GPU name to just manufacturer + model.
/// e.g. "Advanced Micro Devices, Inc. [AMD/ATI] Navi 32 [Radeon RX 7800 XT]" → "AMD Radeon RX 7800 XT"
/// e.g. "NVIDIA Corporation GA106 [GeForce RTX 3060]" → "NVIDIA GeForce RTX 3060"
fn shorten_gpu_name(name: &str) -> String {
    let lower = name.to_lowercase();
    let mfg = if lower.contains("nvidia") {
        "NVIDIA"
    } else if lower.contains("amd") || lower.contains("ati") || lower.contains("radeon") {
        "AMD"
    } else if lower.contains("intel") {
        "Intel"
    } else {
        return name.to_string();
    };

    // Try to extract the marketing name from the last bracket pair, e.g. [Radeon RX 7800 XT]
    if let Some(start) = name.rfind('[') {
        if let Some(end) = name.rfind(']') {
            if start < end {
                let inner = name[start + 1..end].trim();
                // If there are multiple slash-separated names, keep only the first two
                let inner = if inner.contains(" / ") {
                    let parts: Vec<&str> = inner.split(" / ").collect();
                    if parts.len() > 2 {
                        parts[..2].join(" / ")
                    } else {
                        inner.to_string()
                    }
                } else {
                    inner.to_string()
                };
                if inner.to_lowercase().starts_with(&mfg.to_lowercase()) {
                    return inner;
                }
                return format!("{} {}", mfg, inner);
            }
        }
    }

    name.to_string()
}

/// Public wrapper for shorten_gpu_name.
pub fn shorten_gpu_name_public(name: &str) -> String {
    shorten_gpu_name(name)
}
