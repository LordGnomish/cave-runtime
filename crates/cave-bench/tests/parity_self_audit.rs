// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Charter v2 8-gate self-audit — cave-bench must carry an honest,
//! measured `fill_ratio` against dual upstream kube-bench v0.15.5 +
//! kubescape v4.0.8 with pinned `source_sha`, today's `last_audit`,
//! 100% AGPL SPDX header coverage, no stub macros in src/, and
//! mapped+partial+skipped+unmapped summing to total.

use cave_bench::parity_self_audit as a;
use std::path::PathBuf;

// ─── Assertion 1: G1 — both upstreams pinned ────────────────────────────────

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
    assert!(h >= a::FLOOR_HONEST_RATIO, "honest_ratio {h} >= {}", a::FLOOR_HONEST_RATIO);

    let recorded: f64 = a::extract_scalar(&m, "fill_ratio")
        .expect("fill_ratio in [parity]")
        .parse()
        .expect("fill_ratio is a number");
    assert!(
        (recorded - r).abs() < 0.005,
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
        "expected >= 15 .rs files in cave-bench; got {total}"
    );
}

// ─── Assertion 8: G8 — no stub macros in src/ ───────────────────────────────

#[test]
fn assertion_8_gate_8_no_stub_macros() {
    let src: PathBuf = [env!("CARGO_MANIFEST_DIR"), "src"].iter().collect();
    a::gate_8_no_stub_macros(&src).expect("G8 no stubs");
}

// ─── Assertion 9: Bonus — full bench surface reachable + last_audit ─────────

#[test]
fn assertion_9_surface_integrity_and_audit_date() {
    use cave_bench::cis_engine::{BinOp, CisContext, CisRule, Logic, TestItem, ValueSource, evaluate_rule};
    use cave_bench::cis_master::master_checks;
    use cave_bench::cis_node::node_checks;
    use cave_bench::cis_etcd::etcd_checks;
    use cave_bench::cis_control_plane::control_plane_checks;
    use cave_bench::kubescape_nsa::{NsaManifestFacts, evaluate_control, nsa_controls};
    use cave_bench::kubescape_mitre::{Tactic, group_by_tactic, mitre_techniques};
    use cave_bench::profile::{builtin_profiles, find_profile};
    use cave_bench::runner::{RunMode, ScanInput, cis_pairs, run_profile, smoke_run};
    use cave_bench::report::{Format, render};
    use cave_bench::scheduler::{NotifyAction, ScheduleRegistry, ScheduledScan, default_dag};
    use cave_bench::store::FindingStore;
    use cave_bench::observability::{alert_rules, alert_rules_yaml, dashboard_panels};
    use cave_bench::cli::{BenchSubcommand, dispatch};
    use cave_bench::models::{Check, CisLevel, Finding, Framework, NodeType, Severity, Target, Verdict};
    use cave_bench::{MODULE_NAME, State, router};
    use std::sync::Arc;

    let m = a::manifest_text();
    assert_eq!(
        a::extract_scalar(&m, "last_audit").as_deref(),
        Some(a::TODAY),
        "last_audit must equal {}",
        a::TODAY
    );

    // Crate-root identity + router.
    assert_eq!(MODULE_NAME, "bench");
    let _r = router(Arc::new(State::default()));

    // CIS engine round-trip.
    let mut ctx = CisContext::default();
    ctx.set_flag("apiserver", "--anonymous-auth", "false");
    let mut rule = CisRule::new("cis-1.2.1", "Disable anonymous auth");
    rule.logic = Logic::And;
    rule.items.push(TestItem {
        source: ValueSource::Flag("--anonymous-auth".into()),
        op: BinOp::Eq,
        value: "false".into(),
        set: Some(true),
    });
    let meta = Check::new("cis-1.2.1", Framework::CisK8s, NodeType::Master, "x");
    let f = evaluate_rule(&rule, &meta, &ctx, "apiserver", "n1");
    assert_eq!(f.verdict, Verdict::Pass);

    // All 4 CIS catalogues populated.
    assert!(master_checks().len() >= 20);
    assert!(node_checks().len() >= 10);
    assert!(etcd_checks().len() >= 8);
    assert!(control_plane_checks().len() >= 6);

    // NSA control evaluator wired.
    let nsa = nsa_controls();
    assert!(nsa.len() >= 20);
    let facts = NsaManifestFacts::default();
    let f = evaluate_control(&nsa[0], &facts, "h");
    assert!(matches!(f.verdict, Verdict::Pass | Verdict::Fail | Verdict::Warn));

    // MITRE 10 tactics × ≥3 techniques.
    let techs = mitre_techniques();
    let groups = group_by_tactic(&techs);
    assert!(groups.len() >= 9);
    for (_, v) in &groups {
        assert!(v.len() >= 3);
    }
    assert!(groups.contains_key(&Tactic::Impact));

    // Profiles + runner.
    assert!(builtin_profiles().len() >= 4);
    let p = find_profile("cis-1.10").unwrap();
    let t = Target::host_files("/etc/kubernetes", "n1");
    let mut input = ScanInput::new("n1");
    input.cis_context.set_flag("apiserver", "--anonymous-auth", "false");
    let (findings, summary) = run_profile(&p, &t, &input, RunMode::Sequential);
    assert!(!findings.is_empty());
    assert_eq!(summary.profile_id, "cis-1.10");
    assert!(cis_pairs().len() >= 50);
    let n = smoke_run("cis-1.10").unwrap();
    assert!(n > 0);

    // Report renderers.
    for fmt in [Format::Json, Format::Sarif, Format::Html, Format::Markdown] {
        let out = render(fmt, &findings, &summary);
        assert!(!out.is_empty());
    }

    // Scheduler + store.
    let reg = ScheduleRegistry::default();
    reg.add(ScheduledScan::new("s1", "cis-1.10", "0 2 * * *"));
    assert_eq!(reg.due_at(0, 2).len(), 1);
    let _ = NotifyAction::Slack("#ops".into());
    assert_eq!(default_dag().len(), 6);
    let store = FindingStore::new();
    store.record(summary, findings);
    assert_eq!(store.count(), 1);

    // Observability + CLI.
    assert_eq!(dashboard_panels().len(), 8);
    assert_eq!(alert_rules().len(), 5);
    assert!(alert_rules_yaml().contains("alert:"));
    let cli_out = dispatch(BenchSubcommand::Profiles).unwrap();
    assert!(cli_out.contains("cis-1.10"));

    // Verdict + severity types exposed.
    let _ = (Verdict::Pass, Severity::Critical, CisLevel::L1, Finding::pass(&meta, "h", "ok"));
}
