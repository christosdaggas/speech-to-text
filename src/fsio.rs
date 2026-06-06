// Speech to Text - Private, atomic file I/O
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Helpers for writing local state (config, history) that may contain personal
//! data (transcripts, endpoint URLs). Two guarantees:
//!
//! * **Private**: the containing directory is `0700` and files are `0600` on
//!   Unix, so other local users can't read transcripts or settings.
//! * **Atomic**: we write to a temp file in the *same* directory, `fsync`, then
//!   `rename` it into place. A crash mid-write can never truncate or corrupt the
//!   previous good file — readers see either the old or the new content.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

/// Ensure `dir` exists and is private (`0700` on Unix).
pub fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Atomically write `bytes` to `path` with private (`0600`) permissions.
///
/// The temp file is created in `path`'s parent directory (so the final rename
/// stays on one filesystem and is atomic), `fsync`'d, set to `0600`, then
/// persisted onto `path`.
pub fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    ensure_private_dir(parent)?;

    // NamedTempFile is created with mode 0600 via mkstemp; we re-assert it for
    // belt-and-suspenders and to be explicit about intent.
    let mut tmp = tempfile::Builder::new()
        .prefix(".tmp-")
        .tempfile_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    // Atomic rename onto the destination, preserving the 0600 temp permissions.
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn write_private_sets_0600_and_persists_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("secret.json");
        write_private(&path, b"hello").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"hello");
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "file should be private 0600");
        let dmode = fs::metadata(path.parent().unwrap()).unwrap().permissions().mode() & 0o777;
        assert_eq!(dmode, 0o700, "directory should be private 0700");
    }

    #[test]
    fn write_private_overwrites_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("h.json");
        write_private(&path, b"first").unwrap();
        write_private(&path, b"second").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
    }
}
