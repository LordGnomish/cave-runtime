// SPDX-License-Identifier: AGPL-3.0-or-later
//! Layer-apply (read side).
//!
//! Ports `core/diff/walking/walking.go`'s Apply path. Reads a
//! gzipped tarball and unpacks it into a target directory, honoring
//! the OCI whiteout convention:
//!
//! * `.wh..wh..opq` in a directory → opaque marker, delete every
//!   sibling entry already present.
//! * `.wh.<name>` → delete `<name>` from the layer below.
//! * any other entry → unpacked normally.
//!
//! Containerd's implementation additionally tracks parent-relative
//! whiteouts; cave-cri's overlayfs assembly handles those at mount
//! time via the overlay driver, so this implementation focuses on
//! the in-tarball protocol that upstream tar readers understand.

use crate::diff::compression::{decompress_gzip, CompressionError};
use std::fs;
use std::io::Cursor;
use std::path::{Component, Path, PathBuf};
use tar::{Archive, EntryType};

#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("compression: {0}")]
    Compression(#[from] CompressionError),
    #[error("entry path escapes target: {0}")]
    PathEscape(String),
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ApplyStats {
    pub files_written: u64,
    pub dirs_created: u64,
    pub symlinks_created: u64,
    pub whiteouts_applied: u64,
}

/// Apply a gzipped layer tarball into `target_dir`. The caller is
/// expected to have already created `target_dir`. Returns stats on
/// what was applied.
pub fn apply_layer(
    gzipped_tar: &[u8],
    target_dir: &Path,
) -> Result<ApplyStats, ApplyError> {
    let uncompressed = decompress_gzip(gzipped_tar)?;
    apply_uncompressed_tar(&uncompressed, target_dir)
}

/// Same as [`apply_layer`] but takes an already-decompressed tarball.
pub fn apply_uncompressed_tar(
    tar_bytes: &[u8],
    target_dir: &Path,
) -> Result<ApplyStats, ApplyError> {
    fs::create_dir_all(target_dir)?;
    let mut stats = ApplyStats::default();
    let mut archive = Archive::new(Cursor::new(tar_bytes));
    // Don't let tar set ownership — we're not root, and the OCI image
    // metadata bakes the intended uid/gid into the layer descriptors
    // anyway. Containerd's walking_differ behaves the same way for
    // unprivileged mode.
    archive.set_preserve_permissions(true);
    archive.set_preserve_mtime(true);

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path_in_tar = entry.path()?.into_owned();
        let safe_rel = sanitize_path(&path_in_tar)?;
        let file_name = match safe_rel.file_name().and_then(|s| s.to_str()) {
            Some(n) => n.to_string(),
            None => continue, // root entry; skip
        };

        if let Some(whiteout_for) = whiteout_target(&file_name) {
            apply_whiteout(target_dir, &safe_rel, whiteout_for, &mut stats)?;
            continue;
        }

        let dst = target_dir.join(&safe_rel);
        match entry.header().entry_type() {
            EntryType::Directory => {
                fs::create_dir_all(&dst)?;
                stats.dirs_created += 1;
            }
            EntryType::Symlink => {
                if dst.exists() {
                    fs::remove_file(&dst).ok();
                }
                let link_target = entry
                    .link_name()?
                    .ok_or_else(|| ApplyError::Io(std::io::Error::other("symlink without target")))?
                    .into_owned();
                #[cfg(unix)]
                {
                    std::os::unix::fs::symlink(&link_target, &dst)?;
                }
                #[cfg(not(unix))]
                {
                    // On non-unix targets, materialize as a regular
                    // file containing the link target. cave-cri only
                    // ships on Linux so this is a test-only branch.
                    fs::write(&dst, link_target.to_string_lossy().as_bytes())?;
                }
                stats.symlinks_created += 1;
            }
            EntryType::Regular | EntryType::Continuous => {
                if let Some(parent) = dst.parent() {
                    fs::create_dir_all(parent)?;
                }
                entry.unpack(&dst)?;
                stats.files_written += 1;
            }
            _ => {
                // Hard links, char/block devices, FIFOs — containerd's
                // walking_differ handles these too. cave-cri runs in
                // user namespaces so these are no-ops here.
            }
        }
    }
    Ok(stats)
}

/// Returns the target name a whiteout entry refers to, or `None` for
/// non-whiteout entries.
fn whiteout_target(file_name: &str) -> Option<WhiteoutKind<'_>> {
    if file_name == ".wh..wh..opq" {
        return Some(WhiteoutKind::Opaque);
    }
    file_name
        .strip_prefix(".wh.")
        .map(WhiteoutKind::DeleteSibling)
}

enum WhiteoutKind<'a> {
    /// Delete `<target>` in the parent of this entry.
    DeleteSibling(&'a str),
    /// Empty the directory containing this marker.
    Opaque,
}

fn apply_whiteout(
    target_dir: &Path,
    rel_path: &Path,
    kind: WhiteoutKind<'_>,
    stats: &mut ApplyStats,
) -> Result<(), ApplyError> {
    let parent_rel = rel_path.parent().unwrap_or(Path::new(""));
    let parent_abs = target_dir.join(parent_rel);
    match kind {
        WhiteoutKind::Opaque => {
            if let Ok(entries) = fs::read_dir(&parent_abs) {
                for e in entries.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        fs::remove_dir_all(&p).ok();
                    } else {
                        fs::remove_file(&p).ok();
                    }
                }
            }
            stats.whiteouts_applied += 1;
        }
        WhiteoutKind::DeleteSibling(name) => {
            let victim = parent_abs.join(name);
            if victim.is_dir() {
                fs::remove_dir_all(&victim).ok();
            } else {
                fs::remove_file(&victim).ok();
            }
            stats.whiteouts_applied += 1;
        }
    }
    Ok(())
}

fn sanitize_path(path: &Path) -> Result<PathBuf, ApplyError> {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(s) => out.push(s),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(ApplyError::PathEscape(path.display().to_string()));
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err(ApplyError::PathEscape(path.display().to_string()));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::compression::compress_gzip;
    use tar::{Builder, Header};
    use tempfile::TempDir;

    fn make_tar(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut b = Builder::new(Vec::new());
        for (path, data) in files {
            let mut h = Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, path, *data).unwrap();
        }
        b.into_inner().unwrap()
    }

    fn make_whiteout_tar(victims: &[&str]) -> Vec<u8> {
        let mut b = Builder::new(Vec::new());
        for v in victims {
            let mut h = Header::new_gnu();
            h.set_size(0);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, v, std::io::empty()).unwrap();
        }
        b.into_inner().unwrap()
    }

    fn make_dir_tar(dirs: &[&str]) -> Vec<u8> {
        let mut b = Builder::new(Vec::new());
        for d in dirs {
            let mut h = Header::new_gnu();
            h.set_entry_type(EntryType::Directory);
            h.set_size(0);
            h.set_mode(0o755);
            h.set_cksum();
            b.append_data(&mut h, d, std::io::empty()).unwrap();
        }
        b.into_inner().unwrap()
    }

    #[test]
    fn apply_unpacks_regular_files() {
        let tar = make_tar(&[("hello.txt", b"hi!"), ("nested/inner.bin", b"bytes")]);
        let gz = compress_gzip(&tar).unwrap();
        let tmp = TempDir::new().unwrap();
        let stats = apply_layer(&gz, tmp.path()).unwrap();
        assert_eq!(stats.files_written, 2);
        assert_eq!(fs::read(tmp.path().join("hello.txt")).unwrap(), b"hi!");
        assert_eq!(fs::read(tmp.path().join("nested/inner.bin")).unwrap(), b"bytes");
    }

    #[test]
    fn apply_creates_explicit_directories() {
        let tar = make_dir_tar(&["a/", "a/b/", "a/b/c/"]);
        let gz = compress_gzip(&tar).unwrap();
        let tmp = TempDir::new().unwrap();
        let stats = apply_layer(&gz, tmp.path()).unwrap();
        assert_eq!(stats.dirs_created, 3);
        assert!(tmp.path().join("a/b/c").is_dir());
    }

    #[test]
    fn apply_whiteout_removes_file() {
        let tmp = TempDir::new().unwrap();
        // Pre-seed a file as if a lower layer placed it.
        fs::write(tmp.path().join("victim.txt"), b"die").unwrap();
        let tar = make_whiteout_tar(&[".wh.victim.txt"]);
        let gz = compress_gzip(&tar).unwrap();
        let stats = apply_layer(&gz, tmp.path()).unwrap();
        assert_eq!(stats.whiteouts_applied, 1);
        assert!(!tmp.path().join("victim.txt").exists());
    }

    #[test]
    fn apply_opaque_clears_directory() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("d")).unwrap();
        fs::write(tmp.path().join("d/one"), b"1").unwrap();
        fs::write(tmp.path().join("d/two"), b"2").unwrap();
        let tar = make_whiteout_tar(&["d/.wh..wh..opq"]);
        let gz = compress_gzip(&tar).unwrap();
        let stats = apply_layer(&gz, tmp.path()).unwrap();
        assert_eq!(stats.whiteouts_applied, 1);
        assert!(!tmp.path().join("d/one").exists());
        assert!(!tmp.path().join("d/two").exists());
    }

    #[test]
    fn sanitize_path_rejects_parent_dir_component() {
        let err = sanitize_path(Path::new("../escape")).unwrap_err();
        assert!(matches!(err, ApplyError::PathEscape(_)));
    }

    #[test]
    fn sanitize_path_rejects_absolute_path() {
        let err = sanitize_path(Path::new("/etc/passwd")).unwrap_err();
        assert!(matches!(err, ApplyError::PathEscape(_)));
    }

    #[test]
    fn sanitize_path_strips_cur_dir_components() {
        let p = sanitize_path(Path::new("./a/./b")).unwrap();
        assert_eq!(p, PathBuf::from("a/b"));
    }

    #[test]
    fn apply_creates_target_dir_if_missing() {
        let tar = make_tar(&[("f", b"x")]);
        let gz = compress_gzip(&tar).unwrap();
        let tmp = TempDir::new().unwrap();
        let nested = tmp.path().join("not/yet/created");
        let stats = apply_layer(&gz, &nested).unwrap();
        assert_eq!(stats.files_written, 1);
        assert!(nested.join("f").is_file());
    }

    #[test]
    fn apply_empty_layer_returns_zero_stats() {
        let b = Builder::new(Vec::new());
        let tar = b.into_inner().unwrap();
        let gz = compress_gzip(&tar).unwrap();
        let tmp = TempDir::new().unwrap();
        let stats = apply_layer(&gz, tmp.path()).unwrap();
        assert_eq!(stats, ApplyStats::default());
    }

    #[test]
    fn apply_rejects_corrupt_gzip() {
        let mut bytes = compress_gzip(&make_tar(&[("a", b"a")])).unwrap();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0xff;
        let tmp = TempDir::new().unwrap();
        assert!(apply_layer(&bytes, tmp.path()).is_err());
    }

    #[test]
    fn apply_handles_repeated_apply_idempotently() {
        let tar = make_tar(&[("k", b"v")]);
        let gz = compress_gzip(&tar).unwrap();
        let tmp = TempDir::new().unwrap();
        apply_layer(&gz, tmp.path()).unwrap();
        // Second apply over the same target: should overwrite cleanly.
        let stats = apply_layer(&gz, tmp.path()).unwrap();
        assert_eq!(stats.files_written, 1);
        assert_eq!(fs::read(tmp.path().join("k")).unwrap(), b"v");
    }

    #[test]
    #[cfg(unix)]
    fn apply_unpacks_symlinks() {
        let mut buf = Vec::new();
        {
            let mut b = Builder::new(&mut buf);
            // Create a real file first so the link has a target.
            let mut h = Header::new_gnu();
            h.set_size(3);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, "real.txt", &b"abc"[..]).unwrap();
            // Now a symlink to it.
            let mut sh = Header::new_gnu();
            sh.set_entry_type(EntryType::Symlink);
            sh.set_size(0);
            sh.set_mode(0o777);
            sh.set_cksum();
            b.append_link(&mut sh, "link.txt", "real.txt").unwrap();
            b.into_inner().unwrap();
        }
        let gz = compress_gzip(&buf).unwrap();
        let tmp = TempDir::new().unwrap();
        let stats = apply_layer(&gz, tmp.path()).unwrap();
        assert_eq!(stats.symlinks_created, 1);
        assert_eq!(stats.files_written, 1);
        let resolved = fs::read_link(tmp.path().join("link.txt")).unwrap();
        assert_eq!(resolved.to_str().unwrap(), "real.txt");
    }
}
