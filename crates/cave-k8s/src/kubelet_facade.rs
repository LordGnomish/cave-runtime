// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Kubelet facade — exposes per-node lifecycle hooks consumed by the
//! umbrella (pod assignment, status sync, evictions).  The real probe /
//! CSI / cgroup machinery lives in `cave-kubelet` and `cave-cri`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PodPhase {
    Pending,
    Running,
    Succeeded,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodAssignment {
    pub namespace: String,
    pub name: String,
    pub uid: String,
    pub node: String,
    pub phase: PodPhase,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub restart_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeStatus {
    pub name: String,
    pub ready: bool,
    pub cpu_capacity_millis: u32,
    pub memory_capacity_bytes: u64,
    pub cpu_used_millis: u32,
    pub memory_used_bytes: u64,
    pub pods: Vec<PodAssignment>,
}

impl NodeStatus {
    pub fn cpu_available_millis(&self) -> u32 {
        self.cpu_capacity_millis.saturating_sub(self.cpu_used_millis)
    }
    pub fn memory_available_bytes(&self) -> u64 {
        self.memory_capacity_bytes.saturating_sub(self.memory_used_bytes)
    }
    pub fn pod_count(&self) -> usize {
        self.pods.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LifecycleAction {
    Start,
    Stop,
    Restart,
    Remove,
}

pub fn drive_pod_action(
    pod: &mut PodAssignment,
    action: LifecycleAction,
) -> PodPhase {
    match action {
        LifecycleAction::Start => {
            if pod.phase == PodPhase::Pending {
                pod.phase = PodPhase::Running;
                pod.started_at = chrono::Utc::now();
            }
            pod.phase
        }
        LifecycleAction::Stop => {
            pod.phase = PodPhase::Succeeded;
            pod.phase
        }
        LifecycleAction::Restart => {
            pod.restart_count += 1;
            pod.phase = PodPhase::Running;
            pod.phase
        }
        LifecycleAction::Remove => {
            pod.phase = PodPhase::Failed;
            pod.phase
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pod(name: &str, phase: PodPhase) -> PodAssignment {
        PodAssignment {
            namespace: "default".into(),
            name: name.into(),
            uid: uuid::Uuid::new_v4().to_string(),
            node: "n1".into(),
            phase,
            started_at: chrono::Utc::now(),
            restart_count: 0,
        }
    }

    #[test]
    fn start_pending_pod_runs() {
        let mut p = pod("p", PodPhase::Pending);
        let new = drive_pod_action(&mut p, LifecycleAction::Start);
        assert_eq!(new, PodPhase::Running);
    }

    #[test]
    fn restart_increments_counter() {
        let mut p = pod("p", PodPhase::Running);
        drive_pod_action(&mut p, LifecycleAction::Restart);
        assert_eq!(p.restart_count, 1);
        assert_eq!(p.phase, PodPhase::Running);
    }

    #[test]
    fn stop_sets_succeeded() {
        let mut p = pod("p", PodPhase::Running);
        drive_pod_action(&mut p, LifecycleAction::Stop);
        assert_eq!(p.phase, PodPhase::Succeeded);
    }

    #[test]
    fn node_status_capacity_math() {
        let n = NodeStatus {
            name: "n".into(),
            ready: true,
            cpu_capacity_millis: 4000,
            memory_capacity_bytes: 8 * 1024 * 1024 * 1024,
            cpu_used_millis: 1500,
            memory_used_bytes: 1024 * 1024 * 1024,
            pods: vec![],
        };
        assert_eq!(n.cpu_available_millis(), 2500);
        assert_eq!(n.memory_available_bytes(), 7 * 1024 * 1024 * 1024);
        assert_eq!(n.pod_count(), 0);
    }

    #[test]
    fn pod_phase_serializes_pascal_case() {
        let s = serde_json::to_string(&PodPhase::Running).unwrap();
        assert_eq!(s, "\"Running\"");
    }

    #[test]
    fn start_running_pod_no_op() {
        let mut p = pod("p", PodPhase::Running);
        let before = p.started_at;
        drive_pod_action(&mut p, LifecycleAction::Start);
        // started_at unchanged for already-running pods
        assert_eq!(p.started_at, before);
    }

    #[test]
    fn remove_sets_failed() {
        let mut p = pod("p", PodPhase::Running);
        drive_pod_action(&mut p, LifecycleAction::Remove);
        assert_eq!(p.phase, PodPhase::Failed);
    }
}
