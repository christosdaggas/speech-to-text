// Speech to Text - Auto-paste
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Best-effort auto-paste via the `org.freedesktop.portal.RemoteDesktop` and
//! `org.freedesktop.portal.Clipboard` portals, with `ydotool` as a fallback.
//!
//! All portal work runs on a single long-lived actor task that owns ONE
//! persistent RemoteDesktop session for the app's lifetime:
//!
//! * Creating a session costs several D-Bus round-trips (and flashes the
//!   compositor's "remote control" indicator), so paying it once instead of on
//!   every paste removes ~1s of dead time from the dictate→paste loop.
//! * The portal-owned clipboard selection stays serviceable after a delivery
//!   returns — the actor keeps answering `SelectionTransfer` requests with the
//!   latest transcript, so the text genuinely remains "on the clipboard".
//! * Delivery reports honestly whether the target application actually read
//!   the selection ([`DeliveryOutcome`]), so callers can fall back instead of
//!   believing a paste happened when nothing consumed it.
//!
//! The consent dialog is only requested interactively from Settings
//! ([`acquire_permission_interactive`]); at paste time a missing grant means
//! "portal unavailable" rather than a surprise dialog over the target app.
//! A persistence `restore_token` is saved so the one-time grant survives
//! restarts. `wtype` is intentionally not used — Mutter does not implement the
//! virtual-keyboard protocol it relies on.

use std::sync::OnceLock;
use std::time::Duration;

use enumflags2::BitFlags;
use futures::StreamExt;
use tracing::{info, warn};

use ashpd::desktop::clipboard::{Clipboard, RequestClipboardOptions, SetSelectionOptions};
use ashpd::desktop::remote_desktop::{
    DeviceType, KeyState, NotifyKeyboardKeysymOptions, RemoteDesktop, SelectDevicesOptions,
    StartOptions,
};
use ashpd::desktop::Session;
use ashpd::desktop::{CreateSessionOptions, PersistMode};

/// X keysym for the left Control key.
const XK_CONTROL_L: i32 = 0xffe3;
/// X keysym for lowercase `v`.
const XK_V: i32 = 0x0076;

/// How long to wait for the target app's first clipboard read after Ctrl+V.
const FIRST_READ_TIMEOUT: Duration = Duration::from_millis(1500);
/// Quiet period after the last served read before a delivery returns.
const QUIET_PERIOD: Duration = Duration::from_millis(200);

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

/// Result of a portal text delivery ([`deliver_text_via_portal`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryOutcome {
    /// Selection set, Ctrl+V injected, and the target application actually
    /// read the transcript. The portal session keeps serving later reads, so
    /// the text remains on the system clipboard.
    Pasted,
    /// Ctrl+V was injected but nothing read the selection in time. The portal
    /// still owns the clipboard with the current transcript, so a manual
    /// Ctrl+V will paste it — but the auto-paste itself cannot be confirmed.
    NotConsumed,
    /// The portal path is unavailable (no grant yet, denied, no Clipboard
    /// portal, or a D-Bus failure). Nothing was injected; the caller must
    /// fall back to the GTK clipboard.
    Unavailable,
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

/// Whether the user has already granted RemoteDesktop access (a persisted
/// restore token exists). Without it, portal paste is skipped silently instead
/// of opening a consent dialog mid-delivery.
pub fn has_portal_grant() -> bool {
    load_restore_token().is_some()
}

// ---------------------------------------------------------------------------
// Actor: one long-lived task owns the portal session and serves the clipboard.
// ---------------------------------------------------------------------------

enum Cmd {
    /// Own the selection with `text`, inject Ctrl+V, report what happened.
    Deliver {
        text: String,
        reply: async_channel::Sender<DeliveryOutcome>,
    },
    /// Inject Ctrl+V only (whatever currently owns the clipboard serves it).
    Inject { reply: async_channel::Sender<bool> },
    /// Own the selection with `text` without injecting a keystroke.
    SetSelection {
        text: String,
        reply: async_channel::Sender<bool>,
    },
    /// Interactively acquire the RemoteDesktop grant (from Settings).
    AcquireGrant { reply: async_channel::Sender<bool> },
    /// Open the session ahead of time (startup, grant already held) so the
    /// first paste doesn't pay the portal handshake. Never prompts.
    WarmUp,
    /// Close the live session (after the user revokes the permission).
    CloseSession,
}

/// The portal's SelectionTransfer signal stream: (session, mime type, serial).
type TransferStream =
    std::pin::Pin<Box<dyn futures::Stream<Item = (Session<RemoteDesktop>, String, u32)> + Send>>;

struct LiveSession {
    proxy: &'static RemoteDesktop,
    clipboard: &'static Clipboard,
    session: Session<RemoteDesktop>,
    transfers: TransferStream,
    clipboard_enabled: bool,
}

/// The D-Bus proxies are created once and cached for the process lifetime:
/// the SelectionTransfer signal stream borrows the Clipboard proxy, so both
/// must outlive every session that gets recreated on top of them.
async fn proxies() -> Result<(&'static RemoteDesktop, &'static Clipboard), ashpd::Error> {
    static REMOTE: tokio::sync::OnceCell<RemoteDesktop> = tokio::sync::OnceCell::const_new();
    static CLIPBOARD: tokio::sync::OnceCell<Clipboard> = tokio::sync::OnceCell::const_new();
    let remote = REMOTE.get_or_try_init(RemoteDesktop::new).await?;
    let clipboard = CLIPBOARD.get_or_try_init(Clipboard::new).await?;
    Ok((remote, clipboard))
}

fn sender() -> &'static async_channel::Sender<Cmd> {
    static TX: OnceLock<async_channel::Sender<Cmd>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = async_channel::bounded::<Cmd>(16);
        crate::application::tokio_runtime().spawn(actor(rx));
        tx
    })
}

enum Woken {
    Cmd(Cmd),
    Transfer(Option<(Session<RemoteDesktop>, String, u32)>),
    Closed,
}

async fn actor(rx: async_channel::Receiver<Cmd>) {
    let mut live: Option<LiveSession> = None;
    // The bytes served for every SelectionTransfer while we own the selection.
    let mut current: Vec<u8> = Vec::new();
    loop {
        let woken = if let Some(l) = live.as_mut() {
            tokio::select! {
                c = rx.recv() => match c {
                    Ok(c) => Woken::Cmd(c),
                    Err(_) => Woken::Closed,
                },
                t = l.transfers.next() => Woken::Transfer(t),
            }
        } else {
            match rx.recv().await {
                Ok(c) => Woken::Cmd(c),
                Err(_) => Woken::Closed,
            }
        };
        match woken {
            Woken::Closed => break,
            Woken::Transfer(None) => {
                // Signal stream ended: the portal went away. Recreate lazily.
                warn!("RemoteDesktop clipboard stream ended; session dropped");
                live = None;
            }
            Woken::Transfer(Some((sess, _mime, serial))) => {
                if let Some(clipboard) = live.as_ref().map(|l| l.clipboard) {
                    serve_transfer(clipboard, &sess, serial, &current).await;
                }
            }
            Woken::Cmd(cmd) => handle_cmd(cmd, &mut live, &mut current).await,
        }
    }
}

async fn handle_cmd(cmd: Cmd, live: &mut Option<LiveSession>, current: &mut Vec<u8>) {
    match cmd {
        Cmd::Deliver { text, reply } => {
            *current = text.into_bytes();
            let mut outcome = DeliveryOutcome::Unavailable;
            // Two attempts: a stale session (portal restarted underneath us) is
            // dropped and recreated once.
            for _ in 0..2 {
                match ensure_live(live, false).await {
                    Ok(Some(fresh)) => {
                        if !live.as_ref().map(|l| l.clipboard_enabled).unwrap_or(false) {
                            info!("Compositor offers no Clipboard portal — falling back");
                            break;
                        }
                        if fresh {
                            // If an (expired-grant) consent dialog just closed,
                            // give focus a moment to return to the target app.
                            tokio::time::sleep(Duration::from_millis(150)).await;
                        }
                        let l = live.as_mut().expect("ensured above");
                        match deliver_on(l, current).await {
                            Ok(true) => {
                                outcome = DeliveryOutcome::Pasted;
                                break;
                            }
                            Ok(false) => {
                                outcome = DeliveryOutcome::NotConsumed;
                                break;
                            }
                            Err(e) => {
                                warn!("Portal paste failed ({e}); recreating session");
                                close_live(live).await;
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        warn!("RemoteDesktop session error: {e}");
                        close_live(live).await;
                        break;
                    }
                }
            }
            info!("Portal delivery outcome: {outcome:?}");
            let _ = reply.send(outcome).await;
        }
        Cmd::Inject { reply } => {
            let mut ok = false;
            for _ in 0..2 {
                match ensure_live(live, false).await {
                    Ok(Some(_)) => {
                        let (proxy, session) = {
                            let l = live.as_ref().expect("ensured above");
                            (l.proxy, &l.session)
                        };
                        match inject_ctrl_v(proxy, session).await {
                            Ok(()) => {
                                ok = true;
                                break;
                            }
                            Err(e) => {
                                warn!("Portal keystroke failed ({e}); recreating session");
                                close_live(live).await;
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        warn!("RemoteDesktop session error: {e}");
                        close_live(live).await;
                        break;
                    }
                }
            }
            if ok {
                info!("Auto-pasted via RemoteDesktop portal");
            }
            let _ = reply.send(ok).await;
        }
        Cmd::SetSelection { text, reply } => {
            *current = text.into_bytes();
            let mut ok = false;
            for _ in 0..2 {
                match ensure_live(live, false).await {
                    Ok(Some(_)) => {
                        let (clipboard, session, clipboard_enabled) = {
                            let l = live.as_ref().expect("ensured above");
                            (l.clipboard, &l.session, l.clipboard_enabled)
                        };
                        if !clipboard_enabled {
                            break;
                        }
                        match set_selection(clipboard, session).await {
                            Ok(()) => {
                                ok = true;
                                break;
                            }
                            Err(e) => {
                                warn!("Portal SetSelection failed ({e}); recreating session");
                                close_live(live).await;
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        warn!("RemoteDesktop session error: {e}");
                        close_live(live).await;
                        break;
                    }
                }
            }
            let _ = reply.send(ok).await;
        }
        Cmd::AcquireGrant { reply } => {
            let ok = match ensure_live(live, true).await {
                Ok(Some(_)) => true,
                Ok(None) => false,
                Err(e) => {
                    warn!("RemoteDesktop grant request failed: {e}");
                    false
                }
            };
            let _ = reply.send(ok).await;
        }
        Cmd::WarmUp => {
            if let Err(e) = ensure_live(live, false).await {
                warn!("RemoteDesktop warm-up failed: {e}");
            }
        }
        Cmd::CloseSession => {
            close_live(live).await;
        }
    }
}

/// Make sure a live session exists. Returns `Ok(Some(fresh))` with `fresh`
/// telling whether the session was just created, `Ok(None)` when the portal is
/// not an option (no grant in non-interactive mode, or keyboard denied).
async fn ensure_live(
    live: &mut Option<LiveSession>,
    interactive: bool,
) -> Result<Option<bool>, ashpd::Error> {
    if live.is_some() {
        return Ok(Some(false));
    }
    if !interactive && !has_portal_grant() {
        info!("No RemoteDesktop grant yet — portal paste skipped (grant it from Settings)");
        return Ok(None);
    }
    match open_session().await? {
        Some(l) => {
            *live = Some(l);
            Ok(Some(true))
        }
        None => Ok(None),
    }
}

async fn open_session() -> Result<Option<LiveSession>, ashpd::Error> {
    // Ensure the portal can identify this (non-sandboxed) app.
    super::ensure_host_app_registered().await;

    let (proxy, clipboard) = proxies().await?;
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
    clipboard
        .request(&session, RequestClipboardOptions::default())
        .await?;

    let started = match proxy
        .start(&session, None, StartOptions::default())
        .await?
        .response()
    {
        Ok(started) => started,
        Err(e) => {
            // The consent dialog was cancelled/denied. Restore tokens are
            // single-use: the token we sent (if any) was consumed by this
            // failed Start, so keeping the file would make has_portal_grant()
            // lie and every later paste/copy/warm-up re-open the dialog in an
            // endless loop. Forget it — the user can re-grant from Settings.
            warn!("RemoteDesktop start denied/cancelled: {e}");
            forget_restore_token();
            let _ = session.close().await;
            return match e {
                ashpd::Error::Response(_) => Ok(None),
                other => Err(other),
            };
        }
    };

    if !started.devices().contains(DeviceType::Keyboard) {
        warn!("RemoteDesktop session did not grant keyboard access");
        // Same reasoning as the denial above: the grant behind the persisted
        // token no longer covers the keyboard, so the token is dead weight.
        forget_restore_token();
        let _ = session.close().await;
        return Ok(None);
    }
    // Persist the (possibly refreshed) restore token so consent is shown once.
    if let Some(token) = started.restore_token() {
        save_restore_token(token);
    }
    let clipboard_enabled = started.is_clipboard_enabled();

    // Subscribe to transfer requests BEFORE ever advertising a selection so we
    // don't miss the read a target app issues in response to Ctrl+V.
    let transfers = clipboard
        .receive_selection_transfer::<RemoteDesktop>()
        .await?;

    info!("RemoteDesktop session established (clipboard={clipboard_enabled})");
    Ok(Some(LiveSession {
        proxy,
        clipboard,
        session,
        transfers: Box::pin(transfers),
        clipboard_enabled,
    }))
}

async fn close_live(live: &mut Option<LiveSession>) {
    if let Some(l) = live.take() {
        let _ = l.session.close().await;
    }
}

async fn set_selection(
    clipboard: &Clipboard,
    session: &Session<RemoteDesktop>,
) -> Result<(), ashpd::Error> {
    clipboard
        .set_selection(
            session,
            SetSelectionOptions::default()
                .set_mime_types(&["text/plain;charset=utf-8", "text/plain"]),
        )
        .await
}

/// Ctrl down, V down, V up, Ctrl up. Ctrl and 'v' exist in every Latin layout,
/// so this resolves regardless of the transcript's language.
async fn inject_ctrl_v(
    proxy: &RemoteDesktop,
    session: &Session<RemoteDesktop>,
) -> Result<(), ashpd::Error> {
    let opts = NotifyKeyboardKeysymOptions::default;
    proxy
        .notify_keyboard_keysym(session, XK_CONTROL_L, KeyState::Pressed, opts())
        .await?;
    proxy
        .notify_keyboard_keysym(session, XK_V, KeyState::Pressed, opts())
        .await?;
    proxy
        .notify_keyboard_keysym(session, XK_V, KeyState::Released, opts())
        .await?;
    proxy
        .notify_keyboard_keysym(session, XK_CONTROL_L, KeyState::Released, opts())
        .await?;
    Ok(())
}

/// Own the selection, inject Ctrl+V, and serve the target's read. Returns
/// whether at least one SelectionTransfer was actually served. Ctrl+V is
/// deliberately injected exactly once: the caller already waits (event-driven)
/// for focus to leave the panel, and a retry keystroke would paste TWICE into
/// apps that received the first one but read the clipboard slowly.
async fn deliver_on(l: &mut LiveSession, bytes: &[u8]) -> Result<bool, ashpd::Error> {
    set_selection(l.clipboard, &l.session).await?;
    inject_ctrl_v(l.proxy, &l.session).await?;
    Ok(serve_until_quiet(l, bytes, FIRST_READ_TIMEOUT).await)
}

/// Serve SelectionTransfer requests: wait up to `first` for the first read,
/// then only [`QUIET_PERIOD`] for follow-ups. Returns whether any was served.
/// Later reads (clipboard managers, repeat pastes) are handled by the actor's
/// main loop, so this only needs to confirm the immediate paste.
async fn serve_until_quiet(l: &mut LiveSession, bytes: &[u8], first: Duration) -> bool {
    let mut served_any = false;
    loop {
        let wait = if served_any { QUIET_PERIOD } else { first };
        match tokio::time::timeout(wait, l.transfers.next()).await {
            Ok(Some((sess, _mime, serial))) => {
                if serve_transfer(l.clipboard, &sess, serial, bytes).await {
                    served_any = true;
                }
            }
            Ok(None) => break, // signal stream ended — actor will recreate
            Err(_) => break,   // window elapsed
        }
    }
    served_any
}

/// Answer one SelectionTransfer with `bytes`. Returns whether the write went
/// through.
async fn serve_transfer(
    clipboard: &Clipboard,
    sess: &Session<RemoteDesktop>,
    serial: u32,
    bytes: &[u8],
) -> bool {
    use std::io::Write;
    match clipboard.selection_write(sess, serial).await {
        Ok(zfd) => {
            let std_fd: std::os::fd::OwnedFd = zfd.into();
            let mut f = std::fs::File::from(std_fd);
            let ok = f.write_all(bytes).and_then(|_| f.flush()).is_ok();
            drop(f); // closes the pipe write end
            let _ = clipboard.selection_write_done(sess, serial, ok).await;
            ok
        }
        Err(e) => {
            warn!("selection_write failed: {e}");
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Public API (async entry points run on the Tokio runtime).
// ---------------------------------------------------------------------------

/// Deliver `text` into the currently-focused app: the portal owns the system
/// selection (focus-independent) and a Ctrl+V is injected. The caller must
/// ensure the target app holds keyboard focus (i.e. hide the panel first).
/// See [`DeliveryOutcome`] for the honest result semantics.
#[tracing::instrument(name = "portal.deliver_text", skip(text))]
pub async fn deliver_text_via_portal(text: String) -> DeliveryOutcome {
    let (tx, rx) = async_channel::bounded(1);
    if sender()
        .send(Cmd::Deliver { text, reply: tx })
        .await
        .is_err()
    {
        return DeliveryOutcome::Unavailable;
    }
    rx.recv().await.unwrap_or(DeliveryOutcome::Unavailable)
}

/// Own the system selection with `text` WITHOUT injecting a keystroke — a
/// focus-independent "copy to clipboard". Returns whether the portal now owns
/// the selection; on `false` the caller must set the GTK clipboard itself
/// (which requires its surface to be focused on Wayland).
#[tracing::instrument(name = "portal.set_clipboard", skip(text))]
pub async fn set_clipboard_via_portal(text: String) -> bool {
    let (tx, rx) = async_channel::bounded(1);
    if sender()
        .send(Cmd::SetSelection { text, reply: tx })
        .await
        .is_err()
    {
        return false;
    }
    rx.recv().await.unwrap_or(false)
}

/// Attempt to paste the current clipboard contents into the focused app.
/// Returns `true` only if a paste keystroke was actually injected. Never
/// panics; on any failure the caller's clipboard text remains the fallback.
#[tracing::instrument(name = "portal.autopaste")]
pub async fn try_autopaste() -> bool {
    let (tx, rx) = async_channel::bounded(1);
    let via_portal =
        sender().send(Cmd::Inject { reply: tx }).await.is_ok() && rx.recv().await.unwrap_or(false);
    if via_portal {
        return true;
    }
    // Portal didn't work — fall back to ydotool if it's installed. Run the
    // synchronous process wait off the async worker thread.
    if ydotool_available() {
        return tokio::task::spawn_blocking(paste_via_ydotool)
            .await
            .unwrap_or(false);
    }
    info!("Auto-paste unavailable — text remains on the clipboard");
    false
}

/// Open the portal session ahead of time when the grant is already held, so
/// the first paste of the run skips the session handshake. Never prompts.
pub fn warm_up() {
    if has_portal_grant() {
        let _ = sender().try_send(Cmd::WarmUp);
    }
}

/// Interactively request the RemoteDesktop grant (shows the consent dialog if
/// not yet granted). Called from Settings when the user enables auto-paste so
/// the dialog never interrupts a paste. Returns whether the grant is held.
pub async fn acquire_permission_interactive() -> bool {
    let (tx, rx) = async_channel::bounded(1);
    if sender()
        .send(Cmd::AcquireGrant { reply: tx })
        .await
        .is_err()
    {
        return false;
    }
    rx.recv().await.unwrap_or(false)
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

/// Quietly delete a restore token that turned out to be dead (denied/expired
/// grant), so `has_portal_grant()` stops claiming a grant that no longer works.
fn forget_restore_token() {
    match std::fs::remove_file(restore_token_path()) {
        Ok(()) => info!("Forgot stale RemoteDesktop restore token"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => warn!("Failed to remove stale restore token: {e}"),
    }
}

/// Delete the persisted RemoteDesktop restore token and close any live portal
/// session. After this, the next auto-paste falls back to the clipboard until
/// the permission is granted again from Settings. Returns `true` if a token
/// was removed (or none existed); `false` only on an unexpected I/O error.
pub fn revoke_restore_token() -> bool {
    // Best-effort: also tear down the live session so the grant stops working
    // immediately, not just after restart.
    let _ = sender().try_send(Cmd::CloseSession);
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
