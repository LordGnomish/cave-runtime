// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Containment checks for unpacking a layer archive into a directory.
//!
//! A layer archive is untrusted input: nothing in it may create, change or
//! remove a path outside the directory it is unpacked into. The two
//! unpackers of this crate, `rootfs::extract_layer` and
//! `diff::walking_differ`, write files and links with
//! [`tar::Entry::unpack_in`], which does not follow a symlink out of its
//! destination. This module holds the checks for what `tar` knows nothing
//! about: member names, and the directory in which a whiteout, an opaque
//! marker or a directory member acts.
//!
//! The directory is passed as `root` and has to be a canonical path. A
//! member that is refused is reported as a [`Refusal`], which is the reason
//! only: the caller adds the member's name and wraps both in its own error
//! type.
//!
//! As in the `tar` crate, a process that changes `root` while an archive is
//! being unpacked is not defended against.

use std::fmt;
use std::path::{Component, Path, PathBuf};

/// Why a layer member is refused.
#[derive(Debug)]
pub(crate) struct Refusal(String);

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A member name has to be a relative path without `..`: joined to the
/// layer directory, an absolute name replaces it and `..` climbs out of it.
/// Names are not normalised, so `a/../b` is refused as well.
pub(crate) fn check_member_name(member: &Path) -> Result<(), Refusal> {
    for component in member.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => {
                return Err(Refusal(
                    "a `..` in a member name leads outside the layer directory".into(),
                ));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(Refusal(
                    "an absolute member name leads outside the layer directory".into(),
                ));
            }
        }
    }
    Ok(())
}

/// The name after `.wh.` has to be one entry of the whiteout's directory.
/// An empty name or `.` would remove that directory itself and `..` the
/// parent of it.
pub(crate) fn check_whiteout_name(name: &str) -> Result<(), Refusal> {
    if matches!(name, "" | "." | "..") {
        return Err(Refusal(
            "a whiteout has to name one entry of its own directory, not the \
             directory itself or its parent, which can be outside the layer directory"
                .into(),
        ));
    }
    Ok(())
}

/// A directory below the layer directory, with symlinks resolved.
pub(crate) struct ContainedDir {
    /// The canonical path of the part of the directory that exists,
    /// followed by the part that is missing.
    pub(crate) path: PathBuf,
    /// Whether all of it exists.
    pub(crate) exists: bool,
}

/// Where the directory `dir` is, or would be created: `dir` is relative to
/// `root` (a canonical path) and has passed [`check_member_name`].
///
/// The deepest part of the directory which exists decides where the rest
/// is, or would be created. It has to resolve to a path inside `root`,
/// otherwise the member is refused. Nothing is created here.
pub(crate) fn contained_dir(root: &Path, dir: &Path) -> Result<ContainedDir, Refusal> {
    let dir = root.join(dir);

    // `symlink_metadata`, so that a symlink that points nowhere counts as
    // existing: nothing may be created through it.
    let mut existing = dir.as_path();
    let mut missing = Vec::new();
    while existing.symlink_metadata().is_err() {
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name);
                existing = parent;
            }
            _ => break,
        }
    }

    let mut resolved = existing.canonicalize().map_err(|e| {
        Refusal(format!(
            "its directory cannot be resolved inside the layer directory: {e}"
        ))
    })?;
    if !resolved.starts_with(root) {
        return Err(Refusal(format!(
            "its directory resolves to {}, outside the layer directory",
            resolved.display()
        )));
    }

    let exists = missing.is_empty();
    resolved.extend(missing.into_iter().rev());
    Ok(ContainedDir {
        path: resolved,
        exists,
    })
}

/// [`contained_dir`] for the directory part of the member name `member`:
/// the directory a file, link or whiteout member is, or would be, an entry
/// of.
pub(crate) fn member_dir(root: &Path, member: &Path) -> Result<ContainedDir, Refusal> {
    contained_dir(root, member.parent().unwrap_or(Path::new("")))
}

/// The directory a whiteout member acts in, with symlinks resolved: the
/// directory part of the member name below `root` (a canonical path).
///
/// The member is refused if that directory does not resolve to a path
/// inside `root`, see [`contained_dir`]. If part of the directory is
/// missing, it is created for `create`; without `create`, or if it cannot
/// be created, the result is `None` and the whiteout has no effect.
pub(crate) fn whiteout_dir(
    root: &Path,
    member: &Path,
    create: bool,
) -> Result<Option<PathBuf>, Refusal> {
    let dir = member_dir(root, member)?;
    if dir.exists {
        return Ok(Some(dir.path));
    }
    if !create {
        return Ok(None);
    }
    Ok(std::fs::create_dir_all(&dir.path).ok().map(|()| dir.path))
}

/// Remove the entry at `path`: a directory with everything in it, anything
/// else as the one entry it is. The decision is made on the entry itself,
/// not on what a symlink of that name points to, so a symlink is removed
/// and its target stays. An entry that is missing or cannot be removed is
/// not an error.
pub(crate) fn remove_entry(path: &Path) {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => {
            std::fs::remove_dir_all(path).ok();
        }
        Ok(_) => {
            std::fs::remove_file(path).ok();
        }
        Err(_) => {}
    }
}
