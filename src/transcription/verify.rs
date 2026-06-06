// Speech to Text - Artifact integrity verification (SHA-256)
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! SHA-256 verification for downloaded artifacts (runtime ZIPs, model files).
//!
//! Two layers of trust:
//!  - **Provider-declared** hashes: HuggingFace LFS `oid` (the file's sha256)
//!    and GitHub release-asset `digest`. We verify downloads against these
//!    automatically — catches corrupted/partial/CDN-tampered responses.
//!  - **Pinned** hashes: a small in-repo manifest ([`PINNED`]) of known-good
//!    sha256 for fixed-URL artifacts. When an entry exists we verify against it
//!    and **fail closed**. (Populated at release time; see the maintainer notes.)

use crate::error::{AppError, AppResult};
use sha2::{Digest, Sha256};
use std::path::Path;

/// Lowercase hex SHA-256 of a byte slice.
#[allow(dead_code)] // part of the verify API; used by tests and release tooling
pub fn sha256_hex(bytes: &[u8]) -> String {
    to_hex(&Sha256::digest(bytes))
}

/// Stream a file through SHA-256 (constant memory; safe for multi-GB models).
pub fn sha256_file(path: &Path) -> AppResult<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)
        .map_err(|e| AppError::Transcription(format!("Hashing failed for {}: {e}", path.display())))?;
    Ok(to_hex(&hasher.finalize()))
}

/// Verify a file's SHA-256 against `expected_hex` (case-insensitive). On
/// mismatch the file is removed and an error is returned (fail closed).
pub fn verify_file(path: &Path, expected_hex: &str) -> AppResult<()> {
    let actual = sha256_file(path)?;
    if actual.eq_ignore_ascii_case(expected_hex.trim()) {
        Ok(())
    } else {
        let _ = std::fs::remove_file(path);
        Err(AppError::ModelDownloadFailed(format!(
            "Integrity check failed for {} (expected {}, got {}). The download was rejected and removed.",
            path.display(),
            expected_hex.trim(),
            actual
        )))
    }
}

/// A HuggingFace LFS `oid` looks like a bare 64-hex string (sometimes prefixed
/// with `sha256:`). Normalize to the bare hex, or `None` if it isn't a sha256.
pub fn normalize_hf_oid(oid: &str) -> Option<String> {
    let s = oid.trim().strip_prefix("sha256:").unwrap_or(oid.trim());
    if s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(s.to_ascii_lowercase())
    } else {
        None
    }
}

/// Fetch the published SHA-256 `digest` of a GitHub release asset (the API
/// exposes `assets[].digest` as `"sha256:<hex>"` for recent uploads). Returns
/// `None` if unavailable so callers can decide whether to proceed.
pub async fn github_asset_sha256(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    tag: &str,
    asset: &str,
) -> Option<String> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/releases/tags/{tag}");
    let resp = client
        .get(&url)
        .header("User-Agent", "speech-to-text")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: serde_json::Value = resp.json().await.ok()?;
    for a in json["assets"].as_array()? {
        if a["name"].as_str() == Some(asset) {
            return a["digest"].as_str().and_then(normalize_hf_oid);
        }
    }
    None
}

/// Fetch the published LFS SHA-256 of a file in a HuggingFace repo (via the
/// tree API `lfs.oid`). `None` if the file isn't LFS-tracked or unavailable.
pub async fn hf_lfs_sha256(client: &reqwest::Client, repo: &str, path: &str) -> Option<String> {
    let url = format!("https://huggingface.co/api/models/{repo}/tree/main?recursive=true");
    let resp = client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: serde_json::Value = resp.json().await.ok()?;
    for e in json.as_array()? {
        if e["path"].as_str() == Some(path) {
            return e["lfs"]["oid"].as_str().and_then(normalize_hf_oid);
        }
    }
    None
}

/// Pinned, known-good hashes for fixed-URL artifacts. Empty entries are filled
/// at release time (see `scripts/` / SECURITY.md); when an artifact is listed
/// here verification is mandatory and fails closed.
#[allow(dead_code)] // populated at release time; verification fails closed when set
pub fn pinned(_key: &str) -> Option<&'static str> {
    // Intentionally empty for now: provider-declared hashes (HF lfs oid /
    // GitHub asset digest) are verified automatically at download time. Release
    // hardening pins the runtime ZIPs here once their digests are recorded.
    None
}

fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{:02x}", b);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_of_known_input() {
        // SHA-256("abc")
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn verify_roundtrip(/* uses a temp file */) {
        let dir = std::env::temp_dir().join(format!("stt-verify-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("f.bin");
        std::fs::write(&p, b"hello").unwrap();
        let h = sha256_hex(b"hello");
        assert!(verify_file(&p, &h).is_ok());
        // Recreate (verify_file removes on mismatch) and check rejection.
        std::fs::write(&p, b"hello").unwrap();
        assert!(verify_file(&p, &"0".repeat(64)).is_err());
        assert!(!p.exists(), "mismatched file should be removed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn normalize_oid_forms() {
        let hex = "a".repeat(64);
        assert_eq!(normalize_hf_oid(&hex).as_deref(), Some(hex.as_str()));
        assert_eq!(normalize_hf_oid(&format!("sha256:{hex}")).as_deref(), Some(hex.as_str()));
        assert!(normalize_hf_oid("notahash").is_none());
        assert!(normalize_hf_oid("abc").is_none());
    }
}
