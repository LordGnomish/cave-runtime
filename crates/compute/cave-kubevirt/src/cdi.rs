// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CDI — Containerized Data Importer populator.
//!
//! Upstream: kubevirt/containerized-data-importer (companion repo)
//!   pkg/apis/core/v1beta1/types.go (DataVolume API)
//!   pkg/controller/datavolume-controller.go (DataVolumeReconciler)
//!
//! CDI populates PVCs with VM-disk images sourced from URLs, container
//! registries, other PVCs (clone), HTTP endpoints, or upload sessions. The
//! `DataVolume` CRD is the user-facing handle; CDI's controller creates the
//! underlying PVC + importer/cloner pod.
//!
//! This module captures the DataVolume status phase enum, the source
//! taxonomy, and the reconcile decision table that the data-volume
//! controller dispatches against.

use crate::models::{DataVolume, DataVolumeSource};
use serde::{Deserialize, Serialize};

/// DataVolume reconcile phase. Mirrors the upstream `DataVolumePhase`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataVolumePhase {
    /// DataVolume created, not yet scheduled.
    Pending,
    /// PVC created, awaiting binding.
    PvcBound,
    /// Source preparation in progress (image probe / authorization).
    ImportScheduled,
    /// Importer pod scheduled.
    ImportInProgress,
    /// Importer pod completed successfully.
    Succeeded,
    /// Cloning from another PVC.
    CloneScheduled,
    /// Clone in flight.
    CloneInProgress,
    /// Upload session open, awaiting POST.
    UploadScheduled,
    /// Upload in flight.
    UploadInProgress,
    /// Import / clone / upload failed.
    Failed,
    /// User-cancelled.
    Aborted,
}

impl DataVolumePhase {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            DataVolumePhase::Succeeded | DataVolumePhase::Failed | DataVolumePhase::Aborted
        )
    }

    pub fn is_in_progress(&self) -> bool {
        matches!(
            self,
            DataVolumePhase::ImportInProgress
                | DataVolumePhase::CloneInProgress
                | DataVolumePhase::UploadInProgress
        )
    }
}

/// Recognised source kinds for `DataVolume.spec.source.kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// HTTP(S) URL (qcow2 / raw image).
    Http,
    /// Container registry image (containerDisk).
    Registry,
    /// Clone from another PVC in the same namespace.
    Pvc,
    /// Cross-namespace clone via DataSource ref.
    DataSource,
    /// Upload session (kubectl virt image-upload).
    Upload,
    /// Empty volume (no source).
    Blank,
}

impl SourceKind {
    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "http" | "https" => SourceKind::Http,
            "registry" | "containerdisk" => SourceKind::Registry,
            "pvc" => SourceKind::Pvc,
            "datasource" | "datasource-ref" => SourceKind::DataSource,
            "upload" => SourceKind::Upload,
            "blank" => SourceKind::Blank,
            _ => return None,
        })
    }

    /// The initial phase the reconciler should set when picking up a fresh
    /// DataVolume with this source.
    pub fn initial_phase(&self) -> DataVolumePhase {
        match self {
            SourceKind::Http | SourceKind::Registry => DataVolumePhase::ImportScheduled,
            SourceKind::Pvc | SourceKind::DataSource => DataVolumePhase::CloneScheduled,
            SourceKind::Upload => DataVolumePhase::UploadScheduled,
            SourceKind::Blank => DataVolumePhase::PvcBound,
        }
    }

    /// The terminal phase when the source's worker completes.
    pub fn completion_phase(&self) -> DataVolumePhase {
        DataVolumePhase::Succeeded
    }

    /// The progress phase associated with this source.
    pub fn progress_phase(&self) -> Option<DataVolumePhase> {
        match self {
            SourceKind::Http | SourceKind::Registry => Some(DataVolumePhase::ImportInProgress),
            SourceKind::Pvc | SourceKind::DataSource => Some(DataVolumePhase::CloneInProgress),
            SourceKind::Upload => Some(DataVolumePhase::UploadInProgress),
            SourceKind::Blank => None,
        }
    }
}

/// Classify a DataVolume's declared source into a `SourceKind`. Unknown
/// kinds map to `None`; the reconciler emits a `Failed` event.
pub fn classify_source(s: &DataVolumeSource) -> Option<SourceKind> {
    SourceKind::from_str(&s.kind)
}

/// Reconcile decision for a single DataVolume tick.
#[derive(Debug, Clone, PartialEq)]
pub enum ReconcileAction {
    /// No change needed.
    Noop,
    /// Create the underlying PVC if not yet present.
    CreatePvc,
    /// Schedule the worker pod (importer / cloner / upload-receiver).
    SchedulePod {
        kind: SourceKind,
    },
    /// Advance to the next phase.
    AdvancePhase {
        next: DataVolumePhase,
    },
    /// Reject this DataVolume with an unknown-source error.
    RejectUnknownSource,
}

/// One-shot reconcile. Inputs: the DataVolume, plus observed booleans about
/// the world state. Output: the action.
pub fn reconcile(
    dv: &DataVolume,
    pvc_exists: bool,
    worker_done: bool,
    worker_failed: bool,
) -> ReconcileAction {
    let kind = match classify_source(&dv.spec.source) {
        Some(k) => k,
        None => return ReconcileAction::RejectUnknownSource,
    };
    let phase = dv
        .status
        .as_ref()
        .and_then(|s| serde_json::from_str::<DataVolumePhase>(&format!("\"{}\"", s.phase)).ok())
        .unwrap_or(DataVolumePhase::Pending);

    if worker_failed {
        return ReconcileAction::AdvancePhase {
            next: DataVolumePhase::Failed,
        };
    }
    if worker_done && !phase.is_terminal() {
        return ReconcileAction::AdvancePhase {
            next: kind.completion_phase(),
        };
    }
    match (phase, pvc_exists) {
        (DataVolumePhase::Pending, false) => ReconcileAction::CreatePvc,
        (DataVolumePhase::Pending, true) => ReconcileAction::AdvancePhase {
            next: kind.initial_phase(),
        },
        (DataVolumePhase::PvcBound, _) if kind != SourceKind::Blank => {
            ReconcileAction::SchedulePod { kind }
        }
        (DataVolumePhase::ImportScheduled, _)
        | (DataVolumePhase::CloneScheduled, _)
        | (DataVolumePhase::UploadScheduled, _) => kind
            .progress_phase()
            .map(|p| ReconcileAction::AdvancePhase { next: p })
            .unwrap_or(ReconcileAction::Noop),
        _ => ReconcileAction::Noop,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{DataVolume, DataVolumeSource, DataVolumeSpec, DataVolumeStatus};

    fn dv(source_kind: &str, status_phase: Option<&str>) -> DataVolume {
        let mut v = DataVolume::default();
        v.name = "dv-1".into();
        v.namespace = Some("default".into());
        v.spec = DataVolumeSpec {
            source: DataVolumeSource {
                kind: source_kind.into(),
                spec: serde_json::json!({}),
            },
            ..Default::default()
        };
        v.status = status_phase.map(|p| DataVolumeStatus {
            phase: p.into(),
            ..Default::default()
        });
        v
    }

    #[test]
    fn classify_known_kinds() {
        assert_eq!(SourceKind::from_str("http"), Some(SourceKind::Http));
        assert_eq!(SourceKind::from_str("HTTPS"), Some(SourceKind::Http));
        assert_eq!(
            SourceKind::from_str("Registry"),
            Some(SourceKind::Registry)
        );
        assert_eq!(SourceKind::from_str("pvc"), Some(SourceKind::Pvc));
        assert_eq!(SourceKind::from_str("upload"), Some(SourceKind::Upload));
        assert_eq!(SourceKind::from_str("blank"), Some(SourceKind::Blank));
    }

    #[test]
    fn classify_unknown_kind() {
        assert_eq!(SourceKind::from_str("magic"), None);
    }

    #[test]
    fn initial_phase_per_source() {
        assert_eq!(
            SourceKind::Http.initial_phase(),
            DataVolumePhase::ImportScheduled
        );
        assert_eq!(
            SourceKind::Pvc.initial_phase(),
            DataVolumePhase::CloneScheduled
        );
        assert_eq!(
            SourceKind::Upload.initial_phase(),
            DataVolumePhase::UploadScheduled
        );
        assert_eq!(
            SourceKind::Blank.initial_phase(),
            DataVolumePhase::PvcBound
        );
    }

    #[test]
    fn progress_phase_per_source() {
        assert_eq!(
            SourceKind::Http.progress_phase(),
            Some(DataVolumePhase::ImportInProgress)
        );
        assert_eq!(
            SourceKind::DataSource.progress_phase(),
            Some(DataVolumePhase::CloneInProgress)
        );
        assert_eq!(SourceKind::Blank.progress_phase(), None);
    }

    #[test]
    fn terminal_classification() {
        assert!(DataVolumePhase::Succeeded.is_terminal());
        assert!(DataVolumePhase::Failed.is_terminal());
        assert!(DataVolumePhase::Aborted.is_terminal());
        assert!(!DataVolumePhase::Pending.is_terminal());
    }

    #[test]
    fn in_progress_classification() {
        assert!(DataVolumePhase::ImportInProgress.is_in_progress());
        assert!(DataVolumePhase::CloneInProgress.is_in_progress());
        assert!(DataVolumePhase::UploadInProgress.is_in_progress());
        assert!(!DataVolumePhase::Pending.is_in_progress());
    }

    #[test]
    fn reconcile_unknown_source() {
        let action = reconcile(&dv("magic", None), false, false, false);
        assert_eq!(action, ReconcileAction::RejectUnknownSource);
    }

    #[test]
    fn reconcile_pending_no_pvc_creates_pvc() {
        let action = reconcile(&dv("http", None), false, false, false);
        assert_eq!(action, ReconcileAction::CreatePvc);
    }

    #[test]
    fn reconcile_pending_with_pvc_advances_to_import() {
        let action = reconcile(&dv("http", None), true, false, false);
        assert_eq!(
            action,
            ReconcileAction::AdvancePhase {
                next: DataVolumePhase::ImportScheduled,
            }
        );
    }

    #[test]
    fn reconcile_worker_failed_short_circuits() {
        let action = reconcile(&dv("http", Some("ImportInProgress")), true, false, true);
        assert_eq!(
            action,
            ReconcileAction::AdvancePhase {
                next: DataVolumePhase::Failed,
            }
        );
    }

    #[test]
    fn reconcile_worker_done_advances_to_succeeded() {
        let action = reconcile(&dv("registry", Some("ImportInProgress")), true, true, false);
        assert_eq!(
            action,
            ReconcileAction::AdvancePhase {
                next: DataVolumePhase::Succeeded,
            }
        );
    }

    #[test]
    fn reconcile_already_succeeded_noop() {
        let action = reconcile(&dv("registry", Some("Succeeded")), true, true, false);
        assert_eq!(action, ReconcileAction::Noop);
    }

    #[test]
    fn classify_source_returns_some() {
        let src = DataVolumeSource {
            kind: "http".into(),
            spec: serde_json::json!({}),
        };
        assert_eq!(classify_source(&src), Some(SourceKind::Http));
    }
}
