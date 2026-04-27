//! Volume expansion controller — `pkg/controller/volume/expand`.
//!
//! Detects PVCs whose `spec.resources.requests.storage` exceeds their
//! `status.capacity` and drives them through:
//!
//! `Pending → Resizing (controller-side) → FileSystemResizePending
//!     → Resized` (kubelet observes the new capacity).
//!
//! Requirements:
//!
//! * StorageClass must have `allowVolumeExpansion = true`.
//! * Requested capacity must be strictly greater than current.
//! * Shrink is not allowed at the controller-manager level.

use crate::types::{Cite, ControllerError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExpansionPhase {
    /// `spec` requests the same as `status.capacity` — nothing to do.
    InSync,
    /// Controller has not started the resize.
    Pending,
    /// Resize call to the storage backend in progress.
    Resizing,
    /// Backend says capacity changed; now waiting on kubelet to grow FS.
    FileSystemResizePending,
    /// Kubelet has grown the FS; PVC.status.capacity == request.
    Resized,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NextStep {
    NoOp,
    StartControllerResize,
    AwaitFileSystemResize,
    MarkResized,
    Reject(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpansionView {
    pub pvc_name: String,
    pub spec_request_gi: u64,
    pub status_capacity_gi: u64,
    pub allow_expansion: bool,
    pub phase: ExpansionPhase,
    /// True when the controller-side resize completed (set by the CSI
    /// driver or in-tree plugin); kubelet will pick up from here.
    pub controller_resize_complete: bool,
}

/// Validate the expansion request: forbid shrink, require allowExpansion
/// for any change.
pub fn validate(view: &ExpansionView) -> Result<(), ControllerError> {
    if view.spec_request_gi < view.status_capacity_gi {
        return Err(ControllerError::InvalidSpec {
            kind: "PersistentVolumeClaim",
            reason: "shrinking volumes is not allowed".into(),
        });
    }
    if view.spec_request_gi > view.status_capacity_gi && !view.allow_expansion {
        return Err(ControllerError::InvalidSpec {
            kind: "PersistentVolumeClaim",
            reason: "storage class disallows volume expansion".into(),
        });
    }
    Ok(())
}

/// Decide the next state-machine step.
pub fn next_step(view: &ExpansionView) -> NextStep {
    if validate(view).is_err() {
        return NextStep::Reject("validation failed".into());
    }
    match view.phase {
        // No expansion in flight — only respond if there's actually a delta.
        ExpansionPhase::InSync | ExpansionPhase::Resized => NextStep::NoOp,
        ExpansionPhase::Pending => {
            if view.spec_request_gi == view.status_capacity_gi {
                NextStep::NoOp
            } else {
                NextStep::StartControllerResize
            }
        }
        ExpansionPhase::Resizing => {
            if view.controller_resize_complete {
                NextStep::AwaitFileSystemResize
            } else {
                NextStep::NoOp
            }
        }
        // FS-pending: capacity has caught up → mark Resized; else stay.
        ExpansionPhase::FileSystemResizePending => {
            if view.spec_request_gi == view.status_capacity_gi {
                NextStep::MarkResized
            } else {
                NextStep::NoOp
            }
        }
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/volume/expand/expand_controller.go",
    "ExpandController",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn view(req: u64, cap: u64, allow: bool, phase: ExpansionPhase) -> ExpansionView {
        ExpansionView {
            pvc_name: "pvc".into(),
            spec_request_gi: req,
            status_capacity_gi: cap,
            allow_expansion: allow,
            phase,
            controller_resize_complete: false,
        }
    }

    #[test]
    fn shrink_is_rejected() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/expand/expand_controller.go",
            "validate",
            "tenant-pv-exp-shrink"
        );
        let v = view(5, 10, true, ExpansionPhase::Pending);
        assert!(validate(&v).is_err());
    }

    #[test]
    fn no_change_is_in_sync() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/expand/expand_controller.go",
            "syncHandler",
            "tenant-pv-exp-no-change"
        );
        let v = view(10, 10, true, ExpansionPhase::InSync);
        assert_eq!(next_step(&v), NextStep::NoOp);
    }

    #[test]
    fn growth_without_allow_is_rejected() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/expand/expand_controller.go",
            "validate",
            "tenant-pv-exp-not-allowed"
        );
        let v = view(20, 10, false, ExpansionPhase::Pending);
        assert!(validate(&v).is_err());
        assert!(matches!(next_step(&v), NextStep::Reject(_)));
    }

    #[test]
    fn pending_starts_controller_resize() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/expand/expand_controller.go",
            "syncHandler",
            "tenant-pv-exp-start"
        );
        let v = view(20, 10, true, ExpansionPhase::Pending);
        assert_eq!(next_step(&v), NextStep::StartControllerResize);
    }

    #[test]
    fn resizing_waits_until_controller_resize_complete() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/expand/expand_controller.go",
            "syncHandler",
            "tenant-pv-exp-wait-controller"
        );
        let v = view(20, 10, true, ExpansionPhase::Resizing);
        assert_eq!(next_step(&v), NextStep::NoOp);
    }

    #[test]
    fn resizing_with_controller_done_proceeds_to_fs_phase() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/expand/expand_controller.go",
            "syncHandler",
            "tenant-pv-exp-await-fs"
        );
        let mut v = view(20, 10, true, ExpansionPhase::Resizing);
        v.controller_resize_complete = true;
        assert_eq!(next_step(&v), NextStep::AwaitFileSystemResize);
    }

    #[test]
    fn fs_pending_marks_resized() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/expand/expand_controller.go",
            "syncHandler",
            "tenant-pv-exp-mark-resized"
        );
        let v = view(20, 20, true, ExpansionPhase::FileSystemResizePending);
        // status capacity has caught up to request → mark resized.
        assert_eq!(next_step(&v), NextStep::MarkResized);
    }

    #[test]
    fn resized_phase_is_noop() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/expand/expand_controller.go",
            "syncHandler",
            "tenant-pv-exp-resized-noop"
        );
        let v = view(20, 20, true, ExpansionPhase::Resized);
        assert_eq!(next_step(&v), NextStep::NoOp);
    }

    #[test]
    fn validate_growth_with_allow_is_ok() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/expand/expand_controller.go",
            "validate",
            "tenant-pv-exp-validate-ok"
        );
        let v = view(20, 10, true, ExpansionPhase::Pending);
        assert!(validate(&v).is_ok());
    }

    #[test]
    fn next_step_serde_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/expand/expand_controller.go",
            "NextStep",
            "tenant-pv-exp-serde"
        );
        for s in [
            NextStep::NoOp,
            NextStep::StartControllerResize,
            NextStep::AwaitFileSystemResize,
            NextStep::MarkResized,
            NextStep::Reject("x".into()),
        ] {
            let bytes = serde_json::to_string(&s).unwrap();
            let back: NextStep = serde_json::from_str(&bytes).unwrap();
            assert_eq!(s, back);
        }
    }
}
