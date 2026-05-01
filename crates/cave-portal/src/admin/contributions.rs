//! Admin contributions view — "kim ne kadar iş yapıyor" panel.
//!
//! Source of truth: `tools/night-pump/contributions.jsonl`. Each line is one
//! batch outcome appended by the night-pump dispatcher when a worker finishes
//! a batch. Records group naturally by `worker_id`:
//!
//!   - `qwen-3-coder-next`   — local Qwen3 coder (Phase 3 daemon)
//!   - `sonnet-4-6`          — Sonnet 4.6 (cloud)
//!   - `claude-opus-4-7`     — Opus 4.7 (cloud, this assistant)
//!   - `manual-burak`        — keyboard-driven commits
//!   - everything else       — bucketed as `Other`
//!
//! Renders four panels:
//!   - `/admin/contributions`             — overview grouped by worker_id
//!   - `/admin/contributions/<worker_id>` — detail per worker (recent batches)
//!   - `/admin/contributions/timeline`    — hourly bucket activity sparkline
//!   - `/admin/contributions/leaderboard` — top contributors by composite score

use crate::admin::permission::{AuthError, Permission, RequestCtx};
use crate::admin::types::{Cite, TenantId};
use chrono::{DateTime, Datelike, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/explore/src/components/ExplorePage.tsx",
    "ExplorePage",
);

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum WorkerKind {
    Qwen3CoderNext,
    Sonnet46,
    ClaudeOpus47,
    ManualBurak,
    Other,
}

impl WorkerKind {
    pub fn from_worker_id(s: &str) -> Self {
        match s {
            "qwen-3-coder-next" => WorkerKind::Qwen3CoderNext,
            "sonnet-4-6" => WorkerKind::Sonnet46,
            "claude-opus-4-7" => WorkerKind::ClaudeOpus47,
            "manual-burak" => WorkerKind::ManualBurak,
            _ => WorkerKind::Other,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            WorkerKind::Qwen3CoderNext => "qwen-3-coder-next",
            WorkerKind::Sonnet46 => "sonnet-4-6",
            WorkerKind::ClaudeOpus47 => "claude-opus-4-7",
            WorkerKind::ManualBurak => "manual-burak",
            WorkerKind::Other => "other",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contribution {
    pub ts: DateTime<Utc>,
    pub worker_id: String,
    pub batch_id: String,
    pub test_delta: i64,
    pub commit_sha: String,
    pub model: String,
    #[serde(rename = "crate")]
    pub crate_name: String,
    pub eval_seconds: u64,
    pub branch: String,
    pub merged_to: String,
}

impl Contribution {
    pub fn worker_kind(&self) -> WorkerKind {
        WorkerKind::from_worker_id(&self.worker_id)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct WorkerSummary {
    pub worker_id: String,
    pub kind: WorkerKind,
    pub batches: u64,
    pub tests_added: i64,
    pub crates_touched: u64,
    pub eval_seconds_total: u64,
}

impl Default for WorkerKind {
    fn default() -> Self {
        WorkerKind::Other
    }
}

// ── Filter ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ContributionsFilter {
    /// ISO-8601 cutoff; only contributions at-or-after `since` are kept.
    pub since: Option<DateTime<Utc>>,
}

impl ContributionsFilter {
    pub fn since(ts: DateTime<Utc>) -> Self {
        Self { since: Some(ts) }
    }

    pub fn matches(&self, c: &Contribution) -> bool {
        match self.since {
            Some(s) => c.ts >= s,
            None => true,
        }
    }
}

// ── Parsing ───────────────────────────────────────────────────────────────────

/// Parse JSONL bytes, skipping blank lines. Returns the first parse error.
pub fn parse_jsonl(input: &str) -> Result<Vec<Contribution>, String> {
    let mut out = Vec::new();
    for (lineno, raw) in input.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let parsed: Contribution = serde_json::from_str(line)
            .map_err(|e| format!("line {}: {}", lineno + 1, e))?;
        out.push(parsed);
    }
    Ok(out)
}

// ── Aggregation ───────────────────────────────────────────────────────────────

/// Group contributions by worker_id, returning sorted summaries (most batches first).
pub fn aggregate_by_worker(records: &[Contribution]) -> Vec<WorkerSummary> {
    let mut by_id: BTreeMap<String, WorkerSummary> = BTreeMap::new();
    for c in records {
        let entry = by_id
            .entry(c.worker_id.clone())
            .or_insert_with(|| WorkerSummary {
                worker_id: c.worker_id.clone(),
                kind: c.worker_kind(),
                ..Default::default()
            });
        entry.batches += 1;
        entry.tests_added += c.test_delta;
        entry.eval_seconds_total += c.eval_seconds;
    }
    // Compute crates_touched per worker (set of distinct crate names)
    let mut crate_sets: BTreeMap<String, std::collections::BTreeSet<String>> = BTreeMap::new();
    for c in records {
        crate_sets
            .entry(c.worker_id.clone())
            .or_default()
            .insert(c.crate_name.clone());
    }
    for (id, set) in crate_sets {
        if let Some(s) = by_id.get_mut(&id) {
            s.crates_touched = set.len() as u64;
        }
    }
    let mut out: Vec<WorkerSummary> = by_id.into_values().collect();
    out.sort_by(|a, b| {
        b.batches
            .cmp(&a.batches)
            .then_with(|| a.worker_id.cmp(&b.worker_id))
    });
    out
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HourBucket {
    pub hour: DateTime<Utc>,
    pub batches: u64,
    pub tests_added: i64,
}

/// Bucket contributions into UTC-hour rows (oldest → newest, dense — only
/// hours that actually saw activity; empty hours are skipped).
pub fn aggregate_timeline(records: &[Contribution]) -> Vec<HourBucket> {
    let mut by_hour: BTreeMap<DateTime<Utc>, HourBucket> = BTreeMap::new();
    for c in records {
        let key = Utc
            .with_ymd_and_hms(c.ts.year(), c.ts.month(), c.ts.day(), c.ts.hour(), 0, 0)
            .single()
            .expect("UTC hour bucket is always single");
        let entry = by_hour.entry(key).or_insert_with(|| HourBucket {
            hour: key,
            batches: 0,
            tests_added: 0,
        });
        entry.batches += 1;
        entry.tests_added += c.test_delta;
    }
    by_hour.into_values().collect()
}

/// Composite score = batches * 10 + tests_added. Ties broken by tests_added then
/// alphabetical worker_id. Returns at most `limit` rows.
pub fn leaderboard(records: &[Contribution], limit: usize) -> Vec<WorkerSummary> {
    let mut all = aggregate_by_worker(records);
    all.sort_by(|a, b| {
        let sa = (a.batches as i64) * 10 + a.tests_added;
        let sb = (b.batches as i64) * 10 + b.tests_added;
        sb.cmp(&sa)
            .then_with(|| b.tests_added.cmp(&a.tests_added))
            .then_with(|| a.worker_id.cmp(&b.worker_id))
    });
    all.truncate(limit);
    all
}

// ── Detail ────────────────────────────────────────────────────────────────────

/// Most-recent-first slice of a worker's contributions, capped at `limit`.
pub fn worker_detail(records: &[Contribution], worker_id: &str, limit: usize) -> Vec<Contribution> {
    let mut filtered: Vec<Contribution> = records
        .iter()
        .filter(|c| c.worker_id == worker_id)
        .cloned()
        .collect();
    filtered.sort_by(|a, b| b.ts.cmp(&a.ts));
    filtered.truncate(limit);
    filtered
}

// ── Rendering ─────────────────────────────────────────────────────────────────

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn page_shell(title: &str, body: &str, tenant: &TenantId) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>{title} — Cave Portal</title>
  <script src="https://unpkg.com/htmx.org@1.9.10" crossorigin="anonymous"></script>
  <style>
    body {{ font-family: ui-sans-serif, system-ui, sans-serif; margin: 1rem 2rem; color: #1f2937; }}
    h1, h2 {{ font-weight: 600; }}
    table {{ border-collapse: collapse; margin: 1rem 0; width: 100%; }}
    th, td {{ text-align: left; padding: 0.4rem 0.8rem; border-bottom: 1px solid #e5e7eb; }}
    th {{ background: #f3f4f6; font-size: 0.75rem; text-transform: uppercase; letter-spacing: 0.05em; }}
    .badge {{ background: #eef2ff; color: #4338ca; padding: 0.15rem 0.5rem; border-radius: 0.25rem; font-size: 0.75rem; }}
    .num {{ text-align: right; font-variant-numeric: tabular-nums; }}
    nav a {{ margin-right: 1rem; color: #2563eb; text-decoration: none; }}
  </style>
</head>
<body>
  <header>
    <nav>
      <a href="/admin/contributions?tenant_id={t}">Overview</a>
      <a href="/admin/contributions/timeline?tenant_id={t}">Timeline</a>
      <a href="/admin/contributions/leaderboard?tenant_id={t}">Leaderboard</a>
    </nav>
    <h1>{title}</h1>
    <p>Tenant <span class="badge">{t}</span></p>
  </header>
  <main>{body}</main>
</body>
</html>"#,
        title = html_escape(title),
        t = html_escape(tenant.as_str()),
        body = body,
    )
}

pub fn render_overview(
    records: &[Contribution],
    ctx: &RequestCtx,
) -> Result<String, AuthError> {
    ctx.authorise(Permission::ContributionsRead)?;
    let summaries = aggregate_by_worker(records);
    let mut rows = String::new();
    if summaries.is_empty() {
        rows.push_str(r#"<tr><td colspan="5"><em>No contributions in window.</em></td></tr>"#);
    } else {
        for s in &summaries {
            rows.push_str(&format!(
                r#"<tr><td><a href="/admin/contributions/{wid}?tenant_id={t}">{wid}</a></td><td>{kind}</td><td class="num">{batches}</td><td class="num">{tests}</td><td class="num">{crates}</td></tr>"#,
                wid = html_escape(&s.worker_id),
                t = html_escape(ctx.tenant.as_str()),
                kind = s.kind.label(),
                batches = s.batches,
                tests = s.tests_added,
                crates = s.crates_touched,
            ));
        }
    }
    let body = format!(
        r#"<h2>Worker contributions</h2>
<table>
  <thead><tr><th>worker_id</th><th>kind</th><th class="num">batches</th><th class="num">tests +</th><th class="num">crates</th></tr></thead>
  <tbody>{rows}</tbody>
</table>"#,
    );
    Ok(page_shell("Contributions overview", &body, &ctx.tenant))
}

pub fn render_worker_detail(
    records: &[Contribution],
    worker_id: &str,
    ctx: &RequestCtx,
) -> Result<String, AuthError> {
    ctx.authorise(Permission::ContributionsRead)?;
    let recent = worker_detail(records, worker_id, 50);
    let mut rows = String::new();
    if recent.is_empty() {
        rows.push_str(r#"<tr><td colspan="5"><em>No batches recorded for this worker.</em></td></tr>"#);
    } else {
        for c in &recent {
            rows.push_str(&format!(
                r#"<tr><td>{ts}</td><td>{batch}</td><td>{crate_name}</td><td class="num">{tests}</td><td>{sha}</td></tr>"#,
                ts = html_escape(&c.ts.to_rfc3339()),
                batch = html_escape(&c.batch_id),
                crate_name = html_escape(&c.crate_name),
                tests = c.test_delta,
                sha = html_escape(&c.commit_sha[..c.commit_sha.len().min(8)]),
            ));
        }
    }
    let body = format!(
        r#"<h2>Worker: {wid}</h2>
<table>
  <thead><tr><th>ts</th><th>batch</th><th>crate</th><th class="num">tests +</th><th>sha</th></tr></thead>
  <tbody>{rows}</tbody>
</table>"#,
        wid = html_escape(worker_id),
    );
    Ok(page_shell(
        &format!("Worker {worker_id}"),
        &body,
        &ctx.tenant,
    ))
}

pub fn render_timeline(
    records: &[Contribution],
    ctx: &RequestCtx,
) -> Result<String, AuthError> {
    ctx.authorise(Permission::ContributionsRead)?;
    let buckets = aggregate_timeline(records);
    let max = buckets.iter().map(|b| b.batches).max().unwrap_or(0).max(1);
    let mut rows = String::new();
    if buckets.is_empty() {
        rows.push_str(r#"<tr><td colspan="3"><em>No activity in window.</em></td></tr>"#);
    } else {
        for b in &buckets {
            let bar_len = ((b.batches as f64 / max as f64) * 30.0).round() as usize;
            let bar = "█".repeat(bar_len);
            rows.push_str(&format!(
                r#"<tr><td>{hour}</td><td class="num">{batches}</td><td><span style="font-family: monospace; color: #4338ca;">{bar}</span></td></tr>"#,
                hour = html_escape(&b.hour.to_rfc3339()),
                batches = b.batches,
            ));
        }
    }
    let body = format!(
        r#"<h2>Hourly activity (UTC)</h2>
<table>
  <thead><tr><th>hour</th><th class="num">batches</th><th>activity</th></tr></thead>
  <tbody>{rows}</tbody>
</table>"#,
    );
    Ok(page_shell("Contributions timeline", &body, &ctx.tenant))
}

pub fn render_leaderboard(
    records: &[Contribution],
    ctx: &RequestCtx,
) -> Result<String, AuthError> {
    ctx.authorise(Permission::ContributionsRead)?;
    let board = leaderboard(records, 10);
    let mut rows = String::new();
    if board.is_empty() {
        rows.push_str(r#"<tr><td colspan="5"><em>No contributors in window.</em></td></tr>"#);
    } else {
        for (rank, s) in board.iter().enumerate() {
            let score = (s.batches as i64) * 10 + s.tests_added;
            rows.push_str(&format!(
                r#"<tr><td class="num">#{rank}</td><td>{wid}</td><td class="num">{score}</td><td class="num">{batches}</td><td class="num">{tests}</td></tr>"#,
                rank = rank + 1,
                wid = html_escape(&s.worker_id),
                score = score,
                batches = s.batches,
                tests = s.tests_added,
            ));
        }
    }
    let body = format!(
        r#"<h2>Top contributors (composite = 10·batches + tests)</h2>
<table>
  <thead><tr><th class="num">rank</th><th>worker_id</th><th class="num">score</th><th class="num">batches</th><th class="num">tests +</th></tr></thead>
  <tbody>{rows}</tbody>
</table>"#,
    );
    Ok(page_shell("Contributions leaderboard", &body, &ctx.tenant))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn fixture_jsonl() -> String {
        // Realistic sample mirroring the on-disk format.
        let lines = [
            r#"{"ts":"2026-04-26T10:00:00Z","worker_id":"qwen-3-coder-next","batch_id":"b1","test_delta":32,"commit_sha":"abc123ef0011","model":"qwen3","crate":"cave-search","eval_seconds":247,"branch":"qwen/b1","merged_to":"main"}"#,
            r#"{"ts":"2026-04-26T10:30:00Z","worker_id":"qwen-3-coder-next","batch_id":"b2","test_delta":15,"commit_sha":"def456ab0022","model":"qwen3","crate":"cave-net","eval_seconds":190,"branch":"qwen/b2","merged_to":"main"}"#,
            r#"{"ts":"2026-04-26T11:15:00Z","worker_id":"sonnet-4-6","batch_id":"b3","test_delta":47,"commit_sha":"112233445566","model":"sonnet","crate":"cave-etcd","eval_seconds":520,"branch":"sonnet/b3","merged_to":"main"}"#,
            r#"{"ts":"2026-04-26T12:00:00Z","worker_id":"claude-opus-4-7","batch_id":"b4","test_delta":93,"commit_sha":"778899aabbcc","model":"opus","crate":"cave-cli","eval_seconds":600,"branch":"claude/b4","merged_to":"main"}"#,
            r#"{"ts":"2026-04-26T12:30:00Z","worker_id":"manual-burak","batch_id":"b5","test_delta":4,"commit_sha":"ddeeff001122","model":"manual","crate":"cave-portal","eval_seconds":1800,"branch":"main","merged_to":"main"}"#,
            r#"{"ts":"2026-04-25T08:00:00Z","worker_id":"qwen-3-coder-next","batch_id":"b0","test_delta":8,"commit_sha":"000000111122","model":"qwen3","crate":"cave-search","eval_seconds":100,"branch":"qwen/b0","merged_to":"main"}"#,
        ];
        lines.join("\n")
    }

    fn admin_ctx(tenant: &str) -> RequestCtx {
        RequestCtx::developer(tenant, &[Permission::ContributionsRead])
    }

    /// cite: night-pump JSONL — qwen-3-coder-next maps to dedicated WorkerKind
    #[test]
    fn contributions_acme_worker_kind_qwen_mapped() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "acme"
        );
        assert_eq!(
            WorkerKind::from_worker_id("qwen-3-coder-next"),
            WorkerKind::Qwen3CoderNext
        );
    }

    /// cite: night-pump JSONL — sonnet-4-6 maps to Sonnet46
    #[test]
    fn contributions_globex_worker_kind_sonnet_mapped() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "globex"
        );
        assert_eq!(
            WorkerKind::from_worker_id("sonnet-4-6"),
            WorkerKind::Sonnet46
        );
    }

    /// cite: night-pump JSONL — claude-opus-4-7 maps to ClaudeOpus47
    #[test]
    fn contributions_initech_worker_kind_opus_mapped() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "initech"
        );
        assert_eq!(
            WorkerKind::from_worker_id("claude-opus-4-7"),
            WorkerKind::ClaudeOpus47
        );
    }

    /// cite: night-pump JSONL — manual-burak maps to ManualBurak
    #[test]
    fn contributions_dunder_worker_kind_manual_mapped() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "dunder"
        );
        assert_eq!(
            WorkerKind::from_worker_id("manual-burak"),
            WorkerKind::ManualBurak
        );
    }

    /// cite: night-pump JSONL — unknown worker_id buckets as Other
    #[test]
    fn contributions_acme_worker_kind_unknown_is_other() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "acme"
        );
        assert_eq!(WorkerKind::from_worker_id("ghost"), WorkerKind::Other);
        assert_eq!(WorkerKind::from_worker_id(""), WorkerKind::Other);
    }

    /// cite: night-pump JSONL — parses the sample fixture line by line
    #[test]
    fn contributions_acme_parse_fixture_returns_six_records() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "acme"
        );
        let recs = parse_jsonl(&fixture_jsonl()).unwrap();
        assert_eq!(recs.len(), 6);
        assert_eq!(recs[0].worker_id, "qwen-3-coder-next");
        assert_eq!(recs[0].test_delta, 32);
        assert_eq!(recs[0].crate_name, "cave-search");
    }

    /// cite: night-pump JSONL — blank lines are skipped
    #[test]
    fn contributions_globex_parse_skips_blank_lines() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "globex"
        );
        let input = format!("\n\n{}\n\n", fixture_jsonl());
        let recs = parse_jsonl(&input).unwrap();
        assert_eq!(recs.len(), 6);
    }

    /// cite: night-pump JSONL — malformed line surfaces line number in error
    #[test]
    fn contributions_initech_parse_error_includes_line_number() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "initech"
        );
        let bad = "this is not json";
        let err = parse_jsonl(bad).unwrap_err();
        assert!(err.contains("line 1"), "got {err}");
    }

    /// cite: aggregation — by_worker groups all rows of a worker
    #[test]
    fn contributions_acme_aggregate_by_worker_groups_qwen_rows() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "acme"
        );
        let recs = parse_jsonl(&fixture_jsonl()).unwrap();
        let summaries = aggregate_by_worker(&recs);
        let qwen = summaries
            .iter()
            .find(|s| s.worker_id == "qwen-3-coder-next")
            .unwrap();
        assert_eq!(qwen.batches, 3);
        assert_eq!(qwen.tests_added, 32 + 15 + 8);
        // distinct crates: cave-search appears twice but counted once
        assert_eq!(qwen.crates_touched, 2);
    }

    /// cite: aggregation — eval_seconds sum across batches
    #[test]
    fn contributions_globex_aggregate_eval_seconds_summed() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "globex"
        );
        let recs = parse_jsonl(&fixture_jsonl()).unwrap();
        let summaries = aggregate_by_worker(&recs);
        let qwen = summaries
            .iter()
            .find(|s| s.worker_id == "qwen-3-coder-next")
            .unwrap();
        assert_eq!(qwen.eval_seconds_total, 247 + 190 + 100);
    }

    /// cite: aggregation — overview ordering is "most batches first"
    #[test]
    fn contributions_initech_aggregate_sorts_most_batches_first() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "initech"
        );
        let recs = parse_jsonl(&fixture_jsonl()).unwrap();
        let summaries = aggregate_by_worker(&recs);
        // qwen has 3 batches, others 1 each — qwen first
        assert_eq!(summaries[0].worker_id, "qwen-3-coder-next");
    }

    /// cite: timeline — buckets group by UTC hour
    #[test]
    fn contributions_acme_timeline_buckets_by_utc_hour() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "acme"
        );
        let recs = parse_jsonl(&fixture_jsonl()).unwrap();
        let buckets = aggregate_timeline(&recs);
        // distinct hours: 2026-04-25T08, 2026-04-26T10, T11, T12
        assert_eq!(buckets.len(), 4);
    }

    /// cite: timeline — hour 10:00 has 2 batches (qwen b1+b2)
    #[test]
    fn contributions_globex_timeline_hour_10_has_two_batches() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "globex"
        );
        let recs = parse_jsonl(&fixture_jsonl()).unwrap();
        let buckets = aggregate_timeline(&recs);
        let hr10 = buckets
            .iter()
            .find(|b| b.hour.hour() == 10 && b.hour.day() == 26)
            .unwrap();
        assert_eq!(hr10.batches, 2);
        assert_eq!(hr10.tests_added, 32 + 15);
    }

    /// cite: timeline — buckets ordered chronologically (oldest first)
    #[test]
    fn contributions_initech_timeline_chronological_order() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "initech"
        );
        let recs = parse_jsonl(&fixture_jsonl()).unwrap();
        let buckets = aggregate_timeline(&recs);
        for w in buckets.windows(2) {
            assert!(w[0].hour < w[1].hour, "buckets must be ascending");
        }
    }

    /// cite: leaderboard — composite score 10·batches + tests
    #[test]
    fn contributions_acme_leaderboard_composite_score_ranks_opus_top() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "acme"
        );
        let recs = parse_jsonl(&fixture_jsonl()).unwrap();
        let board = leaderboard(&recs, 10);
        // opus: 1*10 + 93 = 103
        // qwen: 3*10 + 55 = 85
        // sonnet: 1*10 + 47 = 57
        assert_eq!(board[0].worker_id, "claude-opus-4-7");
        assert_eq!(board[1].worker_id, "qwen-3-coder-next");
        assert_eq!(board[2].worker_id, "sonnet-4-6");
    }

    /// cite: leaderboard — limit truncates the result set
    #[test]
    fn contributions_globex_leaderboard_limit_truncates() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "globex"
        );
        let recs = parse_jsonl(&fixture_jsonl()).unwrap();
        let board = leaderboard(&recs, 2);
        assert_eq!(board.len(), 2);
    }

    /// cite: filter — `since` excludes earlier rows
    #[test]
    fn contributions_acme_filter_since_excludes_earlier() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "acme"
        );
        let recs = parse_jsonl(&fixture_jsonl()).unwrap();
        let cutoff = Utc.with_ymd_and_hms(2026, 4, 26, 0, 0, 0).unwrap();
        let f = ContributionsFilter::since(cutoff);
        let kept: Vec<_> = recs.iter().filter(|c| f.matches(c)).collect();
        assert_eq!(kept.len(), 5);
    }

    /// cite: filter — None matches everything
    #[test]
    fn contributions_globex_filter_none_keeps_all() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "globex"
        );
        let recs = parse_jsonl(&fixture_jsonl()).unwrap();
        let f = ContributionsFilter::default();
        let kept: Vec<_> = recs.iter().filter(|c| f.matches(c)).collect();
        assert_eq!(kept.len(), recs.len());
    }

    /// cite: detail — recent-first ordering
    #[test]
    fn contributions_acme_worker_detail_most_recent_first() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "acme"
        );
        let recs = parse_jsonl(&fixture_jsonl()).unwrap();
        let det = worker_detail(&recs, "qwen-3-coder-next", 10);
        assert_eq!(det.len(), 3);
        for w in det.windows(2) {
            assert!(w[0].ts >= w[1].ts);
        }
    }

    /// cite: detail — limit caps the slice
    #[test]
    fn contributions_globex_worker_detail_limit_caps_slice() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "globex"
        );
        let recs = parse_jsonl(&fixture_jsonl()).unwrap();
        let det = worker_detail(&recs, "qwen-3-coder-next", 1);
        assert_eq!(det.len(), 1);
    }

    /// cite: detail — unknown worker returns empty
    #[test]
    fn contributions_initech_worker_detail_unknown_is_empty() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "initech"
        );
        let recs = parse_jsonl(&fixture_jsonl()).unwrap();
        let det = worker_detail(&recs, "ghost", 10);
        assert!(det.is_empty());
    }

    /// cite: RBAC — overview rejects without ContributionsRead permission
    #[test]
    fn contributions_acme_overview_requires_contributions_read() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/permission-backend/src/PermissionsService.ts",
            "authorize",
            "acme"
        );
        let ctx = RequestCtx::developer("acme", &[Permission::DashboardRead]);
        let err = render_overview(&[], &ctx).unwrap_err();
        assert_eq!(
            err,
            AuthError::MissingPermission {
                missing: "cluster.contributions.read",
            }
        );
    }

    /// cite: RBAC — overview blocks when WebAuthn missing
    #[test]
    fn contributions_globex_overview_requires_webauthn() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/permission-backend/src/PermissionsService.ts",
            "authorize",
            "globex"
        );
        let mut ctx = admin_ctx("globex");
        ctx.has_webauthn = false;
        let err = render_overview(&[], &ctx).unwrap_err();
        assert_eq!(err, AuthError::WebAuthnRequired);
    }

    /// cite: render — overview lists each worker_id at least once
    #[test]
    fn contributions_acme_overview_includes_each_worker() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "acme"
        );
        let recs = parse_jsonl(&fixture_jsonl()).unwrap();
        let html = render_overview(&recs, &admin_ctx("acme")).unwrap();
        for wid in [
            "qwen-3-coder-next",
            "sonnet-4-6",
            "claude-opus-4-7",
            "manual-burak",
        ] {
            assert!(html.contains(wid), "overview missing {wid}");
        }
    }

    /// cite: render — overview prints tenant_id badge
    #[test]
    fn contributions_globex_overview_renders_tenant_badge() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "globex"
        );
        let html = render_overview(&[], &admin_ctx("globex")).unwrap();
        assert!(html.contains(r#"<span class="badge">globex</span>"#));
    }

    /// cite: render — empty corpus shows the empty-state
    #[test]
    fn contributions_initech_overview_empty_state_when_no_data() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "initech"
        );
        let html = render_overview(&[], &admin_ctx("initech")).unwrap();
        assert!(html.contains("No contributions in window"));
    }

    /// cite: render — worker detail page mentions the worker_id and recent SHA
    #[test]
    fn contributions_acme_detail_renders_worker_and_sha() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "acme"
        );
        let recs = parse_jsonl(&fixture_jsonl()).unwrap();
        let html = render_worker_detail(&recs, "qwen-3-coder-next", &admin_ctx("acme")).unwrap();
        assert!(html.contains("qwen-3-coder-next"));
        assert!(html.contains("def456ab")); // truncated SHA
    }

    /// cite: render — detail empty-state for unknown worker
    #[test]
    fn contributions_globex_detail_empty_state_for_unknown_worker() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "globex"
        );
        let html = render_worker_detail(&[], "ghost", &admin_ctx("globex")).unwrap();
        assert!(html.contains("No batches recorded for this worker"));
    }

    /// cite: render — timeline renders bar visual + hour rows
    #[test]
    fn contributions_acme_timeline_renders_bar_and_hours() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "acme"
        );
        let recs = parse_jsonl(&fixture_jsonl()).unwrap();
        let html = render_timeline(&recs, &admin_ctx("acme")).unwrap();
        assert!(html.contains("Hourly activity"));
        assert!(html.contains("█")); // sparkline char
    }

    /// cite: render — leaderboard ranks #1 first
    #[test]
    fn contributions_acme_leaderboard_renders_rank_1_first() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "acme"
        );
        let recs = parse_jsonl(&fixture_jsonl()).unwrap();
        let html = render_leaderboard(&recs, &admin_ctx("acme")).unwrap();
        let p1 = html.find("#1").unwrap();
        let p_opus = html.find("claude-opus-4-7").unwrap();
        assert!(p1 < p_opus, "rank #1 must be rendered before opus row");
    }

    /// cite: render — tenant_id is now validated at the boundary
    /// (sweep-002 F2-G adoption made TenantId DNS-1123-only). The previous
    /// version of this test exercised the downstream HTML escape as a
    /// defence-in-depth layer; with the canonical newtype the malicious
    /// value cannot construct in the first place. Assert the rejection
    /// directly.
    #[test]
    fn contributions_html_escape_blocks_tenant_injection() {
        let (_cite, _t) = portal_test_ctx!(
            "packages/core-components/src/Page/Page.tsx",
            "Page",
            "acme"
        );
        assert!(TenantId::new("evil<script>").is_err(),
            "TenantId must reject HTML-injection-shaped input at construction");
    }

    /// cite: detail — worker_kind() helper returns the same enum as static fn
    #[test]
    fn contributions_acme_record_worker_kind_consistent() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "acme"
        );
        let recs = parse_jsonl(&fixture_jsonl()).unwrap();
        for c in recs {
            assert_eq!(c.worker_kind(), WorkerKind::from_worker_id(&c.worker_id));
        }
    }
}
