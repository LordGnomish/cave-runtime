//! `volume/ephemeral` — generic-ephemeral-volume controller.
//!
//! Mirrors `pkg/controller/volume/ephemeral/` from upstream.
//! When a Pod declares `volumes[].ephemeral.volumeClaimTemplate`,
//! this controller materialises a real `PersistentVolumeClaim`
//! named `<podName>-<volumeName>` and adopts it via owner
//! reference so deleting the pod garbage-collects the PVC.
//!
//! State machine:
//!
//! 1. Pod created with ephemeral volume → controller creates
//!    PVC if absent.
//! 2. PVC already exists with mismatched owner → reconciler
//!    surfaces an "owner conflict" error rather than adopting
//!    (upstream is strict about this to avoid hijacking).
//! 3. Pod deleted → PVC GC happens automatically via owner
//!    reference; this controller does not delete directly.

use std::collections::BTreeMap;

/// Reference back to the pod that owns this ephemeral volume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PodRef {
    pub namespace: String,
    pub name: String,
    pub uid: String,
}

/// One ephemeral-volume declaration on a pod spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EphemeralVolumeDecl {
    pub pod: PodRef,
    /// `volumes[].name` on the pod spec.
    pub volume_name: String,
    /// `volumeClaimTemplate.spec.storageClassName` (or `""` if
    /// the cluster default applies).
    pub storage_class: String,
    /// `volumeClaimTemplate.spec.resources.requests.storage`
    /// in raw bytes.
    pub size_bytes: u64,
}

/// Reduced view of a `PersistentVolumeClaim` for the
/// reconciler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedPvc {
    pub namespace: String,
    pub name: String,
    /// `metadata.ownerReferences[].uid` for the owning pod, if
    /// any.
    pub owner_pod_uid: Option<String>,
}

/// What the reconciler decides for one ephemeral-volume
/// declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// PVC doesn't exist — create it.
    Create {
        namespace: String,
        name: String,
        storage_class: String,
        size_bytes: u64,
        owner_pod_uid: String,
    },
    /// PVC exists with the correct owner-reference.
    AlreadyAdopted,
    /// PVC exists but the owner reference points elsewhere —
    /// reconciler refuses to hijack.
    OwnerConflict {
        pvc_namespace: String,
        pvc_name: String,
        observed_owner: Option<String>,
        expected_owner: String,
    },
}

/// PVC name convention from upstream: `<podName>-<volumeName>`.
pub fn pvc_name_for(pod_name: &str, volume_name: &str) -> String {
    format!("{pod_name}-{volume_name}")
}

/// Decide what to do for one declaration.
pub fn evaluate(decl: &EphemeralVolumeDecl, observed: Option<&ObservedPvc>) -> Action {
    let name = pvc_name_for(&decl.pod.name, &decl.volume_name);
    match observed {
        None => Action::Create {
            namespace: decl.pod.namespace.clone(),
            name,
            storage_class: decl.storage_class.clone(),
            size_bytes: decl.size_bytes,
            owner_pod_uid: decl.pod.uid.clone(),
        },
        Some(pvc) => match &pvc.owner_pod_uid {
            Some(uid) if *uid == decl.pod.uid => Action::AlreadyAdopted,
            other => Action::OwnerConflict {
                pvc_namespace: pvc.namespace.clone(),
                pvc_name: pvc.name.clone(),
                observed_owner: other.clone(),
                expected_owner: decl.pod.uid.clone(),
            },
        },
    }
}

/// Tracks the last decision per (namespace, pvc_name) so the
/// admin UI can surface "owner conflict" rows.
#[derive(Debug, Default)]
pub struct EphemeralReconciler {
    last_actions: BTreeMap<(String, String), Action>,
}

impl EphemeralReconciler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reconcile(
        &mut self,
        decl: &EphemeralVolumeDecl,
        observed: Option<&ObservedPvc>,
    ) -> Action {
        let action = evaluate(decl, observed);
        let key = (
            decl.pod.namespace.clone(),
            pvc_name_for(&decl.pod.name, &decl.volume_name),
        );
        self.last_actions.insert(key, action.clone());
        action
    }

    pub fn conflicts(&self) -> Vec<&Action> {
        self.last_actions
            .values()
            .filter(|a| matches!(a, Action::OwnerConflict { .. }))
            .collect()
    }

    pub fn tracked(&self) -> usize {
        self.last_actions.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(pod_name: &str, vol: &str, uid: &str) -> EphemeralVolumeDecl {
        EphemeralVolumeDecl {
            pod: PodRef {
                namespace: "ns".into(),
                name: pod_name.into(),
                uid: uid.into(),
            },
            volume_name: vol.into(),
            storage_class: "fast".into(),
            size_bytes: 1024 * 1024,
        }
    }

    #[test]
    fn pvc_name_uses_pod_dash_volume() {
        assert_eq!(pvc_name_for("pod", "scratch"), "pod-scratch");
    }

    #[test]
    fn evaluate_create_when_missing() {
        let d = decl("pod", "scratch", "uid-1");
        let a = evaluate(&d, None);
        match a {
            Action::Create { name, owner_pod_uid, .. } => {
                assert_eq!(name, "pod-scratch");
                assert_eq!(owner_pod_uid, "uid-1");
            }
            other => panic!("wrong action: {other:?}"),
        }
    }

    #[test]
    fn evaluate_already_adopted_when_owner_matches() {
        let d = decl("pod", "scratch", "uid-1");
        let pvc = ObservedPvc {
            namespace: "ns".into(),
            name: "pod-scratch".into(),
            owner_pod_uid: Some("uid-1".into()),
        };
        assert_eq!(evaluate(&d, Some(&pvc)), Action::AlreadyAdopted);
    }

    #[test]
    fn evaluate_owner_conflict_when_owner_differs() {
        let d = decl("pod", "scratch", "uid-1");
        let pvc = ObservedPvc {
            namespace: "ns".into(),
            name: "pod-scratch".into(),
            owner_pod_uid: Some("other-uid".into()),
        };
        match evaluate(&d, Some(&pvc)) {
            Action::OwnerConflict {
                observed_owner,
                expected_owner,
                ..
            } => {
                assert_eq!(observed_owner.as_deref(), Some("other-uid"));
                assert_eq!(expected_owner, "uid-1");
            }
            other => panic!("wrong action: {other:?}"),
        }
    }

    #[test]
    fn evaluate_owner_conflict_when_owner_missing() {
        let d = decl("pod", "scratch", "uid-1");
        let pvc = ObservedPvc {
            namespace: "ns".into(),
            name: "pod-scratch".into(),
            owner_pod_uid: None,
        };
        match evaluate(&d, Some(&pvc)) {
            Action::OwnerConflict { observed_owner, .. } => {
                assert!(observed_owner.is_none());
            }
            other => panic!("wrong action: {other:?}"),
        }
    }

    #[test]
    fn reconcile_records_last_action() {
        let mut r = EphemeralReconciler::new();
        let d = decl("pod", "v", "uid");
        r.reconcile(&d, None);
        assert_eq!(r.tracked(), 1);
    }

    #[test]
    fn conflicts_filters_to_only_owner_conflicts() {
        let mut r = EphemeralReconciler::new();
        // adopted
        r.reconcile(
            &decl("pod-a", "v", "uid-a"),
            Some(&ObservedPvc {
                namespace: "ns".into(),
                name: "pod-a-v".into(),
                owner_pod_uid: Some("uid-a".into()),
            }),
        );
        // conflict
        r.reconcile(
            &decl("pod-b", "v", "uid-b"),
            Some(&ObservedPvc {
                namespace: "ns".into(),
                name: "pod-b-v".into(),
                owner_pod_uid: Some("uid-x".into()),
            }),
        );
        // create
        r.reconcile(&decl("pod-c", "v", "uid-c"), None);
        let c = r.conflicts();
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn reconcile_overwrites_last_action_for_same_pvc() {
        let mut r = EphemeralReconciler::new();
        let d = decl("pod", "v", "uid-1");
        r.reconcile(&d, None);
        r.reconcile(
            &d,
            Some(&ObservedPvc {
                namespace: "ns".into(),
                name: "pod-v".into(),
                owner_pod_uid: Some("uid-1".into()),
            }),
        );
        assert_eq!(r.tracked(), 1);
        let key = ("ns".to_string(), "pod-v".to_string());
        assert_eq!(r.last_actions.get(&key), Some(&Action::AlreadyAdopted));
    }
}
