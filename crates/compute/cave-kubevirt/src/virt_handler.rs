// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! virt-handler: per-node KubeVirt agent.
//!
//! Upstream: kubevirt/kubevirt v1.8.2
//!   cmd/virt-handler/virt-handler.go (entrypoint)
//!   pkg/virt-handler/vm.go (Controller)
//!   pkg/virt-handler/node-labeller (CPU model labelling)
//!
//! The virt-handler is a DaemonSet pod running on every node that hosts
//! VMs. It watches `VirtualMachineInstance` resources placed on its node,
//! synchronises lifecycle commands with the local virt-launcher pods, and
//! reports observed status back to the apiserver.
//!
//! This module implements the pure decision logic — the command dispatcher,
//! node-labeller fingerprinting, the per-VMI watch loop coordination, and
//! the heartbeat schema — without the libvirt-over-gRPC socket binding,
//! which lives in cave-runtime's host-preflight layer.

use crate::models::{VirtualMachineInstance, VmPhase};
use std::collections::BTreeMap;

/// Commands the controller can dispatch to the local virt-launcher.
/// Mirrors the verb set on `pkg/virt-handler/cmd-client`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LauncherCommand {
    /// Define + start the domain.
    Sync,
    /// Graceful shutdown via ACPI signal.
    Shutdown,
    /// Hard stop (kill -9 on qemu).
    Kill,
    /// Pause execution (libvirt suspend).
    Pause,
    /// Resume from a paused state.
    Unpause,
    /// Snapshot the current state.
    Freeze,
    /// Resume from a freeze.
    Unfreeze,
    /// Initiate live migration.
    Migrate,
    /// Abort an in-flight live migration.
    CancelMigration,
}

impl LauncherCommand {
    /// The RPC method name virt-handler uses on the cmd-client socket.
    pub fn cmd_client_method(&self) -> &'static str {
        match self {
            LauncherCommand::Sync => "SyncVMI",
            LauncherCommand::Shutdown => "ShutdownVMI",
            LauncherCommand::Kill => "KillVMI",
            LauncherCommand::Pause => "PauseVMI",
            LauncherCommand::Unpause => "UnpauseVMI",
            LauncherCommand::Freeze => "FreezeVMI",
            LauncherCommand::Unfreeze => "UnfreezeVMI",
            LauncherCommand::Migrate => "MigrateVMI",
            LauncherCommand::CancelMigration => "CancelMigrationVMI",
        }
    }
}

/// Decide which launcher command to dispatch given the desired phase and
/// the currently observed phase. `None` means "in sync, no command".
pub fn decide_command(desired: VmPhase, observed: VmPhase) -> Option<LauncherCommand> {
    use VmPhase::*;
    match (desired, observed) {
        (Starting, Stopped) | (Starting, Error) | (Running, Stopped) => Some(LauncherCommand::Sync),
        (Stopping, Running) | (Stopping, Starting) => Some(LauncherCommand::Shutdown),
        (Stopped, Stopping) => Some(LauncherCommand::Kill),
        (Migrating, Running) => Some(LauncherCommand::Migrate),
        (Running, Migrating) => Some(LauncherCommand::CancelMigration),
        (Terminating, _) => Some(LauncherCommand::Kill),
        _ => None,
    }
}

/// Node-labeller fingerprint emitted as Kubernetes labels.
///
/// Upstream's node-labeller introspects `/proc/cpuinfo` + libvirt CPU
/// model probe and emits ~30 labels of the form
/// `cpu-feature.node.kubevirt.io/<feature>=true`. This struct captures
/// the canonical key/value shape.
#[derive(Debug, Clone, Default)]
pub struct NodeFingerprint {
    pub kvm_present: bool,
    pub cpu_model: String,
    pub cpu_vendor: String,
    pub features: Vec<String>,
    pub kernel_version: String,
    pub hostname: String,
}

impl NodeFingerprint {
    /// Render the fingerprint as the label map placed on the Node object.
    pub fn to_labels(&self) -> BTreeMap<String, String> {
        let mut labels = BTreeMap::new();
        labels.insert(
            "kubevirt.io/schedulable".into(),
            if self.kvm_present { "true".into() } else { "false".into() },
        );
        if !self.cpu_vendor.is_empty() {
            labels.insert(
                "cpu-vendor.node.kubevirt.io".into(),
                self.cpu_vendor.clone(),
            );
        }
        if !self.cpu_model.is_empty() {
            labels.insert(
                format!("cpu-model.node.kubevirt.io/{}", self.cpu_model),
                "true".into(),
            );
        }
        for feat in &self.features {
            labels.insert(
                format!("cpu-feature.node.kubevirt.io/{}", feat),
                "true".into(),
            );
        }
        if !self.kernel_version.is_empty() {
            labels.insert(
                "kernel-version.node.kubevirt.io".into(),
                self.kernel_version.clone(),
            );
        }
        if !self.hostname.is_empty() {
            labels.insert("kubernetes.io/hostname".into(), self.hostname.clone());
        }
        labels
    }

    /// Whether this node is schedulable for VMI placement.
    pub fn is_schedulable(&self) -> bool {
        self.kvm_present
    }
}

/// Heartbeat the virt-handler emits at a fixed cadence so the controller
/// can detect dead handlers.
#[derive(Debug, Clone, PartialEq)]
pub struct Heartbeat {
    pub node_name: String,
    pub timestamp_unix: i64,
    pub vmi_count: usize,
    pub maintenance: bool,
}

impl Heartbeat {
    /// `true` iff this heartbeat is younger than `max_age_secs` against
    /// `now_unix`. Used by the controller to mark a handler dead.
    pub fn is_fresh(&self, now_unix: i64, max_age_secs: i64) -> bool {
        now_unix.saturating_sub(self.timestamp_unix) <= max_age_secs
    }
}

/// State machine that the per-VMI watch goroutine runs in upstream.
/// Captures the transitions between launcher-pod readiness states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LauncherPodState {
    /// Pod scheduled, not yet running.
    Pending,
    /// Launcher socket bound, awaiting Sync.
    Ready,
    /// Active domain, qemu running.
    Running,
    /// Domain terminating or failed.
    Terminating,
}

impl LauncherPodState {
    /// Whether this state can accept the given command without an
    /// intermediate transition.
    pub fn accepts(&self, cmd: LauncherCommand) -> bool {
        use LauncherCommand::*;
        use LauncherPodState::*;
        match (self, cmd) {
            (Pending, _) => false,
            (Ready, Sync) => true,
            (Running, Sync | Shutdown | Pause | Unpause | Freeze | Unfreeze | Migrate | CancelMigration) => true,
            (Running, Kill) => true,
            (Terminating, Kill) => true,
            _ => false,
        }
    }
}

/// The controller queue entry. Each VMI gets at most one in-flight entry.
#[derive(Debug, Clone)]
pub struct WorkItem {
    pub vmi_namespace: String,
    pub vmi_name: String,
    pub desired_phase: VmPhase,
    pub observed_phase: VmPhase,
    pub launcher_state: LauncherPodState,
}

impl WorkItem {
    /// Compute the next command to issue, or `None` if nothing to do.
    /// Respects launcher-pod state — a Pending launcher gets no commands.
    pub fn next_command(&self) -> Option<LauncherCommand> {
        let cmd = decide_command(self.desired_phase, self.observed_phase)?;
        if self.launcher_state.accepts(cmd) {
            Some(cmd)
        } else {
            None
        }
    }
}

/// Compute observed phase from VMI status. Upstream stores it as a free
/// string; we map it to our `VmPhase` enum.
pub fn observed_phase(vmi: &VirtualMachineInstance) -> VmPhase {
    let status_phase = vmi
        .status
        .as_ref()
        .map(|s| s.phase.as_str())
        .unwrap_or("Pending");
    match status_phase {
        "Pending" | "Scheduling" | "Scheduled" => VmPhase::Stopped,
        "Running" => VmPhase::Running,
        "Succeeded" => VmPhase::Stopped,
        "Failed" => VmPhase::Error,
        "Unknown" => VmPhase::Error,
        "Migrating" => VmPhase::Migrating,
        _ => VmPhase::Stopped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_client_method_names() {
        assert_eq!(LauncherCommand::Sync.cmd_client_method(), "SyncVMI");
        assert_eq!(LauncherCommand::Shutdown.cmd_client_method(), "ShutdownVMI");
        assert_eq!(LauncherCommand::Kill.cmd_client_method(), "KillVMI");
        assert_eq!(LauncherCommand::Migrate.cmd_client_method(), "MigrateVMI");
    }

    #[test]
    fn decide_command_starting_from_stopped() {
        assert_eq!(
            decide_command(VmPhase::Starting, VmPhase::Stopped),
            Some(LauncherCommand::Sync)
        );
        assert_eq!(
            decide_command(VmPhase::Starting, VmPhase::Error),
            Some(LauncherCommand::Sync)
        );
    }

    #[test]
    fn decide_command_shutdown_graceful() {
        assert_eq!(
            decide_command(VmPhase::Stopping, VmPhase::Running),
            Some(LauncherCommand::Shutdown)
        );
    }

    #[test]
    fn decide_command_terminating_is_kill() {
        assert_eq!(
            decide_command(VmPhase::Terminating, VmPhase::Running),
            Some(LauncherCommand::Kill)
        );
        assert_eq!(
            decide_command(VmPhase::Terminating, VmPhase::Stopped),
            Some(LauncherCommand::Kill)
        );
    }

    #[test]
    fn decide_command_in_sync_is_none() {
        assert_eq!(decide_command(VmPhase::Running, VmPhase::Running), None);
        assert_eq!(decide_command(VmPhase::Stopped, VmPhase::Stopped), None);
    }

    #[test]
    fn fingerprint_labels_unschedulable_when_no_kvm() {
        let f = NodeFingerprint::default();
        assert!(!f.is_schedulable());
        let labels = f.to_labels();
        assert_eq!(labels.get("kubevirt.io/schedulable"), Some(&"false".into()));
    }

    #[test]
    fn fingerprint_labels_schedulable_when_kvm() {
        let f = NodeFingerprint {
            kvm_present: true,
            cpu_vendor: "GenuineIntel".into(),
            cpu_model: "Haswell".into(),
            features: vec!["aes".into(), "sse4_2".into()],
            kernel_version: "6.10.0".into(),
            hostname: "node-1".into(),
        };
        assert!(f.is_schedulable());
        let labels = f.to_labels();
        assert_eq!(labels.get("kubevirt.io/schedulable"), Some(&"true".into()));
        assert_eq!(
            labels.get("cpu-vendor.node.kubevirt.io"),
            Some(&"GenuineIntel".into())
        );
        assert_eq!(
            labels.get("cpu-model.node.kubevirt.io/Haswell"),
            Some(&"true".into())
        );
        assert_eq!(
            labels.get("cpu-feature.node.kubevirt.io/aes"),
            Some(&"true".into())
        );
        assert_eq!(
            labels.get("cpu-feature.node.kubevirt.io/sse4_2"),
            Some(&"true".into())
        );
        assert_eq!(
            labels.get("kernel-version.node.kubevirt.io"),
            Some(&"6.10.0".into())
        );
    }

    #[test]
    fn heartbeat_freshness() {
        let hb = Heartbeat {
            node_name: "n1".into(),
            timestamp_unix: 1000,
            vmi_count: 2,
            maintenance: false,
        };
        assert!(hb.is_fresh(1010, 30));
        assert!(!hb.is_fresh(1100, 30));
    }

    #[test]
    fn launcher_state_pending_rejects_everything() {
        for cmd in &[
            LauncherCommand::Sync,
            LauncherCommand::Shutdown,
            LauncherCommand::Kill,
        ] {
            assert!(!LauncherPodState::Pending.accepts(*cmd));
        }
    }

    #[test]
    fn launcher_state_ready_accepts_only_sync() {
        assert!(LauncherPodState::Ready.accepts(LauncherCommand::Sync));
        assert!(!LauncherPodState::Ready.accepts(LauncherCommand::Shutdown));
        assert!(!LauncherPodState::Ready.accepts(LauncherCommand::Migrate));
    }

    #[test]
    fn launcher_state_running_accepts_lifecycle() {
        for cmd in &[
            LauncherCommand::Sync,
            LauncherCommand::Shutdown,
            LauncherCommand::Kill,
            LauncherCommand::Pause,
            LauncherCommand::Migrate,
        ] {
            assert!(LauncherPodState::Running.accepts(*cmd));
        }
    }

    #[test]
    fn launcher_state_terminating_only_kill() {
        assert!(LauncherPodState::Terminating.accepts(LauncherCommand::Kill));
        assert!(!LauncherPodState::Terminating.accepts(LauncherCommand::Sync));
    }

    #[test]
    fn work_item_pending_emits_nothing() {
        let w = WorkItem {
            vmi_namespace: "default".into(),
            vmi_name: "vm-1".into(),
            desired_phase: VmPhase::Starting,
            observed_phase: VmPhase::Stopped,
            launcher_state: LauncherPodState::Pending,
        };
        assert_eq!(w.next_command(), None);
    }

    #[test]
    fn work_item_ready_emits_sync() {
        let w = WorkItem {
            vmi_namespace: "default".into(),
            vmi_name: "vm-1".into(),
            desired_phase: VmPhase::Starting,
            observed_phase: VmPhase::Stopped,
            launcher_state: LauncherPodState::Ready,
        };
        assert_eq!(w.next_command(), Some(LauncherCommand::Sync));
    }

    #[test]
    fn work_item_running_emits_shutdown_for_stop() {
        let w = WorkItem {
            vmi_namespace: "default".into(),
            vmi_name: "vm-1".into(),
            desired_phase: VmPhase::Stopping,
            observed_phase: VmPhase::Running,
            launcher_state: LauncherPodState::Running,
        };
        assert_eq!(w.next_command(), Some(LauncherCommand::Shutdown));
    }

    #[test]
    fn observed_phase_maps_status() {
        let mut vmi = VirtualMachineInstance::default();
        vmi.status = Some(crate::models::VirtualMachineInstanceStatus {
            phase: "Running".into(),
            ..Default::default()
        });
        assert_eq!(observed_phase(&vmi), VmPhase::Running);
    }

    #[test]
    fn observed_phase_pending_is_stopped() {
        let mut vmi = VirtualMachineInstance::default();
        vmi.status = Some(crate::models::VirtualMachineInstanceStatus {
            phase: "Pending".into(),
            ..Default::default()
        });
        assert_eq!(observed_phase(&vmi), VmPhase::Stopped);
    }

    #[test]
    fn observed_phase_failed_is_error() {
        let mut vmi = VirtualMachineInstance::default();
        vmi.status = Some(crate::models::VirtualMachineInstanceStatus {
            phase: "Failed".into(),
            ..Default::default()
        });
        assert_eq!(observed_phase(&vmi), VmPhase::Error);
    }

    #[test]
    fn observed_phase_migrating() {
        let mut vmi = VirtualMachineInstance::default();
        vmi.status = Some(crate::models::VirtualMachineInstanceStatus {
            phase: "Migrating".into(),
            ..Default::default()
        });
        assert_eq!(observed_phase(&vmi), VmPhase::Migrating);
    }

    #[test]
    fn observed_phase_no_status_defaults_to_stopped() {
        let vmi = VirtualMachineInstance::default();
        assert_eq!(observed_phase(&vmi), VmPhase::Stopped);
    }
}
