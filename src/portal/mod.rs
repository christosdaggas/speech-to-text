// Speech to Text - XDG Desktop Portal integration
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Thin wrappers around the XDG Desktop Portals used by the mini panel:
//! `GlobalShortcuts` (the global dictation hotkey) and `RemoteDesktop`
//! (best-effort auto-paste). All entry points are best-effort: if a portal is
//! unavailable or the user denies it, the app keeps working with in-app
//! recording and clipboard-only paste.

pub mod paste;
pub mod shortcuts;

use std::sync::atomic::{AtomicBool, Ordering};

/// Tracks whether the host app id has been registered with the portal Registry.
static REGISTERED: AtomicBool = AtomicBool::new(false);

/// Register this process's app id with `org.freedesktop.host.portal.Registry`.
///
/// Non-sandboxed (host) apps have no inherent app id, so portals like
/// GlobalShortcuts reject them with "An app id is required". Registering once
/// associates our app id with ashpd's shared session-bus connection, which all
/// later portal calls reuse. Idempotent and best-effort; a no-op under Flatpak
/// (where the sandbox already provides an app id).
pub async fn ensure_host_app_registered() {
    if REGISTERED.swap(true, Ordering::SeqCst) {
        return;
    }
    let app_id = match ashpd::AppID::try_from(crate::APP_ID) {
        Ok(id) => id,
        Err(e) => {
            tracing::warn!("Invalid app id '{}': {e}", crate::APP_ID);
            return;
        }
    };
    if let Err(e) = ashpd::register_host_app(app_id).await {
        tracing::warn!("Failed to register host app id with portal: {e}");
        // Allow a later retry if registration failed transiently.
        REGISTERED.store(false, Ordering::SeqCst);
    } else {
        tracing::info!("Registered host app id '{}' with the portal", crate::APP_ID);
    }
}
