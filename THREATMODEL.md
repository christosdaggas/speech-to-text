# Threat Model

This is a single-user Linux desktop application. It records audio, transcribes
it locally, downloads ML runtimes/models, optionally sends transcript text to a
user-configured LLM, and can type text into other applications. This document
captures the assets, trust boundaries, threats, and the mitigations in place.

## Assets

- **Transcripts** — may contain passwords, personal/medical/legal/financial
  data, source code, or business secrets.
- **Audio** — the raw microphone input.
- **Secrets** — HuggingFace token, LLM API key.
- **Integrity of execution** — downloaded sidecar runtimes are executed; models
  are loaded into native/FFI code.
- **User trust** — the app's privacy promises.

## Trust boundaries

1. **Network ↔ app.** Model/runtime downloads (HuggingFace, GitHub) and the
   optional LLM endpoint. Upstream repos, CDNs, and DNS are *not* fully trusted.
2. **Other local users / processes.** Files on disk, temp files, and predictable
   paths.
3. **App ↔ desktop portals.** GlobalShortcuts and RemoteDesktop (input
   injection) via XDG portals.
4. **App ↔ other applications.** Auto-paste types into whatever window is
   focused.
5. **Local clients ↔ API server.** The opt-in HTTP API (off by default) accepts
   audio from other local processes / local web pages. Bound to `127.0.0.1`
   only; other local processes and browser pages are *not* trusted.

## Threats & mitigations

| # | Threat | Mitigation |
| - | ------ | ---------- |
| T1 | Compromised/replaced **download** (runtime ZIP or model) → local code execution | HTTPS + verification against provider-published hashes (GitHub asset digest, HF LFS oid) **before** extraction/`chmod +x`/model load; fail-closed, partial removed. Release-time hash pinning hook (`verify::pinned`). |
| T2 | Malicious **archive** (path traversal, zip bomb, symlink/special-file escape) | Hardened extractor: `enclosed_name()` + canonical containment, entry-count and total-decompressed-size caps, special-file rejection, lexically-validated symlink targets. |
| T3 | **Secrets** stolen from disk | Stored in the system keyring; never serialized to config (legacy plaintext field migrated then dropped). |
| T4 | **Transcripts** read by other local users | History/config written `0600` inside `0700` dirs, atomically; no `/tmp` fallback. |
| T5 | **Cleartext exfiltration** of transcripts via a misconfigured/hostile LLM URL | `validate_endpoint`: HTTPS required for public hosts; plain HTTP only for loopback/LAN; non-http(s) schemes rejected. One-time consent names the host. |
| T6 | **Accidental** data send to an LLM | LLM off by default; explicit consent on enable; auto-title gated behind auto-apply; no request while disabled. |
| T7 | **Resource exhaustion** (huge file/recording/download/archive/LLM response) | Bounded ceilings: recording seconds, decoded samples, dropped-file size, download bytes, archive entries/bytes, LLM response bytes. |
| T8 | **Secret/transcript leakage in logs or errors** | `redact_secrets` on user-facing errors and secret-adjacent logs; `skip_all` spans; transcript text never logged. |
| T9 | **Predictable temp files** (race/symlink) | `tempfile` exclusive random 0600 files with RAII cleanup. |
| T10 | **Unexpected input injection** into other apps | Auto-paste off by default; in-app consent + OS portal prompt; revocable restore token. |
| T11 | **Supply-chain drift** (vulnerable/unused/odd-licensed deps) | CI runs `cargo audit` + `cargo deny` (advisories, license allowlist, source allowlist); unused deps removed; tokio features minimized. |
| T12 | **Tampered release binary** | Signed `SHA256SUMS` + SBOM; source-build RPM / COPR for reproducibility. |
| T13 | **Local API server** abused (off by default): network exposure, unauthorized callers, DNS-rebinding from a browser, resource exhaustion, secret/transcript leakage | Binds `127.0.0.1` only (hardcoded, no bind-address knob); required 256-bit bearer token (keyring-stored, constant-time compared, never logged); `Host`-header loopback check defeats DNS rebinding; upload-size cap + bounded inference queue (429) + per-request timeout; CORS reflects Origin but the token still gates access; results never persisted to History; errors run through `redact_secrets`. |

## Explicitly out of scope / accepted

- **At-rest encryption of history.** Not provided; protected by Unix perms and
  the assumption of a trusted local account. Users needing more should use
  full-disk/home encryption.
- **A pre-compromised local account or root.** Cannot be defended in-app.
- **Upstream dependency vulnerabilities.** Tracked via `cargo audit`; fixed by
  updating once upstream patches land.
- **Update check metadata** (IP/version to GitHub). Disclosed and disable-able.
- **The HTTP API is off by default.** When the user enables it, it is bound to
  `127.0.0.1` only and gated by a bearer token; a pre-existing malicious local
  process running as the same user is out of scope (see below).

## Residual risks

- Hash pinning of third-party artifacts depends on provider-published digests;
  dynamic HuggingFace model files can't be pinned and rely on allowlist + path
  safety + the provider oid.
- A cloud LLM the user *chooses* to configure necessarily receives their
  transcript text; the app can only warn and gate, not prevent a deliberate
  choice.
