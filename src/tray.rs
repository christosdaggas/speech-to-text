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
//! channel sender; menu clicks are forwarded to the glib main loop.

use ksni::menu::{MenuItem, StandardItem};
use ksni::{Tray, TrayMethods};

use crate::i18n::gettext;

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
}

impl SttTray {
    fn emit(&self, action: TrayAction) {
        // Non-blocking: never stall the tray's D-Bus handler.
        let _ = self.tx.try_send(action);
    }
}

impl Tray for SttTray {
    fn id(&self) -> String {
        crate::APP_ID.to_string()
    }

    fn title(&self) -> String {
        crate::APP_NAME.to_string()
    }

    fn icon_name(&self) -> String {
        // Symbolic (monochrome) variant so the tray shows a small black-and-white
        // microphone that matches other status-area icons, not the large color icon.
        format!("{}-symbolic", crate::APP_ID)
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
pub fn spawn_tray() -> async_channel::Receiver<TrayAction> {
    let (tx, rx) = async_channel::unbounded::<TrayAction>();
    crate::application::tokio_runtime().spawn(async move {
        let tray = SttTray { tx };
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
