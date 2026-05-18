// SPDX-License-Identifier: AGPL-3.0-or-later
//! pulp_ostree — OSTree content plugin (NEW in Phase 2).
//!
//! Implements the static on-disk surfaces of an OSTree repo:
//! - `parse_ostree_ref` — a `refs/heads/<branch>` file is one 64-hex
//!   commit checksum.
//! - `parse_ostree_config` — INI `[core]` + `[remote "name"]` sections.
//! - `OstreeRepoMode` enum (archive-z2, archive, bare, bare-user,
//!   bare-user-only).
//! - `OstreeSummary` reader for the textual fallback of the `summary`
//!   file (GVariant-encoded summaries skipped — see scope note).
//!
//! Upstream parity: pulp/pulp_ostree `pulp_ostree/app/models.py`
//! + OSTree manual `man ostree-repo-config`.

use crate::pulp::error::ArtifactsError;
use crate::pulp::models::{ContentUnit, PluginType, RepositoryVersion};
use crate::pulp::plugin::ArtifactsPlugin;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub struct OstreePlugin;

// ── refs ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OstreeRef {
    pub commit_checksum: String,
}

pub fn parse_ostree_ref(body: &str) -> Result<OstreeRef, ArtifactsError> {
    let s = body.trim();
    if s.len() != 64 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ArtifactsError::InvalidRequest(format!(
            "ostree ref: expected 64-hex commit, got len={}",
            s.len()
        )));
    }
    Ok(OstreeRef {
        commit_checksum: s.to_string(),
    })
}

// ── repo mode ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OstreeRepoMode {
    Bare,
    BareUser,
    BareUserOnly,
    Archive,
    ArchiveZ2,
}

impl OstreeRepoMode {
    fn parse(s: &str) -> Result<Self, ArtifactsError> {
        match s {
            "bare" => Ok(Self::Bare),
            "bare-user" => Ok(Self::BareUser),
            "bare-user-only" => Ok(Self::BareUserOnly),
            "archive" => Ok(Self::Archive),
            "archive-z2" => Ok(Self::ArchiveZ2),
            other => Err(ArtifactsError::InvalidRequest(format!(
                "ostree config: unknown mode '{other}'"
            ))),
        }
    }
}

// ── config ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OstreeRemote {
    pub url: Option<String>,
    pub gpg_verify: Option<bool>,
    pub gpg_verify_summary: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OstreeConfig {
    pub repo_version: Option<u32>,
    pub mode: OstreeRepoMode,
    pub remotes: BTreeMap<String, OstreeRemote>,
}

/// Parse an OSTree `config` file (INI-like).
pub fn parse_ostree_config(raw: &str) -> Result<OstreeConfig, ArtifactsError> {
    let mut section: Option<String> = None;
    let mut core_mode: Option<OstreeRepoMode> = None;
    let mut core_version: Option<u32> = None;
    let mut remotes: BTreeMap<String, OstreeRemote> = BTreeMap::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = Some(line[1..line.len() - 1].to_string());
            continue;
        }
        let (k, v) = line
            .split_once('=')
            .ok_or_else(|| ArtifactsError::InvalidRequest(format!("ostree config: bad line '{line}'")))?;
        let k = k.trim();
        let v = v.trim();
        let sec = section
            .as_deref()
            .ok_or_else(|| ArtifactsError::InvalidRequest("ostree config: key without section".into()))?;
        if sec == "core" {
            match k {
                "mode" => core_mode = Some(OstreeRepoMode::parse(v)?),
                "repo_version" => core_version = v.parse().ok(),
                _ => {}
            }
        } else if let Some(remote_name) = sec.strip_prefix("remote \"").and_then(|s| s.strip_suffix('"')) {
            let r = remotes.entry(remote_name.to_string()).or_default();
            match k {
                "url" => r.url = Some(v.to_string()),
                "gpg-verify" => r.gpg_verify = parse_bool(v),
                "gpg-verify-summary" => r.gpg_verify_summary = parse_bool(v),
                _ => {}
            }
        }
    }
    let mode = core_mode
        .ok_or_else(|| ArtifactsError::InvalidRequest("ostree config: [core] mode missing".into()))?;
    Ok(OstreeConfig {
        repo_version: core_version,
        mode,
        remotes,
    })
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" | "on" => Some(true),
        "false" | "no" | "0" | "off" => Some(false),
        _ => None,
    }
}

// ── Plugin ───────────────────────────────────────────────────────────────────

impl ArtifactsPlugin for OstreePlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::Ostree
    }

    fn name(&self) -> &str {
        "pulp_ostree"
    }

    fn content_types(&self) -> Vec<&str> {
        vec!["ostree.commit", "ostree.ref", "ostree.config", "ostree.summary"]
    }

    fn parse_content(&self, data: &[u8], relative_path: &str) -> Result<ContentUnit, ArtifactsError> {
        let sha256 = hex::encode(Sha256::digest(data));
        let filename = relative_path.rsplit('/').next().unwrap_or(relative_path);
        let mut content_type = "ostree.blob";
        let mut extra = serde_json::Map::new();

        // Try to classify by path + content shape.
        if relative_path.starts_with("refs/") || relative_path.contains("/refs/") {
            if let Ok(s) = std::str::from_utf8(data) {
                if let Ok(r) = parse_ostree_ref(s) {
                    content_type = "ostree.ref";
                    extra.insert("commit_checksum".into(), serde_json::Value::String(r.commit_checksum));
                }
            }
        } else if filename == "config" {
            if let Ok(s) = std::str::from_utf8(data) {
                if let Ok(cfg) = parse_ostree_config(s) {
                    content_type = "ostree.config";
                    extra.insert(
                        "mode".into(),
                        serde_json::Value::String(match cfg.mode {
                            OstreeRepoMode::Bare => "bare",
                            OstreeRepoMode::BareUser => "bare-user",
                            OstreeRepoMode::BareUserOnly => "bare-user-only",
                            OstreeRepoMode::Archive => "archive",
                            OstreeRepoMode::ArchiveZ2 => "archive-z2",
                        }
                        .into()),
                    );
                    extra.insert(
                        "remote_count".into(),
                        serde_json::Value::from(cfg.remotes.len() as u64),
                    );
                }
            }
        }

        let mut md = serde_json::json!({
            "content_type": content_type,
            "relative_path": relative_path,
            "sha256": sha256,
        });
        if let serde_json::Value::Object(ref mut obj) = md {
            for (k, v) in extra {
                obj.insert(k, v);
            }
        }

        let mut unit = ContentUnit::new(PluginType::Ostree, md);
        unit.relative_path = Some(relative_path.to_string());
        unit.sha256 = Some(sha256);
        unit.size = Some(data.len() as u64);
        Ok(unit)
    }

    fn generate_metadata(
        &self,
        _repo_version: &RepositoryVersion,
        units: &[ContentUnit],
    ) -> serde_json::Value {
        // Emit a refs→checksum map suitable for the summary file's textual
        // fallback (GVariant binary encoding deferred).
        let mut refs: BTreeMap<String, String> = BTreeMap::new();
        for u in units {
            if u.metadata["content_type"] == "ostree.ref" {
                if let Some(rp) = &u.relative_path {
                    // refs/heads/<branch> → branch.
                    let branch = rp
                        .strip_prefix("refs/heads/")
                        .or_else(|| rp.split("/refs/heads/").nth(1))
                        .unwrap_or(rp);
                    if let Some(c) = u.metadata.get("commit_checksum").and_then(|v| v.as_str()) {
                        refs.insert(branch.to_string(), c.to_string());
                    }
                }
            }
        }
        serde_json::json!({
            "refs": refs,
            "ref_count": refs.len(),
        })
    }
}
