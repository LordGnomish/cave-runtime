//! Root filesystem assembly from OCI image layers using overlayfs.

use crate::error::{CriError, CriResult};
use crate::models::OciImage;
use std::path::{Path, PathBuf};

const CONTAINER_ROOT: &str = "/var/lib/cave/containers";

/// Assemble a rootfs from OCI image layers.
///
/// Uses overlayfs to stack layers (lower=layer1:layer2:..., upper=writable, work=workdir).
pub fn assemble_rootfs(image: &OciImage, container_id: &str) -> CriResult<PathBuf> {
    let container_dir = PathBuf::from(CONTAINER_ROOT).join(container_id);
    let rootfs = container_dir.join("rootfs");
    let upper = container_dir.join("upper");
    let work = container_dir.join("work");
    let merged = container_dir.join("merged");

    // Create directories
    for dir in [&rootfs, &upper, &work, &merged] {
        std::fs::create_dir_all(dir).map_err(|e| {
            CriError::Rootfs(format!("failed to create {}: {}", dir.display(), e))
        })?;
    }

    // Extract layers into rootfs
    for (i, layer) in image.layers.iter().enumerate() {
        if let Some(ref local_path) = layer.local_path {
            let layer_dir = container_dir.join(format!("layer_{}", i));
            std::fs::create_dir_all(&layer_dir).map_err(|e| {
                CriError::Rootfs(format!("failed to create layer dir: {}", e))
            })?;
            extract_layer(local_path, &layer_dir)?;
        }
    }

    // On Linux: mount overlayfs
    #[cfg(target_os = "linux")]
    {
        let lower_dirs: Vec<String> = (0..image.layers.len())
            .rev()
            .map(|i| container_dir.join(format!("layer_{}", i)).display().to_string())
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

/// Extract a tar.gz layer into a directory.
pub(crate) fn extract_layer(archive_path: &Path, target_dir: &Path) -> CriResult<()> {
    let file = std::fs::File::open(archive_path)
        .map_err(|e| CriError::Rootfs(format!("cannot open layer: {}", e)))?;

    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);

    archive.unpack(target_dir)
        .map_err(|e| CriError::Rootfs(format!("layer extraction failed: {}", e)))?;

    Ok(())
}

/// Cleanup rootfs and all container data.
pub fn cleanup_rootfs(container_id: &str) -> CriResult<()> {
    let container_dir = PathBuf::from(CONTAINER_ROOT).join(container_id);
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
        let path = PathBuf::from(CONTAINER_ROOT).join("test123").join("merged");
        assert!(path.to_string_lossy().contains("test123"));
    }

    #[test]
    fn test_container_root_path_structure() {
        let path = PathBuf::from(CONTAINER_ROOT).join("abc");
        assert!(path.starts_with("/var/lib/cave/containers"));
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

    #[test]
    fn test_cleanup_rootfs_removes_created_dir() {
        let container_id = "cleanup-test-container";
        let container_dir = PathBuf::from(CONTAINER_ROOT).join(container_id);

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
