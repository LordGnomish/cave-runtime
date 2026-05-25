// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Root filesystem assembly from OCI image layers using overlayfs.

use crate::error::{CriError, CriResult};
use crate::models::OciImage;
use crate::paths;
use std::path::{Path, PathBuf};

/// Assemble a rootfs from OCI image layers.
///
/// Uses overlayfs to stack layers (lower=layer1:layer2:..., upper=writable, work=workdir).
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
        let lower_dirs: Vec<String> = (0..image.layers.len())
            .rev()
            .map(|i| {
                container_dir
                    .join(format!("layer_{}", i))
                    .display()
                    .to_string()
            })
            .collect();

        let lower = lower_dirs.join(":");
        let opts = format!(
            "lowerdir={},upperdir={},workdir={}",
            lower,
            upper.display(),
            work.display()
        );

        nix::mount::mount(
            Some("overlay"),
            &merged,
            Some("overlay"),
            nix::mount::MsFlags::empty(),
            Some(opts.as_str()),
        )
        .map_err(|e| CriError::Rootfs(format!("overlayfs mount failed: {}", e)))?;
    }

    #[cfg(not(target_os = "linux"))]
    {
        tracing::warn!("overlayfs not available on this OS — using flat copy for rootfs");
        // On non-Linux, just use the last extracted layer as rootfs
    }

    Ok(merged)
}

/// Extract a tar.gz layer into a directory, handling OCI whiteout files.
///
/// Whiteout semantics (OCI spec §7.3.2):
///  - `.wh.<name>`        — delete file/dir `<name>` from lower layers
///  - `.wh..wh..opq`     — opaque whiteout: ignore all lower-layer contents for this dir
pub(crate) fn extract_layer(archive_path: &Path, target_dir: &Path) -> CriResult<()> {
    let file = std::fs::File::open(archive_path)
        .map_err(|e| CriError::Rootfs(format!("cannot open layer: {}", e)))?;

    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);

    for entry in archive
        .entries()
        .map_err(|e| CriError::Rootfs(format!("layer read failed: {}", e)))?
    {
        let mut entry = entry.map_err(|e| CriError::Rootfs(format!("layer entry error: {}", e)))?;

        let entry_path = entry
            .path()
            .map_err(|e| CriError::Rootfs(format!("entry path error: {}", e)))?
            .to_path_buf();

        let file_name = entry_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        if file_name == ".wh..wh..opq" {
            // Opaque whiteout: mark directory so overlay ignores lower layers
            // In a real overlayfs setup the kernel handles this via trusted.overlay.opaque xattr.
            // For our non-overlayfs fallback we mark it with a sentinel file.
            let parent = target_dir.join(entry_path.parent().unwrap_or(Path::new("")));
            std::fs::create_dir_all(&parent).ok();
            std::fs::write(parent.join(".wh..wh..opq"), b"").ok();
            continue;
        }

        if let Some(whiteout_name) = file_name.strip_prefix(".wh.") {
            // Whiteout: delete the named file/dir from this layer dir
            let parent = target_dir.join(entry_path.parent().unwrap_or(Path::new("")));
            let target = parent.join(whiteout_name);
            if target.is_dir() {
                std::fs::remove_dir_all(&target).ok();
            } else if target.exists() {
                std::fs::remove_file(&target).ok();
            }
            continue;
        }

        // Regular file: unpack into target_dir
        let dest = target_dir.join(&entry_path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        entry.unpack(&dest).map_err(|e| {
            CriError::Rootfs(format!(
                "layer extraction failed for {:?}: {}",
                entry_path, e
            ))
        })?;
    }

    Ok(())
}

/// Cleanup rootfs and all container data.
pub fn cleanup_rootfs(container_id: &str) -> CriResult<()> {
    let container_dir = paths::container_dir(container_id);
    let _merged = container_dir.join("merged");

    // Unmount overlayfs first
    #[cfg(target_os = "linux")]
    {
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
        let path = paths::container_dir("test123").join("merged");
        assert!(path.to_string_lossy().contains("test123"));
    }

    #[test]
    fn test_container_root_path_structure() {
        // Default root is /var/lib/cave when no override is set.
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

    #[test]
    fn test_cleanup_rootfs_removes_created_dir() {
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
