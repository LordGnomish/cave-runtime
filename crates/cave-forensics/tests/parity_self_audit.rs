// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Charter v2 8-gate self-audit — cave-forensics must carry an honest,
//! measured `fill_ratio` against upstream cilium/tetragon v1.7.0 with
//! pinned `source_sha`, today's `last_audit`, 100% AGPL SPDX header
//! coverage, no stub macros in src/, and mapped+partial+skipped+unmapped
//! summing to total.
//!
//! 9 assertions — one per gate of the close-out checklist + bonus
//! surface-integrity check.

use cave_forensics::parity_self_audit as a;
use std::path::PathBuf;

// ─── Assertion 1: G1 — tetragon v1.7.0 + source_sha pinned ──────────────────

#[test]
fn assertion_1_gate_1_upstream_pinned() {
    let m = a::manifest_text();
    a::gate_1_upstream_pinned(&m).expect("G1 upstream pin");
}

// ─── Assertion 2: G2 — every [[mapped]] local_files exists on disk ──────────

#[test]
fn assertion_2_gate_2_mapped_files_exist() {
    let m = a::manifest_text();
    a::gate_2_mapped_files_exist(&m).expect("G2 mapped local_files exist");
}

// ─── Assertion 3: G3 — every [[partial]] has a gap reason ───────────────────

#[test]
fn assertion_3_gate_3_partial_has_reason() {
    let m = a::manifest_text();
    a::gate_3_partial_has_gap_reason(&m).expect("G3 partial reason");
}

// ─── Assertion 4: G4 — every [[skipped]] has scope_cut target + reason ──────

#[test]
fn assertion_4_gate_4_skipped_has_scope_cut() {
    let m = a::manifest_text();
    a::gate_4_skipped_has_scope_cut(&m).expect("G4 skipped scope_cut");
}

// ─── Assertion 5: G5 — every [[unmapped]] has an honest reason ──────────────

#[test]
fn assertion_5_gate_5_unmapped_has_reason() {
    let m = a::manifest_text();
    a::gate_5_unmapped_has_reason(&m).expect("G5 unmapped reason");
}

// ─── Assertion 6: G6 — fill_ratio ≥ 0.95 ────────────────────────────────────

#[test]
fn assertion_6_gate_6_fill_ratio_meets_floor() {
    let m = a::manifest_text();
    let r = a::gate_6_fill_ratio(&m).expect("G6 fill_ratio");
    assert!(r >= a::FLOOR_FILL_RATIO, "fill_ratio {r} >= floor");
    let h = a::honest_ratio(&m);
    assert!(h >= a::FLOOR_HONEST_RATIO, "honest_ratio {h} >= 0.65");

    // Also assert the manifest's recorded fill_ratio matches the
    // computed value (no manifest/computation drift).
    let recorded: f64 = a::extract_scalar(&m, "fill_ratio")
        .expect("fill_ratio in [parity]")
        .parse()
        .expect("fill_ratio is a number");
    assert!(
        (recorded - r).abs() < 0.001,
        "recorded fill_ratio {recorded} drifted from computed {r}"
    );
}

// ─── Assertion 7: G7 — AGPL SPDX header coverage 100% ───────────────────────

#[test]
fn assertion_7_gate_7_spdx_coverage() {
    let root: PathBuf = [env!("CARGO_MANIFEST_DIR")].iter().collect();
    let total = a::gate_7_spdx_coverage(&root).expect("G7 SPDX coverage");
    assert!(
        total >= 15,
        "expected >= 15 .rs files in cave-forensics; got {total}"
    );
}

// ─── Assertion 8: G8 — no stub macros in src/ ───────────────────────────────

#[test]
fn assertion_8_gate_8_no_stub_macros() {
    let src: PathBuf = [env!("CARGO_MANIFEST_DIR"), "src"].iter().collect();
    a::gate_8_no_stub_macros(&src).expect("G8 no stubs");
}

// ─── Assertion 9: Bonus — full tetragon surface reachable + last_audit ──────

#[test]
fn assertion_9_surface_integrity_and_audit_date() {
    use cave_forensics::case::CaseStore;
    use cave_forensics::enforcer::{Enforcer, EnforcementDecision};
    use cave_forensics::events::KernelEvent;
    use cave_forensics::events::process_exec::ProcessExecEvent;
    use cave_forensics::export::grpc_codec::{decode_events, encode_event};
    use cave_forensics::export::json_stream::{decode_ndjson, encode_ndjson};
    use cave_forensics::filter::{ActionKind, FilterGroup, FilterOp, MatchAction, MatchBinary};
    use cave_forensics::models::{EvidenceType, ForensicSeverity};
    use cave_forensics::observability::{alert_rules, dashboard_panels};
    use cave_forensics::process::{Credentials, Namespaces, Process};
    use cave_forensics::tracing_policy::{KProbeSpec, PolicyKind, PolicyMeta, TracingPolicy, TracingPolicySpec};
    use cave_forensics::{MODULE_NAME, State, router};
    use chrono::{TimeZone, Utc};
    use std::sync::Arc;

    let m = a::manifest_text();
    assert_eq!(
        a::extract_scalar(&m, "last_audit").as_deref(),
        Some(a::TODAY),
        "last_audit must equal {}",
        a::TODAY
    );

    // Crate-root identity + router.
    assert_eq!(MODULE_NAME, "forensics");
    let _r = router(Arc::new(State::default()));

    // Tracing policy round-trip + validate.
    let p = TracingPolicy {
        api_version: "cilium.io/v1alpha1".into(),
        kind: PolicyKind::TracingPolicy,
        metadata: PolicyMeta {
            name: "deny-bash".into(),
            ..Default::default()
        },
        spec: TracingPolicySpec {
            kprobes: vec![KProbeSpec {
                call: "sys_execve".into(),
                syscall: true,
                return_: false,
                args: vec![],
                selectors: vec![],
            }],
            ..Default::default()
        },
    };
    p.validate().unwrap();

    // Filter + enforcer wired against a process_exec event.
    let mut g = FilterGroup::default();
    g.match_binaries.push(MatchBinary {
        operator: FilterOp::Equal,
        values: vec!["/bin/bash".into()],
    });
    g.match_actions.push(MatchAction {
        action: ActionKind::Sigkill,
        arg_error: None,
        arg_sig: Some(9),
        arg_fd: None,
        arg_name: None,
        rate_limit: None,
    });
    let ev = KernelEvent::ProcessExec(ProcessExecEvent {
        process: Process {
            exec_id: "x".into(),
            pid: 99,
            pid_in_ns: 1,
            binary: "/bin/bash".into(),
            arguments: String::new(),
            cwd: "/".into(),
            credentials: Credentials::default(),
            namespaces: Namespaces::default(),
            parent_exec_id: None,
            container_id: None,
            pod_name: None,
            pod_namespace: None,
            start_time: Utc.timestamp_opt(0, 0).unwrap(),
            end_time: None,
        },
        ancestors: vec![],
        observed_at: Utc.timestamp_opt(0, 0).unwrap(),
    });
    let e = Enforcer::default();
    let dec: Vec<EnforcementDecision> = e.decide("p", &g, &ev).unwrap();
    assert_eq!(dec.len(), 1);
    assert_eq!(dec[0].action, ActionKind::Sigkill);

    // Export round-trip via both codecs.
    let bytes = encode_event(&ev).unwrap();
    let back = decode_events(&bytes).unwrap();
    assert_eq!(back, vec![ev.clone()]);
    let json = encode_ndjson(&[ev.clone()]).unwrap();
    let back2 = decode_ndjson(&json).unwrap();
    assert_eq!(back2, vec![ev.clone()]);

    // Case store + evidence ingestion.
    let store = CaseStore::new();
    let c = store.open("bash exec spike", "demo", ForensicSeverity::Critical);
    let c2 = store.ingest_event(c.id, &ev, "tetragon-agent").unwrap();
    assert_eq!(c2.evidence.len(), 1);
    assert!(matches!(c2.evidence[0].evidence_type, EvidenceType::LogFile));

    // Observability artefacts.
    assert_eq!(dashboard_panels().len(), 8);
    assert_eq!(alert_rules().len(), 5);
}
