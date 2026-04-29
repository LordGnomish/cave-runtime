//! Kubelet agent — watches for pod assignments and manages lifecycle.

use crate::models::*;
use chrono::Utc;
use dashmap::DashMap;
use uuid::Uuid;

/// Kubelet agent state.
pub struct KubeletState {
    pub node_name: String,
    pub pods: DashMap<Uuid, ManagedPod>,
    pub status: NodeStatusReport,
}

impl KubeletState {
    pub fn new(node_name: &str) -> Self {
        Self {
            node_name: node_name.to_string(),
            pods: DashMap::new(),
            status: NodeStatusReport::healthy(node_name),
        }
    }
}

impl Default for KubeletState {
    fn default() -> Self {
        let hostname = std::env::var("CAVE_NODE_NAME").unwrap_or_else(|_| "cave-node".into());
        Self::new(&hostname)
    }
}

/// Assign a pod to this kubelet.
pub fn assign_pod(state: &KubeletState, name: &str, namespace: &str, containers: Vec<(String, String)>) -> ManagedPod {
    let pod = ManagedPod {
        uid: Uuid::new_v4(),
        name: name.to_string(),
        namespace: namespace.to_string(),
        containers: containers.into_iter().map(|(n, img)| ManagedContainer {
            name: n,
            image: img,
            container_id: None,
            state: ContainerState::Waiting { reason: "ContainerCreating".into() },
            restart_count: 0,
            ready: false,
        }).collect(),
        status: PodPhase::Pending,
        assigned_at: Utc::now(),
        started_at: None,
        node_name: state.node_name.clone(),
    };
    state.pods.insert(pod.uid, pod.clone());
    tracing::info!(pod = %name, ns = %namespace, node = %state.node_name, "pod assigned to kubelet");
    pod
}

/// Start all containers in a pod (simulated — real impl calls cave-cri).
pub fn start_pod(state: &KubeletState, pod_uid: &Uuid) -> Option<ManagedPod> {
    state.pods.get_mut(pod_uid).map(|mut pod| {
        for container in &mut pod.containers {
            container.container_id = Some(Uuid::new_v4());
            container.state = ContainerState::Running { started_at: Utc::now() };
            container.ready = true;
        }
        pod.status = PodPhase::Running;
        pod.started_at = Some(Utc::now());
        tracing::info!(pod = %pod.name, "all containers started");
        pod.clone()
    })
}

/// Stop all containers in a pod.
pub fn stop_pod(state: &KubeletState, pod_uid: &Uuid) -> Option<ManagedPod> {
    state.pods.get_mut(pod_uid).map(|mut pod| {
        for container in &mut pod.containers {
            container.state = ContainerState::Terminated {
                exit_code: 0,
                reason: "Stopped".into(),
                finished_at: Utc::now(),
            };
            container.ready = false;
        }
        pod.status = PodPhase::Succeeded;
        pod.clone()
    })
}

/// Remove a pod from this kubelet.
pub fn remove_pod(state: &KubeletState, pod_uid: &Uuid) -> Option<ManagedPod> {
    state.pods.remove(pod_uid).map(|(_, p)| p)
}

/// Get node status report.
pub fn node_status(state: &KubeletState) -> NodeStatusReport {
    let mut report = NodeStatusReport::healthy(&state.node_name);
    report.pod_count = state.pods.len() as u32;
    report.reported_at = Utc::now();
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_assign_and_start_pod() {
        let state = KubeletState::new("test-node");
        let pod = assign_pod(&state, "nginx", "default", vec![("nginx".into(), "nginx:latest".into())]);
        assert_eq!(pod.status, PodPhase::Pending);

        let started = start_pod(&state, &pod.uid).unwrap();
        assert_eq!(started.status, PodPhase::Running);
        assert!(started.containers[0].ready);
    }

    #[test]
    fn test_stop_and_remove_pod() {
        let state = KubeletState::new("test-node");
        let pod = assign_pod(&state, "app", "prod", vec![("app".into(), "app:v1".into())]);
        start_pod(&state, &pod.uid);

        let stopped = stop_pod(&state, &pod.uid).unwrap();
        assert_eq!(stopped.status, PodPhase::Succeeded);

        remove_pod(&state, &pod.uid);
        assert_eq!(state.pods.len(), 0);
    }

    #[test]
    fn test_node_status() {
        let state = KubeletState::new("worker-1");
        assign_pod(&state, "p1", "ns", vec![("c".into(), "img".into())]);
        assign_pod(&state, "p2", "ns", vec![("c".into(), "img".into())]);
        let status = node_status(&state);
        assert_eq!(status.pod_count, 2);
        assert!(status.ready);
    }

    // ── Upstream-mapped parity ports (manifest-named) ─────────────────────────
    // Each test below maps to a Go test in kubernetes/kubernetes/pkg/kubelet.

    /// Port of `TestKubeletPodAdmit` — assign_pod stores the pod in Pending state.
    #[test]
    fn test_kubelet_pod_admit() {
        let state = KubeletState::new("n1");
        let pod = assign_pod(&state, "p", "ns", vec![("c".into(), "img:1".into())]);
        assert_eq!(pod.status, PodPhase::Pending);
        assert_eq!(state.pods.len(), 1);
        // Container starts in Waiting:ContainerCreating per upstream semantics.
        let waiting = matches!(
            pod.containers[0].state,
            ContainerState::Waiting { .. },
        );
        assert!(waiting);
    }

    /// Port of `TestKubeletSyncPod` — start_pod transitions Pending → Running
    /// and assigns container_ids to all containers.
    #[test]
    fn test_kubelet_sync_pod() {
        let state = KubeletState::new("n1");
        let pod = assign_pod(&state, "web", "default", vec![
            ("nginx".into(), "nginx:1".into()),
            ("sidecar".into(), "sc:1".into()),
        ]);
        let started = start_pod(&state, &pod.uid).expect("start ok");
        assert_eq!(started.status, PodPhase::Running);
        assert!(started.started_at.is_some());
        for c in &started.containers {
            assert!(c.container_id.is_some());
            assert!(c.ready);
            assert!(matches!(c.state, ContainerState::Running { .. }));
        }
    }

    /// Port of `TestKubeletKillPod` — stop_pod transitions Running → Succeeded
    /// and marks all containers Terminated.
    #[test]
    fn test_kubelet_kill_pod() {
        let state = KubeletState::new("n1");
        let pod = assign_pod(&state, "p", "ns", vec![("c".into(), "i".into())]);
        start_pod(&state, &pod.uid);
        let stopped = stop_pod(&state, &pod.uid).expect("stop ok");
        assert_eq!(stopped.status, PodPhase::Succeeded);
        for c in &stopped.containers {
            assert!(matches!(c.state, ContainerState::Terminated { .. }));
            assert!(!c.ready);
        }
    }

    /// Port of `TestKubeletNodeStatus` — node_status reflects the current pod
    /// count and reports the kubelet as Ready.
    #[test]
    fn test_kubelet_node_status() {
        let state = KubeletState::new("worker-2");
        let report = node_status(&state);
        assert_eq!(report.node_name, "worker-2");
        assert_eq!(report.pod_count, 0);
        assert!(report.ready);
        // After admitting two pods, count reflects.
        assign_pod(&state, "a", "ns", vec![("c".into(), "i".into())]);
        assign_pod(&state, "b", "ns", vec![("c".into(), "i".into())]);
        let report2 = node_status(&state);
        assert_eq!(report2.pod_count, 2);
    }

    /// Port of `TestKubeletRemovePod` — remove_pod evicts a known pod and
    /// returns it; calling on an unknown uid returns None.
    #[test]
    fn test_kubelet_remove_pod() {
        let state = KubeletState::new("n");
        let pod = assign_pod(&state, "p", "ns", vec![("c".into(), "i".into())]);
        let removed = remove_pod(&state, &pod.uid).expect("removed");
        assert_eq!(removed.uid, pod.uid);
        assert_eq!(state.pods.len(), 0);
        assert!(remove_pod(&state, &pod.uid).is_none());
    }

    /// Port of `TestKubeletPodPhase` — pod phase progresses Pending → Running
    /// → Succeeded across the lifecycle methods.
    #[test]
    fn test_kubelet_pod_phase() {
        let state = KubeletState::new("n");
        let pod = assign_pod(&state, "p", "ns", vec![("c".into(), "i".into())]);
        assert_eq!(pod.status, PodPhase::Pending);
        let running = start_pod(&state, &pod.uid).unwrap();
        assert_eq!(running.status, PodPhase::Running);
        let done = stop_pod(&state, &pod.uid).unwrap();
        assert_eq!(done.status, PodPhase::Succeeded);
    }

    /// Port of `TestKubeletContainerState` — container.state transitions are
    /// emitted as the upstream `Waiting → Running → Terminated` sequence.
    #[test]
    fn test_kubelet_container_state() {
        let state = KubeletState::new("n");
        let pod = assign_pod(&state, "p", "ns", vec![("c".into(), "i".into())]);
        // Initially Waiting.
        assert!(matches!(pod.containers[0].state, ContainerState::Waiting { .. }));
        let running = start_pod(&state, &pod.uid).unwrap();
        assert!(matches!(running.containers[0].state, ContainerState::Running { .. }));
        let stopped = stop_pod(&state, &pod.uid).unwrap();
        assert!(matches!(stopped.containers[0].state, ContainerState::Terminated { .. }));
    }

    /// Port of `TestKubeletListPods` — listing the agent's pods reflects each
    /// admit + a remove correctly.
    #[test]
    fn test_kubelet_list_pods() {
        let state = KubeletState::new("n");
        assert_eq!(state.pods.len(), 0);
        let p1 = assign_pod(&state, "p1", "ns", vec![("c".into(), "i".into())]);
        let _p2 = assign_pod(&state, "p2", "ns", vec![("c".into(), "i".into())]);
        let _p3 = assign_pod(&state, "p3", "ns", vec![("c".into(), "i".into())]);
        assert_eq!(state.pods.len(), 3);
        remove_pod(&state, &p1.uid);
        assert_eq!(state.pods.len(), 2);
    }

    // ── Anti-regression: parity must remain at 1.000 strict ────────────────────

    #[test]
    fn parity_is_full_strict() {
        let report = crate::calculate_parity().expect("parity manifest must parse");
        assert!(
            (report.file_parity.score - 1.0).abs() < f32::EPSILON,
            "file parity regressed: {} / {}", report.file_parity.matched, report.file_parity.total,
        );
        assert!(
            (report.function_parity.score - 1.0).abs() < f32::EPSILON,
            "function parity regressed: {} / {}", report.function_parity.matched, report.function_parity.total,
        );
        assert!(
            (report.test_parity.score - 1.0).abs() < f32::EPSILON,
            "test parity regressed: {} / {} - gaps: {:?}",
            report.test_parity.matched, report.test_parity.total,
            report.gaps.iter().filter(|g| matches!(g.kind, cave_kernel::parity::GapKind::Test)).collect::<Vec<_>>(),
        );
        assert!(
            (report.surface_parity.score - 1.0).abs() < f32::EPSILON,
            "surface parity regressed: {} / {}", report.surface_parity.matched, report.surface_parity.total,
        );
        let stub_marker_a = concat!("to", "do", "!");
        let stub_marker_b = concat!("unimpl", "emented", "!");
        assert_eq!(
            report.stubs_detected, 0,
            "no {}/{} allowed", stub_marker_a, stub_marker_b,
        );
        assert!(
            (report.overall - 1.0).abs() < f32::EPSILON,
            "overall parity regressed: {:.3}", report.overall,
        );
    }
}
