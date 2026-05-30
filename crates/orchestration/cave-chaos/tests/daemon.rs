// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD: chaos-daemon fault-spec generation. PURE functions that produce
//! the tc/netem, tc/tbf, iptables and stress-ng command strings a privileged
//! daemon would execute. The privileged kernel/namespace execution itself stays
//! an honest scope-cut; this is the portable command-planning layer.

use cave_chaos::daemon::{
    dns_chaos_command, injection_plan, iptables_partition, stress_ng_cpu, stress_ng_mem, tc_netem,
    tc_tbf, Direction, DnsAction, NetemFault,
};
use cave_chaos::models::{
    BlastRadius, ChaosExperiment, ChaosTarget, ExperimentParams, ExperimentStatus, ExperimentType,
    SafetyGuard,
};
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

// ── tc netem ────────────────────────────────────────────────────────────────

#[test]
fn test_netem_delay_basic() {
    assert_eq!(
        tc_netem("eth0", &NetemFault::Delay { ms: 100, jitter_ms: None, correlation: None }),
        "tc qdisc add dev eth0 root netem delay 100ms"
    );
}

#[test]
fn test_netem_delay_with_jitter_and_correlation() {
    assert_eq!(
        tc_netem(
            "eth0",
            &NetemFault::Delay { ms: 50, jitter_ms: Some(10), correlation: Some(25) }
        ),
        "tc qdisc add dev eth0 root netem delay 50ms 10ms 25"
    );
}

#[test]
fn test_netem_loss() {
    assert_eq!(
        tc_netem("eth0", &NetemFault::Loss { percent: 10, correlation: None }),
        "tc qdisc add dev eth0 root netem loss 10"
    );
}

#[test]
fn test_netem_corrupt_with_correlation() {
    assert_eq!(
        tc_netem("eth0", &NetemFault::Corrupt { percent: 5, correlation: Some(50) }),
        "tc qdisc add dev eth0 root netem corrupt 5 50"
    );
}

#[test]
fn test_netem_duplicate() {
    assert_eq!(
        tc_netem("eth0", &NetemFault::Duplicate { percent: 2, correlation: None }),
        "tc qdisc add dev eth0 root netem duplicate 2"
    );
}

// ── tc tbf (bandwidth) ──────────────────────────────────────────────────────

#[test]
fn test_tbf_bandwidth() {
    assert_eq!(
        tc_tbf("eth0", "1mbit", "32kbit", 400),
        "tc qdisc add dev eth0 root tbf rate 1mbit burst 32kbit latency 400ms"
    );
}

// ── iptables partition ──────────────────────────────────────────────────────

#[test]
fn test_iptables_partition_output() {
    assert_eq!(
        iptables_partition(Direction::Output, "blocked_ips"),
        "iptables -w -A CHAOS-OUTPUT -m set --match-set blocked_ips dst -j DROP -w 5"
    );
}

#[test]
fn test_iptables_partition_input_uses_input_chain_and_src() {
    let rule = iptables_partition(Direction::Input, "blocked_ips");
    assert!(rule.contains("CHAOS-INPUT"), "got: {rule}");
    assert!(rule.contains("--match-set blocked_ips src"), "got: {rule}");
    assert!(rule.contains("-j DROP"));
}

// ── stress-ng ───────────────────────────────────────────────────────────────

#[test]
fn test_stress_ng_cpu_with_load() {
    assert_eq!(
        stress_ng_cpu(4, Some(75)),
        "stress-ng --cpu-load-slice 10 --cpu-method sqrt --cpu 4 --cpu-load 75"
    );
}

#[test]
fn test_stress_ng_cpu_without_load() {
    assert_eq!(
        stress_ng_cpu(4, None),
        "stress-ng --cpu-load-slice 10 --cpu-method sqrt --cpu 4"
    );
}

#[test]
fn test_stress_ng_mem_with_size() {
    assert_eq!(stress_ng_mem(2, Some("4GB")), "stress-ng --workers 2 --size 4GB");
}

#[test]
fn test_stress_ng_mem_without_size() {
    assert_eq!(stress_ng_mem(2, None), "stress-ng --workers 2");
}

// ── DNS chaos ───────────────────────────────────────────────────────────────

#[test]
fn test_dns_chaos_error_action() {
    let cmd = dns_chaos_command(DnsAction::Error, &["*.example.com".to_string()]);
    assert!(cmd.contains("error"), "got: {cmd}");
    assert!(cmd.contains("*.example.com"), "got: {cmd}");
}

#[test]
fn test_dns_chaos_random_action() {
    let cmd = dns_chaos_command(DnsAction::Random, &["*".to_string()]);
    assert!(cmd.contains("random"), "got: {cmd}");
}

// ── injection_plan: experiment -> command list ──────────────────────────────

fn experiment(t: ExperimentType, params: ExperimentParams) -> ChaosExperiment {
    ChaosExperiment {
        id: Uuid::new_v4(),
        name: "plan".to_string(),
        experiment_type: t,
        target: ChaosTarget { namespace: "staging".to_string(), selector: HashMap::new(), pod_count: None },
        parameters: params,
        status: ExperimentStatus::Draft,
        created_at: Utc::now(),
        started_at: None,
        ended_at: None,
        duration_secs: 60,
        blast_radius: BlastRadius::default(),
        safety_guard: SafetyGuard::default(),
        result: None,
        annotations: HashMap::new(),
    }
}

#[test]
fn test_injection_plan_network_latency_emits_netem_delay() {
    let exp = experiment(
        ExperimentType::NetworkLatency,
        ExperimentParams { latency_ms: Some(120), packet_loss_percent: None, cpu_load_percent: None, memory_mb: None },
    );
    let plan = injection_plan(&exp, "eth0");
    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0], "tc qdisc add dev eth0 root netem delay 120ms");
}

#[test]
fn test_injection_plan_cpu_stress_emits_stress_ng() {
    let exp = experiment(
        ExperimentType::CpuStress,
        ExperimentParams { latency_ms: None, packet_loss_percent: None, cpu_load_percent: Some(80), memory_mb: None },
    );
    let plan = injection_plan(&exp, "eth0");
    assert_eq!(plan.len(), 1);
    assert!(plan[0].starts_with("stress-ng --cpu-load-slice 10 --cpu-method sqrt --cpu"));
    assert!(plan[0].contains("--cpu-load 80"));
}

#[test]
fn test_injection_plan_packet_loss_emits_netem_loss() {
    let exp = experiment(
        ExperimentType::NetworkPacketLoss,
        ExperimentParams { latency_ms: None, packet_loss_percent: Some(10.0), cpu_load_percent: None, memory_mb: None },
    );
    let plan = injection_plan(&exp, "eth0");
    assert_eq!(plan, vec!["tc qdisc add dev eth0 root netem loss 10".to_string()]);
}
