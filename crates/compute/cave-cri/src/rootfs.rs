// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Root filesystem assembly from OCI image layers using overlayfs.

use crate::containment;
use crate::error::{CriError, CriResult};
use crate::models::OciImage;
use crate::paths;
use std::path::{Path, PathBuf};

/// Assemble a rootfs from OCI image layers.
///
/// Uses overlayfs to stack layers (lower=layer1:layer2:..., upper=writable, work=workdir).
/// If no layer was extracted there is nothing to stack and nothing is
/// mounted: the returned `merged` directory is then a plain, empty directory.
pub fn assemble_rootfs(image: &OciImage, container_id: &str) -> CriResult<PathBuf> {
    let container_dir = paths::container_dir(container_id);
    let rootfs = container_dir.join("rootfs");
    let upper = container_dir.join("upper");
    let work = container_dir.join("work");
    let merged = container_dir.join("merged");

    // Create directories
    for dir in [&rootfs, &upper, &work, &merged] {
        std::fs::create_dir_all(dir)
            .map_err(|e| CriError::Rootfs(format!("failed to create {}: {}", dir.display(), e)))?;
    }

    // Extract layers into rootfs
    for (i, layer) in image.layers.iter().enumerate() {
        if let Some(ref local_path) = layer.local_path {
            let layer_dir = container_dir.join(format!("layer_{}", i));
            std::fs::create_dir_all(&layer_dir)
                .map_err(|e| CriError::Rootfs(format!("failed to create layer dir: {}", e)))?;
            extract_layer(local_path, &layer_dir)?;
        }
    }

    // On Linux: mount overlayfs
    #[cfg(target_os = "linux")]
    {
        let lower_dirs = overlay_lower_dirs(image, &container_dir);
        if lower_dirs.is_empty() {
            // overlayfs needs at least one lower directory (mount(2) fails
            // with EINVAL otherwise). With no extracted layer there is
            // nothing to stack: the empty `merged` directory is the rootfs.
            tracing::debug!(
                "no extracted layers for {} — rootfs is a plain directory",
                container_id
            );
        } else {
            let opts = overlay_options(&lower_dirs, &upper, &work);
            nix::mount::mount(
                Some("overlay"),
                &merged,
                Some("overlay"),
                nix::mount::MsFlags::empty(),
                Some(opts.as_str()),
            )
            .map_err(|e| CriError::Rootfs(format!("overlayfs mount failed: {}", e)))?;
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        tracing::warn!("overlayfs not available on this OS — using flat copy for rootfs");
        // On non-Linux, just use the last extracted layer as rootfs
    }

    Ok(merged)
}

/// The lower directories for the overlay mount: the directory of every layer
/// that was extracted (a layer without a local blob is skipped by
/// `assemble_rootfs` and has no directory), topmost layer first. overlayfs
/// stacks `lowerdir=a:b:c` with `a` on top, image layers are listed base
/// first.
#[cfg(target_os = "linux")]
fn overlay_lower_dirs(image: &OciImage, container_dir: &Path) -> Vec<PathBuf> {
    image
        .layers
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, layer)| layer.local_path.is_some())
        .map(|(i, _)| container_dir.join(format!("layer_{}", i)))
        .collect()
}

/// The option string for mounting an overlay of `lower_dirs` (topmost
/// first) with a writable `upper` layer and its `work` directory.
#[cfg(target_os = "linux")]
fn overlay_options(lower_dirs: &[PathBuf], upper: &Path, work: &Path) -> String {
    let lower: Vec<String> = lower_dirs
        .iter()
        .map(|dir| dir.display().to_string())
        .collect();
    format!(
        "lowerdir={},upperdir={},workdir={}",
        lower.join(":"),
        upper.display(),
        work.display()
    )
}

/// Extract a tar.gz layer into a directory, handling OCI whiteout files.
///
/// Whiteout semantics (OCI spec §7.3.2):
///  - `.wh.<name>`        — delete file/dir `<name>` from lower layers
///  - `.wh..wh..opq`     — opaque whiteout: ignore all lower-layer contents for this dir
///
/// # Containment
///
/// A layer is untrusted input: nothing in it may create, change or delete a
/// path outside `target_dir`. A member that would do so makes the extraction
/// fail with an error that names the member. It is not skipped: skipping
/// would produce a root filesystem that silently differs from the image, and
/// a layer that tries to leave its directory is malformed or hostile either
/// way, so the image is refused as a whole.
///
/// Refused are:
///  - a member name that is absolute or has a `..` component. Names are not
///    normalised, so `a/../b` is refused as well;
///  - a member whose directory is, after resolving symlinks, not inside
///    `target_dir` (a symlink that an earlier member pointed elsewhere), or
///    cannot be resolved at all;
///  - a hard link whose link name does not resolve to a path inside
///    `target_dir`;
///  - a whiteout whose name after `.wh.` is empty, `.` or `..`: it would
///    remove its own directory or the parent of it.
///
/// A symlink member is stored as it is, wherever it points; it is only
/// never followed out of `target_dir`. A whiteout removes the entry it
/// names, never what a symlink of that name points to.
///
/// Files, directories and links are written by [`tar::Entry::unpack_in`],
/// which does the symlink and hard link checks; the checks for member names
/// and for whiteouts, which `tar` knows nothing about, are those of
/// `crate::containment`, which `diff::walking_differ` uses as well. As in
/// the `tar` crate, a process that changes `target_dir` while a layer is
/// being extracted is not defended against.
pub(crate) fn extract_layer(archive_path: &Path, target_dir: &Path) -> CriResult<()> {
    let file = std::fs::File::open(archive_path)
        .map_err(|e| CriError::Rootfs(format!("cannot open layer: {}", e)))?;

    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);

    // Every containment check below compares resolved paths with this one.
    let layer_root = std::fs::create_dir_all(target_dir)
        .and_then(|()| target_dir.canonicalize())
        .map_err(|e| {
            CriError::Rootfs(format!(
                "cannot resolve layer directory {}: {}",
                target_dir.display(),
                e
            ))
        })?;
    let layer_root = layer_root.as_path();

    for entry in archive
        .entries()
        .map_err(|e| CriError::Rootfs(format!("layer read failed: {}", e)))?
    {
        let mut entry = entry.map_err(|e| CriError::Rootfs(format!("layer entry error: {}", e)))?;

        let entry_path = entry
            .path()
            .map_err(|e| CriError::Rootfs(format!("entry path error: {}", e)))?
            .to_path_buf();

        containment::check_member_name(&entry_path).map_err(|why| rejected(&entry_path, why))?;

        let file_name = entry_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        if file_name == ".wh..wh..opq" {
            // Opaque whiteout: mark directory so overlay ignores lower layers
            // In a real overlayfs setup the kernel handles this via trusted.overlay.opaque xattr.
            // For our non-overlayfs fallback we mark it with a sentinel file.
            if let Some(dir) = containment::whiteout_dir(layer_root, &entry_path, true)
                .map_err(|why| rejected(&entry_path, why))?
            {
                // `create_new` does not follow a symlink that has the
                // marker's name. As before, a marker that cannot be written
                // is not an error.
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(dir.join(".wh..wh..opq"))
                    .ok();
            }
            continue;
        }

        if let Some(whiteout_name) = file_name.strip_prefix(".wh.") {
            // Whiteout: delete the named file/dir from this layer dir
            containment::check_whiteout_name(whiteout_name)
                .map_err(|why| rejected(&entry_path, why))?;
            if let Some(dir) = containment::whiteout_dir(layer_root, &entry_path, false)
                .map_err(|why| rejected(&entry_path, why))?
            {
                // Removes the entry itself, not what a symlink of that name
                // points to.
                containment::remove_entry(&dir.join(whiteout_name));
            }
            continue;
        }

        // Anything else is unpacked below `layer_root`. `unpack_in` creates
        // missing parent directories and refuses to follow a symlink out of
        // `layer_root`, for the member as well as for a hard link's target.
        match entry.unpack_in(layer_root) {
            Ok(true) => {}
            // `unpack_in` reports a member it skipped for its name this way.
            Ok(false) => {
                return Err(rejected(
                    &entry_path,
                    "it would be unpacked outside the layer directory",
                ));
            }
            Err(e) => {
                return Err(CriError::Rootfs(format!(
                    "layer extraction failed for {:?}: {}",
                    entry_path,
                    error_chain(&e)
                )));
            }
        }
    }

    Ok(())
}

/// The error for a layer member that is refused.
fn rejected(member: &Path, why: impl std::fmt::Display) -> CriError {
    CriError::Rootfs(format!("layer member {:?} rejected: {}", member, why))
}

/// `err` followed by its sources. The `tar` crate puts the reason for
/// refusing a member ("trying to unpack outside of destination path") into
/// the source of the error it returns.
fn error_chain(err: &dyn std::error::Error) -> String {
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// Cleanup rootfs and all container data.
pub fn cleanup_rootfs(container_id: &str) -> CriResult<()> {
    let container_dir = paths::container_dir(container_id);

    // Unmount overlayfs first
    #[cfg(target_os = "linux")]
    {
        let merged = container_dir.join("merged");
        if merged.exists() {
            let _ = nix::mount::umount(&merged);
        }
    }

    if container_dir.exists() {
        std::fs::remove_dir_all(&container_dir)
            .map_err(|e| CriError::Rootfs(format!("cleanup failed: {}", e)))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ImageConfig, OciImage, OciLayer};
    use chrono::Utc;
    use std::collections::BTreeMap;
    use std::os::unix::fs::MetadataExt;

    fn make_empty_image() -> OciImage {
        OciImage {
            reference: "test:latest".into(),
            digest: "sha256:empty".into(),
            layers: vec![],
            config: ImageConfig::default(),
            size_bytes: 0,
            pulled_at: Utc::now(),
        }
    }

    #[test]
    fn test_container_root_path() {
        let _root = crate::paths::tests::lock_root_env();
        let path = paths::container_dir("test123").join("merged");
        assert!(path.to_string_lossy().contains("test123"));
    }

    #[test]
    fn test_container_root_path_structure() {
        // Default root is /var/lib/cave when no override is set.
        let _root = crate::paths::tests::lock_root_env();
        let prev = std::env::var("CAVE_ROOT_DIR").ok();
        std::env::remove_var("CAVE_ROOT_DIR");
        let path = paths::container_dir("abc");
        assert!(path.starts_with("/var/lib/cave/containers"));
        if let Some(v) = prev {
            std::env::set_var("CAVE_ROOT_DIR", v);
        }
    }

    #[test]
    fn test_assemble_rootfs_empty_layers_creates_dirs() {
        // With no layers, assemble_rootfs should still attempt to create dirs.
        // On systems without /var/lib/cave, this will fail with Rootfs error.
        // We just verify the error variant is correct if it fails.
        let _root = crate::paths::tests::lock_root_env();
        let image = make_empty_image();
        let result = assemble_rootfs(&image, "empty-layer-test");
        match result {
            Ok(path) => {
                // If the system allowed dir creation, the merged path is returned
                assert!(path.to_string_lossy().contains("merged"));
                // Cleanup
                let _ = cleanup_rootfs("empty-layer-test");
            }
            Err(crate::error::CriError::Rootfs(_)) => {
                // Expected on systems without /var/lib/cave
            }
            Err(e) => panic!("unexpected error type: {:?}", e),
        }
    }

    #[test]
    fn test_assemble_rootfs_layer_without_local_path() {
        // Layers with no local_path are skipped (no extraction attempted).
        let _root = crate::paths::tests::lock_root_env();
        let mut image = make_empty_image();
        image.layers.push(OciLayer {
            digest: "sha256:abc".into(),
            size: 100,
            media_type: "application/vnd.oci.image.layer.v1.tar+gzip".into(),
            local_path: None, // no local file
        });
        let result = assemble_rootfs(&image, "no-local-path-test");
        match result {
            Ok(path) => {
                assert!(path.to_string_lossy().contains("merged"));
                let _ = cleanup_rootfs("no-local-path-test");
            }
            Err(crate::error::CriError::Rootfs(_)) => {}
            Err(e) => panic!("unexpected error: {:?}", e),
        }
    }

    #[test]
    fn test_extract_layer_nonexistent_archive() {
        let target = std::env::temp_dir().join("cave-test-extract");
        std::fs::create_dir_all(&target).ok();
        let result = extract_layer(Path::new("/nonexistent/layer.tar.gz"), &target);
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::error::CriError::Rootfs(msg) => assert!(msg.contains("cannot open layer")),
            e => panic!("wrong error: {:?}", e),
        }
    }

    #[test]
    fn test_extract_layer_corrupted_archive() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("bad.tar.gz");
        std::fs::write(&archive, b"this is not a valid gzip archive").unwrap();
        let target = dir.path().join("extracted");
        std::fs::create_dir_all(&target).unwrap();
        let result = extract_layer(&archive, &target);
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::error::CriError::Rootfs(_) => {}
            e => panic!("expected Rootfs error, got: {:?}", e),
        }
    }

    #[test]
    fn test_cleanup_rootfs_nonexistent_container() {
        // Should succeed if directory doesn't exist
        let _root = crate::paths::tests::lock_root_env();
        let result = cleanup_rootfs("totally-nonexistent-container-id-xyz");
        assert!(result.is_ok());
    }

    fn make_tar_gz(dir: &tempfile::TempDir, entries: &[(&str, &[u8])]) -> PathBuf {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        let archive_path = dir.path().join("layer.tar.gz");
        let file = std::fs::File::create(&archive_path).unwrap();
        let gz = GzEncoder::new(file, Compression::default());
        let mut tar = tar::Builder::new(gz);
        for (name, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, name, *data).unwrap();
        }
        tar.finish().unwrap();
        archive_path
    }

    #[test]
    fn test_extract_layer_regular_files() {
        let dir = tempfile::tempdir().unwrap();
        let archive = make_tar_gz(&dir, &[("hello.txt", b"hello world")]);
        let target = dir.path().join("extracted");
        std::fs::create_dir_all(&target).unwrap();
        extract_layer(&archive, &target).unwrap();
        assert!(target.join("hello.txt").exists());
        assert_eq!(
            std::fs::read_to_string(target.join("hello.txt")).unwrap(),
            "hello world"
        );
    }

    #[test]
    fn test_extract_layer_whiteout_deletes_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("extracted");
        std::fs::create_dir_all(&target).unwrap();
        // Pre-create file that whiteout will delete
        std::fs::write(target.join("todelete.txt"), b"old").unwrap();
        // Layer with whiteout
        let archive = make_tar_gz(&dir, &[(".wh.todelete.txt", b"")]);
        extract_layer(&archive, &target).unwrap();
        assert!(
            !target.join("todelete.txt").exists(),
            "whiteout should have deleted file"
        );
    }

    #[test]
    fn test_extract_layer_whiteout_missing_file_ok() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("extracted");
        std::fs::create_dir_all(&target).unwrap();
        let archive = make_tar_gz(&dir, &[(".wh.nonexistent.txt", b"")]);
        // Should not error even if target doesn't exist
        assert!(extract_layer(&archive, &target).is_ok());
    }

    #[test]
    fn test_extract_layer_opaque_whiteout_creates_sentinel() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("extracted");
        std::fs::create_dir_all(&target).unwrap();
        let archive = make_tar_gz(&dir, &[(".wh..wh..opq", b"")]);
        extract_layer(&archive, &target).unwrap();
        assert!(
            target.join(".wh..wh..opq").exists(),
            "opaque whiteout sentinel should exist"
        );
    }

    #[test]
    fn test_extract_layer_whiteout_in_subdir() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("extracted");
        std::fs::create_dir_all(target.join("subdir")).unwrap();
        std::fs::write(target.join("subdir").join("old.txt"), b"old").unwrap();
        let archive = make_tar_gz(&dir, &[("subdir/.wh.old.txt", b"")]);
        extract_layer(&archive, &target).unwrap();
        assert!(!target.join("subdir").join("old.txt").exists());
    }

    // ── containment: a layer must stay inside its layer directory ────────────
    //
    // These tests run the real `extract_layer` on archives built in memory.
    // The layer directory is `<tmp>/layer`; next to it `<tmp>/outside` holds
    // sentinel files and stands for the rest of the host. A layer that tries
    // to reach outside must be refused with an error that names the member,
    // and nothing outside the layer directory may be created, changed or
    // deleted. None of this needs privileges.

    /// The scratch tree of one test.
    struct Scratch {
        _tmp: tempfile::TempDir,
        /// The crafted archive is kept in a directory of its own.
        archives: tempfile::TempDir,
        /// `<tmp>`, with symlinks resolved.
        root: PathBuf,
        /// `<tmp>/layer`: the directory handed to `extract_layer`.
        layer: PathBuf,
        /// `<tmp>/outside`: `sentinel.txt` and `keep/inner.txt`.
        outside: PathBuf,
    }

    impl Scratch {
        fn new() -> Self {
            let tmp = tempfile::tempdir().unwrap();
            let root = tmp.path().canonicalize().unwrap();
            let layer = root.join("layer");
            let outside = root.join("outside");
            std::fs::create_dir(&layer).unwrap();
            std::fs::create_dir_all(outside.join("keep")).unwrap();
            std::fs::write(outside.join("sentinel.txt"), b"host data").unwrap();
            std::fs::write(outside.join("keep").join("inner.txt"), b"host data").unwrap();
            Scratch {
                _tmp: tmp,
                archives: tempfile::tempdir().unwrap(),
                root,
                layer,
                outside,
            }
        }

        /// Absolute path of an entry of the `outside` directory, as text.
        fn outside_path(&self, name: &str) -> String {
            self.outside.join(name).to_str().unwrap().to_string()
        }

        /// Everything in the scratch tree except the layer directory: path →
        /// kind, mode, link count and content. Equal snapshots before and
        /// after an extraction mean that nothing outside the layer directory
        /// was created, changed or deleted.
        fn snapshot_outside(&self) -> BTreeMap<PathBuf, String> {
            fn walk(dir: &Path, skip: &Path, seen: &mut BTreeMap<PathBuf, String>) {
                for entry in std::fs::read_dir(dir).unwrap() {
                    let path = entry.unwrap().path();
                    if path == skip {
                        continue;
                    }
                    let meta = std::fs::symlink_metadata(&path).unwrap();
                    let state = if meta.file_type().is_symlink() {
                        format!("symlink to {:?}", std::fs::read_link(&path).unwrap())
                    } else if meta.is_dir() {
                        walk(&path, skip, seen);
                        format!("dir, mode {:o}", meta.mode())
                    } else {
                        format!(
                            "file, mode {:o}, {} link(s), content {:?}",
                            meta.mode(),
                            meta.nlink(),
                            String::from_utf8_lossy(&std::fs::read(&path).unwrap())
                        )
                    };
                    seen.insert(path, state);
                }
            }
            let mut seen = BTreeMap::new();
            let root_mode = std::fs::symlink_metadata(&self.root).unwrap().mode();
            seen.insert(self.root.clone(), format!("dir, mode {:o}", root_mode));
            walk(&self.root, &self.layer, &mut seen);
            seen
        }
    }

    /// One archive member. Its header bytes are filled in directly, because
    /// `tar::Builder::append_data` and `tar::Header::set_path` refuse some of
    /// the names these tests have to feed to `extract_layer`.
    struct RawMember<'a> {
        name: &'a str,
        kind: tar::EntryType,
        link: &'a str,
        mode: u32,
        data: &'a [u8],
    }

    fn file<'a>(name: &'a str, data: &'a [u8]) -> RawMember<'a> {
        RawMember {
            name,
            kind: tar::EntryType::Regular,
            link: "",
            mode: 0o644,
            data,
        }
    }

    fn dir(name: &str) -> RawMember<'_> {
        RawMember {
            name,
            kind: tar::EntryType::Directory,
            link: "",
            mode: 0o755,
            data: b"",
        }
    }

    fn symlink<'a>(name: &'a str, target: &'a str) -> RawMember<'a> {
        RawMember {
            name,
            kind: tar::EntryType::Symlink,
            link: target,
            mode: 0o777,
            data: b"",
        }
    }

    fn hard_link<'a>(name: &'a str, target: &'a str) -> RawMember<'a> {
        RawMember {
            name,
            kind: tar::EntryType::Link,
            link: target,
            mode: 0o644,
            data: b"",
        }
    }

    /// Write `members` as a gzipped tar archive, in the order given.
    fn raw_layer(scratch: &Scratch, members: &[RawMember<'_>]) -> PathBuf {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        let archive_path = scratch.archives.path().join("layer.tar.gz");
        let gz = GzEncoder::new(
            std::fs::File::create(&archive_path).unwrap(),
            Compression::default(),
        );
        let mut tar = tar::Builder::new(gz);
        for member in members {
            let mut header = tar::Header::new_ustar();
            {
                let raw = header.as_old_mut();
                assert!(
                    member.name.len() <= raw.name.len() && member.link.len() <= raw.linkname.len(),
                    "{:?} -> {:?} does not fit a plain tar header",
                    member.name,
                    member.link
                );
                raw.name[..member.name.len()].copy_from_slice(member.name.as_bytes());
                raw.linkname[..member.link.len()].copy_from_slice(member.link.as_bytes());
            }
            header.set_entry_type(member.kind);
            header.set_mode(member.mode);
            header.set_uid(0);
            header.set_gid(0);
            header.set_mtime(1_700_000_000);
            header.set_size(member.data.len() as u64);
            header.set_cksum();
            tar.append(&header, member.data).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();
        archive_path
    }

    /// Extract `members` as one layer and assert that the layer is refused
    /// with an error naming `offender`, and that nothing outside the layer
    /// directory was created, changed or deleted. Returns the error text.
    fn assert_layer_rejected(
        scratch: &Scratch,
        members: &[RawMember<'_>],
        offender: &str,
    ) -> String {
        let archive = raw_layer(scratch, members);
        let before = scratch.snapshot_outside();
        let result = extract_layer(&archive, &scratch.layer);
        let after = scratch.snapshot_outside();
        assert_eq!(
            before, after,
            "extracting {offender:?} touched paths outside the layer directory \
             (extract_layer returned {result:?})"
        );
        match result {
            Err(CriError::Rootfs(msg)) => {
                assert!(
                    msg.contains(offender),
                    "the error does not name the member {offender:?}: {msg}"
                );
                msg
            }
            other => panic!("a layer with {offender:?} must be rejected, got {other:?}"),
        }
    }

    /// `assert_layer_rejected` for a member that would land outside the
    /// layer directory: the error also has to say so.
    fn assert_escape_rejected(scratch: &Scratch, members: &[RawMember<'_>], offender: &str) {
        let msg = assert_layer_rejected(scratch, members, offender);
        assert!(
            msg.contains("outside"),
            "the error for {offender:?} does not say that it leads outside: {msg}"
        );
    }

    #[test]
    fn extract_layer_rejects_parent_dir_members() {
        for name in [
            "../escaped.txt",
            "../outside/sentinel.txt",
            "sub/../../escaped.txt",
            "./../outside/keep/inner.txt",
        ] {
            let scratch = Scratch::new();
            assert_escape_rejected(&scratch, &[file(name, b"from the layer")], name);
        }
    }

    /// A directory member has no content to write, but it has a mode.
    #[test]
    fn extract_layer_rejects_parent_dir_directory_member() {
        let scratch = Scratch::new();
        let mut member = dir("..");
        member.mode = 0o700;
        assert_escape_rejected(&scratch, &[member], "..");
    }

    /// `..` is rejected as such, also where the name would resolve to a
    /// path inside the layer directory: member names are not normalised.
    #[test]
    fn extract_layer_rejects_parent_dir_component_that_stays_inside() {
        let scratch = Scratch::new();
        let name = "etc/../hostname";
        assert_layer_rejected(&scratch, &[file(name, b"cave\n")], name);
        assert!(!scratch.layer.join("hostname").exists());
    }

    #[test]
    fn extract_layer_rejects_absolute_members() {
        for target in ["sentinel.txt", "created.txt", "keep/created.txt"] {
            let scratch = Scratch::new();
            let name = scratch.outside_path(target);
            assert_escape_rejected(&scratch, &[file(&name, b"from the layer")], &name);
        }
        let scratch = Scratch::new();
        let name = scratch.outside_path("keep");
        let mut member = dir(&name);
        member.mode = 0o700;
        assert_escape_rejected(&scratch, &[member], &name);
    }

    /// A symlink member may point anywhere: it is data. What must not happen
    /// is that a later member is created by following it out of the layer.
    #[test]
    fn extract_layer_rejects_write_through_symlink_pointing_outside() {
        for relative_target in [false, true] {
            for (through, is_dir) in [
                ("escape/planted.txt", false),
                ("escape/sentinel.txt", false),
                ("escape/sub/deeper/planted.txt", false),
                ("escape/newdir", true),
                ("escape/keep", true),
            ] {
                let scratch = Scratch::new();
                let absolute = scratch.outside_path("");
                let target = if relative_target {
                    "../outside"
                } else {
                    absolute.as_str()
                };
                let mut second = if is_dir {
                    dir(through)
                } else {
                    file(through, b"from the layer")
                };
                if is_dir {
                    second.mode = 0o700;
                }
                assert_escape_rejected(&scratch, &[symlink("escape", target), second], through);
            }
        }
    }

    #[test]
    fn extract_layer_rejects_hard_link_to_target_outside() {
        // Absolute link name.
        let scratch = Scratch::new();
        let target = scratch.outside_path("sentinel.txt");
        assert_escape_rejected(&scratch, &[hard_link("stolen", &target)], "stolen");
        assert!(std::fs::symlink_metadata(scratch.layer.join("stolen")).is_err());

        // Link name that climbs out of the layer directory.
        let scratch = Scratch::new();
        assert_escape_rejected(
            &scratch,
            &[hard_link("sub/stolen", "../outside/sentinel.txt")],
            "sub/stolen",
        );
        assert!(std::fs::symlink_metadata(scratch.layer.join("sub/stolen")).is_err());

        // Link name that is a symlink inside the layer, pointing outside.
        let scratch = Scratch::new();
        let target = scratch.outside_path("sentinel.txt");
        assert_escape_rejected(
            &scratch,
            &[symlink("peek", &target), hard_link("stolen", "peek")],
            "stolen",
        );
        assert!(std::fs::symlink_metadata(scratch.layer.join("stolen")).is_err());
    }

    #[test]
    fn extract_layer_rejects_whiteout_with_escaping_path() {
        for name in [
            "../outside/.wh.sentinel.txt",
            "../outside/.wh.keep",
            "../.wh.outside",
            "sub/../../outside/keep/.wh.inner.txt",
        ] {
            let scratch = Scratch::new();
            assert_escape_rejected(&scratch, &[file(name, b"")], name);
        }
        for name in [".wh.sentinel.txt", ".wh.keep"] {
            let scratch = Scratch::new();
            let name = scratch.outside_path(name);
            assert_escape_rejected(&scratch, &[file(&name, b"")], &name);
        }
    }

    /// The name after `.wh.` has to be one entry of the whiteout's own
    /// directory. `.wh...` would name the parent of the layer directory.
    #[test]
    fn extract_layer_rejects_whiteout_naming_the_parent_directory() {
        let scratch = Scratch::new();
        assert_escape_rejected(&scratch, &[file(".wh...", b"")], ".wh...");
    }

    /// `.wh.` and `.wh..` name the directory the whiteout itself is in.
    #[test]
    fn extract_layer_rejects_whiteout_naming_its_own_directory() {
        for name in [".wh.", ".wh..", "etc/.wh.", "etc/.wh.."] {
            let scratch = Scratch::new();
            std::fs::create_dir(scratch.layer.join("etc")).unwrap();
            std::fs::write(scratch.layer.join("etc/hostname"), b"cave\n").unwrap();
            let msg = assert_layer_rejected(&scratch, &[file(name, b"")], name);
            assert!(msg.contains("whiteout"), "unexpected error: {msg}");
            assert_eq!(
                std::fs::read(scratch.layer.join("etc/hostname")).unwrap(),
                b"cave\n",
                "{name:?} removed the contents of the layer directory"
            );
        }
    }

    #[test]
    fn extract_layer_rejects_opaque_marker_in_escaping_directory() {
        for name in [
            "../.wh..wh..opq",
            "../outside/.wh..wh..opq",
            "../outside/keep/.wh..wh..opq",
            "../created/deeper/.wh..wh..opq",
        ] {
            let scratch = Scratch::new();
            assert_escape_rejected(&scratch, &[file(name, b"")], name);
        }
        for name in [".wh..wh..opq", "created/.wh..wh..opq"] {
            let scratch = Scratch::new();
            let name = scratch.outside_path(name);
            assert_escape_rejected(&scratch, &[file(&name, b"")], &name);
        }
    }

    #[test]
    fn extract_layer_rejects_whiteout_reached_through_symlinked_directory() {
        for relative_target in [false, true] {
            for through in [
                "escape/.wh.sentinel.txt",
                "escape/.wh.keep",
                "escape/keep/.wh.inner.txt",
                "escape/.wh..wh..opq",
                "escape/keep/.wh..wh..opq",
                "escape/created/.wh..wh..opq",
            ] {
                let scratch = Scratch::new();
                let absolute = scratch.outside_path("");
                let target = if relative_target {
                    "../outside"
                } else {
                    absolute.as_str()
                };
                assert_escape_rejected(
                    &scratch,
                    &[symlink("escape", target), file(through, b"")],
                    through,
                );
            }
        }
    }

    /// The opaque marker is a new, empty file: an entry that already has
    /// its name is not written through.
    #[test]
    fn extract_layer_opaque_marker_does_not_write_through_an_existing_symlink() {
        let scratch = Scratch::new();
        std::os::unix::fs::symlink(
            scratch.outside.join("sentinel.txt"),
            scratch.layer.join(".wh..wh..opq"),
        )
        .unwrap();
        let archive = raw_layer(&scratch, &[file(".wh..wh..opq", b"")]);
        let before = scratch.snapshot_outside();

        extract_layer(&archive, &scratch.layer).unwrap();

        assert_eq!(before, scratch.snapshot_outside());
    }

    /// A whiteout removes the entry it names. If that entry is a symlink,
    /// the link goes away and what it points to stays.
    #[test]
    fn extract_layer_whiteout_of_a_symlink_removes_the_link_not_its_target() {
        let scratch = Scratch::new();
        let outside_dir = scratch.outside_path("keep");
        let outside_file = scratch.outside_path("sentinel.txt");
        let archive = raw_layer(
            &scratch,
            &[
                symlink("to-dir", &outside_dir),
                symlink("to-file", &outside_file),
                file(".wh.to-dir", b""),
                file(".wh.to-file", b""),
            ],
        );
        let before = scratch.snapshot_outside();

        extract_layer(&archive, &scratch.layer).unwrap();

        assert_eq!(before, scratch.snapshot_outside());
        assert!(std::fs::symlink_metadata(scratch.layer.join("to-dir")).is_err());
        assert!(std::fs::symlink_metadata(scratch.layer.join("to-file")).is_err());
    }

    /// A regular member whose name is taken by a symlink replaces the link.
    #[test]
    fn extract_layer_file_member_replaces_a_symlink_instead_of_writing_through_it() {
        let scratch = Scratch::new();
        let outside_file = scratch.outside_path("sentinel.txt");
        let archive = raw_layer(
            &scratch,
            &[
                symlink("motd", &outside_file),
                file("motd", b"from the layer"),
            ],
        );
        let before = scratch.snapshot_outside();

        extract_layer(&archive, &scratch.layer).unwrap();

        assert_eq!(before, scratch.snapshot_outside());
        let motd = scratch.layer.join("motd");
        assert!(std::fs::symlink_metadata(&motd).unwrap().is_file());
        assert_eq!(std::fs::read(&motd).unwrap(), b"from the layer");
    }

    // ── legitimate layers keep extracting ─────────────────────────────────────

    #[test]
    fn extract_layer_keeps_legitimate_members_working() {
        let scratch = Scratch::new();
        let archive = raw_layer(
            &scratch,
            &[
                dir("./"),
                dir("etc/"),
                file("etc/hostname", b"cave\n"),
                // Parent directories that have no member of their own.
                file("usr/lib/libx.so", b"x"),
                file("./opt/tool", b"tool"),
                // Symlinks are stored as they are, wherever they point.
                symlink("etc/alias", "hostname"),
                symlink("etc/mtab", "/proc/mounts"),
                symlink("up", "../outside"),
                // A member reached through a symlink that stays inside.
                symlink("lib", "usr/lib"),
                file("lib/liby.so", b"y"),
            ],
        );
        let before = scratch.snapshot_outside();

        extract_layer(&archive, &scratch.layer).unwrap();

        assert_eq!(before, scratch.snapshot_outside());
        let layer = &scratch.layer;
        assert!(layer.join("etc").is_dir());
        assert_eq!(
            std::fs::read(layer.join("etc/hostname")).unwrap(),
            b"cave\n"
        );
        assert_eq!(std::fs::read(layer.join("usr/lib/libx.so")).unwrap(), b"x");
        assert_eq!(std::fs::read(layer.join("opt/tool")).unwrap(), b"tool");
        assert_eq!(
            std::fs::read_link(layer.join("etc/alias")).unwrap(),
            Path::new("hostname")
        );
        assert_eq!(std::fs::read(layer.join("etc/alias")).unwrap(), b"cave\n");
        assert_eq!(
            std::fs::read_link(layer.join("etc/mtab")).unwrap(),
            Path::new("/proc/mounts")
        );
        assert_eq!(
            std::fs::read_link(layer.join("up")).unwrap(),
            Path::new("../outside")
        );
        assert_eq!(
            std::fs::read_link(layer.join("lib")).unwrap(),
            Path::new("usr/lib")
        );
        assert_eq!(std::fs::read(layer.join("usr/lib/liby.so")).unwrap(), b"y");
    }

    /// The link name of a hard link member is a path in the layer, not a
    /// path relative to the working directory of the process.
    #[test]
    fn extract_layer_hard_link_inside_the_layer() {
        let scratch = Scratch::new();
        let archive = raw_layer(
            &scratch,
            &[
                file("bin/busybox", b"applet"),
                hard_link("bin/sh", "bin/busybox"),
                hard_link("sbin/init", "./bin/busybox"),
            ],
        );
        let before = scratch.snapshot_outside();

        extract_layer(&archive, &scratch.layer).unwrap();

        assert_eq!(before, scratch.snapshot_outside());
        let original = std::fs::metadata(scratch.layer.join("bin/busybox")).unwrap();
        for link in ["bin/sh", "sbin/init"] {
            let linked = std::fs::metadata(scratch.layer.join(link)).unwrap();
            assert_eq!(linked.ino(), original.ino(), "{link} is not a hard link");
        }
        assert_eq!(original.nlink(), 3);
    }

    #[test]
    fn extract_layer_whiteouts_inside_the_layer_still_apply() {
        let scratch = Scratch::new();
        let layer = &scratch.layer;
        std::fs::create_dir_all(layer.join("etc")).unwrap();
        std::fs::write(layer.join("etc/old.conf"), b"old").unwrap();
        std::fs::write(layer.join("etc/kept.conf"), b"kept").unwrap();
        std::fs::create_dir_all(layer.join("var/cache/deep")).unwrap();
        std::fs::write(layer.join("var/cache/deep/blob"), b"old").unwrap();
        std::fs::create_dir_all(layer.join("usr/lib")).unwrap();
        std::fs::write(layer.join("usr/lib/old.so"), b"old").unwrap();
        let archive = raw_layer(
            &scratch,
            &[
                file("etc/.wh.old.conf", b""),
                // A whiteout removes a whole directory tree.
                file("./var/.wh.cache", b""),
                // Nothing to remove, not even the directory: no effect.
                file("tmp/.wh.missing", b""),
                // The directory of an opaque marker is created if needed.
                file("srv/data/.wh..wh..opq", b""),
                file("etc/.wh..wh..opq", b""),
                // A directory reached through a symlink that stays inside.
                symlink("lib", "usr/lib"),
                file("lib/.wh.old.so", b""),
            ],
        );
        let before = scratch.snapshot_outside();

        extract_layer(&archive, layer).unwrap();

        assert_eq!(before, scratch.snapshot_outside());
        assert!(!layer.join("etc/old.conf").exists());
        assert_eq!(std::fs::read(layer.join("etc/kept.conf")).unwrap(), b"kept");
        assert!(layer.join("var").is_dir());
        assert!(!layer.join("var/cache").exists());
        assert!(!layer.join("tmp").exists());
        assert_eq!(
            std::fs::read(layer.join("srv/data/.wh..wh..opq")).unwrap(),
            b""
        );
        assert_eq!(std::fs::read(layer.join("etc/.wh..wh..opq")).unwrap(), b"");
        assert!(layer.join("usr/lib").is_dir());
        assert!(!layer.join("usr/lib/old.so").exists());
    }

    // ── overlay mount arguments (Linux) ───────────────────────────────────────
    //
    // The mount itself needs CAP_SYS_ADMIN; what is passed to it is computed
    // by two pure functions that are tested here.

    #[cfg(target_os = "linux")]
    fn image_with_blobs(has_blob: &[bool]) -> OciImage {
        let mut image = make_empty_image();
        for (i, has_blob) in has_blob.iter().enumerate() {
            image.layers.push(OciLayer {
                digest: format!("sha256:{i}"),
                size: 1,
                media_type: "application/vnd.oci.image.layer.v1.tar+gzip".into(),
                local_path: has_blob.then(|| PathBuf::from(format!("/blobs/{i}.tar.gz"))),
            });
        }
        image
    }

    /// overlayfs stacks `lowerdir=a:b:c` with `a` on top. Image layers are
    /// listed base first, so the last layer has to come first.
    #[cfg(target_os = "linux")]
    #[test]
    fn overlay_lower_dirs_lists_the_extracted_layers_topmost_first() {
        let image = image_with_blobs(&[true, true, true]);

        let dirs = overlay_lower_dirs(&image, Path::new("/state/containers/c1"));

        assert_eq!(
            dirs,
            [
                PathBuf::from("/state/containers/c1/layer_2"),
                PathBuf::from("/state/containers/c1/layer_1"),
                PathBuf::from("/state/containers/c1/layer_0"),
            ]
        );
    }

    /// A layer without a local blob is never extracted, so its directory
    /// does not exist; naming it would make mount(2) fail with ENOENT.
    #[cfg(target_os = "linux")]
    #[test]
    fn overlay_lower_dirs_leaves_out_layers_that_were_not_extracted() {
        let image = image_with_blobs(&[true, false, true, false]);

        let dirs = overlay_lower_dirs(&image, Path::new("/c"));

        assert_eq!(
            dirs,
            [PathBuf::from("/c/layer_2"), PathBuf::from("/c/layer_0")]
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn overlay_lower_dirs_is_empty_when_nothing_was_extracted() {
        assert!(overlay_lower_dirs(&make_empty_image(), Path::new("/c")).is_empty());
        assert!(overlay_lower_dirs(&image_with_blobs(&[false, false]), Path::new("/c")).is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn overlay_options_name_the_lower_upper_and_work_directories() {
        let lower = [PathBuf::from("/c/layer_1"), PathBuf::from("/c/layer_0")];

        let opts = overlay_options(&lower, Path::new("/c/upper"), Path::new("/c/work"));

        assert_eq!(
            opts,
            "lowerdir=/c/layer_1:/c/layer_0,upperdir=/c/upper,workdir=/c/work"
        );
    }

    #[test]
    fn test_cleanup_rootfs_removes_created_dir() {
        let _root = crate::paths::tests::lock_root_env();
        let container_id = "cleanup-test-container";
        let container_dir = paths::container_dir(container_id);

        if std::fs::create_dir_all(&container_dir).is_ok() {
            let result = cleanup_rootfs(container_id);
            assert!(result.is_ok());
            assert!(!container_dir.exists());
        } else {
            // No write permission to /var/lib/cave — just verify cleanup on missing path is ok
            let result = cleanup_rootfs(container_id);
            assert!(result.is_ok());
        }
    }
}
