// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//!
//! Charter v2 self-audit — cave-chaos must carry an honest, measured
//! `fill_ratio` against upstream chaos-mesh/chaos-mesh v2.7.0 (Chaos Mesh),
//! a pinned `source_sha`, the 2026-05-28 close-out audit date,
//! `parity_ratio_source = "manifest"`, 100% AGPL SPDX header coverage,
//! no stub macros in `src/`, mapped+partial+skipped+unmapped summing to total,
//! and the full experiment lifecycle + workflow + schedule surface reachable
//! through `cave_chaos`.
//!
//! 9 assertions — one per gate of the close-out checklist.

use std::fs;
use std::path::PathBuf;

const TODAY: &str = "2026-05-28";
const FLOOR_FILL_RATIO: f64 = 0.95;
const PINNED_VERSION: &str = "v2.7.0";
const PINNED_SHA: &str = "bf9cb4c3e79cb7b08c27218fe30bcd6de5ed74a4";

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

// ─── Assertion 1: upstream pinned to v2.7.0 ──────────────────────────────────

#[test]
fn assertion_1_upstream_version_pinned() {
    let m = manifest_text();
    let v = extract_after(&m, "\nversion ").or_else(|| extract_after(&m, "\nversion="));
    assert_eq!(
        v.as_deref(),
        Some(PINNED_VERSION),
        "[upstream] version must pin Chaos Mesh {} — Charter v2 always-latest gate (got {:?})",
        PINNED_VERSION,
        v
    );
}

// ─── Assertion 2: source_sha present and matches v2.7.0 ──────────────────────

#[test]
fn assertion_2_source_sha_present() {
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
        "source_sha must match the v2.7.0 commit (got {:?})",
        sha
    );
}

// ─── Assertion 3: fill_ratio >= 0.95 ─────────────────────────────────────────

#[test]
fn assertion_3_fill_ratio_meets_floor() {
    let m = manifest_text();
    let raw = extract_after(&m, "\nfill_ratio ")
        .or_else(|| extract_after(&m, "\nfill_ratio="))
        .expect("[parity] fill_ratio must be present");
    let ratio: f64 = raw.parse().expect("fill_ratio must parse as float");
    assert!(
        ratio >= FLOOR_FILL_RATIO,
        "cave-chaos close floor: fill_ratio must be >= {} (got {})",
        FLOOR_FILL_RATIO,
        ratio
    );
    assert!(ratio <= 1.0, "fill_ratio must be a fraction (got {})", ratio);
}

// ─── Assertion 4: parity_ratio_source = "manifest" ───────────────────────────

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

// ─── Assertion 5: last_audit == 2026-05-28 ───────────────────────────────────

#[test]
fn assertion_5_last_audit_is_today() {
    let m = manifest_text();
    let when = extract_after(&m, "\nlast_audit ").or_else(|| extract_after(&m, "\nlast_audit="));
    assert!(
        when.as_deref().map(|d| d.starts_with("2026-")).unwrap_or(false),
        "[parity] last_audit must reflect the 2026-* Charter v2 close-out (got {:?})",
        when
    );
    assert_eq!(
        when.as_deref(),
        Some(TODAY),
        "[parity] last_audit must be {} (got {:?})",
        TODAY,
        when
    );
}

// ─── Assertion 6: counts sum to total ────────────────────────────────────────

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
    assert_eq!(unmapped, 0, "unmapped_count must be 0");
    assert!(
        mapped + partial >= 10,
        "cave-chaos close floor: >= 10 mapped/partial Chaos Mesh surfaces (got {})",
        mapped + partial
    );
}

// ─── Assertion 7: AGPL SPDX header coverage 100% ─────────────────────────────

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
        "expected >= 8 .rs files in cave-chaos; got {}",
        total
    );
}

// ─── Assertion 8: no stub macros in src/ ─────────────────────────────────────

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

// ─── Assertion 9: core chaos surface intact ───────────────────────────────────

#[test]
fn assertion_9_chaos_surface_intact() {
    use cave_chaos::engine::{actual_duration_secs, is_active, is_high_risk, validate_experiment};
    use cave_chaos::executor::ChaosExecutor;
    use cave_chaos::models::{
        BlastRadius, ChaosExperiment, ChaosTarget, ExperimentParams, ExperimentSchedule,
        ExperimentStatus, ExperimentType, SafetyGuard,
    };
    use cave_chaos::schedule::{
        cron_field_matches, is_cron_due, next_cron_run, should_run, validate_cron_expression,
        CronField, ScheduledRunDecision,
    };
    use cave_chaos::store::ChaosStore;
    use cave_chaos::workflow::{
        execute_workflow, ChaosWorkflow, WorkflowNode, WorkflowNodeType, WorkflowStatus,
    };
    use cave_chaos::MODULE_NAME;
    use chrono::Utc;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn make_exp(exp_type: ExperimentType, ns: &str) -> ChaosExperiment {
        ChaosExperiment {
            id: Uuid::new_v4(),
            name: "audit-surface".into(),
            experiment_type: exp_type,
            target: ChaosTarget {
                namespace: ns.into(),
                selector: HashMap::new(),
                pod_count: Some(1),
            },
            parameters: ExperimentParams {
                latency_ms: Some(50),
                packet_loss_percent: None,
                cpu_load_percent: None,
                memory_mb: None,
            },
            status: ExperimentStatus::Draft,
            created_at: Utc::now(),
            started_at: None,
            ended_at: None,
            duration_secs: 30,
            blast_radius: BlastRadius::default(),
            safety_guard: SafetyGuard::default(),
            result: None,
            annotations: HashMap::new(),
        }
    }

    // 1. Module constant
    assert_eq!(MODULE_NAME, "chaos");

    // 2. Experiment models roundtrip
    let exp = make_exp(ExperimentType::NetworkLatency, "staging");
    let json = serde_json::to_string(&exp).unwrap();
    let back: ChaosExperiment = serde_json::from_str(&json).unwrap();
    assert_eq!(back.id, exp.id);

    // 3. Engine validation
    let errs = validate_experiment(&exp);
    assert!(errs.is_empty(), "latency exp with latency_ms set should be valid");
    assert!(!is_active(&exp));
    assert!(!is_high_risk(&exp));
    assert!(actual_duration_secs(&exp).is_none());

    // 4. All 17 ExperimentType variants are accessible
    let _ = ExperimentType::NetworkLatency;
    let _ = ExperimentType::NetworkPacketLoss;
    let _ = ExperimentType::NetworkCorruption;
    let _ = ExperimentType::NetworkBandwidth;
    let _ = ExperimentType::NetworkPartition;
    let _ = ExperimentType::CpuStress;
    let _ = ExperimentType::MemoryStress;
    let _ = ExperimentType::DiskFill;
    let _ = ExperimentType::IoLatency;
    let _ = ExperimentType::IoChaos;
    let _ = ExperimentType::PodKill;
    let _ = ExperimentType::ProcessKill;
    let _ = ExperimentType::NodeDrain;
    let _ = ExperimentType::ClockSkew;
    let _ = ExperimentType::HttpFault;
    let _ = ExperimentType::GrpcFault;
    let _ = ExperimentType::JvmException;

    // 5. Executor: execute, validate, rollback, check_safety
    let executor = ChaosExecutor::new();
    let mut exp_run = make_exp(ExperimentType::NetworkLatency, "staging");
    let result = executor.execute(&mut exp_run);
    assert_eq!(result.status, ExperimentStatus::Completed);
    assert!(!result.events.is_empty());
    assert!(result.metrics_after["p99_latency_ms"] > result.metrics_before["p99_latency_ms"]);

    let mut exp_abort = make_exp(ExperimentType::PodKill, "staging");
    exp_abort.status = ExperimentStatus::Running;
    exp_abort.started_at = Some(Utc::now());
    let aborted = executor.rollback(&mut exp_abort);
    assert_eq!(aborted.status, ExperimentStatus::Aborted);
    assert!(aborted.rollback_triggered);

    assert!(executor.check_safety(&exp_abort, 0.2));
    assert!(!executor.check_safety(&exp_abort, 0.9));

    // 6. Store: CRUD + schedule management
    let store = ChaosStore::new();
    let exp_s = make_exp(ExperimentType::PodKill, "staging");
    let eid = exp_s.id;
    store.insert(exp_s.clone());
    let fetched = store.get(eid).unwrap();
    assert_eq!(fetched.id, eid);
    store.update(ChaosExperiment {
        status: ExperimentStatus::Running,
        ..exp_s.clone()
    });
    assert_eq!(store.get(eid).unwrap().status, ExperimentStatus::Running);
    assert_eq!(store.list_by_status(&ExperimentStatus::Running).len(), 1);
    store.remove(eid);
    assert!(store.get(eid).is_none());

    let sched = ExperimentSchedule {
        id: Uuid::new_v4(),
        experiment_id: Uuid::new_v4(),
        cron_expression: "0 * * * *".into(),
        enabled: true,
        last_run: None,
        next_run: None,
        max_runs: Some(10),
        run_count: 0,
    };
    let sid = sched.id;
    store.add_schedule(sched.clone());
    assert!(store.get_schedule(sid).is_some());
    store.remove_schedule(sid);
    assert!(store.get_schedule(sid).is_none());

    // 7. Workflow: sequential and parallel execution
    let exp_wf = make_exp(ExperimentType::NetworkLatency, "staging");
    let wf_id = exp_wf.id;
    let nodes = vec![WorkflowNode {
        id: "s1".into(),
        node_type: WorkflowNodeType::Sequential,
        experiment_id: Some(wf_id),
        children: vec![],
        deadline_secs: Some(60),
    }];
    let wf_result = execute_workflow(&nodes, &[exp_wf]);
    assert_eq!(wf_result.status, WorkflowStatus::Completed);

    let wf = ChaosWorkflow::new(Uuid::new_v4(), "audit-wf");
    assert_eq!(wf.status, WorkflowStatus::Pending);

    // 8. Schedule: cron parsing, validation, evaluation
    assert!(validate_cron_expression("0 2 * * 1").is_ok());
    assert!(validate_cron_expression("").is_err());
    assert!(cron_field_matches(&CronField::Wildcard, 42));
    assert!(cron_field_matches(&CronField::Exact(15), 15));
    assert!(!cron_field_matches(&CronField::Exact(15), 16));

    let now = Utc::now();
    let _ = next_cron_run("0 * * * *", &now).unwrap();

    // every minute fires now
    let always_sched = ExperimentSchedule {
        id: Uuid::new_v4(),
        experiment_id: Uuid::new_v4(),
        cron_expression: "* * * * *".into(),
        enabled: true,
        last_run: None,
        next_run: None,
        max_runs: None,
        run_count: 0,
    };
    assert_eq!(should_run(&always_sched, &now), ScheduledRunDecision::Run);

    let disabled_sched = ExperimentSchedule {
        enabled: false,
        ..always_sched.clone()
    };
    assert_eq!(should_run(&disabled_sched, &now), ScheduledRunDecision::Skip);

    let _ = is_cron_due("* * * * *", &now).unwrap();

    // 9. BlastRadius and SafetyGuard defaults
    let br = BlastRadius::default();
    assert!(br.max_pod_fraction > 0.0);

    let sg = SafetyGuard::default();
    assert!(sg.enabled);
    assert!(sg.protected_namespaces.contains(&"kube-system".to_string()));
}

// ─── helpers ──────────────────────────────────────────────────────────────────

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
