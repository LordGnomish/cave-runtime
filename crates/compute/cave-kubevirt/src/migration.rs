// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Live migration controller — `VirtualMachineInstanceMigration` CRD.
//!
//! Upstream: kubevirt/kubevirt v1.8.2
//!   staging/src/kubevirt.io/api/core/v1/types.go (VirtualMachineInstanceMigration)
//!   pkg/virt-controller/watch/migration.go (MigrationController)
//!
//! A migration is a single ephemeral resource that drives a one-shot live
//! migration of a single VMI from its current node to a target node.
//! Phases mirror the upstream `MigrationPhase` enum.

use serde::{Deserialize, Serialize};

/// Migration lifecycle phase. Mirrors `kubevirt.io/api/core/v1.MigrationPhase`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MigrationPhase {
    /// Resource created, awaiting controller.
    Pending,
    /// Target launcher pod scheduled.
    Scheduling,
    /// Target pod ready, awaiting libvirt handshake.
    PreparingTarget,
    /// Source acknowledged target; libvirt migration in flight.
    TargetReady,
    /// Domain transfer active.
    Running,
    /// Source switched off, target took over.
    Succeeded,
    /// Migration aborted; source retained.
    Failed,
}

impl MigrationPhase {
    /// Whether the migration has finished (succeeded or failed). The
    /// controller stops reconciling terminal migrations.
    pub fn is_terminal(&self) -> bool {
        matches!(self, MigrationPhase::Succeeded | MigrationPhase::Failed)
    }

    /// Whether the source VMI should still be considered running by the
    /// VMI controller during this phase.
    pub fn source_still_running(&self) -> bool {
        !matches!(self, MigrationPhase::Succeeded)
    }
}

/// The migration CRD object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualMachineInstanceMigration {
    pub name: String,
    pub namespace: Option<String>,
    pub spec: MigrationSpec,
    pub status: Option<MigrationStatus>,
}

impl Default for VirtualMachineInstanceMigration {
    fn default() -> Self {
        Self {
            name: String::new(),
            namespace: None,
            spec: MigrationSpec::default(),
            status: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationSpec {
    /// The VMI being migrated, in this namespace.
    pub vmi_name: String,
    /// Optional target-node preference. Empty = scheduler picks.
    pub target_node: Option<String>,
    /// Bandwidth cap in MiB/s. None = uncapped.
    pub bandwidth_mbits_per_sec: Option<u32>,
    /// Whether auto-converge should be used (libvirt slows the guest down
    /// when migration is falling behind dirty page rate).
    pub auto_converge: bool,
    /// Whether post-copy should be enabled (transfer happens after CPU
    /// has switched to the target).
    pub allow_post_copy: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationStatus {
    pub phase: Option<MigrationPhase>,
    pub source_node: Option<String>,
    pub target_node: Option<String>,
    pub start_timestamp_unix: Option<i64>,
    pub end_timestamp_unix: Option<i64>,
    pub migration_uid: Option<String>,
    pub conditions: Vec<MigrationCondition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationCondition {
    pub kind: String,
    pub status: String,
    pub reason: Option<String>,
    pub message: Option<String>,
}

/// State-machine transition: given the current phase + an external trigger,
/// return the next phase. Mirrors `pkg/virt-controller/watch/migration.go`
/// phase progression.
pub fn next_phase(current: MigrationPhase, trigger: MigrationTrigger) -> MigrationPhase {
    use MigrationPhase::*;
    use MigrationTrigger::*;
    match (current, trigger) {
        (Pending, ControllerStart) => Scheduling,
        (Scheduling, TargetPodReady) => PreparingTarget,
        (PreparingTarget, TargetHandshakeOk) => TargetReady,
        (TargetReady, LibvirtStarted) => Running,
        (Running, LibvirtCompleted) => Succeeded,
        (Running, LibvirtFailed) => Failed,
        (Running, Abort) => Failed,
        (Scheduling | PreparingTarget | TargetReady, TargetPodFailed) => Failed,
        (current, _) => current,
    }
}

/// External signals that advance the migration phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationTrigger {
    /// Migration object created and picked up by the controller.
    ControllerStart,
    /// Target launcher pod transitioned to Ready.
    TargetPodReady,
    /// Target pod failed to start (image pull error, schedule failure).
    TargetPodFailed,
    /// libvirt target-side `prepareMigration` finished.
    TargetHandshakeOk,
    /// libvirt initiated the migration on the source.
    LibvirtStarted,
    /// libvirt source-side reported the migration as complete.
    LibvirtCompleted,
    /// libvirt source-side reported a fatal migration error.
    LibvirtFailed,
    /// User or controller aborted the migration.
    Abort,
}

/// In-memory registry of migration objects. Matches `Store`'s pattern.
#[derive(Debug, Default)]
pub struct MigrationStore {
    entries: std::sync::RwLock<
        std::collections::HashMap<String, VirtualMachineInstanceMigration>,
    >,
}

impl MigrationStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put(&self, m: VirtualMachineInstanceMigration) {
        let key = format!(
            "{}/{}",
            m.namespace.clone().unwrap_or_else(|| "default".into()),
            m.name
        );
        self.entries.write().unwrap().insert(key, m);
    }

    pub fn get(&self, namespace: &str, name: &str) -> Option<VirtualMachineInstanceMigration> {
        self.entries
            .read()
            .unwrap()
            .get(&format!("{namespace}/{name}"))
            .cloned()
    }

    pub fn list_for_vmi(&self, namespace: &str, vmi_name: &str) -> Vec<VirtualMachineInstanceMigration> {
        self.entries
            .read()
            .unwrap()
            .values()
            .filter(|m| m.namespace.as_deref().unwrap_or("default") == namespace && m.spec.vmi_name == vmi_name)
            .cloned()
            .collect()
    }

    pub fn delete(&self, namespace: &str, name: &str) -> bool {
        self.entries
            .write()
            .unwrap()
            .remove(&format!("{namespace}/{name}"))
            .is_some()
    }

    /// Advance the phase of the given migration in place. Returns the new
    /// phase, or `None` if the migration does not exist.
    pub fn advance(
        &self,
        namespace: &str,
        name: &str,
        trigger: MigrationTrigger,
    ) -> Option<MigrationPhase> {
        let mut guard = self.entries.write().unwrap();
        let m = guard.get_mut(&format!("{namespace}/{name}"))?;
        let current = m
            .status
            .as_ref()
            .and_then(|s| s.phase)
            .unwrap_or(MigrationPhase::Pending);
        let next = next_phase(current, trigger);
        let status = m.status.get_or_insert(MigrationStatus::default());
        status.phase = Some(next);
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(name: &str, vmi: &str) -> VirtualMachineInstanceMigration {
        VirtualMachineInstanceMigration {
            name: name.into(),
            namespace: Some("default".into()),
            spec: MigrationSpec {
                vmi_name: vmi.into(),
                ..Default::default()
            },
            status: Some(MigrationStatus {
                phase: Some(MigrationPhase::Pending),
                ..Default::default()
            }),
        }
    }

    #[test]
    fn is_terminal_only_on_succeeded_or_failed() {
        assert!(MigrationPhase::Succeeded.is_terminal());
        assert!(MigrationPhase::Failed.is_terminal());
        assert!(!MigrationPhase::Pending.is_terminal());
        assert!(!MigrationPhase::Running.is_terminal());
    }

    #[test]
    fn source_running_until_succeeded() {
        assert!(MigrationPhase::Pending.source_still_running());
        assert!(MigrationPhase::Running.source_still_running());
        assert!(MigrationPhase::Failed.source_still_running());
        assert!(!MigrationPhase::Succeeded.source_still_running());
    }

    #[test]
    fn next_phase_pending_to_scheduling() {
        assert_eq!(
            next_phase(MigrationPhase::Pending, MigrationTrigger::ControllerStart),
            MigrationPhase::Scheduling
        );
    }

    #[test]
    fn next_phase_full_happy_path() {
        let mut p = MigrationPhase::Pending;
        let path: &[(MigrationTrigger, MigrationPhase)] = &[
            (MigrationTrigger::ControllerStart, MigrationPhase::Scheduling),
            (MigrationTrigger::TargetPodReady, MigrationPhase::PreparingTarget),
            (MigrationTrigger::TargetHandshakeOk, MigrationPhase::TargetReady),
            (MigrationTrigger::LibvirtStarted, MigrationPhase::Running),
            (MigrationTrigger::LibvirtCompleted, MigrationPhase::Succeeded),
        ];
        for (t, expected) in path {
            p = next_phase(p, *t);
            assert_eq!(p, *expected);
        }
    }

    #[test]
    fn next_phase_running_libvirt_failed_goes_failed() {
        assert_eq!(
            next_phase(MigrationPhase::Running, MigrationTrigger::LibvirtFailed),
            MigrationPhase::Failed
        );
    }

    #[test]
    fn next_phase_target_pod_failed_during_setup() {
        for setup in &[
            MigrationPhase::Scheduling,
            MigrationPhase::PreparingTarget,
            MigrationPhase::TargetReady,
        ] {
            assert_eq!(
                next_phase(*setup, MigrationTrigger::TargetPodFailed),
                MigrationPhase::Failed
            );
        }
    }

    #[test]
    fn next_phase_abort_during_running_is_failed() {
        assert_eq!(
            next_phase(MigrationPhase::Running, MigrationTrigger::Abort),
            MigrationPhase::Failed
        );
    }

    #[test]
    fn terminal_phases_do_not_advance() {
        for terminal in &[MigrationPhase::Succeeded, MigrationPhase::Failed] {
            for trigger in &[
                MigrationTrigger::ControllerStart,
                MigrationTrigger::LibvirtCompleted,
                MigrationTrigger::Abort,
            ] {
                assert_eq!(next_phase(*terminal, *trigger), *terminal);
            }
        }
    }

    #[test]
    fn store_put_get_round_trip() {
        let s = MigrationStore::new();
        s.put(mk("m1", "vm-1"));
        let got = s.get("default", "m1").unwrap();
        assert_eq!(got.spec.vmi_name, "vm-1");
    }

    #[test]
    fn store_delete_removes() {
        let s = MigrationStore::new();
        s.put(mk("m1", "vm-1"));
        assert!(s.delete("default", "m1"));
        assert!(s.get("default", "m1").is_none());
    }

    #[test]
    fn store_list_for_vmi_filters() {
        let s = MigrationStore::new();
        s.put(mk("m1", "vm-a"));
        s.put(mk("m2", "vm-b"));
        s.put(mk("m3", "vm-a"));
        let for_a = s.list_for_vmi("default", "vm-a");
        assert_eq!(for_a.len(), 2);
        let for_b = s.list_for_vmi("default", "vm-b");
        assert_eq!(for_b.len(), 1);
    }

    #[test]
    fn store_advance_moves_phase() {
        let s = MigrationStore::new();
        s.put(mk("m1", "vm-1"));
        let phase = s
            .advance("default", "m1", MigrationTrigger::ControllerStart)
            .unwrap();
        assert_eq!(phase, MigrationPhase::Scheduling);
        let got = s.get("default", "m1").unwrap();
        assert_eq!(
            got.status.unwrap().phase.unwrap(),
            MigrationPhase::Scheduling
        );
    }

    #[test]
    fn store_advance_missing_returns_none() {
        let s = MigrationStore::new();
        assert!(s
            .advance("default", "missing", MigrationTrigger::ControllerStart)
            .is_none());
    }

    #[test]
    fn serde_round_trip_full_migration() {
        let m = mk("m1", "vm-1");
        let s = serde_json::to_string(&m).unwrap();
        let back: VirtualMachineInstanceMigration = serde_json::from_str(&s).unwrap();
        assert_eq!(back.name, m.name);
        assert_eq!(back.spec.vmi_name, m.spec.vmi_name);
    }
}
