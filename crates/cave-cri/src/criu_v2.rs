//! CRIU second-layer features — pre-dump, incremental snapshots,
//! external resource maps, page-server pipe, descriptor.json.
//!
//! Builds on `crate::criu` (CheckpointOptions + dump/restore argv +
//! manifest). New surfaces:
//!
//! - **Pre-dump** — `criu pre-dump --tree <pid> --images-dir <dir>`
//!   writes the live memory pages to disk while the container keeps
//!   running, leaving only the deltas for the final dump. Used by
//!   live migration to keep down-time minimal.
//! - **Incremental dump** — `--prev-images-dir` chains a dump onto
//!   one or more pre-dump generations. The argv builder records the
//!   chain order so the restore side replays it correctly.
//! - **External resource map** — `--external mnt[<key>]:<value>`
//!   tells CRIU about resources it must not try to dump itself
//!   (bind-mounted host paths, sockets, etc.).
//! - **Page server** — `--page-server <addr>:<port>` ships the
//!   memory pages over the network instead of writing them locally.
//! - **descriptor.json** — the per-image-set manifest CRIU writes
//!   beside `pages-*.img`. cave parses it so we can tell whether a
//!   directory holds a pre-dump, dump, or restore image set.

use crate::error::{CriError, CriResult};
use crate::criu::CheckpointOptions;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const DESCRIPTOR_FILENAME: &str = "descriptor.json";

/// Recognised CRIU image set kinds. Mirrors the `dump_type` field in
/// CRIU's own descriptor.json (one of `dump`, `pre_dump`, `restore`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DumpKind {
    Dump,
    PreDump,
    Restore,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CriuDescriptor {
    pub dump_kind: DumpKind,
    pub criu_version: String,
    /// Chain of prior pre-dump directories, oldest first. Used so the
    /// restore side can verify it has all the deltas.
    #[serde(default)]
    pub prev_image_dirs: Vec<PathBuf>,
    /// `--external` mappings serialised as `key=value` strings.
    #[serde(default)]
    pub externals: Vec<String>,
}

impl CriuDescriptor {
    pub fn write(&self, images_dir: &Path) -> CriResult<()> {
        std::fs::create_dir_all(images_dir).map_err(CriError::Io)?;
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|e| CriError::Runtime(format!("descriptor encode: {}", e)))?;
        std::fs::write(images_dir.join(DESCRIPTOR_FILENAME), bytes).map_err(CriError::Io)?;
        Ok(())
    }

    pub fn read(images_dir: &Path) -> CriResult<CriuDescriptor> {
        let bytes = std::fs::read(images_dir.join(DESCRIPTOR_FILENAME)).map_err(CriError::Io)?;
        serde_json::from_slice(&bytes)
            .map_err(|e| CriError::Runtime(format!("descriptor parse: {}", e)))
    }
}

/// Knobs the kubelet can pass on top of `CheckpointOptions`. These map
/// to CRIU CLI flags that aren't carried in the standard CRI request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvancedDumpOptions {
    /// `--track-mem` — required by `pre-dump` to enable memory diff
    /// tracking with the kernel.
    pub track_mem: bool,
    /// Chain of prior pre-dump directories, oldest first; passed via
    /// repeated `--prev-images-dir` flags (CRIU concatenates them).
    pub prev_image_dirs: Vec<PathBuf>,
    /// `--external` mappings — `mnt[KEY]:PATH`, `dev[…]`, `tty[…]`,
    /// `ifup[…]`, etc.
    pub externals: Vec<String>,
    /// `--page-server <addr>:<port>` — ship pages over a TCP socket
    /// instead of to disk.
    pub page_server: Option<String>,
    /// `--manage-cgroups` mode (`soft`, `full`, `strict`, `ignore`).
    pub manage_cgroups: Option<String>,
}

/// Build the `criu pre-dump` argv. `pre-dump` is a stripped-down
/// version of `dump` that leaves the process running and writes only
/// the memory pages for incremental work.
pub fn build_pre_dump_argv(
    pid: u32,
    images_dir: &Path,
    options: &CheckpointOptions,
    advanced: &AdvancedDumpOptions,
) -> Vec<String> {
    let mut argv = vec![
        "criu".to_string(),
        "pre-dump".to_string(),
        "--tree".to_string(),
        pid.to_string(),
        "--images-dir".to_string(),
        images_dir.display().to_string(),
        "--log-file".to_string(),
        images_dir.join("pre-dump.log").display().to_string(),
        "--track-mem".to_string(),
    ];
    for prev in &advanced.prev_image_dirs {
        argv.push("--prev-images-dir".to_string());
        argv.push(prev.display().to_string());
    }
    for ext in &advanced.externals {
        argv.push("--external".to_string());
        argv.push(ext.clone());
    }
    if options.tcp_established {
        argv.push("--tcp-established".to_string());
    }
    if let Some(server) = &advanced.page_server {
        argv.push("--page-server".to_string());
        argv.push(server.clone());
    }
    let _ = options; // shell_job/external_mounts are dump-only
    argv
}

/// Build the `criu dump` argv as an extension of the basic
/// `crate::criu::build_dump_argv`, with the AdvancedDumpOptions knobs
/// layered on (chain of prev dirs, externals, page server, manage
/// cgroups mode).
pub fn build_advanced_dump_argv(
    pid: u32,
    images_dir: &Path,
    options: &CheckpointOptions,
    advanced: &AdvancedDumpOptions,
) -> Vec<String> {
    let mut argv = crate::criu::build_dump_argv(pid, images_dir, options);
    for prev in &advanced.prev_image_dirs {
        argv.push("--prev-images-dir".to_string());
        argv.push(prev.display().to_string());
    }
    for ext in &advanced.externals {
        argv.push("--external".to_string());
        argv.push(ext.clone());
    }
    if let Some(server) = &advanced.page_server {
        argv.push("--page-server".to_string());
        argv.push(server.clone());
    }
    if let Some(mode) = &advanced.manage_cgroups {
        argv.push("--manage-cgroups".to_string());
        argv.push(mode.clone());
    }
    if advanced.track_mem {
        argv.push("--track-mem".to_string());
    }
    argv
}

/// Build a `descriptor.json` for an outgoing image set.
pub fn build_descriptor(
    kind: DumpKind,
    advanced: &AdvancedDumpOptions,
) -> CriuDescriptor {
    CriuDescriptor {
        dump_kind: kind,
        criu_version: "3.19".to_string(),
        prev_image_dirs: advanced.prev_image_dirs.clone(),
        externals: advanced.externals.clone(),
    }
}

/// Verify a chain of pre-dump directories is well-formed:
/// 1. Every directory must exist.
/// 2. Every non-final directory must hold a `pre_dump` descriptor.
/// 3. Order is oldest → newest (no cycles, no duplicates).
pub fn verify_pre_dump_chain(dirs: &[PathBuf]) -> CriResult<()> {
    let mut seen = std::collections::HashSet::new();
    for (i, dir) in dirs.iter().enumerate() {
        if !dir.exists() {
            return Err(CriError::Runtime(format!(
                "pre-dump chain segment missing: {}", dir.display()
            )));
        }
        let canonical = dir.canonicalize().unwrap_or_else(|_| dir.clone());
        if !seen.insert(canonical.clone()) {
            return Err(CriError::Runtime(format!(
                "pre-dump chain has duplicate segment: {}", dir.display()
            )));
        }
        // Last entry can be a dump or pre-dump; intermediate must be pre-dump.
        if i + 1 < dirs.len() {
            let descriptor = CriuDescriptor::read(dir)?;
            if descriptor.dump_kind != DumpKind::PreDump {
                return Err(CriError::Runtime(format!(
                    "intermediate chain segment {} is not a pre-dump (kind={:?})",
                    dir.display(), descriptor.dump_kind,
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn opts() -> CheckpointOptions { CheckpointOptions::default() }
    fn adv() -> AdvancedDumpOptions { AdvancedDumpOptions::default() }

    // ── DumpKind serde ───────────────────────────────────────────────────────

    #[test]
    fn dump_kind_serializes_snake_case() {
        for (k, expected) in [
            (DumpKind::Dump,    "\"dump\""),
            (DumpKind::PreDump, "\"pre_dump\""),
            (DumpKind::Restore, "\"restore\""),
        ] {
            assert_eq!(serde_json::to_string(&k).unwrap(), expected);
        }
    }

    #[test]
    fn dump_kind_roundtrips_through_json() {
        for k in [DumpKind::Dump, DumpKind::PreDump, DumpKind::Restore] {
            let json = serde_json::to_string(&k).unwrap();
            let back: DumpKind = serde_json::from_str(&json).unwrap();
            assert_eq!(k, back);
        }
    }

    // ── pre-dump argv ────────────────────────────────────────────────────────

    #[test]
    fn pre_dump_argv_uses_pre_dump_subcommand() {
        let argv = build_pre_dump_argv(123, Path::new("/img"), &opts(), &adv());
        assert_eq!(argv[0], "criu");
        assert_eq!(argv[1], "pre-dump");
    }

    #[test]
    fn pre_dump_argv_always_includes_track_mem() {
        let argv = build_pre_dump_argv(1, Path::new("/x"), &opts(), &adv());
        assert!(argv.iter().any(|a| a == "--track-mem"));
    }

    #[test]
    fn pre_dump_argv_chains_prev_image_dirs() {
        let advanced = AdvancedDumpOptions {
            prev_image_dirs: vec![PathBuf::from("/p0"), PathBuf::from("/p1")],
            ..Default::default()
        };
        let argv = build_pre_dump_argv(1, Path::new("/img"), &opts(), &advanced);
        let prevs: Vec<&str> = argv.iter()
            .enumerate()
            .filter_map(|(i, a)| if a == "--prev-images-dir" { Some(argv[i + 1].as_str()) } else { None })
            .collect();
        assert_eq!(prevs, vec!["/p0", "/p1"]);
    }

    #[test]
    fn pre_dump_argv_includes_externals() {
        let advanced = AdvancedDumpOptions {
            externals: vec!["mnt[host]:/tmp/host".into(), "tty[5,3]:tty1".into()],
            ..Default::default()
        };
        let argv = build_pre_dump_argv(1, Path::new("/img"), &opts(), &advanced);
        let count = argv.iter().filter(|a| **a == "--external").count();
        assert_eq!(count, 2);
        assert!(argv.contains(&"mnt[host]:/tmp/host".to_string()));
    }

    #[test]
    fn pre_dump_argv_with_page_server_appends_pair() {
        let advanced = AdvancedDumpOptions {
            page_server: Some("10.0.0.1:5080".into()),
            ..Default::default()
        };
        let argv = build_pre_dump_argv(1, Path::new("/img"), &opts(), &advanced);
        let idx = argv.iter().position(|a| a == "--page-server").unwrap();
        assert_eq!(argv[idx + 1], "10.0.0.1:5080");
    }

    #[test]
    fn pre_dump_argv_propagates_tcp_established() {
        let mut o = opts();
        o.tcp_established = true;
        let argv = build_pre_dump_argv(1, Path::new("/img"), &o, &adv());
        assert!(argv.iter().any(|a| a == "--tcp-established"));
    }

    // ── advanced dump argv ───────────────────────────────────────────────────

    #[test]
    fn advanced_dump_argv_extends_basic_dump() {
        let basic = crate::criu::build_dump_argv(1, Path::new("/img"), &opts());
        let advanced = build_advanced_dump_argv(1, Path::new("/img"), &opts(), &adv());
        // Every basic argv element must appear in the advanced argv.
        for tok in &basic {
            assert!(advanced.contains(tok), "missing token {} in advanced argv", tok);
        }
    }

    #[test]
    fn advanced_dump_argv_appends_manage_cgroups() {
        let advanced = AdvancedDumpOptions {
            manage_cgroups: Some("strict".into()),
            ..Default::default()
        };
        let argv = build_advanced_dump_argv(1, Path::new("/img"), &opts(), &advanced);
        let idx = argv.iter().position(|a| a == "--manage-cgroups").unwrap();
        assert_eq!(argv[idx + 1], "strict");
    }

    #[test]
    fn advanced_dump_argv_appends_track_mem_when_requested() {
        let advanced = AdvancedDumpOptions { track_mem: true, ..Default::default() };
        let argv = build_advanced_dump_argv(1, Path::new("/img"), &opts(), &advanced);
        assert!(argv.iter().any(|a| a == "--track-mem"));
    }

    #[test]
    fn advanced_dump_argv_no_extra_flags_for_default() {
        let basic = crate::criu::build_dump_argv(1, Path::new("/img"), &opts());
        let advanced = build_advanced_dump_argv(1, Path::new("/img"), &opts(), &adv());
        assert_eq!(advanced.len(), basic.len());
    }

    // ── descriptor.json roundtrip ────────────────────────────────────────────

    #[test]
    fn descriptor_write_then_read_roundtrip() {
        let dir = tempdir().unwrap();
        let d = CriuDescriptor {
            dump_kind: DumpKind::PreDump,
            criu_version: "3.19".into(),
            prev_image_dirs: vec![PathBuf::from("/old")],
            externals: vec!["mnt[host]:/tmp/host".into()],
        };
        d.write(dir.path()).unwrap();
        let back = CriuDescriptor::read(dir.path()).unwrap();
        assert_eq!(d, back);
    }

    #[test]
    fn descriptor_read_missing_file_errors() {
        let dir = tempdir().unwrap();
        assert!(CriuDescriptor::read(dir.path()).is_err());
    }

    #[test]
    fn build_descriptor_carries_advanced_options() {
        let advanced = AdvancedDumpOptions {
            prev_image_dirs: vec![PathBuf::from("/p")],
            externals: vec!["mnt[h]:/h".into()],
            ..Default::default()
        };
        let d = build_descriptor(DumpKind::Dump, &advanced);
        assert_eq!(d.dump_kind, DumpKind::Dump);
        assert_eq!(d.prev_image_dirs.len(), 1);
        assert_eq!(d.externals.len(), 1);
    }

    // ── verify_pre_dump_chain ────────────────────────────────────────────────

    #[test]
    fn verify_chain_empty_is_ok() {
        assert!(verify_pre_dump_chain(&[]).is_ok());
    }

    #[test]
    fn verify_chain_missing_segment_errors() {
        let err = verify_pre_dump_chain(&[PathBuf::from("/no/such/dir")]).unwrap_err();
        assert!(err.to_string().contains("missing"));
    }

    #[test]
    fn verify_chain_intermediate_must_be_pre_dump() {
        let dir1 = tempdir().unwrap();
        let dir2 = tempdir().unwrap();
        // dir1 holds a *dump* (not a pre-dump) but is intermediate.
        CriuDescriptor {
            dump_kind: DumpKind::Dump,
            criu_version: "3.19".into(),
            prev_image_dirs: vec![],
            externals: vec![],
        }.write(dir1.path()).unwrap();
        let err = verify_pre_dump_chain(&[dir1.path().to_path_buf(), dir2.path().to_path_buf()]).unwrap_err();
        assert!(err.to_string().contains("not a pre-dump"));
    }

    #[test]
    fn verify_chain_intermediate_pre_dump_ok() {
        let dir1 = tempdir().unwrap();
        let dir2 = tempdir().unwrap();
        CriuDescriptor {
            dump_kind: DumpKind::PreDump,
            criu_version: "3.19".into(),
            prev_image_dirs: vec![],
            externals: vec![],
        }.write(dir1.path()).unwrap();
        assert!(verify_pre_dump_chain(&[dir1.path().to_path_buf(), dir2.path().to_path_buf()]).is_ok());
    }

    #[test]
    fn verify_chain_detects_duplicate_segment() {
        let dir = tempdir().unwrap();
        CriuDescriptor {
            dump_kind: DumpKind::PreDump,
            criu_version: "3.19".into(),
            prev_image_dirs: vec![],
            externals: vec![],
        }.write(dir.path()).unwrap();
        let path = dir.path().to_path_buf();
        let err = verify_pre_dump_chain(&[path.clone(), path]).unwrap_err();
        assert!(err.to_string().contains("duplicate"));
    }

    // ── AdvancedDumpOptions defaults ─────────────────────────────────────────

    #[test]
    fn advanced_dump_options_default_has_no_extras() {
        let a = AdvancedDumpOptions::default();
        assert!(!a.track_mem);
        assert!(a.prev_image_dirs.is_empty());
        assert!(a.externals.is_empty());
        assert!(a.page_server.is_none());
        assert!(a.manage_cgroups.is_none());
    }
}
