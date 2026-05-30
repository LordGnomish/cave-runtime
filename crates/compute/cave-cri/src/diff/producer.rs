// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Layer diff *production* (write side).
//!
//! Ports the `Compare` path of containerd's `core/diff/walking/differ.go`,
//! which layers on `continuity/fs.Changes` (the double-tree walk) and
//! `archive.WriteDiff` (changes → OCI layer tarball). Given a `lower`
//! directory (the parent snapshot) and an `upper` directory (the
//! current rootfs), it computes the change set and serialises it into a
//! tar stream:
//!
//! * **Add / Modify** entries carry the `upper` bytes (files), the
//!   directory header (dirs), or the link target (symlinks).
//! * **Delete** entries become an OCI/aufs whiteout — an empty regular
//!   file named `.wh.<name>` in the deleted entry's parent directory.
//!
//! `continuity/fs.Changes` walks both trees in lexical order and emits
//! one [`Change`] per differing path. cave-cri mirrors the *effect* with
//! a per-directory merge walk: a deleted directory yields a single
//! `Delete` for the directory (the whiteout subsumes the subtree, just
//! as aufs/overlayfs semantics dictate) rather than a delete per child,
//! and an added directory recurses so the tarball carries its contents.
//!
//! The defining correctness invariant — exercised by the round-trip
//! tests against [`crate::diff::walking_differ::apply_uncompressed_tar`]
//! — is that applying the produced layer on top of a copy of `lower`
//! reproduces `upper` byte-for-byte.

use crate::content::digest::{Digest, DigestAlgorithm};
use crate::diff::compression::compress_gzip;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use tar::{Builder, EntryType, Header};

#[derive(Debug, thiserror::Error)]
pub enum DiffError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("compression: {0}")]
    Compression(#[from] crate::diff::compression::CompressionError),
}

/// The kind of change a path underwent between `lower` and `upper`,
/// mirroring containerd's `fs.ChangeKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// Present in `upper`, absent in `lower`.
    Add,
    /// Present in both but the content / type / mode differs.
    Modify,
    /// Present in `lower`, absent in `upper`.
    Delete,
}

/// A single change between the two trees, identified by its path
/// relative to the tree roots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub kind: ChangeKind,
    pub path: PathBuf,
}

/// A produced OCI layer: the gzipped tarball, its uncompressed length,
/// the diff id (sha256 of the *uncompressed* tar), and the change set.
#[derive(Debug, Clone)]
pub struct DiffLayer {
    pub tar_gz: Vec<u8>,
    pub uncompressed_len: usize,
    pub diff_id: Digest,
    pub changes: Vec<Change>,
}

/// Lightweight classification of a directory entry, enough to decide
/// add/modify/delete without re-stat'ing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Dir,
    File,
    Symlink,
}

fn classify(meta: &fs::Metadata) -> Kind {
    let ft = meta.file_type();
    if ft.is_dir() {
        Kind::Dir
    } else if ft.is_symlink() {
        Kind::Symlink
    } else {
        Kind::File
    }
}

/// Read a directory's entries sorted by name. Missing directories are
/// treated as empty so the merge walk degrades cleanly.
fn sorted_entries(dir: &Path) -> Result<Vec<(String, fs::Metadata)>, DiffError> {
    let mut out = Vec::new();
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e.into()),
    };
    for entry in rd {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        // symlink_metadata so we classify symlinks rather than following.
        let meta = fs::symlink_metadata(entry.path())?;
        out.push((name, meta));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// True when a file present in both trees is byte-for-byte identical
/// (and same mode on unix). Containerd's `sameFile` also compares
/// mtime/size; we compare the bytes directly which is stronger and
/// deterministic for our single-host snapshots.
fn files_equal(a: &Path, b: &Path) -> Result<bool, DiffError> {
    let ma = fs::symlink_metadata(a)?;
    let mb = fs::symlink_metadata(b)?;
    if ma.len() != mb.len() {
        return Ok(false);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if ma.permissions().mode() & 0o7777 != mb.permissions().mode() & 0o7777 {
            return Ok(false);
        }
    }
    Ok(fs::read(a)? == fs::read(b)?)
}

fn symlinks_equal(a: &Path, b: &Path) -> Result<bool, DiffError> {
    Ok(fs::read_link(a)? == fs::read_link(b)?)
}

/// Compute the change set between `lower` and `upper`, sorted by path.
pub fn compute_changes(lower: &Path, upper: &Path) -> Result<Vec<Change>, DiffError> {
    let mut out = Vec::new();
    diff_dir(lower, upper, Path::new(""), &mut out)?;
    Ok(out)
}

/// Recursively diff `rel` (relative to both roots), appending changes.
fn diff_dir(
    lower_root: &Path,
    upper_root: &Path,
    rel: &Path,
    out: &mut Vec<Change>,
) -> Result<(), DiffError> {
    let lower_dir = lower_root.join(rel);
    let upper_dir = upper_root.join(rel);
    let lower_entries = sorted_entries(&lower_dir)?;
    let upper_entries = sorted_entries(&upper_dir)?;

    let mut i = 0; // index into lower
    let mut j = 0; // index into upper
    while i < lower_entries.len() || j < upper_entries.len() {
        let in_lower = i < lower_entries.len();
        let in_upper = j < upper_entries.len();

        // Decide which side(s) the next path comes from by lexical name.
        let take_lower_only = in_lower
            && (!in_upper || lower_entries[i].0 < upper_entries[j].0);
        let take_upper_only = in_upper
            && (!in_lower || upper_entries[j].0 < lower_entries[i].0);

        if take_lower_only {
            // Present in lower only → Delete. Do not recurse: a single
            // whiteout of the entry removes the whole subtree on apply.
            let (name, _) = &lower_entries[i];
            out.push(Change { kind: ChangeKind::Delete, path: rel.join(name) });
            i += 1;
        } else if take_upper_only {
            // Present in upper only → Add (recurse into added dirs so
            // their contents land in the layer).
            let (name, meta) = &upper_entries[j];
            let child_rel = rel.join(name);
            out.push(Change { kind: ChangeKind::Add, path: child_rel.clone() });
            if classify(meta) == Kind::Dir {
                add_recursive(upper_root, &child_rel, out)?;
            }
            j += 1;
        } else {
            // Same name in both.
            let (name, lmeta) = &lower_entries[i];
            let umeta = &upper_entries[j].1;
            let child_rel = rel.join(name);
            let lkind = classify(lmeta);
            let ukind = classify(umeta);
            if lkind == Kind::Dir && ukind == Kind::Dir {
                // Recurse; the directory itself is unchanged.
                diff_dir(lower_root, upper_root, &child_rel, out)?;
            } else if lkind != ukind {
                // Type change → Modify, and if it became a dir, add its
                // subtree.
                out.push(Change { kind: ChangeKind::Modify, path: child_rel.clone() });
                if ukind == Kind::Dir {
                    add_recursive(upper_root, &child_rel, out)?;
                }
            } else {
                // Same type, both files or both symlinks: compare.
                let lp = lower_root.join(&child_rel);
                let up = upper_root.join(&child_rel);
                let equal = match ukind {
                    Kind::Symlink => symlinks_equal(&lp, &up)?,
                    _ => files_equal(&lp, &up)?,
                };
                if !equal {
                    out.push(Change { kind: ChangeKind::Modify, path: child_rel });
                }
            }
            i += 1;
            j += 1;
        }
    }
    Ok(())
}

/// Emit Add changes for every descendant of `rel` (already emitted),
/// walking `upper_root` in sorted order.
fn add_recursive(
    upper_root: &Path,
    rel: &Path,
    out: &mut Vec<Change>,
) -> Result<(), DiffError> {
    for (name, meta) in sorted_entries(&upper_root.join(rel))? {
        let child = rel.join(&name);
        out.push(Change { kind: ChangeKind::Add, path: child.clone() });
        if classify(&meta) == Kind::Dir {
            add_recursive(upper_root, &child, out)?;
        }
    }
    Ok(())
}

/// Build the whiteout path for a deleted entry: `.wh.<name>` in the
/// deleted entry's parent directory.
fn whiteout_path(deleted: &Path) -> PathBuf {
    let name = deleted
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match deleted.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.join(format!(".wh.{name}")),
        _ => PathBuf::from(format!(".wh.{name}")),
    }
}

/// Produce the *uncompressed* layer tarball from the diff of the two
/// trees. Mirrors `archive.WriteDiff`.
pub fn write_diff_tar(lower: &Path, upper: &Path) -> Result<Vec<u8>, DiffError> {
    let changes = compute_changes(lower, upper)?;
    write_tar_for_changes(upper, &changes)
}

fn write_tar_for_changes(upper: &Path, changes: &[Change]) -> Result<Vec<u8>, DiffError> {
    let mut builder = Builder::new(Vec::new());
    for change in changes {
        match change.kind {
            ChangeKind::Delete => {
                let wh = whiteout_path(&change.path);
                let mut h = Header::new_gnu();
                h.set_entry_type(EntryType::Regular);
                h.set_size(0);
                h.set_mode(0o644);
                h.set_cksum();
                builder.append_data(&mut h, &wh, std::io::empty())?;
            }
            ChangeKind::Add | ChangeKind::Modify => {
                let src = upper.join(&change.path);
                let meta = fs::symlink_metadata(&src)?;
                match classify(&meta) {
                    Kind::Dir => {
                        let mut h = Header::new_gnu();
                        h.set_entry_type(EntryType::Directory);
                        h.set_size(0);
                        h.set_mode(dir_mode(&meta));
                        h.set_cksum();
                        let dir_path = ensure_trailing_slash(&change.path);
                        builder.append_data(&mut h, &dir_path, std::io::empty())?;
                    }
                    Kind::Symlink => {
                        let target = fs::read_link(&src)?;
                        let mut h = Header::new_gnu();
                        h.set_entry_type(EntryType::Symlink);
                        h.set_size(0);
                        h.set_mode(0o777);
                        h.set_cksum();
                        builder.append_link(&mut h, &change.path, &target)?;
                    }
                    Kind::File => {
                        let data = fs::read(&src)?;
                        let mut h = Header::new_gnu();
                        h.set_entry_type(EntryType::Regular);
                        h.set_size(data.len() as u64);
                        h.set_mode(file_mode(&meta));
                        h.set_cksum();
                        builder.append_data(&mut h, &change.path, Cursor::new(data))?;
                    }
                }
            }
        }
    }
    Ok(builder.into_inner()?)
}

fn ensure_trailing_slash(p: &Path) -> PathBuf {
    let mut s = p.to_string_lossy().replace('\\', "/");
    if !s.ends_with('/') {
        s.push('/');
    }
    PathBuf::from(s)
}

#[cfg(unix)]
fn file_mode(meta: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o7777
}
#[cfg(not(unix))]
fn file_mode(_meta: &fs::Metadata) -> u32 {
    0o644
}

#[cfg(unix)]
fn dir_mode(meta: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o7777
}
#[cfg(not(unix))]
fn dir_mode(_meta: &fs::Metadata) -> u32 {
    0o755
}

/// Produce a complete [`DiffLayer`]: the gzipped tar, its uncompressed
/// length, the diff id (sha256 of the uncompressed tar), and the
/// change set.
pub fn diff_layer(lower: &Path, upper: &Path) -> Result<DiffLayer, DiffError> {
    let changes = compute_changes(lower, upper)?;
    let tar = write_tar_for_changes(upper, &changes)?;
    let diff_id = Digest::compute(DigestAlgorithm::Sha256, &tar);
    let tar_gz = compress_gzip(&tar)?;
    Ok(DiffLayer {
        tar_gz,
        uncompressed_len: tar.len(),
        diff_id,
        changes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn whiteout_path_for_top_level() {
        assert_eq!(whiteout_path(Path::new("foo")), PathBuf::from(".wh.foo"));
    }

    #[test]
    fn whiteout_path_for_nested() {
        assert_eq!(whiteout_path(Path::new("a/b")), PathBuf::from("a/.wh.b"));
    }

    #[test]
    fn classify_distinguishes_dir_and_file() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join("d")).unwrap();
        fs::write(tmp.path().join("f"), b"x").unwrap();
        assert_eq!(classify(&fs::symlink_metadata(tmp.path().join("d")).unwrap()), Kind::Dir);
        assert_eq!(classify(&fs::symlink_metadata(tmp.path().join("f")).unwrap()), Kind::File);
    }
}
