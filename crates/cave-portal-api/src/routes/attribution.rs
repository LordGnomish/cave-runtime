//! Attribution dashboard backend.
//!
//! The pump (`run-cycle.sh`) tags every commit with the model that produced it
//! — `qwen3.6:27b`, `gpt-4o`, `gemini-2.5-flash-lite`, `fallback-stub`, or one
//! of the Claude-family names (`sonnet`, `opus`, `haiku`, generic `claude-`).
//! The marker lands either in the subject, the body, or a `Co-Authored-By:`
//! trailer. Human-authored commits (Burak) carry no marker, and anything that
//! slips through the regex falls into `unattributed` — surfaced explicitly in
//! the UI so we know how much of the history is unaccounted for.
//!
//! Three endpoints:
//!   - GET /api/portal/attribution/summary
//!   - GET /api/portal/attribution/timeline?from=&to=&granularity=day|week|month
//!   - GET /api/portal/attribution/by-module
//!
//! Security: all `git log` invocations use `-z` with NUL/0x1f-separated
//! records so commit bodies cannot inject extra fields. User-provided
//! `from`/`to` strings are validated as ISO-8601 dates and re-formatted before
//! being passed to git.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::get,
    Router,
};
use chrono::{DateTime, Datelike, FixedOffset, NaiveDate};
use serde::{Deserialize, Serialize};
use tracing::instrument;

pub use super::state::PortalDataState as AttributionState;

// ── Prometheus counter ────────────────────────────────────────────────────────

use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::registry::Registry;

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct AttributionLabels {
    pub repo: String,
    pub model: String,
}

pub struct AttributionMetrics {
    pub commits_total: Family<AttributionLabels, Counter>,
}

impl AttributionMetrics {
    pub fn new(registry: &mut Registry) -> Self {
        let commits_total = Family::<AttributionLabels, Counter>::default();
        registry.register(
            "cave_portal_attribution_commits_total",
            "Number of commits attributed to each model category",
            commits_total.clone(),
        );
        Self { commits_total }
    }

    pub fn record(&self, repo: &str, model: &str) {
        self.commits_total
            .get_or_create(&AttributionLabels {
                repo: repo.to_owned(),
                model: model.to_owned(),
            })
            .inc();
    }
}

// ── Model classification ──────────────────────────────────────────────────────

/// Canonical model labels surfaced in the dashboard. Order matters in
/// [`classify_model`]: more-specific markers are checked first because trailer
/// strings like "Claude Opus 4.7" contain both "claude" and "opus".
pub const MODEL_LABELS: &[&str] = &[
    "qwen",
    "gpt-4o",
    "gemini",
    "fallback-stub",
    "opus",
    "sonnet",
    "haiku",
    "claude",
    "burak",
    "unattributed",
];

#[derive(Debug, Clone)]
pub struct CommitRecord {
    pub hash: String,
    pub subject: String,
    pub author_name: String,
    pub body: String,
    pub iso_date: String,
    pub files: Vec<FileChange>,
}

#[derive(Debug, Clone)]
pub struct FileChange {
    pub path: String,
    pub added: u64,
    pub removed: u64,
}

impl CommitRecord {
    pub fn total_added(&self) -> u64 {
        self.files.iter().map(|f| f.added).sum()
    }
    pub fn total_removed(&self) -> u64 {
        self.files.iter().map(|f| f.removed).sum()
    }
    pub fn files_changed(&self) -> u64 {
        self.files.len() as u64
    }
}

/// Classify a commit into a model label. The full subject + body is folded to
/// lowercase once and scanned for canonical substrings; the first match wins.
/// If no model marker is present we fall back to author-based attribution
/// (Burak), and finally to `unattributed`.
pub fn classify_model(rec: &CommitRecord) -> &'static str {
    let blob = format!("{}\n{}", rec.subject, rec.body).to_lowercase();
    let author = rec.author_name.to_lowercase();

    // qwen — pump's local-LLM lane
    if rec.subject.starts_with("[qwen-amele]")
        || blob.contains("qwen")
        || author.contains("qwen")
        || author.contains("cave-local-llm")
    {
        return "qwen";
    }
    // OpenAI: gpt-4o is the canonical bucket; anything gpt-* lands here.
    if blob.contains("gpt-4o") || blob.contains("gpt-4") || blob.contains("gpt-3.5") {
        return "gpt-4o";
    }
    // Google
    if blob.contains("gemini") {
        return "gemini";
    }
    // Pump fallback when no provider is reachable
    if blob.contains("fallback-stub") {
        return "fallback-stub";
    }
    // Claude family — most-specific first
    if blob.contains("opus") {
        return "opus";
    }
    if blob.contains("sonnet") {
        return "sonnet";
    }
    if blob.contains("haiku") {
        return "haiku";
    }
    if blob.contains("co-authored-by: claude") || blob.contains("claude-") {
        return "claude";
    }
    // Human author
    if author.contains("burak") || author.contains("gnomish") {
        return "burak";
    }
    "unattributed"
}

/// Module name for a path. `crates/<X>/...` → `<X>`; everything else uses the
/// first path segment (or `root` for top-level files).
pub fn module_for_path(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match parts.as_slice() {
        ["crates", crate_name, ..] => (*crate_name).to_string(),
        [first, ..] => (*first).to_string(),
        [] => "root".to_string(),
    }
}

// ── Git collection ────────────────────────────────────────────────────────────

/// Walk every commit reachable from any ref (`--all`) and collect metadata +
/// per-file numstats. Two passes: one for headers (NUL-separated, 0x1f-delim
/// fields), one for `--numstat` per file.
#[instrument(skip_all, fields(repo = ?repo_path))]
pub fn collect_all_commits(repo_path: &Path) -> Result<Vec<CommitRecord>> {
    let repo_str = repo_path.to_str().context("non-UTF8 repo path")?;

    let meta = Command::new("git")
        .args([
            "-C",
            repo_str,
            "log",
            "--all",
            "-z",
            "--format=%H\x1f%s\x1f%an\x1f%aI\x1f%b",
        ])
        .output()
        .context("git log (meta) failed")?;
    if !meta.status.success() {
        return Ok(vec![]);
    }
    let raw = String::from_utf8_lossy(&meta.stdout);
    let mut by_hash: HashMap<String, CommitRecord> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for chunk in raw.split('\0').filter(|c| !c.trim().is_empty()) {
        let mut parts = chunk.splitn(5, '\x1f');
        let hash = parts.next().unwrap_or("").trim();
        if hash.is_empty() {
            continue;
        }
        let subject = parts.next().unwrap_or("").trim().to_owned();
        let author = parts.next().unwrap_or("").trim().to_owned();
        let date = parts.next().unwrap_or("").trim().to_owned();
        let body = parts.next().unwrap_or("").trim().to_owned();
        order.push(hash.to_owned());
        by_hash.insert(
            hash.to_owned(),
            CommitRecord {
                hash: hash.to_owned(),
                subject,
                author_name: author,
                body,
                iso_date: date,
                files: Vec::new(),
            },
        );
    }

    if by_hash.is_empty() {
        return Ok(vec![]);
    }

    // numstat pass: each commit prints `<hash>\n` followed by tab-separated
    // numstat lines, then blank line. Binary files render `-\t-\t<path>` —
    // skip those (they don't have line counts).
    let stat = Command::new("git")
        .args(["-C", repo_str, "log", "--all", "--numstat", "--format=%H"])
        .output()
        .context("git log (numstat) failed")?;
    let stat_str = String::from_utf8_lossy(&stat.stdout);
    let mut current_hash = String::new();
    for line in stat_str.lines() {
        if line.len() == 40 && line.chars().all(|c| c.is_ascii_hexdigit()) {
            current_hash = line.to_owned();
            continue;
        }
        if line.is_empty() || current_hash.is_empty() {
            continue;
        }
        let mut cols = line.splitn(3, '\t');
        let added_s = cols.next().unwrap_or("0");
        let removed_s = cols.next().unwrap_or("0");
        let path = cols.next().unwrap_or("").trim().to_owned();
        if path.is_empty() {
            continue;
        }
        let added: u64 = added_s.parse().unwrap_or(0);
        let removed: u64 = removed_s.parse().unwrap_or(0);
        if let Some(rec) = by_hash.get_mut(&current_hash) {
            rec.files.push(FileChange { path, added, removed });
        }
    }

    Ok(order
        .into_iter()
        .filter_map(|h| by_hash.remove(&h))
        .collect())
}

// ── Aggregations ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelStats {
    pub commits: u64,
    pub files_changed: u64,
    pub lines_added: u64,
    pub lines_removed: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SummaryResponse {
    pub total_commits: u64,
    pub by_model: BTreeMap<String, ModelStats>,
    pub generated_at: String,
}

pub fn build_summary(records: &[CommitRecord]) -> SummaryResponse {
    let mut by_model: BTreeMap<String, ModelStats> = BTreeMap::new();
    for rec in records {
        let m = classify_model(rec).to_owned();
        let entry = by_model.entry(m).or_default();
        entry.commits += 1;
        entry.files_changed += rec.files_changed();
        entry.lines_added += rec.total_added();
        entry.lines_removed += rec.total_removed();
    }
    SummaryResponse {
        total_commits: records.len() as u64,
        by_model,
        generated_at: chrono::Utc::now().to_rfc3339(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Granularity {
    Day,
    Week,
    Month,
}

impl Granularity {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "day" | "daily" => Some(Self::Day),
            "week" | "weekly" => Some(Self::Week),
            "month" | "monthly" => Some(Self::Month),
            _ => None,
        }
    }

    fn bucket(&self, iso_date: &str) -> Option<String> {
        let dt = DateTime::<FixedOffset>::parse_from_rfc3339(iso_date).ok()?;
        let date = dt.date_naive();
        Some(match self {
            Self::Day => date.format("%Y-%m-%d").to_string(),
            Self::Week => {
                let iso = date.iso_week();
                format!("{}-W{:02}", iso.year(), iso.week())
            }
            Self::Month => date.format("%Y-%m").to_string(),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TimelineBucket {
    pub bucket: String,
    pub by_model: BTreeMap<String, ModelStats>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimelineResponse {
    pub granularity: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub buckets: Vec<TimelineBucket>,
    pub generated_at: String,
}

pub fn build_timeline(
    records: &[CommitRecord],
    granularity: Granularity,
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
) -> TimelineResponse {
    let mut buckets: BTreeMap<String, BTreeMap<String, ModelStats>> = BTreeMap::new();
    for rec in records {
        let Some(date) = DateTime::<FixedOffset>::parse_from_rfc3339(&rec.iso_date)
            .ok()
            .map(|d| d.date_naive())
        else {
            continue;
        };
        if let Some(f) = from {
            if date < f {
                continue;
            }
        }
        if let Some(t) = to {
            if date > t {
                continue;
            }
        }
        let Some(bucket_key) = granularity.bucket(&rec.iso_date) else {
            continue;
        };
        let bucket = buckets.entry(bucket_key).or_default();
        let model = classify_model(rec).to_owned();
        let stats = bucket.entry(model).or_default();
        stats.commits += 1;
        stats.files_changed += rec.files_changed();
        stats.lines_added += rec.total_added();
        stats.lines_removed += rec.total_removed();
    }
    let buckets: Vec<TimelineBucket> = buckets
        .into_iter()
        .map(|(bucket, by_model)| TimelineBucket { bucket, by_model })
        .collect();
    TimelineResponse {
        granularity: match granularity {
            Granularity::Day => "day",
            Granularity::Week => "week",
            Granularity::Month => "month",
        }
        .to_owned(),
        from: from.map(|d| d.to_string()),
        to: to.map(|d| d.to_string()),
        buckets,
        generated_at: chrono::Utc::now().to_rfc3339(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ByModuleResponse {
    pub modules: BTreeMap<String, BTreeMap<String, ModelStats>>,
    pub generated_at: String,
}

/// Per-module breakdown. A commit can touch multiple modules; each file's
/// stats are credited to its module under the commit's model label.
pub fn build_by_module(records: &[CommitRecord]) -> ByModuleResponse {
    let mut modules: BTreeMap<String, BTreeMap<String, ModelStats>> = BTreeMap::new();
    for rec in records {
        let model = classify_model(rec);
        // Group this commit's files by module to credit `commits` once per
        // (module, model) pair rather than per-file.
        let mut per_module: HashMap<String, (u64, u64, u64)> = HashMap::new();
        for f in &rec.files {
            let m = module_for_path(&f.path);
            let entry = per_module.entry(m).or_default();
            entry.0 += 1;
            entry.1 += f.added;
            entry.2 += f.removed;
        }
        for (module, (files, added, removed)) in per_module {
            let bucket = modules
                .entry(module)
                .or_default()
                .entry(model.to_owned())
                .or_default();
            bucket.commits += 1;
            bucket.files_changed += files;
            bucket.lines_added += added;
            bucket.lines_removed += removed;
        }
    }
    ByModuleResponse {
        modules,
        generated_at: chrono::Utc::now().to_rfc3339(),
    }
}

// ── Handlers ──────────────────────────────────────────────────────────────────

fn err_response(e: anyhow::Error) -> (StatusCode, Json<serde_json::Value>) {
    tracing::error!(error = %e, "attribution failure");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": e.to_string() })),
    )
}

async fn summary_handler(
    State(state): State<AttributionState>,
) -> Result<Json<SummaryResponse>, (StatusCode, Json<serde_json::Value>)> {
    let recs = state.records().map_err(err_response)?;
    Ok(Json(build_summary(&recs)))
}

#[derive(Debug, Deserialize)]
pub struct TimelineQuery {
    pub from: Option<String>,
    pub to: Option<String>,
    #[serde(default = "default_granularity")]
    pub granularity: String,
}

fn default_granularity() -> String {
    "month".to_string()
}

async fn timeline_handler(
    State(state): State<AttributionState>,
    Query(q): Query<TimelineQuery>,
) -> Result<Json<TimelineResponse>, (StatusCode, Json<serde_json::Value>)> {
    let granularity = Granularity::parse(&q.granularity).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({ "error": "granularity must be day|week|month" }),
            ),
        )
    })?;
    let parse_date = |s: &str| {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({ "error": format!("invalid date: {s} — use YYYY-MM-DD") }),
                ),
            )
        })
    };
    let from = q.from.as_deref().map(parse_date).transpose()?;
    let to = q.to.as_deref().map(parse_date).transpose()?;

    let recs = state.records().map_err(err_response)?;
    Ok(Json(build_timeline(&recs, granularity, from, to)))
}

async fn by_module_handler(
    State(state): State<AttributionState>,
) -> Result<Json<ByModuleResponse>, (StatusCode, Json<serde_json::Value>)> {
    let recs = state.records().map_err(err_response)?;
    Ok(Json(build_by_module(&recs)))
}

async fn dashboard_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "text/html; charset=utf-8")],
        include_str!("attribution_dashboard.html"),
    )
}

/// Attribution router: three JSON endpoints + the HTML dashboard at
/// `/admin/attribution`.
pub fn router(state: AttributionState) -> Router {
    Router::new()
        .route("/api/portal/attribution/summary", get(summary_handler))
        .route("/api/portal/attribution/timeline", get(timeline_handler))
        .route("/api/portal/attribution/by-module", get(by_module_handler))
        .route("/admin/attribution", get(dashboard_handler))
        .with_state(state)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::Request;
    use std::fs;
    use tower::ServiceExt;

    fn run(args: &[&str], dir: &Path) {
        let out = Command::new("git").args(args).current_dir(dir).output().unwrap();
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Build a fixture repo with five commits across qwen / sonnet / opus /
    /// gpt-4o / gemini / fallback-stub / burak / unattributed, plus per-file
    /// changes spanning multiple modules.
    fn make_fixture(dir: &Path) {
        run(&["init", "-q", "-b", "main"], dir);
        run(&["config", "user.email", "fixture@test"], dir);
        run(&["config", "user.name", "Fixture"], dir);
        run(&["config", "commit.gpgsign", "false"], dir);

        let commits: &[(&str, &str, &str, &[(&str, &str)])] = &[
            // (subject, author, body, files: (relpath, content))
            (
                "[qwen-amele] tier1: bootstrap",
                "CAVE Contributors",
                "",
                &[
                    ("crates/cave-foo/src/lib.rs", "// foo\n"),
                    ("crates/cave-foo/Cargo.toml", "[package]\nname=\"foo\"\n"),
                ],
            ),
            (
                "feat(cave-bar): add RBAC",
                "CAVE Contributors",
                "Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>",
                &[("crates/cave-bar/src/rbac.rs", "// rbac\n")],
            ),
            (
                "feat: opus refactor",
                "CAVE Contributors",
                "Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>",
                &[
                    ("crates/cave-foo/src/lib.rs", "// foo\n// updated\n"),
                    ("docs/notes.md", "notes\n"),
                ],
            ),
            (
                "experiment via gpt-4o on baz module",
                "Burak Tartan",
                "",
                &[("crates/cave-baz/src/main.rs", "fn main(){}\n")],
            ),
            (
                "draft from gemini-2.5-flash-lite",
                "Burak Tartan",
                "",
                &[("crates/cave-baz/README.md", "# baz\n")],
            ),
            (
                "fallback-stub fill-in for incident playbook",
                "Burak Tartan",
                "",
                &[("docs/playbooks/incident.md", "# incident\n")],
            ),
            (
                "chore: typo fix by burak",
                "Burak Tartan",
                "",
                &[("crates/cave-bar/src/rbac.rs", "// rbac fixed\n")],
            ),
            (
                "merge from upstream",
                "Random Contributor",
                "",
                &[("crates/cave-foo/src/extra.rs", "// extra\n")],
            ),
        ];

        for (subject, author, body, files) in commits {
            for (relpath, content) in files.iter() {
                let full = dir.join(relpath);
                fs::create_dir_all(full.parent().unwrap()).unwrap();
                fs::write(full, content).unwrap();
            }
            run(&["add", "-A"], dir);
            let msg = if body.is_empty() {
                (*subject).to_owned()
            } else {
                format!("{subject}\n\n{body}")
            };
            run(
                &[
                    "-c",
                    &format!("user.name={author}"),
                    "-c",
                    "user.email=test@test",
                    "commit",
                    "-q",
                    "-m",
                    &msg,
                ],
                dir,
            );
        }
    }

    #[test]
    fn classify_handles_all_models() {
        let mk = |subject: &str, body: &str, author: &str| CommitRecord {
            hash: "h".into(),
            subject: subject.into(),
            author_name: author.into(),
            body: body.into(),
            iso_date: "2026-01-01T00:00:00+00:00".into(),
            files: vec![],
        };
        assert_eq!(classify_model(&mk("[qwen-amele] x", "", "")), "qwen");
        assert_eq!(classify_model(&mk("x", "Co-Authored-By: qwen", "")), "qwen");
        assert_eq!(classify_model(&mk("x via gpt-4o", "", "")), "gpt-4o");
        assert_eq!(classify_model(&mk("x", "via gpt-4 model", "")), "gpt-4o");
        assert_eq!(classify_model(&mk("from gemini-2.5-flash-lite", "", "")), "gemini");
        assert_eq!(classify_model(&mk("fallback-stub fill", "", "")), "fallback-stub");
        assert_eq!(
            classify_model(&mk("x", "Co-Authored-By: Claude Opus 4.7", "")),
            "opus"
        );
        assert_eq!(
            classify_model(&mk("x", "Co-Authored-By: Claude Sonnet 4.6", "")),
            "sonnet"
        );
        assert_eq!(
            classify_model(&mk("x", "Co-Authored-By: Claude Haiku 4.5", "")),
            "haiku"
        );
        assert_eq!(
            classify_model(&mk("x", "Co-Authored-By: Claude unknown", "")),
            "claude"
        );
        assert_eq!(classify_model(&mk("typo", "", "Burak Tartan")), "burak");
        assert_eq!(classify_model(&mk("merge", "", "Random Person")), "unattributed");
    }

    #[test]
    fn module_for_path_extracts_crate_name() {
        assert_eq!(module_for_path("crates/cave-foo/src/lib.rs"), "cave-foo");
        assert_eq!(module_for_path("crates/cave-bar/Cargo.toml"), "cave-bar");
        assert_eq!(module_for_path("docs/notes.md"), "docs");
        assert_eq!(module_for_path("README.md"), "README.md");
    }

    #[test]
    fn summary_aggregates_all_buckets() {
        let tmp = tempfile::tempdir().unwrap();
        make_fixture(tmp.path());
        let recs = collect_all_commits(tmp.path()).unwrap();
        let s = build_summary(&recs);
        assert_eq!(s.total_commits, 8);
        assert_eq!(s.by_model.get("qwen").unwrap().commits, 1);
        assert_eq!(s.by_model.get("sonnet").unwrap().commits, 1);
        assert_eq!(s.by_model.get("opus").unwrap().commits, 1);
        assert_eq!(s.by_model.get("gpt-4o").unwrap().commits, 1);
        assert_eq!(s.by_model.get("gemini").unwrap().commits, 1);
        assert_eq!(s.by_model.get("fallback-stub").unwrap().commits, 1);
        assert_eq!(s.by_model.get("burak").unwrap().commits, 1);
        assert_eq!(s.by_model.get("unattributed").unwrap().commits, 1);
        // line totals on the qwen commit: 2 files, both new — non-zero added
        assert!(s.by_model.get("qwen").unwrap().lines_added > 0);
        assert_eq!(s.by_model.get("qwen").unwrap().lines_removed, 0);
    }

    #[test]
    fn by_module_attributes_per_file() {
        let tmp = tempfile::tempdir().unwrap();
        make_fixture(tmp.path());
        let recs = collect_all_commits(tmp.path()).unwrap();
        let bm = build_by_module(&recs);
        // qwen touched cave-foo (2 files in one commit)
        let foo = bm.modules.get("cave-foo").expect("cave-foo present");
        assert!(foo.contains_key("qwen"));
        assert_eq!(foo.get("qwen").unwrap().files_changed, 2);
        // opus touched cave-foo (1) AND docs (1)
        assert!(foo.contains_key("opus"));
        assert!(bm.modules.get("docs").unwrap().contains_key("opus"));
        // unattributed merge commit went into cave-foo
        assert!(foo.contains_key("unattributed"));
    }

    #[test]
    fn timeline_buckets_by_month() {
        let tmp = tempfile::tempdir().unwrap();
        make_fixture(tmp.path());
        let recs = collect_all_commits(tmp.path()).unwrap();
        let t = build_timeline(&recs, Granularity::Month, None, None);
        assert!(!t.buckets.is_empty(), "at least one month bucket");
        let total: u64 = t
            .buckets
            .iter()
            .flat_map(|b| b.by_model.values())
            .map(|s| s.commits)
            .sum();
        assert_eq!(total, 8);
    }

    #[test]
    fn timeline_filters_by_date_range() {
        let tmp = tempfile::tempdir().unwrap();
        make_fixture(tmp.path());
        let recs = collect_all_commits(tmp.path()).unwrap();
        // Bracket the future — should exclude every fixture commit.
        let from = NaiveDate::from_ymd_opt(2099, 1, 1);
        let to = NaiveDate::from_ymd_opt(2099, 12, 31);
        let t = build_timeline(&recs, Granularity::Day, from, to);
        let total: u64 = t
            .buckets
            .iter()
            .flat_map(|b| b.by_model.values())
            .map(|s| s.commits)
            .sum();
        assert_eq!(total, 0);
    }

    #[tokio::test]
    async fn http_summary_returns_json() {
        let tmp = tempfile::tempdir().unwrap();
        make_fixture(tmp.path());
        let app = router(AttributionState::new(tmp.path()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/portal/attribution/summary")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["total_commits"], 8);
        assert_eq!(v["by_model"]["qwen"]["commits"], 1);
        assert_eq!(v["by_model"]["fallback-stub"]["commits"], 1);
        assert_eq!(v["by_model"]["unattributed"]["commits"], 1);
    }

    #[tokio::test]
    async fn http_timeline_rejects_bad_granularity() {
        let tmp = tempfile::tempdir().unwrap();
        make_fixture(tmp.path());
        let app = router(AttributionState::new(tmp.path()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/portal/attribution/timeline?granularity=hourly")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn http_dashboard_serves_html() {
        let tmp = tempfile::tempdir().unwrap();
        make_fixture(tmp.path());
        let app = router(AttributionState::new(tmp.path()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/admin/attribution")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp.headers().get("content-type").unwrap();
        assert!(ct.to_str().unwrap().starts_with("text/html"));
    }

    #[test]
    fn cache_returns_same_records_within_ttl() {
        let tmp = tempfile::tempdir().unwrap();
        make_fixture(tmp.path());
        let st = AttributionState::new(tmp.path());
        let r1 = st.records().unwrap();
        let r2 = st.records().unwrap();
        assert!(std::sync::Arc::ptr_eq(&r1, &r2), "cache should reuse the same Arc");
    }
}
