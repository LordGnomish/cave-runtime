// SPDX-License-Identifier: AGPL-3.0-or-later
//! Cross-crate integration test — drive a SARIF document through the
//! cave-vulns import endpoint end-to-end and assert dedupe + persistence.
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738 dojo/api_v2/views.py
//!         (`ImportScanView.create`).
//!
//! Wire shape simulates what cave-scan would emit when running CodeQL
//! / `cargo geiger` / a generic SAST pipeline that supports SARIF.

use axum::{body::Body, http::Request};
use cave_vulns::{State, router};
use std::sync::Arc;
use tower::util::ServiceExt;

const SARIF_PAYLOAD: &str = r#"{
  "version": "2.1.0",
  "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
  "runs": [{
    "tool": { "driver": {
      "name": "CodeQL",
      "version": "2.18.0",
      "rules": [
        {"id":"rust/integer-overflow",
         "properties":{"tags":["security","external/cwe/cwe-190"]},
         "shortDescription":{"text":"Possible integer overflow"}},
        {"id":"rust/sql-injection",
         "properties":{"tags":["security","external/cwe/cwe-89"]},
         "shortDescription":{"text":"SQL injection"}}
      ]
    }},
    "results": [
      {"ruleId":"rust/integer-overflow","level":"warning",
       "message":{"text":"u32 multiplication may wrap"},
       "locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/math.rs"},
                                          "region":{"startLine":42}}}],
       "properties":{"security-severity":"6.5"}},
      {"ruleId":"rust/sql-injection","level":"error",
       "message":{"text":"Untrusted input flows into query"},
       "locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/db.rs"},
                                          "region":{"startLine":100}}}],
       "properties":{"security-severity":"9.1"}},
      {"ruleId":"rust/sql-injection","level":"error",
       "message":{"text":"Untrusted input flows into query"},
       "locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/db.rs"},
                                          "region":{"startLine":100}}}],
       "properties":{"security-severity":"9.1"}}
    ]
  }]
}"#;

#[tokio::test]
async fn sarif_imports_persist_and_dedupe() {
    let app = router(Arc::new(State::default()));

    // POST /api/vulns/import-scan with the SARIF doc.
    let body = serde_json::json!({
        "scan_type": "SARIF",
        "content": SARIF_PAYLOAD,
    })
    .to_string();
    let req = Request::post("/api/vulns/import-scan")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    let bytes = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    // Three results in, two unique by SARIF dedupe fields
    // (title, cwe, line, file_path, description). The two SQL-injection
    // entries are identical → should collapse to one.
    assert_eq!(
        v["imported"], 2,
        "expected dedup to collapse the duplicate SQLi → 2 findings"
    );
    assert_eq!(v["scan_type"], "SARIF");
    assert_eq!(v["dedup_algorithm"], "hash_code");

    // GET /api/vulns/findings — verify persistence.
    let resp2 = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/vulns/findings")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp2.status(), 200);
    let bytes2 = axum::body::to_bytes(resp2.into_body(), 65536)
        .await
        .unwrap();
    let v2: serde_json::Value = serde_json::from_slice(&bytes2).unwrap();
    assert_eq!(v2["count"], 2);

    // The SQLi finding should be promoted to Critical (security-severity 9.1).
    let results = v2["results"].as_array().unwrap();
    let sqli = results
        .iter()
        .find(|f| f["vuln_id_from_tool"].as_str() == Some("rust/sql-injection"))
        .expect("SQLi finding present");
    assert_eq!(sqli["severity"], "Critical");
    assert_eq!(sqli["cwe"], 89);
    assert_eq!(sqli["file_path"], "src/db.rs");
    assert_eq!(sqli["line"], 100);
    assert_eq!(sqli["found_by_scanner"], "SARIF");
    assert_eq!(sqli["service"], "CodeQL"); // driver name lands on service

    // GET /api/vulns/sla — confirm rollup sees the new findings.
    let resp3 = app
        .oneshot(
            Request::builder()
                .uri("/api/vulns/sla")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp3.status(), 200);
    let bytes3 = axum::body::to_bytes(resp3.into_body(), 65536)
        .await
        .unwrap();
    let v3: serde_json::Value = serde_json::from_slice(&bytes3).unwrap();
    assert_eq!(v3["total"], 2);
}

#[tokio::test]
async fn bandit_then_semgrep_imports_keep_distinct_scan_types() {
    let app = router(Arc::new(State::default()));

    let bandit_body = serde_json::json!({
        "scan_type": "Bandit Scan",
        "content": r#"{"results":[{"test_name":"x","test_id":"B1","filename":"a.py","line_number":1,"issue_severity":"HIGH","issue_text":"y"}]}"#,
    }).to_string();
    let resp = app
        .clone()
        .oneshot(
            Request::post("/api/vulns/import-scan")
                .header("content-type", "application/json")
                .body(Body::from(bandit_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let semgrep_body = serde_json::json!({
        "scan_type": "Semgrep JSON Report",
        "content": r#"{"results":[{"check_id":"rule.x","path":"a.py","start":{"line":1},"extra":{"severity":"WARNING","message":"y","metadata":{}}}]}"#,
    }).to_string();
    let resp2 = app
        .clone()
        .oneshot(
            Request::post("/api/vulns/import-scan")
                .header("content-type", "application/json")
                .body(Body::from(semgrep_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp2.status(), 200);

    let resp3 = app
        .oneshot(
            Request::builder()
                .uri("/api/vulns/findings")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(resp3.into_body(), 65536)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        v["count"], 2,
        "Bandit + Semgrep findings persist independently"
    );
    let scanners: Vec<&str> = v["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["found_by_scanner"].as_str().unwrap())
        .collect();
    assert!(scanners.contains(&"Bandit Scan"));
    assert!(scanners.contains(&"Semgrep JSON Report"));
}

#[tokio::test]
async fn empty_sarif_imports_zero_findings() {
    let app = router(Arc::new(State::default()));
    let body = serde_json::json!({
        "scan_type": "SARIF",
        "content": r#"{"version":"2.1.0","runs":[]}"#,
    })
    .to_string();
    let resp = app
        .oneshot(
            Request::post("/api/vulns/import-scan")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["imported"], 0);
}

#[tokio::test]
async fn dedup_override_uses_legacy_algo() {
    let app = router(Arc::new(State::default()));
    let body = serde_json::json!({
        "scan_type": "Bandit Scan",
        "content": r#"{"results":[
            {"test_name":"x","test_id":"B1","filename":"a.py","line_number":1,"issue_severity":"HIGH","issue_text":"same"},
            {"test_name":"x","test_id":"B1","filename":"a.py","line_number":1,"issue_severity":"HIGH","issue_text":"same"}
        ]}"#,
        "dedup": "legacy",
    }).to_string();
    let resp = app
        .oneshot(
            Request::post("/api/vulns/import-scan")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["dedup_algorithm"], "legacy");
    assert_eq!(
        v["imported"], 1,
        "legacy dedupe collapses identical title/cwe/line/file_path/description"
    );
}
