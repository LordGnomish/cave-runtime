// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CRIU integration — Checkpoint/Restore in Userspace, KEP-2008.
//!
//! KEP-2008 wires the existing CRIU tooling into the CRI as
//! `RuntimeService.CheckpointContainer`. cave-cri owns the plumbing:
//!
//! - Build the CRIU command line (`criu dump --tree <pid> --images-dir
//!   <dir> --shell-job --tcp-established …`) with the right flags for
//!   the requested options.
//! - Lay out the on-disk image directory under
//!   `<root>/checkpoints/<container_id>/{dump.log,…}`.
//! - Read back the produced manifest (`spec.dump.json`,
//!   `descriptor.json`) into a typed `CheckpointManifest` so tests can
//!   assert on what would have been written.
//! - Restore: build the matching `criu restore` command line plus
//!   `--restore-detached` / `--external` mappings.
//!
//! Upstream:
//! - KEP-2008:
//!   <https://github.com/kubernetes/enhancements/tree/master/keps/sig-node/2008-forensic-container-checkpointing>
//! - containerd: `pkg/cri/server/container_checkpoint.go` and
//!   `pkg/cri/server/container_restore.go`
//! - CRIU CLI:    <https://criu.org/CLI/usage>

use crate::error::{CriError, CriResult};
use crate::paths;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Directory layout inside `<root>/checkpoints/<container_id>/` matches
/// what `criu dump --images-dir` writes, plus the manifest containerd
/// writes alongside.
pub const MANIFEST_FILENAME: &str = "spec.dump.json";
pub const DESCRIPTOR_FILENAME: &str = "descriptor.json";

/// Options the kubelet passes via `CheckpointContainerRequest.options`.
/// Mirrors `runtime.v1.CheckpointContainerOptions`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointOptions {
    /// Keep the container alive after dumping. CRIU `--leave-running`.
    pub leave_running: bool,
    /// Allow open TCP connections to be checkpointed/restored.
    /// CRIU `--tcp-established`.
    pub tcp_established: bool,
    /// Allow checkpointing a process whose stdin/out/err is a pty
    /// (kubectl exec). CRIU `--shell-job`.
    pub shell_job: bool,
    /// Bind external mounts back into the container at restore time.
    /// CRIU `--ext-mount-map`.
    pub external_mounts: bool,
    /// Optional override for the on-disk images dir; default is
    /// `<root>/checkpoints/<container_id>/`.
    pub images_dir: Option<PathBuf>,
}

/// Result of a successful checkpoint: paths CRIU wrote and the parsed
/// manifest descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointResult {
    pub container_id: Uuid,
    pub images_dir: PathBuf,
    pub manifest: CheckpointManifest,
    pub created_at: DateTime<Utc>,
    pub size_bytes: u64,
}

/// Type that mirrors containerd's `pkg/cri/server/container_checkpoint.go`
/// `Manifest` struct — the JSON descriptor written next to the CRIU
/// images so a Restore call can verify it's looking at the right state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointManifest {
    pub container_id: Uuid,
    pub container_name: String,
    pub image_reference: String,
    pub runtime_handler: Option<String>,
    pub created_at: DateTime<Utc>,
    pub criu_version: String,
    pub options: CheckpointOptions,
}

/// Default `criu` binary on PATH.
pub const DEFAULT_CRIU_BINARY: &str = "criu";

/// Build the `criu dump` argv for a container PID and option set. Tests
/// assert against this string list to lock down the wire format without
/// actually invoking CRIU (which doesn't exist on macOS).
pub fn build_dump_argv(pid: u32, images_dir: &Path, options: &CheckpointOptions) -> Vec<String> {
    let mut argv = vec![
        DEFAULT_CRIU_BINARY.to_string(),
        "dump".to_string(),
        "--tree".to_string(),
        pid.to_string(),
        "--images-dir".to_string(),
        images_dir.display().to_string(),
        "--log-file".to_string(),
        images_dir.join("dump.log").display().to_string(),
    ];
    if options.leave_running {
        argv.push("--leave-running".to_string());
    }
    if options.tcp_established {
        argv.push("--tcp-established".to_string());
    }
    if options.shell_job {
        argv.push("--shell-job".to_string());
    }
    if options.external_mounts {
        argv.push("--ext-mount-map".to_string());
        argv.push("auto".to_string());
    }
    argv
}

/// Build the `criu restore` argv for a previously-dumped image set.
pub fn build_restore_argv(images_dir: &Path, options: &CheckpointOptions) -> Vec<String> {
    let mut argv = vec![
        DEFAULT_CRIU_BINARY.to_string(),
        "restore".to_string(),
        "--images-dir".to_string(),
        images_dir.display().to_string(),
        "--log-file".to_string(),
        images_dir.join("restore.log").display().to_string(),
        "--restore-detached".to_string(),
    ];
    if options.tcp_established {
        argv.push("--tcp-established".to_string());
    }
    if options.shell_job {
        argv.push("--shell-job".to_string());
    }
    argv
}

/// Compute the default images directory for a container.
pub fn default_images_dir(container_id: Uuid) -> PathBuf {
    paths::checkpoint_dir(&container_id.to_string())
}

/// Write the manifest descriptor next to the CRIU images.
pub fn write_manifest(images_dir: &Path, manifest: &CheckpointManifest) -> CriResult<()> {
    std::fs::create_dir_all(images_dir).map_err(CriError::Io)?;
    let bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|e| CriError::Runtime(format!("manifest serialize: {}", e)))?;
    std::fs::write(images_dir.join(MANIFEST_FILENAME), bytes).map_err(CriError::Io)?;
    Ok(())
}

/// Read a previously-written manifest.
pub fn read_manifest(images_dir: &Path) -> CriResult<CheckpointManifest> {
    let path = images_dir.join(MANIFEST_FILENAME);
    let bytes = std::fs::read(&path).map_err(CriError::Io)?;
    serde_json::from_slice(&bytes).map_err(|e| CriError::Runtime(format!("manifest parse: {}", e)))
}

/// Total size of all CRIU image files in `dir` (recursive). Used by
/// `CheckpointContainerResponse.size_bytes`.
pub fn dir_size_bytes(dir: &Path) -> u64 {
    fn walk(p: &Path) -> u64 {
        let Ok(entries) = std::fs::read_dir(p) else {
            return 0;
        };
        let mut total = 0u64;
        for e in entries.flatten() {
            let path = e.path();
            if let Ok(meta) = e.metadata() {
                if meta.is_file() {
                    total = total.saturating_add(meta.len());
                } else if meta.is_dir() {
                    total = total.saturating_add(walk(&path));
                }
            }
        }
        total
    }
    walk(dir)
}

/// Verify a checkpoint directory looks well-formed before restoring:
/// must exist, must contain a manifest.
pub fn verify_checkpoint(images_dir: &Path) -> CriResult<()> {
    if !images_dir.exists() {
        return Err(CriError::Runtime(format!(
            "checkpoint images dir not found: {}",
            images_dir.display()
        )));
    }
    if !images_dir.join(MANIFEST_FILENAME).exists() {
        return Err(CriError::Runtime(format!(
            "checkpoint manifest missing in {}",
            images_dir.display()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Once;
    use tempfile::tempdir;

    static INIT_ROOT: Once = Once::new();
    fn ensure_test_root() {
        INIT_ROOT.call_once(|| {
            let dir = std::env::temp_dir().join(format!("cave-cri-criu-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            std::env::set_var("CAVE_ROOT_DIR", &dir);
        });
    }

    fn fixture_manifest(container_id: Uuid) -> CheckpointManifest {
        CheckpointManifest {
            container_id,
            container_name: "redis-0".into(),
            image_reference: "redis:7".into(),
            runtime_handler: Some("runc".into()),
            created_at: Utc::now(),
            criu_version: "3.19".into(),
            options: CheckpointOptions::default(),
        }
    }

    // ── build_dump_argv ───────────────────────────────────────────────────────

    #[test]
    fn dump_argv_minimal_no_optional_flags() {
        let argv = build_dump_argv(123, Path::new("/x/y"), &CheckpointOptions::default());
        assert!(argv.contains(&"criu".to_string()));
        assert!(argv.contains(&"dump".to_string()));
        assert!(argv.contains(&"--tree".to_string()));
        assert!(argv.contains(&"123".to_string()));
        assert!(argv.contains(&"--images-dir".to_string()));
        assert!(argv.contains(&"/x/y".to_string()));
        // No optional flags.
        assert!(!argv.iter().any(|a| a == "--leave-running"));
        assert!(!argv.iter().any(|a| a == "--tcp-established"));
        assert!(!argv.iter().any(|a| a == "--shell-job"));
        assert!(!argv.iter().any(|a| a == "--ext-mount-map"));
    }

    #[test]
    fn dump_argv_leave_running_appends_flag() {
        let opts = CheckpointOptions {
            leave_running: true,
            ..Default::default()
        };
        let argv = build_dump_argv(1, Path::new("/x"), &opts);
        assert!(argv.iter().any(|a| a == "--leave-running"));
    }

    #[test]
    fn dump_argv_tcp_established_appends_flag() {
        let opts = CheckpointOptions {
            tcp_established: true,
            ..Default::default()
        };
        let argv = build_dump_argv(1, Path::new("/x"), &opts);
        assert!(argv.iter().any(|a| a == "--tcp-established"));
    }

    #[test]
    fn dump_argv_shell_job_appends_flag() {
        let opts = CheckpointOptions {
            shell_job: true,
            ..Default::default()
        };
        let argv = build_dump_argv(1, Path::new("/x"), &opts);
        assert!(argv.iter().any(|a| a == "--shell-job"));
    }

    #[test]
    fn dump_argv_external_mounts_appends_flag_and_value() {
        let opts = CheckpointOptions {
            external_mounts: true,
            ..Default::default()
        };
        let argv = build_dump_argv(1, Path::new("/x"), &opts);
        let idx = argv.iter().position(|a| a == "--ext-mount-map").unwrap();
        assert_eq!(argv[idx + 1], "auto");
    }

    #[test]
    fn dump_argv_log_file_under_images_dir() {
        let argv = build_dump_argv(1, Path::new("/img"), &CheckpointOptions::default());
        let idx = argv.iter().position(|a| a == "--log-file").unwrap();
        assert_eq!(argv[idx + 1], "/img/dump.log");
    }

    // ── build_restore_argv ───────────────────────────────────────────────────

    #[test]
    fn restore_argv_includes_restore_detached() {
        let argv = build_restore_argv(Path::new("/img"), &CheckpointOptions::default());
        assert!(argv.iter().any(|a| a == "--restore-detached"));
    }

    #[test]
    fn restore_argv_does_not_include_dump_only_flags() {
        let opts = CheckpointOptions {
            leave_running: true,
            external_mounts: true,
            ..Default::default()
        };
        let argv = build_restore_argv(Path::new("/img"), &opts);
        // --leave-running and --ext-mount-map are dump-side only; restore
        // shouldn't propagate them.
        assert!(!argv.iter().any(|a| a == "--leave-running"));
        assert!(!argv.iter().any(|a| a == "--ext-mount-map"));
    }

    #[test]
    fn restore_argv_propagates_tcp_and_shell_options() {
        let opts = CheckpointOptions {
            tcp_established: true,
            shell_job: true,
            ..Default::default()
        };
        let argv = build_restore_argv(Path::new("/img"), &opts);
        assert!(argv.iter().any(|a| a == "--tcp-established"));
        assert!(argv.iter().any(|a| a == "--shell-job"));
    }

    // ── default_images_dir ───────────────────────────────────────────────────

    #[test]
    fn default_images_dir_uses_paths_module() {
        ensure_test_root();
        let id = Uuid::new_v4();
        let dir = default_images_dir(id);
        assert!(dir.to_string_lossy().contains("checkpoints"));
        assert!(dir.to_string_lossy().contains(&id.to_string()));
    }

    // ── manifest write/read roundtrip ───────────────────────────────────────

    #[test]
    fn manifest_write_then_read_roundtrip() {
        let dir = tempdir().unwrap();
        let m = fixture_manifest(Uuid::new_v4());
        write_manifest(dir.path(), &m).unwrap();
        assert!(dir.path().join(MANIFEST_FILENAME).exists());
        let back = read_manifest(dir.path()).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn read_manifest_missing_file_errors() {
        let dir = tempdir().unwrap();
        assert!(read_manifest(dir.path()).is_err());
    }

    #[test]
    fn write_manifest_creates_parent_dirs() {
        let dir = tempdir().unwrap();
        let nested = dir.path().join("a/b/c");
        let m = fixture_manifest(Uuid::new_v4());
        write_manifest(&nested, &m).unwrap();
        assert!(nested.join(MANIFEST_FILENAME).exists());
    }

    // ── dir_size_bytes ───────────────────────────────────────────────────────

    #[test]
    fn dir_size_bytes_sums_files() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a"), vec![0u8; 100]).unwrap();
        std::fs::write(dir.path().join("b"), vec![0u8; 50]).unwrap();
        assert_eq!(dir_size_bytes(dir.path()), 150);
    }

    #[test]
    fn dir_size_bytes_recurses_into_subdirs() {
        let dir = tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("x"), vec![0u8; 80]).unwrap();
        std::fs::write(dir.path().join("y"), vec![0u8; 20]).unwrap();
        assert_eq!(dir_size_bytes(dir.path()), 100);
    }

    #[test]
    fn dir_size_bytes_missing_dir_returns_zero() {
        assert_eq!(dir_size_bytes(Path::new("/totally/nonexistent")), 0);
    }

    // ── verify_checkpoint ────────────────────────────────────────────────────

    #[test]
    fn verify_checkpoint_ok_when_manifest_present() {
        let dir = tempdir().unwrap();
        let m = fixture_manifest(Uuid::new_v4());
        write_manifest(dir.path(), &m).unwrap();
        assert!(verify_checkpoint(dir.path()).is_ok());
    }

    #[test]
    fn verify_checkpoint_missing_dir_errors() {
        assert!(verify_checkpoint(Path::new("/no/such/dir")).is_err());
    }

    #[test]
    fn verify_checkpoint_missing_manifest_errors() {
        let dir = tempdir().unwrap();
        // dir exists but no manifest
        let err = verify_checkpoint(dir.path()).unwrap_err();
        assert!(err.to_string().contains("manifest missing"));
    }

    // ── CheckpointOptions / Manifest serde ───────────────────────────────────

    #[test]
    fn checkpoint_options_default_all_false() {
        let o = CheckpointOptions::default();
        assert!(!o.leave_running);
        assert!(!o.tcp_established);
        assert!(!o.shell_job);
        assert!(!o.external_mounts);
        assert!(o.images_dir.is_none());
    }

    #[test]
    fn checkpoint_options_serializes_with_snake_case() {
        let o = CheckpointOptions {
            leave_running: true,
            ..Default::default()
        };
        let json = serde_json::to_string(&o).unwrap();
        assert!(json.contains("\"leave_running\":true"));
    }

    #[test]
    fn checkpoint_manifest_roundtrip_through_json() {
        let m = fixture_manifest(Uuid::new_v4());
        let json = serde_json::to_string(&m).unwrap();
        let back: CheckpointManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
    }
}
