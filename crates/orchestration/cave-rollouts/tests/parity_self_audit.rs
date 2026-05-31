// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Charter v2 self-audit — cave-rollouts must carry an honest, measured
//! `fill_ratio` against upstream argoproj/argo-rollouts v1.9.0, a pinned
//! `source_sha`, the 2026-05-24 close-out audit date,
//! `parity_ratio_source = "manifest"`, 100% AGPL SPDX header coverage,
//! no stub macros in `src/`, mapped+partial+skipped+unmapped summing to
//! total, and the full Rollout + Experiment + AnalysisRun + traffic_router
//! + notification public surface reachable through `cave_rollouts`.
//!
//! 9 assertions — one per gate of the close-out checklist.

use std::fs;
use std::path::PathBuf;

const TODAY: &str = "2026-05-31";
const FLOOR_FILL_RATIO: f64 = 0.95;
const PINNED_VERSION: &str = "v1.9.0";
const PINNED_SHA: &str = "838d4e792be666ec11bd0c80331e0c5511b5010e";

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

#[test]
fn assertion_1_upstream_version_pinned() {
    let m = manifest_text();
    let v = extract_after(&m, "\nversion ").or_else(|| extract_after(&m, "\nversion="));
    assert_eq!(
        v.as_deref(),
        Some(PINNED_VERSION),
        "[upstream] version must pin Argo Rollouts {} (got {:?})",
        PINNED_VERSION,
        v
    );
}

#[test]
fn assertion_2_source_sha_matches_version() {
    let m = manifest_text();
    let sha = extract_after(&m, "\nsource_sha ").or_else(|| extract_after(&m, "\nsource_sha="));
    assert_eq!(sha.as_deref(), Some(PINNED_SHA), "source_sha mismatch");
}

#[test]
fn assertion_3_fill_ratio_meets_floor() {
    let m = manifest_text();
    let raw = extract_after(&m, "\nfill_ratio ")
        .or_else(|| extract_after(&m, "\nfill_ratio="))
        .expect("[parity] fill_ratio must be present");
    let ratio: f64 = raw.parse().expect("fill_ratio must parse as float");
    assert!(
        ratio >= FLOOR_FILL_RATIO,
        "fill_ratio must be >= {} (got {})",
        FLOOR_FILL_RATIO,
        ratio
    );
    assert!(ratio <= 1.0, "fill_ratio must be a fraction (got {})", ratio);
}

#[test]
fn assertion_4_parity_ratio_source_is_manifest() {
    let m = manifest_text();
    let v = extract_after(&m, "\nparity_ratio_source ")
        .or_else(|| extract_after(&m, "\nparity_ratio_source="));
    assert_eq!(v.as_deref(), Some("manifest"));
}

#[test]
fn assertion_5_last_audit_is_today() {
    let m = manifest_text();
    let when = extract_after(&m, "\nlast_audit ").or_else(|| extract_after(&m, "\nlast_audit="));
    assert_eq!(when.as_deref(), Some(TODAY));
}

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
    assert_eq!(mapped + partial + skipped + unmapped, total);
    assert!(mapped >= 15, ">= 15 mapped subsystems (got {})", mapped);
}

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
    assert!(missing.is_empty(), "missing AGPL SPDX: {:?}", missing);
    assert!(total >= 10);
}

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
    assert!(offenders.is_empty(), "stub gate failed: {}", offenders.join("\n"));
}

#[test]
fn assertion_9_rollouts_surface_intact() {
    use cave_rollouts::experiment::{
        Experiment, ExperimentAnalysis, ExperimentPhase, ExperimentTemplate,
    };
    use cave_rollouts::traffic_router::{render_patch, TrafficProvider, WeightSplit};

    let mut e = Experiment::new(
        "demo",
        "argo",
        10,
        vec![ExperimentTemplate {
            name: "a".into(),
            replicas: 1,
            image: "img:1".into(),
            selector: None,
            weight: None,
        }],
    );
    assert_eq!(e.phase, ExperimentPhase::Pending);
    let t0 = chrono::Utc::now();
    e.start(t0);
    assert_eq!(e.phase, ExperimentPhase::Running);
    e.analyses.push(ExperimentAnalysis {
        template_name: "lat".into(),
        inconclusive_limit: 5,
        failure_limit: 1,
    });
    let phase = e.evaluate(2, 0, t0 + chrono::Duration::seconds(1));
    assert_eq!(phase, ExperimentPhase::Failed);

    let split = WeightSplit::new(25);
    assert_eq!(split.stable, 75);
    for p in [
        TrafficProvider::Istio {
            virtual_service: "vs".into(),
            namespace: "ns".into(),
        },
        TrafficProvider::Smi {
            trafficsplit_name: "ts".into(),
            namespace: "ns".into(),
        },
        TrafficProvider::Nginx {
            stable_ingress: "i".into(),
            namespace: "ns".into(),
        },
        TrafficProvider::Alb {
            ingress: "i".into(),
            annotation_prefix: "alb.ingress.kubernetes.io".into(),
        },
        TrafficProvider::Apisix {
            route: "r".into(),
            namespace: "ns".into(),
        },
        TrafficProvider::Plugin {
            plugin_name: "p".into(),
            config: serde_json::json!({}),
        },
        TrafficProvider::Traefik {
            traefik_service: "ts".into(),
            namespace: "ns".into(),
        },
        TrafficProvider::Ambassador {
            mapping: "m".into(),
            namespace: "ns".into(),
        },
        TrafficProvider::AppMesh {
            virtual_router: "vr".into(),
            namespace: "ns".into(),
        },
    ] {
        let patch = render_patch(&p, &split, "stable", "canary");
        assert!(!patch.is_null());
    }

    use cave_rollouts::analysis::compute_analysis_phase;
    use cave_rollouts::engine::initial_status;
    use cave_rollouts::models::{
        BlueGreenStrategy, CanaryStep, CanaryStrategy, RolloutStrategy,
    };
    use cave_rollouts::notifications::should_notify;
    use cave_rollouts::types::{NotificationEvent, RolloutPhase};

    let _strat = RolloutStrategy::Canary(CanaryStrategy {
        steps: vec![CanaryStep::SetWeight { weight: 10 }],
        stable_service: "s".into(),
        canary_service: "c".into(),
        max_weight: 100,
        step_weight_increment: 10,
        threshold: None,
        max_analysis_failures: None,
        mirror_percentage: None,
    });
    let bg = RolloutStrategy::BlueGreen(BlueGreenStrategy {
        active_service: "active".into(),
        preview_service: "preview".into(),
        scale_down_delay_seconds: 30,
        auto_promote_seconds: 0,
        pre_promotion_analysis: None,
        post_promotion_analysis: None,
        anti_affinity: None,
    });
    let _ = initial_status(&bg);
    let _ = compute_analysis_phase(&[]);
    assert!(should_notify(
        &[NotificationEvent::RolloutPromoted],
        &NotificationEvent::RolloutPromoted,
    ));
    let _ = RolloutPhase::Healthy;
}

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
