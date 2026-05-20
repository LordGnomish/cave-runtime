// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Container metrics aggregator — cgroup v2 stat reader + per-pod
//! rollup.
//!
//! Mirrors Kubernetes v1.36.0 upstream:
//!   `pkg/kubelet/cm/cgroup_manager_linux.go`
//!     (`libcontainerCgroupManager.GetStats`).
//!   `vendor/github.com/google/cadvisor/container/libcontainer/handler.go`
//!     (cgroup v2 cpu.stat / memory.stat / io.stat / pids.current parsers).
//!
//! cgroup v2 fields modeled here (the relevant subset cadvisor scrapes):
//!
//!   * `cpu.stat`    — `usage_usec`, `user_usec`, `system_usec`,
//!                     `nr_throttled`, `throttled_usec`
//!   * `memory.stat` — `anon`, `file`, `kernel`, total + working_set
//!   * `io.stat`     — per-device `rbytes` / `wbytes` / `rios` / `wios`
//!   * `pids.current`
//!
//! Pod-level rollup sums each container's stats; tenant scoping is
//! enforced by `pod_uid → tenant_id` mapping the kubelet maintains.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MetricsError {
    #[error("malformed cgroup line: '{0}'")]
    MalformedLine(String),
    #[error("missing field '{0}'")]
    MissingField(String),
    #[error("pod '{0}' not registered with tracker")]
    UnknownPod(Uuid),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CpuStat {
    pub usage_usec: u64,
    pub user_usec: u64,
    pub system_usec: u64,
    pub nr_throttled: u64,
    pub throttled_usec: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryStat {
    pub anon_bytes: u64,
    pub file_bytes: u64,
    pub kernel_bytes: u64,
    pub working_set_bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IoStat {
    pub rbytes: u64,
    pub wbytes: u64,
    pub rios: u64,
    pub wios: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContainerStats {
    pub container_id: String,
    pub cpu: CpuStat,
    pub memory: MemoryStat,
    pub io: IoStat,
    pub pids_current: u64,
}

/// Parse a cgroup v2 `cpu.stat` block.
pub fn parse_cpu_stat(blob: &str) -> Result<CpuStat, MetricsError> {
    let mut s = CpuStat::default();
    for line in blob.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, ' ');
        let key = parts
            .next()
            .ok_or_else(|| MetricsError::MalformedLine(line.into()))?;
        let val_s = parts
            .next()
            .ok_or_else(|| MetricsError::MalformedLine(line.into()))?;
        let val: u64 = val_s
            .parse()
            .map_err(|_| MetricsError::MalformedLine(line.into()))?;
        match key {
            "usage_usec" => s.usage_usec = val,
            "user_usec" => s.user_usec = val,
            "system_usec" => s.system_usec = val,
            "nr_throttled" => s.nr_throttled = val,
            "throttled_usec" => s.throttled_usec = val,
            _ => {}
        }
    }
    if s.usage_usec == 0 && s.user_usec == 0 && s.system_usec == 0 {
        return Err(MetricsError::MissingField("cpu usage".into()));
    }
    Ok(s)
}

/// Parse a cgroup v2 `memory.stat` block. Caller supplies `working_set`
/// separately when computing it from current usage − inactive_file.
pub fn parse_memory_stat(blob: &str, working_set_bytes: u64) -> Result<MemoryStat, MetricsError> {
    let mut anon = None;
    let mut file = None;
    let mut kernel = None;
    for line in blob.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, ' ');
        let key = parts
            .next()
            .ok_or_else(|| MetricsError::MalformedLine(line.into()))?;
        let val_s = parts
            .next()
            .ok_or_else(|| MetricsError::MalformedLine(line.into()))?;
        let val: u64 = val_s
            .parse()
            .map_err(|_| MetricsError::MalformedLine(line.into()))?;
        match key {
            "anon" => anon = Some(val),
            "file" => file = Some(val),
            "kernel" => kernel = Some(val),
            _ => {}
        }
    }
    Ok(MemoryStat {
        anon_bytes: anon.ok_or_else(|| MetricsError::MissingField("anon".into()))?,
        file_bytes: file.ok_or_else(|| MetricsError::MissingField("file".into()))?,
        kernel_bytes: kernel.unwrap_or(0),
        working_set_bytes,
    })
}

/// Parse a single line of `io.stat`. Format:
///   `8:0 rbytes=1048576 wbytes=2097152 rios=128 wios=256 ...`
pub fn parse_io_line(line: &str) -> Result<IoStat, MetricsError> {
    let mut io = IoStat::default();
    let mut parts = line.split_whitespace();
    // First token is the device id; skip.
    parts
        .next()
        .ok_or_else(|| MetricsError::MalformedLine(line.into()))?;
    for kv in parts {
        let mut split = kv.splitn(2, '=');
        let k = split
            .next()
            .ok_or_else(|| MetricsError::MalformedLine(kv.into()))?;
        let v = split
            .next()
            .ok_or_else(|| MetricsError::MalformedLine(kv.into()))?
            .parse::<u64>()
            .map_err(|_| MetricsError::MalformedLine(kv.into()))?;
        match k {
            "rbytes" => io.rbytes = v,
            "wbytes" => io.wbytes = v,
            "rios" => io.rios = v,
            "wios" => io.wios = v,
            _ => {}
        }
    }
    Ok(io)
}

/// Tracker that aggregates per-container stats into per-pod rollups.
#[derive(Debug, Default)]
pub struct ContainerMetricsAggregator {
    pub pod_tenant: BTreeMap<Uuid, String>,
    pub pod_containers: BTreeMap<Uuid, Vec<ContainerStats>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PodRollup {
    pub pod_uid: Uuid,
    pub tenant_id: String,
    pub cpu_usage_usec: u64,
    pub memory_working_set_bytes: u64,
    pub memory_anon_bytes: u64,
    pub io_rbytes: u64,
    pub io_wbytes: u64,
    pub pids_total: u64,
    pub container_count: u32,
}

impl ContainerMetricsAggregator {
    pub fn register_pod(&mut self, pod_uid: Uuid, tenant_id: &str) {
        self.pod_tenant.insert(pod_uid, tenant_id.into());
        self.pod_containers.entry(pod_uid).or_default();
    }

    pub fn record(&mut self, pod_uid: Uuid, stats: ContainerStats) -> Result<(), MetricsError> {
        let entry = self
            .pod_containers
            .get_mut(&pod_uid)
            .ok_or(MetricsError::UnknownPod(pod_uid))?;
        // Replace existing record for the same container_id; otherwise append.
        if let Some(existing) = entry
            .iter_mut()
            .find(|c| c.container_id == stats.container_id)
        {
            *existing = stats;
        } else {
            entry.push(stats);
        }
        Ok(())
    }

    pub fn rollup(&self, pod_uid: Uuid) -> Result<PodRollup, MetricsError> {
        let containers = self
            .pod_containers
            .get(&pod_uid)
            .ok_or(MetricsError::UnknownPod(pod_uid))?;
        let tenant = self
            .pod_tenant
            .get(&pod_uid)
            .cloned()
            .unwrap_or_else(|| "unknown".into());
        let mut r = PodRollup {
            pod_uid,
            tenant_id: tenant,
            container_count: containers.len() as u32,
            ..Default::default()
        };
        for c in containers {
            r.cpu_usage_usec = r.cpu_usage_usec.saturating_add(c.cpu.usage_usec);
            r.memory_working_set_bytes = r
                .memory_working_set_bytes
                .saturating_add(c.memory.working_set_bytes);
            r.memory_anon_bytes = r.memory_anon_bytes.saturating_add(c.memory.anon_bytes);
            r.io_rbytes = r.io_rbytes.saturating_add(c.io.rbytes);
            r.io_wbytes = r.io_wbytes.saturating_add(c.io.wbytes);
            r.pids_total = r.pids_total.saturating_add(c.pids_current);
        }
        Ok(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cpu_stat_full_block() {
        let blob =
            "usage_usec 1000\nuser_usec 700\nsystem_usec 300\nnr_throttled 5\nthrottled_usec 100\n";
        let s = parse_cpu_stat(blob).unwrap();
        assert_eq!(s.usage_usec, 1000);
        assert_eq!(s.user_usec, 700);
        assert_eq!(s.system_usec, 300);
        assert_eq!(s.nr_throttled, 5);
        assert_eq!(s.throttled_usec, 100);
    }

    #[test]
    fn parse_cpu_stat_empty_errors() {
        let s = parse_cpu_stat("");
        assert!(matches!(s, Err(MetricsError::MissingField(_))));
    }

    #[test]
    fn parse_cpu_stat_malformed_errors() {
        let s = parse_cpu_stat("usage_usec abc");
        assert!(matches!(s, Err(MetricsError::MalformedLine(_))));
    }

    #[test]
    fn parse_memory_stat_required_fields() {
        let blob = "anon 4096\nfile 8192\nkernel 1024\nshmem 0\n";
        let m = parse_memory_stat(blob, 12000).unwrap();
        assert_eq!(m.anon_bytes, 4096);
        assert_eq!(m.file_bytes, 8192);
        assert_eq!(m.kernel_bytes, 1024);
        assert_eq!(m.working_set_bytes, 12000);
    }

    #[test]
    fn parse_memory_stat_missing_anon_errors() {
        let blob = "file 8192\n";
        let r = parse_memory_stat(blob, 0);
        assert!(matches!(r, Err(MetricsError::MissingField(_))));
    }

    #[test]
    fn parse_io_line_extracts_counters() {
        let line = "8:0 rbytes=1048576 wbytes=2097152 rios=128 wios=256 dbytes=0 dios=0";
        let io = parse_io_line(line).unwrap();
        assert_eq!(io.rbytes, 1048576);
        assert_eq!(io.wbytes, 2097152);
        assert_eq!(io.rios, 128);
        assert_eq!(io.wios, 256);
    }

    #[test]
    fn parse_io_line_malformed_kv_errors() {
        let r = parse_io_line("8:0 rbytes=abc");
        assert!(matches!(r, Err(MetricsError::MalformedLine(_))));
    }

    #[test]
    fn aggregator_record_unknown_pod_errors() {
        let mut a = ContainerMetricsAggregator::default();
        let pod = Uuid::new_v4();
        let stats = ContainerStats {
            container_id: "c1".into(),
            ..Default::default()
        };
        assert!(matches!(
            a.record(pod, stats),
            Err(MetricsError::UnknownPod(_))
        ));
    }

    #[test]
    fn aggregator_replace_same_container_id() {
        let mut a = ContainerMetricsAggregator::default();
        let pod = Uuid::new_v4();
        a.register_pod(pod, "acme");
        a.record(
            pod,
            ContainerStats {
                container_id: "c1".into(),
                cpu: CpuStat {
                    usage_usec: 100,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();
        a.record(
            pod,
            ContainerStats {
                container_id: "c1".into(),
                cpu: CpuStat {
                    usage_usec: 250,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();
        let r = a.rollup(pod).unwrap();
        assert_eq!(r.container_count, 1);
        assert_eq!(r.cpu_usage_usec, 250);
    }

    #[test]
    fn aggregator_rollup_sums_multi_container() {
        let mut a = ContainerMetricsAggregator::default();
        let pod = Uuid::new_v4();
        a.register_pod(pod, "acme");
        a.record(
            pod,
            ContainerStats {
                container_id: "c1".into(),
                cpu: CpuStat {
                    usage_usec: 100,
                    ..Default::default()
                },
                memory: MemoryStat {
                    working_set_bytes: 1000,
                    anon_bytes: 800,
                    ..Default::default()
                },
                io: IoStat {
                    rbytes: 5,
                    wbytes: 6,
                    ..Default::default()
                },
                pids_current: 10,
            },
        )
        .unwrap();
        a.record(
            pod,
            ContainerStats {
                container_id: "c2".into(),
                cpu: CpuStat {
                    usage_usec: 200,
                    ..Default::default()
                },
                memory: MemoryStat {
                    working_set_bytes: 2000,
                    anon_bytes: 1500,
                    ..Default::default()
                },
                io: IoStat {
                    rbytes: 7,
                    wbytes: 8,
                    ..Default::default()
                },
                pids_current: 20,
            },
        )
        .unwrap();
        let r = a.rollup(pod).unwrap();
        assert_eq!(r.tenant_id, "acme");
        assert_eq!(r.container_count, 2);
        assert_eq!(r.cpu_usage_usec, 300);
        assert_eq!(r.memory_working_set_bytes, 3000);
        assert_eq!(r.memory_anon_bytes, 2300);
        assert_eq!(r.io_rbytes, 12);
        assert_eq!(r.io_wbytes, 14);
        assert_eq!(r.pids_total, 30);
    }

    #[test]
    fn aggregator_rollup_unknown_pod_errors() {
        let a = ContainerMetricsAggregator::default();
        assert!(matches!(
            a.rollup(Uuid::new_v4()),
            Err(MetricsError::UnknownPod(_))
        ));
    }

    #[test]
    fn rollup_isolated_per_pod() {
        let mut a = ContainerMetricsAggregator::default();
        let p1 = Uuid::new_v4();
        let p2 = Uuid::new_v4();
        a.register_pod(p1, "acme");
        a.register_pod(p2, "rival");
        a.record(
            p1,
            ContainerStats {
                container_id: "c".into(),
                cpu: CpuStat {
                    usage_usec: 50,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();
        a.record(
            p2,
            ContainerStats {
                container_id: "c".into(),
                cpu: CpuStat {
                    usage_usec: 999,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();
        let r1 = a.rollup(p1).unwrap();
        let r2 = a.rollup(p2).unwrap();
        assert_eq!(r1.cpu_usage_usec, 50);
        assert_eq!(r2.cpu_usage_usec, 999);
        assert_eq!(r1.tenant_id, "acme");
        assert_eq!(r2.tenant_id, "rival");
    }
}
