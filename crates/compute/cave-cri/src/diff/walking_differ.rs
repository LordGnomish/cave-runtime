// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
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
//!
//! # Containment
//!
//! A layer is untrusted input: nothing in it may create, change or
//! remove a path outside the target directory. A member that would do
//! so makes the apply fail with [`ApplyError::PathEscape`], which
//! names the member. It is not skipped, and members that precede it in
//! the archive stay applied. These are the rules of
//! `rootfs::extract_layer`, with the checks of `crate::containment`:
//!
//! * a member name that is absolute or has a `..` component is
//!   refused;
//! * a member whose directory is, after resolving symlinks, not inside
//!   the target directory is refused, whether the symlink was in the
//!   target directory before or comes from the same archive. So is one
//!   whose directory cannot be resolved;
//! * a whiteout whose name after `.wh.` is empty, `.` or `..` is
//!   refused: it names no entry of its directory.
//!
//! A symlink member is stored as it is, wherever it points; it is only
//! never followed out of the target directory. Files and symlinks are
//! written by [`tar::Entry::unpack_in`], which replaces an entry that
//! has the member's name rather than writing through it. A whiteout
//! and an opaque marker remove entries themselves, never what a
//! symlink points to. Hard link members are not applied at all, as
//! before, so the target of one is never used. As in the `tar` crate,
//! a process that changes the target directory during an apply is not
//! defended against.

use crate::containment::{self, Refusal};
use crate::diff::compression::{decompress_gzip, CompressionError};
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};
use tar::{Archive, Entry, EntryType};

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
///
/// Nothing outside `target_dir` is created, changed or removed; see the
/// module documentation for what is refused.
pub fn apply_layer(gzipped_tar: &[u8], target_dir: &Path) -> Result<ApplyStats, ApplyError> {
    let uncompressed = decompress_gzip(gzipped_tar)?;
    apply_uncompressed_tar(&uncompressed, target_dir)
}

/// Same as [`apply_layer`] but takes an already-decompressed tarball.
pub fn apply_uncompressed_tar(
    tar_bytes: &[u8],
    target_dir: &Path,
) -> Result<ApplyStats, ApplyError> {
    fs::create_dir_all(target_dir)?;
    // Every containment check below compares resolved paths with this one.
    let root = target_dir.canonicalize()?;
    let root = root.as_path();
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
            apply_whiteout(root, &safe_rel, whiteout_for, &mut stats)
                .map_err(|why| refused(&path_in_tar, why))?;
            continue;
        }

        match entry.header().entry_type() {
            EntryType::Directory => {
                // Resolved first: `create_dir_all` follows symlinks. A
                // directory that exists is left as it is, also if the
                // member's name is a symlink to it.
                let dir = containment::contained_dir(root, &safe_rel)
                    .map_err(|why| refused(&path_in_tar, why))?;
                fs::create_dir_all(&dir.path)?;
                stats.dirs_created += 1;
            }
            EntryType::Symlink => {
                if entry.link_name()?.is_none() {
                    return Err(ApplyError::Io(std::io::Error::other(
                        "symlink without target",
                    )));
                }
                unpack_member(&mut entry, root, &safe_rel, &path_in_tar)?;
                stats.symlinks_created += 1;
            }
            EntryType::Regular | EntryType::Continuous => {
                unpack_member(&mut entry, root, &safe_rel, &path_in_tar)?;
                stats.files_written += 1;
            }
            _ => {
                // Hard links, char/block devices, FIFOs — containerd's
                // walking_differ handles these too. cave-cri runs in
                // user namespaces so these are no-ops here. Nothing is
                // created for them, so a hard link's target is never
                // looked at.
            }
        }
    }
    Ok(stats)
}

/// The error for a member that is refused: its name as the archive has
/// it, then the reason.
fn refused(member: &Path, why: Refusal) -> ApplyError {
    ApplyError::PathEscape(format!("{}: {}", member.display(), why))
}

/// Write the file or symlink member `entry` below `root`, the canonical
/// target directory. `rel_path` is the sanitized member name, `member`
/// the name as the archive has it.
///
/// [`tar::Entry::unpack_in`] creates missing parent directories, replaces
/// an entry that has the member's name instead of writing through it, and
/// stores a symlink as it is. It also refuses to follow a symlink out of
/// `root`, but reports that as an I/O error like any other. The member's
/// directory is therefore resolved here first, so that a member that would
/// leave the target directory is reported as that.
fn unpack_member<R: Read>(
    entry: &mut Entry<'_, R>,
    root: &Path,
    rel_path: &Path,
    member: &Path,
) -> Result<(), ApplyError> {
    containment::member_dir(root, rel_path).map_err(|why| refused(member, why))?;
    if entry.unpack_in(root)? {
        Ok(())
    } else {
        // `unpack_in` reports a member it skipped for its name this way.
        Err(ApplyError::PathEscape(member.display().to_string()))
    }
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

/// Apply the whiteout member `rel_path` (a sanitized member name) below
/// `root`, the canonical target directory.
///
/// The whiteout acts in its own directory, which has to resolve to a path
/// inside `root`; if that directory does not exist there is nothing to
/// remove. Entries are removed as what they are: a symlink goes away, what
/// it points to stays. A whiteout that is refused is not counted.
fn apply_whiteout(
    root: &Path,
    rel_path: &Path,
    kind: WhiteoutKind<'_>,
    stats: &mut ApplyStats,
) -> Result<(), Refusal> {
    if let WhiteoutKind::DeleteSibling(name) = kind {
        containment::check_whiteout_name(name)?;
    }
    let dir = containment::whiteout_dir(root, rel_path, false)?;
    match (kind, dir) {
        (WhiteoutKind::Opaque, Some(dir)) => {
            if let Ok(entries) = fs::read_dir(&dir) {
                for e in entries.flatten() {
                    containment::remove_entry(&e.path());
                }
            }
        }
        (WhiteoutKind::DeleteSibling(name), Some(dir)) => {
            containment::remove_entry(&dir.join(name));
        }
        (_, None) => {}
    }
    stats.whiteouts_applied += 1;
    Ok(())
}

/// The member name `path` as a path relative to the target directory:
/// its normal components. A name that is absolute or has a `..`
/// component is refused.
fn sanitize_path(path: &Path) -> Result<PathBuf, ApplyError> {
    containment::check_member_name(path)
        .map_err(|_| ApplyError::PathEscape(path.display().to_string()))?;
    Ok(path
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part),
            _ => None,
        })
        .collect())
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
        assert_eq!(
            fs::read(tmp.path().join("nested/inner.bin")).unwrap(),
            b"bytes"
        );
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
