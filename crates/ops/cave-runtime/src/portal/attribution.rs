// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Attribution dashboard — answers "who wrote what" by parsing `git log`.
//!
//! Three author buckets, derived from commit metadata:
//!   * **Qwen**   — author or message contains `qwen-amele`
//!   * **Sonnet** — `Co-Authored-By: Claude Sonnet …` line in body
//!   * **Opus**   — `Co-Authored-By: Claude Opus …` line in body
//!   * **Other**  — everything else (humans + automation)
//!
//! Each commit may belong to multiple buckets when several Co-Authored-By
//! lines are present (e.g. Sonnet drafted, Opus reviewed). The dashboard
//! attributes the full LOC to each contributing bucket — totals can exceed
//! the raw repo total, by design.
//!
//! Routes
//! ──────
//!   GET /attribution                          → dashboard HTML
//!   GET /api/v1/attribution/authors           → per-author totals (7d/30d/all)
//!   GET /api/v1/attribution/by-module         → per-(author, module) LOC
//!   GET /api/v1/attribution/timeseries?days=N → daily counts per author

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    process::Command,
};

use axum::{Json, Router, extract::Query, response::Html, routing::get};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::workspace_root;

static PAGE_HTML: &str = include_str!("templates/attribution.html");

const BUCKETS: &[&str] = &["qwen", "sonnet", "opus", "other"];

#[derive(Debug, Clone, Default, Serialize)]
pub struct AuthorStats {
    pub commits_all: u64,
    pub commits_30d: u64,
    pub commits_7d: u64,
    pub loc_added: u64,
    pub loc_removed: u64,
    pub modules_touched: u64,
}

#[derive(Debug, Clone)]
struct CommitRecord {
    when: DateTime<Utc>,
    buckets: HashSet<&'static str>,
    added: u64,
    removed: u64,
    crates_touched: HashSet<String>,
}

pub async fn page() -> Html<&'static str> {
    Html(PAGE_HTML)
}

pub async fn api_authors() -> Json<serde_json::Value> {
    let commits = load_commits(/*days*/ None).unwrap_or_default();
    let now = Utc::now();
    let mut by_author: HashMap<&'static str, AuthorStats> = HashMap::new();
    let mut crates_by_author: HashMap<&'static str, HashSet<String>> = HashMap::new();

    for c in &commits {
        for b in &c.buckets {
            let entry = by_author.entry(*b).or_default();
            entry.commits_all += 1;
            if now - c.when <= Duration::days(30) {
                entry.commits_30d += 1;
            }
            if now - c.when <= Duration::days(7) {
                entry.commits_7d += 1;
            }
            entry.loc_added += c.added;
            entry.loc_removed += c.removed;
            crates_by_author
                .entry(*b)
                .or_default()
                .extend(c.crates_touched.iter().cloned());
        }
    }
    for (b, set) in &crates_by_author {
        if let Some(stats) = by_author.get_mut(b) {
            stats.modules_touched = set.len() as u64;
        }
    }

    let payload: BTreeMap<&str, AuthorStats> = BUCKETS
        .iter()
        .map(|b| (*b, by_author.remove(*b).unwrap_or_default()))
        .collect();

    Json(json!({
        "buckets": BUCKETS,
        "authors": payload,
        "total_commits_analysed": commits.len(),
    }))
}

pub async fn api_by_module() -> Json<serde_json::Value> {
    let commits = load_commits(None).unwrap_or_default();
    // (module, bucket) → (commits, loc_added, loc_removed)
    let mut acc: HashMap<(String, &'static str), (u64, u64, u64)> = HashMap::new();
    for c in &commits {
        for b in &c.buckets {
            for m in &c.crates_touched {
                let entry = acc.entry((m.clone(), *b)).or_default();
                entry.0 += 1;
                entry.1 += c.added;
                entry.2 += c.removed;
            }
        }
    }
    let mut rows: Vec<serde_json::Value> = acc
        .into_iter()
        .map(|((module, bucket), (commits, added, removed))| {
            json!({
                "module": module,
                "bucket": bucket,
                "commits": commits,
                "loc_added": added,
                "loc_removed": removed,
            })
        })
        .collect();
    rows.sort_by(|a, b| {
        b["loc_added"]
            .as_u64()
            .unwrap_or(0)
            .cmp(&a["loc_added"].as_u64().unwrap_or(0))
    });
    Json(json!({ "rows": rows }))
}

#[derive(Debug, Deserialize)]
pub struct TimeseriesQuery {
    #[serde(default = "default_days")]
    pub days: i64,
}

fn default_days() -> i64 {
    14
}

pub async fn api_timeseries(Query(q): Query<TimeseriesQuery>) -> Json<serde_json::Value> {
    let days = q.days.clamp(1, 90);
    let commits = load_commits(Some(days)).unwrap_or_default();
    // date → bucket → count
    let mut acc: BTreeMap<NaiveDate, HashMap<&'static str, u64>> = BTreeMap::new();
    for c in &commits {
        let day = c.when.date_naive();
        let entry = acc.entry(day).or_default();
        for b in &c.buckets {
            *entry.entry(*b).or_default() += 1;
        }
    }
    // Pad zero-days so the chart x-axis is continuous
    let today = Utc::now().date_naive();
    let mut series: Vec<serde_json::Value> = Vec::with_capacity(days as usize);
    for offset in (0..days).rev() {
        let day = today - Duration::days(offset);
        let counts = acc.get(&day).cloned().unwrap_or_default();
        let mut row = serde_json::Map::new();
        row.insert("date".into(), json!(day.format("%Y-%m-%d").to_string()));
        for b in BUCKETS {
            row.insert((*b).into(), json!(counts.get(b).copied().unwrap_or(0)));
        }
        series.push(serde_json::Value::Object(row));
    }
    Json(json!({
        "days": days,
        "buckets": BUCKETS,
        "series": series,
    }))
}

// ---------------------------------------------------------------------------
// git log loader
// ---------------------------------------------------------------------------

fn load_commits(since_days: Option<i64>) -> Option<Vec<CommitRecord>> {
    let root = workspace_root();
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(&root);
    cmd.args([
        "log",
        "--no-merges",
        "--numstat",
        "--date=iso-strict",
        "--pretty=format:CAVE_COMMIT|%H|%aI|%an|%ae|%s%n%b%nCAVE_END",
    ]);
    if let Some(d) = since_days {
        cmd.arg(format!("--since={d}.days.ago"));
    }
    cmd.arg("--max-count=4000");
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return Some(Vec::new());
    }
    Some(parse_log(&String::from_utf8_lossy(&out.stdout)))
}

pub fn parse_log(stdout: &str) -> Vec<CommitRecord> {
    let mut out = Vec::new();
    let mut current: Option<PartialCommit> = None;
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("CAVE_COMMIT|") {
            // Flush previous
            if let Some(p) = current.take() {
                if let Some(c) = p.finish() {
                    out.push(c);
                }
            }
            // Header format after the CAVE_COMMIT| prefix is:
            //   <sha>|<author-iso>|<author-name>|<author-email>|<subject>
            // Subject may itself contain '|', so we split into at most 5 chunks.
            let parts: Vec<&str> = rest.splitn(5, '|').collect();
            if parts.len() < 5 {
                continue;
            }
            current = Some(PartialCommit {
                _sha: parts[0].to_string(),
                when_str: parts[1].to_string(),
                author_name: parts[2].to_string(),
                _author_email: parts[3].to_string(),
                subject: parts[4].to_string(),
                body_lines: Vec::new(),
                stats: Vec::new(),
                in_stats: false,
            });
            continue;
        }
        if line == "CAVE_END" {
            if let Some(c) = current.as_mut() {
                c.in_stats = true;
            }
            continue;
        }
        if let Some(c) = current.as_mut() {
            if c.in_stats {
                // numstat row: <added>\t<removed>\t<path>
                let mut it = line.split('\t');
                let a = it.next();
                let r = it.next();
                let p = it.next();
                if let (Some(a), Some(r), Some(p)) = (a, r, p) {
                    if !p.is_empty() {
                        let added: u64 = a.parse().unwrap_or(0);
                        let removed: u64 = r.parse().unwrap_or(0);
                        c.stats.push((added, removed, p.to_string()));
                    }
                }
            } else {
                c.body_lines.push(line.to_string());
            }
        }
    }
    if let Some(p) = current.take() {
        if let Some(c) = p.finish() {
            out.push(c);
        }
    }
    out
}

struct PartialCommit {
    _sha: String,
    when_str: String,
    author_name: String,
    _author_email: String,
    subject: String,
    body_lines: Vec<String>,
    stats: Vec<(u64, u64, String)>,
    in_stats: bool,
}

impl PartialCommit {
    fn finish(self) -> Option<CommitRecord> {
        let when = DateTime::parse_from_rfc3339(&self.when_str)
            .ok()?
            .with_timezone(&Utc);
        let buckets = classify_buckets(&self.author_name, &self.subject, &self.body_lines);
        let mut added = 0u64;
        let mut removed = 0u64;
        let mut crates_touched = HashSet::new();
        for (a, r, p) in &self.stats {
            added += a;
            removed += r;
            if let Some(rest) = p.strip_prefix("crates/") {
                if let Some(c) = rest.split('/').next() {
                    if !c.is_empty() {
                        crates_touched.insert(c.to_string());
                    }
                }
            }
        }
        Some(CommitRecord {
            when,
            buckets,
            added,
            removed,
            crates_touched,
        })
    }
}

pub fn classify_buckets(
    author_name: &str,
    subject: &str,
    body_lines: &[String],
) -> HashSet<&'static str> {
    let mut set = HashSet::new();
    let lower_author = author_name.to_ascii_lowercase();
    let lower_subject = subject.to_ascii_lowercase();

    if lower_author.contains("qwen-amele")
        || lower_subject.contains("qwen-amele")
        || lower_subject.contains("qwen")
    {
        set.insert("qwen");
    }
    for line in body_lines {
        if let Some(rest) = line.trim().strip_prefix("Co-Authored-By:") {
            let r = rest.to_ascii_lowercase();
            if r.contains("claude sonnet") || r.contains("sonnet") {
                set.insert("sonnet");
            }
            if r.contains("claude opus") || r.contains("opus") {
                set.insert("opus");
            }
            if r.contains("qwen") {
                set.insert("qwen");
            }
        }
    }
    if set.is_empty() {
        set.insert("other");
    }
    set
}

pub fn router() -> Router {
    Router::new()
        .route("/attribution", get(page))
        .route("/api/v1/attribution/authors", get(api_authors))
        .route("/api/v1/attribution/by-module", get(api_by_module))
        .route("/api/v1/attribution/timeseries", get(api_timeseries))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_qwen_from_subject() {
        let buckets = classify_buckets("Burak", "qwen-amele port etcd watch", &[]);
        assert!(buckets.contains("qwen"));
    }

    #[test]
    fn classify_sonnet_and_opus_from_coauthor() {
        let body = vec![
            "First line".to_string(),
            "Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>".to_string(),
            "Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>".to_string(),
        ];
        let buckets = classify_buckets("Burak Tartan", "feat: thing", &body);
        assert!(buckets.contains("sonnet"));
        assert!(buckets.contains("opus"));
    }

    #[test]
    fn classify_falls_back_to_other() {
        let buckets = classify_buckets("Some Human", "fix: typo", &[]);
        assert_eq!(buckets, ["other"].iter().copied().collect());
    }

    #[test]
    fn parse_log_extracts_stats_and_buckets() {
        let log = "\
CAVE_COMMIT|abc|2026-05-02T10:00:00+00:00|Burak Tartan|b@x|feat: foo

Co-Authored-By: Claude Sonnet 4.6 <a@b>
CAVE_END
12\t3\tcrates/cave-net/src/lib.rs
4\t0\tcrates/cave-pg/src/foo.rs
CAVE_COMMIT|def|2026-05-01T09:00:00+00:00|qwen-amele|q@x|qwen-amele port etcd
CAVE_END
100\t10\tcrates/cave-etcd/src/lib.rs
";
        let commits = parse_log(log);
        assert_eq!(commits.len(), 2);
        let c0 = &commits[0];
        assert_eq!(c0.added, 16);
        assert_eq!(c0.removed, 3);
        assert!(c0.crates_touched.contains("cave-net"));
        assert!(c0.crates_touched.contains("cave-pg"));
        assert!(c0.buckets.contains("sonnet"));
        let c1 = &commits[1];
        assert!(c1.buckets.contains("qwen"));
        assert!(c1.crates_touched.contains("cave-etcd"));
        assert_eq!(c1.added, 100);
    }

    #[tokio::test]
    async fn timeseries_pads_empty_days() {
        let _g = crate::portal::WORKSPACE_ROOT_TEST_GUARD
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        // Even if we provide no commits at all the response must contain
        // exactly N day rows so the chart axis is continuous.
        // SAFETY: guarded by WORKSPACE_ROOT_TEST_GUARD.
        unsafe {
            std::env::set_var("CAVE_WORKSPACE_ROOT", "/__no_such_dir_for_test__");
        }
        let resp = api_timeseries(Query(TimeseriesQuery { days: 5 })).await;
        let v: serde_json::Value = serde_json::to_value(&resp.0).unwrap();
        assert_eq!(v["series"].as_array().unwrap().len(), 5);
    }

    #[tokio::test]
    async fn authors_endpoint_returns_all_buckets() {
        let _g = crate::portal::WORKSPACE_ROOT_TEST_GUARD
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        // SAFETY: guarded by WORKSPACE_ROOT_TEST_GUARD.
        unsafe {
            std::env::set_var("CAVE_WORKSPACE_ROOT", "/__no_such_dir_for_test__");
        }
        let resp = api_authors().await;
        let v: serde_json::Value = serde_json::to_value(&resp.0).unwrap();
        for b in &["qwen", "sonnet", "opus", "other"] {
            assert!(v["authors"].get(*b).is_some(), "missing bucket {b}");
        }
    }
}
