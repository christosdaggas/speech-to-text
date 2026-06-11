// Speech to Text - API server: accept loop, routing, auth, CORS
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! The hyper accept loop, the request router, and the cross-cutting checks
//! (Host/DNS-rebinding guard, bearer-token auth, CORS). Endpoint bodies live in
//! [`super::handlers`].

use std::convert::Infallible;

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::header;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;

use super::{handlers, Resp, ServerState};

/// Run the accept loop until the shutdown signal fires. Each connection is
/// served on its own task; the connection future is dropped on shutdown.
pub(super) async fn serve(
    std_listener: std::net::TcpListener,
    state: ServerState,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) {
    let listener = match tokio::net::TcpListener::from_std(std_listener) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("API server: failed to adopt listener: {e}");
            return;
        }
    };

    loop {
        tokio::select! {
            _ = &mut shutdown_rx => {
                tracing::info!("API server shutting down");
                break;
            }
            accepted = listener.accept() => {
                let (stream, _peer) = match accepted {
                    Ok(s) => s,
                    Err(e) => { tracing::warn!("API accept error: {e}"); continue; }
                };
                let io = TokioIo::new(stream);
                let state = state.clone();
                tokio::task::spawn(async move {
                    let svc = service_fn(move |req| {
                        let state = state.clone();
                        async move { Ok::<_, Infallible>(handle(req, state).await) }
                    });
                    if let Err(e) = hyper::server::conn::http1::Builder::new()
                        .serve_connection(io, svc)
                        .await
                    {
                        tracing::debug!("API connection error: {e}");
                    }
                });
            }
        }
    }
}

/// Top-level request handler: Host guard → CORS preflight → route + auth.
async fn handle(req: Request<Incoming>, state: ServerState) -> Resp {
    // DNS-rebinding guard: only serve loopback Host headers. A page tricked into
    // resolving evil.com → 127.0.0.1 sends Host: evil.com, which we reject.
    if !host_is_loopback(&req) {
        return json_error(StatusCode::BAD_REQUEST, "bad_host", "Invalid Host header");
    }

    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // CORS preflight is unauthenticated by spec.
    if req.method() == Method::OPTIONS {
        return with_cors(preflight(), origin.as_deref());
    }

    let method = req.method().clone();
    let path = req.uri().path().to_string();

    let resp = match (&method, path.as_str()) {
        (&Method::GET, "/v1/health") => handlers::health(),
        (&Method::GET, "/v1/models") => match require_auth(&req, &state) {
            Some(deny) => deny,
            None => handlers::models(&state),
        },
        (&Method::POST, "/v1/transcribe") => match require_auth(&req, &state) {
            Some(deny) => deny,
            None => handlers::transcribe(req, &state).await,
        },
        (&Method::POST, "/v1/translate") => match require_auth(&req, &state) {
            Some(deny) => deny,
            None => handlers::translate(req).await,
        },
        (&Method::GET, _) | (&Method::POST, _) => {
            json_error(StatusCode::NOT_FOUND, "not_found", "Unknown endpoint")
        }
        _ => json_error(
            StatusCode::METHOD_NOT_ALLOWED,
            "method_not_allowed",
            "Method not allowed",
        ),
    };

    with_cors(resp, origin.as_deref())
}

/// True when the `Host` header names a loopback address (or is absent).
fn host_is_loopback(req: &Request<Incoming>) -> bool {
    let Some(host) = req.headers().get(header::HOST).and_then(|v| v.to_str().ok()) else {
        // No Host (HTTP/1.0): we are bound to 127.0.0.1 regardless.
        return true;
    };
    // Strip the port; IPv6 hosts arrive as "[::1]:port".
    let hostname = host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host);
    matches!(hostname, "127.0.0.1" | "localhost" | "[::1]" | "::1")
}

/// Returns `Some(401)` when auth is required and the bearer token is missing or
/// wrong, or `None` when the request may proceed.
fn require_auth(req: &Request<Incoming>, state: &ServerState) -> Option<Resp> {
    let token = state.token.as_ref()?; // None ⇒ auth disabled ⇒ allow.
    let provided = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .unwrap_or("");
    if super::constant_time_eq(provided.as_bytes(), token.as_bytes()) {
        None
    } else {
        Some(json_error(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Missing or invalid bearer token",
        ))
    }
}

// ── Response builders (shared with handlers) ─────────────────────────────────

fn json_response(status: StatusCode, body: String) -> Resp {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from(body)))
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::from_static(b"{}"))))
}

/// A JSON error envelope. The message is run through `redact_secrets` so no key
/// or home path can leak to the caller.
pub(super) fn json_error(status: StatusCode, code: &str, message: &str) -> Resp {
    let safe = crate::error::redact_secrets(message);
    let body = serde_json::json!({ "error": { "code": code, "message": safe } }).to_string();
    json_response(status, body)
}

/// A 200 JSON body from any serializable value.
pub(super) fn json_ok<T: serde::Serialize>(value: &T) -> Resp {
    match serde_json::to_string(value) {
        Ok(b) => json_response(StatusCode::OK, b),
        Err(e) => json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "serialize",
            &e.to_string(),
        ),
    }
}

fn preflight() -> Resp {
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(Full::new(Bytes::new()))
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::new())))
}

/// Add CORS headers reflecting the request's Origin. The bearer token (not a
/// cookie) gates real access, so reflecting the origin is safe: a hostile page
/// still lacks the token, and the Host guard blocks DNS rebinding.
fn with_cors(mut resp: Resp, origin: Option<&str>) -> Resp {
    if let Some(origin) = origin {
        let headers = resp.headers_mut();
        if let Ok(value) = origin.parse() {
            headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
        }
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            header::HeaderValue::from_static("GET, POST, OPTIONS"),
        );
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            header::HeaderValue::from_static("authorization, content-type"),
        );
        headers.insert(header::VARY, header::HeaderValue::from_static("Origin"));
    }
    resp
}
