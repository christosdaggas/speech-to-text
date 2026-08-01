// Speech to Text - Warm sidecar server manager
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Keeps ONE warm sidecar server process alive for the CLI-based backends
//! (Qwen `asr-server`, Cohere `transcribe-server`), so multi-GB model weights
//! load once per session instead of on every dictation — the difference
//! between tens of seconds of cold start per hotkey press and inference-only
//! latency.
//!
//! Both upstream runtimes document the same OpenAI-compatible protocol:
//! `POST /v1/audio/transcriptions` (multipart: `file`, optional `language`,
//! `response_format`) and `GET /health`. The server binds `127.0.0.1` on an
//! ephemeral port picked by us. Every failure here is soft: callers fall back
//! to the proven one-shot CLI path.
//!
//! The child is spawned with `PR_SET_PDEATHSIG(SIGKILL)` so it can never
//! outlive the app (even on a crash), and [`shutdown_all`] tears it down
//! explicitly on backend switches and app exit.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tracing::{info, warn};

use crate::error::{AppError, AppResult};
use crate::transcription::engine::{TranscriptionResult, TranscriptionSegment};

/// How long to wait for `/health` after a spawn: the first answer requires the
/// full weight load (tens of seconds for the larger models).
const STARTUP_TIMEOUT: Duration = Duration::from_secs(180);
/// Poll interval while waiting for readiness.
const STARTUP_POLL: Duration = Duration::from_millis(250);
/// Per-request cap for one transcription (matches the generous CLI deadline —
/// long clips on slow CPUs are legitimate).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(600);

/// What to run and with which model. Servers are keyed by (binary, model_dir):
/// a model or backend change tears the old server down.
pub struct ServerSpec {
    pub binary: PathBuf,
    pub model_dir: PathBuf,
    /// Directories prepended to `LD_LIBRARY_PATH` (libtorch lives beside the
    /// binary, not on the system).
    pub ld_dirs: Vec<PathBuf>,
}

struct Running {
    child: Child,
    port: u16,
    binary: PathBuf,
    model_dir: PathBuf,
}

impl Running {
    fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn kill(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The single warm server slot. LOCKING RULE: this mutex is only ever held for
/// the take/insert swap — NEVER across a spawn (minutes of weight loading) or
/// an HTTP request. `shutdown_all` runs on the GTK thread and must never wait
/// behind a worker.
static RUNNING: Mutex<Option<Running>> = Mutex::new(None);

/// Serializes transcriptions across the GUI and API workers (the servers
/// process one request at a time anyway). Held across spawn + request, but
/// ONLY worker threads ever take it — the GTK thread does not.
static REQUEST_LOCK: Mutex<()> = Mutex::new(());

/// Bumped by [`shutdown_all`]. A worker that finished spawning compares the
/// epoch it started with: if a shutdown (backend switch, app exit) happened in
/// between, the fresh server is killed instead of installed.
static EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn epoch() -> u64 {
    EPOCH.load(std::sync::atomic::Ordering::SeqCst)
}

fn take_running() -> Option<Running> {
    RUNNING.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// Transcribe `wav` (a complete 16-bit WAV file) via the warm server for
/// `spec`, starting or replacing the server as needed. `language` is passed
/// through verbatim (each backend maps its own accepted form). Any error means
/// "use the CLI fallback" — nothing here is fatal.
pub fn transcribe_via_server(
    spec: &ServerSpec,
    wav: Vec<u8>,
    language: Option<&str>,
) -> AppResult<TranscriptionResult> {
    let _serialize = REQUEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let started_epoch = epoch();

    // Reuse the running server only if it matches and still lives.
    let mut current = take_running();
    let reusable = current
        .as_mut()
        .is_some_and(|r| r.binary == spec.binary && r.model_dir == spec.model_dir && r.alive());
    if !reusable {
        if let Some(old) = current.take() {
            info!("Replacing sidecar server ({:?})", old.binary);
            old.kill();
        }
        current = Some(spawn_server(spec)?);
    }
    let port = install_running(current.expect("ensured above"), started_epoch)?;

    match request_transcription(port, &wav, language) {
        Ok(result) => Ok(result),
        Err(RequestError::Fatal(e)) => Err(AppError::Transcription(e)),
        Err(RequestError::Retryable(e)) => {
            // One respawn-and-retry, but only for connection-level failures
            // (the server died — OOM-killed under memory pressure is the
            // realistic case). Timeouts and malformed responses must NOT
            // re-send the same clip to a freshly reloaded server: a clip that
            // legitimately needs longer than the deadline would just time out
            // again after paying another full weight load.
            warn!("Sidecar server request failed ({e}); respawning once");
            if epoch() != started_epoch {
                return Err(AppError::Transcription(
                    "Sidecar server was shut down (backend switched)".into(),
                ));
            }
            if let Some(old) = take_running() {
                old.kill();
            }
            let fresh = spawn_server(spec)?;
            let port = install_running(fresh, started_epoch)?;
            request_transcription(port, &wav, language).map_err(|e| match e {
                RequestError::Fatal(m) | RequestError::Retryable(m) => AppError::Transcription(m),
            })
        }
    }
}

/// Install a freshly spawned/reused server into the shared slot — unless a
/// shutdown happened while we were starting it, in which case the newcomer is
/// killed so nothing leaks past a backend switch or app exit.
///
/// The epoch check and the insert form ONE critical section: `shutdown_all`
/// does bump-then-take, so either the bump lands before this check (we refuse
/// and kill the newcomer) or the shutdown's take serializes after our insert
/// and reaps it — there is no window where a live server slips past both.
fn install_running(running: Running, started_epoch: u64) -> AppResult<u16> {
    let port = running.port;
    let mut guard = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    if epoch() != started_epoch {
        drop(guard);
        running.kill();
        return Err(AppError::Transcription(
            "Sidecar server was shut down during startup (backend switched)".into(),
        ));
    }
    *guard = Some(running);
    Ok(port)
}

/// Stop any running sidecar server (backend switch, app shutdown). Safe to
/// call from the GTK thread: the slot lock is only held for the take, and
/// killing an already-signalled child reaps immediately.
pub fn shutdown_all() {
    EPOCH.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    if let Some(old) = take_running() {
        info!("Stopping sidecar server");
        old.kill();
    }
}

fn spawn_server(spec: &ServerSpec) -> AppResult<Running> {
    // Pick an ephemeral port. The tiny bind→drop→reuse race is acceptable:
    // failure lands in the health wait below and the caller falls back to CLI.
    let port = std::net::TcpListener::bind(("127.0.0.1", 0))
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| AppError::Transcription(format!("No free port for sidecar server: {e}")))?;

    let mut ld = spec
        .ld_dirs
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(":");
    if let Ok(existing) = std::env::var("LD_LIBRARY_PATH") {
        ld = format!("{ld}:{existing}");
    }

    let mut cmd = Command::new(&spec.binary);
    cmd.arg("--model-dir")
        .arg(&spec.model_dir)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .env("LD_LIBRARY_PATH", ld)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    {
        use std::os::unix::process::CommandExt;
        // The server must never outlive the app: if we crash without running
        // shutdown_all, the kernel reaps it for us.
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                Ok(())
            });
        }
    }
    let mut child = spawn_on_dedicated_thread(cmd)
        .map_err(|e| AppError::Transcription(format!("Failed to start sidecar server: {e}")))?;

    info!(
        "Sidecar server starting on 127.0.0.1:{port} ({:?}, model {:?})",
        spec.binary, spec.model_dir
    );

    // Wait for /health — the first response comes only after the weights load.
    let health_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|e| AppError::Transcription(format!("HTTP client build failed: {e}")))?;
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(AppError::Transcription(format!(
                "Sidecar server exited during startup ({status})"
            )));
        }
        if health_ok(&health_client, port) {
            info!("Sidecar server ready on port {port}");
            return Ok(Running {
                child,
                port,
                binary: spec.binary.clone(),
                model_dir: spec.model_dir.clone(),
            });
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(AppError::Transcription(
                "Sidecar server did not become ready in time".into(),
            ));
        }
        std::thread::sleep(STARTUP_POLL);
    }
}

/// Spawn `cmd` from a dedicated, app-lifetime thread. `PR_SET_PDEATHSIG` binds
/// the child's fate to the SPAWNING THREAD (prctl(2)), not the process — a
/// server spawned from an ephemeral worker (a one-off file-transcription
/// thread, or the API worker being restarted from Settings) would be silently
/// SIGKILLed the moment that thread exited, defeating the warm cache.
fn spawn_on_dedicated_thread(cmd: Command) -> std::io::Result<Child> {
    type SpawnReq = (Command, std::sync::mpsc::Sender<std::io::Result<Child>>);
    static TX: std::sync::OnceLock<std::sync::mpsc::Sender<SpawnReq>> = std::sync::OnceLock::new();
    let tx = TX.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<SpawnReq>();
        std::thread::Builder::new()
            .name("sidecar-spawn".into())
            .spawn(move || {
                while let Ok((mut cmd, reply)) = rx.recv() {
                    let _ = reply.send(cmd.spawn());
                }
            })
            .expect("failed to start sidecar-spawn thread");
        tx
    });
    let (rtx, rrx) = std::sync::mpsc::channel();
    tx.send((cmd, rtx))
        .map_err(|_| std::io::Error::other("sidecar spawn thread is gone"))?;
    rrx.recv()
        .map_err(|_| std::io::Error::other("sidecar spawn thread is gone"))?
}

fn health_ok(client: &reqwest::Client, port: u16) -> bool {
    crate::application::tokio_runtime().block_on(async {
        client
            .get(format!("http://127.0.0.1:{port}/health"))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    })
}

/// Whether a failed request should trigger the one respawn-and-retry.
/// Retryable = the server is gone (connection-level failure). Timeouts,
/// HTTP errors and malformed bodies are final: retrying re-pays a full weight
/// load only to fail the same way.
enum RequestError {
    Retryable(String),
    Fatal(String),
}

fn request_transcription(
    port: u16,
    wav: &[u8],
    language: Option<&str>,
) -> Result<TranscriptionResult, RequestError> {
    let response: serde_json::Value = crate::application::tokio_runtime().block_on(async {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| RequestError::Fatal(format!("HTTP client build failed: {e}")))?;
        let mut form = reqwest::multipart::Form::new()
            .part(
                "file",
                reqwest::multipart::Part::bytes(wav.to_vec())
                    .file_name("audio.wav")
                    .mime_str("audio/wav")
                    .map_err(|e| RequestError::Fatal(format!("Multipart build failed: {e}")))?,
            )
            .text("response_format", "verbose_json");
        if let Some(lang) = language {
            form = form.text("language", lang.to_string());
        }
        let resp = client
            .post(format!("http://127.0.0.1:{port}/v1/audio/transcriptions"))
            .multipart(form)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    RequestError::Fatal(format!("Sidecar request timed out: {e}"))
                } else {
                    RequestError::Retryable(format!("Sidecar request failed: {e}"))
                }
            })?
            .error_for_status()
            .map_err(|e| RequestError::Fatal(format!("Sidecar returned an error: {e}")))?;
        resp.json::<serde_json::Value>()
            .await
            .map_err(|e| RequestError::Fatal(format!("Sidecar response parse failed: {e}")))
    })?;

    let text = response["text"].as_str().unwrap_or_default().to_string();
    // verbose_json gives the clip duration — synthesize one segment so SRT
    // export has at least whole-clip timing, matching the CLI path's fidelity.
    let duration_ms = (response["duration"].as_f64().unwrap_or(0.0) * 1000.0) as i64;
    let segments = if text.trim().is_empty() {
        Vec::new()
    } else {
        vec![TranscriptionSegment {
            start_ms: Some(0),
            end_ms: (duration_ms > 0).then_some(duration_ms),
            text: text.clone(),
            confidence: None,
        }]
    };
    Ok(TranscriptionResult {
        segments,
        text,
        average_confidence: None,
        detected_language: None,
    })
}
