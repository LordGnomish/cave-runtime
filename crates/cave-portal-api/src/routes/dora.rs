//! DORA metrics computed from git history.
//!
//! Four metrics:
//!   - Deployment Frequency : commits whose subject starts with `deploy:` /
//!     `release:` (or matches an annotated tag).
//!   - Lead Time for Changes: mean (deploy_at − last_non_deploy_commit_at) over
//!     consecutive deploy windows. A coarse proxy — assumes work is shipped in
//!     the order it's committed.
//!   - MTTR: pulled from `cave-oncall` if `CAVE_ONCALL_URL` is set; otherwise
//!     reported as `n/a`. We do **not** invent numbers.
//!   - Change Failure Rate: revert + hotfix commit count divided by deploy
//!     count within the same window.
//!
//! Endpoints:
//!   - GET /api/portal/dora/summary?range=7d|30d|90d
//!   - GET /api/portal/dora/timeline?metric=deployment_frequency&range=30d
//!   - GET /admin/dora

use std::collections::BTreeMap;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::get,
    Router,
};
use chrono::{DateTime, Duration as ChronoDuration, FixedOffset, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use super::activity::Range;
use super::attribution::CommitRecord;
use super::state::PortalDataState;

// ── Classification ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommitKind {
    Deploy,
    Revert,
    Hotfix,
    Regular,
}

fn classify_commit_kind(rec: &CommitRecord) -> CommitKind {
    let s = rec.subject.to_lowercase();
    if s.starts_with("deploy:")
        || s.starts_with("release:")
        || s.starts_with("deploy(")
        || s.starts_with("release(")
        || s.starts_with("chore(release)")
        || s.starts_with("chore: release ")
    {
        return CommitKind::Deploy;
    }
    if s.starts_with("revert:") || s.starts_with("revert ") || s.starts_with("revert(") {
        return CommitKind::Revert;
    }
    if s.starts_with("hotfix:") || s.starts_with("hotfix(") || s.starts_with("fix!:") {
        return CommitKind::Hotfix;
    }
    CommitKind::Regular
}

fn parse_iso(s: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::<FixedOffset>::parse_from_rfc3339(s).ok()
}

// ── Metric values ─────────────────────────────────────────────────────────────

/// DORA performance band per the 2023 State of DevOps benchmarks.
fn band(metric: &str, value_seconds_or_per_day: f64) -> &'static str {
    match metric {
        "deployment_frequency" => {
            // value = deploys per day
            let per_day = value_seconds_or_per_day;
            if per_day >= 1.0 {
                "Elite"
            } else if per_day >= 1.0 / 7.0 {
                "High"
            } else if per_day >= 1.0 / 30.0 {
                "Medium"
            } else {
                "Low"
            }
        }
        "lead_time" => {
            // value = mean lead time in seconds
            let h = value_seconds_or_per_day / 3600.0;
            if h <= 24.0 {
                "Elite"
            } else if h <= 24.0 * 7.0 {
                "High"
            } else if h <= 24.0 * 30.0 {
                "Medium"
            } else {
                "Low"
            }
        }
        "mttr" => {
            let h = value_seconds_or_per_day / 3600.0;
            if h <= 1.0 {
                "Elite"
            } else if h <= 24.0 {
                "High"
            } else if h <= 24.0 * 7.0 {
                "Medium"
            } else {
                "Low"
            }
        }
        "change_failure_rate" => {
            // value = ratio 0.0..1.0
            let pct = value_seconds_or_per_day * 100.0;
            if pct <= 5.0 {
                "Elite"
            } else if pct <= 10.0 {
                "High"
            } else if pct <= 15.0 {
                "Medium"
            } else {
                "Low"
            }
        }
        _ => "Unknown",
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MetricValue {
    pub value: Option<f64>,
    pub display: String,
    pub band: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoraSummary {
    pub range_days: i64,
    pub deployment_frequency: MetricValue,
    pub lead_time_for_changes: MetricValue,
    pub mttr: MetricValue,
    pub change_failure_rate: MetricValue,
    pub deploy_count: u64,
    pub revert_count: u64,
    pub hotfix_count: u64,
    pub generated_at: String,
}

/// Mean lead time, in seconds. Approximation: sort all commits by time;
/// for each deploy commit, walk backwards collecting non-deploy commits
/// until the previous deploy; use the earliest of those as the "first
/// commit of the change", and report (deploy − first) per deploy. Mean
/// over deploys.
fn lead_time_seconds(records: &[CommitRecord]) -> Option<f64> {
    let mut commits: Vec<(DateTime<FixedOffset>, CommitKind)> = records
        .iter()
        .filter_map(|r| Some((parse_iso(&r.iso_date)?, classify_commit_kind(r))))
        .collect();
    commits.sort_by_key(|(t, _)| *t);
    let mut samples: Vec<f64> = Vec::new();
    let mut window_start: Option<DateTime<FixedOffset>> = None;
    for (t, kind) in commits {
        match kind {
            CommitKind::Deploy => {
                if let Some(start) = window_start {
                    let dur = (t - start).num_seconds() as f64;
                    if dur > 0.0 {
                        samples.push(dur);
                    }
                }
                window_start = None;
            }
            CommitKind::Regular | CommitKind::Hotfix | CommitKind::Revert => {
                window_start.get_or_insert(t);
            }
        }
    }
    if samples.is_empty() {
        None
    } else {
        Some(samples.iter().sum::<f64>() / samples.len() as f64)
    }
}

fn fmt_duration(secs: f64) -> String {
    let h = secs / 3600.0;
    if h < 1.0 {
        format!("{:.0} min", secs / 60.0)
    } else if h < 24.0 {
        format!("{h:.1} h")
    } else {
        format!("{:.1} d", h / 24.0)
    }
}

// ── MTTR — opportunistic cave-oncall fetch ───────────────────────────────────

#[derive(Deserialize)]
struct OnCallIncident {
    opened_at: Option<String>,
    resolved_at: Option<String>,
}

/// Fetch incidents from `CAVE_ONCALL_URL` (e.g. `http://127.0.0.1:8080`) and
/// compute mean time-to-restore. Returns `None` if the URL is unset, the
/// service is unreachable, or no resolved incidents fall in the window.
async fn mttr_from_oncall(range: Range) -> Option<f64> {
    let base = std::env::var("CAVE_ONCALL_URL").ok()?;
    let url = format!("{}/api/oncall/incidents", base.trim_end_matches('/'));
    // Best-effort; we deliberately do not pull in a heavy HTTP client crate
    // for an optional integration. axum already pulls hyper transitively, but
    // using it here would couple this module to the framework. Spawn a
    // blocking std-net request instead — simple and correct.
    let body = tokio::task::spawn_blocking(move || -> Option<String> {
        let mut req = std::net::TcpStream::connect_timeout(
            &url_to_socket(&url)?,
            std::time::Duration::from_millis(500),
        )
        .ok()?;
        let path = url_path(&url).unwrap_or_else(|| "/".into());
        let host = url_host(&url).unwrap_or_else(|| "localhost".into());
        use std::io::{Read, Write};
        write!(
            req,
            "GET {path} HTTP/1.0\r\nHost: {host}\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
        )
        .ok()?;
        let _ = req.set_read_timeout(Some(std::time::Duration::from_millis(1500)));
        let mut buf = String::new();
        req.read_to_string(&mut buf).ok()?;
        let body = buf.split("\r\n\r\n").nth(1)?.to_owned();
        Some(body)
    })
    .await
    .ok()??;

    let incidents: Vec<OnCallIncident> = serde_json::from_str(&body).ok()?;
    let now = Utc::now();
    let cutoff = now - ChronoDuration::days(range.days());
    let mut samples: Vec<f64> = Vec::new();
    for i in incidents {
        let (Some(o), Some(r)) = (i.opened_at, i.resolved_at) else {
            continue;
        };
        let (Some(o), Some(r)) = (parse_iso(&o), parse_iso(&r)) else {
            continue;
        };
        if r < cutoff.with_timezone(r.offset()) {
            continue;
        }
        let dur = (r - o).num_seconds() as f64;
        if dur > 0.0 {
            samples.push(dur);
        }
    }
    if samples.is_empty() {
        None
    } else {
        Some(samples.iter().sum::<f64>() / samples.len() as f64)
    }
}

fn url_to_socket(url: &str) -> Option<std::net::SocketAddr> {
    let host = url_host(url)?;
    let port: u16 = url_port(url).unwrap_or(80);
    let mut iter = (host.as_str(), port).to_socket_addrs().ok()?;
    iter.next()
}
fn url_host(url: &str) -> Option<String> {
    let after = url.split_once("://")?.1;
    let host_port = after.split('/').next()?;
    Some(host_port.split(':').next()?.to_owned())
}
fn url_port(url: &str) -> Option<u16> {
    let after = url.split_once("://")?.1;
    let host_port = after.split('/').next()?;
    host_port.split(':').nth(1)?.parse().ok()
}
fn url_path(url: &str) -> Option<String> {
    let after = url.split_once("://")?.1;
    let mut parts = after.splitn(2, '/');
    parts.next();
    Some(format!("/{}", parts.next().unwrap_or("")))
}

use std::net::ToSocketAddrs;

// ── Build summary ─────────────────────────────────────────────────────────────

pub async fn build_summary(records: &[CommitRecord], range: Range) -> DoraSummary {
    let now = Utc::now();
    let cutoff = now - ChronoDuration::days(range.days());

    let mut deploys = 0u64;
    let mut reverts = 0u64;
    let mut hotfixes = 0u64;
    let mut in_window: Vec<&CommitRecord> = Vec::new();
    for rec in records {
        let Some(t) = parse_iso(&rec.iso_date) else {
            continue;
        };
        if t < cutoff.with_timezone(t.offset()) {
            continue;
        }
        in_window.push(rec);
        match classify_commit_kind(rec) {
            CommitKind::Deploy => deploys += 1,
            CommitKind::Revert => reverts += 1,
            CommitKind::Hotfix => hotfixes += 1,
            CommitKind::Regular => {}
        }
    }

    let per_day = if range.days() > 0 {
        deploys as f64 / range.days() as f64
    } else {
        0.0
    };
    let deployment_frequency = MetricValue {
        value: Some(per_day),
        display: if deploys == 0 {
            "—".into()
        } else if per_day >= 1.0 {
            format!("{per_day:.1} / day")
        } else if per_day >= 1.0 / 7.0 {
            format!("{:.1} / week", per_day * 7.0)
        } else {
            format!("{:.1} / month", per_day * 30.0)
        },
        band: if deploys == 0 {
            "Low".into()
        } else {
            band("deployment_frequency", per_day).into()
        },
    };

    let owned: Vec<CommitRecord> = in_window.iter().map(|r| (*r).clone()).collect();
    let lead = lead_time_seconds(&owned);
    let lead_time_for_changes = match lead {
        Some(s) => MetricValue {
            value: Some(s),
            display: fmt_duration(s),
            band: band("lead_time", s).into(),
        },
        None => MetricValue {
            value: None,
            display: "n/a".into(),
            band: "Unknown".into(),
        },
    };

    let mttr_secs = mttr_from_oncall(range).await;
    let mttr = match mttr_secs {
        Some(s) => MetricValue {
            value: Some(s),
            display: fmt_duration(s),
            band: band("mttr", s).into(),
        },
        None => MetricValue {
            value: None,
            display: "n/a (cave-oncall not wired)".into(),
            band: "Unknown".into(),
        },
    };

    let cfr = if deploys == 0 {
        None
    } else {
        Some((reverts + hotfixes) as f64 / deploys as f64)
    };
    let change_failure_rate = match cfr {
        Some(r) => MetricValue {
            value: Some(r),
            display: format!("{:.1} %", r * 100.0),
            band: band("change_failure_rate", r).into(),
        },
        None => MetricValue {
            value: None,
            display: "n/a".into(),
            band: "Unknown".into(),
        },
    };

    DoraSummary {
        range_days: range.days(),
        deployment_frequency,
        lead_time_for_changes,
        mttr,
        change_failure_rate,
        deploy_count: deploys,
        revert_count: reverts,
        hotfix_count: hotfixes,
        generated_at: Utc::now().to_rfc3339(),
    }
}

// ── Timeline ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct DoraTimelinePoint {
    pub date: String,
    pub value: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoraTimeline {
    pub metric: String,
    pub range_days: i64,
    pub points: Vec<DoraTimelinePoint>,
    pub generated_at: String,
}

/// Build a daily series for one DORA metric. For `deployment_frequency` and
/// `change_failure_rate` we compute per-day. For `lead_time` we report the
/// mean lead-time of deploys whose timestamp falls on each day. `mttr` is
/// not supported via timeline (would require day-bucketed incident data).
pub fn build_timeline(
    records: &[CommitRecord],
    range: Range,
    metric: &str,
) -> Result<DoraTimeline, String> {
    let now = Utc::now();
    let cutoff = now - ChronoDuration::days(range.days());

    // Pre-seed daily buckets so the timeline is dense (zeros included).
    let mut deploys_by_day: BTreeMap<NaiveDate, u64> = BTreeMap::new();
    let mut reverts_by_day: BTreeMap<NaiveDate, u64> = BTreeMap::new();
    let mut hotfix_by_day: BTreeMap<NaiveDate, u64> = BTreeMap::new();
    {
        let mut d = cutoff.date_naive();
        let end = now.date_naive();
        while d <= end {
            deploys_by_day.insert(d, 0);
            reverts_by_day.insert(d, 0);
            hotfix_by_day.insert(d, 0);
            d = d.succ_opt().unwrap_or(d);
        }
    }

    for rec in records {
        let Some(t) = parse_iso(&rec.iso_date) else {
            continue;
        };
        if t < cutoff.with_timezone(t.offset()) {
            continue;
        }
        let day = t.date_naive();
        match classify_commit_kind(rec) {
            CommitKind::Deploy => *deploys_by_day.entry(day).or_insert(0) += 1,
            CommitKind::Revert => *reverts_by_day.entry(day).or_insert(0) += 1,
            CommitKind::Hotfix => *hotfix_by_day.entry(day).or_insert(0) += 1,
            CommitKind::Regular => {}
        }
    }

    let points: Vec<DoraTimelinePoint> = match metric {
        "deployment_frequency" => deploys_by_day
            .iter()
            .map(|(d, n)| DoraTimelinePoint {
                date: d.to_string(),
                value: *n as f64,
            })
            .collect(),
        "change_failure_rate" => deploys_by_day
            .iter()
            .map(|(d, deploys)| {
                let r = *reverts_by_day.get(d).unwrap_or(&0);
                let h = *hotfix_by_day.get(d).unwrap_or(&0);
                let value = if *deploys == 0 {
                    0.0
                } else {
                    (r + h) as f64 / *deploys as f64
                };
                DoraTimelinePoint {
                    date: d.to_string(),
                    value,
                }
            })
            .collect(),
        "lead_time" => {
            // Mean lead time per day — average over the deploys that day.
            let mut commits: Vec<(DateTime<FixedOffset>, CommitKind)> = records
                .iter()
                .filter_map(|r| Some((parse_iso(&r.iso_date)?, classify_commit_kind(r))))
                .collect();
            commits.sort_by_key(|(t, _)| *t);
            let mut window_start: Option<DateTime<FixedOffset>> = None;
            let mut by_day: BTreeMap<NaiveDate, (f64, u64)> = BTreeMap::new();
            for (t, kind) in commits {
                if t < cutoff.with_timezone(t.offset()) {
                    if let CommitKind::Deploy = kind {
                        window_start = None;
                    } else {
                        window_start.get_or_insert(t);
                    }
                    continue;
                }
                match kind {
                    CommitKind::Deploy => {
                        if let Some(start) = window_start {
                            let dur = (t - start).num_seconds() as f64;
                            if dur > 0.0 {
                                let entry = by_day.entry(t.date_naive()).or_insert((0.0, 0));
                                entry.0 += dur;
                                entry.1 += 1;
                            }
                        }
                        window_start = None;
                    }
                    _ => {
                        window_start.get_or_insert(t);
                    }
                }
            }
            // Pre-seed every day with 0 so the chart line is dense.
            let mut d = cutoff.date_naive();
            let end = now.date_naive();
            let mut points: Vec<DoraTimelinePoint> = Vec::new();
            while d <= end {
                let v = by_day
                    .get(&d)
                    .map(|(sum, n)| if *n > 0 { sum / *n as f64 } else { 0.0 })
                    .unwrap_or(0.0);
                points.push(DoraTimelinePoint {
                    date: d.to_string(),
                    value: v,
                });
                d = d.succ_opt().unwrap_or(d);
            }
            points
        }
        other => return Err(format!("unsupported metric: {other}")),
    };

    Ok(DoraTimeline {
        metric: metric.to_owned(),
        range_days: range.days(),
        points,
        generated_at: Utc::now().to_rfc3339(),
    })
}

// ── Handlers ──────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct DoraSummaryQuery {
    #[serde(default = "default_range")]
    pub range: String,
}

#[derive(Debug, Deserialize)]
pub struct DoraTimelineQuery {
    #[serde(default = "default_range")]
    pub range: String,
    #[serde(default = "default_metric")]
    pub metric: String,
}

fn default_range() -> String {
    "30d".into()
}
fn default_metric() -> String {
    "deployment_frequency".into()
}

async fn summary_handler(
    State(state): State<PortalDataState>,
    Query(q): Query<DoraSummaryQuery>,
) -> Result<Json<DoraSummary>, (StatusCode, Json<serde_json::Value>)> {
    let range = Range::parse(&q.range).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "range must be e.g. 7d, 30d, 90d" })),
        )
    })?;
    let recs = state.records().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
    })?;
    Ok(Json(build_summary(&recs, range).await))
}

async fn timeline_handler(
    State(state): State<PortalDataState>,
    Query(q): Query<DoraTimelineQuery>,
) -> Result<Json<DoraTimeline>, (StatusCode, Json<serde_json::Value>)> {
    let range = Range::parse(&q.range).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "range must be e.g. 7d, 30d, 90d" })),
        )
    })?;
    let recs = state.records().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
    })?;
    build_timeline(&recs, range, &q.metric)
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e })),
            )
        })
}

async fn dashboard_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "text/html; charset=utf-8")],
        include_str!("dora_dashboard.html"),
    )
}

pub fn router(state: PortalDataState) -> Router {
    Router::new()
        .route("/api/portal/dora/summary", get(summary_handler))
        .route("/api/portal/dora/timeline", get(timeline_handler))
        .route("/admin/dora", get(dashboard_handler))
        .with_state(state)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::attribution::{CommitRecord, FileChange};
    use axum::body::to_bytes;
    use axum::http::Request;
    use std::process::Command;
    use tower::ServiceExt;

    fn mk(subj: &str, days_ago: i64) -> CommitRecord {
        let now = Utc::now();
        let d = now - ChronoDuration::days(days_ago);
        CommitRecord {
            hash: format!("h{days_ago}"),
            subject: subj.into(),
            author_name: "x".into(),
            body: "".into(),
            iso_date: d.to_rfc3339(),
            files: vec![FileChange {
                path: "crates/x/y.rs".into(),
                added: 1,
                removed: 0,
            }],
        }
    }

    #[test]
    fn classify_kinds_correctly() {
        assert_eq!(classify_commit_kind(&mk("deploy: prod v3", 0)), CommitKind::Deploy);
        assert_eq!(classify_commit_kind(&mk("release: 1.4.0", 0)), CommitKind::Deploy);
        assert_eq!(classify_commit_kind(&mk("deploy(api): bump", 0)), CommitKind::Deploy);
        assert_eq!(classify_commit_kind(&mk("revert: bad change", 0)), CommitKind::Revert);
        assert_eq!(classify_commit_kind(&mk("hotfix: live alarm", 0)), CommitKind::Hotfix);
        assert_eq!(classify_commit_kind(&mk("feat(auth): rbac", 0)), CommitKind::Regular);
    }

    #[tokio::test]
    async fn summary_counts_and_bands() {
        let recs = vec![
            mk("feat: a", 25),
            mk("feat: b", 20),
            mk("deploy: prod v1", 18),
            mk("feat: c", 15),
            mk("hotfix: regression", 14),
            mk("deploy: prod v2", 10),
            mk("revert: deploy v2", 8),
            mk("deploy: prod v3", 3),
            mk("feat: d", 100), // outside 30d window
        ];
        let s = build_summary(&recs, Range::Days(30)).await;
        assert_eq!(s.deploy_count, 3);
        assert_eq!(s.revert_count, 1);
        assert_eq!(s.hotfix_count, 1);
        assert!(s.deployment_frequency.value.unwrap() > 0.0);
        // CFR = (1 + 1) / 3 ≈ 0.667
        let cfr = s.change_failure_rate.value.unwrap();
        assert!((cfr - 0.6667).abs() < 0.01);
        // 3 deploys across 30d = 0.1/day → Medium tier (1/30 .. 1/7)
        assert_eq!(s.deployment_frequency.band, "Medium");
        assert_eq!(s.change_failure_rate.band, "Low"); // 67% > 15%
    }

    #[tokio::test]
    async fn summary_with_no_deploys_has_no_cfr() {
        let recs = vec![mk("feat: a", 5), mk("feat: b", 3)];
        let s = build_summary(&recs, Range::Days(30)).await;
        assert_eq!(s.deploy_count, 0);
        assert!(s.change_failure_rate.value.is_none());
        assert_eq!(s.change_failure_rate.display, "n/a");
    }

    #[test]
    fn lead_time_uses_first_change_to_deploy_window() {
        let recs = vec![
            mk("feat: a", 10),
            mk("feat: b", 9),
            mk("deploy: v1", 8), // lead = 2 days
            mk("feat: c", 6),
            mk("deploy: v2", 4), // lead = 2 days
        ];
        let lt = lead_time_seconds(&recs).unwrap();
        // Two windows of ~2 days each; mean ~ 2 days = 172800s
        assert!(lt > 100_000.0 && lt < 300_000.0, "got {lt}");
    }

    #[test]
    fn timeline_deployment_frequency_returns_dense_series() {
        let recs = vec![
            mk("deploy: v1", 5),
            mk("deploy: v2", 5),
            mk("deploy: v3", 1),
        ];
        let t = build_timeline(&recs, Range::Days(7), "deployment_frequency").unwrap();
        assert_eq!(t.metric, "deployment_frequency");
        // 7d window plus today → 8 days minimum
        assert!(t.points.len() >= 7);
        let total: f64 = t.points.iter().map(|p| p.value).sum();
        assert_eq!(total as u64, 3);
    }

    #[test]
    fn timeline_rejects_unknown_metric() {
        let r = build_timeline(&[], Range::Days(7), "ascended_consciousness");
        assert!(r.is_err());
    }

    #[test]
    fn band_thresholds_match_dora_2023() {
        assert_eq!(band("deployment_frequency", 2.0), "Elite");
        assert_eq!(band("deployment_frequency", 0.5), "High");
        assert_eq!(band("deployment_frequency", 0.05), "Medium");
        assert_eq!(band("deployment_frequency", 0.001), "Low");
        assert_eq!(band("change_failure_rate", 0.03), "Elite");
        assert_eq!(band("change_failure_rate", 0.20), "Low");
    }

    fn make_fixture(dir: &std::path::Path) {
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
        for (i, msg) in ["feat: a", "deploy: v1", "feat: b", "hotfix: oops"].iter().enumerate() {
            std::fs::write(dir.join(format!("f{i}.txt")), "x\n").unwrap();
            Command::new("git")
                .args(["add", "-A"])
                .current_dir(dir)
                .output()
                .unwrap();
            Command::new("git")
                .args(["commit", "-q", "-m", msg])
                .current_dir(dir)
                .output()
                .unwrap();
        }
    }

    #[tokio::test]
    async fn http_summary_serves() {
        let tmp = tempfile::tempdir().unwrap();
        make_fixture(tmp.path());
        let app = router(PortalDataState::new(tmp.path()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/portal/dora/summary?range=30d")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["deploy_count"], 1);
        assert_eq!(v["hotfix_count"], 1);
        assert!(v["mttr"]["display"].as_str().unwrap().starts_with("n/a"));
    }
}
