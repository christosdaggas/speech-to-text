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

use ashpd::desktop::clipboard::{Clipboard, RequestClipboardOptions, SetSelectionOptions};
use ashpd::desktop::remote_desktop::{
    DeviceType, KeyState, NotifyKeyboardKeysymOptions, RemoteDesktop, SelectDevicesOptions,
    StartOptions,
};
use ashpd::desktop::{CreateSessionOptions, PersistMode};

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
#[tracing::instrument(name = "portal.autopaste")]
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
    let session = proxy
        .create_session(CreateSessionOptions::default())
        .await?;

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
    proxy
        .notify_keyboard_keysym(&session, XK_CONTROL_L, KeyState::Pressed, opts())
        .await?;
    proxy
        .notify_keyboard_keysym(&session, XK_V, KeyState::Pressed, opts())
        .await?;
    proxy
        .notify_keyboard_keysym(&session, XK_V, KeyState::Released, opts())
        .await?;
    proxy
        .notify_keyboard_keysym(&session, XK_CONTROL_L, KeyState::Released, opts())
        .await?;

    let _ = session.close().await;
    info!("Auto-pasted via RemoteDesktop portal");
    Ok(true)
}

/// Deliver `text` into the currently-focused app via the Clipboard portal +
/// Ctrl+V, using a single RemoteDesktop session.
///
/// Unlike [`try_autopaste`] (which pastes whatever the *caller* already put on
/// the GTK clipboard), this OWNS the system selection through the portal's
/// `SetSelection`/`SelectionTransfer` mechanism. On GNOME/Mutter and KDE/KWin the
/// portal backend sets the selection through a privileged path (`ext-data-control`)
/// that does **not** require our window to hold keyboard focus — so the *current*
/// transcript is delivered even when the mini panel never had focus (the case
/// plain `clipboard.set_text()` silently fails, pasting stale text).
///
/// The caller must ensure the target app holds keyboard focus before calling
/// (i.e. hide the panel first): the injected Ctrl+V lands on the focused surface.
/// Returns `true` only when the portal granted keyboard injection AND a working
/// clipboard and the keystroke was sent; returns `false` (so the caller can fall
/// back) when the compositor has no Clipboard portal or on any error — it never
/// injects a stale paste.
#[tracing::instrument(name = "portal.paste_text", skip(text))]
pub async fn paste_text_via_remote_desktop(text: String) -> bool {
    match paste_text_inner(text).await {
        Ok(v) => v,
        Err(e) => {
            warn!("Portal clipboard paste failed: {e}");
            false
        }
    }
}

async fn paste_text_inner(text: String) -> Result<bool, ashpd::Error> {
    use futures::StreamExt;
    use std::io::Write;
    use std::time::Duration;

    // Ensure the portal can identify this (non-sandboxed) app.
    super::ensure_host_app_registered().await;

    let proxy = RemoteDesktop::new().await?;
    let session = proxy
        .create_session(CreateSessionOptions::default())
        .await?;

    let restore_token = load_restore_token();
    let select = SelectDevicesOptions::default()
        .set_devices(BitFlags::from(DeviceType::Keyboard))
        .set_persist_mode(PersistMode::ExplicitlyRevoked)
        .set_restore_token(restore_token.as_deref());
    proxy.select_devices(&session, select).await?.response()?;

    // RequestClipboard must be called after SelectDevices and before Start.
    let clipboard = Clipboard::new().await?;
    clipboard
        .request(&session, RequestClipboardOptions::default())
        .await?;

    let started = proxy
        .start(&session, None, StartOptions::default())
        .await?
        .response()?;

    if !started.devices().contains(DeviceType::Keyboard) {
        warn!("RemoteDesktop session did not grant keyboard access");
        let _ = session.close().await;
        return Ok(false);
    }
    // Persist the (possibly refreshed) restore token so consent is shown once.
    if let Some(token) = started.restore_token() {
        save_restore_token(token);
    }
    if !started.is_clipboard_enabled() {
        info!("Compositor offers no Clipboard portal — caller will fall back");
        let _ = session.close().await;
        return Ok(false);
    }

    // Subscribe to transfer requests BEFORE advertising the selection so we don't
    // miss the read the target app issues in response to Ctrl+V.
    let transfers = clipboard
        .receive_selection_transfer::<RemoteDesktop>()
        .await?;

    clipboard
        .set_selection(
            &session,
            SetSelectionOptions::default()
                .set_mime_types(&["text/plain;charset=utf-8", "text/plain"]),
        )
        .await?;

    // Inject Ctrl+V into the (now focused) target app. Ctrl and 'v' exist in every
    // Latin layout, so this resolves regardless of the transcript's language.
    let opts = NotifyKeyboardKeysymOptions::default;
    proxy
        .notify_keyboard_keysym(&session, XK_CONTROL_L, KeyState::Pressed, opts())
        .await?;
    proxy
        .notify_keyboard_keysym(&session, XK_V, KeyState::Pressed, opts())
        .await?;
    proxy
        .notify_keyboard_keysym(&session, XK_V, KeyState::Released, opts())
        .await?;
    proxy
        .notify_keyboard_keysym(&session, XK_CONTROL_L, KeyState::Released, opts())
        .await?;

    // Serve the target app's data request(s): answer every SelectionTransfer with
    // the transcript bytes. Wait up to 2.5s for the first read (after Ctrl+V), then
    // only briefly for follow-ups — so we release the session (and the compositor's
    // "remote control" indicator) as soon as the paste has been served.
    let bytes = text.into_bytes();
    let mut transfers = std::pin::pin!(transfers);
    let mut served_any = false;
    loop {
        let wait = if served_any {
            Duration::from_millis(500)
        } else {
            Duration::from_millis(2500)
        };
        match tokio::time::timeout(wait, transfers.next()).await {
            Ok(Some((sess, _mime, serial))) => {
                served_any = true;
                if let Ok(zfd) = clipboard.selection_write(&sess, serial).await {
                    let std_fd: std::os::fd::OwnedFd = zfd.into();
                    let mut f = std::fs::File::from(std_fd);
                    let ok = f.write_all(&bytes).and_then(|_| f.flush()).is_ok();
                    drop(f); // closes the pipe write end
                    let _ = clipboard.selection_write_done(&sess, serial, ok).await;
                }
            }
            Ok(None) => break, // signal stream ended
            Err(_) => break,   // quiet period elapsed — paste served
        }
    }

    let _ = session.close().await;
    info!("Delivered transcript via Clipboard portal + Ctrl+V (served={served_any})");
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
    // The restore token lets us re-acquire input-injection permission without a
    // prompt, so it is sensitive: write it privately (0600) and atomically.
    if let Err(e) = crate::fsio::write_private(&restore_token_path(), token.as_bytes()) {
        warn!("Failed to persist RemoteDesktop restore token: {e}");
    }
}

/// Delete the persisted RemoteDesktop restore token. After this, the next
/// auto-paste re-prompts for input-injection permission. Exposed so the user can
/// revoke the granted permission from Settings. Returns `true` if a token was
/// removed (or none existed); `false` only on an unexpected I/O error.
pub fn revoke_restore_token() -> bool {
    let path = restore_token_path();
    match std::fs::remove_file(&path) {
        Ok(()) => {
            info!("Removed RemoteDesktop restore token");
            true
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
        Err(e) => {
            warn!("Failed to remove RemoteDesktop restore token: {e}");
            false
        }
    }
}
