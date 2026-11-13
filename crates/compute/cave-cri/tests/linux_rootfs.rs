// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Linux code paths of `cave_cri::rootfs`, exercised without privileges.
//!
//! mount(2) needs `CAP_SYS_ADMIN`, so an overlay mount cannot be part of a
//! test that has to pass on an unprivileged runner. What can be tested is
//! everything around it: an image that gives overlayfs nothing to stack
//! must not reach mount(2) at all, and cleanup must cope with a rootfs
//! that was never mounted.
//!
//! overlayfs needs at least one lower directory: mount(2) fails with
//! `EINVAL` for an empty `lowerdir=` (the kernel logs "overlayfs: missing
//! 'lowerdir'") and with `ENOENT` for a lower directory that does not exist.
//! Both were observed on Linux 6.18. With real layers the mount is
//! attempted, and that path is not exercised by any test in this crate.

#![cfg(target_os = "linux")]

use cave_cri::models::{ImageConfig, OciImage, OciLayer};
use cave_cri::rootfs::{assemble_rootfs, cleanup_rootfs};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::Mutex;

/// `cave_cri::paths` reads `CAVE_ROOT_DIR` on every call, and the
/// environment is shared by the whole process: tests take this lock while
/// the variable points at their own tempdir.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Run `test` with cave-cri's state root set to a fresh tempdir.
fn with_state_root(test: impl FnOnce(&Path)) {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let root = tempfile::tempdir().unwrap();
    std::env::set_var("CAVE_ROOT_DIR", root.path());
    test(root.path());
    std::env::remove_var("CAVE_ROOT_DIR");
}

fn image(layers: Vec<OciLayer>) -> OciImage {
    OciImage {
        reference: "linux-rootfs-test:latest".into(),
        digest: "sha256:test".into(),
        layers,
        config: ImageConfig::default(),
        size_bytes: 0,
        pulled_at: chrono::Utc::now(),
    }
}

/// A mount point sits on a different device than its parent directory.
fn is_mount_point(dir: &Path) -> bool {
    let own = std::fs::metadata(dir).unwrap().dev();
    let parent = std::fs::metadata(dir.parent().unwrap()).unwrap().dev();
    own != parent
}

#[test]
fn an_image_without_layers_gets_a_plain_directory_as_rootfs() {
    with_state_root(|root| {
        let rootfs = assemble_rootfs(&image(vec![]), "ctr-empty").unwrap();

        let container_dir = root.join("containers").join("ctr-empty");
        assert_eq!(rootfs, container_dir.join("merged"));
        assert!(rootfs.is_dir());
        assert!(container_dir.join("upper").is_dir());
        assert!(container_dir.join("work").is_dir());
        assert!(
            !is_mount_point(&rootfs),
            "there is nothing to stack, so nothing may be mounted"
        );
        assert_eq!(std::fs::read_dir(&rootfs).unwrap().count(), 0);
    });
}

/// A layer without a local blob is not extracted, so its directory does not
/// exist and must not be named as a lower directory.
#[test]
fn a_layer_without_local_content_is_not_handed_to_overlayfs() {
    with_state_root(|root| {
        let not_downloaded = OciLayer {
            digest: "sha256:abc".into(),
            size: 100,
            media_type: "application/vnd.oci.image.layer.v1.tar+gzip".into(),
            local_path: None,
        };

        let rootfs = assemble_rootfs(&image(vec![not_downloaded]), "ctr-no-blob").unwrap();

        let container_dir = root.join("containers").join("ctr-no-blob");
        assert_eq!(rootfs, container_dir.join("merged"));
        assert!(!container_dir.join("layer_0").exists());
        assert!(!is_mount_point(&rootfs));
    });
}

/// cleanup_rootfs unmounts `merged` before deleting the container
/// directory. For a rootfs that was never mounted umount(2) fails (`EINVAL`
/// as root, `EPERM` otherwise); that must not get in the way of the delete.
#[test]
fn cleanup_removes_a_rootfs_that_was_never_mounted() {
    with_state_root(|root| {
        let rootfs = assemble_rootfs(&image(vec![]), "ctr-cleanup").unwrap();
        std::fs::write(rootfs.join("written-by-the-container"), b"data").unwrap();
        let container_dir = root.join("containers").join("ctr-cleanup");
        assert!(container_dir.is_dir());

        cleanup_rootfs("ctr-cleanup").unwrap();

        assert!(!container_dir.exists());
        assert!(
            root.join("containers").is_dir(),
            "only this container's directory is removed"
        );
    });
}
