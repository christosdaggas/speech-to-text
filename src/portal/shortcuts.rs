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

use ashpd::desktop::global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut};
use ashpd::desktop::CreateSessionOptions;
use futures::StreamExt;
use tracing::{info, warn};

/// Application-provided shortcut ids used in `bind_shortcuts` and matched on the
/// `Activated` signal.
const SHORTCUT_ID: &str = "start_dictation";
const TRANSFORM_SHORTCUT_ID: &str = "transform_selection";

/// Which global shortcut fired (forwarded to the app on the glib loop).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutKind {
    /// Start/stop global dictation.
    Dictation,
    /// Transform the selection/clipboard with the active AI preset.
    TransformSelection,
}

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

/// Long-lived task: create the session, bind the preferred trigger(s), then
/// forward each activation into `tx`. `transform_trigger` is `Some` only when
/// the transform-selection shortcut is enabled.
///
/// The portal session can die mid-run (xdg-desktop-portal crash or upgrade,
/// session teardown). For an app that lives in the tray for days, a one-shot
/// registration silently killed the primary interaction path until restart —
/// so the session is recreated with backoff for as long as the app runs. (If
/// the portal dies without ending the signal stream, activations stop without
/// a detectable event; that residual case still needs an app restart.)
pub async fn run_global_shortcuts(
    dictation_trigger: String,
    transform_trigger: Option<String>,
    tx: async_channel::Sender<ShortcutKind>,
) {
    let mut delay = std::time::Duration::from_secs(2);
    loop {
        let started = std::time::Instant::now();
        let result = run_inner(&dictation_trigger, transform_trigger.clone(), &tx).await;
        if tx.is_closed() {
            return; // app shutting down
        }
        match result {
            Ok(()) => warn!("Global shortcuts stream ended; re-registering"),
            Err(e) => warn!("Global shortcuts unavailable ({e}); retrying in {delay:?}"),
        }
        // A run that survived a while means the portal was healthy; restart
        // the backoff from the beginning.
        if started.elapsed() >= std::time::Duration::from_secs(60) {
            delay = std::time::Duration::from_secs(2);
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(std::time::Duration::from_secs(120));
    }
}

async fn run_inner(
    dictation_trigger: &str,
    transform_trigger: Option<String>,
    tx: &async_channel::Sender<ShortcutKind>,
) -> Result<(), ashpd::Error> {
    // Non-sandboxed apps must register their app id or the portal rejects the
    // request with "An app id is required".
    super::ensure_host_app_registered().await;

    let proxy = GlobalShortcuts::new().await?;
    let session = proxy
        .create_session(CreateSessionOptions::default())
        .await?;

    // Pre-compute the XDG triggers so they outlive the borrowed `NewShortcut`s.
    let dict_trigger = gtk_accel_to_xdg_trigger(dictation_trigger);
    if dict_trigger.is_none() {
        warn!("Could not parse dictation shortcut '{dictation_trigger}'; the desktop will prompt for a binding");
    }
    let xform_trigger = transform_trigger
        .as_deref()
        .map(|t| (t.to_string(), gtk_accel_to_xdg_trigger(t)));

    let mut shortcuts =
        vec![NewShortcut::new(SHORTCUT_ID, "Start dictation")
            .preferred_trigger(dict_trigger.as_deref())];
    if let Some((raw, xdg)) = &xform_trigger {
        if xdg.is_none() {
            warn!(
                "Could not parse transform shortcut '{raw}'; the desktop will prompt for a binding"
            );
        }
        shortcuts.push(
            NewShortcut::new(TRANSFORM_SHORTCUT_ID, "Transform selection with AI")
                .preferred_trigger(xdg.as_deref()),
        );
    }

    // Subscribe before binding so an immediate activation isn't missed.
    let mut activated = proxy.receive_activated().await?;

    let request = proxy
        .bind_shortcuts(&session, &shortcuts, None, BindShortcutsOptions::default())
        .await?;
    match request.response() {
        Ok(bound) => info!(
            "Global shortcut(s) bound ({} registered)",
            bound.shortcuts().len()
        ),
        Err(e) => warn!("BindShortcuts returned an error (continuing): {e}"),
    }

    info!("Listening for global shortcuts");

    while let Some(activation) = activated.next().await {
        let id = activation.shortcut_id();
        let kind = if id == SHORTCUT_ID {
            Some(ShortcutKind::Dictation)
        } else if id == TRANSFORM_SHORTCUT_ID {
            Some(ShortcutKind::TransformSelection)
        } else {
            None
        };
        if let Some(kind) = kind {
            info!("Global shortcut activated: {kind:?}");
            if tx.send(kind).await.is_err() {
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
