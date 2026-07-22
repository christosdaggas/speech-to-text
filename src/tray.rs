// Speech to Text - System Tray
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! System tray icon via the StatusNotifierItem spec (`ksni`).
//!
//! Lets the app run in the background without its main window: the tray icon
//! and global shortcut stay active. Works natively on KDE; on GNOME it requires
//! the "AppIndicator and KStatusNotifierItem Support" extension (the tray host).
//! Best-effort: if no tray host is present, registration fails quietly and the
//! app keeps working via the main window and global shortcut.
//!
//! `ksni::Tray` must be `Send + 'static`, so the tray struct holds only a
//! channel sender and the pre-decoded icon; menu clicks are forwarded to the
//! glib main loop.
//!
//! The icon is shipped as `IconPixmap` (raw ARGB32 decoded from PNGs embedded
//! in the binary) rather than left to an `IconName` + `IconThemePath` lookup.
//! Name-based lookup is unreliable: every host implements the search differently
//! and most only probe `<theme>/<size>/{apps,status,panel}/`, so a symbolic icon
//! living in `<theme>/symbolic/apps/` is never found and the tray shows an empty
//! slot. Sending the pixels makes the icon independent of the host's search
//! rules, of icon caches and of whether the app is installed at all.
//! `icon_name`/`icon_theme_path` stay as a secondary path for hosts that prefer
//! a themed (recolourable) icon.

use ksni::menu::{MenuItem, StandardItem};
use ksni::{Tray, TrayMethods};

use crate::i18n::gettext;

/// Monochrome tray artwork, pre-rendered from
/// `data/icons/hicolor/symbolic/apps/…-symbolic.svg`. Several sizes so the host
/// can pick the one matching its panel instead of rescaling a single bitmap.
const TRAY_PNGS: &[&[u8]] = &[
    include_bytes!("../data/icons/tray/tray-16.png"),
    include_bytes!("../data/icons/tray/tray-22.png"),
    include_bytes!("../data/icons/tray/tray-24.png"),
    include_bytes!("../data/icons/tray/tray-32.png"),
    include_bytes!("../data/icons/tray/tray-48.png"),
    include_bytes!("../data/icons/tray/tray-64.png"),
];

/// Actions the tray can request from the application (handled on the glib loop).
#[derive(Debug, Clone, Copy)]
pub enum TrayAction {
    /// Start/stop global dictation (open the mini panel).
    Dictate,
    /// Transform the current selection/clipboard with the active AI preset.
    TransformSelection,
    /// Show the main window.
    Open,
    /// Quit the application.
    Quit,
}

struct SttTray {
    tx: async_channel::Sender<TrayAction>,
    /// Decoded once at startup; `ksni::Icon` is plain data, so it moves to the
    /// tray's thread with the struct.
    icons: Vec<ksni::Icon>,
}

impl SttTray {
    fn emit(&self, action: TrayAction) {
        // Non-blocking: never stall the tray's D-Bus handler.
        let _ = self.tx.try_send(action);
    }
}

/// Decode a PNG into the `IconPixmap` wire format: ARGB32, network byte order,
/// rows packed tightly (no rowstride padding, which gdk-pixbuf may add).
fn decode_png(bytes: &[u8]) -> Option<ksni::Icon> {
    use gtk4::gdk_pixbuf::PixbufLoader;
    use gtk4::prelude::PixbufLoaderExt;

    let loader = PixbufLoader::new();
    loader.write(bytes).ok()?;
    loader.close().ok()?;

    let pixbuf = loader.pixbuf()?;
    let pixbuf = if pixbuf.has_alpha() {
        pixbuf
    } else {
        pixbuf.add_alpha(false, 0, 0, 0).ok()?
    };

    let (width, height) = (pixbuf.width() as usize, pixbuf.height() as usize);
    let channels = pixbuf.n_channels() as usize;
    let rowstride = pixbuf.rowstride() as usize;
    // SAFETY: the pixbuf is owned by this function and not mutated or shared
    // while the slice is alive.
    let pixels = unsafe { pixbuf.pixels() };

    let mut data = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        let row = &pixels[y * rowstride..y * rowstride + width * channels];
        for px in row.chunks_exact(channels) {
            // RGBA -> ARGB
            data.extend_from_slice(&[px[3], px[0], px[1], px[2]]);
        }
    }

    Some(ksni::Icon {
        width: width as i32,
        height: height as i32,
        data,
    })
}

impl Tray for SttTray {
    fn id(&self) -> String {
        crate::APP_ID.to_string()
    }

    fn title(&self) -> String {
        crate::APP_NAME.to_string()
    }

    /// Only a hint for hosts that resolve `icon_name` themselves — the pixmap
    /// below is what actually gets drawn. Returns the first theme root that
    /// really holds our icon so we never advertise a stale or dev-only path.
    fn icon_theme_path(&self) -> String {
        let roots = [
            "/usr/share/icons/hicolor".to_string(),
            "/usr/local/share/icons/hicolor".to_string(),
            format!(
                "{}/icons/hicolor",
                gtk4::glib::user_data_dir().to_string_lossy()
            ),
            format!("{}/data/icons/hicolor", env!("CARGO_MANIFEST_DIR")),
        ];

        let icon = format!("{}-symbolic.svg", crate::APP_ID);
        roots
            .iter()
            .find(|root| {
                // The layouts hosts actually probe, plus the canonical symbolic one.
                ["symbolic/apps", "scalable/apps"]
                    .iter()
                    .any(|dir| std::path::Path::new(root).join(dir).join(&icon).is_file())
            })
            .cloned()
            .unwrap_or_default()
    }

    fn icon_name(&self) -> String {
        // Symbolic (monochrome) variant so the tray shows a small black-and-white
        // microphone that matches other status-area icons, not the large color icon.
        format!("{}-symbolic", crate::APP_ID)
    }

    /// The authoritative icon: raw pixels, so no host-side theme lookup is
    /// involved. Empty only if PNG decoding somehow fails, in which case hosts
    /// fall back to `icon_name`.
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icons.clone()
    }

    /// Left-click on the tray icon opens the main window.
    fn activate(&mut self, _x: i32, _y: i32) {
        self.emit(TrayAction::Open);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: gettext("Start Dictation"),
                activate: Box::new(|t: &mut Self| t.emit(TrayAction::Dictate)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: gettext("Transform Selection with AI"),
                activate: Box::new(|t: &mut Self| t.emit(TrayAction::TransformSelection)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: gettext("Open"),
                activate: Box::new(|t: &mut Self| t.emit(TrayAction::Open)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: gettext("Quit"),
                icon_name: "application-exit-symbolic".into(),
                activate: Box::new(|t: &mut Self| t.emit(TrayAction::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Spawn the tray on the Tokio runtime and return a receiver of tray actions to
/// be consumed on the glib main loop. The tray lives for the app's lifetime.
///
/// Must be called from the main thread: the icons are decoded here, before the
/// tray struct is moved onto the Tokio runtime.
pub fn spawn_tray() -> async_channel::Receiver<TrayAction> {
    let (tx, rx) = async_channel::unbounded::<TrayAction>();

    let icons: Vec<ksni::Icon> = TRAY_PNGS.iter().filter_map(|png| decode_png(png)).collect();
    if icons.is_empty() {
        tracing::warn!("Tray icons could not be decoded; falling back to icon-name lookup");
    }

    crate::application::tokio_runtime().spawn(async move {
        let tray = SttTray { tx, icons };
        match tray.spawn().await {
            Ok(_handle) => {
                tracing::info!("System tray registered");
                // Hold the handle for the whole app lifetime; dropping it removes
                // the tray icon.
                std::future::pending::<()>().await;
            }
            Err(e) => tracing::warn!("System tray unavailable (no host?): {e}"),
        }
    });
    rx
}
