// SPDX-License-Identifier: AGPL-3.0-or-later
//! Observability surface — metric names + dashboard panels + alert rules.
//!
//! The declarative version (panel/alert TOML) lives at `observability.toml`
//! at the crate root for orchestrator consumption.

use serde::{Deserialize, Serialize};

pub const METRIC_SANDBOX_CREATE_TOTAL: &str = "cave_sandbox_create_total";
pub const METRIC_SANDBOX_RUNNING: &str = "cave_sandbox_running";
pub const METRIC_HYPERVISOR_CPU_USAGE: &str = "cave_sandbox_hypervisor_cpu_usage";
pub const METRIC_HYPERVISOR_MEM_USAGE: &str = "cave_sandbox_hypervisor_mem_usage_bytes";
pub const METRIC_GVISOR_SYSCALL_DENY_TOTAL: &str = "cave_sandbox_gvisor_syscall_deny_total";
pub const METRIC_KATA_SHIM_CRASHES_TOTAL: &str = "cave_sandbox_kata_shim_crashes_total";
pub const METRIC_FC_JAILER_DENIES_TOTAL: &str = "cave_sandbox_firecracker_jailer_denies_total";
pub const METRIC_SANDBOX_UPTIME_SECONDS: &str = "cave_sandbox_uptime_seconds";
pub const METRIC_OCI_BUNDLE_BYTES: &str = "cave_sandbox_oci_bundle_bytes";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Panel {
    pub title: String,
    pub metric: String,
    pub query: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlertRule {
    pub name: String,
    pub expr: String,
    pub for_seconds: u64,
    pub severity: String,
    pub summary: String,
}

pub fn dashboard_panels() -> Vec<Panel> {
    vec![
        Panel {
            title: "Sandbox create rate".into(),
            metric: METRIC_SANDBOX_CREATE_TOTAL.into(),
            query: format!("sum by (runtime) (rate({}[1m]))", METRIC_SANDBOX_CREATE_TOTAL),
        },
        Panel {
            title: "Running sandboxes by runtime".into(),
            metric: METRIC_SANDBOX_RUNNING.into(),
            query: format!("sum by (runtime) ({})", METRIC_SANDBOX_RUNNING),
        },
        Panel {
            title: "Hypervisor CPU usage (per-VM)".into(),
            metric: METRIC_HYPERVISOR_CPU_USAGE.into(),
            query: format!("avg by (vmid) ({})", METRIC_HYPERVISOR_CPU_USAGE),
        },
        Panel {
            title: "Hypervisor memory usage".into(),
            metric: METRIC_HYPERVISOR_MEM_USAGE.into(),
            query: format!("sum by (vmid) ({})", METRIC_HYPERVISOR_MEM_USAGE),
        },
        Panel {
            title: "gVisor syscall denies / second".into(),
            metric: METRIC_GVISOR_SYSCALL_DENY_TOTAL.into(),
            query: format!("sum by (syscall) (rate({}[1m]))", METRIC_GVISOR_SYSCALL_DENY_TOTAL),
        },
        Panel {
            title: "Kata shim crashes / hour".into(),
            metric: METRIC_KATA_SHIM_CRASHES_TOTAL.into(),
            query: format!("increase({}[1h])", METRIC_KATA_SHIM_CRASHES_TOTAL),
        },
        Panel {
            title: "Firecracker jailer denials".into(),
            metric: METRIC_FC_JAILER_DENIES_TOTAL.into(),
            query: format!("rate({}[5m])", METRIC_FC_JAILER_DENIES_TOTAL),
        },
        Panel {
            title: "Sandbox uptime distribution".into(),
            metric: METRIC_SANDBOX_UPTIME_SECONDS.into(),
            query: format!("histogram_quantile(0.95, {})", METRIC_SANDBOX_UPTIME_SECONDS),
        },
        Panel {
            title: "OCI bundle size".into(),
            metric: METRIC_OCI_BUNDLE_BYTES.into(),
            query: format!("avg ({})", METRIC_OCI_BUNDLE_BYTES),
        },
    ]
}

pub fn alert_rules() -> Vec<AlertRule> {
    vec![
        AlertRule {
            name: "SandboxCreateFailing".into(),
            expr: format!("rate({}{{result=\"error\"}}[5m]) > 0.1", METRIC_SANDBOX_CREATE_TOTAL),
            for_seconds: 300,
            severity: "warning".into(),
            summary: "cave-sandbox create error rate > 0.1/s for 5m".into(),
        },
        AlertRule {
            name: "GvisorSyscallDenialsSurge".into(),
            expr: format!("sum(rate({}[5m])) > 100", METRIC_GVISOR_SYSCALL_DENY_TOTAL),
            for_seconds: 600,
            severity: "warning".into(),
            summary: "gVisor denying > 100 syscalls/s — possible escape attempt".into(),
        },
        AlertRule {
            name: "KataShimCrashLoop".into(),
            expr: format!("increase({}[5m]) > 3", METRIC_KATA_SHIM_CRASHES_TOTAL),
            for_seconds: 300,
            severity: "critical".into(),
            summary: "Kata shim crashing > 3 times in 5m".into(),
        },
        AlertRule {
            name: "FirecrackerJailerDenials".into(),
            expr: format!("rate({}[5m]) > 0", METRIC_FC_JAILER_DENIES_TOTAL),
            for_seconds: 60,
            severity: "warning".into(),
            summary: "Firecracker jailer denying syscalls (seccomp violation)".into(),
        },
        AlertRule {
            name: "HypervisorMemoryPressure".into(),
            expr: format!("avg({}) > 8e9", METRIC_HYPERVISOR_MEM_USAGE),
            for_seconds: 600,
            severity: "warning".into(),
            summary: "Hypervisor average memory > 8GiB — review limits".into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nine_panels() {
        assert_eq!(dashboard_panels().len(), 9);
    }

    #[test]
    fn five_alerts() {
        assert_eq!(alert_rules().len(), 5);
    }

    #[test]
    fn panels_metric_names_canonical() {
        for p in dashboard_panels() {
            assert!(p.metric.starts_with("cave_sandbox_"));
            assert!(p.query.contains(&p.metric));
        }
    }

    #[test]
    fn alerts_have_severity() {
        for a in alert_rules() {
            assert!(["warning", "critical", "info"].contains(&a.severity.as_str()));
            assert!(!a.expr.is_empty());
        }
    }

    #[test]
    fn metrics_unique() {
        let names = [
            METRIC_SANDBOX_CREATE_TOTAL, METRIC_SANDBOX_RUNNING, METRIC_HYPERVISOR_CPU_USAGE,
            METRIC_HYPERVISOR_MEM_USAGE, METRIC_GVISOR_SYSCALL_DENY_TOTAL,
            METRIC_KATA_SHIM_CRASHES_TOTAL, METRIC_FC_JAILER_DENIES_TOTAL,
            METRIC_SANDBOX_UPTIME_SECONDS, METRIC_OCI_BUNDLE_BYTES,
        ];
        let set: std::collections::BTreeSet<&str> = names.iter().copied().collect();
        assert_eq!(set.len(), names.len());
    }
}
