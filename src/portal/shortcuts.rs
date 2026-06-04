// Speech to Text - GlobalShortcuts portal
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Global dictation hotkey via the `org.freedesktop.portal.GlobalShortcuts`
//! portal.
//!
//! The portal session must stay alive for the whole app lifetime — dropping the
//! session or proxy ends the registration. [`run_global_shortcuts`] therefore
//! owns both for the entire `Activated` loop and never returns until the app
//! shuts down (the forwarding channel closes).
//!
//! Note for GNOME: the app's `preferred_trigger` is only a *suggestion*. The
//! desktop owns the real binding — the user confirms/changes it in
//! Settings → Keyboard. We never assume the requested accelerator took effect.

use ashpd::desktop::CreateSessionOptions;
use ashpd::desktop::global_shortcuts::{
    BindShortcutsOptions, GlobalShortcuts, NewShortcut,
};
use futures::StreamExt;
use tracing::{info, warn};

/// Application-provided shortcut id used in `bind_shortcuts` and matched on the
/// `Activated` signal.
const SHORTCUT_ID: &str = "start_dictation";

/// Convert a GTK accelerator string (e.g. `"<Ctrl><Alt>space"`) into the XDG
/// "shortcuts" trigger format (e.g. `"CTRL+ALT+space"`). Returns `None` when the
/// input doesn't look like a GTK accelerator, in which case no preferred trigger
/// is suggested and the desktop prompts the user to bind one.
fn gtk_accel_to_xdg_trigger(accel: &str) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut rest = accel.trim();

    // Leading <...> tokens are modifiers.
    while rest.starts_with('<') {
        let end = rest.find('>')?;
        let token = &rest[1..end];
        let modifier = match token.to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "primary" => "CTRL",
            "alt" => "ALT",
            "shift" => "SHIFT",
            "super" | "meta" | "logo" | "mod4" => "LOGO",
            _ => return None,
        };
        parts.push(modifier.to_string());
        rest = rest[end + 1..].trim_start();
    }

    let key = rest.trim();
    if key.is_empty() {
        return None;
    }
    parts.push(key.to_string());
    Some(parts.join("+"))
}

/// Long-lived task: create the session, bind the preferred trigger, then forward
/// each activation into `tx`. Best-effort — logs and returns on any error.
pub async fn run_global_shortcuts(preferred_trigger: String, tx: async_channel::Sender<()>) {
    if let Err(e) = run_inner(preferred_trigger, tx).await {
        warn!("Global shortcuts unavailable: {e}");
    }
}

async fn run_inner(
    preferred_trigger: String,
    tx: async_channel::Sender<()>,
) -> Result<(), ashpd::Error> {
    // Non-sandboxed apps must register their app id or the portal rejects the
    // request with "An app id is required".
    super::ensure_host_app_registered().await;

    let proxy = GlobalShortcuts::new().await?;
    let session = proxy.create_session(CreateSessionOptions::default()).await?;

    let trigger = gtk_accel_to_xdg_trigger(&preferred_trigger);
    if trigger.is_none() {
        warn!("Could not parse preferred shortcut '{preferred_trigger}'; the desktop will prompt for a binding");
    }
    let shortcut = NewShortcut::new(SHORTCUT_ID, "Start dictation")
        .preferred_trigger(trigger.as_deref());

    // Subscribe before binding so an immediate activation isn't missed.
    let mut activated = proxy.receive_activated().await?;

    let request = proxy
        .bind_shortcuts(&session, &[shortcut], None, BindShortcutsOptions::default())
        .await?;
    match request.response() {
        Ok(bound) => info!(
            "Global dictation shortcut bound ({} shortcut(s) registered)",
            bound.shortcuts().len()
        ),
        Err(e) => warn!("BindShortcuts returned an error (continuing): {e}"),
    }

    info!("Listening for global dictation shortcut (id='{SHORTCUT_ID}')");

    while let Some(activation) = activated.next().await {
        if activation.shortcut_id() == SHORTCUT_ID {
            info!("Global dictation shortcut activated");
            if tx.send(()).await.is_err() {
                break; // receiver dropped → app shutting down
            }
        }
    }

    // Hold the session for the whole loop; dropping it ends the registration.
    drop(session);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_gtk_accel_to_xdg_trigger() {
        assert_eq!(
            gtk_accel_to_xdg_trigger("<Ctrl><Alt>space").as_deref(),
            Some("CTRL+ALT+space")
        );
        assert_eq!(
            gtk_accel_to_xdg_trigger("<Primary><Shift>d").as_deref(),
            Some("CTRL+SHIFT+d")
        );
        assert_eq!(
            gtk_accel_to_xdg_trigger("<Super>k").as_deref(),
            Some("LOGO+k")
        );
    }

    #[test]
    fn rejects_non_accelerator() {
        assert_eq!(gtk_accel_to_xdg_trigger(""), None);
        assert_eq!(gtk_accel_to_xdg_trigger("<Ctrl>"), None);
        assert_eq!(gtk_accel_to_xdg_trigger("<Bogus>x"), None);
    }
}
