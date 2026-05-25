// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! virt-launcher: per-VMI pod runner.
//!
//! Upstream: kubevirt/kubevirt v1.8.2
//!   cmd/virt-launcher/virt-launcher.go (entrypoint)
//!   pkg/virt-launcher/virtwrap/manager/manager.go (DomainManager)
//!   pkg/virt-launcher/notify-server/notify-server.go (gRPC notify)
//!
//! Each VMI runs in its own pod with a single container — virt-launcher.
//! The launcher receives lifecycle commands over a Unix-domain socket from
//! virt-handler, drives qemu via libvirt, and emits status notifications
//! back to virt-handler over a notify socket. This module captures the
//! socket protocol shape, the DomainManager state machine, and the
//! notify-event taxonomy. The actual libvirt RPC glue lives in
//! cave-runtime's host-preflight layer.

use crate::libvirt::{emit_domain_xml, EmitOptions};
use crate::models::{VirtualMachineInstance, VmPhase};
use serde::{Deserialize, Serialize};

/// Unix-domain-socket paths the launcher opens. virt-handler connects in.
pub const CMD_SOCKET_PATH: &str = "/var/run/kubevirt/sockets/launcher.sock";
pub const NOTIFY_SOCKET_PATH: &str = "/var/run/kubevirt/sockets/launcher-notify.sock";

/// The DomainManager state. Mirrors `pkg/virt-launcher/virtwrap/manager`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LauncherState {
    /// Process up, libvirt connection not yet established.
    Booting,
    /// Connected to libvirt, awaiting a Sync command.
    Idle,
    /// Domain defined + started.
    Running,
    /// Domain paused via libvirt suspend.
    Paused,
    /// In-flight live migration (source side).
    Migrating,
    /// Graceful shutdown in progress.
    ShuttingDown,
    /// Hard stop in progress.
    Killing,
    /// Process exiting.
    Exited,
}

impl LauncherState {
    /// Whether the state should report "ready" to the readiness probe.
    pub fn is_ready(&self) -> bool {
        matches!(self, LauncherState::Idle | LauncherState::Running | LauncherState::Paused)
    }

    /// Whether libvirt's domain object exists in this state.
    pub fn has_domain(&self) -> bool {
        matches!(
            self,
            LauncherState::Running
                | LauncherState::Paused
                | LauncherState::Migrating
                | LauncherState::ShuttingDown
        )
    }
}

/// Notify-server event sent from the launcher to virt-handler.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NotifyEvent {
    /// Domain object created in libvirt.
    DomainDefined { uuid: String },
    /// Domain started, qemu running.
    DomainStarted { uuid: String },
    /// Domain shutdown signalled.
    DomainShutdown { uuid: String },
    /// Domain crashed or qemu exited unexpectedly.
    DomainCrashed { uuid: String, reason: String },
    /// Live migration completed on the source.
    MigrationCompleted { uuid: String, target_node: String },
    /// Periodic heartbeat from the launcher.
    Heartbeat { uuid: String, state: LauncherState },
}

impl NotifyEvent {
    pub fn uuid(&self) -> &str {
        match self {
            NotifyEvent::DomainDefined { uuid }
            | NotifyEvent::DomainStarted { uuid }
            | NotifyEvent::DomainShutdown { uuid }
            | NotifyEvent::DomainCrashed { uuid, .. }
            | NotifyEvent::MigrationCompleted { uuid, .. }
            | NotifyEvent::Heartbeat { uuid, .. } => uuid,
        }
    }

    /// What VmPhase this event implies for the parent VMI.
    pub fn implied_phase(&self) -> VmPhase {
        match self {
            NotifyEvent::DomainDefined { .. } => VmPhase::Starting,
            NotifyEvent::DomainStarted { .. } => VmPhase::Running,
            NotifyEvent::DomainShutdown { .. } => VmPhase::Stopping,
            NotifyEvent::DomainCrashed { .. } => VmPhase::Error,
            NotifyEvent::MigrationCompleted { .. } => VmPhase::Running,
            NotifyEvent::Heartbeat { state, .. } => match state {
                LauncherState::Running => VmPhase::Running,
                LauncherState::Paused => VmPhase::Running,
                LauncherState::Migrating => VmPhase::Migrating,
                LauncherState::ShuttingDown => VmPhase::Stopping,
                LauncherState::Killing | LauncherState::Exited => VmPhase::Terminating,
                _ => VmPhase::Stopped,
            },
        }
    }
}

/// Bundled output of the prepare-to-run path. The launcher computes this
/// before issuing the libvirt `defineXML` RPC.
#[derive(Debug, Clone)]
pub struct PreparedDomain {
    pub domain_xml: String,
    pub socket_paths: SocketPaths,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketPaths {
    pub cmd: String,
    pub notify: String,
    pub console: String,
}

impl Default for SocketPaths {
    fn default() -> Self {
        Self {
            cmd: CMD_SOCKET_PATH.into(),
            notify: NOTIFY_SOCKET_PATH.into(),
            console: "/var/run/kubevirt/sockets/console.sock".into(),
        }
    }
}

/// Prepare a domain for launch. Produces deterministic XML and the canonical
/// socket paths. Pure function — no I/O, safe to call from tests.
pub fn prepare_domain(vmi: &VirtualMachineInstance, opts: &EmitOptions) -> PreparedDomain {
    let domain_xml = emit_domain_xml(vmi, opts);
    PreparedDomain {
        domain_xml,
        socket_paths: SocketPaths::default(),
    }
}

/// State-transition decision: given the current launcher state + an
/// incoming command, return the next state. Mirrors the DomainManager
/// state-machine in upstream.
pub fn next_state(current: LauncherState, command: LauncherCommand) -> LauncherState {
    use LauncherCommand::*;
    use LauncherState::*;
    match (current, command) {
        (Booting, _) => Booting,
        (Idle, Sync) => Running,
        (Running, Pause) => Paused,
        (Paused, Unpause) => Running,
        (Running, Migrate) => Migrating,
        (Migrating, MigrationCompleted) => Running,
        (Migrating, MigrationFailed) => Running,
        (Running, Shutdown) => ShuttingDown,
        (Paused, Shutdown) => ShuttingDown,
        (ShuttingDown, Kill) | (Running, Kill) | (Paused, Kill) | (Migrating, Kill) => Killing,
        (Killing, _) | (Exited, _) => Exited,
        (s, _) => s,
    }
}

/// Commands the DomainManager observes. Includes internal transitions
/// (MigrationCompleted, MigrationFailed) the launcher uses to drive
/// itself after libvirt callback delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LauncherCommand {
    Sync,
    Pause,
    Unpause,
    Shutdown,
    Kill,
    Migrate,
    MigrationCompleted,
    MigrationFailed,
}

/// Compute the launch-time UUID for a VMI. Stable across launches so
/// libvirt object identity is preserved.
pub fn launch_uuid(vmi: &VirtualMachineInstance) -> String {
    let ns = vmi.namespace.as_deref().unwrap_or("default");
    let mut h: u64 = 0xcbf29ce484222325;
    for b in ns.bytes().chain(vmi.name.bytes()) {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        (h >> 32) as u32,
        (h >> 16) as u16,
        h as u16,
        (h.rotate_left(13)) as u16,
        h & 0xFFFFFFFFFFFF
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Domain;

    fn vmi() -> VirtualMachineInstance {
        let mut v = VirtualMachineInstance::default();
        v.name = "vm-1".into();
        v.namespace = Some("default".into());
        v.spec.domain = Domain::default();
        v
    }

    #[test]
    fn socket_paths_default_under_kubevirt_sockets() {
        let p = SocketPaths::default();
        assert!(p.cmd.contains("kubevirt/sockets"));
        assert!(p.notify.contains("kubevirt/sockets"));
        assert!(p.console.contains("kubevirt/sockets"));
    }

    #[test]
    fn prepare_domain_emits_xml() {
        let p = prepare_domain(&vmi(), &EmitOptions::default());
        assert!(p.domain_xml.starts_with("<domain type='kvm'>"));
        assert!(p.domain_xml.contains("</domain>"));
    }

    #[test]
    fn state_is_ready_when_idle_or_running() {
        assert!(LauncherState::Idle.is_ready());
        assert!(LauncherState::Running.is_ready());
        assert!(LauncherState::Paused.is_ready());
        assert!(!LauncherState::Booting.is_ready());
        assert!(!LauncherState::Exited.is_ready());
    }

    #[test]
    fn state_has_domain() {
        assert!(!LauncherState::Booting.has_domain());
        assert!(!LauncherState::Idle.has_domain());
        assert!(LauncherState::Running.has_domain());
        assert!(LauncherState::Paused.has_domain());
        assert!(LauncherState::Migrating.has_domain());
    }

    #[test]
    fn next_state_idle_to_running_on_sync() {
        assert_eq!(
            next_state(LauncherState::Idle, LauncherCommand::Sync),
            LauncherState::Running
        );
    }

    #[test]
    fn next_state_running_to_paused() {
        assert_eq!(
            next_state(LauncherState::Running, LauncherCommand::Pause),
            LauncherState::Paused
        );
    }

    #[test]
    fn next_state_migration_round_trip() {
        let mut s = LauncherState::Running;
        s = next_state(s, LauncherCommand::Migrate);
        assert_eq!(s, LauncherState::Migrating);
        s = next_state(s, LauncherCommand::MigrationCompleted);
        assert_eq!(s, LauncherState::Running);
    }

    #[test]
    fn next_state_kill_overrides_anything() {
        for from in &[
            LauncherState::Running,
            LauncherState::Paused,
            LauncherState::Migrating,
            LauncherState::ShuttingDown,
        ] {
            assert_eq!(
                next_state(*from, LauncherCommand::Kill),
                LauncherState::Killing
            );
        }
    }

    #[test]
    fn next_state_exited_is_terminal() {
        for cmd in &[
            LauncherCommand::Sync,
            LauncherCommand::Shutdown,
            LauncherCommand::Kill,
        ] {
            assert_eq!(next_state(LauncherState::Exited, *cmd), LauncherState::Exited);
        }
    }

    #[test]
    fn notify_event_uuid_extraction() {
        let e = NotifyEvent::DomainStarted {
            uuid: "abc-def".into(),
        };
        assert_eq!(e.uuid(), "abc-def");
    }

    #[test]
    fn notify_implied_phase_running() {
        let e = NotifyEvent::DomainStarted {
            uuid: "u".into(),
        };
        assert_eq!(e.implied_phase(), VmPhase::Running);
    }

    #[test]
    fn notify_implied_phase_crashed_is_error() {
        let e = NotifyEvent::DomainCrashed {
            uuid: "u".into(),
            reason: "qemu exited".into(),
        };
        assert_eq!(e.implied_phase(), VmPhase::Error);
    }

    #[test]
    fn notify_implied_phase_heartbeat_running() {
        let e = NotifyEvent::Heartbeat {
            uuid: "u".into(),
            state: LauncherState::Running,
        };
        assert_eq!(e.implied_phase(), VmPhase::Running);
    }

    #[test]
    fn notify_implied_phase_heartbeat_exited_terminating() {
        let e = NotifyEvent::Heartbeat {
            uuid: "u".into(),
            state: LauncherState::Exited,
        };
        assert_eq!(e.implied_phase(), VmPhase::Terminating);
    }

    #[test]
    fn notify_event_serde_round_trip() {
        let e = NotifyEvent::MigrationCompleted {
            uuid: "u".into(),
            target_node: "node-2".into(),
        };
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains("\"kind\":\"migration_completed\""));
        let back: NotifyEvent = serde_json::from_str(&s).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn launch_uuid_stable_across_calls() {
        let v = vmi();
        let a = launch_uuid(&v);
        let b = launch_uuid(&v);
        assert_eq!(a, b);
        assert_eq!(a.len(), 36); // standard UUID format
        assert_eq!(a.matches('-').count(), 4);
    }

    #[test]
    fn launch_uuid_differs_per_vmi() {
        let mut a = vmi();
        let mut b = vmi();
        a.name = "vm-a".into();
        b.name = "vm-b".into();
        assert_ne!(launch_uuid(&a), launch_uuid(&b));
    }
}
