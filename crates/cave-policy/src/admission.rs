//! Kubernetes ValidatingAdmissionWebhook-compatible admission control.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Top-level AdmissionReview wrapper (v1).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionReview {
    pub api_version: String,
    pub kind: String,
    pub request: Option<AdmissionRequest>,
    pub response: Option<AdmissionResponse>,
}

impl AdmissionReview {
    pub fn new_response(uid: String, allowed: bool, message: Option<String>) -> Self {
        Self {
            api_version: "admission.k8s.io/v1".to_string(),
            kind: "AdmissionReview".to_string(),
            request: None,
            response: Some(AdmissionResponse {
                uid,
                allowed,
                status: message.map(|m| AdmissionStatus {
                    code: if allowed { 200 } else { 403 },
                    message: m,
                }),
                warnings: None,
                patch: None,
                patch_type: None,
            }),
        }
    }
}

/// Incoming admission request from the Kubernetes API server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionRequest {
    pub uid: String,
    pub kind: GroupVersionKind,
    pub resource: GroupVersionResource,
    pub name: Option<String>,
    pub namespace: Option<String>,
    pub operation: Operation,
    pub object: Option<Value>,
    pub old_object: Option<Value>,
    pub dry_run: Option<bool>,
    pub user_info: Option<UserInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupVersionKind {
    pub group: String,
    pub version: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupVersionResource {
    pub group: String,
    pub version: String,
    pub resource: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Operation {
    Create,
    Update,
    Delete,
    Connect,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub username: Option<String>,
    pub uid: Option<String>,
    pub groups: Option<Vec<String>>,
}

/// Outgoing admission response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionResponse {
    pub uid: String,
    pub allowed: bool,
    pub status: Option<AdmissionStatus>,
    pub warnings: Option<Vec<String>>,
    pub patch: Option<String>,
    pub patch_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdmissionStatus {
    pub code: u16,
    pub message: String,
}

/// Evaluate an admission request against a compiled policy.
///
/// The policy should define a rule named `allow` (bool) and optionally
/// `deny_message` (string).  The request is exposed as `input`.
pub fn evaluate_admission(
    review: &AdmissionReview,
    policy: &crate::engine::ast::Policy,
    data: &Value,
) -> AdmissionReview {
    let request = match &review.request {
        Some(r) => r,
        None => return AdmissionReview::new_response(
            String::new(), false, Some("missing admission request".into()),
        ),
    };

    // Build input from the admission request
    let input = serde_json::json!({
        "request": {
            "uid": request.uid,
            "kind": request.kind,
            "resource": request.resource,
            "name": request.name,
            "namespace": request.namespace,
            "operation": request.operation,
            "object": request.object,
            "oldObject": request.old_object,
            "dryRun": request.dry_run,
            "userInfo": request.user_info,
        }
    });

    let results = crate::engine::evaluate(policy, &input, data);

    let allowed = results
        .get("allow")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let message = results
        .get("deny_message")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            if !allowed {
                Some("Request denied by policy".to_string())
            } else {
                None
            }
        });

    AdmissionReview::new_response(request.uid.clone(), allowed, message)
}
