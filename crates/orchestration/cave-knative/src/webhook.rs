// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Admission webhook — validation + defaulting for Knative CRDs.
//!
//! upstream: knative/serving — cmd/webhook + pkg/webhook
//!
//! The upstream binary wires kubernetes admission review JSON into a set
//! of resource-specific validators and defaulters. We port the
//! validator + defaulter functions and a transport-agnostic dispatcher
//! that takes a `WebhookRequest` and returns a `WebhookResponse`. The
//! HTTP framing layer (TLS + cert rotation + secret-pair install) is
//! owned by cave-admission; this module focuses on the policy decisions.

use crate::ksvc::Ksvc;
use crate::meta::{TrafficTarget, validate_template, validate_traffic};

#[derive(Debug, Clone)]
pub struct WebhookRequest {
    pub kind: String,
    pub operation: AdmissionOp,
    pub namespace: String,
    pub name: String,
    pub object: AdmissionObject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmissionOp {
    Create,
    Update,
    Delete,
}

/// Strongly-typed admission objects so we don't pull in serde for this
/// crate. The upstream review payload is JSON; we let cave-admission
/// translate at the wire boundary.
#[derive(Debug, Clone)]
pub enum AdmissionObject {
    Service(Box<Ksvc>),
    /// Bare traffic-target list for Route/Configuration admission.
    Traffic(Vec<TrafficTarget>),
}

#[derive(Debug, Clone)]
pub struct WebhookResponse {
    pub allowed: bool,
    pub uid: String,
    pub status_message: Option<String>,
    pub warnings: Vec<String>,
    /// JSON-Patch operations to apply (defaulting). Empty when no
    /// defaulting was needed.
    pub patch: Vec<PatchOp>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchOp {
    pub op: String,
    pub path: String,
    pub value: String,
}

/// Run the admission decision against a single webhook request.
pub fn admit(req: &WebhookRequest, uid: &str) -> WebhookResponse {
    let mut response = WebhookResponse {
        allowed: false,
        uid: uid.to_string(),
        status_message: None,
        warnings: Vec::new(),
        patch: Vec::new(),
    };

    if req.operation == AdmissionOp::Delete {
        response.allowed = true;
        return response;
    }

    match &req.object {
        AdmissionObject::Service(svc) => {
            if let Err(e) = svc.validate() {
                response.status_message = Some(format!("validation failed: {}", e));
                return response;
            }
            // Defaulting: if name is empty, inject a placeholder & emit warning.
            if svc.metadata.name.is_empty() {
                response
                    .warnings
                    .push("Ksvc name is empty; controller will reject".into());
            }
            // Defaulting: ensure the autoscaler class annotation is set.
            if svc
                .metadata
                .annotations
                .get(crate::meta::ANNOTATION_AUTOSCALER_CLASS)
                .is_none()
            {
                response.patch.push(PatchOp {
                    op: "add".into(),
                    path: format!(
                        "/metadata/annotations/{}",
                        json_pointer_escape(crate::meta::ANNOTATION_AUTOSCALER_CLASS)
                    ),
                    value: "kpa.autoscaling.knative.dev".into(),
                });
            }
            response.allowed = true;
        }
        AdmissionObject::Traffic(targets) => {
            if let Err(e) = validate_traffic(targets) {
                response.status_message = Some(format!("traffic invalid: {}", e));
                return response;
            }
            response.allowed = true;
        }
    }

    response
}

/// Validate the template inside a Ksvc and return a webhook-style
/// rejection on failure. Mirrors the standalone validator entry point
/// the upstream uses for Configuration admission.
pub fn validate_ksvc_template(svc: &Ksvc) -> Result<(), String> {
    validate_template(&svc.spec.template)
}

/// Escape a JSON pointer segment per RFC 6901 (replace '~' and '/').
pub fn json_pointer_escape(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::{Container, ObjectMeta, PodSpec, RevisionTemplateSpec};

    fn good_svc(name: &str) -> Ksvc {
        let mut s = Ksvc::new("t");
        s.metadata.name = name.to_string();
        s.spec.template = RevisionTemplateSpec {
            metadata: ObjectMeta::default(),
            spec: PodSpec {
                containers: vec![Container {
                    name: "app".into(),
                    image: "nginx:1".into(),
                    env: vec![],
                }],
            },
        };
        s
    }

    #[test]
    fn admit_delete_is_always_allowed() {
        let req = WebhookRequest {
            kind: "Service".into(),
            operation: AdmissionOp::Delete,
            namespace: "default".into(),
            name: "svc".into(),
            object: AdmissionObject::Service(Box::new(good_svc("svc"))),
        };
        let res = admit(&req, "uid-1");
        assert!(res.allowed);
    }

    #[test]
    fn admit_create_allows_valid_ksvc() {
        let req = WebhookRequest {
            kind: "Service".into(),
            operation: AdmissionOp::Create,
            namespace: "default".into(),
            name: "svc".into(),
            object: AdmissionObject::Service(Box::new(good_svc("svc"))),
        };
        let res = admit(&req, "uid-1");
        assert!(res.allowed);
        assert_eq!(res.status_message, None);
    }

    #[test]
    fn admit_create_rejects_ksvc_without_containers() {
        let mut svc = good_svc("svc");
        svc.spec.template.spec.containers.clear();
        let req = WebhookRequest {
            kind: "Service".into(),
            operation: AdmissionOp::Create,
            namespace: "default".into(),
            name: "svc".into(),
            object: AdmissionObject::Service(Box::new(svc)),
        };
        let res = admit(&req, "uid");
        assert!(!res.allowed);
        assert!(
            res.status_message
                .as_deref()
                .unwrap()
                .contains("validation failed")
        );
    }

    #[test]
    fn admit_defaulting_injects_autoscaler_class_annotation() {
        let req = WebhookRequest {
            kind: "Service".into(),
            operation: AdmissionOp::Create,
            namespace: "default".into(),
            name: "svc".into(),
            object: AdmissionObject::Service(Box::new(good_svc("svc"))),
        };
        let res = admit(&req, "uid");
        let any_class_patch = res
            .patch
            .iter()
            .any(|p| p.path.contains("autoscaling.knative.dev~1class"));
        assert!(any_class_patch);
    }

    #[test]
    fn admit_warns_when_name_is_empty() {
        let svc = good_svc("");
        let req = WebhookRequest {
            kind: "Service".into(),
            operation: AdmissionOp::Create,
            namespace: "default".into(),
            name: "".into(),
            object: AdmissionObject::Service(Box::new(svc)),
        };
        let res = admit(&req, "uid");
        assert!(res.warnings.iter().any(|w| w.contains("empty")));
    }

    #[test]
    fn admit_traffic_rejects_sum_not_100() {
        let targets = vec![
            TrafficTarget {
                revision_name: Some("a".into()),
                percent: Some(40),
                ..Default::default()
            },
            TrafficTarget {
                revision_name: Some("b".into()),
                percent: Some(40),
                ..Default::default()
            },
        ];
        let req = WebhookRequest {
            kind: "Route".into(),
            operation: AdmissionOp::Update,
            namespace: "default".into(),
            name: "r".into(),
            object: AdmissionObject::Traffic(targets),
        };
        let res = admit(&req, "uid");
        assert!(!res.allowed);
        assert!(res.status_message.as_deref().unwrap().contains("100"));
    }

    #[test]
    fn admit_traffic_accepts_valid_split() {
        let targets = vec![
            TrafficTarget {
                revision_name: Some("a".into()),
                percent: Some(50),
                ..Default::default()
            },
            TrafficTarget {
                revision_name: Some("b".into()),
                percent: Some(50),
                ..Default::default()
            },
        ];
        let req = WebhookRequest {
            kind: "Route".into(),
            operation: AdmissionOp::Create,
            namespace: "default".into(),
            name: "r".into(),
            object: AdmissionObject::Traffic(targets),
        };
        let res = admit(&req, "uid");
        assert!(res.allowed);
    }

    #[test]
    fn json_pointer_escape_handles_slash_and_tilde() {
        assert_eq!(json_pointer_escape("foo/bar"), "foo~1bar");
        assert_eq!(json_pointer_escape("a~b"), "a~0b");
        assert_eq!(
            json_pointer_escape("autoscaling.knative.dev/class"),
            "autoscaling.knative.dev~1class"
        );
    }
}
