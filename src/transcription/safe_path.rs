// Speech to Text - Safe path joining for untrusted (remote) filenames
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Defends against path traversal when joining filenames that come from a
//! remote source (e.g. HuggingFace `siblings[*].rfilename`). A malicious or
//! compromised manifest must never be able to write outside the target dir.

use std::path::{Component, Path, PathBuf};

/// Join `rel` onto `base`, returning the path only if `rel` is a clean relative
/// path that stays inside `base`. Rejects absolute paths, drive/UNC prefixes,
/// `.`/`..`/empty components, non-UTF-8 names, and anything that would escape.
pub fn safe_join(base: &Path, rel: &str) -> Option<PathBuf> {
    if rel.trim().is_empty() {
        return None;
    }
    // Reject obvious backslash traversal too (a Unix path keeps "a\\..\\b" as a
    // single component, which would otherwise sneak through).
    if rel.contains('\\') {
        return None;
    }

    let rel_path = Path::new(rel);
    if rel_path.is_absolute() {
        return None;
    }

    let mut out = base.to_path_buf();
    for comp in rel_path.components() {
        match comp {
            Component::Normal(c) => {
                let s = c.to_str()?; // reject non-UTF-8
                if s.is_empty() || s == "." || s == ".." {
                    return None;
                }
                out.push(s);
            }
            // CurDir, ParentDir, RootDir, Prefix are all unsafe here.
            _ => return None,
        }
    }

    // Lexical containment guard (no canonicalization needed; dest may not exist).
    if !out.starts_with(base) || out == base {
        return None;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn accepts_clean_relative_names() {
        let base = Path::new("/data/model");
        assert_eq!(
            safe_join(base, "config.json"),
            Some(base.join("config.json"))
        );
        assert_eq!(
            safe_join(base, "sub/dir/model.safetensors"),
            Some(base.join("sub/dir/model.safetensors"))
        );
    }

    #[test]
    fn rejects_traversal_and_absolute() {
        let base = Path::new("/data/model");
        for bad in [
            "../evil",
            "../../etc/passwd",
            "a/../../evil",
            "/etc/passwd",
            "/tmp/evil",
            "",
            "   ",
            ".",
            "..",
            "a/..",
            "foo/../../bar",
            "..\\windows",
            "a\\b",
        ] {
            assert!(safe_join(base, bad).is_none(), "should reject: {bad:?}");
        }
    }

    #[test]
    fn result_stays_inside_base() {
        let base = Path::new("/data/model");
        let p = safe_join(base, "x/y/z.bin").unwrap();
        assert!(p.starts_with(base));
    }
}
