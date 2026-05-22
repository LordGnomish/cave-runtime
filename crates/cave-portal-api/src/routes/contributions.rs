// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `GET /api/v1/contributions?since=<ISO>` — worker batch contributions JSON API.
//!
//! Source of truth: `tools/night-pump/contributions.jsonl`. Each line is one
//! batch outcome with worker_id + test_delta + commit_sha + crate + branch.
//!
//! Override the path via `CAVE_CONTRIBUTIONS_JSONL` env var (used by tests
//! and the dev server). Production points at the night-pump output file.

use std::collections::BTreeMap;
use std::path::PathBuf;

use axum::{
    Router,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::get,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContributionRow {
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkerAggregate {
    pub worker_id: String,
    pub batches: u64,
    pub tests_added: i64,
    pub eval_seconds_total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContributionsResponse {
    pub since: Option<DateTime<Utc>>,
    pub total_rows: usize,
    pub by_worker: Vec<WorkerAggregate>,
    pub rows: Vec<ContributionRow>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ContributionsQuery {
    pub since: Option<DateTime<Utc>>,
}

// ── Parsing + aggregation ─────────────────────────────────────────────────────

pub fn parse_jsonl(input: &str) -> Result<Vec<ContributionRow>, String> {
    let mut out = Vec::new();
    for (lineno, raw) in input.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let parsed: ContributionRow =
            serde_json::from_str(line).map_err(|e| format!("line {}: {}", lineno + 1, e))?;
        out.push(parsed);
    }
    Ok(out)
}

pub fn jsonl_path() -> PathBuf {
    std::env::var("CAVE_CONTRIBUTIONS_JSONL")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("tools/night-pump/contributions.jsonl"))
}

pub fn aggregate(rows: &[ContributionRow]) -> Vec<WorkerAggregate> {
    let mut by_id: BTreeMap<String, WorkerAggregate> = BTreeMap::new();
    for r in rows {
        let agg = by_id
            .entry(r.worker_id.clone())
            .or_insert_with(|| WorkerAggregate {
                worker_id: r.worker_id.clone(),
                ..Default::default()
            });
        agg.batches += 1;
        agg.tests_added += r.test_delta;
        agg.eval_seconds_total += r.eval_seconds;
    }
    let mut out: Vec<WorkerAggregate> = by_id.into_values().collect();
    out.sort_by(|a, b| {
        b.batches
            .cmp(&a.batches)
            .then_with(|| a.worker_id.cmp(&b.worker_id))
    });
    out
}

pub fn build_response(
    rows: Vec<ContributionRow>,
    since: Option<DateTime<Utc>>,
) -> ContributionsResponse {
    let filtered: Vec<ContributionRow> = rows
        .into_iter()
        .filter(|r| match since {
            Some(s) => r.ts >= s,
            None => true,
        })
        .collect();
    let by_worker = aggregate(&filtered);
    ContributionsResponse {
        since,
        total_rows: filtered.len(),
        by_worker,
        rows: filtered,
    }
}

// ── HTTP handler ──────────────────────────────────────────────────────────────

pub async fn handler(Query(q): Query<ContributionsQuery>) -> impl IntoResponse {
    let path = jsonl_path();
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => {
            return (StatusCode::OK, Json(build_response(vec![], q.since))).into_response();
        }
    };
    let rows = match parse_jsonl(&raw) {
        Ok(r) => r,
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
        }
    };
    (StatusCode::OK, Json(build_response(rows, q.since))).into_response()
}

pub fn router() -> Router {
    Router::new().route("/api/v1/contributions", get(handler))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use chrono::TimeZone;
    use std::sync::Mutex;
    use tempfile::NamedTempFile;
    use tower::util::ServiceExt;

    /// Serialize tests that mutate the CAVE_CONTRIBUTIONS_JSONL env var.
    /// Avoids races where parallel runs see each other's path setting.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn fixture_jsonl() -> String {
        let lines = [
            r#"{"ts":"2026-04-26T10:00:00Z","worker_id":"qwen-3-coder-next","batch_id":"b1","test_delta":32,"commit_sha":"abc","model":"qwen3","crate":"cave-search","eval_seconds":247,"branch":"qwen/b1","merged_to":"main"}"#,
            r#"{"ts":"2026-04-26T11:00:00Z","worker_id":"sonnet-4-6","batch_id":"b2","test_delta":47,"commit_sha":"def","model":"sonnet","crate":"cave-etcd","eval_seconds":520,"branch":"sonnet/b2","merged_to":"main"}"#,
            r#"{"ts":"2026-04-26T12:00:00Z","worker_id":"qwen-3-coder-next","batch_id":"b3","test_delta":15,"commit_sha":"ghi","model":"qwen3","crate":"cave-net","eval_seconds":190,"branch":"qwen/b3","merged_to":"main"}"#,
        ];
        lines.join("\n")
    }

    fn write_fixture() -> NamedTempFile {
        let f = NamedTempFile::new().unwrap();
        std::fs::write(f.path(), fixture_jsonl()).unwrap();
        f
    }

    /// cite: night-pump JSONL — parser handles three sample rows
    #[test]
    fn api_contributions_acme_parse_three_rows() {
        let _tenant_id = "acme";
        let rows = parse_jsonl(&fixture_jsonl()).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].worker_id, "qwen-3-coder-next");
    }

    /// cite: night-pump JSONL — empty input parses to empty Vec
    #[test]
    fn api_contributions_globex_parse_empty_input() {
        let _tenant_id = "globex";
        assert_eq!(parse_jsonl("").unwrap().len(), 0);
        assert_eq!(parse_jsonl("\n\n").unwrap().len(), 0);
    }

    /// cite: night-pump JSONL — malformed line returns error with line number
    #[test]
    fn api_contributions_initech_parse_malformed_includes_line() {
        let _tenant_id = "initech";
        let err = parse_jsonl("{not valid").unwrap_err();
        assert!(err.contains("line 1"));
    }

    /// cite: aggregation — qwen has 2 batches, sonnet 1
    #[test]
    fn api_contributions_acme_aggregate_qwen_two_sonnet_one() {
        let _tenant_id = "acme";
        let rows = parse_jsonl(&fixture_jsonl()).unwrap();
        let agg = aggregate(&rows);
        assert_eq!(agg[0].worker_id, "qwen-3-coder-next");
        assert_eq!(agg[0].batches, 2);
        assert_eq!(agg[0].tests_added, 47);
        assert_eq!(agg[1].worker_id, "sonnet-4-6");
        assert_eq!(agg[1].batches, 1);
    }

    /// cite: aggregation — eval_seconds summed per worker
    #[test]
    fn api_contributions_globex_aggregate_eval_seconds() {
        let _tenant_id = "globex";
        let rows = parse_jsonl(&fixture_jsonl()).unwrap();
        let agg = aggregate(&rows);
        let qwen = &agg[0];
        assert_eq!(qwen.eval_seconds_total, 247 + 190);
    }

    /// cite: response — total_rows reflects pre-filter row count
    #[test]
    fn api_contributions_acme_response_total_rows_matches_input() {
        let _tenant_id = "acme";
        let rows = parse_jsonl(&fixture_jsonl()).unwrap();
        let resp = build_response(rows, None);
        assert_eq!(resp.total_rows, 3);
    }

    /// cite: response — `since` filter excludes earlier rows
    #[test]
    fn api_contributions_globex_since_filter_excludes_earlier() {
        let _tenant_id = "globex";
        let rows = parse_jsonl(&fixture_jsonl()).unwrap();
        let cutoff = Utc.with_ymd_and_hms(2026, 4, 26, 11, 30, 0).unwrap();
        let resp = build_response(rows, Some(cutoff));
        assert_eq!(resp.total_rows, 1);
        assert_eq!(resp.rows[0].worker_id, "qwen-3-coder-next");
        assert_eq!(resp.rows[0].batch_id, "b3");
    }

    /// cite: response — `since=None` keeps all rows
    #[test]
    fn api_contributions_initech_since_none_keeps_all() {
        let _tenant_id = "initech";
        let rows = parse_jsonl(&fixture_jsonl()).unwrap();
        let resp = build_response(rows.clone(), None);
        assert_eq!(resp.total_rows, rows.len());
    }

    /// cite: env override — CAVE_CONTRIBUTIONS_JSONL takes precedence
    #[test]
    fn api_contributions_env_override_changes_path() {
        let _tenant_id = "acme";
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let key = "CAVE_CONTRIBUTIONS_JSONL";
        let prev = std::env::var(key).ok();
        // SAFETY: tests in this binary serialize via tokio runtimes; we
        // restore the previous value below.
        unsafe {
            std::env::set_var(key, "/tmp/some-contrib-fixture.jsonl");
        }
        let p = jsonl_path();
        assert_eq!(
            p,
            std::path::PathBuf::from("/tmp/some-contrib-fixture.jsonl")
        );
        unsafe {
            match prev {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }

    /// cite: env override — default path used when env unset
    #[test]
    fn api_contributions_default_path_when_env_unset() {
        let _tenant_id = "globex";
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let key = "CAVE_CONTRIBUTIONS_JSONL";
        let prev = std::env::var(key).ok();
        unsafe {
            std::env::remove_var(key);
        }
        let p = jsonl_path();
        assert!(p.to_string_lossy().contains("contributions.jsonl"));
        unsafe {
            if let Some(v) = prev {
                std::env::set_var(key, v);
            }
        }
    }

    /// cite: HTTP — handler returns 200 OK and JSON body matching ContributionsResponse
    #[tokio::test(flavor = "current_thread")]
    async fn api_contributions_acme_http_200_with_json_body() {
        let _tenant_id = "acme";
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let key = "CAVE_CONTRIBUTIONS_JSONL";
        let prev = std::env::var(key).ok();
        let f = write_fixture();
        unsafe {
            std::env::set_var(key, f.path());
        }
        let app = router();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/contributions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        let parsed: ContributionsResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed.total_rows, 3);
        assert_eq!(parsed.by_worker[0].worker_id, "qwen-3-coder-next");
        unsafe {
            match prev {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }

    /// cite: HTTP — `since` query parameter filters response
    #[tokio::test(flavor = "current_thread")]
    async fn api_contributions_globex_http_since_filter_applied() {
        let _tenant_id = "globex";
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let key = "CAVE_CONTRIBUTIONS_JSONL";
        let prev = std::env::var(key).ok();
        let f = write_fixture();
        unsafe {
            std::env::set_var(key, f.path());
        }
        let app = router();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/contributions?since=2026-04-26T11:30:00Z")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        let parsed: ContributionsResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed.total_rows, 1);
        unsafe {
            match prev {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }

    /// cite: HTTP — missing fixture file returns 200 OK with empty body
    #[tokio::test(flavor = "current_thread")]
    async fn api_contributions_initech_http_missing_file_returns_empty_ok() {
        let _tenant_id = "initech";
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let key = "CAVE_CONTRIBUTIONS_JSONL";
        let prev = std::env::var(key).ok();
        unsafe {
            std::env::set_var(key, "/tmp/this-path-does-not-exist-xyz.jsonl");
        }
        let app = router();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/contributions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        let parsed: ContributionsResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed.total_rows, 0);
        unsafe {
            match prev {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }
}
