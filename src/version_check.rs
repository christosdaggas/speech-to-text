// Speech to Text - Version Checker
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Queries the GitHub Releases API once at startup to determine whether a
//! newer version of the application is available.
//!
//! On any network error or parse failure the check silently returns `None`
//! so the UI simply shows nothing.

use serde::Deserialize;
use tracing::debug;

const GITHUB_OWNER: &str = "christosdaggas";
const GITHUB_REPO: &str = "speech-to-text";

/// Result of a successful version check.
#[derive(Debug, Clone)]
pub struct UpdateInfo {
    /// Latest version string from GitHub (e.g. "1.2.0").
    pub latest_version: String,
}

/// Subset of the GitHub Release API response we care about.
#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
}

/// Check GitHub for the latest release.
///
/// Returns `Some(UpdateInfo)` if a newer version exists,
/// `None` if the local version is current or on any error.
pub async fn check_for_update(current_version: &str) -> Option<UpdateInfo> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/releases/latest",
        GITHUB_OWNER, GITHUB_REPO
    );

    debug!("Checking for updates at {}", url);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .user_agent(format!("SpeechToText/{}", current_version))
        .build()
        .ok()?;

    let response = match client.get(&url).send().await {
        Ok(resp) => resp,
        Err(e) => {
            debug!("Update check HTTP request failed: {}", e);
            return None;
        }
    };

    if !response.status().is_success() {
        debug!("Update check got HTTP {}", response.status());
        return None;
    }

    let release: GitHubRelease = match response.json().await {
        Ok(r) => r,
        Err(e) => {
            debug!("Failed to parse GitHub release JSON: {}", e);
            return None;
        }
    };

    let latest = release
        .tag_name
        .trim_start_matches('v')
        .trim_start_matches('V')
        .to_string();

    debug!("Update check: local={}, remote={}", current_version, latest);

    if is_newer(&latest, current_version) {
        Some(UpdateInfo {
            latest_version: latest,
        })
    } else {
        debug!("Application is up to date");
        None
    }
}

/// Compare two semver-ish version strings.
/// Returns true if `remote` is strictly newer than `local`.
fn is_newer(remote: &str, local: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.split('.')
            .map(|part| {
                // Drop any pre-release/build suffix (e.g. "0-beta", "1+build").
                let numeric: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
                numeric.parse::<u64>().unwrap_or(0)
            })
            .collect()
    };

    let r = parse(remote);
    let l = parse(local);

    let max_len = r.len().max(l.len());
    for i in 0..max_len {
        let rv = r.get(i).copied().unwrap_or(0);
        let lv = l.get(i).copied().unwrap_or(0);
        if rv > lv {
            return true;
        }
        if rv < lv {
            return false;
        }
    }
    false
}
