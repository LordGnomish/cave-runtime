//! HPA target ref + scale subresource — `pkg/controller/podautoscaler/horizontal.go::reconcileAutoscaler`.
//!
//! `spec.scaleTargetRef` points at the controller (Deployment / ReplicaSet /
//! StatefulSet / ReplicationController / a CRD with `/scale` subresource).
//! Validation:
//!
//! * `apiVersion` non-empty, `kind` non-empty, `name` non-empty.
//! * Kind must be among the known scale-able kinds (or a CRD that exposes
//!   `/scale`; we model this by an explicit allow-list).
//! * Cross-namespace targets are rejected (HPA scopes to its own namespace).
//!
//! `scale_subresource_path` returns the API path used to read/write the
//! `Scale` object: `apis/<group>/<version>/namespaces/<ns>/<resource>/<name>/scale`.

use crate::types::{Cite, ControllerError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaleTargetRef {
    pub api_version: String,
    pub kind: String,
    pub name: String,
}

/// Built-in kinds that always carry `/scale`. Mirrors the list maintained
/// in `pkg/registry/autoscaling/horizontalpodautoscaler/storage/storage.go`.
pub const KNOWN_SCALEABLE_KINDS: &[&str] = &[
    "Deployment",
    "ReplicaSet",
    "ReplicationController",
    "StatefulSet",
];

pub fn is_known_scaleable(kind: &str) -> bool {
    KNOWN_SCALEABLE_KINDS.contains(&kind)
}

/// Validate the target ref. Returns the resource segment name that should be
/// used in URL paths (lowercased, plural).
pub fn validate(target: &ScaleTargetRef) -> Result<String, ControllerError> {
    if target.api_version.is_empty() {
        return Err(ControllerError::InvalidSpec {
            kind: "HorizontalPodAutoscaler",
            reason: "scaleTargetRef.apiVersion required".into(),
        });
    }
    if target.kind.is_empty() {
        return Err(ControllerError::InvalidSpec {
            kind: "HorizontalPodAutoscaler",
            reason: "scaleTargetRef.kind required".into(),
        });
    }
    if target.name.is_empty() {
        return Err(ControllerError::InvalidSpec {
            kind: "HorizontalPodAutoscaler",
            reason: "scaleTargetRef.name required".into(),
        });
    }
    if !is_known_scaleable(&target.kind) {
        return Err(ControllerError::InvalidSpec {
            kind: "HorizontalPodAutoscaler",
            reason: format!("kind {} does not support /scale", target.kind),
        });
    }
    Ok(plural_for(&target.kind))
}

fn plural_for(kind: &str) -> String {
    match kind {
        "Deployment" => "deployments".into(),
        "ReplicaSet" => "replicasets".into(),
        "ReplicationController" => "replicationcontrollers".into(),
        "StatefulSet" => "statefulsets".into(),
        // Generic plural: lowercase + "s" (best-effort fallback).
        other => format!("{}s", other.to_ascii_lowercase()),
    }
}

/// Compute the `/scale` subresource path for the target.
pub fn scale_subresource_path(
    target: &ScaleTargetRef,
    namespace: &str,
) -> Result<String, ControllerError> {
    let resource = validate(target)?;
    let (group, version) = match target.api_version.split_once('/') {
        Some((g, v)) => (g, v),
        None => ("", target.api_version.as_str()),
    };
    let group_segment = if group.is_empty() {
        format!("api/{}", version)
    } else {
        format!("apis/{}/{}", group, version)
    };
    Ok(format!(
        "{}/namespaces/{}/{}/{}/scale",
        group_segment, namespace, resource, target.name
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/podautoscaler/horizontal.go",
    "reconcileAutoscaler",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn tref(api: &str, kind: &str, name: &str) -> ScaleTargetRef {
        ScaleTargetRef {
            api_version: api.into(),
            kind: kind.into(),
            name: name.into(),
        }
    }

    #[test]
    fn valid_deployment_target_passes() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "reconcileAutoscaler",
            "tenant-hpa-tref-dep"
        );
        let t = tref("apps/v1", "Deployment", "web");
        assert_eq!(validate(&t).unwrap(), "deployments");
    }

    #[test]
    fn missing_kind_rejected() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "reconcileAutoscaler",
            "tenant-hpa-tref-no-kind"
        );
        let t = tref("apps/v1", "", "web");
        assert!(validate(&t).is_err());
    }

    #[test]
    fn missing_name_rejected() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "reconcileAutoscaler",
            "tenant-hpa-tref-no-name"
        );
        let t = tref("apps/v1", "Deployment", "");
        assert!(validate(&t).is_err());
    }

    #[test]
    fn missing_api_version_rejected() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "reconcileAutoscaler",
            "tenant-hpa-tref-no-api"
        );
        let t = tref("", "Deployment", "web");
        assert!(validate(&t).is_err());
    }

    #[test]
    fn unknown_kind_rejected_for_builtin_path() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "reconcileAutoscaler",
            "tenant-hpa-tref-unknown-kind"
        );
        let t = tref("apps/v1", "Pod", "web");
        assert!(validate(&t).is_err());
    }

    #[test]
    fn statefulset_resolves_to_statefulsets_plural() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/registry/autoscaling/horizontalpodautoscaler/storage/storage.go",
            "ScaleStorage",
            "tenant-hpa-tref-sts"
        );
        let t = tref("apps/v1", "StatefulSet", "db");
        assert_eq!(validate(&t).unwrap(), "statefulsets");
    }

    #[test]
    fn replication_controller_uses_core_group_path() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/registry/core/replicationcontroller/storage/storage.go",
            "ScaleStorage",
            "tenant-hpa-tref-rc-path"
        );
        let t = tref("v1", "ReplicationController", "rc-a");
        let p = scale_subresource_path(&t, "default").unwrap();
        assert_eq!(
            p,
            "api/v1/namespaces/default/replicationcontrollers/rc-a/scale"
        );
    }

    #[test]
    fn deployment_path_uses_apps_group() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/registry/apps/deployment/storage/storage.go",
            "ScaleStorage",
            "tenant-hpa-tref-dep-path"
        );
        let t = tref("apps/v1", "Deployment", "web");
        let p = scale_subresource_path(&t, "prod").unwrap();
        assert_eq!(p, "apis/apps/v1/namespaces/prod/deployments/web/scale");
    }

    #[test]
    fn replicaset_path() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/registry/apps/replicaset/storage/storage.go",
            "ScaleStorage",
            "tenant-hpa-tref-rs-path"
        );
        let t = tref("apps/v1", "ReplicaSet", "rs-1");
        let p = scale_subresource_path(&t, "default").unwrap();
        assert_eq!(p, "apis/apps/v1/namespaces/default/replicasets/rs-1/scale");
    }

    #[test]
    fn target_ref_serde_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/api/autoscaling/v2/types.go",
            "CrossVersionObjectReference",
            "tenant-hpa-tref-serde"
        );
        let t = tref("apps/v1", "Deployment", "web");
        let s = serde_json::to_string(&t).unwrap();
        let back: ScaleTargetRef = serde_json::from_str(&s).unwrap();
        assert_eq!(t.kind, back.kind);
    }

    #[test]
    fn known_scaleable_kinds_constant() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/registry/autoscaling/horizontalpodautoscaler/storage/storage.go",
            "ScaleStorage",
            "tenant-hpa-tref-known"
        );
        assert!(is_known_scaleable("Deployment"));
        assert!(is_known_scaleable("StatefulSet"));
        assert!(is_known_scaleable("ReplicaSet"));
        assert!(is_known_scaleable("ReplicationController"));
        assert!(!is_known_scaleable("DaemonSet"));
    }
}
