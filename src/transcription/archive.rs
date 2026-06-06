// Speech to Text - Safe ZIP extraction
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Hardened ZIP extraction for downloaded runtime archives. Uses the zip
//! crate's `enclosed_name()` for path-traversal safety, enforces entry-count
//! and total-decompressed-size caps (zip-bomb guard), rejects special files,
//! and validates symlink targets stay inside the destination (libtorch ships
//! legitimate symlinks, so we permit contained ones rather than failing).

use crate::error::{AppError, AppResult};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// Max entries in a runtime archive.
const MAX_ENTRIES: usize = 8192;
/// Max total decompressed bytes (zip-bomb guard). Runtimes bundle libtorch, so
/// allow a generous ceiling but still bounded.
const MAX_TOTAL_BYTES: u64 = 6 * 1024 * 1024 * 1024; // 6 GiB

/// Extract `zip_path` into `dest` safely. Caller should verify the archive hash
/// (see `verify`) before calling this.
pub fn safe_extract_zip(zip_path: &Path, dest: &Path) -> AppResult<()> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| AppError::ModelDownloadFailed(format!("Failed to open zip: {e}")))?;

    if archive.len() > MAX_ENTRIES {
        return Err(AppError::ModelDownloadFailed(format!(
            "Archive has too many entries ({})",
            archive.len()
        )));
    }
    std::fs::create_dir_all(dest)?;

    let mut total: u64 = 0;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| AppError::ModelDownloadFailed(format!("Zip read error: {e}")))?;

        // Path-traversal-safe relative name (rejects absolute/`..`).
        let Some(rel) = entry.enclosed_name() else {
            return Err(AppError::ModelDownloadFailed(format!(
                "Unsafe path in archive: {}",
                entry.name()
            )));
        };
        let out = dest.join(&rel);
        if !out.starts_with(dest) {
            return Err(AppError::ModelDownloadFailed(format!(
                "Archive entry escapes destination: {}",
                entry.name()
            )));
        }

        let mode = entry.unix_mode().unwrap_or(0);
        let ftype = mode & 0o170000;

        // Symlink: validate the (lexically-resolved) target stays inside dest.
        if ftype == 0o120000 {
            let mut target = String::new();
            entry
                .read_to_string(&mut target)
                .map_err(|e| AppError::ModelDownloadFailed(format!("Bad symlink in archive: {e}")))?;
            let link_parent = out.parent().unwrap_or(dest);
            if !lexically_within(dest, link_parent, target.trim()) {
                return Err(AppError::ModelDownloadFailed(format!(
                    "Archive symlink escapes destination: {} -> {}",
                    entry.name(),
                    target.trim()
                )));
            }
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            #[cfg(unix)]
            {
                let _ = std::fs::remove_file(&out);
                std::os::unix::fs::symlink(target.trim(), &out).map_err(|e| {
                    AppError::ModelDownloadFailed(format!("Failed to create symlink: {e}"))
                })?;
            }
            continue;
        }

        // Reject anything that isn't a regular file or directory.
        if ftype != 0 && ftype != 0o040000 && ftype != 0o100000 {
            return Err(AppError::ModelDownloadFailed(format!(
                "Archive contains a special file: {}",
                entry.name()
            )));
        }

        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }

        total = total.saturating_add(entry.size());
        if total > MAX_TOTAL_BYTES {
            return Err(AppError::ModelDownloadFailed(
                "Archive decompressed size exceeds the safety limit".into(),
            ));
        }

        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out_file = std::fs::File::create(&out)?;
        std::io::copy(&mut entry, &mut out_file)?;

        // Preserve executable bit for regular files (binaries inside the zip).
        #[cfg(unix)]
        if ftype == 0o100000 && (mode & 0o111) != 0 {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o755));
        }
    }
    Ok(())
}

/// Resolve `target` lexically relative to `base` and confirm it stays within
/// `root`. Rejects absolute targets. No filesystem access (target may not exist).
fn lexically_within(root: &Path, base: &Path, target: &str) -> bool {
    if target.is_empty() || Path::new(target).is_absolute() || target.contains('\\') {
        return false;
    }
    let mut stack: Vec<Component> = base.components().collect();
    for comp in Path::new(target).components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if stack.pop().is_none() {
                    return false;
                }
            }
            Component::Normal(_) => stack.push(comp),
            // Absolute/prefix components are unsafe.
            _ => return false,
        }
    }
    let resolved: PathBuf = stack.iter().collect();
    resolved.starts_with(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::Path;

    /// Write a ZIP at `path` from (name, bytes) entries, stored (no compression).
    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let f = std::fs::File::create(path).unwrap();
        let mut zw = zip::ZipWriter::new(f);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, data) in entries {
            zw.start_file(*name, opts).unwrap();
            zw.write_all(data).unwrap();
        }
        zw.finish().unwrap();
    }

    #[test]
    fn extracts_a_safe_archive() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("ok.zip");
        write_zip(&zip_path, &[("bin/tool", b"hello"), ("data/x.txt", b"y")]);

        let dest = tmp.path().join("out");
        safe_extract_zip(&zip_path, &dest).unwrap();

        assert_eq!(std::fs::read(dest.join("bin/tool")).unwrap(), b"hello");
        assert_eq!(std::fs::read(dest.join("data/x.txt")).unwrap(), b"y");
    }

    #[test]
    fn rejects_too_many_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("many.zip");
        let names: Vec<String> = (0..=MAX_ENTRIES).map(|i| format!("f{i}")).collect();
        let entries: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b""[..])).collect();
        write_zip(&zip_path, &entries);

        let dest = tmp.path().join("out");
        let err = safe_extract_zip(&zip_path, &dest).unwrap_err();
        assert!(err.to_string().contains("too many entries"), "got: {err}");
    }

    #[test]
    fn traversal_entry_does_not_escape_destination() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("evil.zip");
        // A path-traversal entry plus a benign one.
        write_zip(&zip_path, &[("../escaped.txt", b"pwned"), ("safe.txt", b"ok")]);

        let dest = tmp.path().join("out");
        let _ = safe_extract_zip(&zip_path, &dest); // may Err or skip; must not escape

        // The crucial invariant: nothing was written outside `dest`.
        assert!(
            !tmp.path().join("escaped.txt").exists(),
            "traversal entry escaped the destination directory"
        );
    }

    #[test]
    fn symlink_target_containment() {
        let root = Path::new("/rt");
        let base = Path::new("/rt/libtorch/lib");
        assert!(lexically_within(root, base, "libc10.so.1")); // same dir
        assert!(lexically_within(root, base, "../lib/libc10.so")); // sibling, still inside
        assert!(!lexically_within(root, base, "../../../etc/passwd")); // escapes
        assert!(!lexically_within(root, base, "/etc/passwd")); // absolute
        assert!(!lexically_within(root, base, "")); // empty
    }
}
