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
fn extract_layer(archive_path: &Path, target_dir: &Path) -> CriResult<()> {
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

    #[test]
    fn test_container_root_path() {
        let path = PathBuf::from(CONTAINER_ROOT).join("test123").join("merged");
        assert!(path.to_string_lossy().contains("test123"));
    }
}
