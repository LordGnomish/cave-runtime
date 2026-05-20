// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tracked-project discovery.
//!
//! The set of upstreams we watch is derived from each crate's own
//! `parity.manifest.toml::[upstream]` block — workspace-driven, not
//! hand-curated, so a new crate landing on `main` is picked up the
//! next time the daemon starts. A separate hand-curated allow-list
//! lives in `EXTRA_TRACKED` for projects that don't yet have a
//! per-crate manifest (e.g. tooling we follow for ADR justification
//! but don't port).
//!
//! Each `TrackedProject` is the minimum the daemon needs:
//!
//! * `cave_module` — the local crate name (or freeform tag).
//! * `github_repo` — `org/name`, the API key.
//! * `current_pin` — the `[upstream] version` we pinned locally.
//! * `priority` — `"high"` (15-min cadence) / `"normal"` (60-min).

use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

/// One upstream this daemon polls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackedProject {
    pub cave_module: String,
    pub github_repo: String,
    pub current_pin: Option<String>,
    pub priority: Priority,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    /// Kernel modules — 5-minute cadence in production.
    High,
    /// Everything else — 60-minute cadence.
    Normal,
}

#[derive(Debug, Error)]
pub enum TrackedError {
    #[error("workspace root not found: {0}")]
    NotFound(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// High-priority CAVE modules — these get the fastest poll cadence
/// because new upstream releases impact the production critical path.
/// Mirrors `cave-upstream::HIGH_PRIORITY_MODULES` but lives here too
/// so this crate stays self-contained (no transitive dep on
/// `cave-upstream`).
pub const HIGH_PRIORITY_MODULES: &[&str] = &[
    "cave-apiserver",
    "cave-etcd",
    "cave-scheduler",
    "cave-cri",
    "cave-net",
    "cave-mesh",
    "cave-streams",
    "cave-pg",
    "cave-docdb",
    "cave-vault",
    "cave-cache",
    "cave-registry",
];

fn priority_for(cave_module: &str) -> Priority {
    if HIGH_PRIORITY_MODULES.iter().any(|m| *m == cave_module) {
        Priority::High
    } else {
        Priority::Normal
    }
}

/// Walk `<workspace>/crates/*/parity.manifest.toml` and produce one
/// `TrackedProject` per manifest that declares an `[upstream]` block
/// pointing at a GitHub repo. Crates without an upstream block are
/// silently skipped (infra-only / CAVE-internal).
pub fn load_from_workspace(workspace_root: &Path) -> Result<Vec<TrackedProject>, TrackedError> {
    let crates_dir = workspace_root.join("crates");
    if !crates_dir.is_dir() {
        return Err(TrackedError::NotFound(crates_dir.display().to_string()));
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&crates_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest = path.join("parity.manifest.toml");
        if !manifest.is_file() {
            continue;
        }
        let cave_module = match path.file_name().and_then(|n| n.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let Ok(text) = std::fs::read_to_string(&manifest) else {
            continue;
        };
        let Some(repo) = parse_github_repo(&text) else {
            continue; // No upstream block / non-GitHub upstream — skip.
        };
        let pin = parse_pinned_version(&text);
        out.push(TrackedProject {
            priority: priority_for(&cave_module),
            cave_module,
            github_repo: repo,
            current_pin: pin,
        });
    }
    out.sort_by(|a, b| a.cave_module.cmp(&b.cave_module));
    Ok(out)
}

/// Extract the `org/repo` slug from an `[upstream]` block. Recognises
/// two shapes:
///
/// 1. Direct keys: `org = "x"` + `repo = "y"` → `"x/y"`.
/// 2. URL form: `url = "https://github.com/x/y"` → `"x/y"`.
///
/// Returns `None` when the upstream block doesn't exist or its
/// metadata isn't a GitHub repo (gitlab, bitbucket, …).
pub fn parse_github_repo(manifest_text: &str) -> Option<String> {
    let block = extract_upstream_block(manifest_text)?;

    let org = pick_str_value(&block, "org");
    let repo = pick_str_value(&block, "repo");
    if let (Some(o), Some(r)) = (org, repo) {
        return Some(format!("{o}/{r}"));
    }

    let url = pick_str_value(&block, "url")?;
    let trimmed = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))?;
    let parts: Vec<&str> = trimmed.trim_end_matches('/').splitn(3, '/').collect();
    if parts.len() >= 2 {
        return Some(format!("{}/{}", parts[0], parts[1]));
    }
    None
}

/// Pull the `[upstream] version` value, e.g. `"v1.36.0"`. Strips
/// trailing comments and quotes.
pub fn parse_pinned_version(manifest_text: &str) -> Option<String> {
    let block = extract_upstream_block(manifest_text)?;
    pick_str_value(&block, "version")
}

fn extract_upstream_block(text: &str) -> Option<String> {
    let mut in_block = false;
    let mut out = String::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("[upstream]") {
            in_block = true;
            continue;
        }
        if in_block {
            // A new section header ends the block; comments / blanks
            // stay.
            if trimmed.starts_with('[') && !trimmed.starts_with('#') {
                break;
            }
            out.push_str(line);
            out.push('\n');
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

fn pick_str_value(block: &str, key: &str) -> Option<String> {
    for line in block.lines() {
        // Strip trailing `# comment`.
        let line = line.split('#').next().unwrap_or(line);
        let stripped = line.trim_start();
        if let Some(rest) = stripped.strip_prefix(key) {
            let rest = rest.trim_start();
            if let Some(after_eq) = rest.strip_prefix('=') {
                let v = after_eq.trim();
                let v = v.trim_matches('"').to_string();
                if !v.is_empty() {
                    return Some(v);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture() -> tempfile::TempDir {
        let d = tempfile::TempDir::new().unwrap();
        let crates = d.path().join("crates");
        fs::create_dir_all(&crates).unwrap();

        // cave-cri — full upstream block.
        let cri = crates.join("cave-cri");
        fs::create_dir_all(&cri).unwrap();
        fs::write(
            cri.join("parity.manifest.toml"),
            r#"[upstream]
org     = "containerd"
repo    = "containerd"
version = "v1.7.21"
url     = "https://github.com/containerd/containerd"
"#,
        )
        .unwrap();

        // cave-portal — url-only upstream.
        let portal = crates.join("cave-portal");
        fs::create_dir_all(&portal).unwrap();
        fs::write(
            portal.join("parity.manifest.toml"),
            r#"[upstream]
url     = "https://github.com/backstage/backstage"
version = "1.40.0"
"#,
        )
        .unwrap();

        // cave-runtime — no upstream block (CAVE-internal).
        let runtime = crates.join("cave-runtime");
        fs::create_dir_all(&runtime).unwrap();
        fs::write(
            runtime.join("parity.manifest.toml"),
            r#"[parity]
infra_only = true
"#,
        )
        .unwrap();

        // cave-skip — manifest missing.
        let skip = crates.join("cave-skip");
        fs::create_dir_all(&skip).unwrap();

        // cave-etcd — high-priority.
        let etcd = crates.join("cave-etcd");
        fs::create_dir_all(&etcd).unwrap();
        fs::write(
            etcd.join("parity.manifest.toml"),
            r#"[upstream]
org = "etcd-io"
repo = "etcd"
version = "v3.5.13"  # pinned
"#,
        )
        .unwrap();

        d
    }

    #[test]
    fn load_from_workspace_picks_up_two_real_upstreams() {
        let d = fixture();
        let mut got = load_from_workspace(d.path()).unwrap();
        got.sort_by(|a, b| a.cave_module.cmp(&b.cave_module));
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].cave_module, "cave-cri");
        assert_eq!(got[0].github_repo, "containerd/containerd");
        assert_eq!(got[1].cave_module, "cave-etcd");
        assert_eq!(got[1].github_repo, "etcd-io/etcd");
        assert_eq!(got[2].cave_module, "cave-portal");
        assert_eq!(got[2].github_repo, "backstage/backstage");
    }

    #[test]
    fn load_from_workspace_carries_pinned_version() {
        let d = fixture();
        let got = load_from_workspace(d.path()).unwrap();
        let cri = got.iter().find(|p| p.cave_module == "cave-cri").unwrap();
        assert_eq!(cri.current_pin.as_deref(), Some("v1.7.21"));
        let etcd = got.iter().find(|p| p.cave_module == "cave-etcd").unwrap();
        assert_eq!(etcd.current_pin.as_deref(), Some("v3.5.13"));
    }

    #[test]
    fn priority_assigned_from_high_priority_module_list() {
        let d = fixture();
        let got = load_from_workspace(d.path()).unwrap();
        assert_eq!(
            got.iter()
                .find(|p| p.cave_module == "cave-etcd")
                .unwrap()
                .priority,
            Priority::High,
        );
        assert_eq!(
            got.iter()
                .find(|p| p.cave_module == "cave-portal")
                .unwrap()
                .priority,
            Priority::Normal,
        );
    }

    #[test]
    fn parse_github_repo_handles_org_repo_and_url_forms() {
        let direct = r#"org = "etcd-io"
repo = "etcd"
"#;
        assert_eq!(
            parse_github_repo(&format!("[upstream]\n{direct}")).as_deref(),
            Some("etcd-io/etcd")
        );

        let url = r#"url = "https://github.com/etcd-io/etcd"
"#;
        assert_eq!(
            parse_github_repo(&format!("[upstream]\n{url}")).as_deref(),
            Some("etcd-io/etcd")
        );
    }

    #[test]
    fn parse_github_repo_rejects_non_github_url() {
        let body = "[upstream]\nurl = \"https://gitlab.com/foo/bar\"\n";
        assert_eq!(parse_github_repo(body), None);
    }

    #[test]
    fn parse_github_repo_returns_none_when_upstream_missing() {
        let body = "[parity]\ninfra_only = true\n";
        assert_eq!(parse_github_repo(body), None);
    }

    #[test]
    fn parse_pinned_version_strips_trailing_comment() {
        let body = r#"[upstream]
version = "v3.5.13"  # pinned to LTS
"#;
        assert_eq!(parse_pinned_version(body).as_deref(), Some("v3.5.13"));
    }

    #[test]
    fn load_from_workspace_returns_not_found_on_missing_crates_dir() {
        let d = tempfile::TempDir::new().unwrap();
        let err = load_from_workspace(d.path()).unwrap_err();
        assert!(matches!(err, TrackedError::NotFound(_)));
    }

    #[test]
    fn high_priority_list_has_twelve_entries() {
        assert_eq!(HIGH_PRIORITY_MODULES.len(), 12);
        assert!(HIGH_PRIORITY_MODULES.contains(&"cave-etcd"));
    }
}
