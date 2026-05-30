// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Chaos-daemon fault-spec generation — Chaos Mesh `chaos-daemon` port (planning
//! layer only).
//!
//! Pure functions that build the exact `tc`/`netem`, `tc`/`tbf`, `iptables` and
//! `stress-ng` command strings the daemon would run, plus [`injection_plan`]
//! which maps a [`ChaosExperiment`] to its command list. The privileged kernel
//! and namespace execution (nsenter, cgroups, real `tc` syscalls) is an honest
//! scope-cut — it requires a privileged DaemonSet per node and is out of process
//! for this in-process runtime. What is portable, and what this module covers,
//! is the deterministic command planning.

use crate::models::{ChaosExperiment, ExperimentType};

/// A `tc netem` fault and its tunables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetemFault {
    Delay { ms: u32, jitter_ms: Option<u32>, correlation: Option<u8> },
    Loss { percent: u8, correlation: Option<u8> },
    Corrupt { percent: u8, correlation: Option<u8> },
    Duplicate { percent: u8, correlation: Option<u8> },
}

/// Build a `tc qdisc add ... root netem ...` command for a netem fault.
pub fn tc_netem(dev: &str, fault: &NetemFault) -> String {
    let prefix = format!("tc qdisc add dev {dev} root netem");
    match fault {
        NetemFault::Delay { ms, jitter_ms, correlation } => {
            let mut s = format!("{prefix} delay {ms}ms");
            if let Some(j) = jitter_ms {
                s.push_str(&format!(" {j}ms"));
                if let Some(c) = correlation {
                    s.push_str(&format!(" {c}"));
                }
            }
            s
        }
        NetemFault::Loss { percent, correlation } => with_corr(&prefix, "loss", *percent, *correlation),
        NetemFault::Corrupt { percent, correlation } => with_corr(&prefix, "corrupt", *percent, *correlation),
        NetemFault::Duplicate { percent, correlation } => with_corr(&prefix, "duplicate", *percent, *correlation),
    }
}

fn with_corr(prefix: &str, kw: &str, percent: u8, correlation: Option<u8>) -> String {
    let mut s = format!("{prefix} {kw} {percent}");
    if let Some(c) = correlation {
        s.push_str(&format!(" {c}"));
    }
    s
}

/// Build a `tc qdisc add ... root tbf ...` bandwidth-throttle command.
pub fn tc_tbf(dev: &str, rate: &str, burst: &str, latency_ms: u32) -> String {
    format!("tc qdisc add dev {dev} root tbf rate {rate} burst {burst} latency {latency_ms}ms")
}

/// iptables chain direction for a network partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Input,
    Output,
}

/// Build an `iptables` DROP rule that partitions traffic matching an ipset.
pub fn iptables_partition(direction: Direction, ipset: &str) -> String {
    let (chain, md) = match direction {
        Direction::Input => ("CHAOS-INPUT", "src"),
        Direction::Output => ("CHAOS-OUTPUT", "dst"),
    };
    format!("iptables -w -A {chain} -m set --match-set {ipset} {md} -j DROP -w 5")
}

/// Build a `stress-ng` CPU-stress command (`--cpu` flags first, per daemon order).
pub fn stress_ng_cpu(workers: u32, load: Option<u8>) -> String {
    let mut s = format!("stress-ng --cpu-load-slice 10 --cpu-method sqrt --cpu {workers}");
    if let Some(l) = load {
        s.push_str(&format!(" --cpu-load {l}"));
    }
    s
}

/// Build a `stress-ng` memory-stress command.
pub fn stress_ng_mem(workers: u32, size: Option<&str>) -> String {
    let mut s = format!("stress-ng --workers {workers}");
    if let Some(sz) = size {
        s.push_str(&format!(" --size {sz}"));
    }
    s
}

/// DNSChaos action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnsAction {
    /// Return SERVFAIL for matching queries.
    Error,
    /// Return a random IP for matching queries.
    Random,
}

/// Build the chaos-dns-server reconfiguration command for a DNS fault.
pub fn dns_chaos_command(action: DnsAction, patterns: &[String]) -> String {
    let a = match action {
        DnsAction::Error => "error",
        DnsAction::Random => "random",
    };
    format!("chaos-dns-server --action {a} --patterns {}", patterns.join(","))
}

/// Map an experiment to the ordered list of daemon commands that would inject it.
///
/// Returns an empty list for fault types that have no daemon-command
/// representation in this planning layer (e.g. PodKill is an API-server op,
/// not a `tc`/`iptables`/`stress-ng` command).
pub fn injection_plan(exp: &ChaosExperiment, dev: &str) -> Vec<String> {
    let p = &exp.parameters;
    match exp.experiment_type {
        ExperimentType::NetworkLatency => p
            .latency_ms
            .map(|ms| vec![tc_netem(dev, &NetemFault::Delay { ms, jitter_ms: None, correlation: None })])
            .unwrap_or_default(),
        ExperimentType::NetworkPacketLoss => p
            .packet_loss_percent
            .map(|pct| vec![tc_netem(dev, &NetemFault::Loss { percent: pct as u8, correlation: None })])
            .unwrap_or_default(),
        ExperimentType::NetworkCorruption => p
            .packet_loss_percent
            .map(|pct| vec![tc_netem(dev, &NetemFault::Corrupt { percent: pct as u8, correlation: None })])
            .unwrap_or_default(),
        ExperimentType::NetworkPartition => {
            vec![iptables_partition(Direction::Output, "chaos_blocked")]
        }
        ExperimentType::CpuStress => {
            vec![stress_ng_cpu(1, p.cpu_load_percent)]
        }
        ExperimentType::MemoryStress => p
            .memory_mb
            .map(|mb| vec![stress_ng_mem(1, Some(&format!("{mb}MB")))])
            .unwrap_or_default(),
        ExperimentType::IoLatency => p
            .latency_ms
            .map(|ms| vec![tc_netem(dev, &NetemFault::Delay { ms, jitter_ms: None, correlation: None })])
            .unwrap_or_default(),
        _ => vec![],
    }
}
