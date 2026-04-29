//! Development-activity heatmap.
//!
//! Two views, both fed by the shared commit cache:
//!   - 24×7 hour-of-day × weekday grid (with per-model breakdown)
//!   - Daily GitHub-style contribution calendar
//!
//! Endpoint: GET /api/portal/activity/heatmap?range=7d|30d|90d
//! UI:       GET /admin/activity

use std::collections::BTreeMap;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::get,
    Router,
};
use chrono::{DateTime, Datelike, Duration as ChronoDuration, FixedOffset, Timelike, Utc, Weekday};
use serde::{Deserialize, Serialize};

use super::attribution::classify_model;
use super::state::PortalDataState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    Days(i64),
}

impl Range {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_lowercase();
        let (n_str, _) = s.strip_suffix('d').map(|n| (n, "d"))?;
        let n: i64 = n_str.parse().ok()?;
        if n <= 0 || n > 3650 {
            return None;
        }
        Some(Self::Days(n))
    }

    pub fn days(&self) -> i64 {
        match self {
            Self::Days(n) => *n,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CellStats {
    pub commits: u64,
    pub by_model: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DayBucket {
    pub date: String,
    pub commits: u64,
    pub by_model: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HeatmapResponse {
    pub range_days: i64,
    /// 7×24 grid: `grid[weekday][hour]` where weekday 0 = Monday.
    pub grid: Vec<Vec<CellStats>>,
    pub calendar: Vec<DayBucket>,
    pub total_commits: u64,
    pub generated_at: String,
}

/// Build a 7×24 grid + day-by-day calendar of commit activity, restricted to
/// commits whose timestamp is within the last `range.days()` days.
pub fn build_heatmap(records: &[super::attribution::CommitRecord], range: Range) -> HeatmapResponse {
    let now = Utc::now();
    let cutoff = now - ChronoDuration::days(range.days());

    let mut grid: Vec<Vec<CellStats>> = (0..7)
        .map(|_| (0..24).map(|_| CellStats::default()).collect())
        .collect();

    let mut calendar: BTreeMap<String, DayBucket> = BTreeMap::new();
    // Pre-seed every day in the range so the calendar shows zero cells too —
    // a sparse weekend looks the same as "no commits" and should still render.
    {
        let mut d = cutoff.date_naive();
        let end = now.date_naive();
        while d <= end {
            let key = d.format("%Y-%m-%d").to_string();
            calendar.insert(
                key.clone(),
                DayBucket {
                    date: key,
                    commits: 0,
                    by_model: BTreeMap::new(),
                },
            );
            d = d.succ_opt().unwrap_or(d);
        }
    }

    let mut total: u64 = 0;
    for rec in records {
        let Ok(dt) = DateTime::<FixedOffset>::parse_from_rfc3339(&rec.iso_date) else {
            continue;
        };
        if dt < cutoff.with_timezone(dt.offset()) {
            continue;
        }
        let weekday = match dt.weekday() {
            Weekday::Mon => 0,
            Weekday::Tue => 1,
            Weekday::Wed => 2,
            Weekday::Thu => 3,
            Weekday::Fri => 4,
            Weekday::Sat => 5,
            Weekday::Sun => 6,
        };
        let hour = dt.hour() as usize;
        let model = classify_model(rec).to_owned();
        let cell = &mut grid[weekday][hour];
        cell.commits += 1;
        *cell.by_model.entry(model.clone()).or_insert(0) += 1;

        let key = dt.format("%Y-%m-%d").to_string();
        let bucket = calendar.entry(key.clone()).or_insert_with(|| DayBucket {
            date: key,
            commits: 0,
            by_model: BTreeMap::new(),
        });
        bucket.commits += 1;
        *bucket.by_model.entry(model).or_insert(0) += 1;
        total += 1;
    }

    HeatmapResponse {
        range_days: range.days(),
        grid,
        calendar: calendar.into_values().collect(),
        total_commits: total,
        generated_at: Utc::now().to_rfc3339(),
    }
}

// ── Handler ───────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct HeatmapQuery {
    #[serde(default = "default_range")]
    pub range: String,
}

fn default_range() -> String {
    "30d".into()
}

async fn heatmap_handler(
    State(state): State<PortalDataState>,
    Query(q): Query<HeatmapQuery>,
) -> Result<Json<HeatmapResponse>, (StatusCode, Json<serde_json::Value>)> {
    let range = Range::parse(&q.range).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "range must be e.g. 7d, 30d, 90d" })),
        )
    })?;
    let recs = state.records().map_err(|e| {
        tracing::error!(error = %e, "activity collect failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
    })?;
    Ok(Json(build_heatmap(&recs, range)))
}

async fn dashboard_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "text/html; charset=utf-8")],
        include_str!("activity_dashboard.html"),
    )
}

pub fn router(state: PortalDataState) -> Router {
    Router::new()
        .route("/api/portal/activity/heatmap", get(heatmap_handler))
        .route("/admin/activity", get(dashboard_handler))
        .with_state(state)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::attribution::{collect_all_commits, CommitRecord, FileChange};
    use axum::body::to_bytes;
    use axum::http::Request;
    use std::path::Path;
    use std::process::Command;
    use tower::ServiceExt;

    #[test]
    fn range_parses_day_strings() {
        assert_eq!(Range::parse("7d"), Some(Range::Days(7)));
        assert_eq!(Range::parse("30D"), Some(Range::Days(30)));
        assert_eq!(Range::parse("90d"), Some(Range::Days(90)));
        assert_eq!(Range::parse("0d"), None);
        assert_eq!(Range::parse("foo"), None);
        assert_eq!(Range::parse("30"), None);
    }

    fn synth(hour: u32, weekday_offset: i64, model_marker: &str) -> CommitRecord {
        // anchor: a known Monday 2026-04-27, then shift by weekday_offset days
        let base = chrono::NaiveDate::from_ymd_opt(2026, 4, 27).unwrap();
        let date = base
            .checked_add_signed(chrono::Duration::days(weekday_offset))
            .unwrap();
        let iso = format!("{date}T{:02}:30:00+00:00", hour);
        CommitRecord {
            hash: "h".into(),
            subject: format!("{model_marker} commit"),
            author_name: "Test".into(),
            body: String::new(),
            iso_date: iso,
            files: vec![FileChange {
                path: "crates/x/lib.rs".into(),
                added: 1,
                removed: 0,
            }],
        }
    }

    #[test]
    fn grid_lands_in_correct_hour_and_weekday_buckets() {
        // Recent enough that 30d range will cover them — we anchor relative
        // to "today" via two adjacent days, no further than ~7 in the past.
        let today = Utc::now().date_naive();
        let monday = today
            - chrono::Duration::days(today.weekday().num_days_from_monday() as i64);
        let mut recs = vec![];
        // qwen at 03:30 Monday
        let r1 = format!("{monday}T03:30:00+00:00");
        recs.push(CommitRecord {
            hash: "h1".into(),
            subject: "[qwen-amele] x".into(),
            author_name: "x".into(),
            body: "".into(),
            iso_date: r1,
            files: vec![FileChange {
                path: "crates/x/y.rs".into(),
                added: 1,
                removed: 0,
            }],
        });
        // opus at 10:30 Tuesday
        let tuesday = monday + chrono::Duration::days(1);
        let r2 = format!("{tuesday}T10:30:00+00:00");
        recs.push(CommitRecord {
            hash: "h2".into(),
            subject: "feat".into(),
            author_name: "x".into(),
            body: "Co-Authored-By: Claude Opus 4.7".into(),
            iso_date: r2,
            files: vec![FileChange {
                path: "crates/x/y.rs".into(),
                added: 1,
                removed: 0,
            }],
        });
        let h = build_heatmap(&recs, Range::Days(30));
        assert_eq!(h.grid[0][3].commits, 1, "qwen Mon 03:00 cell");
        assert_eq!(h.grid[0][3].by_model.get("qwen").copied(), Some(1));
        assert_eq!(h.grid[1][10].commits, 1, "opus Tue 10:00 cell");
        assert_eq!(h.grid[1][10].by_model.get("opus").copied(), Some(1));
        assert_eq!(h.total_commits, 2);
        assert!(!h.calendar.is_empty(), "calendar populated");
    }

    #[test]
    fn out_of_range_commits_excluded() {
        // Commits from 2020 will be excluded by 30d range
        let r = synth(12, 0, "[qwen-amele]");
        let old = CommitRecord {
            hash: "old".into(),
            subject: r.subject,
            author_name: r.author_name,
            body: r.body,
            iso_date: "2020-01-01T12:30:00+00:00".into(),
            files: r.files,
        };
        let h = build_heatmap(&[old], Range::Days(30));
        assert_eq!(h.total_commits, 0);
    }

    fn make_fixture(dir: &Path) {
        Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .current_dir(dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "x@y"])
            .current_dir(dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "X"])
            .current_dir(dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "commit.gpgsign", "false"])
            .current_dir(dir)
            .output()
            .unwrap();
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-q", "-m", "[qwen-amele] hello"])
            .current_dir(dir)
            .output()
            .unwrap();
    }

    #[tokio::test]
    async fn http_heatmap_returns_grid() {
        let tmp = tempfile::tempdir().unwrap();
        make_fixture(tmp.path());
        // Ensure the cache loads — collect once to verify the fixture is picked up
        let recs = collect_all_commits(tmp.path()).unwrap();
        assert_eq!(recs.len(), 1);

        let app = router(PortalDataState::new(tmp.path()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/portal/activity/heatmap?range=30d")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), 256 * 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["range_days"], 30);
        assert_eq!(v["total_commits"], 1);
        assert_eq!(v["grid"].as_array().unwrap().len(), 7);
        assert_eq!(v["grid"][0].as_array().unwrap().len(), 24);
    }

    #[tokio::test]
    async fn http_heatmap_rejects_bad_range() {
        let tmp = tempfile::tempdir().unwrap();
        make_fixture(tmp.path());
        let app = router(PortalDataState::new(tmp.path()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/portal/activity/heatmap?range=banana")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
