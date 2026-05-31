// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Edge node agent — the "edged" lightweight kubelet.
//!
//! Ports two pieces of upstream behavior in pure Rust:
//!
//!   * the kubelet pod-phase machine, `pkg/kubelet/kubelet_pods.go::getPhase`
//!     (kubernetes/kubernetes, the engine K3s embeds), driven by per-container
//!     state and the pod `RestartPolicy`; and
//!   * the KubeEdge `edge/pkg/edged` pod-worker lifecycle — add/update/delete
//!     dispatch, `cleanupOrphanedPodDirectories` (terminate pods no longer in
//!     the cloud's desired set), and the status manager's 10s report cadence
//!     (`statusUpdateInterval`).
//!
//! No container runtime and no network: this is the scheduling/lifecycle
//! decision layer only.

use std::collections::BTreeMap;

/// Pod restart policy (`v1.RestartPolicy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartPolicy {
    Always,
    OnFailure,
    Never,
}

/// Per-container runtime state (the subset of `v1.ContainerState` that
/// `getPhase` keys off).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerState {
    Waiting,
    Running,
    Terminated { exit_code: i32 },
}

/// A single container's status within a pod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerStatus {
    pub name: String,
    pub state: ContainerState,
}

/// Aggregate pod lifecycle phase (`v1.PodPhase`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PodPhase {
    Pending,
    Running,
    Succeeded,
    Failed,
    Unknown,
}

/// A pod tracked by the edge agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pod {
    pub name: String,
    pub namespace: String,
    pub uid: String,
    pub restart_policy: RestartPolicy,
    pub containers: Vec<ContainerStatus>,
}

impl Pod {
    /// Compute the aggregate phase from the regular containers, faithful to
    /// kubelet's `getPhase`. `terminal` is true when the pod is being torn
    /// down (`podIsTerminal`), which suppresses the Always→Running restart
    /// short-circuit so a terminating pod can settle into Succeeded/Failed.
    pub fn compute_phase(&self, terminal: bool) -> PodPhase {
        if self.containers.is_empty() {
            // No container statuses yet → still scheduling.
            return PodPhase::Pending;
        }

        let mut running = 0usize;
        let mut waiting = 0usize;
        let mut stopped = 0usize; // terminated (any exit code)
        let mut succeeded = 0usize; // terminated exit 0
        let mut failed = 0usize; // terminated non-zero

        for c in &self.containers {
            match c.state {
                ContainerState::Running => running += 1,
                ContainerState::Waiting => waiting += 1,
                ContainerState::Terminated { exit_code } => {
                    stopped += 1;
                    if exit_code == 0 {
                        succeeded += 1;
                    } else {
                        failed += 1;
                    }
                }
            }
        }

        match () {
            // Any container still waiting → the pod is not fully up.
            _ if waiting > 0 => PodPhase::Pending,
            // At least one container running → Running.
            _ if running > 0 => PodPhase::Running,
            // All containers have stopped.
            _ if running == 0 && stopped > 0 => match self.restart_policy {
                RestartPolicy::Always => {
                    if terminal {
                        // Terminating: settle by exit codes.
                        if failed > 0 {
                            PodPhase::Failed
                        } else {
                            PodPhase::Succeeded
                        }
                    } else {
                        // Containers will be restarted.
                        PodPhase::Running
                    }
                }
                RestartPolicy::OnFailure => {
                    if failed > 0 && !terminal {
                        // Failures will be retried.
                        PodPhase::Running
                    } else if succeeded == stopped {
                        PodPhase::Succeeded
                    } else {
                        PodPhase::Failed
                    }
                }
                RestartPolicy::Never => {
                    if succeeded == stopped {
                        PodPhase::Succeeded
                    } else {
                        PodPhase::Failed
                    }
                }
            },
            _ => PodPhase::Pending,
        }
    }
}

/// Work item dispatched to the pod worker (KubeEdge `podAdditions` /
/// `podModifications` / `podDeletions` channels collapsed into one queue).
#[derive(Debug, Clone)]
pub enum PodWork {
    Add(Pod),
    Update(Pod),
    Delete(String),
}

/// Status-report cadence in the unit used by the caller (upstream
/// `statusUpdateInterval` is 10 seconds).
const STATUS_INTERVAL: u64 = 10;

/// The edge node agent: holds the local pod set and the status cadence.
#[derive(Debug, Clone)]
pub struct Edged {
    node_name: String,
    pods: BTreeMap<String, Pod>,
    /// Timestamp of the last status report; `None` until the first report.
    last_report: Option<u64>,
}

impl Edged {
    pub fn new(node_name: &str) -> Self {
        Self {
            node_name: node_name.to_string(),
            pods: BTreeMap::new(),
            last_report: None,
        }
    }

    pub fn node_name(&self) -> &str {
        &self.node_name
    }

    pub fn pod_count(&self) -> usize {
        self.pods.len()
    }

    /// Process one work item, mirroring the edged pod-worker dispatch.
    pub fn dispatch(&mut self, work: PodWork) {
        match work {
            PodWork::Add(p) | PodWork::Update(p) => {
                self.pods.insert(p.name.clone(), p);
            }
            PodWork::Delete(name) => {
                self.pods.remove(&name);
            }
        }
    }

    /// Current phase of a tracked pod, or `None` if not present.
    pub fn phase_of(&self, name: &str) -> Option<PodPhase> {
        self.pods.get(name).map(|p| p.compute_phase(false))
    }

    /// `cleanupOrphanedPodDirectories`: terminate every local pod that is not
    /// in the cloud's desired set. Returns the names removed (sorted).
    pub fn cleanup_orphans(&mut self, desired: &[String]) -> Vec<String> {
        let keep: std::collections::BTreeSet<&str> = desired.iter().map(|s| s.as_str()).collect();
        let orphans: Vec<String> = self
            .pods
            .keys()
            .filter(|k| !keep.contains(k.as_str()))
            .cloned()
            .collect();
        for o in &orphans {
            self.pods.remove(o);
        }
        orphans
    }

    /// Status-manager cadence: true when a status report is due at `now`.
    /// The first call establishes the baseline (always due); subsequent calls
    /// are due once `STATUS_INTERVAL` has elapsed since the last report. A due
    /// call advances the window.
    pub fn status_report_due(&mut self, now: u64) -> bool {
        match self.last_report {
            None => {
                self.last_report = Some(now);
                true
            }
            Some(last) if now.saturating_sub(last) >= STATUS_INTERVAL => {
                self.last_report = Some(now);
                true
            }
            _ => false,
        }
    }
}
