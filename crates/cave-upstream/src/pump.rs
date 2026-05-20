// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Qwen pump payload writer.
//!
//! When [`crate::delta::detect_release_delta`] returns a
//! [`crate::delta::PollOutcome::NewRelease`], this module turns that into a
//! JSON file in the pump's queue directory:
//!
//! ```text
//! ~/Library/Application Support/cave-qwen-pump/queue/upstream-port-<unix-ms>-<repo-slug>.json
//! ```
//!
//! The pump treats the queue dir as an inbox: any file matching
//! `upstream-port-*.json` is consumed, validated, and turned into a TDD
//! port job. The contract here is the **only** thing the pump depends on
//! — keep [`PumpPayload`] backward-compatible (additive changes only) or
//! bump `schema_version`.
//!
//! ## Atomic write
//!
//! Same trick as [`crate::state`]: write `<file>.tmp`, fsync, rename.
//! The pump only sees a complete file. We never `flock` — multiple
//! daemon instances would collide, but that's a deployment error, not a
//! protocol concern.

use crate::delta::{ReleaseDelta, SurfaceItem};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

/// One pump job. Written 1:1 to a JSON file in the queue dir.
///
/// Bump `schema_version` if you make a breaking change. Additive fields
/// (new optional `Option<…>` fields) do not require a bump.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PumpPayload {
    pub schema_version: u32,
    /// CAVE crate the upstream maps to, e.g. `"cave-etcd"`.
    pub cave_module: String,
    /// `owner/repo` slug.
    pub upstream_repo: String,
    /// Tag we were on, or `None` on first observation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_tag: Option<String>,
    pub new_tag: String,
    pub release_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<DateTime<Utc>>,
    pub surfaces_added: Vec<SurfaceItem>,
    pub surfaces_removed: Vec<SurfaceItem>,
    pub surfaces_changed: Vec<SurfaceItem>,
    /// `"high"` | `"normal"` — drives pump's job ordering.
    pub priority: String,
    /// Wall clock time we wrote this payload. Mostly for debugging.
    pub created_at: DateTime<Utc>,
    /// Free-form JSON pump can use for tracing — daemon stamps its
    /// run-id here.
    #[serde(default)]
    pub origin: PayloadOrigin,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PayloadOrigin {
    pub daemon: String,
    pub run_id: String,
}

const PAYLOAD_SCHEMA_VERSION: u32 = 1;

/// Build a payload from a [`ReleaseDelta`] + module mapping + priority.
pub fn build_payload(
    delta: &ReleaseDelta,
    cave_module: &str,
    priority: &str,
    daemon: &str,
    run_id: &str,
) -> PumpPayload {
    PumpPayload {
        schema_version: PAYLOAD_SCHEMA_VERSION,
        cave_module: cave_module.to_string(),
        upstream_repo: delta.github_repo.clone(),
        old_tag: delta.old_tag.clone(),
        new_tag: delta.new_tag.clone(),
        release_url: delta.release_url.clone(),
        release_name: delta.release_name.clone(),
        release_body: delta.release_body.clone(),
        published_at: delta.published_at,
        surfaces_added: delta.surface_diff.added.clone(),
        surfaces_removed: delta.surface_diff.removed.clone(),
        surfaces_changed: delta.surface_diff.changed.clone(),
        priority: priority.to_string(),
        created_at: Utc::now(),
        origin: PayloadOrigin {
            daemon: daemon.to_string(),
            run_id: run_id.to_string(),
        },
    }
}

/// Default queue dir. Honors `$CAVE_QWEN_PUMP_QUEUE` for tests.
pub fn default_queue_dir() -> PathBuf {
    if let Ok(p) = std::env::var("CAVE_QWEN_PUMP_QUEUE") {
        return PathBuf::from(p);
    }
    let base = dirs::data_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("cave-qwen-pump").join("queue")
}

/// Slugify `owner/repo` -> `owner-repo` for filename safety.
fn slugify_repo(repo: &str) -> String {
    repo.replace('/', "-")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Write the payload atomically. Returns the basename (not the full path)
/// so it can be stored in [`crate::state::ProjectState::last_pump_payload_id`].
pub fn write_payload(queue_dir: &Path, payload: &PumpPayload) -> anyhow::Result<String> {
    std::fs::create_dir_all(queue_dir)
        .map_err(|e| anyhow::anyhow!("create queue dir {}: {}", queue_dir.display(), e))?;
    let ts = payload.created_at.timestamp_millis();
    let slug = slugify_repo(&payload.upstream_repo);
    let basename = format!("upstream-port-{ts}-{slug}.json");
    let final_path = queue_dir.join(&basename);
    let tmp_path = queue_dir.join(format!("{basename}.tmp"));

    let body = serde_json::to_vec_pretty(payload)?;
    {
        let mut f = std::fs::File::create(&tmp_path)?;
        f.write_all(&body)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp_path, &final_path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        anyhow::anyhow!(
            "rename {} -> {}: {}",
            tmp_path.display(),
            final_path.display(),
            e
        )
    })?;
    Ok(basename)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delta::{ReleaseDelta, SurfaceDiff, SurfaceItem};
    use tempfile::TempDir;

    fn sample_delta() -> ReleaseDelta {
        ReleaseDelta {
            github_repo: "etcd-io/etcd".to_string(),
            old_tag: Some("v3.5.10".to_string()),
            new_tag: "v3.6.0".to_string(),
            release_url: "https://example.com/release".to_string(),
            release_name: Some("etcd 3.6.0".to_string()),
            release_body: Some("changelog".to_string()),
            published_at: None,
            surface_diff: SurfaceDiff {
                added: vec![SurfaceItem {
                    symbol: "clientv3.NewWithFoo".to_string(),
                    kind: "function".to_string(),
                    note: None,
                }],
                removed: vec![],
                changed: vec![],
            },
        }
    }

    #[test]
    fn build_payload_carries_all_fields() {
        let p = build_payload(&sample_delta(), "cave-etcd", "high", "watchd", "run-1");
        assert_eq!(p.schema_version, PAYLOAD_SCHEMA_VERSION);
        assert_eq!(p.cave_module, "cave-etcd");
        assert_eq!(p.upstream_repo, "etcd-io/etcd");
        assert_eq!(p.old_tag.as_deref(), Some("v3.5.10"));
        assert_eq!(p.new_tag, "v3.6.0");
        assert_eq!(p.priority, "high");
        assert_eq!(p.surfaces_added.len(), 1);
        assert_eq!(p.surfaces_added[0].symbol, "clientv3.NewWithFoo");
        assert_eq!(p.origin.daemon, "watchd");
        assert_eq!(p.origin.run_id, "run-1");
    }

    #[test]
    fn write_payload_creates_file_and_returns_basename() {
        let tmp = TempDir::new().unwrap();
        let p = build_payload(&sample_delta(), "cave-etcd", "high", "watchd", "run-1");
        let name = write_payload(tmp.path(), &p).unwrap();
        assert!(name.starts_with("upstream-port-"));
        assert!(name.ends_with("-etcd-io-etcd.json"));
        let path = tmp.path().join(&name);
        assert!(path.exists());

        let body = std::fs::read_to_string(&path).unwrap();
        let decoded: PumpPayload = serde_json::from_str(&body).unwrap();
        assert_eq!(decoded, p);
    }

    #[test]
    fn write_payload_no_tmp_files_after_success() {
        let tmp = TempDir::new().unwrap();
        let p = build_payload(&sample_delta(), "cave-etcd", "high", "watchd", "run-1");
        write_payload(tmp.path(), &p).unwrap();
        let bad: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(bad.is_empty(), "no .tmp files should remain");
    }

    #[test]
    fn slugify_handles_normal_repos() {
        assert_eq!(slugify_repo("etcd-io/etcd"), "etcd-io-etcd");
        assert_eq!(
            slugify_repo("kubernetes/kubernetes"),
            "kubernetes-kubernetes"
        );
    }

    #[test]
    fn slugify_strips_unsafe_chars() {
        assert_eq!(slugify_repo("foo/../bar"), "foo-_..-bar");
        assert_eq!(slugify_repo("a b/c d"), "a_b-c_d");
    }

    #[test]
    fn env_var_overrides_default_queue() {
        let tmp = TempDir::new().unwrap();
        // SAFETY: serialised access — no other test in this binary touches this var.
        unsafe {
            std::env::set_var("CAVE_QWEN_PUMP_QUEUE", tmp.path());
        }
        let resolved = default_queue_dir();
        unsafe {
            std::env::remove_var("CAVE_QWEN_PUMP_QUEUE");
        }
        assert_eq!(resolved, tmp.path());
    }

    #[test]
    fn payload_roundtrips_through_json_with_optional_fields_omitted() {
        let mut delta = sample_delta();
        delta.old_tag = None;
        delta.release_name = None;
        delta.release_body = None;
        let p = build_payload(&delta, "cave-etcd", "normal", "watchd", "run-2");
        let json = serde_json::to_string(&p).unwrap();
        // Optional fields skipped when None
        assert!(!json.contains("\"old_tag\""), "old_tag elided when None");
        assert!(!json.contains("\"release_name\""), "release_name elided");
        let decoded: PumpPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.old_tag, None);
    }
}
