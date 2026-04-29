//! Kubernetes plugin — per-crate panels for the K8s control-plane and
//! data-plane modules cave-runtime ships.
//!
//! Each panel is a structured view-model the renderer turns into HTML; this
//! module is data-shape-only so it stays unit-testable. There are five panels:
//!
//! - **apiserver**: cluster summary (etcd peers, watch fan-out, request rates).
//! - **kubelet**: per-node detail (pods, conditions, capacity, evictions).
//! - **scheduler**: queue depth + last-scheduling-decisions feed.
//! - **cri**: per-pod runtime detail (containers, restart counts, oom kills).
//! - **net**: L4/L7 visualizer placeholder (flow counts, drop rates).

use super::ViewPersona;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiServerPanel {
    pub cluster: String,
    pub etcd_peer_count: u32,
    pub watch_fan_out: u32,
    pub req_per_sec: f64,
    pub p99_latency_ms: f64,
    pub healthy: bool,
}

impl ApiServerPanel {
    pub fn new(cluster: impl Into<String>) -> Self {
        Self {
            cluster: cluster.into(),
            etcd_peer_count: 0,
            watch_fan_out: 0,
            req_per_sec: 0.0,
            p99_latency_ms: 0.0,
            healthy: false,
        }
    }

    /// View is operator/admin only — tenants never see cluster-level metrics.
    pub fn allowed_for(&self, persona: ViewPersona) -> bool {
        matches!(persona, ViewPersona::Operator | ViewPersona::Admin)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeCondition {
    Ready,
    DiskPressure,
    MemoryPressure,
    NetworkUnavailable,
    PIDPressure,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KubeletPanel {
    pub node: String,
    pub pod_count: u32,
    pub max_pods: u32,
    pub eviction_count_1h: u32,
    pub conditions: Vec<NodeCondition>,
    pub allocatable_cpu_milli: u64,
    pub allocatable_mem_mib: u64,
}

impl KubeletPanel {
    pub fn new(node: impl Into<String>, max_pods: u32) -> Self {
        Self {
            node: node.into(),
            pod_count: 0,
            max_pods,
            eviction_count_1h: 0,
            conditions: vec![NodeCondition::Ready],
            allocatable_cpu_milli: 0,
            allocatable_mem_mib: 0,
        }
    }

    pub fn pods_pct(&self) -> u8 {
        if self.max_pods == 0 {
            return 0;
        }
        ((self.pod_count as u32 * 100) / self.max_pods).min(100) as u8
    }

    pub fn is_pressured(&self) -> bool {
        self.conditions
            .iter()
            .any(|c| !matches!(c, NodeCondition::Ready))
    }

    pub fn allowed_for(&self, persona: ViewPersona) -> bool {
        matches!(persona, ViewPersona::Operator | ViewPersona::Admin)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchedulerPanel {
    pub cluster: String,
    pub queue_depth: u32,
    pub schedule_per_sec: f64,
    pub failed_per_sec: f64,
    pub recent_decisions: Vec<ScheduleDecision>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduleDecision {
    pub pod: String,
    pub node: Option<String>,
    pub reason: String,
    pub success: bool,
    pub latency_ms: u32,
}

impl SchedulerPanel {
    pub fn new(cluster: impl Into<String>) -> Self {
        Self {
            cluster: cluster.into(),
            queue_depth: 0,
            schedule_per_sec: 0.0,
            failed_per_sec: 0.0,
            recent_decisions: Vec::new(),
        }
    }

    pub fn push_decision(&mut self, d: ScheduleDecision) {
        self.recent_decisions.insert(0, d);
        if self.recent_decisions.len() > 50 {
            self.recent_decisions.truncate(50);
        }
    }

    pub fn failure_rate(&self) -> f64 {
        let total = self.schedule_per_sec + self.failed_per_sec;
        if total == 0.0 {
            return 0.0;
        }
        self.failed_per_sec / total
    }

    pub fn allowed_for(&self, persona: ViewPersona) -> bool {
        matches!(persona, ViewPersona::Operator | ViewPersona::Admin)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CriPanel {
    pub pod_uid: String,
    pub containers: Vec<ContainerStatus>,
    pub restart_count_total: u32,
    pub oom_kills: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContainerStatus {
    pub name: String,
    pub image: String,
    pub state: ContainerState,
    pub restarts: u32,
    pub started_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerState {
    Waiting,
    Running,
    Terminated,
    OomKilled,
}

impl CriPanel {
    pub fn new(pod_uid: impl Into<String>) -> Self {
        Self {
            pod_uid: pod_uid.into(),
            containers: Vec::new(),
            restart_count_total: 0,
            oom_kills: 0,
        }
    }

    pub fn add_container(&mut self, c: ContainerStatus) {
        self.restart_count_total += c.restarts;
        if matches!(c.state, ContainerState::OomKilled) {
            self.oom_kills += 1;
        }
        self.containers.push(c);
    }

    pub fn is_unhealthy(&self) -> bool {
        self.oom_kills > 0
            || self
                .containers
                .iter()
                .any(|c| matches!(c.state, ContainerState::Waiting | ContainerState::Terminated))
    }

    /// Pod-level CRI detail is visible to the tenant (it's their pod) plus
    /// operator/admin.
    pub fn allowed_for(&self, persona: ViewPersona) -> bool {
        matches!(
            persona,
            ViewPersona::Tenant | ViewPersona::Operator | ViewPersona::Admin
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NetPanel {
    pub cluster: String,
    pub flows_per_sec: f64,
    pub drops_per_sec: f64,
    pub policy_hits_per_sec: f64,
    pub top_talkers: Vec<NetFlow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NetFlow {
    pub source: String,
    pub destination: String,
    pub bytes_per_sec: u64,
    pub protocol: String,
}

impl NetPanel {
    pub fn new(cluster: impl Into<String>) -> Self {
        Self {
            cluster: cluster.into(),
            flows_per_sec: 0.0,
            drops_per_sec: 0.0,
            policy_hits_per_sec: 0.0,
            top_talkers: Vec::new(),
        }
    }

    pub fn drop_rate(&self) -> f64 {
        let total = self.flows_per_sec + self.drops_per_sec;
        if total == 0.0 {
            return 0.0;
        }
        self.drops_per_sec / total
    }

    pub fn allowed_for(&self, persona: ViewPersona) -> bool {
        matches!(persona, ViewPersona::Operator | ViewPersona::Admin)
    }
}

/// Top-level kubernetes-plugin registry — collects all panels for a cluster.
#[derive(Debug, Default)]
pub struct KubernetesPlugin {
    pub apiserver: Option<ApiServerPanel>,
    pub kubelets: Vec<KubeletPanel>,
    pub scheduler: Option<SchedulerPanel>,
    pub cri_pods: Vec<CriPanel>,
    pub net: Option<NetPanel>,
}

impl KubernetesPlugin {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert_kubelet(&mut self, panel: KubeletPanel) {
        if let Some(idx) = self.kubelets.iter().position(|k| k.node == panel.node) {
            self.kubelets[idx] = panel;
        } else {
            self.kubelets.push(panel);
        }
    }

    pub fn upsert_cri_pod(&mut self, panel: CriPanel) {
        if let Some(idx) = self.cri_pods.iter().position(|p| p.pod_uid == panel.pod_uid) {
            self.cri_pods[idx] = panel;
        } else {
            self.cri_pods.push(panel);
        }
    }

    pub fn panel_count(&self) -> usize {
        let mut n = self.kubelets.len() + self.cri_pods.len();
        if self.apiserver.is_some() {
            n += 1;
        }
        if self.scheduler.is_some() {
            n += 1;
        }
        if self.net.is_some() {
            n += 1;
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apiserver_panel_default_unhealthy() {
        let p = ApiServerPanel::new("c1");
        assert_eq!(p.cluster, "c1");
        assert!(!p.healthy);
    }

    #[test]
    fn apiserver_panel_persona_gates_tenants_out() {
        let p = ApiServerPanel::new("c1");
        assert!(!p.allowed_for(ViewPersona::Tenant));
        assert!(p.allowed_for(ViewPersona::Operator));
        assert!(p.allowed_for(ViewPersona::Admin));
    }

    #[test]
    fn apiserver_panel_serializes() {
        let p = ApiServerPanel::new("c1");
        let s = serde_json::to_string(&p).unwrap();
        assert!(s.contains("\"cluster\":\"c1\""));
    }

    #[test]
    fn kubelet_panel_pods_pct_zero_when_max_pods_zero() {
        let mut k = KubeletPanel::new("n1", 0);
        k.pod_count = 5;
        assert_eq!(k.pods_pct(), 0);
    }

    #[test]
    fn kubelet_panel_pods_pct_capped_at_100() {
        let mut k = KubeletPanel::new("n1", 10);
        k.pod_count = 200;
        assert_eq!(k.pods_pct(), 100);
    }

    #[test]
    fn kubelet_panel_pods_pct_normal() {
        let mut k = KubeletPanel::new("n1", 100);
        k.pod_count = 25;
        assert_eq!(k.pods_pct(), 25);
    }

    #[test]
    fn kubelet_panel_default_ready() {
        let k = KubeletPanel::new("n1", 110);
        assert!(!k.is_pressured());
    }

    #[test]
    fn kubelet_panel_pressured_when_disk_pressure() {
        let mut k = KubeletPanel::new("n1", 110);
        k.conditions.push(NodeCondition::DiskPressure);
        assert!(k.is_pressured());
    }

    #[test]
    fn kubelet_panel_persona_operator_only() {
        let k = KubeletPanel::new("n1", 110);
        assert!(!k.allowed_for(ViewPersona::Tenant));
        assert!(k.allowed_for(ViewPersona::Operator));
    }

    #[test]
    fn scheduler_panel_default() {
        let s = SchedulerPanel::new("c1");
        assert_eq!(s.cluster, "c1");
        assert_eq!(s.queue_depth, 0);
        assert!(s.recent_decisions.is_empty());
    }

    #[test]
    fn scheduler_panel_push_decision_prepends() {
        let mut s = SchedulerPanel::new("c1");
        s.push_decision(ScheduleDecision {
            pod: "p1".into(),
            node: Some("n1".into()),
            reason: "fits".into(),
            success: true,
            latency_ms: 10,
        });
        s.push_decision(ScheduleDecision {
            pod: "p2".into(),
            node: None,
            reason: "no-fit".into(),
            success: false,
            latency_ms: 20,
        });
        assert_eq!(s.recent_decisions[0].pod, "p2");
        assert_eq!(s.recent_decisions[1].pod, "p1");
    }

    #[test]
    fn scheduler_panel_truncates_history_to_50() {
        let mut s = SchedulerPanel::new("c1");
        for i in 0..100 {
            s.push_decision(ScheduleDecision {
                pod: format!("p{i}"),
                node: None,
                reason: "x".into(),
                success: true,
                latency_ms: 1,
            });
        }
        assert_eq!(s.recent_decisions.len(), 50);
    }

    #[test]
    fn scheduler_panel_failure_rate_zero_with_no_traffic() {
        let s = SchedulerPanel::new("c1");
        assert_eq!(s.failure_rate(), 0.0);
    }

    #[test]
    fn scheduler_panel_failure_rate_normal() {
        let mut s = SchedulerPanel::new("c1");
        s.schedule_per_sec = 90.0;
        s.failed_per_sec = 10.0;
        assert!((s.failure_rate() - 0.1).abs() < 1e-9);
    }

    #[test]
    fn cri_panel_add_container_aggregates_restarts() {
        let mut p = CriPanel::new("uid-1");
        p.add_container(ContainerStatus {
            name: "main".into(),
            image: "img:1".into(),
            state: ContainerState::Running,
            restarts: 2,
            started_at: None,
        });
        p.add_container(ContainerStatus {
            name: "side".into(),
            image: "img:2".into(),
            state: ContainerState::Running,
            restarts: 3,
            started_at: None,
        });
        assert_eq!(p.restart_count_total, 5);
    }

    #[test]
    fn cri_panel_oom_killed_counts() {
        let mut p = CriPanel::new("uid-1");
        p.add_container(ContainerStatus {
            name: "c".into(),
            image: "img:1".into(),
            state: ContainerState::OomKilled,
            restarts: 0,
            started_at: None,
        });
        assert_eq!(p.oom_kills, 1);
        assert!(p.is_unhealthy());
    }

    #[test]
    fn cri_panel_waiting_is_unhealthy() {
        let mut p = CriPanel::new("uid-1");
        p.add_container(ContainerStatus {
            name: "c".into(),
            image: "img:1".into(),
            state: ContainerState::Waiting,
            restarts: 0,
            started_at: None,
        });
        assert!(p.is_unhealthy());
    }

    #[test]
    fn cri_panel_running_is_healthy() {
        let mut p = CriPanel::new("uid-1");
        p.add_container(ContainerStatus {
            name: "c".into(),
            image: "img:1".into(),
            state: ContainerState::Running,
            restarts: 0,
            started_at: Some("t".into()),
        });
        assert!(!p.is_unhealthy());
    }

    #[test]
    fn cri_panel_persona_tenant_allowed() {
        let p = CriPanel::new("uid");
        // tenant sees their own pod
        assert!(p.allowed_for(ViewPersona::Tenant));
    }

    #[test]
    fn net_panel_drop_rate_zero_with_no_traffic() {
        let n = NetPanel::new("c1");
        assert_eq!(n.drop_rate(), 0.0);
    }

    #[test]
    fn net_panel_drop_rate_normal() {
        let mut n = NetPanel::new("c1");
        n.flows_per_sec = 950.0;
        n.drops_per_sec = 50.0;
        assert!((n.drop_rate() - 0.05).abs() < 1e-9);
    }

    #[test]
    fn net_panel_persona_operator_only() {
        let n = NetPanel::new("c1");
        assert!(!n.allowed_for(ViewPersona::Tenant));
        assert!(n.allowed_for(ViewPersona::Operator));
    }

    #[test]
    fn plugin_panel_count_starts_at_zero() {
        let k = KubernetesPlugin::new();
        assert_eq!(k.panel_count(), 0);
    }

    #[test]
    fn plugin_panel_count_after_upserts() {
        let mut k = KubernetesPlugin::new();
        k.apiserver = Some(ApiServerPanel::new("c1"));
        k.upsert_kubelet(KubeletPanel::new("n1", 110));
        k.upsert_kubelet(KubeletPanel::new("n2", 110));
        k.scheduler = Some(SchedulerPanel::new("c1"));
        k.upsert_cri_pod(CriPanel::new("p1"));
        k.net = Some(NetPanel::new("c1"));
        assert_eq!(k.panel_count(), 6);
    }

    #[test]
    fn plugin_upsert_kubelet_replaces_existing() {
        let mut k = KubernetesPlugin::new();
        let mut kl = KubeletPanel::new("n1", 110);
        kl.pod_count = 5;
        k.upsert_kubelet(kl);
        let mut kl2 = KubeletPanel::new("n1", 110);
        kl2.pod_count = 7;
        k.upsert_kubelet(kl2);
        assert_eq!(k.kubelets.len(), 1);
        assert_eq!(k.kubelets[0].pod_count, 7);
    }

    #[test]
    fn plugin_upsert_cri_pod_replaces_existing() {
        let mut k = KubernetesPlugin::new();
        let mut p = CriPanel::new("p1");
        p.add_container(ContainerStatus {
            name: "c".into(),
            image: "v1".into(),
            state: ContainerState::Running,
            restarts: 0,
            started_at: None,
        });
        k.upsert_cri_pod(p);
        let mut p2 = CriPanel::new("p1");
        p2.add_container(ContainerStatus {
            name: "c".into(),
            image: "v2".into(),
            state: ContainerState::Running,
            restarts: 1,
            started_at: None,
        });
        k.upsert_cri_pod(p2);
        assert_eq!(k.cri_pods.len(), 1);
        assert_eq!(k.cri_pods[0].containers[0].image, "v2");
    }

    #[test]
    fn view_persona_label() {
        assert_eq!(ViewPersona::Tenant.label(), "tenant");
        assert_eq!(ViewPersona::Operator.label(), "operator");
        assert_eq!(ViewPersona::Admin.label(), "admin");
    }
}
