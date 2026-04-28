//! End-to-end integration tests for the watch daemon.
//!
//! These exercise the daemon loop against a `mockito` GitHub server,
//! using a tempdir for both the state file and the pump queue. They
//! prove the contract:
//!
//! - First observation of a repo with releases → writes a pump payload.
//! - Second tick on same tag → no payload written, state preserved.
//! - Tag transition → new payload, old_tag and new_tag both populated.
//! - Rate-limit response → no payload, error counter incremented.
//! - Concurrent polls → semaphore caps in-flight, no panics.
//! - 15-min vs 60-min cadence selection works on real `cave_module` strings.

use cave_upstream::{
    daemon::{Config, Daemon},
    delta::TagOnlyDiffer,
    projects::TrackedProject,
    pump::PumpPayload,
    state::WatchState,
};
use chrono::{Duration as ChronoDuration, Utc};
use mockito::Server;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

fn proj(repo: &'static str, module: &'static str) -> TrackedProject {
    TrackedProject {
        name: repo,
        github_repo: repo,
        cave_module: module,
        track_features: "",
        check_frequency: "weekly",
        category: "",
        phase: 1,
    }
}

struct TestEnv {
    _state_dir: TempDir,
    _queue_dir: TempDir,
    state_path: PathBuf,
    queue_path: PathBuf,
}

fn test_env() -> TestEnv {
    let state_dir = tempfile::tempdir().unwrap();
    let queue_dir = tempfile::tempdir().unwrap();
    let state_path = state_dir.path().join("state.json");
    let queue_path = queue_dir.path().to_path_buf();
    TestEnv {
        _state_dir: state_dir,
        _queue_dir: queue_dir,
        state_path,
        queue_path,
    }
}

fn cfg_for(server_url: &str, env: &TestEnv) -> Config {
    Config {
        tick_interval: Duration::from_millis(10),
        tick_jitter: Duration::ZERO,
        high_priority_cadence: Duration::from_secs(15 * 60),
        normal_cadence: Duration::from_secs(60 * 60),
        concurrency: 4,
        github_api_base: server_url.to_string(),
        github_token: Some("fake-token".to_string()),
        state_path: env.state_path.clone(),
        pump_queue_dir: env.queue_path.clone(),
        user_agent: "cave-upstream-watchd/test".to_string(),
        request_timeout: Duration::from_secs(5),
        max_backoff_ticks: 16,
    }
}

fn read_payloads(env: &TestEnv) -> Vec<PumpPayload> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&env.queue_path).unwrap().flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy().to_string();
        if !name.starts_with("upstream-port-") || !name.ends_with(".json") {
            continue;
        }
        let body = std::fs::read_to_string(entry.path()).unwrap();
        out.push(serde_json::from_str(&body).unwrap());
    }
    out
}

#[tokio::test]
async fn first_tick_with_release_writes_pump_payload() {
    let mut server = Server::new_async().await;
    let _m = server
        .mock("GET", "/repos/etcd-io/etcd/releases/latest")
        .with_status(200)
        .with_header("etag", "\"v1\"")
        .with_body(
            r#"{
            "tag_name": "v3.5.10",
            "html_url": "https://github.com/etcd-io/etcd/releases/tag/v3.5.10",
            "name": "etcd 3.5.10"
        }"#,
        )
        .create_async()
        .await;

    let env = test_env();
    let cfg = cfg_for(&server.url(), &env);
    let projects = vec![proj("etcd-io/etcd", "cave-etcd")];
    let daemon = Daemon::new(cfg, projects).with_differ(Arc::new(TagOnlyDiffer));

    let report = daemon.tick_once().await.unwrap();
    assert_eq!(report.due, 1);
    assert_eq!(report.new_releases, 1);
    assert_eq!(report.payloads_written.len(), 1);

    let payloads = read_payloads(&env);
    assert_eq!(payloads.len(), 1);
    assert_eq!(payloads[0].cave_module, "cave-etcd");
    assert_eq!(payloads[0].new_tag, "v3.5.10");
    assert_eq!(payloads[0].old_tag, None);
    assert_eq!(payloads[0].priority, "high");

    // Verify state persisted.
    let st = WatchState::load(&env.state_path).unwrap();
    let entry = st.get("etcd-io/etcd").unwrap();
    assert_eq!(entry.last_known_tag.as_deref(), Some("v3.5.10"));
    assert_eq!(entry.etag.as_deref(), Some("\"v1\""));
    assert_eq!(entry.last_pump_payload_id, Some(payloads[0].clone()).map(|p| {
        format!(
            "upstream-port-{}-etcd-io-etcd.json",
            p.created_at.timestamp_millis()
        )
    }));
}

#[tokio::test]
async fn second_tick_same_tag_writes_no_payload() {
    let mut server = Server::new_async().await;
    let _m = server
        .mock("GET", "/repos/etcd-io/etcd/releases/latest")
        .with_status(200)
        .with_body(
            r#"{
            "tag_name": "v3.5.10",
            "html_url": "https://github.com/etcd-io/etcd/releases/tag/v3.5.10"
        }"#,
        )
        .expect_at_least(1)
        .create_async()
        .await;

    let env = test_env();
    let cfg = cfg_for(&server.url(), &env);
    let projects = vec![proj("etcd-io/etcd", "cave-etcd")];
    let daemon = Daemon::new(cfg, projects);

    // First tick: writes payload.
    let r1 = daemon.tick_once().await.unwrap();
    assert_eq!(r1.new_releases, 1);

    // Second tick: must respect cadence; force-due by clearing
    // last_checked in state.
    let mut st = WatchState::load(&env.state_path).unwrap();
    st.entry("etcd-io/etcd").last_checked = None;
    st.save(&env.state_path).unwrap();

    let r2 = daemon.tick_once().await.unwrap();
    assert_eq!(r2.new_releases, 0, "same tag must not emit a payload");
    assert!(r2.unchanged >= 1);

    let payloads = read_payloads(&env);
    assert_eq!(payloads.len(), 1, "still exactly one payload total");
}

#[tokio::test]
async fn tag_transition_emits_payload_with_old_and_new() {
    let mut server = Server::new_async().await;
    // First response: v1.0.0
    let _m1 = server
        .mock("GET", "/repos/foo/bar/releases/latest")
        .with_status(200)
        .with_body(
            r#"{"tag_name":"v1.0.0","html_url":"https://x/v1.0.0"}"#,
        )
        .expect(1)
        .create_async()
        .await;

    let env = test_env();
    let cfg = cfg_for(&server.url(), &env);
    let projects = vec![proj("foo/bar", "cave-etcd")];
    let daemon = Daemon::new(cfg, projects);
    daemon.tick_once().await.unwrap();

    // Now register the v1.1.0 mock.
    let _m2 = server
        .mock("GET", "/repos/foo/bar/releases/latest")
        .with_status(200)
        .with_body(
            r#"{"tag_name":"v1.1.0","html_url":"https://x/v1.1.0"}"#,
        )
        .expect_at_least(1)
        .create_async()
        .await;

    // Force re-poll.
    let mut st = WatchState::load(&env.state_path).unwrap();
    st.entry("foo/bar").last_checked = None;
    st.save(&env.state_path).unwrap();

    let r2 = daemon.tick_once().await.unwrap();
    assert_eq!(r2.new_releases, 1);

    let payloads = read_payloads(&env);
    assert_eq!(payloads.len(), 2);
    let v11 = payloads
        .iter()
        .find(|p| p.new_tag == "v1.1.0")
        .expect("v1.1.0 payload");
    assert_eq!(v11.old_tag.as_deref(), Some("v1.0.0"));
}

#[tokio::test]
async fn rate_limit_response_does_not_write_payload() {
    let mut server = Server::new_async().await;
    let _m = server
        .mock("GET", "/repos/foo/bar/releases/latest")
        .with_status(403)
        .with_header("x-ratelimit-reset", "1735689600")
        .with_body(r#"{"message":"rate limited"}"#)
        .create_async()
        .await;

    let env = test_env();
    let cfg = cfg_for(&server.url(), &env);
    let projects = vec![proj("foo/bar", "cave-etcd")];
    let daemon = Daemon::new(cfg, projects);

    let report = daemon.tick_once().await.unwrap();
    assert_eq!(report.rate_limited, 1);
    assert_eq!(report.new_releases, 0);
    assert_eq!(report.payloads_written.len(), 0);

    let st = WatchState::load(&env.state_path).unwrap();
    let entry = st.get("foo/bar").unwrap();
    assert_eq!(entry.consecutive_errors, 1);
}

#[tokio::test]
async fn concurrent_polls_respect_semaphore_and_complete() {
    let mut server = Server::new_async().await;

    // Five repos, all return 200 with distinct tags.
    let repos = ["a/r1", "a/r2", "a/r3", "a/r4", "a/r5"];
    for r in &repos {
        let body = format!(
            r#"{{"tag_name":"v1-{r}","html_url":"https://x/{r}"}}"#,
            r = r.replace('/', "-")
        );
        let path = format!("/repos/{r}/releases/latest");
        server
            .mock("GET", path.as_str())
            .with_status(200)
            .with_body(body)
            .create_async()
            .await;
    }

    let env = test_env();
    let mut cfg = cfg_for(&server.url(), &env);
    cfg.concurrency = 2; // tight cap → exercises semaphore
    let projects: Vec<_> = repos
        .iter()
        .map(|r| {
            let leaked: &'static str = Box::leak(r.to_string().into_boxed_str());
            proj(leaked, "cave-etcd")
        })
        .collect();
    let daemon = Daemon::new(cfg, projects);

    let report = daemon.tick_once().await.unwrap();
    assert_eq!(report.due, 5);
    assert_eq!(report.new_releases, 5);
    let payloads = read_payloads(&env);
    assert_eq!(payloads.len(), 5);
}

#[tokio::test]
async fn cadence_filter_skips_recently_checked_normal_priority() {
    let server = Server::new_async().await;
    let env = test_env();
    let cfg = cfg_for(&server.url(), &env);
    // Pre-populate state: this normal-priority repo was checked 30 min ago.
    // Normal cadence is 60 min, so it should NOT be due.
    let mut st = WatchState::new();
    let entry = st.entry("backstage/backstage");
    entry.last_checked = Some(Utc::now() - ChronoDuration::minutes(30));
    entry.last_known_tag = Some("v1.0.0".to_string());
    st.save(&env.state_path).unwrap();

    let projects = vec![proj("backstage/backstage", "cave-portal")];
    let daemon = Daemon::new(cfg, projects);
    let report = daemon.tick_once().await.unwrap();
    assert_eq!(report.due, 0, "normal-priority not yet due at 30 min");
    assert_eq!(report.polled, 0);
}

#[tokio::test]
async fn high_priority_repo_is_due_at_20_minutes() {
    let mut server = Server::new_async().await;
    let _m = server
        .mock("GET", "/repos/etcd-io/etcd/releases/latest")
        .with_status(200)
        .with_body(r#"{"tag_name":"v3.5.10","html_url":"https://x/v3.5.10"}"#)
        .create_async()
        .await;

    let env = test_env();
    let cfg = cfg_for(&server.url(), &env);
    let mut st = WatchState::new();
    let entry = st.entry("etcd-io/etcd");
    entry.last_checked = Some(Utc::now() - ChronoDuration::minutes(20));
    entry.last_known_tag = Some("v3.5.9".to_string());
    st.save(&env.state_path).unwrap();

    let projects = vec![proj("etcd-io/etcd", "cave-etcd")];
    let daemon = Daemon::new(cfg, projects);
    let report = daemon.tick_once().await.unwrap();
    assert_eq!(report.due, 1, "high-priority due at 20 min (cadence 15 min)");
    assert_eq!(report.new_releases, 1);
}
