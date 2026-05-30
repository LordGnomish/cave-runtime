// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//!
//! Charter v2 self-audit — cave-uptime must carry an honest, measured
//! `fill_ratio` against upstream louislam/uptime-kuma v1.23.13,
//! a pinned `source_sha` for reproducibility, the 2026-05-28 close-out
//! audit date, `parity_ratio_source = "manifest"`, 100% AGPL SPDX
//! header coverage, no stub macros in `src/`, mapped+partial+skipped+
//! unmapped summing to total, and the full probe execution + scheduler +
//! history + status page + CRUD routes surface reachable through `cave_uptime`.
//!
//! 9 assertions — one per gate of the close-out checklist.

use std::fs;
use std::path::PathBuf;

const TODAY: &str = "2026-05-30";
const FLOOR_FILL_RATIO: f64 = 0.65;
const PINNED_VERSION: &str = "v1.23.13";
const PINNED_SHA: &str = "3b32a5c99be7ace7f5fbc58dd8c12b2a13a5c4d8";

fn manifest_text() -> String {
    let p: PathBuf = [env!("CARGO_MANIFEST_DIR"), "parity.manifest.toml"]
        .iter()
        .collect();
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {:?}: {}", p, e))
}

fn extract_after(text: &str, needle: &str) -> Option<String> {
    let i = text.find(needle)?;
    let rest = &text[i + needle.len()..];
    let line_end = rest.find('\n').unwrap_or(rest.len());
    let line = &rest[..line_end];
    let stripped = line.trim().trim_start_matches('=').trim();
    let comment_split = stripped.split('#').next().unwrap_or(stripped).trim();
    let unquoted = comment_split.trim_matches('"');
    Some(unquoted.to_string())
}

// ─── Assertion 1: upstream pinned to v1.23.13 ───────────────────────────────

#[test]
fn assertion_1_upstream_version_pinned() {
    let m = manifest_text();
    let v = extract_after(&m, "\nversion ").or_else(|| extract_after(&m, "\nversion="));
    assert_eq!(
        v.as_deref(),
        Some(PINNED_VERSION),
        "[upstream] version must pin Uptime Kuma {} — Charter v2 always-latest gate (got {:?})",
        PINNED_VERSION,
        v
    );
}

// ─── Assertion 2: source_sha is set ─────────────────────────────────────────

#[test]
fn assertion_2_source_sha_set() {
    let m = manifest_text();
    let sha = extract_after(&m, "\nsource_sha ").or_else(|| extract_after(&m, "\nsource_sha="));
    assert!(
        sha.is_some() && !sha.as_deref().unwrap_or("").is_empty(),
        "[upstream] source_sha must be set (got {:?})",
        sha
    );
    assert_eq!(
        sha.as_deref(),
        Some(PINNED_SHA),
        "source_sha must match the {} tag commit (got {:?})",
        PINNED_VERSION,
        sha
    );
}

// ─── Assertion 3: fill_ratio >= 0.65 ────────────────────────────────────────

#[test]
fn assertion_3_fill_ratio_meets_floor() {
    let m = manifest_text();
    let raw = extract_after(&m, "\nfill_ratio ")
        .or_else(|| extract_after(&m, "\nfill_ratio="))
        .expect("[parity] fill_ratio must be present");
    let ratio: f64 = raw.parse().expect("fill_ratio must parse as float");
    assert!(
        ratio >= FLOOR_FILL_RATIO,
        "cave-uptime floor: fill_ratio must be >= {} (got {})",
        FLOOR_FILL_RATIO,
        ratio
    );
    assert!(ratio <= 1.0, "fill_ratio must be a fraction (got {})", ratio);
}

// ─── Assertion 4: parity_ratio_source = "manifest" ──────────────────────────

#[test]
fn assertion_4_parity_ratio_source_is_manifest() {
    let m = manifest_text();
    let v = extract_after(&m, "\nparity_ratio_source ")
        .or_else(|| extract_after(&m, "\nparity_ratio_source="));
    assert_eq!(
        v.as_deref(),
        Some("manifest"),
        "parity_ratio_source must be \"manifest\" (got {:?})",
        v
    );
}

// ─── Assertion 5: last_audit is today ───────────────────────────────────────

#[test]
fn assertion_5_last_audit_is_today() {
    let m = manifest_text();
    let when = extract_after(&m, "\nlast_audit ").or_else(|| extract_after(&m, "\nlast_audit="));
    assert_eq!(
        when.as_deref(),
        Some(TODAY),
        "[parity] last_audit must reflect the {} Charter v2 close-out (got {:?})",
        TODAY,
        when
    );
}

// ─── Assertion 6: counts sum to total + >= 10 mapped ────────────────────────

#[test]
fn assertion_6_counts_sum_to_total() {
    let m = manifest_text();
    let read = |k: &str| -> Option<u64> {
        let s = extract_after(&m, &format!("\n{} ", k))
            .or_else(|| extract_after(&m, &format!("\n{}=", k)))?;
        s.parse().ok()
    };
    let mapped = read("mapped_count").expect("mapped_count");
    let partial = read("partial_count").expect("partial_count");
    let skipped = read("skipped_count").expect("skipped_count");
    let unmapped = read("unmapped_count").expect("unmapped_count");
    let total = read("total").expect("total");
    assert_eq!(
        mapped + partial + skipped + unmapped,
        total,
        "mapped+partial+skipped+unmapped must equal total"
    );
    assert!(
        mapped >= 10,
        "cave-uptime floor: >= 10 mapped Uptime Kuma surfaces (got {})",
        mapped
    );
    assert_eq!(
        unmapped, 0,
        "unmapped_count must be 0 after honest close (got {})",
        unmapped
    );
}

// ─── Assertion 7: AGPL SPDX header coverage 100% ────────────────────────────

#[test]
fn assertion_7_agpl_spdx_header_coverage() {
    let root: PathBuf = [env!("CARGO_MANIFEST_DIR")].iter().collect();
    let mut missing = Vec::new();
    let mut total = 0usize;
    walk(&root, &mut |p| {
        if p.extension().map(|e| e == "rs").unwrap_or(false) {
            total += 1;
            let head = fs::read_to_string(p)
                .ok()
                .and_then(|s| s.lines().next().map(|l| l.to_string()))
                .unwrap_or_default();
            if !head.contains("SPDX-License-Identifier: AGPL-3.0-or-later") {
                missing.push(p.display().to_string());
            }
        }
    });
    assert!(
        missing.is_empty(),
        "{} of {} .rs files missing AGPL SPDX header: {:?}",
        missing.len(),
        total,
        missing
    );
    assert!(
        total >= 8,
        "expected >= 8 .rs files in cave-uptime; got {}",
        total
    );
}

// ─── Assertion 8: no stub macros in src/ ────────────────────────────────────

#[test]
fn assertion_8_no_stub_macros_in_src() {
    let src: PathBuf = [env!("CARGO_MANIFEST_DIR"), "src"].iter().collect();
    let mut offenders: Vec<String> = Vec::new();
    walk(&src, &mut |p| {
        if !p.extension().map(|e| e == "rs").unwrap_or(false) {
            return;
        }
        let Ok(text) = fs::read_to_string(p) else {
            return;
        };
        for (lineno, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            if trimmed.contains("todo!(")
                || trimmed.contains("unimplemented!(")
                || trimmed.contains("panic!(\"stub")
                || trimmed.contains("panic!(\"todo")
            {
                offenders.push(format!("{}:{}: {}", p.display(), lineno + 1, line.trim()));
            }
        }
    });
    assert!(
        offenders.is_empty(),
        "Charter v2 no-stub gate failed in src/:\n{}",
        offenders.join("\n")
    );
}

// ─── Assertion 9: core uptime surface reachable ──────────────────────────────

#[test]
fn assertion_9_uptime_surface_intact() {
    use cave_uptime::history::{HeartbeatStore, UptimeWindow, compute_window_stats};
    use cave_uptime::models::{ProbeResult, ProbeType, UptimeProbe, UptimeStats};
    use cave_uptime::probe::{
        HttpProbeConfig, ProbeError, build_probe_result, evaluate_push_probe,
    };
    use cave_uptime::retry::{RetryConfig, execute_with_retry};
    use cave_uptime::scheduler::{ProbeScheduler, SchedulerConfig};
    use cave_uptime::status::{MonitorStatus, ProbeStatusSummary, StatusPage, build_status_page};
    use cave_uptime::store::ProbeStore;
    use cave_uptime::{AppState, MODULE_NAME};
    use chrono::Utc;
    use std::sync::Arc;
    use uuid::Uuid;

    // MODULE_NAME constant
    assert_eq!(MODULE_NAME, "uptime");

    // ── 1. UptimeProbe construction ──
    let probe = UptimeProbe {
        id: Uuid::new_v4(),
        name: "API".to_string(),
        target_url: "https://api.example.com/health".to_string(),
        probe_type: ProbeType::Http,
        interval_seconds: 60,
        timeout_ms: 5000,
        enabled: true,
    };
    assert!(probe.enabled);

    // ── 2. ProbeType variants ──
    let _http = ProbeType::Http;
    let _tcp = ProbeType::Tcp;
    let _dns = ProbeType::Dns;
    let _ping = ProbeType::Ping;

    // ── 3. ProbeStore CRUD ──
    let store = ProbeStore::new();
    let id = probe.id;
    store.insert(probe.clone());
    assert!(store.get(id).is_some());
    assert_eq!(store.list().len(), 1);
    assert!(store.delete(id));
    assert!(store.get(id).is_none());

    // ── 4. HttpProbeConfig ──
    let cfg = HttpProbeConfig::new("https://x.com".to_string());
    assert_eq!(cfg.method, "GET");
    assert_eq!(cfg.timeout_ms, 5000);

    // ── 5. build_probe_result ──
    let r = build_probe_result(Uuid::new_v4(), true, 42, Some(200), None);
    assert!(r.success);
    assert_eq!(r.latency_ms, 42);

    // ── 6. evaluate_push_probe ──
    let now = Utc::now().timestamp();
    let push_ok = evaluate_push_probe(Uuid::new_v4(), Some(now - 10), 60);
    assert!(push_ok.success);
    let push_fail = evaluate_push_probe(Uuid::new_v4(), None, 60);
    assert!(!push_fail.success);

    // ── 7. HeartbeatStore + window stats ──
    let id2 = Uuid::new_v4();
    let hs = HeartbeatStore::new(100);
    for i in 0..10 {
        hs.record(build_probe_result(id2, true, 50 + i, Some(200), None));
    }
    let s = hs.window_stats(id2, UptimeWindow::Hours24);
    assert_eq!(s.total_checks, 10);
    assert!((s.uptime_pct - 100.0).abs() < 0.01);

    let labels = [
        UptimeWindow::Hours24.label(),
        UptimeWindow::Days7.label(),
        UptimeWindow::Days30.label(),
    ];
    assert_eq!(labels, ["24h", "7d", "30d"]);

    // ── 8. ProbeScheduler ──
    let sched = ProbeScheduler::new(SchedulerConfig::default());
    sched.register(probe.clone());
    assert_eq!(sched.probe_count(), 1);
    let due = sched.due_probes();
    assert!(!due.is_empty());
    sched.mark_executed(probe.id);

    // ── 9. MonitorStatus ──
    assert!(MonitorStatus::Up.is_up());
    assert!(!MonitorStatus::Down.is_up());
    assert_eq!(MonitorStatus::Up.label(), "up");

    // ── 10. StatusPage ──
    let summaries = vec![ProbeStatusSummary {
        probe_id: Uuid::new_v4(),
        name: "API".to_string(),
        status: MonitorStatus::Up,
        uptime_24h: 99.9,
        avg_latency_ms: 42.0,
        last_check_ms: 40,
    }];
    let page = build_status_page("Cave Platform", summaries);
    assert!(page.all_operational());
    assert_eq!(page.up_count(), 1);
    assert_eq!(page.down_count(), 0);

    // ── 11. RetryConfig ──
    let rc = RetryConfig::default();
    assert!(rc.max_attempts > 0);
    let d0 = rc.delay_for_attempt(0);
    let d1 = rc.delay_for_attempt(1);
    assert!(d1 >= d0);

    // ── 12. AppState ──
    let state = Arc::new(AppState::new());
    let _router = cave_uptime::router(state);

    // ── 13. UptimeStats from engine ──
    let results: Vec<ProbeResult> = (0..5)
        .map(|_| build_probe_result(Uuid::new_v4(), true, 100, Some(200), None))
        .collect();
    let pid = Uuid::new_v4();
    let stats: UptimeStats = cave_uptime::engine::calculate_stats(pid, &results);
    assert_eq!(stats.total_checks, 5);
    assert!((stats.uptime_percentage - 100.0).abs() < 0.01);

    // ── 14. ProbeError variants exist ──
    let _timeout = ProbeError::Timeout("5000ms".to_string());
    let _conn = ProbeError::ConnectionFailed("refused".to_string());
    let _dns = ProbeError::DnsResolutionFailed("NXDOMAIN".to_string());
}

// ─── helpers ────────────────────────────────────────────────────────────────

fn walk(dir: &PathBuf, cb: &mut dyn FnMut(&PathBuf)) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name()
                .map(|n| n.to_string_lossy().starts_with('.'))
                .unwrap_or(false)
            {
                continue;
            }
            if p.file_name().map(|n| n == "target").unwrap_or(false) {
                continue;
            }
            walk(&p, cb);
        } else {
            cb(&p);
        }
    }
}
