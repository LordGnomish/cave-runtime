// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
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
pub fn assign_pod(
    state: &KubeletState,
    name: &str,
    namespace: &str,
    containers: Vec<(String, String)>,
) -> ManagedPod {
    let pod = ManagedPod {
        uid: Uuid::new_v4(),
        name: name.to_string(),
        namespace: namespace.to_string(),
        containers: containers
            .into_iter()
            .map(|(n, img)| ManagedContainer {
                name: n,
                image: img,
                container_id: None,
                state: ContainerState::Waiting {
                    reason: "ContainerCreating".into(),
                },
                restart_count: 0,
                ready: false,
            })
            .collect(),
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
            container.state = ContainerState::Running {
                started_at: Utc::now(),
            };
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
        let pod = assign_pod(
            &state,
            "nginx",
            "default",
            vec![("nginx".into(), "nginx:latest".into())],
        );
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

    #[test]
    fn test_start_pod_unknown_uid_returns_none() {
        // Starting a pod that was never assigned must return None, not panic
        // and not create a phantom pod entry.
        let state = KubeletState::new("test-node");
        let bogus = Uuid::new_v4();
        assert!(start_pod(&state, &bogus).is_none());
        assert_eq!(state.pods.len(), 0);
    }

    #[test]
    fn test_assign_pod_initial_container_state_is_waiting() {
        // A freshly assigned pod's containers must start in Waiting/ContainerCreating
        // — the lifecycle invariant that mirrors kubelet.
        let state = KubeletState::new("test-node");
        let pod = assign_pod(&state, "p", "ns", vec![("c".into(), "img:1".into())]);
        match &pod.containers[0].state {
            ContainerState::Waiting { reason } => {
                assert_eq!(reason, "ContainerCreating");
            }
            other => panic!("expected Waiting, got {:?}", other),
        }
        assert!(!pod.containers[0].ready);
        assert!(pod.containers[0].container_id.is_none());
        assert!(pod.started_at.is_none());
    }

    #[test]
    fn test_start_pod_assigns_container_id() {
        // After start_pod every container must have a non-None container_id —
        // this is what cave-cri later uses to address each container.
        let state = KubeletState::new("test-node");
        let pod = assign_pod(
            &state,
            "multi",
            "ns",
            vec![("c1".into(), "img1".into()), ("c2".into(), "img2".into())],
        );
        let started = start_pod(&state, &pod.uid).unwrap();
        assert!(started.containers.iter().all(|c| c.container_id.is_some()));
        assert!(started.containers.iter().all(|c| c.ready));
        assert!(started.started_at.is_some());
    }

    #[test]
    fn test_remove_unknown_pod_is_noop() {
        // Removing a UID we don't have must not panic and must not affect
        // existing pods.
        let state = KubeletState::new("n");
        let alive = assign_pod(&state, "alive", "ns", vec![("c".into(), "i".into())]);
        let bogus = Uuid::new_v4();
        assert!(remove_pod(&state, &bogus).is_none());
        // alive pod still there.
        assert!(state.pods.get(&alive.uid).is_some());
    }

    #[test]
    fn test_stop_pod_zero_exit_code() {
        // After stop_pod, every container must be Terminated with exit_code 0
        // (graceful path) — this is what propagates up to the apiserver as
        // PodPhase::Succeeded.
        let state = KubeletState::new("n");
        let pod = assign_pod(&state, "p", "ns", vec![("c".into(), "i".into())]);
        start_pod(&state, &pod.uid);
        let stopped = stop_pod(&state, &pod.uid).unwrap();
        for c in &stopped.containers {
            match &c.state {
                ContainerState::Terminated {
                    exit_code, reason, ..
                } => {
                    assert_eq!(*exit_code, 0);
                    assert_eq!(reason, "Stopped");
                }
                other => panic!("expected Terminated, got {:?}", other),
            }
            assert!(!c.ready);
        }
    }
}
