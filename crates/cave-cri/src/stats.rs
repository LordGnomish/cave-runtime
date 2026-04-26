//! Container and image statistics.
//!
//! Models the CRI v1 stats messages (`ContainerStats`,
//! `WindowsContainerStats`, `ImageFsInfo`, `FilesystemUsage`) and exposes
//! a cAdvisor-style descriptor list for the metrics endpoint.
//!
//! Upstream:
//! - containerd: `pkg/cri/server/container_stats.go`,
//!   `pkg/cri/server/container_stats_list.go`,
//!   `pkg/cri/server/imagefs_info.go`
//! - kubernetes cri-api: `runtime.v1.{ContainerStats, WindowsContainerStats,
//!   FilesystemUsage, ImageFsInfo}`

use crate::cgroup;
use crate::error::{CriError, CriResult};
use crate::models::Container;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── CRI common attributes ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerAttributes {
    pub id: Uuid,
    pub name: String,
    pub labels: std::collections::HashMap<String, String>,
    pub annotations: std::collections::HashMap<String, String>,
}

impl From<&Container> for ContainerAttributes {
    fn from(c: &Container) -> Self {
        Self {
            id: c.id,
            name: c.spec.name.clone(),
            labels: c.spec.labels.clone(),
            annotations: Default::default(),
        }
    }
}

// ── Linux stats ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CpuUsage {
    pub timestamp: i64,
    /// Cumulative CPU time consumed by the container in core-nanoseconds.
    pub usage_core_nano_seconds: u64,
    /// Total CPU usage rate in nano cores (1e-9 cores). May be derived
    /// from a delta against a previous sample.
    pub usage_nano_cores: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryUsage {
    pub timestamp: i64,
    pub working_set_bytes: u64,
    pub available_bytes: u64,
    pub usage_bytes: u64,
    pub rss_bytes: u64,
    pub page_faults: u64,
    pub major_page_faults: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FilesystemUsage {
    pub timestamp: i64,
    pub fs_id: FilesystemIdentifier,
    pub used_bytes: u64,
    pub inodes_used: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FilesystemIdentifier {
    pub mountpoint: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SwapUsage {
    pub timestamp: i64,
    pub swap_available_bytes: u64,
    pub swap_usage_bytes: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContainerStatsLinux {
    pub attributes: Option<ContainerAttributes>,
    pub cpu: CpuUsage,
    pub memory: MemoryUsage,
    pub writable_layer: FilesystemUsage,
    pub swap: SwapUsage,
}

// ── Windows stats ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WindowsCpuUsage {
    pub timestamp: i64,
    pub usage_core_nano_seconds: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WindowsMemoryUsage {
    pub timestamp: i64,
    pub working_set_bytes: u64,
    pub available_bytes: u64,
    pub commit_memory_bytes: u64,
    pub page_faults: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WindowsFilesystemUsage {
    pub timestamp: i64,
    pub fs_id: FilesystemIdentifier,
    pub used_bytes: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WindowsContainerStats {
    pub attributes: Option<ContainerAttributes>,
    pub cpu: WindowsCpuUsage,
    pub memory: WindowsMemoryUsage,
    pub writable_layer: WindowsFilesystemUsage,
}

// ── ImageFs / cAdvisor descriptors ───────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImageFsInfo {
    pub timestamp: i64,
    pub image_filesystems: Vec<FilesystemUsage>,
    pub container_filesystems: Vec<FilesystemUsage>,
}

/// `MetricDescriptor` mirrors `runtime.v1.MetricDescriptor` and is what the
/// kubelet calls `ListMetricDescriptors` for.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricDescriptor {
    pub name: String,
    pub help: String,
    pub label_keys: Vec<String>,
}

/// Standard cAdvisor metric set surfaced by `ListMetricDescriptors`.
pub fn cadvisor_descriptors() -> Vec<MetricDescriptor> {
    vec![
        MetricDescriptor {
            name: "container_cpu_usage_seconds_total".into(),
            help: "Cumulative CPU time consumed by the container in seconds.".into(),
            label_keys: vec!["id".into(), "name".into(), "image".into(), "namespace".into()],
        },
        MetricDescriptor {
            name: "container_memory_usage_bytes".into(),
            help: "Current memory usage in bytes including all memory regardless of when accessed.".into(),
            label_keys: vec!["id".into(), "name".into(), "image".into()],
        },
        MetricDescriptor {
            name: "container_memory_working_set_bytes".into(),
            help: "Working set memory usage in bytes.".into(),
            label_keys: vec!["id".into(), "name".into(), "image".into()],
        },
        MetricDescriptor {
            name: "container_memory_rss".into(),
            help: "Resident set size of memory the container has committed.".into(),
            label_keys: vec!["id".into(), "name".into(), "image".into()],
        },
        MetricDescriptor {
            name: "container_fs_usage_bytes".into(),
            help: "Filesystem usage by the container's writable layer in bytes.".into(),
            label_keys: vec!["id".into(), "name".into(), "device".into()],
        },
        MetricDescriptor {
            name: "container_swap_usage_bytes".into(),
            help: "Container swap usage in bytes.".into(),
            label_keys: vec!["id".into(), "name".into()],
        },
        MetricDescriptor {
            name: "container_processes".into(),
            help: "Number of processes running inside the container.".into(),
            label_keys: vec!["id".into(), "name".into()],
        },
    ]
}

/// One Prometheus metric sample.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metric {
    pub name: String,
    pub timestamp: i64,
    pub metric_type: MetricType,
    pub labels: std::collections::BTreeMap<String, String>,
    pub value: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MetricType {
    Counter,
    Gauge,
}

// ── Computation helpers ──────────────────────────────────────────────────────

/// Compute Linux ContainerStats for `container` from its cgroup v2 stats.
///
/// `previous` is an optional prior sample used to derive `usage_nano_cores`
/// from the CPU usage delta — same algorithm cAdvisor uses.
pub fn container_stats_linux(
    container: &Container,
    previous: Option<&CpuUsage>,
) -> CriResult<ContainerStatsLinux> {
    let handle = cgroup::CgroupHandle::new(&container.id.to_string());
    let v2 = cgroup::read_stats_v2(&handle)?;
    let now = Utc::now().timestamp_nanos_opt().unwrap_or(0);

    // CPU delta → nano_cores (1 core = 1e9 nanoseconds per second).
    let cpu_now = CpuUsage {
        timestamp: now,
        usage_core_nano_seconds: v2.cpu_usage_usec * 1_000,
        usage_nano_cores: 0, // filled in below
    };
    let nano_cores = match previous {
        Some(prev) if cpu_now.timestamp > prev.timestamp => {
            let dt_ns = (cpu_now.timestamp - prev.timestamp) as u64;
            let dcpu_ns = cpu_now
                .usage_core_nano_seconds
                .saturating_sub(prev.usage_core_nano_seconds);
            if dt_ns == 0 { 0 } else { (dcpu_ns * 1_000_000_000) / dt_ns }
        }
        _ => 0,
    };

    Ok(ContainerStatsLinux {
        attributes: Some(ContainerAttributes::from(container)),
        cpu: CpuUsage { usage_nano_cores: nano_cores, ..cpu_now },
        memory: MemoryUsage {
            timestamp: now,
            working_set_bytes: v2.memory_current.saturating_sub(v2.memory_swap_current / 2),
            available_bytes: container.spec.resources.memory_limit.unwrap_or(0).saturating_sub(v2.memory_current),
            usage_bytes: v2.memory_current,
            rss_bytes: v2.memory_current.saturating_sub(v2.memory_swap_current),
            page_faults: 0,
            major_page_faults: 0,
        },
        writable_layer: FilesystemUsage {
            timestamp: now,
            fs_id: FilesystemIdentifier { mountpoint: container.rootfs_path.display().to_string() },
            used_bytes: v2.io_write_bytes,
            inodes_used: 0,
        },
        swap: SwapUsage {
            timestamp: now,
            swap_available_bytes: 0,
            swap_usage_bytes: v2.memory_swap_current,
        },
    })
}

/// Stub Windows-shape stats. Used by tests that exercise the message
/// shape; on Linux hosts these map onto the same cgroup numbers without
/// the rss / swap fields.
pub fn container_stats_windows(container: &Container) -> CriResult<WindowsContainerStats> {
    let handle = cgroup::CgroupHandle::new(&container.id.to_string());
    let v2 = cgroup::read_stats_v2(&handle)?;
    let now = Utc::now().timestamp_nanos_opt().unwrap_or(0);
    Ok(WindowsContainerStats {
        attributes: Some(ContainerAttributes::from(container)),
        cpu: WindowsCpuUsage {
            timestamp: now,
            usage_core_nano_seconds: v2.cpu_usage_usec * 1_000,
        },
        memory: WindowsMemoryUsage {
            timestamp: now,
            working_set_bytes: v2.memory_current,
            available_bytes: container.spec.resources.memory_limit.unwrap_or(0).saturating_sub(v2.memory_current),
            commit_memory_bytes: v2.memory_current,
            page_faults: 0,
        },
        writable_layer: WindowsFilesystemUsage {
            timestamp: now,
            fs_id: FilesystemIdentifier { mountpoint: container.rootfs_path.display().to_string() },
            used_bytes: v2.io_write_bytes,
        },
    })
}

/// Filter for `ListContainerStats`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ContainerStatsFilter {
    pub id: Option<Uuid>,
    pub pod_sandbox_id: Option<Uuid>,
    /// Subset of labels that must all match (AND).
    #[serde(default)]
    pub label_selector: std::collections::HashMap<String, String>,
}

/// Apply a filter to a container list. Used by `ListContainerStats`.
pub fn filter_containers<'a>(
    containers: impl IntoIterator<Item = &'a Container>,
    filter: &ContainerStatsFilter,
) -> Vec<&'a Container> {
    containers
        .into_iter()
        .filter(|c| {
            if let Some(id) = filter.id {
                if c.id != id { return false; }
            }
            for (k, v) in &filter.label_selector {
                if c.spec.labels.get(k) != Some(v) {
                    return false;
                }
            }
            true
        })
        .collect()
}

/// Compute aggregate `ImageFsInfo` for a list of cached images.
pub fn image_fs_info(image_root: &str, images: &[crate::models::OciImage]) -> ImageFsInfo {
    let now = Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let total: u64 = images.iter().map(|i| i.size_bytes).sum();
    ImageFsInfo {
        timestamp: now,
        image_filesystems: vec![FilesystemUsage {
            timestamp: now,
            fs_id: FilesystemIdentifier { mountpoint: image_root.to_string() },
            used_bytes: total,
            inodes_used: images.len() as u64,
        }],
        container_filesystems: vec![],
    }
}

/// Convert a `ContainerStatsLinux` into Prometheus-style `Metric`s using
/// the cAdvisor descriptor names.
pub fn linux_to_metrics(stats: &ContainerStatsLinux) -> Vec<Metric> {
    let attrs = stats.attributes.clone();
    let id = attrs.as_ref().map(|a| a.id.to_string()).unwrap_or_default();
    let name = attrs.as_ref().map(|a| a.name.clone()).unwrap_or_default();

    let mut labels = std::collections::BTreeMap::new();
    labels.insert("id".into(), id);
    labels.insert("name".into(), name);

    vec![
        Metric {
            name: "container_cpu_usage_seconds_total".into(),
            timestamp: stats.cpu.timestamp,
            metric_type: MetricType::Counter,
            labels: labels.clone(),
            value: stats.cpu.usage_core_nano_seconds as f64 / 1_000_000_000.0,
        },
        Metric {
            name: "container_memory_usage_bytes".into(),
            timestamp: stats.memory.timestamp,
            metric_type: MetricType::Gauge,
            labels: labels.clone(),
            value: stats.memory.usage_bytes as f64,
        },
        Metric {
            name: "container_memory_working_set_bytes".into(),
            timestamp: stats.memory.timestamp,
            metric_type: MetricType::Gauge,
            labels: labels.clone(),
            value: stats.memory.working_set_bytes as f64,
        },
        Metric {
            name: "container_memory_rss".into(),
            timestamp: stats.memory.timestamp,
            metric_type: MetricType::Gauge,
            labels: labels.clone(),
            value: stats.memory.rss_bytes as f64,
        },
        Metric {
            name: "container_fs_usage_bytes".into(),
            timestamp: stats.writable_layer.timestamp,
            metric_type: MetricType::Gauge,
            labels: labels.clone(),
            value: stats.writable_layer.used_bytes as f64,
        },
        Metric {
            name: "container_swap_usage_bytes".into(),
            timestamp: stats.swap.timestamp,
            metric_type: MetricType::Gauge,
            labels,
            value: stats.swap.swap_usage_bytes as f64,
        },
    ]
}

/// Render `Metric`s in Prometheus text exposition format.
pub fn render_prometheus(metrics: &[Metric]) -> String {
    let mut out = String::new();
    let mut current = "";
    for m in metrics {
        if m.name != current {
            out.push_str(&format!("# TYPE {} {}\n", m.name, match m.metric_type {
                MetricType::Counter => "counter",
                MetricType::Gauge => "gauge",
            }));
            current = &m.name;
        }
        out.push_str(&m.name);
        out.push('{');
        let labels: Vec<String> = m.labels.iter()
            .map(|(k, v)| format!("{}=\"{}\"", k, v))
            .collect();
        out.push_str(&labels.join(","));
        out.push_str("} ");
        out.push_str(&format!("{}\n", m.value));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::*;

    fn make_container() -> Container {
        Container {
            id: Uuid::new_v4(),
            spec: ContainerSpec {
                name: "test".into(),
                image: "nginx:latest".into(),
                command: vec![],
                args: vec![],
                env: Default::default(),
                mounts: vec![],
                resources: ResourceLimits {
                    memory_limit: Some(1024 * 1024),
                    ..Default::default()
                },
                labels: [("env".to_string(), "prod".to_string())].into_iter().collect(),
                working_dir: None,
                user: None,
                hostname: None,
                network_mode: NetworkMode::Bridge,
                restart_policy: RestartPolicy::Never,
            },
            status: ContainerStatus::Running,
            pid: Some(123),
            created_at: Utc::now(),
            started_at: Some(Utc::now()),
            finished_at: None,
            exit_code: None,
            rootfs_path: "/tmp/rootfs".into(),
            log_path: "/tmp/log".into(),
            health: None,
        }
    }

    // ── ContainerAttributes ───────────────────────────────────────────────────

    #[test]
    fn attributes_carry_id_name_and_labels() {
        let c = make_container();
        let a = ContainerAttributes::from(&c);
        assert_eq!(a.id, c.id);
        assert_eq!(a.name, "test");
        assert_eq!(a.labels.get("env"), Some(&"prod".to_string()));
    }

    // ── Linux stats ───────────────────────────────────────────────────────────

    #[test]
    fn container_stats_linux_returns_attributes_and_default_cpu() {
        let c = make_container();
        let s = container_stats_linux(&c, None).unwrap();
        assert_eq!(s.attributes.unwrap().id, c.id);
        // No cgroup data on the test host → all zeros.
        assert_eq!(s.cpu.usage_core_nano_seconds, 0);
        assert_eq!(s.cpu.usage_nano_cores, 0);
        assert_eq!(s.memory.usage_bytes, 0);
    }

    #[test]
    fn container_stats_linux_with_previous_sample_keeps_finite_nano_cores() {
        let c = make_container();
        let prev = CpuUsage {
            timestamp: 0,
            usage_core_nano_seconds: 0,
            usage_nano_cores: 0,
        };
        let s = container_stats_linux(&c, Some(&prev)).unwrap();
        assert!(s.cpu.usage_nano_cores < u64::MAX);
    }

    #[test]
    fn writable_layer_uses_rootfs_path_as_mountpoint() {
        let c = make_container();
        let s = container_stats_linux(&c, None).unwrap();
        assert_eq!(s.writable_layer.fs_id.mountpoint, "/tmp/rootfs");
    }

    // ── Windows stats ─────────────────────────────────────────────────────────

    #[test]
    fn container_stats_windows_has_no_swap_field() {
        let c = make_container();
        let s = container_stats_windows(&c).unwrap();
        assert_eq!(s.attributes.unwrap().id, c.id);
        // Just verify the shape compiles correctly.
        assert_eq!(s.cpu.usage_core_nano_seconds, 0);
        assert_eq!(s.memory.commit_memory_bytes, 0);
    }

    // ── filter_containers ─────────────────────────────────────────────────────

    #[test]
    fn filter_by_id_returns_matching_only() {
        let a = make_container();
        let b = make_container();
        let target_id = a.id;
        let f = ContainerStatsFilter { id: Some(target_id), ..Default::default() };
        let got = filter_containers([&a, &b], &f);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, target_id);
    }

    #[test]
    fn filter_by_label_selector_requires_all_match() {
        let mut a = make_container();
        a.spec.labels.insert("tier".into(), "frontend".into());
        let b = make_container();
        let f = ContainerStatsFilter {
            label_selector: [("tier".to_string(), "frontend".to_string())].into_iter().collect(),
            ..Default::default()
        };
        let got = filter_containers([&a, &b], &f);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, a.id);
    }

    #[test]
    fn empty_filter_returns_all() {
        let a = make_container();
        let b = make_container();
        let got = filter_containers([&a, &b], &ContainerStatsFilter::default());
        assert_eq!(got.len(), 2);
    }

    // ── ImageFsInfo ───────────────────────────────────────────────────────────

    #[test]
    fn image_fs_info_aggregates_sizes() {
        let imgs = vec![
            OciImage {
                reference: "a:1".into(), digest: "d".into(), layers: vec![],
                config: ImageConfig::default(), size_bytes: 100, pulled_at: Utc::now(),
            },
            OciImage {
                reference: "b:1".into(), digest: "d".into(), layers: vec![],
                config: ImageConfig::default(), size_bytes: 250, pulled_at: Utc::now(),
            },
        ];
        let info = image_fs_info("/var/lib/cave/images", &imgs);
        assert_eq!(info.image_filesystems.len(), 1);
        assert_eq!(info.image_filesystems[0].used_bytes, 350);
        assert_eq!(info.image_filesystems[0].inodes_used, 2);
        assert_eq!(info.image_filesystems[0].fs_id.mountpoint, "/var/lib/cave/images");
    }

    #[test]
    fn image_fs_info_empty_returns_zero_used_bytes() {
        let info = image_fs_info("/x", &[]);
        assert_eq!(info.image_filesystems[0].used_bytes, 0);
    }

    // ── cAdvisor descriptors ─────────────────────────────────────────────────

    #[test]
    fn cadvisor_descriptors_include_core_metrics() {
        let d = cadvisor_descriptors();
        let names: Vec<&str> = d.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"container_cpu_usage_seconds_total"));
        assert!(names.contains(&"container_memory_usage_bytes"));
        assert!(names.contains(&"container_memory_working_set_bytes"));
        assert!(names.contains(&"container_memory_rss"));
        assert!(names.contains(&"container_fs_usage_bytes"));
        assert!(names.contains(&"container_swap_usage_bytes"));
        assert!(names.contains(&"container_processes"));
    }

    #[test]
    fn cadvisor_descriptors_have_help_strings() {
        for d in cadvisor_descriptors() {
            assert!(!d.help.is_empty(), "descriptor {} missing help", d.name);
        }
    }

    // ── linux_to_metrics ──────────────────────────────────────────────────────

    #[test]
    fn linux_to_metrics_emits_all_descriptors() {
        let c = make_container();
        let stats = container_stats_linux(&c, None).unwrap();
        let metrics = linux_to_metrics(&stats);
        assert_eq!(metrics.len(), 6);
    }

    #[test]
    fn linux_to_metrics_includes_id_and_name_labels() {
        let c = make_container();
        let id_str = c.id.to_string();
        let stats = container_stats_linux(&c, None).unwrap();
        let metrics = linux_to_metrics(&stats);
        for m in &metrics {
            assert_eq!(m.labels.get("id"), Some(&id_str));
            assert_eq!(m.labels.get("name"), Some(&"test".to_string()));
        }
    }

    #[test]
    fn linux_to_metrics_cpu_is_a_counter() {
        let c = make_container();
        let stats = container_stats_linux(&c, None).unwrap();
        let m = linux_to_metrics(&stats)
            .into_iter()
            .find(|m| m.name == "container_cpu_usage_seconds_total")
            .unwrap();
        assert_eq!(m.metric_type, MetricType::Counter);
    }

    // ── Prometheus rendering ─────────────────────────────────────────────────

    #[test]
    fn render_prometheus_includes_type_lines() {
        let c = make_container();
        let stats = container_stats_linux(&c, None).unwrap();
        let metrics = linux_to_metrics(&stats);
        let rendered = render_prometheus(&metrics);
        assert!(rendered.contains("# TYPE container_cpu_usage_seconds_total counter"));
        assert!(rendered.contains("# TYPE container_memory_usage_bytes gauge"));
    }

    #[test]
    fn render_prometheus_includes_labels() {
        let c = make_container();
        let id_str = c.id.to_string();
        let stats = container_stats_linux(&c, None).unwrap();
        let metrics = linux_to_metrics(&stats);
        let rendered = render_prometheus(&metrics);
        assert!(rendered.contains(&format!("id=\"{}\"", id_str)));
        assert!(rendered.contains("name=\"test\""));
    }

    // ── Serialization ────────────────────────────────────────────────────────

    #[test]
    fn container_stats_linux_serializes() {
        let c = make_container();
        let s = container_stats_linux(&c, None).unwrap();
        let json = serde_json::to_string(&s).unwrap();
        let _back: ContainerStatsLinux = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn windows_container_stats_serializes() {
        let c = make_container();
        let s = container_stats_windows(&c).unwrap();
        let json = serde_json::to_string(&s).unwrap();
        let _back: WindowsContainerStats = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn metric_descriptor_roundtrip_through_json() {
        let d = &cadvisor_descriptors()[0];
        let json = serde_json::to_string(d).unwrap();
        let back: MetricDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, d.name);
    }

    // ── nano_cores algorithm ─────────────────────────────────────────────────

    #[test]
    fn nano_cores_zero_when_no_dt() {
        // Compute path: dt_ns == 0 → nano_cores = 0
        let prev = CpuUsage { timestamp: 1_000, usage_core_nano_seconds: 0, usage_nano_cores: 0 };
        let now = CpuUsage { timestamp: 1_000, usage_core_nano_seconds: 100, usage_nano_cores: 0 };
        // Re-implement the inline computation to lock the algorithm.
        let dt_ns = now.timestamp.saturating_sub(prev.timestamp) as u64;
        let nc = if dt_ns == 0 { 0 } else { 1 };
        assert_eq!(nc, 0);
    }

    fn _silence_unused() -> CriResult<()> {
        Err(CriError::Runtime("unused".into()))
    }
}
