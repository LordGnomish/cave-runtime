// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Integration tests for the Sift forensic-analysis layer
//! (grafana/sift companion port). One test module per analyzer.

// ─── Shared event-builder helpers ───────────────────────────────────────────
#[allow(dead_code)]
mod common {
    use cave_forensics::events::KernelEvent;
    use cave_forensics::events::process_exec::{ProcessExecEvent, ProcessExitEvent};
    use cave_forensics::process::{Credentials, Namespaces, Process};
    use chrono::{DateTime, TimeZone, Utc};

    pub fn ts(s: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(s, 0).unwrap()
    }

    pub fn proc(binary: &str, container: Option<&str>) -> Process {
        Process {
            exec_id: format!("{binary}-{:?}", container),
            pid: 100,
            pid_in_ns: 1,
            binary: binary.into(),
            arguments: String::new(),
            cwd: "/".into(),
            credentials: Credentials::default(),
            namespaces: Namespaces::default(),
            parent_exec_id: None,
            container_id: container.map(|c| c.into()),
            pod_name: container.map(|c| format!("pod-{c}")),
            pod_namespace: Some("default".into()),
            start_time: ts(0),
            end_time: None,
        }
    }

    pub fn exec(binary: &str, container: Option<&str>, at: i64) -> KernelEvent {
        KernelEvent::ProcessExec(ProcessExecEvent {
            process: proc(binary, container),
            ancestors: vec![],
            observed_at: ts(at),
        })
    }

    pub fn exit(binary: &str, container: Option<&str>, status: i32, signal: Option<i32>, at: i64) -> KernelEvent {
        KernelEvent::ProcessExit(ProcessExitEvent {
            process: proc(binary, container),
            status,
            signal,
            observed_at: ts(at),
        })
    }
}

// ─── Cycle 1: Sift "Error Pattern Logs" — drain-style log clustering ────────
mod error_pattern {
    use cave_forensics::sift::error_pattern::{
        cluster_log_lines, dominant_error_pattern, normalize_template,
    };

    #[test]
    fn test_normalize_template_replaces_numbers_and_hex() {
        // Digits → <NUM>, 0x-hex → <HEX>, dotted-quad → <IP>.
        assert_eq!(
            normalize_template("connection 42 to 10.0.0.5 failed at 0xdeadbeef"),
            "connection <NUM> to <IP> failed at <HEX>"
        );
    }

    #[test]
    fn test_normalize_template_stable_for_constant_lines() {
        let a = normalize_template("OOMKilled container web pid 1234");
        let b = normalize_template("OOMKilled container web pid 9981");
        assert_eq!(a, b, "varying pids must collapse to the same template");
    }

    #[test]
    fn test_cluster_groups_by_template_sorted_by_count_desc() {
        let lines = [
            "request 1 failed code 500",
            "request 2 failed code 500",
            "request 3 failed code 500",
            "healthcheck ok",
        ];
        let clusters = cluster_log_lines(&lines);
        assert_eq!(clusters.len(), 2);
        // Most frequent template first.
        assert_eq!(clusters[0].count, 3);
        assert_eq!(clusters[0].template, "request <NUM> failed code <NUM>");
        assert_eq!(clusters[1].count, 1);
        // Examples are retained (capped).
        assert!(clusters[0].examples.contains(&"request 1 failed code 500".to_string()));
    }

    #[test]
    fn test_dominant_error_pattern_picks_top_error_cluster() {
        let lines = [
            "GET /health 200 ok",
            "GET /health 200 ok",
            "panic: nil pointer at 0x10",
            "panic: nil pointer at 0x20",
            "panic: nil pointer at 0x30",
        ];
        let top = dominant_error_pattern(&lines).expect("an error cluster exists");
        assert_eq!(top.count, 3);
        assert_eq!(top.template, "panic: nil pointer at <HEX>");
    }

    #[test]
    fn test_dominant_error_pattern_none_when_no_errors() {
        let lines = ["GET /health 200 ok", "served 12 requests"];
        assert!(dominant_error_pattern(&lines).is_none());
    }
}

// ─── Cycle 2: Sift "Kube Crashes" — crashloop detection ─────────────────────
mod crashloop {
    use super::common::{exec, exit};
    use cave_forensics::models::ForensicSeverity;
    use cave_forensics::sift::crashloop::detect_crashloops;

    #[test]
    fn test_detects_repeated_abnormal_exits_in_window() {
        // 4 non-zero exits of the same container in 30s, window=60s,
        // min_restarts=3 → one crashloop finding.
        let events = vec![
            exit("/app", Some("c1"), 1, None, 0),
            exit("/app", Some("c1"), 1, None, 10),
            exit("/app", Some("c1"), 137, Some(9), 20),
            exit("/app", Some("c1"), 1, None, 30),
        ];
        let f = detect_crashloops(&events, 60, 3);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].key, "c1");
        assert_eq!(f[0].restarts, 4);
    }

    #[test]
    fn test_clean_exits_are_not_crashes() {
        // status 0 + no signal = graceful; below threshold → no finding.
        let events = vec![
            exit("/app", Some("c1"), 0, None, 0),
            exit("/app", Some("c1"), 0, None, 10),
            exit("/app", Some("c1"), 0, None, 20),
            exit("/app", Some("c1"), 0, None, 30),
        ];
        assert!(detect_crashloops(&events, 60, 3).is_empty());
    }

    #[test]
    fn test_exits_outside_window_do_not_accumulate() {
        // 3 crashes but spread over 5 minutes; window=60s → no single
        // 60s span holds >= 3, so no finding.
        let events = vec![
            exit("/app", Some("c1"), 1, None, 0),
            exit("/app", Some("c1"), 1, None, 120),
            exit("/app", Some("c1"), 1, None, 300),
        ];
        assert!(detect_crashloops(&events, 60, 3).is_empty());
    }

    #[test]
    fn test_keys_separated_per_container() {
        let events = vec![
            exit("/app", Some("c1"), 1, None, 0),
            exit("/app", Some("c1"), 1, None, 5),
            exit("/app", Some("c1"), 1, None, 10),
            exit("/app", Some("c2"), 1, None, 0),
        ];
        let f = detect_crashloops(&events, 60, 3);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].key, "c1");
    }

    #[test]
    fn test_severity_scales_with_restart_count() {
        let many: Vec<_> = (0..10).map(|i| exit("/app", Some("c1"), 1, None, i * 2)).collect();
        let f = detect_crashloops(&many, 60, 3);
        assert_eq!(f[0].severity, ForensicSeverity::Critical);
    }

    #[test]
    fn test_exec_events_are_ignored() {
        // Pure exec churn (no exits) is not a crashloop.
        let events = vec![
            exec("/app", Some("c1"), 0),
            exec("/app", Some("c1"), 5),
            exec("/app", Some("c1"), 10),
        ];
        assert!(detect_crashloops(&events, 60, 3).is_empty());
    }
}
