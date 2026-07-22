// Speech to Text - Local HTTP API server
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Opt-in, localhost-only HTTP API so other local apps (and local web pages)
//! can POST audio files for transcription and optional translation.
//!
//! Security posture (see also `THREATMODEL.md`):
//! - Binds `127.0.0.1` ONLY — never `0.0.0.0`. Enforced in code, not config.
//! - Requires a bearer token by default (random, stored in the system keyring,
//!   never written to the config or any log).
//! - Rejects non-loopback `Host` headers (DNS-rebinding guard).
//! - Caps the upload size before decoding, bounds the inference queue (429 on
//!   overflow), and times out a stuck request.
//! - Never logs the transcript, the audio, the temp path, or the token.
//! - Does NOT persist results to the in-app History.

mod handlers;
mod server;

use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::sync::atomic::AtomicBool;

use bytes::Bytes;
use http_body_util::Full;
use hyper::Response;

use crate::recording::{DictationOutcome, DictationParams};
use crate::transcription::{ModelCatalog, TranscriptionEngine};

/// Shared response type: every endpoint returns a buffered JSON body.
type Resp = Response<Full<Bytes>>;

/// Max bytes accepted for one upload before decoding (256 MiB). Far below the
/// 2 GiB file-drop ceiling — API audio is modest and we buffer it in memory.
pub const MAX_API_UPLOAD_BYTES: usize = 256 * 1024 * 1024;

/// Overall per-request timeout (10 min): covers a cold model load plus a long
/// clip, without letting a stuck client pin a worker forever.
pub const REQUEST_TIMEOUT_SECS: u64 = 600;

/// Maximum idle time between request-body frames. Local uploads should make
/// steady progress; this prevents slow clients from holding admission slots.
pub const BODY_FRAME_TIMEOUT_SECS: u64 = 30;

/// Inference queue depth. The single worker runs one job at a time (matching the
/// engine Mutex); excess concurrent requests get 429.
const JOB_QUEUE_DEPTH: usize = 2;

/// Limit expensive body buffering before requests reach the inference queue.
const MAX_CONCURRENT_UPLOADS: usize = 2;

/// Bound keep-alive connections and their Tokio tasks.
const MAX_API_CONNECTIONS: usize = 16;

/// One transcription job handed to the inference worker.
struct Job {
    /// Uploaded audio, written to a private temp file (unlinked when dropped).
    audio: tempfile::TempPath,
    params: DictationParams,
    reply: async_channel::Sender<Result<DictationOutcome, String>>,
    cancelled: Arc<AtomicBool>,
}

/// Shared server state (cheap to clone; one per connection task).
#[derive(Clone)]
struct ServerState {
    /// Bearer token required on requests, or `None` when token auth is disabled.
    token: Option<Arc<String>>,
    /// Bounded queue to the inference worker.
    jobs: async_channel::Sender<Job>,
    /// Admission control acquired before an upload body is read into memory.
    uploads: Arc<tokio::sync::Semaphore>,
    /// Connection limit shared by the accept loop.
    connections: Arc<tokio::sync::Semaphore>,
    /// Model catalog (for `GET /v1/models`).
    catalog: Arc<ModelCatalog>,
}

/// Handle to a running server. Dropping it (or calling [`ApiServerHandle::stop`])
/// signals a graceful shutdown and closes the port.
pub struct ApiServerHandle {
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    port: u16,
}

impl ApiServerHandle {
    /// The port the server is bound to.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Stop the server (idempotent).
    pub fn stop(mut self) {
        self.signal_stop();
    }

    fn signal_stop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for ApiServerHandle {
    fn drop(&mut self) {
        self.signal_stop();
    }
}

/// Generate a fresh 256-bit token (two UUIDv4s, hex) suitable as a bearer token.
pub fn generate_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// Constant-time byte comparison (no early-exit timing leak). The length check
/// can leak length, which is fine for fixed-length tokens.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Start the API server bound to `127.0.0.1:port`. Binds synchronously (so a
/// "port in use" error can be surfaced to the UI), then spawns the hyper accept
/// loop on the global Tokio runtime and a dedicated OS thread for blocking
/// inference. `token` is the bearer token to require, or `None` to disable auth.
pub fn start(
    engine: Arc<Mutex<Option<TranscriptionEngine>>>,
    catalog: Arc<ModelCatalog>,
    port: u16,
    token: Option<String>,
) -> Result<ApiServerHandle, String> {
    let std_listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port))
        .map_err(|e| format!("Could not bind 127.0.0.1:{port}: {e}"))?;
    std_listener
        .set_nonblocking(true)
        .map_err(|e| format!("Could not configure listener: {e}"))?;

    let (job_tx, job_rx) = async_channel::bounded::<Job>(JOB_QUEUE_DEPTH);
    std::thread::Builder::new()
        .name("api-transcribe".into())
        .spawn(move || worker_loop(engine, job_rx))
        .map_err(|e| format!("Could not start worker thread: {e}"))?;

    let state = ServerState {
        token: token.map(Arc::new),
        jobs: job_tx,
        uploads: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_UPLOADS)),
        connections: Arc::new(tokio::sync::Semaphore::new(MAX_API_CONNECTIONS)),
        catalog,
    };

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    crate::application::tokio_runtime().spawn(server::serve(std_listener, state, shutdown_rx));

    tracing::info!("API server listening on 127.0.0.1:{port}");
    Ok(ApiServerHandle {
        shutdown: Some(shutdown_tx),
        port,
    })
}

/// Dedicated inference worker: pulls jobs off the bounded queue and runs the
/// blocking decode + transcription, serialized on the shared engine. Exits when
/// the queue's senders are all dropped (i.e. the server stopped).
fn worker_loop(
    engine: Arc<Mutex<Option<TranscriptionEngine>>>,
    jobs: async_channel::Receiver<Job>,
) {
    while let Ok(job) = jobs.recv_blocking() {
        if job.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            continue;
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_job(&engine, &job)
        }))
        .unwrap_or_else(|_| Err("Transcription worker recovered from an invalid request.".into()));
        let _ = job.reply.send_blocking(result);
        // `job.audio` (TempPath) drops here → the temp file is unlinked.
    }
    tracing::debug!("API inference worker stopped");
}

fn run_job(
    engine: &Arc<Mutex<Option<TranscriptionEngine>>>,
    job: &Job,
) -> Result<DictationOutcome, String> {
    let config = crate::config::AppConfig::load();
    // Whisper needs a loaded engine; Cohere/Qwen run via CLI in run_transcription.
    if job.params.backend == "whisper" {
        crate::recording::ensure_engine_loaded(engine, &config)?;
    }
    let audio = crate::audio::file_decoder::decode_audio_file(job.audio.as_ref())
        .map_err(|e| e.user_message())?;
    let duration = audio.len() as f32 / crate::limits::SAMPLE_RATE as f32;
    crate::recording::run_transcription(engine, &audio, &job.params, duration)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn constant_time_eq_matches_only_equal_slices() {
        assert!(constant_time_eq(b"abc123", b"abc123"));
        assert!(!constant_time_eq(b"abc123", b"abc124"));
        assert!(!constant_time_eq(b"abc", b"abcd")); // different length
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn generated_token_is_256_bit_hex() {
        let t = generate_token();
        assert_eq!(t.len(), 64, "two UUIDv4 simple hex = 64 chars");
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(generate_token(), generate_token());
    }

    /// Drive the real hyper server over a real socket. Exercises auth, routing,
    /// CORS, the Host (DNS-rebinding) guard, and the no-audio guard — none of
    /// which need a loaded model or the GUI.
    #[test]
    fn http_layer_end_to_end() {
        let engine = Arc::new(Mutex::new(None));
        let catalog = Arc::new(ModelCatalog::new());
        let port = 17_787;
        let token = "secrettoken123".to_string();
        let handle = start(engine, catalog, port, Some(token.clone()))
            .expect("server should bind");

        // Use a minimal raw TCP client (below) for full control over headers.

        // 1. Health needs no auth and returns JSON with "ok".
        let (status, _h, body) = raw_get(port, "/v1/health", &[]);
        assert_eq!(status, 200, "health body: {body}");
        assert!(body.contains("\"status\":\"ok\""), "health body: {body}");

        // 2. Protected route without a token → 401.
        let (status, _h, _b) = raw_get(port, "/v1/models", &[]);
        assert_eq!(status, 401);

        // 3. Wrong token → 401.
        let (status, _h, _b) =
            raw_get(port, "/v1/models", &[("Authorization", "Bearer nope")]);
        assert_eq!(status, 401);

        // 4. Correct token → 200.
        let auth = format!("Bearer {token}");
        let (status, _h, body) = raw_get(port, "/v1/models", &[("Authorization", &auth)]);
        assert_eq!(status, 200, "models body: {body}");

        // 5. Unknown route → 404.
        let (status, _h, _b) = raw_get(port, "/v1/nope", &[("Authorization", &auth)]);
        assert_eq!(status, 404);

        // 6. DNS-rebinding guard: a non-loopback Host is rejected with 400.
        let (status, _h, _b) = raw_request(port, "GET", "/v1/health", &[("Host", "evil.example.com")], b"");
        assert_eq!(status, 400);

        // 7. CORS preflight reflects the Origin.
        let (status, headers, _b) = raw_request(
            port,
            "OPTIONS",
            "/v1/transcribe",
            &[("Origin", "http://localhost:3000")],
            b"",
        );
        assert_eq!(status, 204);
        assert!(
            headers.to_lowercase().contains("access-control-allow-origin: http://localhost:3000"),
            "preflight headers: {headers}"
        );

        // 8. transcribe with a valid token but an empty body → 422 (no audio),
        //    proving auth passed and the body path ran without touching a model.
        let (status, _h, body) =
            raw_request(port, "POST", "/v1/transcribe", &[("Authorization", &auth)], b"");
        assert_eq!(status, 422, "transcribe body: {body}");
        assert!(body.contains("no_audio"), "transcribe body: {body}");

        drop(handle); // signals shutdown
    }

    /// Minimal raw HTTP/1.1 client so the test fully controls headers (including
    /// Host) without depending on a higher-level client's behaviour.
    fn raw_get(port: u16, path: &str, headers: &[(&str, &str)]) -> (u16, String, String) {
        raw_request(port, "GET", path, headers, b"")
    }

    fn raw_request(
        port: u16,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> (u16, String, String) {
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port))
            .expect("connect to API server");
        let mut have_host = false;
        let mut req = format!("{method} {path} HTTP/1.1\r\n");
        for (k, v) in headers {
            if k.eq_ignore_ascii_case("host") {
                have_host = true;
            }
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        if !have_host {
            req.push_str(&format!("Host: 127.0.0.1:{port}\r\n"));
        }
        req.push_str(&format!("Content-Length: {}\r\n", body.len()));
        req.push_str("Connection: close\r\n\r\n");
        stream.write_all(req.as_bytes()).unwrap();
        stream.write_all(body).unwrap();
        stream.flush().unwrap();

        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).unwrap();
        let text = String::from_utf8_lossy(&raw).to_string();
        let (head, body) = text.split_once("\r\n\r\n").unwrap_or((text.as_str(), ""));
        let status = head
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(0);
        (status, head.to_string(), body.to_string())
    }
}
