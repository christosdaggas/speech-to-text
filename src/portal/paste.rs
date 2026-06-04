// Speech to Text - Auto-paste
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Best-effort auto-paste. The clipboard is ALWAYS set by the caller first and
//! is the reliable fallback; this module only attempts to *additionally*
//! synthesize Ctrl+V into the currently focused application.
//!
//! Primary path on GNOME/Wayland: the `org.freedesktop.portal.RemoteDesktop`
//! portal (no extra install). A persistence `restore_token` is saved to disk so
//! the one-time consent dialog is not shown again on later runs. `ydotool` is
//! used only as a fallback when present (it needs the `ydotoold` daemon and
//! uinput access). `wtype` is intentionally not used — Mutter does not implement
//! the virtual-keyboard protocol it relies on.

use enumflags2::BitFlags;
use tracing::{info, warn};

use ashpd::desktop::{CreateSessionOptions, PersistMode};
use ashpd::desktop::remote_desktop::{
    DeviceType, KeyState, NotifyKeyboardKeysymOptions, RemoteDesktop, SelectDevicesOptions,
    StartOptions,
};

/// X keysym for the left Control key.
const XK_CONTROL_L: i32 = 0xffe3;
/// X keysym for lowercase `v`.
const XK_V: i32 = 0x0076;

/// Which auto-paste helper to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteHelper {
    /// XDG RemoteDesktop portal (preferred on GNOME/Wayland).
    RemoteDesktopPortal,
    /// External `ydotool` binary (needs daemon + uinput access).
    Ydotool,
    /// No automated paste available — clipboard only.
    None,
}

/// Decide which helper to use. Prefers the RemoteDesktop portal (always tried
/// first by [`try_autopaste`]); reports `Ydotool` only when the portal isn't an
/// option and `ydotool` is installed.
pub fn detect_paste_helper() -> PasteHelper {
    // The portal can't be cheaply probed without creating a session, so we
    // optimistically prefer it and degrade gracefully at use time.
    if remote_desktop_portal_likely() {
        PasteHelper::RemoteDesktopPortal
    } else if ydotool_available() {
        PasteHelper::Ydotool
    } else {
        PasteHelper::None
    }
}

/// Heuristic: a portal frontend is reachable (we're in a desktop session bus).
fn remote_desktop_portal_likely() -> bool {
    std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some()
        || std::env::var_os("XDG_RUNTIME_DIR").is_some()
}

/// Whether a `ydotool` binary is on `PATH`.
pub fn ydotool_available() -> bool {
    binary_on_path("ydotool")
}

fn binary_on_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(bin).is_file()))
        .unwrap_or(false)
}

/// Attempt to paste the current clipboard contents into the focused app.
/// Returns `true` only if a paste was actually injected. Never panics; on any
/// failure the caller's clipboard text remains the fallback.
pub async fn try_autopaste() -> bool {
    match detect_paste_helper() {
        PasteHelper::RemoteDesktopPortal => {
            match paste_via_remote_desktop().await {
                Ok(true) => return true,
                Ok(false) => {}
                Err(e) => warn!("RemoteDesktop auto-paste failed: {e}"),
            }
            // Portal didn't work — fall back to ydotool if it's installed.
            if ydotool_available() {
                return paste_via_ydotool();
            }
            info!("Auto-paste unavailable — text remains on the clipboard");
            false
        }
        PasteHelper::Ydotool => paste_via_ydotool(),
        PasteHelper::None => {
            info!("No auto-paste helper available — text remains on the clipboard");
            false
        }
    }
}

/// Inject Ctrl+V via the RemoteDesktop portal. Uses a persisted restore token so
/// the consent dialog is only shown once (per grant).
async fn paste_via_remote_desktop() -> Result<bool, ashpd::Error> {
    // Ensure the portal can identify this (non-sandboxed) app.
    super::ensure_host_app_registered().await;

    let proxy = RemoteDesktop::new().await?;
    let session = proxy.create_session(CreateSessionOptions::default()).await?;

    let restore_token = load_restore_token();
    let select = SelectDevicesOptions::default()
        .set_devices(BitFlags::from(DeviceType::Keyboard))
        .set_persist_mode(PersistMode::ExplicitlyRevoked)
        .set_restore_token(restore_token.as_deref());
    proxy.select_devices(&session, select).await?.response()?;

    let started = proxy
        .start(&session, None, StartOptions::default())
        .await?
        .response()?;

    if !started.devices().contains(DeviceType::Keyboard) {
        warn!("RemoteDesktop session did not grant keyboard access");
        let _ = session.close().await;
        return Ok(false);
    }

    // Persist the restore token so the next run skips the consent dialog.
    if let Some(token) = started.restore_token() {
        save_restore_token(token);
    }

    // Ctrl down, V down, V up, Ctrl up.
    let opts = NotifyKeyboardKeysymOptions::default;
    proxy.notify_keyboard_keysym(&session, XK_CONTROL_L, KeyState::Pressed, opts()).await?;
    proxy.notify_keyboard_keysym(&session, XK_V, KeyState::Pressed, opts()).await?;
    proxy.notify_keyboard_keysym(&session, XK_V, KeyState::Released, opts()).await?;
    proxy.notify_keyboard_keysym(&session, XK_CONTROL_L, KeyState::Released, opts()).await?;

    let _ = session.close().await;
    info!("Auto-pasted via RemoteDesktop portal");
    Ok(true)
}

/// Inject Ctrl+V via `ydotool` (best effort). Runs synchronously.
fn paste_via_ydotool() -> bool {
    // 29 = KEY_LEFTCTRL, 47 = KEY_V (Linux input event codes); :1 down, :0 up.
    match std::process::Command::new("ydotool")
        .args(["key", "29:1", "47:1", "47:0", "29:0"])
        .status()
    {
        Ok(status) if status.success() => {
            info!("Auto-pasted via ydotool");
            true
        }
        Ok(status) => {
            warn!("ydotool exited with status {status}");
            false
        }
        Err(e) => {
            warn!("Failed to run ydotool: {e}");
            false
        }
    }
}

fn restore_token_path() -> std::path::PathBuf {
    crate::config::AppConfig::config_dir().join("remote_desktop.token")
}

fn load_restore_token() -> Option<String> {
    std::fs::read_to_string(restore_token_path())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn save_restore_token(token: &str) {
    let path = restore_token_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&path, token) {
        warn!("Failed to persist RemoteDesktop restore token: {e}");
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
}
