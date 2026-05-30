// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Integration tests for the Sift forensic-analysis layer
//! (grafana/sift companion port). One test module per analyzer.

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
