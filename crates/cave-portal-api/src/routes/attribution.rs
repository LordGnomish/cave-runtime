// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! GET /api/attribution?days=7&repo=all|cave-runtime|pipeline-platform
//!
//! Security: uses `git log -z` with NUL-separated records so commit bodies
//! containing pipes or newlines cannot inject extra fields.
//! The `days` parameter is validated as u32 before being interpolated.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::instrument;

// ── Prometheus counter ────────────────────────────────────────────────────────

use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::registry::Registry;

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct AttributionLabels {
    pub repo: String,
    pub author: String,
}

pub struct AttributionMetrics {
    pub commits_total: Family<AttributionLabels, Counter>,
}

impl AttributionMetrics {
    pub fn new(registry: &mut Registry) -> Self {
        let commits_total = Family::<AttributionLabels, Counter>::default();
        registry.register(
            "cave_portal_attribution_commits_total",
            "Number of commits attributed to each author category",
            commits_total.clone(),
        );
        Self { commits_total }
    }

    pub fn record(&self, repo: &str, author: &str) {
        self.commits_total
            .get_or_create(&AttributionLabels {
                repo: repo.to_owned(),
                author: author.to_owned(),
            })
            .inc();
    }
}

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthorBreakdown {
    pub qwen3: u64,
    pub sonnet: u64,
    pub burak: u64,
    pub other: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AttributionResponse {
    pub period_days: u32,
    pub repo: String,
    pub by_commits: AuthorBreakdown,
    pub by_loc_net: AuthorBreakdown,
    pub by_files: AuthorBreakdown,
    pub timestamps: Timestamps,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Timestamps {
    pub earliest: Option<String>,
    pub latest: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AttributionQuery {
    #[serde(default = "default_days")]
    pub days: u32,
    #[serde(default = "default_repo")]
    pub repo: String,
}

fn default_days() -> u32 {
    7
}
fn default_repo() -> String {
    "all".into()
}

// ── Cache ─────────────────────────────────────────────────────────────────────

const CACHE_TTL: Duration = Duration::from_secs(60);

struct CacheEntry {
    computed_at: Instant,
    response: AttributionResponse,
}

pub struct AttributionCache {
    inner: Mutex<HashMap<(u32, String), CacheEntry>>,
}

impl Default for AttributionCache {
    fn default() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }
}

impl AttributionCache {
    pub fn get(&self, days: u32, repo: &str) -> Option<AttributionResponse> {
        let guard = self.inner.lock().unwrap();
        let entry = guard.get(&(days, repo.to_owned()))?;
        if entry.computed_at.elapsed() < CACHE_TTL {
            Some(entry.response.clone())
        } else {
            None
        }
    }

    pub fn set(&self, days: u32, repo: &str, response: AttributionResponse) {
        let mut guard = self.inner.lock().unwrap();
        guard.insert(
            (days, repo.to_owned()),
            CacheEntry {
                computed_at: Instant::now(),
                response,
            },
        );
    }
}

// ── Attribution logic ─────────────────────────────────────────────────────────

#[derive(Debug)]
struct CommitRecord {
    subject: String,
    author_name: String,
    body: String,
    added: i64,
    deleted: i64,
    files_changed: u64,
    iso_date: String,
}

/// Classify a commit into one of: qwen3 | sonnet | burak | other
fn classify(rec: &CommitRecord) -> &'static str {
    if rec.subject.starts_with("[qwen-amele]")
        || rec.body.to_lowercase().contains("co-authored-by: qwen")
    {
        return "qwen3";
    }
    let has_sonnet_trailer = rec.body.to_lowercase().contains("co-authored-by: claude");
    let is_burak = rec.author_name.to_lowercase().contains("burak")
        || rec.author_name.to_lowercase().contains("gnomish");
    if has_sonnet_trailer && !is_burak {
        return "sonnet";
    }
    if is_burak {
        return "burak";
    }
    "other"
}

/// Run `git log -z` with NUL-separated records and parse into CommitRecords.
/// Format: %H\x1f%s\x1f%an\x1f%aI\x1f%b  — fields separated by unit-separator (0x1f)
/// Commits separated by NUL (0x00) via -z flag.
#[instrument(skip_all, fields(repo = ?repo_path, days))]
fn collect_commits(repo_path: &Path, days: u32) -> Result<Vec<CommitRecord>> {
    // days is a u32 — safe to interpolate
    let since = format!("{days} days ago");

    // Phase 1: collect commit metadata
    let meta_out = Command::new("git")
        .args([
            "-C",
            repo_path.to_str().context("non-UTF8 repo path")?,
            "log",
            "-z",
            &format!("--since={since}"),
            "--format=%H\x1f%s\x1f%an\x1f%aI\x1f%b",
        ])
        .output()
        .context("git log failed")?;

    if !meta_out.status.success() {
        return Ok(vec![]);
    }

    // Split on NUL; each chunk is one commit's metadata
    let raw = String::from_utf8_lossy(&meta_out.stdout);
    let mut records: Vec<CommitRecord> = raw
        .split('\0')
        .filter(|c| !c.trim().is_empty())
        .filter_map(|chunk| {
            let mut parts = chunk.splitn(5, '\x1f');
            let hash = parts.next()?.trim();
            let subject = parts.next()?.trim().to_owned();
            let author = parts.next()?.trim().to_owned();
            let date = parts.next()?.trim().to_owned();
            let body = parts.next().unwrap_or("").trim().to_owned();
            if hash.is_empty() {
                return None;
            }
            Some(CommitRecord {
                subject,
                author_name: author,
                body,
                iso_date: date,
                added: 0,
                deleted: 0,
                files_changed: 0,
            })
        })
        .collect();

    if records.is_empty() {
        return Ok(records);
    }

    // Phase 2: get numstat totals — one pass over all commits in range
    let numstat_out = Command::new("git")
        .args([
            "-C",
            repo_path.to_str().context("non-UTF8 repo path")?,
            "log",
            &format!("--since={since}"),
            "--numstat",
            "--format=%H",
        ])
        .output()
        .context("git log --numstat failed")?;

    // Build map: commit_hash -> (added, deleted, files)
    let mut stat_map: HashMap<String, (i64, i64, u64)> = HashMap::new();
    let numstat_str = String::from_utf8_lossy(&numstat_out.stdout);
    let mut current_hash = String::new();
    for line in numstat_str.lines() {
        // A line that is exactly a git hash (40 hex chars)
        if line.len() == 40 && line.chars().all(|c| c.is_ascii_hexdigit()) {
            current_hash = line.to_owned();
            continue;
        }
        if current_hash.is_empty() || line.is_empty() {
            continue;
        }
        // numstat lines: "<added>\t<deleted>\t<path>" (binary files show "-")
        let mut cols = line.splitn(3, '\t');
        let added: i64 = cols.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let deleted: i64 = cols.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let entry = stat_map.entry(current_hash.clone()).or_default();
        entry.0 += added;
        entry.1 += deleted;
        entry.2 += 1;
    }

    // Enrich records with stats
    // We need hashes — re-run a quick hash-only log to get ordered hashes
    let hash_out = Command::new("git")
        .args([
            "-C",
            repo_path.to_str().context("non-UTF8 repo path")?,
            "log",
            "-z",
            &format!("--since={since}"),
            "--format=%H",
        ])
        .output()
        .context("git log hash pass failed")?;
    let hash_str = String::from_utf8_lossy(&hash_out.stdout);
    let hashes: Vec<&str> = hash_str
        .split('\0')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();

    for (rec, hash) in records.iter_mut().zip(hashes.iter()) {
        if let Some((a, d, f)) = stat_map.get(*hash) {
            rec.added = *a;
            rec.deleted = *d;
            rec.files_changed = *f;
        }
    }

    Ok(records)
}

fn aggregate(
    records: &[CommitRecord],
) -> (
    AuthorBreakdown,
    AuthorBreakdown,
    AuthorBreakdown,
    Timestamps,
) {
    let mut by_commits = AuthorBreakdown::default();
    let mut by_loc = AuthorBreakdown::default();
    let mut by_files = AuthorBreakdown::default();
    let mut earliest: Option<String> = None;
    let mut latest: Option<String> = None;

    for rec in records {
        // track timestamps
        match (&earliest, &rec.iso_date) {
            (None, d) if !d.is_empty() => earliest = Some(d.clone()),
            (Some(e), d) if !d.is_empty() && d < e => earliest = Some(d.clone()),
            _ => {}
        }
        match (&latest, &rec.iso_date) {
            (None, d) if !d.is_empty() => latest = Some(d.clone()),
            (Some(l), d) if !d.is_empty() && d > l => latest = Some(d.clone()),
            _ => {}
        }

        let author = classify(rec);
        let net_loc = (rec.added - rec.deleted).max(0) as u64;
        match author {
            "qwen3" => {
                by_commits.qwen3 += 1;
                by_loc.qwen3 += net_loc;
                by_files.qwen3 += rec.files_changed;
            }
            "sonnet" => {
                by_commits.sonnet += 1;
                by_loc.sonnet += net_loc;
                by_files.sonnet += rec.files_changed;
            }
            "burak" => {
                by_commits.burak += 1;
                by_loc.burak += net_loc;
                by_files.burak += rec.files_changed;
            }
            _ => {
                by_commits.other += 1;
                by_loc.other += net_loc;
                by_files.other += rec.files_changed;
            }
        }
    }

    (
        by_commits,
        by_loc,
        by_files,
        Timestamps { earliest, latest },
    )
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Config passed to the handler (repo paths for cave-runtime and pipeline-platform).
#[derive(Clone, Debug)]
pub struct AttributionConfig {
    pub cave_runtime_root: PathBuf,
    pub pipeline_platform_root: Option<PathBuf>,
}

/// Compute attribution for `days` days, optionally filtered by repo.
/// Results are cached for 60 seconds.
pub fn compute(
    config: &AttributionConfig,
    cache: &AttributionCache,
    days: u32,
    repo: &str,
) -> Result<AttributionResponse> {
    if let Some(cached) = cache.get(days, repo) {
        return Ok(cached);
    }

    let mut all_records: Vec<CommitRecord> = Vec::new();

    let include_runtime = matches!(repo, "all" | "cave-runtime");
    let include_pipeline = matches!(repo, "all" | "pipeline-platform");

    if include_runtime {
        let mut recs = collect_commits(&config.cave_runtime_root, days)?;
        all_records.append(&mut recs);
    }

    if include_pipeline {
        if let Some(pp_root) = &config.pipeline_platform_root {
            let mut recs = collect_commits(pp_root, days)?;
            all_records.append(&mut recs);
        }
    }

    let (by_commits, by_loc_net, by_files, timestamps) = aggregate(&all_records);

    let response = AttributionResponse {
        period_days: days,
        repo: repo.to_owned(),
        by_commits,
        by_loc_net,
        by_files,
        timestamps,
    };

    cache.set(days, repo, response.clone());
    Ok(response)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_fixture_repo(dir: &std::path::Path) {
        // init
        Command::new("git")
            .args(["-C", dir.to_str().unwrap(), "init"])
            .output()
            .unwrap();
        Command::new("git")
            .args([
                "-C",
                dir.to_str().unwrap(),
                "config",
                "user.email",
                "test@test.com",
            ])
            .output()
            .unwrap();
        Command::new("git")
            .args(["-C", dir.to_str().unwrap(), "config", "user.name", "Test"])
            .output()
            .unwrap();

        for (msg, author) in [
            (
                "[qwen-amele] cave-runtime/abc12345: tier1(cave-sign): draft ed25519 — ed25519-dalek/ed25519-dalek",
                "CAVE Contributors",
            ),
            (
                "feat(cave-auth): add RBAC middleware\n\nCo-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>",
                "CAVE Contributors",
            ),
            ("chore: fix typo in README", "Burak Tartan"),
        ] {
            fs::write(dir.join("file.txt"), msg.as_bytes()).unwrap();
            Command::new("git")
                .args(["-C", dir.to_str().unwrap(), "add", "."])
                .output()
                .unwrap();
            Command::new("git")
                .args([
                    "-C",
                    dir.to_str().unwrap(),
                    "-c",
                    &format!("user.name={author}"),
                    "-c",
                    "user.email=test@test.com",
                    "commit",
                    "-m",
                    msg,
                ])
                .output()
                .unwrap();
        }
    }

    #[test]
    fn test_attribution_fixture_repo() {
        let tmp = tempfile::tempdir().unwrap();
        make_fixture_repo(tmp.path());

        let config = AttributionConfig {
            cave_runtime_root: tmp.path().to_path_buf(),
            pipeline_platform_root: None,
        };
        let cache = AttributionCache::default();
        let result = compute(&config, &cache, 1, "cave-runtime").unwrap();

        assert_eq!(result.by_commits.qwen3, 1, "qwen3 commit");
        assert_eq!(result.by_commits.sonnet, 1, "sonnet commit");
        assert_eq!(result.by_commits.burak, 1, "burak commit");
        assert_eq!(result.by_commits.other, 0, "no other");
    }

    #[test]
    fn test_cache_returns_same_object() {
        let tmp = tempfile::tempdir().unwrap();
        make_fixture_repo(tmp.path());
        let config = AttributionConfig {
            cave_runtime_root: tmp.path().to_path_buf(),
            pipeline_platform_root: None,
        };
        let cache = AttributionCache::default();
        let r1 = compute(&config, &cache, 1, "all").unwrap();
        let r2 = compute(&config, &cache, 1, "all").unwrap();
        assert_eq!(r1.by_commits.qwen3, r2.by_commits.qwen3);
    }
}
